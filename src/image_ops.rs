use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct ImageEdits {
    pub rotation: u16, // 0, 90, 180, 270
    pub flip_h: bool,
    pub flip_v: bool,
    pub crop_rect: Option<[f64; 4]>, // [x, y, w, h] in 0.0..1.0
    pub exposure: f64,               // -100.0 .. 100.0
    pub contrast: f64,               // -100.0 .. 100.0
    pub saturation: f64,             // -100.0 .. 100.0
    pub warmth: f64,                 // -100.0 .. 100.0
}

impl Default for ImageEdits {
    fn default() -> Self {
        Self {
            rotation: 0,
            flip_h: false,
            flip_v: false,
            crop_rect: None,
            exposure: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            warmth: 0.0,
        }
    }
}

#[allow(dead_code)]
impl ImageEdits {
    pub fn has_adjustments(&self) -> bool {
        self.exposure.abs() > 0.001
            || self.contrast.abs() > 0.001
            || self.saturation.abs() > 0.001
            || self.warmth.abs() > 0.001
    }

    pub fn has_geometry_edits(&self) -> bool {
        self.rotation != 0 || self.flip_h || self.flip_v || self.crop_rect.is_some()
    }

    pub fn has_any_edits(&self) -> bool {
        self.has_adjustments() || self.has_geometry_edits()
    }

    pub fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
    }

    pub fn rotate_ccw(&mut self) {
        self.rotation = (self.rotation + 270) % 360;
    }

    pub fn toggle_flip_h(&mut self) {
        self.flip_h = !self.flip_h;
    }

    pub fn toggle_flip_v(&mut self) {
        self.flip_v = !self.flip_v;
    }

    pub fn reset_adjustments(&mut self) {
        self.exposure = 0.0;
        self.contrast = 0.0;
        self.saturation = 0.0;
        self.warmth = 0.0;
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }

    /// Map a selection in the displayed (rotated/flipped) image back to the
    /// original crop coordinates, composing repeated crops without baking edits.
    pub fn add_crop(&mut self, rect: [f64; 4]) {
        let [mut x, mut y, mut w, mut h] = rect;
        if self.flip_h {
            x = 1.0 - x - w;
        }
        if self.flip_v {
            y = 1.0 - y - h;
        }
        [x, y, w, h] = match self.rotation {
            90 => [y, 1.0 - x - w, h, w],
            180 => [1.0 - x - w, 1.0 - y - h, w, h],
            270 => [1.0 - y - h, x, h, w],
            _ => [x, y, w, h],
        };
        let [ox, oy, ow, oh] = self.crop_rect.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        self.crop_rect = Some([ox + x * ow, oy + y * oh, w * ow, h * oh]);
    }
}

/// Applies color adjustments (exposure, contrast, saturation, warmth) to an RGBA image buffer.
pub fn apply_adjustments(
    rgba: &RgbaImage,
    exposure: f64,
    contrast: f64,
    saturation: f64,
    warmth: f64,
) -> RgbaImage {
    let (width, height) = rgba.dimensions();
    let mut out = RgbaImage::new(width, height);

    let exp_mult = 2.0f64.powf(exposure / 50.0);
    let contrast_factor = (100.0 + contrast) / (100.0 - contrast.min(99.0));
    let sat_factor = 1.0 + saturation / 100.0;
    let warmth_shift = warmth * 0.35;

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        if a == 0 {
            out.put_pixel(x, y, *pixel);
            continue;
        }

        // 1. Exposure
        let mut rf = (r as f64) * exp_mult;
        let mut gf = (g as f64) * exp_mult;
        let mut bf = (b as f64) * exp_mult;

        // 2. Contrast around midpoint 128.0
        rf = (rf - 128.0) * contrast_factor + 128.0;
        gf = (gf - 128.0) * contrast_factor + 128.0;
        bf = (bf - 128.0) * contrast_factor + 128.0;

        // 3. Warmth
        rf += warmth_shift;
        bf -= warmth_shift;

        // 4. Saturation
        let luma = 0.299 * rf + 0.587 * gf + 0.114 * bf;
        rf = luma + (rf - luma) * sat_factor;
        gf = luma + (gf - luma) * sat_factor;
        bf = luma + (bf - luma) * sat_factor;

        let ro = rf.round().clamp(0.0, 255.0) as u8;
        let go = gf.round().clamp(0.0, 255.0) as u8;
        let bo = bf.round().clamp(0.0, 255.0) as u8;

        out.put_pixel(x, y, Rgba([ro, go, bo, a]));
    }

    out
}

