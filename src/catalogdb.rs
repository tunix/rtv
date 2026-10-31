//! Per-source catalog storage: one SQLite database per source under
//! `$XDG_DATA_HOME/rtv/sources/<id>/catalog.db` (the storage-decision
//! doc's ruling: catalogs are DATA, not config). Writes are atomic
//! (temp file + rename) and transactional; the whole snapshot is
//! rewritten per successful ingest while per-category visibility flags
//! get patched in place on toggle.

use std::path::Path;

use rusqlite::Connection;

use crate::catalog::{Catalog, Category, Item, Kind, SeriesMeta};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS categories (
    kind    INTEGER NOT NULL,
    id      TEXT NOT NULL,
    name    TEXT NOT NULL,
    visible INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (kind, id)
);
CREATE TABLE IF NOT EXISTS items (
    kind        INTEGER NOT NULL,
    item_id     INTEGER NOT NULL,
    category_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    icon        TEXT,
    rating      TEXT,
    url         TEXT,
    PRIMARY KEY (kind, item_id)
);
CREATE TABLE IF NOT EXISTS series_meta (
    series_id        INTEGER PRIMARY KEY,
    plot             TEXT,
    genre            TEXT,
    cast_list        TEXT,
    director         TEXT,
    rating           TEXT,
    release_date     TEXT,
    backdrop         TEXT,
    trailer          TEXT,
    cover            TEXT,
    episode_run_time TEXT
);
CREATE INDEX IF NOT EXISTS idx_items_cat ON items (kind, category_id);
";

fn kind_id(kind: Kind) -> i64 {
    match kind {
        Kind::Live => 0,
        Kind::Vod => 1,
        Kind::Series => 2,
    }
}

fn kind_of(value: i64) -> Kind {
    match value {
        1 => Kind::Vod,
        2 => Kind::Series,
        _ => Kind::Live,
    }
}

/// Replace the stored snapshot with `catalog`. Atomic: everything goes
/// into `<path>.tmp` inside one transaction, then a rename swaps it in.
pub fn write(path: &Path, catalog: &Catalog) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("db.tmp");
    let conn = Connection::open(&tmp)?;
    conn.execute_batch(SCHEMA)?;
    let tx = conn.unchecked_transaction()?;

    tx.execute("DELETE FROM categories", [])?;
    tx.execute("DELETE FROM items", [])?;
    tx.execute("DELETE FROM series_meta", [])?;

    let mut stmt =
        tx.prepare("INSERT INTO categories (kind, id, name, visible) VALUES (?1, ?2, ?3, ?4)")?;
    for cat in &catalog.categories {
        stmt.execute((kind_id(cat.kind), &cat.id, &cat.name, cat.visible as i64))?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO items (kind, item_id, category_id, name, icon, rating, url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for item in &catalog.items {
        stmt.execute((
            kind_id(item.kind),
            item.id,
            &item.category_id,
            &item.name,
            &item.icon,
            &item.rating,
            &item.url,
        ))?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO series_meta (series_id, plot, genre, cast_list, director, rating,
                                  release_date, backdrop, trailer, cover, episode_run_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for (id, meta) in &catalog.series_meta {
        stmt.execute((
            *id,
            &meta.plot,
            &meta.genre,
            &meta.cast,
            &meta.director,
            &meta.rating,
            &meta.release_date,
            &meta.backdrop,
            &meta.trailer,
            &meta.cover,
            &meta.episode_run_time,
        ))?;
    }
    drop(stmt);

    tx.commit()?;
    drop(conn);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read the whole snapshot back.
pub fn read(path: &Path) -> Result<Catalog, Box<dyn std::error::Error>> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    let mut catalog = Catalog::default();

    let mut stmt = conn.prepare("SELECT kind, id, name, visible FROM categories")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        catalog.categories.push(Category {
            kind: kind_of(row.get(0)?),
            id: row.get(1)?,
            name: row.get(2)?,
            visible: row.get::<_, i64>(3)? != 0,
        });
    }
    drop(rows);
    drop(stmt);

    let mut stmt =
        conn.prepare("SELECT kind, item_id, category_id, name, icon, rating, url FROM items")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        catalog.items.push(Item {
            kind: kind_of(row.get(0)?),
            id: row.get(1)?,
            category_id: row.get(2)?,
            name: row.get(3)?,
            icon: row.get(4)?,
            rating: row.get(5)?,
            url: row.get(6)?,
        });
    }
    drop(rows);
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT series_id, plot, genre, cast_list, director, rating, release_date,
                backdrop, trailer, cover, episode_run_time FROM series_meta",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        catalog.series_meta.insert(
            row.get(0)?,
            SeriesMeta {
                plot: row.get(1)?,
                genre: row.get(2)?,
                cast: row.get(3)?,
                director: row.get(4)?,
                rating: row.get(5)?,
                release_date: row.get(6)?,
                backdrop: row.get(7)?,
                trailer: row.get(8)?,
                cover: row.get(9)?,
                episode_run_time: row.get(10)?,
            },
        );
    }
    drop(rows);
    drop(stmt);

    Ok(catalog)
}

/// Patch per-category visibility flags in place (the settings toggles).
/// Only rows present in `categories` are touched — a stale toggle after
/// a re-ingest that dropped the category no-ops harmlessly.
pub fn update_visibility(
    path: &Path,
    categories: &[Category],
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    let tx = conn.unchecked_transaction()?;
    let mut stmt =
        tx.prepare_cached("UPDATE categories SET visible = ?1 WHERE kind = ?2 AND id = ?3")?;
    for cat in categories {
        stmt.execute((cat.visible as i64, kind_id(cat.kind), &cat.id))?;
    }
    drop(stmt);
    tx.commit()?;
    Ok(())
}

/// Whether a usable catalog file exists for a source.
pub fn exists(path: &Path) -> bool {
    path.exists()
}
