//! Live ratings lookup: the panel's own rating is often stale, so scores
//! are resolved from open, keyless metadata sources on demand — libsoup3
//! on the GLib main loop (never blocking, never crashing), an in-memory
//! cache plus a disk cache with a freshness window, graceful failure.
//! (Mock mode fetches artwork the same way, so lookups run there too;
//! offline they fail gracefully.)
//!
//! Provider chain (all keyless, all speaking JSON):
//! - **IMDb suggestion API** (`v2.sg.media-imdb.com`): resolves the
//!   title to its exact IMDb id — the IMDb chip links to the exact
//!   title page instead of a search.
//! - **Cinemeta** (Stremio's metadata addon; movies AND series): the
//!   meta endpoint by IMDb id carries `imdbRating` + the 16:9 backdrop.
//! - **Cinemeta title search** (fallback when the suggestion missed).
//! - **TVmaze search** (series fallback): carries the IMDb-sourced
//!   average rating.
//!
//! When no source provides a score, the UI falls back to the panel's
//! own rating and finally to "N/A". The Rotten Tomatoes and Wikipedia
//! chips are deliberately link-only: RT has no keyless JSON source and
//! Wikipedia carries no film ratings.

use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    rc::Rc,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use gtk::glib;
use serde_json::Value;
use soup::prelude::*;

use crate::sample::Kind;

/// Fetched ratings for one title. Fields stay `None` when no source
/// provides them (the UI then falls back to the panel rating / "N/A").
#[derive(Debug, Clone, Default)]
pub struct Ratings {
    /// IMDb score ("8.4"), when a source provides it.
    pub imdb: Option<String>,
    /// Resolved IMDb id ("tt1375666").
    pub imdb_id: Option<String>,
    /// 16:9 backdrop artwork from Cinemeta — doubles as the trailer
    /// row thumbnail.
    pub backdrop: Option<String>,
}

/// Disk cache freshness window; ratings drift slowly.
const TTL_SECONDS: u64 = 7 * 24 * 60 * 60;

/// The single-shot callback handed through the provider chain.
type DoneCallback = Box<dyn FnOnce(Option<Ratings>)>;

