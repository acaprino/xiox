use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

const NOW: i64 = 1_800_000_000_000;

fn ago(days: i64) -> i64 {
    NOW - days * DAY_MS
}

// ───────────────────────────────── fakes ─────────────────────────────────

struct FakeScanner {
    items: Vec<ProjectCandidate>,
    warnings: Vec<String>,
}

impl ProjectScanner for FakeScanner {
    fn scan(&self, _: &[PathBuf], _: &ScanOptions, _: Option<&dyn DirIndex>) -> Loaded<ProjectCandidate> {
        Loaded { items: self.items.clone(), warnings: self.warnings.clone() }
    }
}

struct FakeSource {
    agent: Agent,
    sessions: Vec<SessionRecord>,
    warnings: Vec<String>,
    worktrees: Vec<PathBuf>,
    panics: bool,
}

impl FakeSource {
    fn new(agent: Agent, sessions: Vec<SessionRecord>) -> Self {
        Self { agent, sessions, warnings: vec![], worktrees: vec![], panics: false }
    }
}

impl SessionSource for FakeSource {
    fn agent(&self) -> Agent {
        self.agent
    }
    fn load(&self) -> Loaded<SessionRecord> {
        if self.panics {
            panic!("boom");
        }
        Loaded { items: self.sessions.clone(), warnings: self.warnings.clone() }
    }
    fn managed_worktrees(&self) -> Vec<PathBuf> {
        self.worktrees.clone()
    }
}

#[derive(Default)]
struct FakeGit {
    states: HashMap<String, GitState>,
    /// (path_key(root), lowercase relative path with `\`)
    tracked: HashSet<(String, String)>,
    state_calls: Mutex<HashMap<String, u32>>,
}

impl FakeGit {
    fn with_state(mut self, root: &str, s: GitState) -> Self {
        self.states.insert(path_key(Path::new(root)), s);
        self
    }
    fn with_tracked(mut self, root: &str, rel: &str) -> Self {
        self.tracked.insert((path_key(Path::new(root)), rel.to_lowercase()));
        self
    }
}

impl GitProbe for FakeGit {
    fn state(&self, root: &Path) -> GitState {
        let k = path_key(root);
        *self.state_calls.lock().unwrap().entry(k.clone()).or_default() += 1;
        self.states.get(&k).cloned().unwrap_or_default()
    }
    fn is_tracked(&self, root: &Path, rel: &Path) -> bool {
        self.tracked
            .contains(&(path_key(root), rel.to_string_lossy().to_lowercase().replace('/', "\\")))
    }
}

// ──────────────────────────────── builders ────────────────────────────────

fn art(root: &str, name: &str, bytes: u64, verified: bool) -> Artifact {
    Artifact {
        path: PathBuf::from(root).join(name),
        kind: ArtifactKind::RustTarget,
        label: format!("Rust {name}/"),
        size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
        verified,
    }
}

fn cand(root: &str, bytes: u64, mtime: Option<i64>) -> ProjectCandidate {
    ProjectCandidate {
        root: PathBuf::from(root),
        name: Path::new(root).file_name().unwrap().to_string_lossy().into_owned(),
        ecosystems: vec!["rust".into()],
        artifacts: vec![art(root, "target", bytes, true)],
        source_mtime_ms: mtime,
    }
}

fn sess(agent: Agent, path: &str, ms: i64, live: bool) -> SessionRecord {
    SessionRecord {
        agent,
        session_id: None,
        project_path: PathBuf::from(path),
        last_activity_ms: ms,
        archived: false,
        live,
    }
}

fn cfg() -> EngineConfig {
    EngineConfig {
        roots: vec![PathBuf::from(r"C:\dev")],
        container_roots: vec![PathBuf::from(r"C:\dev")],
        inactive_days: 14,
        scan: ScanOptions::default(),
        now_ms: NOW,
    }
}

