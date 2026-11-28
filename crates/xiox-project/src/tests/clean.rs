//! Selection, re-validation and removal, on fake trees and real git repositories under a temp
//! dir. The Recycle Bin is never touched for real: files go through a fake `Trash`, and the
//! engine's folder recycling (the `trash` crate) is not exercised here.

use super::fixtures::*;
use super::history::fake::{bash, descriptor, input, listing, FakeCalls};
use crate::history::{ReadFailure, UserDirs};
use crate::clean::{Io, check_dir, english_error, strip_os_text, clean_with, overlaps_project, shortfall_with};
use crate::*;
use xiox_model::{Agent, ArtifactKind, RemoveMode, path_key};
use xiox_mukade::{ArchivedSession, Descriptor, ToolCall};
use xiox_sessions::{HarnessCli, Trash};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MB64: u64 = MB as u64;

fn scan_in(p: &Path, env: &Env) -> ProjectScan {
    analyze_with(p, &Options::default(), env).unwrap()
}

fn scan_of(p: &Path) -> ProjectScan {
    scan_in(p, &env())
}

fn req(categories: &[Category], include: &[&Path]) -> CleanRequest {
    CleanRequest {
        categories: categories.to_vec(),
        include: include.iter().map(|p| p.to_path_buf()).collect(),
        confirm_danger: Vec::new(),
        mode: RemoveMode::Permanent,
    }
}

struct NoCli;
impl HarnessCli for NoCli {
    fn codex_delete(&self, _: &str) -> Result<(), String> {
        Err("no cli in tests".into())
    }
    fn opencode_export(&self, _: &str, _: &Path) -> Result<(), String> {
        Err("no cli in tests".into())
    }
    fn opencode_delete(&self, _: &str) -> Result<(), String> {
        Err("no cli in tests".into())
    }
}

/// A "Recycle Bin" that moves into a folder of the test and remembers what it was given.
pub(super) struct FakeBin {
    dir: PathBuf,
    taken: Mutex<Vec<PathBuf>>,
}
impl FakeBin {
    pub(super) fn new(dir: PathBuf) -> FakeBin {
        FakeBin { dir, taken: Mutex::new(Vec::new()) }
    }
}
impl Trash for FakeBin {
    fn to_bin(&self, path: &Path) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let n = self.taken.lock().unwrap().len();
        fs::rename(path, self.dir.join(format!("{n}-{}", path.file_name().unwrap().to_string_lossy())))
            .map_err(|e| e.to_string())?;
        self.taken.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

pub(super) fn run_with(
    scan: &ProjectScan,
    chosen: &Chosen,
    mode: RemoveMode,
    dry: bool,
    env: &Env,
    bin: &dyn Trash,
    fresh: &dyn Fn(Agent) -> Vec<ArchivedSession>,
) -> CleanOutcome {
    clean_with(scan, chosen, mode, dry, env, &Io { cli: &NoCli, trash: bin, fresh })
}

fn run(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, dry: bool) -> CleanOutcome {
    let bin = FakeBin::new(PathBuf::from("unused"));
    run_with(scan, chosen, mode, dry, &env(), &bin, &|_| Vec::new())
}

/// `dir`, named after the project, as a clean clone of an unrelated repository: git vouches for
/// it and on the fixture clock it is old, so it is neither blocked nor `danger`.
fn named_clone(base: &Path, dir: &Path) -> PathBuf {
    clone(&repo(&base.join("upstream")), dir)
}

/// The outcome of a log file `k`.
fn log_outcome(outcome: Outcome) -> OutcomeItem {
    OutcomeItem { key: "k".into(), path: None, category: Category::Logs, method: Method::Permanent, outcome }
}

fn outcome_of<'a>(out: &'a CleanOutcome, path: &Path) -> &'a Outcome {
    let k = path_key(path);
    &out.items.iter().find(|i| i.key == k).unwrap_or_else(|| panic!("no outcome for {k}: {:#?}", out.items)).outcome
}

fn archived(agent: Agent, id: &str, cwd: &Path, file: Option<PathBuf>, blocked: bool) -> ArchivedSession {
    ArchivedSession {
        agent,
        id: id.to_string(),
        title: Some(format!("title {id}")),
        cwd: Some(cwd.to_path_buf()),
        updated_ms: Some(1),
        file,
        bytes: 1234,
        blocked: blocked.then(|| "bloccata".to_string()),
    }
}

fn write_sized(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

/// `proj` with `*.log` ignored by git.
fn project_ignoring_logs(base: &Path) -> PathBuf {
    let root = repo(&base.join("proj"));
    fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-q", "-m", "ignore logs"]);
    root
}

/// Every reason and error of the outcome is plain English: nothing the engine says in Italian.
fn assert_english(out: &CleanOutcome) {
    for OutcomeItem { key, outcome, .. } in &out.items {
        let text = match outcome {
            Outcome::Removed { .. } => continue,
            Outcome::Skipped { reason } => reason,
            Outcome::Failed { error } => error,
        };
        for italian in ["non ", "bloccat", "Annullat", "gi\u{e0}", "cartella", "sessione", "percorso"] {
            assert!(!text.contains(italian), "{key}: '{text}' has '{italian}'");
        }
    }
}

// ───────────────────────────────── select ─────────────────────────────────

#[test]
fn no_category_and_no_include_chooses_nothing() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    rust_build(&root, 2 * MB);
    let scan = scan_of(&root);
    assert!(!scan.items.is_empty());

    let chosen = select(&scan, &req(&[], &[])).unwrap();
    assert_eq!(chosen, Chosen::default());
}

#[test]
fn builds_flag_takes_recommended_builds_and_leaves_caution_ones() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let nm = node_modules(&root, 2 * MB);
    git(&root, &["add", "node_modules", "package.json"]);
    git(&root, &["commit", "-q", "-m", "track node_modules"]);
    let scan = scan_of(&root);
    let nm_item = scan.items.iter().find(|i| i.key == path_key(&nm)).unwrap();
    assert_eq!(nm_item.level, Level::Caution);

    let chosen = select(&scan, &req(&[Category::Builds], &[])).unwrap();
    assert_eq!(chosen.keys, vec![path_key(&target)]);
    assert!(chosen.bytes >= 2 * MB64 && chosen.bytes < 4 * MB64, "{}", chosen.bytes);
    assert!(chosen.permanent_anyway.is_empty());
}

#[test]
fn copies_or_named_as_a_category_is_refused() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    rust_build(&root, 2 * MB);
    let scan = scan_of(&root);

    assert_eq!(select(&scan, &req(&[Category::Copies], &[])), Err(SelectError::CategoryNeedsInclude(Category::Copies)));
    assert_eq!(
        select(&scan, &req(&[Category::Builds, Category::Named], &[])),
        Err(SelectError::CategoryNeedsInclude(Category::Named))
    );
}

#[test]
fn include_takes_a_named_folder_and_a_caution_build() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let nm = node_modules(&root, 2 * MB);
    git(&root, &["add", "node_modules", "package.json"]);
    git(&root, &["commit", "-q", "-m", "track node_modules"]);
    let named = base.join("proj-scratch");
    fill(&named, 1, MB);
    let scan = scan_of(&root);
    assert_eq!(scan.items.iter().filter(|i| i.category == Category::Named).count(), 1, "{:#?}", scan.items);

    let chosen = select(&scan, &asked(&[&named, &nm], &[], RemoveMode::Recycle)).unwrap();
    let mut keys = chosen.keys.clone();
    keys.sort();
    let mut want = vec![path_key(&named), path_key(&nm)];
    want.sort();
    assert_eq!(keys, want);
    assert!(chosen.bytes >= 3 * MB64, "{}", chosen.bytes);
    // Without git the named folder is danger: it waits for its confirmation.
    let pending: Vec<&str> = chosen.pending.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(pending, [path_key(&named).as_str()]);
}

#[test]
fn include_of_a_path_not_in_the_analysis_is_an_error_and_nothing_is_chosen() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let elsewhere = base.join("other-folder");
    fill(&elsewhere, 1, 1024);
    let scan = scan_of(&root);

    // A folder inside a listed one is not listed either, nor is the project itself.
    for bad in [elsewhere.clone(), target.join("debug"), root.clone()] {
        let err = select(&scan, &req(&[Category::Builds], &[&target, &bad])).unwrap_err();
        assert_eq!(err, SelectError::NotInAnalysis(bad.clone()));
    }
}

#[test]
fn include_of_a_dirty_worktree_is_refused_as_blocked() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    fs::write(wt.join("new.txt"), "work in progress").unwrap();
    let scan = scan_of(&root);

    match select(&scan, &req(&[], &[&wt])).unwrap_err() {
        SelectError::Blocked { path, why } => {
            assert_eq!(path, wt);
            assert!(why.contains(&Block::DirtyGit), "{why:?}");
        }
        e => panic!("{e:?}"),
    }
    // The category flag simply leaves it out.
    assert!(select(&scan, &req(&[Category::Worktrees], &[])).unwrap().keys.is_empty());
}

#[test]
fn the_worktrees_flag_takes_a_recommended_worktree_but_not_a_caution_one() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let kept = worktree(&root, &base.join("proj-kept"), "kept");
    let own = worktree(&root, &base.join("proj-own"), "own");
    commit(&own, "only-here.txt");
    let scan = scan_of(&root);
    let key = |p: &Path| path_key(p);

    let chosen = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();
    assert!(chosen.keys.contains(&key(&kept)), "the recommended worktree goes with the flag: {:?}", chosen.keys);
    assert!(!chosen.keys.contains(&key(&own)), "the caution one does not: {:?}", chosen.keys);
    let chosen = select(&scan, &req(&[], &[&own])).unwrap();
    assert!(chosen.keys.contains(&key(&own)), "--include takes it");
}

#[test]
fn sessions_flag_takes_unblocked_sessions_only() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let sessions = [
        archived(Agent::Claude, "s1", &root, None, false),
        archived(Agent::Claude, "s2", &root, None, true),
        archived(Agent::Codex, "s3", &root, None, false),
    ];
    let e = Env {
        sessions: &[],
        archived: &sessions,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
        history: crate::history::HistoryInput::none(),
    };
    let scan = scan_in(&root, &e);

    let mut chosen = select(&scan, &req(&[Category::Sessions], &[])).unwrap();
    chosen.keys.sort();
    assert_eq!(chosen.keys, vec!["session:claude:s1".to_string(), "session:codex:s3".to_string()]);
    assert_eq!(chosen.bytes, 2468);
    // A session has no path: it cannot be named with --include.
    assert!(matches!(
        select(&scan, &req(&[], &[Path::new("session:claude:s1")])),
        Err(SelectError::NotInAnalysis(_))
    ));
}

// ───────────────────────────────── clean ─────────────────────────────────

