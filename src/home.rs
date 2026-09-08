use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerMotion, GestureClick,
    Image, Label, Orientation, PolicyType, ScrolledWindow, Window,
};

use crate::albums::{add_album, load_albums, remove_album};
use crate::image_loader::{load_dynamic_image, generate_thumbnail, rgba_to_cairo_surface, ImageCache};
use crate::theme::OmarchyColors;
use crate::util::scan_directory_images;

const CARD_WIDTH: f64 = 180.0;
const CARD_HEIGHT: f64 = 116.0;
const CARD_CORNER_RADIUS: f64 = 10.0;

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

thread_local! {
    static HOME_STATE_REF: RefCell<Option<std::rc::Weak<RefCell<HomeScreenState>>>> = const { RefCell::new(None) };
}

fn trigger_home_thumb_redraw(path: PathBuf) {
    glib::idle_add_once(move || {
        HOME_STATE_REF.with(|cell| {
            if let Some(weak) = cell.borrow().as_ref() {
                if let Some(rc) = weak.upgrade() {
                    let mut s = rc.borrow_mut();
                    s.loading.remove(&path);
                    if let Some(weak_area) = s.thumb_areas.get(&path) {
                        if let Some(a) = weak_area.upgrade() {
                            a.queue_draw();
                        }
                    }
                }
            }
        });
    });
}

pub struct HomeScreenState {
    pub cache: ImageCache,
    pub colors: OmarchyColors,
    pub surfaces: HashMap<PathBuf, cairo::ImageSurface>,
    pub thumb_areas: HashMap<PathBuf, glib::WeakRef<DrawingArea>>,
    pub loading: std::collections::HashSet<PathBuf>,
    pub on_open_image: Option<Rc<dyn Fn(Vec<PathBuf>, usize)>>,
}

#[derive(Clone)]
pub struct HomeScreen {
    container: GtkBox,
    albums_box: GtkBox,
    state: Rc<RefCell<HomeScreenState>>,
}

impl HomeScreen {
    pub fn new(cache: ImageCache, colors: OmarchyColors) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("home-screen");
        container.set_vexpand(true);
        container.set_hexpand(true);

        // 1. Top Header Area with Centered Floating Pill Button
        let top_bar = GtkBox::new(Orientation::Horizontal, 0);
        top_bar.add_css_class("home-top-bar");
        top_bar.set_halign(gtk4::Align::Center);
        top_bar.set_margin_top(20);
        top_bar.set_margin_bottom(12);

        let btn_add_album = Button::new();
        btn_add_album.add_css_class("floating-pill");
        btn_add_album.add_css_class("home-add-album-btn");
        btn_add_album.set_has_frame(false);

        let btn_content = GtkBox::new(Orientation::Horizontal, 8);
        let btn_icon = Image::from_icon_name("folder-new-symbolic");
        let btn_label = Label::new(Some("Add Folder as Album"));
        btn_label.add_css_class("home-add-btn-label");
        btn_content.append(&btn_icon);
        btn_content.append(&btn_label);
        btn_add_album.set_child(Some(&btn_content));
        btn_add_album.set_tooltip_text(Some("Add a directory of photos as an album"));

        top_bar.append(&btn_add_album);
        container.append(&top_bar);

        // 2. Scrolled Area containing Album rows
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_policy(PolicyType::Never, PolicyType::Automatic);

        let albums_box = GtkBox::new(Orientation::Vertical, 24);
        albums_box.add_css_class("home-albums-container");
        albums_box.set_margin_top(12);
        albums_box.set_margin_bottom(40);
        albums_box.set_margin_start(28);
        albums_box.set_margin_end(28);

        scrolled.set_child(Some(&albums_box));
        container.append(&scrolled);

        let state = Rc::new(RefCell::new(HomeScreenState {
            cache,
            colors,
            surfaces: HashMap::new(),
            thumb_areas: HashMap::new(),
            loading: std::collections::HashSet::new(),
            on_open_image: None,
        }));

        HOME_STATE_REF.with(|cell| {
            *cell.borrow_mut() = Some(Rc::downgrade(&state));
        });

        let home = Self {
            container,
            albums_box,
            state,
        };

