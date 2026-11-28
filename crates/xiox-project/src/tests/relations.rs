//! `analyze_relations` (worktrees, copies and named folders of one repository) and `keep_once`.

use super::fixtures::*;
use crate::*;
use xiox_model::RemoveMode;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// base/proj (a repository with a plain `target` folder), a worktree far from it, a clone elsewhere, and a folder named
/// after it beside it.
struct World {
    _t: tempfile::TempDir,
    proj: PathBuf,
    wt: PathBuf,
    copy: PathBuf,
    old: PathBuf,
}

fn world() -> World {
    let (t, base) = tmp();
    let proj = repo(&base.join("proj"));
    rust_build(&proj, 1000);
    let wt = worktree(&proj, &base.join("far").join("proj-wt"), "feature");
    let copy = clone(&proj, &base.join("elsewhere").join("copy"));
    let old = base.join("proj-old");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("notes.txt"), "old").unwrap();
    World { _t: t, proj, wt, old: canon(&old), copy }
}

fn input() -> RelationsInput<'static> {
    // The clock of the fixtures, two days ahead: nothing made by the test is recent activity.
    RelationsInput { sessions: &[], sessions_failed: false, now_ms: env().now_ms }
}

#[test]
fn relations_list_worktrees_copies_and_named_folders_and_nothing_else() {
    let w = world();
    let scan = analyze_relations(&w.proj, &[w.copy.clone()], &input()).unwrap();
    let cats: HashSet<Category> = scan.items.iter().map(|i| i.category).collect();
    assert_eq!(cats, HashSet::from([Category::Worktrees, Category::Copies, Category::Named]), "{:?}", scan.items);
    let has = |cat: Category, p: &PathBuf| scan.items.iter().any(|i| i.category == cat && i.path.as_ref() == Some(p));
    assert!(has(Category::Worktrees, &w.wt), "a worktree outside the folder: {:?}", scan.items);
    assert!(has(Category::Copies, &w.copy), "a copy far away, given as known: {:?}", scan.items);
    assert!(has(Category::Named, &w.old), "a folder named after it, beside it: {:?}", scan.items);
    assert_eq!(scan.search, SearchMode::ParentOnly);
    assert!(scan.history.is_none());
    assert!(scan.branches.is_empty());
}

#[test]
fn a_relations_scan_round_trips_through_select_and_clean_dry_run() {
    let w = world();
    // The real clock, the one `clean` checks again with: the copy made a moment ago is recent activity, so `danger`, and
    // it goes only with its own confirmation, as the desktop will ask for it.
    let input = RelationsInput { sessions: &[], sessions_failed: false, now_ms: xiox_app::now_ms() };
    let scan = analyze_relations(&w.proj, &[w.copy.clone()], &input).unwrap();
    let listed = scan.items.iter().find(|i| i.path.as_ref() == Some(&w.copy)).unwrap();
    assert_eq!(listed.level, Level::Danger, "{listed:?}");
    let req = CleanRequest { categories: vec![], include: vec![w.copy.clone()], confirm_danger: vec![w.copy.clone()], mode: RemoveMode::Recycle };
    let chosen = select(&scan, &req).unwrap();
    let out = clean_without_sessions(&scan, &chosen, RemoveMode::Recycle, true, &[]);
    let item = out.items.iter().find(|o| o.path.as_ref() == Some(&w.copy)).unwrap();
    assert!(!matches!(item.outcome, Outcome::Skipped { .. }), "the copy is found again, not skipped as changed: {:?}", item.outcome);
    assert!(w.copy.exists(), "a dry run removes nothing");
}

#[test]
fn a_relations_scan_round_trips_for_a_caution_copy_a_confirmed_named_folder_and_a_worktree() {
    let w = world();
    // Three days old: no recent activity with the real clock that `clean` checks again with.
    let old = xiox_app::now_ms() - 3 * 24 * 60 * 60 * 1000;
    for dir in [&w.copy, &w.old, &w.wt] {
        set_times(dir, None, old);
    }
    let input = RelationsInput { sessions: &[], sessions_failed: false, now_ms: xiox_app::now_ms() };
    let scan = analyze_relations(&w.proj, &[w.copy.clone()], &input).unwrap();
    let level = |p: &PathBuf| scan.items.iter().find(|i| i.path.as_ref() == Some(p)).unwrap().level;
    // A folder known only by its name and without git is always danger (no git): it goes with its own confirmation.
    assert_eq!((level(&w.copy), level(&w.old), level(&w.wt)), (Level::Caution, Level::Danger, Level::Recommended));
    let req = CleanRequest {
        categories: vec![],
        include: vec![w.copy.clone(), w.old.clone(), w.wt.clone()],
        confirm_danger: vec![w.old.clone()],
        mode: RemoveMode::Recycle,
    };
    let chosen = select(&scan, &req).unwrap();
    let out = clean_without_sessions(&scan, &chosen, RemoveMode::Recycle, true, &[]);
    for p in [&w.copy, &w.old, &w.wt] {
        let item = out.items.iter().find(|o| o.path.as_ref() == Some(p)).unwrap();
        assert!(matches!(item.outcome, Outcome::Removed { .. }), "{} was not taken: {:?}", p.display(), item.outcome);
        assert!(p.exists(), "a dry run removes nothing");
    }
}

