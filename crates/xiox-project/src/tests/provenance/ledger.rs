use serde_json::{json, Value};
use xiox_mukade::{Outcome, ToolCall, CUT_MARK, MAX_INPUT_CHARS};

use crate::provenance::{
    folder_of, session_ledgers, Env, EvidenceRef, Ledger, OpKind, PatchAction, SessionKey, Source, Strength,
};

fn env() -> Env {
    Env { home: r"C:\Users\me".into(), temp: r"C:\Temp".into() }
}

fn call(canonical: &str, tool: &str, input: Value, cwd: Option<&str>) -> ToolCall {
    ToolCall {
        harness: "test".into(),
        store: "store".into(),
        session: "s1".into(),
        parent: None,
        entry: Some("e1".into()),
        call: Some("c1".into()),
        tool: Some(tool.into()),
        canonical: Some(canonical.into()),
        input,
        cwd: cwd.map(Into::into),
        time: Some("2026-11-15T10:00:00Z".into()),
        error: None,
        exit: None,
        result_head: Some("ok".into()),
    }
}

fn bash(command: &str, cwd: &str) -> ToolCall {
    call("bash", "Bash", json!({ "command": command }), Some(cwd))
}

/// `text` cut the way Mukade cuts long inputs: `MAX_INPUT_CHARS` characters and the cut mark.
fn cut_to_limit(text: &str) -> String {
    let mut cut: String = text.chars().take(MAX_INPUT_CHARS).collect();
    cut.push(CUT_MARK);
    cut
}

fn ledger_of(calls: &[ToolCall]) -> Ledger {
    let mut ledger = Ledger::default();
    for call in calls {
        ledger.record(call, &env());
    }
    ledger
}

fn strength(ledger: &Ledger, folder: &str) -> Option<Strength> {
    ledger.get(folder).unwrap_or_else(|| panic!("no evidence for {folder}")).strength
}

fn sources(ledger: &Ledger, folder: &str) -> Vec<Source> {
    ledger.get(folder).unwrap().refs.iter().map(|r| r.source.clone()).collect()
}

#[test]
fn strongest_evidence_wins_per_folder() {
    let ledger = ledger_of(&[
        call("read", "Read", json!({ "file_path": r"D:\r\a.txt" }), Some(r"D:\P")),
        bash("mkdir D:/r", r"D:\P"),
    ]);
    let folder = ledger.get(r"D:\r").unwrap();
    assert_eq!(folder.strength, Some(Strength::Created));
    assert_eq!(folder.refs.len(), 2);
}

#[test]
fn folder_keys_ignore_case() {
    let ledger = ledger_of(&[bash("mkdir D:/Runs/X", r"D:\P"), bash("mkdir d:/runs/x", r"D:\P")]);
    let folder = ledger.get(r"d:\RUNS\x").unwrap();
    assert_eq!(folder.path, r"D:\Runs\X");
    assert_eq!(folder.refs.len(), 2);
    assert_eq!(ledger.folders().filter(|f| f.path.to_lowercase() == r"d:\runs\x").count(), 1);
}

