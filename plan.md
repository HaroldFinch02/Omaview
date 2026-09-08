# Implementation Plan: Omaview (omaview) — Minimal Image Viewer for Omarchy Linux (Hyprland)

## 1. Executive Summary & Design Goals

**Omaview** (internal codename / window class: `omaview`) is a dead-simple, ultra-lightweight, hardware-accelerated image viewer built exclusively for **Omarchy Linux** under **Hyprland (Wayland)**.

It follows the same spirit as official Omarchy apps (`omacalc`, `omawrite`, `omacut`):

- Minimal surface area
- Keyboard-first
- Live theme sync from Omarchy
- No feature bloat
- Zero unnecessary third-party dependencies — implement everything feasible in-process with pure Rust (or the absolute minimum system libraries already present on Omarchy)

### Core UX (must match this description exactly)

1. **Primary viewport** occupies ~85–90 % of the window. The active image is always centered and the only thing that matters.
2. **Right vertical filmstrip** (fixed width ~96–120 px):
   - Active thumbnail is **pinned to vertical center**.
   - Previous images scroll upward above it.
   - Next images scroll downward below it.
   - Soft alpha fade toward top and bottom edges (CSS mask or equivalent).
   - Active thumbnail has a high-contrast accent ring taken from the current Omarchy `@accent` color.
3. **Bottom floating pill toolbar** (centered, rounded capsule, glass-like translucency):
   - Navigation, zoom, crop, rotate/flip, adjustments, EXIF, trash, grid toggle.
4. **Top floating metadata pill** (optional, auto-hides in Zen mode): filename · resolution · zoom % · index (`3 / 28`).
5. **Zen mode** (`Tab` / `z`) hides every chrome element so only the image remains.
6. Window is borderless / rounded, uses Hyprland blur + opacity rules, and stays out of the way.

Inspiration: the clean single-image focus of tools like Pixea, but deliberately **not** the same layout, chrome density, or interaction model. Omaview must feel native to Hyprland + Omarchy.

---

## 2. Technology Stack (Minimal by Design)

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Language | **Rust (2024 edition)** | Memory safety, instant startup, excellent cross-compilation, high-quality AI-generated code, matches modern Omarchy community tools. |
| UI / Windowing | **GTK4 + Libadwaita** (`gtk4-rs`) | Native Wayland, fractional scaling, GSK (Vulkan/NGL) hardware acceleration, CSS custom properties, excellent transparency & blur support under Hyprland. |
| Image decode / encode | **`image` crate** (+ optional pure-Rust backends) | Pure Rust. Supports JPEG, PNG, WebP, GIF, BMP, TIFF, etc. Prefer this over `libvips` or heavy FFI. Add `image-webp`, `image-jpeg` features only as needed. |
| Advanced formats (optional) | `libheif` / `libjxl` system packages only if required at runtime | Keep optional; do not force them into the core binary. |
| Theme watching | `notify` crate + direct read of Omarchy state | Watch `~/.local/state/omarchy/current/theme/colors.toml` (and fallback paths). |
| Async / threading | `tokio` (or `async-std`) + `glib` main loop integration | Preload neighboring images without blocking UI. |
| Build | Cargo (primary) + simple Meson or just Cargo for PKGBUILD | Keep packaging trivial. |

**Hard rule — No unnecessary dependencies**

- Do **not** pull in full image-editor frameworks, Electron, Qt (unless rewriting entirely), OpenCV, heavy ML crates, or network stacks.
- Basic pixel operations (crop, rotate 90°, flip, exposure/contrast/saturation/warmth) must be implemented with the `image` crate or simple CPU/GPU shaders via GSK.
- ffmpeg is **not** required for core features. Only consider it later for animated formats or export if pure-Rust paths prove insufficient — and even then keep it an optional runtime dependency, never a hard compile-time one.
- Prefer writing small helpers over adding crates.

---

## 3. Architecture & Target Support

Targets (via standard Rust/LLVM):

- `x86_64`
- `x86_64-v3` (optional optimized build)
- `aarch64`
- `riscv64`

Window class / app-id: `omaview` (or `org.omarchy.omaview`) so Hyprland rules apply cleanly.

### High-level module layout (suggested)

```
src/
├── main.rs                 # entry, CLI args, application setup
├── app.rs                  # AdwApplication / GtkApplication
├── window.rs               # main window, surface transparency, CSS provider
├── viewport.rs             # custom drawing area / scrolled window + pan/zoom
├── filmstrip.rs            # right vertical centered thumbnail rail
├── toolbar.rs              # bottom floating pill
├── metadata.rs             # top pill + EXIF popover
├── image_loader.rs         # async load + preloader (prev 1 / next 2)
├── image_ops.rs            # crop, rotate, flip, basic adjustments (pure image crate)
├── theme.rs                # Omarchy colors.toml watcher + CSS variable injection
├── shortcuts.rs            # keybinding map
└── util.rs                 # trash via gio, path helpers, etc.
```

Data model:

- `Vec<PathBuf>` or `Vec<ImageEntry>` for the open set (folder or explicit files).
- Current index.
- Cached decoded `DynamicImage` or GPU texture for current + neighbors.
- Edit state (non-destructive until explicit save).