fn run(cands: Vec<ProjectCandidate>, sources: &[&FakeSource], git: &FakeGit) -> AnalysisResult {
    let scanner = FakeScanner { items: cands, warnings: vec![] };
    let dyns: Vec<&dyn SessionSource> = sources.iter().map(|s| *s as &dyn SessionSource).collect();
    analyze(&cfg(), &scanner, None, &dyns, git)
}

fn only(r: &AnalysisResult) -> &ProjectReport {
    assert_eq!(r.reports.len(), 1);
    &r.reports[0]
}

fn clean_state() -> GitState {
    GitState {
        is_repo: true,
        dirty: Some(false),
        unpushed_commits: Some(0),
        has_upstream: Some(true),
        ..Default::default()
    }
}

// ────────────────────────────────── analyze ─────────────────────────────────

#[test]
fn session_in_subfolder_joins_project() {
    let src = FakeSource::new(
        Agent::Claude,
        vec![sess(Agent::Claude, r"C:\dev\app\crates\core", ago(30), false)],
    );
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.agents, vec![Agent::Claude]);
    assert_eq!(p.status, ProjectStatus::Inactive);
    assert_eq!(p.last_activity_source, Some(ActivitySource::Claude));
    assert_eq!(p.inactive_days, Some(30));
    assert!(p.reason.starts_with("Inattivo da 30 giorni (ultima sessione Claude)"), "{}", p.reason);
}

#[test]
fn prefix_match_is_component_aware_and_case_insensitive() {
    let src = FakeSource::new(
        Agent::Codex,
        vec![sess(Agent::Codex, r"C:\a\bc\x", ago(30), false)],
    );
    let r = run(vec![cand(r"C:\a\b", 100, None)], &[&src], &FakeGit::default());
    assert_eq!(only(&r).status, ProjectStatus::NoSessions);
    assert!(only(&r).agents.is_empty());

    let src = FakeSource::new(Agent::Codex, vec![sess(Agent::Codex, r"c:\A\B\", ago(30), false)]);
    let r = run(vec![cand(r"C:\a\b", 100, None)], &[&src], &FakeGit::default());
    assert_eq!(only(&r).status, ProjectStatus::Inactive);
    assert_eq!(only(&r).agents, vec![Agent::Codex]);
}

#[test]
fn drive_root_prefix_helper() {
    assert!(key_is_within(r"c:\foo", r"c:\"));
    assert!(key_is_within(r"c:\foo", r"c:\foo"));
    assert!(!key_is_within(r"c:\foobar", r"c:\foo"));
    assert!(!key_is_within(r"c:\foo", ""));
}

#[test]
fn live_session_means_in_use() {
    let src = FakeSource::new(
        Agent::Claude,
        vec![
            sess(Agent::Claude, r"C:\dev\app", ago(60), false),
            sess(Agent::Claude, r"C:\dev\app\sub", ago(0), true),
        ],
    );
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::InUse);
    assert_eq!(p.blocked_by, vec![BlockReason::LiveSession]);
    assert!(!p.preselected);
    assert_eq!(p.reason, "Sessione Claude attiva ora");
    assert_eq!(p.reclaimable_bytes, 100);
}

#[test]
fn tracked_artifact_blocks() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), false)]);
    let git = FakeGit::default().with_tracked(r"C:\dev\app", "target");
    let r = run(vec![cand(r"C:\dev\app", 500, None)], &[&src], &git);
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Blocked);
    assert_eq!(p.blocked_by, vec![BlockReason::TrackedArtifact]);
    assert_eq!(p.reason, "Un target/ è tracciato da git");
    assert!(!p.preselected);
    assert_eq!(p.reclaimable_bytes, 500, "blocked projects still show reclaimable bytes");
}

#[test]
fn unverified_artifact_blocks() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts[0].verified = false;
    let r = run(vec![c], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Blocked);
    assert_eq!(p.blocked_by, vec![BlockReason::UnverifiedArtifact]);
    assert!(!p.preselected);
}

