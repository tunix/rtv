//! Sample source: exposes an in-memory catalog from credential-free JSON
//! dumps. In `--mock` mode the committed `mock/` dataset is read (works
//! everywhere, no network); in real mode the gitignored `samples/` dumps
//! are used when present. This exercises the UI before the network/SQLite
//! backend lands.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Live,
    Series,
    Vod,
}

impl Kind {
    /// Sidebar display order: Live TV, Movies, Series.
    pub const SIDEBAR: [Kind; 3] = [Kind::Live, Kind::Vod, Kind::Series];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Live => "Live TV",
            Kind::Series => "Series",
            Kind::Vod => "Movies",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Live => "video-display-symbolic",
            Kind::Series => "folder-videos-symbolic",
            Kind::Vod => "video-x-generic-symbolic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Category {
    pub kind: Kind,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub id: i64,
    pub category_id: String,
    pub name: String,
    pub icon: Option<String>,
    /// Textual rating when the dump carries one ("0" is treated as none).
    pub rating: Option<String>,
}

/// Offline series metadata from `series.json`: enough for full series
/// detail pages without any network round-trip. Fields the dump omits
/// stay `None`.
#[derive(Debug, Clone, Default)]
pub struct SeriesMeta {
    pub plot: Option<String>,
    pub genre: Option<String>,
    pub cast: Option<String>,
    pub director: Option<String>,
    pub rating: Option<String>,
    pub release_date: Option<String>,
    /// Stored for the upcoming player rounds (backdrop art, trailer).
    #[allow(dead_code)]
    pub backdrop: Option<String>,
    #[allow(dead_code)]
    pub trailer: Option<String>,
    pub cover: Option<String>,
    pub episode_run_time: Option<String>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// True when the samples/ dumps were found and parsed.
    pub found: bool,
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
    /// Series id -> offline metadata (series detail pages).
    pub series_meta: HashMap<i64, SeriesMeta>,
}

#[derive(Deserialize)]
struct XCategory {
    #[serde(rename = "category_id")]
    id: String,
    #[serde(rename = "category_name")]
    name: String,
}

#[derive(Deserialize)]
struct XLiveStream {
    #[serde(rename = "stream_id")]
    id: i64,
    name: String,
    #[serde(rename = "category_id")]
    category_id: String,
    #[serde(rename = "stream_icon", default)]
    icon: String,
}

#[derive(Deserialize)]
struct XSeries {
    #[serde(rename = "series_id")]
    id: i64,
    name: String,
    #[serde(rename = "category_id")]
    category_id: String,
    #[serde(rename = "cover", default)]
    icon: String,
    #[serde(default)]
    plot: String,
    #[serde(default)]
    cast: String,
    #[serde(default)]
    director: String,
    #[serde(default)]
    genre: String,
    #[serde(default)]
    rating: String,
    #[serde(rename = "releaseDate", default)]
    release_date: String,
    #[serde(default)]
    youtube_trailer: String,
    #[serde(rename = "episode_run_time", default)]
    episode_run_time: String,
    #[serde(rename = "backdrop_path", default)]
    backdrop: Vec<String>,
}

#[derive(Deserialize)]
struct XVodStream {
    #[serde(rename = "stream_id")]
    id: i64,
    name: String,
    #[serde(rename = "category_id")]
    category_id: String,
    #[serde(rename = "stream_icon", default)]
    icon: String,
    /// Ratings arrive as string, number or null depending on the panel.
    #[serde(default)]
    rating: Option<Value>,
}

fn read_json(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// Panels emit "" or the literal string "null" for missing metadata.
fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty() && value != "null").then(|| value.to_string())
}

/// Rating from a lenient JSON value; "0" and null mean "no rating".
fn rating_text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => non_empty(&s).filter(|s| s != "0"),
        Some(Value::Number(n)) => {
            let s = n.to_string();
            (s != "0").then_some(s)
        }
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

/// Mock mode reads the committed `mock/` dataset, real mode the optional
/// (gitignored) `samples/` dumps.
fn dir() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"))).join(crate::mock::data_dir())
}

impl Catalog {
    pub fn load() -> Catalog {
        let dir = dir();
        let mut catalog = Catalog::default();

        let live_categories: Vec<XCategory> = read_json(&dir.join("live_categories.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();
        let live_streams: Vec<XLiveStream> = read_json(&dir.join("live_streams.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();
        let series_categories: Vec<XCategory> = read_json(&dir.join("series_categories.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();
        let series: Vec<XSeries> = read_json(&dir.join("series.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();
        let vod_categories: Vec<XCategory> = read_json(&dir.join("vod_categories.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();
        let vod_streams: Vec<XVodStream> = read_json(&dir.join("vod_streams.json"))
            .map(|t| parse(&t))
            .unwrap_or_default();

        catalog.found =
            !(live_categories.is_empty() && series.is_empty() && vod_streams.is_empty());
        if !catalog.found {
            return catalog;
        }

        for (kind, list) in [
            (Kind::Live, &live_categories),
            (Kind::Series, &series_categories),
            (Kind::Vod, &vod_categories),
        ] {
            for c in list {
                catalog.categories.push(Category {
                    kind,
                    id: c.id.clone(),
                    name: c.name.clone(),
                });
            }
        }

        for s in &live_streams {
            catalog.items.push(Item {
                kind: Kind::Live,
                id: s.id,
                category_id: s.category_id.clone(),
                name: s.name.clone(),
                icon: (!s.icon.is_empty()).then_some(s.icon.clone()),
                rating: None,
            });
        }
        for s in &series {
            catalog.items.push(Item {
                kind: Kind::Series,
                id: s.id,
                category_id: s.category_id.clone(),
                name: s.name.clone(),
                icon: (!s.icon.is_empty()).then_some(s.icon.clone()),
                rating: non_empty(&s.rating),
            });
            catalog.series_meta.insert(
                s.id,
                SeriesMeta {
                    plot: non_empty(&s.plot),
                    genre: non_empty(&s.genre),
                    cast: non_empty(&s.cast),
                    director: non_empty(&s.director),
                    rating: non_empty(&s.rating),
                    release_date: non_empty(&s.release_date),
                    backdrop: s.backdrop.iter().find(|b| !b.is_empty()).cloned(),
                    trailer: non_empty(&s.youtube_trailer),
                    cover: (!s.icon.is_empty()).then_some(s.icon.clone()),
                    episode_run_time: non_empty(&s.episode_run_time),
                },
            );
        }
        for s in &vod_streams {
            catalog.items.push(Item {
                kind: Kind::Vod,
                id: s.id,
                category_id: s.category_id.clone(),
                name: s.name.clone(),
                icon: (!s.icon.is_empty()).then_some(s.icon.clone()),
                rating: rating_text(s.rating.clone()),
            });
        }

        // Categories seen in items but missing from the category files.
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
                });
            }
        }

        catalog.items.sort_by_key(|a| a.name.to_lowercase());

        catalog
    }

    pub fn item_count(&self, kind: Kind) -> usize {
        self.items.iter().filter(|i| i.kind == kind).count()
    }
}