/// Applies all edits (crop, rotation, flip, adjustments) to produce a new DynamicImage.
pub fn apply_all_edits(base: &DynamicImage, edits: &ImageEdits) -> DynamicImage {
    let mut current = base.clone();

    // 1. Crop
    if let Some([cx, cy, cw, ch]) = edits.crop_rect {
        let (w, h) = current.dimensions();
        let px = (cx * (w as f64))
            .round()
            .clamp(0.0, w.saturating_sub(1) as f64) as u32;
        let py = (cy * (h as f64))
            .round()
            .clamp(0.0, h.saturating_sub(1) as f64) as u32;
        let pw = (cw * (w as f64)).round().clamp(1.0, (w - px) as f64) as u32;
        let ph = (ch * (h as f64)).round().clamp(1.0, (h - py) as f64) as u32;
        current = current.crop_imm(px, py, pw, ph);
    }

    // 2. Rotate
    match edits.rotation {
        90 => current = current.rotate90(),
        180 => current = current.rotate180(),
        270 => current = current.rotate270(),
        _ => {}
    }

    // 3. Flip
    if edits.flip_h {
        current = current.fliph();
    }
    if edits.flip_v {
        current = current.flipv();
    }

    // 4. Adjustments
    if edits.has_adjustments() {
        let rgba = current.to_rgba8();
        let adjusted = apply_adjustments(
            &rgba,
            edits.exposure,
            edits.contrast,
            edits.saturation,
            edits.warmth,
        );
        current = DynamicImage::ImageRgba8(adjusted);
    }

    current
}

