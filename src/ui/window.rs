use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use crate::details::{self, Episode, SeriesInfo};
use crate::sample::{self, Item, Kind};

/// Search results are capped so a one-letter query cannot flood the grid;
/// the backend phase will replace this with real incremental search.
const SEARCH_CAP: usize = 500;
/// Cards appended to a grid per idle tick (progressive rendering).
const FILL_CHUNK: usize = 40;

/// Sidebar section header row: icon + label, click toggles collapse.
#[derive(Debug)]
pub struct HeaderRow {
    row: gtk::ListBoxRow,
}

/// Sidebar category row: activating it shows the category's content grid.
#[derive(Debug)]
pub struct CategoryRow {
    row: gtk::ListBoxRow,
    category_index: usize,
    group: i32,
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

    use super::{CategoryRow, HeaderRow};

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
        pub favorites_grid: TemplateChild<gtk::FlowBox>,
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
        pub detail_content: TemplateChild<gtk::Box>,
        #[template_child]
        pub episodes_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub episodes_list: TemplateChild<gtk::ListBox>,

        /// Sample catalog loaded from samples/ (mock phase).
        pub catalog: OnceLock<Catalog>,
        /// UI-mock state.
        pub playing: Cell<bool>,
        pub muted: Cell<bool>,
        pub has_source: Cell<bool>,
        pub on_player: Cell<bool>,
        pub selected_category: Cell<i32>,
        pub last_pointer: Cell<(f64, f64)>,
        pub favorites: RefCell<Vec<String>>,
        pub current: RefCell<Option<String>>,
        pub collapsed: RefCell<Vec<bool>>,
        pub query: RefCell<String>,
        pub categories: RefCell<Vec<CategoryRow>>,
        pub headers: RefCell<Vec<HeaderRow>>,
        /// Pinned "Favorites" sidebar row, always visible above the sections.
        pub favorites_row: RefCell<Option<gtk::ListBoxRow>>,
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
        window.setup_actions();
        window.setup_browse();
        window.imp().selected_category.set(-1);
        // Live expanded, Series/Movies collapsed on first open.
        {
            let mut collapsed = window.imp().collapsed.borrow_mut();
            collapsed.clear();
            collapsed.extend(Kind::ALL.iter().enumerate().map(|(i, _)| i != 0));
        }
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
            win.rebuild_favorites();
            imp.main_view.pop_to_page(&*imp.welcome_page);
        });
        self.add_action(&reset);

        // Classic media-player shortcuts (accelerators registered in app.rs).
        // They only act while the player page is on screen.
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
                win.toast("Seek back 10s (mock)");
            }
        });
        self.add_action(&seek_back);

        let win = self.downgrade();
        let seek_forward = gio::SimpleAction::new("seek-forward", None);
        seek_forward.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            if win.imp().on_player.get() {
                win.toast("Seek forward 10s (mock)");
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
        let favorites_row = gtk::ListBoxRow::new();
        favorites_row.set_child(Some(
            &gtk::Label::builder()
                .label("Favorites (0)")
                .halign(gtk::Align::Start)
                .margin_start(6)
                .build(),
        ));
        imp.content_list.prepend(&favorites_row);
        imp.favorites_row.replace(Some(favorites_row.clone()));
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

        // Favorites cards route by content kind: Live plays, Series/Movies
        // open their detail page (name-only fallback cards keep playing).
        let win = self.downgrade();
        imp.favorites_grid.connect_child_activated(move |_, child| {
            if let Some(win) = win.upgrade() {
                win.route_card(child);
            }
        });

        // "Clear all" empties the favorites at once and refreshes whatever
        // grid is showing so the card hearts fall back to inactive.
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
                win.rebuild_favorites();
                let selected = i.selected_category.get();
                if selected >= 0 {
                    win.select_category(selected as usize);
                } else {
                    let query = i.query.borrow().clone();
                    if !query.trim().is_empty() {
                        win.apply_search(&query);
                    }
                }
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

        // When any page pops off the navigation view (player, details or
        // episodes), clear player/episodes state if they were on screen.
        let win = self.downgrade();
        imp.main_view.connect_popped(move |_, popped_page| {
            let Some(win) = win.upgrade() else { return };
            let i = win.imp();
            if popped_page == &*i.player_page {
                i.on_player.set(false);
                win.cancel_hide();
            }
            if popped_page == &*i.episodes_page {
                while let Some(child) = i.episodes_list.first_child() {
                    i.episodes_list.remove(&child);
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
                win.toast("Seek back 10s (mock)");
            }
        });
        let win = self.downgrade();
        imp.seek_forward.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toast("Seek forward 10s (mock)");
            }
        });
        let win = self.downgrade();
        imp.mute_button.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_mute();
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
                let name = win.imp().current.borrow().clone();
                if let Some(name) = name {
                    win.toggle_fav(&name);
                }
            }
        });

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
            i.live_badge.set_visible(!fullscreen);
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
    /// section per content kind, its categories underneath.
    fn setup_sidebar(&self) {
        let imp = self.imp();
        let Some(catalog) = imp.catalog.get() else {
            return;
        };

        for (group, kind) in Kind::ALL.iter().enumerate() {
            let group = group as i32;
            let cats: Vec<(usize, &sample::Category)> = catalog
                .categories
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == *kind)
                .collect();

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
            header_box.append(
                &gtk::Label::builder()
                    .label(format!("{} · {}", kind.title(), cats.len()))
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build(),
            );
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
            imp.headers.borrow_mut().push(HeaderRow { row: header });

            for (cat_index, cat) in cats {
                let count = catalog
                    .items
                    .iter()
                    .filter(|it| it.kind == *kind && it.category_id == cat.id)
                    .count();
                let row = gtk::ListBoxRow::new();
                let row_box = gtk::Box::builder().margin_start(26).build();
                row_box.append(
                    &gtk::Label::builder()
                        .label(format!("{} ({})", cat.name, count))
                        .halign(gtk::Align::Start)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
                row.set_child(Some(&row_box));
                imp.content_list.append(&row);
                imp.categories.borrow_mut().push(CategoryRow {
                    row,
                    category_index: cat_index,
                    group,
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
                imp.content_stack.set_visible_child_name("grid");
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
        let indices: Vec<usize> = catalog
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.name.to_lowercase().contains(&q))
            .take(SEARCH_CAP)
            .map(|(i, _)| i)
            .collect();
        let favs = imp.favorites.borrow().clone();
        imp.content_stack.set_visible_child_name("search");
        self.fill_grid(indices, &imp.search_grid, &favs, true);
    }

    /// Visibility of every sidebar row, driven by the search query and the
    /// per-section collapse state.
    fn refresh_rows_visibility(&self) {
        let imp = self.imp();
        let q = imp.query.borrow().to_lowercase();

        if q.is_empty() {
            for c in imp.categories.borrow().iter() {
                c.row.set_visible(!self.collapsed(c.group));
            }
            for h in imp.headers.borrow().iter() {
                h.row.set_visible(true);
            }
        } else {
            // Search filters CONTENT (the middle grid), not the sidebar:
            // every section and category stays visible while searching.
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

    /// Show a category's content as a card grid in the middle pane.
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
        let indices: Vec<usize> = catalog
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.kind == cat.kind && it.category_id == cat.id)
            .map(|(i, _)| i)
            .collect();
        let favs = imp.favorites.borrow().clone();
        imp.content_stack.set_visible_child_name("grid");
        self.fill_grid(indices, &imp.content_grid, &favs, false);
    }

    /// Fill a grid progressively: FILL_CHUNK cards per idle tick so a
    /// 1800-card category never blocks the UI. A generation counter makes
    /// stale fillers (category switch, new search) stop immediately.
    fn fill_grid(
        &self,
        mut indices: Vec<usize>,
        flowbox: &gtk::FlowBox,
        favs: &[String],
        with_categories: bool,
    ) {
        let imp = self.imp();
        imp.fill_generation.set(imp.fill_generation.get() + 1);
        let generation = imp.fill_generation.get();
        indices.reverse(); // pop() from the end preserves order
        let remaining = std::rc::Rc::new(RefCell::new(indices));
        let flowbox = flowbox.clone();
        let favs = favs.to_vec();
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
                flowbox.append(&content_card(
                    item,
                    &favs,
                    category_label.as_deref(),
                    win.clone(),
                ));
            }
            if rem.is_empty() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn toggle_fav(&self, name: &str) -> bool {
        let imp = self.imp();
        {
            let mut favs = imp.favorites.borrow_mut();
            if let Some(pos) = favs.iter().position(|n| n == name) {
                favs.remove(pos);
            } else {
                favs.push(name.to_string());
            }
        }
        let is_fav = imp.favorites.borrow().iter().any(|n| n == name);
        self.rebuild_favorites();
        if imp.current.borrow().as_deref() == Some(name) {
            set_heart_active(&imp.player_heart, is_fav);
        }
        is_fav
    }

    /// Switch the middle pane to the favorites grid.
    fn show_favorites(&self) {
        self.imp().content_stack.set_visible_child_name("favorites");
    }

    /// Refresh everything favorites-related: the pinned sidebar row label
    /// (with count), the favorites page (grid vs. empty status) and the
    /// "Clear all" sensitivity. Favorite counts are small, so the grid
    /// refills synchronously instead of going through the chunk filler.
    fn rebuild_favorites(&self) {
        let imp = self.imp();
        let favs = imp.favorites.borrow().clone();
        let count = favs.len();

        if let Some(row) = imp.favorites_row.borrow().as_ref() {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                label.set_text(&format!("Favorites ({count})"));
            }
        }
        imp.clear_favorites.set_sensitive(count > 0);
        imp.favorites_stack
            .set_visible_child_name(if count == 0 { "empty" } else { "grid" });

        while let Some(child) = imp.favorites_grid.first_child() {
            imp.favorites_grid.remove(&child);
        }
        let win = self.downgrade();
        for name in &favs {
            // Items are looked up by name; a name without a catalog match
            // still gets a bare card (no cover, no kind action).
            let card = match imp
                .catalog
                .get()
                .and_then(|c| c.items.iter().find(|it| &it.name == name))
            {
                Some(item) => content_card(item, &favs, None, win.clone()),
                None => name_card(name, &favs, win.clone()),
            };
            imp.favorites_grid.append(&card);
        }
    }

    /// Card activation routes by the content kind the card carries:
    /// Live -> playback, Series/Movies -> the detail page, unknown
    /// (name-only favorites) -> playback by name.
    fn route_card(&self, child: &gtk::FlowBoxChild) {
        let imp = self.imp();
        if let (Some(kind), Some(id)) = (card_kind(child), card_id(child)) {
            if let Some(item) = imp
                .catalog
                .get()
                .and_then(|c| c.items.iter().find(|it| it.kind == kind && it.id == id))
            {
                match kind {
                    Kind::Live => self.play(&item.name),
                    Kind::Series | Kind::Vod => self.open_details(kind, id, &item.name),
                }
                return;
            }
        }
        // Fallback: cards without kind data (old-style name cards) play by
        // their title, which is all the information they carry.
        if let Some(name) = card_name(child) {
            self.play(&name);
        }
    }

    fn play(&self, name: &str) {
        let imp = self.imp();
        *imp.current.borrow_mut() = Some(name.to_string());
        imp.player_page.set_title(name);
        imp.content_title.set_label(name);
        imp.content_status.set_label("Live · 1080p");
        imp.playing.set(false);
        self.set_play_icon(false);
        let is_fav = imp.favorites.borrow().iter().any(|n| n == name);
        set_heart_active(&imp.player_heart, is_fav);
        imp.on_player.set(true);
        imp.main_view.push(&*imp.player_page);
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

        while let Some(child) = imp.detail_content.first_child() {
            imp.detail_content.remove(&child);
        }
        while let Some(child) = imp.episodes_list.first_child() {
            imp.episodes_list.remove(&child);
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

        imp.detail_content.append(&detail_header(
            name,
            cover.or(meta.cover.as_deref()),
            &[
                meta.genre.clone(),
                meta.rating.clone(),
                meta.release_date.clone(),
            ],
            meta.cast.as_deref(),
            meta.director.as_deref(),
            meta.episode_run_time.as_deref(),
        ));

        if let Some(plot) = &meta.plot {
            imp.detail_content.append(&plot_label(plot));
        }

        // Fragman varsa tıklanabilir bir satır olarak göster; GTK'nın
        // gösterdiği URI'yi harici tarayıcıda açar.
        if let Some(trailer) = &meta.trailer {
            imp.detail_content.append(
                &gtk::LinkButton::builder()
                    .uri(trailer)
                    .label("Watch trailer")
                    .halign(gtk::Align::Start)
                    .build(),
            );
        }

        imp.detail_content.append(
            &gtk::Label::builder()
                .label("Seasons")
                .halign(gtk::Align::Start)
                .css_classes(["title-3"])
                .build(),
        );
        let seasons_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        seasons_box.append(
            &gtk::Spinner::builder()
                .spinning(true)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .margin_top(12)
                .margin_bottom(12)
                .build(),
        );
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
            win.fill_seasons(&seasons_box, result);
        });
    }

    /// Replace the seasons spinner with the season rows (or a graceful
    /// empty/failure state). Runs on the main loop from the fetch callback.
    fn fill_seasons(&self, seasons_box: &gtk::Box, result: Option<SeriesInfo>) {
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
            .css_classes(["boxed-list"])
            .build();
        for season in &info.seasons {
            let count = season.episode_count.or_else(|| {
                info.episodes
                    .get(&season.number)
                    .map(|episodes| episodes.len() as i64)
            });
            let title = match count {
                Some(1) => format!("Season {} — 1 episode", season.number),
                Some(n) => format!("Season {} — {n} episodes", season.number),
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
                row_box.prepend(&cover_art(Some(cover), 34, 51));
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

            let episodes = info
                .episodes
                .get(&season.number)
                .cloned()
                .unwrap_or_default();
            let season_title = format!("Season {}", season.number);
            let win = self.downgrade();
            row.connect_activated(move |_| {
                if let Some(win) = win.upgrade() {
                    win.show_episodes(&season_title, &episodes);
                }
            });
            list.append(&row);
        }
        seasons_box.append(&list);
    }

    /// Push the episodes page for one season and fill its list.
    fn show_episodes(&self, season_title: &str, episodes: &[Episode]) {
        let imp = self.imp();
        imp.episodes_page.set_title(season_title);
        while let Some(child) = imp.episodes_list.first_child() {
            imp.episodes_list.remove(&child);
        }

        if episodes.is_empty() {
            // A season listed in seasons[] may have no episode entries.
            let row = gtk::ListBoxRow::builder()
                .activatable(false)
                .selectable(false)
                .build();
            row.set_child(Some(&dim_label("No episodes available for this season.")));
            imp.episodes_list.append(&row);
        }
        for episode in episodes {
            let row = adw::ActionRow::new();
            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(3)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            row_box.append(
                &gtk::Label::builder()
                    .label(format!("E{} — {}", episode.number, episode.title))
                    .halign(gtk::Align::Start)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            let mut subtitle = episode.duration.clone();
            if let Some(container) = &episode.container {
                subtitle = Some(match subtitle {
                    Some(duration) => format!("{duration} · {container}"),
                    None => container.clone(),
                });
            }
            if let Some(subtitle) = subtitle {
                row_box.append(
                    &gtk::Label::builder()
                        .label(subtitle)
                        .halign(gtk::Align::Start)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .css_classes(["caption", "dim-label"])
                        .build(),
                );
            }
            if let Some(plot) = &episode.plot {
                row_box.append(
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
            row.set_child(Some(&row_box));

            let win = self.downgrade();
            let episode_title = episode.title.clone();
            row.connect_activated(move |_| {
                if let Some(win) = win.upgrade() {
                    win.play(&episode_title);
                }
            });
            imp.episodes_list.append(&row);
        }
        imp.main_view.push(&*imp.episodes_page);
    }

    /// Movie body: offline name/rating/cover immediately, plot & co. from
    /// the async get_vod_info fetch, plus a prominent Play button.
    fn open_vod_details(
        &self,
        id: i64,
        name: &str,
        rating: Option<String>,
        cover: Option<&str>,
        generation: u64,
    ) {
        let imp = self.imp();

        imp.detail_content.append(&detail_header(
            name,
            cover,
            &[None, rating.clone(), None],
            None,
            None,
            None,
        ));

        // Details area: spinner while the fetch runs, replaced by the
        // meta/plot block on success or a graceful note on failure.
        let details_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        details_box.append(
            &gtk::Spinner::builder()
                .spinning(true)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .margin_top(12)
                .margin_bottom(12)
                .build(),
        );
        imp.detail_content.append(&details_box);

        let play = gtk::Button::builder()
            .label("Play")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        let win = self.downgrade();
        let title = name.to_string();
        play.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.play(&title);
            }
        });
        imp.detail_content.append(&play);

        let win = self.downgrade();
        let details_box = details_box.clone();
        details::vod_info(id, move |result| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let i = win.imp();
            if i.details_generation.get() != generation {
                return;
            }
            win.fill_vod_details(&details_box, result, rating);
        });
    }

    /// Replace the movie-details spinner with the fetched metadata (or a
    /// graceful failure note; name/rating/cover are already on the page).
    fn fill_vod_details(
        &self,
        details_box: &gtk::Box,
        result: Option<details::VodInfo>,
        rating: Option<String>,
    ) {
        while let Some(child) = details_box.first_child() {
            details_box.remove(&child);
        }
        let Some(info) = result else {
            self.toast("Could not load movie details");
            details_box.append(&dim_label(
                "Movie details are unavailable right now. Play still works.",
            ));
            return;
        };

        if let Some(meta) = meta_line(&[info.genre, rating, info.release_date]) {
            details_box.append(&dim_label(&meta));
        }
        if let Some(plot) = &info.plot {
            details_box.append(&plot_label(plot));
        }
        if let Some(cast) = &info.cast {
            details_box.append(&dim_label(&format!("Cast: {cast}")));
        }
        if let Some(director) = &info.director {
            details_box.append(&dim_label(&format!("Director: {director}")));
        }
        if let Some(duration) = &info.duration {
            details_box.append(&dim_label(&format!("Duration: {duration}")));
        }
    }

    fn toggle_playing(&self) {
        let imp = self.imp();
        imp.playing.set(!imp.playing.get());
        self.set_play_icon(imp.playing.get());
        imp.content_status.set_label(if imp.playing.get() {
            "Playing (mock)"
        } else {
            "Live · 1080p"
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
            imp.live_badge.set_visible(true);
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
    }

    fn toggle_fullscreen(&self) {
        if self.is_fullscreen() {
            self.unfullscreen();
        } else {
            self.fullscreen();
        }
    }
}

/// Detail-page header: cover art on the left, title + "genre · rating ·
/// date" meta line (+ optional cast, director and run-time lines) on the
/// right.
#[allow(clippy::too_many_arguments)]
fn detail_header(
    title: &str,
    cover: Option<&str>,
    meta: &[Option<String>],
    cast: Option<&str>,
    director: Option<&str>,
    run_time: Option<&str>,
) -> gtk::Box {
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
    if let Some(meta) = meta_line(meta) {
        info.append(&dim_label(&meta));
    }
    if let Some(cast) = cast {
        info.append(&dim_label(&format!("Cast: {cast}")));
    }
    if let Some(director) = director {
        info.append(&dim_label(&format!("Director: {director}")));
    }
    if let Some(run_time) = run_time {
        info.append(&dim_label(&format!("~{run_time} min per episode")));
    }
    box_.append(&info);
    box_
}

/// Cover art box with placeholder + async picture fill (same mechanics as
/// the content cards). Falls back to the placeholder on any failure.
fn cover_art(url: Option<&str>, width: i32, height: i32) -> gtk::Overlay {
    let art = gtk::Box::builder()
        .css_classes(["card"])
        .width_request(width)
        .height_request(height)
        .valign(gtk::Align::Start)
        .build();
    let placeholder = gtk::Image::builder()
        .icon_name("video-display-symbolic")
        .pixel_size(40)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .width_request(width)
        .height_request(height)
        .visible(false)
        .build();

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    overlay.add_overlay(&placeholder);
    overlay.add_overlay(&picture);

    if let Some(url) = url {
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
    overlay
}

fn plot_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .halign(gtk::Align::Start)
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

/// "genre · rating · date" from whichever parts are present.
fn meta_line(parts: &[Option<String>]) -> Option<String> {
    let joined: Vec<&str> = parts.iter().flatten().map(String::as_str).collect();
    (!joined.is_empty()).then(|| joined.join(" · "))
}

fn content_card(
    item: &Item,
    favs: &[String],
    category_label: Option<&str>,
    win: glib::WeakRef<Window>,
) -> gtk::FlowBoxChild {
    card_body(
        &item.name,
        item.icon.as_deref(),
        Some(item.kind),
        Some(item.id),
        favs,
        category_label,
        win,
    )
}

/// Fallback card for a favorited name that no longer matches a catalog
/// item: placeholder art, no cover, no kind action.
fn name_card(name: &str, favs: &[String], win: glib::WeakRef<Window>) -> gtk::FlowBoxChild {
    card_body(name, None, None, None, favs, None, win)
}

#[allow(clippy::too_many_arguments)]
fn card_body(
    name: &str,
    icon: Option<&str>,
    kind: Option<Kind>,
    item_id: Option<i64>,
    favs: &[String],
    category_label: Option<&str>,
    win: glib::WeakRef<Window>,
) -> gtk::FlowBoxChild {
    let is_fav = favs.iter().any(|n| n == name);

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

    // Hover scrim over the art area: dark backdrop giving the action
    // buttons contrast; it fades in via CSS :hover. The heart keeps the
    // .heart-btn styling (the scrim replaces the old .card-heart chip).
    let heart = gtk::Button::builder()
        .icon_name("rtv-heart-symbolic")
        .tooltip_text("Toggle favorite")
        .css_classes(["flat", "heart-btn"])
        .build();
    set_heart_active(&heart, is_fav);
    let actions = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    actions.append(&heart);
    if let Some(kind) = kind {
        let action = match kind {
            Kind::Live => gtk::Button::builder()
                .icon_name("media-playback-start-symbolic")
                .tooltip_text("Play")
                .css_classes(["flat"])
                .build(),
            Kind::Series | Kind::Vod => gtk::Button::builder()
                .icon_name("view-more-symbolic")
                .tooltip_text("Details")
                .css_classes(["flat"])
                .build(),
        };
        let win = win.clone();
        let card_name = name.to_string();
        match kind {
            Kind::Live => {
                action.connect_clicked(move |_| {
                    if let Some(win) = win.upgrade() {
                        win.play(&card_name);
                    }
                });
            }
            Kind::Series | Kind::Vod => {
                // Details push the content detail page (series seasons or
                // movie info); the id resolves the item in the catalog.
                if let Some(id) = item_id {
                    let win = win.clone();
                    action.connect_clicked(move |_| {
                        if let Some(win) = win.upgrade() {
                            win.open_details(kind, id, &card_name);
                        }
                    });
                }
            }
        }
        actions.append(&action);
    }
    let scrim = gtk::Box::builder().css_classes(["card-scrim"]).build();
    scrim.append(&actions);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    overlay.add_overlay(&placeholder);
    overlay.add_overlay(&picture);
    overlay.add_overlay(&scrim);

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

    let card_name = name.to_string();
    heart.connect_clicked(move |button| {
        if let Some(win) = win.upgrade() {
            let is_fav = win.toggle_fav(&card_name);
            set_heart_active(button, is_fav);
        }
    });

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
