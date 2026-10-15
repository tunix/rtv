use adw::prelude::*;
use gtk::{gio, glib};

/// Static (blueprint) channel row: name, its heart button, category group.
#[derive(Debug)]
pub struct ChannelRow {
    row: gtk::ListBoxRow,
    name: String,
    heart: gtk::Button,
    group: i32,
}

/// Static category header row: the whole row is a click target that
/// collapses/expands its group.
#[derive(Debug)]
pub struct HeaderRow {
    row: gtk::ListBoxRow,
    chevron: gtk::Image,
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

    use super::{ChannelRow, HeaderRow};

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
        pub channel_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub player_heart: TemplateChild<gtk::Button>,
        #[template_child]
        pub play_button: TemplateChild<gtk::Button>,
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
        pub channel_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub channel_status: TemplateChild<gtk::Label>,

        /// UI-mock state.
        pub playing: Cell<bool>,
        pub muted: Cell<bool>,
        pub has_source: Cell<bool>,
        pub last_selected: Cell<u32>,
        pub favorites: RefCell<Vec<String>>,
        pub current: RefCell<Option<String>>,
        pub collapsed: RefCell<Vec<bool>>,
        pub query: RefCell<String>,
        pub channels: RefCell<Vec<ChannelRow>>,
        pub headers: RefCell<Vec<HeaderRow>>,
        pub dropdown: RefCell<Option<gtk::DropDown>>,
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

