use crate::util::Callback;
use gtk4::prelude::*;
use gtk4::{
    DrawingArea, DropTarget, EventControllerScroll, EventControllerScrollFlags, GestureClick,
    GestureDrag,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

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
    display_zoom: f64,
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
    on_zoom_changed: Callback<dyn Fn(u32)>,
    on_crop_applied: Callback<dyn Fn()>,
    on_drop_files: Callback<dyn Fn(Vec<PathBuf>)>,
    rendering: Rc<Cell<bool>>,
    zoom_animating: Rc<Cell<bool>>,
}

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

fn start_zoom_animation(
    area: &DrawingArea,
    state: Rc<RefCell<ViewportState>>,
    animating: Rc<Cell<bool>>,
) {
    if animating.replace(true) {
        return;
    }
    let last_frame = Cell::new(None);
    area.add_tick_callback(move |area, clock| {
        let now = clock.frame_time();
        let dt = last_frame
            .get()
            .map_or(1.0 / 60.0, |last| (now - last) as f64 / 1_000_000.0);
        last_frame.set(Some(now));
        let mut s = state.borrow_mut();
        s.display_zoom += (s.zoom - s.display_zoom) * (1.0 - (-dt / 0.035).exp());
        let done = s.is_fit
            || !area.settings().is_gtk_enable_animations()
            || (s.zoom - s.display_zoom).abs() < s.zoom * 0.001;
        if done {
            s.display_zoom = s.zoom;
            animating.set(false);
        }
        drop(s);
        area.queue_draw();
        if done {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
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
            display_zoom: 1.0,
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
        let rendering = Rc::new(Cell::new(false));
        let zoom_animating = Rc::new(Cell::new(false));

        let resize_state = state.clone();
        let resize_zoom_cb = on_zoom_changed.clone();
        area.connect_resize(move |_, width, height| {
            let zoom_pct = {
                let mut s = resize_state.borrow_mut();
                if width <= 0
                    || height <= 0
                    || !s.is_fit
                    || s.rendered_width == 0
                    || s.rendered_height == 0
                {
                    return;
                }
                s.zoom = (width as f64 / s.rendered_width as f64)
                    .min(height as f64 / s.rendered_height as f64)
                    .min(1.0);
                s.display_zoom = s.zoom;
                (s.zoom * 100.0).round() as u32
            };
            if let Some(ref callback) = *resize_zoom_cb.borrow() {
                callback(zoom_pct);
            }
        });

        // 1. Draw function
        let state_draw = state.clone();
        area.set_draw_func(move |_, cr, width, height| {
            let mut s = state_draw.borrow_mut();
            let surface = match s.rendered_surface.clone() {
                Some(surf) => surf,
                None => {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                    cr.select_font_face(
                        "sans-serif",
                        cairo::FontSlant::Normal,
                        cairo::FontWeight::Normal,
                    );
                    cr.set_font_size(15.0);
                    let text = "Open an image or drag & drop files here";
                    if let Ok(ext) = cr.text_extents(text) {
                        cr.move_to(
                            (width as f64 - ext.width()) / 2.0,
                            (height as f64 + ext.height()) / 2.0,
                        );
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
                s.display_zoom = s.zoom;
                s.pan_x = 0.0;
                s.pan_y = 0.0;
            }

            let dw = iw * s.display_zoom;
            let dh = ih * s.display_zoom;
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
            cr.scale(s.display_zoom, s.display_zoom);

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
                    cr.rectangle(
                        hx - handle_sz / 2.0,
                        hy - handle_sz / 2.0,
                        handle_sz,
                        handle_sz,
                    );
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
                let area_w = if let Some(a) = area_weak_drag.upgrade() {
                    a.width() as f64
                } else {
                    0.0
                };
                let area_h = if let Some(a) = area_weak_drag.upgrade() {
                    a.height() as f64
                } else {
                    0.0
                };
                let iw = s.rendered_width as f64;
                let ih = s.rendered_height as f64;
                let dw = iw * s.display_zoom;
                let dh = ih * s.display_zoom;
                let origin_x = (area_w - dw) / 2.0 + s.pan_x;
                let origin_y = (area_h - dh) / 2.0 + s.pan_y;

                let [cx, cy, cw, ch] = s.crop_rect;
                let crop_x = origin_x + cx * dw;
                let crop_y = origin_y + cy * dh;
                let crop_w = cw * dw;
                let crop_h = ch * dh;

                let thresh = 14.0;
                let handle =
                    if (start_x - crop_x).abs() < thresh && (start_y - crop_y).abs() < thresh {
                        CropHandle::TopLeft
                    } else if (start_x - (crop_x + crop_w)).abs() < thresh
                        && (start_y - crop_y).abs() < thresh
                    {
                        CropHandle::TopRight
                    } else if (start_x - crop_x).abs() < thresh
                        && (start_y - (crop_y + crop_h)).abs() < thresh
                    {
                        CropHandle::BottomLeft
                    } else if (start_x - (crop_x + crop_w)).abs() < thresh
                        && (start_y - (crop_y + crop_h)).abs() < thresh
                    {
                        CropHandle::BottomRight
                    } else if (start_y - crop_y).abs() < thresh
                        && start_x >= crop_x
                        && start_x <= crop_x + crop_w
                    {
                        CropHandle::Top
                    } else if (start_y - (crop_y + crop_h)).abs() < thresh
                        && start_x >= crop_x
                        && start_x <= crop_x + crop_w
                    {
                        CropHandle::Bottom
                    } else if (start_x - crop_x).abs() < thresh
                        && start_y >= crop_y
                        && start_y <= crop_y + crop_h
                    {
                        CropHandle::Left
                    } else if (start_x - (crop_x + crop_w)).abs() < thresh
                        && start_y >= crop_y
                        && start_y <= crop_y + crop_h
                    {
                        CropHandle::Right
                    } else if start_x >= crop_x
                        && start_x <= crop_x + crop_w
                        && start_y >= crop_y
                        && start_y <= crop_y + crop_h
                    {
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
                let dw = iw * s.display_zoom;
                let dh = ih * s.display_zoom;
                if dw <= 0.0 || dh <= 0.0 {
                    return;
                }

                let ndx = offset_x / dw;
                let ndy = offset_y / dh;
                let [ox, oy, ow, oh] = s.crop_drag_start_rect;
                let min_w = 0.05f64.min(ow);
                let min_h = 0.05f64.min(oh);

                if let Some(target_ratio) = s.crop_ratio.ratio() {
                    let img_w = s.rendered_width.max(1) as f64;
                    let img_h = s.rendered_height.max(1) as f64;
                    let norm_ratio = target_ratio / (img_w / img_h);

                    match s.crop_active_handle {
                        CropHandle::Inside => {
                            let nx = (ox + ndx).clamp(0.0, 1.0 - ow);
                            let ny = (oy + ndy).clamp(0.0, 1.0 - oh);
                            s.crop_rect[0] = nx;
                            s.crop_rect[1] = ny;
                        }
                        CropHandle::BottomRight => {
                            let mut nw = (ow + ndx).clamp(min_w, 1.0 - ox);
                            let mut nh = nw / norm_ratio;
                            if oy + nh > 1.0 {
                                nh = 1.0 - oy;
                                nw = (nh * norm_ratio).min(1.0 - ox);
                            }
                            s.crop_rect[2] = nw.max(min_w);
                            s.crop_rect[3] = nh.max(min_h);
                        }
                        CropHandle::TopLeft => {
                            let max_w = ox + ow;
                            let max_h = oy + oh;
                            let mut nw = (ow - ndx).clamp(min_w, max_w);
                            let mut nh = nw / norm_ratio;
                            if nh > max_h {
                                nh = max_h;
                                nw = (nh * norm_ratio).min(max_w);
                            }
                            s.crop_rect[0] = ox + ow - nw;
                            s.crop_rect[1] = oy + oh - nh;
                            s.crop_rect[2] = nw.max(min_w);
                            s.crop_rect[3] = nh.max(min_h);
                        }
                        CropHandle::TopRight => {
                            let max_h = oy + oh;
                            let mut nw = (ow + ndx).clamp(min_w, 1.0 - ox);
                            let mut nh = nw / norm_ratio;
                            if nh > max_h {
                                nh = max_h;
                                nw = (nh * norm_ratio).min(1.0 - ox);
                            }
                            s.crop_rect[1] = oy + oh - nh;
                            s.crop_rect[2] = nw.max(min_w);
                            s.crop_rect[3] = nh.max(min_h);
                        }
                        CropHandle::BottomLeft => {
                            let max_w = ox + ow;
                            let mut nw = (ow - ndx).clamp(min_w, max_w);
                            let mut nh = nw / norm_ratio;
                            if oy + nh > 1.0 {
                                nh = 1.0 - oy;
                                nw = (nh * norm_ratio).min(max_w);
                            }
                            s.crop_rect[0] = ox + ow - nw;
                            s.crop_rect[2] = nw.max(min_w);
                            s.crop_rect[3] = nh.max(min_h);
                        }
                        CropHandle::Right | CropHandle::Left => {
                            let sign = if s.crop_active_handle == CropHandle::Right {
                                1.0
                            } else {
                                -1.0
                            };
                            let mut nw = (ow + sign * ndx)
                                .clamp(min_w, if sign > 0.0 { 1.0 - ox } else { ox + ow });
                            let nh = (nw / norm_ratio).clamp(min_h, 1.0);
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
                            let sign = if s.crop_active_handle == CropHandle::Bottom {
                                1.0
                            } else {
                                -1.0
                            };
                            let mut nh = (oh + sign * ndy)
                                .clamp(min_h, if sign > 0.0 { 1.0 - oy } else { oy + oh });
                            let nw = (nh * norm_ratio).clamp(min_w, 1.0);
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
                            let nx = (ox + ndx).clamp(0.0, ox + ow - min_w);
                            let ny = (oy + ndy).clamp(0.0, oy + oh - min_h);
                            s.crop_rect[0] = nx;
                            s.crop_rect[1] = ny;
                            s.crop_rect[2] = (ox + ow) - nx;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::TopRight => {
                            let ny = (oy + ndy).clamp(0.0, oy + oh - min_h);
                            let nw = (ow + ndx).clamp(min_w, 1.0 - ox);
                            s.crop_rect[1] = ny;
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::BottomLeft => {
                            let nx = (ox + ndx).clamp(0.0, ox + ow - min_w);
                            let nh = (oh + ndy).clamp(min_h, 1.0 - oy);
                            s.crop_rect[0] = nx;
                            s.crop_rect[2] = (ox + ow) - nx;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::BottomRight => {
                            let nw = (ow + ndx).clamp(min_w, 1.0 - ox);
                            let nh = (oh + ndy).clamp(min_h, 1.0 - oy);
                            s.crop_rect[2] = nw;
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::Top => {
                            let ny = (oy + ndy).clamp(0.0, oy + oh - min_h);
                            s.crop_rect[1] = ny;
                            s.crop_rect[3] = (oy + oh) - ny;
                        }
                        CropHandle::Bottom => {
                            let nh = (oh + ndy).clamp(min_h, 1.0 - oy);
                            s.crop_rect[3] = nh;
                        }
                        CropHandle::Left => {
                            let nx = (ox + ndx).clamp(0.0, ox + ow - min_w);
                            s.crop_rect[0] = nx;
                            s.crop_rect[2] = (ox + ow) - nx;
                        }
                        CropHandle::Right => {
                            let nw = (ow + ndx).clamp(min_w, 1.0 - ox);
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
        let crop_cb_click = on_crop_applied.clone();
        let drop_cb_click = on_drop_files.clone();
        let rendering_click = rendering.clone();
        let zoom_animating_click = zoom_animating.clone();
        click_gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                let is_crop = state_click.borrow().crop_mode;
                if is_crop {
                    if let Some(a) = area_weak_click.upgrade() {
                        let viewport = Self {
                            area: a,
                            state: state_click.clone(),
                            on_zoom_changed: zoom_cb_click.clone(),
                            on_crop_applied: crop_cb_click.clone(),
                            on_drop_files: drop_cb_click.clone(),
                            rendering: rendering_click.clone(),
                            zoom_animating: zoom_animating_click.clone(),
                        };
                        viewport.apply_crop();
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
                            if let Some(a) = area_weak_click.upgrade() {
                                s.zoom = (a.width() as f64 / s.rendered_width.max(1) as f64)
                                    .min(a.height() as f64 / s.rendered_height.max(1) as f64)
                                    .min(1.0);
                            }
                        }
                        (s.zoom * 100.0).round() as u32
                    };
                    if let Some(ref cb) = *zoom_cb_click.borrow() {
                        cb(zoom_pct);
                    }
                    if let Some(a) = area_weak_click.upgrade() {
                        start_zoom_animation(&a, state_click.clone(), zoom_animating_click.clone());
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
        let zoom_animating_scroll = zoom_animating.clone();
        scroll_controller.connect_scroll(move |_, _, dy| {
            if state_scroll.borrow().crop_mode || dy == 0.0 {
                return glib::Propagation::Proceed;
            }

            let zoom_pct = {
                let mut s = state_scroll.borrow_mut();
                let factor = 1.15f64.powf(-dy.clamp(-4.0, 4.0));
                let new_zoom = (s.zoom * factor).clamp(0.05, 50.0);
                s.zoom = new_zoom;
                s.is_fit = false;
                (s.zoom * 100.0).round() as u32
            };

            if let Some(ref cb) = *zoom_cb_scroll.borrow() {
                cb(zoom_pct);
            }

            if let Some(a) = area_weak_scroll.upgrade() {
                start_zoom_animation(&a, state_scroll.clone(), zoom_animating_scroll.clone());
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
            rendering,
            zoom_animating,
        }
    }

    pub fn widget(&self) -> &DrawingArea {
        &self.area
    }

    pub fn set_image(&self, img: Arc<DecodedImage>) {
        let mut s = self.state.borrow_mut();
        let surface =
            crate::image_loader::cairo_data_to_surface(&img.cairo_data, img.width, img.height).ok();

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
        s.zoom = self.fit_scale(img.width, img.height);
        s.display_zoom = s.zoom;
        let zoom_pct = (s.zoom * 100.0).round() as u32;
        drop(s);

        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(zoom_pct);
        }
        self.area.queue_draw();
    }

    /// Instantly displays a thumbnail preview while the full-resolution image decodes in the background.
    pub fn set_thumbnail_preview(&self, thumb: &crate::image_loader::DecodedThumbnail) {
        if let Ok(surf) = rgba_to_cairo_surface(&thumb.rgba) {
            let mut s = self.state.borrow_mut();
            s.rendered_width = thumb.width;
            s.rendered_height = thumb.height;
            s.rendered_surface = Some(surf);
            s.is_fit = true;
            s.pan_x = 0.0;
            s.pan_y = 0.0;
            drop(s);
            self.area.queue_draw();
        }
    }

    pub fn re_render_edits(&self) {
        if self.rendering.get() {
            return;
        }
        let (loaded, edits) = {
            let s = self.state.borrow();
            let Some(loaded) = s.image.clone() else {
                return;
            };
            (loaded, s.edits.clone())
        };
        self.rendering.set(true);
        let (sender, receiver) = std::sync::mpsc::channel();
        let image = loaded.clone();
        let job_edits = edits.clone();
        crate::image_loader::run_background(move || {
            let processed = apply_all_edits(&image.image, &job_edits);
            let rgba = processed.to_rgba8();
            let data = crate::image_loader::rgba_to_argb32_bytes(&rgba);
            let _ = sender.send((data, processed.width(), processed.height()));
        });
        let viewport = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
            match receiver.try_recv() {
                Ok((data, width, height)) => {
                    viewport.rendering.set(false);
                    let is_current = viewport
                        .state
                        .borrow()
                        .image
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &loaded));
                    if is_current && viewport.get_edits() == edits {
                        if let Ok(surface) =
                            crate::image_loader::cairo_owned_data_to_surface(data, width, height)
                        {
                            let mut s = viewport.state.borrow_mut();
                            s.rendered_width = width;
                            s.rendered_height = height;
                            s.rendered_surface = Some(surface);
                            if s.is_fit {
                                s.zoom = viewport.fit_scale(width, height);
                            }
                        }
                        viewport.zoom_changed();
                        viewport.area.queue_draw();
                    } else if viewport.get_edits().has_any_edits() || is_current {
                        // One job at a time, with only the latest slider values queued.
                        viewport.re_render_edits();
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    viewport.rendering.set(false);
                    glib::ControlFlow::Break
                }
            }
        });
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
        start_zoom_animation(&self.area, self.state.clone(), self.zoom_animating.clone());
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
        start_zoom_animation(&self.area, self.state.clone(), self.zoom_animating.clone());
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
        start_zoom_animation(&self.area, self.state.clone(), self.zoom_animating.clone());
        self.area.queue_draw();
    }

    pub fn zoom_fit(&self) {
        let zoom_pct = {
            let mut s = self.state.borrow_mut();
            s.is_fit = true;
            s.pan_x = 0.0;
            s.pan_y = 0.0;
            s.zoom = self.fit_scale(s.rendered_width, s.rendered_height);
            s.display_zoom = s.zoom;
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
        if self.state.borrow().image.is_none() {
            return false;
        }
        if self.rendering.get() && !self.state.borrow().crop_mode {
            return false;
        }
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
                let norm_ratio = target_ratio / img_aspect;

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
                let rect = s.crop_rect;
                s.edits.add_crop(rect);
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

    /// Clear edit ownership immediately when navigation starts. A thumbnail must
    /// never expose the previous image as the new image's editable original.
    pub fn begin_loading(&self) {
        let mut s = self.state.borrow_mut();
        s.image = None;
        s.edits = ImageEdits::default();
        s.crop_mode = false;
        s.rendered_surface = None;
        s.rendered_width = 0;
        s.rendered_height = 0;
        drop(s);
        self.area.queue_draw();
    }

    fn fit_scale(&self, width: u32, height: u32) -> f64 {
        if width == 0 || height == 0 || self.area.width() <= 0 || self.area.height() <= 0 {
            return 1.0;
        }
        (self.area.width() as f64 / width as f64)
            .min(self.area.height() as f64 / height as f64)
            .min(1.0)
    }

    fn zoom_changed(&self) {
        if let Some(ref cb) = *self.on_zoom_changed.borrow() {
            cb(self.get_zoom_pct());
        }
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
