use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use gtk4::prelude::*;
use gtk4::{Box, Button, EventControllerMotion, Label, Orientation, Overlay, Stack, StackTransitionType};
use libadwaita::prelude::*;
use libadwaita::ApplicationWindow;

use crate::crop_bar::CropBar;
use crate::filmstrip::Filmstrip;
use crate::home::HomeScreen;
use crate::image_loader::{ImageCache, DecodedImage, load_decoded_image};
use crate::image_ops::{apply_all_edits, save_image};
use crate::metadata::{MetadataPill, create_exif_popover};
use crate::shortcuts::{AppAction, create_key_controller, show_shortcuts_dialog};
use crate::theme::ThemeManager;
use crate::toolbar::BottomToolbar;
use crate::util::{trash_file, is_supported_image};
use crate::viewport::{Viewport, CropRatio};

thread_local! {
    static ACTIVE_WINDOW: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
}

pub struct WindowState {
    pub paths: Vec<PathBuf>,
    pub current_index: usize,
    pub zen_mode: bool,
    pub filmstrip_visible: bool,
    pub idle_hide_pill_source: Option<glib::SourceId>,
}

#[derive(Clone)]
pub struct MainWindow {
    window: ApplicationWindow,
    stack: Stack,
    home_screen: HomeScreen,
    viewport: Viewport,
    crop_bar: CropBar,
    filmstrip: Filmstrip,
    toolbar: BottomToolbar,
    metadata_pill: MetadataPill,
    btn_home: Button,
    btn_top_save: Button,
    top_left_box: Box,
    btn_nav_left: Button,
    btn_nav_right: Button,
    cache: ImageCache,
    _theme_manager: Rc<ThemeManager>,
    state: Rc<RefCell<WindowState>>,
}

impl MainWindow {
    pub fn new(app: &libadwaita::Application, theme_manager: Rc<ThemeManager>) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Omaview")
            .default_width(1120)
            .default_height(760)
            .build();

        window.add_css_class("omaview-window");

        let cache = ImageCache::new(10, 150);
        let colors = theme_manager.get_colors();

        let home_screen = HomeScreen::new(cache.clone(), colors.clone());
        let viewport = Viewport::new();
        let crop_bar = CropBar::new();
        let filmstrip = Filmstrip::new(cache.clone(), colors);
        let toolbar = BottomToolbar::new();
        let metadata_pill = MetadataPill::new();

        let filmstrip_theme = filmstrip.clone();
        let home_theme = home_screen.clone();
        theme_manager.on_theme_changed(move |new_colors| {
            filmstrip_theme.set_colors(new_colors.clone());
            home_theme.set_colors(new_colors.clone());
        });

        // Top-left box holding Home Button, Metadata Pill, and Save Button
        let top_left_box = Box::new(Orientation::Horizontal, 8);
        top_left_box.add_css_class("top-bar-box");
        top_left_box.set_halign(gtk4::Align::Start);
        top_left_box.set_valign(gtk4::Align::Start);
        top_left_box.set_margin_top(16);
        top_left_box.set_margin_start(16);

        let btn_home = Button::from_icon_name("go-home-symbolic");
        btn_home.add_css_class("nav-home-btn");
        btn_home.set_has_frame(false);
        btn_home.set_valign(gtk4::Align::Center);
        btn_home.set_tooltip_text(Some("Albums / Home (Home / Esc)"));

        let btn_top_save = Button::new();
        btn_top_save.set_has_frame(false);
        btn_top_save.add_css_class("save-pill-btn");
        btn_top_save.set_halign(gtk4::Align::End);
        btn_top_save.set_valign(gtk4::Align::Start);
        btn_top_save.set_margin_top(16);
        btn_top_save.set_margin_end(16);
        btn_top_save.set_tooltip_text(Some("Save Changes (Ctrl+S / s)"));
        let save_lbl = Label::new(Some("Save"));
        btn_top_save.set_child(Some(&save_lbl));
        btn_top_save.set_visible(false);

