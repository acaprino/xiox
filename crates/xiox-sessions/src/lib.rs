//! Deletes archived harness sessions. Claude Code and Pi sessions are plain
//! transcript files; Codex and OpenCode keep state in their own stores, so those go
//! through the harness CLI (never by touching the stores directly).

mod cli;

use std::io;
use std::path::{Path, PathBuf};

use xiox_model::{Agent, ItemOutcome, RemoveMode};
use xiox_mukade::ArchivedSession;

pub use cli::SystemCli;

pub trait HarnessCli: Send + Sync {
    fn codex_delete(&self, id: &str) -> Result<(), String>;
    fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String>;
    fn opencode_delete(&self, id: &str) -> Result<(), String>;
    /// Whether the Codex CLI can be run at all. Checked before a backup copy is made
    /// so a missing CLI does not leave useless entries in the Recycle Bin.
    fn codex_available(&self) -> bool {
        true
    }
}

pub trait Trash: Send + Sync {
    fn to_bin(&self, path: &Path) -> Result<(), String>;
}

pub struct SystemTrash;

impl Trash for SystemTrash {
    fn to_bin(&self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct SessionOutcome {
    pub agent: Agent,
    pub id: String,
    pub outcome: ItemOutcome,
}

/// Session ids end up in file names and on command lines (`.cmd` shims go through
/// cmd.exe), so accept only the characters real ids contain.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !id.starts_with('-')
}

pub fn delete_sessions(
    items: &[ArchivedSession],
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
) -> Vec<SessionOutcome> {
    delete_sessions_in(items, mode, cli, trash, &scratch_dir())
}

/// Same as [`delete_sessions`] with an explicit scratch folder for backup copies/exports.
pub fn delete_sessions_in(
    items: &[ArchivedSession],
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
    scratch: &Path,
) -> Vec<SessionOutcome> {
    items
        .iter()
        .map(|s| SessionOutcome {
            agent: s.agent.clone(),
            id: s.id.clone(),
            outcome: delete_one(s, mode, cli, trash, scratch),
        })
        .collect()
}

fn failed(error: impl Into<String>) -> ItemOutcome {
    ItemOutcome::Failed { error: error.into(), locked_by: vec![] }
}

fn delete_one(
    s: &ArchivedSession,
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
    scratch: &Path,
) -> ItemOutcome {
    if let Some(reason) = &s.blocked {
        return ItemOutcome::Skipped { reason: reason.clone() };
    }
    if !valid_id(&s.id) {
        return failed("invalid id");
    }
    match s.agent {
        Agent::Claude | Agent::Pi => delete_file_session(s, mode, trash),
        Agent::Codex => delete_codex(s, mode, cli, trash, scratch),
        Agent::Opencode => delete_opencode(s, mode, cli, trash, scratch),
    }
}

/// True only when the OS positively says the path does not exist.
fn is_gone(p: &Path) -> bool {
    matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

fn delete_file_session(s: &ArchivedSession, mode: RemoveMode, trash: &dyn Trash) -> ItemOutcome {
    let Some(file) = &s.file else {
        return failed("session file not given");
    };
    let res = match mode {
        RemoveMode::Recycle => trash.to_bin(file),
        RemoveMode::Permanent => std::fs::remove_file(file).map_err(|e| e.to_string()),
    };
    // Success is measured: the call must have succeeded AND the file must really be gone.
    match res {
        Ok(()) if is_gone(file) => ItemOutcome::Removed { bytes_freed: s.bytes },
        Ok(()) => failed("the file still exists after the removal"),
        Err(e) => failed(e),
    }
}

fn delete_codex(
    s: &ArchivedSession,
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
    scratch: &Path,
) -> ItemOutcome {
    if !cli.codex_available() {
        return failed("Codex CLI not found");
    }
    // The analysis saw a rollout that is gone now: the CLI would delete the thread with no copy
    // in the Recycle Bin. Leave it alone; a new analysis shows what it has become.
    if s.file.as_ref().is_some_and(|f| !f.exists()) {
        return failed("the session file is gone: session left untouched");
    }
    let rollout = s.file.as_ref().filter(|f| f.exists());
    // A paginated thread keeps earlier pages beside its rollout and the CLI deletes them all:
    // every page is backed up, measured and checked.
    let pages: Vec<(PathBuf, u64)> = rollout
        .map(|f| xiox_mukade::codex_thread_files(f, &s.id))
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let len = std::fs::metadata(&p).map_or(0, |m| m.len());
            (p, len)
        })
        .collect();
    if mode == RemoveMode::Recycle {
        // The CLI deletes the rollouts permanently, so bin a copy of each first.
        for (page, _) in &pages {
            let name = match (pages.len(), page.file_name()) {
                (2.., Some(n)) => n.to_string_lossy().into_owned(),
                _ => format!("{}.jsonl", s.id),
            };
            if let Err(e) = copy_and_bin(page, &scratch.join(name), trash) {
                return failed(e);
            }
        }
    }
    if let Err(e) = cli.codex_delete(&s.id) {
        return failed(e);
    }
    let freed = pages
        .iter()
        .filter(|(p, _)| is_gone(p))
        .fold(0u64, |acc, (_, n)| acc.saturating_add(*n));
    ItemOutcome::Removed { bytes_freed: freed }
}

