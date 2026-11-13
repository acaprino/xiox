use super::fixtures::*;
use crate::*;
use xiox_model::{Agent, ArtifactKind, Level, Loaded, SessionRecord, path_key};
use xiox_mukade::ArchivedSession;
use std::fs;

fn of(scan: &ProjectScan, cat: Category) -> Vec<&Item> {
    scan.items.iter().filter(|i| i.category == cat).collect()
}

fn item<'a>(scan: &'a ProjectScan, path: &std::path::Path) -> &'a Item {
    let k = path_key(path);
    scan.items.iter().find(|i| i.key == k).unwrap_or_else(|| panic!("no item {k} in {:#?}", scan.items))
}

fn archived(agent: Agent, id: &str, cwd: Option<&std::path::Path>, blocked: bool) -> ArchivedSession {
    ArchivedSession {
        agent,
        id: id.to_string(),
        title: Some(format!("title {id}")),
        cwd: cwd.map(|p| p.to_path_buf()),
        updated_ms: Some(1),
        file: None,
        bytes: 1234,
        blocked: blocked.then(|| "bloccata".to_string()),
    }
}

#[test]
fn subfolder_and_linked_worktree_resolve_to_the_main_repository() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    commit(&root, "src/lib.rs");
    let wt = worktree(&root, &base.join("proj-wt"), "feature");

    for p in [root.join("src"), wt.clone(), wt.join("src")] {
        let pr = resolve_project(&p).unwrap();
        assert_eq!(path_key(&pr.root), path_key(&root), "from {}", p.display());
        assert_eq!(pr.name, "proj");
        assert!(pr.is_repo);
        assert_eq!(pr.worktrees.iter().map(|w| path_key(w)).collect::<Vec<_>>(), vec![path_key(&wt)]);
    }
}

#[test]
fn linked_worktree_count_tells_none_from_a_git_failure() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    // A repository with no linked worktree: git answered, the count is 0 (not a failure).
    assert_eq!(linked_worktree_count(&root), Some(0));
    worktree(&root, &base.join("proj-wt"), "feature");
    assert_eq!(linked_worktree_count(&root), Some(1));
    // Not a repository: git fails, and that is not "none".
    let plain = base.join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(linked_worktree_count(&plain), None);
}

#[test]
fn plain_folder_is_its_own_project_with_a_not_a_repository_note() {
    let (_t, base) = tmp();
    let dir = base.join("plain");
    fs::create_dir_all(&dir).unwrap();

    let pr = resolve_project(&dir).unwrap();
    assert_eq!(path_key(&pr.root), path_key(&dir));
    assert_eq!(pr.name, "plain");
    assert!(!pr.is_repo);
    assert!(pr.worktrees.is_empty());
    assert_eq!(pr.origin, None);

    let scan = analyze_with(&dir, &Options::default(), &env()).unwrap();
    assert!(scan.notes.contains(&Note::NotARepository), "{:?}", scan.notes);
    assert_eq!(scan.schema, 1);
}

#[test]
fn missing_path_is_path_not_found() {
    let (_t, base) = tmp();
    let p = base.join("nope");
    assert!(matches!(resolve_project(&p), Err(Error::PathNotFound(_))));
    assert!(matches!(analyze_with(&p, &Options::default(), &env()), Err(Error::PathNotFound(_))));
    let f = base.join("file.txt");
    fs::write(&f, "x").unwrap();
    assert!(matches!(resolve_project(&f), Err(Error::NotADirectory(_))));
    assert!(Error::PathNotFound(p).to_string().starts_with("path not found"));
}

#[test]
fn rust_target_and_node_modules_are_builds_recommended_even_if_the_project_was_used_today() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let nm = node_modules(&root, 2 * MB);
    let now = xiox_app::now_ms();
    let sessions = vec![xiox_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: Some("s1".into()),
                project_path: root.clone(),
                last_activity_ms: now,
                archived: false,
                live: false,
            }],
            warnings: Vec::new(),
        },
        worktrees: Vec::new(),
    }];
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: now,
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let t = item(&scan, &target);
    assert_eq!(t.category, Category::Builds);
    assert_eq!(t.level, Level::Recommended);
    assert!(t.blocked.is_empty());
    assert_eq!(t.target, Target::Dir { kind: ArtifactKind::RustTarget });
    assert_eq!(t.why, Why::CatalogFolder { label_en: "Rust build".into() });
    assert!(t.bytes.unwrap() >= 2 * MB as u64);
    // node_modules keeps the engine's level for a dependency folder (Optional), not Caution.
    let n = item(&scan, &nm);
    assert_eq!(n.category, Category::Builds);
    assert_eq!(n.level, Level::Optional);
    assert!(n.blocked.is_empty());
}

