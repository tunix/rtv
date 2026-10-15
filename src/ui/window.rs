use adw::prelude::*;
use gtk::{gio, glib};

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

mod imp {
    use adw::subclass::prelude::*;
    use gtk::{glib, TemplateChild};
    use std::cell::Cell;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/tunix/rtv/window.ui")]
    pub struct Window {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub channel_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub channel_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub channel_status: TemplateChild<gtk::Label>,
        #[template_child]
        pub play_button: TemplateChild<gtk::Button>,

        /// UI-mock state: whether the fake player is "playing".
        pub playing: Cell<bool>,
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
        window
    }

    fn imp(&self) -> &imp::Window {
        glib::subclass::types::ObjectSubclassIsExt::imp(self)
    }

    fn setup_actions(&self) {
        let win = self.downgrade();
        let add_source = gio::SimpleAction::new("add-source", None);
        add_source.connect_activate(move |_, _| {
            if let Some(win) = win.upgrade() {
                win.imp()
                    .toast_overlay
                    .add_toast(adw::Toast::new("Source setup is not implemented yet"));
            }
        });
        self.add_action(&add_source);

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

    /// UI-mock wiring: selecting a channel row updates the content header and
    /// the info row; the play button only toggles a fake playing state.
    pub fn setup_mock_handlers(&self) {
        let imp = self.imp();

        let win = self.downgrade();
        imp.channel_list.connect_row_activated(move |_, row| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let Some(box_) = row.child().and_downcast::<gtk::Box>() else {
                return;
            };
            // First Gtk.Label child inside the row holds the channel name.
            let mut name = None;
            let mut child = box_.first_child();
            while let Some(widget) = child {
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    name = Some(label.label().to_string());
                    break;
                }
                child = widget.next_sibling();
            }
            let Some(name) = name else { return };

            let i = win.imp();
            i.content_page.set_title(&name);
            i.channel_title.set_label(&name);
            i.channel_status.set_label("Live · 1080p");
        });

        let win = self.downgrade();
        imp.play_button.connect_clicked(move |button| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let i = win.imp();
            i.playing.set(!i.playing.get());
            let playing = i.playing.get();
            if let Some(image) = button.child().and_downcast::<gtk::Image>() {
                image.set_icon_name(Some(if playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                }));
            }
            i.channel_status.set_label(if playing {
                "Playing (mock)"
            } else {
                "Live · 1080p"
            });
        });
    }
}
