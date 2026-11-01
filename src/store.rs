//! Source persistence — the storage decision from the desktop-config
//! doc, implemented:
//!
//! - Source DEFINITIONS live in one human-readable file under
//!   `$XDG_CONFIG_HOME/rtv/sources.json` (editable, backupable). The
//!   file NEVER holds credentials: secrets (Xtream password, M3U URL —
//!   which may embed credentials) live in the Secret Service under
//!   `rtv/source/<id>`; the file stores only the reference.
//! - Per-source CATALOGS live in SQLite under
//!   `$XDG_DATA_HOME/rtv/sources/<id>/catalog.db` — written
//!   transactionally after a successful ingest, read whole at startup.
//! - Last active source rides GSettings (schema key `last-source`).
//!
//! All disk touches are logged with durations (the perf-record goal).

use gtk::gio::prelude::*;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::ui::sources::{Source, SourceKind, SourceState};

// ------------------------------------------------------------- paths

fn config_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join(".config")
        });
    base.join("rtv")
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join(".local/share")
        });
    base.join("rtv")
}

fn sources_file() -> std::path::PathBuf {
    config_dir().join("sources.json")
}

fn catalog_db(source_id: u64) -> std::path::PathBuf {
    data_dir()
        .join("sources")
        .join(source_id.to_string())
        .join("catalog.db")
}

fn secret_name(source_id: u64) -> String {
    format!("rtv/source/{source_id}")
}

// ------------------------------------------------- serialized shapes

/// On-disk source definition. Credentials are NEVER here — `secret_ref`
/// names the keyring entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct StoredSource {
    id: u64,
    name: String,
    kind: StoredKind,
    /// Per-section hidden flags ([Live, Vod, Series]).
    hidden: [bool; 3],
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum StoredKind {
    Xtream {
        server: String,
        username: String,
        secret_ref: String,
    },
    M3U {
        secret_ref: String,
        epg: String,
    },
}

// ------------------------------------------------------- config file

/// Load definitions. Missing file => empty (first run). Corrupt file =>
/// backed up, treated as empty, logged loudly — the keyring secrets are
/// untouched so a re-add heals the setup.
fn load_definitions() -> (Vec<StoredSource>, u64) {
    let path = sources_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return (Vec::new(), 1),
    };
    match serde_json::from_str::<Vec<StoredSource>>(&text) {
        Ok(list) => {
            let next = list.iter().map(|d| d.id).max().unwrap_or(0) + 1;
            (list, next)
        }
        Err(err) => {
            tracing::error!(
                target: "rtv::store",
                error = %err,
                "sources.json is corrupt — backed up, starting empty; keyring secrets untouched"
            );
            let backup = path.with_extension("json.corrupt");
            let _ = std::fs::rename(&path, &backup);
            (Vec::new(), 1)
        }
    }
}

/// Write definitions atomically (tmp + rename).
fn save_definitions(list: &[StoredSource]) -> Result<(), std::io::Error> {
    let started = std::time::Instant::now();
    let path = sources_file();
    std::fs::create_dir_all(path.parent().expect("config dir has a parent"))?;
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(list).expect("definitions serialize");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    tracing::info!(
        target: "rtv::store",
        ms = started.elapsed().as_millis(),
        sources = list.len(),
        "sources.json written"
    );
    Ok(())
}

// ------------------------------------------------------------ keyring

fn secret_store(name: &str, secret: &str) {
    match crate::keyring::store(name, secret) {
        Ok(()) => tracing::info!(target: "rtv::store", secret = name, "secret stored in keyring"),
        Err(err) => tracing::warn!(
            target: "rtv::store",
            secret = name,
            error = %err,
            "could NOT store secret in keyring — re-fetch will need re-entry"
        ),
    }
}

fn secret_lookup(name: &str) -> Option<String> {
    match crate::keyring::lookup(name) {
        Ok(Some(secret)) => Some(secret),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(target: "rtv::store", secret = name, error = %err, "keyring lookup failed");
            None
        }
    }
}

fn secret_delete(name: &str) {
    match crate::keyring::delete(name) {
        Ok(()) => {
            tracing::info!(target: "rtv::store", secret = name, "secret removed from keyring")
        }
        Err(err) => {
            tracing::warn!(target: "rtv::store", secret = name, error = %err, "secret removal failed")
        }
    }
}

// --------------------------------------------------------- public API

/// Load all sources: definitions from disk + secrets resolved into
/// memory. Sources whose secret is missing are skipped with a warning
/// (they simply do not appear; re-adding restores them).
pub fn load_sources() -> (Vec<Source>, u64) {
    let started = std::time::Instant::now();
    let (defs, next_id) = load_definitions();
    let mut sources = Vec::new();
    for def in defs {
        let id = def.id;
        let kind = match def.kind {
            StoredKind::Xtream {
                server,
                username,
                secret_ref,
            } => match secret_lookup(&secret_ref) {
                Some(password) => SourceKind::Xtream {
                    server,
                    username,
                    password,
                },
                None => {
                    tracing::warn!(target: "rtv::store", id, "keyring secret missing — source skipped");
                    continue;
                }
            },
            StoredKind::M3U { secret_ref, epg } => match secret_lookup(&secret_ref) {
                Some(url) => SourceKind::M3U { url, epg },
                None => {
                    tracing::warn!(target: "rtv::store", id, "playlist URL missing from keyring — source skipped");
                    continue;
                }
            },
        };
        sources.push(Source {
            id,
            name: def.name,
            kind,
            // Real state decided below: a stored catalog means Ready.
            state: SourceState::Indexing,
            failure: None,
            live_cats: 0,
            live_items: 0,
            vod_cats: 0,
            vod_items: 0,
            series_cats: 0,
            series_items: 0,
            section_hidden: def.hidden,
            snapshot: None,
        });
    }
    tracing::info!(
        target: "rtv::store",
        ms = started.elapsed().as_millis(),
        sources = sources.len(),
        "sources loaded from disk + keyring"
    );
    (sources, next_id)
}

