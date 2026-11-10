//! Combines scanner + sessions + git into [`ProjectReport`]s and builds [`CleanPlan`]s.
//!
//! The engine depends only on `dc-model`: every collaborator (scanner, directory index,
//! session stores, git) arrives as a trait object, so it is tested with hand-written fakes.
//!
//! # Rules (deliberate, see doc comments on [`analyze`] and [`build_plan`])
//!
//! * A session belongs to a project when its `project_path` equals the project root or is
//!   inside it (component-aware: `C:\a\b` does NOT contain `C:\a\bc`).
//! * Last activity is the newest of session activity, last git commit and newest source
//!   file mtime; the winner is reported as `last_activity_source`.
//! * Anything that could break a live process, or lose the work of an agent worktree, blocks
//!   removal; a folder that is merely doubtful (tracked, unverified, ambiguous name) stays
//!   selectable with a `Caution` level and a note, and is never preselected.

use dc_model::*;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DAY_MS: i64 = 86_400_000;
/// Fixed display/ordering of agents in `ProjectReport::agents`.
const AGENT_ORDER: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi];

// ───────────────────────────────── path helpers ─────────────────────────────────

fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// Component-aware "child is parent or inside parent" on `path_key` strings.
fn key_is_within(child: &str, parent: &str) -> bool {
    if parent.is_empty() {
        return false;
    }
    if child == parent {
        return true;
    }
    match child.strip_prefix(parent) {
        // `c:\a\b` vs `c:\a\bc` -> rest = "c", rejected. `c:\` (drive root) already ends
        // with a separator, so everything after it is inside.
        Some(rest) => parent.ends_with(is_sep) || rest.starts_with(is_sep),
        None => false,
    }
}

/// `path` relative to `root`, comparing components case-insensitively. `None` if outside.
fn relative_to(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = normalize_path(root);
    let path = normalize_path(path);
    let mut r = root.components();
    let mut p = path.components();
    loop {
        match r.next() {
            None => return Some(p.as_path().to_path_buf()),
            Some(rc) => match p.next() {
                Some(pc)
                    if rc.as_os_str().to_string_lossy().to_lowercase()
                        == pc.as_os_str().to_string_lossy().to_lowercase() => {}
                _ => return None,
            },
        }
    }
}

// ─────────────────────────────── message helpers (it) ───────────────────────────

fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "Claude",
        Agent::Codex => "Codex",
        Agent::Opencode => "OpenCode",
        Agent::Pi => "Pi",
    }
}

fn source_of(a: Agent) -> ActivitySource {
    match a {
        Agent::Claude => ActivitySource::Claude,
        Agent::Codex => ActivitySource::Codex,
        Agent::Opencode => ActivitySource::Opencode,
        Agent::Pi => ActivitySource::Pi,
    }
}

fn giorni(n: u32) -> String {
    if n == 1 { "1 giorno".into() } else { format!("{n} giorni") }
}

fn fa(n: u32) -> String {
    match n {
        0 => "oggi".into(),
        1 => "1 giorno fa".into(),
        n => format!("{n} giorni fa"),
    }
}

/// Italian description of where the last activity came from, used inside parentheses.
fn source_phrase(s: ActivitySource) -> String {
    match s {
        ActivitySource::Claude => "ultima sessione Claude".into(),
        ActivitySource::Codex => "ultima sessione Codex".into(),
        ActivitySource::Opencode => "ultima sessione OpenCode".into(),
        ActivitySource::Pi => "ultima sessione Pi".into(),
        ActivitySource::Git => "ultimo commit git".into(),
        ActivitySource::Files => "ultima modifica ai file".into(),
    }
}

// ──────────────────────────────────── analyze ───────────────────────────────────

/// A non-repo folder with at most this many candidate projects below it counts as a "workspace"
/// whose sessions belong to those projects (see `analyze`).
const MAX_WORKSPACE_PROJECTS: usize = 8;

