pub type Callback<F> = std::rc::Rc<std::cell::RefCell<Option<Box<F>>>>;

use crate::raw_loader::{RAW_EXTENSIONS, is_raw_image};
use gio::prelude::FileExt;
use std::path::{Path, PathBuf};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "gif", "bmp", "tiff", "tif", "ico",
];

pub fn is_supported_image(path: &Path) -> bool {
    if let Some(ext) = path.extension()
        && let Some(ext_str) = ext.to_str()
    {
        let lower = ext_str.to_ascii_lowercase();
        return SUPPORTED_EXTENSIONS.contains(&lower.as_str())
            || RAW_EXTENSIONS.contains(&lower.as_str());
    }
    false
}

/// Natural key for alphanumeric sorting (e.g. "image_2.jpg" before "image_10.jpg")
fn natural_sort_key(s: &str) -> Vec<SortChunk> {
    let mut chunks = Vec::new();
    let mut current_digits = String::new();
    let mut current_chars = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            if !current_chars.is_empty() {
                chunks.push(SortChunk::Text(current_chars.to_lowercase()));
                current_chars.clear();
            }
            current_digits.push(c);
        } else {
            if !current_digits.is_empty() {
                if let Ok(num) = current_digits.parse::<u64>() {
                    chunks.push(SortChunk::Number(num));
                }
                current_digits.clear();
            }
            current_chars.push(c);
        }
    }

    if !current_chars.is_empty() {
        chunks.push(SortChunk::Text(current_chars.to_lowercase()));
    }
    if !current_digits.is_empty()
        && let Ok(num) = current_digits.parse::<u64>()
    {
        chunks.push(SortChunk::Number(num));
    }

    chunks
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SortChunk {
    Text(String),
    Number(u64),
}

/// Scans a directory and returns sorted paths of supported images.
/// If both a RAW file and its companion JPEG exist (e.g. photo.ARW and photo.JPG),
/// the RAW file is kept and the duplicate JPEG card is hidden, using the JPEG as companion.
pub fn scan_directory_images(dir: &Path) -> Vec<PathBuf> {
    let mut raw_stems = std::collections::HashSet::new();
    let mut all_files = Vec::new();

    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_file() && is_supported_image(&path) {
                if is_raw_image(&path)
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    raw_stems.insert(stem.to_ascii_lowercase());
                }
                all_files.push(path);
            }
        }
    }

    let mut entries = Vec::new();
    for path in all_files {
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && raw_stems.contains(&stem.to_ascii_lowercase())
        {
            continue;
        }
        entries.push(path);
    }

    entries.sort_by_cached_key(|p| {
        natural_sort_key(&p.file_name().unwrap_or_default().to_string_lossy())
    });

    entries
}

/// Discovers image set and initial index given CLI inputs.
pub fn discover_images(args: &[PathBuf]) -> (Vec<PathBuf>, usize) {
    if args.is_empty() {
        if let Ok(cwd) = std::env::current_dir() {
            let images = scan_directory_images(&cwd);
            return (images, 0);
        }
        return (Vec::new(), 0);
    }

    if args.len() == 1 {
        let p = &args[0];
        if p.is_dir() {
            let images = scan_directory_images(p);
            return (images, 0);
        } else if p.is_file() {
            if let Some(parent) = p.parent() {
                let images = scan_directory_images(parent);
                let canonical_target = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                for (idx, img_path) in images.iter().enumerate() {
                    let c = std::fs::canonicalize(img_path).unwrap_or_else(|_| img_path.clone());
                    if c == canonical_target {
                        return (images, idx);
                    }
                }
                return (vec![p.clone()], 0);
            } else {
                return (vec![p.clone()], 0);
            }
        }
    }

    // Multiple paths passed explicitly
    let mut images = Vec::new();
    for path in args {
        if path.is_file() && is_supported_image(path) {
            images.push(path.clone());
        } else if path.is_dir() {
            let mut sub = scan_directory_images(path);
            images.append(&mut sub);
        }
    }

    (images, 0)
}

/// Moves a file to trash using gio::File::trash.
pub fn trash_file(path: &Path) -> Result<(), String> {
    let file = gio::File::for_path(path);
    file.trash(gio::Cancellable::NONE)
        .map_err(|e| format!("Failed to move {} to trash: {}", path.display(), e))
}

pub fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_image() {
        assert!(is_supported_image(Path::new("photo.PNG")));
        assert!(is_supported_image(Path::new("photo.jpg")));
        assert!(is_supported_image(Path::new("photo.jpeg")));
        assert!(is_supported_image(Path::new("photo.webp")));
        assert!(is_supported_image(Path::new("photo.gif")));
        assert!(is_supported_image(Path::new("photo.bmp")));
        assert!(is_supported_image(Path::new("photo.tiff")));
        assert!(is_supported_image(Path::new("photo.ico")));
        assert!(!is_supported_image(Path::new("notes.txt")));
        assert!(!is_supported_image(Path::new("video.mp4")));
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(2048), "2 KB");
        assert_eq!(format_file_size(1024 * 1024 * 3), "3.0 MB");
        assert_eq!(format_file_size(1024 * 1024 * 1024 * 2), "2.0 GB");
    }

    #[test]
    fn test_natural_sort() {
        let mut names = vec!["img10.png", "img1.png", "img2.png", "img20.png", "img3.png"];
        names.sort_by_key(|a| natural_sort_key(a));
        assert_eq!(
            names,
            vec!["img1.png", "img2.png", "img3.png", "img10.png", "img20.png"]
        );
    }

    #[test]
    fn test_scan_directory_images_with_companion_pairing() {
        let temp_dir =
            std::env::temp_dir().join(format!("omaview_test_pairing_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // Create pair: DSC001.ARW and DSC001.JPG
        let _ = std::fs::write(temp_dir.join("DSC001.ARW"), b"rawdata");
        let _ = std::fs::write(temp_dir.join("DSC001.JPG"), b"jpgdata");
        let _ = std::fs::write(temp_dir.join("DSC001.png"), b"pngdata");

        // Create standalone raw: DSC002.NEF
        let _ = std::fs::write(temp_dir.join("DSC002.NEF"), b"rawdata2");

        // Create standalone png: photo.png
        let _ = std::fs::write(temp_dir.join("photo.png"), b"pngdata");

        let scanned = scan_directory_images(&temp_dir);
        let filenames: Vec<String> = scanned
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        // DSC001.ARW should be included, companion DSC001.JPG must be omitted
        assert!(filenames.contains(&"DSC001.ARW".to_string()));
        assert!(!filenames.contains(&"DSC001.JPG".to_string()));
        assert!(filenames.contains(&"DSC002.NEF".to_string()));
        assert!(filenames.contains(&"photo.png".to_string()));
        assert!(filenames.contains(&"DSC001.png".to_string()));
        assert_eq!(scanned.len(), 4);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
