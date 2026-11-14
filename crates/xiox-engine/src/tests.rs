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
    /// (path_key(root), lowercase relative path with `\`) -> answer of `is_ignored`
    ignored: HashMap<(String, String), bool>,
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
    fn with_ignored(mut self, root: &str, rel: &str, ignored: bool) -> Self {
        self.ignored.insert((path_key(Path::new(root)), rel.to_lowercase()), ignored);
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
    fn is_ignored(&self, root: &Path, rel: &Path) -> Option<bool> {
        self.ignored
            .get(&(path_key(root), rel.to_string_lossy().to_lowercase().replace('/', "\\")))
            .copied()
    }
}

// ──────────────────────────────── builders ────────────────────────────────

fn art(root: &str, name: &str, bytes: u64, verified: bool) -> Artifact {
    art_of(root, name, bytes, verified, ArtifactKind::RustTarget, Class::Build)
}

/// As the scanner gives it: placeholder `level` (`class.base_level()`), no note, not preselected.
fn art_of(root: &str, name: &str, bytes: u64, verified: bool, kind: ArtifactKind, class: Class) -> Artifact {
    let path = PathBuf::from(root).join(name);
    Artifact {
        id: path_key(&path),
        path,
        kind,
        label: format!("{name}/"),
        size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
        verified,
        class,
        level: class.base_level(),
        note: String::new(),
        ambiguous: false,
        preselected: false,
    }
}

fn node_modules(root: &str, bytes: u64) -> Artifact {
    art_of(root, "node_modules", bytes, true, ArtifactKind::NodeModules, Class::Dependency)
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
    assert!(p.reason.starts_with("Inactive for 30 days (last Claude session)"), "{}", p.reason);
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
    assert_eq!(p.reason, "Claude session active now");
    assert_eq!(p.reclaimable_bytes, 100);
}

#[test]
fn dependency_is_optional_and_build_is_recommended() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(30), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts.push(node_modules(r"C:\dev\app", 900));
    let r = run(vec![c], &[&src], &FakeGit::default().with_state(r"C:\dev\app", clean_state()));
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Inactive);
    let by = |n: &str| p.artifacts.iter().find(|a| a.path.ends_with(n)).unwrap();
    assert_eq!(by("node_modules").level, Level::Optional);
    assert!(!by("node_modules").preselected, "Dependency: never preselected");
    assert_eq!(by("node_modules").note, "");
    assert_eq!(by("target").level, Level::Recommended);
    assert!(by("target").preselected);
    assert!(p.preselected, "a project is preselected when one of its folders is");
    assert_eq!(by("target").id, path_key(Path::new(r"C:\dev\app\target")));
}

#[test]
fn project_with_only_optional_folders_is_not_preselected() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(30), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts = vec![node_modules(r"C:\dev\app", 900)];
    let r = run(vec![c], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Inactive);
    assert!(!p.preselected);
}

#[test]
fn tracked_artifact_is_caution_not_blocked() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), false)]);
    let git = FakeGit::default().with_tracked(r"C:\dev\app", "target");
    let r = run(vec![cand(r"C:\dev\app", 500, None)], &[&src], &git);
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Inactive, "a tracked folder no longer blocks the project");
    assert!(p.blocked_by.is_empty());
    let a = &p.artifacts[0];
    assert_eq!(a.level, Level::Caution);
    assert!(a.note.contains("tracked by git"), "{}", a.note);
    assert_eq!(a.note, "Contains files tracked by git");
    assert!(!a.preselected);
    assert!(!p.preselected);
    assert_eq!(p.reclaimable_bytes, 500);
}

#[test]
fn unverified_artifact_is_caution_not_blocked() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts[0].verified = false;
    let r = run(vec![c], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Inactive);
    assert!(p.blocked_by.is_empty());
    assert_eq!(p.artifacts[0].level, Level::Caution);
    assert_eq!(p.artifacts[0].note, "Folder not recognized with certainty");
    assert!(!p.preselected);
}

#[test]
fn notes_add_up_after_the_catalog_one() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts[0].verified = false;
    c.artifacts[0].note = "Catalog note".into();
    let git = FakeGit::default().with_tracked(r"C:\dev\app", "target");
    let r = run(vec![c], &[&src], &git);
    let a = &only(&r).artifacts[0];
    assert_eq!(a.level, Level::Caution);
    assert_eq!(
        a.note,
        "Catalog note; Contains files tracked by git; Folder not recognized with certainty"
    );
}

