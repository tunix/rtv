<p align="center">
  <img src="data/icons/hicolor/scalable/apps/io.github.tunix.rtv.svg" width="192" alt="RTV logo"/>
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

## Settings

Stored in GSettings (`gsettings list-recursively io.github.tunix.rtv`).

## License

GPL-3.0
