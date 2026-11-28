//! The index of the PC's repositories: grouping by identity, the reference of a group, the bounded walk.

use super::fixtures::*;
use crate::repo_index::groups;
use crate::{RepoIndex, repositories_under};
use std::fs;
use std::path::Path;

fn id(roots: &[&str], origin: Option<&str>) -> (Vec<String>, Option<String>) {
    (roots.iter().map(|s| s.to_string()).collect(), origin.map(str::to_string))
}

#[test]
fn groups_join_by_root_commit_or_origin_transitively() {
    let ids = vec![
        id(&["r1"], None),             // 0: A
        id(&["r1", "r2"], Some("gh/x")), // 1: B, joined to A by r1
        id(&["r9"], Some("gh/x")),      // 2: C, joined to B by the origin
        id(&["r5"], None),             // 3: D, alone
    ];
    assert_eq!(groups(&ids), vec![vec![0, 1, 2]]);
}

#[test]
fn same_origin_alone_makes_a_group() {
    let ids = vec![id(&["a"], Some("gh/fork")), id(&["b"], Some("gh/fork"))];
    assert_eq!(groups(&ids), vec![vec![0, 1]]);
}

#[test]
fn no_commit_and_no_origin_joins_nothing() {
    let ids = vec![id(&[], None), id(&[], None), id(&[""], Some(""))];
    assert!(groups(&ids).is_empty());
}

#[test]
fn build_groups_a_clone_and_leaves_an_unrelated_repository_alone() {
    let (_t, base) = tmp();
    let proj = repo(&base.join("proj"));
    let zzz = clone(&proj, &base.join("elsewhere").join("zzz"));
    // Not `repo()`: its fixed first commit would be the same commit in every repository it makes.
    let other = base.join("other");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    commit(&other, "OTHER.md");
    let other = canon(&other);

    let index = RepoIndex::build(&[proj.clone(), zzz.clone(), other.clone(), proj.clone()]);
    assert_eq!(index.repos.len(), 3, "the double of proj is dropped: {:?}", index.repos);
    let group = index.group_of(&proj).expect("proj and its clone are one group");
    let roots: Vec<&Path> = group.iter().map(|&i| index.repos[i].root.as_path()).collect();
    assert_eq!(roots.len(), 2);
    assert!(roots.contains(&proj.as_path()) && roots.contains(&zzz.as_path()), "{roots:?}");
    assert!(index.group_of(&other).is_none());
}

#[test]
fn a_folder_that_is_not_a_repository_does_not_enter() {
    let (_t, base) = tmp();
    let plain = base.join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert!(RepoIndex::build(&[plain]).repos.is_empty());
}

#[test]
fn a_repository_on_a_network_share_does_not_enter() {
    let (_t, base) = tmp();
    let cand = base.join("cand");
    fs::create_dir_all(&cand).unwrap();
    fs::write(cand.join(".git"), "gitdir: \\\\127.0.0.1\\share\\x.git\n").unwrap();
    assert!(RepoIndex::build(&[cand]).repos.is_empty(), "git is not asked about a repository it must not read");
}

#[test]
fn reference_is_the_most_recent_and_ties_go_to_the_first_path() {
    let (_t, base) = tmp();
    let a = repo(&base.join("a"));
    let b = clone(&a, &base.join("b"));
    let index = RepoIndex::build(&[a.clone(), b.clone()]);
    let group = index.group_of(&a).unwrap().to_vec();
    let root = |i: usize| index.repos[i].root.clone();

    let newer_b = |p: &Path| Some(if p == b.as_path() { 2 } else { 1 });
    assert_eq!(root(index.reference(&group, &newer_b)), b);
    let none = |_: &Path| None;
    assert_eq!(root(index.reference(&group, &none)), a, "no activity: the first by path");
    let tie = |_: &Path| Some(5);
    assert_eq!(root(index.reference(&group, &tie)), a, "a tie: the first by path");
}

#[test]
fn repositories_under_stops_at_a_repository_and_at_the_depth() {
    let (_t, base) = tmp();
    let a = repo(&base.join("a"));
    repo(&a.join("inner"));
    repo(&base.join("x").join("y").join("z").join("deep"));
    fs::create_dir_all(base.join("AppData").join("r")).unwrap();
    repo(&base.join("AppData").join("r").join("hidden"));

    assert_eq!(repositories_under(&[(base.clone(), 3)]), vec![a.clone()]);
    assert_eq!(repositories_under(&[(a.clone(), 3)]), vec![a], "a root that is a repository is listed itself");
}
