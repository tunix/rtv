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
    /// Release year when the panel's name carries one (see `split_title`).
    pub year: Option<String>,
    pub icon: Option<String>,
    /// Textual rating when the panel carries one ("0" = none).
    pub rating: Option<String>,
    /// Playback URL for M3U entries (each entry carries its own);
    /// Xtream items leave it None — their URLs are composed from the
    /// source's credentials. NEVER logged.
    pub url: Option<String>,
}

impl Item {
    /// Listing label: name plus the year in parentheses — the panel's
    /// score never renders here.
    pub fn display_name(&self) -> String {
        match &self.year {
            Some(y) if !y.trim().is_empty() => format!("{} ({y})", self.name),
            _ => self.name.clone(),
        }
    }

    /// The rating as a number, for sorting and the min-rating filter.
    pub fn rating_value(&self) -> Option<f32> {
        self.rating.as_deref().and_then(parse_rating)
    }
}

/// Parse a panel rating like "6.5" (0.0..=10.0, one decimal).
fn parse_rating(text: &str) -> Option<f32> {
    let value: f32 = text.trim().parse().ok()?;
    (0.0..=10.0).contains(&value).then_some(value)
}

fn is_year(text: &str) -> bool {
    text.len() == 4 && text.chars().all(|c| c.is_ascii_digit())
}

/// Panels embed the year and the score in the item name
/// ("Son Damla | 2025 | 6.5"). Split a trailing
/// "| <year> [| <rating>]" segment off: the pieces feed the year-aware
/// listing label, the rating chip and the clean-title lookups. Only a
/// trailing segment in exactly that shape is parsed — anything else
/// stays verbatim (live channel names, "Movie (2026)" etc.).
pub fn split_title(raw: &str) -> (String, Option<String>, Option<String>) {
    let raw = raw.trim();
    let parts: Vec<&str> = raw.split('|').collect();
    let mut name_parts: &[&str] = &parts;
    let mut year = None;
    let mut rating = None;
    if parts.len() >= 2 {
        let last = parts[parts.len() - 1].trim();
        let before = parts[parts.len() - 2].trim();
        if parts.len() >= 3 && is_year(before) && parse_rating(last).is_some() {
            rating = Some(last.to_string());
            year = Some(before.to_string());
            name_parts = &parts[..parts.len() - 2];
        } else if is_year(last) {
            year = Some(last.to_string());
            name_parts = &parts[..parts.len() - 1];
        }
    }
    let name = name_parts.join("|").trim().to_string();
    (name, year, rating)
}

/// Grid view ordering for the Series/Movies toolbars. `Panel` is the
/// catalog (source) order the sidebar's counts and search are built on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Panel,
    NameAZ,
    YearNewest,
    RatingHighest,
}

/// One grid "view": a min-rating filter, a genre filter and a sort
/// order. Filters drop items (items without a rating never pass a
/// min-rating filter); sorting keeps panel order as the tiebreaker so
/// the result is stable.
pub struct GridView<'a> {
    pub items: &'a [Item],
    pub series_meta: &'a HashMap<i64, SeriesMeta>,
    pub min_rating: Option<f32>,
    pub genre: Option<String>,
    pub sort: Sort,
}

