use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use crate::sample::{self, Item, Kind};

/// Search results are capped so a one-letter query cannot flood the grid;
/// the backend phase will replace this with real incremental search.
const SEARCH_CAP: usize = 500;
/// Cards appended to a grid per idle tick (progressive rendering).
const FILL_CHUNK: usize = 40;

/// Sidebar section header row: chevron + label, click toggles collapse.
#[derive(Debug)]
pub struct HeaderRow {
    row: gtk::ListBoxRow,
    chevron: gtk::Image,
    group: i32,
}

/// Sidebar category row: activating it shows the category's content grid.
#[derive(Debug)]
pub struct CategoryRow {
    row: gtk::ListBoxRow,
    category_index: usize,
    name: String,
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
        pub favorites_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub favorites_list: TemplateChild<gtk::ListBox>,
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
        pub search_grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub content_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub content_status: TemplateChild<gtk::Label>,

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
        pub dropdown: RefCell<Option<gtk::DropDown>>,
        /// Bumped on every show/cancel; only the timer matching the latest
        /// generation may hide the controls (stale timers must no-op, since
        /// `SourceId::remove` panics on already-fired ids).
        pub controls_generation: Cell<u64>,
        /// Bumped on every grid (re)fill; stale chunk fillers no-op.
        pub fill_generation: Cell<u64>,
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
        // First run: adding a (mock) source unlocks browsing. Later this
        // opens the real source-setup dialog.
        let win = self.downgrade();
        let add_source = gio::SimpleAction::new("add-source", None);
        add_source.connect_activate(move |_, _| {
            let Some(win) = win.upgrade() else { return };
            let imp = win.imp();
            if !imp.has_source.get() {
                imp.has_source.set(true);
                imp.main_view.push(&*imp.browse_page);
                win.toast("Mock source added — the real setup dialog comes later");
            } else {
                win.toast("Source dialog comes later — the mock already has 2 sources");
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
                dialog.set_developer_name("Tunix");
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

        self.setup_sidebar();

        // Sidebar category activation -> content grid in the middle.
        let win = self.downgrade();
        imp.content_list.connect_row_activated(move |_, row| {
            let Some(win) = win.upgrade() else { return };
            let i = win.imp();
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

        // Favorites activation -> the player.
        let win = self.downgrade();
        imp.favorites_list.connect_row_activated(move |_, row| {
            if let Some(win) = win.upgrade() {
                if let Some(name) = row_label(row) {
                    win.play(&name);
                }
            }
        });

        // Grid cards -> the player.
        let win = self.downgrade();
        imp.content_grid.connect_child_activated(move |_, child| {
            if let Some(win) = win.upgrade() {
                if let Some(b) = child.child().and_downcast::<gtk::Box>() {
                    if let Some(name) = first_label(&b) {
                        win.play(&name);
                    }
                }
            }
        });
        let win = self.downgrade();
        imp.search_grid.connect_child_activated(move |_, child| {
            if let Some(win) = win.upgrade() {
                if let Some(b) = child.child().and_downcast::<gtk::Box>() {
                    if let Some(name) = first_label(&b) {
                        win.play(&name);
                    }
                }
            }
        });

        // Search across the whole catalog.
        let win = self.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(win) = win.upgrade() {
                win.apply_search(&entry.text());
            }
        });

        // Leaving the player page (back button / pop) resets player state.
        let win = self.downgrade();
        imp.main_view.connect_popped(move |_, _| {
            if let Some(win) = win.upgrade() {
                let i = win.imp();
                i.on_player.set(false);
                win.cancel_hide();
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
            let chevron = gtk::Image::builder()
                .icon_name("chevron-down-symbolic")
                .pixel_size(14)
                .valign(gtk::Align::Center)
                .build();
            header_box.append(&chevron);
            header_box.append(
                &gtk::Label::builder()
                    .label(format!("{} · {}", kind.title(), cats.len()))
                    .xalign(0.0)
                    .css_classes(["caption", "dim-label"])
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
            imp.headers.borrow_mut().push(HeaderRow {
                row: header,
                chevron,
                group,
            });

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
                    name: cat.name.clone(),
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
        let open = !self.collapsed(group);
        for h in imp.headers.borrow().iter() {
            if h.group == group {
                h.chevron.set_icon_name(Some(if open {
                    "chevron-down-symbolic"
                } else {
                    "chevron-right-symbolic"
                }));
            }
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
        self.fill_grid(indices, &imp.search_grid, &favs);
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
            let n_groups = imp.headers.borrow().len();
            let mut counts = vec![0usize; n_groups];
            for c in imp.categories.borrow().iter() {
                let matches = c.name.to_lowercase().contains(&q);
                c.row.set_visible(matches);
                if matches && c.group >= 0 {
                    counts[c.group as usize] += 1;
                }
            }
            for h in imp.headers.borrow().iter() {
                h.row.set_visible(counts[h.group as usize] > 0);
            }
        }

        // Favorites follow the same query.
        let mut child = imp.favorites_list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>() {
                let matches = q.is_empty()
                    || row_label(row)
                        .map(|n| n.to_lowercase().contains(&q))
                        .unwrap_or(false);
                row.set_visible(matches);
            }
        }
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
        self.fill_grid(indices, &imp.content_grid, &favs);
    }

    /// Fill a grid progressively: FILL_CHUNK cards per idle tick so a
    /// 1800-card category never blocks the UI. A generation counter makes
    /// stale fillers (category switch, new search) stop immediately.
    fn fill_grid(&self, mut indices: Vec<usize>, flowbox: &gtk::FlowBox, favs: &[String]) {
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
                flowbox.append(&content_card(item, &favs, win.clone()));
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

    /// Rebuild the pinned Favorites section from the in-memory list; the
    /// section stays hidden while empty (favorites sit above categories or
    /// don't show at all).
    fn rebuild_favorites(&self) {
        let imp = self.imp();
        while let Some(child) = imp.favorites_list.first_child() {
            imp.favorites_list.remove(&child);
        }
        let empty = imp.favorites.borrow().is_empty();
        imp.favorites_title.set_visible(!empty);
        imp.favorites_list.set_visible(!empty);

        for name in imp.favorites.borrow().iter() {
            let row = gtk::ListBoxRow::new();
            let b = gtk::Box::builder().spacing(12).build();
            let heart = gtk::Button::builder()
                .icon_name("rtv-heart-symbolic")
                .tooltip_text("Remove from favorites")
                .css_classes(["flat", "heart-btn", "active"])
                .valign(gtk::Align::Center)
                .build();
            let label = gtk::Label::builder()
                .label(name)
                .halign(gtk::Align::Start)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            b.append(&heart);
            b.append(&label);
            row.set_child(Some(&b));

            let win = self.downgrade();
            let name = name.clone();
            heart.connect_clicked(move |_| {
                if let Some(win) = win.upgrade() {
                    win.toggle_fav(&name);
                }
            });
            imp.favorites_list.append(&row);
        }
        self.refresh_rows_visibility();
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

fn content_card(item: &Item, favs: &[String], win: glib::WeakRef<Window>) -> gtk::FlowBoxChild {
    let is_fav = favs.iter().any(|n| n == &item.name);

    let art = gtk::Box::builder()
        .css_classes(["card"])
        .width_request(150)
        .height_request(90)
        .build();
    art.append(
        &gtk::Image::builder()
            .icon_name("video-display-symbolic")
            .pixel_size(56)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build(),
    );
    let heart = gtk::Button::builder()
        .icon_name("rtv-heart-symbolic")
        .tooltip_text("Toggle favorite")
        .css_classes(["flat", "heart-btn"])
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(2)
        .margin_end(2)
        .build();
    set_heart_active(&heart, is_fav);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&art));
    overlay.add_overlay(&heart);

    let label = gtk::Label::builder()
        .label(&item.name)
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

    let card_name = item.name.clone();
    heart.connect_clicked(move |button| {
        if let Some(win) = win.upgrade() {
            let is_fav = win.toggle_fav(&card_name);
            set_heart_active(button, is_fav);
        }
    });

    gtk::FlowBoxChild::builder().child(&b).build()
}

fn first_label(container: &gtk::Box) -> Option<String> {
    let mut child = container.first_child();
    while let Some(widget) = child {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return Some(label.label().to_string());
        }
        child = widget.next_sibling();
    }
    None
}

fn row_label(row: &gtk::ListBoxRow) -> Option<String> {
    let container = row.child().and_downcast::<gtk::Box>()?;
    first_label(&container)
}

fn set_heart_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class("active");
    } else {
        button.remove_css_class("active");
    }
}
