use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, GestureClick, EventControllerScroll, EventControllerScrollFlags, Orientation};

use crate::image_loader::{ImageCache, load_dynamic_image, generate_thumbnail, rgba_to_cairo_surface};
use crate::theme::OmarchyColors;

const FILMSTRIP_WIDTH: i32 = 148;
const SLOT_HEIGHT: f64 = 96.0;
const THUMB_MAX_WIDTH: f64 = 114.0;
const THUMB_MAX_HEIGHT: f64 = 70.0;
const CORNER_RADIUS: f64 = 8.0;

thread_local! {
    static REDRAW_NOTIFIER: RefCell<Option<glib::WeakRef<DrawingArea>>> = const { RefCell::new(None) };
}

fn trigger_filmstrip_redraw() {
    glib::idle_add_once(|| {
        REDRAW_NOTIFIER.with(|cell| {
            if let Some(weak) = cell.borrow().as_ref() {
                if let Some(a) = weak.upgrade() {
                    a.queue_draw();
                }
            }
        });
    });
}

pub struct FilmstripState {
    pub paths: Vec<PathBuf>,
    pub active_index: usize,
    pub cache: ImageCache,
    pub surfaces: HashMap<PathBuf, cairo::ImageSurface>,
    pub colors: OmarchyColors,
}

#[derive(Clone)]
pub struct Filmstrip {
    container: GtkBox,
    area: DrawingArea,
    #[allow(dead_code)]
    btn_up: Button,
    #[allow(dead_code)]
    btn_down: Button,
    state: Rc<RefCell<FilmstripState>>,
    on_select: Rc<RefCell<Option<Box<dyn Fn(usize) + 'static>>>>,
}

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

impl Filmstrip {
    pub fn new(cache: ImageCache, colors: OmarchyColors) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_width_request(FILMSTRIP_WIDTH);
        container.add_css_class("filmstrip-panel");
        container.set_vexpand(true);
        container.set_hexpand(false);

        let btn_up = Button::from_icon_name("go-up-symbolic");
        btn_up.add_css_class("filmstrip-scroll-btn");
        btn_up.set_has_frame(false);
        container.append(&btn_up);

        let area = DrawingArea::new();
        area.set_vexpand(true);
        area.set_hexpand(true);
        container.append(&area);

        let btn_down = Button::from_icon_name("go-down-symbolic");
        btn_down.add_css_class("filmstrip-scroll-btn");
        btn_down.set_has_frame(false);
        container.append(&btn_down);

        REDRAW_NOTIFIER.with(|cell| {
            *cell.borrow_mut() = Some(area.downgrade());
        });

