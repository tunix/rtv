use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

use gstreamer::prelude::*;

use crate::details::{self, Episode, SeriesInfo};
use crate::sample::{self, Item, Kind, SeriesMeta};

/// Search results are capped so a one-letter query cannot flood the grid;
/// the backend phase will replace this with real incremental search.
const SEARCH_CAP: usize = 500;
/// Cards appended to a grid per idle tick (progressive rendering).
const FILL_CHUNK: usize = 40;

/// Identifies one favorite: kind + catalog id, so same-named content in
/// different sections never collides. The name rides along for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavKey {
    pub kind: Kind,
    pub id: i64,
    pub name: String,
}

/// What the series detail and episodes pages need to render the shared
/// header, kept together so both pages can show the same top section.
#[derive(Debug, Clone)]
pub struct SeriesCtx {
    pub id: i64,
    pub name: String,
    pub cover: Option<String>,
    pub meta: SeriesMeta,
}

/// Sidebar section header row: icon + bold label, click toggles collapse.
#[derive(Debug)]
pub struct HeaderRow {
    row: gtk::ListBoxRow,
    /// Bold count label ("Live TV (5)"), refreshed on every search change.
    label: gtk::Label,
    kind: Kind,
}

/// Sidebar category row: activating it shows the category's content grid.
#[derive(Debug)]
pub struct CategoryRow {
    row: gtk::ListBoxRow,
    category_index: usize,
    group: i32,
    /// Count label ("News (7)"), refreshed on every search change.
    label: gtk::Label,
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

mod imp {
    use adw::subclass::prelude::*;
    use gtk::{glib, TemplateChild};
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    use crate::sample::Catalog;

    use super::{CategoryRow, FavKey, HeaderRow};

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/tunix/rtv/window.ui")]
    pub struct Window {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub main_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub welcome_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub welcome_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub browse_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub player_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub dropdown_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub clear_favorites: TemplateChild<gtk::Button>,
        #[template_child]
        pub content_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub player_headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub player_heart: TemplateChild<gtk::Button>,
        #[template_child]
        pub player_content: TemplateChild<gtk::Box>,
        #[template_child]
        pub player_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub app_watermark: TemplateChild<gtk::Image>,
        #[template_child]
        pub video_surface: TemplateChild<gtk::Picture>,
        #[template_child]
        pub controls_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub play_button_small: TemplateChild<gtk::Button>,
        #[template_child]
        pub seek_back: TemplateChild<gtk::Button>,
        #[template_child]
        pub seek_forward: TemplateChild<gtk::Button>,
        #[template_child]
        pub mute_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub volume_scale: TemplateChild<gtk::Scale>,
        #[template_child]
        pub fullscreen_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub content_info_row: TemplateChild<gtk::Box>,
        #[template_child]
        pub live_badge: TemplateChild<gtk::Label>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub content_grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub favorites_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub search_grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub favorites_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub content_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub content_status: TemplateChild<gtk::Label>,
        #[template_child]
        pub detail_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub detail_heart: TemplateChild<gtk::Button>,
        #[template_child]
        pub detail_content: TemplateChild<gtk::Box>,
        #[template_child]
        pub episodes_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub episodes_content: TemplateChild<gtk::Box>,

        /// Sample catalog loaded from samples/ (mock phase).
        pub catalog: OnceLock<Catalog>,
        /// UI-mock state.
        pub playing: Cell<bool>,
        pub muted: Cell<bool>,
        pub has_source: Cell<bool>,
        pub on_player: Cell<bool>,
        /// Whether the current playback target is live TV (drives the
        /// LIVE badge).
        pub is_live: Cell<bool>,
        /// Active GStreamer pipeline for real playback (trailers); None
        /// means the demo player is on screen.
        pub pipeline: RefCell<Option<gstreamer::Element>>,
        pub selected_category: Cell<i32>,
        pub last_pointer: Cell<(f64, f64)>,
        pub favorites: RefCell<Vec<FavKey>>,
        /// What the player page is showing, so the heart button can
        /// favorite the CHANNEL / MOVIE / SERIES — never an episode.
        pub current: RefCell<Option<FavKey>>,
        /// Status line for the playback screen, restored on play/pause.
        pub current_status: RefCell<String>,
        /// The series/movie currently open on the detail page, for the
        /// headerbar heart.
        pub detail_target: RefCell<Option<FavKey>>,
        pub collapsed: RefCell<Vec<bool>>,
        pub query: RefCell<String>,
        pub categories: RefCell<Vec<CategoryRow>>,
        pub headers: RefCell<Vec<HeaderRow>>,
        /// Pinned "Favorites" sidebar row's bold count label
        /// ("Favorites (3)"), refreshed by rebuild_favorites.
        pub favorites_label: RefCell<Option<gtk::Label>>,
        /// Its row index, used to dispatch row activation before the
        /// category lookup.
        pub favorites_row_index: Cell<i32>,
        pub dropdown: RefCell<Option<gtk::DropDown>>,
        /// Bumped on every show/cancel; only the timer matching the latest
        /// generation may hide the controls (stale timers must no-op, since
        /// `SourceId::remove` panics on already-fired ids).
        pub controls_generation: Cell<u64>,
        /// Bumped on every grid (re)fill; stale chunk fillers no-op.
        pub fill_generation: Cell<u64>,
        /// Bumped on every open_details; stale async detail fillers no-op
        /// instead of touching a page that already shows other content.
        pub details_generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "RtvWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {}
    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl adw::subclass::application_window::AdwApplicationWindowImpl for Window {}
}

impl Window {
    pub fn new(app: &adw::Application) -> Self {
        let window: Window = glib::Object::builder()
            .property("application", app)
            .property("icon-name", "io.github.tunix.rtv")
            .build();

        let _ = window.imp().catalog.set(sample::Catalog::load());
        // Alt-tab shows the toplevel title; with a header bar title
        // widget GTK would otherwise fall back to the app id.
        window.set_title(Some("RTV"));
        window.setup_actions();
        window.setup_browse();
        window.imp().selected_category.set(-1);
        // Live expanded, Movies/Series collapsed on first open.
        {
            let mut collapsed = window.imp().collapsed.borrow_mut();
            collapsed.clear();
            collapsed.extend(Kind::SIDEBAR.iter().enumerate().map(|(i, _)| i != 0));
        }
        window.refresh_rows_visibility();
        // Template children of the NavigationView (detail/episodes/player
        // pages) end up on the stack while the builder constructs the widget
        // tree, so the window would otherwise open on an empty page. Reset
        // the stack to the welcome page before anything is shown.
        window
            .imp()
            .main_view
            .pop_to_page(&*window.imp().welcome_page);
        window
    }

    pub fn sample_catalog_missing(&self) -> bool {
        !self.imp().catalog.get().map(|c| c.found).unwrap_or(false)
    }

    fn imp(&self) -> &imp::Window {
        glib::subclass::types::ObjectSubclassIsExt::imp(self)
    }

    pub fn toast(&self, msg: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(msg));
    }

    fn setup_actions(&self) {
        // First run: adding a source unlocks browsing. In --mock mode the
        // mock catalog is attached at once; in real mode the source-setup
        // dialog (backend work) has not landed yet, so the user stays on
        // the welcome screen with an explanatory toast.
        let win = self.downgrade();
        let add_source = gio::SimpleAction::new("add-source", None);
        add_source.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            let imp = win.imp();
            if imp.has_source.get() {
                win.toast("Source dialog comes later — the mock already has 2 sources");
                return;
            }
            if crate::mock::enabled() {
                imp.has_source.set(true);
                imp.main_view.push(&*imp.browse_page);
                win.toast("Mock source added — the real setup dialog comes later");
            } else {
                win.toast(
                    "Real source setup is not implemented yet — run `rtv --mock` to explore the UI with sample data",
                );
            }
        });
        self.add_action(&add_source);

