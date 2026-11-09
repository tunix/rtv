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

/// Parse a panel rating: "6.5", "6,5" (comma decimal), "6.5/10" —
/// 0.0..=10.0 or nothing.
fn parse_rating(text: &str) -> Option<f32> {
    let text = text.trim();
    let text = text.strip_suffix("/10").unwrap_or(text);
    let value: f32 = text.replace(",", ".").parse().ok()?;
    (0.0..=10.0).contains(&value).then_some(value)
}

fn is_year(text: &str) -> bool {
    text.len() == 4
        && text.chars().all(|c| c.is_ascii_digit())
        && text
            .parse::<u32>()
            .is_ok_and(|y| (1900..=2099).contains(&y))
}

/// Panels decorate the item name in several shapes: "Ad | 2025 | 6.5",
/// "Ad | 2023 | 7.0 | DSNP | 4K" (year-anchored pipes with trailing
/// junk tags), "Captain Marvel (2019)" (trailing parenthesized year).
/// Peel what matches; anything that does not fit stays verbatim —
/// live channel names, "Movie | 6.5" (a rating without a year is
/// never peeled), "Movie | 26" and so on.
pub fn split_title(raw: &str) -> (String, Option<String>, Option<String>) {
    let raw = raw.trim();
    let parts: Vec<&str> = raw.split('|').collect();
    if parts.len() >= 2 {
        // Year-anchored: the first pipe segment (after the name) that is
        // a bare 4-digit year ends the display name; a segment parsing
        // as a rating right after it is the score; everything beyond is
        // decoration ("DSNP", "4K", …) and is dropped.
        if let Some(year_idx) = (1..parts.len()).find(|&i| is_year(parts[i].trim())) {
            let name = parts[..year_idx].join("|").trim().to_string();
            if !name.is_empty() {
                let year = parts[year_idx].trim().to_string();
                let rating = parts
                    .get(year_idx + 1)
                    .and_then(|s| parse_rating(s.trim()))
                    .map(|_| parts[year_idx + 1].trim().to_string());
                return (name, Some(year), rating);
            }
        }
    }
    // Trailing parenthesized year: "Captain Marvel (2019)".
    if raw.ends_with(')') {
        if let Some(open) = raw.rfind('(') {
            let inner = raw[open + 1..raw.len() - 1].trim();
            if is_year(inner) && open > 0 {
                let name = raw[..open]
                    .trim()
                    .trim_end_matches(['-', '–', ':'])
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    return (name, Some(inner.to_string()), None);
                }
            }
        }
    }
    (raw.to_string(), None, None)
}

/// Pick the display title from the panel's name/title fields. Some
/// panels keep a clean `name` with a decorated `title`, others the
/// reverse: prefer the clean candidate for the display name and take
/// the year/rating from the decorated one.
pub fn best_title(
    name_raw: &str,
    title_raw: Option<&str>,
) -> (String, Option<String>, Option<String>) {
    let (n, ny, nr) = split_title(name_raw);
    let title_differs = title_raw
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != name_raw.trim());
    let Some(t) = title_differs else {
        return (n, ny, nr);
    };
    let (tn, ty, tr) = split_title(t);
    let name_clean = ny.is_none() && nr.is_none();
    let title_clean = ty.is_none() && tr.is_none();
    match (name_clean, title_clean) {
        (true, false) => (n, ty, tr),
        (false, true) => (tn, ny, nr),
        _ => (n, ny.or(ty), nr.or(tr)),
    }
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
/// the result is stable. The genre of a movie comes from the harvested
/// vod-info map (`vod_genres`), a series' from its inline metadata.
pub struct GridView<'a> {
    pub items: &'a [Item],
    pub series_meta: &'a HashMap<i64, SeriesMeta>,
    pub vod_genres: &'a HashMap<i64, String>,
    pub min_rating: Option<f32>,
    pub genre: Option<String>,
    pub sort: Sort,
}

