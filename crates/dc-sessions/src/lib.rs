//! Deletes archived harness sessions. Claude Code and Pi sessions are plain
//! transcript files; Codex and OpenCode keep state in their own stores, so those go
//! through the harness CLI (never by touching the stores directly).

mod cli;

use std::path::{Path, PathBuf};

use dc_model::{Agent, ItemOutcome, RemoveMode};
use dc_mukade::ArchivedSession;

pub use cli::SystemCli;

pub trait HarnessCli: Send + Sync {
    fn codex_delete(&self, id: &str) -> Result<(), String>;
    fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String>;
    fn opencode_delete(&self, id: &str) -> Result<(), String>;
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

pub fn delete_sessions(
    items: &[ArchivedSession],
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
) -> Vec<SessionOutcome> {
    items
        .iter()
        .map(|s| SessionOutcome {
            agent: s.agent.clone(),
            id: s.id.clone(),
            outcome: delete_one(s, mode, cli, trash),
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
) -> ItemOutcome {
    if let Some(reason) = &s.blocked {
        return ItemOutcome::Skipped { reason: reason.clone() };
    }
    match s.agent {
        Agent::Claude | Agent::Pi => delete_file_session(s, mode, trash),
        Agent::Codex => delete_codex(s, mode, cli, trash),
        Agent::Opencode => delete_opencode(s, mode, cli, trash),
    }
}

fn delete_file_session(s: &ArchivedSession, mode: RemoveMode, trash: &dyn Trash) -> ItemOutcome {
    let Some(file) = &s.file else {
        return failed("file della sessione non indicato");
    };
    let res = match mode {
        RemoveMode::Recycle => trash.to_bin(file),
        RemoveMode::Permanent => std::fs::remove_file(file).map_err(|e| e.to_string()),
    };
    // Success is measured: the file must really be gone.
    if !file.exists() {
        return ItemOutcome::Removed { bytes_freed: s.bytes };
    }
    match res {
        Err(e) => failed(e),
        Ok(()) => failed("il file esiste ancora dopo la rimozione"),
    }
}

fn delete_codex(
    s: &ArchivedSession,
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
) -> ItemOutcome {
    let rollout = s.file.as_ref().filter(|f| f.exists());
    if mode == RemoveMode::Recycle {
        if let Some(rollout) = rollout {
            // The CLI deletes the rollout permanently, so bin a copy first.
            let copy = scratch_dir().join(format!("{}.jsonl", s.id));
            if let Err(e) = copy_and_bin(rollout, &copy, trash) {
                return failed(e);
            }
        }
    }
    if let Err(e) = cli.codex_delete(&s.id) {
        return failed(e);
    }
    let freed = match rollout {
        Some(f) if !f.exists() => s.bytes,
        _ => 0,
    };
    ItemOutcome::Removed { bytes_freed: freed }
}

fn delete_opencode(
    s: &ArchivedSession,
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
) -> ItemOutcome {
    if mode == RemoveMode::Recycle {
        let export = scratch_dir().join(format!("opencode-{}.json", s.id));
        if let Err(e) = cli.opencode_export(&s.id, &export) {
            return failed(format!("esportazione non riuscita: {e}"));
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
    std::env::temp_dir().join("PulisciPC")
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
        error: Option<String>,
    }
    impl HarnessCli for FakeCli {
        fn codex_delete(&self, id: &str) -> Result<(), String> {
            self.log.push(format!("codex_delete {id}"));
            self.error.clone().map_or(Ok(()), Err)
        }
        fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String> {
            self.log.push(format!("export {id}"));
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::fs::write(out, b"{\"export\":true}").unwrap();
            self.error.clone().map_or(Ok(()), Err)
        }
        fn opencode_delete(&self, id: &str) -> Result<(), String> {
            self.log.push(format!("delete {id}"));
            self.error.clone().map_or(Ok(()), Err)
        }
    }

    struct FakeTrash {
        log: Arc<Log>,
        bin: tempfile::TempDir,
        binned: Mutex<Vec<(PathBuf, Vec<u8>)>>,
    }
    impl Trash for FakeTrash {
        fn to_bin(&self, path: &Path) -> Result<(), String> {
            let content = std::fs::read(path).map_err(|e| e.to_string())?;
            self.log.push(format!("to_bin {}", path.file_name().unwrap().to_string_lossy()));
            self.binned.lock().unwrap().push((path.to_path_buf(), content));
            let dest = self.bin.path().join(path.file_name().unwrap());
            std::fs::copy(path, dest).map_err(|e| e.to_string())?;
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }
    }

    fn fakes(error: Option<&str>) -> (FakeCli, FakeTrash, Arc<Log>) {
        let log = Arc::new(Log::default());
        (
            FakeCli { log: log.clone(), error: error.map(String::from) },
            FakeTrash {
                log: log.clone(),
                bin: tempfile::tempdir().unwrap(),
                binned: Mutex::new(vec![]),
            },
            log,
        )
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

    fn outcome(v: &[SessionOutcome]) -> &ItemOutcome {
        assert_eq!(v.len(), 1);
        &v[0].outcome
    }

    #[test]
    fn blocked_session_is_skipped_and_nothing_called() {
        let (cli, trash, log) = fakes(None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jsonl");
        std::fs::write(&f, b"x").unwrap();
        for agent in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
            let mut s = session(agent, "t-blocked", Some(f.clone()), 1);
            s.blocked = Some("sessione in uso".into());
            let r = delete_sessions(&[s], RemoveMode::Permanent, &cli, &trash);
            assert!(matches!(outcome(&r), ItemOutcome::Skipped { reason } if reason == "sessione in uso"));
        }
        assert!(log.all().is_empty());
        assert!(f.exists());
    }

    #[test]
    fn claude_permanent_removes_the_transcript() {
        let (cli, trash, log) = fakes(None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c.jsonl");
        std::fs::write(&f, b"hello").unwrap();
        let s = session(Agent::Claude, "t-claude-perm", Some(f.clone()), 5);
        let r = delete_sessions(&[s], RemoveMode::Permanent, &cli, &trash);
        assert!(!f.exists());
        assert_removed(outcome(&r), 5);
        assert!(log.all().is_empty());
    }

    #[test]
    fn claude_recycle_sends_the_transcript_to_the_bin() {
        let (cli, trash, _log) = fakes(None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("p.jsonl");
        std::fs::write(&f, b"hello").unwrap();
        let s = session(Agent::Claude, "t-claude-recycle", Some(f.clone()), 5);
        let r = delete_sessions(&[s], RemoveMode::Recycle, &cli, &trash);
        assert_eq!(trash.binned.lock().unwrap()[0].0, f);
        assert_removed(outcome(&r), 5);
        assert!(!f.exists());
    }

    #[test]
    fn codex_recycle_bins_a_copy_then_deletes() {
        let (cli, trash, log) = fakes(None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        std::fs::write(&f, b"rollout-data").unwrap();
        let s = session(Agent::Codex, "t-codex-recycle", Some(f.clone()), 12);
        let r = delete_sessions(&[s], RemoveMode::Recycle, &cli, &trash);
        let binned = trash.binned.lock().unwrap();
        assert_ne!(binned[0].0, f);
        assert_eq!(binned[0].1, b"rollout-data");
        assert_eq!(log.all(), vec!["to_bin t-codex-recycle.jsonl", "codex_delete t-codex-recycle"]);
        assert!(matches!(outcome(&r), ItemOutcome::Removed { .. }));
    }

    #[test]
    fn codex_without_rollout_is_one_cli_call() {
        let (cli, trash, log) = fakes(None);
        let s = session(Agent::Codex, "t-codex-norollout", None, 0);
        let r = delete_sessions(&[s], RemoveMode::Recycle, &cli, &trash);
        assert_eq!(log.all(), vec!["codex_delete t-codex-norollout"]);
        assert_removed(outcome(&r), 0);
    }

    #[test]
    fn cli_failure_keeps_the_session() {
        let (cli, trash, _log) = fakes(Some("boom: database locked"));
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("rollout.jsonl");
        std::fs::write(&f, b"keep me").unwrap();
        let s = session(Agent::Codex, "t-codex-fail", Some(f.clone()), 7);
        let r = delete_sessions(&[s], RemoveMode::Permanent, &cli, &trash);
        match outcome(&r) {
            ItemOutcome::Failed { error, locked_by } => {
                assert!(error.contains("boom: database locked"));
                assert!(locked_by.is_empty());
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(f.exists());
    }

    #[test]
    fn opencode_recycle_exports_then_deletes() {
        let (cli, trash, log) = fakes(None);
        let s = session(Agent::Opencode, "t-oc-recycle", None, 100);
        let r = delete_sessions(&[s], RemoveMode::Recycle, &cli, &trash);
        assert_eq!(
            log.all(),
            vec!["export t-oc-recycle", "to_bin opencode-t-oc-recycle.json", "delete t-oc-recycle"]
        );
        assert_removed(outcome(&r), 100);
    }

    #[test]
    fn missing_codex_cli_fails_only_codex() {
        let cli = SystemCli { codex: None, opencode: None, timeout: Duration::from_secs(1) };
        assert_eq!(cli.codex_delete("abc"), Err("CLI di Codex non trovata".into()));
        assert_eq!(cli.opencode_delete("abc"), Err("CLI di OpenCode non trovata".into()));
        assert_eq!(
            cli.opencode_export("abc", Path::new("x.json")),
            Err("CLI di OpenCode non trovata".into())
        );
    }
}
