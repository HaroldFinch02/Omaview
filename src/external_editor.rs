use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box, Button, CheckButton, Entry, Image, Label, Orientation, Popover, ScrolledWindow, Separator};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub editor: EditorConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            editor: EditorConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    #[serde(default)]
    pub preferred_raw_editor: Option<String>,
    #[serde(default)]
    pub always_launch_default: bool,
    #[serde(default)]
    pub custom_command: Option<String>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            preferred_raw_editor: None,
            always_launch_default: false,
            custom_command: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEditorOption {
    pub id: String,
    pub display_name: String,
    pub binary: String,
    pub description: String,
    pub icon_name: String,
    pub is_installed: bool,
    pub is_pro_raw: bool,
    pub desktop_id: Option<String>,
}

pub fn config_path() -> PathBuf {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("omaview/config.toml");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/omaview/config.toml");
    }
    PathBuf::from("config.toml")
}

pub fn load_config() -> AppConfig {
    let p = config_path();
    if p.exists() {
        if let Ok(content) = std::fs::read_to_string(&p) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&content) {
                return cfg;
            }
        }
    }
    AppConfig::default()
}

pub fn save_config(config: &AppConfig) -> Result<(), String> {
    let p = config_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| format!("Serialization error: {}", e))?;
    std::fs::write(&p, toml_str)
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    Ok(())
}

pub fn check_binary_in_path(bin: &str) -> Result<PathBuf, ()> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(())
}

/// Detects all RAW and photo editors installed on the system, ordered by priority.
pub fn detect_all_raw_editors() -> Vec<RawEditorOption> {
    let mut list = Vec::new();

    // 1. Pro RAW Developers
    let pro_editors = [
        ("darktable", "Darktable", "Pro non-destructive RAW developer & workflow", "darktable"),
        ("rawtherapee", "RawTherapee", "Advanced RAW photo processing system", "rawtherapee"),
        ("digikam", "digiKam", "Professional photo management & RAW editor", "digikam"),
        ("ansel", "Ansel", "Darktable fork focused on color science", "ansel"),
        ("ART", "ART", "Another RawTherapee RAW developer", "ART"),
        ("vkdt", "VKDT", "GPU-accelerated Vulkan RAW developer", "camera-photo-symbolic"),
        ("luminance-hdr", "Luminance HDR", "HDR workflow and tone mapping", "luminance-hdr"),
    ];

    for (bin, name, desc, icon) in pro_editors {
        if check_binary_in_path(bin).is_ok() {
            list.push(RawEditorOption {
                id: bin.to_string(),
                display_name: name.to_string(),
                binary: bin.to_string(),
                description: desc.to_string(),
                icon_name: icon.to_string(),
                is_installed: true,
                is_pro_raw: true,
                desktop_id: None,
            });
        }
    }

    // 2. Popular Graphic / Image Editors
    let general_editors = [
        ("gimp", "GIMP", "GNU Image Manipulation Program", "gimp"),
        ("krita", "Krita", "Digital painting and photo retouching", "krita"),
        ("pinta", "Pinta", "Simple image drawing and editing", "pinta"),
        ("imv", "imv", "Lightweight image and RAW viewer", "image-x-generic-symbolic"),
    ];

    for (bin, name, desc, icon) in general_editors {
        if check_binary_in_path(bin).is_ok() && !list.iter().any(|e| e.binary == bin) {
            list.push(RawEditorOption {
                id: bin.to_string(),
                display_name: name.to_string(),
                binary: bin.to_string(),
                description: desc.to_string(),
                icon_name: icon.to_string(),
                is_installed: true,
                is_pro_raw: false,
                desktop_id: None,
            });
        }
    }

    // 3. System Desktop Apps discovered via GIO
    for mime in ["image/x-sony-arw", "image/tiff", "image/x-raw"] {
        let apps = gio::AppInfo::all_for_type(mime);
        for app in apps {
            let exe_path = app.executable();
            let bin_name = exe_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            const IGNORED_BINARIES: &[&str] = &[
                "omaview",
                "evince",
                "evince-previewer",
                "gnome-disks",
                "gnome-disk-image-mounter",
                "gnome-disk-image-writer",
                "brave",
                "brave-origin",
                "chromium",
                "firefox",
                "libreoffice",
            ];
            if bin_name.is_empty() || IGNORED_BINARIES.contains(&bin_name) {
                continue;
            }
            let app_id_str = app.id().map(|s| s.to_string());
            if list.iter().any(|e| e.binary == bin_name || app_id_str.as_ref() == Some(&e.id)) {
                continue;
            }
            let display_name = app.name().to_string();
            let desc = app
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|| format!("Installed desktop application ({})", bin_name));
            let icon_str = app
                .icon()
                .and_then(|i| i.to_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "applications-graphics-symbolic".to_string());

            list.push(RawEditorOption {
                id: app_id_str.clone().unwrap_or_else(|| bin_name.to_string()),
                display_name,
                binary: bin_name.to_string(),
                description: desc,
                icon_name: icon_str,
                is_installed: true,
                is_pro_raw: false,
                desktop_id: app_id_str,
            });
        }
    }

    // 4. System Default (xdg-open)
    if check_binary_in_path("xdg-open").is_ok() && !list.iter().any(|e| e.binary == "xdg-open") {
        list.push(RawEditorOption {
            id: "xdg-open".to_string(),
            display_name: "System Default App".to_string(),
            binary: "xdg-open".to_string(),
            description: "Open with default system handler (xdg-open)".to_string(),
            icon_name: "system-run-symbolic".to_string(),
            is_installed: true,
            is_pro_raw: false,
            desktop_id: None,
        });
    }

    list
}

