use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerMotion, FlowBox, GestureClick, Image, Label,
    Orientation, Overlay, PolicyType, Revealer, RevealerTransitionType, ScrolledWindow, Stack,
    StackTransitionType, Window,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::albums::{add_album, load_albums, remove_album};
use crate::image_loader::{ImageCache, rgba_to_cairo_surface};
use crate::theme::OmarchyColors;
use crate::util::scan_directory_images;

const CARD_WIDTH: f64 = 180.0;
const CARD_HEIGHT: f64 = 116.0;
const CARD_CORNER_RADIUS: f64 = 10.0;
const GRID_FIRST_BATCH: usize = 24;
const GRID_FRAME_BATCH: usize = 16;
const ALBUM_EXPAND_MS: u32 = 200;

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

thread_local! {
    static HOME_STATE_REF: RefCell<Option<std::rc::Weak<RefCell<HomeScreenState>>>> = const { RefCell::new(None) };
}

fn trigger_home_thumb_redraw(path: PathBuf, success: bool) {
    glib::idle_add_once(move || {
        HOME_STATE_REF.with(|cell| {
            if let Some(weak) = cell.borrow().as_ref()
                && let Some(rc) = weak.upgrade()
            {
                let mut s = rc.borrow_mut();
                if success {
                    s.loading.remove(&path);
                }
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
        });
    });
}

fn update_raw_extraction_progress(dir: PathBuf, done: usize, total: usize) {
    glib::idle_add_once(move || {
        HOME_STATE_REF.with(|cell| {
            if let Some(weak) = cell.borrow().as_ref()
                && let Some(rc) = weak.upgrade()
            {
                let s = rc.borrow();
                if let Some(row) = s.album_rows.iter().find(|r| r.dir == dir) {
                    if done >= total {
                        row.progress_label.set_visible(false);
                    } else {
                        let pct = (done * 100) / total;
                        row.progress_label
                            .set_text(&format!("Extracting RAW: {}/{} ({}%)", done, total, pct));
                        row.progress_label.set_visible(true);
                    }
                }
            }
        });
    });
}

#[derive(Clone)]
pub struct AlbumRow {
    pub dir: PathBuf,
    pub images: Rc<Vec<PathBuf>>,
    pub section: GtkBox,
    #[allow(dead_code)]
    pub header: GtkBox,
    pub progress_label: Label,
    pub btn_expand: Button,
    pub revealer: Revealer,
    pub content_stack: Stack,
    pub grid_initialized: Rc<RefCell<bool>>,
}

type OpenImageCallback = Rc<dyn Fn(Vec<PathBuf>, usize)>;

pub struct HomeScreenState {
    pub cache: ImageCache,
    pub colors: OmarchyColors,
    pub surfaces: HashMap<PathBuf, cairo::ImageSurface>,
    pub thumb_areas: HashMap<PathBuf, Vec<glib::WeakRef<DrawingArea>>>,
    pub loading: std::collections::HashSet<PathBuf>,
    pub on_open_image: Option<OpenImageCallback>,
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

        let initial_expand = std::env::var("OMAVIEW_EXPAND_ALBUM")
            .ok()
            .map(PathBuf::from);
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
                if let Ok(folder) = res
                    && let Some(path) = folder.path()
                {
                    let _ = add_album(path);
                    home_cb.refresh();
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

        // Make the first screen available immediately. The rest yields to GTK
        // between frames, so large albums cannot monopolize the expand click.
        let initial_count = row.images.len().min(GRID_FIRST_BATCH);
        for idx in 0..initial_count {
            let card = Self::create_thumbnail_card(
                &self.state,
                &row.images[idx],
                &row.images,
                idx,
                ar,
                ag,
                ab,
            );
            flow.append(&card);
        }
        if initial_count < row.images.len() {
            let next = Cell::new(initial_count);
            let images = row.images.clone();
            let state_weak = Rc::downgrade(&self.state);
            let dir = row.dir.clone();
            flow.add_tick_callback(move |flow, _| {
                let Some(state) = state_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if !flow.is_mapped() || state.borrow().expanded_album.as_ref() != Some(&dir) {
                    return glib::ControlFlow::Continue;
                }
                let started = std::time::Instant::now();
                let end = (next.get() + GRID_FRAME_BATCH).min(images.len());
                while next.get() < end {
                    let idx = next.get();
                    let card =
                        Self::create_thumbnail_card(&state, &images[idx], &images, idx, ar, ag, ab);
                    flow.append(&card);
                    next.set(idx + 1);
                    // Leave most of the frame budget for layout and rendering.
                    if started.elapsed() >= std::time::Duration::from_millis(2) {
                        break;
                    }
                }
                if next.get() == images.len() {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }

        grid_scroll.set_child(Some(&flow));
        row.content_stack.add_named(&grid_scroll, Some("grid"));
    }

    fn apply_expansion(&self, animate: bool) {
        let expanded_dir_opt = self.state.borrow().expanded_album.clone();
        let is_any_expanded = expanded_dir_opt.is_some();

        if is_any_expanded {
            self.scrolled
                .set_policy(PolicyType::Never, PolicyType::Never);
            self.scrolled.vadjustment().set_value(0.0);
            self.albums_box.set_vexpand(true);
            self.albums_box.set_spacing(14);
            self.albums_box.set_margin_top(8);
            self.albums_box.set_margin_bottom(16);
        } else {
            self.scrolled
                .set_policy(PolicyType::Never, PolicyType::Automatic);
            self.albums_box.set_vexpand(false);
            self.albums_box.set_spacing(18);
            self.albums_box.set_margin_top(12);
            self.albums_box.set_margin_bottom(40);
        }

        let animate = animate && self.container.settings().is_gtk_enable_animations();
        let rows = self.state.borrow().album_rows.clone();
        for row in rows {
            let is_this_expanded = expanded_dir_opt.as_ref() == Some(&row.dir);

            // A single ease-out height transition clips the closing strips and
            // lets the selected album grow into the freed space. `None` hides
            // content but still reserves its height, leaving empty album cards.
            row.revealer
                .set_transition_duration(if animate { ALBUM_EXPAND_MS } else { 0 });
            row.content_stack.set_transition_duration(0);

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
                row.btn_expand
                    .set_tooltip_text(Some("Expand album to fill available space"));
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
                row.btn_expand
                    .set_tooltip_text(Some("Expand album to fill available space"));
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
        self.state.borrow_mut().loading.clear();

        let album_dirs = load_albums();
        let colors = self.state.borrow().colors.clone();
        let accent_hex = colors.accent.as_deref().unwrap_or("#7aa2f7").to_string();
        let (ar, ag, ab) = crate::theme::parse_hex_color(&accent_hex).unwrap_or((0.48, 0.64, 0.97));

        if album_dirs.is_empty() {
            self.state.borrow_mut().expanded_album = None;
            self.scrolled
                .set_policy(PolicyType::Never, PolicyType::Automatic);
            self.albums_box.set_vexpand(false);
            self.albums_box.set_spacing(18);
            let empty_lbl = Label::new(Some(
                "No albums added yet. Click above to add a photo folder!",
            ));
            empty_lbl.add_css_class("dim-label");
            empty_lbl.set_margin_top(40);
            self.albums_box.append(&empty_lbl);
            return;
        }

        // Clean up expanded_album if it's no longer in album_dirs
        let expanded = self.state.borrow().expanded_album.clone();
        if expanded.is_some_and(|exp| !album_dirs.contains(&exp)) {
            self.state.borrow_mut().expanded_album = None;
        }

        let mut rows = Vec::new();
        for dir in album_dirs {
            let images = Rc::new(scan_directory_images(&dir));
            let image_count = images.len();

            let section = GtkBox::new(Orientation::Vertical, 8);
            section.add_css_class("album-section");

            // Header row
            let header = GtkBox::new(Orientation::Horizontal, 10);
            header.add_css_class("album-header");

            let folder_icon = Image::from_icon_name("folder-pictures-symbolic");
            folder_icon.add_css_class("album-folder-icon");
            header.append(&folder_icon);

            let folder_name = dir.file_name().and_then(|f| f.to_str()).unwrap_or("Album");
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
                let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                for raw_file in pending_raw {
                    let done = done.clone();
                    let dir = dir.clone();
                    cache_clone.prefetch_thumbnail(raw_file.clone(), move |result| {
                        trigger_home_thumb_redraw(raw_file, result.is_ok());
                        let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        update_raw_extraction_progress(dir, count, total);
                    });
                }
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
                    let cb = state_open.borrow().on_open_image.clone();
                    if let Some(cb) = cb {
                        cb(images_all.as_ref().clone(), 0);
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
            revealer.set_transition_duration(0);
            revealer.set_reveal_child(true);
            revealer.add_css_class("album-revealer");

            let content_stack = Stack::new();
            content_stack.set_transition_type(StackTransitionType::None);
            content_stack.set_transition_duration(0);
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
                    let card = Self::create_thumbnail_card(
                        &self.state,
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

        // Build expanded grids only when opened; eager construction duplicates every
        // card and its image list even for albums that are never expanded.
    }

    fn create_thumbnail_card(
        state: &Rc<RefCell<HomeScreenState>>,
        img_path: &Path,
        all_images: &Rc<Vec<PathBuf>>,
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

        let state_card = state.clone();
        let path_clone = img_path.to_path_buf();
        state
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
            if !s.surfaces.contains_key(&path_clone)
                && let Some(dec) = s.cache.get_thumbnail(&path_clone)
                && let Ok(surf) = rgba_to_cairo_surface(&dec.rgba)
            {
                s.surfaces.insert(path_clone.clone(), surf);
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

                s.cache
                    .request_thumbnail(path_async.clone(), move |result| {
                        // Keep failed paths in `loading` until refresh to avoid retries
                        // on every hover/redraw of a corrupt or unsupported image.
                        trigger_home_thumb_redraw(path_async, result.is_ok());
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
        let filename = img_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        let lbl = Label::new(Some(filename));
        lbl.add_css_class("album-card-label");
        lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        lbl.set_max_width_chars(22);
        lbl.set_halign(gtk4::Align::Center);
        card.append(&lbl);

        // Click gesture on card
        let click = GestureClick::new();
        let images_for_click = all_images.clone();
        let state_click = state.clone();
        click.connect_pressed(move |_, _, _, _| {
            let cb_opt = state_click.borrow().on_open_image.clone();
            if let Some(cb) = cb_opt {
                cb(images_for_click.as_ref().clone(), idx);
            }
        });
        card.add_controller(click);

        card
    }

    pub fn invalidate(&self, path: &Path) {
        let mut s = self.state.borrow_mut();
        s.surfaces.remove(path);
        s.loading.remove(path);
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
        let test_dir = std::env::temp_dir().join(format!("omaview-expand-{}", std::process::id()));
        // Use an isolated album, independent of the user's saved album list.
        while let Some(child) = home.albums_box.first_child() {
            home.albums_box.remove(&child);
        }
        let section = GtkBox::new(Orientation::Vertical, 8);
        let header = GtkBox::new(Orientation::Horizontal, 10);
        let revealer = Revealer::new();
        revealer.set_transition_type(RevealerTransitionType::SlideDown);
        let stack = Stack::new();
        stack.set_transition_type(StackTransitionType::None);
        stack.set_interpolate_size(false);
        stack.add_named(&GtkBox::new(Orientation::Horizontal, 0), Some("strip"));
        revealer.set_child(Some(&stack));
        section.append(&header);
        section.append(&revealer);
        home.albums_box.append(&section);
        let images = Rc::new(
            (0..80)
                .map(|idx| test_dir.join(format!("photo{idx}.png")))
                .collect(),
        );
        let row = AlbumRow {
            dir: test_dir.clone(),
            images,
            section,
            header,
            progress_label: Label::new(None),
            btn_expand: Button::new(),
            revealer,
            content_stack: stack.clone(),
            grid_initialized: Rc::new(RefCell::new(false)),
        };
        let neighbour_section = GtkBox::new(Orientation::Vertical, 8);
        let neighbour_header = GtkBox::new(Orientation::Horizontal, 0);
        neighbour_header.append(&Label::new(Some("Neighbour album")));
        let neighbour_revealer = Revealer::new();
        neighbour_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        let neighbour_stack = Stack::new();
        let neighbour_strip = GtkBox::new(Orientation::Horizontal, 0);
        neighbour_strip.set_height_request(162);
        neighbour_stack.add_named(&neighbour_strip, Some("strip"));
        neighbour_revealer.set_child(Some(&neighbour_stack));
        neighbour_section.append(&neighbour_header);
        neighbour_section.append(&neighbour_revealer);
        home.albums_box.append(&neighbour_section);
        let neighbour = AlbumRow {
            dir: test_dir.join("neighbour"),
            images: Rc::new(Vec::new()),
            section: neighbour_section,
            header: neighbour_header,
            progress_label: Label::new(None),
            btn_expand: Button::new(),
            revealer: neighbour_revealer,
            content_stack: neighbour_stack,
            grid_initialized: Rc::new(RefCell::new(false)),
        };
        home.state.borrow_mut().album_rows = vec![row.clone(), neighbour.clone()];
        home.state.borrow_mut().expanded_album = None;
        home.apply_expansion(false);

        // Initially no album is expanded
        assert!(home.state.borrow().expanded_album.is_none());
        assert_eq!(home.scrolled.hscrollbar_policy(), PolicyType::Never);
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Automatic);

        // Expand an album
        home.expand_album(test_dir.clone());

        assert_eq!(home.state.borrow().expanded_album, Some(test_dir.clone()));
        // When expanded, outside scrollbar must be disabled
        assert_eq!(home.scrolled.hscrollbar_policy(), PolicyType::Never);
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Never);
        assert!(home.albums_box.vexpands());

        // Verify FlowBox has Align::Start and does not vexpand so gap is below last row
        let first_section = home.albums_box.first_child().unwrap();
        let header = first_section.first_child().unwrap();
        let revealer = header
            .next_sibling()
            .unwrap()
            .downcast::<Revealer>()
            .unwrap();
        let stack = revealer.child().unwrap().downcast::<Stack>().unwrap();
        let scroll = stack
            .child_by_name("grid")
            .unwrap()
            .downcast::<ScrolledWindow>()
            .unwrap();
        let scroll_child = scroll.child().unwrap();
        let flow = if let Ok(viewport) = scroll_child.clone().downcast::<gtk4::Viewport>() {
            viewport.child().unwrap().downcast::<FlowBox>().unwrap()
        } else {
            scroll_child.downcast::<FlowBox>().unwrap()
        };
        assert_eq!(flow.valign(), gtk4::Align::Start);
        assert!(!flow.vexpands());
        assert!(flow.child_at_index(0).is_some());
        assert!(
            flow.child_at_index(GRID_FIRST_BATCH as i32).is_none(),
            "the click must not synchronously build the entire album"
        );
        let expected_duration = if home.widget().settings().is_gtk_enable_animations() {
            ALBUM_EXPAND_MS
        } else {
            0
        };
        assert_eq!(row.revealer.transition_duration(), expected_duration);

        // Now collapse the album
        home.state.borrow_mut().expanded_album = None;
        home.apply_expansion(false);

        assert!(home.state.borrow().expanded_album.is_none());
        assert_eq!(home.scrolled.vscrollbar_policy(), PolicyType::Automatic);
        assert!(!home.albums_box.vexpands());

        let test_window = Window::builder()
            .title("Omaview expansion regression")
            .build();
        test_window.set_default_size(800, 600);
        test_window.set_child(Some(home.widget()));
        test_window.present();
        fn pump_until(condition: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !condition() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "album expansion timed out"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        pump_until(|| home.widget().is_mapped());
        // Hidden grids pause their work; reopening resumes the same grid.
        assert!(flow.child_at_index(GRID_FIRST_BATCH as i32).is_none());
        let settings = home.widget().settings();
        let animations_enabled = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(false);
        home.expand_album(test_dir.clone());
        assert_eq!(stack.transition_duration(), 0);
        pump_until(|| flow.child_at_index(79).is_some() && neighbour.revealer.height() == 0);
        assert!(
            neighbour.section.height() < 100,
            "other albums must occupy only a compact header row"
        );
        assert!(row.section.height() > neighbour.section.height() * 2);
        assert!(flow.child_at_index(80).is_none());
        for idx in 0..80 {
            let card = flow.child_at_index(idx).unwrap().child().unwrap();
            let label = card.last_child().unwrap().downcast::<Label>().unwrap();
            assert_eq!(label.text(), format!("photo{idx}.png"));
        }
        let opened = Rc::new(RefCell::new(None));
        let opened_cb = opened.clone();
        home.connect_open_image(move |paths, idx| *opened_cb.borrow_mut() = Some((paths, idx)));
        let last_card = flow.child_at_index(79).unwrap().child().unwrap();
        let controllers = last_card.observe_controllers();
        let click = (0..controllers.n_items())
            .find_map(|idx| controllers.item(idx)?.downcast::<GestureClick>().ok())
            .unwrap();
        click.emit_by_name::<()>("pressed", &[&1i32, &0.0f64, &0.0f64]);
        assert_eq!(*opened.borrow(), Some((row.images.as_ref().clone(), 79)));
        home.state.borrow_mut().expanded_album = None;
        home.apply_expansion(false);
        pump_until(|| neighbour.revealer.height() >= 162);
        settings.set_gtk_enable_animations(true);
        let heights = Rc::new(RefCell::new(Vec::new()));
        let heights_tick = heights.clone();
        let revealer_weak = neighbour.revealer.downgrade();
        neighbour.header.add_tick_callback(move |_, _| {
            let Some(revealer) = revealer_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let height = revealer.height();
            heights_tick.borrow_mut().push(height);
            if height == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        home.expand_album(test_dir.clone());
        assert_eq!(row.revealer.transition_duration(), ALBUM_EXPAND_MS);
        assert_eq!(stack.transition_duration(), 0);
        pump_until(|| !neighbour.revealer.is_child_revealed() && neighbour.revealer.height() == 0);
        assert!(
            heights
                .borrow()
                .iter()
                .filter(|&&h| h > 0 && h < 162)
                .count()
                >= 2,
            "collapse should pass through intermediate heights instead of snapping"
        );
        // Interrupt and reverse a transition; neighbours must regain their strip height.
        home.state.borrow_mut().expanded_album = None;
        home.apply_expansion(true);
        home.expand_album(test_dir.clone());
        home.state.borrow_mut().expanded_album = None;
        home.apply_expansion(true);
        pump_until(|| neighbour.revealer.is_child_revealed() && neighbour.revealer.height() >= 162);
        assert!(
            flow.child_at_index(80).is_none(),
            "reopening must not duplicate cards"
        );
        settings.set_gtk_enable_animations(animations_enabled);
        test_window.close();

        // Test thumbnail card badge overlay logic
        let temp_dir =
            std::env::temp_dir().join(format!("omaview_test_badges_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Regular non-raw image
        let png_path = temp_dir.join("image.png");
        let _ = std::fs::write(&png_path, b"png");
        let card_png = HomeScreen::create_thumbnail_card(
            &home.state,
            &png_path,
            &Rc::new(vec![png_path.clone()]),
            0,
            0.5,
            0.5,
            0.5,
        );
        let first_child_png = card_png.first_child().unwrap();
        // Regular card directly holds DrawingArea
        assert!(first_child_png.downcast::<DrawingArea>().is_ok());

        // 2. Standalone RAW image
        let raw_path = temp_dir.join("photo.arw");
        let _ = std::fs::write(&raw_path, b"raw");
        let card_raw = HomeScreen::create_thumbnail_card(
            &home.state,
            &raw_path,
            &Rc::new(vec![raw_path.clone()]),
            0,
            0.5,
            0.5,
            0.5,
        );
        let first_child_raw = card_raw.first_child().unwrap();
        // RAW card wraps DrawingArea in an Overlay with badge
        let overlay = first_child_raw
            .downcast::<Overlay>()
            .expect("RAW image should be wrapped in Overlay");
        let mut found_arw_badge = false;
        let mut child = overlay.first_child();
        while let Some(w) = child {
            if let Ok(lbl) = w.clone().downcast::<Label>()
                && lbl.text() == "ARW"
                && lbl.has_css_class("raw-badge")
            {
                found_arw_badge = true;
            }
            child = w.next_sibling();
        }
        assert!(found_arw_badge, "RAW card must display ARW format badge");

        // 3. RAW with companion JPEG
        let cr3_path = temp_dir.join("photo2.cr3");
        let jpg_path = temp_dir.join("photo2.jpg");
        let _ = std::fs::write(&cr3_path, b"raw");
        let _ = std::fs::write(&jpg_path, b"jpg");
        let card_companion = HomeScreen::create_thumbnail_card(
            &home.state,
            &cr3_path,
            &Rc::new(vec![cr3_path.clone()]),
            0,
            0.5,
            0.5,
            0.5,
        );
        let first_child_comp = card_companion.first_child().unwrap();
        let overlay_comp = first_child_comp
            .downcast::<Overlay>()
            .expect("RAW+JPG should be wrapped in Overlay");
        let mut found_companion_badge = false;
        let mut child_comp = overlay_comp.first_child();
        while let Some(w) = child_comp {
            if let Ok(lbl) = w.clone().downcast::<Label>()
                && lbl.text() == "RAW+JPG"
                && lbl.has_css_class("raw-badge-companion")
            {
                found_companion_badge = true;
            }
            child_comp = w.next_sibling();
        }
        assert!(
            found_companion_badge,
            "RAW+JPG card must display RAW+JPG badge with companion class"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        crate::window::verify_gui_regressions();
    }
}