thread_local! {
    static SESSION: soup::Session = soup::Session::builder().timeout(30).build();
    static MEM_CACHE: RefCell<HashMap<String, Ratings>> = RefCell::new(HashMap::new());
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
        let dir = base.join("rtv/ratings");
        let _ = fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

fn cache_key(kind: Kind, name: &str, year: Option<&str>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (kind as i32).hash(&mut hasher);
    name.to_lowercase().hash(&mut hasher);
    year.unwrap_or_default().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Look up ratings for a title; `done` runs on the main loop with the
/// ratings (possibly partial) or `None` on total failure. A fresh cache
/// answers instantly; a fetch result is cached in memory and on disk.
pub fn lookup(
    kind: Kind,
    name: &str,
    year: Option<&str>,
    done: impl FnOnce(Option<Ratings>) + 'static,
) {
    let key = cache_key(kind, name, year);
    if let Some(ratings) = MEM_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        done(Some(ratings));
        return;
    }
    if let Some(ratings) = read_cache(&key) {
        MEM_CACHE.with(|c| c.borrow_mut().insert(key, ratings.clone()));
        done(Some(ratings));
        return;
    }

    // Mock mode fetches artwork over the network already (covers), so
    // live lookups run there too; offline they fail gracefully.
    fetch_chain(
        kind,
        name.to_string(),
        year.map(str::to_string),
        move |ratings| {
            if let Some(ratings) = &ratings {
                write_cache(&key, ratings);
                MEM_CACHE.with(|c| c.borrow_mut().insert(key.clone(), ratings.clone()));
            }
            done(ratings);
        },
    );
}

fn read_cache(key: &str) -> Option<Ratings> {
    let text = fs::read_to_string(cache_dir().join(key)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let fetched_at = value.get("fetched_at").and_then(Value::as_u64)?;
    if fetched_at + TTL_SECONDS <= now() {
        return None;
    }
    // Entries from older schema versions (missing fields) count as
    // stale, so a lookup refreshes them with the new data.
    value.get("backdrop")?;
    Some(Ratings {
        imdb: str_field(&value, "imdb"),
        imdb_id: str_field(&value, "imdb_id"),
        backdrop: str_field(&value, "backdrop"),
    })
}

fn write_cache(key: &str, ratings: &Ratings) {
    let entry = serde_json::json!({
        "fetched_at": now(),
        "imdb": ratings.imdb,
        "imdb_id": ratings.imdb_id,
        "backdrop": ratings.backdrop,
    });
    let _ = fs::write(cache_dir().join(key), entry.to_string());
}

/// The provider chain: IMDb's own keyless suggestion endpoint resolves
/// the title to its exact IMDb id first (so the IMDb chip links to the
/// exact title page); Cinemeta's meta endpoint then carries the
/// IMDb-sourced score + backdrop by that id. Cinemeta title-search is
/// the fallback when the suggestion misses, TVmaze the series fallback
/// when both miss. `done` runs exactly once.
fn fetch_chain(
    kind: Kind,
    name: String,
    year: Option<String>,
    done: impl FnOnce(Option<Ratings>) + 'static,
) {
    let ratings: Rc<RefCell<Ratings>> = Rc::new(RefCell::new(Ratings::default()));
    let done: Rc<RefCell<Option<DoneCallback>>> = Rc::new(RefCell::new(Some(Box::new(done))));

    // Step 1: IMDb suggestion (path-keyed by the first character).
    let trimmed = name.trim();
    let initial = trimmed
        .chars()
        .next()
        .map(|c| c.to_lowercase().next().unwrap_or('_'))
        .unwrap_or('_');
    let initial = if initial.is_ascii_alphanumeric() {
        initial.to_string()
    } else {
        "_".to_string()
    };
    let query = glib::uri_escape_string(trimmed, None::<&str>, false).to_string();
    let suggestion_url = format!("https://v2.sg.media-imdb.com/suggestion/{initial}/{query}.json");

    get_json(&suggestion_url, move |value| {
        let suggestion_id = value
            .as_ref()
            .and_then(|v| pick_suggestion(v, &name, &year));
        match suggestion_id {
            // Step 2a: exact IMDb id — Cinemeta meta by id.
            Some(imdb_id) => {
                ratings.borrow_mut().imdb_id = Some(imdb_id.clone());
                cinemeta_meta(kind, imdb_id, name.clone(), year, done, ratings);
            }
            // Step 2b: suggestion missed — Cinemeta title search.
            None => cinemeta_search(kind, name, year, done, ratings),
        }
    });
}

/// Cinemeta meta by exact IMDb id: the freshest score + backdrop, then
/// the TVmaze fallback for series when the meta carries no rating.
fn cinemeta_meta(
    kind: Kind,
    imdb_id: String,
    name: String,
    year: Option<String>,
    done: Rc<RefCell<Option<DoneCallback>>>,
    ratings: Rc<RefCell<Ratings>>,
) {
    let catalog_type = match kind {
        Kind::Vod => "movie",
        _ => "series",
    };
    let meta_url = format!("https://v3-cinemeta.strem.io/meta/{catalog_type}/{imdb_id}.json");
    get_json(&meta_url, move |value| {
        if let Some(value) = &value {
            {
                let mut pending = ratings.borrow_mut();
                pending.imdb = cinemeta_rating(value);
                pending.backdrop = value.get("meta").and_then(|m| str_field(m, "background"));
            }
        }
        if kind == Kind::Series && ratings.borrow().imdb.is_none() {
            // Cinemeta carried no rating for this series: TVmaze still
            // covers it.
            tvmaze_lookup(name, year, done, ratings);
        } else {
            fire(&done, &ratings);
        }
    });
}

/// Cinemeta title search (fallback when the IMDb suggestion missed).
fn cinemeta_search(
    kind: Kind,
    name: String,
    year: Option<String>,
    done: Rc<RefCell<Option<DoneCallback>>>,
    ratings: Rc<RefCell<Ratings>>,
) {
    let catalog_type = match kind {
        Kind::Vod => "movie",
        _ => "series",
    };
    let query = glib::uri_escape_string(name.trim(), None::<&str>, false).to_string();
    let search_url =
        format!("https://v3-cinemeta.strem.io/catalog/{catalog_type}/top/search={query}.json");
    get_json(&search_url, move |value| {
        let picked = value.as_ref().and_then(|v| pick_cinemeta(v, &name, &year));
        let Some((imdb_id, meta_type)) = picked else {
            // Cinemeta missed: TVmaze still covers series.
            if kind == Kind::Series {
                tvmaze_lookup(name, year, done, ratings);
            } else {
                fire(&done, &ratings);
            }
            return;
        };
        ratings.borrow_mut().imdb_id = Some(imdb_id.clone());
        let meta_url = format!("https://v3-cinemeta.strem.io/meta/{meta_type}/{imdb_id}.json");
        get_json(&meta_url, move |value| {
            if let Some(value) = &value {
                {
                    let mut pending = ratings.borrow_mut();
                    pending.imdb = cinemeta_rating(value);
                    pending.backdrop = value.get("meta").and_then(|m| str_field(m, "background"));
                }
            }
            if kind == Kind::Series && ratings.borrow().imdb.is_none() {
                tvmaze_lookup(name, year, done, ratings);
            } else {
                fire(&done, &ratings);
            }
        });
    });
}

/// TVmaze search: the IMDb-sourced average rating for series (the last
/// keyless fallback).
fn tvmaze_lookup(
    name: String,
    year: Option<String>,
    done: Rc<RefCell<Option<DoneCallback>>>,
    ratings: Rc<RefCell<Ratings>>,
) {
    let query = glib::uri_escape_string(name.trim(), None::<&str>, false).to_string();
    let tvmaze_url = format!("https://api.tvmaze.com/search/shows?q={query}");
    get_json(&tvmaze_url, move |value| {
        if let Some(value) = &value {
            fill_tvmaze(value, &name, &year, &mut ratings.borrow_mut());
        }
        fire(&done, &ratings);
    });
}

/// Fire the pending callback with the current state.
fn fire(done: &Rc<RefCell<Option<DoneCallback>>>, ratings: &Rc<RefCell<Ratings>>) {
    let snapshot = ratings.borrow().clone();
    if let Some(done) = done.borrow_mut().take() {
        done(Some(snapshot));
    }
}

/// The Cinemeta meta endpoint's `meta.imdbRating` ("8.8"), when sane.
fn cinemeta_rating(value: &Value) -> Option<String> {
    let rating = value
        .get("meta")
        .and_then(|m| m.get("imdbRating"))
        .and_then(Value::as_str)?;
    let score = rating.parse::<f64>().ok()?;
    ((0.0..=10.0).contains(&score)).then(|| format!("{score:.1}"))
}

/// Pick the Cinemeta catalog entry matching the title (and year, when
/// known): exact title + year, then any exact title match, then a title
/// prefix match. Returns (imdb_id, meta_type).
fn pick_cinemeta(value: &Value, name: &str, year: &Option<String>) -> Option<(String, String)> {
    let metas = value.get("metas")?.as_array()?;
    let wanted = name.trim().to_lowercase();
    let year_num = year.as_deref().and_then(year_prefix);
    let mut exact = None;
    let mut prefix = None;
    for meta in metas {
        let Some(id) = meta
            .get("imdb_id")
            .or_else(|| meta.get("id"))
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("tt"))
        else {
            continue;
        };
        let Some(title) = meta.get("name").and_then(Value::as_str) else {
            continue;
        };
        let meta_type = meta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("movie")
            .to_string();
        let title = title.to_lowercase();
        let release_year = meta
            .get("releaseInfo")
            .and_then(Value::as_str)
            .and_then(year_prefix);
        let year_match = match (year_num, release_year) {
            (Some(wanted), Some(actual)) => actual == wanted,
            _ => true,
        };
        if title == wanted {
            if year_match {
                return Some((id.to_string(), meta_type));
            }
            exact = exact.or_else(|| Some((id.to_string(), meta_type)));
        } else if title.starts_with(&wanted) {
            prefix = prefix.or_else(|| Some((id.to_string(), meta_type)));
        }
    }
    exact.or(prefix)
}

/// Pick the IMDb suggestion entry matching the title (and year, when
/// known): exact title + year, then any exact title match, then a title
/// prefix match. Returns the IMDb id.
fn pick_suggestion(value: &Value, name: &str, year: &Option<String>) -> Option<String> {
    let entries = value.get("d")?.as_array()?;
    let wanted = name.trim().to_lowercase();
    let year_num = year.as_deref().and_then(year_prefix);
    let mut exact = None;
    let mut prefix = None;
    for entry in entries {
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("tt"))
        else {
            continue;
        };
        let Some(title) = entry.get("l").and_then(Value::as_str) else {
            continue;
        };
        let title = title.to_lowercase();
        let entry_year = entry.get("y").and_then(Value::as_i64);
        let year_match = match (year_num, entry_year) {
            (Some(wanted), Some(actual)) => actual == wanted,
            _ => true,
        };
        if title == wanted {
            if year_match {
                return Some(id.to_string());
            }
            exact = exact.or_else(|| Some(id.to_string()));
        } else if title.starts_with(&wanted) {
            prefix = prefix.or_else(|| Some(id.to_string()));
        }
    }
    exact.or(prefix)
}

