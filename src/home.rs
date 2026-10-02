use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerMotion, FlowBox, GestureClick,
    Image, Label, Orientation, Overlay, PolicyType, Revealer, RevealerTransitionType,
    ScrolledWindow, Stack, StackTransitionType, Window,
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
                    if let Some(areas) = s.thumb_areas.get_mut(&path) {
                        areas.retain(|w| {
                            if let Some(a) = w.upgrade() {
                                a.queue_draw();
                                true
                            } else {
                                false
                            }
                        });
                    }
                }
            }
        });
    });
}

fn update_raw_extraction_progress(dir: PathBuf, done: usize, total: usize) {
    glib::idle_add_once(move || {
        HOME_STATE_REF.with(|cell| {
            if let Some(weak) = cell.borrow().as_ref() {
                if let Some(rc) = weak.upgrade() {
                    let s = rc.borrow();
                    if let Some(row) = s.album_rows.iter().find(|r| r.dir == dir) {
                        if done >= total {
                            row.progress_label.set_visible(false);
                        } else {
                            let pct = (done * 100) / total;
                            row.progress_label.set_text(&format!("Extracting RAW: {}/{} ({}%)", done, total, pct));
                            row.progress_label.set_visible(true);
                        }
                    }
                }
            }
        });
    });
}

#[derive(Clone)]
pub struct AlbumRow {
    pub dir: PathBuf,
    pub images: Vec<PathBuf>,
    pub section: GtkBox,
    #[allow(dead_code)]
    pub header: GtkBox,
    pub progress_label: Label,
    pub btn_expand: Button,
    pub revealer: Revealer,
    pub content_stack: Stack,
    pub grid_initialized: Rc<RefCell<bool>>,
}

pub struct HomeScreenState {
    pub cache: ImageCache,
    pub colors: OmarchyColors,
    pub surfaces: HashMap<PathBuf, cairo::ImageSurface>,
    pub thumb_areas: HashMap<PathBuf, Vec<glib::WeakRef<DrawingArea>>>,
    pub loading: std::collections::HashSet<PathBuf>,
    pub on_open_image: Option<Rc<dyn Fn(Vec<PathBuf>, usize)>>,
    pub expanded_album: Option<PathBuf>,
    pub album_rows: Vec<AlbumRow>,
}

#[derive(Clone)]
pub struct HomeScreen {
    container: GtkBox,
    scrolled: ScrolledWindow,
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

        let initial_expand = std::env::var("OMAVIEW_EXPAND_ALBUM").ok().map(PathBuf::from);
        let state = Rc::new(RefCell::new(HomeScreenState {
            cache,
            colors,
            surfaces: HashMap::new(),
            thumb_areas: HashMap::new(),
            loading: std::collections::HashSet::new(),
            on_open_image: None,
            expanded_album: initial_expand,
            album_rows: Vec::new(),
        }));

        HOME_STATE_REF.with(|cell| {
            *cell.borrow_mut() = Some(Rc::downgrade(&state));
        });