impl GridView<'_> {
    /// The item's genre string from whichever source carries it.
    fn genre_of(&self, it: &Item) -> Option<String> {
        if let Some(meta) = self.series_meta.get(&it.id) {
            if let Some(g) = &meta.genre {
                return Some(g.clone());
            }
        }
        self.vod_genres.get(&it.id).cloned()
    }

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
                self.items.get(i).is_some_and(|it| {
                    self.genre_of(it).is_some_and(|g| {
                        g.split(',')
                            .any(|part| part.trim().to_lowercase() == genre.as_str())
                    })
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
        // Decorated pipes: tags after the rating are dropped.
        assert_eq!(
            split_title("Bursa Bülbülü | 2023 | 7.0 | DSNP | 4K"),
            (
                "Bursa Bülbülü".into(),
                Some("2023".into()),
                Some("7.0".into())
            )
        );
        // Trailing parenthesized year (no pipes at all).
        assert_eq!(
            split_title("Captain Marvel (2019)"),
            ("Captain Marvel".into(), Some("2019".into()), None)
        );
        assert_eq!(
            split_title("Star Wars: Bölüm XIII - Son Jedi (2017)"),
            (
                "Star Wars: Bölüm XIII - Son Jedi".into(),
                Some("2017".into()),
                None
            )
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
    fn best_title_prefers_the_clean_candidate() {
        // name decorated, title clean → title wins the display name.
        assert_eq!(
            best_title("Son Damla | 2025 | 6.5", Some("Son Damla")),
            ("Son Damla".into(), Some("2025".into()), Some("6.5".into()))
        );
        // name clean, title decorated → name wins, pieces from the title.
        assert_eq!(
            best_title("Son Damla", Some("Son Damla | 2025 | 6.5")),
            ("Son Damla".into(), Some("2025".into()), Some("6.5".into()))
        );
        // Both decorated → name's parse wins.
        assert_eq!(
            best_title("A | 2020 | 5.0", Some("A | 2021")),
            ("A".into(), Some("2020".into()), Some("5.0".into()))
        );
        // Missing/identical title → name only.
        assert_eq!(
            best_title("Bursa Bülbülü | 2023 | 7.0 | DSNP", None),
            (
                "Bursa Bülbülü".into(),
                Some("2023".into()),
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
        // Comma decimals and "x/10" suffixes parse too.
        it.rating = Some("6,3".into());
        assert_eq!(it.rating_value(), Some(6.3));
        it.rating = Some("8.1/10".into());
        assert_eq!(it.rating_value(), Some(8.1));
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
        let vod_genres: HashMap<i64, String> = HashMap::new();
        let view = |min_rating, genre, sort| GridView {
            items: &c.items,
            series_meta: &c.series_meta,
            vod_genres: &vod_genres,
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
            vod_genres: &vod_genres,
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

    #[test]
    fn grid_view_genre_reaches_harvested_movies() {
        let c = sample();
        let mut vod_genres: HashMap<i64, String> = HashMap::new();
        // Movie id 10 harvested as "Gerilim, Dram".
        vod_genres.insert(10, "Gerilim, Dram".into());
        let view = |genre: Option<String>| GridView {
            items: &c.items,
            series_meta: &c.series_meta,
            vod_genres: &vod_genres,
            min_rating: None,
            genre,
            sort: Sort::Panel,
        };
        assert_eq!(view(Some("Dram".into())).apply(vec![3usize]), vec![3]);
        assert_eq!(view(Some("Gerilim".into())).apply(vec![3usize]), vec![3]);
        assert_eq!(
            view(Some("Comedy".into())).apply(vec![3usize]),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn grid_view_series_meta_beats_harvested_movie_genre() {
        let mut c = sample();
        c.series_meta.insert(
            10,
            SeriesMeta {
                genre: Some("Comedy".into()),
                ..Default::default()
            },
        );
        let mut vod_genres: HashMap<i64, String> = HashMap::new();
        vod_genres.insert(10, "Dram".into());
        let view = |genre: Option<String>| GridView {
            items: &c.items,
            series_meta: &c.series_meta,
            vod_genres: &vod_genres,
            min_rating: None,
            genre,
            sort: Sort::Panel,
        };
        assert_eq!(view(Some("Comedy".into())).apply(vec![3usize]), vec![3]);
        assert_eq!(
            view(Some("Dram".into())).apply(vec![3usize]),
            Vec::<usize>::new()
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
