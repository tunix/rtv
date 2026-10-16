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
- Terminology: the UI says "content", never "channel" (Xtream/M3U carry more than live TV).
- Loading policy: adding a source shows the UI immediately and streams content in progressively; later launches render from the local cache first and refresh async in the background. Never block the user on loading, but always surface progress.
- Mock phase: `src/sample.rs` feeds the UI from `samples/*.json` (gitignored dumps). Grids fill progressively via generation-guarded idle chunks (`Window::fill_grid`) — never render a 24k-item category in one pass.
- Artwork/covers are cached locally (`$XDG_CACHE_HOME/rtv/covers`, URL-hash keys — see `covers.rs`); fetches run via libsoup3 on the GLib main loop (NOT tokio), on-demand only for rendered cards; on any fetch failure use placeholder art, degrade gracefully, and NEVER crash. HTTP requests go through libsoup3 with the app-like User-Agent (see UA gotcha).
- Xtream categories are flat (parent_id=0) and M3U nests by group-title; the real tree will be modeled after the dumps in `samples/` (gitignored — never commit them, they contain credentials).
- Data model (confirmed from samples/): Xtream sections (live/series/vod) each carry their OWN flat category list (parent_id=0, no hierarchy). Nesting is: section → category → content; series add one more level: series → seasons[] → episodes{} (episodes dict is keyed by season STRING and may omit special seasons that appear in `seasons[]` — handle missing keys).
- dogu1's origin kills `get.php` entirely (m3u export blocked at origin, UA-independent, all params): for Xtream sources derive playback from the API; the M3U source type targets generic providers only.
- Playback/episode URLs embed credentials (`/live/U/P/id.ts`, `/series/U/P/167662.mkv`) — never log, cache to disk, or commit them.
- Stream URLs can expire or 404: re-resolve on failure, surface a toast/banner, never strand the player.

## Gotchas

- gtk4-rs/glib-rs APIs drift between versions: verify signatures against vendored sources in `~/.cargo/registry` before writing bindings code.
- GTK 4.22.5 (container) ignores icon-theme resource paths: `add_resource_path` is a no-op on display AND standalone themes. Bundled icons are exported to `$XDG_RUNTIME_DIR/rtv-icons` at startup (`app::install_runtime_icons`) and that dir is registered as a search path; use plain `icon-name` afterwards. Diagnose with `cargo run -- --check-icons`.
- The provider's origin filters User-Agents: CLI UAs (curl etc.) get the connection killed and Cloudflare surfaces it as HTTP 520. ALL HTTP requests (API fetches AND stream playback) must send an app-like User-Agent, e.g. `okhttp/4.9.3` (GStreamer: set `user-agent` on `souphttpsrc`).
- glib timers: never `SourceId::remove()` an id that may have already fired — it panics inside signal trampolines and aborts the app. Neutralize stale timers with a generation counter instead (see `Window::show_controls`).
- glib objects are not `Send`; once the tokio runtime lands, settings access from workers needs a Send wrapper (see valhalla's `SendSettings` pattern).
- Xtream API responses are JSON but endpoints differ per panel version; don't assume fields are always present.

## Release pipeline

- release-please reads `release-please-config.json` at the repo ROOT (a copy under `.github/` is ignored by the action). Merging its "chore(main): release X.Y.Z" PRs creates DRAFT releases.
- `release.yml` builds tarball + `.flatpak` bundle (x86_64/aarch64), attaches them to the draft, then `publish` flips the draft to published; `notify-flatpak-repo` then sends `repository_dispatch` (`app-released`, payload `app_id` + `tag`) to tunix/flatpak, which rebuilds the GPG-signed self-hosted repo at tunix.github.io/flatpak.
- `flatpak/generated-sources.json` is regenerated from Cargo.lock on every build (CI and tunix/flatpak alike); never hand-edit it.
- Flatpak manifest trap: `sdk-extensions` must use bare ids (`org.freedesktop.Sdk.Extension.rust-stable`); versioned refs fail with "extension not installed".
- Actions secrets in this repo: `FLATPAK_REPO_TOKEN` (PAT with contents:write on tunix/flatpak — used only by the dispatch step) and `RENOVATE_TOKEN` (fine-grained PAT, repo + workflow scopes on this repo). The GPG key lives only in tunix/flatpak's `GPG_KEY` secret — nothing GPG-related to manage here.