#[test]
fn clean_permanent_removes_the_build_folder_and_reports_its_bytes() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Builds], &[])).unwrap();

    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(!target.exists());
    match outcome_of(&out, &target) {
        Outcome::Removed { bytes_freed } => assert!(*bytes_freed >= 2 * MB64, "{bytes_freed}"),
        o => panic!("{o:?}"),
    }
    assert_eq!((out.schema, out.mode, out.dry_run), (SCHEMA, RemoveMode::Permanent, false));
    assert!(out.bytes_freed >= 2 * MB64);
    assert_eq!(out.recycled_bytes, 0);
    assert!(!out.partial());
    assert_eq!(out.items[0].path.as_deref(), Some(target.as_path()));
    assert_eq!(out.items[0].category, Category::Builds);
    // The project itself is untouched.
    assert!(root.join("Cargo.toml").exists() && root.join("README.md").exists());
}

#[test]
fn clean_removes_a_clean_worktree_with_git_and_the_main_repo_no_longer_lists_it() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();
    assert_eq!(chosen.keys, vec![path_key(&wt)]);
    // Removed with git whatever the mode: the bin is not involved.
    assert_eq!(chosen.permanent_anyway, vec![path_key(&wt)]);

    let out = run(&scan, &chosen, RemoveMode::Recycle, false);
    assert!(matches!(outcome_of(&out, &wt), Outcome::Removed { .. }), "{:#?}", out.items);
    assert!(!wt.exists());
    assert_eq!(out.recycled_bytes, 0, "a worktree never goes to the bin");
    assert!(!out.partial());
    assert!(resolve_project(&root).unwrap().worktrees.is_empty());
    assert!(!git(&root, &["worktree", "list", "--porcelain"]).contains("proj-wt"));
    // Never forced, and the branch is left alone.
    assert!(git(&root, &["branch", "--list", "feature"]).contains("feature"));
}

#[test]
fn worktree_that_became_dirty_after_the_analysis_is_skipped_and_outcome_is_partial() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 1);

    fs::write(wt.join("precious.txt"), "unsaved work").unwrap();
    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    match outcome_of(&out, &wt) {
        Outcome::Skipped { reason } => assert!(reason.contains("uncommitted changes"), "{reason}"),
        o => panic!("{o:?}"),
    }
    assert!(out.partial());
    assert_eq!(out.bytes_freed, 0);
    assert_eq!(fs::read_to_string(wt.join("precious.txt")).unwrap(), "unsaved work");
    assert_english(&out);
}

#[test]
fn worktree_that_gained_a_private_commit_after_the_analysis_is_skipped() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();

    commit(&wt, "only-here.txt");
    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    match outcome_of(&out, &wt) {
        // Its commit is only on its branch now: no longer blocked, but caution, so the flag that chose it no longer does.
        Outcome::Skipped { reason } => assert!(reason.starts_with("now needs a manual review"), "{reason}"),
        o => panic!("{o:?}"),
    }
    assert!(wt.join("only-here.txt").exists());
    assert_english(&out);
}

#[test]
fn folder_replaced_by_a_junction_after_the_analysis_is_left_alone_and_the_target_survives() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Builds], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 1);

    let precious = base.join("precious");
    write_sized(&precious.join("keep.txt"), 100);
    fs::remove_dir_all(&target).unwrap();
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&target)
        .arg(&precious)
        .output()
        .is_ok_and(|o| o.status.success());
    if !made {
        eprintln!("skipped: cannot create a junction here");
        return;
    }

    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    // End to end the fresh analysis no longer lists the junction as a build folder, so it is
    // skipped before `check_dir`; the link refusal itself is tested on `check_dir` below.
    assert!(matches!(outcome_of(&out, &target), Outcome::Skipped { .. }), "{:#?}", out.items);
    assert!(out.partial());
    assert_eq!(out.bytes_freed, 0);
    assert!(precious.join("keep.txt").exists(), "the junction was followed");
    assert!(target.exists(), "even the link must be left as it is");
    let _ = fs::remove_dir(&target);
    assert_english(&out);
}

#[test]
fn log_file_is_removed_and_a_file_that_vanished_is_skipped() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let a = root.join("logs").join("a.log");
    let b = root.join("logs").join("b.log");
    write_sized(&a, 2 * MB);
    write_sized(&b, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Logs], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 2);
    assert_eq!(chosen.bytes, 4 * MB64);

    fs::remove_file(&b).unwrap();
    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert_eq!(outcome_of(&out, &a), &Outcome::Removed { bytes_freed: 2 * MB64 });
    match outcome_of(&out, &b) {
        Outcome::Skipped { reason } => assert_eq!(reason, "no longer there"),
        o => panic!("{o:?}"),
    }
    assert!(!a.exists());
    assert!(out.partial());
    assert_eq!(out.bytes_freed, 2 * MB64);
    assert!(root.join("logs").is_dir(), "only the file goes, not its folder");
    assert_english(&out);
}

#[test]
fn log_file_in_recycle_mode_goes_through_the_bin_and_counts_as_recycled() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let a = root.join("a.log");
    write_sized(&a, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Logs], &[])).unwrap();

    let bin = FakeBin::new(base.join("fake-bin"));
    let out = run_with(&scan, &chosen, RemoveMode::Recycle, false, &env(), &bin, &|_| Vec::new());
    assert_eq!(outcome_of(&out, &a), &Outcome::Removed { bytes_freed: 2 * MB64 });
    assert_eq!(bin.taken.lock().unwrap().as_slice(), [a.clone()]);
    assert!(!a.exists());
    assert_eq!((out.bytes_freed, out.recycled_bytes), (2 * MB64, 2 * MB64));
}

#[cfg(windows)]
#[test]
fn a_log_that_turned_into_a_link_is_skipped_and_its_target_survives() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let a = root.join("a.log");
    write_sized(&a, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Logs], &[])).unwrap();

    let precious = base.join("precious.txt");
    write_sized(&precious, 100);
    fs::remove_file(&a).unwrap();
    if std::os::windows::fs::symlink_file(&precious, &a).is_err() {
        eprintln!("skipped: cannot create a file symlink here");
        return;
    }
    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(matches!(outcome_of(&out, &a), Outcome::Skipped { .. }), "{:#?}", out.items);
    assert!(precious.exists());
    let _ = fs::remove_file(&a);
}

#[test]
fn dry_run_touches_nothing_and_reports_what_it_would_free() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let target = rust_build(&root, 2 * MB);
    let log = root.join("a.log");
    write_sized(&log, 2 * MB);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Builds, Category::Logs, Category::Worktrees], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 3, "{chosen:?}");

    for mode in [RemoveMode::Permanent, RemoveMode::Recycle] {
        let out = run(&scan, &chosen, mode, true);
        assert!(out.dry_run);
        assert!(target.exists() && log.exists() && wt.exists());
        assert!(git(&root, &["worktree", "list", "--porcelain"]).contains("proj-wt"));
        for p in [&target, &log, &wt] {
            assert!(
                matches!(outcome_of(&out, p), Outcome::Removed { bytes_freed } if *bytes_freed > 0),
                "{:#?}",
                out.items
            );
        }
        assert_eq!(outcome_of(&out, &log), &Outcome::Removed { bytes_freed: 2 * MB64 });
        assert!(!out.partial());
        assert!(out.bytes_freed >= 4 * MB64, "{}", out.bytes_freed);
    }
}

#[test]
fn dry_run_still_runs_the_checks() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();
    fs::write(wt.join("new.txt"), "x").unwrap();

    let out = run(&scan, &chosen, RemoveMode::Permanent, true);
    assert!(matches!(outcome_of(&out, &wt), Outcome::Skipped { .. }), "{:#?}", out.items);
    assert!(out.partial());
}

#[test]
fn a_named_folder_chosen_with_include_is_removed_and_nothing_else() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let named = named_clone(&base, &base.join("proj-scratch"));
    let bystander = base.join("proj2");
    fill(&bystander, 1, MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[], &[&named])).unwrap();

    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(matches!(outcome_of(&out, &named), Outcome::Removed { .. }), "{:#?}", out.items);
    assert!(!named.exists());
    assert!(root.exists() && bystander.exists(), "only the chosen folder goes");
}

// ───────────────────────── overlapping choices ─────────────────────────

#[test]
fn a_log_inside_a_selected_worktree_is_skipped_as_already_removed_and_not_counted_twice() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let log = wt.join("logs").join("a.log");
    write_sized(&log, 2 * MB);
    let scan = scan_of(&root);
    let only_wt = select(&scan, &req(&[Category::Worktrees], &[])).unwrap();
    let chosen = select(&scan, &req(&[Category::Worktrees, Category::Logs], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 2, "{chosen:?}");
    assert_eq!(chosen.bytes, only_wt.bytes, "the log is inside the worktree: counted once");

    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(matches!(outcome_of(&out, &wt), Outcome::Removed { .. }), "{:#?}", out.items);
    match outcome_of(&out, &log) {
        Outcome::Skipped { reason } => {
            assert!(reason.starts_with("already removed with "), "{reason}");
            assert!(reason.contains("proj-wt"), "{reason}");
        }
        o => panic!("{o:?}"),
    }
    assert!(!wt.exists());
    assert!(!out.items.iter().any(|i| matches!(i.outcome, Outcome::Failed { .. })));
    assert_english(&out);
}

#[test]
fn when_the_outer_item_is_blocked_the_inner_one_is_still_removed() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let log = wt.join("a.log");
    write_sized(&log, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees, Category::Logs], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 2);

    // The worktree turns dirty: it is skipped, so nothing removed it and the log still goes.
    fs::write(wt.join("new.txt"), "work").unwrap();
    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(matches!(outcome_of(&out, &wt), Outcome::Skipped { .. }));
    assert_eq!(outcome_of(&out, &log), &Outcome::Removed { bytes_freed: 2 * MB64 });
    assert!(wt.join("new.txt").exists() && !log.exists());
}

// ───────────────────────────── sessions ─────────────────────────────

#[test]
fn sessions_are_revalidated_then_deleted_and_one_that_got_locked_is_skipped() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let f1 = base.join("sessions").join("s1.jsonl");
    let f2 = base.join("sessions").join("s2.jsonl");
    write_sized(&f1, 100);
    write_sized(&f2, 100);
    let sessions = [
        archived(Agent::Claude, "s1", &root, Some(f1.clone()), false),
        archived(Agent::Claude, "s2", &root, Some(f2.clone()), false),
    ];
    let e = Env {
        sessions: &[],
        archived: &sessions,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
        history: crate::history::HistoryInput::none(),
    };
    let scan = scan_in(&root, &e);
    let chosen = select(&scan, &req(&[Category::Sessions], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 2);

    // The fresh listing (read right before deleting) says s2 is locked now.
    let fresh = |_: Agent| {
        vec![
            archived(Agent::Claude, "s1", &root, Some(f1.clone()), false),
            archived(Agent::Claude, "s2", &root, Some(f2.clone()), true),
        ]
    };
    let bin = FakeBin::new(base.join("fake-bin"));
    let out = run_with(&scan, &chosen, RemoveMode::Permanent, false, &e, &bin, &fresh);

    let by_key = |k: &str| out.items.iter().find(|i| i.key == k).unwrap();
    assert_eq!(by_key("session:claude:s1").outcome, Outcome::Removed { bytes_freed: 1234 });
    assert_eq!(by_key("session:claude:s1").path, None);
    assert_eq!(by_key("session:claude:s1").category, Category::Sessions);
    assert!(matches!(by_key("session:claude:s2").outcome, Outcome::Skipped { .. }), "{:#?}", out.items);
    assert!(!f1.exists() && f2.exists());
    assert!(out.partial());
    assert_eq!(out.bytes_freed, 1234);
    assert_english(&out);
}

