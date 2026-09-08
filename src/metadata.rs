use std::fs::File;
use std::path::Path;
use gtk4::prelude::*;
use gtk4::{Box, Label, Orientation, Popover, Grid};

#[derive(Debug, Clone, Default)]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<String>,
    pub aperture: Option<String>,
    pub exposure_time: Option<String>,
    pub iso: Option<String>,
    pub datetime: Option<String>,
}

pub fn extract_exif(path: &Path) -> Option<ExifInfo> {
    let file = File::open(path).ok()?;
    let mut bufreader = std::io::BufReader::new(file);
    let exifreader = exif::Reader::new();
    let exif = exifreader.read_from_container(&mut bufreader).ok()?;

    let mut info = ExifInfo::default();

    if let Some(field) = exif.get_field(exif::Tag::Make, exif::In::PRIMARY) {
        info.make = Some(field.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::Model, exif::In::PRIMARY) {
        info.model = Some(field.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::LensModel, exif::In::PRIMARY) {
        info.lens = Some(field.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::FocalLength, exif::In::PRIMARY) {
        info.focal_length = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::FNumber, exif::In::PRIMARY) {
        info.aperture = Some(format!("f/{}", field.display_value()));
    }
    if let Some(field) = exif.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
        info.exposure_time = Some(format!("{}s", field.display_value()));
    }
    if let Some(field) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
        info.iso = Some(format!("ISO {}", field.display_value()));
    }
    if let Some(field) = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
        info.datetime = Some(field.display_value().to_string());
    }

    Some(info)
}

#[derive(Clone)]
pub struct MetadataPill {
    container: Box,
    label: Label,
}

impl MetadataPill {
    pub fn new() -> Self {
        let container = Box::new(Orientation::Horizontal, 0);
        container.add_css_class("metadata-pill");
        container.set_halign(gtk4::Align::Start);
        container.set_valign(gtk4::Align::Center);

        let label = Label::new(None);
        label.set_use_markup(true);
        container.append(&label);

        Self { container, label }
    }

    pub fn widget(&self) -> &Box {
        &self.container
    }

    pub fn update(
        &self,
        filename: &str,
        width: u32,
        height: u32,
        zoom_pct: u32,
        current_idx: usize,
        total: usize,
    ) {
        let text = if total > 0 {
            format!(
                "{} | {}x{} | {}% | {}/{}",
                glib::markup_escape_text(filename),
                width,
                height,
                zoom_pct,
                current_idx + 1,
                total
            )
        } else {
            "No image open".to_string()
        };
        self.label.set_text(&text);
    }
}

pub fn create_exif_popover(
    path: &Path,
    width: u32,
    height: u32,
    file_size_str: &str,
) -> Popover {
    let popover = Popover::new();
    popover.add_css_class("exif-popover");

    let vbox = Box::new(Orientation::Vertical, 10);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let title = Label::new(None);
    title.set_markup("<span weight='bold' size='large'>Image Details</span>");
    title.set_halign(gtk4::Align::Start);
    vbox.append(&title);

    let grid = Grid::new();
    grid.set_column_spacing(16);
    grid.set_row_spacing(6);

    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("Unknown");
    let mut row = 0;

    let add_row = |g: &Grid, r: &mut i32, key: &str, val: &str| {
        let key_lbl = Label::new(Some(key));
        key_lbl.set_halign(gtk4::Align::Start);
        key_lbl.add_css_class("dim-label");
        let val_lbl = Label::new(Some(val));
        val_lbl.set_halign(gtk4::Align::Start);
        val_lbl.set_selectable(true);

        g.attach(&key_lbl, 0, *r, 1, 1);
        g.attach(&val_lbl, 1, *r, 1, 1);
        *r += 1;
    };

    add_row(&grid, &mut row, "File Name", filename);
    add_row(&grid, &mut row, "Dimensions", &format!("{} × {} px", width, height));
    add_row(&grid, &mut row, "File Size", file_size_str);

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        add_row(&grid, &mut row, "Format", &ext.to_ascii_uppercase());
    }

    if let Some(exif) = extract_exif(path) {
        if let Some(camera) = exif.make.or(exif.model) {
            add_row(&grid, &mut row, "Camera", &camera);
        }
        if let Some(lens) = exif.lens {
            add_row(&grid, &mut row, "Lens", &lens);
        }
        if let Some(fl) = exif.focal_length {
            add_row(&grid, &mut row, "Focal Length", &fl);
        }
        if let Some(ap) = exif.aperture {
            add_row(&grid, &mut row, "Aperture", &ap);
        }
        if let Some(shutter) = exif.exposure_time {
            add_row(&grid, &mut row, "Shutter", &shutter);
        }
        if let Some(iso) = exif.iso {
            add_row(&grid, &mut row, "ISO", &iso);
        }
        if let Some(dt) = exif.datetime {
            add_row(&grid, &mut row, "Captured", &dt);
        }
    }

    vbox.append(&grid);
    popover.set_child(Some(&vbox));
    popover
}
