# Omaview - Agent Handbook & Developer Guide

## 1. Mission & Vision
**Omaview** (`omaview`) is a lightning-fast, hardware-accelerated image viewer and album manager built natively for **Omarchy Linux** and the **Hyprland** (Wayland) compositor.
It prioritizes:
- **Instant startup & zero lag**: Async decoding, LRU caching, and smooth Wayland rendering.
- **Glassmorphic minimalism**: Native window translucency (`rgba(..., 0.80)`) harmonizing with Hyprland compositor blur, dynamic color extraction from Omarchy's system theme (`~/.local/state/omarchy/current/theme/colors.toml`), and sleek floating pill controls.
- **Non-destructive workflows**: Quick rotations, aspect-ratio crops (Freeform, 1:1, 16:9), live color adjustments (brightness, contrast, saturation), and safe explicit saving (Ctrl+S or top-right "Save" pill).

---

## 2. Technology Stack & Dependencies
- **Language**: Rust (2024 edition)
- **GUI Engine**: GTK 4 (`gtk4` crate v0.11, feature `v4_16`) and Libadwaita (`libadwaita` crate v0.9, feature `v1_6`).
- **2D Graphics & Rendering**: `cairo-rs` (v0.22, feature `png`) for high-performance canvas rendering of thumbnails, specular rims, and viewport images.
- **Image Pipeline**: Pure Rust `image` crate (v0.25) with codecs: PNG, JPEG, WebP, GIF, BMP, TIFF, ICO.
- **EXIF & Metadata**: `kamadak-exif` (v0.6).
- **Filesystem & Theme Watcher**: `notify` (v6.1) for inotify-driven live theme reloads (`colors.toml`).
- **Data Persistence**: `serde` + `toml` (v0.8) for album configuration (`~/.config/omaview/albums.toml`).

---

## 3. Project Architecture & Code Map
```
src/
├── main.rs         # Process initialization, sets prgname ("omaview"), app name ("Omaview"), initializes GTK
├── app.rs          # Libadwaita Application wrapper, handles CLI argument resolution, single-instance activation
├── window.rs       # MainWindow orchestration, GTK Overlay structure, floating chrome, keybindings, save workflows
├── home.rs         # Home / Albums view, horizontal album strips, Cairo-rendered cards with hover effects
├── albums.rs       # Album metadata model & TOML storage (~/.config/omaview/albums.toml)
├── viewport.rs     # GtkDrawingArea image surface, pan & zoom with smooth interpolation, interactive crop box
├── crop_bar.rs     # Floating crop toolbar pill (Freeform, 1:1 Square, 16:9 Widescreen, Apply, Cancel)
├── filmstrip.rs    # Vertical right-side thumbnail strip, async background thumbnail prefetching
├── toolbar.rs      # Floating bottom pill toolbar (Home, Filmstrip, Zoom, Crop, Rotate, Adjustments, Info, Trash)
├── metadata.rs     # Floating top-left metadata pill (filename, dimensions, zoom level, file size, index counter)
├── shortcuts.rs    # Interactive keyboard shortcuts modal dialog
├── theme.rs        # Dynamic theme watcher for Omarchy colors.toml & GTK CSS provider generation
├── image_ops.rs    # Pure functional image manipulation (rotations, flips, crops, color adjustments, saving)
├── image_loader.rs # Multi-threaded image decoding, LRU in-memory pixbuf/surface caching
└── util.rs         # Natural alphanumeric sorting, human-readable file size formatting, file extensions
```

---

## 4. UI Layout & Chrome Hierarchy
The viewer uses a unified `GtkOverlay` layout over the image viewport:
- **Top-Left Box**: Home button (`nav-home-btn`) + Metadata pill (`metadata-pill`).
- **Top-Center**: Floating crop bar pill (`crop-pill`), active only during crop mode (`c`).
- **Top-Right**: Text-only **"Save"** pill (`save-pill-btn`), dynamically visible only when unsaved edits exist.
- **Left / Right Center**: Minimal 36×36px circular navigation chevrons (`nav-chevron-btn`).
- **Bottom-Center**: Floating toolbar pill (`floating-pill`) containing view, edit, and filter controls.
- **Right Strip**: Collapsible vertical filmstrip (`filmstrip-container`).

---

## 5. Critical Invariants & Rules for Agents

### A. RefCell Borrow Scoping Across FFI Trampolines (CRITICAL)
In Rust, temporaries created in function argument positions remain active until the end of the entire statement.
**Never do this**:
```rust
// DANGEROUS: state.borrow() lives until after go_to_index returns!
// If go_to_index calls state.borrow_mut(), it triggers BorrowMutError panic / abort!
win_clone.go_to_index(win_clone.state.borrow().current_index);
```
**Always do this**:
```rust
// SAFE: Borrow is dropped immediately before calling into mutating method:
let current_idx = win_clone.state.borrow().current_index;
win_clone.go_to_index(current_idx);
```

### B. Translucency & Compositor Blur (Hyprland)
- The window background MUST use `rgba({dbr}, {dbg}, {dbb}, 0.80);` on `window.omaview-window`.
- All child layout containers (`.omaview-main-box`, `stack`, `overlay`, `scrolledwindow`, `box`) MUST have `background-color: transparent;`.
- **DO NOT** re-introduce fake wallpaper images, screenshot blurs, or heavy opaque CSS gradients to the main window. Hyprland natively blurs the user's desktop wallpaper behind Omaview.

### C. Desktop Entry & App ID
- Application ID: `org.omarchy.omaview`
- Process Name (`prgname`): `omaview`
- Desktop Entry: `~/.local/share/applications/omaview.desktop`
- **DO NOT** create a separate duplicate `.desktop` file with `Name=Omaview` in `~/.local/share/applications/` (such as `org.omarchy.omaview.desktop`), otherwise app launchers (wofi, rofi, fuzzel) will display duplicate icons.

### D. Icon System
- Canonical icon source: `data/icons/hicolor/scalable/apps/omaview_icon.png` (and `omaview.png`).
- Standard resolutions (`16x16` up to `512x512`) and scalable SVG (with embedded PNG) are installed in `~/.local/share/icons/hicolor/`.
- Rebuild icon cache after updates: `gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor`.

### E. Binary Installation
- To replace the running binary safely without encountering `cp: Text file busy`, always use:
  ```bash
  install -m 755 target/release/omaview ~/.local/bin/omaview
  ```

---

## 6. Build, Test & Verification Commands
- **Run Unit Tests**:
  ```bash
  cargo test
  ```
- **Build Optimized Release**:
  ```bash
  cargo build --release
  ```
- **Live GUI Testing on Hyprland**:
  ```bash
  # Launch in background
  ~/.local/bin/omaview /path/to/image.jpg &
  OMA_PID=$!
  
  # Target window precisely by Hyprland class (NOT title, to avoid Nautilus collisions):
  hyprctl dispatch 'hl.dsp.focus({ window = "class:org.omarchy.omaview" })'
  
  # Send keystrokes via wtype:
  wtype -k r        # Rotate CW
  wtype -k s        # Save dialog
  wtype -k Return   # Overwrite
  wtype -k c        # Enter crop mode
  wtype -k 1        # 1:1 Square
  wtype -k 9        # 16:9 Widescreen
  wtype -k 0        # Freeform
  wtype -k Escape   # Cancel / Home
  
  # Capture screenshot for visual inspection:
  grim /path/to/screenshot.png
  kill $OMA_PID
  ```

---

## 7. Logging & Documentation
- Always record completed phases, bugfixes, and visual verifications in `work_done.md`.