/// TVmaze search: the best name (and year) match carries the score —
/// exact title with matching year first, then any exact title match.
fn fill_tvmaze(value: &Value, name: &str, year: &Option<String>, ratings: &mut Ratings) {
    let Some(entries) = value.as_array() else {
        return;
    };
    let wanted = name.trim().to_lowercase();
    let year_num = year.as_deref().and_then(year_prefix);
    let mut exact_with_year = None;
    let mut exact_any = None;
    for entry in entries {
        let Some(show) = entry.get("show") else {
            continue;
        };
        let Some(title) = show.get("name").and_then(Value::as_str) else {
            continue;
        };
        if title.to_lowercase() != wanted {
            continue;
        }
        let premiered = show
            .get("premiered")
            .and_then(Value::as_str)
            .and_then(year_prefix);
        let year_match = match (year_num, premiered) {
            (Some(wanted), Some(actual)) => actual == wanted,
            _ => true,
        };
        if year_match {
            exact_with_year = Some(show);
            break;
        }
        exact_any = exact_any.or(Some(show));
    }
    let Some(show) = exact_with_year.or(exact_any) else {
        return;
    };
    if let Some(score) = show
        .get("rating")
        .and_then(|r| r.get("average"))
        .and_then(Value::as_f64)
    {
        ratings.imdb = Some(format!("{score:.1}"));
    }
    if ratings.imdb_id.is_none() {
        ratings.imdb_id = show
            .get("externals")
            .and_then(|x| x.get("imdb"))
            .and_then(Value::as_str)
            .map(String::from);
    }
}

