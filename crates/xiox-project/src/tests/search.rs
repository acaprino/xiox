//! Copies and name-matched folders: the rule, the walk limits, the identity of a repository.

use super::fixtures::*;
use crate::search::{mft_hits, name_matches_project, walk_candidates};
use crate::{Block, Category, Env, Note, Options, ProjectScan, SearchMode, Target, Why, analyze_with, resolve_project};
use xiox_disktree::{DiskIndex, Node};
use xiox_model::{ArtifactKind, Level};
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn name_rule() {
    assert!(name_matches_project("mukade", "mukade-test-scratch"));
    assert!(name_matches_project("mukade", "Mukade_bench"));
    assert!(name_matches_project("mukade", "mukade.bak"));
    assert!(name_matches_project("mukade", "mukade old"));
    assert!(!name_matches_project("mukade", "mukade"));
    assert!(!name_matches_project("mukade", "mukade-"));
    assert!(!name_matches_project("mukade", "mukadesk"));
    assert!(!name_matches_project("mukade", "my-mukade-x"));
}

/// `<base>/work/mukade`, a repository with one commit; its parent is `<base>/work`.
fn project_in(base: &Path) -> PathBuf {
    repo(&base.join("work").join("mukade"))
}

fn scan(project: &Path, roots: Vec<PathBuf>, opts: &Options) -> ProjectScan {
    let env = Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(roots),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };
    analyze_with(project, opts, &env).unwrap()
}

fn paths(scan: &ProjectScan, category: Category) -> Vec<PathBuf> {
    scan.items.iter().filter(|i| i.category == category).filter_map(|i| i.path.clone()).collect()
}

fn found(scan: &ProjectScan) -> Vec<PathBuf> {
    let mut v = paths(scan, Category::Copies);
    v.extend(paths(scan, Category::Named));
    v
}

/// `git clone` of `src` into `dest` (not checked out with line-ending conversion).
fn clone(src: &Path, dest: &Path) -> PathBuf {
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    git(src, &["clone", "-q", "-c", "core.autocrlf=false", &src.to_string_lossy(), &dest.to_string_lossy()]);
    canon(dest)
}

#[test]
fn sibling_folder_with_the_project_name_is_named_caution() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let sibling = base.join("work").join("mukade-test-scratch");
    fill(&sibling, 2, 1000);
    fill(&base.join("work").join("mukadesk"), 1, 10);
    fill(&base.join("work").join("unrelated"), 1, 10);

    let s = scan(&project, Vec::new(), &Options::default());
    let named: Vec<_> = s.items.iter().filter(|i| i.category == Category::Named).collect();
    assert_eq!(named.len(), 1, "{named:?}");
    let item = named[0];
    assert_eq!(item.path.as_deref(), Some(canon(&sibling).as_path()));
    assert_eq!(item.level, Level::Caution);
    assert_eq!(item.why, Why::NameOnly);
    assert_eq!(item.target, Target::Dir { kind: ArtifactKind::ProjectFolder });
    assert!(item.blocked.is_empty());
    assert!(item.bytes.is_some_and(|b| b >= 2000), "{:?}", item.bytes);
    assert!(paths(&s, Category::Copies).is_empty());

    let light = scan(&project, Vec::new(), &Options { measure: false, ..Options::default() });
    assert_eq!(light.items.iter().find(|i| i.category == Category::Named).unwrap().bytes, None);
}

#[test]
fn clone_elsewhere_is_a_copy_even_with_another_name() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let zzz = clone(&project, &base.join("other").join("zzz"));

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let copies: Vec<_> = s.items.iter().filter(|i| i.category == Category::Copies).collect();
    assert_eq!(copies.len(), 1, "{:?}", s.items);
    assert_eq!(copies[0].path.as_deref(), Some(zzz.as_path()));
    assert_eq!(copies[0].level, Level::Caution);
    assert_eq!(copies[0].target, Target::Dir { kind: ArtifactKind::ProjectFolder });
    assert!(copies[0].blocked.is_empty());
    assert!(
        matches!(copies[0].why, Why::SameRepository { shared_root_commit: true, same_origin: false, .. }),
        "{:?}",
        copies[0].why
    );
    assert!(paths(&s, Category::Named).is_empty());
}

