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