#[test]
fn dirty_ordinary_project_is_not_blocked() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(40), false)]);
    let git = FakeGit::default().with_state(
        r"C:\dev\app",
        GitState { is_repo: true, dirty: Some(true), unpushed_commits: Some(3), ..Default::default() },
    );
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[&src], &git);
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Inactive);
    assert!(p.blocked_by.is_empty());
    assert!(!p.preselected, "dirty git: selectable but never preselected");
    assert!(p.reason.contains("modifiche git non committate"), "{}", p.reason);
    assert!(!p.is_agent_worktree);
}

#[test]
fn dirty_agent_worktree_is_blocked() {
    let mut src = FakeSource::new(
        Agent::Codex,
        vec![sess(Agent::Codex, r"C:\Users\u\.codex\worktrees\ab12\proj", ago(40), false)],
    );
    src.worktrees = vec![PathBuf::from(r"C:\Users\u\.codex\worktrees")];
    let root = r"C:\Users\u\.codex\worktrees\ab12\proj";
    let git = FakeGit::default().with_state(
        root,
        GitState {
            is_repo: true,
            dirty: Some(true),
            unpushed_commits: Some(2),
            is_linked_worktree: true,
            main_worktree: Some(PathBuf::from(r"C:\dev\proj")),
            ..Default::default()
        },
    );
    let r = run(vec![cand(root, 100, None)], &[&src], &git);
    let p = only(&r);
    assert!(p.is_agent_worktree);
    assert_eq!(p.status, ProjectStatus::Blocked);
    assert_eq!(p.blocked_by, vec![BlockReason::DirtyGit, BlockReason::UnpushedCommits]);
    assert!(p.reason.contains("Modifiche git non committate nel worktree"), "{}", p.reason);
    assert!(!p.preselected);
}

#[test]
fn worktree_dir_itself_counts_as_managed() {
    let mut src = FakeSource::new(Agent::Claude, vec![]);
    src.worktrees = vec![PathBuf::from(r"C:\dev\app\.claude\worktrees\wt1")];
    let r = run(
        vec![cand(r"C:\dev\app\.claude\worktrees\wt1", 10, None), cand(r"C:\dev\app", 10, None)],
        &[&src],
        &FakeGit::default(),
    );
    let wt = r.reports.iter().find(|p| p.name == "wt1").unwrap();
    let app = r.reports.iter().find(|p| p.name == "app").unwrap();
    assert!(wt.is_agent_worktree);
    assert!(!app.is_agent_worktree);
}

#[test]
fn no_sessions_is_selectable_but_not_preselected() {
    let r = run(
        vec![cand(r"C:\dev\app", 100, Some(ago(61)))],
        &[],
        &FakeGit::default(),
    );
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::NoSessions);
    assert!(!p.preselected);
    assert!(p.blocked_by.is_empty());
    assert_eq!(p.last_activity_source, Some(ActivitySource::Files));
    assert_eq!(p.reason, "Nessuna sessione trovata; ultima modifica ai file 61 giorni fa");
}

#[test]
fn no_sessions_with_git_commit_names_the_commit() {
    let git = FakeGit::default().with_state(
        r"C:\dev\app",
        GitState { is_repo: true, last_commit_ms: Some(ago(12)), ..Default::default() },
    );
    let r = run(vec![cand(r"C:\dev\app", 100, Some(ago(61)))], &[], &git);
    let p = only(&r);
    assert_eq!(p.last_activity_source, Some(ActivitySource::Git));
    assert_eq!(p.reason, "Nessuna sessione trovata; ultimo commit 12 giorni fa");
}

#[test]
fn no_data_at_all_has_no_activity() {
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::NoSessions);
    assert_eq!(p.last_activity_ms, None);
    assert_eq!(p.inactive_days, None);
    assert_eq!(p.last_activity_source, None);
}

