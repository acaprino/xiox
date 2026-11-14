//! Puts the prebuilt `vendor/mukade/mukade.exe` beside the executables of this build
//! (`target/<profile>/`), where [`mukade_exe`](src/lib.rs) looks first. Nothing is compiled.

use std::path::{Path, PathBuf};

fn main() {
    let name = if std::env::var("CARGO_CFG_WINDOWS").is_ok() { "mukade.exe" } else { "mukade" };
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/mukade").join(name);
    println!("cargo:rerun-if-changed={}", src.display());
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let Some(profile_dir) = PathBuf::from(out).ancestors().nth(3).map(Path::to_path_buf) else { return };
    if src.is_file() {
        let dst = profile_dir.join(name);
        // A running copy is locked on Windows: keep it and say so.
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!("cargo:warning=could not copy {} to {}: {e}", src.display(), dst.display());
        }
    } else {
        println!("cargo:warning={} is missing: sessions will not be listed", src.display());
    }
}
