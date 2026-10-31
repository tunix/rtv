//! On-demand content details: Xtream `get_series_info` and `get_vod_info`
//! calls against the ACTIVE source's panel, mirroring `covers.rs`:
//! libsoup3 on the GLib main loop, results delivered through a callback,
//! in-memory caching, graceful failure — never blocking, never crashing.
//!
//! Credentials come from the active source (the window supplies a
//! provider closure): the values are NEVER logged, printed or persisted —
//! only placed into the request URL in memory.

use std::{cell::RefCell, collections::HashMap};

use gtk::glib;
use serde_json::Value;
use soup::prelude::*;

/// The panel's origin filters User-Agents (see AGENTS.md): requests with
/// CLI UAs get the connection killed and Cloudflare reports HTTP 520.
/// Every request MUST present an app-like User-Agent.
pub const USER_AGENT: &str = "okhttp/4.9.3";

thread_local! {
    static SESSION: soup::Session = soup::Session::builder().timeout(30).build();
    static SERIES_CACHE: RefCell<HashMap<i64, SeriesInfo>> = RefCell::new(HashMap::new());
    static VOD_CACHE: RefCell<HashMap<i64, VodInfo>> = RefCell::new(HashMap::new());
}

/// The active source's credentials, installed by the window (the model
/// lives there). Called per request — the closure reads the CURRENT
/// selection, so detail fetches follow source switches.
/// The per-call credentials lookup installed by the window.
type CredsProvider = Box<dyn Fn() -> Option<crate::xtream::XtreamCreds>>;

thread_local! {
    static CREDS_PROVIDER: RefCell<Option<CredsProvider>> = RefCell::new(None);
}

/// Install the credentials provider (done once by the window).
pub fn install_creds_provider(provider: impl Fn() -> Option<crate::xtream::XtreamCreds> + 'static) {
    CREDS_PROVIDER.with(|slot| *slot.borrow_mut() = Some(Box::new(provider)));
}

/// Compose the player_api.php URL for one action against the active
/// source. Credentials only ever live inside the returned String, in
/// memory. `None` when no Xtream source is active (M3U sources have no
/// details API).
fn api_url(query: &str) -> Option<String> {
    let creds = CREDS_PROVIDER.with(|slot| slot.borrow().as_ref().and_then(|f| f()))?;
    Some(creds.api_url(query, ""))
}

/// One season as reported by `get_series_info`.
#[derive(Debug, Clone)]
pub struct Season {
    pub number: i64,
    pub episode_count: Option<i64>,
    pub air_date: Option<String>,
    pub cover: Option<String>,
}

/// One episode of one season.
#[derive(Debug, Clone)]
pub struct Episode {
    pub number: i64,
    pub title: String,
    pub duration: Option<String>,
    /// Short summary from `info.plot`, shown on the episode rows.
    pub plot: Option<String>,
    /// Episode thumbnail (`info.movie_image`), shown on the episode rows.
    pub image: Option<String>,
}

/// Parsed `get_series_info` response. A season listed in `seasons[]` may
/// be missing from the `episodes` map (and vice versa) — both are kept.
#[derive(Debug, Clone, Default)]
pub struct SeriesInfo {
    pub seasons: Vec<Season>,
    /// Season number -> episodes (the API keys seasons by STRING).
    pub episodes: HashMap<i64, Vec<Episode>>,
}

/// Parsed `get_vod_info` response (`info` object only — what the UI needs).
#[derive(Debug, Clone, Default)]
pub struct VodInfo {
    pub plot: Option<String>,
    pub genre: Option<String>,
    pub cast: Option<String>,
    pub director: Option<String>,
    pub duration: Option<String>,
    pub release_date: Option<String>,
    /// YouTube trailer link, when the panel carries one.
    pub trailer: Option<String>,
}

/// Fetch series info (seasons + episodes) asynchronously; `done` runs on
/// the main loop with `None` on any failure. Results are cached in memory.
/// Queries the ACTIVE source's panel.
pub fn series_info(series_id: i64, done: impl FnOnce(Option<SeriesInfo>) + 'static) {
    if let Some(cached) = SERIES_CACHE.with(|c| c.borrow().get(&series_id).cloned()) {
        done(Some(cached));
        return;
    }
    let url = api_url(&format!("action=get_series_info&series_id={series_id}"));
    if let Some(url) = url {
        get_json(&url, move |value| {
            let result = value.as_ref().and_then(parse_series_info);
            if let Some(info) = &result {
                SERIES_CACHE.with(|c| c.borrow_mut().insert(series_id, info.clone()));
            }
            done(result);
        });
    } else {
        // No active Xtream source (M3U has no details API).
        done(None);
    }
}

/// Fetch VOD info (plot/genre/cast/...) asynchronously; `done` runs on the
/// main loop with `None` on any failure. Results are cached in memory.
/// Queries the ACTIVE source's panel.
pub fn vod_info(vod_id: i64, done: impl FnOnce(Option<VodInfo>) + 'static) {
    if let Some(cached) = VOD_CACHE.with(|c| c.borrow().get(&vod_id).cloned()) {
        done(Some(cached));
        return;
    }
    let url = api_url(&format!("action=get_vod_info&vod_id={vod_id}"));
    if let Some(url) = url {
        get_json(&url, move |value| {
            let result = value.as_ref().and_then(parse_vod_info);
            if let Some(info) = &result {
                VOD_CACHE.with(|c| c.borrow_mut().insert(vod_id, info.clone()));
            }
            done(result);
        });
    } else {
        // No active Xtream source (M3U has no details API).
        done(None);
    }
}

