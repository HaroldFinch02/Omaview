use std::ffi::CString;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use image::DynamicImage;

pub const RAW_EXTENSIONS: &[&str] = &[
    "arw", "cr2", "cr3", "nef", "nrw", "raf", "rw2", "orf", "dng", "pef", "3fr", "iiq",
];

pub fn is_raw_image(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        RAW_EXTENSIONS.contains(&lower.as_str())
    } else {
        false
    }
}

pub fn raw_format_badge(path: &Path) -> &'static str {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "arw" => "ARW",
            "cr2" => "CR2",
            "cr3" => "CR3",
            "nef" | "nrw" => "NEF",
            "raf" => "RAF",
            "rw2" => "RW2",
            "orf" => "ORF",
            "dng" => "DNG",
            "pef" => "PEF",
            "3fr" => "3FR",
            "iiq" => "IIQ",
            _ => "RAW",
        }
    } else {
        "RAW"
    }
}

pub fn find_companion_jpeg(raw_path: &Path) -> Option<PathBuf> {
    let stem = raw_path.file_stem()?.to_str()?;
    let parent = raw_path.parent()?;

    for ext in &["jpg", "jpeg", "JPG", "JPEG"] {
        let candidate = parent.join(format!("{}.{}", stem, ext));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn get_raw_cache_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".cache")
        });
    let dir = base.join("omaview").join("raw_previews");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn compute_raw_cache_key(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0));

    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in canonical.to_string_lossy().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= size;
    hash = hash.wrapping_mul(0x100000001b3);
    hash ^= mtime.0;
    hash = hash.wrapping_mul(0x100000001b3);
    hash ^= mtime.1 as u64;
    hash = hash.wrapping_mul(0x100000001b3);

    Some(format!("{:016x}", hash))
}

pub fn get_cached_raw_preview_path(raw_path: &Path) -> Option<PathBuf> {
    let key = compute_raw_cache_key(raw_path)?;
    let cache_dir = get_raw_cache_dir();
    let candidate_jpg = cache_dir.join(format!("{}.jpg", key));
    if candidate_jpg.is_file() {
        return Some(candidate_jpg);
    }
    let candidate_ppm = cache_dir.join(format!("{}.ppm", key));
    if candidate_ppm.is_file() {
        return Some(candidate_ppm);
    }
    None
}

// ---------------------------------------------------------------------------
// Extraction Engine 1: LibRaw Dynamic Loader via dlopen
// ---------------------------------------------------------------------------

type LibrawInitFn = unsafe extern "C" fn(flags: std::os::raw::c_uint) -> *mut std::ffi::c_void;
type LibrawOpenFileFn = unsafe extern "C" fn(
    lr: *mut std::ffi::c_void,
    file: *const std::os::raw::c_char,
) -> std::os::raw::c_int;
type LibrawUnpackThumbFn =
    unsafe extern "C" fn(lr: *mut std::ffi::c_void) -> std::os::raw::c_int;
type LibrawDcrawThumbWriterFn = unsafe extern "C" fn(
    lr: *mut std::ffi::c_void,
    fname: *const std::os::raw::c_char,
) -> std::os::raw::c_int;
type LibrawCloseFn = unsafe extern "C" fn(lr: *mut std::ffi::c_void);

struct LibRawApi {
    _handle: *mut std::ffi::c_void,
    init: LibrawInitFn,
    open_file: LibrawOpenFileFn,
    unpack_thumb: LibrawUnpackThumbFn,
    dcraw_thumb_writer: LibrawDcrawThumbWriterFn,
    close: LibrawCloseFn,
}

unsafe impl Send for LibRawApi {}
unsafe impl Sync for LibRawApi {}

static LIBRAW_API: OnceLock<Option<LibRawApi>> = OnceLock::new();

