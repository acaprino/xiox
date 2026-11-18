//! The session history end to end: the real `xiox` reads a fake home's stores through the real
//! `mukade`. In a test binary of its own, so these heavier runs never share the machine with
//! the timing of the observer and MCP tests in `cli.rs`.

// The test switches exist in debug builds only: without them a release binary would read the
// real drives and sessions.
#![cfg(debug_assertions)]

mod common;

use common::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

    // Confirmed, in a dry run: only the controlled test below reaches the real Recycle Bin.
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
