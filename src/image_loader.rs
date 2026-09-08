use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use image::{DynamicImage, GenericImageView, ImageReader, RgbaImage};
use crate::util::format_file_size;

pub struct DecodedImage {
    pub path: PathBuf,
    pub image: Arc<DynamicImage>,
    pub rgba: Arc<RgbaImage>,
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

pub fn rgba_to_cairo_surface(rgba: &RgbaImage) -> Result<cairo::ImageSurface, String> {
    let width = rgba.width() as i32;
    let height = rgba.height() as i32;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .map_err(|e| format!("{:?}", e))?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().map_err(|e| format!("{:?}", e))?;
        let raw_pixels = rgba.as_raw();
        for y in 0..height as usize {
            let row_start = y * stride;
            let img_row_start = y * (width as usize) * 4;
            for x in 0..width as usize {
                let px = img_row_start + x * 4;
                let r = raw_pixels[px] as u32;
                let g = raw_pixels[px + 1] as u32;
                let b = raw_pixels[px + 2] as u32;
                let a = raw_pixels[px + 3] as u32;

                let premul = if a == 255 {
                    (255 << 24) | (r << 16) | (g << 8) | b
                } else if a == 0 {
                    0
                } else {
                    let pr = (r * a + 127) / 255;
                    let pg = (g * a + 127) / 255;
                    let pb = (b * a + 127) / 255;
                    (a << 24) | (pr << 16) | (pg << 8) | pb
                };

                let dest_idx = row_start + x * 4;
                data[dest_idx..dest_idx + 4].copy_from_slice(&premul.to_ne_bytes());
            }
        }
    }
    surface.mark_dirty();
    Ok(surface)
}

pub fn load_dynamic_image(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| format!("Cannot open {}: {}", path.display(), e))?
        .with_guessed_format()
        .map_err(|e| format!("Cannot guess format for {}: {}", path.display(), e))?;

    reader
        .decode()
        .map_err(|e| format!("Cannot decode {}: {}", path.display(), e))
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

    Ok(DecodedImage {
        path: path.to_path_buf(),
        image: Arc::new(img),
        rgba: Arc::new(rgba),
        width,
        height,
        file_size_formatted: format_file_size(file_size),
    })
}

struct CacheState {
    images: HashMap<PathBuf, Arc<DecodedImage>>,
    image_order: VecDeque<PathBuf>,
    max_images: usize,

    thumbnails: HashMap<PathBuf, Arc<DecodedThumbnail>>,
    thumbnail_order: VecDeque<PathBuf>,
    max_thumbnails: usize,
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
                thumbnails: HashMap::new(),
                thumbnail_order: VecDeque::new(),
                max_thumbnails,
            })),
        }
    }

    pub fn get_image(&self, path: &Path) -> Option<Arc<DecodedImage>> {
        let state = self.state.lock().ok()?;
        state.images.get(path).cloned()
    }

    pub fn put_image(&self, path: PathBuf, img: Arc<DecodedImage>) {
        if let Ok(mut state) = self.state.lock() {
            if state.images.contains_key(&path) {
                state.images.insert(path, img);
                return;
            }
            while state.image_order.len() >= state.max_images {
                if let Some(oldest) = state.image_order.pop_front() {
                    state.images.remove(&oldest);
                }
            }
            state.image_order.push_back(path.clone());
            state.images.insert(path, img);
        }
    }

    pub fn get_thumbnail(&self, path: &Path) -> Option<Arc<DecodedThumbnail>> {
        let state = self.state.lock().ok()?;
        state.thumbnails.get(path).cloned()
    }

    pub fn put_thumbnail(&self, path: PathBuf, thumb: Arc<DecodedThumbnail>) {
        if let Ok(mut state) = self.state.lock() {
            if state.thumbnails.contains_key(&path) {
                state.thumbnails.insert(path, thumb);
                return;
            }
            while state.thumbnail_order.len() >= state.max_thumbnails {
                if let Some(oldest) = state.thumbnail_order.pop_front() {
                    state.thumbnails.remove(&oldest);
                }
            }
            state.thumbnail_order.push_back(path.clone());
            state.thumbnails.insert(path, thumb);
        }
    }

    pub fn remove(&self, path: &Path) {
        if let Ok(mut state) = self.state.lock() {
            state.images.remove(path);
            state.image_order.retain(|p| p != path);
            state.thumbnails.remove(path);
            state.thumbnail_order.retain(|p| p != path);
        }
    }
}
