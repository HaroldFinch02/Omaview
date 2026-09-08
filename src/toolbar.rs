use gtk4::prelude::*;
use gtk4::{Box, Button, Label, Orientation, Popover, Scale};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct BottomToolbar {
    container: Box,
    btn_prev: Button,
    btn_grid: Button,
    btn_zoom_in: Button,
    btn_zoom_indicator: Button,
    btn_zoom_out: Button,
    btn_crop: Button,
    btn_rotate_cw: Button,
    #[allow(dead_code)]
    btn_adjust: Button,
    btn_info: Button,
    btn_trash: Button,
    btn_save: Button,

    adjustments_popover: Popover,
    scale_exposure: Scale,
    scale_contrast: Scale,
    scale_saturation: Scale,
    scale_warmth: Scale,
}

impl BottomToolbar {
    pub fn new() -> Self {
        let container = Box::new(Orientation::Horizontal, 3);
        container.add_css_class("floating-pill");
        container.set_halign(gtk4::Align::Center);
        container.set_valign(gtk4::Align::End);
        container.set_margin_bottom(20);

        let make_btn = |icon: &str, tooltip: &str| -> Button {
            let btn = Button::from_icon_name(icon);
            btn.set_tooltip_text(Some(tooltip));
            btn.set_has_frame(false);
            btn
        };

        // 1. Previous
        let btn_prev = make_btn("go-previous-symbolic", "Previous Image (k / ← / Backspace)");
        container.append(&btn_prev);

        // 2. Filmstrip / Grid toggle (matches sample_ui.jpeg 2nd icon)
        let btn_grid = make_btn("view-grid-symbolic", "Toggle Filmstrip (g)");
        btn_grid.add_css_class("active-highlight");
        container.append(&btn_grid);

        // 3. Zoom in (matches sample_ui.jpeg 3rd icon)
        let btn_zoom_in = make_btn("zoom-in-symbolic", "Zoom In (+ / Ctrl+↑)");
        container.append(&btn_zoom_in);

        // 4. Center Zoom Indicator Dot Button (matches sample_ui.jpeg 4th icon)
        let btn_zoom_indicator = Button::new();
        btn_zoom_indicator.set_has_frame(false);
        btn_zoom_indicator.set_tooltip_text(Some("Toggle Fit / 100% (0 / f)"));
        btn_zoom_indicator.add_css_class("zoom-indicator-box");

        let dot = Box::new(Orientation::Horizontal, 0);
        dot.add_css_class("zoom-indicator-dot");
        dot.set_halign(gtk4::Align::Center);
        dot.set_valign(gtk4::Align::Center);
        btn_zoom_indicator.set_child(Some(&dot));
        container.append(&btn_zoom_indicator);

        // 5. Zoom out (matches sample_ui.jpeg 5th icon)
        let btn_zoom_out = make_btn("zoom-out-symbolic", "Zoom Out (- / Ctrl+↓)");
        container.append(&btn_zoom_out);

        // 6. Crop (matches sample_ui.jpeg 6th icon)
        let btn_crop = make_btn("edit-cut-symbolic", "Crop Mode (c)");
        container.append(&btn_crop);

        // 7. Rotate (matches sample_ui.jpeg 7th icon)
        let btn_rotate_cw = make_btn("object-rotate-right-symbolic", "Rotate 90° CW (r / Shift+R)");
        container.append(&btn_rotate_cw);

        // 8. Adjustments (matches sample_ui.jpeg 8th icon)
        let btn_adjust = make_btn("preferences-color-symbolic", "Adjustments (a / e)");
        container.append(&btn_adjust);

        // 9. Info / EXIF (matches sample_ui.jpeg 9th icon)
        let btn_info = make_btn("dialog-information-symbolic", "Image Info / EXIF (i)");
        container.append(&btn_info);

        // 10. Trash (matches sample_ui.jpeg 10th icon)
        let btn_trash = make_btn("user-trash-symbolic", "Move to Trash (Delete / d)");
        container.append(&btn_trash);

        // 11. Save Changes (visible when changes are made)
        let btn_save = make_btn("document-save-symbolic", "Save Changes (Ctrl+S / s)");
        btn_save.add_css_class("toolbar-save-btn");
        btn_save.set_visible(false);
        container.append(&btn_save);

        // Adjustments Popover
        let adjustments_popover = Popover::new();
        adjustments_popover.add_css_class("adjustments-panel");
        adjustments_popover.set_parent(&btn_adjust);

        let adj_box = Box::new(Orientation::Vertical, 8);
        adj_box.set_margin_top(10);
        adj_box.set_margin_bottom(10);
        adj_box.set_margin_start(14);
        adj_box.set_margin_end(14);
        adj_box.set_size_request(240, -1);

        let adj_title = Label::new(None);
        adj_title.set_markup("<b>Adjustments</b>");
        adj_title.set_halign(gtk4::Align::Start);
        adj_box.append(&adj_title);

        let create_slider = |label_text: &str| -> (Scale, Label) {
            let row = Box::new(Orientation::Horizontal, 8);
            let lbl = Label::new(Some(label_text));
            lbl.set_halign(gtk4::Align::Start);
            lbl.set_hexpand(true);

            let val_lbl = Label::new(Some("0"));
            val_lbl.set_width_chars(4);
            val_lbl.set_xalign(1.0);

            row.append(&lbl);
            row.append(&val_lbl);
            adj_box.append(&row);

            let scale = Scale::with_range(Orientation::Horizontal, -100.0, 100.0, 1.0);
            scale.set_value(0.0);
            scale.set_hexpand(true);
            scale.set_draw_value(false);

            let val_clone = val_lbl.clone();
            scale.connect_value_changed(move |s| {
                let v = s.value().round() as i32;
                val_clone.set_text(&format!("{}", v));
            });

            adj_box.append(&scale);
            (scale, val_lbl)
        };

        let (scale_exposure, _) = create_slider("Exposure");
        let (scale_contrast, _) = create_slider("Contrast");
        let (scale_saturation, _) = create_slider("Saturation");
        let (scale_warmth, _) = create_slider("Warmth");

        let reset_btn = Button::with_label("Reset Adjustments");
        reset_btn.set_margin_top(8);
        let se = scale_exposure.clone();
        let sc = scale_contrast.clone();
        let ss = scale_saturation.clone();
        let sw = scale_warmth.clone();
        reset_btn.connect_clicked(move |_| {
            se.set_value(0.0);
            sc.set_value(0.0);
            ss.set_value(0.0);
            sw.set_value(0.0);
        });
        adj_box.append(&reset_btn);

        adjustments_popover.set_child(Some(&adj_box));

        let pop_clone = adjustments_popover.clone();
        btn_adjust.connect_clicked(move |_| {
            if pop_clone.is_visible() {
                pop_clone.popdown();
            } else {
                pop_clone.popup();
            }
        });

        Self {
            container,
            btn_prev,
            btn_grid,
            btn_zoom_in,
            btn_zoom_indicator,
            btn_zoom_out,
            btn_crop,
            btn_rotate_cw,
            btn_adjust,
            btn_info,
            btn_trash,
            btn_save,
            adjustments_popover,
            scale_exposure,
            scale_contrast,
            scale_saturation,
            scale_warmth,
        }
    }