#[test]
fn sessions_dry_run_deletes_nothing() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let f1 = base.join("sessions").join("s1.jsonl");
    write_sized(&f1, 100);
    let sessions = [archived(Agent::Claude, "s1", &root, Some(f1.clone()), false)];
    let e = Env {
        sessions: &[],
        archived: &sessions,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
        history: crate::history::HistoryInput::none(),
    };
    let scan = scan_in(&root, &e);
    let chosen = select(&scan, &req(&[Category::Sessions], &[])).unwrap();
    let fresh = |_: Agent| vec![archived(Agent::Claude, "s1", &root, Some(f1.clone()), false)];
    let bin = FakeBin::new(base.join("fake-bin"));

    let out = run_with(&scan, &chosen, RemoveMode::Permanent, true, &e, &bin, &fresh);
    assert_eq!(out.items[0].outcome, Outcome::Removed { bytes_freed: 1234 });
    assert!(f1.exists());
}

// ───────────────────────────── safety rules ─────────────────────────────

#[test]
fn a_project_folder_that_contains_or_is_inside_the_project_or_a_worktree_overlaps() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("elsewhere").join("wt"), "feature");
    let project = resolve_project(&root).unwrap();
    let k = |p: &Path| path_key(p);

    assert!(overlaps_project(&project, &k(&root)));
    assert!(overlaps_project(&project, &k(&wt)));
    assert!(overlaps_project(&project, &k(&root.join("src"))), "inside the project");
    assert!(overlaps_project(&project, &k(&wt.join("src"))), "inside a worktree");
    assert!(overlaps_project(&project, &k(&base)), "contains the project");
    assert!(overlaps_project(&project, &k(&base.join("elsewhere"))), "contains a worktree");
    assert!(!overlaps_project(&project, &k(&base.join("proj-scratch"))), "a sibling with the same prefix");
    assert!(!overlaps_project(&project, &k(&base.join("elsewhere2"))));
}

#[test]
fn recycle_shortfall_counts_only_what_goes_to_the_bin_once() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let target = rust_build(&root, 2 * MB);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let log = wt.join("a.log");
    write_sized(&log, 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Builds, Category::Logs, Category::Worktrees], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 3);
    let letter = target.to_string_lossy().chars().next().unwrap().to_ascii_uppercase();

    // The bin has room for one byte: the build folder goes there. The worktree is removed by
    // git, and the log inside it goes with it, so neither counts.
    let short = shortfall_with(&scan, &chosen, RemoveMode::Recycle, |_| Some(1));
    assert_eq!(short.len(), 1, "{short:?}");
    assert_eq!(short[0].0, letter);
    assert_eq!(short[0].2, 1);
    let build_bytes = scan.items.iter().find(|i| i.key == path_key(&target)).unwrap().bytes.unwrap();
    assert_eq!(short[0].1, build_bytes);

    assert!(shortfall_with(&scan, &chosen, RemoveMode::Permanent, |_| Some(1)).is_empty());
    assert!(shortfall_with(&scan, &chosen, RemoveMode::Recycle, |_| Some(u64::MAX)).is_empty());
    assert!(shortfall_with(&scan, &chosen, RemoveMode::Recycle, |_| None).is_empty());
    assert_eq!(recycle_shortfall(&scan, &Chosen::default(), RemoveMode::Recycle), Vec::new());
}

/// The fresh analysis may still list a folder that is a link by the time it goes (the window
/// between the two is short, not zero): `check_dir` is the last look before the plan.
#[test]
fn check_dir_refuses_a_junction_a_file_a_missing_path_and_a_project_folder_around_the_project() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let project = resolve_project(&root).unwrap();
    let dir_item = |path: &Path, kind: ArtifactKind| Item {
        key: path_key(path),
        category: Category::Builds,
        path: Some(path.to_path_buf()),
        bytes: None,
        level: Level::Recommended,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::NameOnly,
        target: Target::Dir { kind },
        provenance: None,
    };

    let real = base.join("proj").join("target");
    fill(&real, 1, 1024);
    assert!(check_dir(&dir_item(&real, ArtifactKind::RustTarget), 7, &project).is_ok());

    let precious = base.join("precious");
    write_sized(&precious.join("keep.txt"), 100);
    let link = base.join("proj").join("node_modules");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&precious)
        .output()
        .is_ok_and(|o| o.status.success());
    if made {
        let err = check_dir(&dir_item(&link, ArtifactKind::NodeModules), 7, &project).unwrap_err();
        assert_eq!(err, "now a link: not followed");
        let _ = fs::remove_dir(&link);
    } else {
        eprintln!("skipped: cannot create a junction here");
    }

    let file = base.join("proj").join("dist");
    write_sized(&file, 10);
    assert_eq!(check_dir(&dir_item(&file, ArtifactKind::Other), 7, &project).unwrap_err(), "no longer a folder");
    let gone = base.join("proj").join("build");
    assert_eq!(check_dir(&dir_item(&gone, ArtifactKind::Other), 7, &project).unwrap_err(), "no longer there");

    // A name-matched folder that is, holds or sits in the project (or a worktree) is refused,
    // whatever the guard thinks of its name.
    assert_eq!(
        check_dir(&dir_item(&root, ArtifactKind::ProjectFolder), 7, &project).unwrap_err(),
        "it overlaps the project or one of its worktrees"
    );
    assert!(check_dir(&dir_item(&base, ArtifactKind::ProjectFolder), 7, &project).is_err());
    // The plan item is built from the fresh path with the right action.
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let project = resolve_project(&root).unwrap();
    let mut w = dir_item(&wt, ArtifactKind::WorktreeDir);
    w.target = Target::Worktree { repo: PathBuf::from("Z:\not-the-repo") };
    let pi = check_dir(&w, 9, &project).unwrap();
    assert_eq!(pi.action, xiox_model::PlanAction::GitWorktreeRemove { repo: root.clone() });
    assert_eq!((pi.kind, pi.expected_bytes), (ArtifactKind::WorktreeDir, 9));
    // A worktree that holds the project's main folder is refused.
    let mut around = dir_item(&base, ArtifactKind::WorktreeDir);
    around.target = Target::Worktree { repo: root.clone() };
    assert_eq!(check_dir(&around, 0, &project).unwrap_err(), "it is, or contains, the project's main folder");
}

#[test]
fn operating_system_and_git_text_never_reach_the_outcome() {
    // Windows describes its errors in the user's language: only the code is kept.
    assert_eq!(
        strip_os_text("could not remove C-x-y: Accesso negato. (os error 5); the folder may be partially deleted"),
        "could not remove C-x-y: os error 5; the folder may be partially deleted"
    );
    assert_eq!(strip_os_text("Impossibile trovare il file. (os error 2)"), "os error 2");
    assert_eq!(
        strip_os_text("a: uno (os error 1); b: due (os error 22) fine"),
        "a: os error 1; b: os error 22 fine"
    );
    assert_eq!(strip_os_text("plain English text"), "plain English text");
    assert_eq!(strip_os_text("broken (os error x)"), "broken (os error x)");
    // git speaks the language of the machine too: its own words are dropped.
    assert_eq!(
        english_error("git worktree remove failed (not forced; local changes are never discarded): contiene file modificati", &[]),
        "git refused to remove the worktree (not forced; local changes are never discarded)"
    );
    assert_eq!(
        english_error("could not remove x: Accesso negato. (os error 5)", &["a.exe".into(), "b.exe".into()]),
        "could not remove x: os error 5 (in use by: a.exe, b.exe)"
    );
}

// ───────────────────────────── fix round 1 ─────────────────────────────

fn copy_of(base: &Path, root: &Path) -> PathBuf {
    let dest = base.join("other").join("copy");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    git(base, &["clone", "-q", &root.to_string_lossy(), &dest.to_string_lossy()]);
    canon(&dest)
}

/// The fixture env with the copies searched in `base/other`.
fn copies_env(base: &Path) -> Env<'static> {
    Env { search_roots: Some(vec![base.join("other")]), ..env() }
}

/// `scan_in` for a project whose copies are searched in `base/other`.
fn scan_with_copies(root: &Path, base: &Path) -> ProjectScan {
    scan_in(root, &copies_env(base))
}

fn rerun(scan: &ProjectScan, chosen: &Chosen, base: &Path) -> CleanOutcome {
    let bin = FakeBin::new(PathBuf::from("unused"));
    run_with(scan, chosen, RemoveMode::Permanent, false, &copies_env(base), &bin, &|_| Vec::new())
}

#[test]
fn a_copy_that_gained_work_after_the_analysis_is_skipped_and_stays() {
    for how in ["dirty", "commit"] {
        let (_t, base) = tmp();
        let root = repo(&base.join("proj"));
        let copy = copy_of(&base, &root);
        let scan = scan_with_copies(&root, &base);
        let item = scan.items.iter().find(|i| i.key == path_key(&copy)).expect("copy listed");
        assert_eq!(item.category, Category::Copies);
        assert!(item.blocked.is_empty(), "{:?}", item.blocked);
        let chosen = select(&scan, &req(&[], &[&copy])).unwrap();

        if how == "dirty" {
            fs::write(copy.join("wip.txt"), "unsaved").unwrap();
        } else {
            commit(&copy, "private.txt");
        }
        let out = rerun(&scan, &chosen, &base);
        // The fresh analysis blocks it (UnsavedWork) before the own-work check is even needed.
        assert!(matches!(outcome_of(&out, &copy), Outcome::Skipped { .. }), "{how}: {:#?}", out.items);
        assert!(out.partial());
        assert!(copy.join("README.md").exists(), "{how}: the copy was touched");
        assert_english(&out);
    }
}

#[test]
fn recheck_skips_a_copy_whose_fresh_listing_says_it_has_own_work_even_if_unblocked() {
    let mk = |own: Option<bool>, blocked: Vec<Block>| Item {
        key: "k".into(),
        category: Category::Copies,
        path: Some(PathBuf::from("X-a-b")),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked,
        why: Why::SameRepository { shared_root_commit: true, same_origin: false, has_own_work: own },
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance: None,
    };
    let old = mk(Some(false), Vec::new());
    assert!(crate::clean::recheck(&old, Some(&mk(Some(false), Vec::new())), None, RemoveMode::Permanent).is_ok());
    // No answer from git now is not "no own work": see the m1 test further down.
    assert!(crate::clean::recheck(&old, Some(&mk(None, Vec::new())), None, RemoveMode::Permanent).is_err());
    assert_eq!(
        crate::clean::recheck(&old, Some(&mk(Some(true), Vec::new())), None, RemoveMode::Permanent).unwrap_err(),
        "now has commits or changes the project does not have"
    );
}

