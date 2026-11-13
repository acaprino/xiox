//! Embeds the Windows manifest: `asInvoker`, so `xiox` never asks for elevation.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=xiox.rc");
    println!("cargo:rerun-if-changed=xiox.manifest");
    embed_resource::compile("xiox.rc", embed_resource::NONE).manifest_required().unwrap();
}
