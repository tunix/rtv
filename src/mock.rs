//! Global `--mock` runtime mode.
//!
//! `rtv --mock` runs the UI against the committed `mock/` dataset instead
//! of a real Xtream source: the catalog loads from `mock/`, detail fetches
//! are served from `mock/series_info/` + `mock/vod_info/` and no network
//! request is ever made. Without the flag the app runs in REAL mode: it
//! starts on the welcome screen, detail fetches go to the real panel API
//! and missing pieces degrade gracefully (toasts).
//!
//! Mock mode only exists in DEVELOPMENT builds. [`AVAILABLE`] is derived
//! from `debug_assertions`, so `cargo build` / `cargo run` / `cargo test`
//! have it while `--release` builds (CI artifacts, flatpak bundle) do not:
//! `main()` rejects the flag there and [`enabled`] folds to a constant
//! `false`, making every mock branch in the app dead code the optimizer
//! eliminates.
//!
//! The flag is parsed once in `main()` and read everywhere through this
//! module so neither the sample catalog nor the details layer needs to
//! thread a mode parameter through their signatures.

use std::sync::atomic::{AtomicBool, Ordering};

static MOCK: AtomicBool = AtomicBool::new(false);

/// True in development builds: `--mock` is accepted and functional.
/// Release builds (CI artifacts, flatpak bundle) compile this to `false`
/// and can only run in real mode.
pub const AVAILABLE: bool = cfg!(debug_assertions);

/// Called once from `main()` after argument parsing.
pub fn set_enabled(enabled: bool) {
    if AVAILABLE {
        MOCK.store(enabled, Ordering::SeqCst);
    }
}

/// True when the app was started with `--mock` (dev builds only).
pub fn enabled() -> bool {
    AVAILABLE && MOCK.load(Ordering::SeqCst)
}

/// Directory the active mode reads its offline data from:
/// `mock/` in mock mode, `samples/` (credential-free local dumps) as an
/// OPTIONAL convenience in real mode.
pub fn data_dir() -> &'static str {
    if enabled() {
        "mock"
    } else {
        "samples"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_flag_defaults_off_and_toggles_in_dev() {
        // No test runs main(), so the flag must start off.
        assert!(!enabled());
        assert_eq!(data_dir(), "samples");

        // Tests compile with debug_assertions, so mock mode exists here —
        // a const block makes it a compile-time guarantee, not an assumption.
        const { assert!(AVAILABLE) };
        set_enabled(true);
        assert!(enabled());
        set_enabled(false);
        assert!(!enabled());
    }
}