fn get_libraw_api() -> Option<&'static LibRawApi> {
    LIBRAW_API
        .get_or_init(|| {
            let lib_names = ["libraw.so.25", "libraw.so", "libraw.so.23"];
            for name in lib_names {
                let c_name = CString::new(name).ok()?;
                let handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_LAZY) };
                if !handle.is_null() {
                    unsafe {
                        let init_sym = libc::dlsym(handle, b"libraw_init\0".as_ptr() as _);
                        let open_sym = libc::dlsym(handle, b"libraw_open_file\0".as_ptr() as _);
                        let unpack_sym = libc::dlsym(handle, b"libraw_unpack_thumb\0".as_ptr() as _);
                        let writer_sym =
                            libc::dlsym(handle, b"libraw_dcraw_thumb_writer\0".as_ptr() as _);
                        let close_sym = libc::dlsym(handle, b"libraw_close\0".as_ptr() as _);

                        if !init_sym.is_null()
                            && !open_sym.is_null()
                            && !unpack_sym.is_null()
                            && !writer_sym.is_null()
                            && !close_sym.is_null()
                        {
                            return Some(LibRawApi {
                                _handle: handle,
                                init: std::mem::transmute(init_sym),
                                open_file: std::mem::transmute(open_sym),
                                unpack_thumb: std::mem::transmute(unpack_sym),
                                dcraw_thumb_writer: std::mem::transmute(writer_sym),
                                close: std::mem::transmute(close_sym),
                            });
                        } else {
                            libc::dlclose(handle);
                        }
                    }
                }
            }
            None
        })
        .as_ref()
}

fn extract_via_libraw(raw_path: &Path, out_file: &Path) -> Result<PathBuf, String> {
    let api = get_libraw_api().ok_or_else(|| "LibRaw shared library not found".to_string())?;

    let c_path = CString::new(raw_path.to_string_lossy().as_bytes())
        .map_err(|e| format!("Invalid path: {}", e))?;
    let c_out = CString::new(out_file.to_string_lossy().as_bytes())
        .map_err(|e| format!("Invalid out path: {}", e))?;

    unsafe {
        let lr = (api.init)(0);
        if lr.is_null() {
            return Err("Failed to initialize LibRaw".to_string());
        }

        let ret = (api.open_file)(lr, c_path.as_ptr());
        if ret != 0 {
            (api.close)(lr);
            return Err(format!("LibRaw open_file error code {}", ret));
        }

        let ret_thumb = (api.unpack_thumb)(lr);
        if ret_thumb != 0 {
            (api.close)(lr);
            return Err(format!("LibRaw unpack_thumb error code {}", ret_thumb));
        }

        let ret_writer = (api.dcraw_thumb_writer)(lr, c_out.as_ptr());
        (api.close)(lr);

        if ret_writer != 0 {
            return Err(format!("LibRaw thumb_writer error code {}", ret_writer));
        }
    }

    if out_file.is_file() && fs::metadata(out_file).map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(out_file.to_path_buf());
    }

    let ppm_out = out_file.with_extension("ppm");
    if ppm_out.is_file() && fs::metadata(&ppm_out).map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(ppm_out);
    }

    Err("LibRaw writer did not produce output file".to_string())
}

// ---------------------------------------------------------------------------
// Extraction Engine 2: Pure-Rust TIFF IFD Tag Reader
// ---------------------------------------------------------------------------

