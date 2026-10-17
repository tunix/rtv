# Changelog

## [0.1.0](https://github.com/tunix/rtv/compare/v0.0.1...v0.1.0) (2026-10-17)


### Features

* **data:** new app icon — fanned channel cards ([4ff9f8a](https://github.com/tunix/rtv/commit/4ff9f8a223809c0e698eac2f673926fdc22d5bd8))
* **details:** offline-first — local samples/series_info|vod_info dumps win over the network ([0a5e18e](https://github.com/tunix/rtv/commit/0a5e18e7159bca462e1b1f285ebdc1e6d0e6d063))
* initial project skeleton ([1c38c10](https://github.com/tunix/rtv/commit/1c38c105bfc6e21d14ed2415a6ac9bd9999b7fdb))
* **ui:** always-visible favorites with clear-all, Live TV rename, header polish, square card art with heart chip, About credit ([d3273c9](https://github.com/tunix/rtv/commit/d3273c91e979e553587037799c01fd22b743fc5e))
* **ui:** app-frame mock — forced source setup, source picker, nested categories, favorites, full-window player ([e3dd6a3](https://github.com/tunix/rtv/commit/e3dd6a3d4b734088fd5e24052aac0cec91fb9228))
* **ui:** category content grid, chrome-free fullscreen, flap-free controls, content terminology ([321b5ab](https://github.com/tunix/rtv/commit/321b5ab55fa65042318567ab2a3c561ee8112dca))
* **ui:** cover art cache via libsoup3 with disk cache and placeholders; section icons; search UX (sidebar stays visible, category labels, Esc resets) ([6f3805f](https://github.com/tunix/rtv/commit/6f3805f5973006d605e5063013cd2826bf49d487))
* **ui:** main window mock — channel list + player placeholder ([f71111f](https://github.com/tunix/rtv/commit/f71111f44fe4659f2dbdb3c00ddd87024d25a34e))
* **ui:** new logo everywhere, auto-hiding player controls with media shortcuts, fullscreen chrome-free; fix icon lookup on GTK 4.22 ([f0adf1e](https://github.com/tunix/rtv/commit/f0adf1e4163a322cfd58fa30a8b102f77936a80e))
* **ui:** player control bar (seek/play/mute/volume/fullscreen), centered welcome CTA, accurate source-switch toast ([53d84f2](https://github.com/tunix/rtv/commit/53d84f21542290c6126b309714b657f8bac128dc))
* **ui:** sample catalog loader feeds the mock at real scale — dynamic sections/categories, progressive grid fills, catalog-wide search ([91d6cc5](https://github.com/tunix/rtv/commit/91d6cc5ebce475deff0e28c61dabfa569f52b1f4))
* **ui:** series/movie detail pages with on-demand xtream info fetches, seasons/episodes navigation, graceful failure paths ([0b3b0bf](https://github.com/tunix/rtv/commit/0b3b0bf21b9aafc54677f706d993d86c7704b455))


### Bug Fixes

* **data:** regenerate PNGs with real alpha (ImageMagick flattened SVGs onto white) ([f4fc329](https://github.com/tunix/rtv/commit/f4fc32915ffb74c29605770212e7019d174447db))
* **ui:** auto-hide timer no-ops via generation guard (SourceId::remove panicked on already-fired ids) ([df95321](https://github.com/tunix/rtv/commit/df953217c06501673a64bf434172667e883ea91e))
