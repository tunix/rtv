//! UI-mock sources layer behind the Settings dialog.
//!
//! Sources live in memory (`SourcesModel`): the "indexing" spinner, stats and
//! category lists are all mock data. The backend phase will replace the
//! model with real fetching/persistence; the dialog layout stays.
//!
//! Layout mirrors valhalla's preferences: a plain [`adw::Dialog`] holding a
//! sidebar (tab list) + a content pane. The Sources pane roots a
//! NavigationView so "Add/Edit source" and the per-source detail (stats +
//! category visibility) live as subpages inside the dialog.
//!
//! Cycle safety: every closure built here captures a [`Ctx`] of WEAK widget
//! refs (plus the model), never a strong `SettingsUi` — the window caches
//! the dialog, so strong widget refs in closures would form widget->closure
//! cycles and leak popped pages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use crate::ui::window::Window;

/// Tag identifying the per-source detail page in the NavigationView.
const DETAIL_TAG: &str = "source-detail";

/// Catalog section, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Live,
    Vod,
    Series,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Live => "Live TV",
            Self::Vod => "Movies",
            Self::Series => "Series",
        }
    }

    /// Row icons (Adwaita stock, verified against the container theme).
    fn icon(self) -> &'static str {
        match self {
            Self::Live => "video-display-symbolic",
            Self::Vod => "video-x-generic-symbolic",
            Self::Series => "view-list-bullet-symbolic",
        }
    }

    fn all() -> [Self; 3] {
        [Self::Live, Self::Vod, Self::Series]
    }
}

/// Connection details of one source. Kept verbatim in memory; the backend
/// phase moves credentials into the keyring (see AGENTS.md: never logged,
/// never cached to disk).
#[derive(Debug, Clone)]
pub enum SourceKind {
    Xtream {
        server: String,
        username: String,
        password: String,
    },
    M3U {
        url: String,
        epg: String,
    },
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Xtream { .. } => "Xtream Codes",
            Self::M3U { .. } => "M3U playlist",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Self::Xtream { .. } => "tv-symbolic",
            Self::M3U { .. } => "text-x-generic-symbolic",
        }
    }
}

/// Mock statistics shown once "indexing" finishes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub live_cats: u32,
    pub live_items: u32,
    pub vod_cats: u32,
    pub vod_items: u32,
    pub series_cats: u32,
    pub series_items: u32,
}

impl Stats {
    fn total_cats(&self) -> u32 {
        self.live_cats + self.vod_cats + self.series_cats
    }

    fn total_items(&self) -> u32 {
        self.live_items + self.vod_items + self.series_items
    }

    fn section(&self, section: Section) -> (u32, u32) {
        match section {
            Section::Live => (self.live_cats, self.live_items),
            Section::Vod => (self.vod_cats, self.vod_items),
            Section::Series => (self.series_cats, self.series_items),
        }
    }
}

/// A top-level category with a mock visibility flag (the backend phase will
/// wire these switches into the grid-rendering filters).
#[derive(Debug, Clone)]
pub struct MockCategory {
    pub name: String,
    pub section: Section,
    pub visible: bool,
}

/// One configured source. `indexing` is the mock stand-in for the async
/// indexing step.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: u64,
    pub name: String,
    pub kind: SourceKind,
    pub indexing: bool,
    pub stats: Option<Stats>,
    pub cats: Vec<MockCategory>,
}

/// Shared in-memory source list, owned by the window and read/written by the
/// settings dialog.
#[derive(Debug, Default)]
pub struct SourcesModel {
    items: RefCell<Vec<Source>>,
    next_id: Cell<u64>,
}

impl SourcesModel {
    pub fn all(&self) -> Vec<Source> {
        self.items.borrow().clone()
    }

    pub fn get(&self, id: u64) -> Option<Source> {
        self.items.borrow().iter().find(|s| s.id == id).cloned()
    }

    pub fn clear(&self) {
        self.items.borrow_mut().clear();
        self.next_id.set(0);
    }