/// Returns true if a dedicated pro RAW developer, general image editor with RAW support,
/// or user-configured custom command is available on the system.
pub fn has_raw_compatible_app() -> bool {
    let config = load_config();
    if config.editor.custom_command.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
        return true;
    }
    if let Some(ref pref) = config.editor.preferred_raw_editor {
        if pref != "xdg-open" && check_binary_in_path(pref).is_ok() {
            return true;
        }
    }
    let editors = detect_all_raw_editors();
    editors.iter().any(|e| e.is_pro_raw || e.id == "gimp" || e.id == "krita")
}

/// Resolves the user's preferred editor, or returns the best available editor if not configured.
pub fn get_preferred_editor() -> Option<RawEditorOption> {
    let config = load_config();
    let editors = detect_all_raw_editors();

    if let Some(ref pref_id) = config.editor.preferred_raw_editor {
        if let Some(found) = editors.iter().find(|e| &e.id == pref_id || &e.binary == pref_id) {
            return Some(found.clone());
        }
    }

    // Default to first available (pro editors first, then general, then xdg-open)
    editors.into_iter().next()
}

/// Spawns the specified editor with the given file path in the background.
pub fn launch_raw_editor(
    editor: &RawEditorOption,
    path: &Path,
    custom_cmd: Option<&str>,
) -> Result<String, String> {
    if editor.id == "custom" {
        let cmd = custom_cmd.unwrap_or("").trim();
        if cmd.is_empty() {
            return Err("No custom command specified".to_string());
        }
        let escaped_path = path.to_string_lossy().replace('\'', "'\\''");
        let formatted = if cmd.contains("%f") || cmd.contains("%F") {
            cmd.replace("%f", &format!("'{}'", escaped_path))
                .replace("%F", &format!("'{}'", escaped_path))
        } else {
            format!("{} '{}'", cmd, escaped_path)
        };

        Command::new("sh")
            .arg("-c")
            .arg(&formatted)
            .spawn()
            .map_err(|e| format!("Failed to launch custom command: {}", e))?;
        return Ok("Custom Command".to_string());
    }

    // Try desktop launch if desktop_id exists
    if let Some(ref d_id) = editor.desktop_id {
        let all = gio::AppInfo::all();
        if let Some(app) = all.into_iter().find(|a| a.id().as_deref() == Some(d_id.as_str())) {
            let file = gio::File::for_path(path);
            if app.launch(&[file], None::<&gio::AppLaunchContext>).is_ok() {
                return Ok(editor.display_name.clone());
            }
        }
    }

    // Standard binary launch
    Command::new(&editor.binary)
        .arg(path)
        .spawn()
        .map_err(|e| format!("Failed to launch {}: {}", editor.display_name, e))?;
    Ok(editor.display_name.clone())
}

