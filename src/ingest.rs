//! Source ingestion pipeline: turn a saved source into catalog data on
//! the GLib main loop (libsoup3 async, same pattern as details.rs).
//! Progress callbacks let the settings dialog show live counts while the
//! fetch chain runs; a failure on one section degrades that section to
//! empty instead of failing the source.
//!
//! PERF LOGGING (the point of this phase): every request logs tag/status/
//! duration/size in xtream.rs; this module logs stage boundaries, per-
//! section totals, parse counts and the overall ingest wall time — the
//! numbers needed to catch slowness or crashes during real load.

use std::time::Instant;

use serde_json::Value;

use crate::catalog::{Catalog, Category, Item, Kind, SeriesMeta, DEFAULT_GROUP};
use crate::ui::sources::SourceKind;
use crate::xtream::{self, XtreamCreds};

/// What the pipeline reports back at each step. Counts accumulate
/// monotonically so the settings rows can show live totals.
#[derive(Debug, Default, Clone)]
pub struct Progress {
    /// Stage label (reserved for richer progress reporting; unused now).
    #[allow(dead_code)]
    pub stage: String,
    pub live_cats: usize,
    pub live_items: usize,
    pub vod_cats: usize,
    pub vod_items: usize,
    pub series_cats: usize,
    pub series_items: usize,
}

impl Progress {
    #[allow(dead_code)]
    pub fn total_items(&self) -> usize {
        self.live_items + self.vod_items + self.series_items
    }

    #[allow(dead_code)]
    pub fn total_cats(&self) -> usize {
        self.live_cats + self.vod_cats + self.series_cats
    }
}

/// Outcome of one full ingest.
pub enum IngestOutcome {
    /// Success: the snapshot is ready for `Window::apply_catalog`.
    Done(Catalog),
    /// Hard failure before anything could load (auth rejected, unreachable).
    Failed(String),
}

/// Kick off ingestion for a source. `on_progress` fires on the main loop
/// between stages; `done` fires exactly once at the end.
pub fn start(
    kind: &SourceKind,
    on_progress: impl Fn(&Progress) + 'static,
    done: impl FnOnce(IngestOutcome) + 'static,
) {
    let started = Instant::now();
    match kind {
        SourceKind::Xtream {
            server,
            username,
            password,
        } => {
            let creds = match parse_server(server) {
                Some(base) => XtreamCreds {
                    base_url: base,
                    username: username.clone(),
                    password: password.clone(),
                },
                None => {
                    done(IngestOutcome::Failed(format!(
                        "Invalid server URL: {server}"
                    )));
                    return;
                }
            };
            tracing::info!(target: "rtv::ingest", "starting Xtream ingest");
            start_xtream(creds, started, on_progress, done);
        }
        SourceKind::M3U { url, .. } => {
            tracing::info!(target: "rtv::ingest", "starting M3U ingest");
            start_m3u(url.clone(), started, on_progress, done);
        }
    }
}

