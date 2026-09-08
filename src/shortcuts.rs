use gtk4::prelude::*;
use gtk4::{EventControllerKey, Window};
use gdk4::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AppAction {
    Home,
    NextImage,
    PrevImage,
    ZoomIn,
    ZoomOut,
    Zoom100,
    ZoomFit,
    RotateCw,
    RotateCcw,
    FlipH,
    FlipV,
    ToggleCrop,
    ApplyCrop,
    CropRatioFreeform,
    CropRatioSquare,
    CropRatio16_9,
    ToggleAdjustments,
    ToggleInfo,
    Trash,
    ToggleZen,
    ToggleFilmstrip,
    Save,
    ShowHelp,
    ToggleFullscreen,
    Escape,
    Quit,
}

pub fn create_key_controller<F: Fn(AppAction) -> glib::Propagation + 'static>(
    handler: F,
) -> EventControllerKey {
    let controller = EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    controller.connect_key_pressed(move |_, key, _keycode, state| {
        let is_ctrl = state.contains(gdk4::ModifierType::CONTROL_MASK);
        let is_shift = state.contains(gdk4::ModifierType::SHIFT_MASK);

        let action = match key {
            Key::Home => Some(AppAction::Home),
            Key::j | Key::Right | Key::space => Some(AppAction::NextImage),
            Key::k | Key::Left | Key::BackSpace => Some(AppAction::PrevImage),

            Key::plus | Key::equal | Key::KP_Add => Some(AppAction::ZoomIn),
            Key::Up if is_ctrl => Some(AppAction::ZoomIn),

            Key::minus | Key::underscore | Key::KP_Subtract => Some(AppAction::ZoomOut),
            Key::Down if is_ctrl => Some(AppAction::ZoomOut),

            Key::_0 | Key::KP_0 => Some(AppAction::Zoom100),
            Key::_1 | Key::KP_1 => Some(AppAction::CropRatioSquare),
            Key::_2 | Key::KP_2 => Some(AppAction::CropRatio16_9),
            Key::f | Key::F => Some(AppAction::ZoomFit),

            Key::r if !is_shift => Some(AppAction::RotateCw),
            Key::R | Key::r if is_shift => Some(AppAction::RotateCcw),

            Key::h | Key::H => Some(AppAction::FlipH),
            Key::v | Key::V => Some(AppAction::FlipV),

            Key::c | Key::C => Some(AppAction::ToggleCrop),
            Key::Return | Key::KP_Enter => Some(AppAction::ApplyCrop),

            Key::a | Key::A | Key::e | Key::E => Some(AppAction::ToggleAdjustments),
            Key::i | Key::I => Some(AppAction::ToggleInfo),

            Key::Delete | Key::d | Key::D => Some(AppAction::Trash),

            Key::Tab | Key::z | Key::Z => Some(AppAction::ToggleZen),
            Key::g | Key::G => Some(AppAction::ToggleFilmstrip),

            Key::s | Key::S => Some(AppAction::Save),

            Key::question | Key::slash if is_shift => Some(AppAction::ShowHelp),
            Key::F1 => Some(AppAction::ShowHelp),
            Key::F11 => Some(AppAction::ToggleFullscreen),

            Key::Escape => Some(AppAction::Escape),
            Key::q | Key::Q => Some(AppAction::Quit),

            _ => None,
        };

        if let Some(act) = action {
            handler(act)
        } else {
            glib::Propagation::Proceed
        }
    });

    controller
}

pub fn show_shortcuts_dialog(parent: &impl IsA<Window>) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Keyboard Shortcuts")
        .default_width(380)
        .default_height(480)
        .build();

    dialog.add_css_class("floating-pill");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let title = gtk4::Label::new(None);
    title.set_markup("<span weight='bold' size='large'>Omaview Shortcuts</span>");
    title.set_halign(gtk4::Align::Center);
    vbox.append(&title);

    let shortcuts = [
        ("Home", "Return to Albums / Home screen"),
        ("j / → / Space", "Next image"),
        ("k / ← / Backspace", "Previous image"),
        ("+ / = / Ctrl+↑", "Zoom in"),
        ("- / Ctrl+↓", "Zoom out"),
        ("0", "100% Actual size"),
        ("f", "Fit to window"),
        ("r / R", "Rotate CW / CCW"),
        ("h / v", "Flip Horizontal / Vertical"),
        ("c / Enter", "Crop mode / Apply crop"),
        ("a / e", "Adjustments drawer"),
        ("i", "Image details / EXIF"),
        ("Delete / d", "Move to trash (gio safe)"),
        ("Tab / z", "Zen mode (hide chrome)"),
        ("g", "Toggle filmstrip"),
        ("s", "Save changes"),
        ("F11", "Toggle fullscreen"),
        ("q / Esc", "Quit / Close drawer"),
    ];

    let grid = gtk4::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(8);

    for (row, (key, desc)) in shortcuts.iter().enumerate() {
        let key_lbl = gtk4::Label::new(Some(key));
        key_lbl.set_halign(gtk4::Align::Start);
        key_lbl.add_css_class("dim-label");

        let desc_lbl = gtk4::Label::new(Some(desc));
        desc_lbl.set_halign(gtk4::Align::Start);

        grid.attach(&key_lbl, 0, row as i32, 1, 1);
        grid.attach(&desc_lbl, 1, row as i32, 1, 1);
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&grid));
    scrolled.set_vexpand(true);
    vbox.append(&scrolled);

    let close_btn = gtk4::Button::with_label("Close");
    let dialog_weak = dialog.downgrade();
    close_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.close();
        }
    });
    close_btn.set_halign(gtk4::Align::Center);
    vbox.append(&close_btn);

    dialog.set_child(Some(&vbox));
    dialog.present();
}