/// Creates a glassmorphic Popover Chooser for external RAW/image editors.
pub fn create_raw_chooser_popover<F, G>(
    current_path: &Path,
    on_launched: F,
    on_preference_changed: G,
) -> Popover
where
    F: Fn(&str) + Clone + 'static,
    G: Fn() + Clone + 'static,
{
    let popover = Popover::new();
    popover.add_css_class("raw-chooser-popover");
    popover.set_has_arrow(true);

    let vbox = Box::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(14);
    vbox.set_margin_end(14);
    vbox.set_size_request(340, -1);

    // Header
    let header_box = Box::new(Orientation::Vertical, 2);
    let title = Label::new(None);
    title.set_markup("<span weight='bold' size='large'>Open RAW Photo With…</span>");
    title.set_halign(gtk4::Align::Start);
    header_box.append(&title);

    let filename = current_path.file_name().and_then(|f| f.to_str()).unwrap_or("RAW Photo");
    let ext_badge = if crate::raw_loader::is_raw_image(current_path) {
        crate::raw_loader::raw_format_badge(current_path).to_string()
    } else {
        "Image".to_string()
    };
    let subtitle = Label::new(Some(&format!("{} • {}", filename, ext_badge)));
    subtitle.set_halign(gtk4::Align::Start);
    subtitle.add_css_class("dim-label");
    header_box.append(&subtitle);
    vbox.append(&header_box);

    let sep1 = Separator::new(Orientation::Horizontal);
    sep1.set_margin_top(4);
    sep1.set_margin_bottom(6);
    vbox.append(&sep1);

    // Dynamic list container for detected editors
    let list_box = Box::new(Orientation::Vertical, 4);

    let editors = detect_all_raw_editors();
    let config = Rc::new(RefCell::new(load_config()));
    let target_path = current_path.to_path_buf();

    // Rebuild helper with self-reference
    let rebuild_cell = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
    {
        let list_box = list_box.clone();
        let editors = editors.clone();
        let config = config.clone();
        let popover_weak = popover.downgrade();
        let on_launched = on_launched.clone();
        let on_preference_changed = on_preference_changed.clone();
        let target_path = target_path.clone();
        let rebuild_cell_clone = rebuild_cell.clone();

        let rebuild_fn = Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let pref_id = config.borrow().editor.preferred_raw_editor.clone();

            for editor in &editors {
                let is_default = match &pref_id {
                    Some(id) => id == &editor.id || id == &editor.binary,
                    None => editors.first().map(|e| &e.id) == Some(&editor.id),
                };

                let row = Box::new(Orientation::Horizontal, 6);
                row.add_css_class("raw-chooser-item");
                if is_default {
                    row.add_css_class("is-default");
                }

                let btn_launch = Button::new();
                btn_launch.set_has_frame(false);
                btn_launch.set_hexpand(true);
                btn_launch.add_css_class("raw-chooser-launch-btn");

                let btn_content = Box::new(Orientation::Horizontal, 10);
                btn_content.set_halign(gtk4::Align::Start);

                let icon = Image::from_icon_name(&editor.icon_name);
                icon.set_pixel_size(24);
                icon.set_valign(gtk4::Align::Center);
                btn_content.append(&icon);

                let text_box = Box::new(Orientation::Vertical, 1);
                text_box.set_halign(gtk4::Align::Start);

                let name_box = Box::new(Orientation::Horizontal, 6);
                let name_lbl = Label::new(None);
                name_lbl.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(&editor.display_name)));
                name_lbl.set_halign(gtk4::Align::Start);
                name_box.append(&name_lbl);

                if is_default {
                    let default_badge = Label::new(Some("★ Default"));
                    default_badge.add_css_class("raw-default-badge");
                    name_box.append(&default_badge);
                }

                let desc_lbl = Label::new(Some(&editor.description));
                desc_lbl.set_halign(gtk4::Align::Start);
                desc_lbl.add_css_class("dim-label");
                desc_lbl.add_css_class("raw-chooser-desc");

                text_box.append(&name_box);
                text_box.append(&desc_lbl);
                btn_content.append(&text_box);
                btn_launch.set_child(Some(&btn_content));

                let ed_clone = editor.clone();
                let pop_weak = popover_weak.clone();
                let path_clone = target_path.clone();
                let on_l = on_launched.clone();
                btn_launch.connect_clicked(move |_| {
                    if let Ok(name) = launch_raw_editor(&ed_clone, &path_clone, None) {
                        on_l(&name);
                    }
                    if let Some(pop) = pop_weak.upgrade() {
                        pop.popdown();
                    }
                });

                row.append(&btn_launch);

                let star_btn = Button::from_icon_name(if is_default {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
                star_btn.set_has_frame(false);
                star_btn.set_valign(gtk4::Align::Center);
                star_btn.add_css_class("raw-star-btn");
                if is_default {
                    star_btn.add_css_class("is-active");
                    star_btn.set_tooltip_text(Some("Current default editor"));
                } else {
                    star_btn.set_tooltip_text(Some("Set as default editor"));
                }

                let ed_id = editor.id.clone();
                let cfg_ref = config.clone();
                let on_p = on_preference_changed.clone();
                let rb_cell = rebuild_cell_clone.clone();

                star_btn.connect_clicked(move |_| {
                    cfg_ref.borrow_mut().editor.preferred_raw_editor = Some(ed_id.clone());
                    let _ = save_config(&cfg_ref.borrow());
                    on_p();
                    if let Some(ref rb) = *rb_cell.borrow() {
                        rb();
                    }
                });

                row.append(&star_btn);
                list_box.append(&row);
            }
        });

        *rebuild_cell.borrow_mut() = Some(rebuild_fn);
    }

    if let Some(ref rb) = *rebuild_cell.borrow() {
        rb();
    }

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&list_box));
    scrolled.set_max_content_height(280);
    scrolled.set_propagate_natural_height(true);
    vbox.append(&scrolled);

    // Custom Command section
    let custom_box = Box::new(Orientation::Vertical, 4);
    custom_box.set_margin_top(4);

    let custom_header = Box::new(Orientation::Horizontal, 6);
    let custom_icon = Image::from_icon_name("utilities-terminal-symbolic");
    custom_icon.set_pixel_size(16);
    let custom_title = Label::new(Some("Custom Command"));
    custom_title.set_markup("<b>Custom Command</b>");
    custom_title.set_halign(gtk4::Align::Start);
    custom_header.append(&custom_icon);
    custom_header.append(&custom_title);
    custom_box.append(&custom_header);

    let entry_row = Box::new(Orientation::Horizontal, 6);
    let entry = Entry::new();
    entry.set_hexpand(true);
    entry.add_css_class("raw-custom-entry");
    entry.set_placeholder_text(Some("e.g. gimp %f or raw-script.sh"));
    if let Some(ref c) = config.borrow().editor.custom_command {
        entry.set_text(c);
    }

    let btn_run = Button::with_label("Run");
    btn_run.add_css_class("suggested-action");
    btn_run.add_css_class("raw-run-btn");

    let run_custom = {
        let entry = entry.clone();
        let cfg_ref = config.clone();
        let target_path = target_path.clone();
        let pop_weak = popover.downgrade();
        let on_l = on_launched.clone();

        Rc::new(move || {
            let cmd_text = entry.text().trim().to_string();
            if cmd_text.is_empty() {
                return;
            }
            cfg_ref.borrow_mut().editor.custom_command = Some(cmd_text.clone());
            let _ = save_config(&cfg_ref.borrow());

            let custom_opt = RawEditorOption {
                id: "custom".to_string(),
                display_name: "Custom Command".to_string(),
                binary: "sh".to_string(),
                description: cmd_text.clone(),
                icon_name: "utilities-terminal-symbolic".to_string(),
                is_installed: true,
                is_pro_raw: false,
                desktop_id: None,
            };

            if let Ok(name) = launch_raw_editor(&custom_opt, &target_path, Some(&cmd_text)) {
                on_l(&name);
            }
            if let Some(pop) = pop_weak.upgrade() {
                pop.popdown();
            }
        })
    };

    let rc1 = run_custom.clone();
    btn_run.connect_clicked(move |_| rc1());

    let rc2 = run_custom.clone();
    entry.connect_activate(move |_| rc2());

    entry_row.append(&entry);
    entry_row.append(&btn_run);
    custom_box.append(&entry_row);
    vbox.append(&custom_box);

    // Separator & Preferences
    let sep2 = Separator::new(Orientation::Horizontal);
    sep2.set_margin_top(6);
    sep2.set_margin_bottom(6);
    vbox.append(&sep2);

    let check_always = CheckButton::with_label("Always open with default on button click");
    check_always.set_active(config.borrow().editor.always_launch_default);
    let cfg_toggle = config.clone();
    let on_pref_toggle = on_preference_changed.clone();
    check_always.connect_toggled(move |chk| {
        cfg_toggle.borrow_mut().editor.always_launch_default = chk.is_active();
        let _ = save_config(&cfg_toggle.borrow());
        on_pref_toggle();
    });
    vbox.append(&check_always);

    // Information hint if no pro RAW developers are installed
    let has_pro = editors.iter().any(|e| e.is_pro_raw);
    if !has_pro {
        let tip_box = Box::new(Orientation::Horizontal, 6);
        tip_box.set_margin_top(4);
        let tip_icon = Image::from_icon_name("dialog-information-symbolic");
        tip_icon.set_pixel_size(16);
        tip_icon.add_css_class("dim-label");
        let tip_lbl = Label::new(Some("Pro RAW tools (Darktable, RawTherapee) can be installed via pacman."));
        tip_lbl.set_wrap(true);
        tip_lbl.set_max_width_chars(36);
        tip_lbl.add_css_class("dim-label");
        tip_box.append(&tip_icon);
        tip_box.append(&tip_lbl);
        vbox.append(&tip_box);
    }

    popover.set_child(Some(&vbox));
    popover
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_binary_in_path() {
        assert!(check_binary_in_path("sh").is_ok());
        assert!(check_binary_in_path("non_existent_binary_xyz_123").is_err());
    }

    #[test]
    fn test_detect_all_raw_editors() {
        let editors = detect_all_raw_editors();
        for ed in &editors {
            println!("  -> [{}] {} ({}) - is_pro: {}", ed.id, ed.display_name, ed.binary, ed.is_pro_raw);
        }
        if check_binary_in_path("xdg-open").is_ok() {
            assert!(!editors.is_empty());
            assert!(editors.iter().any(|e| e.binary == "xdg-open" || e.id == "xdg-open"));
        }
    }

    #[test]
    fn test_config_save_load() {
        let temp_dir = std::env::temp_dir().join(format!("omaview_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("test_config.toml");

        let mut cfg = AppConfig::default();
        cfg.editor.preferred_raw_editor = Some("darktable".to_string());
        cfg.editor.always_launch_default = true;
        cfg.editor.custom_command = Some("darktable %f".to_string());

        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        std::fs::write(&test_file, &toml_str).unwrap();

        let read_back = std::fs::read_to_string(&test_file).unwrap();
        let loaded: AppConfig = toml::from_str(&read_back).unwrap();

        assert_eq!(loaded.editor.preferred_raw_editor.as_deref(), Some("darktable"));
        assert!(loaded.editor.always_launch_default);
        assert_eq!(loaded.editor.custom_command.as_deref(), Some("darktable %f"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_custom_command_formatting() {
        let path = Path::new("/tmp/test photo.arw");
        let cmd = "my-tool %f --flag";
        let escaped_path = path.to_string_lossy().replace('\'', "'\\''");
        let formatted = cmd.replace("%f", &format!("'{}'", escaped_path));
        assert_eq!(formatted, "my-tool '/tmp/test photo.arw' --flag");
    }

    #[test]
    fn test_get_preferred_editor_fallback() {
        let pref = get_preferred_editor();
        if check_binary_in_path("xdg-open").is_ok() {
            assert!(pref.is_some());
            let ed = pref.unwrap();
            assert!(!ed.display_name.is_empty());
            assert!(!ed.binary.is_empty());
        }
    }

    #[test]
    fn test_has_raw_compatible_app() {
        let has = has_raw_compatible_app();
        let config = load_config();
        let custom_set = config.editor.custom_command.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false);
        let pref_set = config.editor.preferred_raw_editor.as_ref().map(|s| s != "xdg-open" && check_binary_in_path(s).is_ok()).unwrap_or(false);
        let editors = detect_all_raw_editors();
        let expected = custom_set || pref_set || editors.iter().any(|e| e.is_pro_raw || e.id == "gimp" || e.id == "krita");
        assert_eq!(has, expected);
    }
}