/// Persist one source definition (after add/save). Extracts the secret
/// from the kind and (re)stores it in the keyring first.
pub fn save_source(source: &Source) {
    let name = secret_name(source.id);
    let kind = match &source.kind {
        SourceKind::Xtream {
            server,
            username,
            password,
        } => {
            secret_store(&name, password);
            StoredKind::Xtream {
                server: server.clone(),
                username: username.clone(),
                secret_ref: name,
            }
        }
        SourceKind::M3U { url, epg } => {
            secret_store(&name, url);
            StoredKind::M3U {
                secret_ref: name,
                epg: epg.clone(),
            }
        }
    };
    let (mut defs, _next) = load_definitions();
    let record = StoredSource {
        id: source.id,
        name: source.name.clone(),
        kind,
        hidden: source.section_hidden,
    };
    if let Some(slot) = defs.iter_mut().find(|d| d.id == source.id) {
        *slot = record;
    } else {
        defs.push(record);
    }
    if let Err(err) = save_definitions(&defs) {
        tracing::error!(target: "rtv::store", error = %err, "could not write sources.json");
    }
}

/// Remove one source's definition, secret and catalog DB.
pub fn delete_source(id: u64) {
    let (defs, _next) = load_definitions();
    let defs: Vec<StoredSource> = defs.into_iter().filter(|d| d.id != id).collect();
    if let Err(err) = save_definitions(&defs) {
        tracing::error!(target: "rtv::store", error = %err, "could not rewrite sources.json after delete");
    }
    secret_delete(&secret_name(id));
    let db = catalog_db(id);
    if db.exists() {
        match std::fs::remove_dir_all(db.parent().expect("catalog dir has a parent")) {
            Ok(()) => tracing::info!(target: "rtv::store", id, "catalog db removed"),
            Err(err) => {
                tracing::warn!(target: "rtv::store", error = %err, "catalog dir removal failed")
            }
        }
    }
}

/// Persist the section-hidden flags (bulk All/None) of one source.
pub fn save_section_flags(id: u64, hidden: [bool; 3]) {
    let (mut defs, _next) = load_definitions();
    if let Some(slot) = defs.iter_mut().find(|d| d.id == id) {
        slot.hidden = hidden;
        if let Err(err) = save_definitions(&defs) {
            tracing::error!(target: "rtv::store", error = %err, "could not persist section flags");
        }
    }
}

/// The per-source catalog DB path, for callers outside this module
/// (details.rs detail cache). None for id 0.
pub fn catalog_db_path(source_id: u64) -> Option<std::path::PathBuf> {
    (source_id > 0).then(|| catalog_db(source_id))
}

/// Save a full catalog snapshot for one source (post-ingest).
pub fn save_catalog(id: u64, catalog: &Catalog) {
    let started = std::time::Instant::now();
    if let Err(err) = crate::catalogdb::write(&catalog_db(id), catalog) {
        tracing::error!(
            target: "rtv::store",
            error = %err,
            "catalog write FAILED — snapshot stays memory-only for this run"
        );
        return;
    }
    let size_kb = std::fs::metadata(catalog_db(id))
        .map(|m| m.len() / 1024)
        .unwrap_or(0);
    tracing::info!(
        target: "rtv::store",
        ms = started.elapsed().as_millis(),
        kb = size_kb,
        "catalog persisted to sqlite"
    );
}

/// Load the catalog snapshot of one source. `None` when no DB exists
/// (the source never finished an ingest).
pub fn load_catalog(id: u64) -> Option<Catalog> {
    let started = std::time::Instant::now();
    let db = catalog_db(id);
    if !crate::catalogdb::exists(&db) {
        return None;
    }
    match crate::catalogdb::read(&db) {
        Ok(catalog) => {
            tracing::info!(
                target: "rtv::store",
                ms = started.elapsed().as_millis(),
                categories = catalog.categories.len(),
                items = catalog.items.len(),
                "catalog loaded from sqlite"
            );
            Some(catalog)
        }
        Err(err) => {
            tracing::error!(
                target: "rtv::store",
                error = %err,
                "catalog read FAILED — treating as empty; re-ingest will heal"
            );
            None
        }
    }
}

/// Patch per-category visibility flags in the stored snapshot.
pub fn persist_category_flags(id: u64, catalog: &Catalog) {
    let started = std::time::Instant::now();
    if let Err(err) = crate::catalogdb::update_visibility(&catalog_db(id), &catalog.categories) {
        tracing::error!(target: "rtv::store", error = %err, "category visibility persist failed");
        return;
    }
    tracing::info!(
        target: "rtv::store",
        ms = started.elapsed().as_millis(),
        "category flags persisted"
    );
}

// ----------------------------------------------------------- GSettings

/// Remember the active source id (0 = none) in GSettings.
pub fn set_last_source(id: u64) {
    let settings = gtk::gio::Settings::new("io.github.tunix.rtv");
    if settings.set_string("last-source", &id.to_string()).is_err() {
        tracing::warn!(target: "rtv::store", "could not persist last-source");
    }
}

/// The remembered active source id (0 = none / unset).
pub fn last_source() -> u64 {
    let settings = gtk::gio::Settings::new("io.github.tunix.rtv");
    settings.string("last-source").parse::<u64>().unwrap_or(0)
}