        // Mock affordance: go back to the locked first-run state.
        let win = self.downgrade();
        let reset = gio::SimpleAction::new("reset-mock", None);
        reset.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            let imp = win.imp();
            imp.has_source.set(false);
            imp.favorites.borrow_mut().clear();
            *imp.current.borrow_mut() = None;
            win.stop_pipeline();
            win.rebuild_favorites();
            imp.main_view.pop_to_page(&*imp.welcome_page);
        });
        self.add_action(&reset);

        // Classic media-player shortcuts (accelerators registered in app.rs).
        // They only act while the player page is on screen. Play/pause
        // lives on the player page itself (below): a global `space`
        // accelerator would swallow the space key while typing in the
        // search entry, so it must stay scoped to the player.
        let win = self.downgrade();
        let play_pause = gio::SimpleAction::new("play-pause", None);
        play_pause.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.toggle_playing();
            }
        });
        self.add_action(&play_pause);

        let win = self.downgrade();
        let seek_back = gio::SimpleAction::new("seek-back", None);
        seek_back.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.seek_relative(-10);
            }
        });
        self.add_action(&seek_back);

        let win = self.downgrade();
        let seek_forward = gio::SimpleAction::new("seek-forward", None);
        seek_forward.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.seek_relative(10);
            }
        });
        self.add_action(&seek_forward);

        let win = self.downgrade();
        let mute = gio::SimpleAction::new("toggle-mute", None);
        mute.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.toggle_mute();
            }
        });
        self.add_action(&mute);

        let win = self.downgrade();
        let fullscreen = gio::SimpleAction::new("toggle-fullscreen", None);
        fullscreen.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.toggle_fullscreen();
            }
        });
        self.add_action(&fullscreen);

        let win = self.downgrade();
        let exit_fullscreen = gio::SimpleAction::new("exit-fullscreen", None);
        exit_fullscreen.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.is_fullscreen() {
                win.unfullscreen();
            } else if !win.imp().query.borrow().trim().is_empty() {
                // Outside fullscreen, Escape resets the search instead.
                win.imp().search_entry.set_text("");
            }
        });
        self.add_action(&exit_fullscreen);

        let win2 = self.downgrade();
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(move |_, _| {
            if let Some(win) = win2.upgrade() {
                let dialog = adw::AboutDialog::new();
                dialog.set_application_name("RTV");
                dialog.set_application_icon("io.github.tunix.rtv");
                dialog.set_version(env!("CARGO_PKG_VERSION"));
                dialog.set_website("https://github.com/tunix/rtv");
                dialog.set_developer_name("Alper Kanat");
                dialog.set_comments("IPTV player for the GNOME desktop");
                dialog.set_license_type(gtk::License::Gpl30Only);
                dialog.present(Some(&win));
            }
        });
        self.add_action(&about);
    }

    fn setup_browse(&self) {
        let imp = self.imp();

        // Source picker, valhalla-style, in the header title area. Switching
        // is silent for now; real per-source loading arrives with the
        // source/back-end work.
        let model = gtk::StringList::new(&["Xtream — Home Panel", "M3U — Trial List"]);
        let dropdown = gtk::DropDown::new(Some(model), None::<gtk::Expression>);
        dropdown.set_selected(0);
        imp.dropdown_box.append(&dropdown);
        imp.dropdown.replace(Some(dropdown));

        // Pinned Favorites row at the very top of the sidebar: always
        // visible, unaffected by section collapse and search filtering.
        // Formatted like the main-category headers: icon + bold label.
        let favorites_row = gtk::ListBoxRow::new();
        let favorites_box = gtk::Box::builder().spacing(6).build();
        favorites_box.append(
            // Bundled icon: the theme's emblem-favorite-symbolic is missing
            // in current Adwaita, and the colored fallback renders RED.
            &gtk::Image::builder()
                .icon_name("rtv-heart-symbolic")
                .pixel_size(14)
                .valign(gtk::Align::Center)
                .build(),
        );
        let favorites_label = gtk::Label::builder()
            .label("Favorites (0)")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build();
        favorites_box.append(&favorites_label);
        favorites_row.set_child(Some(&favorites_box));
        imp.content_list.prepend(&favorites_row);
        imp.favorites_label.replace(Some(favorites_label));
        imp.favorites_row_index.set(favorites_row.index());

        self.setup_sidebar();

        // Sidebar activation: the Favorites row first, then categories.
        let win = self.downgrade();
        imp.content_list.connect_row_activated(move |_, row| {
            let Some(win) = win.upgrade() else { return };
            let i = win.imp();
            if row.index() == i.favorites_row_index.get() {
                win.show_favorites();
                return;
            }
            let index = row.index();
            let target = i
                .categories
                .borrow()
                .iter()
                .find(|c| c.row.index() == index)
                .map(|c| c.category_index);
            if let Some(cat_idx) = target {
                win.select_category(cat_idx);
            }
        });

        // "Clear all" empties the favorites at once and syncs the heart
        // buttons on the player/detail pages, which are the only places
        // that can still show a now-removed favorite.
        let win = self.downgrade();
        imp.clear_favorites.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                let i = win.imp();
                if i.favorites.borrow().is_empty() {
                    return;
                }
                i.favorites.borrow_mut().clear();
                if i.current.borrow().is_some() {
                    set_heart_active(&i.player_heart, false);
                }
                if i.detail_target.borrow().is_some() {
                    set_heart_active(&i.detail_heart, false);
                }
                win.rebuild_favorites();
            }
        });

        // Grid cards route by content kind, exactly like the favorites grid.
        let win = self.downgrade();
        imp.content_grid.connect_child_activated(move |_, child| {
            if let Some(win) = win.upgrade() {
                win.route_card(child);
            }
        });
        let win = self.downgrade();
        imp.search_grid.connect_child_activated(move |_, child| {
            if let Some(win) = win.upgrade() {
                win.route_card(child);
            }
        });

        // Search across the whole catalog.
        let win = self.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(win) = win.upgrade() {
                win.apply_search(&entry.text());
            }
        });
        // Escape inside the search field resets the search.
        imp.search_entry.connect_stop_search(move |entry| {
            entry.set_text("");
        });

        // When the player or the episodes page pops off, their state is
        // gone; the player tears down any real pipeline, the episodes
        // page drops its (cover-fetching) children.
        let win = self.downgrade();
        imp.main_view.connect_popped(move |_, popped_page| {
            let Some(win) = win.upgrade() else { return };
            let i = win.imp();
            if popped_page == &*i.player_page {
                i.on_player.set(false);
                win.cancel_hide();
                win.stop_pipeline();
            }
            if popped_page == &*i.episodes_page {
                while let Some(child) = i.episodes_content.first_child() {
                    i.episodes_content.remove(&child);
                }
            }
        });

        // Player control bar.
        let win = self.downgrade();
        imp.play_button_small.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_playing();
            }
        });
        let win = self.downgrade();
        imp.seek_back.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.seek_relative(-10);
            }
        });
        let win = self.downgrade();
        imp.seek_forward.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.seek_relative(10);
            }
        });
        let win = self.downgrade();
        imp.mute_button.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_mute();
            }
        });
        // Real pipelines take the volume directly from the slider.
        let win = self.downgrade();
        imp.volume_scale.connect_value_changed(move |scale| {
            if let Some(win) = win.upgrade() {
                if let Some(pipeline) = win.imp().pipeline.borrow().as_ref() {
                    pipeline.set_property("volume", scale.value());
                }
            }
        });
        let win = self.downgrade();
        imp.fullscreen_button.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_fullscreen();
            }
        });
        let win = self.downgrade();
        imp.player_heart.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                // The heart favorites the CHANNEL / MOVIE / SERIES being
                // played — never an individual episode.
                let target = win.imp().current.borrow().clone();
                if let Some(t) = target {
                    win.toggle_fav(t.kind, t.id, &t.name);
                }
            }
        });

        // The detail-page heart favorites the open series/movie.
        let win = self.downgrade();
        imp.detail_heart.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                let target = win.imp().detail_target.borrow().clone();
                if let Some(t) = target {
                    win.toggle_fav(t.kind, t.id, &t.name);
                }
            }
        });

        // Play/pause is scoped to the player page so the space key keeps
        // reaching the search entry everywhere else.
        let play_shortcut = gtk::Shortcut::new(
            Some(gtk::KeyvalTrigger::new(
                gtk::gdk::Key::space,
                gtk::gdk::ModifierType::empty(),
            )),
            Some(gtk::NamedAction::new("win.play-pause")),
        );
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.add_shortcut(play_shortcut);
        imp.player_content.add_controller(shortcuts);

        // Mouse movement over the player reveals auto-hidden controls.
        // Real movement only: the sub-pixel "motion" events GTK emits when
        // overlay widgets reallocate under a stationary pointer must not
        // re-reveal the controls (they caused a show/hide flap).
        let motion = gtk::EventControllerMotion::new();
        let win = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(win) = win.upgrade() {
                let i = win.imp();
                let (lx, ly) = i.last_pointer.get();
                let moved = (x - lx).abs() > 2.0 || (y - ly).abs() > 2.0;
                i.last_pointer.set((x, y));
                if moved && i.playing.get() && i.on_player.get() {
                    win.show_controls();
                }
            }
        });
        let win = self.downgrade();
        motion.connect_enter(move |_, x, y| {
            if let Some(win) = win.upgrade() {
                win.imp().last_pointer.set((x, y));
            }
        });
        imp.player_content.add_controller(motion);

        // The fullscreen icon and chrome visibility follow the real window
        // state. Fullscreen means PURE video: no headerbar, no info row, no
        // LIVE badge, no margins or rounded corners — chrome comes back
        // together with the controls on mouse movement.
        self.connect_fullscreened_notify(|win| {
            let i = win.imp();
            let fullscreen = win.is_fullscreen();
            i.fullscreen_button.set_icon_name(if fullscreen {
                "view-restore-symbolic"
            } else {
                "view-fullscreen-symbolic"
            });
            i.player_headerbar.set_visible(!fullscreen);
            i.content_info_row.set_visible(!fullscreen);
            i.live_badge.set_visible(!fullscreen && i.is_live.get());
            let margin = if fullscreen { 0 } else { 12 };
            i.player_content.set_margin_start(margin);
            i.player_content.set_margin_end(margin);
            i.player_content.set_margin_bottom(margin);
            i.player_content
                .set_spacing(if fullscreen { 0 } else { 12 });
            if fullscreen {
                i.player_box.add_css_class("video-fill");
            } else {
                i.player_box.remove_css_class("video-fill");
            }
            if fullscreen {
                win.show_controls();
            }
        });

        self.rebuild_favorites();
    }

    /// Build the sidebar dynamically from the sample catalog: one collapsible
    /// section per content kind (Live TV, Movies, Series), its categories
    /// underneath. Headers carry the sub-category count, category rows the
    /// item count; `refresh_rows_visibility` keeps both in sync with the
    /// active search query.
    fn setup_sidebar(&self) {
        let imp = self.imp();
        let Some(catalog) = imp.catalog.get() else {
            return;
        };

        for (group, kind) in Kind::SIDEBAR.iter().enumerate() {
            let group = group as i32;
            let cats: Vec<(usize, &sample::Category)> = catalog
                .categories
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == *kind)
                .collect();

            let header_label = gtk::Label::builder()
                .label(format!("{} ({})", kind.title(), cats.len()))
                .xalign(0.0)
                .css_classes(["heading"])
                .build();
            let header = gtk::ListBoxRow::builder()
                .activatable(false)
                .selectable(false)
                .build();
            let header_box = gtk::Box::builder().spacing(6).build();
            header_box.append(
                &gtk::Image::builder()
                    .icon_name(kind.icon())
                    .pixel_size(14)
                    .valign(gtk::Align::Center)
                    .build(),
            );
            header_box.append(&header_label);
            header.set_child(Some(&header_box));
            let win = self.downgrade();
            let gesture = gtk::GestureClick::new();
            gesture.connect_pressed(move |_, _, _, _| {
                if let Some(win) = win.upgrade() {
                    win.toggle_group(group);
                }
            });
            header.add_controller(gesture);
            imp.content_list.append(&header);
            imp.headers.borrow_mut().push(HeaderRow {
                row: header,
                label: header_label,
                kind: *kind,
            });

            for (cat_index, cat) in cats {
                let count = catalog
                    .items
                    .iter()
                    .filter(|it| it.kind == *kind && it.category_id == cat.id)
                    .count();
                let label = gtk::Label::builder()
                    .label(format!("{} ({})", cat.name, count))
                    .halign(gtk::Align::Start)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let row = gtk::ListBoxRow::new();
                // Icon (14px) + spacing (6px) put the header label at 20px;
                // category rows align their text with it.
                let row_box = gtk::Box::builder().margin_start(20).build();
                row_box.append(&label);
                row.set_child(Some(&row_box));
                imp.content_list.append(&row);
                imp.categories.borrow_mut().push(CategoryRow {
                    row,
                    category_index: cat_index,
                    group,
                    label,
                });
            }
        }
    }

    fn toggle_group(&self, group: i32) {
        let imp = self.imp();
        {
            let mut collapsed = imp.collapsed.borrow_mut();
            while collapsed.len() <= group as usize {
                collapsed.push(false);
            }
            collapsed[group as usize] = !collapsed[group as usize];
        }
        self.refresh_rows_visibility();
    }

    fn collapsed(&self, group: i32) -> bool {
        let imp = self.imp();
        let c = imp.collapsed.borrow();
        match c.get(group as usize) {
            Some(&v) => v,
            None => false,
        }
    }

    fn apply_search(&self, text: &str) {
        let imp = self.imp();
        *imp.query.borrow_mut() = text.to_string();
        self.refresh_rows_visibility();

        let q = text.trim().to_lowercase();
        if q.is_empty() {
            if imp.selected_category.get() >= 0 {
                // The category grid may still hold query-filtered cards —
                // refill it unfiltered.
                let selected = imp.selected_category.get() as usize;
                self.select_category(selected);
            } else {
                imp.content_stack.set_visible_child_name("empty");
            }
            return;
        }

        imp.fill_generation.set(imp.fill_generation.get() + 1);
        while let Some(child) = imp.search_grid.first_child() {
            imp.search_grid.remove(&child);
        }
        let catalog = imp.catalog.get().expect("catalog");
        // Search filters the CURRENT category's content when one is
        // selected; with no category it spans the whole catalog (cards
        // then carry their category label).
        let selected = imp.selected_category.get();
        let scope = |it: &Item| {
            if selected >= 0 {
                match catalog.categories.get(selected as usize) {
                    Some(cat) => it.kind == cat.kind && it.category_id == cat.id,
                    None => true,
                }
            } else {
                true
            }
        };
        let indices: Vec<usize> = catalog
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| scope(it) && it.name.to_lowercase().contains(&q))
            .take(SEARCH_CAP)
            .map(|(i, _)| i)
            .collect();
        imp.content_stack.set_visible_child_name("search");
        self.fill_grid(indices, &imp.search_grid, selected < 0);
    }

    /// Visibility of every sidebar row, driven by the search query and the
    /// per-section collapse state — plus the count labels. Search filters
    /// CONTENT (the middle grid), never the sidebar: every row stays
    /// visible, but the counts drop to the number of matching items
    /// (0 when a category has no match).
    fn refresh_rows_visibility(&self) {
        let imp = self.imp();
        let q = imp.query.borrow().trim().to_lowercase();

        if let Some(catalog) = imp.catalog.get() {
            // Matching item count per (kind, category id) in one pass.
            let mut counts: std::collections::HashMap<(Kind, &str), usize> =
                std::collections::HashMap::new();
            for it in &catalog.items {
                if q.is_empty() || it.name.to_lowercase().contains(&q) {
                    *counts
                        .entry((it.kind, it.category_id.as_str()))
                        .or_default() += 1;
                }
            }

            for c in imp.categories.borrow().iter() {
                let count = catalog
                    .categories
                    .get(c.category_index)
                    .and_then(|cat| counts.get(&(cat.kind, cat.id.as_str())))
                    .copied()
                    .unwrap_or(0);
                if let Some(cat) = catalog.categories.get(c.category_index) {
                    c.label.set_text(&format!("{} ({count})", cat.name));
                }
            }
            // Headers normally show the sub-category count; while
            // searching, the count of sub-categories that have matches.
            for h in imp.headers.borrow().iter() {
                let count = if q.is_empty() {
                    catalog
                        .categories
                        .iter()
                        .filter(|c| c.kind == h.kind)
                        .count()
                } else {
                    catalog
                        .categories
                        .iter()
                        .filter(|c| {
                            c.kind == h.kind
                                && counts.get(&(c.kind, c.id.as_str())).copied().unwrap_or(0) > 0
                        })
                        .count()
                };
                h.label.set_text(&format!("{} ({count})", h.kind.title()));
            }
        }

        if q.is_empty() {
            for c in imp.categories.borrow().iter() {
                c.row.set_visible(!self.collapsed(c.group));
            }
            for h in imp.headers.borrow().iter() {
                h.row.set_visible(true);
            }
        } else {
            for c in imp.categories.borrow().iter() {
                c.row.set_visible(true);
            }
            for h in imp.headers.borrow().iter() {
                h.row.set_visible(true);
            }
        }

        // The pinned Favorites row is not touched: it stays visible in
        // every collapse and search state.
    }

    /// Show a category's content as a card grid in the middle pane. An
    /// active search query keeps filtering within the newly selected
    /// category.
    fn select_category(&self, category_index: usize) {
        let imp = self.imp();
        imp.selected_category.set(category_index as i32);

        imp.fill_generation.set(imp.fill_generation.get() + 1);
        while let Some(child) = imp.content_grid.first_child() {
            imp.content_grid.remove(&child);
        }
        let catalog = imp.catalog.get().expect("catalog");
        let Some(cat) = catalog.categories.get(category_index) else {
            return;
        };
        let q = imp.query.borrow().trim().to_lowercase();
        let indices: Vec<usize> = catalog
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.kind == cat.kind
                    && it.category_id == cat.id
                    && (q.is_empty() || it.name.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        imp.content_stack.set_visible_child_name("grid");
        self.fill_grid(indices, &imp.content_grid, false);
    }

    /// Fill a grid progressively: FILL_CHUNK cards per idle tick so a
    /// 1800-card category never blocks the UI. A generation counter makes
    /// stale fillers (category switch, new search) stop immediately.
    fn fill_grid(&self, mut indices: Vec<usize>, flowbox: &gtk::FlowBox, with_categories: bool) {
        let imp = self.imp();
        imp.fill_generation.set(imp.fill_generation.get() + 1);
        let generation = imp.fill_generation.get();
        indices.reverse(); // pop() from the end preserves order
        let remaining = std::rc::Rc::new(RefCell::new(indices));
        let flowbox = flowbox.clone();
        let win = self.downgrade();
        glib::idle_add_local(move || {
            let Some(window) = win.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let i = window.imp();
            if i.fill_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let mut rem = remaining.borrow_mut();
            for _ in 0..FILL_CHUNK {
                let Some(idx) = rem.pop() else {
                    return glib::ControlFlow::Break;
                };
                let Some(item) = i.catalog.get().and_then(|c| c.items.get(idx)) else {
                    continue;
                };
                let category_label = with_categories
                    .then(|| {
                        i.catalog
                            .get()
                            .and_then(|c| {
                                c.categories
                                    .iter()
                                    .find(|cat| cat.kind == item.kind && cat.id == item.category_id)
                            })
                            .map(|cat| cat.name.clone())
                    })
                    .flatten();
                flowbox.append(&content_card(item, category_label.as_deref()));
            }
            if rem.is_empty() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Add or remove one favorite, identified by kind + id so same-named
    /// content across sections never collides. Syncs the player and detail
    /// page hearts and the whole favorites UI.
    fn toggle_fav(&self, kind: Kind, id: i64, name: &str) -> bool {
        let imp = self.imp();
        let is_fav = {
            let mut favs = imp.favorites.borrow_mut();
            if let Some(pos) = favs.iter().position(|f| f.kind == kind && f.id == id) {
                favs.remove(pos);
                false
            } else {
                favs.push(FavKey {
                    kind,
                    id,
                    name: name.to_string(),
                });
                true
            }
        };
        self.rebuild_favorites();
        if let Some(cur) = imp.current.borrow().as_ref() {
            if cur.kind == kind && cur.id == id {
                set_heart_active(&imp.player_heart, is_fav);
            }
        }
        if let Some(target) = imp.detail_target.borrow().as_ref() {
            if target.kind == kind && target.id == id {
                set_heart_active(&imp.detail_heart, is_fav);
            }
        }
        is_fav
    }

    fn is_fav(&self, kind: Kind, id: i64) -> bool {
        self.imp()
            .favorites
            .borrow()
            .iter()
            .any(|f| f.kind == kind && f.id == id)
    }

    /// Switch the middle pane to the favorites view.
    fn show_favorites(&self) {
        self.imp().content_stack.set_visible_child_name("favorites");
    }

    /// Refresh everything favorites-related: the pinned sidebar row label,
    /// the favorites page (per-kind sections vs. empty status) and the
    /// "Clear all" sensitivity. Favorite counts are small, so the section
    /// grids refill synchronously instead of going through the chunk
    /// filler. Favorites are grouped by main category: one grid each for
    /// Live TV, Movies and Series, stacked vertically.
    fn rebuild_favorites(&self) {
        let imp = self.imp();
        let favs = imp.favorites.borrow().clone();
        let count = favs.len();

        if let Some(label) = imp.favorites_label.borrow().as_ref() {
            label.set_text(&format!("Favorites ({count})"));
        }
        imp.clear_favorites.set_sensitive(count > 0);
        imp.favorites_stack
            .set_visible_child_name(if count == 0 { "empty" } else { "grid" });

        while let Some(child) = imp.favorites_box.first_child() {
            imp.favorites_box.remove(&child);
        }
        let win = self.downgrade();
        for kind in Kind::SIDEBAR {
            let section: Vec<&FavKey> = favs.iter().filter(|f| f.kind == kind).collect();
            if section.is_empty() {
                continue;
            }
            imp.favorites_box.append(
                &gtk::Label::builder()
                    .label(format!("{} ({})", kind.title(), section.len()))
                    .halign(gtk::Align::Start)
                    .css_classes(["title-4"])
                    .build(),
            );

            let grid = gtk::FlowBox::builder()
                .column_spacing(12)
                .row_spacing(12)
                .min_children_per_line(3)
                .max_children_per_line(5)
                .activate_on_single_click(true)
                .selection_mode(gtk::SelectionMode::None)
                .valign(gtk::Align::Start)
                .build();
            let win = win.clone();
            grid.connect_child_activated(move |_, child| {
                if let Some(win) = win.upgrade() {
                    win.route_card(child);
                }
            });
            for fav in section {
                // Items are looked up by kind + id; the name is only for
                // display, so a stale favorite still routes correctly.
                let icon = imp
                    .catalog
                    .get()
                    .and_then(|c| {
                        c.items
                            .iter()
                            .find(|it| it.kind == fav.kind && it.id == fav.id)
                    })
                    .and_then(|it| it.icon.clone());
                grid.append(&card_body(
                    &fav.name,
                    icon.as_deref(),
                    Some(fav.kind),
                    Some(fav.id),
                    None,
                ));
            }
            imp.favorites_box.append(&grid);
        }
    }

    /// Card activation routes by the content kind the card carries:
    /// Live -> playback, Series/Movies -> the detail page.
    fn route_card(&self, child: &gtk::FlowBoxChild) {
        let imp = self.imp();
        if let (Some(kind), Some(id)) = (card_kind(child), card_id(child)) {
            if let Some(item) = imp
                .catalog
                .get()
                .and_then(|c| c.items.iter().find(|it| it.kind == kind && it.id == id))
            {
                match kind {
                    Kind::Live => self.play(kind, id, &item.name),
                    Kind::Series | Kind::Vod => self.open_details(kind, id, &item.name),
                }
                return;
            }
        }
        // Fallback: cards without kind data resolve by their title.
        if let Some(name) = card_name(child) {
            if let Some(item) = imp
                .catalog
                .get()
                .and_then(|c| c.items.iter().find(|it| it.name == name))
            {
                match item.kind {
                    Kind::Live => self.play(item.kind, item.id, &item.name),
                    Kind::Series | Kind::Vod => self.open_details(item.kind, item.id, &item.name),
                }
            }
        }
    }

    fn play(&self, kind: Kind, id: i64, name: &str) {
        let status = match kind {
            Kind::Live => "Live · 1080p",
            Kind::Vod => "Movie · 1080p",
            Kind::Series => "Series",
        };
        self.play_with_status(kind, id, name, status);
    }

    /// Playing an episode keeps the HEART on the parent series: the
    /// favorite target is the series, the title only names the episode.
    fn play_episode(
        &self,
        series_id: i64,
        series_name: &str,
        season: i64,
        episode_number: i64,
        episode_title: &str,
    ) {
        self.play_with_status(
            Kind::Series,
            series_id,
            episode_title,
            &format!("{series_name} · Season {season} · Episode {episode_number}"),
        );
    }

    fn play_with_status(&self, kind: Kind, id: i64, name: &str, status: &str) {
        let imp = self.imp();
        *imp.current.borrow_mut() = Some(FavKey {
            kind,
            id,
            name: name.to_string(),
        });
        *imp.current_status.borrow_mut() = status.to_string();
        imp.player_page.set_title(name);
        imp.content_title.set_label(name);
        imp.content_status.set_label(status);
        imp.playing.set(false);
        self.set_play_icon(false);
        // The LIVE badge is for live TV only.
        imp.is_live.set(kind == Kind::Live);
        imp.live_badge.set_visible(kind == Kind::Live);
        set_heart_active(&imp.player_heart, self.is_fav(kind, id));
        imp.on_player.set(true);
        imp.main_view.push(&*imp.player_page);
        self.show_controls();
    }

    /// Trailers are the first real-playback step: playable video URLs run
    /// through GStreamer inside the player page; YouTube links still need
    /// a video backend and keep the demo player.
    fn play_trailer(&self, kind: Kind, id: i64, title: &str, url: Option<String>) {
        self.play_with_status(kind, id, title, "Trailer");
        let Some(url) = url else { return };
        if url.contains("youtube.com") || url.contains("youtu.be") {
            self.toast("YouTube trailer playback arrives with the video backend — the demo player shows for now");
            return;
        }
        if self.start_playback(&url) {
            self.imp().playing.set(true);
            self.set_play_icon(true);
        } else {
            self.toast("Real playback is unavailable here — the demo player shows instead");
        }
    }

    /// Start real playback for `uri` with GStreamer's playbin, rendering
    /// into the player page via the GTK4 paintable sink when available.
    /// Returns whether real playback actually started.
    fn start_playback(&self, uri: &str) -> bool {
        let imp = self.imp();
        self.stop_pipeline();

        if gstreamer::init().is_err() {
            eprintln!("RTV_PLAYBACK: gst init failed");
            return false;
        }
        let Ok(playbin) = gstreamer::ElementFactory::make("playbin").build() else {
            eprintln!("RTV_PLAYBACK: playbin missing");
            return false;
        };
        let mut video = false;
        // The GTK4 paintable sink renders into our own page; without it
        // (older runtimes, headless GL) the audio still plays while the
        // demo surface stays up.
        if let Ok(sink) = gstreamer::ElementFactory::make("gtk4paintablesink").build() {
            let paintable = sink.property::<Option<gtk::gdk::Paintable>>("paintable");
            if let Some(paintable) = paintable {
                imp.video_surface.set_paintable(Some(&paintable));
                video = true;
                playbin.set_property("video-sink", &sink);
            }
        }
        if !video {
            if let Ok(fake) = gstreamer::ElementFactory::make("fakesink").build() {
                playbin.set_property("video-sink", &fake);
            }
        }
        playbin.set_property("uri", uri);
        playbin.set_property("volume", imp.volume_scale.value());
        playbin.set_property("mute", imp.muted.get());

        let playing = playbin.set_state(gstreamer::State::Playing).is_ok();
        if !playing {
            let _ = playbin.set_state(gstreamer::State::Null);
            return false;
        }

        // Bus events (EOS, errors) are drained on the main loop; the
        // poller stops itself once the pipeline is torn down. A failing
        // video sink (headless GL, old runtimes) is retried once without
        // it — audio keeps playing on the demo surface.
        if playbin.bus().is_some() {
            let win = self.downgrade();
            let retried = Rc::new(std::cell::Cell::new(false));
            glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                let Some(win) = win.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let Some(pipeline) = win.imp().pipeline.borrow().as_ref().cloned() else {
                    return glib::ControlFlow::Break;
                };
                let Some(bus) = pipeline.bus() else {
                    return glib::ControlFlow::Break;
                };
                let mut stop = false;
                while let Some(message) = bus.timed_pop(gstreamer::ClockTime::from_seconds(0)) {
                    match message.view() {
                        gstreamer::MessageView::Eos(..) => stop = true,
                        gstreamer::MessageView::Error(err) => {
                            let _ = err;
                            if retried.get() {
                                win.toast("Playback failed — returning to the demo player");
                                stop = true;
                            } else {
                                retried.set(true);
                                let _ = pipeline.set_state(gstreamer::State::Null);
                                if let Ok(fake) =
                                    gstreamer::ElementFactory::make("fakesink").build()
                                {
                                    pipeline.set_property("video-sink", &fake);
                                }
                                let i = win.imp();
                                i.video_surface.set_paintable(None::<&gtk::gdk::Paintable>);
                                i.video_surface.set_visible(false);
                                i.app_watermark.set_visible(true);
                                if pipeline.set_state(gstreamer::State::Playing).is_ok() {
                                    win.toast("Video sink unavailable — playing audio only");
                                    continue;
                                }
                                stop = true;
                            }
                        }
                        _ => {}
                    }
                }
                if stop {
                    win.stop_pipeline();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }

        imp.pipeline.replace(Some(playbin));
        imp.video_surface.set_visible(video);
        imp.app_watermark.set_visible(!video);
        video
    }

    /// Tear down any active pipeline and return the player to the demo
    /// surface. Safe to call when nothing is playing.
    fn stop_pipeline(&self) {
        let imp = self.imp();
        if let Some(pipeline) = imp.pipeline.take() {
            let _ = pipeline.set_state(gstreamer::State::Null);
            if let Some(bus) = pipeline.bus() {
                bus.remove_signal_watch();
            }
        }
        imp.video_surface
            .set_paintable(None::<&gtk::gdk::Paintable>);
        imp.video_surface.set_visible(false);
        imp.app_watermark.set_visible(true);
        if imp.playing.get() {
            imp.playing.set(false);
            self.set_play_icon(false);
        }
    }

    /// Seek the real pipeline by `delta` seconds; a no-op toast on the
    /// demo player.
    fn seek_relative(&self, delta: i64) {
        if self.imp().pipeline.borrow().is_none() {
            self.toast("Seek 10s (demo)");
            return;
        }
        let pipeline = self.imp().pipeline.borrow().as_ref().cloned();
        let Some(pipeline) = pipeline else {
            return;
        };
        let position = pipeline.query_position::<gstreamer::ClockTime>();
        let duration = pipeline.query_duration::<gstreamer::ClockTime>();
        let (Some(position), Some(duration)) = (position, duration) else {
            self.toast("Seeking is not available for this stream");
            return;
        };
        let target = if delta >= 0 {
            (position + gstreamer::ClockTime::from_seconds(delta as u64)).min(duration)
        } else {
            position.saturating_sub(gstreamer::ClockTime::from_seconds(delta.unsigned_abs()))
        };
        let _ = pipeline.seek(
            1.0,
            gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::KEY_UNIT,
            gstreamer::SeekType::Set,
            target,
            gstreamer::SeekType::Set,
            duration,
        );
        self.show_controls();
    }

    /// Open the content detail page for a Series/Movies card. Live cards
    /// have a play button instead and never reach this. The page body is
    /// rebuilt on every call; a generation counter neutralizes detail
    /// fetches that outlive the page they were started for.
    fn open_details(&self, kind: Kind, id: i64, name: &str) {
        let imp = self.imp();
        let Some(item) = imp
            .catalog
            .get()
            .and_then(|c| c.items.iter().find(|it| it.kind == kind && it.id == id))
        else {
            return;
        };
        imp.details_generation.set(imp.details_generation.get() + 1);
        let generation = imp.details_generation.get();
        imp.detail_page.set_title(name);
        imp.detail_target.replace(Some(FavKey {
            kind,
            id,
            name: name.to_string(),
        }));
        set_heart_active(&imp.detail_heart, self.is_fav(kind, id));

        while let Some(child) = imp.detail_content.first_child() {
            imp.detail_content.remove(&child);
        }

        match kind {
            Kind::Live => return,
            Kind::Series => self.open_series_details(id, name, item.icon.as_deref(), generation),
            Kind::Vod => self.open_vod_details(
                id,
                name,
                item.rating.clone(),
                item.icon.as_deref(),
                generation,
            ),
        }
        imp.main_view.push(&*imp.detail_page);
    }

    /// Series body: offline metadata from series.json on top, seasons list
    /// filled in by the async get_series_info fetch.
    fn open_series_details(&self, id: i64, name: &str, cover: Option<&str>, generation: u64) {
        let imp = self.imp();
        let meta = imp
            .catalog
            .get()
            .and_then(|c| c.series_meta.get(&id).cloned())
            .unwrap_or_default();

        let ctx = SeriesCtx {
            id,
            name: name.to_string(),
            cover: cover.map(str::to_string).or_else(|| meta.cover.clone()),
            meta: meta.clone(),
        };

        let (header, _) = detail_header(name, ctx.cover.as_deref(), &series_meta_lines(&meta));
        imp.detail_content.append(&header);

        // Rating-source chips sit right below the künye (cover + metadata
        // block). The IMDb score arrives from the live ratings lookup;
        // until then the chip shows a small placeholder.
        let (chips_row, chips) = source_chips_row(name);
        imp.detail_content.append(&chips_row);

        let win = self.downgrade();
        let panel_rating = meta.rating.clone().filter(|r| r != "0");
        let year = meta
            .release_date
            .as_deref()
            .map(|d| &d[..4])
            .map(str::to_string);
        crate::ratings::lookup(Kind::Series, name, year.as_deref(), move |ratings| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if win.imp().details_generation.get() != generation {
                return;
            }
            chips.apply(ratings.as_ref(), panel_rating.as_deref());
        });

        // Trailers get their own section above Seasons: a series can carry
        // several of them (even one per season).
        let trailers: Vec<String> = meta.trailer.clone().into_iter().collect();
        if let Some(section) = trailers_section(self, Kind::Series, id, name, &trailers) {
            imp.detail_content.append(&section);
        }

        imp.detail_content.append(&section_title("Seasons"));
        let seasons_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        seasons_box.append(&spinner());
        imp.detail_content.append(&seasons_box);

        let win = self.downgrade();
        let seasons_box = seasons_box.clone();
        details::series_info(id, move |result| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let i = win.imp();
            if i.details_generation.get() != generation {
                return;
            }
            win.fill_seasons(&seasons_box, result, ctx);
        });
    }

    /// Replace the seasons spinner with the season rows (or a graceful
    /// empty/failure state). Runs on the main loop from the fetch callback.
    fn fill_seasons(&self, seasons_box: &gtk::Box, result: Option<SeriesInfo>, ctx: SeriesCtx) {
        while let Some(child) = seasons_box.first_child() {
            seasons_box.remove(&child);
        }
        let Some(info) = result else {
            self.toast("Could not load series details");
            seasons_box.append(&dim_label(
                "Season information could not be loaded. Try again later.",
            ));
            return;
        };
        if info.seasons.is_empty() {
            seasons_box.append(&dim_label("No season information available."));
            return;
        }

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .css_classes(["boxed-list"])
            .build();
        for season in &info.seasons {
            let count = season.episode_count.or_else(|| {
                info.episodes
                    .get(&season.number)
                    .map(|episodes| episodes.len() as i64)
            });
            let title = match count {
                Some(n) => format!("Season {} ({n})", season.number),
                None => format!("Season {}", season.number),
            };

            let row = adw::ActionRow::new();
            let row_box = gtk::Box::builder()
                .spacing(12)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            if let Some(cover) = &season.cover {
                row_box.prepend(&cover_art(Some(cover), 40, 60));
            }
            row_box.append(
                &gtk::Label::builder()
                    .label(title)
                    .halign(gtk::Align::Start)
                    .valign(gtk::Align::Center)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            row_box.append(
                &gtk::Image::builder()
                    .icon_name("go-next-symbolic")
                    .css_classes(["dim-label"])
                    .valign(gtk::Align::Center)
                    .build(),
            );
            if let Some(air_date) = &season.air_date {
                row_box.append(
                    &gtk::Label::builder()
                        .label(air_date)
                        .css_classes(["caption", "dim-label"])
                        .valign(gtk::Align::Center)
                        .build(),
                );
            }
            row.set_child(Some(&row_box));
            // GtkListBox with selection NONE marks its rows inert: rows
            // must be explicitly activatable for boxed-list click rows.
            row.set_activatable(true);

            // Activating a season opens the episodes page: the detail
            // page's header stays on top, the season's episodes are
            // listed below.
            list.append(&row);
        }
        // Routed via the list's row-activated signal (the same proven
        // mechanism as the sidebar): per-row ActionRow "activated" does
        // not fire reliably in every panel setup.
        let info = info.clone();
        let ctx = ctx.clone();
        let win = self.downgrade();
        list.connect_row_activated(move |_, row| {
            if let Some(season) = info.seasons.get(row.index() as usize) {
                if let Some(win) = win.upgrade() {
                    win.show_episodes(&info, &ctx, season.number);
                }
            }
        });
        seasons_box.append(&list);
    }

    /// Push the episodes page for one season: the detail header (cover,
    /// title, Key: value metadata) stays on top, the season's episodes
    /// are listed below.
    fn show_episodes(&self, info: &SeriesInfo, ctx: &SeriesCtx, season_number: i64) {
        let imp = self.imp();
        imp.episodes_page
            .set_title(&format!("{} · Season {season_number}", ctx.name));
        while let Some(child) = imp.episodes_content.first_child() {
            imp.episodes_content.remove(&child);
        }

        let (header, _) = detail_header(
            &ctx.name,
            ctx.cover.as_deref(),
            &series_meta_lines(&ctx.meta),
        );
        imp.episodes_content.append(&header);
        // Some breathing room before the season heading, like the Play
        // pill on the movie page.
        imp.episodes_content.append(
            &gtk::Label::builder()
                .label(format!("Season {season_number}"))
                .halign(gtk::Align::Start)
                .margin_top(12)
                .css_classes(["title-3"])
                .build(),
        );

        let episodes = info
            .episodes
            .get(&season_number)
            .cloned()
            .unwrap_or_default();
        if episodes.is_empty() {
            // A season listed in seasons[] may have no episode entries.
            imp.episodes_content
                .append(&dim_label("No episodes available for this season."));
            imp.main_view.push(&*imp.episodes_page);
            return;
        }

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .css_classes(["boxed-list"])
            .build();
        for episode in &episodes {
            let row = episode_row(episode);
            // GtkListBox with selection NONE marks its rows inert; the
            // click routing lives on the list (see below).
            row.set_activatable(true);
            list.append(&row);
        }
        let episodes = episodes.clone();
        let ctx = ctx.clone();
        let win = self.downgrade();
        list.connect_row_activated(move |_, row| {
            if let Some(episode) = episodes.get(row.index() as usize) {
                if let Some(win) = win.upgrade() {
                    win.play_episode(
                        ctx.id,
                        &ctx.name,
                        season_number,
                        episode.number,
                        &episode.title,
                    );
                }
            }
        });
        imp.episodes_content.append(&list);
        imp.main_view.push(&*imp.episodes_page);
    }

    /// Movie body, laid out like the series page: cover with the Key:
    /// value details to its right, rating-source chips below the künye,
    /// a prominent Play button, then trailers (when present). The plot
    /// renders as the last künye line ("Plot"), under Duration.
    fn open_vod_details(
        &self,
        id: i64,
        name: &str,
        panel_rating: Option<String>,
        cover: Option<&str>,
        generation: u64,
    ) {
        let imp = self.imp();
        let (header, info_col) = detail_header(name, cover, &[]);
        imp.detail_content.append(&header);
        info_col.append(&spinner());

        // Rating-source chips sit right below the künye, above Play; the
        // ratings lookup refines the IMDb link and score in place, with
        // the panel's rating as the offline fallback.
        let (chips_row, chips) = source_chips_row(name);
        imp.detail_content.append(&chips_row);
        let win = self.downgrade();
        let panel_rating = panel_rating.filter(|r| r != "0");
        crate::ratings::lookup(Kind::Vod, name, None, move |ratings| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if win.imp().details_generation.get() != generation {
                return;
            }
            chips.apply(ratings.as_ref(), panel_rating.as_deref());
        });

        // A little distance from the künye/chips block.
        let play = gtk::Button::builder()
            .label("Play")
            .halign(gtk::Align::Center)
            .margin_top(18)
            .css_classes(["suggested-action", "pill"])
            .build();
        let win = self.downgrade();
        let title = name.to_string();
        play.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.play(Kind::Vod, id, &title);
            }
        });
        imp.detail_content.append(&play);

        let win = self.downgrade();
        let info_col = info_col.clone();
        let title = name.to_string();
        details::vod_info(id, move |result| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let i = win.imp();
            if i.details_generation.get() != generation {
                return;
            }
            win.fill_vod_details(id, &title, &info_col, result);
        });
    }

    /// Fill the movie header's info column with the fetched Key: value
    /// details (the plot is the last line, under Duration) and add the
    /// trailers section when the panel carries one.
    fn fill_vod_details(
        &self,
        id: i64,
        name: &str,
        info_col: &gtk::Box,
        result: Option<details::VodInfo>,
    ) {
        let imp = self.imp();
        while let Some(child) = info_col.first_child() {
            info_col.remove(&child);
        }
        let Some(info) = result else {
            self.toast("Could not load movie details");
            info_col.append(&dim_label(
                "Movie details are unavailable right now. Play still works.",
            ));
            return;
        };

        for (key, value) in [
            info.genre.map(|v| ("Genre", v)),
            info.release_date.map(|v| ("Released", v)),
            info.cast.map(|v| ("Cast", v)),
            info.director.map(|v| ("Director", v)),
            info.duration.map(|v| ("Duration", format_duration(&v))),
            info.plot.map(|v| ("Plot", v)),
        ]
        .into_iter()
        .flatten()
        {
            info_col.append(&kv_line(key, &value));
        }

        // Trailers play in-app like the series ones.
        let trailers: Vec<String> = info.trailer.into_iter().collect();
        if let Some(section) = trailers_section(self, Kind::Vod, id, name, &trailers) {
            imp.detail_content.append(&section);
        }
    }

    fn toggle_playing(&self) {
        let imp = self.imp();
        // Real pipeline: pause/resume the stream.
        if let Some(pipeline) = imp.pipeline.borrow().as_ref() {
            let target = if imp.playing.get() {
                gstreamer::State::Paused
            } else {
                gstreamer::State::Playing
            };
            let _ = pipeline.set_state(target);
            imp.playing.set(!imp.playing.get());
            self.set_play_icon(imp.playing.get());
            self.show_controls();
            return;
        }
        // Demo player (no real stream attached).
        imp.playing.set(!imp.playing.get());
        self.set_play_icon(imp.playing.get());
        let status = imp.current_status.borrow().clone();
        imp.content_status.set_label(if imp.playing.get() {
            "Playing (demo)"
        } else {
            &status
        });
        if imp.playing.get() {
            self.show_controls();
        } else {
            self.cancel_hide();
            imp.controls_revealer.set_reveal_child(true);
        }
    }

    fn set_play_icon(&self, playing: bool) {
        let icon = if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        };
        self.imp().play_button_small.set_icon_name(icon);
    }

    /// Reveal the control bar; while playing, schedule it to fade away
    /// again after a short delay (mouse motion or pause re-reveals).
    /// Stale timers are neutralized by a generation check instead of
    /// `SourceId::remove`, which panics on already-fired ids.
    fn show_controls(&self) {
        let imp = self.imp();
        let generation = imp.controls_generation.get() + 1;
        imp.controls_generation.set(generation);
        imp.controls_revealer.set_reveal_child(true);
        if self.is_fullscreen() {
            imp.live_badge.set_visible(imp.is_live.get());
        }
        if imp.playing.get() && imp.on_player.get() {
            let win = self.downgrade();
            glib::timeout_add_seconds_local(3, move || {
                if let Some(win) = win.upgrade() {
                    let i = win.imp();
                    if i.controls_generation.get() == generation {
                        i.controls_revealer.set_reveal_child(false);
                        if win.is_fullscreen() {
                            i.live_badge.set_visible(false);
                        }
                    }
                }
                glib::ControlFlow::Break
            });
        }
    }

    /// Invalidate any pending hide timer without touching its SourceId.
    fn cancel_hide(&self) {
        let imp = self.imp();
        imp.controls_generation
            .set(imp.controls_generation.get() + 1);
    }

    fn toggle_mute(&self) {
        let imp = self.imp();
        imp.muted.set(!imp.muted.get());
        imp.mute_button.set_icon_name(if imp.muted.get() {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        if let Some(pipeline) = imp.pipeline.borrow().as_ref() {
            pipeline.set_property("mute", imp.muted.get());
        }
    }

    fn toggle_fullscreen(&self) {
        if self.is_fullscreen() {
            self.unfullscreen();
        } else {
            self.fullscreen();
        }
    }
}

/// Detail-page header: cover art on the left (click opens a full-size
/// preview popup), title + stacked "Key: value" lines on the right.
/// Returns the whole header box and its info column, so async fetches
/// (movie details) can append more lines later.
fn detail_header(
    title: &str,
    cover: Option<&str>,
    lines: &[Option<(String, String)>],
) -> (gtk::Box, gtk::Box) {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(18)
        .build();
    box_.append(&cover_art(cover, 130, 195));

    let info = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Start)
        .hexpand(true)
        .build();
    info.append(
        &gtk::Label::builder()
            .label(title)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .halign(gtk::Align::Start)
            .css_classes(["title-2"])
            .build(),
    );
    for (key, value) in lines.iter().flatten() {
        info.append(&kv_line(key, value));
    }
    box_.append(&info);
    (box_, info)
}