pub fn extract_via_tiff_scan(raw_path: &Path, out_file: &Path) -> Result<PathBuf, String> {
    let mut file = File::open(raw_path)
        .map_err(|e| format!("Failed to open {}: {}", raw_path.display(), e))?;

    let mut header = [0u8; 8];
    if file.read_exact(&mut header).is_err() {
        return Err("File too small for TIFF header".to_string());
    }

    let is_le = match &header[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err("Not a TIFF-based RAW file".to_string()),
    };

    let magic = if is_le {
        u16::from_le_bytes([header[2], header[3]])
    } else {
        u16::from_be_bytes([header[2], header[3]])
    };

    if magic != 42 && magic != 0x55 && magic != 0x4352 {
        return Err(format!("Unrecognized TIFF magic: {}", magic));
    }

    let first_ifd_offset = if is_le {
        u32::from_le_bytes([header[4], header[5], header[6], header[7]])
    } else {
        u32::from_be_bytes([header[4], header[5], header[6], header[7]])
    };

    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut ifd_offset = first_ifd_offset as u64;
    let mut visited = 0;

    while ifd_offset > 0 && ifd_offset < file_len && visited < 10 {
        visited += 1;
        if file.seek(SeekFrom::Start(ifd_offset)).is_err() {
            break;
        }

        let mut num_entries_bytes = [0u8; 2];
        if file.read_exact(&mut num_entries_bytes).is_err() {
            break;
        }

        let num_entries = if is_le {
            u16::from_le_bytes(num_entries_bytes)
        } else {
            u16::from_be_bytes(num_entries_bytes)
        };

        let mut jpeg_offset = None;
        let mut jpeg_length = None;
        let mut sub_ifd_offsets = Vec::new();

        for _ in 0..num_entries {
            let mut entry = [0u8; 12];
            if file.read_exact(&mut entry).is_err() {
                break;
            }

            let tag = if is_le {
                u16::from_le_bytes([entry[0], entry[1]])
            } else {
                u16::from_be_bytes([entry[0], entry[1]])
            };
            let count = if is_le {
                u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]])
            } else {
                u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]])
            };
            let val_or_offset = if is_le {
                u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]])
            } else {
                u32::from_be_bytes([entry[8], entry[9], entry[10], entry[11]])
            };

            match tag {
                0x0201 => jpeg_offset = Some(val_or_offset as u64),
                0x0202 => jpeg_length = Some(val_or_offset as u64),
                0x014a => {
                    if count == 1 {
                        sub_ifd_offsets.push(val_or_offset as u64);
                    }
                }
                _ => {}
            }
        }

        if let (Some(offset), Some(length)) = (jpeg_offset, jpeg_length) {
            if offset + length <= file_len && length >= 4 {
                if file.seek(SeekFrom::Start(offset)).is_ok() {
                    let mut magic_check = [0u8; 2];
                    if file.read_exact(&mut magic_check).is_ok() && magic_check == [0xFF, 0xD8] {
                        file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                        let mut buf = vec![0u8; length as usize];
                        file.read_exact(&mut buf).map_err(|e| e.to_string())?;

                        let mut out = File::create(out_file).map_err(|e| e.to_string())?;
                        out.write_all(&buf).map_err(|e| e.to_string())?;
                        return Ok(out_file.to_path_buf());
                    }
                }
            }
        }

        let mut next_ifd_bytes = [0u8; 4];
        if file.read_exact(&mut next_ifd_bytes).is_ok() {
            let next_offset = if is_le {
                u32::from_le_bytes(next_ifd_bytes)
            } else {
                u32::from_be_bytes(next_ifd_bytes)
            };
            if next_offset > 0 {
                ifd_offset = next_offset as u64;
                continue;
            }
        }

        if let Some(sub_offset) = sub_ifd_offsets.pop() {
            ifd_offset = sub_offset;
        } else {
            break;
        }
    }

    Err("No embedded JPEG tag found in TIFF structure".to_string())
}

// ---------------------------------------------------------------------------
// Extraction Engine 3: CLI fallback (exiv2 / dcraw_emu)
// ---------------------------------------------------------------------------

