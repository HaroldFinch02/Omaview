use crate::util::format_file_size;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, RgbaImage};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

/// A fixed number of workers prevents rapid navigation/redraws from flooding the
/// machine with decoders. Thumbnail work cannot delay full-size images or edits.
type QueuedJob = (Arc<AtomicBool>, Job);
struct WorkerPool {
    queue: Arc<(Mutex<VecDeque<QueuedJob>>, Condvar)>,
}

impl WorkerPool {
    fn new() -> Self {
        let queue = Arc::new((Mutex::new(VecDeque::<QueuedJob>::new()), Condvar::new()));
        for _ in 0..2 {
            let queue = queue.clone();
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let (lock, ready) = &*queue;
                        let mut jobs = lock.lock().unwrap();
                        while jobs.is_empty() {
                            jobs = ready.wait(jobs).unwrap();
                        }
                        let idx = jobs
                            .iter()
                            .position(|(priority, _)| priority.load(Ordering::Relaxed))
                            .unwrap_or(0);
                        jobs.remove(idx).unwrap().1
                    };
                    job();
                }
            });
        }
        Self { queue }
    }

    fn submit(&self, priority: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
        self.queue
            .0
            .lock()
            .unwrap()
            .push_back((priority, Box::new(job)));
        self.queue.1.notify_one();
    }
}

pub fn run_background(job: impl FnOnce() + Send + 'static) {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(WorkerPool::new)
        .submit(Arc::new(AtomicBool::new(true)), job);
}

fn run_image_job(priority: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(WorkerPool::new).submit(priority, job);
}

fn run_thumbnail_job(priority: Arc<AtomicBool>, job: impl FnOnce() + Send + 'static) {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(WorkerPool::new).submit(priority, job);
}

type LoadCallback<T> = Box<dyn FnOnce(Result<Arc<T>, String>) + Send>;
struct PendingLoad<T> {
    token: Arc<()>,
    priority: Arc<AtomicBool>,
    callbacks: Vec<LoadCallback<T>>,
}

pub struct DecodedImage {
    pub path: PathBuf,
    pub image: Arc<DynamicImage>,
    pub rgba: Arc<RgbaImage>,
    pub cairo_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub file_size_formatted: String,
}

#[allow(dead_code)]
pub struct DecodedThumbnail {
    pub rgba: Arc<RgbaImage>,
    pub width: u32,
    pub height: u32,
}

/// Converts RGBA pixels to Cairo ARgb32 format (premultiplied BGRA in native memory order).
/// Fast-paths opaque images (most JPEGs/RAWs) with zero per-pixel branching.
pub fn rgba_to_argb32_bytes(rgba: &RgbaImage) -> Vec<u8> {
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;
    let stride = width * 4;
    let mut data = vec![0u8; stride * height];
    let raw = rgba.as_raw();

    let is_opaque = raw.as_chunks::<4>().0.iter().all(|px| px[3] == 255);

    if is_opaque {
        // Direct conversion from RGBA to native little-endian ARgb32 [B, G, R, 255]
        for (src, dst) in raw
            .as_chunks::<4>()
            .0
            .iter()
            .zip(data.as_chunks_mut::<4>().0.iter_mut())
        {
            dst[0] = src[2]; // B
            dst[1] = src[1]; // G
            dst[2] = src[0]; // R
            dst[3] = 255; // A
        }
    } else {
        for (src, dst) in raw
            .as_chunks::<4>()
            .0
            .iter()
            .zip(data.as_chunks_mut::<4>().0.iter_mut())
        {
            let r = src[0] as u32;
            let g = src[1] as u32;
            let b = src[2] as u32;
            let a = src[3] as u32;

            if a == 0 {
                dst[0] = 0;
                dst[1] = 0;
                dst[2] = 0;
                dst[3] = 0;
            } else if a == 255 {
                dst[0] = b as u8;
                dst[1] = g as u8;
                dst[2] = r as u8;
                dst[3] = 255;
            } else {
                let pr = ((r * a + 127) / 255) as u8;
                let pg = ((g * a + 127) / 255) as u8;
                let pb = ((b * a + 127) / 255) as u8;
                dst[0] = pb;
                dst[1] = pg;
                dst[2] = pr;
                dst[3] = a as u8;
            }
        }
    }
    data
}