        window.setup_actions();
        window.setup_browse();
        window
    }

    fn imp(&self) -> &imp::Window {
        glib::subclass::types::ObjectSubclassIsExt::imp(self)
    }

    fn toast(&self, msg: &str) {
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
            for c in imp.channels.borrow().iter() {
                set_heart_active(&c.heart, false);
            }
            *imp.current.borrow_mut() = None;
            win.rebuild_favorites();
            imp.main_view.pop_to_page(&*imp.welcome_page);
        });
        self.add_action(&reset);

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

        // Source picker, valhalla-style, in the header title area.
        let model = gtk::StringList::new(&["Xtream — Home Panel", "M3U — Trial List"]);
        let dropdown = gtk::DropDown::new(Some(model), None::<gtk::Expression>);
        dropdown.set_selected(0);
        imp.dropdown_box.append(&dropdown);
        imp.dropdown.replace(Some(dropdown.clone()));
        imp.last_selected.set(0);
        let win = self.downgrade();
        dropdown.connect_selected_notify(move |dd| {
            let Some(win) = win.upgrade() else { return };
            let sel = dd.selected();
            // The property also notifies outside user interaction; only
            // toast genuine switches, and name the ACTUALLY selected item.
            if sel == win.imp().last_selected.get() {
                return;
            }
            win.imp().last_selected.set(sel);
            let Some(item) = dd.selected_item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            win.toast(&format!("Source switched (mock): {}", item.string()));
        });

        // Classify the static rows: category headers vs channel rows.
        let mut group: i32 = -1;
        let mut child = imp.channel_list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>() else {
                continue;
            };
            let Some(box_) = row.child().and_downcast::<gtk::Box>() else {
                continue;
            };
            let is_header = box_
                .first_child()
                .and_downcast::<gtk::Image>()
                .and_then(|i| i.icon_name())
                .map(|n| n.starts_with("chevron"))
                .unwrap_or(false);
            if is_header {
                group += 1;
                let chevron = box_
                    .first_child()
                    .and_downcast::<gtk::Image>()
                    .expect("header chevron");
                let win = self.downgrade();
                let gesture = gtk::GestureClick::new();
                gesture.connect_pressed(move |_, _, _, _| {
                    if let Some(win) = win.upgrade() {
                        win.toggle_group(group);
                    }
                });
                row.add_controller(gesture);
                imp.headers.borrow_mut().push(HeaderRow {
                    row: row.clone(),
                    chevron: chevron.clone(),
                    group,
                });
            } else {
                let name = first_label(&box_).unwrap_or_default();
                let heart = row_heart(&box_);
                let win = self.downgrade();
                let closure_name = name.clone();
                heart.connect_clicked(move |_| {
                    if let Some(win) = win.upgrade() {
                        win.toggle_fav(&closure_name);
                    }
                });
                imp.channels.borrow_mut().push(ChannelRow {
                    row: row.clone(),
                    name,
                    heart,
                    group,
                });
            }
        }

        // Channel activation -> full-window player.
        let win = self.downgrade();
        imp.channel_list.connect_row_activated(move |_, row| {
            if let Some(win) = win.upgrade() {
                if let Some(name) = row_label(row) {
                    win.play(&name);
                }
            }
        });

        // Favorites activation -> the same player.
        let win = self.downgrade();
        imp.favorites_list.connect_row_activated(move |_, row| {
            if let Some(win) = win.upgrade() {
                if let Some(name) = row_label(row) {
                    win.play(&name);
                }
            }
        });

        // Search across favorites + channels.
        let win = self.downgrade();
        imp.search_entry.connect_search_changed(move |entry| {
            if let Some(win) = win.upgrade() {
                win.apply_search(&entry.text());
            }
        });

        // Player controls.
        let win = self.downgrade();
        imp.play_button.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_playing();
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

        // Player control bar (mock): seek, play/pause, mute, fullscreen.
        // Volume gets its state from the VolumeButton itself.
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
        imp.play_button_small.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.toggle_playing();
            }
        });
        let win = self.downgrade();
        imp.mute_button.connect_clicked(move |_| {
            let Some(win) = win.upgrade() else { return };
            let imp = win.imp();
            imp.muted.set(!imp.muted.get());
            imp.mute_button.set_icon_name(if imp.muted.get() {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            });
        });
        let win = self.downgrade();
        imp.fullscreen_button.connect_clicked(move |_| {
            let Some(win) = win.upgrade() else { return };
            if win.is_fullscreen() {
                win.unfullscreen();
            } else {
                win.fullscreen();
            }
        });
        // The fullscreen icon follows the real window state.
        self.connect_fullscreened_notify(|win| {
            win.imp()
                .fullscreen_button
                .set_icon_name(if win.is_fullscreen() {
                    "view-restore-symbolic"
                } else {
                    "view-fullscreen-symbolic"
                });
        });

        self.rebuild_favorites();
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
        *self.imp().query.borrow_mut() = text.to_string();
        self.refresh_rows_visibility();
    }

    /// Visibility of every sidebar row, driven by the search query and the
    /// per-group collapse state. While searching, groups are implicitly
    /// expanded (matching rows show even in collapsed groups).
    fn refresh_rows_visibility(&self) {
        let imp = self.imp();
        let q = imp.query.borrow().to_lowercase();

        if q.is_empty() {
            for c in imp.channels.borrow().iter() {
                c.row.set_visible(!self.collapsed(c.group));
            }
            for h in imp.headers.borrow().iter() {
                h.row.set_visible(true);
            }
        } else {
            let n_groups = imp.headers.borrow().len();
            let mut counts = vec![0usize; n_groups];
            for c in imp.channels.borrow().iter() {
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

    fn toggle_fav(&self, name: &str) {
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
        for c in imp.channels.borrow().iter() {
            if c.name == name {
                set_heart_active(&c.heart, is_fav);
            }
        }
        if imp.current.borrow().as_deref() == Some(name) {
            set_heart_active(&imp.player_heart, is_fav);
        }
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
        imp.channel_title.set_label(name);
        imp.channel_status.set_label("Live · 1080p");
        imp.playing.set(false);
        self.set_play_icon(false);
        let is_fav = imp.favorites.borrow().iter().any(|n| n == name);
        set_heart_active(&imp.player_heart, is_fav);
        imp.main_view.push(&*imp.player_page);
    }

    fn toggle_playing(&self) {
        let imp = self.imp();
        imp.playing.set(!imp.playing.get());
        self.set_play_icon(imp.playing.get());
        imp.channel_status.set_label(if imp.playing.get() {
            "Playing (mock)"
        } else {
            "Live · 1080p"
        });
    }

    /// Keep the big centered play button and the bar's play button in sync.
    fn set_play_icon(&self, playing: bool) {
        let imp = self.imp();
        let icon = if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        };
        if let Some(img) = imp.play_button.child().and_downcast::<gtk::Image>() {
            img.set_icon_name(Some(icon));
        }
        imp.play_button_small.set_icon_name(icon);
    }
}

fn set_heart_active(button: &gtk::Button, active: bool) {
    if active {
        button.add_css_class("active");
    } else {
        button.remove_css_class("active");
    }
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

fn row_heart(container: &gtk::Box) -> gtk::Button {
    let mut found = None;
    let mut child = container.first_child();
    while let Some(widget) = child {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            found = Some(button.clone());
        }
        child = widget.next_sibling();
    }
    found.expect("channel row has a heart button")
}

fn row_label(row: &gtk::ListBoxRow) -> Option<String> {
    let container = row.child().and_downcast::<gtk::Box>()?;
    first_label(&container)
}
