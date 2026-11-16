//! Copies found from the working directories of listed sessions: scope, deduplication, provenance.

use std::fs;
use std::path::{Path, PathBuf};

use xiox_model::{path_key, Agent, Loaded};
use xiox_mukade::{Descriptor, parse_time_ms};

use super::fake::{descriptor, input, listing};
use crate::history::scope::local_dir;
use crate::history::UserDirs;
use crate::tests::fixtures::*;
use crate::*;

#[test]
fn local_dir_table() {
    let table: [(&str, Result<&str, Exclusion>); 7] = [
        (r"\\?\C:\w\a", Ok(r"C:\w\a")),
        ("/d/x", Err(Exclusion::NotLocal)),
        ("D:/x/y", Ok(r"D:\x\y")),
        (r"\\srv\share\x", Err(Exclusion::NotLocal)),
        (r"\\.\pipe\x", Err(Exclusion::NotLocal)),
        (r"rel\x", Err(Exclusion::NotLocal)),
        (r"D:\a\..\b", Err(Exclusion::NotLocal)),
    ];
    for (raw, expected) in table {
        assert_eq!(local_dir(raw), expected.map(PathBuf::from), "{raw}");
    }
}

/// `<base>/work/mukade`: a repository with one commit, its parent being `<base>/work`.
fn project_in(base: &Path) -> PathBuf {
    repo(&base.join("work").join("mukade"))
}

fn scan(project: &Path, opts: &Options, roots: Vec<PathBuf>, listings: &[(Agent, Loaded<Descriptor>)]) -> ProjectScan {
    let user = UserDirs::default();
    let env = Env { search_roots: Some(roots), history: input(listings, None, &user), ..env() };
    crate::analyze_with(project, opts, &env).unwrap()
}

fn copies(scan: &ProjectScan) -> Vec<&Item> {
    scan.items.iter().filter(|i| matches!(i.category, Category::Copies | Category::Named)).collect()
}

fn cwd(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn deep_copy_in_a_session_cwd_is_a_copy_with_provenance() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    // Five levels below the base: out of reach of the walk (depth 3).
    let deep = clone(&project, &base.join("a").join("b").join("c").join("d").join("e"));
    let src = deep.join("src");
    fs::create_dir_all(&src).unwrap();
    let in_src = cwd(&src);
    let listings = [
        listing(
            Agent::Claude,
            vec![
                descriptor(Agent::Claude, "main", "s1", &in_src, "2026-11-10T10:00:00Z", None),
                descriptor(Agent::Claude, "main", "s2", &in_src, "2026-11-12T10:00:00Z", None),
            ],
        ),
        listing(Agent::Codex, vec![descriptor(Agent::Codex, "main", "s3", &in_src, "2026-11-11T10:00:00Z", None)]),
    ];

    let s = scan(&project, &Options::default(), vec![base.clone()], &listings);

    let found = copies(&s);
    assert_eq!(found.len(), 1, "{found:?}");
    let item = found[0];
    assert_eq!((item.category, &item.key), (Category::Copies, &path_key(&deep)));
    assert!(matches!(item.why, Why::SameRepository { shared_root_commit: true, .. }), "{:?}", item.why);
    let provenance = item.provenance.as_ref().expect("a copy found by a session has provenance");
    assert_eq!(provenance.sessions.len(), 3);
    for session in &provenance.sessions {
        assert!(session.listed_cwd, "{session:?}");
        assert_eq!(session.strength, Some(Strength::Used));
    }
    assert_eq!(provenance.last_activity_ms, parse_time_ms("2026-11-12T10:00:00Z"));
    assert!(s.history.is_some());
}

#[test]
fn cwd_copy_found_by_walk_is_listed_once() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let sibling = clone(&project, &base.join("work").join("zzz"));
    // The same folder spelled as a verbatim path, lowercase, with forward slashes.
    let spelled = format!(r"\\?\{}", cwd(&sibling).to_lowercase().replace('\\', "/"));
    let listings = [listing(
        Agent::Claude,
        vec![descriptor(Agent::Claude, "main", "s1", &spelled, "2026-11-10T10:00:00Z", None)],
    )];

    let s = scan(&project, &Options::default(), Vec::new(), &listings);

    let keys: Vec<&String> = copies(&s).iter().map(|i| &i.key).collect();
    assert_eq!(keys, vec![&path_key(&sibling)]);
    assert!(copies(&s)[0].provenance.is_some());
}