/// Normalize what the user typed into `scheme://host:port`.
pub(crate) fn parse_server(server: &str) -> Option<String> {
    let server = server.trim().trim_end_matches('/');
    if server.is_empty() {
        return None;
    }
    let (scheme, rest) = match server.split_once("://") {
        Some(("http", rest)) => ("http", rest),
        Some(("https", rest)) => ("https", rest),
        Some(_) => return None,
        None => ("http", server),
    };
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

// ---------------------------------------------------------------- Xtream

fn start_xtream(
    creds: XtreamCreds,
    started: Instant,
    on_progress: impl Fn(&Progress) + 'static,
    done: impl FnOnce(IngestOutcome) + 'static,
) {
    let creds_for_auth = creds.clone();
    xtream::auth_check(&creds_for_auth, move |ok| {
        if !ok {
            done(IngestOutcome::Failed(
                "Authentication failed — check the server URL, username and password.".into(),
            ));
            return;
        }
        fetch_section(creds, started, Vec::new(), on_progress, done);
    });
}

/// Fetch one Xtream section's categories + lists, then recurse to the
/// next section; the last one assembles the Catalog. Sections run
/// SEQUENTIALLY (not parallel) so each completed stage reports progress
/// deterministically — the logs show clean stage boundaries this way.
const ACTIONS: &[(&str, Kind, &str, &str)] = &[
    (
        "get_live_categories",
        Kind::Live,
        "get_live_streams",
        "Live",
    ),
    ("get_vod_categories", Kind::Vod, "get_vod_streams", "Movies"),
    (
        "get_series_categories",
        Kind::Series,
        "get_series",
        "Series",
    ),
];

/// Accumulated per-section results while the fetch chain runs.
type SectionData = (Kind, Vec<Category>, Vec<Item>, Vec<(i64, SeriesMeta)>);

fn fetch_section(
    creds: XtreamCreds,
    started: Instant,
    mut acc: Vec<SectionData>,
    on_progress: impl Fn(&Progress) + 'static,
    done: impl FnOnce(IngestOutcome) + 'static,
) {
    let (cat_action, kind, list_action, label) = ACTIONS[acc.len()];
    let tag = format!("categories:{label}");
    let cats_url = creds.api_url(&format!("action={cat_action}"), "");
    xtream::get_json(cats_url, tag, move |cats_value| {
        // Some panels return `{"……": …}` on auth failure instead of an array.
        let cats = cats_value
            .as_ref()
            .and_then(Value::as_array)
            .map(|list| list.as_slice())
            .unwrap_or(&[]);
        let categories: Vec<Category> = cats
            .iter()
            .filter_map(|c| {
                let id = c.get("category_id")?.as_str()?.to_string();
                let name = c.get("category_name")?.as_str()?.to_string();
                Some(Category {
                    kind,
                    id,
                    name,
                    visible: true,
                })
            })
            .collect();

        let list_tag = format!("items:{label}");
        let list_url = creds.api_url(&format!("action={list_action}"), "");
        xtream::get_json(list_url, list_tag, move |list_value| {
            let mut series_meta = Vec::new();
            let items: Vec<Item> = match (kind, list_value.as_ref()) {
                (Kind::Series, Some(Value::Array(list))) => {
                    // get_series entries carry the metadata inline.
                    for s in list {
                        let id = s.get("series_id").and_then(Value::as_i64).or_else(|| {
                            s.get("series_id")
                                .and_then(Value::as_str)
                                .and_then(|v| v.parse().ok())
                        });
                        let Some(id) = id else { continue };
                        let cover = s
                            .get("cover")
                            .and_then(Value::as_str)
                            .filter(|c| !c.is_empty() && *c != "null");
                        let rating = s
                            .get("rating")
                            .and_then(Value::as_str)
                            .filter(|r| !r.is_empty() && *r != "null" && *r != "0")
                            .map(str::to_string);
                        series_meta.push((
                            id,
                            SeriesMeta {
                                plot: str_field(s, "plot"),
                                genre: str_field(s, "genre"),
                                cast: str_field(s, "cast"),
                                director: str_field(s, "director"),
                                rating,
                                release_date: str_field(s, "releaseDate"),
                                backdrop: None,
                                trailer: str_field(s, "youtube_trailer"),
                                cover: cover.map(str::to_string),
                                episode_run_time: str_field(s, "episode_run_time"),
                            },
                        ));
                    }
                    list.iter()
                        .filter_map(|s| {
                            let id = s.get("series_id").and_then(Value::as_i64)?;
                            let category_id =
                                s.get("category_id").and_then(Value::as_str)?.to_string();
                            let name = s.get("name").and_then(Value::as_str)?.to_string();
                            Some(Item {
                                kind,
                                id,
                                category_id,
                                name,
                                icon: s
                                    .get("cover")
                                    .and_then(Value::as_str)
                                    .filter(|c| !c.is_empty() && *c != "null")
                                    .map(str::to_string),
                                rating: s
                                    .get("rating")
                                    .and_then(Value::as_str)
                                    .filter(|r| !r.is_empty() && *r != "0" && *r != "null")
                                    .map(str::to_string),
                                url: None,
                            })
                        })
                        .collect()
                }
                (_, Some(Value::Array(list))) => list
                    .iter()
                    .filter_map(|s| {
                        let id = s.get("stream_id").and_then(Value::as_i64).or_else(|| {
                            s.get("stream_id")
                                .and_then(Value::as_str)
                                .and_then(|v| v.parse().ok())
                        })?;
                        let category_id = s.get("category_id").and_then(Value::as_str)?.to_string();
                        let name = s.get("name").and_then(Value::as_str)?.to_string();
                        Some(Item {
                            kind,
                            id,
                            category_id,
                            name,
                            icon: s
                                .get("stream_icon")
                                .and_then(Value::as_str)
                                .filter(|c| !c.is_empty() && *c != "null")
                                .map(str::to_string),
                            rating: None,
                            url: None,
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            };

            let n_cats = categories.len();
            let n_items = items.len();
            tracing::info!(
                target: "rtv::ingest",
                section = label,
                categories = n_cats,
                items = n_items,
                "section fetched"
            );

            acc.push((kind, categories, items, series_meta));

            // Report progress with what's accumulated so far.
            let mut progress = Progress::default();
            for (k, cats, items, _) in &acc {
                let (c, i) = match k {
                    Kind::Live => (&mut progress.live_cats, &mut progress.live_items),
                    Kind::Vod => (&mut progress.vod_cats, &mut progress.vod_items),
                    Kind::Series => (&mut progress.series_cats, &mut progress.series_items),
                };
                *c = cats.len();
                *i = items.len();
            }
            on_progress(&progress);

            if acc.len() < ACTIONS.len() {
                fetch_section(creds, started, acc, on_progress, done);
                return;
            }

            // Assemble the catalog.
            let mut catalog = Catalog::default();
            let mut orphans = 0usize;
            for (_k, cats, items, meta) in acc {
                catalog.categories.extend(cats);
                catalog.items.extend(items);
                catalog.series_meta.extend(meta);
            }
            // Items pointing at unknown categories (panels do this).
            for item in &catalog.items {
                if !catalog
                    .categories
                    .iter()
                    .any(|c| c.kind == item.kind && c.id == item.category_id)
                {
                    catalog.categories.push(Category {
                        kind: item.kind,
                        id: item.category_id.clone(),
                        name: format!("Category {}", item.category_id),
                        visible: true,
                    });
                    orphans += 1;
                }
            }
            if orphans > 0 {
                tracing::warn!(
                    target: "rtv::ingest",
                    orphans,
                    "items referenced categories missing from the category list — synthesized"
                );
            }
            tracing::info!(
                target: "rtv::ingest",
                ms = started.elapsed().as_millis(),
                categories = catalog.categories.len(),
                items = catalog.items.len(),
                "INGEST COMPLETE"
            );
            done(IngestOutcome::Done(catalog));
        });
    });
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(s)) if !s.is_empty() && s != "null" => Some(s.clone()),
        _ => None,
    }
}

// ------------------------------------------------------------------- M3U

fn start_m3u(
    url: String,
    started: Instant,
    on_progress: impl Fn(&Progress) + 'static,
    done: impl FnOnce(IngestOutcome) + 'static,
) {
    xtream::get_text(url, "m3u playlist".to_string(), move |text| {
        let Some(text) = text else {
            done(IngestOutcome::Failed(
                "Playlist could not be downloaded — check the URL.".into(),
            ));
            return;
        };
        let entries = xtream::parse_m3u(&text);
        if entries.is_empty() {
            done(IngestOutcome::Failed(
                "Playlist contained no entries.".into(),
            ));
            return;
        }

        // Group by group-title; empty group folds into one bucket.
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<&xtream::M3UEntry>> =
            std::collections::HashMap::new();
        for entry in &entries {
            let group = if entry.group.trim().is_empty() {
                DEFAULT_GROUP.to_string()
            } else {
                entry.group.clone()
            };
            let slot = groups.entry(group.clone()).or_insert_with(|| {
                order.push(group.clone());
                Vec::new()
            });
            slot.push(entry);
        }

        let mut catalog = Catalog::default();
        let mut next_id = 0i64;
        for (idx, group) in order.iter().enumerate() {
            let id = format!("m3u-{idx}");
            catalog.categories.push(Category {
                kind: Kind::Live,
                id: id.clone(),
                name: group.clone(),
                visible: true,
            });
            // SQLite PK is (kind, item_id): ids must be unique across
            // the WHOLE source, not per group — use a running counter.
            for entry in groups[group].iter() {
                catalog.items.push(Item {
                    kind: Kind::Live,
                    id: next_id,
                    category_id: id.clone(),
                    name: entry.name.clone(),
                    icon: None,
                    rating: None,
                    url: Some(entry.url.clone()),
                });
                next_id += 1;
            }
        }
        let progress = Progress {
            stage: "m3u".to_string(),
            live_cats: catalog.categories.len(),
            live_items: catalog.items.len(),
            ..Default::default()
        };
        on_progress(&progress);
        tracing::info!(
            target: "rtv::ingest",
            ms = started.elapsed().as_millis(),
            categories = catalog.categories.len(),
            items = catalog.items.len(),
            "M3U INGEST COMPLETE"
        );
        done(IngestOutcome::Done(catalog));
    });
}
