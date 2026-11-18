//! Fake trees and real git repositories under a temp dir.

use crate::Env;
use xiox_model::normalize_path;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sets the modification time of `path`, a file or a folder, and its creation time when given.
pub fn set_times(path: &Path, created_ms: Option<i64>, modified_ms: i64) {
    use std::time::{Duration, SystemTime};
    let at = |ms: i64| SystemTime::UNIX_EPOCH + Duration::from_millis(ms as u64);
    let mut open = fs::OpenOptions::new();
    open.write(true);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut open, 0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS
    let mut times = fs::FileTimes::new().set_modified(at(modified_ms));
    #[cfg(windows)]
    if let Some(ms) = created_ms {
        times = std::os::windows::fs::FileTimesExt::set_created(times, at(ms));
    }
    #[cfg(not(windows))]
    let _ = created_ms;
    open.open(path).unwrap().set_times(times).unwrap();
}

/// A fresh temp dir and its canonical path. The first call sets `GIT_CEILING_DIRECTORIES` to the
/// temp root for the whole test process, so a temp folder inside someone's repository (CI or
/// sandbox) is never taken for part of it.
pub fn tmp() -> (tempfile::TempDir, PathBuf) {
    static CEILING: std::sync::Once = std::sync::Once::new();
    CEILING.call_once(|| {
        let t = std::env::temp_dir();
        let root = normalize_path(&fs::canonicalize(&t).unwrap_or(t));
        // SAFETY: set once, before any git call of this process reads it, always to the same value.
        unsafe { std::env::set_var("GIT_CEILING_DIRECTORIES", root) };
    });
    let t = tempfile::tempdir().unwrap();
    let base = canon(t.path());
    (t, base)
}

/// Canonical, normalized form of an existing folder (what git prints).
pub fn canon(p: &Path) -> PathBuf {
    normalize_path(&fs::canonicalize(p).unwrap())
}

/// The command the fixtures run git with: a fixed identity, no prompt, and never an inherited
/// repository selection (a test run from a git hook must not read or write the hook's repository).
pub fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0");
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"] {
        c.env_remove(var);
    }
    c
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir, args).output().expect("git must be installed to run these tests");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_fixture_git_never_inherits_a_repository_selection() {
    let c = git_command(Path::new("."), &["status"]);
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"] {
        let set = c.get_envs().find(|(k, _)| *k == std::ffi::OsStr::new(var));
        assert!(matches!(set, Some((_, None))), "{var} must be removed from the child environment");
    }
}

/// `dir` (created) as a git repository with one commit; returns its canonical path.
pub fn repo(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    commit(dir, "README.md");
    canon(dir)
}

/// Write `file` (relative) with some content and commit everything.
pub fn commit(repo: &Path, file: &str) {
    let p = repo.join(file);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, format!("{file}\n")).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", &format!("add {file}")]);
}

/// `git worktree add -b <branch> <dest>` from `repo`; returns the canonical worktree path.
pub fn worktree(repo: &Path, dest: &Path, branch: &str) -> PathBuf {
    git(repo, &["worktree", "add", "-q", "-b", branch, &dest.to_string_lossy()]);
    canon(dest)
}

/// `files` files of `bytes` bytes each inside `dir` (created).
pub fn fill(dir: &Path, files: usize, bytes: usize) {
    fs::create_dir_all(dir).unwrap();
    let data = vec![b'x'; bytes];
    for i in 0..files {
        fs::write(dir.join(format!("f{i}.bin")), &data).unwrap();
    }
}

/// A Rust build folder of about `bytes` inside `project` (Cargo.toml + target/CACHEDIR.TAG).
pub fn rust_build(project: &Path, bytes: usize) -> PathBuf {
    fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let target = project.join("target");
    fill(&target.join("debug"), 1, bytes);
    fs::write(target.join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
    target
}

/// A verified node_modules of about `bytes` inside `project`.
pub fn node_modules(project: &Path, bytes: usize) -> PathBuf {
    fs::write(project.join("package.json"), "{}\n").unwrap();
    let nm = project.join("node_modules");
    fill(&nm.join("pkg"), 1, bytes);
    fs::write(nm.join(".package-lock.json"), "{}\n").unwrap();
    nm
}

/// Two days, in milliseconds.
pub const TWO_DAYS_MS: i64 = 2 * 24 * 60 * 60 * 1000;

/// No sessions, no archived sessions, nothing searched. The clock is two days after the test
/// made its folders: nothing is recent unless a test sets its own clock.
pub fn env() -> Env<'static> {
    Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms() + TWO_DAYS_MS,
        health: Default::default(),
        history: crate::history::HistoryInput::none(),
    }
}

pub const MB: usize = 1 << 20;

/// `git clone` of `src` into `dest` (not checked out with line-ending conversion).
pub fn clone(src: &Path, dest: &Path) -> PathBuf {
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    git(src, &["clone", "-q", "-c", "core.autocrlf=false", &src.to_string_lossy(), &dest.to_string_lossy()]);
    canon(dest)
}

/// A junction (or symlink) at `link` leading to `target`; `false` when this machine cannot make one.
pub fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(target, link).is_ok();
    made
}