// ───────────────────────────── keep_once ─────────────────────────────

fn listed(cat: Category, path: &str) -> Item {
    let p = PathBuf::from(path);
    Item {
        key: xiox_model::path_key(&p),
        category: cat,
        path: Some(p),
        bytes: Some(1),
        level: Level::Caution,
        risks: Vec::new(),
        identity: None,
        blocked: Vec::new(),
        why: match cat {
            Category::Worktrees => Why::LinkedWorktree { branch: None, head: None, own_commits: None },
            Category::Copies => Why::SameRepository { shared_root_commit: true, same_origin: false, has_own_work: Some(false) },
            _ => Why::NameOnly,
        },
        target: Target::Dir { kind: xiox_model::ArtifactKind::ProjectFolder },
        provenance: None,
    }
}

fn scan_at(root: &str, items: Vec<Item>) -> ProjectScan {
    let root = PathBuf::from(root);
    ProjectScan {
        schema: SCHEMA,
        project: Project {
            name: root.file_name().unwrap().to_string_lossy().into_owned(),
            root,
            is_repo: true,
            origin: None,
            worktrees: Vec::new(),
        },
        items,
        branches: Vec::new(),
        notes: Vec::new(),
        history: None,
        search: SearchMode::ParentOnly,
        scanned_at_ms: 0,
        elapsed_ms: 0,
    }
}

fn paths(scan: &ProjectScan) -> Vec<String> {
    scan.items.iter().map(|i| i.path.as_ref().unwrap().display().to_string()).collect()
}

fn keys(roots: &[&str]) -> HashSet<String> {
    roots.iter().map(|r| xiox_model::path_key(std::path::Path::new(r))).collect()
}

#[test]
fn keep_once_leaves_each_folder_in_one_scan() {
    // The scan that is not a reference comes first: only the reference rank can give the copy to the reference.
    let mut scans = vec![
        scan_at(r"D:\q\supads", vec![listed(Category::Copies, r"D:\x\supads-copy")]),
        scan_at(r"D:\p\supads", vec![listed(Category::Copies, r"D:\x\supads-copy"), listed(Category::Named, r"D:\p\supads-work")]),
        scan_at(r"D:\x\supads-copy", vec![listed(Category::Copies, r"D:\p\supads")]),
        scan_at(r"D:\p\supads-studio", vec![listed(Category::Named, r"D:\p\supads-studio-old")]),
    ];
    keep_once(&mut scans, &keys(&[r"D:\p\supads"]));
    assert!(paths(&scans[0]).is_empty(), "the reference's scan keeps the copy");
    assert_eq!(paths(&scans[1]), [r"D:\x\supads-copy", r"D:\p\supads-work"]);
    assert!(paths(&scans[2]).is_empty(), "the reference is never a copy");
    assert_eq!(paths(&scans[3]), [r"D:\p\supads-studio-old"]);
}

#[test]
fn a_folder_that_is_named_here_and_a_copy_there_stays_a_copy() {
    let mut scans = vec![
        scan_at(r"D:\p\supads", vec![listed(Category::Named, r"D:\p\supads-x")]),
        scan_at(r"D:\q\other", vec![listed(Category::Copies, r"D:\p\supads-x")]),
    ];
    keep_once(&mut scans, &HashSet::new());
    assert!(paths(&scans[0]).is_empty());
    assert_eq!(paths(&scans[1]), [r"D:\p\supads-x"]);
    assert_eq!(scans[1].items[0].category, Category::Copies);
}

#[test]
fn a_folder_named_after_two_projects_goes_to_the_longer_name() {
    let mut scans = vec![
        scan_at(r"D:\p\supads", vec![listed(Category::Named, r"D:\p\supads-work-build")]),
        scan_at(r"D:\p\supads-work", vec![listed(Category::Named, r"D:\p\supads-work-build")]),
    ];
    keep_once(&mut scans, &HashSet::new());
    assert!(paths(&scans[0]).is_empty());
    assert_eq!(paths(&scans[1]), [r"D:\p\supads-work-build"]);
}

#[test]
fn a_worktree_listed_as_a_copy_elsewhere_stays_a_worktree() {
    let mut scans = vec![
        scan_at(r"D:\p\supads", vec![listed(Category::Copies, r"D:\w\clone-wt")]),
        scan_at(r"D:\x\clone", vec![listed(Category::Worktrees, r"D:\w\clone-wt")]),
    ];
    keep_once(&mut scans, &keys(&[r"D:\p\supads"]));
    assert!(paths(&scans[0]).is_empty(), "never removed as a folder: git would keep a broken registration");
    assert_eq!(paths(&scans[1]), [r"D:\w\clone-wt"]);
    assert_eq!(scans[1].items[0].category, Category::Worktrees);
}