/// "Key: value" with the key in bold; values are markup-escaped.
fn kv_line(key: &str, value: &str) -> gtk::Label {
    let escaped = glib::markup_escape_text(value);
    gtk::Label::builder()
        .use_markup(true)
        .label(format!("<b>{key}:</b> {escaped}"))
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .build()
}

/// The shared series header lines: Genre, Released, Cast, Director,
/// episode length and Plot — identical structure to the movie künye.
fn series_meta_lines(meta: &SeriesMeta) -> Vec<Option<(String, String)>> {
    [
        meta.genre.clone().map(|v| ("Genre".to_string(), v)),
        meta.release_date
            .clone()
            .map(|v| ("Released".to_string(), v)),
        meta.cast.clone().map(|v| ("Cast".to_string(), v)),
        meta.director.clone().map(|v| ("Director".to_string(), v)),
        meta.episode_run_time
            .clone()
            .map(|v| ("Episode length".to_string(), format!("~{v} min"))),
        meta.plot.clone().map(|v| ("Plot".to_string(), v)),
    ]
    .into_iter()
    .collect()
}

/// Trailers listed as rows with a thumbnail; activating one plays it in
/// the app (mock player) instead of leaving for a website. Returns None
/// when there is nothing to list.
fn trailers_section(
    win: &Window,
    kind: Kind,
    id: i64,
    title: &str,
    trailers: &[String],
) -> Option<gtk::Box> {
    if trailers.is_empty() {
        return None;
    }
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    section.append(&section_title("Trailers"));

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .css_classes(["boxed-list"])
        .build();
    let mut entries: Vec<(String, String)> = Vec::new();
    for (index, trailer) in trailers.iter().enumerate() {
        let row = adw::ActionRow::new();
        let row_box = gtk::Box::builder()
            .spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        // Placeholder with a centered play chip when no thumbnail loads;
        // once it does, the play chip is revealed on hover instead.
        row_box.append(&trailer_thumb(youtube_thumb(trailer).as_deref()));
        row_box.append(
            &gtk::Label::builder()
                .label(if trailers.len() == 1 {
                    "Trailer".to_string()
                } else {
                    format!("Trailer {}", index + 1)
                })
                .halign(gtk::Align::Start)
                .valign(gtk::Align::Center)
                .hexpand(true)
                .build(),
        );
        row.set_child(Some(&row_box));
        // GtkListBox with selection NONE marks its rows inert.
        row.set_activatable(true);
        list.append(&row);

        // The playback heart keeps favoriting the movie/series itself.
        entries.push((format!("{title} — Trailer {}", index + 1), trailer.clone()));
    }
    // Routed via the list's row-activated signal (same mechanism as the
    // sidebar; per-row signals miss inert selection-NONE rows).
    let win = win.downgrade();
    list.connect_row_activated(move |_, row| {
        let Some((trailer_title, url)) = entries.get(row.index() as usize) else {
            return;
        };
        if let Some(win) = win.upgrade() {
            win.play_trailer(kind, id, trailer_title, Some(url.clone()));
        }
    });
    section.append(&list);
    Some(section)
}