#[test]
fn include_of_a_copy_with_its_own_work_is_refused_as_blocked_and_it_survives() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let copy = copy_of(&base, &root);
    commit(&copy, "private.txt");
    let scan = scan_with_copies(&root, &base);

    match select(&scan, &req(&[], &[&copy])).unwrap_err() {
        SelectError::Blocked { path, why } => {
            assert_eq!(path, copy);
            assert_eq!(why, vec![Block::UnsavedWork]);
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(
        SelectError::Blocked { path: copy.clone(), why: vec![Block::UnsavedWork] }.to_string(),
        format!("cannot be removed (has commits or changes the project does not have): {}", copy.display())
    );
    assert!(copy.join("private.txt").exists());
}

#[test]
fn an_item_skipped_because_its_outer_item_went_is_not_a_partial_result() {
    let (_t, base) = tmp();
    let root = project_ignoring_logs(&base);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    write_sized(&wt.join("logs").join("a.log"), 2 * MB);
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Worktrees, Category::Logs], &[])).unwrap();

    let out = run(&scan, &chosen, RemoveMode::Permanent, false);
    assert!(out.items.iter().any(|i| matches!(&i.outcome, Outcome::Skipped { reason } if reason.starts_with("already removed with "))));
    assert!(!out.partial(), "{:#?}", out.items);

    // A real skip or failure still is.
    let mut other = out.clone();
    other.items.push(log_outcome(Outcome::Skipped { reason: "no longer there".into() }));
    assert!(other.partial());
    let mut failed = out;
    failed.items.push(log_outcome(Outcome::Failed { error: "x".into() }));
    assert!(failed.partial());
}

#[test]
fn clean_with_nothing_chosen_returns_an_empty_outcome_at_once() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let scan = scan_of(&root);
    let out = clean(&scan, &Chosen::default(), RemoveMode::Recycle, false);
    assert!(out.items.is_empty() && !out.partial());
    assert_eq!((out.schema, out.mode, out.bytes_freed, out.recycled_bytes), (SCHEMA, RemoveMode::Recycle, 0, 0));
}

#[test]
fn a_top_level_project_folder_is_refused_with_an_english_reason() {
    // `guard::validate` refuses drive-root children and that is not relaxed for project
    // folders. `clean` cannot reach it end to end (a name-matched folder at a drive root would
    // have to be created there), so the check is made on `check_dir` with a real top-level
    // folder, read only: nothing is deleted.
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let project = resolve_project(&root).unwrap();
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let top = PathBuf::from(format!("{drive}/Windows"));
    if !top.is_dir() {
        eprintln!("skipped: no top-level folder to test with");
        return;
    }
    let item = Item {
        key: path_key(&top),
        category: Category::Named,
        path: Some(top.clone()),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::NameOnly,
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance: None,
    };
    let reason = check_dir(&item, 0, &project).unwrap_err();
    assert!(reason.starts_with("path too shallow"), "{reason}");
    assert!(top.is_dir());
}

#[test]
fn clean_without_sessions_removes_a_chosen_build_and_reads_no_session() {
    let (_t, base) = tmp();
    let p = base.join("work").join("proj");
    fs::create_dir_all(&p).unwrap();
    let target = rust_build(&p, MB);
    let roots = vec![base.join("work")];
    let scan = analyze_project_without_sessions(&p, &Options { drives: false, measure: true, admin: false, history: false }, &roots).unwrap();
    assert!(scan.items.iter().all(|i| i.category != Category::Sessions));
    let chosen = select(&scan, &req(&[Category::Builds], &[])).unwrap();
    assert_eq!(chosen.keys.len(), 1);

    // A dry run touches nothing.
    let dry = clean_without_sessions(&scan, &chosen, RemoveMode::Permanent, true, &roots);
    assert!(!dry.partial() && dry.dry_run, "{dry:?}");
    assert!(target.exists());

    let out = clean_without_sessions(&scan, &chosen, RemoveMode::Permanent, false, &roots);
    assert!(!out.partial(), "{out:?}");
    assert!(matches!(out.items[0].outcome, Outcome::Removed { .. }), "{out:?}");
    assert!(!target.exists());
    assert!(p.join("Cargo.toml").exists());

    // Nothing chosen: nothing done, no analysis.
    let none = clean_without_sessions(&scan, &Chosen::default(), RemoveMode::Permanent, false, &roots);
    assert!(none.items.is_empty() && !none.partial());
}

#[test]
fn not_removed_counts_skipped_and_failed_but_not_the_overlap_with_a_removed_folder() {
    let item = |k: &str, o: Outcome| OutcomeItem {
        key: k.to_string(),
        path: Some(PathBuf::from(k)),
        category: Category::Builds,
        method: Method::Permanent,
        outcome: o,
    };
    let out = CleanOutcome {
        schema: SCHEMA,
        mode: RemoveMode::Permanent,
        dry_run: false,
        items: vec![
            item("a", Outcome::Removed { bytes_freed: 1 }),
            item("a\\inner", Outcome::Skipped { reason: format!("{}a", crate::clean::ALREADY_REMOVED) }),
            item("b", Outcome::Skipped { reason: "no longer there".into() }),
            item("c", Outcome::Failed { error: "x".into() }),
        ],
        bytes_freed: 1,
        recycled_bytes: 0,
        elapsed_ms: 0,
    };
    assert_eq!(out.not_removed(), 2);
    assert!(out.partial());
    let only_overlap = CleanOutcome { items: out.items[..2].to_vec(), ..out };
    assert_eq!(only_overlap.not_removed(), 0);
    assert!(!only_overlap.partial());
}

// ─────────────────────── final review: blocked items inside a folder ───────────────────────

/// `base/other/proj-old` (a folder named after the project) holding `copy`, a clone of the project
/// with a private commit: the copy is blocked, the folder around it is only a name match.
fn named_folder_holding_a_blocked_copy(base: &Path, root: &Path) -> (PathBuf, PathBuf) {
    let named = base.join("other").join("proj-old");
    let copy = named.join("copy");
    fs::create_dir_all(&named).unwrap();
    git(base, &["clone", "-q", &root.to_string_lossy(), &copy.to_string_lossy()]);
    let copy = canon(&copy);
    commit(&copy, "private.txt");
    (canon(&named), copy)
}

#[test]
fn c1_an_included_named_folder_never_takes_a_blocked_copy_inside_it() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let (named, copy) = named_folder_holding_a_blocked_copy(&base, &root);
    let scan = scan_with_copies(&root, &base);
    let copy_item = scan.items.iter().find(|i| i.key == path_key(&copy)).expect("the copy is listed");
    assert_eq!(copy_item.blocked, vec![Block::UnsavedWork]);
    let named_item = scan.items.iter().find(|i| i.key == path_key(&named)).expect("the folder is listed");
    assert_eq!(named_item.category, Category::Named);

    // Either the request is refused, or the removal skips the folder: the copy must survive.
    match select(&scan, &req(&[], &[&named])) {
        Err(_) => {}
        Ok(chosen) => {
            let out = rerun(&scan, &chosen, &base);
            assert!(matches!(outcome_of(&out, &named), Outcome::Skipped { .. }), "{:#?}", out.items);
        }
    }
    assert!(copy.join("private.txt").exists(), "the copy with its private commit was removed");
}

#[test]
fn c1_select_refuses_a_folder_that_holds_a_blocked_item_and_says_which() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let (named, copy) = named_folder_holding_a_blocked_copy(&base, &root);
    let scan = scan_with_copies(&root, &base);
    match select(&scan, &req(&[], &[&named])).unwrap_err() {
        SelectError::ContainsBlocked { path, inner, why } => {
            assert_eq!(path, named);
            assert_eq!(inner, copy);
            assert_eq!(why, vec![Block::UnsavedWork]);
        }
        e => panic!("{e:?}"),
    }
    let text = select(&scan, &req(&[], &[&named])).unwrap_err().to_string();
    assert_eq!(
        text,
        format!(
            "cannot be removed: it holds {} (has commits or changes the project does not have): {}",
            copy.display(),
            named.display()
        )
    );
    assert!(copy.join("private.txt").exists());
}

