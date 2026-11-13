//! Runs the built `xiox` on fake trees. Never touches a real project or a real session:
//! `XIOX_SEARCH_ROOTS` replaces the drives and the home, `XIOX_NO_SESSIONS` skips the session
//! listing (both are read by debug builds only).

// The test switches (`XIOX_SEARCH_ROOTS`, `XIOX_NO_SESSIONS`) exist in debug builds only: without them
// a release binary would read the real drives and sessions.
#![cfg(debug_assertions)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn xiox() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xiox"));
    c.env("XIOX_NO_SESSIONS", "1");
    c.env_remove("XIOX_SEARCH_ROOTS");
    c.env("GIT_TERMINAL_PROMPT", "0");
    c
}

/// A fake project (not a git repository) with a Rust `target` folder; returns (temp dir, project).
fn fake_project() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let canon = std::fs::canonicalize(t.path()).unwrap();
    // `canonicalize` gives the verbatim form (backslash, backslash, question mark, backslash, then the
    // drive); xiox prints the plain one.
    let verbatim = String::from("\\\\?\\");
    let base = PathBuf::from(canon.to_string_lossy().trim_start_matches(verbatim.as_str()));
    let project = base.join("work").join("fakeproj");
    std::fs::create_dir_all(project.join("target").join("debug")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(project.join("target").join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
    std::fs::write(project.join("target").join("debug").join("big.bin"), vec![b'x'; 3 * 1024 * 1024]).unwrap();
    (t, project)
}

fn run_analyze(project: &Path, extra: &[&str]) -> Output {
    xiox()
        .arg("analyze")
        .arg(project)
        .args(extra)
        .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
        .env("GIT_CEILING_DIRECTORIES", project.parent().unwrap().parent().unwrap())
        .output()
        .unwrap()
}

#[test]
fn analyze_on_a_fake_project_exits_0_and_lists_its_target_folder() {
    let (_t, project) = fake_project();
    let out = run_analyze(&project, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("Project  fakeproj"), "{stdout}");
    assert!(stdout.contains("BUILDS"), "{stdout}");
    assert!(stdout.contains(&project.join("target").to_string_lossy().into_owned()), "{stdout}");
    assert!(stdout.contains("Rust build"), "{stdout}");
    assert!(stdout.contains("(not a git repository)"), "{stdout}");
    assert!(stdout.contains("Clean with: xiox clean"), "{stdout}");
    assert!(stdout.contains("--builds"), "{stdout}");
}

#[test]
fn analyze_json_stdout_is_one_json_object() {
    let (_t, project) = fake_project();
    let out = run_analyze(&project, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is exactly one JSON value");
    assert!(v.is_object());
    assert_eq!(v["schema"], 1);
    assert_eq!(v["project"]["name"], "fakeproj");
    assert_eq!(v["items"][0]["category"], "builds");
    assert!(out.stdout.ends_with(b"}\n"), "one object and a newline");
    assert_eq!(out.stdout.iter().filter(|&&b| b == b'\n').count(), 1);
    // Progress goes to stderr, not into the JSON.
    assert!(!out.stderr.is_empty());
}

#[test]
fn analyze_missing_path_exits_1_and_bad_flag_exits_2() {
    let (_t, project) = fake_project();
    let missing = project.join("nope");
    let out = xiox().arg("analyze").arg(&missing).env("XIOX_SEARCH_ROOTS", project.parent().unwrap()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: path not found: "), "{err}");
    assert!(err.contains("nope"), "{err}");

    let out = xiox().args(["analyze", "--bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown flag: --bogus"));

    let out = xiox().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = xiox().args(["clean", "--copies"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--include <path>"));
}

#[test]
fn help_and_version_exit_0() {
    let out = xiox().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("xiox analyze") && s.contains("xiox clean"), "{s}");
    let out = xiox().arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("xiox {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn without_a_project_argument_the_current_folder_is_the_project() {
    let (_t, project) = fake_project();
    let out = xiox()
        .arg("analyze")
        .current_dir(&project)
        .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
        // The temp folder may sit inside someone's repository: git must not look above it.
        .env("GIT_CEILING_DIRECTORIES", project.parent().unwrap().parent().unwrap())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("Project  fakeproj"), "{stdout}");
    assert!(stdout.contains(&project.join("target").to_string_lossy().into_owned()), "{stdout}");
}

// ───────────────────────────── clean ─────────────────────────────
//
// Every test that removes something passes `--permanent` (or `--dry-run`): the real Recycle Bin
// is never used. The one exception is the worktree test, where git removes the folder and the
// bin is not involved. Stdin of a spawned process is not a terminal.

fn run_clean(project: &Path, extra: &[&str]) -> Output {
    xiox()
        .arg("clean")
        .arg(project)
        .args(extra)
        .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
        .env("GIT_CEILING_DIRECTORIES", project.parent().unwrap().parent().unwrap())
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git must be installed to run these tests");
    assert!(out.status.success(), "git {args:?} failed: {}", text(&out.stderr));
}

/// The fake project as a git repository with one commit, and a linked worktree `wt` next to it.
fn project_with_worktree() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (t, project) = fake_project();
    // Keep the build folder out of the commit: the worktree is then the only chosen thing.
    std::fs::remove_dir_all(project.join("target")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "first"]);
    let wt = project.parent().unwrap().join("wt");
    git(&project, &["worktree", "add", "-q", "-b", "side", &wt.to_string_lossy()]);
    (t, project, wt)
}

#[test]
fn clean_without_flags_lists_and_removes_nothing() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &[]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(stdout.starts_with("Project  fakeproj"), "{stdout}");
    assert!(stdout.contains("BUILDS"), "{stdout}");
    assert!(
        stdout.trim_end().ends_with(
            "Nothing selected. Name what to clean: --builds --worktrees --logs --sessions --include <path>"
        ),
        "{stdout}"
    );
    assert!(project.join("target").join("debug").join("big.bin").exists());
}

#[test]
fn clean_builds_yes_permanent_removes_target_and_exits_0() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--builds", "--yes", "--permanent"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(!project.join("target").exists());
    assert!(project.join("Cargo.toml").exists(), "only the named category goes");
    assert!(stdout.contains("permanently"), "{stdout}");
    assert!(stdout.contains("Freed 3 MB"), "{stdout}");
    assert!(!stdout.contains("Recycle Bin"), "permanent mode never mentions the bin: {stdout}");
}

