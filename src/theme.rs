use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use serde::Deserialize;
use notify::{Watcher, RecursiveMode, RecommendedWatcher};

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct OmarchyColors {
    pub mode: Option<String>,
    pub accent: Option<String>,
    pub selection: Option<String>,
    pub muted: Option<String>,
    pub background: Option<String>,
    pub dark_background: Option<String>,
    pub darker_background: Option<String>,
    pub lighter_background: Option<String>,
    pub foreground: Option<String>,
    pub dark_foreground: Option<String>,
    pub light_foreground: Option<String>,
    pub bright_foreground: Option<String>,
    pub red: Option<String>,
    pub yellow: Option<String>,
    pub green: Option<String>,
    pub cyan: Option<String>,
    pub blue: Option<String>,
    pub magenta: Option<String>,
}

pub fn parse_hex_color(hex: &str) -> Option<(f64, f64, f64)> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()? as f64 / 255.0;
        let g = u8::from_str_radix(&s[2..4], 16).ok()? as f64 / 255.0;
        let b = u8::from_str_radix(&s[4..6], 16).ok()? as f64 / 255.0;
        Some((r, g, b))
    } else {
        None
    }
}

pub fn theme_file_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let p1 = PathBuf::from(&home).join(".local/state/omarchy/current/theme/colors.toml");
        if p1.exists() {
            return Some(p1);
        }
        let p2 = PathBuf::from(&home).join(".config/omarchy/current/theme/colors.toml");
        if p2.exists() {
            return Some(p2);
        }
    }
    None
}

pub fn load_current_theme() -> OmarchyColors {
    if let Some(path) = theme_file_path() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(colors) = toml::from_str::<OmarchyColors>(&content) {
                return colors;
            }
        }
    }
    OmarchyColors {
        mode: Some("dark".into()),
        accent: Some("#7aa2f7".into()),
        selection: Some("#292e42".into()),
        muted: Some("#414868".into()),
        background: Some("#1a1b26".into()),
        dark_background: Some("#13141c".into()),
        darker_background: Some("#0e0e14".into()),
        lighter_background: Some("#24283b".into()),
        foreground: Some("#a9b1d6".into()),
        dark_foreground: Some("#565f89".into()),
        light_foreground: Some("#b4bee6".into()),
        bright_foreground: Some("#c0caf5".into()),
        red: Some("#f7768e".into()),
        yellow: Some("#e0af68".into()),
        green: Some("#9ece6a".into()),
        cyan: Some("#449dab".into()),
        blue: Some("#7aa2f7".into()),
        magenta: Some("#ad8ee6".into()),
    }
}

pub fn generate_css(colors: &OmarchyColors) -> String {
    let accent = colors.accent.as_deref().unwrap_or("#7aa2f7");
    let bg = colors.background.as_deref().unwrap_or("#1a1b26");
    let dark_bg = colors.dark_background.as_deref().unwrap_or("#13141c");
    let lighter_bg = colors.lighter_background.as_deref().unwrap_or("#24283b");
    let fg = colors.foreground.as_deref().unwrap_or("#a9b1d6");
    let muted = colors.muted.as_deref().unwrap_or("#414868");
    let selection = colors.selection.as_deref().unwrap_or("#292e42");
    let red = colors.red.as_deref().unwrap_or("#f7768e");

    let (ar, ag, ab) = parse_hex_color(accent).unwrap_or((0.48, 0.64, 0.97));
    let (dbr, dbg, dbb) = parse_hex_color(dark_bg).unwrap_or((0.07, 0.08, 0.11));
    let (rr, rg, rb) = parse_hex_color(red).unwrap_or((0.97, 0.46, 0.56));

    let ar_255 = (ar * 255.0).round() as u8;
    let ag_255 = (ag * 255.0).round() as u8;
    let ab_255 = (ab * 255.0).round() as u8;

    let dbr_255 = (dbr * 255.0).round() as u8;
    let dbg_255 = (dbg * 255.0).round() as u8;
    let dbb_255 = (dbb * 255.0).round() as u8;

    let rr_255 = (rr * 255.0).round() as u8;
    let rg_255 = (rg * 255.0).round() as u8;
    let rb_255 = (rb * 255.0).round() as u8;

    format!(
        r#"
:root {{
  @define-color destructive_color {red};
  @define-color destructive_bg_color {red};
  @define-color destructive_fg_color #ffffff;
  --accent: {accent};
  --accent-r: {ar};
  --accent-g: {ag};
  --accent-b: {ab};
  --bg: {bg};
  --dark-bg: {dark_bg};
  --lighter-bg: {lighter_bg};
  --fg: {fg};
  --muted: {muted};
  --selection: {selection};
  --window-bg: {dark_bg};
  --pill-bg: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.80);
  --pill-border: rgba(255, 255, 255, 0.18);
}}

/* Frosted translucent window background - desktop wallpaper slightly shows through with compositor blur */
window.omaview-window,
window.omaview-window.background {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.80);
  background-image: none;
  color: var(--fg);
}}