    pub fn widget(&self) -> &Box {
        &self.container
    }

    pub fn set_save_visible(&self, visible: bool) {
        self.btn_save.set_visible(visible);
        if visible {
            self.btn_save.add_css_class("active-save-highlight");
        } else {
            self.btn_save.remove_css_class("active-save-highlight");
        }
    }

    pub fn connect_save<F: Fn() + 'static>(&self, callback: F) {
        self.btn_save.connect_clicked(move |_| callback());
    }

    pub fn set_filmstrip_active(&self, active: bool) {
        if active {
            self.btn_grid.add_css_class("active-highlight");
        } else {
            self.btn_grid.remove_css_class("active-highlight");
        }
    }

    pub fn toggle_adjustments(&self) {
        if self.adjustments_popover.is_visible() {
            self.adjustments_popover.popdown();
        } else {
            self.adjustments_popover.popup();
        }
    }

    pub fn set_crop_active(&self, active: bool) {
        if active {
            self.btn_crop.add_css_class("active-highlight");
        } else {
            self.btn_crop.remove_css_class("active-highlight");
        }
    }

    pub fn info_button(&self) -> &Button {
        &self.btn_info
    }

    pub fn connect_prev<F: Fn() + 'static>(&self, f: F) {
        self.btn_prev.connect_clicked(move |_| f());
    }

    pub fn connect_grid_toggle<F: Fn() + 'static>(&self, f: F) {
        self.btn_grid.connect_clicked(move |_| f());
    }

    pub fn connect_zoom_in<F: Fn() + 'static>(&self, f: F) {
        self.btn_zoom_in.connect_clicked(move |_| f());
    }

    pub fn connect_zoom_indicator<F: Fn() + 'static>(&self, f: F) {
        self.btn_zoom_indicator.connect_clicked(move |_| f());
    }

    pub fn connect_zoom_out<F: Fn() + 'static>(&self, f: F) {
        self.btn_zoom_out.connect_clicked(move |_| f());
    }

    pub fn connect_crop<F: Fn() + 'static>(&self, f: F) {
        self.btn_crop.connect_clicked(move |_| f());
    }

    pub fn connect_rotate_cw<F: Fn() + 'static>(&self, f: F) {
        self.btn_rotate_cw.connect_clicked(move |_| f());
    }

    pub fn connect_trash<F: Fn() + 'static>(&self, f: F) {
        self.btn_trash.connect_clicked(move |_| f());
    }

    pub fn connect_adjustments_changed<F: Fn(f64, f64, f64, f64) + 'static>(&self, f: F) {
        let f = Rc::new(f);
        let se = self.scale_exposure.clone();
        let sc = self.scale_contrast.clone();
        let ss = self.scale_saturation.clone();
        let sw = self.scale_warmth.clone();

        let trigger = {
            let f = f.clone();
            let se = se.clone();
            let sc = sc.clone();
            let ss = ss.clone();
            let sw = sw.clone();
            move || {
                f(se.value(), sc.value(), ss.value(), sw.value());
            }
        };

        let trig1 = Rc::new(RefCell::new(trigger));
        let t1 = trig1.clone();
        self.scale_exposure.connect_value_changed(move |_| (t1.borrow())());
        let t2 = trig1.clone();
        self.scale_contrast.connect_value_changed(move |_| (t2.borrow())());
        let t3 = trig1.clone();
        self.scale_saturation.connect_value_changed(move |_| (t3.borrow())());
        let t4 = trig1.clone();
        self.scale_warmth.connect_value_changed(move |_| (t4.borrow())());
    }

    pub fn reset_adjustments_ui(&self) {
        self.scale_exposure.set_value(0.0);
        self.scale_contrast.set_value(0.0);
        self.scale_saturation.set_value(0.0);
        self.scale_warmth.set_value(0.0);
    }
}