#[test]
fn structured_and_shell_sources_are_distinguished() {
    let ledger = ledger_of(&[
        call("write", "Write", json!({ "file_path": r"D:\w\f.txt" }), Some(r"D:\P")),
        call("glob", "Glob", json!({ "path": r"D:\g", "pattern": "*" }), Some(r"D:\P")),
        bash("mkdir D:/m", r"D:\P"),
        bash(r#"python -c "os.makedirs('D:/s')""#, r"D:\P"),
    ]);
    assert_eq!(sources(&ledger, r"D:\w"), [Source::Structured("Write".into())]);
    assert_eq!(strength(&ledger, r"D:\w"), Some(Strength::Written));
    assert_eq!(sources(&ledger, r"D:\g"), [Source::Structured("Glob".into())]);
    assert_eq!(strength(&ledger, r"D:\g"), Some(Strength::Used));
    assert_eq!(sources(&ledger, r"D:\m"), [Source::Shell { kind: OpKind::Create, script: false }]);
    assert_eq!(sources(&ledger, r"D:\s"), [Source::Shell { kind: OpKind::Create, script: true }]);
    assert_eq!(sources(&ledger, r"D:\P").first(), Some(&Source::Cwd));
}

#[test]
fn cd_evidence_is_resolved_where_it_ran() {
    let ledger = ledger_of(&[bash("cd /d/a; cd b", r"D:\P")]);
    assert_eq!(strength(&ledger, r"D:\a\b"), Some(Strength::Used));
    assert_eq!(sources(&ledger, r"D:\a\b"), [Source::Cd]);
    assert!(ledger.get(r"D:\P\b").is_none());
}

#[test]
fn patch_actions_keep_their_meaning() {
    let patch = "*** Begin Patch\n*** Add File: D:/n/new.rs\n+x\n*** Update File: src/a.rs\n@@\n-a\n+b\n\
                 *** Delete File: D:/old/x.rs\n*** Update File: D:/m/a.rs\n*** Move to: D:/k/a.rs\n@@\n-a\n+b\n*** End Patch\n";
    let ledger = ledger_of(&[call("edit", "apply_patch", json!({ "_": patch }), Some(r"D:\P"))]);
    assert_eq!(strength(&ledger, r"D:\n"), Some(Strength::Written));
    assert_eq!(sources(&ledger, r"D:\n"), [Source::Patch(PatchAction::Add)]);
    assert_eq!(strength(&ledger, r"D:\P\src"), Some(Strength::Written));
    assert_eq!(strength(&ledger, r"D:\old"), Some(Strength::Used));
    assert_eq!(sources(&ledger, r"D:\old"), [Source::Patch(PatchAction::Delete)]);
    assert_eq!(strength(&ledger, r"D:\m"), Some(Strength::Used));
    assert_eq!(sources(&ledger, r"D:\m"), [Source::Patch(PatchAction::MoveFrom)]);
    assert_eq!(strength(&ledger, r"D:\k"), Some(Strength::Written));
    // R1: a patch call gives patch evidence and no structured evidence.
    assert!(ledger.folders().all(|f| f.refs.iter().all(|r| !matches!(r.source, Source::Structured(_)))));
}

#[test]
fn a_cut_patch_drops_its_unfinished_header() {
    let text = format!("*** Begin Patch\n*** Add File: D:/a/f.rs\n*** Add File: D:/b/{}", "y".repeat(MAX_INPUT_CHARS));
    let ledger = ledger_of(&[call("edit", "apply_patch", json!({ "_": cut_to_limit(&text) }), Some(r"D:\P"))]);
    assert_eq!(strength(&ledger, r"D:\a"), Some(Strength::Written));
    assert!(ledger.folders().all(|f| !f.path.starts_with(r"D:\b")));
}

#[test]
fn verbatim_cwd_and_outcomes() {
    let ledger = ledger_of(&[call("read", "Read", json!({ "file_path": "f.txt" }), Some(r"\\?\C:\w"))]);
    let folder = ledger.get(r"C:\w").unwrap();
    assert_eq!(folder.path, r"C:\w");
    assert!(folder.refs.iter().all(|r| r.cwd.as_deref() == Some(r"C:\w")));

    let clone = |error: Option<bool>, result: Option<&str>| {
        let mut c = bash("git clone https://h/r.git D:/c", r"D:\P");
        c.error = error;
        c.result_head = result.map(Into::into);
        ledger_of(&[c])
    };
    let failed = clone(Some(true), Some("fatal"));
    let folder = failed.get(r"D:\c").unwrap();
    assert_eq!(folder.strength, None);
    assert!(folder.refs.is_empty());
    assert_eq!(folder.failed.len(), 1);
    assert_eq!(folder.failed[0].outcome, Outcome::Failed);

    let unknown = clone(None, None);
    assert_eq!(strength(&unknown, r"D:\c"), Some(Strength::Created));
    assert_eq!(unknown.get(r"D:\c").unwrap().refs[0].outcome, Outcome::Unknown);

    let done = clone(None, Some("Cloning"));
    assert_eq!(strength(&done, r"D:\c"), Some(Strength::Created));
    assert_eq!(done.get(r"D:\c").unwrap().refs[0].outcome, Outcome::Succeeded);
}

#[test]
fn same_session_id_in_two_stores() {
    let mut other = bash("mkdir D:/o", r"D:\P");
    other.store = "other".into();
    other.harness = "codex".into();
    let first = bash("mkdir D:/f", r"D:\P");
    let ledgers = session_ledgers([&first, &other], &env());
    assert_eq!(ledgers.len(), 2);
    let key = SessionKey { store: "other".into(), harness: "codex".into(), session: "s1".into() };
    let r: &EvidenceRef = &ledgers[&key].get(r"D:\o").unwrap().refs[0];
    assert_eq!(r.session, key);
    assert_eq!((r.entry.as_deref(), r.call.as_deref()), (Some("e1"), Some("c1")));
    assert_eq!(r.time.as_deref(), Some("2026-11-15T10:00:00Z"));
    assert_eq!(r.cwd.as_deref(), Some(r"D:\P"));
    assert!(ledgers.values().all(|l| l.get(r"D:\P").is_some()));
}

#[test]
fn coverage_counts_what_was_not_read() {
    let long = format!("mkdir D:/ok && echo {}", "z".repeat(MAX_INPUT_CHARS));
    let ledger = ledger_of(&[
        bash("mkdir $X/a", r"D:\P"),
        bash(r#"python -c "import os; n='q'; os.makedirs(f'D:/{n}')""#, r"D:\P"),
        bash("for d in *; do mkdir out/$d; done", r"D:\P"),
        bash(&cut_to_limit(&long), r"D:\P"),
    ]);
    let c = ledger.coverage();
    assert_eq!(c.shell_calls, 4);
    assert!(c.unresolved >= 3, "{}", c.unresolved);
    assert_eq!((c.looped, c.script, c.truncated), (1, 1, 1));
}

#[test]
fn folder_of_table() {
    let cases = [
        (OpKind::Write, r"D:\a\b.txt", r"D:\a"),
        (OpKind::Write, r"D:\a\out", r"D:\a\out"),
        (OpKind::Create, r"D:\a.b", r"D:\a.b"),
        (OpKind::Copy, r"D:\x.txt", r"D:\"),
        (OpKind::Move, r"\\srv\share\f.rs", r"\\srv\share"),
        (OpKind::Write, r"D:\a\.git", r"D:\a\.git"),
    ];
    for (kind, path, expected) in cases {
        assert_eq!(folder_of(kind, path), expected, "{kind:?} {path}");
    }
}