#[test]
fn inactive_threshold_boundary() {
    // exactly 14 days -> Inactive; 1 ms short of 14 days -> 13 full days -> Recent.
    let at = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", NOW - 14 * DAY_MS, false)]);
    let r = run(vec![cand(r"C:\dev\a", 1, None)], &[&at], &FakeGit::default());
    assert_eq!(only(&r).status, ProjectStatus::Inactive);
    assert!(only(&r).preselected);

    let short = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", NOW - 14 * DAY_MS + 1, false)]);
    let r = run(vec![cand(r"C:\dev\a", 1, None)], &[&short], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Recent);
    assert_eq!(p.inactive_days, Some(13));
    assert!(!p.preselected);
    assert!(p.reason.starts_with("Attività recente: 13 giorni fa"), "{}", p.reason);
}

#[test]
fn newer_files_make_session_project_recent() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", ago(90), false)]);
    let r = run(vec![cand(r"C:\dev\a", 1, Some(ago(2)))], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Recent);
    assert_eq!(p.last_activity_source, Some(ActivitySource::Files));
    assert_eq!(p.agents, vec![Agent::Claude]);
}

#[test]
fn newest_activity_wins_and_names_its_source() {
    let git_newest = FakeGit::default().with_state(
        r"C:\dev\a",
        GitState { is_repo: true, last_commit_ms: Some(ago(1)), ..Default::default() },
    );
    let src = FakeSource::new(
        Agent::Opencode,
        vec![sess(Agent::Opencode, r"C:\dev\a", ago(20), false)],
    );
    let r = run(vec![cand(r"C:\dev\a", 1, Some(ago(30)))], &[&src], &git_newest);
    assert_eq!(only(&r).last_activity_source, Some(ActivitySource::Git));

    let r = run(vec![cand(r"C:\dev\a", 1, Some(ago(30)))], &[&src], &FakeGit::default());
    assert_eq!(only(&r).last_activity_source, Some(ActivitySource::Opencode));
    assert_eq!(only(&r).inactive_days, Some(20));
}

#[test]
fn agents_are_distinct_and_ordered() {
    let a = FakeSource::new(
        Agent::Codex,
        vec![
            sess(Agent::Codex, r"C:\dev\a", ago(20), false),
            sess(Agent::Codex, r"C:\dev\a\x", ago(21), false),
        ],
    );
    let b = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", ago(25), false)]);
    let r = run(vec![cand(r"C:\dev\a", 1, None)], &[&a, &b], &FakeGit::default());
    assert_eq!(only(&r).agents, vec![Agent::Claude, Agent::Codex]);
}

#[test]
fn reports_sorted_by_reclaimable_desc() {
    let r = run(
        vec![
            cand(r"C:\dev\small", 10, None),
            cand(r"C:\dev\big", 1000, None),
            cand(r"C:\dev\mid", 100, None),
        ],
        &[],
        &FakeGit::default(),
    );
    let names: Vec<&str> = r.reports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["big", "mid", "small"]);
}

#[test]
fn allocated_bytes_preferred_and_summed() {
    let mut c = cand(r"C:\dev\a", 100, None);
    c.artifacts[0].size.allocated_bytes = Some(4096);
    c.artifacts.push(art(r"C:\dev\a", "node_modules", 50, true));
    let r = run(vec![c], &[], &FakeGit::default());
    assert_eq!(only(&r).reclaimable_bytes, 4096 + 50);
}

#[test]
fn sources_failing_or_warning_never_abort() {
    let mut ok = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", ago(30), false)]);
    ok.warnings = vec!["claude: file X corrotto".into()];
    let mut bad = FakeSource::new(Agent::Codex, vec![]);
    bad.panics = true;
    let mut warn = FakeSource::new(Agent::Opencode, vec![]);
    warn.warnings = vec!["opencode: db bloccato".into()];

    let scanner = FakeScanner { items: vec![cand(r"C:\dev\a", 1, None)], warnings: vec!["scan: accesso negato".into()] };
    let dyns: Vec<&dyn SessionSource> = vec![&ok, &bad, &warn];
    let r = analyze(&cfg(), &scanner, None, &dyns, &FakeGit::default());
    assert_eq!(only(&r).status, ProjectStatus::Inactive);
    assert!(r.warnings.contains(&"scan: accesso negato".to_string()));
    assert!(r.warnings.contains(&"claude: file X corrotto".to_string()));
    assert!(r.warnings.contains(&"opencode: db bloccato".to_string()));
    assert_eq!(r.warnings.len(), 4, "panicking source adds exactly one warning: {:?}", r.warnings);
}

