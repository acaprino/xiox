//! Log and temporary files, and the stale branch list.

use super::fixtures::*;
use crate::{Category, Item, Note, Options, ProjectScan, Target, Why, analyze_with, resolve_project};
use dc_model::Level;
use std::fs;
use std::path::Path;

fn scan(project: &Path) -> ProjectScan {
    analyze_with(project, &Options::default(), &env()).unwrap()
}

fn logs(scan: &ProjectScan) -> Vec<&Item> {
    scan.items.iter().filter(|i| i.category == Category::Logs).collect()
}

fn write_sized(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

fn names(items: &[&Item]) -> Vec<String> {
    let mut v: Vec<String> = items
        .iter()
        .map(|i| i.path.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn ignored_big_log_is_recommended_and_untracked_big_tmp_is_caution() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    fs::write(p.join(".gitignore"), "*.log\n").unwrap();
    git(&p, &["add", ".gitignore"]);
    git(&p, &["commit", "-q", "-m", "ignore logs"]);
    write_sized(&p.join("logs").join("app.LOG"), 2 * MB);
    write_sized(&p.join("scratch.tmp"), 2 * MB);
    write_sized(&p.join("notes.txt"), 2 * MB);

    let s = scan(&p);
    let l = logs(&s);
    assert_eq!(names(&l), vec!["app.LOG", "scratch.tmp"], "{l:?}");
    let log = l.iter().find(|i| i.path.as_ref().unwrap().ends_with("app.LOG")).unwrap();
    assert_eq!(log.level, Level::Recommended);
    assert_eq!(log.why, Why::LogFile { ignored: true });
    assert_eq!(log.target, Target::File);
    assert_eq!(log.bytes, Some(2 * MB as u64));
    assert!(log.blocked.is_empty());
    let tmp = l.iter().find(|i| i.path.as_ref().unwrap().ends_with("scratch.tmp")).unwrap();
    assert_eq!(tmp.level, Level::Caution);
    assert_eq!(tmp.why, Why::LogFile { ignored: false });
    assert_eq!(tmp.target, Target::File);
}

#[test]
fn tracked_log_and_small_log_are_not_listed() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    write_sized(&p.join("tracked.log"), 3 * MB);
    git(&p, &["add", "tracked.log"]);
    git(&p, &["commit", "-q", "-m", "track a log"]);
    write_sized(&p.join("small.log"), MB - 1);
    write_sized(&p.join("exact.log"), MB);

    let s = scan(&p);
    assert_eq!(names(&logs(&s)), vec!["exact.log"], "{:?}", logs(&s));
}

#[test]
fn log_inside_a_build_folder_is_not_listed_twice() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    fs::write(p.join(".gitignore"), "target/\n").unwrap();
    git(&p, &["add", ".gitignore"]);
    git(&p, &["commit", "-q", "-m", "ignore target"]);
    let target = rust_build(&p, MB);
    write_sized(&target.join("x.log"), 2 * MB);
    write_sized(&p.join("outside.log"), 2 * MB);

    let s = scan(&p);
    assert!(s.items.iter().any(|i| i.category == Category::Builds && i.path.as_deref() == Some(target.as_path())));
    assert_eq!(names(&logs(&s)), vec!["outside.log"], "{:?}", logs(&s));
}

#[test]
fn log_in_a_linked_worktree_is_listed() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    let wt = worktree(&p, &base.join("proj-feat"), "feat");
    write_sized(&wt.join("debug.dmp"), 2 * MB);

    let s = scan(&p);
    let l = logs(&s);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].path.as_deref(), Some(wt.join("debug.dmp").as_path()));
    assert_eq!(l[0].level, Level::Caution);
}

#[test]
fn plain_folder_has_no_log_items() {
    let (_t, base) = tmp();
    let p = base.join("plain");
    write_sized(&p.join("big.log"), 2 * MB);
    let project = resolve_project(&p).unwrap();
    assert!(!project.is_repo);

    let s = scan(&p);
    assert!(logs(&s).is_empty());
}

#[test]
fn merged_branch_is_reported_as_stale_and_unmerged_is_not() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    git(&p, &["branch", "merged-b"]);
    git(&p, &["checkout", "-q", "-b", "unmerged-b"]);
    commit(&p, "extra.txt");
    git(&p, &["checkout", "-q", "main"]);
    // Checked out in a live linked worktree: never reported as "worktree missing".
    worktree(&p, &base.join("proj-live"), "live-b");

    let s = scan(&p);
    let merged = s.branches.iter().find(|b| b.name == "merged-b").expect("merged branch listed");
    assert!(merged.merged && !merged.worktree_missing);
    assert!(s.branches.iter().all(|b| b.name != "unmerged-b"), "{:?}", s.branches);
    assert!(s.branches.iter().all(|b| !b.worktree_missing), "{:?}", s.branches);
}

#[test]
fn m5_a_root_git_cannot_list_is_a_note_not_a_silent_nothing() {
    let (_t, base) = tmp();
    let p = repo(&base.join("proj"));
    let wt = worktree(&p, &base.join("proj-wt"), "feature");
    let s = scan(&p);
    assert!(!s.notes.contains(&Note::LogsNotChecked), "{:?}", s.notes);

    // The worktree folder is there, but git cannot read it any more.
    fs::write(wt.join(".git"), "gitdir: nowhere\n").unwrap();
    let s = scan(&p);
    assert!(s.items.iter().any(|i| i.category == Category::Worktrees), "{:?}", s.items);
    assert!(s.notes.contains(&Note::LogsNotChecked), "{:?}", s.notes);
    assert_eq!(s.notes.iter().filter(|n| **n == Note::LogsNotChecked).count(), 1, "once, not per root");
}