        top_left_box.append(&btn_home);
        top_left_box.append(metadata_pill.widget());

        // Floating Nav Chevrons (matches sample_ui.jpeg)
        let btn_nav_left = Button::from_icon_name("go-previous-symbolic");
        btn_nav_left.add_css_class("nav-chevron-btn");
        btn_nav_left.set_has_frame(false);
        btn_nav_left.set_halign(gtk4::Align::Start);
        btn_nav_left.set_valign(gtk4::Align::Center);
        btn_nav_left.set_margin_start(16);

        let btn_nav_right = Button::from_icon_name("go-next-symbolic");
        btn_nav_right.add_css_class("nav-chevron-btn");
        btn_nav_right.set_has_frame(false);
        btn_nav_right.set_halign(gtk4::Align::End);
        btn_nav_right.set_valign(gtk4::Align::Center);
        btn_nav_right.set_margin_end(16);

        let state = Rc::new(RefCell::new(WindowState {
            paths: Vec::new(),
            current_index: 0,
            zen_mode: false,
            filmstrip_visible: true,
            idle_hide_pill_source: None,
        }));

        // Layout:
        // Viewport Overlay covers the image area with pills & chevrons
        let viewport_overlay = Overlay::new();
        viewport_overlay.set_hexpand(true);
        viewport_overlay.set_vexpand(true);
        viewport_overlay.set_child(Some(viewport.widget()));

        viewport_overlay.add_overlay(&top_left_box);
        viewport_overlay.add_overlay(&btn_top_save);
        viewport_overlay.add_overlay(crop_bar.widget());
        viewport_overlay.add_overlay(&btn_nav_left);
        viewport_overlay.add_overlay(&btn_nav_right);
        viewport_overlay.add_overlay(toolbar.widget());

        let main_h_box = Box::new(Orientation::Horizontal, 0);
        main_h_box.add_css_class("omaview-main-box");
        main_h_box.append(&viewport_overlay);
        main_h_box.append(filmstrip.widget());

        // Stack with Home Screen and Image Viewer
        let stack = Stack::new();
        stack.set_transition_type(StackTransitionType::Crossfade);
        stack.set_transition_duration(180);
        stack.add_named(home_screen.widget(), Some("home"));
        stack.add_named(&main_h_box, Some("viewer"));

        window.set_content(Some(&stack));

        let win = Self {
            window,
            stack,
            home_screen,
            viewport,
            crop_bar,
            filmstrip,
            toolbar,
            metadata_pill,
            btn_home,
            btn_top_save,
            top_left_box,
            btn_nav_left,
            btn_nav_right,
            cache,
            _theme_manager: theme_manager,
            state,
        };

        win.setup_controllers(&viewport_overlay);
        win.setup_signals();

        ACTIVE_WINDOW.with(|cell| {
            *cell.borrow_mut() = Some(win.clone());
        });

