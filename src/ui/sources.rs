//! Source management: the Settings dialog's Sources tab backed by the
//! REAL ingestion pipeline. Adding a source kicks off `ingest::start`:
//! progress callbacks update the rows live, failures surface as toasts +
//! error rows, and the finished catalog lands on the window via
//! `Window::apply_catalog`.
//!
//! Bulk category selection: each section group carries "All"/"None"
//! buttons so users do not toggle hundreds of switches one by one.

use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

/// Connection details of one source. Kept verbatim in memory; the
/// persistence phase moves credentials into the keyring (see AGENTS.md:
/// never logged, never cached to disk).
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

/// LIVE index state per source — one of the numbers the user asked to
/// see in the logs and on the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// Waiting for the fetch chain to finish.
    Indexing,
    /// Ingest finished; a catalog snapshot was published.
    Ready,
    /// Ingest failed (auth, network, parse). `message` explains.
    Failed,
}

/// One configured source. `state` tracks the real ingest; `snapshot`
/// holds the catalog THIS source produced so switching sources can
/// re-publish it without re-fetching.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: u64,
    pub name: String,
    pub kind: SourceKind,
    pub state: SourceState,
    pub failure: Option<String>,
    /// Counters from the last ingest (0 while indexing).
    pub live_cats: u32,
    pub live_items: u32,
    pub vod_cats: u32,
    pub vod_items: u32,
    pub series_cats: u32,
    pub series_items: u32,
    /// Whether each section's categories are visible. Indexed by
    /// Section::all() position; per-category flags live in the catalog
    /// snapshot (Category.visible) — the model carries the fast path.
    pub section_hidden: [bool; 3],
    /// The catalog this source produced (None until the ingest finished;
    /// kept so switching sources can re-publish without re-fetching).
    pub snapshot: Option<std::rc::Rc<crate::catalog::Catalog>>,
}

impl Source {
    pub fn section(&self, section: Section) -> (u32, u32) {
        match section {
            Section::Live => (self.live_cats, self.live_items),
            Section::Vod => (self.vod_cats, self.vod_items),
            Section::Series => (self.series_cats, self.series_items),
        }
    }

    fn total_cats(&self) -> u32 {
        self.live_cats + self.vod_cats + self.series_cats
    }

    fn total_items(&self) -> u32 {
        self.live_items + self.vod_items + self.series_items
    }

    fn is_indexing(&self) -> bool {
        self.state == SourceState::Indexing
    }
}

/// Shared source list, owned by the window and read/written by the
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

    /// Reserves the next id, hands it to the factory and appends.
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

    /// Patch one source in place with a mutable closure.
    pub fn update(&self, id: u64, f: impl FnOnce(&mut Source)) {
        let mut items = self.items.borrow_mut();
        if let Some(source) = items.iter_mut().find(|s| s.id == id) {
            f(source);
        }
    }

    /// Flip one section's whole visibility (bulk All/None).
    pub fn set_section_visible(&self, id: u64, section: Section, visible: bool) {
        self.update(id, |source| {
            source.section_hidden[section.index()] = !visible;
        });
    }

    /// Category-level visibility lives on the catalog snapshot (window);
    /// the model keeps no per-category copy.
    pub fn set_category_visible(&self, _id: u64, _section: Section, _name: &str, _visible: bool) {}
}