#[test]
fn clone_with_an_extra_commit_has_own_work_true_and_plain_clone_false() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let plain = clone(&project, &base.join("other").join("plain"));
    let ahead = clone(&project, &base.join("other").join("ahead"));
    commit(&ahead, "extra.txt");
    let dirty = clone(&project, &base.join("other").join("dirty"));
    fs::write(dirty.join("scratch.txt"), "wip\n").unwrap();

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let own_work = |p: &Path| {
        let i = s.items.iter().find(|i| i.path.as_deref() == Some(p)).unwrap_or_else(|| panic!("{p:?} missing"));
        match &i.why {
            Why::SameRepository { has_own_work, .. } => *has_own_work,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(own_work(&plain), Some(false));
    assert_eq!(own_work(&ahead), Some(true));
    assert_eq!(own_work(&dirty), Some(true));
    // A copy with work of its own cannot be removed at all; a plain one can.
    let blocked = |p: &Path| s.items.iter().find(|i| i.path.as_deref() == Some(p)).unwrap().blocked.clone();
    assert_eq!(blocked(&plain), Vec::<Block>::new());
    assert_eq!(blocked(&ahead), vec![Block::UnsavedWork]);
    assert_eq!(blocked(&dirty), vec![Block::UnsavedWork]);
}

#[test]
fn same_origin_url_makes_a_copy_without_a_shared_root_commit() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    git(&project, &["remote", "add", "origin", "git@github.com:Someone/Mukade.git"]);
    let other = base.join("other").join("zzz");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    commit(&other, "different.txt");
    git(&other, &["remote", "add", "origin", "https://github.com/someone/mukade"]);

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let copies: Vec<_> = s.items.iter().filter(|i| i.category == Category::Copies).collect();
    assert_eq!(copies.len(), 1, "{:?}", s.items);
    assert!(matches!(
        copies[0].why,
        Why::SameRepository { shared_root_commit: false, same_origin: true, .. }
    ));
}

#[test]
fn unrelated_repository_with_a_matching_name_is_named_not_copy() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let other = base.join("work").join("mukade-other");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    commit(&other, "something-else.txt");

    let s = scan(&project, Vec::new(), &Options::default());
    assert!(paths(&s, Category::Copies).is_empty(), "{:?}", s.items);
    assert_eq!(paths(&s, Category::Named), vec![canon(&other)]);
}

#[test]
fn registered_worktree_is_never_listed_as_copy_or_named() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let wt = worktree(&project, &base.join("work").join("mukade-wt"), "wt");
    let far = worktree(&project, &base.join("other").join("zzz-wt"), "wt2");

    let s = scan(&project, vec![base.join("work"), base.join("other")], &Options::default());
    assert!(found(&s).is_empty(), "{:?}", s.items);
    let listed = paths(&s, Category::Worktrees);
    assert!(listed.contains(&wt) && listed.contains(&far), "{listed:?}");
}

#[test]
fn nothing_is_taken_from_system_folders_appdata_or_inside_another_repository() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let roots = base.join("roots");
    let clone_in = |parent: &Path| {
        fs::create_dir_all(parent).unwrap();
        clone(&project, &parent.join("zzz"));
        fs::create_dir_all(parent.join("mukade-x")).unwrap();
    };
    for system in ["$Recycle.Bin", "System Volume Information", "AppData"] {
        clone_in(&roots.join(system));
    }
    // A folder that is a repository is tested itself, never entered.
    let host = roots.join("host");
    fs::create_dir_all(&host).unwrap();
    git(&host, &["init", "-q", "-b", "main"]);
    commit(&host, "host.txt");
    clone_in(&host);
    // Control: the search does see a normal folder.
    fs::create_dir_all(roots.join("mukade-ok")).unwrap();

    let s = scan(&project, vec![roots.clone()], &Options::default());
    assert_eq!(found(&s), vec![canon(&roots.join("mukade-ok"))], "{:?}", s.items);
}