        let home = Self {
            container,
            scrolled,
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

    #[allow(dead_code)]
    pub fn expand_album(&self, path: PathBuf) {
        self.state.borrow_mut().expanded_album = Some(path);
        self.apply_expansion(true);
    }

    fn ensure_album_grid(&self, row: &AlbumRow) {
        if *row.grid_initialized.borrow() {
            return;
        }
        *row.grid_initialized.borrow_mut() = true;

        if row.images.is_empty() {
            let empty_strip_box = GtkBox::new(Orientation::Horizontal, 0);
            empty_strip_box.set_margin_start(16);
            empty_strip_box.set_margin_top(12);
            empty_strip_box.set_margin_bottom(12);
            let lbl = Label::new(Some("No supported images found in this folder."));
            lbl.add_css_class("dim-label");
            empty_strip_box.append(&lbl);
            row.content_stack.add_named(&empty_strip_box, Some("grid"));
            return;
        }

        let grid_scroll = ScrolledWindow::new();
        grid_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
        grid_scroll.set_vexpand(true);
        grid_scroll.set_hexpand(true);
        grid_scroll.add_css_class("album-expanded-scroll");

        let flow = FlowBox::new();
        flow.set_selection_mode(gtk4::SelectionMode::None);
        flow.set_homogeneous(false);
        flow.set_max_children_per_line(50);
        flow.set_min_children_per_line(1);
        flow.set_row_spacing(14);
        flow.set_column_spacing(14);
        flow.set_hexpand(true);
        flow.set_vexpand(false);
        flow.set_valign(gtk4::Align::Start);
        flow.add_css_class("album-expanded-flowbox");

        let colors = self.state.borrow().colors.clone();
        let accent_hex = colors.accent.as_deref().unwrap_or("#7aa2f7").to_string();
        let (ar, ag, ab) = crate::theme::parse_hex_color(&accent_hex).unwrap_or((0.48, 0.64, 0.97));

        for (idx, img_path) in row.images.iter().enumerate() {
            let card = self.create_thumbnail_card(
                img_path,
                &row.images,
                idx,
                ar,
                ag,
                ab,
            );
            flow.append(&card);
        }

        grid_scroll.set_child(Some(&flow));
        row.content_stack.add_named(&grid_scroll, Some("grid"));
    }

    fn apply_expansion(&self, animate: bool) {
        let expanded_dir_opt = self.state.borrow().expanded_album.clone();
        let is_any_expanded = expanded_dir_opt.is_some();

        if is_any_expanded {
            self.scrolled.set_policy(PolicyType::Never, PolicyType::Never);
            self.scrolled.vadjustment().set_value(0.0);
            self.albums_box.set_vexpand(true);
            self.albums_box.set_spacing(14);
            self.albums_box.set_margin_top(8);
            self.albums_box.set_margin_bottom(16);
        } else {
            self.scrolled.set_policy(PolicyType::Never, PolicyType::Automatic);
            self.albums_box.set_vexpand(false);
            self.albums_box.set_spacing(18);
            self.albums_box.set_margin_top(12);
            self.albums_box.set_margin_bottom(40);
        }

        let rows = self.state.borrow().album_rows.clone();
        for row in rows {
            let is_this_expanded = expanded_dir_opt.as_ref() == Some(&row.dir);

            let dur_revealer = if animate { 250 } else { 0 };
            let dur_stack = if animate { 140 } else { 0 };

            row.revealer.set_transition_duration(dur_revealer);
            row.content_stack.set_transition_duration(dur_stack);

            if is_this_expanded {
                self.ensure_album_grid(&row);

                row.section.remove_css_class("album-section-collapsed");
                row.section.add_css_class("album-section-expanded");
                row.section.set_vexpand(true);
                row.section.set_valign(gtk4::Align::Fill);

                row.revealer.set_vexpand(true);
                row.revealer.set_reveal_child(true);

                row.content_stack.set_vexpand(true);
                row.content_stack.set_visible_child_name("grid");

                row.btn_expand.set_icon_name("view-restore-symbolic");
                row.btn_expand.add_css_class("active-highlight");
                row.btn_expand.set_tooltip_text(Some("Collapse album view"));
            } else if is_any_expanded {
                row.section.remove_css_class("album-section-expanded");
                row.section.add_css_class("album-section-collapsed");
                row.section.set_vexpand(false);
                row.section.set_valign(gtk4::Align::Start);

                row.revealer.set_vexpand(false);
                row.revealer.set_reveal_child(false);

                row.content_stack.set_vexpand(false);
                row.content_stack.set_visible_child_name("strip");

                row.btn_expand.set_icon_name("view-fullscreen-symbolic");
                row.btn_expand.remove_css_class("active-highlight");
                row.btn_expand.set_tooltip_text(Some("Expand album to fill available space"));
            } else {
                row.section.remove_css_class("album-section-expanded");
                row.section.remove_css_class("album-section-collapsed");
                row.section.set_vexpand(false);
                row.section.set_valign(gtk4::Align::Start);

                row.revealer.set_vexpand(false);
                row.revealer.set_reveal_child(true);

                row.content_stack.set_vexpand(false);
                row.content_stack.set_visible_child_name("strip");

                row.btn_expand.set_icon_name("view-fullscreen-symbolic");
                row.btn_expand.remove_css_class("active-highlight");
                row.btn_expand.set_tooltip_text(Some("Expand album to fill available space"));
            }
        }
    }

    pub fn refresh(&self) {
        // Clear current album rows
        while let Some(child) = self.albums_box.first_child() {
            self.albums_box.remove(&child);
        }
        self.state.borrow_mut().thumb_areas.clear();
        self.state.borrow_mut().album_rows.clear();

        let album_dirs = load_albums();
        let colors = self.state.borrow().colors.clone();
        let accent_hex = colors.accent.as_deref().unwrap_or("#7aa2f7").to_string();
        let (ar, ag, ab) = crate::theme::parse_hex_color(&accent_hex).unwrap_or((0.48, 0.64, 0.97));

        if album_dirs.is_empty() {
            self.state.borrow_mut().expanded_album = None;
            self.scrolled.set_policy(PolicyType::Never, PolicyType::Automatic);
            self.albums_box.set_vexpand(false);
            self.albums_box.set_spacing(18);
            let empty_lbl = Label::new(Some("No albums added yet. Click above to add a photo folder!"));
            empty_lbl.add_css_class("dim-label");
            empty_lbl.set_margin_top(40);
            self.albums_box.append(&empty_lbl);
            return;
        }

        // Clean up expanded_album if it's no longer in album_dirs
        if let Some(ref exp) = self.state.borrow().expanded_album {
            if !album_dirs.contains(exp) {
                self.state.borrow_mut().expanded_album = None;
            }
        }

        let mut rows = Vec::new();
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

            // Live progress badge for RAW preview extraction
            let progress_label = Label::new(None);
            progress_label.add_css_class("album-progress-badge");
            progress_label.set_visible(false);
            header.append(&progress_label);

            let pending_raw: Vec<PathBuf> = images
                .iter()
                .filter(|p| {
                    crate::raw_loader::is_raw_image(p)
                        && crate::raw_loader::find_companion_jpeg(p).is_none()
                        && crate::raw_loader::get_cached_raw_preview_path(p).is_none()
                })
                .cloned()
                .collect();

            if !pending_raw.is_empty() {
                let total = pending_raw.len();
                progress_label.set_text(&format!("Extracting RAW: 0/{} (0%)", total));
                progress_label.set_visible(true);

                let cache_clone = self.state.borrow().cache.clone();
                let dir_for_thread = dir.clone();
                std::thread::spawn(move || {
                    for (i, raw_file) in pending_raw.into_iter().enumerate() {
                        let _ = crate::raw_loader::get_or_extract_raw_preview(&raw_file);
                        if let Ok(img) = crate::image_loader::load_dynamic_image(&raw_file) {
                            let thumb = crate::image_loader::generate_thumbnail(&img, 240);
                            cache_clone.put_thumbnail(raw_file.clone(), Arc::new(thumb));
                        }
                        trigger_home_thumb_redraw(raw_file);

                        let done = i + 1;
                        update_raw_extraction_progress(dir_for_thread.clone(), done, total);
                    }
                });
            }

            let btn_expand = Button::from_icon_name("view-fullscreen-symbolic");
            btn_expand.set_has_frame(false);
            btn_expand.add_css_class("album-action-btn");
            btn_expand.set_tooltip_text(Some("Expand album to fill available space"));

            if image_count > 0 {
                // 1. Open Folder in Viewer button
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

                // 2. Expand Button
                let home_expand = self.clone();
                let dir_for_expand = dir.clone();
                btn_expand.connect_clicked(move |_| {
                    let mut s = home_expand.state.borrow_mut();
                    if s.expanded_album.as_ref() == Some(&dir_for_expand) {
                        s.expanded_album = None;
                    } else {
                        s.expanded_album = Some(dir_for_expand.clone());
                    }
                    drop(s);
                    home_expand.apply_expansion(true);
                });
                header.append(&btn_expand);
            }

            // 3. Remove Album button
            let btn_remove = Button::from_icon_name("window-close-symbolic");
            btn_remove.set_has_frame(false);
            btn_remove.add_css_class("album-action-btn");
            btn_remove.set_tooltip_text(Some("Remove album from home screen"));
            let home_remove = self.clone();
            let dir_for_remove = dir.clone();
            btn_remove.connect_clicked(move |_| {
                let mut s = home_remove.state.borrow_mut();
                if s.expanded_album.as_ref() == Some(&dir_for_remove) {
                    s.expanded_album = None;
                }
                drop(s);
                let _ = remove_album(&dir_for_remove);
                home_remove.refresh();
            });
            header.append(&btn_remove);

            // Clicking collapsed header row switches expansion to this album
            if image_count > 0 {
                let home_row_expand = self.clone();
                let dir_row = dir.clone();
                let click_expand = GestureClick::new();
                click_expand.connect_pressed(move |_, _, _, _| {
                    let should_expand = {
                        let s = home_row_expand.state.borrow();
                        s.expanded_album.is_some() && s.expanded_album.as_ref() != Some(&dir_row)
                    };
                    if should_expand {
                        home_row_expand.state.borrow_mut().expanded_album = Some(dir_row.clone());
                        home_row_expand.apply_expansion(true);
                    }
                });
                header.add_controller(click_expand);
            }

            section.append(&header);

            // Content Revealer + Stack
            let revealer = Revealer::new();
            revealer.set_transition_type(RevealerTransitionType::SlideDown);
            revealer.set_transition_duration(250);
            revealer.set_reveal_child(true);
            revealer.add_css_class("album-revealer");

            let content_stack = Stack::new();
            content_stack.set_transition_type(StackTransitionType::Crossfade);
            content_stack.set_transition_duration(140);
            content_stack.set_interpolate_size(false);
            content_stack.add_css_class("album-content-stack");

            if image_count == 0 {
                let empty_strip_box = GtkBox::new(Orientation::Horizontal, 0);
                empty_strip_box.set_margin_start(16);
                empty_strip_box.set_margin_top(12);
                empty_strip_box.set_margin_bottom(12);
                let lbl = Label::new(Some("No supported images found in this folder."));
                lbl.add_css_class("dim-label");
                empty_strip_box.append(&lbl);
                content_stack.add_named(&empty_strip_box, Some("strip"));
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
                content_stack.add_named(&strip_scroll, Some("strip"));
            }

            revealer.set_child(Some(&content_stack));
            section.append(&revealer);
            self.albums_box.append(&section);

            rows.push(AlbumRow {
                dir,
                images,
                section,
                header,
                progress_label,
                btn_expand,
                revealer,
                content_stack,
                grid_initialized: Rc::new(RefCell::new(false)),
            });
        }

        self.state.borrow_mut().album_rows = rows.clone();
        self.apply_expansion(false);

        // Pre-initialize album grids in idle tick so expanding is 100% instant with 0 dropped frames
        for row in rows {
            let row_clone = row.clone();
            let home_clone = self.clone();
            glib::idle_add_local_once(move || {
                home_clone.ensure_album_grid(&row_clone);
            });
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
        self.state
            .borrow_mut()
            .thumb_areas
            .entry(path_clone.clone())
            .or_default()
            .push(area.downgrade());
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

        let is_raw = crate::raw_loader::is_raw_image(img_path);
        let image_widget: gtk4::Widget = if is_raw {
            let overlay = Overlay::new();
            overlay.set_child(Some(&area));

            let companion = crate::raw_loader::find_companion_jpeg(img_path);
            let badge_text = if companion.is_some() {
                "RAW+JPG".to_string()
            } else {
                crate::raw_loader::raw_format_badge(img_path).to_string()
            };

            let badge = Label::new(Some(&badge_text));
            badge.add_css_class("raw-badge");
            if companion.is_some() {
                badge.add_css_class("raw-badge-companion");
            }
            badge.set_can_target(false);
            badge.set_halign(gtk4::Align::End);
            badge.set_valign(gtk4::Align::Start);
            badge.set_margin_top(6);
            badge.set_margin_end(6);

            overlay.add_overlay(&badge);
            overlay.upcast()
        } else {
            area.upcast()
        };

        card.append(&image_widget);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_home_expanded_album_logic() {
        gtk4::init().ok();
        let cache = ImageCache::new(2, 10);
        let colors = OmarchyColors::default();
        let home = HomeScreen::new(cache, colors);

        // Initially no album is expanded
        assert!(home.state.borrow().expanded_album.is_none());
        assert_eq!(home.scrolled.hscrollbar_policy(), PolicyType::Never);
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Automatic);

        // Expand an album
        let test_dir = PathBuf::from("/home/sr/Pictures");
        home.state.borrow_mut().expanded_album = Some(test_dir.clone());
        home.refresh();

        assert_eq!(home.state.borrow().expanded_album, Some(test_dir.clone()));
        // When expanded, outside scrollbar must be disabled
        assert_eq!(home.scrolled.hscrollbar_policy(), PolicyType::Never);
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Never);
        assert!(home.albums_box.vexpands());

        // Verify FlowBox has Align::Start and does not vexpand so gap is below last row
        let first_section = home.albums_box.first_child().unwrap();
        let header = first_section.first_child().unwrap();
        let revealer = header.next_sibling().unwrap().downcast::<Revealer>().unwrap();
        let stack = revealer.child().unwrap().downcast::<Stack>().unwrap();
        let scroll = stack.child_by_name("grid").unwrap().downcast::<ScrolledWindow>().unwrap();
        let scroll_child = scroll.child().unwrap();
        let flow = if let Ok(viewport) = scroll_child.clone().downcast::<gtk4::Viewport>() {
            viewport.child().unwrap().downcast::<FlowBox>().unwrap()
        } else {
            scroll_child.downcast::<FlowBox>().unwrap()
        };
        assert_eq!(flow.valign(), gtk4::Align::Start);
        assert!(!flow.vexpands());

        // Now collapse the album
        home.state.borrow_mut().expanded_album = None;
        home.refresh();

        assert!(home.state.borrow().expanded_album.is_none());
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Automatic);
        assert!(!home.albums_box.vexpands());