#[test]
fn c1_without_the_wide_search_the_copy_is_not_listed_and_the_removal_still_refuses() {
    // `--no-drives`: only the project's parent is searched, one level deep. The folder is listed,
    // the copy inside it is not: nothing in the analysis says it is there.
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let named = base.join("proj-old");
    let copy = named.join("copy");
    fs::create_dir_all(&named).unwrap();
    git(&base, &["clone", "-q", &root.to_string_lossy(), &copy.to_string_lossy()]);
    let copy = canon(&copy);
    commit(&copy, "private.txt");
    let named = canon(&named);

    let opts = Options { drives: false, ..Options::default() };
    let scan = analyze_with(&root, &opts, &env()).unwrap();
    assert_eq!(scan.search, SearchMode::ParentOnly);
    assert!(scan.items.iter().all(|i| i.key != path_key(&copy)), "the copy must not be listed at depth 2");
    let chosen = select(&scan, &asked(&[&named], &[&named], RemoveMode::Recycle))
        .expect("nothing in the scan says it is blocked");

    // Without git the folder is danger, so Recycle Bin only: a dry run runs the same checks.
    let out = run(&scan, &chosen, RemoveMode::Recycle, true);
    match outcome_of(&out, &named) {
        Outcome::Skipped { reason } => {
            assert!(reason.contains("git repository"), "{reason}");
            assert!(reason.contains(&copy.display().to_string()), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(out.partial());
    assert!(copy.join("private.txt").exists());
    assert_english(&out);
}

#[test]
fn c1_a_hand_made_choice_of_the_folder_is_skipped_too_when_the_copy_is_listed_and_blocked() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let (named, copy) = named_folder_holding_a_blocked_copy(&base, &root);
    let scan = scan_with_copies(&root, &base);
    // Without git the folder is danger: its plan is confirmed, so the blocked copy is what stops it.
    let risks = scan.items.iter().find(|i| i.key == path_key(&named)).unwrap().risks.clone();
    let plan = Confirmation { key: path_key(&named), risks, method: Method::Permanent };
    let chosen = Chosen { keys: vec![path_key(&named)], confirmed: vec![plan], ..Chosen::default() };
    let out = rerun(&scan, &chosen, &base);
    match outcome_of(&out, &named) {
        Outcome::Skipped { reason } => assert!(reason.starts_with("cannot be removed: it holds "), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(copy.join("private.txt").exists());
    assert_english(&out);
}

#[test]
fn a_named_folder_holding_a_clean_listed_copy_goes_with_it() {
    // Not over-cautious: the copy inside is listed, unblocked and has no work of its own.
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let named = base.join("other").join("proj-old");
    let copy = named.join("copy");
    fs::create_dir_all(&named).unwrap();
    git(&base, &["clone", "-q", &root.to_string_lossy(), &copy.to_string_lossy()]);
    let (named, copy) = (canon(&named), canon(&copy));
    let scan = scan_with_copies(&root, &base);
    assert!(scan.items.iter().find(|i| i.key == path_key(&copy)).is_some_and(|i| i.blocked.is_empty()));
    let chosen = select(&scan, &asked(&[&named], &[&named], RemoveMode::Recycle)).unwrap();
    // Without git the folder is danger, so Recycle Bin only: a dry run says it would go.
    let bin = FakeBin::new(PathBuf::from("unused"));
    let out = run_with(&scan, &chosen, RemoveMode::Recycle, true, &copies_env(&base), &bin, &|_| Vec::new());
    assert!(matches!(outcome_of(&out, &named), Outcome::Removed { .. }), "{:#?}", out.items);
}

#[test]
fn a_named_folder_with_an_unlisted_git_repository_inside_is_refused_whatever_that_repository_is() {
    // An independent repository the search never lists (it is not a copy and its name does not
    // match): the folder around it is not removed, there is nothing to prove it is safe.
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let named = base.join("proj-old");
    fs::create_dir_all(named.join("vendor")).unwrap();
    let inner = repo(&named.join("vendor").join("lib"));
    let opts = Options { drives: false, ..Options::default() };
    let scan = analyze_with(&root, &opts, &env()).unwrap();
    let named = canon(&named);
    let chosen = select(&scan, &asked(&[&named], &[&named], RemoveMode::Recycle)).unwrap();
    // Without git the folder is danger, so Recycle Bin only: a dry run runs the same checks.
    let out = run(&scan, &chosen, RemoveMode::Recycle, true);
    assert!(matches!(outcome_of(&out, &named), Outcome::Skipped { .. }), "{:#?}", out.items);
    assert!(inner.join("README.md").exists());
}

fn contents_item(path: &Path, kind: ArtifactKind, category: Category) -> Item {
    Item {
        key: path_key(path),
        category,
        path: Some(path.to_path_buf()),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::NameOnly,
        target: Target::Dir { kind },
        provenance: None,
    }
}

#[test]
fn i7_a_catalog_folder_that_holds_a_worktree_or_the_project_is_refused() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    // A worktree under a folder the catalog calls a build folder.
    let out_dir = root.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let wt = worktree(&root, &out_dir.join("wt"), "feature");
    let project = resolve_project(&root).unwrap();
    assert_eq!(project.worktrees, vec![wt.clone()]);

    let folder = contents_item(&out_dir, ArtifactKind::Other, Category::Builds);
    let err = crate::clean::check_contents(&folder, &project, &[]).unwrap_err();
    assert_eq!(err, "it holds the project or one of its worktrees");

    // A plain build folder of the project is fine.
    let target = root.join("target");
    fill(&target, 1, 100);
    let ok = contents_item(&target, ArtifactKind::RustTarget, Category::Builds);
    assert!(crate::clean::check_contents(&ok, &project, &[]).is_ok());

    // A build folder that holds an item the analysis lists as a worktree, copy or named folder.
    let holder = base.join("proj").join("build");
    let inner = holder.join("proj-extra");
    fs::create_dir_all(&inner).unwrap();
    let holder_item = contents_item(&holder, ArtifactKind::Other, Category::Builds);
    let listed = vec![contents_item(&inner, ArtifactKind::ProjectFolder, Category::Named)];
    let err = crate::clean::check_contents(&holder_item, &project, &listed).unwrap_err();
    assert!(err.starts_with("it holds "), "{err}");
    assert!(err.contains("listed separately"), "{err}");
}

#[test]
fn check_contents_lets_a_named_folder_hold_only_unblocked_listed_items() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let project = resolve_project(&root).unwrap();
    let named = base.join("proj-old");
    let inner = named.join("copy");
    git(&base, &["clone", "-q", &root.to_string_lossy(), &inner.to_string_lossy()]);
    let (named, inner) = (canon(&named), canon(&inner));
    let named_item = contents_item(&named, ArtifactKind::ProjectFolder, Category::Named);

    // Listed and unblocked: allowed.
    let mut listed_inner = contents_item(&inner, ArtifactKind::ProjectFolder, Category::Copies);
    assert!(crate::clean::check_contents(&named_item, &project, std::slice::from_ref(&listed_inner)).is_ok());
    // Listed and blocked: refused, and the reason names it.
    listed_inner.blocked = vec![Block::UnsavedWork];
    let err = crate::clean::check_contents(&named_item, &project, std::slice::from_ref(&listed_inner)).unwrap_err();
    assert!(err.contains("is blocked") && err.contains(&inner.display().to_string()), "{err}");
    // Not listed at all: refused.
    let err = crate::clean::check_contents(&named_item, &project, &[]).unwrap_err();
    assert!(err.contains("git repository") && err.contains(&inner.display().to_string()), "{err}");
    // A copy is its own repository: the check is about what is below it.
    let copy_item = contents_item(&inner, ArtifactKind::ProjectFolder, Category::Copies);
    assert!(crate::clean::check_contents(&copy_item, &project, &[]).is_ok());
}

#[test]
fn m1_a_copy_whose_fresh_own_work_check_gives_no_answer_is_skipped() {
    let mk = |own: Option<bool>| Item {
        key: "k".into(),
        category: Category::Copies,
        path: Some(PathBuf::from("X-a-b")),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::SameRepository { shared_root_commit: true, same_origin: false, has_own_work: own },
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance: None,
    };
    let old = mk(Some(false));
    assert!(crate::clean::recheck(&old, Some(&mk(Some(false))), None, RemoveMode::Permanent).is_ok());
    assert_eq!(
        crate::clean::recheck(&old, Some(&mk(None)), None, RemoveMode::Permanent).unwrap_err(),
        "git could not say whether it has commits or changes the project does not have"
    );
    assert_eq!(
        crate::clean::recheck(&old, Some(&mk(Some(true))), None, RemoveMode::Permanent).unwrap_err(),
        "now has commits or changes the project does not have"
    );
}

#[test]
fn m4_a_failed_session_read_skips_worktrees_copies_and_named_folders_but_not_builds() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let target = rust_build(&root, 2 * MB);
    let named = named_clone(&base, &base.join("proj-scratch"));
    let scan = scan_of(&root);
    let chosen = select(&scan, &req(&[Category::Builds, Category::Worktrees], &[&named])).unwrap();
    assert!(chosen.keys.contains(&path_key(&wt)) && chosen.keys.contains(&path_key(&target)));

    let blind = Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: SessionHealth { live_failed: true, archived_failed: false },
        history: crate::history::HistoryInput::none(),
    };
    let bin = FakeBin::new(PathBuf::from("unused"));
    let out = run_with(&scan, &chosen, RemoveMode::Permanent, false, &blind, &bin, &|_| Vec::new());
    for p in [&wt, &named] {
        match outcome_of(&out, p) {
            Outcome::Skipped { reason } => {
                assert_eq!(
                    reason,
                    "the sessions of this PC could not be read, so a live agent session cannot be ruled out"
                );
            }
            other => panic!("{p:?}: {other:?}"),
        }
        assert!(p.exists());
    }
    assert!(matches!(outcome_of(&out, &target), Outcome::Removed { .. }), "{:#?}", out.items);
    assert_english(&out);
}

#[test]
fn i6_localized_trash_and_os_text_never_reaches_the_outcome() {
    let trash = "could not move D:\\p\\target to the Recycle Bin: Error during a `trash` operation: \
Os { code: -2147024864, description: \"Il file \\\"x.dll\\\" (in uso) \u{e8} utilizzato da un altro processo.\" }\
; blocked by D:\\p\\target\\x.dll: Il processo non pu\u{f2} accedere al file perch\u{e9} \u{e8} utilizzato da un altro processo. (os error 32)";
    let text = english_error(trash, &["devenv.exe".to_string()]);
    assert_eq!(
        text,
        "could not move D:\\p\\target to the Recycle Bin: Error during a `trash` operation: os error -2147024864\
; blocked by D:\\p\\target\\x.dll: os error 32 (in use by: devenv.exe)"
    );
    // The same cut on its own.
    assert_eq!(strip_os_text("a: Os { code: 5, description: \"Accesso negato.\" } b"), "a: os error 5 b");
    // A description with an escaped quote and a closing brace inside.
    assert_eq!(strip_os_text("Os { code: 5, description: \"x \\\" } y\" } end"), "os error 5 end");
    // Text that only looks like it is left alone.
    assert_eq!(strip_os_text("Os { code: x }"), "Os { code: x }");
    assert_eq!(strip_os_text("no code here"), "no code here");
}

fn listed(category: Category, path: &Path, why: Why) -> Item {
    Item {
        key: path_key(path),
        category,
        path: Some(path.to_path_buf()),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why,
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance: None,
    }
}

fn scratch_scan(base: &Path, items: Vec<Item>) -> ProjectScan {
    ProjectScan {
        schema: SCHEMA,
        project: Project { root: base.join("proj"), name: "proj".into(), is_repo: false, origin: None, worktrees: vec![] },
        items,
        branches: vec![],
        notes: vec![],
        history: None,
        search: SearchMode::ParentOnly,
        scanned_at_ms: 0,
        elapsed_ms: 0,
    }
}

fn session_folder() -> Why {
    Why::SessionFolder {
        strength: Strength::Created,
        from_script: false,
        activity: Activity::Unknown,
    }
}

// ─────────────────────────── admission and confirmations ───────────────────────────

fn at_risk(item: Item, risks: &[Risk]) -> Item {
    Item { level: Level::Danger, risks: risks.to_vec(), ..item }
}

fn worktree_item(path: &Path, base: &Path) -> Item {
    let item = listed(Category::Worktrees, path, Why::NameOnly);
    Item { target: Target::Worktree { repo: base.join("proj") }, ..item }
}

fn asked(include: &[&Path], confirm: &[&Path], mode: RemoveMode) -> CleanRequest {
    CleanRequest {
        categories: Vec::new(),
        include: include.iter().map(|p| p.to_path_buf()).collect(),
        confirm_danger: confirm.iter().map(|p| p.to_path_buf()).collect(),
        mode,
    }
}

fn plan(path: &Path, risks: &[Risk], method: Method) -> Confirmation {
    Confirmation { key: path_key(path), risks: risks.to_vec(), method }
}

#[test]
fn scratch_is_selectable_with_confirmation() {
    let (_t, base) = tmp();
    let bench = base.join("agent-runs").join("bench-1");
    let scan = scratch_scan(&base, vec![at_risk(listed(Category::Scratch, &bench, session_folder()), &[Risk::NoGit])]);
    let expected = plan(&bench, &[Risk::NoGit], Method::RecycleBin);

    let chosen = select(&scan, &asked(&[&bench], &[], RemoveMode::Recycle)).unwrap();
    assert_eq!((chosen.keys, chosen.confirmed, chosen.pending), (vec![path_key(&bench)], vec![], vec![expected.clone()]));

    let chosen = select(&scan, &asked(&[&bench], &[&bench], RemoveMode::Recycle)).unwrap();
    assert_eq!((chosen.confirmed, chosen.pending), (vec![expected], vec![]));
}