#[test]
fn junction_with_a_matching_name_is_ignored() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let roots = base.join("roots");
    let target = base.join("elsewhere").join("data");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&roots).unwrap();
    let link = roots.join("mukade-link");
    #[cfg(windows)]
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(&target, &link).is_ok();
    if !made {
        eprintln!("skipped: cannot create a junction or symlink here");
        return;
    }
    fs::create_dir_all(roots.join("mukade-real")).unwrap();

    let s = scan(&project, vec![roots.clone()], &Options::default());
    assert_eq!(found(&s), vec![canon(&roots.join("mukade-real"))], "{:?}", s.items);
    let _ = fs::remove_dir(&link);
}

#[test]
fn deeper_than_the_depth_limit_is_not_found() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let roots = base.join("roots");
    fs::create_dir_all(roots.join("a").join("b").join("mukade-depth3")).unwrap();
    fs::create_dir_all(roots.join("a").join("b").join("c").join("mukade-depth4")).unwrap();
    // The project's parent is walked one level only.
    fs::create_dir_all(base.join("work").join("sub").join("mukade-depth2")).unwrap();
    fs::create_dir_all(base.join("work").join("mukade-depth1")).unwrap();

    let s = scan(&project, vec![roots.clone()], &Options::default());
    let mut names: Vec<String> =
        found(&s).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["mukade-depth1", "mukade-depth3"]);
    assert!(s.notes.contains(&Note::WalkMayMissDeepFolders));
    assert_eq!(s.search, SearchMode::BoundedWalk);

    let p = resolve_project(&project).unwrap();
    assert_eq!(walk_candidates(&p, &[(roots.clone(), 1)]).len(), 0);
    assert_eq!(walk_candidates(&p, &[(roots, 3)]).len(), 1);
}

#[test]
fn three_letter_project_name_finds_copies_but_no_named_folders_and_says_so() {
    let (_t, base) = tmp();
    let project = repo(&base.join("work").join("abc"));
    fill(&base.join("work").join("abc-old"), 1, 10);
    let zzz = clone(&project, &base.join("other").join("zzz"));

    let s = scan(&project, vec![base.join("other")], &Options::default());
    assert_eq!(paths(&s, Category::Copies), vec![zzz], "{:?}", s.items);
    assert!(paths(&s, Category::Named).is_empty(), "{:?}", s.items);
    assert!(s.notes.contains(&Note::NameTooShort));
    // Nothing by name was searched (no index), only the walk: the mode says so.
    assert_eq!(s.search, SearchMode::BoundedWalk);
    assert!(s.notes.contains(&Note::WalkMayMissDeepFolders));

    let near = scan(&project, vec![base.join("other")], &Options { drives: false, ..Options::default() });
    assert!(paths(&near, Category::Copies).is_empty());
    assert_eq!(near.search, SearchMode::ParentOnly);
    assert!(near.notes.contains(&Note::NameTooShort));
}

#[test]
fn no_drives_option_looks_only_at_the_parent() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let near = base.join("work").join("mukade-near");
    fs::create_dir_all(&near).unwrap();
    fs::create_dir_all(base.join("work").join("sub").join("mukade-deeper")).unwrap();
    fs::create_dir_all(base.join("roots").join("mukade-far")).unwrap();

    let opts = Options { drives: false, ..Options::default() };
    let s = scan(&project, vec![base.join("roots")], &opts);
    assert_eq!(found(&s), vec![canon(&near)]);
    assert_eq!(s.search, SearchMode::ParentOnly);
    assert!(!s.notes.contains(&Note::WalkMayMissDeepFolders));
    assert!(!s.notes.contains(&Note::NameTooShort));
}

#[test]
fn a_folder_that_contains_the_project_is_never_a_candidate() {
    let (_t, base) = tmp();
    let project = repo(&base.join("mukade-archive").join("mukade"));

    let s = scan(&project, vec![base.clone()], &Options::default());
    assert!(found(&s).is_empty(), "{:?}", s.items);
}

fn dir(parent: u32, name: &str) -> Node {
    Node { parent, is_dir: true, name: Some(name.into()), ..Node::default() }
}

