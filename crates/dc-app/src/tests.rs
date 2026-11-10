use super::*;
use dc_model::{
    Artifact, ArtifactKind, Class, GitState, IndexedDir, ItemOutcome, Level, PlanAction, PlanItem, ProjectCandidate,
    ProjectStatus, SizeInfo,
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
            id: path_key(&root.join("target")),
            path: root.join("target"),
            kind: ArtifactKind::RustTarget,
            label: "Rust target/".into(),
            size: SizeInfo { logical_bytes: 10 << 20, allocated_bytes: None, file_count: 3 },
            verified: true,
            class: Class::Build,
            level: Level::Recommended,
            note: String::new(),
            ambiguous: false,
            preselected: true,
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

/// A plan item as a caller would send it for `project` (by root).
fn item_for(project: &Path, path: &Path, kind: ArtifactKind, action: PlanAction) -> PlanItem {
    PlanItem { action, project_id: path_key(project), ..item(path, kind) }
}

/// An agent worktree the engine plans as a whole (`GitWorktreeRemove { repo }`): inactive,
/// known clean, with a main worktree other than itself.
fn whole_worktree(root: &str, repo: &str) -> ProjectReport {
    let mut r = report(root, true, true);
    r.git.dirty = Some(false);
    r.git.main_worktree = Some(PathBuf::from(repo));
    r
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
        scanned_at_ms: now_ms(),
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
    let repo = base.join("repo");
    let scan = DevScan {
        projects: vec![report(proj.to_str().unwrap(), false, false)],
        worktrees: vec![whole_worktree(wt.to_str().unwrap(), repo.to_str().unwrap())],
        scanned_at_ms: now_ms(),
        ..Default::default()
    };
    // Different case and a trailing separator: compared through `path_key`.
    let upper = PathBuf::from(format!("{}\\", proj.join("TARGET").display()));
    let repo_upper = PathBuf::from(repo.to_string_lossy().to_uppercase());
    let plan = plan_of(vec![
        item_for(&proj, &upper, ArtifactKind::RustTarget, PlanAction::RemoveDir),
        item_for(&wt, &wt, ArtifactKind::WorktreeDir, PlanAction::GitWorktreeRemove { repo: repo_upper }),
    ]);
    let res = execute_plan(&scan, &plan).expect("allowed");
    assert!(!res.dry_run);
    // The canonical paths are executed, not the caller's spelling.
    let paths: Vec<_> = res.items.iter().map(|i| i.path.clone()).collect();
    assert_eq!(paths, vec![proj.join("target"), wt.clone()]);
    assert!(res.items.iter().all(|i| matches!(i.outcome, ItemOutcome::Skipped { .. })), "{res:?}");
}

/// Agent worktree planned whole + ordinary project; all under a folder that does not exist.
fn canonical_scan() -> (DevScan, PathBuf, PathBuf) {
    let base = PathBuf::from(r"C:\dc-app-test-missing");
    let (proj, wt, repo) = (base.join("proj"), base.join("wt"), base.join("repo"));
    let scan = DevScan {
        projects: vec![report(proj.to_str().unwrap(), false, false)],
        worktrees: vec![whole_worktree(wt.to_str().unwrap(), repo.to_str().unwrap())],
        scanned_at_ms: now_ms(),
        ..Default::default()
    };
    (scan, proj, wt)
}

#[derive(Default)]
struct CountingRemover {
    runs: Mutex<usize>,
}

impl Remover for CountingRemover {
    /// Counts the runs; always a dry run, so a test can never remove anything.
    fn execute(&self, plan: &CleanPlan, _dry_run: bool) -> CleanResult {
        *self.runs.lock().unwrap() += 1;
        dc_cleaner::DefaultRemover.execute(plan, true)
    }
}

#[test]
fn a_plan_from_an_old_analysis_is_refused_before_anything_runs() {
    let (mut scan, proj, _) = canonical_scan();
    let t = 1_800_000_000_000;
    scan.scanned_at_ms = t;
    let plan = plan_of(vec![item_for(&proj, &proj.join("target"), ArtifactKind::RustTarget, PlanAction::RemoveDir)]);
    let remover = CountingRemover::default();

    // Dialog opened at T, «Continua» pressed 31 minutes later.
    let err = execute_plan_with(&scan, &plan, t + 31 * 60 * 1000, &remover).unwrap_err();
    assert_eq!(err, STALE_ANALYSIS);
    assert_eq!(err, "L'analisi è troppo vecchia: premi «Analizza» di nuovo.");
    assert_eq!(*remover.runs.lock().unwrap(), 0, "nothing executed");
    assert_eq!(execute_plan_at(&scan, &plan, t + 31 * 60 * 1000).unwrap_err(), STALE_ANALYSIS);
    // Unknown age: refused too.
    let unknown = DevScan { scanned_at_ms: 0, ..scan.clone() };
    assert_eq!(execute_plan_with(&unknown, &plan, t, &remover).unwrap_err(), STALE_ANALYSIS);
    assert_eq!(*remover.runs.lock().unwrap(), 0);

    // Within the half hour the plan runs (the paths do not exist: nothing is touched).
    execute_plan_with(&scan, &plan, t + 29 * 60 * 1000, &remover).expect("fresh enough");
    assert_eq!(*remover.runs.lock().unwrap(), 1);
}

/// Refused by both `checked_plan` and `execute_plan`, with the same "rifiutato" error.
fn refused(scan: &DevScan, items: Vec<PlanItem>) -> String {
    let err = checked_plan(scan, &plan_of(items.clone())).expect_err("must be refused");
    assert!(err.contains("rifiutato"), "{err}");
    assert_eq!(execute_plan(scan, &plan_of(items)).unwrap_err(), err);
    err
}

#[test]
fn a_canonical_plan_and_any_subset_of_it_are_accepted() {
    let (scan, proj, wt) = canonical_scan();
    let ids = [path_key(&proj), path_key(&wt)];
    let canonical = build_plan(&scan, &ids, RemoveMode::Recycle).unwrap();
    assert_eq!(canonical.items.len(), 2);
    let checked = checked_plan(&scan, &canonical).unwrap();
    assert_eq!(checked.id, canonical.id);
    assert_eq!(checked.mode, RemoveMode::Recycle);
    assert_eq!(checked.expected_bytes, canonical.expected_bytes);
    let key = |v: &[PlanItem]| v.iter().map(|i| (path_key(&i.path), i.kind, i.action.clone())).collect::<Vec<_>>();
    assert_eq!(key(&checked.items), key(&canonical.items));
    // Only the worktree, sent twice: executed once.
    let wt_item = canonical.items.iter().find(|i| i.path == wt).unwrap().clone();
    let one = checked_plan(&scan, &plan_of(vec![wt_item.clone(), wt_item])).unwrap();
    assert_eq!(one.items.len(), 1);
}

#[test]
fn execute_plan_refuses_a_swapped_action_or_kind() {
    let (scan, proj, wt) = canonical_scan();
    // The worktree root as a plain delete: git would keep a dangling worktree entry.
    let err = refused(&scan, vec![item_for(&wt, &wt, ArtifactKind::WorktreeDir, PlanAction::RemoveDir)]);
    assert!(err.contains("non corrisponde all'ultima analisi"), "{err}");
    let target = proj.join("target");
    // Another kind for a real artifact (the remover checks the folder name against the kind).
    refused(&scan, vec![item_for(&proj, &target, ArtifactKind::NodeModules, PlanAction::RemoveDir)]);
    // A worktree removal on an artifact path.
    let repo = PathBuf::from(r"C:\dc-app-test-missing\repo");
    let action = PlanAction::GitWorktreeRemove { repo };
    refused(&scan, vec![item_for(&proj, &target, ArtifactKind::RustTarget, action)]);
    // The artifact of a worktree planned whole is not a canonical item.
    refused(&scan, vec![item_for(&wt, &wt.join("target"), ArtifactKind::RustTarget, PlanAction::RemoveDir)]);
}

#[test]
fn execute_plan_refuses_a_wrong_repo() {
    let (scan, _, wt) = canonical_scan();
    let action = PlanAction::GitWorktreeRemove { repo: PathBuf::from(r"C:\dc-app-test-missing\other-repo") };
    let err = refused(&scan, vec![item_for(&wt, &wt, ArtifactKind::WorktreeDir, action)]);
    assert!(err.contains("non corrisponde all'ultima analisi"), "{err}");
}

#[test]
fn execute_plan_refuses_artifacts_of_projects_in_use_or_blocked() {
    let (mut scan, _, _) = canonical_scan();
    let mut busy = report(r"C:\dc-app-test-missing\busy", false, false);
    busy.status = ProjectStatus::InUse;
    busy.preselected = false;
    let mut locked = report(r"C:\dc-app-test-missing\locked", false, false);
    locked.status = ProjectStatus::Blocked;
    locked.blocked_by = vec![dc_model::BlockReason::TrackedArtifact];
    locked.preselected = false;
    scan.projects.extend([busy.clone(), locked.clone()]);
    for r in [&busy, &locked] {
        let a = &r.artifacts[0];
        refused(&scan, vec![item_for(&r.root, &a.path, a.kind, PlanAction::RemoveDir)]);
    }
}

#[test]
fn execute_plan_refuses_a_child_path_or_a_forged_project_id() {
    let (scan, proj, wt) = canonical_scan();
    let child = proj.join("target").join("debug");
    let err = refused(&scan, vec![item_for(&proj, &child, ArtifactKind::RustTarget, PlanAction::RemoveDir)]);
    assert!(err.contains(&child.display().to_string()), "{err}");
    // A real artifact filed under another project, or under no known project.
    let target = proj.join("target");
    refused(&scan, vec![item_for(&wt, &target, ArtifactKind::RustTarget, PlanAction::RemoveDir)]);
    refused(&scan, vec![item(&target, ArtifactKind::RustTarget)]);
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
fn delete_archived_refuses_a_session_listed_twice() {
    let scan = DevScan {
        archived: vec![archived(Agent::Opencode, "twin", None), archived(Agent::Opencode, "twin", None)],
        ..Default::default()
    };
    let cli = FakeCli::default();
    let trash = FakeTrash::default();
    let picked = [pick(Agent::Opencode, "twin")];
    let err = delete_archived(&scan, &picked, RemoveMode::Permanent, &cli, &trash).unwrap_err();
    assert!(err.contains("rifiutata"), "{err}");
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

fn with_file(mut s: ArchivedSession, file: &str) -> ArchivedSession {
    s.file = Some(PathBuf::from(file));
    s
}

#[test]
fn revalidation_keeps_only_sessions_still_archived_and_free() {
    // The kept session's rollout must exist: dc-sessions refuses one that vanished.
    let td = tempfile::tempdir().unwrap();
    let fine = td.path().join("fine.jsonl");
    std::fs::write(&fine, "x").unwrap();
    let fine_other_spelling = fine.to_string_lossy().to_uppercase().replace('\\', "/");
    let scan = DevScan {
        archived: vec![
            with_file(archived(Agent::Codex, "fine", None), fine.to_str().unwrap()),
            with_file(archived(Agent::Codex, "reopened", None), r"C:\s\reopened.jsonl"),
            with_file(archived(Agent::Codex, "gone", None), r"C:\s\gone.jsonl"),
            with_file(archived(Agent::Codex, "moved", None), r"C:\s\moved.jsonl"),
            archived(Agent::Opencode, "oc", None),
        ],
        ..Default::default()
    };
    let calls = Mutex::new(Vec::new());
    let fresh = |agent: Agent| {
        calls.lock().unwrap().push(agent);
        match agent {
            Agent::Codex => vec![
                with_file(archived(Agent::Codex, "fine", None), &fine_other_spelling),
                // Reopened (live) or with an open descendant since the analysis.
                with_file(archived(Agent::Codex, "reopened", Some("sessione in uso")), r"C:\s\reopened.jsonl"),
                // Continued into a new rollout: not the session the analysis saw.
                with_file(archived(Agent::Codex, "moved", None), r"C:\s\moved-2.jsonl"),
            ],
            // A failed or empty listing: nothing of that harness may go.
            _ => vec![],
        }
    };
    let picked = [
        pick(Agent::Codex, "fine"),
        pick(Agent::Codex, "reopened"),
        pick(Agent::Codex, "gone"),
        pick(Agent::Codex, "moved"),
        pick(Agent::Opencode, "oc"),
        pick(Agent::Codex, "fine"),
    ];
    let r = revalidate_archived(&scan, &picked, &fresh).unwrap();
    assert_eq!(r.keep, vec![pick(Agent::Codex, "fine")]);
    let skipped: Vec<&str> = r.skipped.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(skipped, vec!["reopened", "gone", "moved", "oc"]);
    for o in &r.skipped {
        assert!(
            matches!(&o.outcome, ItemOutcome::Skipped { reason } if reason == NO_LONGER_ARCHIVED),
            "{:?}",
            o.outcome
        );
    }
    assert_eq!(*calls.lock().unwrap(), vec![Agent::Codex, Agent::Opencode], "one listing per harness");

    // Only the kept picks reach the deletion.
    let cli = FakeCli::default();
    let trash = FakeTrash::default();
    let out = delete_archived(&scan, &r.keep, RemoveMode::Permanent, &cli, &trash).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(*cli.calls.lock().unwrap(), vec!["codex_delete fine".to_string()]);
}

#[test]
fn revalidation_refuses_a_pick_the_scan_never_listed() {
    let scan = DevScan { archived: vec![archived(Agent::Codex, "free", None)], ..Default::default() };
    let fresh = |_: Agent| vec![archived(Agent::Codex, "other", None)];
    let err = revalidate_archived(&scan, &[pick(Agent::Codex, "other")], &fresh).unwrap_err();
    assert!(err.contains("rifiutata"), "{err}");
}

#[test]
fn an_analysis_older_than_half_an_hour_is_stale() {
    let at = 1_800_000_000_000;
    let scan = DevScan { scanned_at_ms: at, ..Default::default() };
    assert!(!scan.is_stale(at));
    assert!(!scan.is_stale(at + MAX_SCAN_AGE_MS));
    assert!(scan.is_stale(at + MAX_SCAN_AGE_MS + 1));
    assert!(!scan.is_stale(at - 5_000), "a clock that went back is not a reason to refuse");
    assert!(DevScan::default().is_stale(at), "unknown age");
    assert_eq!(MAX_SCAN_AGE_MS, 30 * 60 * 1000);
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
fn a_project_tick_plans_only_its_recommended_folders() {
    let mut proj = report(r"C:\dc-app-test-missing\mixed", false, false);
    let folder = |name: &str, kind, class: Class, level| Artifact {
        id: path_key(&proj.root.join(name)),
        path: proj.root.join(name),
        kind,
        label: name.into(),
        class,
        level,
        preselected: false,
        ..proj.artifacts[0].clone()
    };
    let nm = folder("node_modules", ArtifactKind::NodeModules, Class::Dependency, Level::Optional);
    let tracked = folder("build", ArtifactKind::CMake, Class::Build, Level::Caution);
    proj.artifacts.extend([nm.clone(), tracked.clone()]);
    let scan = DevScan { projects: vec![proj.clone()], scanned_at_ms: now_ms(), ..Default::default() };

    let plan = build_plan(&scan, std::slice::from_ref(&proj.id), RemoveMode::Recycle).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].path, proj.root.join("target"));
    for a in [&nm, &tracked] {
        refused(&scan, vec![item_for(&proj.root, &a.path, a.kind, PlanAction::RemoveDir)]);
    }
    // Nothing Recommended: nothing to plan.
    let mut only_deps = proj.clone();
    only_deps.artifacts = vec![nm];
    assert!(!plans_something(&only_deps));
    assert!(plans_something(&proj));
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
    // Only folders under the user's home, never a fixed drive path.
    let home = path_key(&home_dir().expect("USERPROFILE"));
    assert!(s.roots.iter().all(|r| key_within(&path_key(r), &home) && path_key(r) != home), "{:?}", s.roots);
}

#[test]
fn default_roots_are_the_conventional_home_folders_that_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    assert!(default_settings_for(Some(home)).roots.is_empty(), "no home fallback");
    assert!(default_settings_for(None).roots.is_empty());
    let dirs: [&[&str]; 4] = [&["code"], &["source", "repos"], &["Documents", "GitHub"], &["Downloads"]];
    for d in dirs {
        std::fs::create_dir_all(d.iter().fold(home.to_path_buf(), |p, c| p.join(c))).unwrap();
    }
    std::fs::write(home.join("dev"), "a file, not a folder").unwrap();
    let want = vec![
        home.join("source").join("repos"),
        home.join("source"),
        home.join("code"),
        home.join("Documents").join("GitHub"),
    ];
    assert_eq!(default_settings_for(Some(home)).roots, want);
}

// ───────────────────────────── panics ─────────────────────────────

#[test]
fn a_panicking_index_build_falls_back_to_the_walker() {
    let mut warnings = vec!["before".to_string()];
    let ix: Option<u8> = guarded_index(|| panic!("mft"), &mut warnings);
    assert!(ix.is_none());
    assert_eq!(warnings, ["before", "Lettura veloce del disco non riuscita: uso la scansione normale."]);
    let ok = guarded_index(|| (Some(7u8), vec!["Unità E: non disponibile".to_string()]), &mut warnings);
    assert_eq!(ok, Some(7));
    assert_eq!(warnings.last().unwrap(), "Unità E: non disponibile");
}

#[test]
fn a_panicking_analysis_gives_no_reports_and_a_warning() {
    let r = guarded_analysis(|| panic!("engine"));
    assert!(r.reports.is_empty());
    assert_eq!(r.warnings, ["Errore interno durante l'analisi dei progetti."]);
    let ok = guarded_analysis(|| AnalysisResult { reports: vec![report(r"C:\x\p", false, false)], warnings: vec![] });
    assert_eq!(ok.reports.len(), 1);
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