    /// Reserves the next id, hands it to the factory (mock stats are seeded
    /// from it) and appends the result.
    pub fn add(&self, source_factory: impl FnOnce(u64) -> Source) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.items.borrow_mut().push(source_factory(id));
        id
    }

    pub fn replace(&self, source: Source) {
        let mut items = self.items.borrow_mut();
        if let Some(slot) = items.iter_mut().find(|s| s.id == source.id) {
            *slot = source;
        }
    }

    pub fn remove(&self, id: u64) {
        self.items.borrow_mut().retain(|s| s.id != id);
    }

    /// Mock indexing: flips the source to ready.
    pub fn set_ready(&self, id: u64) {
        let mut items = self.items.borrow_mut();
        if let Some(source) = items.iter_mut().find(|s| s.id == id) {
            source.indexing = false;
        }
    }

    /// Toggles one category's mock visibility flag.
    pub fn set_category_visible(&self, id: u64, section: Section, name: &str, visible: bool) {
        let mut items = self.items.borrow_mut();
        if let Some(source) = items.iter_mut().find(|s| s.id == id) {
            for cat in &mut source.cats {
                if cat.section == section && cat.name == name {
                    cat.visible = visible;
                }
            }
        }
    }
}

/// Whether one of the form's entry rows holds non-blank text. Generic over
/// EntryRow/PasswordEntryRow (both extend GtkEditable).
fn filled<T: glib::object::IsA<gtk::Editable>>(row: &glib::WeakRef<T>) -> bool {
    row.upgrade()
        .map(|r| !r.text().trim().is_empty())
        .unwrap_or(false)
}

/// Thousands separator for the mock stats ("24,300").
fn fmt_count(n: u32) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len.saturating_sub(1) / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Row subtitle: kind + catalog size, or the indexing state.
fn source_subtitle(source: &Source) -> String {
    match (&source.stats, source.indexing) {
        (Some(stats), false) => format!(
            "{} · {} categories · {} contents",
            source.kind.label(),
            fmt_count(stats.total_cats()),
            fmt_count(stats.total_items())
        ),
        _ => format!("{} · Indexing…", source.kind.label()),
    }
}

/// Counts entries in one of the local dumps (mock/ or samples/); mock stats
/// prefer real local data so numbers look plausible.
fn count_json(path: &str) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    match value {
        serde_json::Value::Array(items) => Some(items.len() as u32),
        serde_json::Value::Object(map) => Some(map.len() as u32),
        _ => None,
    }
}

fn local_counts(data_dir: &str) -> Option<[u32; 6]> {
    let files = [
        "live_categories.json",
        "live_streams.json",
        "vod_categories.json",
        "vod_streams.json",
        "series_categories.json",
        "series.json",
    ];
    let mut out = [0u32; 6];
    for (i, file) in files.iter().enumerate() {
        out[i] = count_json(&format!("{data_dir}/{file}"))?;
    }
    Some(out)
}

/// Category names for one section from the local dumps (or canned).
fn local_category_names(data_dir: &str, section: Section) -> Vec<String> {
    let file = match section {
        Section::Live => "live_categories.json",
        Section::Vod => "vod_categories.json",
        Section::Series => "series_categories.json",
    };
    let fallback = || -> Vec<String> {
        let names: &[&str] = match section {
            Section::Live => &[
                "Sports",
                "News",
                "Entertainment",
                "Movies HD",
                "Documentary",
                "Kids",
            ],
            Section::Vod => &["Feature Films", "Independent", "Classics", "Documentary"],
            Section::Series => &["Scripted", "Reality", "Animation", "Documentary Series"],
        };
        names.iter().map(|s| s.to_string()).collect()
    };
    let text = match std::fs::read_to_string(format!("{data_dir}/{file}")) {
        Ok(text) => text,
        Err(_) => return fallback(),
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&text) {
        Ok(entries) => entries,
        Err(_) => return fallback(),
    };
    let names: Vec<String> = entries
        .iter()
        .filter_map(|e| e.get("category_name")?.as_str().map(String::from))
        .collect();
    if names.is_empty() {
        fallback()
    } else {
        names
    }
}

