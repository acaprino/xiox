//! Embeds the Windows manifest: the release exe always asks for administrator rights;
//! debug builds and test binaries use `asInvoker`, so `cargo test` and `cargo run`
//! work from a shell without elevation.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let manifest = if release { "xiox-ui.manifest" } else { "xiox-ui.dev.manifest" };
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=xiox-ui.manifest");
    println!("cargo:rerun-if-changed=xiox-ui.dev.manifest");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=capabilities");
    let windows = tauri_build::WindowsAttributes::new().app_manifest(std::fs::read_to_string(manifest)?);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))?;
    Ok(())
}
