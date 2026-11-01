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

/// Normalize what the user typed into `scheme://host:port`. Shared by
/// the form's live validation and the ingest itself; `None` means the
/// input is not a usable server address (invalid scheme, no host).
pub fn parse_server(server: &str) -> Option<String> {
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
    if rest.is_empty() || rest.contains(char::is_whitespace) || rest == ":" {
        return None;
    }
    // Scheme-only input ("http:" after trailing-slash trim) would
    // otherwise surface as "http://http:".
    if rest.starts_with(':') || rest.ends_with(':') {
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
        let categories: Vec<Category> = parse_category_list(cats, kind);

        let list_tag = format!("items:{label}");
        let list_url = creds.api_url(&format!("action={list_action}"), "");
        xtream::get_json(list_url, list_tag, move |list_value| {
            let (items, series_meta) = match (kind, list_value.as_ref()) {
                (Kind::Series, Some(Value::Array(list))) => parse_series_list(list),
                (_, Some(Value::Array(list))) => (parse_stream_list(list, kind), Vec::new()),
                _ => (Vec::new(), Vec::new()),
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

            // Assemble the catalog (orphan categories synthesized).
            let (catalog, orphans) = assemble_catalog(acc);
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

// ---------------------------------------------------- pure parsing fns
// Extracted from the fetch chain so the panel-shape handling is unit-
// testable without network or GTK.

/// Parse one section's category array. Non-array/broken entries are
/// skipped; every parsed row is visible by default.
fn parse_category_list(cats: &[Value], kind: Kind) -> Vec<Category> {
    cats.iter()
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
        .collect()
}

/// Parse the get_series array: items plus inline SeriesMeta (the panel
/// carries the metadata inline in get_series). Lenient on shapes: ids
/// may arrive as numbers or strings; null/empty/0 ratings are absent.
fn parse_series_list(list: &[Value]) -> (Vec<Item>, Vec<(i64, SeriesMeta)>) {
    let mut meta = Vec::new();
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
        meta.push((
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
    let items = list
        .iter()
        .filter_map(|s| {
            let id = s.get("series_id").and_then(Value::as_i64)?;
            let category_id = s.get("category_id").and_then(Value::as_str)?.to_string();
            let name = s.get("name").and_then(Value::as_str)?.to_string();
            Some(Item {
                kind: Kind::Series,
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
        .collect();
    (items, meta)
}

/// Parse a live/vod stream array (stream_id + category_id + name).
fn parse_stream_list(list: &[Value], kind: Kind) -> Vec<Item> {
    list.iter()
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
        .collect()
}

/// Merge parsed sections into a Catalog. Items referencing categories
/// missing from the category lists are synthesized (panels do this) and
/// the count is returned alongside.
fn assemble_catalog(sections: Vec<SectionData>) -> (Catalog, usize) {
    let mut catalog = Catalog::default();
    let mut orphans = 0usize;
    for (_k, cats, items, meta) in sections {
        catalog.categories.extend(cats);
        catalog.items.extend(items);
        catalog.series_meta.extend(meta);
    }
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
    (catalog, orphans)
}

/// Build the M3U catalog from parsed entries: one category per
/// group-title (empty groups fold into one bucket), running item ids
/// (SQLite PK is kind+item_id — must be unique source-wide).
fn build_m3u_catalog(entries: &[xtream::M3UEntry]) -> Catalog {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<&xtream::M3UEntry>> =
        std::collections::HashMap::new();
    for entry in entries {
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
    catalog
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

        // Group by group-title (empty groups fold into one bucket) and
        // build the catalog with running item ids.
        let catalog = build_m3u_catalog(&entries);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- parse_server ----

    #[test]
    fn parse_server_adds_http_scheme_when_missing() {
        assert_eq!(
            parse_server("panel.example.com:8080"),
            Some("http://panel.example.com:8080".into())
        );
    }

    #[test]
    fn parse_server_keeps_explicit_schemes_and_trims_slashes() {
        assert_eq!(
            parse_server("https://panel.example.com///"),
            Some("https://panel.example.com".into())
        );
        assert_eq!(
            parse_server("http://panel.example.com:80"),
            Some("http://panel.example.com:80".into())
        );
    }

    #[test]
    fn parse_server_rejects_bad_input() {
        assert_eq!(parse_server(""), None);
        assert_eq!(parse_server("   "), None);
        assert_eq!(parse_server("ftp://host"), None);
        assert_eq!(parse_server("http://host with space"), None);
        assert_eq!(parse_server("http://"), None);
    }

    // ---- parse_category_list ----

    #[test]
    fn parse_categories_keeps_wellformed_rows_only() {
        let raw = vec![
            json!({"category_id": "1", "category_name": "News"}),
            json!({"category_id": 2, "category_name": "Numeric id"}),
            json!({"category_name": "no id"}),
            json!({"category_id": "3"}),
        ];
        let cats = parse_category_list(&raw, Kind::Live);
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].id, "1");
        assert_eq!(cats[0].name, "News");
        assert!(cats[0].visible);
    }

    // ---- parse_stream_list ----

    #[test]
    fn parse_streams_accepts_numeric_and_string_ids() {
        let raw = vec![
            json!({"stream_id": 10, "category_id": "L1", "name": "Numeric"}),
            json!({"stream_id": "20", "category_id": "L1", "name": "String"}),
            json!({"stream_id": 30, "category_id": "L1", "name": "With icon", "stream_icon": "http://i"}),
            json!({"stream_id": 40, "category_id": "L1", "name": "Null icon", "stream_icon": "null"}),
            json!({"category_id": "L1", "name": "No id"}),
            json!({"stream_id": 50, "name": "No category"}),
        ];
        let items = parse_stream_list(&raw, Kind::Live);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].id, 10);
        assert_eq!(items[1].id, 20);
        assert_eq!(items[2].icon.as_deref(), Some("http://i"));
        assert_eq!(items[3].icon, None);
        assert!(items.iter().all(|i| i.kind == Kind::Live));
    }

    // ---- parse_series_list ----

    #[test]
    fn parse_series_extracts_inline_metadata() {
        let raw = vec![json!({
            "series_id": 7001,
            "category_id": "S1",
            "name": "A Series",
            "cover": "http://cover",
            "plot": "The plot.",
            "genre": "Drama",
            "rating": "8.5",
            "releaseDate": "2020-01-01",
            "youtube_trailer": "yt"
        })];
        let (items, meta) = parse_series_list(&raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rating.as_deref(), Some("8.5"));
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].0, 7001);
        assert_eq!(meta[0].1.plot.as_deref(), Some("The plot."));
        assert_eq!(meta[0].1.genre.as_deref(), Some("Drama"));
        assert_eq!(meta[0].1.trailer.as_deref(), Some("yt"));
    }

    #[test]
    fn parse_series_treats_zero_null_and_empty_ratings_as_absent() {
        let raw = vec![
            json!({"series_id": 1, "category_id": "S", "name": "a", "rating": "0"}),
            json!({"series_id": 2, "category_id": "S", "name": "b", "rating": "null"}),
            json!({"series_id": 3, "category_id": "S", "name": "c", "rating": ""}),
            json!({"series_id": 4, "category_id": "S", "name": "d", "rating": "7"}),
        ];
        let (items, _) = parse_series_list(&raw);
        assert_eq!(items[0].rating, None);
        assert_eq!(items[1].rating, None);
        assert_eq!(items[2].rating, None);
        assert_eq!(items[3].rating.as_deref(), Some("7"));
    }

    #[test]
    fn parse_series_meta_accepts_string_ids() {
        // meta extraction accepts string ids; the item filter (as_i64)
        // skips them, keeping the two paths' lenience documented.
        let raw = vec![json!({"series_id": "8000", "category_id": "S", "name": "x"})];
        let (items, meta) = parse_series_list(&raw);
        assert_eq!(items.len(), 0);
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].0, 8000);
    }

    // ---- assemble_catalog ----

    fn live_item(id: i64, cat: &str) -> Item {
        Item {
            kind: Kind::Live,
            id,
            category_id: cat.into(),
            name: format!("item {id}"),
            icon: None,
            rating: None,
            url: None,
        }
    }

    #[test]
    fn assemble_synthesizes_orphan_categories_and_counts_them() {
        let sections = vec![(
            Kind::Live,
            vec![Category {
                kind: Kind::Live,
                id: "L1".into(),
                name: "Live".into(),
                visible: true,
            }],
            vec![live_item(1, "L1"), live_item(2, "GHOST")],
            Vec::new(),
        )];
        let (catalog, orphans) = assemble_catalog(sections);
        assert_eq!(orphans, 1);
        assert_eq!(catalog.categories.len(), 2);
        assert!(catalog
            .categories
            .iter()
            .any(|c| c.id == "GHOST" && c.name == "Category GHOST"));
    }

    #[test]
    fn assemble_merges_series_meta() {
        let sections = vec![(
            Kind::Series,
            Vec::new(),
            Vec::new(),
            vec![(
                42i64,
                SeriesMeta {
                    plot: Some("p".into()),
                    ..Default::default()
                },
            )],
        )];
        let (catalog, orphans) = assemble_catalog(sections);
        assert_eq!(orphans, 0);
        assert_eq!(catalog.series_meta.len(), 1);
        assert_eq!(catalog.series_meta[&42].plot.as_deref(), Some("p"));
    }

    // ---- build_m3u_catalog ----

    fn entry(name: &str, group: &str, url: &str) -> xtream::M3UEntry {
        xtream::M3UEntry {
            name: name.into(),
            group: group.into(),
            url: url.into(),
        }
    }

    #[test]
    fn m3u_groups_by_group_title_with_running_ids() {
        let entries = vec![
            entry("A", "Sports", "http://a"),
            entry("B", "News", "http://b"),
            entry("C", "Sports", "http://c"),
        ];
        let catalog = build_m3u_catalog(&entries);
        assert_eq!(catalog.categories.len(), 2);
        assert_eq!(catalog.categories[0].name, "Sports");
        assert_eq!(catalog.categories[1].name, "News");
        // Running ids across groups (SQLite PK is kind+item_id). Items
        // are appended GROUP BY GROUP: Sports first (A, C), then News
        // (B) — so the second item is C, back in the first category.
        let ids: Vec<i64> = catalog.items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        assert_eq!(catalog.items[0].category_id, "m3u-0");
        assert_eq!(catalog.items[1].category_id, "m3u-0");
        assert_eq!(catalog.items[2].category_id, "m3u-1");
        assert_eq!(catalog.items[2].name, "B");
        // Per-entry URLs land on the items.
        assert_eq!(catalog.items[0].url.as_deref(), Some("http://a"));
    }

    #[test]
    fn m3u_empty_groups_fold_into_one_bucket() {
        let entries = vec![entry("X", "", "http://x"), entry("Y", "  ", "http://y")];
        let catalog = build_m3u_catalog(&entries);
        assert_eq!(catalog.categories.len(), 1);
        assert_eq!(catalog.categories[0].name, DEFAULT_GROUP);
        assert_eq!(catalog.items.len(), 2);
    }
}