/// Build one [`ProjectReport`] per scanned project, sorted by `reclaimable_bytes` desc.
///
/// # Rules
///
/// **Join.** A session belongs to a project if its `project_path` (compared through
/// [`path_key`]) equals the project root or lies inside it, so a session whose cwd is a
/// subfolder counts. Matching is component-aware. A candidate with no direct session
/// inherits the sessions of its nearest ancestor candidate (of the same scan) that has
/// some: the scanner emits one candidate per marker dir (`<repo>` and `<repo>/src-tauri`)
/// while the agent runs in `<repo>`. Non-candidate ancestors (`D:\Projects`, home) are
/// never used. Inherited live sessions mark the project `InUse` too.
///
/// **Activity.** `last_activity_ms = max(session.last_activity_ms, git.last_commit_ms,
/// candidate.source_mtime_ms)`; ties go to session, then git, then files.
/// `inactive_days = (now_ms - last) / 86_400_000` (floored, never negative).
///
/// **Agent worktree.** The root equals or lies inside a directory returned by any
/// `SessionSource::managed_worktrees()`.
///
/// **Blocking (`blocked_by`, all applicable reasons are listed).**
/// * a matching session with `live == true` -> `LiveSession`, status `InUse`;
/// * dirty git (`dirty == Some(true)`) or unpushed commits (`> 0`) block ONLY agent
///   worktrees, because removing a whole worktree loses that work. For ordinary projects
///   only rebuildable artifacts are removed, so dirty git does not block (it is noted in
///   `reason`). Unknown (`None`) git facts never block.
///
/// Any non-live block gives status `Blocked`.
///
/// **Status (when nothing blocks).** A session exists and `inactive_days >=
/// cfg.inactive_days` -> `Inactive`; session exists but newer -> `Recent`; no session at
/// all -> `NoSessions` (selectable).
///
/// **Folders.** Each artifact gets its `level`, `note` and `preselected` from `assess`: a
/// folder tracked by git (`GitProbe::is_tracked`), not verified, or with an ambiguous name git
/// does not ignore (`GitProbe::is_ignored`) is `Caution`; none of these blocks the project
/// any more (`TrackedArtifact`/`UnverifiedArtifact` are not emitted). A folder is preselected
/// when it is `Recommended`, the project is inactive (`inactive_days >= cfg.inactive_days`,
/// with or without sessions), nothing blocks it and git has no uncommitted changes.
///
/// **Preselection.** Ordinary project: `preselected` = any of its folders is. Agent worktree
/// (removed as a whole): `status == Inactive && blocked_by.is_empty() && dirty != Some(true)`.
///
/// **Size.** `reclaimable_bytes` sums `effective_bytes()` of ALL artifacts, even when the
/// project is blocked, so the UI can still show what is locked.
///
/// Warnings from the scanner and from every session source are collected; a failing (or
/// panicking) source never aborts the analysis. `git.state` is queried once per project.
pub fn analyze(
    cfg: &EngineConfig,
    scanner: &dyn ProjectScanner,
    index: Option<&dyn DirIndex>,
    sources: &[&dyn SessionSource],
    git: &dyn GitProbe,
) -> AnalysisResult {
    let mut warnings: Vec<String> = Vec::new();

    let scanned = scanner.scan(&cfg.roots, &cfg.scan, index);
    warnings.extend(scanned.warnings);

    let mut sessions: Vec<SessionRecord> = Vec::new();
    let mut worktree_keys: Vec<String> = Vec::new();
    for src in sources {
        match catch_unwind(AssertUnwindSafe(|| src.load())) {
            Ok(loaded) => {
                warnings.extend(loaded.warnings);
                sessions.extend(loaded.items);
            }
            Err(_) => warnings.push(
                "Una sorgente di sessioni ha causato un errore interno ed è stata ignorata".to_string(),
            ),
        }
        match catch_unwind(AssertUnwindSafe(|| src.managed_worktrees())) {
            Ok(dirs) => worktree_keys.extend(dirs.iter().map(|d| path_key(d))),
            Err(_) => warnings.push(
                "Impossibile leggere i worktree gestiti da una sorgente di sessioni".to_string(),
            ),
        }
    }
    worktree_keys.retain(|k| !k.is_empty());

    let keyed: Vec<(String, &SessionRecord)> =
        sessions.iter().map(|s| (path_key(&s.project_path), s)).collect();

    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<(String, ProjectCandidate)> = Vec::new();
    for cand in scanned.items {
        let id = path_key(&cand.root);
        if !seen.insert(id.clone()) {
            warnings.push(format!("Progetto duplicato ignorato: {}", cand.root.display()));
            continue;
        }
        unique.push((id, cand));
    }

    // Sessions that directly belong to each candidate: equal to / inside its root, or started in
    // a strict ANCESTOR of it (typical: the agent runs in `<repo>` while the candidate is
    // `<repo>/backend`). An ancestor equal to or above a container root (`cfg.container_roots`:
    // the home, the folders walked in full) is never used, otherwise a session started in
    // `D:\Projects` or in the home dir would match every project.
    let root_keys: Vec<String> = cfg.container_roots.iter().map(|r| path_key(r)).collect();
    // A strict-ancestor session counts only if the ancestor is a git repository OR a small
    // workspace folder (at most MAX_WORKSPACE_PROJECTS candidates below it, e.g. a folder with
    // `backend` + `frontend`). A session started in `D:\Projects`, a folder holding 100+ projects,
    // must not credit every project below it, which a scan rooted at `D:\` would otherwise do.
    // Computed once per distinct session path.
    let mut ancestor_ok: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    for (k, s) in &keyed {
        if ancestor_ok.contains_key(k.as_str()) {
            continue;
        }
        let below = unique.iter().filter(|(id, _)| id != k && key_is_within(id, k)).count();
        let ok = below > 0
            && !root_keys.iter().any(|rk| key_is_within(rk, k))
            && (below <= MAX_WORKSPACE_PROJECTS || git.state(&s.project_path).is_repo);
        ancestor_ok.insert(k.as_str(), ok);
    }
    let direct: Vec<Vec<&SessionRecord>> = unique
        .iter()
        .map(|(id, _)| {
            keyed
                .iter()
                .filter(|(k, _)| {
                    key_is_within(k, id)
                        || (key_is_within(id, k) && ancestor_ok.get(k.as_str()) == Some(&true))
                })
                .map(|(_, s)| *s)
                .collect()
        })
        .collect();

    // Inheritance: a candidate with NO direct session takes the sessions of its nearest
    // ancestor CANDIDATE of this scan that has some (e.g. `<repo>/src-tauri` inherits from
    // `<repo>`, where the agent was started). Never from arbitrary non-candidate ancestors.
    let matching: Vec<Vec<&SessionRecord>> = unique
        .iter()
        .enumerate()
        .map(|(i, (id, _))| {
            if !direct[i].is_empty() {
                return direct[i].clone();
            }
            unique
                .iter()
                .enumerate()
                .filter(|(j, (aid, _))| *j != i && !direct[*j].is_empty() && key_is_within(id, aid))
                .max_by_key(|(_, (aid, _))| aid.len())
                .map(|(j, _)| direct[j].clone())
                .unwrap_or_default()
        })
        .collect();

    let mut reports: Vec<ProjectReport> = unique
        .into_iter()
        .zip(matching)
        .map(|((id, cand), m)| analyze_one(cfg, git, m, &worktree_keys, cand, id))
        .collect();

    reports.sort_by(|a, b| {
        b.reclaimable_bytes.cmp(&a.reclaimable_bytes).then_with(|| a.id.cmp(&b.id))
    });

    AnalysisResult { reports, warnings }
}