/// Canned M3U group names (the M3U backend is a stub).
fn mock_m3u_groups() -> Vec<String> {
    ["Sports", "News", "Entertainment", "Kids", "Music", "24/7"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Mock stats for a fresh source: item counts scaled from the local dump
/// sizes so the UX resembles a real provider's catalog size.
fn mock_stats(id: u64, kind: &SourceKind, data_dir: &str) -> Stats {
    if matches!(kind, SourceKind::M3U { .. }) {
        let scale = 1 + (id % 4) as u32;
        let groups = mock_m3u_groups().len() as u32;
        return Stats {
            live_cats: groups,
            live_items: 4_200 * scale,
            vod_cats: groups / 2,
            vod_items: 800 * scale,
            series_cats: groups / 2,
            series_items: 1_500 * scale,
        };
    }
    let counts = local_counts(data_dir).unwrap_or([24, 19, 9, 26, 8, 24]);
    let factor = |raw: u32, salt: u32| -> u32 {
        let scale = 250 + ((id as u32).wrapping_mul(617) + salt) % 1200;
        raw * scale
    };
    Stats {
        live_cats: counts[0],
        live_items: factor(counts[1], 0),
        vod_cats: counts[2],
        vod_items: factor(counts[3], 131),
        series_cats: counts[4],
        series_items: factor(counts[5], 389),
    }
}

/// Builds a fresh in-memory source (still indexing, stats pre-computed).
fn new_source(id: u64, name: &str, kind: SourceKind) -> Source {
    let data_dir = crate::mock::data_dir();
    let stats = mock_stats(id, &kind, data_dir);
    let cats = match kind {
        SourceKind::M3U { .. } => mock_m3u_groups()
            .into_iter()
            .map(|name| MockCategory {
                name,
                section: Section::Live,
                visible: true,
            })
            .collect(),
        SourceKind::Xtream { .. } => Section::all()
            .iter()
            .flat_map(|&section| {
                local_category_names(data_dir, section)
                    .into_iter()
                    .map(|name| MockCategory {
                        name,
                        section,
                        visible: true,
                    })
                    .collect::<Vec<_>>()
            })
            .collect(),
    };
    Source {
        id,
        name: name.to_string(),
        kind,
        indexing: true,
        stats: Some(stats),
        cats,
    }
}

/// Tracking for the open per-source detail page: re-renders after mock
/// indexing finishes and cleans up when the page pops.
#[derive(Default)]
struct DetailState {
    open_id: Cell<Option<u64>>,
    content: RefCell<Option<gtk::Box>>,
}

impl DetailState {
    fn clear(&self) {
        self.open_id.set(None);
        *self.content.borrow_mut() = None;
    }
}

/// Weak-ref bundle captured by every closure built here (see module docs).
#[derive(Clone)]
struct Ctx {
    model: Rc<SourcesModel>,
    detail: Rc<DetailState>,
    dialog: glib::WeakRef<adw::Dialog>,
    toasts: glib::WeakRef<adw::ToastOverlay>,
    nav: glib::WeakRef<adw::NavigationView>,
    list: glib::WeakRef<gtk::ListBox>,
    stack: glib::WeakRef<gtk::Stack>,
    win: glib::WeakRef<Window>,
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx").finish_non_exhaustive()
    }
}

impl Ctx {
    /// Toasts inside the dialog while it is presented; on the window after.
    fn toast(&self, msg: &str) {
        if let Some(overlay) = self.toasts.upgrade() {
            overlay.add_toast(adw::Toast::new(msg));
        } else if let Some(win) = self.win.upgrade() {
            win.toast(msg);
        }
    }

    /// After any mutation: refresh the rows and sync the window (dropdown,
    /// welcome/browse gating).
    fn sync(&self) {
        self.refresh();
        if let Some(win) = self.win.upgrade() {
            win.sync_sources_ui();
        }
    }

    fn refresh(&self) {
        let (Some(list), Some(stack)) = (self.list.upgrade(), self.stack.upgrade()) else {
            return;
        };
        let sources = self.model.all();
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        for source in &sources {
            list.append(&self.build_row(source));
        }
        stack.set_visible_child_name(if sources.is_empty() { "empty" } else { "list" });
    }

    fn build_row(&self, source: &Source) -> gtk::ListBoxRow {
        let row = adw::ActionRow::builder()
            .title(&source.name)
            .subtitle(source_subtitle(source))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(source.kind.icon()));

        let suffix = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        if source.indexing {
            suffix.append(
                &gtk::Spinner::builder()
                    .spinning(true)
                    .valign(gtk::Align::Center)
                    .build(),
            );
        }
        let edit = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit source")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let trash = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete source")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        suffix.append(&edit);
        suffix.append(&trash);
        row.add_suffix(&suffix);

        let id = source.id;
        let edit_ctx = self.clone();
        edit.connect_clicked(move |_| {
            let Some(source) = edit_ctx.model.get(id) else {
                return;
            };
            edit_ctx.push_form(Some(&source));
        });
        let trash_ctx = self.clone();
        trash.connect_clicked(move |_| trash_ctx.confirm_delete(id));

        // GtkListBox's row activation does not fire reliably for AdwActionRow
        // here (the click gesture is consumed before the list's activation),
        // so the empty row area is activated explicitly: a click that is not
        // on the suffix buttons opens the detail page.
        let click_ctx = self.clone();
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        let row_weak = row.downgrade();
        click.connect_released(move |_, _, x, _| {
            // Ignore clicks that land on the suffix buttons.
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            if x > row.width() as f64 - 120.0 {
                return;
            }
            click_ctx.push_detail(id);
        });
        row.add_controller(click);

        row.into()
    }

    /// Simulated indexing: the source becomes ready after a short delay; the
    /// list (and an open detail page) re-render. The callback holds only the
    /// Ctx, so a deleted source no-ops.
    fn schedule_indexing(&self, id: u64) {
        let ctx = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(2200 + (id % 3) * 900), move || {
            let Some(source) = ctx.model.get(id) else {
                return;
            };
            if !source.indexing {
                return;
            }
            ctx.model.set_ready(id);
            ctx.refresh();
            if ctx.detail.open_id.get() == Some(id) {
                ctx.rebuild_detail(id);
            }
            if let Some(stats) = source.stats {
                ctx.toast(&format!(
                    "Source ready — {} categories, {} contents",
                    fmt_count(stats.total_cats()),
                    fmt_count(stats.total_items())
                ));
            }
        });
    }

    fn confirm_delete(&self, id: u64) {
        let Some(source) = self.model.get(id) else {
            return;
        };
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let alert = adw::AlertDialog::builder()
            .heading("Delete source?")
            .body(format!(
                "\u{201c}{}\u{201d} and its content will be removed from RTV.",
                source.name
            ))
            .build();
        alert.add_response("cancel", "_Cancel");
        alert.add_response("delete", "_Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");

        let ctx = self.clone();
        alert.choose(Some(&dialog), None::<&gio::Cancellable>, move |response| {
            if response != "delete" {
                return;
            }
            ctx.model.remove(id);
            if ctx.detail.open_id.get() == Some(id) {
                if let Some(nav) = ctx.nav.upgrade() {
                    nav.pop();
                }
            }
            ctx.toast("Source deleted");
            ctx.sync();
        });
    }

    /// Add/Edit form subpage. `existing` selects edit mode.
    fn push_form(&self, existing: Option<&Source>) {
        let Some(nav) = self.nav.upgrade() else {
            return;
        };
        let adding = existing.is_none();
        let existing = existing.cloned();

        let kind_toggle = adw::ToggleGroup::builder()
            .valign(gtk::Align::Center)
            .build();
        kind_toggle.add(
            adw::Toggle::builder()
                .name("xtream")
                .label("Xtream Codes")
                .build(),
        );
        kind_toggle.add(
            adw::Toggle::builder()
                .name("m3u")
                .label("M3U playlist")
                .build(),
        );
        kind_toggle.set_sensitive(adding);

        let type_group = adw::PreferencesGroup::new();
        let type_row = adw::ActionRow::builder()
            .title("Source type")
            .activatable(false)
            .build();
        type_row.add_suffix(&kind_toggle);
        type_group.add(&type_row);

        let name_row = adw::EntryRow::builder().title("Name").build();
        let details_group = adw::PreferencesGroup::builder().title("Details").build();
        details_group.add(&name_row);

        let server_row = adw::EntryRow::builder().title("Server URL").build();
        let username_row = adw::EntryRow::builder().title("Username").build();
        let password_row = adw::PasswordEntryRow::builder().title("Password").build();
        let xtream_group = adw::PreferencesGroup::builder()
            .title("Xtream account")
            .description("Credentials are stored in the keyring.")
            .build();
        xtream_group.add(&server_row);
        xtream_group.add(&username_row);
        xtream_group.add(&password_row);

        let url_row = adw::EntryRow::builder().title("Playlist URL").build();
        let epg_row = adw::EntryRow::builder().title("EPG URL (optional)").build();
        let m3u_group = adw::PreferencesGroup::builder().title("Playlist").build();
        m3u_group.add(&url_row);
        m3u_group.add(&epg_row);

        let page = adw::PreferencesPage::new();
        page.set_margin_top(12);
        page.set_margin_bottom(24);
        page.set_margin_start(12);
        page.set_margin_end(12);
        page.add(&type_group);
        page.add(&details_group);
        page.add(&xtream_group);
        page.add(&m3u_group);

        // Prefill + set the initial kind before the handlers below are wired
        // (so the initial set does not fire them).
        let is_xtream = match &existing {
            Some(source) => match &source.kind {
                SourceKind::Xtream {
                    server,
                    username,
                    password,
                } => {
                    name_row.set_text(&source.name);
                    server_row.set_text(server);
                    username_row.set_text(username);
                    password_row.set_text(password);
                    true
                }
                SourceKind::M3U { url, epg } => {
                    name_row.set_text(&source.name);
                    url_row.set_text(url);
                    epg_row.set_text(epg);
                    false
                }
            },
            None => true,
        };
        kind_toggle.set_active_name(Some(if is_xtream { "xtream" } else { "m3u" }));
        xtream_group.set_visible(is_xtream);
        m3u_group.set_visible(!is_xtream);

        let groups_weak = (xtream_group.downgrade(), m3u_group.downgrade());
        kind_toggle.connect_active_name_notify(move |toggle| {
            let (Some(xtream), Some(m3u)) = (groups_weak.0.upgrade(), groups_weak.1.upgrade())
            else {
                return;
            };
            let is_xtream = toggle.active_name().as_deref() == Some("xtream");
            xtream.set_visible(is_xtream);
            m3u.set_visible(!is_xtream);
        });

        let save = gtk::Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        let save_weak = save.downgrade();
        let update_save = {
            let name_weak = name_row.downgrade();
            let server_weak = server_row.downgrade();
            let username_weak = username_row.downgrade();
            let password_weak = password_row.downgrade();
            let url_weak = url_row.downgrade();
            let kind_weak = kind_toggle.downgrade();
            move || {
                let (Some(save), Some(kind)) = (save_weak.upgrade(), kind_weak.upgrade()) else {
                    return;
                };
                let name_ok = filled(&name_weak);
                let kind_ok = match kind.active_name().as_deref() {
                    Some("xtream") => {
                        filled(&server_weak) && filled(&username_weak) && filled(&password_weak)
                    }
                    Some("m3u") => filled(&url_weak),
                    _ => false,
                };
                save.set_sensitive(name_ok && kind_ok);
            }
        };
        for row in [&name_row, &server_row, &username_row, &url_row] {
            let update = update_save.clone();
            row.connect_changed(move |_| update());
        }
        {
            let update = update_save.clone();
            password_row.connect_changed(move |_| update());
        }
        let update_on_toggle = update_save.clone();
        kind_toggle.connect_active_name_notify(move |_| update_on_toggle());
        update_save();

        // No manual back button here: adw::NavigationView automatically adds
        // one to the page's header bar.
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(
            if adding { "Add source" } else { "Edit source" },
            "",
        )));
        header.pack_end(&save);

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&page)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroll));
        let nav_page = adw::NavigationPage::new(&toolbar, "Source");

        let save_ctx = self.clone();
        let nav_weak = nav.downgrade();
        let name_weak = name_row.downgrade();
        let kind_weak = kind_toggle.downgrade();
        let server_weak = server_row.downgrade();
        let username_weak = username_row.downgrade();
        let password_weak = password_row.downgrade();
        let url_weak = url_row.downgrade();
        save.connect_clicked(move |_| {
            let (Some(name_row), Some(kind_toggle)) = (name_weak.upgrade(), kind_weak.upgrade())
            else {
                return;
            };
            let name = name_row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let kind = match kind_toggle.active_name().as_deref() {
                Some("xtream") => {
                    let (Some(server_row), Some(username_row), Some(password_row)) = (
                        server_weak.upgrade(),
                        username_weak.upgrade(),
                        password_weak.upgrade(),
                    ) else {
                        return;
                    };
                    SourceKind::Xtream {
                        server: server_row.text().trim().to_string(),
                        username: username_row.text().trim().to_string(),
                        password: password_row.text().trim().to_string(),
                    }
                }
                Some("m3u") => {
                    let Some(url_row) = url_weak.upgrade() else {
                        return;
                    };
                    SourceKind::M3U {
                        url: url_row.text().trim().to_string(),
                        epg: String::new(),
                    }
                }
                _ => return,
            };
            match existing.as_ref() {
                Some(source) => {
                    let mut updated = source.clone();
                    updated.name = name;
                    updated.kind = kind;
                    save_ctx.model.replace(updated);
                    save_ctx.toast("Source saved");
                }
                None => {
                    let id = save_ctx.model.add(|id| new_source(id, &name, kind));
                    save_ctx.toast("Source added — indexing…");
                    save_ctx.schedule_indexing(id);
                }
            }
            save_ctx.sync();
            if let Some(nav) = nav_weak.upgrade() {
                nav.pop();
            }
        });

        nav.push(&nav_page);
    }

    /// Per-source subpage: stats overview + mock category visibility.
    fn push_detail(&self, id: u64) {
        let Some(nav) = self.nav.upgrade() else {
            return;
        };
        let Some(source) = self.model.get(id) else {
            return;
        };

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        self.fill_detail(&content, &source);

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(
            &source.name,
            source.kind.label(),
        )));
        // No manual back button: NavigationView provides one automatically.

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroll));
        let nav_page = adw::NavigationPage::new(&toolbar, &source.name);
        nav_page.set_tag(Some(DETAIL_TAG));

        self.detail.open_id.set(Some(id));
        *self.detail.content.borrow_mut() = Some(content);
        nav.push(&nav_page);
    }

    /// Re-renders the open detail page after mock indexing finished.
    fn rebuild_detail(&self, id: u64) {
        let Some(source) = self.model.get(id) else {
            return;
        };
        let Some(content) = self.detail.content.borrow().clone() else {
            return;
        };
        while let Some(child) = content.first_child() {
            content.remove(&child);
        }
        self.fill_detail(&content, &source);
    }

    fn fill_detail(&self, content: &gtk::Box, source: &Source) {
        let page = adw::PreferencesPage::new();
        page.set_margin_top(12);
        page.set_margin_bottom(24);
        page.set_margin_start(12);
        page.set_margin_end(12);

        let overview = adw::PreferencesGroup::builder()
            .title("Overview")
            .description("Statistics come from the source's catalog.")
            .build();
        page.add(&overview);

        if source.indexing {
            let row = adw::ActionRow::builder()
                .title("Indexing…")
                .subtitle("Stats and categories appear here when the source is ready.")
                .activatable(false)
                .build();
            row.add_prefix(&gtk::Spinner::builder().spinning(true).build());
            overview.add(&row);
        } else if let Some(stats) = source.stats {
            for section in Section::all() {
                let (cats, items) = stats.section(section);
                let row = adw::ActionRow::builder()
                    .title(section.label())
                    .subtitle(format!(
                        "{} categories · {} contents",
                        fmt_count(cats),
                        fmt_count(items)
                    ))
                    .activatable(false)
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name(section.icon()));
                overview.add(&row);
            }
        }

        if !source.indexing {
            for section in Section::all() {
                let cats: Vec<_> = source
                    .cats
                    .iter()
                    .filter(|c| c.section == section)
                    .collect();
                if cats.is_empty() {
                    continue;
                }
                let group = adw::PreferencesGroup::builder()
                    .title(format!("{} categories", section.label()))
                    .description("Hidden categories stay in the source but are skipped when rendering content grids.")
                    .build();
                for cat in cats {
                    let switch_row = adw::SwitchRow::builder()
                        .title(&cat.name)
                        .active(cat.visible)
                        .build();
                    let ctx = self.clone();
                    let (id, name) = (source.id, cat.name.clone());
                    switch_row.connect_active_notify(move |row| {
                        ctx.model
                            .set_category_visible(id, section, &name, row.is_active());
                    });
                    group.add(&switch_row);
                }
                page.add(&group);
            }
        }

        let danger = adw::PreferencesGroup::new();
        let delete_row = adw::ButtonRow::builder().title("Delete source…").build();
        delete_row.add_css_class("destructive-action");
        let delete_ctx = self.clone();
        let id = source.id;
        delete_row.connect_activated(move |_| delete_ctx.confirm_delete(id));
        danger.add(&delete_row);
        page.add(&danger);

        content.append(&page);
    }
}

