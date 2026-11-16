//! Session folders inside the project: a Git check of the subtree instead of `inside-project`.

use std::collections::BTreeMap;
use std::fs;

use super::scratch::{Do::*, Site};
use crate::tests::fixtures::*;
use crate::*;

/// What the session leaves inside the project, and what comes out of it.
enum Expect {
    Item(&'static str, Vec<Block>),
    Excluded(Exclusion),
}

#[test]
fn session_folders_inside_the_project_get_the_subtree_policy() {
    let table: Vec<(&str, Vec<super::scratch::Do>, fn(&Site), Expect)> = vec![
        ("an untracked folder", vec![Mkdir("work/proj/tmp/run1")], |_| {}, Expect::Item("work/proj/tmp/run1", vec![])),
        (
            "a folder with a committed file",
            vec![Mkdir("work/proj/bench")],
            |s| commit(&s.project.root, "bench/a.txt"),
            Expect::Item("work/proj/bench", vec![Block::TrackedFiles]),
        ),
        (
            "a folder holding a repository",
            vec![Write("work/proj/vendor/lib/x.txt")],
            |s| {
                repo(&s.project.root.join("vendor").join("lib").join("inner"));
            },
            Expect::Item("work/proj/vendor/lib", vec![Block::HoldsRepositories]),
        ),
        ("a file in the project itself", vec![Write("work/proj/notes.txt")], |_| {}, Expect::Excluded(Exclusion::InsideProject)),
        ("the git folder", vec![Write("work/proj/.git/x/a.txt")], |_| {}, Expect::Excluded(Exclusion::Protected)),
        (
            "a folder holding a worktree",
            vec![Write("work/proj/wt-parent/x.txt")],
            |s| {
                worktree(&s.project.root, &s.project.root.join("wt-parent").join("wt2"), "wt2");
            },
            Expect::Excluded(Exclusion::Protected),
        ),
    ];
    for (name, session, prepare, expect) in table {
        let mut site = Site::new();
        prepare(&site);
        site.project = resolve_project(&site.project.root).unwrap();
        for d in &session {
            let rel = match d {
                Mkdir(r) => *r,
                Write(r) => std::path::Path::new(r).parent().unwrap().to_str().unwrap(),
                _ => unreachable!(),
            };
            fs::create_dir_all(site.at(rel)).unwrap();
        }
        let (items, excluded) = site.within_base(&session);
        match expect {
            Expect::Item(folder, blocked) => {
                assert_eq!(items.len(), 1, "{name}: {excluded:?}");
                let item = &items[0];
                assert_eq!(item.key, xiox_model::path_key(&site.at(folder)), "{name}");
                assert_eq!(item.category, Category::Scratch, "{name}");
                assert_eq!(item.target, Target::Subtree { repo: site.project.root.clone() }, "{name}");
                assert_eq!(item.blocked, blocked, "{name}");
            }
            Expect::Excluded(why) => {
                assert!(items.is_empty(), "{name}: {items:?}");
                assert_eq!(excluded, BTreeMap::from([(why, 1)]), "{name}");
            }
        }
    }
}

#[test]
fn a_subtree_git_cannot_read_is_blocked_as_unknown() {
    let site = Site::new();
    fs::create_dir_all(site.at("work/proj/tmp/run2")).unwrap();
    fs::write(site.project.root.join(".git").join("HEAD"), "not a ref").unwrap();
    let (items, excluded) = site.within_base(&[Mkdir("work/proj/tmp/run2")]);
    assert_eq!(items.len(), 1, "{excluded:?}");
    assert!(items[0].blocked.contains(&Block::UnsavedWorkUnknown), "{:?}", items[0].blocked);
}
