use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    // 1. Compile Blueprint markup into GtkBuilder XML.
    let src = manifest.join("resources").join("window.blp");
    let dest = out.join("window.ui");
    println!("cargo:rerun-if-changed={}", src.display());
    let file = fs::File::create(&dest).expect("failed to create ui file");
    let status = Command::new("blueprint-compiler")
        .arg("compile")
        .arg(&src)
        .stdout(Stdio::from(file))
        .status()
        .expect("blueprint-compiler not found; install it (dnf install blueprint-compiler)");
    assert!(status.success(), "blueprint-compiler failed on window.blp");

    // 2. Bundle the GSettings schema so Settings::new() works without a system install.
    let schema = "io.github.tunix.rtv.gschema.xml";
    fs::copy(manifest.join("data").join(schema), out.join(schema)).expect("copy gschema");
    println!("cargo:rerun-if-changed=data/{schema}");

    // 3. Generate and compile the GResource bundle.
    let icon_dir = manifest.join("data/icons/hicolor/scalable/apps");
    println!(
        "cargo:rerun-if-changed={}",
        icon_dir.join("io.github.tunix.rtv.svg").display()
    );
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><gresources><gresource prefix="/io/github/tunix/rtv">"#,
    );
    xml.push_str(r#"<file compressed="true" alias="window.ui">window.ui</file>"#);
    xml.push_str(&format!(
        r#"<file compressed="true" alias="{schema}">{schema}</file>"#
    ));
    xml.push_str(
        r#"<file compressed="true" alias="icons/hicolor/scalable/apps/io.github.tunix.rtv.svg">io.github.tunix.rtv.svg</file>"#,
    );
    xml.push_str(
        r#"<file compressed="true" alias="icons/hicolor/scalable/apps/rtv-heart-symbolic.svg">rtv-heart-symbolic.svg</file>"#,
    );
    xml.push_str("</gresource></gresources>");
    let gxml = out.join("rtv.gresources.xml");
    fs::write(&gxml, xml).expect("write gresource xml");

    glib_build_tools::compile_resources(&[out, icon_dir], gxml.to_str().unwrap(), "rtv.gresource");

    // 4. Install the schema into the user schema dir so dev runs (outside
    //    Flatpak) find it via dconf.
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = env::var_os("HOME").map(PathBuf::from).expect("HOME");
            home.join(".local/share")
        });
    let user_schema_dir = data_home.join("glib-2.0/schemas");
    fs::create_dir_all(&user_schema_dir).expect("create user schema dir");
    let installed = user_schema_dir.join(schema);
    let changed = fs::read(&installed)
        .map(|old| old != fs::read(manifest.join("data").join(schema)).unwrap())
        .unwrap_or(true);
    if changed {
        fs::copy(manifest.join("data").join(schema), &installed).expect("install gschema");
        let status = Command::new("glib-compile-schemas")
            .arg(&user_schema_dir)
            .status()
            .expect("glib-compile-schemas not found");
        assert!(status.success(), "glib-compile-schemas failed");
    }
}
