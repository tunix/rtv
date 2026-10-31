# Changelog

## [1.0.0](https://github.com/tunix/rtv/compare/v0.0.1...v1.0.0) (2026-10-31)


### ⚠ BREAKING CHANGES

* remove mock mode entirely ([#21](https://github.com/tunix/rtv/issues/21))

### Features

* **data:** new app icon — fanned channel cards ([ffbe643](https://github.com/tunix/rtv/commit/ffbe643145aef30028e6d84ff784e7758fd4049e))
* **details:** offline-first — local samples/series_info|vod_info dumps win over the network ([fdb46bb](https://github.com/tunix/rtv/commit/fdb46bbb93837207ec465be37f7106c45bb1aee9))
* episodes page, image previews, trailer playback, sidebar polish ([7d4d19d](https://github.com/tunix/rtv/commit/7d4d19da3ec874fd597392bae6f68510120d1e99))
* initial project skeleton ([0a49797](https://github.com/tunix/rtv/commit/0a49797aa0f23e5ef01fcda98d8f7741d92fd6ff))
* remove mock mode entirely ([#21](https://github.com/tunix/rtv/issues/21)) ([7d4d19d](https://github.com/tunix/rtv/commit/7d4d19da3ec874fd597392bae6f68510120d1e99))
* **ui:** always-visible favorites with clear-all, Live TV rename, header polish, square card art with heart chip, About credit ([d59c4bf](https://github.com/tunix/rtv/commit/d59c4bf3ae1fbeb3ac2e5dc92d5725313bdb4b4c))
* **ui:** app-frame mock — forced source setup, source picker, nested categories, favorites, full-window player ([e4d74de](https://github.com/tunix/rtv/commit/e4d74de5adabbeff7f4f94fb6c95762001778a73))
* **ui:** category content grid, chrome-free fullscreen, flap-free controls, content terminology ([52bf63a](https://github.com/tunix/rtv/commit/52bf63a19eaae2bdaf696337ce784006d9d851ad))
* **ui:** cover art cache via libsoup3 with disk cache and placeholders; section icons; search UX (sidebar stays visible, category labels, Esc resets) ([ece621f](https://github.com/tunix/rtv/commit/ece621fdf9ec4c33a43fb9f73b6925a1e5c7d7fd))
* **ui:** main window mock — channel list + player placeholder ([17a809b](https://github.com/tunix/rtv/commit/17a809bd883de8fa036549c668ec3aeca66a7c04))
* **ui:** new logo everywhere, auto-hiding player controls with media shortcuts, fullscreen chrome-free; fix icon lookup on GTK 4.22 ([40c394a](https://github.com/tunix/rtv/commit/40c394a7800c5a083e1c4fcc7971fe417a2627ac))
* **ui:** player control bar (seek/play/mute/volume/fullscreen), centered welcome CTA, accurate source-switch toast ([b7faaeb](https://github.com/tunix/rtv/commit/b7faaebae318ae78de905428f82b160e38e8d5bb))
* **ui:** sample catalog loader feeds the mock at real scale — dynamic sections/categories, progressive grid fills, catalog-wide search ([ee9a7fd](https://github.com/tunix/rtv/commit/ee9a7fdf5c0dbea6e536fa84d0b01e45ea45f571))
* **ui:** series/movie detail pages with on-demand xtream info fetches, seasons/episodes navigation, graceful failure paths ([f59a082](https://github.com/tunix/rtv/commit/f59a0826dcaf11de17859485e7bf5b6c7bd0dd6a))


### Bug Fixes

* clickable boxed-list rows, sidebar icons, previews, rating chips ([7d4d19d](https://github.com/tunix/rtv/commit/7d4d19da3ec874fd597392bae6f68510120d1e99))
* **data:** regenerate PNGs with real alpha (ImageMagick flattened SVGs onto white) ([2e0085a](https://github.com/tunix/rtv/commit/2e0085a5c3a653a4b28c581835531374b0d46b38))
* **ui:** auto-hide timer no-ops via generation guard (SourceId::remove panicked on already-fired ids) ([0d2d493](https://github.com/tunix/rtv/commit/0d2d493a5ba7574c1eb898b0be8909b917d07f32))