#[test]
fn clean_without_yes_and_without_a_terminal_exits_2_and_removes_nothing() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--builds", "--permanent"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    assert!(text(&out.stderr).contains("not a terminal: pass --yes to remove without asking"), "{}", text(&out.stderr));
    assert!(project.join("target").exists());
    // The summary was shown before refusing.
    assert!(text(&out.stdout).contains("permanently"), "{}", text(&out.stdout));
}

#[test]
fn clean_json_without_yes_exits_2_and_removes_nothing() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--builds", "--permanent", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("--json needs --yes to remove anything"), "{}", text(&out.stderr));
    assert!(project.join("target").exists());
}

#[test]
fn clean_dry_run_exits_0_and_the_folder_is_still_there() {
    let (_t, project) = fake_project();
    // Recycle mode on purpose: a dry run never reaches the bin.
    let out = run_clean(&project, &["--builds", "--dry-run"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(stdout.contains("to the Recycle Bin"), "{stdout}");
    assert!(stdout.contains("Dry run: nothing was removed."), "{stdout}");
    assert!(stdout.contains("Would free 3 MB"), "{stdout}");
    assert!(project.join("target").join("debug").join("big.bin").exists());
}

#[test]
fn clean_include_of_an_unknown_path_exits_2() {
    let (_t, project) = fake_project();
    let unknown = project.join("src");
    std::fs::create_dir_all(&unknown).unwrap();
    let out = run_clean(&project, &["--include", &unknown.to_string_lossy(), "--yes", "--permanent"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("not found in the analysis of this project"), "{err}");
    assert!(unknown.exists() && project.join("target").exists());
}

#[test]
fn clean_include_removes_a_name_matched_folder() {
    let (_t, project) = fake_project();
    let named = project.parent().unwrap().join("fakeproj-old");
    std::fs::create_dir_all(&named).unwrap();
    std::fs::write(named.join("a.txt"), "x").unwrap();
    let out = run_clean(&project, &["--include", &named.to_string_lossy(), "--yes", "--permanent"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(!named.exists());
    assert!(project.join("target").exists(), "no category was named");
    assert!(stdout.contains("matched by name only"), "an --include is listed prominently: {stdout}");
}

#[test]
fn clean_worktrees_removes_the_clean_one_and_says_git_removes_them_permanently() {
    let (_t, project, wt) = project_with_worktree();
    let out = run_clean(&project, &["--worktrees", "--yes"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(stdout.contains("Worktrees are removed by git and do not go to the Recycle Bin."), "{stdout}");
    assert!(!wt.exists(), "{stdout}");
    assert!(project.join("Cargo.toml").exists());
}

#[test]
fn clean_include_of_a_dirty_worktree_exits_2_and_it_survives() {
    let (_t, project, wt) = project_with_worktree();
    std::fs::write(wt.join("unsaved.txt"), "work in progress").unwrap();
    let out = run_clean(&project, &["--include", &wt.to_string_lossy(), "--yes", "--permanent"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    assert!(text(&out.stderr).contains("cannot be removed (uncommitted changes)"), "{}", text(&out.stderr));
    assert!(wt.join("unsaved.txt").exists());
}

#[test]
fn clean_json_yes_stdout_is_one_object_with_scan_and_outcome() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--builds", "--yes", "--permanent", "--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is exactly one JSON value");
    assert_eq!(v["schema"], 1);
    assert_eq!(v["scan"]["project"]["name"], "fakeproj");
    assert_eq!(v["scan"]["items"][0]["category"], "builds");
    assert_eq!(v["outcome"]["mode"], "permanent");
    assert_eq!(v["outcome"]["dryRun"], false);
    assert_eq!(v["outcome"]["items"][0]["category"], "builds");
    assert_eq!(v["outcome"]["items"][0]["outcome"]["type"], "removed");
    assert!(v["outcome"]["bytesFreed"].as_u64().unwrap() >= 3 * 1024 * 1024);
    assert!(out.stdout.ends_with(b"}\n") && out.stdout.iter().filter(|&&b| b == b'\n').count() == 1);
    assert!(!project.join("target").exists());
}

#[test]
fn clean_json_with_nothing_named_prints_the_scan_and_a_null_outcome() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], 1);
    assert!(v["scan"].is_object() && v["outcome"].is_null());
    assert!(project.join("target").exists());
}

#[test]
fn clean_with_flags_but_nothing_eligible_says_so_and_exits_0() {
    let (_t, project) = fake_project();
    let out = run_clean(&project, &["--logs", "--yes", "--permanent"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Nothing to clean."), "{}", text(&out.stdout));
    assert!(project.join("target").exists());
}

#[test]
#[cfg(windows)]
fn clean_exits_3_when_an_item_cannot_be_removed_and_says_why() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_t, project) = fake_project();
    // A file held open without share-delete: Windows refuses to delete it (and so its folder).
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(project.join("target").join("debug").join("big.bin"))
        .unwrap();
    let out = run_clean(&project, &["--builds", "--yes", "--permanent"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(3), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(stdout.contains("failed"), "{stdout}");
    assert!(stdout.contains("Not everything was removed: 1 item skipped or failed."), "{stdout}");
    assert!(project.join("target").join("debug").join("big.bin").exists());
    drop(held);
}