fn analyze_one(
    cfg: &EngineConfig,
    git: &dyn GitProbe,
    matching: Vec<&SessionRecord>,
    worktree_keys: &[String],
    cand: ProjectCandidate,
    id: String,
) -> ProjectReport {

    let agents: Vec<Agent> = AGENT_ORDER
        .into_iter()
        .filter(|a| matching.iter().any(|s| s.agent == *a))
        .collect();
    let live_agents: Vec<Agent> = AGENT_ORDER
        .into_iter()
        .filter(|a| matching.iter().any(|s| s.live && s.agent == *a))
        .collect();

    let is_agent_worktree = worktree_keys.iter().any(|w| key_is_within(&id, w));
    let gs = git.state(&cand.root);

    // Last activity: sessions first so that ties go to the most specific source.
    let mut best: Option<(i64, ActivitySource)> = None;
    let mut consider = |ms: i64, src: ActivitySource| {
        if best.is_none_or(|(b, _)| ms > b) {
            best = Some((ms, src));
        }
    };
    for s in &matching {
        consider(s.last_activity_ms, source_of(s.agent));
    }
    if let Some(ms) = gs.last_commit_ms {
        consider(ms, ActivitySource::Git);
    }
    if let Some(ms) = cand.source_mtime_ms {
        consider(ms, ActivitySource::Files);
    }
    let last_activity_ms = best.map(|(ms, _)| ms);
    let last_activity_source = best.map(|(_, s)| s);
    let inactive_days = last_activity_ms.map(|ms| {
        ((cfg.now_ms - ms).max(0) / DAY_MS).min(i64::from(u32::MAX)) as u32
    });

    // Blocking.
    let mut blocked_by: Vec<BlockReason> = Vec::new();
    let mut block_msgs: Vec<String> = Vec::new();
    if !live_agents.is_empty() {
        blocked_by.push(BlockReason::LiveSession);
        let names: Vec<&str> = live_agents.iter().map(|a| agent_name(*a)).collect();
        block_msgs.push(if names.len() == 1 {
            format!("Sessione {} attiva ora", names[0])
        } else {
            format!("Sessioni attive ora ({})", names.join(", "))
        });
    }
    let dirty = gs.dirty == Some(true);
    let unpushed = gs.unpushed_commits.filter(|n| *n > 0);
    if is_agent_worktree {
        if dirty {
            blocked_by.push(BlockReason::DirtyGit);
            block_msgs.push("Modifiche git non committate nel worktree".to_string());
        }
        if let Some(n) = unpushed {
            blocked_by.push(BlockReason::UnpushedCommits);
            block_msgs.push(if n == 1 {
                "1 commit non pushato nel worktree".to_string()
            } else {
                format!("{n} commit non pushati nel worktree")
            });
        }
    }

    // Status.
    let has_sessions = !matching.is_empty();
    let status = if !live_agents.is_empty() {
        ProjectStatus::InUse
    } else if !blocked_by.is_empty() {
        ProjectStatus::Blocked
    } else if !has_sessions {
        ProjectStatus::NoSessions
    } else if inactive_days.is_some_and(|d| d >= cfg.inactive_days) {
        ProjectStatus::Inactive
    } else {
        ProjectStatus::Recent
    };

    // Reason (Italian).
    let src_phrase = last_activity_source.map(source_phrase);
    let mut reason = match status {
        ProjectStatus::InUse | ProjectStatus::Blocked | ProjectStatus::Unreachable => {
            block_msgs.join("; ")
        }
        ProjectStatus::Inactive => format!(
            "Inattivo da {} ({})",
            giorni(inactive_days.unwrap_or(0)),
            src_phrase.clone().unwrap_or_default()
        ),
        ProjectStatus::Recent => format!(
            "Attività recente: {} ({})",
            fa(inactive_days.unwrap_or(0)),
            src_phrase.clone().unwrap_or_default()
        ),
        ProjectStatus::NoSessions => match (inactive_days, last_activity_source) {
            (Some(d), Some(ActivitySource::Git)) => {
                format!("Nessuna sessione trovata; ultimo commit {}", fa(d))
            }
            (Some(d), _) => {
                format!("Nessuna sessione trovata; ultima modifica ai file {}", fa(d))
            }
            (None, _) => "Nessuna sessione trovata; data dell'ultima modifica sconosciuta".to_string(),
        },
    };
    if blocked_by.is_empty() && !is_agent_worktree {
        if dirty {
            reason.push_str("; modifiche git non committate (non impediscono la pulizia)");
        }
        if let Some(n) = unpushed {
            reason.push_str(&format!("; {n} commit non pushati (non impediscono la pulizia)"));
        }
    }

    let reclaimable_bytes = cand
        .artifacts
        .iter()
        .fold(0u64, |acc, a| acc.saturating_add(a.size.effective_bytes()));

    // Levels, notes and per-folder preselection. Uncommitted changes keep every folder
    // unticked, as before (they do not block an ordinary project).
    let project_inactive = inactive_days.is_some_and(|d| d >= cfg.inactive_days);
    let hold = !blocked_by.is_empty() || dirty;
    let mut artifacts = cand.artifacts;
    for a in &mut artifacts {
        let rel = relative_to(&cand.root, &a.path).unwrap_or_else(|| a.path.clone());
        // An artifact equal to the root (e.g. a whole worktree dir) has no meaningful
        // "tracked"/"ignored" answer: the project's own files are tracked by definition.
        let inside = !rel.as_os_str().is_empty();
        let tracked = inside && git.is_tracked(&cand.root, &rel);
        // `git check-ignore` is only asked for the names that need it.
        let ignored = if a.ambiguous && inside { git.is_ignored(&cand.root, &rel) } else { None };
        assess(a, tracked, ignored, project_inactive, hold);
    }

    // Agent worktrees keep the project rule (they are removed as a whole): inactive, nothing
    // blocking, clean (or unknown) git tree. Ordinary projects: any preselected folder.
    let preselected = if is_agent_worktree {
        status == ProjectStatus::Inactive && blocked_by.is_empty() && gs.dirty != Some(true)
    } else {
        artifacts.iter().any(|a| a.preselected)
    };

    ProjectReport {
        id,
        root: cand.root,
        name: cand.name,
        ecosystems: cand.ecosystems,
        agents,
        last_activity_ms,
        last_activity_source,
        inactive_days,
        status,
        blocked_by,
        reason,
        artifacts,
        reclaimable_bytes,
        preselected,
        is_agent_worktree,
        git: gs,
    }
}

