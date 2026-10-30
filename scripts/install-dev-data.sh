#!/usr/bin/env bash
# Install the desktop entry + app icon into the user's XDG data dirs so
# GNOME's window switcher (alt-tab) matches the dev build by its Wayland
# app id (io.github.tunix.rtv) and shows the RTV name and logo instead of
# the generic executable icon. Runs on the HOST (the repo is shared with
# the distrobox container); re-run after changes.
#
# IMPORTANT: GLib refuses desktop entries whose Exec binary does not
# exist on PATH (GDesktopAppInfo returns NULL), so Exec is rewritten to
# the absolute binary path — default ~/.cargo/bin/rtv (created by
# `cargo install --path .` inside the container, visible on the host via
# the shared home), overridable as the first argument.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
ICONS_DIR="$DATA_HOME/icons/hicolor/scalable/apps"
APPS_DIR="$DATA_HOME/applications"
BIN="${1:-$HOME/.cargo/bin/rtv}"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found — install the binary first:" >&2
    echo "  distrobox enter valhalla -- bash -lc 'cd ~/Projects/rtv && cargo install --path .'" >&2
    exit 1
fi

mkdir -p "$ICONS_DIR" "$APPS_DIR"
cp "$REPO/data/icons/hicolor/scalable/apps/io.github.tunix.rtv.svg" "$ICONS_DIR/"
# Exec with an absolute path, so GDesktopAppInfo (and the shell's app
# tracker) accept the entry regardless of PATH.
sed "s|^Exec=.*$|Exec=$BIN|" "$REPO/data/io.github.tunix.rtv.desktop" \
    > "$APPS_DIR/io.github.tunix.rtv.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$DATA_HOME/icons/hicolor" 2>/dev/null || true
fi

echo "Installed:"
echo "  $APPS_DIR/io.github.tunix.rtv.desktop (Exec=$BIN)"
echo "  $ICONS_DIR/io.github.tunix.rtv.svg"
