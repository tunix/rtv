#!/usr/bin/env bash
# Install the desktop entry + app icon into the user's XDG data dirs so
# GNOME's window switcher (alt-tab) matches the dev build by its Wayland
# app id (io.github.tunix.rtv) and shows the RTV name and logo instead of
# the generic executable icon. Runs on the HOST (the repo is shared with
# the distrobox container); re-run after changes.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
ICONS_DIR="$DATA_HOME/icons/hicolor/scalable/apps"
APPS_DIR="$DATA_HOME/applications"

mkdir -p "$ICONS_DIR" "$APPS_DIR"
cp "$REPO/data/icons/hicolor/scalable/apps/io.github.tunix.rtv.svg" "$ICONS_DIR/"
cp "$REPO/data/io.github.tunix.rtv.desktop" "$APPS_DIR/"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$DATA_HOME/icons/hicolor" 2>/dev/null || true
fi

echo "Installed:"
echo "  $APPS_DIR/io.github.tunix.rtv.desktop"
echo "  $ICONS_DIR/io.github.tunix.rtv.svg"
echo "Note: Exec=rtv expects the binary on PATH (e.g. ~/.cargo/bin/rtv)."
