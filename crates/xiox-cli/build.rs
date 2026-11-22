//! Embeds the Windows manifest (`asInvoker`, so `xiox` never asks for elevation) and the icon, Imago's copy of the app's.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=xiox.rc");
    println!("cargo:rerun-if-changed=xiox.manifest");
    println!("cargo:rerun-if-changed=icon.ico");
    embed_resource::compile("xiox.rc", embed_resource::NONE).manifest_required().unwrap();
}