impl GridView<'_> {
    /// Sort + filter a list of catalog indices; the panel order is the
    /// fallback everywhere (missing years/ratings sort last, ties keep
    /// the source order).
    pub fn apply(&self, indices: Vec<usize>) -> Vec<usize> {
        let genre = self.genre.as_ref().map(|g| g.to_lowercase());
        let mut out: Vec<usize> = indices
            .into_iter()
            .filter(|&i| match (self.min_rating, self.items.get(i)) {
                (Some(min), Some(it)) => it.rating_value().is_some_and(|r| r >= min),
                (Some(_), None) => false,
                _ => true,
            })
            .filter(|&i| {
                let Some(genre) = &genre else {
                    return true;
                };
                self.items
                    .get(i)
                    .and_then(|it| self.series_meta.get(&it.id))
                    .and_then(|m| m.genre.as_deref())
                    .is_some_and(|g| {
                        g.split(',')
                            .any(|part| part.trim().to_lowercase() == genre.as_str())
                    })
            })
            .collect();
        match self.sort {
            Sort::Panel => {}
            Sort::NameAZ => {
                out.sort_by(|a, b| {
                    let (x, y) = (self.items.get(*a), self.items.get(*b));
                    x.map(|i| i.name.to_lowercase())
                        .cmp(&y.map(|i| i.name.to_lowercase()))
                        .then(a.cmp(b))
                });
            }
            Sort::YearNewest => {
                out.sort_by(|a, b| {
                    let year = |i: usize| {
                        self.items
                            .get(i)
                            .and_then(|it| it.year.as_deref())
                            .and_then(|y| y.parse::<i32>().ok())
                    };
                    year(*b).cmp(&year(*a)).then(a.cmp(b))
                });
            }
            Sort::RatingHighest => {
                out.sort_by(|a, b| {
                    let rating = |i: usize| self.items.get(i).and_then(Item::rating_value);
                    // Highest first; unrated titles sink below rated ones.
                    match (rating(*b), rating(*a)) {
                        (Some(x), Some(y)) => x.total_cmp(&y).then(a.cmp(b)),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.cmp(b),
                    }
                });
            }
        }
        out
    }
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
            year: None,
            icon: None,
            rating: None,
            url: None,
        }
    }

    #[test]
    fn split_title_peels_year_and_rating() {
        assert_eq!(
            split_title("Son Damla | 2025 | 6.5"),
            ("Son Damla".into(), Some("2025".into()), Some("6.5".into()))
        );
        // Year without a rating segment.
        assert_eq!(
            split_title("GTA VI | 2026"),
            ("GTA VI".into(), Some("2026".into()), None)
        );
        // No year/rating segment stays verbatim.
        assert_eq!(split_title("Fox News"), ("Fox News".into(), None, None));
        assert_eq!(split_title("beIN | HD"), ("beIN | HD".into(), None, None));
        // A rating-only tail is never peeled without a year.
        assert_eq!(
            split_title("Movie | 6.5"),
            ("Movie | 6.5".into(), None, None)
        );
        // Year-only sanity: not a year, not peeled.
        assert_eq!(split_title("Movie | 26"), ("Movie | 26".into(), None, None));
        // Whitespace and stray pipes inside the name survive.
        assert_eq!(
            split_title("  Movie: Part | 2 | 2024 | 7.0 "),
            (
                "Movie: Part | 2".into(),
                Some("2024".into()),
                Some("7.0".into())
            )
        );
    }

    #[test]
    fn display_name_carries_year_without_rating() {
        let mut it = item(Kind::Vod, 1, "V");
        it.name = "Son Damla".into();
        it.year = Some("2025".into());
        it.rating = Some("6.5".into());
        assert_eq!(it.display_name(), "Son Damla (2025)");
        it.year = None;
        assert_eq!(it.display_name(), "Son Damla");
    }

    #[test]
    fn rating_value_rejects_out_of_range_and_garbage() {
        let mut it = item(Kind::Vod, 1, "V");
        it.rating = Some("7.4".into());
        assert_eq!(it.rating_value(), Some(7.4));
        it.rating = Some("11".into());
        assert_eq!(it.rating_value(), None);
        it.rating = Some("none".into());
        assert_eq!(it.rating_value(), None);
        it.rating = None;
        assert_eq!(it.rating_value(), None);
    }

    #[test]
    fn grid_view_sorts_and_filters() {
        let mut c = sample();
        // Movie 10: no year/rating; add two rated, one rated movie.
        c.items[3].name = "C Movie".into();
        c.items[3].year = Some("2020".into());
        c.items[3].rating = Some("6.0".into());
        c.items.push(item(Kind::Vod, 11, "V1"));
        c.items[4].name = "A Movie".into();
        c.items[4].year = Some("2024".into());
        c.items[4].rating = Some("8.0".into());
        c.items.push(item(Kind::Vod, 12, "V1"));
        c.items[5].name = "B Movie".into();
        c.items[5].year = Some("2022".into());
        c.items[5].rating = Some("5.0".into());

        let base = vec![3usize, 4, 5];
        let view = |min_rating, genre, sort| GridView {
            items: &c.items,
            series_meta: &c.series_meta,
            min_rating,
            genre,
            sort,
        };

        assert_eq!(
            view(None, None, Sort::NameAZ).apply(base.clone()),
            vec![4, 5, 3]
        );
        assert_eq!(
            view(None, None, Sort::YearNewest).apply(base.clone()),
            vec![4, 5, 3]
        );
        assert_eq!(
            view(None, None, Sort::RatingHighest).apply(base.clone()),
            vec![4, 3, 5]
        );
        assert_eq!(
            view(None, None, Sort::Panel).apply(base.clone()),
            vec![3, 4, 5]
        );
        // Min-rating drops the unrated item too.
        assert_eq!(
            view(Some(5.5), None, Sort::Panel).apply(base.clone()),
            vec![3, 4]
        );
        assert_eq!(
            view(Some(7.0), None, Sort::Panel).apply(base.clone()),
            vec![4]
        );
        // Genre filter: only items with a matching genre pass.
        c.series_meta.insert(
            11,
            SeriesMeta {
                genre: Some("Drama, Thriller".into()),
                ..Default::default()
            },
        );
        let with_genre = |min_rating: Option<f32>, genre: Option<String>, sort: Sort| GridView {
            items: &c.items,
            series_meta: &c.series_meta,
            min_rating,
            genre,
            sort,
        };
        assert_eq!(
            with_genre(None, Some("drama".into()), Sort::Panel).apply(base.clone()),
            vec![4]
        );
        assert_eq!(
            with_genre(None, Some("Comedy".into()), Sort::Panel).apply(base.clone()),
            Vec::<usize>::new()
        );
        // Combined filter + sort.
        assert_eq!(
            with_genre(Some(5.5), Some("Thriller".into()), Sort::NameAZ).apply(base),
            vec![4]
        );
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
