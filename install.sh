#!/usr/bin/env bash
# Omaview Installer
# Installs pre-compiled x86_64 or aarch64 (ARM) Linux binary and desktop integration

set -e

REPO="HaroldFinch02/Omaview"
INSTALL_BIN_DIR="${HOME}/.local/bin"
INSTALL_APP_DIR="${HOME}/.local/share/applications"
INSTALL_ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"

echo "✦ Installing Omaview Image Viewer..."

# 1. Detect architecture
RAW_ARCH="$(uname -m)"
case "${RAW_ARCH}" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  aarch64|arm64)
    ARCH="aarch64"
    ;;
  *)
    echo "Error: Unsupported architecture '${RAW_ARCH}'. Supported: x86_64, aarch64." >&2
    exit 1
    ;;
esac

echo "✦ Detected architecture: ${ARCH} Linux"

# 2. Prepare directories
mkdir -p "${INSTALL_BIN_DIR}" "${INSTALL_APP_DIR}" "${INSTALL_ICON_DIR}"

# 3. Create temp workspace
TMP_DIR="$(mktemp -d -t omaview-install-XXXXXX)"
trap 'rm -rf "${TMP_DIR}"' EXIT

TARBALL_NAME="omaview-${ARCH}-linux.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${TARBALL_NAME}"

echo "✦ Downloading ${TARBALL_NAME} from GitHub Releases..."
if curl -fsSL -o "${TMP_DIR}/${TARBALL_NAME}" "${DOWNLOAD_URL}" 2>/dev/null; then
  echo "✦ Extracting release archive..."
  tar -xzf "${TMP_DIR}/${TARBALL_NAME}" -C "${TMP_DIR}"
  EXTRACT_DIR="${TMP_DIR}/omaview-${ARCH}-linux"
  
  if [ -d "${EXTRACT_DIR}" ]; then
    install -Dm755 "${EXTRACT_DIR}/omaview" "${INSTALL_BIN_DIR}/omaview"
    [ -f "${EXTRACT_DIR}/omaview.desktop" ] && install -Dm644 "${EXTRACT_DIR}/omaview.desktop" "${INSTALL_APP_DIR}/omaview.desktop"
    [ -f "${EXTRACT_DIR}/icons/omaview.svg" ] && install -Dm644 "${EXTRACT_DIR}/icons/omaview.svg" "${INSTALL_ICON_DIR}/omaview.svg"
    [ -f "${EXTRACT_DIR}/icons/omaview.png" ] && install -Dm644 "${EXTRACT_DIR}/icons/omaview.png" "${INSTALL_ICON_DIR}/omaview.png"
  else
    echo "Error: Unexpected archive layout" >&2
    exit 1
  fi
else
  echo "Notice: Pre-built GitHub release tarball not yet published or unreachable."
  if command -v cargo >/dev/null 2>&1; then
    echo "✦ Fallback: Building from source using cargo..."
    cargo install --git "https://github.com/${REPO}.git" --locked
  else
    echo "Error: Could not download pre-compiled release and cargo is not installed." >&2
    echo "Visit: https://github.com/${REPO}/releases" >&2
    exit 1
  fi
fi

# Update desktop database if available
update-desktop-database "${INSTALL_APP_DIR}" 2>/dev/null || true

echo ""
echo "✔ Omaview installed successfully to ${INSTALL_BIN_DIR}/omaview"
if ! echo "${PATH}" | tr ':' '\n' | grep -qx "${INSTALL_BIN_DIR}"; then
  echo "Tip: Make sure ${INSTALL_BIN_DIR} is in your PATH (e.g. export PATH=\"\$HOME/.local/bin:\$PATH\")"
fi
echo "Launch with: omaview [path/to/image]"
