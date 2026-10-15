mod app;
mod ui;

use gtk::gio;

fn main() {
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