#[test]
fn container_needs_the_confirmations_of_its_descendants() {
    let (_t, base) = tmp();
    let runs = base.join("agent-runs");
    let (a, b) = (runs.join("a"), runs.join("b"));
    let scratch = |p: &Path| at_risk(listed(Category::Scratch, p, session_folder()), &[Risk::NoGit]);
    let scan = scratch_scan(&base, vec![listed(Category::Named, &runs, Why::NameOnly), scratch(&a), scratch(&b)]);
    let planned = |p: &Path| plan(p, &[Risk::NoGit], Method::RecycleBin);
    let recycle = |scan: &ProjectScan, confirm: &[&Path]| select(scan, &asked(&[&runs], confirm, RemoveMode::Recycle));

    let chosen = recycle(&scan, &[]).unwrap();
    assert_eq!((chosen.keys, chosen.confirmed, chosen.pending), (vec![path_key(&runs)], vec![], vec![planned(&a), planned(&b)]));
    assert_eq!(recycle(&scan, &[&runs]).unwrap_err(), SelectError::ConfirmNotDanger(runs.clone()));
    let chosen = recycle(&scan, &[&a]).unwrap();
    assert_eq!((chosen.confirmed, chosen.pending), (vec![planned(&a)], vec![planned(&b)]));

    let mut blocked = scan.clone();
    blocked.items[2].blocked = vec![Block::OtherProjectWrites];
    assert_eq!(
        recycle(&blocked, &[]).unwrap_err(),
        SelectError::ContainsBlocked { path: runs.clone(), inner: b.clone(), why: vec![Block::OtherProjectWrites] }
    );
}

#[test]
fn confirm_must_name_an_included_danger_item() {
    let (_t, base) = tmp();
    let bench = base.join("agent-runs").join("bench-1");
    let target = base.join("proj").join("target");
    let unknown = base.join("nowhere");
    let scan = scratch_scan(
        &base,
        vec![
            at_risk(listed(Category::Scratch, &bench, session_folder()), &[Risk::NoGit]),
            Item { level: Level::Recommended, ..listed(Category::Builds, &target, Why::NameOnly) },
        ],
    );
    // (name, included, confirmed, error)
    let table: [(&str, &Path, &Path, SelectError); 3] = [
        ("not analysed", &bench, &unknown, SelectError::NotInAnalysis(unknown.clone())),
        ("danger but not included", &target, &bench, SelectError::ConfirmNotIncluded(bench.clone())),
        ("included but not danger", &target, &target, SelectError::ConfirmNotDanger(target.clone())),
    ];
    for (name, include, confirm, expected) in table {
        let err = select(&scan, &asked(&[include], &[confirm], RemoveMode::Recycle)).unwrap_err();
        assert_eq!(err, expected, "{name}");
    }
}

#[test]
fn category_never_takes_danger() {
    let (_t, base) = tmp();
    let wt = base.join("proj-wt");
    let target = base.join("proj").join("target");
    let build = Item { level: Level::Recommended, ..listed(Category::Builds, &target, Why::NameOnly) };
    let scan = scratch_scan(
        &base,
        vec![
            build.clone(),
            at_risk(listed(Category::Scratch, &target.join("run"), session_folder()), &[Risk::NoGit]),
            at_risk(worktree_item(&wt, &base), &[Risk::RecentActivity]),
        ],
    );
    let take = |scan: &ProjectScan, c: Category| {
        select(scan, &CleanRequest { categories: vec![c], ..asked(&[], &[], RemoveMode::Recycle) }).unwrap().keys
    };
    assert_eq!(take(&scan, Category::Worktrees), Vec::<String>::new());
    assert_eq!(take(&scan, Category::Builds), Vec::<String>::new());
    // The same build with nothing at risk inside is taken.
    assert_eq!(take(&scratch_scan(&base, vec![build]), Category::Builds), vec![path_key(&target)]);
}

#[test]
fn permanent_refuses_danger_folders_but_not_worktrees() {
    let (_t, base) = tmp();
    let runs = base.join("agent-runs");
    let bench = runs.join("bench-1");
    let wt = base.join("proj-wt");
    let scan = scratch_scan(
        &base,
        vec![
            listed(Category::Named, &runs, Why::NameOnly),
            at_risk(listed(Category::Scratch, &bench, session_folder()), &[Risk::NoGit]),
            at_risk(worktree_item(&wt, &base), &[Risk::RecentActivity]),
        ],
    );
    let permanent = |include: &Path| select(&scan, &asked(&[include], &[], RemoveMode::Permanent));

    assert_eq!(permanent(&bench).unwrap_err(), SelectError::DangerPermanent { path: bench.clone(), inner: None });
    assert_eq!(permanent(&runs).unwrap_err(), SelectError::DangerPermanent { path: runs.clone(), inner: Some(bench.clone()) });
    let chosen = permanent(&wt).unwrap();
    assert_eq!(chosen.pending, vec![plan(&wt, &[Risk::RecentActivity], Method::GitWorktreeRemove)]);
    assert_eq!(chosen.permanent_anyway, vec![path_key(&wt)]);
}

#[test]
fn chosen_confirm_moves_only_pending_keys() {
    let planned = |k: &str| Confirmation { key: k.into(), risks: vec![Risk::NoGit], method: Method::RecycleBin };
    let mut chosen =
        Chosen { keys: vec!["a".into(), "b".into()], pending: vec![planned("a")], confirmed: vec![planned("b")], ..Chosen::default() };
    let before = chosen.clone();

    assert!(!chosen.confirm("b"), "already confirmed");
    assert!(!chosen.confirm("z"), "not chosen");
    assert_eq!(chosen, before);
    assert!(chosen.confirm("a"));
    assert_eq!((chosen.pending, chosen.confirmed), (vec![], vec![planned("b"), planned("a")]));
}

#[test]
fn method_follows_the_target_and_the_mode() {
    use Method::*;
    let dir = Target::Dir { kind: ArtifactKind::ProjectFolder };
    let worktree = Target::Worktree { repo: PathBuf::from(r"C:\p") };
    let subtree = Target::Subtree { repo: PathBuf::from(r"C:\p") };
    let session = Target::Session { agent: Agent::Claude, id: "s".into() };
    // (target, method in Recycle mode, method in Permanent mode)
    let table = [
        (&dir, RecycleBin, Permanent),
        (&Target::File, RecycleBin, Permanent),
        (&subtree, RecycleBin, Permanent),
        (&worktree, GitWorktreeRemove, GitWorktreeRemove),
        (&session, HarnessSession, HarnessSession),
    ];
    for (target, recycle, permanent) in table {
        let methods = (method_of(target, RemoveMode::Recycle), method_of(target, RemoveMode::Permanent));
        assert_eq!(methods, (recycle, permanent), "{target:?}");
    }
    let texts: Vec<&str> = [RecycleBin, Permanent, GitWorktreeRemove, HarnessSession].iter().map(|m| m.describe()).collect();
    assert_eq!(
        texts,
        [
            "to the Recycle Bin",
            "deleted for good",
            "removed by git for good, not to the Recycle Bin",
            "removed with the session tools of its harness",
        ]
    );
}

#[test]
fn the_confirmation_errors_say_what_to_do() {
    let path = PathBuf::from(r"C:\w\agent-runs");
    let inner = path.join("bench-1");
    let table = [
        (
            SelectError::ConfirmNotIncluded(path.clone()),
            format!("--confirm-danger needs that folder, or a folder that holds it, with --include: {}", path.display()),
        ),
        (
            SelectError::ConfirmNotDanger(path.clone()),
            format!("--confirm-danger is only for danger items, and this one is not: {}", path.display()),
        ),
        (
            SelectError::DangerPermanent { path: path.clone(), inner: None },
            format!("refusing --permanent: danger folders go to the Recycle Bin only: {}", path.display()),
        ),
        (
            SelectError::DangerPermanent { path: path.clone(), inner: Some(inner.clone()) },
            format!(
                "refusing --permanent: it holds {}, a danger folder that goes to the Recycle Bin only: {}",
                inner.display(),
                path.display()
            ),
        ),
    ];
    for (err, text) in table {
        assert_eq!(err.to_string(), text);
    }
}

#[test]
fn check_dir_refuses_a_target_under_a_linked_ancestor() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let project = resolve_project(&root).unwrap();
    let real = base.join("real");
    fill(&real.join("copy"), 1, 1024);
    let link = base.join("linked");
    if !link_dir(&link, &real) {
        eprintln!("skipped: cannot create a junction here");
        return;
    }
    let through = link.join("copy");
    let item = Item {
        key: path_key(&through),
        category: Category::Copies,
        path: Some(through),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::NameOnly,
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        provenance: None,
    };

    assert_eq!(check_dir(&item, 7, &project).unwrap_err(), "now behind a link: not followed");
    assert!(real.join("copy").exists());
    let _ = fs::remove_dir(&link);
}

#[test]
fn a_copy_only_the_session_history_found_is_chosen_and_checked_again() {
    use crate::history::UserDirs;
    use crate::tests::history::fake::{descriptor, input, listing};
    let (_t, base) = tmp();
    let root = repo(&base.join("work").join("proj"));
    // Five levels down: out of the walk's reach, found only through a session's working directory.
    let deep = clone(&root, &base.join("a").join("b").join("c").join("d").join("e"));
    let listings = [listing(
        Agent::Claude,
        vec![descriptor(Agent::Claude, "main", "s1", &deep.to_string_lossy(), "2026-11-10T10:00:00Z", None)],
    )];
    let user = UserDirs::default();
    let with_history = Env { search_roots: Some(vec![base.clone()]), history: input(&listings, None, &user), ..env() };
    let scan = scan_in(&root, &with_history);
    let item = scan.items.iter().find(|i| i.key == path_key(&deep)).expect("the copy is listed");
    assert_eq!(item.category, Category::Copies);
    let chosen = select(&scan, &req(&[], &[&deep])).unwrap();

    // The clean reads the history again: without it the copy the history found is left alone.
    let without = Env { search_roots: Some(vec![base.clone()]), ..env() };
    let bin = FakeBin::new(PathBuf::from("unused"));
    let out = run_with(&scan, &chosen, RemoveMode::Permanent, true, &without, &bin, &|_| Vec::new());
    assert_eq!(outcome_of(&out, &deep), &skipped_with("the session history could not be read again"));

    let out = run_with(&scan, &chosen, RemoveMode::Permanent, true, &with_history, &bin, &|_| Vec::new());
    assert!(matches!(outcome_of(&out, &deep), Outcome::Removed { .. }), "{:#?}", out.items);
    assert!(deep.join("README.md").exists());
}

