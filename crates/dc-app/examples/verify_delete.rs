//! Verifies the real deletion path of archived Codex sessions on a COPY of the store.
//!
//! Refuses to run unless `CODEX_HOME` is set and equals `<workspace>\target\verify-codexhome`,
//! so it can never touch the real `~/.codex`. Deletes two archived sessions through the Codex
//! CLI: the first permanently, the second after a copy went to the Recycle Bin.
//!
//! `set CODEX_HOME=...\target\verify-codexhome` then
//! `cargo run -p dc-app --example verify_delete`
use dc_model::{Agent, RemoveMode};
use dc_mukade::MukadeSessions;
use dc_sessions::{SystemCli, SystemTrash, delete_sessions};
use std::path::{Path, PathBuf};

fn main() {
    let expected: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("verify-codexhome");
    let expected = std::fs::canonicalize(&expected).unwrap_or_else(|_| {
        eprintln!("scratch folder missing: {}", expected.display());
        std::process::exit(2);
    });
    let Some(home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) else {
        eprintln!("CODEX_HOME is not set: refusing to run");
        std::process::exit(2);
    };
    let home = std::fs::canonicalize(PathBuf::from(home)).unwrap_or_else(|e| {
        eprintln!("CODEX_HOME is not readable: {e}");
        std::process::exit(2);
    });
    assert_eq!(home, expected, "CODEX_HOME must be the scratch folder, refusing to run");

    let listed = MukadeSessions::from_env(Agent::Codex).archived();
    for w in &listed.warnings {
        println!("warning: {w}");
    }
    let removable: Vec<_> = listed.items.into_iter().filter(|s| s.blocked.is_none()).take(2).collect();
    assert_eq!(removable.len(), 2, "need at least two removable archived sessions");
    for s in &removable {
        println!("picked {} file={:?} bytes={}", s.id, s.file, s.bytes);
    }

    let cli = SystemCli::locate();
    println!("codex cli: {:?}", cli.codex);
    let first = delete_sessions(&removable[..1], RemoveMode::Permanent, &cli, &SystemTrash);
    println!("permanent: {first:?}");
    let second = delete_sessions(&removable[1..], RemoveMode::Recycle, &cli, &SystemTrash);
    println!("recycle:   {second:?}");
}