        // Wire Add Album Button Click
        let home_for_picker = home.clone();
        btn_add_album.connect_clicked(move |btn| {
            let root = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select Folder for Album");

            let home_cb = home_for_picker.clone();
            dialog.select_folder(root.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(folder) = res {
                    if let Some(path) = folder.path() {
                        let _ = add_album(path);
                        home_cb.refresh();
                    }
                }
            });
        });

        home.refresh();
        home
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_colors(&self, colors: OmarchyColors) {
        self.state.borrow_mut().colors = colors;
        self.refresh();
    }

    pub fn connect_open_image<F: Fn(Vec<PathBuf>, usize) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_open_image = Some(Rc::new(callback));
    }

    pub fn refresh(&self) {
        // Clear current album rows
        while let Some(child) = self.albums_box.first_child() {
            self.albums_box.remove(&child);
        }

        let album_dirs = load_albums();
        let colors = self.state.borrow().colors.clone();
        let accent_hex = colors.accent.as_deref().unwrap_or("#7aa2f7").to_string();
        let (ar, ag, ab) = crate::theme::parse_hex_color(&accent_hex).unwrap_or((0.48, 0.64, 0.97));

        if album_dirs.is_empty() {
            let empty_lbl = Label::new(Some("No albums added yet. Click above to add a photo folder!"));
            empty_lbl.add_css_class("dim-label");
            empty_lbl.set_margin_top(40);
            self.albums_box.append(&empty_lbl);
            return;
        }

        for dir in album_dirs {
            let images = scan_directory_images(&dir);
            let image_count = images.len();

            let section = GtkBox::new(Orientation::Vertical, 8);
            section.add_css_class("album-section");

            // Header row
            let header = GtkBox::new(Orientation::Horizontal, 10);
            header.add_css_class("album-header");

            let folder_icon = Image::from_icon_name("folder-pictures-symbolic");
            folder_icon.add_css_class("album-folder-icon");
            header.append(&folder_icon);

            let folder_name = dir
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("Album");
            let title_lbl = Label::new(Some(folder_name));
            title_lbl.add_css_class("album-title-label");
            header.append(&title_lbl);

            let count_badge = Label::new(Some(&format!("{} photos", image_count)));
            count_badge.add_css_class("album-count-badge");
            header.append(&count_badge);

            let path_hint = Label::new(Some(&dir.display().to_string()));
            path_hint.add_css_class("album-path-hint");
            path_hint.add_css_class("dim-label");
            path_hint.set_hexpand(true);
            path_hint.set_halign(gtk4::Align::Start);
            path_hint.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            header.append(&path_hint);

            // Open Folder in Viewer button
            if image_count > 0 {
                let btn_open_all = Button::from_icon_name("media-playback-start-symbolic");
                btn_open_all.set_has_frame(false);
                btn_open_all.add_css_class("album-action-btn");
                btn_open_all.set_tooltip_text(Some("Open this album in viewer"));
                let images_all = images.clone();
                let state_open = self.state.clone();
                btn_open_all.connect_clicked(move |_| {
                    if let Some(ref cb) = state_open.borrow().on_open_image {
                        cb(images_all.clone(), 0);
                    }
                });
                header.append(&btn_open_all);
            }

            // Remove Album button
            let btn_remove = Button::from_icon_name("window-close-symbolic");
            btn_remove.set_has_frame(false);
            btn_remove.add_css_class("album-action-btn");
            btn_remove.set_tooltip_text(Some("Remove album from home screen"));
            let home_remove = self.clone();
            let dir_for_remove = dir.clone();
            btn_remove.connect_clicked(move |_| {
                let _ = remove_album(&dir_for_remove);
                home_remove.refresh();
            });
            header.append(&btn_remove);

            section.append(&header);

            // Horizontal Strip of Thumbnails
            if image_count == 0 {
                let empty_strip_box = GtkBox::new(Orientation::Horizontal, 0);
                empty_strip_box.set_margin_start(16);
                empty_strip_box.set_margin_top(12);
                empty_strip_box.set_margin_bottom(12);
                let lbl = Label::new(Some("No supported images found in this folder."));
                lbl.add_css_class("dim-label");
                empty_strip_box.append(&lbl);
                section.append(&empty_strip_box);
            } else {
                let strip_scroll = ScrolledWindow::new();
                strip_scroll.set_policy(PolicyType::Automatic, PolicyType::Never);
                strip_scroll.set_min_content_height(162);

                let strip_box = GtkBox::new(Orientation::Horizontal, 14);
                strip_box.add_css_class("album-strip-box");
                strip_box.set_margin_top(4);
                strip_box.set_margin_bottom(8);
                strip_box.set_margin_start(4);
                strip_box.set_margin_end(4);

                // Add cards for images
                for (idx, img_path) in images.iter().enumerate() {
                    let card = self.create_thumbnail_card(
                        img_path,
                        &images,
                        idx,
                        ar,
                        ag,
                        ab,
                    );
                    strip_box.append(&card);
                }

                strip_scroll.set_child(Some(&strip_box));
                section.append(&strip_scroll);
            }

            self.albums_box.append(&section);
        }
    }

    fn create_thumbnail_card(
        &self,
        img_path: &Path,
        all_images: &[PathBuf],
        idx: usize,
        ar: f64,
        ag: f64,
        ab: f64,
    ) -> GtkBox {
        let card = GtkBox::new(Orientation::Vertical, 5);
        card.add_css_class("album-card");
        card.set_width_request(CARD_WIDTH as i32);
        card.set_cursor_from_name(Some("pointer"));

        let area = DrawingArea::new();
        area.set_content_width(CARD_WIDTH as i32);
        area.set_content_height(CARD_HEIGHT as i32);
        area.add_css_class("album-card-image");

        let state_card = self.state.clone();
        let path_clone = img_path.to_path_buf();
        self.state.borrow_mut().thumb_areas.insert(path_clone.clone(), area.downgrade());
        let is_hovered = Rc::new(RefCell::new(false));

        let is_hov_draw = is_hovered.clone();
        area.set_draw_func(move |_drawing_area, cr, width, height| {
            let mut s = state_card.borrow_mut();
            let w = width as f64;
            let h = height as f64;
            let hov = *is_hov_draw.borrow();

            // Card background
            cr.save().ok();
            draw_rounded_rect(cr, 0.0, 0.0, w, h, CARD_CORNER_RADIUS);
            cr.set_source_rgba(0.08, 0.09, 0.13, 0.70);
            let _ = cr.fill();
            cr.restore().ok();

            // Look up surface or cache
            if !s.surfaces.contains_key(&path_clone) {
                if let Some(dec) = s.cache.get_thumbnail(&path_clone) {
                    if let Ok(surf) = rgba_to_cairo_surface(&dec.rgba) {
                        s.surfaces.insert(path_clone.clone(), surf);
                    }
                }
            }

            if let Some(surface) = s.surfaces.get(&path_clone) {
                let tw = surface.width() as f64;
                let th = surface.height() as f64;
                let scale = (w / tw).max(h / th);
                let dw = tw * scale;
                let dh = th * scale;
                let dx = (w - dw) / 2.0;
                let dy = (h - dh) / 2.0;

                cr.save().ok();
                draw_rounded_rect(cr, 0.0, 0.0, w, h, CARD_CORNER_RADIUS);
                cr.clip();
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(surface, dx / scale, dy / scale);
                let _ = cr.paint();
                cr.restore().ok();
            } else if !s.loading.contains(&path_clone) {
                s.loading.insert(path_clone.clone());
                let path_async = path_clone.clone();
                let cache_async = s.cache.clone();

                std::thread::spawn(move || {
                    if let Ok(img) = load_dynamic_image(&path_async) {
                        let thumb = generate_thumbnail(&img, 240);
                        cache_async.put_thumbnail(path_async.clone(), Arc::new(thumb));
                        trigger_home_thumb_redraw(path_async);
                    }
                });
            }

            // Glass card border & specular highlight
            cr.save().ok();
            draw_rounded_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, CARD_CORNER_RADIUS);
            if hov {
                cr.set_source_rgba(ar, ag, ab, 0.95);
                cr.set_line_width(2.0);
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();

            // Specular top highlight line (gradient shine)
            cr.move_to(CARD_CORNER_RADIUS, 1.0);
            cr.line_to(w - CARD_CORNER_RADIUS, 1.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.55);
            cr.set_line_width(1.0);
            let _ = cr.stroke();

            // Specular bottom subtle highlight line
            cr.move_to(CARD_CORNER_RADIUS, h - 1.0);
            cr.line_to(w - CARD_CORNER_RADIUS, h - 1.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            cr.restore().ok();
        });

        // Hover controller
        let motion_ctrl = EventControllerMotion::new();
        let is_hov_enter = is_hovered.clone();
        let area_weak = area.downgrade();
        motion_ctrl.connect_enter(move |_, _, _| {
            *is_hov_enter.borrow_mut() = true;
            if let Some(a) = area_weak.upgrade() {
                a.queue_draw();
            }
        });
        let is_hov_leave = is_hovered.clone();
        let area_weak_leave = area.downgrade();
        motion_ctrl.connect_leave(move |_| {
            *is_hov_leave.borrow_mut() = false;
            if let Some(a) = area_weak_leave.upgrade() {
                a.queue_draw();
            }
        });
        area.add_controller(motion_ctrl);

        card.append(&area);

        // Filename label
        let filename = img_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");
        let lbl = Label::new(Some(filename));
        lbl.add_css_class("album-card-label");
        lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        lbl.set_max_width_chars(22);
        lbl.set_halign(gtk4::Align::Center);
        card.append(&lbl);

        // Click gesture on card
        let click = GestureClick::new();
        let images_for_click = all_images.to_vec();
        let state_click = self.state.clone();
        click.connect_pressed(move |_, _, _, _| {
            let cb_opt = state_click.borrow().on_open_image.clone();
            if let Some(cb) = cb_opt {
                cb(images_for_click.clone(), idx);
            }
        });
        card.add_controller(click);

        card
    }
}