pub fn save_image(img: &DynamicImage, path: &Path) -> Result<(), String> {
    use std::fs::{self, OpenOptions};
    use std::io::{BufWriter, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);

    // Never silently write PNG bytes over a RAW file or an unknown extension.
    let format = ImageFormat::from_path(path).map_err(|e| e.to_string())?;
    let target = if path.exists() {
        path.canonicalize().map_err(|e| e.to_string())?
    } else {
        path.to_path_buf()
    };
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(
        ".omaview-save-{}-{}",
        std::process::id(),
        NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("Cannot create save file: {e}"))?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        if format == ImageFormat::Jpeg {
            // JPEG does not support RGBA; edits produce RGBA even for source JPEGs.
            DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut writer, format)
        } else {
            img.write_to(&mut writer, format)
        }
        .map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        writer.get_ref().sync_all().map_err(|e| e.to_string())?;
        if let Ok(metadata) = fs::metadata(&target) {
            fs::set_permissions(&temp, metadata.permissions()).map_err(|e| e.to_string())?;
        }
        fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("Failed to save image to {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn crop_matches_display_after_geometry_and_repeated_crops() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_fn(16, 16, |x, y| {
            Rgba([x as u8, y as u8, 0, 255])
        }));
        for rotation in [0, 90, 180, 270] {
            for flip_h in [false, true] {
                for flip_v in [false, true] {
                    let mut edits = ImageEdits {
                        rotation,
                        flip_h,
                        flip_v,
                        crop_rect: Some([0.25, 0.25, 0.5, 0.5]),
                        ..Default::default()
                    };
                    let expected = apply_all_edits(&base, &edits).crop_imm(2, 2, 4, 4);
                    edits.add_crop([0.25, 0.25, 0.5, 0.5]);
                    assert_eq!(
                        apply_all_edits(&base, &edits).to_rgba8(),
                        expected.to_rgba8()
                    );
                    let expected = expected.crop_imm(1, 1, 2, 2);
                    edits.add_crop([0.25, 0.25, 0.5, 0.5]);
                    assert_eq!(
                        apply_all_edits(&base, &edits).to_rgba8(),
                        expected.to_rgba8()
                    );
                }
            }
        }
    }

    #[test]
    fn crop_matches_asymmetric_rotated_selection() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_fn(16, 8, |x, y| {
            Rgba([x as u8, y as u8, 0, 255])
        }));
        for rotation in [0, 90, 180, 270] {
            for flip_h in [false, true] {
                let mut edits = ImageEdits {
                    rotation,
                    flip_h,
                    ..Default::default()
                };
                let displayed = apply_all_edits(&base, &edits);
                let expected = displayed.crop_imm(
                    0,
                    displayed.height() / 4,
                    displayed.width() / 4,
                    displayed.height() / 2,
                );
                edits.add_crop([0.0, 0.25, 0.25, 0.5]);
                assert_eq!(
                    apply_all_edits(&base, &edits).to_rgba8(),
                    expected.to_rgba8()
                );
            }
        }
    }

    #[test]
    fn jpeg_adjustments_save_and_failed_encoding_preserves_original() {
        let dir = std::env::temp_dir().join(format!("omaview-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photo.jpg");
        let base = DynamicImage::new_rgb8(8, 8);
        let edits = ImageEdits {
            exposure: 25.0,
            ..Default::default()
        };
        save_image(&apply_all_edits(&base, &edits), &path).unwrap();
        assert_eq!(image::open(&path).unwrap().dimensions(), (8, 8));
        let unsupported = dir.join("photo.arw");
        std::fs::write(&unsupported, b"original RAW").unwrap();
        assert!(save_image(&base, &unsupported).is_err());
        assert_eq!(std::fs::read(&unsupported).unwrap(), b"original RAW");
        let failing = dir.join("photo.hdr");
        std::fs::write(&failing, b"original bytes").unwrap();
        assert!(save_image(&base, &failing).is_err());
        assert_eq!(std::fs::read(&failing).unwrap(), b"original bytes");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_rotation_cycle() {
        let mut edits = ImageEdits::default();
        assert_eq!(edits.rotation, 0);

        edits.rotate_cw();
        assert_eq!(edits.rotation, 90);

        edits.rotate_cw();
        assert_eq!(edits.rotation, 180);

        edits.rotate_cw();
        assert_eq!(edits.rotation, 270);

        edits.rotate_cw();
        assert_eq!(edits.rotation, 0);

        edits.rotate_ccw();
        assert_eq!(edits.rotation, 270);

        edits.rotate_ccw();
        assert_eq!(edits.rotation, 180);
    }

    #[test]
    fn test_flip_toggles() {
        let mut edits = ImageEdits::default();
        assert!(!edits.flip_h);
        assert!(!edits.flip_v);

        edits.toggle_flip_h();
        assert!(edits.flip_h);

        edits.toggle_flip_v();
        assert!(edits.flip_v);

        edits.toggle_flip_h();
        assert!(!edits.flip_h);
    }

    #[test]
    fn test_apply_crop_and_rotate() {
        // Create a 100x50 test image
        let mut img = RgbaImage::new(100, 50);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = Rgba([x as u8, y as u8, 128, 255]);
        }
        let dynamic = DynamicImage::ImageRgba8(img);

        let edits = ImageEdits {
            crop_rect: Some([0.0, 0.0, 0.5, 0.5]),
            rotation: 90,
            ..Default::default()
        };

        let result = apply_all_edits(&dynamic, &edits);
        // Original 100x50 cropped to 50x25, then rotated 90 degrees becomes 25x50
        let (w, h) = result.dimensions();
        assert_eq!(w, 25);
        assert_eq!(h, 50);
    }

    #[test]
    fn test_apply_adjustments() {
        let mut img = RgbaImage::new(10, 10);
        for pixel in img.pixels_mut() {
            *pixel = Rgba([128, 128, 128, 255]);
        }

        // Increase exposure
        let adjusted = apply_adjustments(&img, 50.0, 0.0, 0.0, 0.0);
        let px = adjusted.get_pixel(0, 0);
        // 2^(50/50) = 2.0x -> 128 * 2 = 255 (clamped)
        assert!(px[0] > 200);
        assert!(px[1] > 200);
        assert!(px[2] > 200);
        assert_eq!(px[3], 255);
    }
}