/// Thumbnail URL for a YouTube trailer link (mqdefault); mock ids do not
/// resolve, and the cover placeholder covers that gracefully.
fn youtube_thumb(url: &str) -> Option<String> {
    let id = url
        .split('?')
        .nth(1)
        .and_then(|query| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("v=").map(str::to_string))
        })
        .or_else(|| {
            url.split("youtu.be/")
                .nth(1)
                .and_then(|rest| rest.split(['?', '/']).next().map(str::to_string))
        })?;
    (!id.is_empty()).then(|| format!("https://img.youtube.com/vi/{id}/mqdefault.jpg"))
}

/// Cover art box with placeholder + async picture fill (same mechanics as
/// the content cards). Falls back to the placeholder on any failure. Once
/// a real image is loaded, clicking it opens a full-size preview popup.
fn cover_art(url: Option<&str>, width: i32, height: i32) -> gtk::Overlay {
    image_card(
        url,
        width,
        height,
        Some(("video-display-symbolic", 40)),
        None,
    )
}

/// Trailer thumbnail: a centered play chip acts as the placeholder; once
/// the YouTube thumb loads, the play chip is only revealed on hover.
fn trailer_thumb(url: Option<&str>) -> gtk::Overlay {
    let play = gtk::Image::builder()
        .icon_name("media-playback-start-symbolic")
        .pixel_size(20)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["play-chip"])
        .build();
    image_card(url, 64, 36, None, Some(play))
}

