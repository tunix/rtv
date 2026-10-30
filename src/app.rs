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

    // The fallback window title (alt-tab for unmatched windows) — libadwaita
    // ignores GtkWindow:title on AdwWindow, so GTK would otherwise fall
    // back to the raw application id.
    glib::set_application_name("RTV");

    let app = adw::Application::new(Some("io.github.tunix.rtv"), gio::ApplicationFlags::empty());

    // Classic media-player shortcuts. Play/pause is intentionally NOT
    // here: a global space accelerator would swallow the space key while
    // typing in the search entry, so it is scoped to the player page in
    // window.rs instead.
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
        if window.sample_catalog_missing() {
            window.toast("samples/ not found — mock catalog is empty");
        }
    });

    let quit = gio::SimpleAction::new("quit", None);
    {
        let app = app.clone();
        quit.connect_activate(move |_, _| app.quit());
    }
    app.add_action(&quit);

    // GApplication parses argv for its own options and aborts on unknown
    // flags like --mock (glib "Unknown option", exit 0 before our loop
    // even mattered). Run with ONLY the program name so the app-level
    // option parsing sees nothing; --mock was already consumed in main().
    app.run_with_args(&["rtv"][..]);
}

fn load_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(concat!(
        ".player { background: #171723; border-radius: 16px; }\n",
        ".player.video-fill { border-radius: 0; }\n",
        // The player chrome adapts to the theme: a light pill with dark
        // controls in light mode, dark pill with light controls in dark
        // mode — always readable over the video.
        ".controls {\n",
        "  background: alpha(@window_bg_color, 0.92);\n",
        "  color: @window_fg_color;\n",
        "  border-radius: 12px;\n",
        "  padding: 6px 10px;\n",
        "}\n",
        ".controls button {\n",
        "  color: @window_fg_color;\n",
        "}\n",
        ".controls button:hover, .controls button:active {\n",
        "  background: alpha(@window_fg_color, 0.12);\n",
        "}\n",
        ".controls scale trough {\n",
        "  background: alpha(@window_fg_color, 0.28);\n",
        "}\n",
        ".controls scale highlight {\n",
        "  background: @accent_bg_color;\n",
        "}\n",
        ".controls scale slider {\n",
        "  background: @window_bg_color;\n",
        "  color: @window_fg_color;\n",
        "  border: 1px solid alpha(@window_fg_color, 0.4);\n",
        "}\n",
        ".controls label {\n",
        "  color: @window_fg_color;\n",
        "}\n",
        ".heart-btn { opacity: 0.55; }\n",
        ".heart-btn.active { opacity: 1; color: #e01b24; }\n",
        ".play-chip { opacity: 1; transition: opacity 160ms ease; }\n",
        ".play-chip.hover-reveal { opacity: 0; }\n",
        "row:hover .play-chip.hover-reveal { opacity: 1; }\n",
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