#[test]
fn builds_inside_a_linked_worktree_are_listed_too() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let target = rust_build(&wt, 2 * MB);

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    let t = item(&scan, &target);
    assert_eq!(t.category, Category::Builds);
    assert_eq!(t.level, Level::Recommended);
}

#[test]
fn clean_worktree_at_a_commit_main_has_is_recommended_and_not_blocked() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    let w = item(&scan, &wt);
    assert_eq!(w.category, Category::Worktrees);
    assert_eq!(w.level, Level::Recommended);
    assert!(w.blocked.is_empty(), "{:?}", w.blocked);
    assert_eq!(w.target, Target::Worktree { repo: root.clone() });
    match &w.why {
        Why::LinkedWorktree { branch, head } => {
            assert_eq!(branch.as_deref(), Some("feature"));
            assert_eq!(head.as_deref(), Some(git(&root, &["rev-parse", "HEAD"]).as_str()));
        }
        other => panic!("{other:?}"),
    }
    assert!(w.bytes.is_some());
    // The main working tree is never an item.
    assert!(scan.items.iter().all(|i| i.key != path_key(&root)));
}

#[test]
fn dirty_worktree_is_blocked_dirty_git() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    fs::write(wt.join("new.txt"), "work in progress").unwrap();

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    let w = item(&scan, &wt);
    assert!(w.blocked.contains(&Block::DirtyGit), "{:?}", w.blocked);
    assert!(!w.blocked.contains(&Block::UnpushedCommits), "{:?}", w.blocked);
    assert_eq!(w.level, Level::Caution);
}

#[test]
fn worktree_with_a_commit_nobody_else_has_is_blocked_unpushed_commits() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    commit(&wt, "only-here.txt");

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    let w = item(&scan, &wt);
    assert_eq!(w.blocked, vec![Block::UnpushedCommits]);
    assert_eq!(w.level, Level::Caution);
}

#[test]
fn deleted_worktree_folder_gives_a_prunable_note_and_no_item() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    fs::remove_dir_all(&wt).unwrap();

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    assert!(of(&scan, Category::Worktrees).is_empty(), "{:?}", scan.items);
    assert!(
        scan.notes.iter().any(|n| matches!(n, Note::PrunableWorktree { path } if path_key(path) == path_key(&wt))),
        "{:?}",
        scan.notes
    );
    assert!(scan.project.worktrees.is_empty());
}

#[test]
fn archived_session_started_in_a_worktree_belongs_to_the_project_and_one_elsewhere_does_not() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let other = base.join("proj-other");
    fs::create_dir_all(&other).unwrap();
    let archived = vec![
        archived(Agent::Codex, "in-wt", Some(&wt.join("src")), false),
        archived(Agent::Claude, "in-root", Some(&root), true),
        archived(Agent::Claude, "elsewhere", Some(&other), false),
        archived(Agent::Pi, "no-cwd", None, false),
    ];
    let env = Env {
        sessions: &[],
        archived: &archived,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let s = of(&scan, Category::Sessions);
    let keys: Vec<&str> = s.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(s.len(), 2, "{keys:?}");
    let wt_s = s.iter().find(|i| i.key == "session:codex:in-wt").expect("worktree session");
    assert_eq!(wt_s.path, None);
    assert_eq!(wt_s.bytes, Some(1234));
    assert_eq!(wt_s.level, Level::Optional);
    assert!(wt_s.blocked.is_empty());
    assert_eq!(wt_s.target, Target::Session { agent: Agent::Codex, id: "in-wt".into() });
    assert_eq!(wt_s.why, Why::ArchivedSession { agent: Agent::Codex, title: Some("title in-wt".into()) });
    let root_s = s.iter().find(|i| i.key == "session:claude:in-root").expect("root session");
    assert_eq!(root_s.blocked, vec![Block::SessionLocked]);
}