/// Creates a Cairo ImageSurface from precomputed ARgb32 pixel data in ~0.8ms via SIMD memcpy.
pub fn cairo_data_to_surface(
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<cairo::ImageSurface, String> {
    cairo_owned_data_to_surface(data.to_vec(), width, height)
}

pub fn cairo_owned_data_to_surface(
    data: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<cairo::ImageSurface, String> {
    let stride = width
        .checked_mul(4)
        .filter(|v| *v <= i32::MAX as u32)
        .ok_or("Image width exceeds Cairo limits")?;
    let expected = (stride as usize)
        .checked_mul(height as usize)
        .ok_or("Image size overflow")?;
    if height > i32::MAX as u32 || expected > i32::MAX as usize || data.len() != expected {
        return Err("Invalid Cairo image buffer size".into());
    }
    cairo::ImageSurface::create_for_data(
        data,
        cairo::Format::ARgb32,
        width as i32,
        height as i32,
        stride as i32,
    )
    .map_err(|e| e.to_string())
}

pub fn rgba_to_cairo_surface(rgba: &RgbaImage) -> Result<cairo::ImageSurface, String> {
    let data = rgba_to_argb32_bytes(rgba);
    cairo_owned_data_to_surface(data, rgba.width(), rgba.height())
}

pub fn load_dynamic_image(path: &Path) -> Result<DynamicImage, String> {
    if crate::raw_loader::is_raw_image(path) {
        return crate::raw_loader::load_raw_as_dynamic_image(path);
    }

    let reader = ImageReader::open(path)
        .map_err(|e| format!("Cannot open {}: {}", path.display(), e))?
        .with_guessed_format()
        .map_err(|e| format!("Cannot guess format for {}: {}", path.display(), e))?;

    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("Cannot decode {}: {e}", path.display()))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Cannot decode {}: {e}", path.display()))?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub fn generate_thumbnail(img: &DynamicImage, max_dim: u32) -> DecodedThumbnail {
    let thumb = img.thumbnail(max_dim, max_dim);
    let rgba = thumb.to_rgba8();
    DecodedThumbnail {
        rgba: Arc::new(rgba),
        width: thumb.width(),
        height: thumb.height(),
    }
}

pub fn load_decoded_image(path: &Path) -> Result<DecodedImage, String> {
    let img = load_dynamic_image(path)?;
    let rgba = img.to_rgba8();
    let (width, height) = img.dimensions();
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let cairo_data = rgba_to_argb32_bytes(&rgba);

    Ok(DecodedImage {
        path: path.to_path_buf(),
        image: Arc::new(img),
        rgba: Arc::new(rgba),
        cairo_data: Arc::new(cairo_data),
        width,
        height,
        file_size_formatted: format_file_size(file_size),
    })
}

struct CacheState {
    images: HashMap<PathBuf, Arc<DecodedImage>>,
    image_order: VecDeque<PathBuf>,
    max_images: usize,
    image_bytes: usize,
    pending_images: HashMap<PathBuf, PendingLoad<DecodedImage>>,

    thumbnails: HashMap<PathBuf, Arc<DecodedThumbnail>>,
    thumbnail_order: VecDeque<PathBuf>,
    max_thumbnails: usize,
    pending_thumbnails: HashMap<PathBuf, PendingLoad<DecodedThumbnail>>,
}

#[derive(Clone)]
pub struct ImageCache {
    state: Arc<Mutex<CacheState>>,
}

impl ImageCache {
    pub fn new(max_images: usize, max_thumbnails: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(CacheState {
                images: HashMap::new(),
                image_order: VecDeque::new(),
                max_images,
                image_bytes: 0,
                pending_images: HashMap::new(),
                thumbnails: HashMap::new(),
                thumbnail_order: VecDeque::new(),
                max_thumbnails,
                pending_thumbnails: HashMap::new(),
            })),
        }
    }

    pub fn get_image(&self, path: &Path) -> Option<Arc<DecodedImage>> {
        let mut state = self.state.lock().ok()?;
        if let Some(img) = state.images.get(path).cloned() {
            // Move accessed image to end of LRU queue
            if let Some(pos) = state.image_order.iter().position(|p| p == path) {
                state.image_order.remove(pos);
                state.image_order.push_back(path.to_path_buf());
            }
            Some(img)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn put_image(&self, path: PathBuf, img: Arc<DecodedImage>) {
        if let Ok(mut state) = self.state.lock() {
            put_loaded_image(&mut state, path, img);
        }
    }

    pub fn get_thumbnail(&self, path: &Path) -> Option<Arc<DecodedThumbnail>> {
        let mut state = self.state.lock().ok()?;
        let thumb = state.thumbnails.get(path).cloned()?;
        state.thumbnail_order.retain(|p| p != path);
        state.thumbnail_order.push_back(path.to_path_buf());
        Some(thumb)
    }

    #[cfg(test)]
    pub fn put_thumbnail(&self, path: PathBuf, thumb: Arc<DecodedThumbnail>) {
        if let Ok(mut state) = self.state.lock() {
            put_loaded_thumbnail(&mut state, path, thumb);
        }
    }

    pub fn remove(&self, path: &Path) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(old) = state.images.remove(path) {
                state.image_bytes -= decoded_bytes(&old);
            }
            state.image_order.retain(|p| p != path);
            state.thumbnails.remove(path);
            state.thumbnail_order.retain(|p| p != path);
            // Discard pending completions from before an overwrite/trash.
            state.pending_images.remove(path);
            state.pending_thumbnails.remove(path);
        }
    }

    pub fn request_image(
        &self,
        path: PathBuf,
        callback: impl FnOnce(Result<Arc<DecodedImage>, String>) + Send + 'static,
    ) {
        self.request_image_with_priority(path, true, callback);
    }

    pub fn prefetch_image(&self, path: PathBuf) {
        self.request_image_with_priority(path, false, |_| {});
    }

    pub fn retain_image_requests(&self, paths: &[PathBuf]) {
        self.state
            .lock()
            .unwrap()
            .pending_images
            .retain(|path, _| paths.contains(path));
    }

    fn request_image_with_priority(
        &self,
        path: PathBuf,
        foreground: bool,
        callback: impl FnOnce(Result<Arc<DecodedImage>, String>) + Send + 'static,
    ) {
        if let Some(image) = self.get_image(&path) {
            callback(Ok(image));
            return;
        }
        let priority = Arc::new(AtomicBool::new(foreground));
        let token = {
            let mut state = self.state.lock().unwrap();
            if let Some(pending) = state.pending_images.get_mut(&path) {
                if foreground {
                    pending.priority.store(true, Ordering::Relaxed);
                }
                pending.callbacks.push(Box::new(callback));
                return;
            }
            let token = Arc::new(());
            state.pending_images.insert(
                path.clone(),
                PendingLoad {
                    token: token.clone(),
                    priority: priority.clone(),
                    callbacks: vec![Box::new(callback)],
                },
            );
            token
        };
        let cache = self.clone();
        run_image_job(priority, move || {
            if !cache
                .state
                .lock()
                .unwrap()
                .pending_images
                .get(&path)
                .is_some_and(|p| Arc::ptr_eq(&p.token, &token))
            {
                return;
            }
            let result = load_decoded_image(&path).map(Arc::new);
            let mut state = cache.state.lock().unwrap();
            if !state
                .pending_images
                .get(&path)
                .is_some_and(|p| Arc::ptr_eq(&p.token, &token))
            {
                return;
            }
            let pending = state.pending_images.remove(&path).unwrap();
            // Publish under the same lock as invalidation, preventing stale results.
            if let Ok(ref image) = result {
                // Use the standard insertion routine on the already locked state.
                put_loaded_image(&mut state, path, image.clone());
            }
            drop(state);
            for cb in pending.callbacks {
                cb(result.clone());
            }
        });
    }

    pub fn request_thumbnail(
        &self,
        path: PathBuf,
        callback: impl FnOnce(Result<Arc<DecodedThumbnail>, String>) + Send + 'static,
    ) {
        self.request_thumbnail_with_priority(path, true, callback);
    }

    pub fn prefetch_thumbnail(
        &self,
        path: PathBuf,
        callback: impl FnOnce(Result<Arc<DecodedThumbnail>, String>) + Send + 'static,
    ) {
        self.request_thumbnail_with_priority(path, false, callback);
    }

    fn request_thumbnail_with_priority(
        &self,
        path: PathBuf,
        foreground: bool,
        callback: impl FnOnce(Result<Arc<DecodedThumbnail>, String>) + Send + 'static,
    ) {
        if let Some(thumb) = self.get_thumbnail(&path) {
            callback(Ok(thumb));
            return;
        }
        let priority = Arc::new(AtomicBool::new(foreground));
        let token = {
            let mut state = self.state.lock().unwrap();
            if let Some(pending) = state.pending_thumbnails.get_mut(&path) {
                if foreground {
                    pending.priority.store(true, Ordering::Relaxed);
                }
                pending.callbacks.push(Box::new(callback));
                return;
            }
            let token = Arc::new(());
            state.pending_thumbnails.insert(
                path.clone(),
                PendingLoad {
                    token: token.clone(),
                    priority: priority.clone(),
                    callbacks: vec![Box::new(callback)],
                },
            );
            token
        };
        let cache = self.clone();
        run_thumbnail_job(priority, move || {
            let result = if let Some(image) = cache.get_image(&path) {
                Ok(Arc::new(generate_thumbnail(&image.image, 240)))
            } else {
                load_dynamic_image(&path).map(|image| Arc::new(generate_thumbnail(&image, 240)))
            };
            let mut state = cache.state.lock().unwrap();
            if !state
                .pending_thumbnails
                .get(&path)
                .is_some_and(|p| Arc::ptr_eq(&p.token, &token))
            {
                return;
            }
            let pending = state.pending_thumbnails.remove(&path).unwrap();
            if let Ok(ref thumb) = result {
                put_loaded_thumbnail(&mut state, path, thumb.clone());
            }
            drop(state);
            for cb in pending.callbacks {
                cb(result.clone());
            }
        });
    }
}