#[test]
fn check_dir_keeps_subtrees_inside_their_repo() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let other = repo(&base.join("other"));
    let project = resolve_project(&root).unwrap();
    let make = |path: &Path, target: Target| Item {
        key: path_key(path),
        category: Category::Scratch,
        path: Some(path.to_path_buf()),
        bytes: None,
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: Why::NameOnly,
        target,
        provenance: None,
    };
    let run1 = root.join("tmp").join("run1");
    fs::create_dir_all(&run1).unwrap();
    let outside = base.join("loose");
    fs::create_dir_all(&outside).unwrap();
    let in_git = root.join(".git").join("x");
    fs::create_dir_all(&in_git).unwrap();
    let subtree = |repo: &Path| Target::Subtree { repo: repo.to_path_buf() };
    let refused = [
        (make(&other.join("x"), subtree(&other)), "its repository is no longer the project or one of its worktrees"),
        (make(&outside, subtree(&root)), "it overlaps the project or one of its worktrees"),
        (make(&in_git, subtree(&root)), "it is inside the git folder"),
        (make(&run1, Target::Dir { kind: ArtifactKind::ProjectFolder }), "it overlaps the project or one of its worktrees"),
    ];
    fs::create_dir_all(other.join("x")).unwrap();
    for (item, why) in refused {
        assert_eq!(check_dir(&item, 1, &project).unwrap_err(), why, "{:?}", item.path);
    }
    let plan = check_dir(&make(&run1, subtree(&root)), 1, &project).unwrap();
    assert_eq!((plan.kind, plan.action), (ArtifactKind::Other, xiox_model::PlanAction::RemoveDir));
}

// ─────────────────────────── revalidation at removal ───────────────────────────

const DAY: i64 = 24 * 60 * 60 * 1000;

/// A session the fake store lists: `(id, cwd, updated, calls)`.
type Session = (&'static str, PathBuf, i64, Vec<ToolCall>);

/// What the fake stores list, and the calls each session made.
struct Stores {
    listings: Vec<(Agent, xiox_model::Loaded<Descriptor>)>,
    calls: FakeCalls,
}

impl Stores {
    fn of(sessions: Vec<Session>) -> Stores {
        let mut calls = FakeCalls::default();
        let mut listed = Vec::new();
        for (id, cwd, updated, made) in sessions {
            listed.push(descriptor(Agent::Claude, "main", id, &cwd.to_string_lossy(), &updated.to_string(), None));
            calls = calls.answer("main", id, Ok(made));
        }
        Stores { listings: vec![listing(Agent::Claude, listed)], calls }
    }

    /// A store whose listing fails and whose sessions cannot be read.
    fn failing(sessions: Vec<Session>) -> Stores {
        let ids: Vec<&str> = sessions.iter().map(|s| s.0).collect();
        let mut stores = Stores::of(sessions.clone());
        stores.listings[0].1.warnings.push("the store could not be listed".into());
        stores.calls = ids.into_iter().fold(FakeCalls::default(), |c, id| c.answer("main", id, Err(ReadFailure::Failed)));
        stores
    }
}

/// The project `<base>\work\proj`, whose session `s1` made `<base>\agent-runs\bench-1` and wrote
/// in it.
struct Lab {
    _guard: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    user: UserDirs,
}

impl Lab {
    fn new() -> Lab {
        let (guard, base) = tmp();
        let project = repo(&base.join("work").join("proj"));
        let home = base.join("home");
        fs::create_dir_all(&home).unwrap();
        let user = UserDirs { home: home.clone(), temp: home.join("AppData").join("Local").join("Temp"), resolved: true };
        let lab = Lab { _guard: guard, base, project, user };
        fs::create_dir_all(lab.bench()).unwrap();
        fs::write(lab.bench().join("a.txt"), "x").unwrap();
        lab
    }

    fn bench(&self) -> PathBuf {
        self.base.join("agent-runs").join("bench-1")
    }

    fn slash(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    /// `s1`, updated at `updated`: it made bench-1 and wrote `a.txt` in it.
    fn s1(&self, updated: i64) -> Session {
        let (bench, cwd) = (Lab::slash(&self.bench()), self.project.to_string_lossy().into_owned());
        let calls = vec![bash(&format!("mkdir -p {bench}"), &cwd), bash(&format!("echo x > {bench}/a.txt"), &cwd)];
        ("s1", self.project.clone(), updated, calls)
    }

    /// A session `id` working in `cwd`, updated at `updated`, that wrote `file` in bench-1.
    fn writer(&self, id: &'static str, cwd: &Path, updated: i64, file: &str) -> Session {
        let target = Lab::slash(&self.bench().join(file));
        (id, cwd.to_path_buf(), updated, vec![bash(&format!("echo y > {target}"), &cwd.to_string_lossy())])
    }

    fn env<'a>(&'a self, stores: &'a Stores, now_ms: i64) -> Env<'a> {
        Env {
            search_roots: Some(vec![self.base.clone()]),
            now_ms,
            history: input(&stores.listings, Some(&stores.calls), &self.user),
            ..env()
        }
    }

    /// The analysis, and bench-1 chosen with its confirmation in Recycle mode.
    fn confirmed_bench(&self, env: &Env) -> (ProjectScan, Chosen) {
        let scan = scan_in(&self.project, env);
        let bench = self.bench();
        let chosen = select(&scan, &asked(&[&bench], &[&bench], RemoveMode::Recycle)).unwrap();
        assert_eq!(chosen.confirmed.len(), 1, "{:#?}", scan.items);
        (scan, chosen)
    }
}

fn clean_in_recycle(scan: &ProjectScan, chosen: &Chosen, dry: bool, env: &Env, bin: &FakeBin) -> CleanOutcome {
    run_with(scan, chosen, RemoveMode::Recycle, dry, env, bin, &|_| Vec::new())
}

fn scan_item<'a>(scan: &'a ProjectScan, path: &Path) -> &'a Item {
    let key = path_key(path);
    scan.items.iter().find(|i| i.key == key).unwrap_or_else(|| panic!("{key} not listed: {:#?}", scan.items))
}

fn skipped_with(reason: impl Into<String>) -> Outcome {
    Outcome::Skipped { reason: reason.into() }
}

const RISKS_CHANGED: &str = "its risks changed since it was confirmed: choose it again";

#[test]
fn hand_made_chosen_gets_no_more_than_select() {
    let lab = Lab::new();
    // The copy made now is recent activity, so it is danger inside the named folder.
    let now = xiox_app::now_ms();
    let named = lab.base.join("work").join("proj-box");
    fs::create_dir_all(&named).unwrap();
    let copy = clone(&lab.project, &named.join("c"));
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let env = lab.env(&stores, now);
    let scan = scan_in(&lab.project, &env);
    let bench = lab.bench();
    let plan = |p: &Path, risks: Vec<Risk>| Confirmation { key: path_key(p), risks, method: Method::RecycleBin };
    let right = |p: &Path| plan(p, scan_item(&scan, p).risks.clone());
    assert_eq!(scan_item(&scan, &copy).level, Level::Danger);

    // (name, chosen folder, its confirmations, outcome)
    let table = [
        ("the container alone", &named, vec![right(&named)], skipped_with(format!("it holds {}, which needs an explicit confirmation", copy.display()))),
        (
            "the container and a false plan of its copy",
            &named,
            vec![right(&named), plan(&copy, Vec::new())],
            skipped_with(format!("it holds {}, whose risks changed since it was confirmed", copy.display())),
        ),
        (
            "the container and a plan of its copy that goes another way",
            &named,
            vec![right(&named), Confirmation { method: Method::Permanent, ..right(&copy) }],
            skipped_with(format!("it holds {}, whose removal method changed since it was confirmed", copy.display())),
        ),
        ("a danger folder without its confirmation", &bench, Vec::new(), skipped_with(DANGER_TEXT)),
        ("a danger folder with false risks", &bench, vec![plan(&bench, Vec::new())], skipped_with(RISKS_CHANGED)),
    ];
    let bin = FakeBin::new(lab.base.join("fake-bin"));
    for (name, folder, confirmed, expected) in table {
        let chosen = Chosen { keys: vec![path_key(folder)], confirmed, ..Chosen::default() };
        // A dry run: the same checks, and a regression never reaches the real Recycle Bin.
        let out = clean_in_recycle(&scan, &chosen, true, &env, &bin);
        assert_eq!(outcome_of(&out, folder), &expected, "{name}");
    }
    assert!(bench.join("a.txt").exists() && copy.join("README.md").exists() && named.exists());
    assert!(bin.taken.lock().unwrap().is_empty());
}

#[test]
fn replaced_folder_is_skipped_by_identity() {
    for case in ["recreated", "junction"] {
        let lab = Lab::new();
        let now = env().now_ms;
        let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
        let env = lab.env(&stores, now);
        let (scan, chosen) = lab.confirmed_bench(&env);
        let bench = lab.bench();
        let target = lab.base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), "keep").unwrap();

        fs::remove_dir_all(&bench).unwrap();
        let expected = match case {
            "recreated" => {
                fs::create_dir(&bench).unwrap();
                "it is not the folder that was analysed: it was replaced"
            }
            _ => {
                if !link_dir(&bench, &target) {
                    println!("skipped: this machine cannot make a junction");
                    continue;
                }
                // The fresh analysis never follows the link: the folder it analysed is gone.
                "no longer there"
            }
        };
        let out = clean_in_recycle(&scan, &chosen, true, &env, &FakeBin::new(lab.base.join("fake-bin")));
        assert_eq!(outcome_of(&out, &bench), &skipped_with(expected), "{case}");
        assert!(target.join("keep.txt").exists(), "{case}");
    }
}

#[test]
fn a_replaced_danger_folder_inside_a_chosen_one_stops_it() {
    let lab = Lab::new();
    // The copy made now is recent activity, so it is danger inside the named folder.
    let now = xiox_app::now_ms();
    let named = lab.base.join("work").join("proj-box");
    fs::create_dir_all(&named).unwrap();
    let copy = clone(&lab.project, &named.join("c"));
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let env = lab.env(&stores, now);
    let scan = scan_in(&lab.project, &env);
    let chosen = select(&scan, &asked(&[&named], &[&named, &copy], RemoveMode::Recycle)).unwrap();
    assert_eq!(chosen.confirmed.len(), 2, "{chosen:?}");

    fs::rename(&copy, lab.base.join("moved-c")).unwrap();
    clone(&lab.project, &copy);
    // A dry run: the same checks, and a regression never reaches the real Recycle Bin.
    let out = clean_in_recycle(&scan, &chosen, true, &env, &FakeBin::new(lab.base.join("fake-bin")));
    let expected = format!("it holds {}, which is not the folder that was analysed: it was replaced", copy.display());
    assert_eq!(outcome_of(&out, &named), &skipped_with(expected));
}

#[test]
fn an_identity_that_cannot_be_read_now_is_not_called_a_replacement() {
    let (_t, base) = tmp();
    let dir = base.join("work").join("proj-old");
    fs::create_dir_all(&dir).unwrap();
    let old = Item { identity: crate::identity::folder_id(&dir), ..listed(Category::Named, &dir, Why::NameOnly) };
    assert!(old.identity.is_some(), "this machine gives folders an identity");
    let unread = Item { identity: None, ..old.clone() };
    let checked = crate::clean::recheck(&old, Some(&unread), None, RemoveMode::Recycle);
    assert_eq!(checked, Err("its identity could not be checked".to_string()));
}