window.omaview-window > contents,
stack,
stack > widget,
.omaview-main-box,
.home-screen,
scrolledwindow,
scrolledwindow > viewport {{
  background-color: transparent;
}}

.floating-pill {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.82);
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 14px;
  box-shadow: 0 10px 32px rgba(0, 0, 0, 0.45);
  padding: 4px 6px;
}}

.floating-pill button {{
  min-width: 32px;
  min-height: 32px;
  padding: 5px;
  border-radius: 8px;
  border: 1.5px solid transparent;
  background: transparent;
  color: rgba(240, 243, 250, 0.82);
  transition: all 120ms ease;
}}

.floating-pill button:hover {{
  background-color: rgba(255, 255, 255, 0.14);
  color: #ffffff;
}}

.floating-pill button:active, .floating-pill button.active-highlight {{
  border: 1.5px solid var(--accent);
  color: var(--accent);
  background: rgba({ar_255}, {ag_255}, {ab_255}, 0.12);
}}

.zoom-indicator-box {{
  min-width: 26px;
  min-height: 26px;
  border-radius: 13px;
  border: 1.5px solid rgba(255, 255, 255, 0.35);
  padding: 0;
}}

.zoom-indicator-box:hover {{
  border-color: var(--accent);
}}

.zoom-indicator-dot {{
  min-width: 8px;
  min-height: 8px;
  border-radius: 4px;
  background-color: var(--accent);
}}

.nav-chevron-btn {{
  min-width: 36px;
  min-height: 36px;
  border-radius: 18px;
  padding: 4px;
  background-color: rgba(20, 25, 35, 0.45);
  border: 1px solid rgba(255, 255, 255, 0.16);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.35);
  color: rgba(255, 255, 255, 0.85);
  transition: all 120ms ease;
}}

.nav-chevron-btn:hover {{
  color: #ffffff;
  background-color: rgba(255, 255, 255, 0.22);
  border-color: rgba(255, 255, 255, 0.45);
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.45);
}}

.filmstrip-panel {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.58);
  background-image: linear-gradient(180deg, rgba(255, 255, 255, 0.08) 0%, transparent 40%, rgba(255, 255, 255, 0.04) 100%);
  backdrop-filter: blur(16px);
  border-left: 1px solid rgba(255, 255, 255, 0.20);
  box-shadow: inset 1px 0 0 rgba(255, 255, 255, 0.15),
              inset 0 1px 0 rgba(255, 255, 255, 0.35);
}}

.filmstrip-scroll-btn {{
  min-height: 24px;
  background: transparent;
  border: none;
  color: rgba(255, 255, 255, 0.40);
  padding: 2px;
}}

.filmstrip-scroll-btn:hover {{
  color: var(--accent);
  background: rgba(255, 255, 255, 0.10);
}}

.adjustments-panel {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.82);
  background-image: linear-gradient(135deg, rgba(255, 255, 255, 0.18) 0%, rgba(255, 255, 255, 0.02) 100%);
  backdrop-filter: blur(18px);
  border: 1px solid rgba(255, 255, 255, 0.28);
  border-radius: 18px;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65),
              inset 0 1px 0 rgba(255, 255, 255, 0.60),
              inset 0 -1px 0 rgba(255, 255, 255, 0.12);
  padding: 16px;
  color: var(--fg);
}}