        win
    }

    pub fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    fn setup_controllers(&self, overlay: &Overlay) {
        // Keyboard Controller
        let win_self = self.clone();
        let key_controller = create_key_controller(move |action| {
            win_self.handle_action(action)
        });
        self.window.add_controller(key_controller);

        // Mouse Motion Controller for top pill auto-hide
        let top_left = self.top_left_box.clone();
        let nav_l = self.btn_nav_left.clone();
        let nav_r = self.btn_nav_right.clone();
        let state_motion = self.state.clone();
        let motion_controller = EventControllerMotion::new();

        motion_controller.connect_motion(move |_, _, _| {
            let mut s = state_motion.borrow_mut();
            if s.zen_mode {
                return;
            }

            top_left.set_visible(true);
            nav_l.set_visible(true);
            nav_r.set_visible(true);

            if let Some(src) = s.idle_hide_pill_source.take() {
                src.remove();
            }

            let top_left_clone = top_left.clone();
            let nav_l_clone = nav_l.clone();
            let nav_r_clone = nav_r.clone();
            let state_timer = state_motion.clone();
            let source_id = glib::timeout_add_local_once(Duration::from_millis(3500), move || {
                let mut st = state_timer.borrow_mut();
                st.idle_hide_pill_source = None;
                if !st.zen_mode {
                    top_left_clone.set_visible(false);
                    nav_l_clone.set_visible(false);
                    nav_r_clone.set_visible(false);
                }
            });

            s.idle_hide_pill_source = Some(source_id);
        });

        overlay.add_controller(motion_controller);
    }

    fn setup_signals(&self) {
        // Home button click
        let win_home = self.clone();
        self.btn_home.connect_clicked(move |_| {
            win_home.show_home();
        });

        // Home screen open album image
        let win_h_open = self.clone();
        self.home_screen.connect_open_image(move |paths, idx| {
            win_h_open.load_initial_paths(paths, idx);
        });

        // Filmstrip click selection
        let win_f = self.clone();
        self.filmstrip.connect_select(move |idx| {
            win_f.go_to_index(idx);
        });

        // Viewport drop files
        let win_d = self.clone();
        self.viewport.connect_drop_files(move |paths| {
            win_d.handle_dropped_paths(paths);
        });

        // Viewport zoom changed
        let win_z = self.clone();
        self.viewport.connect_zoom_changed(move |pct| {
            win_z.update_metadata_pill(pct);
        });

        // Viewport crop applied
        let win_c = self.clone();
        self.viewport.connect_crop_applied(move || {
            win_c.toolbar.set_crop_active(false);
            win_c.crop_bar.set_visible(false);
            win_c.refresh_current_metadata();
            win_c.update_save_button();
        });

        // Crop Bar signals
        let win_cb_ratio = self.clone();
        self.crop_bar.connect_ratio_selected(move |ratio| {
            win_cb_ratio.viewport.set_crop_ratio(ratio);
        });

        let win_cb_apply = self.clone();
        self.crop_bar.connect_apply(move || {
            win_cb_apply.handle_action(AppAction::ApplyCrop);
        });

        let win_cb_cancel = self.clone();
        self.crop_bar.connect_cancel(move || {
            win_cb_cancel.handle_action(AppAction::Escape);
        });

        // Save buttons (top pill & bottom toolbar)
        let win_save_top = self.clone();
        self.btn_top_save.connect_clicked(move |_| {
            win_save_top.save_current();
        });

        let win_save_bar = self.clone();
        self.toolbar.connect_save(move || {
            win_save_bar.save_current();
        });

        // Floating Nav Chevrons
        let win_nl = self.clone();
        self.btn_nav_left.connect_clicked(move |_| win_nl.prev_image());

        let win_nr = self.clone();
        self.btn_nav_right.connect_clicked(move |_| win_nr.next_image());

        // Toolbar buttons
        let win_prev = self.clone();
        self.toolbar.connect_prev(move || win_prev.prev_image());

        let win_grid = self.clone();
        self.toolbar.connect_grid_toggle(move || win_grid.toggle_filmstrip());

        let win_zi = self.clone();
        self.toolbar.connect_zoom_in(move || win_zi.viewport.zoom_in());

        let win_z_ind = self.clone();
        self.toolbar.connect_zoom_indicator(move || {
            let is_fit = win_z_ind.viewport.get_zoom_pct() == 100;
            if is_fit {
                win_z_ind.viewport.zoom_fit();
            } else {
                win_z_ind.viewport.zoom_100();
            }
        });

        let win_zo = self.clone();
        self.toolbar.connect_zoom_out(move || win_zo.viewport.zoom_out());

        let win_crop = self.clone();
        self.toolbar.connect_crop(move || {
            let active = win_crop.viewport.toggle_crop();
            win_crop.toolbar.set_crop_active(active);
            win_crop.crop_bar.set_visible(active);
        });

        let win_rcw = self.clone();
        self.toolbar.connect_rotate_cw(move || {
            win_rcw.viewport.rotate_cw();
            win_rcw.refresh_current_metadata();
            win_rcw.update_save_button();
        });

        let win_adj = self.clone();
        self.toolbar.connect_adjustments_changed(move |exp, con, sat, warm| {
            win_adj.viewport.update_adjustments(exp, con, sat, warm);
            win_adj.update_save_button();
        });

        let win_info = self.clone();
        self.toolbar.info_button().connect_clicked(move |_| {
            win_info.show_info_popover();
        });

        let win_trash = self.clone();
        self.toolbar.connect_trash(move || win_trash.trash_current());
    }

    pub fn handle_action(&self, action: AppAction) -> glib::Propagation {
        match action {
            AppAction::Home => self.show_home(),
            AppAction::NextImage => self.next_image(),
            AppAction::PrevImage => self.prev_image(),
            AppAction::ZoomIn => self.viewport.zoom_in(),
            AppAction::ZoomOut => self.viewport.zoom_out(),
            AppAction::Zoom100 => {
                if self.viewport.is_in_crop_mode() {
                    self.crop_bar.set_active_ratio(CropRatio::Freeform);
                    self.viewport.set_crop_ratio(CropRatio::Freeform);
                } else {
                    self.viewport.zoom_100();
                }
            }
            AppAction::ZoomFit => self.viewport.zoom_fit(),
            AppAction::RotateCw => {
                self.viewport.rotate_cw();
                self.refresh_current_metadata();
                self.update_save_button();
            }
            AppAction::RotateCcw => {
                self.viewport.rotate_ccw();
                self.refresh_current_metadata();
                self.update_save_button();
            }
            AppAction::FlipH => {
                self.viewport.flip_h();
                self.update_save_button();
            }
            AppAction::FlipV => {
                self.viewport.flip_v();
                self.update_save_button();
            }
            AppAction::ToggleCrop => {
                let active = self.viewport.toggle_crop();
                self.toolbar.set_crop_active(active);
                self.crop_bar.set_visible(active);
            }
            AppAction::CropRatioFreeform => {
                if self.viewport.is_in_crop_mode() {
                    self.crop_bar.set_active_ratio(CropRatio::Freeform);
                    self.viewport.set_crop_ratio(CropRatio::Freeform);
                }
            }
            AppAction::CropRatioSquare => {
                if self.viewport.is_in_crop_mode() {
                    self.crop_bar.set_active_ratio(CropRatio::Square);
                    self.viewport.set_crop_ratio(CropRatio::Square);
                }
            }
            AppAction::CropRatio16_9 => {
                if self.viewport.is_in_crop_mode() {
                    self.crop_bar.set_active_ratio(CropRatio::SixteenNine);
                    self.viewport.set_crop_ratio(CropRatio::SixteenNine);
                }
            }
            AppAction::ApplyCrop => {
                if self.viewport.is_in_crop_mode() {
                    self.viewport.apply_crop();
                    self.toolbar.set_crop_active(false);
                    self.crop_bar.set_visible(false);
                    self.refresh_current_metadata();
                    self.update_save_button();
                }
            }
            AppAction::ToggleAdjustments => self.toolbar.toggle_adjustments(),
            AppAction::ToggleInfo => self.show_info_popover(),
            AppAction::Trash => self.trash_current(),
            AppAction::ToggleZen => self.toggle_zen(),
            AppAction::ToggleFilmstrip => self.toggle_filmstrip(),
            AppAction::Save => self.save_current(),
            AppAction::ShowHelp => show_shortcuts_dialog(&self.window),
            AppAction::ToggleFullscreen => {
                if self.window.is_fullscreen() {
                    self.window.unfullscreen();
                } else {
                    self.window.fullscreen();
                }
            }
            AppAction::Escape => {
                if self.stack.visible_child_name().as_deref() == Some("home") {
                    self.window.close();
                } else if self.viewport.is_in_crop_mode() {
                    self.viewport.cancel_crop();
                    self.toolbar.set_crop_active(false);
                    self.crop_bar.set_visible(false);
                } else {
                    self.show_home();
                }
            }
            AppAction::Quit => self.window.close(),
        }
        glib::Propagation::Stop
    }

    pub fn show_home(&self) {
        self.home_screen.refresh();
        self.stack.set_visible_child_name("home");
        self.window.set_title(Some("Omaview - Albums"));
    }

    pub fn show_viewer(&self) {
        self.stack.set_visible_child_name("viewer");
        self.window.set_title(Some("Omaview"));
    }

    pub fn load_initial_paths(&self, paths: Vec<PathBuf>, initial_index: usize) {
        {
            let mut s = self.state.borrow_mut();
            s.paths = paths.clone();
            s.current_index = initial_index;
        }

        self.filmstrip.set_paths(paths, initial_index);
        self.go_to_index(initial_index);
        self.show_viewer();
    }

    pub fn go_to_index(&self, index: usize) {
        let (path, total) = {
            let mut s = self.state.borrow_mut();
            let total = s.paths.len();
            if total == 0 {
                return;
            }
            let idx = index.min(total.saturating_sub(1));
            s.current_index = idx;
            (s.paths[idx].clone(), total)
        };

        if self.viewport.is_in_crop_mode() {
            self.viewport.cancel_crop();
            self.toolbar.set_crop_active(false);
            self.crop_bar.set_visible(false);
        }

        self.filmstrip.set_active_index(index);
        self.toolbar.reset_adjustments_ui();
        self.update_save_button();

        // Check if already in memory cache
        if let Some(loaded) = self.cache.get_image(&path) {
            self.viewport.set_image(loaded.clone());
            self.update_metadata_pill_with(&loaded, index, total);
            self.preload_neighbors(index);
            return;
        }

        // Otherwise load on background thread
        let cache_clone = self.cache.clone();
        let path_clone = path.clone();

        std::thread::spawn(move || {
            if let Ok(loaded) = load_decoded_image(&path_clone) {
                let arc_loaded = Arc::new(loaded);
                cache_clone.put_image(path_clone, arc_loaded);

                glib::idle_add_once(|| {
                    ACTIVE_WINDOW.with(|cell| {
                        if let Some(win) = cell.borrow().as_ref() {
                            win.check_active_image();
                        }
                    });
                });
            }
        });
    }

    pub fn check_active_image(&self) {
        let (path, idx, total) = {
            let s = self.state.borrow();
            if s.paths.is_empty() {
                return;
            }
            (s.paths[s.current_index].clone(), s.current_index, s.paths.len())
        };

        if let Some(loaded) = self.cache.get_image(&path) {
            self.viewport.set_image(loaded.clone());
            self.update_metadata_pill_with(&loaded, idx, total);
            self.preload_neighbors(idx);
            self.update_save_button();
        }
    }

    pub fn next_image(&self) {
        let (curr, total) = {
            let s = self.state.borrow();
            (s.current_index, s.paths.len())
        };
        if total > 0 && curr + 1 < total {
            self.go_to_index(curr + 1);
        }
    }

    pub fn prev_image(&self) {
        let curr = self.state.borrow().current_index;
        if curr > 0 {
            self.go_to_index(curr - 1);
        }
    }

    pub fn toggle_zen(&self) {
        let mut s = self.state.borrow_mut();
        s.zen_mode = !s.zen_mode;
        let zen = s.zen_mode;

        if zen {
            self.top_left_box.set_visible(false);
            self.btn_top_save.set_visible(false);
            self.toolbar.widget().set_visible(false);
            self.btn_nav_left.set_visible(false);
            self.btn_nav_right.set_visible(false);
            self.filmstrip.widget().set_visible(false);
            self.crop_bar.widget().set_visible(false);
        } else {
            self.top_left_box.set_visible(true);
            self.toolbar.widget().set_visible(true);
            self.btn_nav_left.set_visible(true);
            self.btn_nav_right.set_visible(true);
            self.filmstrip.widget().set_visible(s.filmstrip_visible);
            self.crop_bar.set_visible(self.viewport.is_in_crop_mode());
            self.update_save_button();
        }
    }

    pub fn toggle_filmstrip(&self) {
        let mut s = self.state.borrow_mut();
        s.filmstrip_visible = !s.filmstrip_visible;
        let vis = s.filmstrip_visible && !s.zen_mode;
        self.filmstrip.widget().set_visible(vis);
        self.toolbar.set_filmstrip_active(vis);
    }

    pub fn trash_current(&self) {
        let (current_path, curr_idx) = {
            let s = self.state.borrow();
            if s.paths.is_empty() {
                return;
            }
            (s.paths[s.current_index].clone(), s.current_index)
        };

        if let Err(e) = trash_file(&current_path) {
            eprintln!("Error moving file to trash: {}", e);
            return;
        }

        self.cache.remove(&current_path);

        let new_paths = {
            let mut s = self.state.borrow_mut();
            s.paths.retain(|p| p != &current_path);
            s.paths.clone()
        };

        if new_paths.is_empty() {
            let mut s = self.state.borrow_mut();
            s.current_index = 0;
            self.filmstrip.set_paths(Vec::new(), 0);
            self.metadata_pill.update("", 0, 0, 100, 0, 0);
            drop(s);
            self.show_home();
            return;
        }

        let new_idx = if curr_idx >= new_paths.len() {
            new_paths.len() - 1
        } else {
            curr_idx
        };

        self.load_initial_paths(new_paths, new_idx);
    }

    pub fn update_save_button(&self) {
        let has_edits = self.viewport.get_edits().has_any_edits();
        self.toolbar.set_save_visible(has_edits);
        self.btn_top_save.set_visible(has_edits && !self.state.borrow().zen_mode);
    }

    pub fn save_current(&self) {
        let (current_path, edits) = {
            let s = self.state.borrow();
            if s.paths.is_empty() {
                return;
            }
            (s.paths[s.current_index].clone(), self.viewport.get_edits())
        };

        if !edits.has_any_edits() {
            return;
        }

        let loaded = match self.viewport.get_current_image() {
            Some(img) => img,
            None => return,
        };

        let dialog = libadwaita::AlertDialog::new(
            Some("Save Changes?"),
            Some("Do you want to overwrite the original image file or save as a new copy?"),
        );

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("overwrite", "Overwrite");
        dialog.add_response("save_as", "Save As…");
        dialog.set_response_appearance("overwrite", libadwaita::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save_as", libadwaita::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("overwrite"));
        dialog.set_close_response("cancel");

        let win_clone = self.clone();
        let loaded_img = loaded.image.clone();
        let path_to_save = current_path.clone();

        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |response| {
            match response.as_str() {
                "overwrite" => {
                    let processed = apply_all_edits(&loaded_img, &edits);
                    match save_image(&processed, &path_to_save) {
                        Ok(()) => {
                            win_clone.cache.remove(&path_to_save);
                            let current_idx = win_clone.state.borrow().current_index;
                            win_clone.go_to_index(current_idx);
                            win_clone.update_save_button();
                        }
                        Err(e) => {
                            eprintln!("Failed to overwrite image: {}", e);
                        }
                    }
                }
                "save_as" => {
                    let file_dialog = gtk4::FileDialog::new();
                    file_dialog.set_title("Save As…");
                    if let Some(name) = path_to_save.file_name().and_then(|f| f.to_str()) {
                        file_dialog.set_initial_name(Some(name));
                    }
                    if let Some(parent) = path_to_save.parent() {
                        file_dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
                    }

                    let win_save = win_clone.clone();
                    let window_for_dialog = win_save.window.clone();
                    let processed = apply_all_edits(&loaded_img, &edits);

                    file_dialog.save(Some(&window_for_dialog), gio::Cancellable::NONE, move |res| {
                        if let Ok(file) = res {
                            if let Some(dest_path) = file.path() {
                                match save_image(&processed, &dest_path) {
                                    Ok(()) => {
                                        win_save.cache.remove(&dest_path);
                                        let (new_paths, idx) = {
                                            let mut s = win_save.state.borrow_mut();
                                            if let Some(pos) = s.paths.iter().position(|p| p == &dest_path) {
                                                (s.paths.clone(), pos)
                                            } else {
                                                s.paths.push(dest_path.clone());
                                                let p = s.paths.clone();
                                                let idx = p.len() - 1;
                                                (p, idx)
                                            }
                                        };
                                        win_save.load_initial_paths(new_paths, idx);
                                        win_save.update_save_button();
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to save image copy: {}", e);
                                    }
                                }
                            }
                        }
                    });
                }
                _ => {}
            }
        });
    }

    pub fn show_info_popover(&self) {
        let (current_path, loaded) = {
            let s = self.state.borrow();
            if s.paths.is_empty() {
                return;
            }
            (s.paths[s.current_index].clone(), self.viewport.get_current_image())
        };

        if let Some(loaded_img) = loaded {
            let popover = create_exif_popover(
                &current_path,
                loaded_img.width,
                loaded_img.height,
                &loaded_img.file_size_formatted,
            );
            popover.set_parent(self.toolbar.info_button());
            popover.popup();
        }
    }

    fn update_metadata_pill(&self, zoom_pct: u32) {
        let s = self.state.borrow();
        if s.paths.is_empty() {
            self.metadata_pill.update("", 0, 0, zoom_pct, 0, 0);
            return;
        }

        if let Some(loaded) = self.viewport.get_current_image() {
            let filename = loaded.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            self.metadata_pill.update(
                filename,
                loaded.width,
                loaded.height,
                zoom_pct,
                s.current_index,
                s.paths.len(),
            );
        }
    }

    fn update_metadata_pill_with(&self, loaded: &DecodedImage, idx: usize, total: usize) {
        let filename = loaded.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        let zoom_pct = self.viewport.get_zoom_pct();
        self.metadata_pill.update(
            filename,
            loaded.width,
            loaded.height,
            zoom_pct,
            idx,
            total,
        );
    }

    fn refresh_current_metadata(&self) {
        let zoom_pct = self.viewport.get_zoom_pct();
        self.update_metadata_pill(zoom_pct);
    }

    fn preload_neighbors(&self, current_idx: usize) {
        let paths_to_preload = {
            let s = self.state.borrow();
            let mut list = Vec::new();
            if current_idx + 1 < s.paths.len() {
                list.push(s.paths[current_idx + 1].clone());
            }
            if current_idx > 0 {
                list.push(s.paths[current_idx - 1].clone());
            }
            if current_idx + 2 < s.paths.len() {
                list.push(s.paths[current_idx + 2].clone());
            }
            list
        };

        let cache = self.cache.clone();
        std::thread::spawn(move || {
            for path in paths_to_preload {
                if cache.get_image(&path).is_none() {
                    if let Ok(loaded) = load_decoded_image(&path) {
                        cache.put_image(path, Arc::new(loaded));
                    }
                }
            }
        });
    }

    fn handle_dropped_paths(&self, raw_paths: Vec<PathBuf>) {
        let mut valid_images = Vec::new();
        for p in raw_paths {
            if p.is_dir() {
                valid_images.append(&mut crate::util::scan_directory_images(&p));
            } else if p.is_file() && is_supported_image(&p) {
                valid_images.push(p);
            }
        }

        if !valid_images.is_empty() {
            self.load_initial_paths(valid_images, 0);
        }
    }
}