        // Test thumbnail card badge overlay logic
        let temp_dir = std::env::temp_dir().join(format!("omaview_test_badges_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Regular non-raw image
        let png_path = temp_dir.join("image.png");
        let _ = std::fs::write(&png_path, b"png");
        let card_png = home.create_thumbnail_card(&png_path, &[png_path.clone()], 0, 0.5, 0.5, 0.5);
        let first_child_png = card_png.first_child().unwrap();
        // Regular card directly holds DrawingArea
        assert!(first_child_png.downcast::<DrawingArea>().is_ok());

        // 2. Standalone RAW image
        let raw_path = temp_dir.join("photo.arw");
        let _ = std::fs::write(&raw_path, b"raw");
        let card_raw = home.create_thumbnail_card(&raw_path, &[raw_path.clone()], 0, 0.5, 0.5, 0.5);
        let first_child_raw = card_raw.first_child().unwrap();
        // RAW card wraps DrawingArea in an Overlay with badge
        let overlay = first_child_raw.downcast::<Overlay>().expect("RAW image should be wrapped in Overlay");
        let mut found_arw_badge = false;
        let mut child = overlay.first_child();
        while let Some(w) = child {
            if let Ok(lbl) = w.clone().downcast::<Label>() {
                if lbl.text() == "ARW" && lbl.has_css_class("raw-badge") {
                    found_arw_badge = true;
                }
            }
            child = w.next_sibling();
        }
        assert!(found_arw_badge, "RAW card must display ARW format badge");

        // 3. RAW with companion JPEG
        let cr3_path = temp_dir.join("photo2.cr3");
        let jpg_path = temp_dir.join("photo2.jpg");
        let _ = std::fs::write(&cr3_path, b"raw");
        let _ = std::fs::write(&jpg_path, b"jpg");
        let card_companion = home.create_thumbnail_card(&cr3_path, &[cr3_path.clone()], 0, 0.5, 0.5, 0.5);
        let first_child_comp = card_companion.first_child().unwrap();
        let overlay_comp = first_child_comp.downcast::<Overlay>().expect("RAW+JPG should be wrapped in Overlay");
        let mut found_companion_badge = false;
        let mut child_comp = overlay_comp.first_child();
        while let Some(w) = child_comp {
            if let Ok(lbl) = w.clone().downcast::<Label>() {
                if lbl.text() == "RAW+JPG" && lbl.has_css_class("raw-badge-companion") {
                    found_companion_badge = true;
                }
            }
            child_comp = w.next_sibling();
        }
        assert!(found_companion_badge, "RAW+JPG card must display RAW+JPG badge with companion class");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