#[test]
fn the_mft_index_lists_folders_by_name_prefix_without_system_folders_or_the_project() {
    // 0 C:\  1 Projects  2 mukade  3 mukade-bench  4 mukadesk  5 Windows  6 Windows\mukade-sys
    // 7 Users  8 Users\x  9 AppData  10 AppData\mukade-app  11 Documents  12 Documents\Mukade_old
    // 13 mukade-archive (contains the project, below)  14 mukade-archive\mukade
    let nodes = vec![
        Node { is_dir: true, ..Node::default() },
        dir(0, "Projects"),
        dir(1, "mukade"),
        dir(1, "mukade-bench"),
        dir(1, "mukadesk"),
        dir(0, "Windows"),
        dir(5, "mukade-sys"),
        dir(0, "Users"),
        dir(7, "x"),
        dir(8, "AppData"),
        dir(9, "mukade-app"),
        dir(8, "Documents"),
        dir(11, "Mukade_old"),
        dir(0, "mukade-archive"),
        dir(13, "mukade"),
    ];
    let index = DiskIndex::from_nodes(r"C:\", nodes);
    let (_t, base) = tmp();
    let mut project = resolve_project(&project_in(&base)).unwrap();
    project.root = PathBuf::from(r"C:\mukade-archive\mukade");

    let mut got: Vec<String> = mft_hits(&index, &project, &[PathBuf::from(r"C:\")])
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    got.sort();
    assert_eq!(got, [r"C:\Projects\mukade-bench", r"C:\Users\x\Documents\Mukade_old"]);
}

#[test]
fn clone_with_another_local_branch_carrying_its_own_commit_has_own_work_true() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let c = clone(&project, &base.join("other").join("branchy"));
    git(&c, &["checkout", "-q", "-b", "side"]);
    commit(&c, "only-on-side.txt");
    git(&c, &["checkout", "-q", "main"]);
    assert_eq!(git(&c, &["status", "--porcelain"]), "");
    assert_eq!(git(&c, &["rev-parse", "HEAD"]), git(&project, &["rev-parse", "HEAD"]));

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let item = s.items.iter().find(|i| i.path.as_deref() == Some(c.as_path())).expect("copy listed");
    assert!(matches!(item.why, Why::SameRepository { has_own_work: Some(true), .. }), "{:?}", item.why);
    assert_eq!(item.blocked, vec![Block::UnsavedWork]);
}

#[test]
fn project_nested_in_an_outer_repository_lists_no_sibling() {
    let (_t, base) = tmp();
    let outer = repo(&base.join("outer"));
    let project = repo(&outer.join("vendor").join("mukade"));
    fs::create_dir_all(outer.join("vendor").join("mukade-sibling")).unwrap();
    clone(&project, &outer.join("vendor").join("zzz"));
    fs::create_dir_all(outer.join("mukade-top")).unwrap();

    let s = scan(&project, vec![outer.clone()], &Options::default());
    assert!(found(&s).is_empty(), "{:?}", s.items);
}

// ───────────────────────────── final review ─────────────────────────────

fn copy_item<'a>(s: &'a ProjectScan, p: &Path) -> &'a crate::Item {
    s.items.iter().find(|i| i.path.as_deref() == Some(p)).unwrap_or_else(|| panic!("{p:?} not listed: {:#?}", s.items))
}

#[test]
fn i1_a_clean_copy_whose_only_work_is_a_stash_is_blocked() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let copy = clone(&project, &base.join("other").join("stashy"));
    fs::write(copy.join("README.md"), "changed\n").unwrap();
    git(&copy, &["stash", "push", "-q"]);
    assert!(git(&copy, &["status", "--porcelain"]).is_empty(), "the stash leaves the copy clean");

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let i = copy_item(&s, &copy);
    assert!(matches!(i.why, Why::SameRepository { has_own_work: Some(true), .. }), "{:?}", i.why);
    assert_eq!(i.blocked, vec![Block::UnsavedWork]);
}