#[test]
fn no_drives_keeps_session_roots_next_to_the_project() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let far = clone(&project, &base.join("far").join("copy"));
    let ghost = base.join("far").join("ghost");
    let listings = [listing(
        Agent::Claude,
        vec![
            descriptor(Agent::Claude, "main", "s1", &cwd(&far), "2026-11-10T10:00:00Z", None),
            descriptor(Agent::Claude, "main", "s2", &cwd(&ghost), "2026-11-10T10:00:00Z", None),
        ],
    )];
    let opts = Options { drives: false, ..Options::default() };

    let s = scan(&project, &opts, Vec::new(), &listings);

    assert!(copies(&s).is_empty());
    let excluded = &s.history.as_ref().unwrap().excluded;
    assert_eq!(excluded.get(&Exclusion::OutsideScope), Some(&2));
    // Nothing was looked at on disk: the missing folder was not even found missing.
    assert!(!excluded.contains_key(&Exclusion::Missing), "{excluded:?}");
}

#[test]
fn unrelated_repository_in_a_cwd_is_not_listed() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    // Beyond the walk's reach, with a root commit and no origin of its own.
    let other = base.join("elsewhere").join("a").join("b").join("c").join("tool");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    commit(&other, "tool.txt");
    let listings = [listing(
        Agent::Claude,
        vec![descriptor(Agent::Claude, "main", "s1", &cwd(&other), "2026-11-10T10:00:00Z", None)],
    )];

    let s = scan(&project, &Options::default(), vec![base.join("elsewhere")], &listings);

    assert!(copies(&s).is_empty(), "{:?}", copies(&s));
}

#[test]
fn session_inside_the_project_counts_as_inside() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let src = project.join("src");
    fs::create_dir_all(&src).unwrap();
    let listings = [listing(
        Agent::Claude,
        vec![descriptor(Agent::Claude, "main", "s1", &cwd(&src), "2026-11-10T10:00:00Z", None)],
    )];

    let s = scan(&project, &Options::default(), Vec::new(), &listings);

    assert!(copies(&s).is_empty());
    assert_eq!(s.history.as_ref().unwrap().excluded.get(&Exclusion::InsideProject), Some(&1));
}

#[test]
fn repository_above_a_session_folder_outside_the_scope_is_dropped_and_counted() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    // The session folder lies inside the scope (the project's parent), the repository above it does not.
    git(&base, &["init", "-q", "-b", "main"]);
    commit(&base, "base.txt");
    let tools = base.join("work").join("tools");
    fs::create_dir_all(&tools).unwrap();
    let listings = [listing(
        Agent::Claude,
        vec![descriptor(Agent::Claude, "main", "s1", &cwd(&tools), "2026-11-10T10:00:00Z", None)],
    )];
    let opts = Options { drives: false, ..Options::default() };

    let s = scan(&project, &opts, Vec::new(), &listings);

    assert!(copies(&s).is_empty(), "{:?}", copies(&s));
    assert_eq!(s.history.as_ref().unwrap().excluded.get(&Exclusion::OutsideScope), Some(&1));
}

#[test]
fn session_in_the_project_or_a_worktree_counts_as_inside() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let tree = worktree(&project, &base.join("work").join("tree"), "side");
    let table = [project.clone(), tree.clone(), tree.join("src")];
    for dir in table {
        let listings = [listing(
            Agent::Claude,
            vec![descriptor(Agent::Claude, "main", "s1", &cwd(&dir), "2026-11-10T10:00:00Z", None)],
        )];

        let s = scan(&project, &Options::default(), Vec::new(), &listings);

        assert!(copies(&s).is_empty(), "{dir:?}");
        assert_eq!(s.history.as_ref().unwrap().excluded.get(&Exclusion::InsideProject), Some(&1), "{dir:?}");
    }
}

#[test]
fn non_local_working_directories_are_counted_through_the_listing() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let listings = [listing(
        Agent::Claude,
        vec![
            descriptor(Agent::Claude, "main", "s1", "/d/x", "2026-11-10T10:00:00Z", None),
            descriptor(Agent::Claude, "main", "s2", r"\srv\share\x", "2026-11-10T10:00:00Z", None),
        ],
    )];

    let s = scan(&project, &Options::default(), Vec::new(), &listings);

    assert_eq!(s.history.as_ref().unwrap().excluded.get(&Exclusion::NotLocal), Some(&2));
}
