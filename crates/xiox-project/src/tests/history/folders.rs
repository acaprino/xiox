//! What one session's ledger adds to the evidence kept per folder.

use std::path::Path;

use serde_json::json;
use xiox_model::{path_key, Agent};
use xiox_mukade::ToolCall;

use super::fake::{bash, call, descriptor};
use crate::history::{Folders, UserDirs};
use crate::provenance::session_ledgers;
use crate::*;

/// Folds the calls of one session into a fresh `Folders`, as the reading stage does.
fn folded(calls: &[ToolCall]) -> (Folders, Coverage) {
    let d = descriptor(Agent::Claude, "main", "s1", r"D:\p", "2026-11-10T10:00:00Z", None);
    let ledgers = session_ledgers(calls, &UserDirs::default().path_env());
    let mut folders = Folders::default();
    let mut coverage = Coverage::default();
    for ledger in ledgers.values() {
        folders.add_ledger(&d, ledger);
        coverage = ledger.coverage().clone();
    }
    (folders, coverage)
}

fn evidence_of(folders: &Folders, dir: &str) -> Option<SessionEvidence> {
    let provenance = folders.provenance_within(&path_key(Path::new(dir)))?;
    assert_eq!(provenance.sessions.len(), 1, "{provenance:?}");
    provenance.sessions.into_iter().next()
}

#[test]
fn failed_create_keeps_only_failed_refs() {
    let mut mkdir = bash("mkdir D:/runs/x", r"D:\p");
    mkdir.error = Some(true);

    let (folders, _) = folded(&[mkdir]);

    let e = evidence_of(&folders, r"D:\runs\x").expect("the failed attempt is still evidence");
    assert_eq!((e.strength, e.failed), (None, 1));
    assert!(e.operations.is_empty(), "{:?}", e.operations);
}

#[test]
fn operations_are_capped_and_counted() {
    let writes: Vec<ToolCall> = (0..25)
        .map(|i| call("write", "Write", json!({ "file_path": format!(r"D:\runs\x\f{i}.txt") }), Some(r"D:\p")))
        .collect();

    let (folders, _) = folded(&writes);

    let e = evidence_of(&folders, r"D:\runs\x").unwrap();
    assert_eq!((e.operations.len(), e.omitted), (20, 5));
    assert_eq!(e.strength, Some(Strength::Written));
}

#[test]
fn relative_path_without_context_stays_unresolved() {
    // After a `cd` into an unknown place, `out` has no base to be resolved against.
    let (folders, coverage) = folded(&[bash("cd $UNKNOWN_DIR && mkdir out", r"D:\p")]);

    // Only the call's own working directory is evidence; `out` is placed nowhere.
    assert!(folders.provenance_within(&path_key(Path::new(r"D:\p\out"))).is_none());
    assert!(folders.provenance_within(&path_key(Path::new(r"D:\out"))).is_none());
    assert!(coverage.unresolved >= 1, "{coverage:?}");
}

#[test]
fn shell_gaps_are_marked_on_the_evidence() {
    let mut calls = vec![bash("mkdir D:/runs/x", r"D:\p")];
    let (folders, _) = folded(&calls);
    assert!(!evidence_of(&folders, r"D:\runs\x").unwrap().gaps);

    // A command the parsers cannot resolve makes the whole session's coverage partial.
    calls.push(bash("cd $UNKNOWN_DIR && mkdir out", r"D:\p"));
    let (folders, coverage) = folded(&calls);
    assert!(coverage.unresolved >= 1, "{coverage:?}");
    assert!(evidence_of(&folders, r"D:\runs\x").unwrap().gaps);
}
