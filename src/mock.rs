//! Global `--mock` runtime mode.
//!
//! `rtv --mock` runs the UI against the committed `mock/` dataset instead
//! of a real Xtream source: the catalog loads from `mock/`, detail fetches
//! are served from `mock/series_info/` + `mock/vod_info/` and no network
//! request is ever made. Without the flag the app runs in REAL mode: it
//! starts on the welcome screen, detail fetches go to the real panel API
//! and missing pieces degrade gracefully (toasts).
//!
//! The flag is parsed once in `main()` and read everywhere through this
//! module so neither the sample catalog nor the details layer needs to
//! thread a mode parameter through their signatures.

use std::sync::atomic::{AtomicBool, Ordering};

static MOCK: AtomicBool = AtomicBool::new(false);

/// Called once from `main()` after argument parsing.
pub fn set_enabled(enabled: bool) {
    MOCK.store(enabled, Ordering::SeqCst);
}

/// True when the app was started with `--mock`.
pub fn enabled() -> bool {
    MOCK.load(Ordering::SeqCst)
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
    fn defaults_to_real_mode() {
        // No test runs main(), so the flag must still be off here.
        assert!(!enabled());
        assert_eq!(data_dir(), "samples");
    }
}