#[test]
fn i1_a_clean_copy_whose_only_work_is_a_tag_on_a_unique_commit_is_blocked() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let copy = clone(&project, &base.join("other").join("tagged"));
    commit(&copy, "private.txt");
    git(&copy, &["tag", "keep-me"]);
    git(&copy, &["reset", "-q", "--hard", "HEAD~1"]);

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let i = copy_item(&s, &copy);
    assert!(matches!(i.why, Why::SameRepository { has_own_work: Some(true), .. }), "{:?}", i.why);
    assert_eq!(i.blocked, vec![Block::UnsavedWork]);
}

#[test]
fn i1_a_commit_the_project_only_has_as_a_dangling_object_is_the_copys_own_work() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    // The project had a branch with one more commit, a copy took it, the project dropped the branch.
    git(&project, &["checkout", "-q", "-b", "temp"]);
    commit(&project, "temp.txt");
    git(&project, &["checkout", "-q", "main"]);
    let copy = clone(&project, &base.join("other").join("took-it"));
    git(&copy, &["fetch", "-q", &project.to_string_lossy(), "temp"]);
    git(&copy, &["checkout", "-q", "-b", "mine", "FETCH_HEAD"]);
    git(&copy, &["checkout", "-q", "main"]);
    git(&project, &["branch", "-D", "temp"]);
    // The object is still in the project's database.
    let tip = git(&copy, &["rev-parse", "mine"]);
    git(&project, &["cat-file", "-e", &tip]);

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let i = copy_item(&s, &copy);
    assert!(matches!(i.why, Why::SameRepository { has_own_work: Some(true), .. }), "{:?}", i.why);
    assert_eq!(i.blocked, vec![Block::UnsavedWork]);
}

#[test]
fn i2_the_search_runs_git_only_through_the_hardened_helper_of_dc_git() {
    // `xiox_git::git` has the timeout, the clean environment (no inherited GIT_DIR/GIT_WORK_TREE) and
    // the literal pathspecs; a second copy of that helper in the search would lose them.
    let source = include_str!("../search.rs");
    assert!(!source.contains("Command::new"), "the search must not start processes by itself");
}

/// A folder where git cannot find a repository although `.git` is there.
fn broken_repo(dir: &Path) {
    fs::create_dir_all(dir.join(".git")).unwrap();
}

#[test]
fn i3_a_named_folder_that_is_a_repository_git_cannot_read_is_blocked_and_listed() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let broken = base.join("work").join("mukade-tools");
    broken_repo(&broken);

    let s = scan(&project, Vec::new(), &Options::default());
    let i = copy_item(&s, &canon(&broken));
    assert_eq!(i.category, Category::Named);
    assert_eq!(i.blocked, vec![Block::UnsavedWorkUnknown]);
}

#[test]
fn i3_a_named_folder_that_holds_a_repository_git_cannot_read_is_blocked() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let named = base.join("work").join("mukade-old");
    broken_repo(&named.join("deep").join("inner"));

    let s = scan(&project, Vec::new(), &Options::default());
    let i = copy_item(&s, &canon(&named));
    assert_eq!(i.category, Category::Named);
    assert_eq!(i.blocked, vec![Block::UnsavedWorkUnknown]);
}

#[test]
fn i3_a_plain_named_folder_is_not_blocked() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let plain = base.join("work").join("mukade-scratch");
    fill(&plain.join("sub"), 2, 100);
    let s = scan(&project, Vec::new(), &Options::default());
    assert!(copy_item(&s, &canon(&plain)).blocked.is_empty());
}

#[test]
fn m2_a_copy_whose_own_work_cannot_be_checked_is_blocked() {
    // Same origin, no commit at all: git cannot say what the copy has that the project lacks.
    let (_t, base) = tmp();
    let project = project_in(&base);
    git(&project, &["remote", "add", "origin", "https://example.com/acme/mukade.git"]);
    let other = base.join("other").join("zzz");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    git(&other, &["remote", "add", "origin", "https://example.com/acme/mukade.git"]);

    let s = scan(&project, vec![base.join("other")], &Options::default());
    let i = copy_item(&s, &canon(&other));
    assert!(matches!(i.why, Why::SameRepository { has_own_work: None, .. }), "{:?}", i.why);
    assert_eq!(i.blocked, vec![Block::UnsavedWorkUnknown]);
}

