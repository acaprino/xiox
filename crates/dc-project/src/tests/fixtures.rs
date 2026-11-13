//! Fake trees and real git repositories under a temp dir.

use crate::Env;
use dc_model::normalize_path;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// No sessions, no archived sessions, nothing searched.
pub fn env() -> Env<'static> {
    Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: dc_app::now_ms(),
        health: Default::default(),
    }
}

pub const MB: usize = 1 << 20;
