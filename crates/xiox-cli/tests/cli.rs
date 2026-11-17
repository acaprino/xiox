//! Runs the built `xiox` on fake trees. Never touches a real project or a real session:
//! `XIOX_SEARCH_ROOTS` replaces the drives and the home, `XIOX_NO_SESSIONS` skips the session
//! listing (both are read by debug builds only).

// The test switches (`XIOX_SEARCH_ROOTS`, `XIOX_NO_SESSIONS`) exist in debug builds only: without them
// a release binary would read the real drives and sessions.
#![cfg(debug_assertions)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Variables that make git work on a repository chosen by the environment (set when the tests run
/// from a git hook): never passed to `xiox` or to the git the tests run.
const GIT_SELECTION: [&str; 5] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_PREFIX"];

fn xiox() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xiox"));
    c.env("XIOX_NO_SESSIONS", "1");
    c.env_remove("XIOX_SEARCH_ROOTS");
    c.env("GIT_TERMINAL_PROMPT", "0");
    for var in GIT_SELECTION {
        c.env_remove(var);
    }
    c
}

#[test]
fn the_test_commands_never_inherit_a_repository_selection() {
    let (x, g) = (xiox(), git_command(Path::new("."), &["status"]));
    for c in [&x, &g] {
        for var in GIT_SELECTION {
            let set = c.get_envs().find(|(k, _)| *k == std::ffi::OsStr::new(var));
            assert!(matches!(set, Some((_, None))), "{var} must be removed from the child environment");
        }
    }
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
    assert_eq!(v["schema"], 2);
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

fn git_command(dir: &Path, args: &[&str]) -> Command {
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

fn git(dir: &Path, args: &[&str]) {
    let out = git_command(dir, args).output().expect("git must be installed to run these tests");
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
    // The summary was shown before refusing, on stderr (where the question would be asked).
    assert!(text(&out.stderr).contains("permanently"), "{}", text(&out.stderr));
    assert!(out.stdout.is_empty(), "{}", text(&out.stdout));
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
    old_clean_clone(&named);
    let out = run_clean(&project, &["--include", &named.to_string_lossy(), "--yes", "--permanent"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}\nstdout: {stdout}", text(&out.stderr));
    assert!(!named.exists());
    assert!(project.join("target").exists(), "no category was named");
    assert!(stdout.contains("matched by name only"), "an --include is listed prominently: {stdout}");
}

#[test]
fn clean_permanent_refuses_a_danger_folder_and_it_survives() {
    let (_t, project) = fake_project();
    let named = project.parent().unwrap().join("fakeproj-old");
    std::fs::create_dir_all(&named).unwrap();
    std::fs::write(named.join("a.txt"), "x").unwrap();
    let out = run_clean(&project, &["--include", &named.to_string_lossy(), "--yes", "--permanent"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("refusing --permanent: danger folders go to the Recycle Bin only"), "{err}");
    assert!(named.join("a.txt").exists());
}

/// `dir` as a clean clone of a new, unrelated repository, dated three days back: git vouches for
/// it and it is not recent, so it is not `danger`.
fn old_clean_clone(dir: &Path) {
    let parent = dir.parent().unwrap();
    let upstream = parent.join("upstream");
    std::fs::create_dir_all(&upstream).unwrap();
    git(&upstream, &["init", "-q", "-b", "main"]);
    std::fs::write(upstream.join("a.txt"), "x").unwrap();
    git(&upstream, &["add", "-A"]);
    git(&upstream, &["commit", "-q", "-m", "first"]);
    git(parent, &["clone", "-q", &upstream.to_string_lossy(), &dir.to_string_lossy()]);
    age(dir);
}

/// Moves the modified date of `dir` three days back: the folder is no longer recent activity.
fn age(dir: &Path) {
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut open, 0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS
    let three_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 60 * 60);
    open.open(dir).unwrap().set_times(std::fs::FileTimes::new().set_modified(three_days_ago)).unwrap();
}

#[test]
fn clean_worktrees_removes_the_clean_one_and_says_git_removes_them_permanently() {
    let (_t, project, wt) = project_with_worktree();
    age(&wt);
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
    assert_eq!(v["schema"], 2);
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
    assert_eq!(v["schema"], 2);
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

// ───────────────────────────── final review ─────────────────────────────

/// The fake project as a git repository, and `fakeproj-old` next to it holding `copy`: a clone with a
/// commit of its own, so the copy is blocked and the folder around it is only a name match.
fn project_with_a_named_folder_holding_a_copy_with_work() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let (t, project) = fake_project();
    std::fs::remove_dir_all(project.join("target")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "first"]);
    let named = project.parent().unwrap().join("fakeproj-old");
    std::fs::create_dir_all(&named).unwrap();
    let copy = named.join("copy");
    git(project.parent().unwrap(), &["clone", "-q", &project.to_string_lossy(), &copy.to_string_lossy()]);
    std::fs::write(copy.join("private.txt"), "only here\n").unwrap();
    git(&copy, &["add", "-A"]);
    git(&copy, &["commit", "-q", "-m", "private"]);
    (t, project, named, copy)
}

#[test]
fn c1_include_of_a_named_folder_that_holds_a_copy_with_work_exits_2_and_the_copy_survives() {
    let (_t, project, named, copy) = project_with_a_named_folder_holding_a_copy_with_work();
    let out = run_clean(&project, &["--include", &named.to_string_lossy(), "--yes", "--permanent"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}\nstderr: {}", text(&out.stdout), text(&out.stderr));
    let err = text(&out.stderr);
    assert!(err.contains("cannot be removed: it holds "), "{err}");
    assert!(err.contains("has commits or changes the project does not have"), "{err}");
    assert!(copy.join("private.txt").exists(), "the copy with its private commit was removed");
    assert!(named.exists());
}

#[test]
fn c1_with_no_drives_the_copy_is_not_listed_but_the_folder_is_still_not_removed() {
    let (_t, project, named, copy) = project_with_a_named_folder_holding_a_copy_with_work();
    // Without git the folder is danger: --permanent is refused before anything is checked.
    let path = named.to_string_lossy().into_owned();
    let out = run_clean(&project, &["--include", &path, "--yes", "--permanent", "--no-drives"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}\nstderr: {}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stderr).contains("refusing --permanent"), "{}", text(&out.stderr));
    // Confirmed, in a dry run (the Recycle Bin is never reached from a test): the removal-time
    // check still finds the repository the analysis did not list.
    let out = run_clean(&project, &["--include", &path, "--confirm-danger", &path, "--yes", "--dry-run", "--no-drives"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}\nstderr: {}", text(&out.stderr));
    assert!(stdout.contains("skipped") && stdout.contains("git repository"), "{stdout}");
    assert!(copy.join("private.txt").exists(), "the copy with its private commit was removed");
}

#[test]
fn clean_danger_without_confirmation_exits_2() {
    let (_t, project) = fake_project();
    let named = project.parent().unwrap().join("fakeproj-old");
    std::fs::create_dir_all(&named).unwrap();
    std::fs::write(named.join("a.txt"), "x").unwrap();
    let path = named.to_string_lossy().into_owned();
    let out = run_clean(&project, &["--include", &path, "--yes", "--no-history"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("danger: explicit confirmation required (no git repository of its own"), "{err}");
    assert!(err.contains(&format!("add --confirm-danger \"{path}\"")), "{err}");
    assert!(named.join("a.txt").exists());

    // Confirmed, in a dry run: no removal ever reaches the real Recycle Bin from a test.
    let out = run_clean(&project, &["--include", &path, "--confirm-danger", &path, "--yes", "--dry-run", "--no-history"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}\nstderr: {}", text(&out.stderr));
    assert!(stdout.contains("would remove"), "{stdout}");
    assert!(named.join("a.txt").exists());
}

#[test]
fn a_folder_holding_several_repositories_refuses_the_category_flags_with_exit_2() {
    let (_t, project) = fake_project();
    let work = project.parent().unwrap().to_path_buf();
    for name in ["alpha", "beta"] {
        let r = work.join(name);
        std::fs::create_dir_all(&r).unwrap();
        git(&r, &["init", "-q", "-b", "main"]);
    }
    // `analyze` still works, with a note.
    let out = run_analyze(&work, &["--no-drives"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert!(
        stdout.contains("note: this folder is a drive root, your user profile, or holds several projects"),
        "{stdout}"
    );
    assert!(!stdout.contains("Clean with:"), "{stdout}");

    let out = xiox()
        .args(["clean"])
        .arg(&work)
        .args(["--builds", "--yes", "--permanent", "--no-drives"])
        .env("XIOX_SEARCH_ROOTS", work.parent().unwrap())
        .env("GIT_CEILING_DIRECTORIES", work.parent().unwrap())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "stdout: {}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("refusing to clean everything under "), "{err}");
    assert!(err.contains("run it on one project"), "{err}");
    assert!(project.join("target").exists(), "nothing was removed");

    // The same folder with an explicit path still works.
    let target = project.join("target");
    let out = xiox()
        .args(["clean"])
        .arg(&work)
        .args(["--include"])
        .arg(&target)
        .args(["--yes", "--permanent", "--no-drives"])
        .env("XIOX_SEARCH_ROOTS", work.parent().unwrap())
        .env("GIT_CEILING_DIRECTORIES", work.parent().unwrap())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stdout: {}\nstderr: {}", text(&out.stdout), text(&out.stderr));
    assert!(!target.exists());
}

// ───────────────────────────────── observe ─────────────────────────────────

/// A running `xiox observe` on a fake project: its stdout lines arrive on a channel (so that no test
/// waits on a pipe without a deadline) and the process is killed when the value is dropped.
struct Observer {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
    stdin: Option<std::process::ChildStdin>,
}

impl Observer {
    fn spawn(project: &Path, extra: &[&str]) -> Observer {
        use std::io::BufRead;
        let mut child = xiox()
            .arg("observe")
            .arg(project)
            .args(extra)
            .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
            .env("GIT_CEILING_DIRECTORIES", project.parent().unwrap().parent().unwrap())
            .env("XIOX_OBSERVE_INTERVAL_MS", "200")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        Observer { child, lines, stdin }
    }

    /// The next stdout line, parsed; panics after `secs` seconds.
    fn next(&self, secs: u64) -> serde_json::Value {
        let line = self
            .lines
            .recv_timeout(std::time::Duration::from_secs(secs))
            .unwrap_or_else(|e| panic!("no line on stdout within {secs}s: {e}"));
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line:?}"))
    }

    /// Lines until one has this event; returns it with the ones read before it.
    fn until(&self, event: &str, secs: u64) -> (serde_json::Value, Vec<serde_json::Value>) {
        let mut before = Vec::new();
        loop {
            let v = self.next(secs);
            if v["event"] == event {
                return (v, before);
            }
            before.push(v);
        }
    }

    /// Waits for the process to end on its own; panics (after killing it) past `secs`.
    fn exit_code(&mut self, secs: u64) -> Option<i32> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            if std::time::Instant::now() > end {
                let _ = self.child.kill();
                panic!("xiox observe did not exit within {secs}s");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The fake project as a git repository with one commit (only `Cargo.toml`: the build folder stays
/// untracked).
fn repo_project() -> (tempfile::TempDir, PathBuf) {
    let (t, project) = fake_project();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["add", "Cargo.toml"]);
    git(&project, &["commit", "-q", "-m", "first"]);
    (t, project)
}

#[test]
fn observe_binary_prints_started_then_a_changed_event_after_a_worktree_is_added() {
    let (_t, project) = repo_project();
    let mut o = Observer::spawn(&project, &["--no-drives"]);
    let started = o.next(60);
    assert_eq!(started["event"], "started");
    assert_eq!(started["schema"], 2);
    assert_eq!(started["project"]["name"], "fakeproj");
    // The baseline has weights: the 3 MB build folder is counted at the start.
    assert!(started["totals"]["byCategory"]["builds"].as_u64().unwrap() >= 3 * 1024 * 1024, "{started}");

    let wt = project.parent().unwrap().join("wt");
    git(&project, &["worktree", "add", "-q", "-b", "side", &wt.to_string_lossy()]);

    // `git worktree add` creates the folder before it registers the worktree: a check that runs in
    // between lists it as a copy, and the next one as a worktree. Both are right; what is checked
    // is the final state, waited for with a bound (every `next` has its own deadline).
    let mut seen = Vec::new();
    let mut category = String::new();
    while category != "worktrees" {
        let line = o.next(60);
        assert_eq!(line["event"], "changed", "only changes until the worktree is listed: {line}");
        assert_eq!(line["schema"], 2);
        for added in line["added"].as_array().unwrap() {
            if added["path"].as_str().unwrap().ends_with("wt") {
                assert_eq!(added["new"], true, "{line}");
                category = added["category"].as_str().unwrap().to_string();
            }
        }
        seen.push(line);
        assert!(seen.len() <= 10, "the worktree is still not listed as one: {seen:?}");
    }

    // `report` on stdin answers with the whole list, the worktree marked new and the build folder not.
    use std::io::Write;
    o.stdin.as_mut().unwrap().write_all(b"report\n").unwrap();
    let (report, _) = o.until("report", 60);
    assert_eq!(report["stale"], false);
    let items = report["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["category"] == "worktrees" && i["new"] == true), "{report}");
    assert!(items.iter().any(|i| i["category"] == "builds" && i["new"] == false), "{report}");

    // `stop` ends it with 0 and nothing more on stdout.
    o.stdin.as_mut().unwrap().write_all(b"stop\n").unwrap();
    assert_eq!(o.exit_code(30), Some(0));
    assert!(o.lines.recv_timeout(std::time::Duration::from_millis(500)).is_err());
}

#[test]
fn observe_binary_exits_when_stdin_closes() {
    let (_t, project) = fake_project();
    let mut o = Observer::spawn(&project, &["--no-drives"]);
    assert_eq!(o.next(60)["event"], "started");
    drop(o.stdin.take());
    assert_eq!(o.exit_code(30), Some(0));
    assert!(o.lines.recv_timeout(std::time::Duration::from_millis(500)).is_err());
}

#[test]
fn observe_binary_interval_below_60_exits_2() {
    // The debug-only override must not rescue a bad flag: the flag itself is checked first.
    let out = xiox().args(["observe", "--interval", "30"]).env("XIOX_OBSERVE_INTERVAL_MS", "200").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--interval"));
}

#[test]
fn observe_binary_on_a_missing_project_exits_1_with_empty_stdout() {
    let (_t, project) = fake_project();
    let out = xiox()
        .arg("observe")
        .arg(project.join("nope"))
        .env("XIOX_SEARCH_ROOTS", project.parent().unwrap())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("path not found"));
}

#[test]
fn mcp_binary_answers_initialize_list_and_call_over_stdio() {
    use std::io::Write;
    let (_t, project) = repo_project();
    let mut o = Observer::spawn(&project, &["--no-drives", "--mcp"]);
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_leftovers","arguments":{}}}"#,
    ];
    for line in requests {
        writeln!(o.stdin.as_mut().unwrap(), "{line}").unwrap();
    }
    let init = o.next(60);
    assert_eq!(init["id"], 1, "{init}");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "xiox");
    let list = o.next(60);
    assert_eq!(list["id"], 2, "{list}");
    assert_eq!(list["result"]["tools"][0]["name"], "project_leftovers");
    assert_eq!(list["result"]["tools"][0]["annotations"]["readOnlyHint"], true);
    let call = o.next(60);
    assert_eq!(call["id"], 3, "{call}");
    assert_eq!(call["result"]["isError"], false, "{call}");
    let report = &call["result"]["structuredContent"];
    assert_eq!(report["project"]["name"], "fakeproj");
    assert_eq!(report["stale"], false);
    assert!(report["items"].as_array().unwrap().iter().any(|i| i["category"] == "builds" && i["new"] == false), "{report}");
    // Closing stdin ends it with 0, and nothing else was ever written to stdout.
    drop(o.stdin.take());
    assert_eq!(o.exit_code(30), Some(0));
    assert!(o.lines.recv_timeout(std::time::Duration::from_millis(500)).is_err());
}

// ───────────────────────── the session history, end to end ─────────────────────────

const S1: &str = "11111111-1111-1111-1111-111111111111";
const S2: &str = "22222222-2222-2222-2222-222222222222";

/// The variables a harness's store, or the program that reads it, is taken from: a test's `xiox`
/// never inherits them, so it reads the fake home's stores and nothing of this PC.
const STORE_VARS: [&str; 7] =
    ["XIOX_NO_SESSIONS", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_DB", "PI_CODING_AGENT_DIR", "XIOX_OPENCODE_EXE", "XIOX_MUKADE_EXE"];

/// A git project `<base>\work\phazly` and a home of its own, whose Claude Code store holds the
/// sessions the test writes. The real `xiox` reads them through the real `mukade`.
struct SessionWorld {
    _t: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl SessionWorld {
    fn new() -> SessionWorld {
        let t = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(t.path()).unwrap();
        let verbatim = String::from("\\\\?\\");
        let base = PathBuf::from(canon.to_string_lossy().trim_start_matches(verbatim.as_str()));
        let project = base.join("work").join("phazly");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("README.md"), "phazly\n").unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-q", "-m", "first"]);
        let home = base.join("home");
        std::fs::create_dir_all(home.join("AppData").join("Local").join("Temp")).unwrap();
        SessionWorld { _t: t, base, home, project }
    }

    /// Session `id`, run in `cwd`, made one `Bash` call: `command`.
    fn session(&self, id: &str, cwd: &Path, command: &str) {
        xiox_mukade::test_support::write_claude_session(&self.home.join(".claude"), id, &cwd.to_string_lossy(), command);
    }

    fn xiox(&self) -> Command {
        let mut c = xiox();
        for var in STORE_VARS {
            c.env_remove(var);
        }
        let local = self.home.join("AppData").join("Local");
        c.env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("LOCALAPPDATA", &local)
            .env("TEMP", local.join("Temp"))
            .env("TMP", local.join("Temp"))
            .env("XIOX_SEARCH_ROOTS", &self.base)
            .env("GIT_CEILING_DIRECTORIES", self.base.parent().unwrap());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.xiox().args(args).output().unwrap()
    }
}

/// Exit 0 and stdout as one JSON value.
fn json_of(out: &Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "stdout: {}\nstderr: {}", text(&out.stdout), text(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("stdout is exactly one JSON value")
}

/// The item of `scan` at `path` (paths compare as Windows does, without case).
fn item_at<'a>(scan: &'a serde_json::Value, path: &Path) -> &'a serde_json::Value {
    let want = path.to_string_lossy().to_lowercase();
    let items = scan["items"].as_array().expect("items");
    items
        .iter()
        .find(|i| i["path"].as_str().is_some_and(|p| p.to_lowercase() == want))
        .unwrap_or_else(|| panic!("{want} not listed: {items:#?}"))
}

/// `<base>\runs\bench-1`, made by session S1 of the project.
fn world_with_a_bench() -> (SessionWorld, PathBuf) {
    let w = SessionWorld::new();
    let bench = w.base.join("runs").join("bench-1");
    std::fs::create_dir_all(&bench).unwrap();
    std::fs::write(bench.join("a.txt"), "x").unwrap();
    w.session(S1, &w.project, &format!("mkdir \"{}\"", bench.display()));
    (w, bench)
}

#[test]
fn the_session_world_reads_only_its_own_store() {
    let (w, _bench) = world_with_a_bench();
    let scan = json_of(&w.run(&["analyze", &w.project.to_string_lossy(), "--json"]));
    let sources = scan["history"]["sources"].as_array().expect("history.sources");
    let listed: Vec<(String, u64)> = sources.iter().map(|s| (s["agent"].to_string(), s["listed"].as_u64().unwrap())).collect();
    assert_eq!(listed.iter().map(|(_, n)| n).sum::<u64>(), 1, "only the one session of the fake store: {listed:?}");
    assert_eq!(scan["history"]["sessionsRead"], 1, "{}", scan["history"]);
}

#[test]
fn analyze_lists_what_a_session_made_end_to_end() {
    let (w, bench) = world_with_a_bench();
    let project = w.project.to_string_lossy().into_owned();
    let scan = json_of(&w.run(&["analyze", &project, "--json"]));
    let item = item_at(&scan, &bench);
    assert_eq!((&item["category"], &item["level"]), (&"scratch".into(), &"danger".into()), "{item:#}");
    assert!(item["risks"].as_array().unwrap().contains(&"no-git".into()), "{item:#}");
    let session = &item["provenance"]["sessions"][0];
    assert_eq!(session["harness"], "claude_code", "{session:#}");
    let op = &session["operations"][0];
    assert!(!op["entry"].is_null() && !op["call"].is_null(), "the operation names its entry and call: {op:#}");

    let out = w.run(&["analyze", &project]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert!(text(&out.stdout).contains("danger: explicit confirmation required: no git repository of its own"), "{}", text(&out.stdout));
}

#[test]
fn clean_takes_a_scratch_item_only_with_its_confirmation_end_to_end() {
    let (w, bench) = world_with_a_bench();
    let (project, b) = (w.project.to_string_lossy().into_owned(), bench.to_string_lossy().into_owned());
    let out = w.run(&["clean", &project, "--include", &b, "--yes"]);
    assert_eq!(out.status.code(), Some(2), "stdout: {}\nstderr: {}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stderr).contains(&format!("add --confirm-danger \"{b}\"")), "{}", text(&out.stderr));
    assert!(bench.join("a.txt").exists());

    // Confirmed, in a dry run: only Task 2's controlled test reaches the real Recycle Bin.
    let v = json_of(&w.run(&["clean", &project, "--include", &b, "--confirm-danger", &b, "--dry-run", "--json"]));
    let done = &v["outcome"]["items"][0];
    assert_eq!((&done["outcome"]["type"], &done["method"]), (&"removed".into(), &"recycle-bin".into()), "{done:#}");
    assert_eq!(v["chosen"]["confirmed"][0]["risks"], item_at(&v["scan"], &bench)["risks"], "{:#}", v["chosen"]);
    assert!(bench.join("a.txt").exists());
}

#[test]
fn a_confirmed_danger_folder_goes_to_the_real_recycle_bin() {
    let (w, bench) = world_with_a_bench();
    let (project, b) = (w.project.to_string_lossy().into_owned(), bench.to_string_lossy().into_owned());
    let out = w.run(&["clean", &project, "--include", &b, "--confirm-danger", &b, "--yes", "--json"]);
    // The real Recycle Bin: the test empties its own entry before any check, so a failing assert
    // never leaves it there.
    let want = b.to_lowercase();
    let mine: Vec<_> = trash::os_limited::list()
        .unwrap()
        .into_iter()
        .filter(|e| e.original_path().to_string_lossy().to_lowercase() == want)
        .collect();
    trash::os_limited::purge_all(&mine).unwrap();
    let v = json_of(&out);
    assert_eq!(mine.len(), 1, "the folder is not in the Recycle Bin: {:#}", v["outcome"]);
    assert!(!bench.exists());
    assert!(v["outcome"]["recycledBytes"].as_u64().unwrap() > 0, "{:#}", v["outcome"]);
}

#[test]
fn a_folder_two_projects_wrote_in_is_shown_blocked_end_to_end() {
    let w = SessionWorld::new();
    let shared = w.base.join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("a.txt"), "x").unwrap();
    std::fs::write(shared.join("b.txt"), "y").unwrap();
    let (a, b) = (shared.join("a.txt"), shared.join("b.txt"));
    w.session(S1, &w.project, &format!("mkdir \"{}\" && echo x > \"{}\"", shared.display(), a.display()));
    w.session(S2, &shared, &format!("echo y > \"{}\"", b.display()));
    let project = w.project.to_string_lossy().into_owned();

    let scan = json_of(&w.run(&["analyze", &project, "--json"]));
    let item = item_at(&scan, &shared);
    assert_eq!(item["blocked"], serde_json::json!(["other-project-writes"]), "{item:#}");
    assert_eq!(item["provenance"]["otherProjects"][0]["id"], S2, "{item:#}");
    let out = w.run(&["analyze", &project]);
    assert!(text(&out.stdout).contains("sessions of another project wrote here"), "{}", text(&out.stdout));
}