/// The settings dialog plus the widgets its closures reference. Cached on the
/// window so the dialog (and its subpage stack) survives across presents.
#[derive(Debug)]
pub struct SettingsUi {
    pub dialog: adw::Dialog,
    ctx: Ctx,
}

impl SettingsUi {
    pub fn build(win: glib::WeakRef<Window>, model: Rc<SourcesModel>) -> SettingsUi {
        let nav = adw::NavigationView::new();
        let list = gtk::ListBox::builder()
            .css_classes(["boxed-list"])
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .valign(gtk::Align::Start)
            .build();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vhomogeneous(false)
            .build();
        let toasts = adw::ToastOverlay::new();
        let detail = Rc::new(DetailState::default());

        // Sources pane root: empty state (first run) vs source list. The
        // icon matches the sidebar's Sources tab (tv-symbolic) — one glyph
        // for the concept, monochrome Adwaita stock.
        let empty = adw::StatusPage::builder()
            .icon_name("tv-symbolic")
            .title("No sources yet")
            .description("Add an IPTV source — an Xtream Codes account or an M3U playlist — to start watching.")
            .build();
        let add_empty = gtk::Button::builder()
            .label("Add source…")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        empty.set_child(Some(&add_empty));
        stack.add_named(&empty, Some("empty"));

        let sources_group = adw::PreferencesGroup::builder()
            .title("Sources")
            .description("Open a source for stats and category management.")
            .build();
        sources_group.add(&list);

        let add_row = adw::ButtonRow::builder().title("Add source…").build();
        add_row.add_css_class("suggested-action");
        let add_group = adw::PreferencesGroup::new();
        add_group.add(&add_row);

        let list_page = adw::PreferencesPage::new();
        list_page.set_margin_top(12);
        list_page.set_margin_bottom(24);
        list_page.set_margin_start(12);
        list_page.set_margin_end(12);
        list_page.add(&sources_group);
        list_page.add(&add_group);

        let list_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(620)
                    .child(&list_page)
                    .build(),
            )
            .build();
        stack.add_named(&list_scroll, Some("list"));

