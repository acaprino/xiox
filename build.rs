//! Incorpora il manifest di Windows. L'exe di rilascio parte sempre da amministratore;
//! le build di sviluppo e i test usano `asInvoker`, così `cargo test` e `cargo run`
//! funzionano da una shell senza elevazione.

fn main() {
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let (rc, manifest) = if release {
        ("xiox-ui.rc", "xiox-ui.manifest")
    } else {
        ("xiox-ui.dev.rc", "xiox-ui.dev.manifest")
    };
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={rc}");
    println!("cargo:rerun-if-changed={manifest}");
    embed_resource::compile(rc, embed_resource::NONE).manifest_required().unwrap();
}