#[test]
fn ambiguous_name_is_caution_unless_git_ignores_it() {
    let root = r"C:\dev\app";
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, root, ago(60), false)]);
    let ambiguous = || {
        let mut c = cand(root, 500, None);
        c.artifacts[0].ambiguous = true;
        c
    };

    let r = run(vec![ambiguous()], &[&src], &FakeGit::default().with_ignored(root, "target", false));
    let a = &only(&r).artifacts[0];
    assert_eq!(a.level, Level::Caution);
    assert_eq!(a.note, "May contain sources: git does not ignore it");
    assert!(!a.preselected);

    let r = run(vec![ambiguous()], &[&src], &FakeGit::default());
    let a = &only(&r).artifacts[0];
    assert_eq!(a.level, Level::Caution);
    assert_eq!(a.note, "May contain sources: not in a git repository");

    let r = run(vec![ambiguous()], &[&src], &FakeGit::default().with_ignored(root, "target", true));
    let a = &only(&r).artifacts[0];
    assert_eq!(a.level, Level::Recommended, "ignored by git: the level of its class");
    assert_eq!(a.note, "");
    assert!(a.preselected);

    // Not ambiguous: git's answer does not matter.
    let r = run(vec![cand(root, 500, None)], &[&src], &FakeGit::default().with_ignored(root, "target", false));
    assert_eq!(only(&r).artifacts[0].level, Level::Recommended);
}

#[test]
fn live_session_never_preselects_folders() {
    let src = FakeSource::new(
        Agent::Claude,
        vec![sess(Agent::Claude, r"C:\dev\app", ago(60), true)],
    );
    // Old activity, but the process is alive: the project is blocked, nothing is preselected.
    let r = run(vec![cand(r"C:\dev\app", 500, None)], &[&src], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::InUse);
    assert_eq!(p.inactive_days, Some(60));
    assert!(!p.artifacts[0].preselected);
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
    assert!(p.artifacts.iter().all(|a| !a.preselected && a.level == Level::Recommended));
    assert!(p.reason.contains("uncommitted git changes"), "{}", p.reason);
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
    assert!(p.reason.contains("Uncommitted git changes in the worktree"), "{}", p.reason);
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
fn no_sessions_old_project_is_preselected() {
    let r = run(
        vec![cand(r"C:\dev\app", 100, Some(ago(61)))],
        &[],
        &FakeGit::default(),
    );
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::NoSessions);
    assert!(p.preselected, "inactivity counts with or without sessions");
    assert!(p.artifacts[0].preselected);
    assert!(p.blocked_by.is_empty());
    assert_eq!(p.last_activity_source, Some(ActivitySource::Files));
    assert_eq!(p.reason, "No sessions found; last file change 61 days ago");
}