/// "2015-09-01" / "2008–2013" (en dash) -> 2008.
fn year_prefix(year: &str) -> Option<i64> {
    year.split(['-', '–']).next()?.trim().parse().ok()
}

/// Lenient string field: `None` for missing, null and empty values.
fn str_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(s)) if !s.is_empty() && s != "null" => Some(s.clone()),
        _ => None,
    }
}

/// GET a JSON document; `done` receives the parsed value or `None` on any
/// transport/HTTP/parse failure. Runs on the GLib main loop, exactly like
/// `covers.rs` and `details.rs`.
fn get_json(url: &str, done: impl FnOnce(Option<Value>) + 'static) {
    let Ok(message) = soup::Message::new("GET", url) else {
        done(None);
        return;
    };
    if let Some(headers) = message.request_headers() {
        headers.append("User-Agent", crate::details::USER_AGENT);
    }
    let status_message = message.clone();
    SESSION.with(|session| {
        session.send_and_read_async(
            &message,
            glib::Priority::DEFAULT,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                let value = match result {
                    Ok(bytes) if (200..300).contains(&status_message.status_code()) => {
                        serde_json::from_slice(&bytes).ok()
                    }
                    _ => None,
                };
                done(value);
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn suggestion_picks_title_and_year() {
        let value = json!({
            "d": [
                {"id": "tt999", "l": "1883: The Sequel", "y": 2025},
                {"id": "tt111", "l": "1883", "y": 2021},
                {"id": "tt222", "l": "1883", "y": 2019}
            ]
        });
        // Exact title + matching year beats everything.
        let year = Some("2021-06-01".to_string());
        assert_eq!(
            pick_suggestion(&value, "1883", &year).as_deref(),
            Some("tt111")
        );
        // No year: the exact title match still wins over a prefix match.
        assert_eq!(
            pick_suggestion(&value, "1883", &None).as_deref(),
            Some("tt111")
        );
        // Unrelated title: no match.
        assert_eq!(pick_suggestion(&value, "yellowstone", &None), None);
    }

    #[test]
    fn cinemeta_picks_title_and_type() {
        let value = json!({
            "metas": [
                {"id": "tt999", "imdb_id": "tt999", "type": "series",
                 "name": "1883: The Sequel", "releaseInfo": "2025"},
                {"id": "tt111", "imdb_id": "tt111", "type": "series",
                 "name": "1883", "releaseInfo": "2021–2022"},
                {"id": "tt222", "imdb_id": "tt222", "type": "movie",
                 "name": "1883", "releaseInfo": "2019"}
            ]
        });
        // Exact title + matching year (en-dash ranges parse).
        let year = Some("2021-06-01".to_string());
        assert_eq!(
            pick_cinemeta(&value, "1883", &year),
            Some(("tt111".to_string(), "series".to_string()))
        );
        // No year: the exact title match still wins over a prefix match.
        assert_eq!(
            pick_cinemeta(&value, "1883", &None),
            Some(("tt111".to_string(), "series".to_string()))
        );
        // Unrelated title: no match.
        assert_eq!(pick_cinemeta(&value, "yellowstone", &None), None);
    }

    #[test]
    fn cinemeta_meta_rating_is_sane() {
        let meta = json!({"meta": {"name": "Inception", "imdbRating": "8.8"}});
        assert_eq!(cinemeta_rating(&meta).as_deref(), Some("8.8"));
        let junk = json!({"meta": {"imdbRating": "null"}});
        assert_eq!(cinemeta_rating(&junk), None);
    }

    #[test]
    fn tvmaze_fill_matches_name_and_year() {
        let value = json!([
            {"show": {"name": "Dexter", "premiered": "2006-10-01",
                      "rating": {"average": 8.4}, "externals": {"imdb": "tt0903747"}}},
            {"show": {"name": "Dexter: New Blood", "premiered": "2021-11-07",
                      "rating": {"average": 7.6}, "externals": {"imdb": "tt9999999"}}}
        ]);
        let mut ratings = Ratings::default();
        fill_tvmaze(&value, "Dexter", &Some("2006".to_string()), &mut ratings);
        assert_eq!(ratings.imdb.as_deref(), Some("8.4"));
        assert_eq!(ratings.imdb_id.as_deref(), Some("tt0903747"));

        // A year with no matching exact-title entry falls back to the
        // exact title match ("Dexter: New Blood" is a different name).
        let mut ratings = Ratings::default();
        fill_tvmaze(&value, "Dexter", &Some("2021".to_string()), &mut ratings);
        assert_eq!(ratings.imdb.as_deref(), Some("8.4"));

        // Without a year the first exact title match wins.
        let mut ratings = Ratings::default();
        fill_tvmaze(&value, "Dexter", &None, &mut ratings);
        assert_eq!(ratings.imdb.as_deref(), Some("8.4"));

        // Unrelated title: nothing filled.
        let mut ratings = Ratings::default();
        fill_tvmaze(&value, "Yellowstone", &None, &mut ratings);
        assert!(ratings.imdb.is_none());
    }

    #[test]
    fn cache_round_trips_ratings() {
        let ratings = Ratings {
            imdb: Some("9.2".to_string()),
            imdb_id: Some("tt0903747".to_string()),
            backdrop: Some(
                "https://images.metahub.space/background/medium/tt0903747/img".to_string(),
            ),
        };
        let entry = serde_json::json!({
            "fetched_at": now(),
            "imdb": ratings.imdb,
            "imdb_id": ratings.imdb_id,
            "backdrop": ratings.backdrop,
        });
        // Re-parse what write_cache stores.
        let value: Value = serde_json::from_str(&entry.to_string()).unwrap();
        let round = Ratings {
            imdb: str_field(&value, "imdb"),
            imdb_id: str_field(&value, "imdb_id"),
            backdrop: str_field(&value, "backdrop"),
        };
        assert_eq!(round.imdb.as_deref(), Some("9.2"));
        assert_eq!(round.imdb_id.as_deref(), Some("tt0903747"));
        assert!(round.backdrop.is_some());
    }

    /// Live provider chain: IMDb suggestion + TVmaze. Ignored by default
    /// (network); run manually with `xvfb-run cargo test -- --ignored`.
    #[test]
    #[ignore = "hits the network; run manually: xvfb-run cargo test -- --ignored"]
    fn live_lookup_resolves_a_real_series() {
        gtk::init().expect("gtk init");
        let received: Rc<RefCell<Option<Ratings>>> = Rc::new(RefCell::new(None));
        let loop_ = glib::MainLoop::new(None, false);
        // The lookup is scheduled from an idle callback: a synchronous
        // cache hit would otherwise call quit() before run() starts —
        // which GLib ignores, hanging the loop forever.
        glib::idle_add_local_once({
            let quit = loop_.clone();
            let sink = received.clone();
            move || {
                lookup(Kind::Series, "Dexter", Some("2006"), move |ratings| {
                    *sink.borrow_mut() = ratings;
                    quit.quit();
                });
            }
        });
        loop_.run();
        let ratings = received.borrow().clone().expect("lookup succeeded");
        assert!(ratings.imdb.is_some(), "expected an IMDb score");
        assert!(
            ratings
                .imdb_id
                .as_deref()
                .unwrap_or_default()
                .starts_with("tt"),
            "expected a resolved IMDb id"
        );
    }
}