fn add_note(a: &mut Artifact, note: &str) {
    if !a.note.is_empty() {
        a.note.push_str("; ");
    }
    a.note.push_str(note);
}

/// Level, note and preselection of one folder. The first rule that applies sets the level; every
/// applicable note is appended to the catalog's one (`; ` between them).
///
/// 1. `level = class.base_level()`;
/// 2. tracked by git -> `Caution`;
/// 3. identity not verified -> `Caution`;
/// 4. ambiguous name (`†`) that git does not ignore (`Some(false)`, or `None` outside a
///    repository) -> `Caution`;
/// 5. `preselected = project_inactive && !project_blocked && level == Recommended`.
fn assess(a: &mut Artifact, tracked: bool, ignored: Option<bool>, project_inactive: bool, project_blocked: bool) {
    a.level = a.class.base_level();
    if tracked {
        a.level = Level::Caution;
        add_note(a, "Contiene file tracciati da git");
    }
    if !a.verified {
        a.level = Level::Caution;
        add_note(a, "Cartella non riconosciuta con certezza");
    }
    if a.ambiguous && ignored != Some(true) {
        a.level = Level::Caution;
        add_note(
            a,
            if ignored == Some(false) {
                "Potrebbe contenere sorgenti: git non la ignora"
            } else {
                "Potrebbe contenere sorgenti: non è in un repository git"
            },
        );
    }
    a.preselected = project_inactive && !project_blocked && a.level == Level::Recommended;
}

