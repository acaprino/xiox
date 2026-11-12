use super::*;
use dc_model::{CleanPlan, PlanAction, PlanItem, RemoveMode, ItemOutcome, ArtifactKind, CleanResult};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn item(path: &Path, kind: ArtifactKind) -> PlanItem {
    PlanItem {
        path: path.to_path_buf(),
        kind,
        action: PlanAction::RemoveDir,
        // Deliberately wrong: the remover must measure, not trust this.
        expected_bytes: 1,
        project_id: "p".into(),
        reason: "test".into(),
    }
}

fn plan(mode: RemoveMode, items: Vec<PlanItem>) -> CleanPlan {
    CleanPlan { id: "t".into(), mode, items, expected_bytes: 0 }
}

fn run(mode: RemoveMode, it: PlanItem, dry: bool) -> (ItemOutcome, CleanResult) {
    let res = DefaultRemover.execute(&plan(mode, vec![it]), dry);
    (res.items[0].outcome.clone(), res)
}

fn write(p: &Path, n: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; n]).unwrap();
}

/// proj/target with 3 files of 10_000 bytes in nested dirs.
fn make_target(root: &Path) -> PathBuf {
    let t = root.join("proj").join("target");
    write(&t.join("a.bin"), 10_000);
    write(&t.join("debug").join("b.bin"), 10_000);
    write(&t.join("debug").join("deps").join("c.bin"), 10_000);
    t
}

fn skipped_reason(o: &ItemOutcome) -> String {
    match o {
        ItemOutcome::Skipped { reason } => reason.clone(),
        other => panic!("expected Skipped, got {other:?}"),
    }
}

fn mklink_junction(link: &Path, target: &Path) {
    let st = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
    assert!(st.status.success(), "mklink failed: {}", String::from_utf8_lossy(&st.stdout));
}

#[test]
fn permanent_deletes_tree_including_readonly_file() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let ro = t.join("debug").join("readonly.txt");
    write(&ro, 100);
    let mut perm = fs::metadata(&ro).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&ro, perm).unwrap();

    let (o, res) = run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), false);
    assert!(matches!(o, ItemOutcome::Removed { bytes_freed } if bytes_freed >= 30_100), "{o:?}");
    assert!(!t.exists());
    assert!(tmp.path().join("proj").exists(), "parent must stay");
    assert!(res.bytes_freed >= 30_100 && !res.dry_run);
}

#[test]
fn bytes_freed_is_measured_not_expected() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let size = measure_dir(&t);
    assert_eq!(size.logical_bytes, 30_000);
    assert_eq!(size.file_count, 3);
    let alloc = size.allocated_bytes.unwrap();
    assert!(alloc >= 30_000 && alloc.is_multiple_of(4096), "allocated {alloc}");

    let (o, res) = run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), false);
    // expected_bytes was 1: the figure must come from the measurement.
    assert!(matches!(o, ItemOutcome::Removed { bytes_freed } if bytes_freed == size.effective_bytes()), "{o:?}");
    assert_eq!(res.bytes_freed, size.effective_bytes());
}

#[test]
fn hardlinks_count_only_when_all_links_are_inside() {
    let tmp = TempDir::new().unwrap();
    let t = tmp.path().join("proj").join("target");
    write(&t.join("one.bin"), 100_000);
    fs::hard_link(t.join("one.bin"), t.join("two.bin")).unwrap();
    let inside = measure_dir(&t);
    assert_eq!(inside.logical_bytes, 200_000);
    assert!(inside.allocated_bytes.unwrap() < 200_000, "hardlink double counted: {inside:?}");

    // Third link outside the tree: deleting the tree frees nothing for this file.
    fs::hard_link(t.join("one.bin"), tmp.path().join("proj").join("outside.bin")).unwrap();
    assert_eq!(measure_dir(&t).allocated_bytes, Some(0));
}

