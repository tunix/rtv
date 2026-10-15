# AGENTS.md — RTV

IPTV player for the GNOME desktop. Rust + GTK4/libadwaita, Xtream Codes first, M3U second.

## Environment

- Develop inside the `rtv` distrobox container (Fedora): `distrobox enter rtv`
  (create once with: `distrobox create -n rtv -i registry.fedoraproject.org/fedora-toolbox:latest`)
- Run all cargo/git commands in the container; the repo is shared with the host.

## Commands

- `cargo build` / `cargo test` / `cargo fmt` / `cargo clippy --all-targets` — clippy and fmt must be clean before committing.
- GUI: `cargo run`
- Commits: small and meaningful, conventional style (`feat:`, `chore:`). Never push unless explicitly asked.

## Product rules

- Xtream Codes is the primary source type; M3U support is secondary but must keep working once Xtream features land.
- Xtream credentials (server URL, username, password) live in the GNOME Keyring, never in plain config files.
- Never block the UI thread on network I/O: fetches run on the tokio runtime and results reach the UI through channels.
- UI follows GNOME HIG and stays libadwaita-native (Blueprint templates compiled in build.rs).
- Iterate on UI mocks in `resources/*.blp` before wiring business logic; empty states must always look intentional.
- Stream URLs can expire or 404: re-resolve on failure, surface a toast/banner, never strand the player.

## Gotchas

- gtk4-rs/glib-rs APIs drift between versions: verify signatures against vendored sources in `~/.cargo/registry` before writing bindings code.
- glib objects are not `Send`; once the tokio runtime lands, settings access from workers needs a Send wrapper (see valhalla's `SendSettings` pattern).
- Xtream API responses are JSON but endpoints differ per panel version; don't assume fields are always present.
