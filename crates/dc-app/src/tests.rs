use super::*;
use dc_model::{
    Artifact, ArtifactKind, GitState, IndexedDir, ItemOutcome, PlanAction, PlanItem, ProjectCandidate, ProjectStatus,
    SizeInfo,
};
use std::path::Path;
use std::sync::Mutex;

fn report(root: &str, agent_worktree: bool, linked: bool) -> ProjectReport {
    let root = PathBuf::from(root);
    ProjectReport {
        id: path_key(&root),
        name: root.file_name().unwrap().to_string_lossy().into_owned(),
        ecosystems: vec!["rust".into()],
        agents: vec![],
        last_activity_ms: None,
        last_activity_source: None,
        inactive_days: Some(100),
        status: ProjectStatus::Inactive,
        blocked_by: vec![],
        reason: "Inattivo da 100 giorni".into(),
        artifacts: vec![Artifact {
            path: root.join("target"),
            kind: ArtifactKind::RustTarget,
            label: "Rust target/".into(),
            size: SizeInfo { logical_bytes: 10 << 20, allocated_bytes: None, file_count: 3 },
            verified: true,
        }],
        reclaimable_bytes: 10 << 20,
        preselected: true,
        is_agent_worktree: agent_worktree,
        git: GitState { is_repo: true, is_linked_worktree: linked, ..Default::default() },
        root,
    }
}

fn item(path: &Path, kind: ArtifactKind) -> PlanItem {
    PlanItem {
        path: path.to_path_buf(),
        kind,
        action: PlanAction::RemoveDir,
        expected_bytes: 1,
        project_id: String::new(),
        reason: String::new(),
    }
}

fn plan_of(items: Vec<PlanItem>) -> CleanPlan {
    CleanPlan { id: "t".into(), mode: RemoveMode::Permanent, items, expected_bytes: 0 }
}

// ───────────────────────────── execute_plan allow-list ─────────────────────────────

#[test]
fn execute_plan_refuses_paths_not_in_the_last_scan() {
    let scan = DevScan {
        projects: vec![report(r"C:\dc-app-test-missing\proj", false, false)],
        worktrees: vec![report(r"C:\dc-app-test-missing\wt", true, true)],
        other_worktrees: vec![report(r"C:\dc-app-test-missing\other", false, true)],
        ..Default::default()
    };
    let refused = [
        // Never seen by the scan.
        item(Path::new(r"C:\dc-app-test-missing\elsewhere\target"), ArtifactKind::RustTarget),
        // The root of an ordinary project: only its artifacts are ever removed.
        item(Path::new(r"C:\dc-app-test-missing\proj"), ArtifactKind::WorktreeDir),
        // Worktrees that are not the agents' own are shown, never cleaned.
        item(Path::new(r"C:\dc-app-test-missing\other\target"), ArtifactKind::RustTarget),
        item(Path::new(r"C:\dc-app-test-missing\other"), ArtifactKind::WorktreeDir),
    ];
    for bad in refused {
        let shown = bad.path.display().to_string();
        let ok = item(Path::new(r"C:\dc-app-test-missing\proj\target"), ArtifactKind::RustTarget);
        let err = execute_plan(&scan, &plan_of(vec![ok, bad])).expect_err(&shown);
        assert!(err.contains("rifiutato"), "{shown}: {err}");
        assert!(err.contains(&shown), "{shown}: {err}");
    }
}

#[test]
fn execute_plan_passes_scanned_paths_to_the_remover() {
    // The paths do not exist: the remover reports them as skipped and touches nothing.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("missing");
    let proj = base.join("proj");
    let wt = base.join("wt");
    let scan = DevScan {
        projects: vec![report(proj.to_str().unwrap(), false, false)],
        worktrees: vec![report(wt.to_str().unwrap(), true, true)],
        ..Default::default()
    };
    // Different case and a trailing separator: compared through `path_key`.
    let upper = PathBuf::from(format!("{}\\", proj.join("TARGET").display()));
    let plan = plan_of(vec![item(&upper, ArtifactKind::RustTarget), item(&wt, ArtifactKind::WorktreeDir)]);
    let res = execute_plan(&scan, &plan).expect("allowed");
    assert_eq!(res.items.len(), 2);
    assert!(!res.dry_run);
    assert!(res.items.iter().all(|i| matches!(i.outcome, ItemOutcome::Skipped { .. })), "{res:?}");
}

// ───────────────────────────── archived sessions ─────────────────────────────

#[derive(Default)]
struct FakeCli {
    calls: Mutex<Vec<String>>,
}

impl HarnessCli for FakeCli {
    fn codex_delete(&self, id: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("codex_delete {id}"));
        Ok(())
    }
    fn opencode_export(&self, id: &str, _out: &Path) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("opencode_export {id}"));
        Ok(())
    }
    fn opencode_delete(&self, id: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("opencode_delete {id}"));
        Ok(())
    }
}