---

## 4. Automatic Theme Synchronization (Omarchy Integration)

Official Omarchy apps read:

```
~/.local/state/omarchy/current/theme/colors.toml
```

(Also tolerate older / alternate locations such as `~/.config/omarchy/current/theme/` if present.)

### Mechanism

1. On startup and on `notify` events, parse the TOML (keys: `accent`, `background`, `foreground`, `selection`, `muted`, etc.).
2. Map them to CSS custom properties and inject via `GtkCssProvider`:
   ```css
   :root {
     --accent: #...;
     --window-bg: alpha(...);
     --card-bg: ...;
     --fg: ...;
   }
   ```
3. Window background uses translucent card color so Hyprland’s Gaussian blur shows through.
4. Active thumbnail ring and accent highlights always use `--accent`.
5. Live re-theme without restart (same behavior as `omacalc`).

### Recommended Hyprland rules (document in README)

```ini
windowrulev2 = blur, class:^(omaview)$
windowrulev2 = opacity 0.95 override 0.85 override, class:^(omaview)$
windowrulev2 = rounding 12, class:^(omaview)$
windowrulev2 = float, class:^(omaview)$   # optional; tiling also works
```

---

## 5. UI Components — Precise Specification

### 5.1 Main Viewport

- `GtkDrawingArea` or `GtkPicture` inside a `GtkScrolledWindow` / custom gesture area.
- Support:
  - Pinch / Ctrl+scroll / `+` `-` zoom
  - Drag pan when zoomed
  - Fit-to-window (`f`) and 1:1 (`0`)
  - Smooth bicubic (or Mitchell) resampling via GSK when possible
- Preload: current ±1 (and optionally ±2) on background threads.
- Drop target for additional images/folders.

### 5.2 Right Vertical Filmstrip (Center-Pinned)

- Fixed-width vertical box of thumbnails.
- **Centering algorithm** (must be exact):

  Let  
  \( H \) = container height,  
  \( h \) = thumbnail slot height (including padding),  
  \( i \) = active index (0-based).

  Then vertical offset of the whole strip:

  \[
  \text{offset}_y = \frac{H}{2} - \bigl(i \cdot h + \tfrac{h}{2}\bigr)
  \]

- Apply a vertical CSS mask (or Cairo/GSK gradient) for fade:

  ```css
  .filmstrip {
    mask-image: linear-gradient(
      to bottom,
      transparent 0%,
      rgba(0,0,0,0.35) 10%,
      black 30%,
      black 70%,
      rgba(0,0,0,0.35) 90%,
      transparent 100%
    );
  }
  ```

- Active thumbnail: 2–3 px accent border + subtle scale or glow.
- Clicking a thumbnail jumps to that image and re-centers the strip.
- Mouse-wheel or trackpad over the strip scrolls the sequence.

### 5.3 Bottom Floating Pill

Rounded capsule (`border-radius: 24–32px`, translucent background, optional backdrop-filter if supported):

Contents (left → right, icons only + tooltips):

- ◀ Previous
- ▶ Next
- Grid / filmstrip toggle
- Zoom −   ·   indicator   ·   Zoom +
- Crop (`c`)
- Rotate CW / CCW (`r` / `R`)
- Flip H / V
- Adjustments drawer (`a` / `e`) — lightweight sliders: Exposure, Contrast, Saturation, Warmth (real-time preview via simple pixel math or GSK shader)
- Info / EXIF (`i`)
- Trash (`Delete` / `d`) — uses `gio::File::trash` (never permanent delete)

All controls are keyboard-reachable; mouse is secondary.

### 5.4 Top Metadata Pill

Small centered or left-aligned pill showing:

`filename.ext  ·  4032×3024  ·  87%  ·  3/28`

Auto-hides in Zen mode and after a short idle timeout.

---

## 6. Keyboard Ergonomics (Hyprland / Vim style)

| Binding | Action |
|---------|--------|
| `j` / `→` / `Space` | Next image |
| `k` / `←` / `Backspace` | Previous image |
| `+` / `=` / `Ctrl+↑` | Zoom in |
| `-` / `Ctrl+↓` | Zoom out |
| `0` | 100 % / actual size |
| `f` | Fit to window |
| `r` / `R` | Rotate 90° CW / CCW |
| `h` / `v` | Flip horizontal / vertical |
| `c` | Enter crop mode (rule-of-thirds overlay) |
| `a` / `e` | Toggle adjustments panel |
| `i` | Toggle EXIF / metadata |
| `Delete` / `d` | Move to trash |
| `Tab` / `z` | Zen mode (hide all chrome) |
| `g` | Toggle grid / filmstrip visibility |
| `s` | Save current edit (overwrite with confirmation or Save As via portal) |
| `q` / `Esc` | Quit (or exit crop / adjustments first) |

All bindings must work when the window has focus; document them in a simple `?` or `Ctrl+?` overlay if desired.

---

## 7. Basic Editing Features (In-App Only)

Implement the following **inside the process** with the `image` crate (or equivalent pure-Rust pixel buffers). No external editor launch for core features.