/// Image card shared by every thumbnail on the detail pages: a rounded
/// `.card` tile, an optional placeholder icon, an optional image that
/// fades in on load and an optional overlay image revealed by CSS on row
/// hover. A loaded image gets the pointer cursor and opens the preview
/// popup on click; failures keep the placeholder (never crash).
fn image_card(
    url: Option<&str>,
    width: i32,
    height: i32,
    placeholder: Option<(&str, i32)>,
    hover_reveal: Option<gtk::Image>,
) -> gtk::Overlay {
    let art = gtk::Box::builder()
        .css_classes(["card"])
        .width_request(width)
        .height_request(height)
        .valign(gtk::Align::Start)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    // Clip every layer (picture, placeholders, chips) to the rounded
    // card, so loaded images never poke past the frame corners.
    overlay.set_overflow(gtk::Overflow::Hidden);

    if let Some((icon, pixel)) = placeholder {
        overlay.add_overlay(
            &gtk::Image::builder()
                .icon_name(icon)
                .pixel_size(pixel)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build(),
        );
    }

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .width_request(width)
        .height_request(height)
        .visible(false)
        .build();
    overlay.add_overlay(&picture);
    if let Some(hover) = &hover_reveal {
        overlay.add_overlay(hover);
    }

    let loaded = std::rc::Rc::new(RefCell::new(None::<gtk::gdk::Texture>));
    if let Some(url) = url {
        let loaded = loaded.clone();
        crate::covers::fetch(url.to_string(), {
            let picture = picture.clone();
            let overlay = overlay.clone();
            move |path| {
                if let Some(path) = path {
                    if let Ok(texture) = gtk::gdk::Texture::from_filename(&path) {
                        loaded.replace(Some(texture.clone()));
                        picture.set_paintable(Some(&texture));
                        picture.set_visible(true);
                        if let Some(hover) = &hover_reveal {
                            hover.add_css_class("hover-reveal");
                        }
                        if let Some(cursor) = gtk::gdk::Cursor::from_name("pointer", None) {
                            overlay.set_cursor(Some(&cursor));
                        }
                    }
                }
            }
        });
    }

    let click = gtk::GestureClick::new();
    // Capture phase + explicit claim: the click belongs to the preview —
    // it must not ALSO activate the surrounding list row.
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed({
        let loaded = loaded.clone();
        let overlay = overlay.clone();
        move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let _ = (x, y);
            if let Some(texture) = loaded.borrow().as_ref() {
                open_preview(&overlay, texture);
            }
        }
    });
    overlay.add_controller(click);

    overlay
}