#[test]
fn junction_inside_tree_is_not_followed() {
    let tmp = TempDir::new().unwrap();
    let outside = tmp.path().join("outside");
    write(&outside.join("precious.txt"), 5_000);
    let t = make_target(tmp.path());
    mklink_junction(&t.join("debug").join("link"), &outside);

    // The junction target must not be measured either.
    assert_eq!(measure_dir(&t).logical_bytes, 30_000);

    let (o, _) = run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), false);
    assert!(matches!(o, ItemOutcome::Removed { .. }), "{o:?}");
    assert!(!t.exists());
    assert!(outside.join("precious.txt").exists(), "junction target was followed and deleted!");
}

#[test]
fn item_that_is_a_junction_removes_only_the_link() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real").join("stuff");
    write(&real.join("keep.txt"), 5_000);
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    let link = proj.join("target");
    mklink_junction(&link, &real);

    let (dry, _) = run(RemoveMode::Permanent, item(&link, ArtifactKind::RustTarget), true);
    assert!(skipped_reason(&dry).starts_with("dry-run: would free 0 bytes"));
    assert!(link.exists());

    for mode in [RemoveMode::Permanent, RemoveMode::Recycle] {
        let (o, _) = run(mode, item(&link, ArtifactKind::RustTarget), false);
        assert!(matches!(o, ItemOutcome::Removed { bytes_freed: 0 }), "{o:?}");
        assert!(!link.exists());
        assert!(real.join("keep.txt").exists(), "link target was touched");
        if mode == RemoveMode::Permanent {
            mklink_junction(&link, &real); // recreate for the Recycle round
        }
    }
}

#[test]
fn refuses_dangerous_or_corrupt_items() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let src = tmp.path().join("proj").join("sources");
    write(&src.join("main.rs"), 10);
    let file = tmp.path().join("proj2").join("target");
    write(&file, 10);

    let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
    let cases: Vec<(PathBuf, ArtifactKind, &str)> = vec![
        (PathBuf::from(r"proj\target"), ArtifactKind::RustTarget, "not absolute"),
        (tmp.path().join("nope").join("target"), ArtifactKind::RustTarget, "does not exist"),
        (PathBuf::from(r"C:\"), ArtifactKind::Other, "shallow"),
        (PathBuf::from(r"C:\target"), ArtifactKind::RustTarget, "shallow"),
        (PathBuf::from(r"C:\Windows\System32\target"), ArtifactKind::RustTarget, "system"),
        (PathBuf::from(r"C:\Program Files\App\target"), ArtifactKind::RustTarget, "system"),
        (PathBuf::from(r"C:\Program Files (x86)\App\node_modules"), ArtifactKind::NodeModules, "system"),
        (PathBuf::from(r"C:\ProgramData\x\target"), ArtifactKind::RustTarget, "system"),
        (profile.clone(), ArtifactKind::Other, "profile"),
        (profile.parent().unwrap().to_path_buf(), ArtifactKind::Other, "shallow"),
        (profile.join("Documents"), ArtifactKind::Other, "never a removable"),
        (t.join("..").join("target"), ArtifactKind::RustTarget, "'..'"),
        (src.clone(), ArtifactKind::RustTarget, "does not look like"),
        (tmp.path().join("proj").join("node_modules"), ArtifactKind::RustTarget, "does not look like"),
        (file, ArtifactKind::RustTarget, "not a directory"),
    ];
    for (path, kind, needle) in cases {
        let (o, _) = run(RemoveMode::Permanent, item(&path, kind), false);
        let r = skipped_reason(&o);
        assert!(r.contains(needle), "{}: reason '{r}' lacks '{needle}'", path.display());
    }
    assert!(t.exists() && src.join("main.rs").exists(), "a refused item was touched");
}

#[test]
fn dry_run_changes_nothing_and_reports_size() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let size = measure_dir(&t).effective_bytes();
    let (o, res) = run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), true);
    assert_eq!(skipped_reason(&o), format!("dry-run: would free {size} bytes"));
    assert!(res.dry_run && res.bytes_freed == 0);
    assert_eq!(measure_dir(&t).logical_bytes, 30_000);
}