fn delete_opencode(
    s: &ArchivedSession,
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
    scratch: &Path,
) -> ItemOutcome {
    if mode == RemoveMode::Recycle {
        let export = scratch.join(format!("opencode-{}.json", s.id));
        if let Some(parent) = export.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return failed(e.to_string());
            }
        }
        if let Err(e) = cli.opencode_export(&s.id, &export) {
            let _ = std::fs::remove_file(&export);
            return failed(format!("export failed: {e}"));
        }
        if !export_is_valid(&export, &s.id) {
            let _ = std::fs::remove_file(&export);
            return failed("invalid export: the session was left untouched");
        }
        if let Err(e) = trash.to_bin(&export) {
            let _ = std::fs::remove_file(&export);
            return failed(e);
        }
    }
    match cli.opencode_delete(&s.id) {
        Ok(()) => ItemOutcome::Removed { bytes_freed: s.bytes },
        Err(e) => failed(e),
    }
}

/// The export is the only backup of a session about to be deleted: it must exist, be
/// non-empty, be valid JSON and, when it names a session id, name this one.
fn export_is_valid(path: &Path, id: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    if bytes.is_empty() {
        return false;
    }
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false };
    let named = json
        .pointer("/info/id")
        .or_else(|| json.pointer("/id"))
        .and_then(|v| v.as_str());
    named.is_none_or(|n| n == id)
}