impl Section {
    /// The catalog Kind this section maps to.
    pub fn kind(self) -> crate::catalog::Kind {
        match self {
            Self::Live => crate::catalog::Kind::Live,
            Self::Vod => crate::catalog::Kind::Vod,
            Self::Series => crate::catalog::Kind::Series,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Live => 0,
            Self::Vod => 1,
            Self::Series => 2,
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

/// Thousands separator for the stats ("24,300").
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

/// Snapshot-derived per-section counters (the settings rows' numbers).
struct SectionCounts {
    live_cats: u32,
    live_items: u32,
    vod_cats: u32,
    vod_items: u32,
    series_cats: u32,
    series_items: u32,
}

impl SectionCounts {
    fn of(catalog: &crate::catalog::Catalog) -> Self {
        let cats = |kind: crate::catalog::Kind| {
            catalog.categories.iter().filter(|c| c.kind == kind).count() as u32
        };
        let items = |kind: crate::catalog::Kind| {
            catalog.items.iter().filter(|i| i.kind == kind).count() as u32
        };
        Self {
            live_cats: cats(crate::catalog::Kind::Live),
            live_items: items(crate::catalog::Kind::Live),
            vod_cats: cats(crate::catalog::Kind::Vod),
            vod_items: items(crate::catalog::Kind::Vod),
            series_cats: cats(crate::catalog::Kind::Series),
            series_items: items(crate::catalog::Kind::Series),
        }
    }
}

/// Row subtitle: state-aware — indexing with live counts, failure reason
/// or the ready stats line.
fn source_subtitle(source: &Source) -> String {
    match source.state {
        SourceState::Indexing => format!(
            "{} · Loading… ({} categories · {} contents so far)",
            source.kind.label(),
            fmt_count(source.total_cats()),
            fmt_count(source.total_items())
        ),
        SourceState::Failed => format!(
            "{} · Failed{}",
            source.kind.label(),
            source
                .failure
                .as_deref()
                .map(|msg| format!(" — {msg}"))
                .unwrap_or_default()
        ),
        SourceState::Ready => format!(
            "{} · {} categories · {} contents",
            source.kind.label(),
            fmt_count(source.total_cats()),
            fmt_count(source.total_items())
        ),
    }
}

/// Tracking for the open per-source detail page: re-renders after
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
        // Source names and subtitles carry user input / panel data, which
        // can include a bare "&" — Adwaita parses row titles as Pango
        // markup, so plain text wins (see the PR #20 lesson).
        let row = adw::ActionRow::builder()
            .title(&source.name)
            .subtitle(source_subtitle(source))
            .use_markup(false)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(source.kind.icon()));

        let suffix = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        if source.is_indexing() {
            suffix.append(
                &gtk::Spinner::builder()
                    .spinning(true)
                    .valign(gtk::Align::Center)
                    .build(),
            );
        }
        if source.state == SourceState::Failed {
            suffix.append(
                &gtk::Image::builder()
                    .icon_name("dialog-warning-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["warning"])
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
        // Retry button on failed sources: re-runs the ingest with the
        // same credentials without re-typing anything.
        if source.state == SourceState::Failed {
            let retry = gtk::Button::builder()
                .icon_name("view-refresh-symbolic")
                .tooltip_text("Retry loading")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let retry_ctx = self.clone();
            let retry_id = source.id;
            retry.connect_clicked(move |_| retry_ctx.start_ingest(retry_id));
            suffix.append(&retry);
        }
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

        // GtkListBox's row activation does not fire reliably for
        // AdwActionRow (the click gesture is consumed first), so the
        // empty row area is activated explicitly — the same workaround
        // as before, minus clicks on the suffix buttons.
        let click_ctx = self.clone();
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        let row_weak = row.downgrade();
        click.connect_released(move |_, _, x, _| {
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

    /// The REAL ingest: fires `ingest::start` with this source's
    /// credentials. Progress callbacks re-render the row subtitle with
    /// live counts (cheap: rows are rebuilt only on stage boundaries, a
    /// handful of times per ingest — not per item). Completion publishes
    /// the catalog to the window and flips the state to Ready.
    fn start_ingest(&self, id: u64) {
        let Some(source) = self.model.get(id) else {
            return;
        };
        tracing::info!(target: "rtv::ingest", id, name = %source.name, "ingest kicked off from settings");

        self.model.update(id, |s| {
            s.state = SourceState::Indexing;
            s.failure = None;
            s.live_cats = 0;
            s.live_items = 0;
            s.vod_cats = 0;
            s.vod_items = 0;
            s.series_cats = 0;
            s.series_items = 0;
        });
        self.refresh();
        self.sync();

        // The dialog's owner (window) receives the finished catalog even
        // if the settings dialog got closed meanwhile — weak refs keep
        // this safe: window upgrade wins, snapshot lands.
        let kind = source.kind.clone();
        let ctx = self.clone();
        let on_progress = move |progress: &crate::ingest::Progress| {
            ctx.model.update(id, |s| {
                s.live_cats = progress.live_cats as u32;
                s.live_items = progress.live_items as u32;
                s.vod_cats = progress.vod_cats as u32;
                s.vod_items = progress.vod_items as u32;
                s.series_cats = progress.series_cats as u32;
                s.series_items = progress.series_items as u32;
            });
            ctx.refresh();
        };
        let ctx = self.clone();
        let win_weak = self.win.clone();
        crate::ingest::start(&kind, on_progress, move |outcome| match outcome {
            crate::ingest::IngestOutcome::Done(catalog) => {
                tracing::info!(
                    target: "rtv::ingest",
                    id,
                    "ingest DONE — publishing snapshot to window"
                );
                let counts = SectionCounts::of(&catalog);
                let snapshot = std::rc::Rc::new(catalog.clone());
                ctx.model.update(id, |s| {
                    s.state = SourceState::Ready;
                    s.snapshot = Some(snapshot.clone());
                    s.live_cats = counts.live_cats;
                    s.live_items = counts.live_items;
                    s.vod_cats = counts.vod_cats;
                    s.vod_items = counts.vod_items;
                    s.series_cats = counts.series_cats;
                    s.series_items = counts.series_items;
                });
                if let Some(win) = win_weak.upgrade() {
                    win.apply_catalog(catalog);
                    win.sync_sources_ui();
                }
                ctx.refresh();
                if let Some(updated) = ctx.model.get(id) {
                    ctx.toast(&format!(
                        "Source ready — {} categories, {} contents",
                        fmt_count(updated.total_cats()),
                        fmt_count(updated.total_items())
                    ));
                }
            }
            crate::ingest::IngestOutcome::Failed(message) => {
                tracing::error!(target: "rtv::ingest", id, reason = %message, "ingest FAILED");
                ctx.model.update(id, |s| {
                    s.state = SourceState::Failed;
                    s.failure = Some(message.clone());
                    s.snapshot = None;
                });
                ctx.refresh();
                ctx.toast(&format!("Source failed: {message}"));
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
                    let id = save_ctx.model.add(|id| Source {
                        id,
                        name: name.clone(),
                        kind: kind.clone(),
                        state: SourceState::Indexing,
                        failure: None,
                        live_cats: 0,
                        live_items: 0,
                        vod_cats: 0,
                        vod_items: 0,
                        series_cats: 0,
                        series_items: 0,
                        section_hidden: [false; 3],
                        snapshot: None,
                    });
                    save_ctx.toast("Source added — loading…");
                    save_ctx.start_ingest(id);
                }
            }
            save_ctx.sync();
            if let Some(nav) = nav_weak.upgrade() {
                nav.pop();
            }
        });

        nav.push(&nav_page);
    }

    /// Per-source subpage: stats overview + category visibility.
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

    /// Re-renders the open detail page after simulated indexing finished.
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

        if source.is_indexing() {
            let row = adw::ActionRow::builder()
                .title("Loading…")
                .subtitle(format!(
                    "{} categories · {} contents fetched so far. The page updates when the load finishes.",
                    fmt_count(source.total_cats()),
                    fmt_count(source.total_items())
                ))
                .activatable(false)
                .build();
            row.add_prefix(&gtk::Spinner::builder().spinning(true).build());
            overview.add(&row);
        } else if source.state == SourceState::Failed {
            let row = adw::ActionRow::builder()
                .title("Loading failed")
                .subtitle(
                    source
                        .failure
                        .as_deref()
                        .unwrap_or("The source could not be loaded."),
                )
                .activatable(false)
                .build();
            row.add_prefix(
                &gtk::Image::builder()
                    .icon_name("dialog-warning-symbolic")
                    .build(),
            );
            overview.add(&row);
        } else {
            for section in Section::all() {
                let (cats, items) = source.section(section);
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

        // Category visibility groups. Bulk selection lives in the group
        // header: All / None buttons per section, so a provider with
        // hundreds of categories stays manageable.
        if source.state == SourceState::Ready {
            let catalog = self.win.upgrade().map(|w| w.catalog_rc());
            for section in Section::all() {
                let Some(catalog) = &catalog else { break };
                let cats: Vec<&crate::catalog::Category> = catalog
                    .categories
                    .iter()
                    .filter(|c| c.kind == section.kind())
                    .collect();
                if cats.is_empty() {
                    continue;
                }
                let visible_count = cats.iter().filter(|c| c.visible).count();

                let group = adw::PreferencesGroup::builder()
                    .title(format!("{} categories", section.label()))
                    .description(format!(
                        "{visible_count} of {} visible. Hidden categories are skipped by browse and search.",
                        cats.len()
                    ))
                    .build();

                // Bulk-select header row: All / None.
                let bulk_row = adw::ActionRow::builder()
                    .title("Show all categories")
                    .activatable(false)
                    .build();
                let all_btn = gtk::Button::builder()
                    .label("All")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                let none_btn = gtk::Button::builder()
                    .label("None")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                bulk_row.add_suffix(&all_btn);
                bulk_row.add_suffix(&none_btn);
                group.add(&bulk_row);
                let bulk_ctx = self.clone();
                let section_all = section;
                all_btn.connect_clicked(move |_| {
                    tracing::info!(target: "rtv::ui", section = ?section_all, "bulk: show all");
                    bulk_ctx.set_section_visible_bulk(section_all, true);
                });
                let none_ctx = self.clone();
                none_btn.connect_clicked(move |_| {
                    tracing::info!(target: "rtv::ui", section = ?section_all, "bulk: hide all");
                    none_ctx.set_section_visible_bulk(section_all, false);
                });

                for cat in cats {
                    // Panel category names can carry a bare "&"; row
                    // titles parse as Pango markup unless use_markup is
                    // off (the PR #20 lesson — keep it everywhere).
                    let switch_row = adw::SwitchRow::builder()
                        .title(&cat.name)
                        .use_markup(false)
                        .active(cat.visible)
                        .build();
                    let win_weak = self.win.clone();
                    let kind = section.kind();
                    let cat_id = cat.id.clone();
                    switch_row.connect_active_notify(move |row| {
                        if let Some(win) = win_weak.upgrade() {
                            win.set_category_visible(kind, &cat_id, row.is_active());
                        }
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

    /// Bulk All/None for one section: delegates to the window, which
    /// patches the snapshot AND re-renders everything. Then rebuilds
    /// this detail page so switches reflect the new state.
    fn set_section_visible_bulk(&self, section: Section, visible: bool) {
        let Some(source) = self.detail.open_id.get() else {
            return;
        };
        let id = source;
        self.model.set_section_visible(id, section, visible);
        if let Some(win) = self.win.upgrade() {
            win.set_section_visible(section.kind(), visible);
        }
        self.rebuild_detail(id);
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
            // Explicit minimum size: Adwaita warns loudly ("AdwDialog does
            // not have a minimum size") when only content_width/height are
            // set — width-request/height-request silence that.
            .width_request(360)
            .height_request(360)
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

    /// Rebuilds the rows without presenting (used by the reset action).
    pub fn refresh(&self) {
        self.ctx.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xtream_source() -> SourceKind {
        SourceKind::Xtream {
            server: "http://panel.example:80".into(),
            username: "u".into(),
            password: "p".into(),
        }
    }

    fn m3u_source() -> SourceKind {
        SourceKind::M3U {
            url: "http://example.com/list.m3u".into(),
            epg: String::new(),
        }
    }

    #[test]
    fn fmt_count_inserts_thousands_separators() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1,000");
        assert_eq!(fmt_count(24_300), "24,300");
        assert_eq!(fmt_count(1_234_567), "1,234,567");
    }

    #[test]
    fn model_add_update_remove_roundtrip() {
        let model = SourcesModel::default();
        let id = model.add(|id| Source {
            id,
            name: "A".into(),
            kind: m3u_source(),
            state: SourceState::Indexing,
            failure: None,
            live_cats: 0,
            live_items: 0,
            vod_cats: 0,
            vod_items: 0,
            series_cats: 0,
            series_items: 0,
            section_hidden: [false; 3],
            snapshot: None,
        });
        let added = model.get(id).expect("added");
        assert_eq!(added.name, "A");
        assert!(added.is_indexing());

        model.update(id, |s| {
            s.state = SourceState::Ready;
            s.live_items = 42;
        });
        assert_eq!(model.get(id).expect("ready").live_items, 42);

        model.remove(id);
        assert!(model.get(id).is_none());
        assert!(model.all().is_empty());
    }

    #[test]
    fn section_visible_bulk_flips_one_section_only() {
        let model = SourcesModel::default();
        let id = model.add(|id| Source {
            id,
            name: "X".into(),
            kind: xtream_source(),
            state: SourceState::Ready,
            failure: None,
            live_cats: 3,
            live_items: 10,
            vod_cats: 2,
            vod_items: 5,
            series_cats: 1,
            series_items: 1,
            section_hidden: [false; 3],
            snapshot: None,
        });
        model.set_section_visible(id, Section::Live, false);
        let source = model.get(id).expect("source");
        assert!(source.section_hidden[0]);
        assert!(!source.section_hidden[1]);
        assert!(!source.section_hidden[2]);
        model.set_section_visible(id, Section::Live, true);
        assert!(!model.get(id).expect("source").section_hidden[0]);
    }

    #[test]
    fn subtitles_cover_all_states() {
        let mut source = Source {
            id: 1,
            name: "N".into(),
            kind: xtream_source(),
            state: SourceState::Indexing,
            failure: None,
            live_cats: 1,
            live_items: 2,
            vod_cats: 0,
            vod_items: 0,
            series_cats: 0,
            series_items: 0,
            section_hidden: [false; 3],
            snapshot: None,
        };
        assert!(source_subtitle(&source).contains("Loading…"));

        source.state = SourceState::Ready;
        let subtitle = source_subtitle(&source);
        assert!(subtitle.starts_with("Xtream Codes"));
        assert!(subtitle.contains("categories"));

        source.state = SourceState::Failed;
        source.failure = Some("boom".into());
        assert!(source_subtitle(&source).contains("Failed — boom"));
    }
}
