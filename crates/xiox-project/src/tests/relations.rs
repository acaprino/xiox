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
    fs::create_dir_all(proj.join("target")).unwrap();
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
    let scan = analyze_relations(&w.proj, &[w.copy.clone()], &input()).unwrap();
    let req = CleanRequest { categories: vec![], include: vec![w.copy.clone()], confirm_danger: vec![], mode: RemoveMode::Recycle };
    let chosen = select(&scan, &req).unwrap();
    let out = clean_without_sessions(&scan, &chosen, RemoveMode::Recycle, true, &[]);
    let item = out.items.iter().find(|o| o.path.as_ref() == Some(&w.copy)).unwrap();
    assert!(!matches!(item.outcome, Outcome::Skipped { .. }), "the copy is found again, not skipped as changed: {:?}", item.outcome);
    assert!(w.copy.exists(), "a dry run removes nothing");
}
