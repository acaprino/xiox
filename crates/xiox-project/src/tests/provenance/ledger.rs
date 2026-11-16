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
    assert_eq!(sources(&ledger, r"D:\w"), [Source::Structured { tool: "Write".into(), canonical: "write".into() }]);
    assert_eq!(strength(&ledger, r"D:\w"), Some(Strength::Written));
    assert_eq!(sources(&ledger, r"D:\g"), [Source::Structured { tool: "Glob".into(), canonical: "glob".into() }]);
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
    assert!(ledger.folders().all(|f| f.refs.iter().all(|r| !matches!(r.source, Source::Structured { .. }))));
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

#[test]
fn patch_paths_are_always_files() {
    let patch = "*** Begin Patch\n*** Update File: D:/P/Makefile\n@@\n-a\n+b\n*** Delete File: D:/old/LICENSE\n\
                 *** Add File: src/.gitignore\n+x\n*** End Patch\n";
    let ledger = ledger_of(&[call("edit", "apply_patch", json!({ "_": patch }), Some(r"D:\P"))]);
    assert_eq!(strength(&ledger, r"D:\P"), Some(Strength::Written));
    assert!(ledger.get(r"D:\P\Makefile").is_none());
    assert_eq!(strength(&ledger, r"D:\old"), Some(Strength::Used));
    assert!(ledger.get(r"D:\old\LICENSE").is_none());
    assert_eq!(strength(&ledger, r"D:\P\src"), Some(Strength::Written));
    assert!(ledger.get(r"D:\P\src\.gitignore").is_none());
}

#[test]
fn a_shell_command_carrying_a_patch_gives_both_evidences() {
    let command = "cd /d/w && apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: n.rs\n+x\n*** End Patch\nEOF";
    let ledger = ledger_of(&[bash(command, r"D:\P")]);
    assert_eq!(strength(&ledger, r"D:\w"), Some(Strength::Used));
    assert_eq!(sources(&ledger, r"D:\w"), [Source::Cd]);
    assert_eq!(strength(&ledger, r"D:\P"), Some(Strength::Written));
    assert!(sources(&ledger, r"D:\P").contains(&Source::Patch(PatchAction::Add)));
    assert_eq!(ledger.coverage().shell_calls, 1);
}

#[test]
fn a_failed_call_still_ran_in_its_cwd() {
    let mut failed = bash("cd /d/a && git clone https://h/x.git D:/c", r"D:\P");
    failed.exit = Some(1);
    let ledger = ledger_of(&[failed]);
    for (folder, source) in [(r"D:\P", Source::Cwd), (r"D:\a", Source::Cd)] {
        assert_eq!(strength(&ledger, folder), Some(Strength::Used), "{folder}");
        let refs = &ledger.get(folder).unwrap().refs;
        assert_eq!((refs.len(), &refs[0].source, refs[0].outcome), (1, &source, Outcome::Failed), "{folder}");
    }
    let clone = ledger.get(r"D:\c").unwrap();
    assert_eq!(clone.strength, None);
    assert!(clone.refs.is_empty());
    assert_eq!(clone.failed.len(), 1);
}

const PATCH_EXAMPLE: &str = "# Notes\n*** Begin Patch\n*** Add File: D:\\x\\y\n+z\n*** End Patch\n";

#[test]
fn a_structured_call_holding_a_patch_example_keeps_its_own_evidence() {
    // The text a Write or Edit stores is not a patch the session applied.
    let ledger = ledger_of(&[
        call("write", "Write", json!({ "file_path": r"D:\docs\a.md", "content": PATCH_EXAMPLE }), Some(r"D:\P")),
        call(
            "edit",
            "Edit",
            json!({ "file_path": r"D:\docs\b.md", "old_string": "a", "new_string": PATCH_EXAMPLE }),
            Some(r"D:\P"),
        ),
    ]);
    assert_eq!(strength(&ledger, r"D:\docs"), Some(Strength::Written));
    assert!(ledger.get(r"D:\x").is_none());
    assert!(ledger.folders().all(|f| f.refs.iter().all(|r| !matches!(r.source, Source::Patch(_)))));
}

#[test]
fn patch_text_comes_only_from_the_keys_a_patch_tool_uses() {
    for key in ["_", "command", "cmd", "input", "patch"] {
        let ledger = ledger_of(&[call("edit", "apply_patch", json!({ key: PATCH_EXAMPLE }), Some(r"D:\P"))]);
        assert_eq!(strength(&ledger, r"D:\x"), Some(Strength::Written), "{key}");
    }
    let ledger = ledger_of(&[call("edit", "other", json!({ "note": PATCH_EXAMPLE }), Some(r"D:\P"))]);
    assert!(ledger.get(r"D:\x").is_none());
}

#[test]
fn every_reference_carries_its_strength() {
    let mut failed = bash("mkdir D:/f", r"D:\P");
    failed.exit = Some(1);
    let ledger = ledger_of(&[
        call("write", "Write", json!({ "file_path": r"D:\w\f.txt" }), Some(r"D:\P")),
        bash("mkdir D:/m && cd /d/c", r"D:\P"),
        failed,
    ]);
    let strengths = |folder: &str| -> Vec<Option<Strength>> {
        ledger.get(folder).unwrap().refs.iter().map(|r| r.strength).collect()
    };
    assert_eq!(strengths(r"D:\w"), [Some(Strength::Written)]);
    assert_eq!(strengths(r"D:\m"), [Some(Strength::Created)]);
    assert_eq!(strengths(r"D:\c"), [Some(Strength::Used)]);
    assert_eq!(strengths(r"D:\P"), [Some(Strength::Used); 3]);
    assert_eq!(ledger.get(r"D:\f").unwrap().failed[0].strength, None);
    assert_eq!(
        sources(&ledger, r"D:\w"),
        [Source::Structured { tool: "Write".into(), canonical: "write".into() }]
    );
}

#[test]
fn ledger_types_serialize_for_the_probe() {
    let ledger = ledger_of(&[call("write", "Write", json!({ "file_path": r"D:\w\f.txt" }), Some(r"D:\P"))]);
    let value = serde_json::to_value(ledger.get(r"D:\w").unwrap()).unwrap();
    assert_eq!(value["strength"], "written");
    let reference = &value["refs"][0];
    assert_eq!(reference["strength"], "written");
    assert_eq!(reference["source"]["structured"]["canonical"], "write");
    assert_eq!(reference["session"]["store"], "store");
    let coverage = serde_json::to_value(ledger.coverage()).unwrap();
    assert_eq!(coverage["shellCalls"], 0);
}