#[test]
fn duplicate_items_are_skipped_and_counted_once() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let p = plan(RemoveMode::Permanent, vec![item(&t, ArtifactKind::RustTarget), item(&t, ArtifactKind::RustTarget)]);
    let res = DefaultRemover.execute(&p, false);
    assert!(matches!(res.items[0].outcome, ItemOutcome::Removed { .. }));
    assert!(skipped_reason(&res.items[1].outcome).contains("duplicate"));
}

#[test]
#[cfg(windows)]
fn locked_file_fails_naming_the_file_and_bytes_not_counted() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let locked = t.join("debug").join("locked.bin");
    write(&locked, 10);
    // No sharing at all: delete must fail with a sharing violation.
    let _held = fs::OpenOptions::new().read(true).share_mode(0).open(&locked).unwrap();

    let (o, res) = run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), false);
    match o {
        ItemOutcome::Failed { error, locked_by } => {
            assert!(error.contains("locked.bin"), "error should name the blocking file: {error}");
            eprintln!("locked_by = {locked_by:?}");
            assert!(!locked_by.is_empty(), "Restart Manager reported no holder");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(res.bytes_freed, 0);
    assert!(locked.exists());
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

fn git_available() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn make_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q"]));
    assert!(git(&repo, &["config", "user.email", "t@t"]));
    assert!(git(&repo, &["config", "user.name", "t"]));
    assert!(git(&repo, &["config", "commit.gpgsign", "false"]));
    fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    fs::write(repo.join("f.txt"), "hi").unwrap();
    assert!(git(&repo, &["add", "."]));
    assert!(git(&repo, &["commit", "-q", "-m", "init"]));
    repo
}

fn wt_item(wt: &Path, repo: &Path) -> PlanItem {
    PlanItem { action: PlanAction::GitWorktreeRemove { repo: repo.to_path_buf() }, ..item(wt, ArtifactKind::WorktreeDir) }
}

#[test]
fn git_worktree_removed_cleanly_and_dirty_one_is_not_forced() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let repo = make_repo(tmp.path());
    let clean = tmp.path().join("wt-clean");
    let dirty = tmp.path().join("wt-dirty");
    assert!(git(&repo, &["worktree", "add", "-q", "-b", "b1", clean.to_str().unwrap()]));
    assert!(git(&repo, &["worktree", "add", "-q", "-b", "b2", dirty.to_str().unwrap()]));
    // Ignored build output inside the clean worktree is fine to delete with it.
    write(&clean.join("target").join("big.bin"), 50_000);
    // Uncommitted work in the other one must survive.
    fs::write(dirty.join("wip.txt"), "unsaved work").unwrap();

    let (dry, _) = run(RemoveMode::Permanent, wt_item(&clean, &repo), true);
    assert!(skipped_reason(&dry).starts_with("dry-run: would free"));
    assert!(clean.exists());

    let (o, _) = run(RemoveMode::Permanent, wt_item(&clean, &repo), false);
    assert!(matches!(o, ItemOutcome::Removed { bytes_freed } if bytes_freed >= 50_000), "{o:?}");
    assert!(!clean.exists());
    let list = Command::new("git").arg("-C").arg(&repo).args(["worktree", "list", "--porcelain"]).output().unwrap();
    let list = String::from_utf8_lossy(&list.stdout).to_lowercase();
    assert!(!list.contains("wt-clean"), "worktree still registered:\n{list}");

    let (o, res) = run(RemoveMode::Permanent, wt_item(&dirty, &repo), false);
    assert!(matches!(&o, ItemOutcome::Failed { error, .. } if error.contains("git worktree remove failed")), "{o:?}");
    assert!(dirty.join("wip.txt").exists(), "dirty worktree was force-removed");
    assert_eq!(res.bytes_freed, 0);

    // The repo's main worktree and its own dir are refused outright.
    let (o, _) = run(RemoveMode::Permanent, wt_item(&repo, &repo), false);
    assert!(skipped_reason(&o).contains("worktree itself"));
}