fn copy_and_bin(src: &Path, copy: &Path, trash: &dyn Trash) -> Result<(), String> {
    if let Some(parent) = copy.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(src, copy).map_err(|e| e.to_string())?;
    trash.to_bin(copy).inspect_err(|_| {
        let _ = std::fs::remove_file(copy);
    })
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("Xiox")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);
    impl Log {
        fn push(&self, s: String) {
            self.0.lock().unwrap().push(s);
        }
        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct FakeCli {
        log: Arc<Log>,
        /// Error returned by codex_delete / opencode_delete.
        error: Option<String>,
        /// Error returned by opencode_export (nothing written).
        export_error: Option<String>,
        /// What opencode_export writes (exit 0).
        export_body: Vec<u8>,
        codex_missing: bool,
        /// Emulates the real CLI: codex_delete removes the thread's rollouts.
        codex_rollouts: Vec<PathBuf>,
    }
    impl HarnessCli for FakeCli {
        fn codex_delete(&self, id: &str) -> Result<(), String> {
            self.log.push(format!("codex_delete {id}"));
            if self.codex_missing {
                return Err("Codex CLI not found".into());
            }
            if let Some(e) = &self.error {
                return Err(e.clone());
            }
            for f in &self.codex_rollouts {
                std::fs::remove_file(f).unwrap();
            }
            Ok(())
        }
        fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String> {
            self.log.push(format!("export {id}"));
            if let Some(e) = &self.export_error {
                return Err(e.clone());
            }
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::fs::write(out, &self.export_body).unwrap();
            Ok(())
        }
        fn opencode_delete(&self, id: &str) -> Result<(), String> {
            self.log.push(format!("delete {id}"));
            self.error.clone().map_or(Ok(()), Err)
        }
        fn codex_available(&self) -> bool {
            !self.codex_missing
        }
    }

    struct FakeTrash {
        log: Arc<Log>,
        bin: tempfile::TempDir,
        binned: Mutex<Vec<(PathBuf, Vec<u8>)>>,
        fail: bool,
    }
    impl Trash for FakeTrash {
        fn to_bin(&self, path: &Path) -> Result<(), String> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if self.fail {
                self.log.push(format!("to_bin_failed {name}"));
                return Err("Recycle Bin not available".into());
            }
            let content = std::fs::read(path).map_err(|e| e.to_string())?;
            self.log.push(format!("to_bin {name}"));
            self.binned.lock().unwrap().push((path.to_path_buf(), content));
            std::fs::copy(path, self.bin.path().join(&name)).map_err(|e| e.to_string())?;
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }
    }

    struct Env {
        cli: FakeCli,
        trash: FakeTrash,
        log: Arc<Log>,
        scratch: tempfile::TempDir,
        work: tempfile::TempDir,
    }
    impl Env {
        fn new() -> Env {
            let log = Arc::new(Log::default());
            Env {
                cli: FakeCli {
                    log: log.clone(),
                    error: None,
                    export_error: None,
                    export_body: br#"{"info":{"id":"t-oc"},"messages":[]}"#.to_vec(),
                    codex_missing: false,
                    codex_rollouts: vec![],
                },
                trash: FakeTrash {
                    log: log.clone(),
                    bin: tempfile::tempdir().unwrap(),
                    binned: Mutex::new(vec![]),
                    fail: false,
                },
                log,
                scratch: tempfile::tempdir().unwrap(),
                work: tempfile::tempdir().unwrap(),
            }
        }
        fn file(&self, name: &str, body: &[u8]) -> PathBuf {
            let p = self.work.path().join(name);
            std::fs::write(&p, body).unwrap();
            p
        }
        fn run(&self, items: &[ArchivedSession], mode: RemoveMode) -> Vec<SessionOutcome> {
            delete_sessions_in(items, mode, &self.cli, &self.trash, self.scratch.path())
        }
        fn scratch_is_empty(&self) -> bool {
            std::fs::read_dir(self.scratch.path()).unwrap().next().is_none()
        }
    }

    fn session(agent: Agent, id: &str, file: Option<PathBuf>, bytes: u64) -> ArchivedSession {
        ArchivedSession {
            agent,
            id: id.into(),
            title: None,
            cwd: None,
            updated_ms: None,
            file,
            bytes,
            blocked: None,
        }
    }

    fn assert_removed(o: &ItemOutcome, expected: u64) {
        match o {
            ItemOutcome::Removed { bytes_freed } => assert_eq!(*bytes_freed, expected),
            other => panic!("expected Removed, got {other:?}"),
        }
    }

    fn failed_text(o: &ItemOutcome) -> &str {
        match o {
            ItemOutcome::Failed { error, locked_by } => {
                assert!(locked_by.is_empty());
                error
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    fn outcome(v: &[SessionOutcome]) -> &ItemOutcome {
        assert_eq!(v.len(), 1);
        &v[0].outcome
    }

    #[test]
    fn blocked_session_is_skipped_and_nothing_called() {
        let env = Env::new();
        let f = env.file("a.jsonl", b"x");
        for agent in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
            let mut s = session(agent, "t-blocked", Some(f.clone()), 1);
            s.blocked = Some("session in use".into());
            let r = env.run(&[s], RemoveMode::Permanent);
            assert!(matches!(outcome(&r), ItemOutcome::Skipped { reason } if reason == "session in use"));
        }
        assert!(env.log.all().is_empty());
        assert!(f.exists());
    }

    #[test]
    fn claude_permanent_removes_the_transcript() {
        let env = Env::new();
        let f = env.file("c.jsonl", b"hello");
        let s = session(Agent::Claude, "t-claude-perm", Some(f.clone()), 5);
        let r = env.run(&[s], RemoveMode::Permanent);
        assert!(!f.exists());
        assert_removed(outcome(&r), 5);
        assert!(env.log.all().is_empty());
    }

    #[test]
    fn claude_recycle_sends_the_transcript_to_the_bin() {
        let env = Env::new();
        let f = env.file("p.jsonl", b"hello");
        let s = session(Agent::Claude, "t-claude-recycle", Some(f.clone()), 5);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert_eq!(env.trash.binned.lock().unwrap()[0].0, f);
        assert_removed(outcome(&r), 5);
        assert!(!f.exists());
    }

    #[test]
    fn file_session_fails_when_the_file_is_still_there() {
        let mut env = Env::new();
        env.trash.fail = true;
        let f = env.file("p.jsonl", b"hello");
        let s = session(Agent::Pi, "t-pi-trashfail", Some(f.clone()), 5);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert!(failed_text(outcome(&r)).contains("Recycle Bin not available"));
        assert!(f.exists());
    }

    #[test]
    fn codex_recycle_bins_a_copy_then_deletes() {
        let mut env = Env::new();
        let f = env.file("rollout.jsonl", b"rollout-data");
        env.cli.codex_rollouts = vec![f.clone()];
        let s = session(Agent::Codex, "t-codex-recycle", Some(f.clone()), 12);
        let r = env.run(&[s], RemoveMode::Recycle);
        {
            let binned = env.trash.binned.lock().unwrap();
            assert_eq!(binned.len(), 1);
            assert_ne!(binned[0].0, f);
            assert_eq!(binned[0].1, b"rollout-data");
            assert_eq!(binned[0].1.len(), 12);
        }
        assert_eq!(
            env.log.all(),
            vec!["to_bin t-codex-recycle.jsonl", "codex_delete t-codex-recycle"]
        );
        assert_removed(outcome(&r), 12);
        assert!(!f.exists());
    }

    #[test]
    fn codex_recycle_bins_every_page_of_a_paginated_thread() {
        let mut env = Env::new();
        let id = "019a0000-1111-2222-3333-444455556666";
        let day = |d: &str| {
            let dir = env.work.path().join("sessions").join("2026").join("11").join(d);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        let first = day("09").join(format!("rollout-2026-11-09T10-00-00-{id}.jsonl"));
        let latest = day("10").join(format!("rollout-2026-11-10T09-00-00-{id}_019a0000-aaaa-bbbb-cccc-ddddeeeeffff.jsonl"));
        let other = day("10").join("rollout-2026-11-10T08-00-00-019a0000-9999-2222-3333-444455556666.jsonl");
        std::fs::write(&first, b"page-one").unwrap();
        std::fs::write(&latest, b"two").unwrap();
        std::fs::write(&other, b"not this thread").unwrap();
        env.cli.codex_rollouts = vec![first.clone(), latest.clone()];
        let s = session(Agent::Codex, id, Some(latest.clone()), 3);
        let r = env.run(&[s], RemoveMode::Recycle);
        {
            let binned = env.trash.binned.lock().unwrap();
            let mut contents: Vec<&[u8]> = binned.iter().map(|(_, c)| c.as_slice()).collect();
            contents.sort();
            assert_eq!(contents, vec![b"page-one".as_slice(), b"two".as_slice()]);
            assert!(binned.iter().all(|(p, _)| p.starts_with(env.scratch.path())), "copies, not the originals");
        }
        let log = env.log.all();
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(log[..2].iter().all(|l| l.starts_with("to_bin rollout-")), "{log:?}");
        assert_eq!(log[2], format!("codex_delete {id}"));
        assert_removed(outcome(&r), 11);
        assert!(!first.exists() && !latest.exists());
        assert!(other.exists(), "another thread is never touched");
    }

    #[test]
    fn codex_permanent_counts_only_the_pages_that_are_gone() {
        let mut env = Env::new();
        let id = "019a0000-1111-2222-3333-444455556666";
        let dir = env.work.path().join("sessions").join("2026");
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join(format!("rollout-a-{id}.jsonl"));
        let latest = dir.join(format!("rollout-b-{id}_p.jsonl"));
        std::fs::write(&first, b"12345").unwrap();
        std::fs::write(&latest, b"12").unwrap();
        // The CLI leaves the earlier page behind: only what is really gone is counted.
        env.cli.codex_rollouts = vec![latest.clone()];
        let s = session(Agent::Codex, id, Some(latest.clone()), 2);
        let r = env.run(&[s], RemoveMode::Permanent);
        assert_eq!(env.log.all(), vec![format!("codex_delete {id}")]);
        assert_removed(outcome(&r), 2);
        assert!(first.exists());
    }

    #[test]
    fn codex_whose_rollout_vanished_is_not_deleted() {
        let env = Env::new();
        let gone = env.work.path().join("rollout-gone.jsonl");
        for mode in [RemoveMode::Recycle, RemoveMode::Permanent] {
            let s = session(Agent::Codex, "t-codex-vanished", Some(gone.clone()), 5);
            let r = env.run(&[s], mode);
            assert!(failed_text(outcome(&r)).contains("is gone"), "{mode:?}");
        }
        assert!(env.log.all().is_empty(), "CLI never called, nothing binned");
        assert!(env.scratch_is_empty());
    }

    #[test]
    fn codex_without_rollout_is_one_cli_call() {
        let env = Env::new();
        let s = session(Agent::Codex, "t-codex-norollout", None, 0);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert_eq!(env.log.all(), vec!["codex_delete t-codex-norollout"]);
        assert_removed(outcome(&r), 0);
    }

    #[test]
    fn cli_failure_keeps_the_session() {
        let mut env = Env::new();
        env.cli.error = Some("boom: database locked".into());
        let f = env.file("rollout.jsonl", b"keep me");
        let s = session(Agent::Codex, "t-codex-fail", Some(f.clone()), 7);
        let r = env.run(&[s], RemoveMode::Permanent);
        assert!(failed_text(outcome(&r)).contains("boom: database locked"));
        assert!(f.exists());
    }

    #[test]
    fn cli_failure_in_recycle_keeps_the_original_rollout() {
        let mut env = Env::new();
        env.cli.error = Some("boom: database locked".into());
        let f = env.file("rollout.jsonl", b"keep me");
        let s = session(Agent::Codex, "t-codex-recfail", Some(f.clone()), 7);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert!(failed_text(outcome(&r)).contains("boom: database locked"));
        assert_eq!(std::fs::read(&f).unwrap(), b"keep me");
    }

    #[test]
    fn codex_recycle_with_failing_bin_does_not_delete_and_cleans_the_copy() {
        let mut env = Env::new();
        env.trash.fail = true;
        let f = env.file("rollout.jsonl", b"rollout-data");
        let s = session(Agent::Codex, "t-codex-binfail", Some(f.clone()), 12);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert!(failed_text(outcome(&r)).contains("Recycle Bin not available"));
        assert_eq!(env.log.all(), vec!["to_bin_failed t-codex-binfail.jsonl"]);
        assert!(f.exists());
        assert!(env.scratch_is_empty(), "the temp copy must be removed");
    }

    #[test]
    fn codex_recycle_with_missing_cli_makes_no_backup() {
        let mut env = Env::new();
        env.cli.codex_missing = true;
        let f = env.file("rollout.jsonl", b"data");
        let s = session(Agent::Codex, "t-codex-nocli", Some(f.clone()), 4);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert_eq!(failed_text(outcome(&r)), "Codex CLI not found");
        assert!(env.log.all().is_empty());
        assert!(env.scratch_is_empty());
        assert!(f.exists());
    }

    #[test]
    fn opencode_recycle_exports_then_deletes() {
        let mut env = Env::new();
        env.cli.export_body = br#"{"info":{"id":"t-oc-recycle"}}"#.to_vec();
        let s = session(Agent::Opencode, "t-oc-recycle", None, 100);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert_eq!(
            env.log.all(),
            vec!["export t-oc-recycle", "to_bin opencode-t-oc-recycle.json", "delete t-oc-recycle"]
        );
        assert_removed(outcome(&r), 100);
    }

    #[test]
    fn opencode_export_without_id_fields_is_accepted() {
        let mut env = Env::new();
        env.cli.export_body = br#"{"messages":[]}"#.to_vec();
        let s = session(Agent::Opencode, "t-oc-noid", None, 1);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert_removed(outcome(&r), 1);
        assert_eq!(env.log.all().last().unwrap(), "delete t-oc-noid");
    }

    #[test]
    fn opencode_failed_export_stops_the_delete() {
        let mut env = Env::new();
        env.cli.export_error = Some("Session not found".into());
        let s = session(Agent::Opencode, "t-oc-exportfail", None, 1);
        let r = env.run(&[s], RemoveMode::Recycle);
        assert!(failed_text(outcome(&r)).contains("Session not found"));
        assert_eq!(env.log.all(), vec!["export t-oc-exportfail"]);
    }

    #[test]
    fn opencode_unusable_export_stops_the_delete() {
        let cases: [(&str, &[u8]); 4] = [
            ("empty", b""),
            ("truncated", br#"{"info":{"id":"t-oc-bad-"#),
            ("not json", b"hello world"),
            ("other session", br#"{"info":{"id":"ses_other"}}"#),
        ];
        for (label, body) in cases {
            let mut env = Env::new();
            env.cli.export_body = body.to_vec();
            let id = "t-oc-bad-";
            let s = session(Agent::Opencode, id, None, 1);
            let r = env.run(&[s], RemoveMode::Recycle);
            assert_eq!(
                failed_text(outcome(&r)),
                "invalid export: the session was left untouched",
                "{label}"
            );
            assert_eq!(env.log.all(), vec![format!("export {id}")], "{label}");
            assert!(env.scratch_is_empty(), "{label}: export file must be removed");
        }
    }

    #[test]
    fn opencode_recycle_with_failing_bin_does_not_delete() {
        let mut env = Env::new();
        env.trash.fail = true;
        env.cli.export_body = br#"{"id":"t-oc-binfail"}"#.to_vec();
        let s = session(Agent::Opencode, "t-oc-binfail", None, 1);
        let r = env.run(&[s], RemoveMode::Recycle);
        failed_text(outcome(&r));
        assert!(!env.log.all().iter().any(|l| l.starts_with("delete")));
    }

    #[test]
    fn invalid_ids_fail_without_side_effects() {
        let env = Env::new();
        let f = env.file("a.jsonl", b"x");
        for id in [r"..\x", r"C:\x", "-rf", "a b", "", "a/b"] {
            for agent in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
                for mode in [RemoveMode::Recycle, RemoveMode::Permanent] {
                    let s = session(agent.clone(), id, Some(f.clone()), 1);
                    let r = env.run(&[s], mode);
                    assert_eq!(failed_text(outcome(&r)), "invalid id", "{id:?}");
                }
            }
        }
        assert!(env.log.all().is_empty());
        assert!(env.scratch_is_empty());
        assert!(f.exists());
    }

    #[test]
    fn missing_codex_cli_fails_only_codex() {
        let cli = SystemCli { codex: None, opencode: None, timeout: Duration::from_secs(1) };
        assert_eq!(cli.codex_delete("abc"), Err("Codex CLI not found".into()));
        assert_eq!(cli.opencode_delete("abc"), Err("OpenCode CLI not found".into()));
        assert_eq!(
            cli.opencode_export("abc", Path::new("x.json")),
            Err("OpenCode CLI not found".into())
        );
        assert!(!cli.codex_available());
    }

    #[test]
    fn a_missing_codex_cli_does_not_stop_the_other_harnesses() {
        let mut env = Env::new();
        env.cli.codex_missing = true;
        let claude_file = env.file("c.jsonl", b"hello");
        let codex_file = env.file("rollout.jsonl", b"data");
        let items = [
            session(Agent::Codex, "t-mixed-codex", Some(codex_file.clone()), 4),
            session(Agent::Claude, "t-mixed-claude", Some(claude_file.clone()), 5),
        ];
        let r = env.run(&items, RemoveMode::Permanent);
        assert_eq!(r.len(), 2);
        assert_eq!(failed_text(&r[0].outcome), "Codex CLI not found");
        assert_removed(&r[1].outcome, 5);
        assert!(codex_file.exists());
        assert!(!claude_file.exists());
    }
}
