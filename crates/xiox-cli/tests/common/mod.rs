//! What every integration test of the `xiox` binary needs: the binary with no session of this
//! PC and no inherited repository selection, and a git that never asks for anything.

use std::path::Path;
use std::process::Command;

/// Variables that make git work on a repository chosen by the environment (set when the tests run
/// from a git hook): never passed to `xiox` or to the git the tests run.
pub const GIT_SELECTION: [&str; 5] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"];

pub fn xiox() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xiox"));
    c.env("XIOX_NO_SESSIONS", "1");
    c.env_remove("XIOX_SEARCH_ROOTS");
    c.env("GIT_TERMINAL_PROMPT", "0");
    for var in GIT_SELECTION {
        c.env_remove(var);
    }
    c
}

pub fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

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
    for var in GIT_SELECTION {
        c.env_remove(var);
    }
    c
}

pub fn git(dir: &Path, args: &[&str]) {
    let out = git_command(dir, args).output().expect("git must be installed to run these tests");
    assert!(out.status.success(), "git {args:?} failed: {}", text(&out.stderr));
}