#[test]
fn recycle_mode_moves_tree_to_bin() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let (o, _) = run(RemoveMode::Recycle, item(&t, ArtifactKind::RustTarget), false);
    match o {
        ItemOutcome::Removed { bytes_freed } => {
            assert!(bytes_freed >= 30_000);
            assert!(!t.exists());
        }
        // Headless CI sessions may have no shell/recycle bin; the contract is still "Failed + intact".
        ItemOutcome::Failed { error, .. } => {
            eprintln!("recycle unavailable in this environment: {error}");
            assert!(t.exists());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn catalog_items_are_checked_against_the_catalog() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("proj");
    for d in [".nx/cache", "cypress/videos", "src", "videos", "cache", "node_modules"] {
        fs::create_dir_all(p.join(d)).unwrap();
    }
    for ok in [p.join(".nx").join("cache"), p.join("cypress").join("videos")] {
        assert!(guard::validate(&item(&ok, ArtifactKind::Catalog)).is_ok(), "{}", ok.display());
    }
    let refused = [
        (p.join("src"), "never a removable"),
        (p.join("videos"), "never a removable"), // denied alone: only `cypress/videos` is an entry
        (p.join("cache"), "does not look like"),
        (p.join("node_modules"), "does not look like"), // not a catalog entry
    ];
    for (path, needle) in refused {
        let r = guard::validate(&item(&path, ArtifactKind::Catalog)).err().unwrap_or_default();
        assert!(r.contains(needle), "{}: reason '{r}' lacks '{needle}'", path.display());
    }
    // the DENY_NAMES exception is for catalog items only
    let r = guard::validate(&item(&p.join("cypress").join("videos"), ArtifactKind::Other)).err().unwrap_or_default();
    assert!(r.contains("never a removable"), "{r}");
}

/// Every folder of the production catalog passes the guard as a `Catalog` item (prefix and
/// suffix entries with a sample name, Meson with its `meson-private`). Needs a drive letter:
/// Windows only.
#[test]
#[cfg_attr(not(windows), ignore = "the guard needs a drive-letter path")]
fn guard_accepts_every_catalog_dir() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("proj");
    let dirs = dc_scanner::catalog_dirs();
    assert_eq!(dirs.len(), 95, "distinct folders of the catalog");
    for d in dirs {
        let rel = if d == "*" { "builddir".to_string() } else { d.replace("*.", "x.").replace('*', "1") };
        let path = rel.split('/').fold(p.clone(), |acc, seg| acc.join(seg));
        fs::create_dir_all(&path).unwrap();
        if d == "*" {
            fs::create_dir_all(path.join("meson-private")).unwrap();
            fs::write(path.join("build.ninja"), b"x").unwrap();
        }
        let r = guard::validate(&item(&path, ArtifactKind::Catalog));
        assert!(r.is_ok(), "{d}: {:?}", r.err());
    }
    // a Meson-looking name without meson-private is refused
    fs::create_dir_all(p.join("other")).unwrap();
    assert!(guard::validate(&item(&p.join("other"), ArtifactKind::Catalog)).is_err());
}

// ───────────────────────────── parallel workers ─────────────────────────────

/// `root/proj/target` with 20 chains of 10 nested folders (200 folders), 50 files of 100
/// bytes in each: 10,000 files.
fn make_big_target(root: &Path, chains: usize, depth: usize, per_dir: usize) -> PathBuf {
    let t = root.join("proj").join("target");
    for c in 0..chains {
        let mut d = t.join(format!("c{c}"));
        for level in 0..depth {
            fs::create_dir_all(&d).unwrap();
            for f in 0..per_dir {
                fs::write(d.join(format!("f{f}.bin")), [b'x'; 100]).unwrap();
            }
            d = d.join(format!("l{level}"));
        }
    }
    t
}