/// GET a JSON document; `done` receives the parsed value or `None` on any
/// transport/HTTP/parse failure. Never blocks: the callback runs on the
/// GLib main loop, exactly like `covers.rs`.
fn get_json(url: &str, done: impl FnOnce(Option<Value>) + 'static) {
    let Ok(message) = soup::Message::new("GET", url) else {
        done(None);
        return;
    };
    if let Some(headers) = message.request_headers() {
        headers.append("User-Agent", USER_AGENT);
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

/// Lenient string field: `None` for missing, null, empty and the literal
/// string "null" (panels emit that for absent metadata).
fn str_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(s)) if !s.is_empty() && s != "null" => Some(s.clone()),
        _ => None,
    }
}

/// Lenient integer field: accepts numbers and numeric strings.
fn num_field(value: &Value, key: &str) -> Option<i64> {
    match value.get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_series_info(value: &Value) -> Option<SeriesInfo> {
    let seasons_json = value.get("seasons").and_then(Value::as_array);
    let episodes_json = value.get("episodes").and_then(Value::as_object);
    if seasons_json.is_none() && episodes_json.is_none() {
        // Not a series response at all (error page, other panel version).
        return None;
    }

    let mut info = SeriesInfo::default();
    if let Some(seasons) = seasons_json {
        for season in seasons {
            info.seasons.push(Season {
                number: num_field(season, "season_number").unwrap_or(0),
                episode_count: num_field(season, "episode_count"),
                air_date: str_field(season, "air_date"),
                cover: str_field(season, "cover"),
            });
        }
    }
    if let Some(episodes) = episodes_json {
        for (season_key, list) in episodes {
            let Ok(season) = season_key.parse::<i64>() else {
                continue;
            };
            let eps: Vec<Episode> = list
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|e| {
                            let number = num_field(e, "episode_num").unwrap_or(0);
                            Episode {
                                number,
                                title: str_field(e, "title")
                                    .unwrap_or_else(|| format!("Episode {number}")),
                                duration: e.get("info").and_then(|i| str_field(i, "duration")),
                                plot: e.get("info").and_then(|i| str_field(i, "plot")),
                                image: e
                                    .get("info")
                                    .and_then(|i| str_field(i, "movie_image"))
                                    .or_else(|| str_field(e, "movie_image")),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            info.episodes.insert(season, eps);
        }
    }

    // Seasons may exist only in `seasons[]` (episodes map missing the key)
    // or only in the episodes map — union both, sorted by season number.
    for season in info.episodes.keys() {
        if !info.seasons.iter().any(|s| s.number == *season) {
            info.seasons.push(Season {
                number: *season,
                episode_count: None,
                air_date: None,
                cover: None,
            });
        }
    }
    info.seasons.sort_by_key(|s| s.number);
    Some(info)
}

fn parse_vod_info(value: &Value) -> Option<VodInfo> {
    let info = value.get("info")?;
    Some(VodInfo {
        plot: str_field(info, "plot"),
        genre: str_field(info, "genre"),
        cast: str_field(info, "cast"),
        director: str_field(info, "director"),
        duration: str_field(info, "duration"),
        release_date: str_field(info, "releasedate").or_else(|| str_field(info, "releaseDate")),
        trailer: str_field(info, "youtube_trailer"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_sample_series_info_dump() {
        // samples/ is gitignored — the test only runs where the dump exists.
        let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR")))
            .join("samples")
            .join("series_info_sample.json");
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let value: Value = serde_json::from_str(&text).expect("sample is JSON");
        let info = parse_series_info(&value).expect("sample parses");

        // "Specials" (season 0) comes from seasons[], season 1 carries its
        // episodes under the STRING key "1" — the union must contain both
        // seasons, sorted.
        let numbers: Vec<i64> = info.seasons.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![0, 1]);
        assert_eq!(info.seasons[0].episode_count, Some(1));
        let episodes = &info.episodes[&1];
        assert_eq!(episodes.len(), 10);
        assert_eq!(episodes[0].number, 1);
        assert!(episodes[0].duration.is_some());
        // The season without an episodes entry still yields an empty list.
        assert!(info.episodes.get(&0).unwrap_or(&Vec::new()).is_empty());
    }

    #[test]
    fn rejects_non_details_responses() {
        // What the panel answers when the credentials no longer authenticate.
        let auth_fail: Value = serde_json::from_str(r#"{"user_info":{"auth":0}}"#).unwrap();
        assert!(parse_series_info(&auth_fail).is_none());
        assert!(parse_vod_info(&auth_fail).is_none());
    }

    #[test]
    fn treats_literal_null_metadata_as_absent() {
        let vod: Value =
            serde_json::from_str(r#"{"info":{"plot":"null","genre":"Dram","releasedate":null}}"#)
                .unwrap();
        let info = parse_vod_info(&vod).unwrap();
        assert!(info.plot.is_none());
        assert_eq!(info.genre.as_deref(), Some("Dram"));
        assert!(info.release_date.is_none());
    }
}