#[test]
fn no_sessions_preselection_follows_the_last_commit() {
    let at = |days: i64| {
        FakeGit::default().with_state(
            r"C:\dev\app",
            GitState { last_commit_ms: Some(ago(days)), ..clean_state() },
        )
    };
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[], &at(30));
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::NoSessions);
    assert!(p.artifacts[0].preselected, "target of a project last committed 30 days ago");
    assert!(p.preselected);

    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[], &at(3));
    let p = only(&r);
    assert!(!p.artifacts[0].preselected, "3 days ago: recent");
    assert!(!p.preselected);

    // No date at all: never preselected.
    let r = run(vec![cand(r"C:\dev\app", 100, None)], &[], &FakeGit::default());
    assert!(!only(&r).preselected);
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
    assert_eq!(p.reason, "No sessions found; last commit 12 days ago");
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
    assert!(only(&r).artifacts[0].preselected);

    let short = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\a", NOW - 14 * DAY_MS + 1, false)]);
    let r = run(vec![cand(r"C:\dev\a", 1, None)], &[&short], &FakeGit::default());
    let p = only(&r);
    assert_eq!(p.status, ProjectStatus::Recent);
    assert_eq!(p.inactive_days, Some(13));
    assert!(!p.preselected);
    assert!(!p.artifacts[0].preselected);

    // Same boundary without sessions (age from the files).
    let r = run(vec![cand(r"C:\dev\a", 1, Some(NOW - 14 * DAY_MS))], &[], &FakeGit::default());
    assert!(only(&r).preselected);
    let r = run(vec![cand(r"C:\dev\a", 1, Some(NOW - 14 * DAY_MS + 1))], &[], &FakeGit::default());
    assert!(!only(&r).preselected);
    assert!(p.reason.starts_with("Recent activity: 13 days ago"), "{}", p.reason);
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
    ok.warnings = vec!["claude: file X corrupt".into()];
    let mut bad = FakeSource::new(Agent::Codex, vec![]);
    bad.panics = true;
    let mut warn = FakeSource::new(Agent::Opencode, vec![]);
    warn.warnings = vec!["opencode: db locked".into()];

    let scanner = FakeScanner { items: vec![cand(r"C:\dev\a", 1, None)], warnings: vec!["scan: access denied".into()] };
    let dyns: Vec<&dyn SessionSource> = vec![&ok, &bad, &warn];
    let r = analyze(&cfg(), &scanner, None, &dyns, &FakeGit::default());
    assert_eq!(only(&r).status, ProjectStatus::Inactive);
    assert!(r.warnings.contains(&"scan: access denied".to_string()));
    assert!(r.warnings.contains(&"claude: file X corrupt".to_string()));
    assert!(r.warnings.contains(&"opencode: db locked".to_string()));
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
        reason: "Inactive for 30 days (last Claude session)".into(),
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

/// Every artifact of `rep`, as ticked folder by folder.
fn folders(rep: &ProjectReport) -> Selection {
    Selection { artifacts: rep.artifacts.iter().map(|a| a.id.clone()).collect(), worktrees: vec![] }
}

fn whole(rep: &ProjectReport) -> Selection {
    Selection { artifacts: vec![], worktrees: vec![rep.id.clone()] }
}

#[test]
fn plan_normal_project_has_one_item_per_artifact() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[100, 50]);
    let plan = build_plan(std::slice::from_ref(&rep), &folders(&rep), RemoveMode::Recycle).unwrap();
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
fn plan_takes_only_the_ticked_folders() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(30), false)]);
    let mut c = cand(r"C:\dev\app", 500, None);
    c.artifacts.push(node_modules(r"C:\dev\app", 900));
    let r = run(vec![c], &[&src], &FakeGit::default());
    let target = r.reports[0].artifacts.iter().find(|a| a.path.ends_with("target")).unwrap();
    let sel = Selection { artifacts: vec![target.id.clone()], ..Default::default() };
    let plan = build_plan(&r.reports, &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].path, PathBuf::from(r"C:\dev\app\target"));
    assert_eq!(plan.expected_bytes, 500);
    // An Optional (or Caution) folder is planned when the user ticks it.
    let nm = r.reports[0].artifacts.iter().find(|a| a.path.ends_with("node_modules")).unwrap();
    let sel = Selection { artifacts: vec![nm.id.clone()], ..Default::default() };
    let plan = build_plan(&r.reports, &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].kind, ArtifactKind::NodeModules);
}

#[test]
fn plan_recent_and_no_sessions_are_selectable() {
    let a = report(r"C:\dev\a", ProjectStatus::Recent, &[10]);
    let b = report(r"C:\dev\b", ProjectStatus::NoSessions, &[20]);
    let sel = Selection { artifacts: vec![a.artifacts[0].id.clone(), b.artifacts[0].id.clone()], ..Default::default() };
    let plan = build_plan(&[a, b], &sel, RemoveMode::Permanent).unwrap();
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
    let plan = build_plan(std::slice::from_ref(&rep), &whole(&rep), RemoveMode::Permanent).unwrap();
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
    let plan = build_plan(std::slice::from_ref(&rep), &whole(&rep), RemoveMode::Recycle).unwrap();
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
        let plan = build_plan(std::slice::from_ref(&rep), &whole(&rep), RemoveMode::Recycle).unwrap();
        assert_eq!(plan.items.len(), 2, "{}", rep.name);
        assert!(plan.items.iter().all(|i| i.action == PlanAction::RemoveDir), "{}", rep.name);
        // Folder by folder works too for these.
        let plan = build_plan(std::slice::from_ref(&rep), &folders(&rep), RemoveMode::Recycle).unwrap();
        assert_eq!(plan.items.len(), 2, "{}", rep.name);
    }
}

