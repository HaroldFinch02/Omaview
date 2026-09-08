use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use gtk4::prelude::*;
use gtk4::{DrawingArea, GestureClick, GestureDrag, EventControllerScroll, EventControllerScrollFlags, DropTarget};

use crate::image_loader::{DecodedImage, rgba_to_cairo_surface};
use crate::image_ops::{ImageEdits, apply_all_edits};

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum CropRatio {
    Freeform,
    Square,      // 1:1
    SixteenNine, // 16:9
    FourThree,   // 4:3
}

impl CropRatio {
    pub fn ratio(&self) -> Option<f64> {
        match self {
            CropRatio::Freeform => None,
            CropRatio::Square => Some(1.0),
            CropRatio::SixteenNine => Some(16.0 / 9.0),
            CropRatio::FourThree => Some(4.0 / 3.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CropHandle {
    None,
    Inside,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Top,
    Bottom,
    Left,
    Right,
}

pub struct ViewportState {
    pub image: Option<Arc<DecodedImage>>,
    pub rendered_surface: Option<cairo::ImageSurface>,
    pub rendered_width: u32,
    pub rendered_height: u32,

    pub edits: ImageEdits,

    pub zoom: f64,
    pub is_fit: bool,
    pub pan_x: f64,
    pub pan_y: f64,

    // Drag / Pan
    pub drag_start_pan_x: f64,
    pub drag_start_pan_y: f64,

    // Crop Mode
    pub crop_mode: bool,
    pub crop_ratio: CropRatio,
    pub crop_rect: [f64; 4], // [x, y, w, h] normalized 0.0 .. 1.0
    crop_active_handle: CropHandle,
    crop_drag_start_rect: [f64; 4],
}

#[derive(Clone)]
pub struct Viewport {
    area: DrawingArea,
    state: Rc<RefCell<ViewportState>>,
    on_zoom_changed: Rc<RefCell<Option<Box<dyn Fn(u32) + 'static>>>>,
    on_crop_applied: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    on_drop_files: Rc<RefCell<Option<Box<dyn Fn(Vec<PathBuf>) + 'static>>>>,
}

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

impl Viewport {
    pub fn new() -> Self {
        let area = DrawingArea::new();
        area.set_vexpand(true);
        area.set_hexpand(true);
        area.set_focusable(true);

        let state = Rc::new(RefCell::new(ViewportState {
            image: None,
            rendered_surface: None,
            rendered_width: 0,
            rendered_height: 0,
            edits: ImageEdits::default(),
            zoom: 1.0,
            is_fit: true,
            pan_x: 0.0,
            pan_y: 0.0,
            drag_start_pan_x: 0.0,
            drag_start_pan_y: 0.0,
            crop_mode: false,
            crop_ratio: CropRatio::Freeform,
            crop_rect: [0.1, 0.1, 0.8, 0.8],
            crop_active_handle: CropHandle::None,
            crop_drag_start_rect: [0.1, 0.1, 0.8, 0.8],
        }));

        let on_zoom_changed = Rc::new(RefCell::new(None::<Box<dyn Fn(u32) + 'static>>));
        let on_crop_applied = Rc::new(RefCell::new(None::<Box<dyn Fn() + 'static>>));
        let on_drop_files = Rc::new(RefCell::new(None::<Box<dyn Fn(Vec<PathBuf>) + 'static>>));

        // 1. Draw function
        let state_draw = state.clone();
        area.set_draw_func(move |_, cr, width, height| {
            let mut s = state_draw.borrow_mut();
            let surface = match s.rendered_surface.clone() {
                Some(surf) => surf,
                None => {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                    cr.select_font_face("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                    cr.set_font_size(15.0);
                    let text = "Open an image or drag & drop files here";
                    if let Ok(ext) = cr.text_extents(text) {
                        cr.move_to((width as f64 - ext.width()) / 2.0, (height as f64 + ext.height()) / 2.0);
                        let _ = cr.show_text(text);
                    }
                    return;
                }
            };

            let iw = s.rendered_width as f64;
            let ih = s.rendered_height as f64;
            let vw = width as f64;
            let vh = height as f64;

            if s.is_fit {
                let fit_scale = (vw / iw).min(vh / ih);
                s.zoom = if iw <= vw && ih <= vh { 1.0 } else { fit_scale };
                s.pan_x = 0.0;
                s.pan_y = 0.0;
            }

            let dw = iw * s.zoom;
            let dh = ih * s.zoom;
            let origin_x = (vw - dw) / 2.0 + s.pan_x;
            let origin_y = (vh - dh) / 2.0 + s.pan_y;

            // Soft shadow under image for glassmorphic floating effect
            cr.save().ok();
            draw_rounded_rect(cr, origin_x - 1.0, origin_y + 2.0, dw + 2.0, dh + 3.0, 14.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.30);
            let _ = cr.fill();
            cr.restore().ok();

            // Draw image with rounded corners matching sample_ui.jpeg
            cr.save().ok();
            draw_rounded_rect(cr, origin_x, origin_y, dw, dh, 12.0);
            cr.clip();
            cr.translate(origin_x, origin_y);
            cr.scale(s.zoom, s.zoom);

            let pattern = cairo::SurfacePattern::create(&surface);
            pattern.set_filter(cairo::Filter::Bilinear);
            let _ = cr.set_source(&pattern);
            let _ = cr.paint();
            cr.restore().ok();

            // Subtle image border
            cr.save().ok();
            draw_rounded_rect(cr, origin_x, origin_y, dw, dh, 12.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            cr.restore().ok();

            // If Crop Mode is active, draw rule-of-thirds overlay
            if s.crop_mode {
                let [cx, cy, cw, ch] = s.crop_rect;
                let crop_x = origin_x + cx * dw;
                let crop_y = origin_y + cy * dh;
                let crop_w = cw * dw;
                let crop_h = ch * dh;

                // Dimmed outer area
                cr.save().ok();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
                cr.rectangle(0.0, 0.0, vw, crop_y.max(0.0));
                let bottom_y = (crop_y + crop_h).min(vh);
                cr.rectangle(0.0, bottom_y, vw, (vh - bottom_y).max(0.0));
                cr.rectangle(0.0, crop_y.max(0.0), crop_x.max(0.0), crop_h);
                let right_x = (crop_x + crop_w).min(vw);
                cr.rectangle(right_x, crop_y.max(0.0), (vw - right_x).max(0.0), crop_h);
                let _ = cr.fill();
                cr.restore().ok();

                // Crisp border
                cr.save().ok();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                cr.set_line_width(1.5);
                cr.rectangle(crop_x, crop_y, crop_w, crop_h);
                let _ = cr.stroke();

                // Rule-of-thirds dashed lines
                cr.set_dash(&[4.0, 4.0], 0.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
                cr.set_line_width(1.0);

                cr.move_to(crop_x + crop_w / 3.0, crop_y);
                cr.line_to(crop_x + crop_w / 3.0, crop_y + crop_h);
                cr.move_to(crop_x + 2.0 * crop_w / 3.0, crop_y);
                cr.line_to(crop_x + 2.0 * crop_w / 3.0, crop_y + crop_h);

                cr.move_to(crop_x, crop_y + crop_h / 3.0);
                cr.line_to(crop_x + crop_w, crop_y + crop_h / 3.0);
                cr.move_to(crop_x, crop_y + 2.0 * crop_h / 3.0);
                cr.line_to(crop_x + crop_w, crop_y + 2.0 * crop_h / 3.0);
                let _ = cr.stroke();

                // Handles
                let handle_sz = 8.0;
                cr.set_dash(&[], 0.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                let draw_handle = |cr: &cairo::Context, hx: f64, hy: f64| {
                    cr.rectangle(hx - handle_sz / 2.0, hy - handle_sz / 2.0, handle_sz, handle_sz);
                    let _ = cr.fill();
                };

                draw_handle(cr, crop_x, crop_y);
                draw_handle(cr, crop_x + crop_w, crop_y);
                draw_handle(cr, crop_x, crop_y + crop_h);
                draw_handle(cr, crop_x + crop_w, crop_y + crop_h);

                draw_handle(cr, crop_x + crop_w / 2.0, crop_y);
                draw_handle(cr, crop_x + crop_w / 2.0, crop_y + crop_h);
                draw_handle(cr, crop_x, crop_y + crop_h / 2.0);
                draw_handle(cr, crop_x + crop_w, crop_y + crop_h / 2.0);

                cr.restore().ok();
            }
        });

        // 2. Drag Gesture
        let drag_gesture = GestureDrag::new();
        let state_drag = state.clone();
        let area_weak_drag = area.downgrade();

        drag_gesture.connect_drag_begin(move |_, start_x, start_y| {
            let mut s = state_drag.borrow_mut();
            if s.crop_mode {
                let area_w = if let Some(a) = area_weak_drag.upgrade() { a.width() as f64 } else { 0.0 };
                let area_h = if let Some(a) = area_weak_drag.upgrade() { a.height() as f64 } else { 0.0 };
                let iw = s.rendered_width as f64;
                let ih = s.rendered_height as f64;
                let dw = iw * s.zoom;
                let dh = ih * s.zoom;
                let origin_x = (area_w - dw) / 2.0 + s.pan_x;
                let origin_y = (area_h - dh) / 2.0 + s.pan_y;

                let [cx, cy, cw, ch] = s.crop_rect;
                let crop_x = origin_x + cx * dw;
                let crop_y = origin_y + cy * dh;
                let crop_w = cw * dw;
                let crop_h = ch * dh;

                let thresh = 14.0;
                let handle = if (start_x - crop_x).abs() < thresh && (start_y - crop_y).abs() < thresh {
                    CropHandle::TopLeft
                } else if (start_x - (crop_x + crop_w)).abs() < thresh && (start_y - crop_y).abs() < thresh {
                    CropHandle::TopRight
                } else if (start_x - crop_x).abs() < thresh && (start_y - (crop_y + crop_h)).abs() < thresh {
                    CropHandle::BottomLeft
                } else if (start_x - (crop_x + crop_w)).abs() < thresh && (start_y - (crop_y + crop_h)).abs() < thresh {
                    CropHandle::BottomRight
                } else if (start_y - crop_y).abs() < thresh && start_x >= crop_x && start_x <= crop_x + crop_w {
                    CropHandle::Top
                } else if (start_y - (crop_y + crop_h)).abs() < thresh && start_x >= crop_x && start_x <= crop_x + crop_w {
                    CropHandle::Bottom
                } else if (start_x - crop_x).abs() < thresh && start_y >= crop_y && start_y <= crop_y + crop_h {
                    CropHandle::Left
                } else if (start_x - (crop_x + crop_w)).abs() < thresh && start_y >= crop_y && start_y <= crop_y + crop_h {
                    CropHandle::Right
                } else if start_x >= crop_x && start_x <= crop_x + crop_w && start_y >= crop_y && start_y <= crop_y + crop_h {
                    CropHandle::Inside
                } else {
                    CropHandle::None
                };

                s.crop_active_handle = handle;
                s.crop_drag_start_rect = s.crop_rect;
            } else {
                s.drag_start_pan_x = s.pan_x;
                s.drag_start_pan_y = s.pan_y;
            }
        });

        let state_drag_update = state.clone();
        let area_weak_drag_update = area.downgrade();
        drag_gesture.connect_drag_update(move |_, offset_x, offset_y| {
            let mut s = state_drag_update.borrow_mut();
            if s.crop_mode {
                let iw = s.rendered_width as f64;
                let ih = s.rendered_height as f64;
                let dw = iw * s.zoom;
                let dh = ih * s.zoom;
                if dw <= 0.0 || dh <= 0.0 {
                    return;
                }

                let ndx = offset_x / dw;
                let ndy = offset_y / dh;
                let [ox, oy, ow, oh] = s.crop_drag_start_rect;

                if let Some(target_ratio) = s.crop_ratio.ratio() {
                    let img_w = s.rendered_width.max(1) as f64;
                    let img_h = s.rendered_height.max(1) as f64;
                    let norm_ratio = (target_ratio / (img_w / img_h)).max(0.01);

                    match s.crop_active_handle {
                        CropHandle::Inside => {
                            let nx = (ox + ndx).clamp(0.0, 1.0 - ow);
                            let ny = (oy + ndy).clamp(0.0, 1.0 - oh);
                            s.crop_rect[0] = nx;
                            s.crop_rect[1] = ny;
                        }
                        CropHandle::BottomRight => {
                            let mut nw = (ow + ndx).clamp(0.05, 1.0 - ox);
                            let mut nh = nw / norm_ratio;
                            if oy + nh > 1.0 {
                                nh = 1.0 - oy;
                                nw = (nh * norm_ratio).min(1.0 - ox);
                            }
                            s.crop_rect[2] = nw.max(0.05);
                            s.crop_rect[3] = nh.max(0.05);
                        }
                        CropHandle::TopLeft => {
                            let max_w = ox + ow;
                            let max_h = oy + oh;
                            let mut nw = (ow - ndx).clamp(0.05, max_w);
                            let mut nh = nw / norm_ratio;
                            if nh > max_h {
                                nh = max_h;
                                nw = (nh * norm_ratio).min(max_w);
                            }
                            s.crop_rect[0] = ox + ow - nw;
                            s.crop_rect[1] = oy + oh - nh;
                            s.crop_rect[2] = nw.max(0.05);
                            s.crop_rect[3] = nh.max(0.05);
                        }
                        CropHandle::TopRight => {
                            let max_h = oy + oh;
                            let mut nw = (ow + ndx).clamp(0.05, 1.0 - ox);
                            let mut nh = nw / norm_ratio;
                            if nh > max_h {
                                nh = max_h;
                                nw = (nh * norm_ratio).min(1.0 - ox);
                            }
                            s.crop_rect[1] = oy + oh - nh;
                            s.crop_rect[2] = nw.max(0.05);
                            s.crop_rect[3] = nh.max(0.05);
                        }
                        CropHandle::BottomLeft => {
                            let max_w = ox + ow;
                            let mut nw = (ow - ndx).clamp(0.05, max_w);
                            let mut nh = nw / norm_ratio;
                            if oy + nh > 1.0 {
                                nh = 1.0 - oy;
                                nw = (nh * norm_ratio).min(max_w);
                            }
                            s.crop_rect[0] = ox + ow - nw;
                            s.crop_rect[2] = nw.max(0.05);
                            s.crop_rect[3] = nh.max(0.05);
                        }
                        CropHandle::Right | CropHandle::Left => {
                            let sign = if s.crop_active_handle == CropHandle::Right { 1.0 } else { -1.0 };
                            let mut nw = (ow + sign * ndx).clamp(0.05, if sign > 0.0 { 1.0 - ox } else { ox + ow });
                            let nh = (nw / norm_ratio).clamp(0.05, 1.0);
                            nw = nh * norm_ratio;
                            let cy_center = oy + oh / 2.0;
                            let ny = (cy_center - nh / 2.0).clamp(0.0, 1.0 - nh);
                            if sign < 0.0 {
                                s.crop_rect[0] = ox + ow - nw;
                            }
                            s.crop_rect[1] = ny;
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::Bottom | CropHandle::Top => {
                            let sign = if s.crop_active_handle == CropHandle::Bottom { 1.0 } else { -1.0 };
                            let mut nh = (oh + sign * ndy).clamp(0.05, if sign > 0.0 { 1.0 - oy } else { oy + oh });
                            let nw = (nh * norm_ratio).clamp(0.05, 1.0);
                            nh = nw / norm_ratio;
                            let cx_center = ox + ow / 2.0;
                            let nx = (cx_center - nw / 2.0).clamp(0.0, 1.0 - nw);
                            if sign < 0.0 {
                                s.crop_rect[1] = oy + oh - nh;
                            }
                            s.crop_rect[0] = nx;
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::None => {}
                    }
                } else {
                    match s.crop_active_handle {
                        CropHandle::Inside => {
                            let nx = (ox + ndx).clamp(0.0, 1.0 - ow);
                            let ny = (oy + ndy).clamp(0.0, 1.0 - oh);
                            s.crop_rect[0] = nx;
                            s.crop_rect[1] = ny;
                        }
                        CropHandle::TopLeft => {
                            let nx = (ox + ndx).clamp(0.0, ox + ow - 0.05);
                            let ny = (oy + ndy).clamp(0.0, oy + oh - 0.05);
                            s.crop_rect[0] = nx;
                            s.crop_rect[1] = ny;
                            s.crop_rect[2] = (ox + ow) - nx;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::TopRight => {
                            let ny = (oy + ndy).clamp(0.0, oy + oh - 0.05);
                            let nw = (ow + ndx).clamp(0.05, 1.0 - ox);
                            s.crop_rect[1] = ny;
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::BottomLeft => {
                            let nx = (ox + ndx).clamp(0.0, ox + ow - 0.05);
                            let nh = (oh + ndy).clamp(0.05, 1.0 - oy);
                            s.crop_rect[0] = nx;
                            s.crop_rect[2] = (ox + ow) - nx;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::BottomRight => {
                            let nw = (ow + ndx).clamp(0.05, 1.0 - ox);
                            let nh = (oh + ndy).clamp(0.05, 1.0 - oy);
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::Top => {
                            let ny = (oy + ndy).clamp(0.0, oy + oh - 0.05);
                            s.crop_rect[1] = ny;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::Bottom => {
                            let nh = (oh + ndy).clamp(0.05, 1.0 - oy);
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::Left => {
                            let nx = (ox + ndx).clamp(0.0, ox + ow - 0.05);
                            s.crop_rect[0] = nx;
                            s.crop_rect[2] = (ox + ow) - nx;
                        }
                        CropHandle::Right => {
                            let nw = (ow + ndx).clamp(0.05, 1.0 - ox);
                            s.crop_rect[2] = nw;
                        }
                        CropHandle::None => {}
                    }
                }
            } else {
                s.pan_x = s.drag_start_pan_x + offset_x;
                s.pan_y = s.drag_start_pan_y + offset_y;
                s.is_fit = false;
            }

            if let Some(a) = area_weak_drag_update.upgrade() {
                a.queue_draw();
            }
        });
        area.add_controller(drag_gesture);

        // 3. Double-click Gesture
        let click_gesture = GestureClick::new();
        let state_click = state.clone();
        let area_weak_click = area.downgrade();
        let zoom_cb_click = on_zoom_changed.clone();
        click_gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                let is_crop = state_click.borrow().crop_mode;
                if is_crop {
                    let mut s = state_click.borrow_mut();
                    let [cx, cy, cw, ch] = s.crop_rect;
                    s.edits.crop_rect = Some([cx, cy, cw, ch]);
                    s.crop_mode = false;
                    drop(s);
                    if let Some(a) = area_weak_click.upgrade() {
                        a.queue_draw();
                    }
                } else {
                    let zoom_pct = {
                        let mut s = state_click.borrow_mut();
                        if s.is_fit {
                            s.is_fit = false;
                            s.zoom = 1.0;
                            s.pan_x = 0.0;
                            s.pan_y = 0.0;
                        } else {
                            s.is_fit = true;
                        }
                        (s.zoom * 100.0).round() as u32
                    };
                    if let Some(ref cb) = *zoom_cb_click.borrow() {
                        cb(zoom_pct);
                    }
                    if let Some(a) = area_weak_click.upgrade() {
                        a.queue_draw();
                    }
                }
            }
        });
        area.add_controller(click_gesture);

        // 4. Scroll / Zoom controller
        let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        let state_scroll = state.clone();
        let area_weak_scroll = area.downgrade();
        let zoom_cb_scroll = on_zoom_changed.clone();
        scroll_controller.connect_scroll(move |_, _, dy| {
            if state_scroll.borrow().crop_mode {
                return glib::Propagation::Proceed;
            }

            let zoom_pct = {
                let mut s = state_scroll.borrow_mut();
                let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
                let new_zoom = (s.zoom * factor).clamp(0.05, 50.0);
                s.zoom = new_zoom;
                s.is_fit = false;
                (s.zoom * 100.0).round() as u32
            };

            if let Some(ref cb) = *zoom_cb_scroll.borrow() {
                cb(zoom_pct);
            }

            if let Some(a) = area_weak_scroll.upgrade() {
                a.queue_draw();
            }

            glib::Propagation::Stop
        });
        area.add_controller(scroll_controller);

        // 5. Drop target
        let drop_target = DropTarget::new(gdk4::FileList::static_type(), gdk4::DragAction::COPY);
        let drop_cb = on_drop_files.clone();
        drop_target.connect_drop(move |_, val, _, _| {
            if let Ok(file_list) = val.get::<gdk4::FileList>() {
                let paths: Vec<PathBuf> = file_list
                    .files()
                    .into_iter()
                    .filter_map(|f| f.path())
                    .collect();
                if !paths.is_empty() {
                    if let Some(ref cb) = *drop_cb.borrow() {
                        cb(paths);
                    }
                    return true;
                }
            }
            false
        });
        area.add_controller(drop_target);

        Self {
            area,
            state,
            on_zoom_changed,
            on_crop_applied,
            on_drop_files,
        }
    }

    pub fn widget(&self) -> &DrawingArea {
        &self.area
    }

    pub fn set_image(&self, img: Arc<DecodedImage>) {
        let surface = rgba_to_cairo_surface(&img.rgba).ok();
        let zoom_pct = {
            let mut s = self.state.borrow_mut();
            s.image = Some(img.clone());
            s.edits = ImageEdits::default();
            s.crop_mode = false;
            s.crop_rect = [0.1, 0.1, 0.8, 0.8];
            s.rendered_width = img.width;
            s.rendered_height = img.height;
            s.rendered_surface = surface;
            s.is_fit = true;
            s.pan_x = 0.0;
            s.pan_y = 0.0;
            (s.zoom * 100.0).round() as u32
        };

        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(zoom_pct);
        }
        self.area.queue_draw();
    }

    pub fn re_render_edits(&self) {
        let mut s = self.state.borrow_mut();
        if let Some(ref loaded) = s.image {
            let processed = apply_all_edits(&loaded.image, &s.edits);
            let rgba = processed.to_rgba8();
            if let Ok(surface) = rgba_to_cairo_surface(&rgba) {
                s.rendered_width = processed.width();
                s.rendered_height = processed.height();
                s.rendered_surface = Some(surface);
            }
        }
        self.area.queue_draw();
    }

    pub fn zoom_in(&self) {
        let zoom_pct = {
            let mut s = self.state.borrow_mut();
            s.zoom = (s.zoom * 1.25).min(50.0);
            s.is_fit = false;
            (s.zoom * 100.0).round() as u32
        };
        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(zoom_pct);
        }
        self.area.queue_draw();
    }

    pub fn zoom_out(&self) {
        let zoom_pct = {
            let mut s = self.state.borrow_mut();
            s.zoom = (s.zoom / 1.25).max(0.05);
            s.is_fit = false;
            (s.zoom * 100.0).round() as u32
        };
        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(zoom_pct);
        }
        self.area.queue_draw();
    }

    pub fn zoom_100(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.zoom = 1.0;
            s.is_fit = false;
            s.pan_x = 0.0;
            s.pan_y = 0.0;
        }
        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(100);
        }
        self.area.queue_draw();
    }

    pub fn zoom_fit(&self) {
        let zoom_pct = {
            let mut s = self.state.borrow_mut();
            s.is_fit = true;
            s.pan_x = 0.0;
            s.pan_y = 0.0;
            (s.zoom * 100.0).round() as u32
        };
        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(zoom_pct);
        }
        self.area.queue_draw();
    }

    pub fn get_zoom_pct(&self) -> u32 {
        (self.state.borrow().zoom * 100.0).round() as u32
    }

    pub fn rotate_cw(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.edits.rotate_cw();
        }
        self.re_render_edits();
    }

    pub fn rotate_ccw(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.edits.rotate_ccw();
        }
        self.re_render_edits();
    }

    pub fn flip_h(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.edits.toggle_flip_h();
        }
        self.re_render_edits();
    }

    pub fn flip_v(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.edits.toggle_flip_v();
        }
        self.re_render_edits();
    }

    pub fn toggle_crop(&self) -> bool {
        let active = {
            let mut s = self.state.borrow_mut();
            s.crop_mode = !s.crop_mode;
            if s.crop_mode {
                s.crop_ratio = CropRatio::Freeform;
                s.crop_rect = [0.1, 0.1, 0.8, 0.8];
            }
            s.crop_mode
        };
        self.area.queue_draw();
        active
    }

    pub fn set_crop_ratio(&self, ratio: CropRatio) {
        {
            let mut s = self.state.borrow_mut();
            s.crop_ratio = ratio;
            if let Some(target_ratio) = ratio.ratio() {
                let img_w = s.rendered_width.max(1) as f64;
                let img_h = s.rendered_height.max(1) as f64;
                let img_aspect = img_w / img_h;
                let norm_ratio = (target_ratio / img_aspect).max(0.01);

                let mut cw: f64;
                let mut ch: f64;
                if norm_ratio <= 1.0 {
                    ch = 0.8;
                    cw = (ch * norm_ratio).min(0.9);
                    ch = cw / norm_ratio;
                } else {
                    cw = 0.8;
                    ch = (cw / norm_ratio).min(0.9);
                    cw = ch * norm_ratio;
                }
                let cx = ((1.0 - cw) / 2.0).clamp(0.0, 1.0 - cw);
                let cy = ((1.0 - ch) / 2.0).clamp(0.0, 1.0 - ch);
                s.crop_rect = [cx, cy, cw, ch];
            }
        }
        self.area.queue_draw();
    }

    pub fn apply_crop(&self) {
        {
            let mut s = self.state.borrow_mut();
            if s.crop_mode {
                let [cx, cy, cw, ch] = s.crop_rect;
                s.edits.crop_rect = Some([cx, cy, cw, ch]);
                s.crop_mode = false;
            }
        }
        self.re_render_edits();
        if let Some(ref cb) = *self.on_crop_applied.borrow() {
            cb();
        }
    }

    pub fn cancel_crop(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.crop_mode = false;
        }
        self.area.queue_draw();
    }

    pub fn is_in_crop_mode(&self) -> bool {
        self.state.borrow().crop_mode
    }

    pub fn update_adjustments(&self, exposure: f64, contrast: f64, saturation: f64, warmth: f64) {
        {
            let mut s = self.state.borrow_mut();
            s.edits.exposure = exposure;
            s.edits.contrast = contrast;
            s.edits.saturation = saturation;
            s.edits.warmth = warmth;
        }
        self.re_render_edits();
    }

    pub fn get_edits(&self) -> ImageEdits {
        self.state.borrow().edits.clone()
    }

    pub fn get_current_image(&self) -> Option<Arc<DecodedImage>> {
        self.state.borrow().image.clone()
    }

    pub fn connect_zoom_changed<F: Fn(u32) + 'static>(&self, callback: F) {
        *self.on_zoom_changed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_drop_files<F: Fn(Vec<PathBuf>) + 'static>(&self, callback: F) {
        *self.on_drop_files.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_crop_applied<F: Fn() + 'static>(&self, callback: F) {
        *self.on_crop_applied.borrow_mut() = Some(Box::new(callback));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crop_ratios() {
        assert_eq!(CropRatio::Freeform.ratio(), None);
        assert!((CropRatio::Square.ratio().unwrap() - 1.0).abs() < 1e-6);
        assert!((CropRatio::SixteenNine.ratio().unwrap() - (16.0 / 9.0)).abs() < 1e-6);
    }
}

