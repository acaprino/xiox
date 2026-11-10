//! Real harness CLIs. Verified against the installed tools on 2026-11-10:
//!
//! * `codex delete --force <id>`: permanent delete by id; `--force` is required
//!   when no terminal is attached (stdin is null here). Works on a row that has no
//!   rollout file (verified on a scratch copy of the store). `CODEX_HOME` is
//!   inherited from the parent environment unchanged.
//! * `opencode --standalone session delete <id>`: no confirmation prompt and no
//!   confirmation flag exist (`--help` lists none). `--standalone` runs a private
//!   server so we never depend on the background service. Exit code 1 plus
//!   "Session not found: <id>" on stderr when the id is unknown.
//! * `opencode --standalone session export <id>`: has no output-file flag, it writes
//!   the JSON to STDOUT (only `--sanitize` exists, which we do NOT use: the export is
//!   a backup, it must be complete). We redirect stdout straight into the target file.
//!   Unknown id: exit code 1, empty stdout, "Session not found: <id>" on stderr.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};


/// Longest we wait for stderr to be collected after the child ended or was killed.
const DRAIN_WAIT: Duration = Duration::from_secs(2);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct SystemCli {
    pub codex: Option<PathBuf>,
    pub opencode: Option<PathBuf>,
    pub timeout: Duration,
}

impl SystemCli {
    pub fn locate() -> SystemCli {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from);

        let codex = find_in_path(&["codex.exe"]).or_else(|| {
            local
                .map(|l| l.join(r"Programs\OpenAI\Codex\bin\codex.exe"))
                .filter(|p| p.is_file())
        });
        let opencode = find_in_path(&["opencode.cmd", "opencode.exe"]).or_else(|| {
            roaming
                .map(|r| r.join(r"npm\opencode.cmd"))
                .filter(|p| p.is_file())
        });
        SystemCli { codex, opencode, timeout: Duration::from_secs(60) }
    }
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn check_id(id: &str) -> Result<(), String> {
    if crate::valid_id(id) { Ok(()) } else { Err(format!("identificativo di sessione non valido: {id}")) }
}

impl crate::HarnessCli for SystemCli {
    fn codex_delete(&self, id: &str) -> Result<(), String> {
        let exe = self.codex.as_ref().ok_or("CLI di Codex non trovata")?;
        check_id(id)?;
        run(exe, &["delete".into(), "--force".into(), id.into()], Stdio::null(), self.timeout)
    }

    fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String> {
        let exe = self.opencode.as_ref().ok_or("CLI di OpenCode non trovata")?;
        check_id(id)?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let file = File::create(out).map_err(|e| e.to_string())?;
        let res = run(
            exe,
            &["--standalone".into(), "session".into(), "export".into(), id.into()],
            Stdio::from(file),
            self.timeout,
        );
        if res.is_err() {
            let _ = std::fs::remove_file(out);
        }
        res
    }

    fn opencode_delete(&self, id: &str) -> Result<(), String> {
        let exe = self.opencode.as_ref().ok_or("CLI di OpenCode non trovata")?;
        check_id(id)?;
        run(
            exe,
            &["--standalone".into(), "session".into(), "delete".into(), id.into()],
            Stdio::null(),
            self.timeout,
        )
    }

    fn codex_available(&self) -> bool {
        self.codex.is_some()
    }
}

/// Runs `exe args` with null stdin, no console window, the parent's environment
/// untouched. `Err` = non-zero exit (last stderr lines) or timeout (child killed).
fn run(exe: &Path, args: &[OsString], stdout: Stdio, timeout: Duration) -> Result<(), String> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(stdout).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let name = exe.file_name().unwrap_or(OsStr::new("cli")).to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("{name}: avvio non riuscito: {e}"))?;

    // stderr is drained on a thread (a full pipe would block the child). The result is
    // awaited with a bounded wait: a grandchild (node behind a .cmd shim) can keep the
    // pipe open after the shim exits or was killed, and that must never hang us.
    let mut stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.take(256 * 1024).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    let collect_stderr = || rx.recv_timeout(DRAIN_WAIT).unwrap_or_default();

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() >= timeout => {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = collect_stderr();
                return Err(format!("{name}: tempo scaduto dopo {} s", timeout.as_secs()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = collect_stderr();
                return Err(format!("{name}: {e}"));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    let err_text = String::from_utf8_lossy(&collect_stderr()).into_owned();
    let tail = last_lines(&err_text, 5);
    let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
    if tail.is_empty() {
        Err(format!("{name}: uscita con codice {code}"))
    } else {
        Err(format!("{name}: uscita con codice {code}: {tail}"))
    }
}

/// `.cmd` shims spawn node as a grandchild; kill the whole tree (best effort).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join(" | ")
}