// ─────────────────────────────────── build_plan ─────────────────────────────────

static PLAN_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_plan_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = PLAN_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("plan-{ms}-{n}")
}

/// The main repository to run `git worktree remove` from when [`build_plan`] removes this report
/// as a whole worktree: an agent worktree, `Inactive` or `Recent`, known clean, with a known
/// `main_worktree` other than itself. `None`: only its artifacts can go.
pub fn whole_worktree_repo(report: &ProjectReport) -> Option<&PathBuf> {
    let whole = report.is_agent_worktree
        && matches!(report.status, ProjectStatus::Inactive | ProjectStatus::Recent)
        && report.git.dirty == Some(false);
    report
        .git
        .main_worktree
        .as_ref()
        .filter(|m| whole && path_key(m) != report.id)
}

/// Whether [`build_plan`] gives at least one item for this report once its status and blocks let
/// it through: the whole worktree ([`whole_worktree_repo`]) or at least one artifact.
pub fn plans_something(report: &ProjectReport) -> bool {
    whole_worktree_repo(report).is_some() || !report.artifacts.is_empty()
}

/// The errors of today for a report whose folders cannot be touched: `InUse`, `Blocked`,
/// `Unreachable`, or any `blocked_by` reason.
fn check_cleanable(report: &ProjectReport) -> Result<()> {
    match report.status {
        ProjectStatus::InUse => {
            return Err(Error::Other(format!(
                "«{}» è in uso da una sessione attiva e non può essere pulito",
                report.name
            )));
        }
        ProjectStatus::Blocked => {
            return Err(Error::Other(format!(
                "«{}» è bloccato e non può essere pulito: {}",
                report.name, report.reason
            )));
        }
        ProjectStatus::Unreachable => {
            return Err(Error::Other(format!(
                "«{}» non è raggiungibile e non può essere pulito",
                report.name
            )));
        }
        ProjectStatus::Inactive | ProjectStatus::Recent | ProjectStatus::NoSessions => {}
    }
    if !report.blocked_by.is_empty() {
        return Err(Error::Other(format!(
            "«{}» ha dei blocchi attivi e non può essere pulito: {}",
            report.name, report.reason
        )));
    }
    Ok(())
}

