//! The in-app catalog: what the sidebar and grids render from. Post-mock,
//! this is fed by ingested sources (`ingest.rs`); the window holds the
//! current snapshot and rebuilds its sidebar when a new one lands.
//!
//! Kinds and item/category shapes stay identical to the former
//! `sample.rs` so window.rs routing keeps working unchanged.

use std::collections::HashMap;

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
            Kind::Live => "tv-symbolic",
            Kind::Series => "folder-videos-symbolic",
            Kind::Vod => "video-x-generic-symbolic",
        }
    }
}

/// M3U has no per-kind sections — everything lands in one bucket.
pub const DEFAULT_GROUP: &str = "All content";

#[derive(Debug, Clone)]
pub struct Category {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    /// Hidden categories stay in the source but are skipped when
    /// rendering content grids (the settings toggles write this).
    pub visible: bool,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub id: i64,
    pub category_id: String,
    pub name: String,
    pub icon: Option<String>,
    /// Textual rating when the panel carries one ("0" = none).
    pub rating: Option<String>,
    /// Playback URL for M3U entries (each entry carries its own);
    /// Xtream items leave it None — their URLs are composed from the
    /// source's credentials. NEVER logged.
    pub url: Option<String>,
}

/// Offline series metadata — the series detail page renders it on top
/// before/without the on-demand info fetch.
#[derive(Debug, Clone, Default)]
pub struct SeriesMeta {
    pub plot: Option<String>,
    pub genre: Option<String>,
    pub cast: Option<String>,
    pub director: Option<String>,
    pub rating: Option<String>,
    pub release_date: Option<String>,
    #[allow(dead_code)]
    pub backdrop: Option<String>,
    #[allow(dead_code)]
    pub trailer: Option<String>,
    pub cover: Option<String>,
    pub episode_run_time: Option<String>,
}

/// One catalog snapshot. Immutable once built; replacing the snapshot is
/// the update mechanism (the window rebuilds from the new one). Cloning
/// is cheap-ish (used for visibility patches) — vector copies, not
/// network data.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
    /// Series id -> offline metadata for the detail page.
    pub series_meta: HashMap<i64, SeriesMeta>,
}

impl Catalog {
    pub fn item_count(&self, kind: Kind) -> usize {
        self.items.iter().filter(|i| i.kind == kind).count()
    }

    /// Whether anything at all is browseable (drives empty states).
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.items.is_empty()
    }

    /// Indices of items in one category, honoring the category's visible
    /// flag and (optionally) a search filter — the grid feed.
    pub fn indices_for(&self, kind: Kind, category_id: &str, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        let visible = self
            .categories
            .iter()
            .any(|c| c.kind == kind && c.id == category_id && c.visible);
        if !visible {
            return Vec::new();
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == kind
                    && it.category_id == category_id
                    && (q.is_empty() || it.name.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn item(&self, kind: Kind, id: i64) -> Option<&Item> {
        self.items.iter().find(|it| it.kind == kind && it.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(kind: Kind, id: &str, name: &str, visible: bool) -> Category {
        Category {
            kind,
            id: id.into(),
            name: name.into(),
            visible,
        }
    }

    fn item(kind: Kind, id: i64, cat: &str) -> Item {
        Item {
            kind,
            id,
            category_id: cat.into(),
            name: format!("item {id}"),
            icon: None,
            rating: None,
            url: None,
        }
    }

    fn sample() -> Catalog {
        Catalog {
            categories: vec![
                cat(Kind::Live, "L1", "Live", true),
                cat(Kind::Live, "L2", "Hidden Live", false),
                cat(Kind::Vod, "V1", "Movies", true),
            ],
            items: vec![
                item(Kind::Live, 1, "L1"),
                item(Kind::Live, 2, "L1"),
                item(Kind::Live, 3, "L2"),
                item(Kind::Vod, 10, "V1"),
            ],
            series_meta: HashMap::new(),
        }
    }

    #[test]
    fn item_count_filters_by_kind() {
        let c = sample();
        assert_eq!(c.item_count(Kind::Live), 3);
        assert_eq!(c.item_count(Kind::Vod), 1);
        assert_eq!(c.item_count(Kind::Series), 0);
    }

    #[test]
    fn is_empty_reflects_both_collections() {
        assert!(Catalog::default().is_empty());
        let mut c = Catalog::default();
        c.categories.push(cat(Kind::Live, "x", "x", true));
        assert!(!c.is_empty());
        c.categories.clear();
        c.items.push(item(Kind::Live, 1, "x"));
        assert!(!c.is_empty());
    }

    #[test]
    fn indices_for_honors_visibility() {
        let c = sample();
        assert_eq!(c.indices_for(Kind::Live, "L1", ""), vec![0, 1]);
        // Hidden category yields nothing even though items exist.
        assert_eq!(c.indices_for(Kind::Live, "L2", ""), Vec::<usize>::new());
        // Unknown category yields nothing.
        assert_eq!(c.indices_for(Kind::Live, "GHOST", ""), Vec::<usize>::new());
    }

    #[test]
    fn indices_for_applies_query_case_insensitively() {
        let mut c = sample();
        c.items[0].name = "Fox News HD".into();
        c.items[1].name = "CNN".into();
        assert_eq!(c.indices_for(Kind::Live, "L1", "fox"), vec![0]);
        assert_eq!(c.indices_for(Kind::Live, "L1", "FOX NEWS"), vec![0]);
        assert_eq!(c.indices_for(Kind::Live, "L1", "news hd"), vec![0]);
        assert_eq!(c.indices_for(Kind::Live, "L1", "cnn"), vec![1]);
        assert_eq!(c.indices_for(Kind::Live, "L1", "zzz"), Vec::<usize>::new());
    }

    #[test]
    fn item_lookup_is_kind_scoped() {
        let c = sample();
        assert_eq!(c.item(Kind::Live, 1).map(|i| i.id), Some(1));
        // Same numeric id in another kind is NOT returned.
        assert!(c.item(Kind::Vod, 1).is_none());
        assert_eq!(c.item(Kind::Vod, 10).map(|i| i.id), Some(10));
    }
}