/// Full-size image preview popup, sized to the image's aspect ratio and
/// generously large. Closes via the corner button, Escape or by clicking
/// anywhere outside it (losing keyboard focus).
fn open_preview(parent: &impl glib::object::IsA<gtk::Widget>, texture: &gtk::gdk::Texture) {
    let window = adw::Window::new();
    if let Some(root) = parent.root().and_downcast::<adw::ApplicationWindow>() {
        window.set_transient_for(Some(&root));
    }
    // Not modal on purpose: a modal grab would swallow the outside
    // clicks that are supposed to dismiss the preview.
    window.set_title(Some("Preview"));
    let (width, height) = preview_size(texture);
    window.set_default_size(width, height);

    let picture = gtk::Picture::builder()
        .paintable(texture)
        .content_fit(gtk::ContentFit::Contain)
        .hexpand(true)
        .vexpand(true)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));

    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Close")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_end(12)
        .css_classes(["osd", "circular"])
        .build();
    close.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    overlay.add_overlay(&close);

    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                window.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    overlay.add_controller(keys);

    // Clicking anything outside the popup takes focus away from it.
    window.connect_is_active_notify(|window| {
        if !window.is_active() {
            window.close();
        }
    });

    window.set_content(Some(&overlay));
    window.present();
}

/// Window size for the preview: the image's aspect ratio, scaled into a
/// generous maximum (it may exceed the app window a little). Tiny mock
/// thumbnails are upscaled at most 3x so the window stays reasonable.
fn preview_size(texture: &gtk::gdk::Texture) -> (i32, i32) {
    const MAX_W: f64 = 1200.0;
    const MAX_H: f64 = 800.0;
    const MAX_UPSCALE: f64 = 3.0;
    let width = texture.intrinsic_width() as f64;
    let height = texture.intrinsic_height() as f64;
    if width <= 0.0 || height <= 0.0 {
        return (900, 640);
    }
    let scale = (MAX_W / width).min(MAX_H / height).min(MAX_UPSCALE);
    (
        (width * scale).round() as i32,
        (height * scale).round() as i32,
    )
}

