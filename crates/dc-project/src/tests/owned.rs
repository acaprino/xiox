use super::fixtures::*;
use crate::*;
use dc_model::{Agent, ArtifactKind, Level, Loaded, SessionRecord, path_key};
use dc_mukade::ArchivedSession;
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
    let now = dc_app::now_ms();
    let sessions = vec![dc_app::Preloaded {
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
    let env = Env { sessions: &sessions, archived: &[], search_roots: Some(Vec::new()), now_ms: now };

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
    let env = Env { sessions: &[], archived: &archived, search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

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
    let env = Env { sessions: &[], archived: &archived, search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

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
    let env = Env { sessions: &[], archived: &archived, search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

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
fn sessions_in(dir: &std::path::Path, live: bool) -> Vec<dc_app::Preloaded> {
    vec![dc_app::Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: Some("live".into()),
                project_path: dir.to_path_buf(),
                last_activity_ms: dc_app::now_ms(),
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
    let env = Env { sessions: &sessions, archived: &[], search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

    let scan = analyze_with(&root, &Options::default(), &env).unwrap();
    let w = item(&scan, &wt);
    assert_eq!(w.blocked, vec![Block::LiveSession]);
    assert_eq!(w.level, Level::Caution);

    // The same session, not live: nothing blocks.
    let sessions = sessions_in(&wt.join("src"), false);
    let env = Env { sessions: &sessions, archived: &[], search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };
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
    let env = Env { sessions: &sessions, archived: &[], search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

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
    let report = dc_model::ProjectReport {
        id: path_key(&root),
        root: root.clone(),
        name: "proj".into(),
        ecosystems: vec!["rust".into()],
        agents: Vec::new(),
        last_activity_ms: None,
        last_activity_source: None,
        inactive_days: None,
        status: dc_model::ProjectStatus::Unreachable,
        blocked_by: vec![dc_model::BlockReason::Unreachable],
        reason: String::new(),
        artifacts: vec![dc_model::Artifact {
            id: path_key(&path),
            path: path.clone(),
            kind: ArtifactKind::RustTarget,
            label: String::new(),
            size: dc_model::SizeInfo { logical_bytes: 10, allocated_bytes: Some(4096), file_count: 1 },
            verified: true,
            class: dc_model::Class::Build,
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
    let env = Env { sessions: &sessions, archived: &[], search_roots: Some(Vec::new()), now_ms: dc_app::now_ms() };

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
