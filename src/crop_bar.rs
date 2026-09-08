use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Image, Label, Orientation, Separator};
use std::cell::RefCell;
use std::rc::Rc;
use crate::viewport::CropRatio;

#[derive(Clone)]
pub struct CropBar {
    container: GtkBox,
    btn_freeform: Button,
    btn_square: Button,
    btn_16_9: Button,
    btn_apply: Button,
    btn_cancel: Button,
    active_ratio: Rc<RefCell<CropRatio>>,
    on_ratio_selected: Rc<RefCell<Option<Box<dyn Fn(CropRatio) + 'static>>>>,
    on_apply: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    on_cancel: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
}

impl CropBar {
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 6);
        container.add_css_class("crop-pill");
        container.set_halign(gtk4::Align::Center);
        container.set_valign(gtk4::Align::Start);
        container.set_margin_top(16);
        container.set_visible(false);

        let icon = Image::from_icon_name("edit-cut-symbolic");
        icon.add_css_class("crop-icon");
        container.append(&icon);

        let lbl_crop = Label::new(Some("Crop:"));
        lbl_crop.add_css_class("crop-title-lbl");
        container.append(&lbl_crop);

        let make_ratio_btn = |label: &str| -> Button {
            let btn = Button::with_label(label);
            btn.set_has_frame(false);
            btn.add_css_class("crop-ratio-btn");
            btn
        };

        let btn_freeform = make_ratio_btn("Freeform");
        btn_freeform.add_css_class("active-ratio");
        container.append(&btn_freeform);

        let btn_square = make_ratio_btn("1:1 Square");
        container.append(&btn_square);

        let btn_16_9 = make_ratio_btn("16:9");
        container.append(&btn_16_9);

        let sep = Separator::new(Orientation::Vertical);
        sep.set_margin_start(4);
        sep.set_margin_end(4);
        container.append(&sep);

        // Apply Button
        let btn_apply = Button::new();
        btn_apply.set_has_frame(false);
        btn_apply.add_css_class("crop-apply-btn");
        btn_apply.set_tooltip_text(Some("Apply Crop (Enter)"));
        let apply_box = GtkBox::new(Orientation::Horizontal, 4);
        let apply_icon = Image::from_icon_name("object-select-symbolic");
        let apply_lbl = Label::new(Some("Apply (Enter)"));
        apply_box.append(&apply_icon);
        apply_box.append(&apply_lbl);
        btn_apply.set_child(Some(&apply_box));
        container.append(&btn_apply);

        // Cancel Button
        let btn_cancel = Button::from_icon_name("window-close-symbolic");
        btn_cancel.set_has_frame(false);
        btn_cancel.add_css_class("crop-cancel-btn");
        btn_cancel.set_tooltip_text(Some("Cancel (Esc)"));
        container.append(&btn_cancel);

        let active_ratio = Rc::new(RefCell::new(CropRatio::Freeform));
        let on_ratio_selected = Rc::new(RefCell::new(None));
        let on_apply = Rc::new(RefCell::new(None));
        let on_cancel = Rc::new(RefCell::new(None));

        let bar = Self {
            container,
            btn_freeform,
            btn_square,
            btn_16_9,
            btn_apply,
            btn_cancel,
            active_ratio,
            on_ratio_selected,
            on_apply,
            on_cancel,
        };

        bar.setup_signals();
        bar
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible);
        if visible {
            self.set_active_ratio(CropRatio::Freeform);
        }
    }

    pub fn set_active_ratio(&self, ratio: CropRatio) {
        *self.active_ratio.borrow_mut() = ratio;
        self.btn_freeform.remove_css_class("active-ratio");
        self.btn_square.remove_css_class("active-ratio");
        self.btn_16_9.remove_css_class("active-ratio");

        match ratio {
            CropRatio::Freeform => self.btn_freeform.add_css_class("active-ratio"),
            CropRatio::Square => self.btn_square.add_css_class("active-ratio"),
            CropRatio::SixteenNine => self.btn_16_9.add_css_class("active-ratio"),
            _ => {}
        }
    }

    fn setup_signals(&self) {
        let bar_free = self.clone();
        self.btn_freeform.connect_clicked(move |_| {
            bar_free.set_active_ratio(CropRatio::Freeform);
            if let Some(ref cb) = *bar_free.on_ratio_selected.borrow() {
                cb(CropRatio::Freeform);
            }
        });

        let bar_sq = self.clone();
        self.btn_square.connect_clicked(move |_| {
            bar_sq.set_active_ratio(CropRatio::Square);
            if let Some(ref cb) = *bar_sq.on_ratio_selected.borrow() {
                cb(CropRatio::Square);
            }
        });

        let bar_16_9 = self.clone();
        self.btn_16_9.connect_clicked(move |_| {
            bar_16_9.set_active_ratio(CropRatio::SixteenNine);
            if let Some(ref cb) = *bar_16_9.on_ratio_selected.borrow() {
                cb(CropRatio::SixteenNine);
            }
        });

        let cb_apply = self.on_apply.clone();
        self.btn_apply.connect_clicked(move |_| {
            if let Some(ref cb) = *cb_apply.borrow() {
                cb();
            }
        });

        let cb_cancel = self.on_cancel.clone();
        self.btn_cancel.connect_clicked(move |_| {
            if let Some(ref cb) = *cb_cancel.borrow() {
                cb();
            }
        });
    }

    pub fn connect_ratio_selected<F: Fn(CropRatio) + 'static>(&self, callback: F) {
        *self.on_ratio_selected.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_apply<F: Fn() + 'static>(&self, callback: F) {
        *self.on_apply.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_cancel<F: Fn() + 'static>(&self, callback: F) {
        *self.on_cancel.borrow_mut() = Some(Box::new(callback));
    }
}
