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

## Release pipeline

- release-please reads `release-please-config.json` at the repo ROOT (a copy under `.github/` is ignored by the action). Merging its "chore(main): release X.Y.Z" PRs creates DRAFT releases.
- `release.yml` builds tarball + `.flatpak` bundle (x86_64/aarch64), attaches them to the draft, then `publish` flips the draft to published; `notify-flatpak-repo` then sends `repository_dispatch` (`app-released`, payload `app_id` + `tag`) to tunix/flatpak, which rebuilds the GPG-signed self-hosted repo at tunix.github.io/flatpak.
- `flatpak/generated-sources.json` is regenerated from Cargo.lock on every build (CI and tunix/flatpak alike); never hand-edit it.
- Flatpak manifest trap: `sdk-extensions` must use bare ids (`org.freedesktop.Sdk.Extension.rust-stable`); versioned refs fail with "extension not installed".
- Actions secrets in this repo: `FLATPAK_REPO_TOKEN` (PAT with contents:write on tunix/flatpak — used only by the dispatch step) and `RENOVATE_TOKEN` (fine-grained PAT, repo + workflow scopes on this repo). The GPG key lives only in tunix/flatpak's `GPG_KEY` secret — nothing GPG-related to manage here.
