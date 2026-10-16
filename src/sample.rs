//! Sample source: reads the credential-free dumps in `samples/` (gitignored)
//! and exposes them as an in-memory catalog. This exercises the UI at real
//! scale (~30k items) before the network/SQLite backend lands.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Live,
    Series,
    Vod,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Live, Kind::Series, Kind::Vod];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Live => "Live",
            Kind::Series => "Series",
            Kind::Vod => "Movies",
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
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// True when the samples/ dumps were found and parsed.
    pub found: bool,
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
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
}

fn read_json(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

fn dir() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"))).join("samples")
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
            });
        }
        for s in &series {
            catalog.items.push(Item {
                kind: Kind::Series,
                id: s.id,
                category_id: s.category_id.clone(),
                name: s.name.clone(),
                icon: (!s.icon.is_empty()).then_some(s.icon.clone()),
            });
        }
        for s in &vod_streams {
            catalog.items.push(Item {
                kind: Kind::Vod,
                id: s.id,
                category_id: s.category_id.clone(),
                name: s.name.clone(),
                icon: (!s.icon.is_empty()).then_some(s.icon.clone()),
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
