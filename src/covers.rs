//! Cover artwork cache: fetch-on-demand with a persistent disk cache.
//!
//! Files live under `$XDG_CACHE_HOME/rtv/covers/<hash>` keyed by URL hash;
//! failures never surface (the caller keeps its placeholder art) and
//! requests run on the GLib main loop via libsoup3 — no blocking, no extra
//! runtime.

use std::{
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::OnceLock,
};

use gtk::glib;
use soup::prelude::*;

thread_local! {
    static SESSION: soup::Session = soup::Session::new();
}

fn cache_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME");
                PathBuf::from(home).join(".cache")
            });
        let dir = base.join("rtv/covers");
        let _ = fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

fn cache_key(url: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Fetch a cover asynchronously; `done` runs on the main loop with the
/// cached file path, or `None` on failure (the caller keeps its placeholder
/// art — this must never crash or block the UI).
pub fn fetch(url: impl Into<String>, done: impl FnOnce(Option<PathBuf>) + 'static) {
    let url: String = url.into();
    if url.trim().is_empty() {
        done(None);
        return;
    }
    let path = cache_dir().join(cache_key(&url));
    if path.exists() {
        done(Some(path));
        return;
    }

    let Ok(message) = soup::Message::new("GET", &url) else {
        done(None);
        return;
    };
    SESSION.with(|session| {
        session.send_and_read_async(
            &message,
            glib::Priority::DEFAULT,
            None::<&gtk::gio::Cancellable>,
            move |result| match result {
                Ok(bytes) => {
                    if fs::write(&path, &bytes).is_ok() {
                        done(Some(path));
                    } else {
                        done(None);
                    }
                }
                Err(_) => done(None),
            },
        );
    });
}