/// Centered spinner used as the placeholder of async-filled sections.
fn spinner() -> gtk::Spinner {
    gtk::Spinner::builder()
        .spinning(true)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_top(12)
        .margin_bottom(12)
        .build()
}

fn dim_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .css_classes(["dim-label"])
        .build()
}

/// Bold section heading inside a detail page ("Seasons", "Trailers").
fn section_title(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .css_classes(["title-3"])
        .build()
}

/// The updatable parts of the source chips row: the IMDb chip's score
/// label and its URL (upgraded to the exact title page once the ratings
/// lookup resolves the IMDb id).
struct SourceChips {
    imdb_score: gtk::Label,
    imdb_url: Rc<RefCell<String>>,
}

impl SourceChips {
    /// Apply a ratings lookup result: fill the IMDb score and/or point
    /// the IMDb chip at the exact title page. When no source provides a
    /// score, fall back to the panel's own rating; with neither, show
    /// "N/A" (objective over invented).
    fn apply(&self, ratings: Option<&crate::ratings::Ratings>, panel: Option<&str>) {
        match ratings.and_then(|r| r.imdb.as_ref()) {
            Some(score) => {
                self.imdb_score.remove_css_class("dim-label");
                self.imdb_score.set_text(score);
            }
            None => match panel {
                Some(score) => {
                    self.imdb_score.remove_css_class("dim-label");
                    self.imdb_score.set_text(score);
                }
                None => {
                    self.imdb_score.add_css_class("dim-label");
                    self.imdb_score.set_text("N/A");
                }
            },
        }
        if let Some(id) = ratings.and_then(|r| r.imdb_id.as_ref()) {
            self.imdb_url
                .replace(format!("https://www.imdb.com/title/{id}/"));
        }
    }
}