#[test]
fn git_state_queried_once_per_project() {
    let git = FakeGit::default();
    let mut c = cand(r"C:\dev\a", 1, None);
    c.artifacts.push(art(r"C:\dev\a", "node_modules", 1, true));
    run(vec![c, cand(r"C:\dev\b", 1, None)], &[], &git);
    let calls = git.state_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.values().all(|n| *n == 1), "{calls:?}");
}

#[test]
fn duplicate_roots_are_reported_once() {
    let scanner = FakeScanner {
        items: vec![cand(r"C:\dev\A", 1, None), cand(r"c:\dev\a\", 1, None)],
        warnings: vec![],
    };
    let r = analyze(&cfg(), &scanner, None, &[], &FakeGit::default());
    assert_eq!(r.reports.len(), 1);
    assert_eq!(r.warnings.len(), 1);
}

#[test]
fn report_id_is_path_key() {
    let r = run(vec![cand(r"C:\Dev\App", 1, None)], &[], &FakeGit::default());
    assert_eq!(only(&r).id, path_key(Path::new(r"C:\Dev\App")));
}

// ────────────────────────────────── build_plan ──────────────────────────────

/// Hand-made report so plan tests do not depend on analyze().
fn report(root: &str, status: ProjectStatus, bytes: &[u64]) -> ProjectReport {
    let artifacts: Vec<Artifact> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| art(root, &format!("a{i}"), *b, true))
        .collect();
    ProjectReport {
        id: path_key(Path::new(root)),
        root: PathBuf::from(root),
        name: Path::new(root).file_name().unwrap().to_string_lossy().into_owned(),
        ecosystems: vec![],
        agents: vec![],
        last_activity_ms: Some(ago(30)),
        last_activity_source: Some(ActivitySource::Claude),
        inactive_days: Some(30),
        status,
        blocked_by: vec![],
        reason: "Inattivo da 30 giorni (ultima sessione Claude)".into(),
        reclaimable_bytes: bytes.iter().sum(),
        artifacts,
        preselected: status == ProjectStatus::Inactive,
        is_agent_worktree: false,
        git: GitState::default(),
    }
}

fn worktree_report(root: &str, main: Option<&str>, dirty: Option<bool>, status: ProjectStatus) -> ProjectReport {
    let mut r = report(root, status, &[700, 300]);
    r.is_agent_worktree = true;
    r.git = GitState {
        is_repo: true,
        dirty,
        is_linked_worktree: main.is_some(),
        main_worktree: main.map(PathBuf::from),
        ..Default::default()
    };
    r
}

#[test]
fn plan_normal_project_has_one_item_per_artifact() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[100, 50]);
    let plan = build_plan(std::slice::from_ref(&rep), std::slice::from_ref(&rep.id), RemoveMode::Recycle).unwrap();
    assert_eq!(plan.mode, RemoveMode::Recycle);
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.expected_bytes, 150);
    assert!(plan.items.iter().all(|i| i.action == PlanAction::RemoveDir && i.project_id == rep.id));
    assert_eq!(plan.items[0].path, rep.artifacts[0].path);
    assert_eq!(plan.items[0].kind, ArtifactKind::RustTarget);
    assert_eq!(plan.items[0].expected_bytes, 100);
    assert_eq!(plan.items[0].reason, rep.reason);
}

#[test]
fn plan_recent_and_no_sessions_are_selectable() {
    let a = report(r"C:\dev\a", ProjectStatus::Recent, &[10]);
    let b = report(r"C:\dev\b", ProjectStatus::NoSessions, &[20]);
    let plan = build_plan(&[a.clone(), b.clone()], &[a.id, b.id], RemoveMode::Permanent).unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.expected_bytes, 30);
    assert_eq!(plan.mode, RemoveMode::Permanent);
}