fn extract_via_cli(raw_path: &Path, out_file: &Path) -> Result<PathBuf, String> {
    let out_dir = out_file.parent().unwrap_or_else(|| Path::new("."));
    
    // Try exiv2 first
    let res = std::process::Command::new("exiv2")
        .arg("-e")
        .arg("p3")
        .arg("-l")
        .arg(out_dir)
        .arg(raw_path)
        .output();

    if let Ok(output) = res {
        if output.status.success() {
            let stem = raw_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            for ext in &["jpg", "jpeg"] {
                let candidate = out_dir.join(format!("{}-preview3.{}", stem, ext));
                if candidate.is_file() {
                    let _ = fs::rename(&candidate, out_file);
                    return Ok(out_file.to_path_buf());
                }
            }
        }
    }

    // Try dcraw_emu
    let res_dcraw = std::process::Command::new("dcraw_emu")
        .arg("-e")
        .arg(raw_path)
        .current_dir(out_dir)
        .output();

    if let Ok(output) = res_dcraw {
        if output.status.success() {
            let stem = raw_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            for ext in &["thumb.jpg", "thumb.ppm"] {
                let candidate = out_dir.join(format!("{}.{}", stem, ext));
                if candidate.is_file() {
                    let _ = fs::rename(&candidate, out_file);
                    return Ok(out_file.to_path_buf());
                }
            }
        }
    }

    Err("CLI extraction (exiv2 / dcraw_emu) failed".to_string())
}

// ---------------------------------------------------------------------------
// Unified Public Extraction API
// ---------------------------------------------------------------------------

pub fn get_or_extract_raw_preview(raw_path: &Path) -> Result<PathBuf, String> {
    if let Some(companion) = find_companion_jpeg(raw_path) {
        return Ok(companion);
    }

    if let Some(cached) = get_cached_raw_preview_path(raw_path) {
        return Ok(cached);
    }

    let key = compute_raw_cache_key(raw_path)
        .ok_or_else(|| "Failed to compute cache key for RAW file".to_string())?;
    let target_cache_file = get_raw_cache_dir().join(format!("{}.jpg", key));

    // Try LibRaw first (covers 100% of cameras including CR3 and RAF)
    if let Ok(path) = extract_via_libraw(raw_path, &target_cache_file) {
        return Ok(path);
    }

    // Try Pure-Rust TIFF parser
    if let Ok(path) = extract_via_tiff_scan(raw_path, &target_cache_file) {
        return Ok(path);
    }

    // Try CLI fallback
    if let Ok(path) = extract_via_cli(raw_path, &target_cache_file) {
        return Ok(path);
    }

    Err(format!(
        "Failed to extract embedded preview from RAW image: {}",
        raw_path.display()
    ))
}

