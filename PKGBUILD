# Maintainer: Omarchy Community <https://github.com/HaroldFinch02/Omaview>
pkgname=omaview
pkgver=1.0.2
pkgrel=1
pkgdesc="Dead-simple, theme-aware image viewer for Omarchy / Hyprland"
arch=('x86_64' 'aarch64' 'riscv64')
url="https://github.com/HaroldFinch02/Omaview"
license=('MIT')
depends=('gtk4' 'libadwaita' 'glib2')
optdepends=(
    'libheif: HEIC support'
    'libjxl: JPEG XL support'
)
makedepends=('cargo' 'pkgconf')
source=()
sha256sums=()

build() {
    if [ -d "$srcdir/$pkgname-$pkgver" ]; then
        cd "$srcdir/$pkgname-$pkgver"
    fi
    cargo build --release --locked
}

package() {
    if [ -d "$srcdir/$pkgname-$pkgver" ]; then
        cd "$srcdir/$pkgname-$pkgver"
    fi
    install -Dm755 "target/release/omaview" "$pkgdir/usr/bin/omaview"
    install -Dm644 "data/omaview.desktop" "$pkgdir/usr/share/applications/omaview.desktop"
    install -Dm644 "data/icons/hicolor/scalable/apps/omaview.svg" \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/omaview.svg"
    install -Dm644 "data/icons/hicolor/scalable/apps/omaview.png" \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/omaview.png"
    for size in 16x16 32x32 48x48 64x64 128x128 256x256 512x512; do
        if [ -f "data/icons/hicolor/$size/apps/omaview.png" ]; then
            install -Dm644 "data/icons/hicolor/$size/apps/omaview.png" \
                "$pkgdir/usr/share/icons/hicolor/$size/apps/omaview.png"
        fi
    done
}