#[test]
fn an_independent_repository_named_like_the_project_is_blocked_while_it_has_unpushed_commits_or_changes() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    // No remote: its only history is here.
    let lone = base.join("work").join("mukade-lone");
    fs::create_dir_all(&lone).unwrap();
    git(&lone, &["init", "-q", "-b", "main"]);
    commit(&lone, "x.txt");
    // Everything pushed, clean.
    let bare = base.join("remotes").join("mukade-pushed.git");
    fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    let pushed = base.join("work").join("mukade-pushed");
    fs::create_dir_all(&pushed).unwrap();
    git(&pushed, &["init", "-q", "-b", "main"]);
    commit(&pushed, "y.txt");
    git(&pushed, &["remote", "add", "origin", &bare.to_string_lossy()]);
    git(&pushed, &["push", "-q", "-u", "origin", "main"]);
    // Pushed, then edited.
    let edited = base.join("work").join("mukade-edited");
    git(&base, &["clone", "-q", &bare.to_string_lossy(), &edited.to_string_lossy()]);
    fs::write(edited.join("wip.txt"), "wip").unwrap();

    let s = scan(&project, Vec::new(), &Options::default());
    assert_eq!(copy_item(&s, &canon(&lone)).blocked, vec![Block::UnsavedWork]);
    assert_eq!(copy_item(&s, &canon(&edited)).blocked, vec![Block::UnsavedWork]);
    assert!(copy_item(&s, &canon(&pushed)).blocked.is_empty());
}

fn live_session_in(dir: &Path, live: bool) -> Vec<xiox_app::Preloaded> {
    use xiox_model::{Agent, Loaded, SessionRecord};
    vec![xiox_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: Some("s1".into()),
                project_path: dir.to_path_buf(),
                last_activity_ms: xiox_app::now_ms(),
                archived: false,
                live,
            }],
            warnings: Vec::new(),
        },
        worktrees: Vec::new(),
    }]
}

#[test]
fn i4_a_live_session_inside_a_copy_or_a_named_folder_blocks_it() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let copy = clone(&project, &base.join("other").join("zzz"));
    let named = base.join("other").join("mukade-scratch");
    fill(&named.join("src"), 1, 100);
    let named = canon(&named);

    let live = live_session_in(&named.join("src"), true);
    let live_copy = live_session_in(&copy.join("src"), true);
    let analyze = |sessions: &[xiox_app::Preloaded]| {
        let env = Env {
            sessions,
            archived: &[],
            search_roots: Some(vec![base.join("other")]),
            now_ms: xiox_app::now_ms(),
            health: Default::default(),
        };
        analyze_with(&project, &Options::default(), &env).unwrap()
    };

    let s = analyze(&live);
    assert_eq!(copy_item(&s, &named).blocked, vec![Block::LiveSession]);
    assert!(copy_item(&s, &copy).blocked.is_empty(), "the session is not in the copy");

    let s = analyze(&live_copy);
    assert_eq!(copy_item(&s, &copy).blocked, vec![Block::LiveSession]);
    assert!(copy_item(&s, &named).blocked.is_empty());

    // The same session, not live: nothing blocks.
    let s = analyze(&live_session_in(&named.join("src"), false));
    assert!(copy_item(&s, &named).blocked.is_empty());
}

#[test]
fn i4_a_copy_with_own_work_and_a_live_session_lists_both_blocks() {
    let (_t, base) = tmp();
    let project = project_in(&base);
    let copy = clone(&project, &base.join("other").join("zzz"));
    commit(&copy, "private.txt");
    let sessions = live_session_in(&copy, true);
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(vec![base.join("other")]),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };
    let s = analyze_with(&project, &Options::default(), &env).unwrap();
    let blocked = &copy_item(&s, &copy).blocked;
    assert!(blocked.contains(&Block::UnsavedWork) && blocked.contains(&Block::LiveSession), "{blocked:?}");
}