#[derive(Default)]
struct FakeTrash {
    calls: Mutex<Vec<PathBuf>>,
}

impl Trash for FakeTrash {
    fn to_bin(&self, path: &Path) -> Result<(), String> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

fn archived(agent: Agent, id: &str, blocked: Option<&str>) -> ArchivedSession {
    ArchivedSession {
        agent,
        id: id.into(),
        title: None,
        cwd: None,
        updated_ms: None,
        file: None,
        bytes: 0,
        blocked: blocked.map(Into::into),
    }
}

fn pick(agent: Agent, id: &str) -> (Agent, String) {
    (agent, id.to_string())
}

#[test]
fn delete_archived_refuses_unknown_or_blocked_ids() {
    let scan = DevScan {
        archived: vec![
            archived(Agent::Codex, "free", None),
            archived(Agent::Codex, "busy", Some("sessione in uso")),
        ],
        ..Default::default()
    };
    let cli = FakeCli::default();
    let trash = FakeTrash::default();
    let refused: [Vec<(Agent, String)>; 4] = [
        vec![pick(Agent::Codex, "unknown")],
        // Same id, other harness: not the session the scan listed.
        vec![pick(Agent::Claude, "free")],
        vec![pick(Agent::Codex, "busy")],
        // One bad pick refuses the whole request, before anything is deleted.
        vec![pick(Agent::Codex, "free"), pick(Agent::Codex, "busy")],
    ];
    for picked in refused {
        let res = delete_archived(&scan, &picked, RemoveMode::Permanent, &cli, &trash);
        assert!(res.is_err(), "{picked:?}");
    }
    assert!(cli.calls.lock().unwrap().is_empty());
    assert!(trash.calls.lock().unwrap().is_empty());
}

#[test]
fn delete_archived_deletes_listed_sessions_once() {
    let scan = DevScan { archived: vec![archived(Agent::Codex, "free", None)], ..Default::default() };
    let cli = FakeCli::default();
    let trash = FakeTrash::default();
    let picked = [pick(Agent::Codex, "free"), pick(Agent::Codex, "free")];
    let out = delete_archived(&scan, &picked, RemoveMode::Permanent, &cli, &trash).unwrap();
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0].outcome, ItemOutcome::Removed { .. }), "{:?}", out[0].outcome);
    assert_eq!(*cli.calls.lock().unwrap(), vec!["codex_delete free".to_string()]);
}

// ───────────────────────────── reports split ─────────────────────────────

#[test]
fn reports_are_split_into_projects_worktrees_and_other_worktrees() {
    let reports = vec![
        report(r"C:\x\plain", false, false),
        report(r"C:\x\agent-linked", true, true),
        report(r"C:\x\agent-unlinked", true, false),
        report(r"C:\x\user-worktree", false, true),
    ];
    let (projects, worktrees, others) = split_reports(reports);
    let names = |v: &[ProjectReport]| v.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&projects), ["plain"]);
    assert_eq!(names(&worktrees), ["agent-linked", "agent-unlinked"]);
    assert_eq!(names(&others), ["user-worktree"]);
}

#[test]
fn build_plan_ignores_worktrees_that_are_not_the_agents() {
    let scan = DevScan {
        projects: vec![report(r"C:\x\plain", false, false)],
        other_worktrees: vec![report(r"C:\x\user-worktree", false, true)],
        ..Default::default()
    };
    let plain = path_key(Path::new(r"C:\x\plain"));
    let plan = build_plan(&scan, std::slice::from_ref(&plain), RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.mode, RemoveMode::Recycle);
    let other = path_key(Path::new(r"C:\x\user-worktree"));
    assert!(build_plan(&scan, &[other], RemoveMode::Recycle).is_err());
}

// ───────────────────────────── roots ─────────────────────────────

fn settings_walking(roots: Vec<PathBuf>) -> Settings {
    Settings {
        session_projects: false,
        roots,
        scan_roots: true,
        inactive_days: 14,
        remove_mode: RemoveMode::Recycle,
    }
}

#[test]
fn plan_roots_keeps_only_the_outer_root() {
    let tmp = tempfile::tempdir().unwrap();
    let outer = tmp.path().join("outer");
    let inner = outer.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let plan = plan_roots(&settings_walking(vec![inner.clone(), outer.clone()]), &[], None);
    assert_eq!(plan.roots, vec![normalize_path(&outer)]);
}