fn decoded_bytes(img: &DecodedImage) -> usize {
    img.image.as_bytes().len() + img.rgba.as_raw().len() + img.cairo_data.len()
}

fn put_loaded_image(state: &mut CacheState, path: PathBuf, img: Arc<DecodedImage>) {
    if state.max_images == 0 {
        return;
    }
    if let Some(old) = state.images.remove(&path) {
        state.image_bytes -= decoded_bytes(&old);
        state.image_order.retain(|p| p != &path);
    }
    let bytes = decoded_bytes(&img);
    // Keep one oversized image so the active image can still be delivered.
    while !state.image_order.is_empty()
        && (state.image_order.len() >= state.max_images
            || state.image_bytes.saturating_add(bytes) > 256 * 1024 * 1024)
    {
        if let Some(oldest) = state.image_order.pop_front()
            && let Some(old) = state.images.remove(&oldest)
        {
            state.image_bytes -= decoded_bytes(&old);
        }
    }
    state.image_bytes += bytes;
    state.image_order.push_back(path.clone());
    state.images.insert(path, img);
}

fn put_loaded_thumbnail(state: &mut CacheState, path: PathBuf, thumb: Arc<DecodedThumbnail>) {
    if state.max_thumbnails == 0 {
        return;
    }
    state.thumbnails.remove(&path);
    state.thumbnail_order.retain(|p| p != &path);
    while state.thumbnail_order.len() >= state.max_thumbnails {
        if let Some(oldest) = state.thumbnail_order.pop_front() {
            state.thumbnails.remove(&oldest);
        }
    }
    state.thumbnail_order.push_back(path.clone());
    state.thumbnails.insert(path, thumb);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_exif_orientation_is_applied_before_editing() {
        let path =
            std::env::temp_dir().join(format!("omaview-orientation-{}.jpg", std::process::id()));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        DynamicImage::new_rgb8(8, 4)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        // EXIF/TIFF IFD with Orientation = 6 (90 degrees clockwise).
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let original = jpeg.into_inner();
        let mut tagged = original[..2].to_vec();
        tagged.extend_from_slice(&[0xff, 0xe1]);
        tagged.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        tagged.extend_from_slice(exif);
        tagged.extend_from_slice(&original[2..]);
        std::fs::write(&path, tagged).unwrap();
        assert_eq!(load_dynamic_image(&path).unwrap().dimensions(), (4, 8));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn zero_capacity_and_thumbnail_lru() {
        let empty = ImageCache::new(0, 0);
        let thumb = Arc::new(generate_thumbnail(&DynamicImage::new_rgb8(2, 2), 2));
        let path = PathBuf::from("a.png");
        let image = Arc::new(DecodedImage {
            path: path.clone(),
            image: Arc::new(DynamicImage::new_rgb8(2, 2)),
            rgba: thumb.rgba.clone(),
            cairo_data: Arc::new(vec![0; 16]),
            width: 2,
            height: 2,
            file_size_formatted: "0 B".into(),
        });
        empty.put_image(path.clone(), image);
        empty.put_thumbnail(path.clone(), thumb.clone());
        assert!(empty.get_image(&path).is_none());
        assert!(empty.get_thumbnail(&path).is_none());
        let cache = ImageCache::new(0, 2);
        cache.put_thumbnail(path.clone(), thumb.clone());
        cache.put_thumbnail("b.png".into(), thumb.clone());
        cache.get_thumbnail(&path).unwrap();
        cache.put_thumbnail("c.png".into(), thumb);
        assert!(cache.get_thumbnail(Path::new("b.png")).is_none());
        assert!(cache.get_thumbnail(&path).is_some());
    }

    #[test]
    fn concurrent_requests_share_decode_and_invalidation_reloads() {
        let path =
            std::env::temp_dir().join(format!("omaview-decode-test-{}.png", std::process::id()));
        DynamicImage::new_rgb8(32, 24).save(&path).unwrap();
        let cache = ImageCache::new(2, 2);
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..10 {
            let tx = tx.clone();
            cache.request_image(path.clone(), move |result| {
                tx.send(result).unwrap();
            });
        }
        let images: Vec<_> = (0..10)
            .map(|_| {
                rx.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .unwrap()
            })
            .collect();
        assert!(images.iter().all(|i| Arc::ptr_eq(i, &images[0])));
        cache.remove(&path);
        DynamicImage::new_rgb8(12, 8).save(&path).unwrap();
        cache.request_image(path.clone(), move |result| {
            tx.send(result).unwrap();
        });
        let new = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!((new.width, new.height), (12, 8));
        assert!(!Arc::ptr_eq(&new, &images[0]));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_surface_buffer_is_rejected_and_alpha_is_premultiplied() {
        assert!(cairo_data_to_surface(&[0; 3], 1, 1).is_err());
        assert!(cairo_data_to_surface(&[], u32::MAX, 1).is_err());
        let rgba = RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 128]));
        assert_eq!(rgba_to_argb32_bytes(&rgba), [25, 50, 100, 128]);
    }

    #[test]
    fn test_cairo_fast_conversion() {
        let mut rgba = RgbaImage::new(100, 100);
        for pixel in rgba.pixels_mut() {
            *pixel = image::Rgba([10, 20, 30, 255]);
        }

        let argb = rgba_to_argb32_bytes(&rgba);
        assert_eq!(argb.len(), 100 * 100 * 4);
        // Little endian ARgb32 is [B, G, R, A]
        assert_eq!(argb[0], 30); // B
        assert_eq!(argb[1], 20); // G
        assert_eq!(argb[2], 10); // R
        assert_eq!(argb[3], 255); // A

        let surface =
            cairo_data_to_surface(&argb, 100, 100).expect("Surface creation must succeed");
        assert_eq!(surface.width(), 100);
        assert_eq!(surface.height(), 100);
    }

    #[test]
    fn test_lru_cache_reordering() {
        let cache = ImageCache::new(2, 5);
        let p1 = PathBuf::from("img1.png");
        let p2 = PathBuf::from("img2.png");
        let p3 = PathBuf::from("img3.png");

        let make_dummy = |p: &Path| {
            Arc::new(DecodedImage {
                path: p.to_path_buf(),
                image: Arc::new(DynamicImage::new_rgb8(1, 1)),
                rgba: Arc::new(RgbaImage::new(1, 1)),
                cairo_data: Arc::new(vec![0; 4]),
                width: 1,
                height: 1,
                file_size_formatted: "1 B".to_string(),
            })
        };

        cache.put_image(p1.clone(), make_dummy(&p1));
        cache.put_image(p2.clone(), make_dummy(&p2));

        // Access p1 so it becomes most recently used
        assert!(cache.get_image(&p1).is_some());

        // Now put p3. Since max_images is 2, p2 (least recently used) should be evicted, NOT p1!
        cache.put_image(p3.clone(), make_dummy(&p3));

        assert!(
            cache.get_image(&p1).is_some(),
            "p1 was recently accessed and must not be evicted"
        );
        assert!(
            cache.get_image(&p2).is_none(),
            "p2 was LRU and should be evicted"
        );
        assert!(cache.get_image(&p3).is_some(), "p3 was just inserted");
    }
}