#[test]
fn plan_worktree_is_single_git_worktree_remove() {
    let rep = worktree_report(
        r"C:\Users\u\.codex\worktrees\ab12\proj",
        Some(r"C:\dev\proj"),
        Some(false),
        ProjectStatus::Inactive,
    );
    let plan = build_plan(std::slice::from_ref(&rep), std::slice::from_ref(&rep.id), RemoveMode::Permanent).unwrap();
    assert_eq!(plan.items.len(), 1);
    let item = &plan.items[0];
    assert_eq!(item.kind, ArtifactKind::WorktreeDir);
    assert_eq!(item.path, rep.root);
    assert_eq!(item.action, PlanAction::GitWorktreeRemove { repo: PathBuf::from(r"C:\dev\proj") });
    assert_eq!(item.expected_bytes, 1000);
    assert_eq!(plan.expected_bytes, 1000);
}

#[test]
fn plan_recent_clean_worktree_also_uses_git_worktree_remove() {
    let rep = worktree_report(r"C:\wt\x", Some(r"C:\dev\proj"), Some(false), ProjectStatus::Recent);
    let plan = build_plan(std::slice::from_ref(&rep), std::slice::from_ref(&rep.id), RemoveMode::Recycle).unwrap();
    assert!(matches!(plan.items[0].action, PlanAction::GitWorktreeRemove { .. }));
}

#[test]
fn plan_worktree_falls_back_to_artifacts_when_unsure() {
    // unknown main worktree
    let a = worktree_report(r"C:\wt\a", None, Some(false), ProjectStatus::Inactive);
    // cleanliness unknown
    let b = worktree_report(r"C:\wt\b", Some(r"C:\dev\proj"), None, ProjectStatus::Inactive);
    // "main worktree" is the directory itself: never git-remove the main checkout
    let c = worktree_report(r"C:\dev\proj", Some(r"c:\DEV\proj\"), Some(false), ProjectStatus::Inactive);
    // no sessions: not eligible for whole-worktree removal
    let d = worktree_report(r"C:\wt\d", Some(r"C:\dev\proj"), Some(false), ProjectStatus::NoSessions);
    for rep in [a, b, c, d] {
        let plan = build_plan(std::slice::from_ref(&rep), std::slice::from_ref(&rep.id), RemoveMode::Recycle).unwrap();
        assert_eq!(plan.items.len(), 2, "{}", rep.name);
        assert!(plan.items.iter().all(|i| i.action == PlanAction::RemoveDir), "{}", rep.name);
    }
}

#[test]
fn plans_something_matches_what_build_plan_gives() {
    let clean = worktree_report(r"C:\wt\a", Some(r"C:\dev\proj"), Some(false), ProjectStatus::Inactive);
    let mut dirty = worktree_report(r"C:\wt\b", Some(r"C:\dev\proj"), Some(true), ProjectStatus::Inactive);
    let mut unknown = worktree_report(r"C:\wt\c", Some(r"C:\dev\proj"), None, ProjectStatus::Recent);
    let mut empty_clean = clean.clone();
    empty_clean.artifacts.clear();
    dirty.artifacts.clear();
    unknown.artifacts.clear();
    let mut project = report(r"C:\dev\app", ProjectStatus::Inactive, &[]);
    assert!(project.artifacts.is_empty());
    for (rep, want) in [(&clean, true), (&empty_clean, true), (&dirty, false), (&unknown, false), (&project, false)] {
        let plan = build_plan(std::slice::from_ref(rep), std::slice::from_ref(&rep.id), RemoveMode::Recycle).unwrap();
        assert_eq!(plans_something(rep), want, "{}", rep.name);
        assert_eq!(!plan.items.is_empty(), want, "{}", rep.name);
    }
    project.artifacts = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]).artifacts;
    assert!(plans_something(&project));
    assert_eq!(whole_worktree_repo(&clean), Some(&PathBuf::from(r"C:\dev\proj")));
    assert_eq!(whole_worktree_repo(&dirty), None);
}