#[test]
fn a_confirmed_item_inside_the_chosen_one_depends_on_the_history() {
    let lab = Lab::new();
    let now = env().now_ms;
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let (mut scan, confirmed) = lab.confirmed_bench(&lab.env(&stores, now));
    // A container without provenance and not danger itself, as a hand-made request may name it.
    let runs = lab.base.join("agent-runs");
    scan.items.push(listed(Category::Named, &runs, Why::NameOnly));
    let chosen = Chosen { keys: vec![path_key(&runs)], ..confirmed };
    assert!(crate::clean::wants_history(&scan, &chosen), "{chosen:?}");

    // When that history cannot be read again, the container waits like an item with provenance.
    let failing = Stores::failing(vec![lab.s1(now - 3 * DAY)]);
    let out = clean_in_recycle(&scan, &chosen, true, &lab.env(&failing, now), &FakeBin::new(lab.base.join("bin")));
    assert_eq!(outcome_of(&out, &runs), &skipped_with("the session history could not be read again"));
}

#[test]
fn a_changed_risk_needs_a_new_choice() {
    let later = env().now_ms;
    let now = xiox_app::now_ms();
    // (name, clock of the analysis, clock of the removal, what happens in between, outcome)
    type Change = fn(&Lab, i64) -> Vec<Session>;
    let table: [(&str, i64, i64, Change, &str); 4] = [
        // Made now it is recent; two days later it is not: fewer risks are a change too.
        ("a risk goes away", now, later, |_, _| Vec::new(), RISKS_CHANGED),
        ("the project used it a minute ago", later, later, |lab, at| vec![lab.writer("s2", &lab.project, at - 60_000, "b.txt")], RISKS_CHANGED),
        (
            "another project wrote in it",
            later,
            later,
            |lab, at| vec![lab.writer("x1", &lab.base.join("agent-runs"), at - 3 * DAY, "c.txt")],
            "now blocked: sessions of another project wrote here",
        ),
        (
            "a repository with unsaved work appeared in it",
            later,
            later,
            |lab, _| {
                git(&lab.bench(), &["init", "-q", "-b", "main"]);
                Vec::new()
            },
            "now blocked: ",
        ),
    ];
    for (name, analysed_at, removed_at, change, expected) in table {
        let lab = Lab::new();
        let stores = Stores::of(vec![lab.s1(analysed_at - 3 * DAY)]);
        let (scan, chosen) = lab.confirmed_bench(&lab.env(&stores, analysed_at));

        let mut sessions = vec![lab.s1(analysed_at - 3 * DAY)];
        sessions.extend(change(&lab, removed_at));
        let again = Stores::of(sessions);
        let out = clean_in_recycle(&scan, &chosen, true, &lab.env(&again, removed_at), &FakeBin::new(lab.base.join("bin")));
        match outcome_of(&out, &lab.bench()) {
            Outcome::Skipped { reason } => assert!(reason.starts_with(expected), "{name}: {reason}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn history_is_read_again_at_removal() {
    let lab = Lab::new();
    let now = env().now_ms;
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let (scan, chosen) = lab.confirmed_bench(&lab.env(&stores, now));
    let bin = FakeBin::new(lab.base.join("bin"));

    let again = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let out = clean_in_recycle(&scan, &chosen, true, &lab.env(&again, now), &bin);
    assert!(again.calls.asked().contains(&"s1".to_string()), "{:?}", again.calls.asked());
    assert!(matches!(outcome_of(&out, &lab.bench()), Outcome::Removed { .. }), "{:#?}", out.items);

    let failing = Stores::failing(vec![lab.s1(now - 3 * DAY)]);
    let out = clean_in_recycle(&scan, &chosen, true, &lab.env(&failing, now), &bin);
    assert_eq!(outcome_of(&out, &lab.bench()), &skipped_with("the session history could not be read again"));
}

#[test]
fn confirmed_scratch_passes_every_check_in_a_dry_run() {
    let lab = Lab::new();
    let now = env().now_ms;
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let env = lab.env(&stores, now);
    let (scan, chosen) = lab.confirmed_bench(&env);

    let out = clean_in_recycle(&scan, &chosen, true, &env, &FakeBin::new(lab.base.join("bin")));
    let item = out.items.iter().find(|i| i.key == path_key(&lab.bench())).expect("bench-1 has an outcome");
    assert!(matches!(item.outcome, Outcome::Removed { .. }), "{item:?}");
    assert_eq!((item.method, item.category), (Method::RecycleBin, Category::Scratch));
    assert!(lab.bench().join("a.txt").exists());
}

#[test]
fn a_confirmed_folder_inside_the_project_is_checked_as_a_folder() {
    let lab = Lab::new();
    let now = env().now_ms;
    let runs = lab.project.join("runs");
    fs::create_dir_all(&runs).unwrap();
    fs::write(runs.join("r.txt"), "x").unwrap();
    let cwd = lab.project.to_string_lossy().into_owned();
    let stores = Stores::of(vec![("s1", lab.project.clone(), now - 3 * DAY, vec![bash(&format!("mkdir -p {}", Lab::slash(&runs)), &cwd)])]);
    let env = lab.env(&stores, now);
    let scan = scan_in(&lab.project, &env);
    assert!(matches!(scan_item(&scan, &runs).target, Target::Subtree { .. }), "{:#?}", scan.items);
    let chosen = select(&scan, &asked(&[&runs], &[&runs], RemoveMode::Recycle)).unwrap();

    let out = clean_in_recycle(&scan, &chosen, true, &env, &FakeBin::new(lab.base.join("bin")));
    assert!(matches!(outcome_of(&out, &runs), Outcome::Removed { .. }), "{:#?}", out.items);
}

#[test]
fn permanent_never_reaches_a_danger_folder() {
    let lab = Lab::new();
    let now = env().now_ms;
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let env = lab.env(&stores, now);
    let scan = scan_in(&lab.project, &env);
    let bench = lab.bench();
    // A plan that names the permanent method, so only the danger rule can stop it.
    let plan = Confirmation { key: path_key(&bench), risks: scan_item(&scan, &bench).risks.clone(), method: Method::Permanent };
    let chosen = Chosen { keys: vec![path_key(&bench)], confirmed: vec![plan], ..Chosen::default() };

    let out = run_with(&scan, &chosen, RemoveMode::Permanent, false, &env, &FakeBin::new(lab.base.join("bin")), &|_| Vec::new());
    assert_eq!(outcome_of(&out, &bench), &skipped_with("refusing --permanent: danger folders go to the Recycle Bin only"));
    assert!(bench.join("a.txt").exists());
}

#[test]
fn danger_worktree_is_removed_by_git_and_a_locked_one_is_left() {
    let lab = Lab::new();
    // Worktrees made now are recent activity: danger.
    let now = xiox_app::now_ms();
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let env = lab.env(&stores, now);
    let one = worktree(&lab.project, &lab.base.join("wt").join("one"), "one");
    let two = worktree(&lab.project, &lab.base.join("wt").join("two"), "two");
    let scan = scan_in(&lab.project, &env);
    assert_eq!((scan_item(&scan, &one).level, &scan_item(&scan, &one).risks), (Level::Danger, &vec![Risk::RecentActivity]));
    let chosen = select(&scan, &asked(&[&one, &two], &[&one, &two], RemoveMode::Recycle)).unwrap();
    assert!(chosen.confirmed.iter().all(|c| c.method == Method::GitWorktreeRemove), "{:?}", chosen.confirmed);
    // Locked after the choice: the fresh analysis blocks it before git is asked.
    git(&lab.project, &["worktree", "lock", &two.to_string_lossy()]);

    let out = clean_in_recycle(&scan, &chosen, false, &env, &FakeBin::new(lab.base.join("bin")));
    let removed = out.items.iter().find(|i| i.key == path_key(&one)).expect("one has an outcome");
    assert!(matches!(removed.outcome, Outcome::Removed { .. }), "{removed:?}");
    assert_eq!(removed.method, Method::GitWorktreeRemove);
    assert!(!one.exists());
    assert_eq!(outcome_of(&out, &two), &skipped_with("now blocked: locked with git worktree lock"));
    assert!(two.exists());
}

#[test]
fn a_failed_session_read_leaves_session_folders_alone_too() {
    let lab = Lab::new();
    let now = env().now_ms;
    let stores = Stores::of(vec![lab.s1(now - 3 * DAY)]);
    let (scan, chosen) = lab.confirmed_bench(&lab.env(&stores, now));
    let blind = Env { health: SessionHealth { live_failed: true, archived_failed: false }, ..lab.env(&stores, now) };

    let out = clean_in_recycle(&scan, &chosen, true, &blind, &FakeBin::new(lab.base.join("bin")));
    let expected = "the sessions of this PC could not be read, so a live agent session cannot be ruled out";
    assert_eq!(outcome_of(&out, &lab.bench()), &skipped_with(expected));
}

#[test]
fn a_scan_made_without_the_history_is_checked_without_it() {
    let lab = Lab::new();
    let now = env().now_ms;
    let named = lab.base.join("work").join("proj-old");
    fs::create_dir_all(&named).unwrap();
    let cwd = lab.project.to_string_lossy().into_owned();
    let wrote = bash(&format!("echo x > {}/n.txt", Lab::slash(&named)), &cwd);
    let stores = Stores::of(vec![("s1", lab.project.clone(), now - 3 * DAY, vec![wrote])]);
    let env = lab.env(&stores, now);
    // `--no-history`: the folder is danger for its missing git alone, and no session is read.
    let scan = analyze_with(&lab.project, &Options { history: false, ..Options::default() }, &env).unwrap();
    assert_eq!(scan_item(&scan, &named).risks, vec![Risk::NoGit]);
    let chosen = select(&scan, &asked(&[&named], &[&named], RemoveMode::Recycle)).unwrap();

    let out = clean_in_recycle(&scan, &chosen, true, &env, &FakeBin::new(lab.base.join("bin")));
    assert!(matches!(outcome_of(&out, &named), Outcome::Removed { .. }), "{:#?}", out.items);
    assert!(stores.calls.asked().is_empty(), "the history was read: {:?}", stores.calls.asked());
}

#[test]
fn a_folder_inside_the_project_counts_for_the_recycle_bin() {
    let (_t, base) = tmp();
    let runs = base.join("proj").join("runs");
    fill(&runs, 1, MB);
    let scratch = at_risk(listed(Category::Scratch, &runs, session_folder()), &[Risk::NoGit]);
    let scan = scratch_scan(&base, vec![Item { target: Target::Subtree { repo: base.join("proj") }, ..scratch }]);
    let chosen = Chosen { keys: vec![path_key(&runs)], ..Chosen::default() };

    // It goes to the bin like any folder; unmeasured in the scan, it is measured now.
    let short = shortfall_with(&scan, &chosen, RemoveMode::Recycle, |_| Some(1));
    assert_eq!(short.len(), 1, "{short:?}");
    assert!(short[0].1 >= MB64, "{short:?}");
}