#[test]
fn plan_worktree_fallback_plans_only_recommended_folders() {
    // Not removable whole (unknown main worktree): a worktree tick must not plan folders that need
    // a tick of their own (Optional/Caution).
    let mut rep = worktree_report(r"C:\wt\a", None, Some(false), ProjectStatus::Inactive);
    let mut nm = node_modules(r"C:\wt\a", 900);
    nm.level = Level::Optional;
    let mut tracked = art("C:\\wt\\a", "build", 40, true);
    tracked.level = Level::Caution;
    rep.artifacts.extend([nm.clone(), tracked.clone()]);
    let plan = build_plan(std::slice::from_ref(&rep), &whole(&rep), RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 2, "{:?}", plan.items);
    assert!(plan.items.iter().all(|i| i.path != nm.path && i.path != tracked.path));
    // Ticked folder by folder they are planned.
    let sel = Selection { artifacts: vec![nm.id.clone(), tracked.id.clone()], ..Default::default() };
    let plan = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 2);
}

#[test]
fn plan_refuses_a_folder_of_a_worktree_removed_whole() {
    let rep = worktree_report(r"C:\wt\a", Some(r"C:\dev\proj"), Some(false), ProjectStatus::Inactive);
    let err = build_plan(std::slice::from_ref(&rep), &folders(&rep), RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("only be removed whole"), "{err}");
}

#[test]
fn plan_worktrees_must_be_agent_worktrees() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]);
    let err = build_plan(std::slice::from_ref(&rep), &whole(&rep), RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("not an agent worktree"), "{err}");
    let sel = Selection { worktrees: vec!["c:\\nope".into()], ..Default::default() };
    let err = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
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
    for (rep, want) in [(&clean, true), (&empty_clean, true), (&dirty, false), (&unknown, false)] {
        let plan = build_plan(std::slice::from_ref(rep), &whole(rep), RemoveMode::Recycle).unwrap();
        assert_eq!(plans_something(rep), want, "{}", rep.name);
        assert_eq!(!plan.items.is_empty(), want, "{}", rep.name);
    }
    let mut project = report(r"C:\dev\app", ProjectStatus::Inactive, &[]);
    assert!(!plans_something(&project));
    let plan = build_plan(std::slice::from_ref(&project), &folders(&project), RemoveMode::Recycle).unwrap();
    assert!(plan.items.is_empty());
    project.artifacts = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]).artifacts;
    assert!(plans_something(&project));
    let plan = build_plan(std::slice::from_ref(&project), &folders(&project), RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(whole_worktree_repo(&clean), Some(&PathBuf::from(r"C:\dev\proj")));
    assert_eq!(whole_worktree_repo(&dirty), None);
}

#[test]
fn plan_rejects_unknown_id() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]);
    let sel = Selection { artifacts: vec!["c:\\nope".into()], ..Default::default() };
    let err = build_plan(&[rep], &sel, RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("Folder not found in the analysis"), "{err}");
    assert_eq!(err.to_string(), "Folder not found in the analysis: c:\\nope");
}

#[test]
fn plan_rejects_in_use_blocked_and_unreachable() {
    for (status, needle) in [
        (ProjectStatus::InUse, "in use"),
        (ProjectStatus::Blocked, "blocked"),
        (ProjectStatus::Unreachable, "reachable"),
    ] {
        let rep = report(r"C:\dev\app", status, &[1]);
        let ok = report(r"C:\dev\ok", ProjectStatus::Inactive, &[1]);
        let ids = vec![ok.artifacts[0].id.clone(), rep.artifacts[0].id.clone()];
        let sel = Selection { artifacts: ids, ..Default::default() };
        let err = build_plan(&[ok.clone(), rep.clone()], &sel, RemoveMode::Recycle).unwrap_err();
        assert!(err.to_string().contains(needle), "{status:?}: {err}");
        // The same through the worktree list.
        let mut wt = rep.clone();
        wt.is_agent_worktree = true;
        let err = build_plan(std::slice::from_ref(&wt), &whole(&wt), RemoveMode::Recycle).unwrap_err();
        assert!(err.to_string().contains(needle), "{status:?}: {err}");
    }
}