/// "Ying-yang" source chips: the left half names the source, the right
/// half shows its score — or an external-link arrow when no score is
/// (yet) available. Activating opens the source in the browser; the IMDb
/// chip's score/url are updated in place by the ratings lookup.
fn source_chips_row(title: &str) -> (gtk::Box, SourceChips) {
    let query = glib::uri_escape_string(title, None::<&str>, false);
    let row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Start)
        .build();

    let imdb_url = Rc::new(RefCell::new(format!(
        "https://www.imdb.com/find/?q={query}"
    )));
    // While the ratings lookup runs, the score half shows a small
    // placeholder; it settles into the score, the panel's rating or
    // "N/A" once the lookup lands.
    let imdb_score = gtk::Label::builder()
        .label("…")
        .css_classes(["dim-label"])
        .build();
    row.append(&source_chip("IMDb", imdb_url.clone(), imdb_score.clone()));
    let rotten_url = format!("https://www.rottentomatoes.com/search?search={query}");
    row.append(&source_chip(
        "Rotten Tomatoes",
        Rc::new(RefCell::new(rotten_url)),
        arrow_label(),
    ));
    let wiki_url = format!("https://en.wikipedia.org/w/index.php?search={query}");
    row.append(&source_chip(
        "Wikipedia",
        Rc::new(RefCell::new(wiki_url)),
        arrow_label(),
    ));

    (
        row,
        SourceChips {
            imdb_score,
            imdb_url,
        },
    )
}

fn arrow_label() -> gtk::Label {
    gtk::Label::builder()
        .label("↗")
        .css_classes(["dim-label"])
        .build()
}

/// One source chip: source name | right-half label; the URL is kept in a
/// RefCell so an upgraded link (exact IMDb title) is used when clicked.
fn source_chip(source: &str, url: Rc<RefCell<String>>, right: gtk::Label) -> gtk::Button {
    let content = gtk::Box::builder().spacing(8).build();
    content.append(
        &gtk::Label::builder()
            .label(source)
            .css_classes(["heading"])
            .build(),
    );
    content.append(
        &gtk::Separator::builder()
            .orientation(gtk::Orientation::Vertical)
            .build(),
    );
    content.append(&right);
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(format!("Open {source}"))
        .css_classes(["pill"])
        .build();
    button.connect_clicked(move |_| {
        // gtk_uri_launcher_present would be the modern API; show_uri
        // still opens the default browser with zero ceremony.
        #[allow(deprecated)]
        gtk::show_uri(
            None::<&gtk::Window>,
            &url.borrow().clone(),
            gtk::gdk::CURRENT_TIME,
        );
    });
    button
}

/// "0:02:06:00" / "0:36:00" panel durations become "2 hours 6 minutes" /
/// "36 minutes"; anything that does not parse is shown verbatim.
fn format_duration(raw: &str) -> String {
    let parts: Vec<u64> = raw
        .split(':')
        .map(|p| p.trim().parse::<u64>())
        .collect::<Result<_, _>>()
        .unwrap_or_default();
    // Panels vary: days:hours:minutes:seconds, hours:minutes:seconds or
    // minutes:seconds.
    let (hours, minutes) = match parts.as_slice() {
        [days, hours, minutes, _seconds] => (days * 24 + hours, *minutes),
        [hours, minutes, _seconds] => (*hours, *minutes),
        [minutes, _seconds] => (0, *minutes),
        _ => return raw.to_string(),
    };
    let mut phrase: Vec<String> = Vec::new();
    if hours > 0 {
        phrase.push(if hours == 1 {
            "1 hour".to_string()
        } else {
            format!("{hours} hours")
        });
    }
    if minutes > 0 {
        phrase.push(if minutes == 1 {
            "1 minute".to_string()
        } else {
            format!("{minutes} minutes")
        });
    }
    if phrase.is_empty() {
        "0 minutes".to_string()
    } else {
        phrase.join(" ")
    }
}

/// One episode row inside the episodes page: thumbnail, title, duration,
/// container and plot. Activation (playing the episode, favoriting the
/// parent series) is routed by the surrounding list.
fn episode_row(episode: &Episode) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let row_box = gtk::Box::builder()
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    if let Some(image) = &episode.image {
        row_box.append(&cover_art(Some(image), 64, 36));
    }

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(format!("E{} — {}", episode.number, episode.title))
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let subtitle = episode.duration.as_deref().map(format_duration);
    if let Some(subtitle) = subtitle {
        text.append(
            &gtk::Label::builder()
                .label(subtitle)
                .halign(gtk::Align::Start)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    if let Some(plot) = &episode.plot {
        text.append(
            &gtk::Label::builder()
                .label(plot)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .xalign(0.0)
                .halign(gtk::Align::Start)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    row_box.append(&text);
    row.set_child(Some(&row_box));
    row
}

fn content_card(item: &Item, category_label: Option<&str>) -> gtk::FlowBoxChild {
    card_body(
        &item.name,
        item.icon.as_deref(),
        Some(item.kind),
        Some(item.id),
        category_label,
    )
}

/// Content card: cover art + title, no overlay chrome — activating the
/// card itself routes to playback (Live) or the detail page
/// (Series/Movies). Favorites live on the detail/player pages.
fn card_body(
    name: &str,
    icon: Option<&str>,
    kind: Option<Kind>,
    item_id: Option<i64>,
    category_label: Option<&str>,
) -> gtk::FlowBoxChild {
    let art = gtk::Box::builder()
        .css_classes(["card"])
        .width_request(150)
        .height_request(150)
        .build();
    let placeholder = gtk::Image::builder()
        .icon_name("video-display-symbolic")
        .pixel_size(56)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .visible(false)
        .build();

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    // Clip every layer (picture, placeholders, chips) to the rounded
    // card, so loaded images never poke past the frame corners.
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.add_overlay(&placeholder);
    overlay.add_overlay(&picture);

    // Cover artwork: fetch-on-demand with a persistent disk cache; the
    // placeholder stays until the fetch succeeds and on any failure.
    if let Some(url) = icon {
        let picture = picture.clone();
        let placeholder = placeholder.clone();
        crate::covers::fetch(url.to_string(), move |path| {
            if let Some(path) = path {
                if let Ok(texture) = gtk::gdk::Texture::from_filename(&path) {
                    picture.set_paintable(Some(&texture));
                    picture.set_visible(true);
                    placeholder.set_visible(false);
                }
            }
        });
    }

    let label = gtk::Label::builder()
        .label(name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .max_width_chars(16)
        .build();
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    b.append(&overlay);
    b.append(&label);
    if let Some(category) = category_label {
        b.append(
            &gtk::Label::builder()
                .label(category)
                .wrap(true)
                .justify(gtk::Justification::Center)
                .halign(gtk::Align::Center)
                .max_width_chars(16)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }

    let child = gtk::FlowBoxChild::builder().child(&b).build();
    // Activation data: the card's content kind, catalog id and title ride
    // on the FlowBoxChild so child_activated can route Live -> playback
    // and Series/Movies -> the detail page without re-parsing labels.
    if let Some(kind) = kind {
        unsafe {
            child.set_data(CARD_KIND_KEY, kind as i32);
        }
    }
    if let Some(id) = item_id {
        unsafe {
            child.set_data(CARD_ID_KEY, id);
        }
    }
    unsafe {
        child.set_data(CARD_NAME_KEY, name.to_string());
    }
    child
}

/// FlowBoxChild data keys for the card-activation router.
const CARD_KIND_KEY: &str = "rtv-item-kind";
const CARD_ID_KEY: &str = "rtv-item-id";
const CARD_NAME_KEY: &str = "rtv-item-name";

// `NonNull::get` collides with `TreeModelExtManual::get` from the gtk4
// prelude; all derefs stay inside a single unsafe block.
fn card_kind(child: &gtk::FlowBoxChild) -> Option<Kind> {
    let kind =
        unsafe { glib::object::ObjectExt::data::<i32>(child, CARD_KIND_KEY).map(|v| *v.as_ref()) }?;
    Some(match kind {
        1 => Kind::Live,
        2 => Kind::Series,
        _ => Kind::Vod,
    })
}

fn card_id(child: &gtk::FlowBoxChild) -> Option<i64> {
    unsafe { glib::object::ObjectExt::data::<i64>(child, CARD_ID_KEY).map(|v| *v.as_ref()) }
}

fn card_name(child: &gtk::FlowBoxChild) -> Option<String> {
    unsafe {
        glib::object::ObjectExt::data::<String>(child, CARD_NAME_KEY).map(|v| v.as_ref().clone())
    }
}

fn set_heart_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class("active");
    } else {
        button.remove_css_class("active");
    }
}