#[test]
fn plan_roots_drops_a_session_root_inside_a_walked_folder_and_missing_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let outer = tmp.path().join("outer");
    let proj = outer.join("proj");
    // Its own `.git`: the temp folder may itself lie inside a repository.
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let wt = tmp.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let sessions = [Preloaded {
        agent: Agent::Claude,
        loaded: Loaded {
            items: vec![SessionRecord {
                agent: Agent::Claude,
                session_id: None,
                project_path: proj.clone(),
                last_activity_ms: 0,
                archived: false,
                live: false,
            }],
            warnings: vec![],
        },
        worktrees: vec![wt.clone(), tmp.path().join("gone")],
    }];
    let mut settings = settings_walking(vec![outer.clone()]);
    settings.session_projects = true;
    let plan = plan_roots(&settings, &sessions, None);
    assert_eq!(plan.from_sessions.roots, vec![normalize_path(&proj)]);
    let mut want = vec![normalize_path(&outer), normalize_path(&wt)];
    want.sort_by_key(|p| path_key(p));
    assert_eq!(plan.roots, want);
}

#[test]
fn key_within_is_component_aware() {
    assert!(key_within(r"c:\a\b", r"c:\a"));
    assert!(key_within(r"c:\a", r"c:\a"));
    assert!(key_within(r"c:\a", r"c:\"));
    assert!(!key_within(r"c:\ab", r"c:\a"));
}

// ───────────────────────────── index / walker split ─────────────────────────────

struct FakeIndex {
    known: Vec<String>,
}

impl DirIndex for FakeIndex {
    fn backend_name(&self) -> &'static str {
        "fake"
    }
    fn size_of(&self, dir: &Path) -> Option<SizeInfo> {
        self.known.contains(&path_key(dir)).then(SizeInfo::default)
    }
    fn find_dirs_named(&self, _names: &[&str], _roots: &[PathBuf]) -> Vec<IndexedDir> {
        Vec::new()
    }
}

#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<(Vec<PathBuf>, bool)>>,
}

impl ProjectScanner for Recording {
    fn scan(&self, roots: &[PathBuf], _opts: &ScanOptions, index: Option<&dyn DirIndex>) -> Loaded<ProjectCandidate> {
        self.calls.lock().unwrap().push((roots.to_vec(), index.is_some()));
        Loaded {
            items: roots
                .iter()
                .map(|r| ProjectCandidate {
                    root: r.clone(),
                    name: String::new(),
                    ecosystems: vec![],
                    artifacts: vec![],
                    source_mtime_ms: None,
                })
                .collect(),
            warnings: vec![format!("{} roots", roots.len())],
        }
    }
}

#[test]
fn roots_missing_from_the_index_are_walked() {
    let indexed = PathBuf::from(r"C:\dev\indexed");
    let link = PathBuf::from(r"C:\dev\onedrive-or-unreadable");
    let index = FakeIndex { known: vec![path_key(&indexed)] };
    let rec = Recording::default();
    let out = SplitScanner { inner: &rec }.scan(
        &[link.clone(), indexed.clone()],
        &ScanOptions::default(),
        Some(&index),
    );
    assert_eq!(*rec.calls.lock().unwrap(), vec![(vec![indexed], true), (vec![link], false)]);
    assert_eq!(out.items.len(), 2);
    assert_eq!(out.warnings, ["1 roots", "1 roots"]);
}

#[test]
fn without_an_index_every_root_is_walked_once() {
    let roots = vec![PathBuf::from(r"C:\dev\a"), PathBuf::from(r"C:\dev\b")];
    let rec = Recording::default();
    let out = SplitScanner { inner: &rec }.scan(&roots, &ScanOptions::default(), None);
    assert_eq!(*rec.calls.lock().unwrap(), vec![(roots, false)]);
    assert_eq!(out.items.len(), 2);
}

// ───────────────────────────── settings ─────────────────────────────

#[test]
fn default_settings_recycle_and_sessions_only() {
    let s = default_settings();
    assert_eq!(s.remove_mode, RemoveMode::Recycle);
    assert!(s.session_projects);
    assert!(!s.scan_roots);
    assert_eq!(s.inactive_days, dc_model::DEFAULT_INACTIVE_DAYS);
    assert!(!s.roots.is_empty());
}

#[test]
fn settings_and_cache_live_under_pulisci_pc() {
    assert!(settings_path().ends_with(Path::new("PulisciPC").join("settings.json")));
    assert!(cache_dir().ends_with(Path::new("PulisciPC").join("mukade")));
}

#[test]
fn settings_round_trip_and_fall_back_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested").join("settings.json");
    assert_eq!(read_settings(&path).remove_mode, RemoveMode::Recycle);
    let mut s = settings_walking(vec![PathBuf::from(r"D:\Projects")]);
    s.inactive_days = 30;
    s.remove_mode = RemoveMode::Permanent;
    write_settings(&path, &s).unwrap();
    let back = read_settings(&path);
    assert_eq!(back.inactive_days, 30);
    assert_eq!(back.remove_mode, RemoveMode::Permanent);
    assert_eq!(back.roots, vec![PathBuf::from(r"D:\Projects")]);
    assert!(back.scan_roots);
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(read_settings(&path).remove_mode, RemoveMode::Recycle);
}
