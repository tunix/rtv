use std::{fs, path::PathBuf};

use gtk::{gdk, gio, prelude::*};

use crate::ui;

/// Bundled (gresource) icons exported to a private runtime dir.
const RUNTIME_ICONS: &[(&str, &str)] = &[
    (
        "/io/github/tunix/rtv/icons/hicolor/scalable/apps/io.github.tunix.rtv.svg",
        "hicolor/scalable/apps/io.github.tunix.rtv.svg",
    ),
    (
        "/io/github/tunix/rtv/icons/hicolor/scalable/apps/rtv-heart-symbolic.svg",
        "hicolor/scalable/apps/rtv-heart-symbolic.svg",
    ),
];

/// GTK 4.22 (regression found via --check-icons) ignores icon-theme
/// resource paths entirely — `add_resource_path` is a no-op on both the
/// display theme and standalone themes. The bundled icons are therefore
/// exported once to a private XDG_RUNTIME_DIR tree that IS on the icon
/// theme search path, keeping icon-name usage and symbolic recoloring
/// working everywhere (buttons, StatusPage, About dialog).
pub fn install_runtime_icons() {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return;
    };
    let base = PathBuf::from(runtime).join("rtv-icons");

    for (resource, relative) in RUNTIME_ICONS {
        let Ok(data) = gio::resources_lookup_data(resource, gio::ResourceLookupFlags::NONE) else {
            continue;
        };
        let path = base.join(relative);
        if let Some(parent) = path.parent() {
            if fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        // Rewrite only on content changes so the mtime stays stable.
        let stale = fs::read(&path)
            .map(|old| old.as_slice() != data.as_ref())
            .unwrap_or(true);
        if stale {
            let _ = fs::write(&path, data.as_ref());
        }
    }

    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(base.to_string_lossy().as_ref());
    }
}

pub fn run() {
    crate::register_resources();

    let app = adw::Application::new(Some("io.github.tunix.rtv"), gio::ApplicationFlags::empty());

    // Classic media-player shortcuts; the actions no-op outside the player.
    app.set_accels_for_action("win.play-pause", &["space"]);
    app.set_accels_for_action("win.seek-back", &["Left"]);
    app.set_accels_for_action("win.seek-forward", &["Right"]);
    app.set_accels_for_action("win.toggle-mute", &["m"]);
    app.set_accels_for_action("win.toggle-fullscreen", &["f"]);
    app.set_accels_for_action("win.exit-fullscreen", &["Escape"]);

    app.connect_activate(|app| {
        install_runtime_icons();
        load_css();

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

fn load_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(concat!(
        ".player { background: #171723; border-radius: 16px; }\n",
        ".player.video-fill { border-radius: 0; }\n",
        ".heart-btn { opacity: 0.55; }\n",
        ".heart-btn.active { opacity: 1; color: #e01b24; }\n",
        ".controls { background: rgba(23, 23, 35, 0.85); border-radius: 12px; padding: 4px 6px; }\n",
        ".card { background: rgba(255, 255, 255, 0.06); border-radius: 12px; }\n",
    ));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
