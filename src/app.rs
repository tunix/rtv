use gtk::{gdk, gio, prelude::*};

use crate::ui;

pub fn run() {
    crate::register_resources();

    let app = adw::Application::new(Some("io.github.tunix.rtv"), gio::ApplicationFlags::empty());

    app.connect_activate(|app| {
        register_icon_theme();
        load_css();

        if let Some(existing) = app.active_window() {
            existing.present();
            return;
        }

        let window = ui::window::Window::new(app);
        window.setup_mock_handlers();
        window.present();
    });

    let quit = gio::SimpleAction::new("quit", None);
    {
        let app = app.clone();
        quit.connect_activate(move |_, _| app.quit());
    }
    app.add_action(&quit);

    app.run();
}

fn register_icon_theme() {
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/io/github/tunix/rtv/icons");
    }
}

fn load_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(".player { background: #171723; border-radius: 16px; }\n");
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