#[test]
fn ten_thousand_files_go_with_one_or_eight_workers() {
    let mut freed = Vec::new();
    for workers in [1, 8] {
        let tmp = TempDir::new().unwrap();
        let t = make_big_target(tmp.path(), 20, 10, 50);
        let size = measure_dir(&t);
        assert_eq!(size.file_count, 10_000);
        let ticks = std::sync::Mutex::new(Vec::<Tick>::new());
        let res = ParallelRemover { workers }.execute_with(
            &plan(RemoveMode::Permanent, vec![item(&t, ArtifactKind::RustTarget)]),
            false,
            &|t| ticks.lock().unwrap().push(t),
            &Cancel::default(),
        );
        assert!(matches!(res.items[0].outcome, ItemOutcome::Removed { .. }), "{workers}: {:?}", res.items[0]);
        assert!(!t.exists(), "{workers}: the tree is still there");
        assert!(tmp.path().join("proj").exists(), "parent must stay");
        assert_eq!(res.files_removed, 10_000, "{workers} workers");
        assert_eq!(res.dirs_removed, 201, "{workers} workers: 200 folders and the item itself");
        assert_eq!(res.bytes_freed, size.effective_bytes(), "{workers} workers");
        assert!(!res.cancelled && res.recycled_bytes == 0);
        for t in ticks.into_inner().unwrap() {
            assert!(t.done_bytes <= t.total_bytes && t.done_items <= t.total_items && t.total_items == 1, "{t:?}");
        }
        freed.push(res.bytes_freed);
    }
    assert_eq!(freed[0], freed[1], "same bytes with 1 and 8 workers");
}