pub fn load_raw_as_dynamic_image(raw_path: &Path) -> Result<DynamicImage, String> {
    let preview_path = get_or_extract_raw_preview(raw_path)?;
    image::ImageReader::open(&preview_path)
        .map_err(|e| format!("Cannot open extracted preview {}: {}", preview_path.display(), e))?
        .with_guessed_format()
        .map_err(|e| format!("Cannot guess format for {}: {}", preview_path.display(), e))?
        .decode()
        .map_err(|e| format!("Cannot decode extracted preview {}: {}", preview_path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_raw_image() {
        assert!(is_raw_image(Path::new("photo.arw")));
        assert!(is_raw_image(Path::new("photo.ARW")));
        assert!(is_raw_image(Path::new("photo.cr2")));
        assert!(is_raw_image(Path::new("photo.cr3")));
        assert!(is_raw_image(Path::new("photo.nef")));
        assert!(is_raw_image(Path::new("photo.dng")));
        assert!(is_raw_image(Path::new("photo.raf")));
        assert!(is_raw_image(Path::new("photo.rw2")));
        assert!(is_raw_image(Path::new("photo.orf")));
        assert!(!is_raw_image(Path::new("photo.jpg")));
        assert!(!is_raw_image(Path::new("photo.png")));
        assert!(!is_raw_image(Path::new("photo.webp")));
    }

    #[test]
    fn test_raw_format_badge() {
        assert_eq!(raw_format_badge(Path::new("test.arw")), "ARW");
        assert_eq!(raw_format_badge(Path::new("test.cr3")), "CR3");
        assert_eq!(raw_format_badge(Path::new("test.nef")), "NEF");
        assert_eq!(raw_format_badge(Path::new("test.dng")), "DNG");
        assert_eq!(raw_format_badge(Path::new("test.raf")), "RAF");
        assert_eq!(raw_format_badge(Path::new("test.rw2")), "RW2");
        assert_eq!(raw_format_badge(Path::new("test.orf")), "ORF");
    }

    #[test]
    fn test_find_companion_jpeg() {
        let temp_dir = std::env::temp_dir().join("omaview_test_companion");
        let _ = fs::create_dir_all(&temp_dir);

        let raw_file = temp_dir.join("DSC_0001.ARW");
        let companion_file = temp_dir.join("DSC_0001.JPG");
        let other_file = temp_dir.join("DSC_0002.ARW");

        let _ = File::create(&raw_file);
        let _ = File::create(&companion_file);
        let _ = File::create(&other_file);

        let found = find_companion_jpeg(&raw_file);
        assert_eq!(found, Some(companion_file.clone()));

        let not_found = find_companion_jpeg(&other_file);
        assert_eq!(not_found, None);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_synthetic_tiff_extraction() {
        let temp_dir = std::env::temp_dir().join("omaview_test_tiff");
        let _ = fs::create_dir_all(&temp_dir);
        let test_raw = temp_dir.join("mock.dng");
        let out_jpg = temp_dir.join("extracted.jpg");

        // Construct a minimal valid TIFF with JPEGInterchangeFormat tag
        let fake_jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9];
        let jpeg_offset = 200u32;
        let jpeg_length = fake_jpeg.len() as u32;

        let mut tiff_bytes = Vec::new();
        // Header
        tiff_bytes.extend_from_slice(b"II\x2a\x00"); // Little-endian, magic 42
        tiff_bytes.extend_from_slice(&8u32.to_le_bytes()); // First IFD at offset 8

        // IFD: 2 entries (JPEGInterchangeFormat and JPEGInterchangeFormatLength)
        tiff_bytes.extend_from_slice(&2u16.to_le_bytes());
        // Entry 1: 0x0201 (JPEGInterchangeFormat), Type 4 (LONG), Count 1, Value: jpeg_offset
        tiff_bytes.extend_from_slice(&0x0201u16.to_le_bytes());
        tiff_bytes.extend_from_slice(&4u16.to_le_bytes());
        tiff_bytes.extend_from_slice(&1u32.to_le_bytes());
        tiff_bytes.extend_from_slice(&jpeg_offset.to_le_bytes());

        // Entry 2: 0x0202 (JPEGInterchangeFormatLength), Type 4 (LONG), Count 1, Value: jpeg_length
        tiff_bytes.extend_from_slice(&0x0202u16.to_le_bytes());
        tiff_bytes.extend_from_slice(&4u16.to_le_bytes());
        tiff_bytes.extend_from_slice(&1u32.to_le_bytes());
        tiff_bytes.extend_from_slice(&jpeg_length.to_le_bytes());

        // Next IFD = 0
        tiff_bytes.extend_from_slice(&0u32.to_le_bytes());

        // Pad to jpeg_offset
        while tiff_bytes.len() < jpeg_offset as usize {
            tiff_bytes.push(0);
        }
        tiff_bytes.extend_from_slice(&fake_jpeg);

        let mut f = File::create(&test_raw).unwrap();
        f.write_all(&tiff_bytes).unwrap();
        drop(f);

        // Run extraction
        let res = extract_via_tiff_scan(&test_raw, &out_jpg);
        assert!(res.is_ok());
        let extracted_bytes = fs::read(&out_jpg).unwrap();
        assert_eq!(extracted_bytes, fake_jpeg);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_real_raw_samples() {
        let test_dir = Path::new("/home/sr/Downloads/RawTests");
        if !test_dir.exists() {
            return;
        }

        for file in ["sony_sample.ARW", "nikon_sample.NEF", "canon_sample.CR3", "fuji_sample.RAF"] {
            let path = test_dir.join(file);
            if path.exists() {
                let preview = get_or_extract_raw_preview(&path);
                assert!(preview.is_ok(), "Failed for {}: {:?}", file, preview.err());
                let p = preview.unwrap();
                assert!(p.exists());
                assert!(fs::metadata(&p).unwrap().len() > 0);
            }
        }
    }
}