1. **Crop** — interactive rectangle with rule-of-thirds guides; apply on confirm.
2. **Rotate** — lossless 90° steps (or re-encode if format requires it).
3. **Flip** — horizontal / vertical.
4. **Adjustments** — Exposure, Contrast, Saturation, Warmth/Temperature (simple matrix or per-channel math). Live preview on a downscaled version if full-res is expensive.
5. **Save** — overwrite (with confirmation) or “Save As” via XDG portal. Preserve EXIF when possible.
6. **Trash** — `gio` trash, never permanent delete by default.

Do **not** implement layers, healing, complex filters, or AI features. Keep the surface area tiny.

If a pure-Rust path for a particular format proves painful, document the limitation rather than adding heavy dependencies.

---

## 8. Implementation Roadmap (Detailed for Coding Agent)

### Phase 0 — Project Skeleton
- `cargo init --name omaview` (binary name `omaview` or `omaview`).
- Add `gtk4`, `libadwaita`, `image`, `notify`, `toml`, `serde`, `gio` (via gtk).
- Minimal `AdwApplication` that opens a transparent window and loads a test image.
- Desktop file + icon (SVG) following Omarchy packaging style.
- Acceptance: window appears, shows a static image, respects basic Wayland transparency.

### Phase 1 — Viewport + Navigation
- Load a directory or list of files.
- Asynchronous decoder with cancellation.
- Pan / zoom gestures + keyboard.
- Basic prev/next.
- Acceptance: open folder, navigate with j/k, zoom, fit.

### Phase 2 — Filmstrip
- Generate small thumbnails (cached on disk or in memory).
- Implement exact center-pinning math.
- CSS / mask fade.
- Click-to-jump + wheel scroll.
- Acceptance: active thumb always visually centered; fade looks correct.

### Phase 3 — Chrome (Pills)
- Bottom floating pill with all icons and actions wired.
- Top metadata pill.
- Zen mode that hides both.
- Acceptance: all listed keyboard shortcuts work; UI matches the description.

### Phase 4 — Theme Engine
- Parse `~/.local/state/omarchy/current/theme/colors.toml`.
- Hot-reload on change.
- Inject CSS variables; accent ring updates live.
- Acceptance: switching Omarchy theme instantly recolors the viewer.

### Phase 5 — Editing Suite
- Crop overlay + apply.
- Rotate / flip.
- Adjustments panel with live preview.
- Save / trash via portals and gio.
- Acceptance: crop → adjust → save produces a correct new file; trash works.

### Phase 6 — Polish & Packaging
- Preloader (neighbors).
- Memory limits / eviction for large folders.
- Error handling (corrupt files, missing permissions).
- PKGBUILD modeled after `omacalc` / `omawrite` style (simple, no unnecessary makedepends).
- Cross-compile notes for aarch64 / riscv64.
- README with Hyprland rules, shortcuts, and “dead-simple” philosophy.

---

## 9. Package Specification (PKGBUILD sketch)

```bash
# Maintainer: Omarchy community / contributor
pkgname=omaview
pkgver=0.1.0
pkgrel=1
pkgdesc="Dead-simple, theme-aware image viewer for Omarchy / Hyprland"
arch=('x86_64' 'aarch64' 'riscv64')
url="https://github.com/..."   # replace with actual
license=('MIT')                # or GPL-3.0-or-later — match project
depends=('gtk4' 'libadwaita' 'glib2')
optdepends=('libheif: HEIC support' 'libjxl: JPEG XL support')
makedepends=('cargo' 'pkgconf')

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 "target/release/omaview" "$pkgdir/usr/bin/omaview"
  # or name the binary omaview if preferred
  install -Dm644 "data/omaview.desktop" "$pkgdir/usr/share/applications/omaview.desktop"
  install -Dm644 "data/icons/hicolor/scalable/apps/omaview.svg" \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/omaview.svg"
}
```

Keep the package as lean as the official `omacalc` / `omawrite` packages.

---

## 10. Explicit Non-Goals

- No full photo library / catalog / tagging system.
- No cloud sync, accounts, or network features.
- No plugin system.
- No AI / generative features.
- No permanent delete by default.
- No dependence on heavy C libraries beyond what GTK already pulls in (prefer pure Rust image pipeline).
- Do not clone or vendor large upstream projects; only take inspiration from the public style of `omacalc` / `omawrite`.

---

## 11. Success Criteria (for the coding agent)

The finished application must:

1. Open a folder or single image and display it full-viewport.
2. Keep the active thumbnail perfectly centered in the right filmstrip with soft fade.
3. Provide the exact bottom-pill controls listed above.
4. Respond to every keyboard shortcut in the table.
5. Live-recolor when the user changes the Omarchy theme.
6. Perform crop / rotate / flip / basic adjustments and save without launching external tools.
7. Start in under ~150 ms on a typical machine and stay lightweight.
8. Contain no unnecessary crates or system dependencies.

## Agent Instructions
1. Create suitable agents.md if required and helpful for agents to continue work
2. Always write important checkpoints completed to work_done.md so other agents and refer and resume