        let narrow_back = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .visible(false)
            .build();

        let root_header = adw::HeaderBar::new();
        root_header.set_title_widget(Some(&adw::WindowTitle::new("Sources", "")));
        root_header.pack_start(&narrow_back);

        let root_toolbar = adw::ToolbarView::new();
        root_toolbar.add_top_bar(&root_header);
        root_toolbar.set_content(Some(&stack));
        let root_page = adw::NavigationPage::new(&root_toolbar, "Sources");
        nav.add(&root_page);

        // Settings sidebar (valhalla-style tab list). Only "Sources" now —
        // more tabs land later.
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.add_css_class("navigation-sidebar");
        let sources_tab = gtk::ListBoxRow::new();
        let tab_box = gtk::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        tab_box.append(
            &gtk::Image::builder()
                .icon_name("tv-symbolic")
                .pixel_size(24)
                .build(),
        );
        tab_box.append(&gtk::Label::builder().label("Sources").xalign(0.0).build());
        sources_tab.set_child(Some(&tab_box));
        sidebar_list.append(&sources_tab);

        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(&adw::HeaderBar::new());
        sidebar.set_content(Some(&sidebar_list));
        let sidebar_page = adw::NavigationPage::new(&sidebar, "Preferences");

        let content_page = adw::NavigationPage::new(&nav, "Content");