        let on_select = Rc::new(RefCell::new(None::<Box<dyn Fn(usize) + 'static>>));
        let state = Rc::new(RefCell::new(FilmstripState {
            paths: Vec::new(),
            active_index: 0,
            cache,
            surfaces: HashMap::new(),
            colors,
        }));

        // Draw function
        let state_clone = state.clone();
        area.set_draw_func(move |_, cr, width, height| {
            let mut s = state_clone.borrow_mut();
            let total = s.paths.len();
            if total == 0 {
                return;
            }

            let container_h = height as f64;
            let container_w = width as f64;
            let h = SLOT_HEIGHT;
            let i = s.active_index as f64;

            // Centering algorithm: offset_y = H / 2 - (i * h + h / 2)
            let offset_y = (container_h / 2.0) - (i * h + (h / 2.0));

            let accent_hex = s.colors.accent.as_deref().unwrap_or("#7aa2f7");
            let (ar, ag, ab) = crate::theme::parse_hex_color(accent_hex).unwrap_or((0.48, 0.64, 0.97));

            let bg_hex = s.colors.background.as_deref().unwrap_or("#1a1b26");
            let (bgr, bgg, bgb) = crate::theme::parse_hex_color(bg_hex).unwrap_or((0.10, 0.11, 0.15));

            let paths_to_draw = s.paths.clone();

            for (idx, path) in paths_to_draw.iter().enumerate() {
                let slot_top = offset_y + (idx as f64) * h;
                let slot_bottom = slot_top + h;

                if slot_bottom < 0.0 || slot_top > container_h {
                    continue;
                }

                let slot_center_x = container_w / 2.0 + 3.0;
                let slot_center_y = slot_top + (h - 14.0) / 2.0;

                let thumb_box_x = slot_center_x - THUMB_MAX_WIDTH / 2.0;
                let thumb_box_y = slot_center_y - THUMB_MAX_HEIGHT / 2.0;

                let is_active = idx == s.active_index;

                if !s.surfaces.contains_key(path) {
                    if let Some(dec) = s.cache.get_thumbnail(path) {
                        if let Ok(surf) = rgba_to_cairo_surface(&dec.rgba) {
                            s.surfaces.insert(path.clone(), surf);
                        }
                    }
                }

                if let Some(surface) = s.surfaces.get(path) {
                    let tw = surface.width() as f64;
                    let th = surface.height() as f64;
                    let scale = (THUMB_MAX_WIDTH / tw).min(THUMB_MAX_HEIGHT / th);
                    let dw = tw * scale;
                    let dh = th * scale;
                    let dx = slot_center_x - dw / 2.0;
                    let dy = slot_center_y - dh / 2.0;

                    cr.save().ok();
                    draw_rounded_rect(cr, dx, dy, dw, dh, CORNER_RADIUS);
                    cr.clip();
                    cr.scale(scale, scale);
                    let _ = cr.set_source_surface(surface, dx / scale, dy / scale);
                    let _ = cr.paint();
                    cr.restore().ok();

                    // Borders and indicators
                    if is_active {
                        // Glowing accent border around active thumbnail
                        cr.save().ok();
                        draw_rounded_rect(cr, dx - 1.5, dy - 1.5, dw + 3.0, dh + 3.0, CORNER_RADIUS + 1.0);
                        cr.set_source_rgba(ar, ag, ab, 0.35);
                        cr.set_line_width(4.5);
                        let _ = cr.stroke();

                        draw_rounded_rect(cr, dx - 1.0, dy - 1.0, dw + 2.0, dh + 2.0, CORNER_RADIUS + 0.5);
                        cr.set_source_rgba(ar, ag, ab, 1.0);
                        cr.set_line_width(2.2);
                        let _ = cr.stroke();
                        cr.restore().ok();

                        // Vertical indicator pill on left edge (matches sample_ui.jpeg)
                        cr.save().ok();
                        draw_rounded_rect(cr, 3.5, slot_center_y - 14.0, 3.5, 28.0, 2.0);
                        cr.set_source_rgba(ar, ag, ab, 1.0);
                        let _ = cr.fill();
                        cr.restore().ok();

                        // Index number below thumbnail (matches sample_ui.jpeg)
                        cr.save().ok();
                        cr.select_font_face("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
                        cr.set_font_size(11.0);
                        cr.set_source_rgba(ar, ag, ab, 1.0);
                        let idx_str = format!("{}", idx + 1);
                        if let Ok(ext) = cr.text_extents(&idx_str) {
                            cr.move_to(slot_center_x - ext.width() / 2.0, dy + dh + 13.0);
                            let _ = cr.show_text(&idx_str);
                        }
                        cr.restore().ok();
                    } else {
                        cr.save().ok();
                        draw_rounded_rect(cr, dx, dy, dw, dh, CORNER_RADIUS);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                        cr.restore().ok();
                    }
                } else {
                    // Placeholder box
                    cr.save().ok();
                    draw_rounded_rect(cr, thumb_box_x, thumb_box_y, THUMB_MAX_WIDTH, THUMB_MAX_HEIGHT, CORNER_RADIUS);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.05);
                    let _ = cr.fill();

                    if is_active {
                        draw_rounded_rect(cr, thumb_box_x, thumb_box_y, THUMB_MAX_WIDTH, THUMB_MAX_HEIGHT, CORNER_RADIUS);
                        cr.set_source_rgba(ar, ag, ab, 1.0);
                        cr.set_line_width(2.5);
                        let _ = cr.stroke();
                    }
                    cr.restore().ok();

                    // Async load
                    let path_buf = path.clone();
                    let cache_clone = s.cache.clone();

                    std::thread::spawn(move || {
                        if let Ok(img) = load_dynamic_image(&path_buf) {
                            let thumb = generate_thumbnail(&img, 180);
                            cache_clone.put_thumbnail(path_buf, Arc::new(thumb));
                            trigger_filmstrip_redraw();
                        }
                    });
                }
            }

            // Soft alpha fade toward top and bottom edges
            let grad_top = cairo::LinearGradient::new(0.0, 0.0, 0.0, container_h * 0.18);
            grad_top.add_color_stop_rgba(0.0, bgr, bgg, bgb, 0.95);
            grad_top.add_color_stop_rgba(0.5, bgr, bgg, bgb, 0.55);
            grad_top.add_color_stop_rgba(1.0, bgr, bgg, bgb, 0.0);
            cr.set_source(&grad_top).ok();
            cr.rectangle(0.0, 0.0, container_w, container_h * 0.18);
            let _ = cr.fill();

            let grad_bottom = cairo::LinearGradient::new(0.0, container_h * 0.82, 0.0, container_h);
            grad_bottom.add_color_stop_rgba(0.0, bgr, bgg, bgb, 0.0);
            grad_bottom.add_color_stop_rgba(0.5, bgr, bgg, bgb, 0.55);
            grad_bottom.add_color_stop_rgba(1.0, bgr, bgg, bgb, 0.95);
            cr.set_source(&grad_bottom).ok();
            cr.rectangle(0.0, container_h * 0.82, container_w, container_h * 0.18);
            let _ = cr.fill();
        });

        // Click Gesture
        let click_gesture = GestureClick::new();
        let state_click = state.clone();
        let on_select_click = on_select.clone();
        let area_weak_click = area.downgrade();
        click_gesture.connect_pressed(move |_, _, _, y| {
            let target_opt = {
                let s = match state_click.try_borrow() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let total = s.paths.len();
                if total == 0 {
                    return;
                }

                let height = if let Some(a) = area_weak_click.upgrade() {
                    a.height() as f64
                } else {
                    return;
                };

                let h = SLOT_HEIGHT;
                let i = s.active_index as f64;
                let offset_y = (height / 2.0) - (i * h + (h / 2.0));

                let relative_y = y - offset_y;
                let clicked_idx = (relative_y / h).floor() as isize;

                if clicked_idx >= 0 && (clicked_idx as usize) < total {
                    let target = clicked_idx as usize;
                    if target != s.active_index {
                        Some(target)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(target) = target_opt {
                if let Ok(mut s) = state_click.try_borrow_mut() {
                    s.active_index = target;
                }
                if let Some(ref cb) = *on_select_click.borrow() {
                    cb(target);
                }
                if let Some(a) = area_weak_click.upgrade() {
                    a.queue_draw();
                }
            }
        });
        area.add_controller(click_gesture);

        // Scroll Gesture
        let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        let state_scroll = state.clone();
        let on_select_scroll = on_select.clone();
        let area_weak_scroll = area.downgrade();
        scroll_controller.connect_scroll(move |_, _, dy| {
            let target_opt = {
                let s = match state_scroll.try_borrow() {
                    Ok(s) => s,
                    Err(_) => return glib::Propagation::Stop,
                };
                let total = s.paths.len();
                if total == 0 {
                    None
                } else if dy > 0.0 {
                    if s.active_index + 1 < total {
                        Some(s.active_index + 1)
                    } else {
                        None
                    }
                } else if dy < 0.0 {
                    if s.active_index > 0 {
                        Some(s.active_index - 1)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(target) = target_opt {
                if let Ok(mut s) = state_scroll.try_borrow_mut() {
                    s.active_index = target;
                }
                if let Some(ref cb) = *on_select_scroll.borrow() {
                    cb(target);
                }
                if let Some(a) = area_weak_scroll.upgrade() {
                    a.queue_draw();
                }
            }

            glib::Propagation::Stop
        });
        area.add_controller(scroll_controller);

        // Up/Down button clicks
        let state_up = state.clone();
        let on_select_up = on_select.clone();
        let area_weak_up = area.downgrade();
        btn_up.connect_clicked(move |_| {
            let target_opt = {
                let s = match state_up.try_borrow() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                if s.active_index > 0 {
                    Some(s.active_index - 1)
                } else {
                    None
                }
            };

            if let Some(target) = target_opt {
                if let Ok(mut s) = state_up.try_borrow_mut() {
                    s.active_index = target;
                }
                if let Some(ref cb) = *on_select_up.borrow() {
                    cb(target);
                }
                if let Some(a) = area_weak_up.upgrade() {
                    a.queue_draw();
                }
            }
        });

        let state_down = state.clone();
        let on_select_down = on_select.clone();
        let area_weak_down = area.downgrade();
        btn_down.connect_clicked(move |_| {
            let target_opt = {
                let s = match state_down.try_borrow() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let total = s.paths.len();
                if total > 0 && s.active_index + 1 < total {
                    Some(s.active_index + 1)
                } else {
                    None
                }
            };

            if let Some(target) = target_opt {
                if let Ok(mut s) = state_down.try_borrow_mut() {
                    s.active_index = target;
                }
                if let Some(ref cb) = *on_select_down.borrow() {
                    cb(target);
                }
                if let Some(a) = area_weak_down.upgrade() {
                    a.queue_draw();
                }
            }
        });

        Self {
            container,
            area,
            btn_up,
            btn_down,
            state,
            on_select,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_paths(&self, paths: Vec<PathBuf>, active_index: usize) {
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.paths = paths;
            s.active_index = active_index;
            s.surfaces.clear();
        }
        self.area.queue_draw();
    }

    pub fn set_active_index(&self, index: usize) {
        if let Ok(mut s) = self.state.try_borrow_mut() {
            if s.active_index != index {
                s.active_index = index;
                self.area.queue_draw();
            }
        }
    }

    pub fn set_colors(&self, colors: OmarchyColors) {
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.colors = colors;
        }
        self.area.queue_draw();
    }

    pub fn connect_select<F: Fn(usize) + 'static>(&self, callback: F) {
        *self.on_select.borrow_mut() = Some(std::boxed::Box::new(callback));
    }
}