.adjustments-panel scale highlight {{
  background-color: var(--accent);
}}

.exif-popover {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.82);
  background-image: linear-gradient(135deg, rgba(255, 255, 255, 0.18) 0%, rgba(255, 255, 255, 0.02) 100%);
  backdrop-filter: blur(18px);
  border: 1px solid rgba(255, 255, 255, 0.28);
  border-radius: 18px;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65),
              inset 0 1px 0 rgba(255, 255, 255, 0.60),
              inset 0 -1px 0 rgba(255, 255, 255, 0.12);
  padding: 14px;
  color: var(--fg);
}}

.home-screen {{
  background-color: transparent;
}}

.home-top-bar {{
  background: transparent;
}}

.home-add-album-btn {{
  padding: 9px 26px;
  border-radius: 9999px;
  background-color: rgba(255, 255, 255, 0.10);
  background-image: linear-gradient(135deg, rgba(255, 255, 255, 0.22) 0%, rgba(255, 255, 255, 0.03) 100%);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(255, 255, 255, 0.32);
  box-shadow: 0 12px 34px rgba(0, 0, 0, 0.40),
              inset 0 1px 0 rgba(255, 255, 255, 0.65),
              inset 0 -1px 0 rgba(255, 255, 255, 0.12);
  color: #ffffff;
}}

.home-add-album-btn:hover {{
  background-color: rgba(255, 255, 255, 0.18);
  border-color: rgba(255, 255, 255, 0.48);
  box-shadow: 0 14px 40px rgba(0, 0, 0, 0.48),
              inset 0 1px 0 rgba(255, 255, 255, 0.80),
              inset 0 -1px 0 rgba(255, 255, 255, 0.15);
  color: #ffffff;
}}

.home-add-btn-label {{
  font-weight: 600;
  font-size: 13.5px;
}}

.album-section {{
  background-color: rgba(255, 255, 255, 0.05);
  background-image: linear-gradient(135deg, rgba(255, 255, 255, 0.12) 0%, rgba(255, 255, 255, 0.01) 70%, rgba(255, 255, 255, 0.05) 100%);
  backdrop-filter: blur(16px);
  border-radius: 20px;
  padding: 18px 22px;
  border: 1px solid rgba(255, 255, 255, 0.24);
  box-shadow: 0 14px 38px rgba(0, 0, 0, 0.35),
              inset 0 1px 0 rgba(255, 255, 255, 0.50),
              inset 0 -1px 0 rgba(255, 255, 255, 0.08);
}}

.album-header {{
  min-height: 32px;
}}

.album-folder-icon {{
  color: var(--accent);
}}

.album-title-label {{
  font-weight: 700;
  font-size: 16px;
  color: var(--fg);
}}

.album-count-badge {{
  font-size: 11px;
  font-weight: 600;
  padding: 3px 10px;
  border-radius: 9999px;
  background-color: rgba(255, 255, 255, 0.08);
  background-image: linear-gradient(135deg, rgba(255, 255, 255, 0.15) 0%, rgba(255, 255, 255, 0.02) 100%);
  border: 1px solid rgba(255, 255, 255, 0.22);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.35);
  color: rgba(255, 255, 255, 0.85);
}}

.album-path-hint {{
  font-size: 11.5px;
  margin-left: 8px;
}}

.album-action-btn {{
  min-width: 28px;
  min-height: 28px;
  padding: 4px;
  border-radius: 8px;
  color: rgba(255, 255, 255, 0.50);
  background: transparent;
  border: none;
}}

.album-action-btn:hover {{
  color: var(--accent);
  background: rgba(255, 255, 255, 0.10);
}}

.album-card {{
  padding: 5px;
  border-radius: 16px;
  background-color: transparent;
  transition: all 180ms ease;
}}

.album-card:hover {{
  background-color: rgba(255, 255, 255, 0.08);
}}

.album-card-label {{
  font-size: 11.5px;
  color: rgba(255, 255, 255, 0.75);
}}