#[test]
fn plan_rejects_unknown_id() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]);
    let err = build_plan(&[rep], &["c:\\nope".to_string()], RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("non trovato"), "{err}");
}

#[test]
fn plan_rejects_in_use_blocked_and_unreachable() {
    for (status, needle) in [
        (ProjectStatus::InUse, "in uso"),
        (ProjectStatus::Blocked, "bloccato"),
        (ProjectStatus::Unreachable, "raggiungibile"),
    ] {
        let rep = report(r"C:\dev\app", status, &[1]);
        let ok = report(r"C:\dev\ok", ProjectStatus::Inactive, &[1]);
        let err = build_plan(&[ok.clone(), rep.clone()], &[ok.id, rep.id], RemoveMode::Recycle)
            .unwrap_err();
        assert!(err.to_string().contains(needle), "{status:?}: {err}");
    }
}

#[test]
fn plan_rejects_report_with_blocked_by_even_if_status_looks_fine() {
    let mut rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]);
    rep.blocked_by = vec![BlockReason::TrackedArtifact];
    assert!(build_plan(std::slice::from_ref(&rep), std::slice::from_ref(&rep.id), RemoveMode::Recycle).is_err());
}

#[test]
fn plan_dedups_ids_and_paths_and_ids_are_unique() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[100]);
    let sel = vec![rep.id.clone(), rep.id.clone()];
    let p1 = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap();
    let p2 = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(p1.items.len(), 1);
    assert_eq!(p1.expected_bytes, 100);
    assert_ne!(p1.id, p2.id);
}

#[test]
fn plan_empty_selection_is_empty_plan() {
    let plan = build_plan(&[], &[], RemoveMode::Recycle).unwrap();
    assert!(plan.items.is_empty());
    assert_eq!(plan.expected_bytes, 0);
}

// ───────────────────────── analyze -> plan end to end ───────────────────────

#[test]
fn analyze_then_plan_roundtrip() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\old", ago(100), false)]);
    let r = run(
        vec![cand(r"C:\dev\old", 900, None), cand(r"C:\dev\fresh", 5, Some(ago(1)))],
        &[&src],
        &FakeGit::default().with_state(r"C:\dev\old", clean_state()),
    );
    let pre: Vec<String> = r.reports.iter().filter(|p| p.preselected).map(|p| p.id.clone()).collect();
    assert_eq!(pre.len(), 1);
    let plan = build_plan(&r.reports, &pre, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.expected_bytes, 900);
}

// ─────────────────────── session inheritance between candidates ──────────────

#[test]
fn candidate_without_sessions_inherits_from_nearest_ancestor_candidate() {
    // Tauri repo: <repo> (package.json) and <repo>/src-tauri (Cargo.toml); agent ran in <repo>.
    let src = FakeSource::new(
        Agent::Claude,
        vec![
            sess(Agent::Claude, r"C:\dev\repo", ago(40), false),
            sess(Agent::Claude, r"C:\dev\repo", ago(0), true),
        ],
    );
    let r = run(
        vec![cand(r"C:\dev\repo", 10, None), cand(r"C:\dev\repo\src-tauri", 20, None)],
        &[&src],
        &FakeGit::default(),
    );
    let tauri = r.reports.iter().find(|p| p.name == "src-tauri").unwrap();
    assert_eq!(tauri.status, ProjectStatus::InUse, "inherited live session must block");
    assert_eq!(tauri.agents, vec![Agent::Claude]);
    assert_eq!(tauri.blocked_by, vec![BlockReason::LiveSession]);

    // Inactive variant, and nearest ancestor wins over a farther one.
    let src = FakeSource::new(
        Agent::Codex,
        vec![sess(Agent::Codex, r"C:\dev\repo", ago(30), false), sess(Agent::Claude, r"C:\dev\repo\app", ago(50), false)],
    );
    let r = run(
        vec![
            cand(r"C:\dev\repo", 1, None),
            cand(r"C:\dev\repo\app", 1, None),
            cand(r"C:\dev\repo\app\src-tauri\deep", 1, None),
        ],
        &[&src],
        &FakeGit::default()
            .with_state(r"C:\dev\repo", clean_state())
            .with_state(r"C:\dev\repo\app", clean_state()),
    );
    let deep = r.reports.iter().find(|p| p.name == "deep").unwrap();
    assert_eq!(deep.status, ProjectStatus::Inactive);
    // Sessions started in any ancestor below the scan root count; the most recent wins (conservative).
    assert_eq!(deep.inactive_days, Some(30), "codex@30d in <repo> is the most recent ancestor session");
    assert!(deep.agents.contains(&Agent::Codex) && deep.agents.contains(&Agent::Claude));
}