#[test]
fn plan_rejects_a_folder_of_a_project_with_a_live_session() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\app", ago(60), true)]);
    let r = run(vec![cand(r"C:\dev\app", 500, None)], &[&src], &FakeGit::default());
    let err = build_plan(&r.reports, &folders(&r.reports[0]), RemoveMode::Recycle).unwrap_err();
    assert!(err.to_string().contains("in use"), "{err}");
}

#[test]
fn plan_rejects_report_with_blocked_by_even_if_status_looks_fine() {
    let mut rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[1]);
    rep.blocked_by = vec![BlockReason::TrackedArtifact];
    assert!(build_plan(std::slice::from_ref(&rep), &folders(&rep), RemoveMode::Recycle).is_err());
}

#[test]
fn plan_dedups_ids_and_paths_and_ids_are_unique() {
    let rep = report(r"C:\dev\app", ProjectStatus::Inactive, &[100]);
    let id = rep.artifacts[0].id.clone();
    let sel = Selection { artifacts: vec![id.clone(), id], ..Default::default() };
    let p1 = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap();
    let p2 = build_plan(std::slice::from_ref(&rep), &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(p1.items.len(), 1);
    assert_eq!(p1.expected_bytes, 100);
    assert_ne!(p1.id, p2.id);

    // A worktree falling back to its folders, its folders ticked too: each path once.
    let wt = worktree_report(r"C:\wt\a", None, Some(false), ProjectStatus::Inactive);
    let sel = Selection { worktrees: vec![wt.id.clone(), wt.id.clone()], ..folders(&wt) };
    let plan = build_plan(std::slice::from_ref(&wt), &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.expected_bytes, 1000);
}

#[test]
fn plan_empty_selection_is_empty_plan() {
    let plan = build_plan(&[], &Selection::default(), RemoveMode::Recycle).unwrap();
    assert!(plan.items.is_empty());
    assert_eq!(plan.expected_bytes, 0);
}

// ───────────────────────── analyze -> plan end to end ───────────────────────

#[test]
fn analyze_then_plan_roundtrip() {
    let src = FakeSource::new(Agent::Claude, vec![sess(Agent::Claude, r"C:\dev\old", ago(100), false)]);
    let mut old = cand(r"C:\dev\old", 900, None);
    old.artifacts.push(node_modules(r"C:\dev\old", 300));
    let r = run(
        vec![old, cand(r"C:\dev\fresh", 5, Some(ago(1)))],
        &[&src],
        &FakeGit::default().with_state(r"C:\dev\old", clean_state()),
    );
    let pre: Vec<String> =
        r.reports.iter().flat_map(|p| &p.artifacts).filter(|a| a.preselected).map(|a| a.id.clone()).collect();
    assert_eq!(pre.len(), 1, "target of the old project only, not its node_modules");
    let sel = Selection { artifacts: pre, ..Default::default() };
    let plan = build_plan(&r.reports, &sel, RemoveMode::Recycle).unwrap();
    assert_eq!(plan.expected_bytes, 900);
}

#[test]
fn clean_agent_worktree_keeps_the_project_preselection_rule() {
    // Its only folder is Optional, but an inactive clean agent worktree is preselected as a whole.
    let root = r"C:\Users\u\.codex\worktrees\ab12\proj";
    let mut src = FakeSource::new(Agent::Codex, vec![sess(Agent::Codex, root, ago(40), false)]);
    src.worktrees = vec![PathBuf::from(r"C:\Users\u\.codex\worktrees")];
    let mut c = cand(root, 100, None);
    c.artifacts = vec![node_modules(root, 100)];
    let git = FakeGit::default().with_state(
        root,
        GitState { is_linked_worktree: true, main_worktree: Some(PathBuf::from(r"C:\dev\proj")), ..clean_state() },
    );
    let r = run(vec![c], &[&src], &git);
    let p = only(&r);
    assert!(p.is_agent_worktree);
    assert_eq!(p.status, ProjectStatus::Inactive);
    assert!(p.preselected);
    assert!(!p.artifacts[0].preselected);
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