.top-bar-box {{
  background: transparent;
}}

.nav-home-btn {{
  min-width: 36px;
  min-height: 34px;
  padding: 5px 10px;
  border-radius: 10px;
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.75);
  border: 1px solid rgba(255, 255, 255, 0.18);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  color: rgba(230, 235, 245, 0.90);
  transition: all 120ms ease;
}}

.nav-home-btn:hover {{
  color: var(--accent);
  background-color: rgba(255, 255, 255, 0.15);
  border-color: rgba(255, 255, 255, 0.35);
}}

.metadata-pill {{
  min-height: 34px;
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.75);
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 10px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  padding: 5px 14px;
  color: rgba(240, 243, 250, 0.95);
  font-weight: 500;
  font-size: 12px;
}}

/* Crop pill and aspect ratio options at top center */
.crop-pill {{
  background-color: rgba({dbr_255}, {dbg_255}, {dbb_255}, 0.82);
  border: 1px solid rgba(255, 255, 255, 0.20);
  border-radius: 9999px;
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.50);
  padding: 4px 10px;
}}

.crop-icon {{
  color: var(--accent);
  margin-right: 2px;
  margin-left: 4px;
}}

.crop-title-lbl {{
  font-weight: 600;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.90);
  margin-right: 4px;
}}

.crop-ratio-btn {{
  padding: 4px 12px;
  min-height: 28px;
  border-radius: 9999px;
  font-size: 12px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.80);
  background: transparent;
  border: 1px solid transparent;
  transition: all 120ms ease;
}}

.crop-ratio-btn:hover {{
  background-color: rgba(255, 255, 255, 0.14);
  color: #ffffff;
}}

.crop-ratio-btn.active-ratio {{
  background-color: var(--accent);
  color: {dark_bg};
  border-color: var(--accent);
  font-weight: 700;
}}

.crop-apply-btn {{
  padding: 4px 14px;
  min-height: 28px;
  border-radius: 9999px;
  font-size: 12px;
  font-weight: 700;
  background-color: rgba({ar_255}, {ag_255}, {ab_255}, 0.35);
  border: 1px solid var(--accent);
  color: #ffffff;
  transition: all 120ms ease;
}}

.crop-apply-btn:hover {{
  background-color: var(--accent);
  color: {dark_bg};
}}

.crop-cancel-btn {{
  min-width: 28px;
  min-height: 28px;
  padding: 4px;
  border-radius: 14px;
  color: rgba(255, 255, 255, 0.60);
  background: transparent;
}}

.crop-cancel-btn:hover {{
  color: #ffffff;
  background-color: rgba(255, 255, 255, 0.16);
}}

/* Save Changes Pill & Toolbar Button */
.save-pill-btn {{
  min-height: 34px;
  padding: 0 16px;
  border-radius: 10px;
  background-color: rgba({ar_255}, {ag_255}, {ab_255}, 0.35);
  border: 1px solid var(--accent);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35), inset 0 1px 0 rgba(255, 255, 255, 0.35);
  backdrop-filter: blur(16px);
  color: #ffffff;
  font-weight: 600;
  font-size: 13px;
  letter-spacing: 0.2px;
  transition: all 150ms ease;
}}

.save-pill-btn:hover {{
  background-color: var(--accent);
  color: {dark_bg};
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.50);
}}

.floating-pill button.toolbar-save-btn.active-save-highlight {{
  background-color: rgba({ar_255}, {ag_255}, {ab_255}, 0.35);
  border: 1px solid var(--accent);
  color: var(--accent);
}}

.floating-pill button.toolbar-save-btn.active-save-highlight:hover {{
  background-color: var(--accent);
  color: {dark_bg};
}}

/* Dialog Action Buttons (Overwrite & Destructive Actions) - bright, solid presence matching suggested-action */
button.destructive-action,
dialog button.destructive-action,
window.dialog button.destructive-action,
.alert-dialog button.destructive-action {{
  background-color: {red};
  color: #ffffff;
  font-weight: 600;
  opacity: 1.0;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.30);
}}