fn artifact_item(report: &ProjectReport, a: &Artifact) -> PlanItem {
    PlanItem {
        path: a.path.clone(),
        kind: a.kind,
        action: PlanAction::RemoveDir,
        expected_bytes: a.size.effective_bytes(),
        project_id: report.id.clone(),
        reason: report.reason.clone(),
    }
}

/// Turn a user selection into a [`CleanPlan`].
///
/// * `sel.artifacts`: every id must be the [`Artifact::id`] of an artifact of some report
///   (otherwise `Cartella non trovata nell'analisi: {id}`), and that report must not be `InUse`,
///   `Blocked` or `Unreachable` nor carry any `blocked_by` reason. One `RemoveDir` item each
///   (`expected_bytes` = `effective_bytes`), whatever its level: the level only decides the
///   preselection. A folder of an agent worktree that is removed whole
///   ([`whole_worktree_repo`]) is refused: such a worktree goes as a whole or not at all.
/// * `sel.worktrees`: ids of agent-worktree reports (an ordinary project is refused), with the
///   same status checks. A worktree that is `Inactive` or `Recent`, known clean
///   (`dirty == Some(false)`) and with a known `main_worktree` different from itself -> ONE
///   `WorktreeDir` item with `GitWorktreeRemove { repo: main_worktree }` and `expected_bytes =
///   reclaimable_bytes` (so git deregisters the worktree instead of leaving a dangling entry).
///   Anything less certain falls back to removing its `Recommended` artifacts only: a worktree
///   tick never plans a folder that needs a tick of its own.
///
/// Any error fails the whole call (Italian `Error::Other`) and nothing is planned. Duplicate
/// ids, and items with an identical `path_key`, are planned once. The plan id is unique per
/// call (timestamp + process-wide counter).
pub fn build_plan(reports: &[ProjectReport], sel: &Selection, mode: RemoveMode) -> Result<CleanPlan> {
    let mut items: Vec<PlanItem> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    let mut seen_ids: HashSet<&str> = HashSet::new();
    for id in &sel.artifacts {
        if !seen_ids.insert(id.as_str()) {
            continue;
        }
        let (report, a) = reports
            .iter()
            .find_map(|r| r.artifacts.iter().find(|a| &a.id == id).map(|a| (r, a)))
            .ok_or_else(|| Error::Other(format!("Cartella non trovata nell'analisi: {id}")))?;
        check_cleanable(report)?;
        if whole_worktree_repo(report).is_some() {
            return Err(Error::Other(format!(
                "«{}» è un worktree che si rimuove solo per intero: {}",
                report.name,
                a.path.display()
            )));
        }
        if seen_paths.insert(path_key(&a.path)) {
            items.push(artifact_item(report, a));
        }
    }

    let mut seen_ids: HashSet<&str> = HashSet::new();
    for id in &sel.worktrees {
        if !seen_ids.insert(id.as_str()) {
            continue;
        }
        let report = reports
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| Error::Other(format!("Progetto non trovato nell'analisi: {id}")))?;
        if !report.is_agent_worktree {
            return Err(Error::Other(format!("«{}» non è un worktree di un agente", report.name)));
        }
        check_cleanable(report)?;
        if let Some(repo) = whole_worktree_repo(report) {
            if seen_paths.insert(report.id.clone()) {
                items.push(PlanItem {
                    path: report.root.clone(),
                    kind: ArtifactKind::WorktreeDir,
                    action: PlanAction::GitWorktreeRemove { repo: repo.clone() },
                    expected_bytes: report.reclaimable_bytes,
                    project_id: report.id.clone(),
                    reason: report.reason.clone(),
                });
            }
        } else {
            for a in report.artifacts.iter().filter(|a| a.level == Level::Recommended) {
                if seen_paths.insert(path_key(&a.path)) {
                    items.push(artifact_item(report, a));
                }
            }
        }
    }

    let expected_bytes = items.iter().fold(0u64, |acc, i| acc.saturating_add(i.expected_bytes));
    Ok(CleanPlan { id: new_plan_id(), mode, items, expected_bytes })
}

#[cfg(test)]
mod tests;