        let split = adw::NavigationSplitView::new();
        split.set_min_sidebar_width(200.0);
        split.set_max_sidebar_width(240.0);
        split.set_sidebar(Some(&sidebar_page));
        split.set_content(Some(&content_page));

        toasts.set_child(Some(&split));

        let dialog = adw::Dialog::builder()
            .title("Settings")
            .content_width(820)
            .content_height(600)
            .child(&toasts)
            .build();

        // Narrow widths: collapse the split; the root pane then needs a back
        // button to reach the sidebar.
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 560sp").expect("valid breakpoint"),
        );
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&narrow_back, "visible", Some(&true.to_value()));
        dialog.add_breakpoint(narrow);

        let split_weak = split.downgrade();
        narrow_back.connect_clicked(move |_| {
            if let Some(split) = split_weak.upgrade() {
                split.set_show_content(false);
            }
        });

        // Sidebar selection: show the content pane (narrow) and pop any
        // subpages back to the Sources root.
        {
            let split_weak = split.downgrade();
            let nav_weak = nav.downgrade();
            let root_weak = root_page.downgrade();
            sidebar_list.connect_row_selected(move |_, selected| {
                if selected.is_some() {
                    if let Some(split) = split_weak.upgrade() {
                        split.set_show_content(true);
                    }
                    if let Some(nav) = nav_weak.upgrade() {
                        if let Some(root) = root_weak.upgrade() {
                            nav.pop_to_page(&root);
                        }
                    }
                }
            });
        }
        sidebar_list.select_row(Some(&sources_tab));

        let ctx = Ctx {
            model: model.clone(),
            detail: detail.clone(),
            dialog: dialog.downgrade(),
            toasts: toasts.downgrade(),
            nav: nav.downgrade(),
            list: list.downgrade(),
            stack: stack.downgrade(),
            win: win.clone(),
        };

        // Wiring. Row activation maps by list position — refresh() appends
        // rows in model order.
        let list_ctx = ctx.clone();
        list.connect_row_activated(move |_, row| {
            let index = row.index();
            let source = list_ctx.model.all().get(index as usize).cloned();
            if let Some(source) = source {
                list_ctx.push_detail(source.id);
            }
        });
        let add_ctx = ctx.clone();
        add_empty.connect_clicked(move |_| add_ctx.push_form(None));
        let add_ctx = ctx.clone();
        add_row.connect_activated(move |_| add_ctx.push_form(None));

        let detail_ctx = detail.clone();
        nav.connect_popped(move |_, page| {
            if page.tag().as_deref() == Some(DETAIL_TAG) {
                detail_ctx.clear();
            }
        });

        // On dialog close: pop subpages back to the Sources root so the next
        // present starts fresh, and unlock browsing when a source exists.
        {
            let nav_weak = nav.downgrade();
            let root_weak = root_page.downgrade();
            let win_weak = win.clone();
            dialog.connect_closed(move |_| {
                if let Some(nav) = nav_weak.upgrade() {
                    if let Some(root) = root_weak.upgrade() {
                        nav.pop_to_page(&root);
                    }
                }
                if let Some(win) = win_weak.upgrade() {
                    win.on_sources_closed();
                }
            });
        }

        ctx.refresh();

        SettingsUi { dialog, ctx }
    }

    /// Rebuilds the rows and presents the dialog.
    pub fn present(&self, win: &Window) {
        self.ctx.refresh();
        self.dialog.present(Some(win));
    }

    /// Rebuilds the rows without presenting (mock reset).
    pub fn refresh(&self) {
        self.ctx.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_count_inserts_thousands_separators() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1,000");
        assert_eq!(fmt_count(24_300), "24,300");
        assert_eq!(fmt_count(1_234_567), "1,234,567");
    }

    #[test]
    fn model_add_replace_remove_roundtrip() {
        let model = SourcesModel::default();
        let id = model.add(|id| {
            new_source(
                id,
                "A",
                SourceKind::M3U {
                    url: "u".into(),
                    epg: String::new(),
                },
            )
        });
        let added = model.get(id).expect("added");
        assert_eq!(added.name, "A");
        assert!(added.indexing);
        assert!(added.stats.is_some());

        model.set_ready(id);
        assert!(!model.get(id).expect("ready").indexing);

        let mut updated = model.get(id).expect("ready");
        updated.name = "B".into();
        model.replace(updated);
        assert_eq!(model.get(id).expect("replaced").name, "B");

        model.remove(id);
        assert!(model.get(id).is_none());
        assert!(model.all().is_empty());
    }

    #[test]
    fn set_category_visible_flips_only_one() {
        let model = SourcesModel::default();
        let id = model.add(|id| {
            new_source(
                id,
                "A",
                SourceKind::Xtream {
                    server: "https://x".into(),
                    username: "u".into(),
                    password: "p".into(),
                },
            )
        });
        let live_first = model
            .get(id)
            .expect("source")
            .cats
            .iter()
            .find(|c| c.section == Section::Live)
            .expect("live category")
            .name
            .clone();
        model.set_category_visible(id, Section::Live, &live_first, false);
        let source = model.get(id).expect("source");
        let live = source
            .cats
            .iter()
            .filter(|c| c.section == Section::Live)
            .count();
        let hidden_live = source
            .cats
            .iter()
            .filter(|c| c.section == Section::Live && !c.visible)
            .count();
        assert_eq!(hidden_live, 1);
        let hidden_total = source.cats.iter().filter(|c| !c.visible).count();
        assert_eq!(hidden_total, 1);
        assert!(live > 0);
    }

    #[test]
    fn source_subtitle_covers_indexing_and_ready() {
        let model = SourcesModel::default();
        let id = model.add(|id| {
            new_source(
                id,
                "A",
                SourceKind::Xtream {
                    server: "s".into(),
                    username: "u".into(),
                    password: "p".into(),
                },
            )
        });
        let indexing = model.get(id).expect("source");
        assert!(source_subtitle(&indexing).ends_with("Indexing…"));
        model.set_ready(id);
        let ready = model.get(id).expect("source");
        let subtitle = source_subtitle(&ready);
        assert!(subtitle.starts_with("Xtream Codes"));
        assert!(subtitle.contains("categories"));
    }
}