button.destructive-action:hover,
dialog button.destructive-action:hover,
window.dialog button.destructive-action:hover,
.alert-dialog button.destructive-action:hover {{
  background-color: rgba({rr_255}, {rg_255}, {rb_255}, 0.85);
  color: #ffffff;
  opacity: 1.0;
}}
"#,
        accent = accent,
        ar = (ar * 255.0) as u8,
        ag = (ag * 255.0) as u8,
        ab = (ab * 255.0) as u8,
        bg = bg,
        dark_bg = dark_bg,
        lighter_bg = lighter_bg,
        fg = fg,
        muted = muted,
        selection = selection,
        dbr_255 = dbr_255,
        dbg_255 = dbg_255,
        dbb_255 = dbb_255,
        ar_255 = ar_255,
        ag_255 = ag_255,
        ab_255 = ab_255,
        red = red,
        rr_255 = rr_255,
        rg_255 = rg_255,
        rb_255 = rb_255,
    )
}

pub struct ThemeManager {
    colors: Arc<RwLock<OmarchyColors>>,
    _css_provider: gtk4::CssProvider,
    _watcher: Option<RecommendedWatcher>,
    callbacks: Arc<RwLock<Vec<Box<dyn Fn(&OmarchyColors) + 'static>>>>,
}

thread_local! {
    static GLOBAL_THEME_MANAGER: std::cell::RefCell<Option<ThemeManagerHandle>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
struct ThemeManagerHandle {
    colors: Arc<RwLock<OmarchyColors>>,
    css_provider: gtk4::CssProvider,
    callbacks: Arc<RwLock<Vec<Box<dyn Fn(&OmarchyColors) + 'static>>>>,
}

impl ThemeManager {
    pub fn new() -> Self {
        let colors = Arc::new(RwLock::new(load_current_theme()));
        let css_provider = gtk4::CssProvider::new();

        let initial_colors = colors.read().unwrap().clone();
        let css = generate_css(&initial_colors);
        css_provider.load_from_string(&css);

        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let callbacks = Arc::new(RwLock::new(Vec::new()));

        let handle = ThemeManagerHandle {
            colors: colors.clone(),
            css_provider: css_provider.clone(),
            callbacks: callbacks.clone(),
        };

        GLOBAL_THEME_MANAGER.with(|cell| {
            *cell.borrow_mut() = Some(handle);
        });

        let mut manager = Self {
            colors,
            _css_provider: css_provider,
            _watcher: None,
            callbacks,
        };

        manager.start_watching();
        manager
    }

    pub fn get_colors(&self) -> OmarchyColors {
        self.colors.read().unwrap().clone()
    }

    pub fn on_theme_changed<F: Fn(&OmarchyColors) + 'static>(&self, callback: F) {
        if let Ok(mut cbs) = self.callbacks.write() {
            cbs.push(Box::new(callback));
        }
    }

    fn start_watching(&mut self) {
        let res = notify::recommended_watcher(|res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                use notify::EventKind;
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        glib::idle_add_once(|| {
                            GLOBAL_THEME_MANAGER.with(|cell| {
                                if let Some(handle) = cell.borrow().as_ref() {
                                    let new_colors = load_current_theme();
                                    let css = generate_css(&new_colors);
                                    handle.css_provider.load_from_string(&css);
                                    if let Ok(mut w) = handle.colors.write() {
                                        *w = new_colors.clone();
                                    }
                                    if let Ok(cbs) = handle.callbacks.read() {
                                        for cb in cbs.iter() {
                                            cb(&new_colors);
                                        }
                                    }
                                }
                            });
                        });
                    }
                    _ => {}
                }
            }
        });

        if let Ok(mut watcher) = res {
            if let Some(path) = theme_file_path() {
                if let Some(parent) = path.parent() {
                    let _ = watcher.watch(parent, RecursiveMode::NonRecursive);
                }
            }

            if let Ok(home) = std::env::var("HOME") {
                let current_dir = PathBuf::from(home).join(".local/state/omarchy/current");
                if current_dir.exists() {
                    let _ = watcher.watch(&current_dir, RecursiveMode::NonRecursive);
                }
            }

            self._watcher = Some(watcher);
        }
    }
}

