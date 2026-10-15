use gtk::{gdk, gio, prelude::*};

use crate::ui;

pub fn run() {
    crate::register_resources();

    let app = adw::Application::new(Some("io.github.tunix.rtv"), gio::ApplicationFlags::empty());

    app.connect_activate(|app| {
        register_icon_theme();

        if let Some(existing) = app.active_window() {
            existing.present();
            return;
        }

        let window = ui::window::Window::new(app);
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