#[test]
fn ancestor_session_below_scan_root_counts_but_not_at_or_above_root() {
    // Real case: agent started in the repo root, candidate is its backend/ subfolder.
    let src = FakeSource::new(
        Agent::Claude,
        vec![sess(Agent::Claude, r"C:\dev\repo", ago(2), false), sess(Agent::Claude, r"C:\dev", ago(1), false)],
    );
    let git = FakeGit::default().with_state(r"C:\dev\repo", clean_state());
    let r = run(vec![cand(r"C:\dev\repo\backend", 1, None)], &[&src], &git);
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Recent);
    assert_eq!(p.inactive_days, Some(2), "a session at the scan root must be ignored; repo@2d is used");
}

#[test]
fn no_inheritance_from_non_candidate_ancestor_or_unrelated_sibling() {
    // Session in the parent directory D:\dev (not a candidate) and in a sibling candidate.
    let src = FakeSource::new(
        Agent::Claude,
        vec![sess(Agent::Claude, r"C:\dev", ago(30), false), sess(Agent::Claude, r"C:\dev\other", ago(30), false)],
    );
    let r = run(
        vec![cand(r"C:\dev\repo\src-tauri", 1, None), cand(r"C:\dev\other", 1, None), cand(r"C:\dev\otherx", 1, None)],
        &[&src],
        &FakeGit::default(),
    );
    let by = |n: &str| r.reports.iter().find(|p| p.name == n).unwrap();
    assert_eq!(by("src-tauri").status, ProjectStatus::NoSessions);
    assert_eq!(by("otherx").status, ProjectStatus::NoSessions, "component-aware: otherx is not inside other");
    assert_eq!(by("other").status, ProjectStatus::Inactive);
}

#[test]
fn session_in_a_big_non_repo_folder_does_not_credit_projects_but_a_small_workspace_does() {
    // Real case: a session started in D:\Projects (a folder holding 100+ projects) made every project
    // below it look active when the scan root was D:\. A small workspace folder (backend+frontend)
    // is different: its sessions do belong to its few projects.
    let src = FakeSource::new(
        Agent::Claude,
        vec![
            sess(Agent::Claude, r"C:\dev\projects", ago(2), false),
            sess(Agent::Claude, r"C:\dev\work", ago(3), false),
        ],
    );
    let mut cands: Vec<ProjectCandidate> =
        (0..10).map(|i| cand(&format!(r"C:\dev\projects\p{i}"), 1, None)).collect();
    cands.push(cand(r"C:\dev\work\backend", 1, None));
    cands.push(cand(r"C:\dev\work\frontend", 1, None));
    let r = run(cands, &[&src], &FakeGit::default());
    for p in &r.reports {
        let big = p.root.to_string_lossy().contains("projects");
        if big {
            assert_eq!(p.status, ProjectStatus::NoSessions, "{}", p.root.display());
        } else {
            assert_eq!(p.status, ProjectStatus::Recent, "{}", p.root.display());
            assert_eq!(p.inactive_days, Some(3));
        }
    }
}