#[test]
#[cfg(windows)]
fn a_locked_file_fails_only_its_item_and_the_others_go() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = TempDir::new().unwrap();
    let targets: Vec<PathBuf> = ["a", "b", "c"].iter().map(|n| make_target(&tmp.path().join(n))).collect();
    let locked = targets[0].join("debug").join("locked.bin");
    write(&locked, 10);
    let others: u64 = targets[1..].iter().map(|t| measure_dir(t).effective_bytes()).sum();
    let _held = fs::OpenOptions::new().read(true).share_mode(0).open(&locked).unwrap();

    let items = targets.iter().map(|t| item(t, ArtifactKind::RustTarget)).collect();
    let res = ParallelRemover { workers: 8 }.execute(&plan(RemoveMode::Permanent, items), false);
    match &res.items[0].outcome {
        ItemOutcome::Failed { error, locked_by } => {
            assert!(error.contains("locked.bin"), "error should name the blocking file: {error}");
            eprintln!("locked_by = {locked_by:?}");
            assert!(!locked_by.is_empty(), "Restart Manager reported no holder");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    for (t, r) in targets[1..].iter().zip(&res.items[1..]) {
        assert!(matches!(r.outcome, ItemOutcome::Removed { .. }), "{r:?}");
        assert!(!t.exists());
    }
    assert!(locked.exists());
    assert_eq!(res.bytes_freed, others, "the failed item frees nothing");
    // 3 files of each removed item, and the 3 unlocked files of the failed one.
    assert_eq!(res.files_removed, 9);
    assert!(!res.cancelled);
}

/// "Annullato: liberati {n} byte" -> n; "Annullato" -> 0.
fn cancelled_bytes(reason: &str) -> u64 {
    match reason.strip_prefix("Annullato: liberati ") {
        Some(rest) => rest.strip_suffix(" byte").unwrap().parse().unwrap(),
        None => {
            assert_eq!(reason, "Annullato");
            0
        }
    }
}

#[test]
fn cancel_after_the_first_tick_stops_and_counts_what_went() {
    let tmp = TempDir::new().unwrap();
    let targets: Vec<PathBuf> =
        (0..4).map(|i| make_big_target(&tmp.path().join(format!("p{i}")), 10, 6, 50)).collect();
    let total: u64 = targets.iter().map(|t| measure_dir(t).effective_bytes()).sum();
    let cancel = Cancel::default();
    let items = targets.iter().map(|t| item(t, ArtifactKind::RustTarget)).collect();
    let res = ParallelRemover { workers: 1 }.execute_with(
        &plan(RemoveMode::Permanent, items),
        false,
        &|_| cancel.cancel(),
        &cancel,
    );
    assert!(cancel.is_cancelled(), "no tick came: the cleanup ended within 100 ms");
    assert!(res.cancelled);
    let mut skipped = 0;
    for r in &res.items {
        match &r.outcome {
            ItemOutcome::Removed { .. } => {}
            ItemOutcome::Skipped { reason } => skipped += cancelled_bytes(reason),
            other => panic!("{other:?}"),
        }
    }
    assert!(res.items.iter().any(|r| matches!(r.outcome, ItemOutcome::Skipped { .. })), "{res:?}");
    assert!(skipped + res.bytes_freed <= total, "{skipped} + {} > {total}", res.bytes_freed);
    assert!(res.files_removed < 4 * 3000);
}

#[test]
fn a_cancel_before_the_start_touches_nothing() {
    let tmp = TempDir::new().unwrap();
    let t = make_target(tmp.path());
    let cancel = Cancel::default();
    cancel.cancel();
    let res = ParallelRemover { workers: 2 }.execute_with(
        &plan(RemoveMode::Permanent, vec![item(&t, ArtifactKind::RustTarget)]),
        false,
        &|_| {},
        &cancel,
    );
    assert_eq!(skipped_reason(&res.items[0].outcome), "Annullato");
    assert!(res.cancelled && res.bytes_freed == 0 && res.files_removed == 0);
    assert_eq!(measure_dir(&t).logical_bytes, 30_000);
}

#[test]
fn loose_files_go_folders_and_missing_files_do_not() {
    let tmp = TempDir::new().unwrap();
    let mut files: Vec<(PathBuf, u64)> = (0..100)
        .map(|i| {
            let p = tmp.path().join(format!("f{i}.tmp"));
            write(&p, 10);
            (p, 10)
        })
        .collect();
    let dir = tmp.path().join("adir");
    fs::create_dir_all(&dir).unwrap();
    files.push((dir.clone(), 5));
    files.push((tmp.path().join("missing.tmp"), 7));
    let (bytes, n, failed) = remove_files_parallel(&files, 4, &|_| {}, &Cancel::default());
    assert_eq!((bytes, n), (1000, 100));
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].0, dir);
    assert!(dir.exists());
    assert!(files[..100].iter().all(|(p, _)| !p.exists()));
}

#[test]
#[cfg(windows)]
fn remove_files_parallel_leaves_the_three_locked_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = TempDir::new().unwrap();
    let files: Vec<(PathBuf, u64)> = (0..1000)
        .map(|i| {
            let p = tmp.path().join(format!("f{i}.tmp"));
            write(&p, 50);
            (p, 50)
        })
        .collect();
    let locked = [7, 500, 999];
    let _held: Vec<fs::File> = locked
        .iter()
        .map(|&i| fs::OpenOptions::new().read(true).share_mode(0).open(&files[i].0).unwrap())
        .collect();
    let (bytes, n, failed) = remove_files_parallel(&files, 8, &|_| {}, &Cancel::default());
    assert_eq!(n, 997);
    assert_eq!(bytes, 997 * 50);
    let failed_paths: Vec<_> = failed.iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(failed_paths, locked.iter().map(|&i| files[i].0.clone()).collect::<Vec<_>>());
    assert!(locked.iter().all(|&i| files[i].0.exists()));
}
