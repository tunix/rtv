<p align="center">
  <img src="docs/logo.png" width="256" alt="RTV logo"/>
</p>

# RTV

An IPTV player for the GNOME desktop, written in Rust with GTK 4 and
libadwaita.

RTV streams live TV from **Xtream Codes** panels and **M3U** playlists, in a
UI that follows the GNOME Human Interface Guidelines.

## Features

- **Xtream Codes** accounts (live TV first; VOD/series on the roadmap)
- **M3U / M3U8 playlists**, local file or URL
- Native GTK4/libadwaita UI with Blueprint templates
- Credentials kept in the GNOME Keyring

> Work in progress — currently in the UI mock phase.

## Building

Development happens in a distrobox/toolbox container (or any distro with the
GNOME stack):

```sh
distrobox create -n rtv -i registry.fedoraproject.org/fedora-toolbox:latest
distrobox enter rtv
cd ~/Projects/rtv
cargo build --release
```

Requirements (Fedora, run inside the container):

```sh
sudo dnf install -y rust cargo rustfmt clippy gcc pkgconf-pkg-config \
    gtk4-devel libadwaita-devel glib2-devel blueprint-compiler git
```

## Running

```sh
cargo run
```

## Flatpak

Every release ships ready-to-install `.flatpak` bundles (x86_64 and
aarch64), built in CI from [`flatpak/io.github.tunix.rtv.json`](flatpak/io.github.tunix.rtv.json)
on the GNOME 49 runtime. Grab one from the
[releases page](https://github.com/tunix/rtv/releases) and install:

```sh
flatpak install ./rtv-<tag>-x86_64.flatpak
```

The bundle references its runtime, so the GNOME 49 platform is pulled
from Flathub automatically on first install.

To install from the self-hosted repo instead (automatic updates included):

```sh
flatpak remote-add --if-not-exists tunix https://tunix.github.io/flatpak/tunix.flatpakrepo
flatpak install tunix io.github.tunix.rtv
```

To build the bundle locally:

```sh
# regenerate the vendored-crate source list from Cargo.lock
python3 flatpak/flatpak-cargo-generator.py Cargo.lock \
    -o flatpak/generated-sources.json

# build (offers to install the GNOME 49 Sdk and the rust-stable
# extension on first run)
flatpak-builder --force-clean --user --default-branch=stable \
    --repo=flatpak-repo flatpak-build flatpak/io.github.tunix.rtv.json

# optional: single-file bundle to share or install
flatpak build-bundle \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    flatpak-repo rtv.flatpak io.github.tunix.rtv stable
```

## Settings

Stored in GSettings (`gsettings list-recursively io.github.tunix.rtv`).

## License

GPL-3.0
