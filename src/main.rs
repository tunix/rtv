mod app;
mod ui;

use gtk::gio;
use gtk::gio::prelude::*;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--check-icons") {
        register_resources();
        gtk::init().expect("gtk init");

        // Mechanism: export to XDG_RUNTIME_DIR + display-theme search path.
        crate::app::install_runtime_icons();
        let display_theme =
            gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("display"));
        for name in ["io.github.tunix.rtv", "rtv-heart-symbolic"] {
            let icon = display_theme.lookup_icon(
                name,
                &[],
                16,
                1,
                gtk::TextDirection::Ltr,
                gtk::IconLookupFlags::FORCE_REGULAR,
            );
            println!("{name}: {:?}", icon.file().map(|f| f.uri().to_string()));
        }
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,rtv=debug")),
        )
        .with_target(false)
        .init();

    app::run();
}

fn register_resources() {
    let resource = gio::Resource::load(concat!(env!("OUT_DIR"), "/rtv.gresource"))
        .expect("failed to load compiled GResource");
    gio::resources_register(&resource);
}