#[test]
fn light_mode_measures_nothing() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    rust_build(&root, 2 * MB);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let archived = vec![archived(Agent::Claude, "a", Some(&root), false)];
    let env = Env {
        sessions: &[],
        archived: &archived,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let opts = Options { measure: false, ..Options::default() };
    let scan = analyze_with(&root, &opts, &env).unwrap();
    assert!(!of(&scan, Category::Builds).is_empty());
    assert!(scan.items.iter().any(|i| i.key == path_key(&wt)));
    for i in &scan.items {
        if i.category == Category::Sessions {
            assert_eq!(i.bytes, Some(1234));
        } else {
            assert_eq!(i.bytes, None, "{}", i.key);
        }
    }
}

#[test]
fn items_are_ordered_by_category_then_size() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let small = node_modules(&root, 2 * MB);
    let big = rust_build(&root, 4 * MB);
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let wt_build = rust_build(&wt, 3 * MB);
    let archived = vec![archived(Agent::Claude, "a", Some(&root), false)];
    let env = Env {
        sessions: &[],
        archived: &archived,
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let keys: Vec<&str> = scan.items.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            path_key(&big).as_str(),
            path_key(&wt_build).as_str(),
            path_key(&small).as_str(),
            path_key(&wt).as_str(),
            "session:claude:a",
        ]
    );
    let json = serde_json::to_value(&scan).unwrap();
    assert_eq!(json["items"][0]["category"], "builds");
    assert_eq!(json["items"][0]["why"]["type"], "catalog-folder");
    assert_eq!(json["items"][0]["why"]["labelEn"], "Rust build");
    assert!(json["scannedAtMs"].is_i64());
}

// ───────────────────────────── blocks and levels ─────────────────────────────

/// One Claude agent with one session in `dir`, live or not, active now.
fn sessions_in(dir: &std::path::Path, live: bool) -> Vec<xiox_app::Preloaded> {
    vec![xiox_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: Some("live".into()),
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
fn worktree_with_a_live_session_inside_it_is_blocked_live_session() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    // No build folder: the engine has no report for this worktree, the session record decides.
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    let sessions = sessions_in(&wt.join("src"), true);
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let w = item(&scan, &wt);
    assert_eq!(w.blocked, vec![Block::LiveSession]);
    assert_eq!(w.level, Level::Caution);

    // The same session, not live: nothing blocks.
    let sessions = sessions_in(&wt.join("src"), false);
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };
    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let w = item(&scan, &wt);
    assert!(w.blocked.is_empty(), "{:?}", w.blocked);
    assert_eq!(w.level, Level::Recommended);
}

#[test]
fn build_of_a_project_with_a_live_session_is_blocked_live_session_and_keeps_its_level() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    let sessions = sessions_in(&root, true);
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let t = item(&scan, &target);
    assert_eq!(t.blocked, vec![Block::LiveSession]);
    assert_eq!(t.level, Level::Recommended);
}

#[test]
fn build_of_an_unreachable_report_is_blocked_unreachable() {
    // The engine never emits `Unreachable` today, so the report is built by hand.
    let (_t, base) = tmp();
    let root = base.join("proj");
    let path = root.join("target");
    let report = xiox_model::ProjectReport {
        id: path_key(&root),
        root: root.clone(),
        name: "proj".into(),
        ecosystems: vec!["rust".into()],
        agents: Vec::new(),
        last_activity_ms: None,
        last_activity_source: None,
        inactive_days: None,
        status: xiox_model::ProjectStatus::Unreachable,
        blocked_by: vec![xiox_model::BlockReason::Unreachable],
        reason: String::new(),
        artifacts: vec![xiox_model::Artifact {
            id: path_key(&path),
            path: path.clone(),
            kind: ArtifactKind::RustTarget,
            label: String::new(),
            size: xiox_model::SizeInfo { logical_bytes: 10, allocated_bytes: Some(4096), file_count: 1 },
            verified: true,
            class: xiox_model::Class::Build,
            level: Level::Recommended,
            note: String::new(),
            ambiguous: false,
            preselected: false,
        }],
        reclaimable_bytes: 4096,
        preselected: false,
        is_agent_worktree: false,
        git: Default::default(),
    };
    let project = Project { root, name: "proj".into(), is_repo: false, origin: None, worktrees: Vec::new() };

    let items = crate::owned::builds(&project, &[report], &Options::default());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].blocked, vec![Block::Unreachable]);
    assert_eq!(items[0].level, Level::Recommended);
    assert_eq!(items[0].bytes, Some(4096));
}

#[test]
fn build_folder_tracked_by_git_is_caution() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let target = rust_build(&root, 2 * MB);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "commit the build folder"]);

    let scan = analyze_with(&root, &Options::default(), &env()).unwrap();
    let t = item(&scan, &target);
    assert_eq!(t.category, Category::Builds);
    assert_eq!(t.level, Level::Caution);
    assert!(t.blocked.is_empty(), "{:?}", t.blocked);
}

#[test]
fn build_inside_a_worktree_nested_under_the_root_is_listed_once_with_the_live_session_block() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    fs::write(root.join(".gitignore"), ".claude/\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "ignore .claude"]);
    let wt = worktree(&root, &root.join(".claude").join("worktrees").join("agent-1"), "agent-1");
    let wt_build = rust_build(&wt, 2 * MB);
    let root_build = rust_build(&root, 2 * MB);
    let sessions = sessions_in(&wt, true);
    let env = Env {
        sessions: &sessions,
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    // The worktree's build folder is reachable from both scan roots but listed once.
    let k = path_key(&wt_build);
    assert_eq!(scan.items.iter().filter(|i| i.key == k).count(), 1, "{:#?}", scan.items);
    assert_eq!(item(&scan, &wt_build).blocked, vec![Block::LiveSession]);
    // Dirty too: the fixture writes an untracked Cargo.toml (and target/) in the worktree.
    assert_eq!(item(&scan, &wt).blocked, vec![Block::DirtyGit, Block::LiveSession]);
    // The engine credits a session inside the root to the root too (component-aware "inside").
    assert_eq!(item(&scan, &root_build).blocked, vec![Block::LiveSession]);
}

#[test]
fn analyze_project_without_sessions_searches_only_the_given_roots_and_lists_no_session() {
    let (_t, base) = tmp();
    let root = repo(&base.join("work").join("proj"));
    let target = rust_build(&root, 2 * MB);
    let other = base.join("other");
    let copy = base.join("other").join("proj-scratch");
    fs::create_dir_all(&copy).unwrap();
    let opts = Options { drives: true, measure: false, admin: false };
    let scan = analyze_project_without_sessions(&root, &opts, &[other]).unwrap();
    assert_eq!(item(&scan, &target).category, Category::Builds);
    assert!(of(&scan, Category::Sessions).is_empty());
    assert_eq!(scan.search, SearchMode::BoundedWalk);
    // The folder with the project's name, found only below the injected root.
    assert_eq!(item(&scan, &copy).category, Category::Named);
    // Elevation is a plain query that never fails.
    let _ = is_elevated();
}

// ───────────────────────────── final review ─────────────────────────────

#[test]
fn i5_a_folder_holding_several_repositories_is_a_container_and_says_so() {
    let (_t, base) = tmp();
    let holder = base.join("Projects");
    repo(&holder.join("alpha"));
    repo(&holder.join("beta"));
    fs::create_dir_all(holder.join("alpha").join("target")).unwrap();

    let scan = analyze_with(&holder, &Options { drives: false, ..Options::default() }, &env()).unwrap();
    assert!(!scan.project.is_repo);
    assert!(scan.notes.contains(&Note::ContainerRoot), "{:?}", scan.notes);
    // `--builds`, `--worktrees`, `--logs`, `--sessions` are refused; naming a path still works.
    for c in [Category::Builds, Category::Worktrees, Category::Logs, Category::Sessions] {
        let req = CleanRequest { categories: vec![c], include: Vec::new(), mode: xiox_model::RemoveMode::Permanent };
        assert_eq!(select(&scan, &req).unwrap_err(), SelectError::ContainerRoot(holder.clone()), "{c}");
    }
    assert_eq!(
        SelectError::ContainerRoot(holder.clone()).to_string(),
        format!("refusing to clean everything under {}: run it on one project", holder.display())
    );
}

#[test]
fn i5_one_project_in_a_plain_folder_is_not_a_container() {
    let (_t, base) = tmp();
    let proj = base.join("work").join("plain");
    fs::create_dir_all(&proj).unwrap();
    let target = rust_build(&proj, 2 * MB);
    // One repository below is still one project.
    repo(&proj.join("only"));
    let scan = analyze_with(&proj, &Options { drives: false, ..Options::default() }, &env()).unwrap();
    assert!(!scan.notes.contains(&Note::ContainerRoot), "{:?}", scan.notes);
    let req = CleanRequest { categories: vec![Category::Builds], include: Vec::new(), mode: xiox_model::RemoveMode::Permanent };
    let chosen = select(&scan, &req).unwrap();
    assert_eq!(chosen.keys, vec![path_key(&target)]);
}

#[test]
fn i5_include_of_an_explicit_path_still_works_in_a_container() {
    let (_t, base) = tmp();
    let holder = base.join("Projects");
    repo(&holder.join("alpha"));
    repo(&holder.join("beta"));
    let target = rust_build(&holder.join("alpha"), MB);
    let scan = analyze_with(&holder, &Options { drives: false, ..Options::default() }, &env()).unwrap();
    assert!(scan.notes.contains(&Note::ContainerRoot));
    let req = CleanRequest { categories: Vec::new(), include: vec![target.clone()], mode: xiox_model::RemoveMode::Permanent };
    let chosen = select(&scan, &req).unwrap();
    assert_eq!(chosen.keys, vec![path_key(&target)]);
}

#[test]
fn i5_the_container_rule() {
    use crate::container::is_container;
    let home = Path::new(r"C:\Users\ann");
    let h = Some(home);
    // A drive root, the profile folder and any folder above it, whatever they hold.
    assert!(is_container(Path::new(r"D:\"), false, h, 1, 0));
    assert!(is_container(Path::new(r"D:\"), true, h, 1, 0));
    assert!(is_container(home, false, h, 1, 0));
    assert!(is_container(home, true, h, 1, 0), "a dotfiles repository at the profile folder");
    assert!(is_container(Path::new(r"C:\Users"), false, h, 1, 0));
    assert!(is_container(Path::new(r"C:\"), false, h, 1, 0));
    // A folder inside the profile, one project.
    assert!(!is_container(Path::new(r"C:\Users\ann\Projects\x"), false, h, 1, 0));
    assert!(!is_container(Path::new(r"C:\Users\anna"), false, h, 1, 0), "a sibling with a longer name is not an ancestor");
    // More than one repository, or more than one project root, in a folder that is not a repository.
    assert!(is_container(Path::new(r"D:\Projects"), false, h, 1, 2));
    assert!(is_container(Path::new(r"D:\Projects"), false, h, 2, 0));
    assert!(!is_container(Path::new(r"D:\Projects"), false, h, 1, 1));
    // A repository that holds other repositories (vendored clones) is still one project.
    assert!(!is_container(Path::new(r"D:\Projects\big"), true, h, 3, 5));
    // No home known: only the drive rule and the counts.
    assert!(!is_container(Path::new(r"D:\Projects\x"), false, None, 1, 0));
}

#[test]
fn the_repository_count_is_bounded_and_stops_at_two() {
    use crate::container::repos_below;
    let (_t, base) = tmp();
    let holder = base.join("many");
    for n in 0..5 {
        repo(&holder.join(format!("r{n}")));
    }
    assert_eq!(repos_below(&holder, 2), 2);
    assert_eq!(repos_below(&holder.join("r0"), 2), 0, "a repository is not below itself");
    // Not descended into a repository: its nested repository is not counted separately.
    let nest = base.join("nest");
    repo(&nest.join("outer"));
    repo(&nest.join("outer").join("inner"));
    assert_eq!(repos_below(&nest, 2), 1);
    // Depth is limited.
    let deep = base.join("deep").join("a").join("b").join("c").join("d");
    repo(&deep.join("r"));
    assert_eq!(repos_below(&base.join("deep"), 2), 0);
}

#[test]
fn the_remote_credentials_never_reach_the_project() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    git(&root, &["remote", "add", "origin", "https://user:s3cr3t@github.com/acme/proj.git"]);
    let project = resolve_project(&root).unwrap();
    assert_eq!(project.origin.as_deref(), Some("https://github.com/acme/proj.git"));
    let json = serde_json::to_string(&project).unwrap();
    assert!(!json.contains("s3cr3t") && !json.contains("user:"), "{json}");
    // A copy with the same remote (without the credentials) is still recognised.
    let copy = base.join("other").join("zzz");
    fs::create_dir_all(&copy).unwrap();
    git(&copy, &["init", "-q", "-b", "main"]);
    commit(&copy, "different.txt");
    git(&copy, &["remote", "add", "origin", "https://github.com/acme/proj"]);
    let e = Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(vec![base.join("other")]),
        now_ms: xiox_app::now_ms(),
        health: Default::default(),
    };
    let scan = analyze_with(&root, &Options::default(), &e).unwrap();
    assert!(scan.items.iter().any(|i| i.category == Category::Copies), "{:?}", scan.items);
}

#[test]
fn m4_a_failed_session_read_is_a_note_and_a_clean_read_is_not() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let mk = |health| Env {
        sessions: &[],
        archived: &[],
        search_roots: Some(Vec::new()),
        now_ms: xiox_app::now_ms(),
        health,
    };
    let ok = analyze_with(&root, &Options::default(), &mk(SessionHealth::default())).unwrap();
    assert!(!ok.notes.contains(&Note::SessionsNotRead), "{:?}", ok.notes);
    for health in [
        SessionHealth { live_failed: true, archived_failed: false },
        SessionHealth { live_failed: false, archived_failed: true },
    ] {
        let s = analyze_with(&root, &Options::default(), &mk(health)).unwrap();
        assert!(s.notes.contains(&Note::SessionsNotRead), "{health:?}: {:?}", s.notes);
    }
}

#[test]
fn m4_the_health_of_a_session_read_comes_from_its_warnings_and_what_it_returned() {
    use xiox_model::Loaded;
    let loaded = |items: usize, warnings: usize| xiox_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: (0..items)
                .map(|n| xiox_model::SessionRecord {
                    agent: Agent::Claude,
                    session_id: Some(format!("s{n}")),
                    project_path: std::path::PathBuf::from("X:\\p"),
                    last_activity_ms: 1,
                    archived: false,
                    live: false,
                })
                .collect(),
            warnings: vec!["w".to_string(); warnings],
        },
        worktrees: Vec::new(),
    };
    let h = |sessions: &[xiox_app::Preloaded], extra: &[String], archived: &[ArchivedSession], aw: &[String]| {
        crate::session_health(sessions, extra, archived, aw)
    };
    // Nothing read, nothing said: an empty PC, not a failure.
    assert_eq!(h(&[loaded(0, 0)], &[], &[], &[]), SessionHealth::default());
    // Sessions read, with a warning about some that were left out: not a failure.
    assert_eq!(h(&[loaded(3, 2)], &[], &[], &[]), SessionHealth::default());
    // A listing that returned nothing and complained: failed.
    assert!(h(&[loaded(0, 1)], &[], &[], &[]).live_failed);
    // A harness that failed beside one that worked: failed.
    assert!(h(&[loaded(5, 0), loaded(0, 1)], &[], &[], &[]).live_failed);
    // A warning of the loader itself (the managed worktrees).
    assert!(h(&[loaded(5, 0)], &["x".to_string()], &[], &[]).live_failed);
    // The archived listing: only the archived flag.
    let a = h(&[loaded(1, 0)], &[], &[], &["w".to_string()]);
    assert!(a.archived_failed && !a.live_failed);
}

#[test]
fn block_wording_has_one_source_of_truth() {
    for b in [
        Block::LiveSession,
        Block::DirtyGit,
        Block::UnpushedCommits,
        Block::Unreachable,
        Block::SessionLocked,
        Block::UnsavedWork,
        Block::UnsavedWorkUnknown,
        Block::Unknown,
    ] {
        assert_eq!(b.to_string(), b.reason());
        assert!(b.reason().is_ascii() && !b.reason().is_empty());
    }
    assert_eq!(Block::LiveSession.reason(), "an agent session is open");
    assert_eq!(Block::UnsavedWorkUnknown.reason(), "could not be checked for unsaved work");
}
