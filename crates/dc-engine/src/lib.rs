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
//! * Anything that could lose work or break a live process blocks removal; anything that
//!   is merely "recent" or "unknown" stays selectable but is never preselected.

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

/// `target/` style label of a directory, for messages.
fn dir_label(path: &Path) -> String {
    match path.file_name() {
        Some(n) => format!("{}/", n.to_string_lossy()),
        None => path.display().to_string(),
    }
}

fn labels(artifacts: &[&Artifact]) -> String {
    let mut seen = HashSet::new();
    let names: Vec<String> = artifacts
        .iter()
        .map(|a| dir_label(&a.path))
        .filter(|n| seen.insert(n.clone()))
        .collect();
    if names.len() > 3 {
        format!("{}, ...", names[..3].join(", "))
    } else {
        names.join(", ")
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
/// * an artifact tracked by git (`GitProbe::is_tracked`) -> `TrackedArtifact`;
/// * an artifact with `verified == false` -> `UnverifiedArtifact`;
/// * dirty git (`dirty == Some(true)`) or unpushed commits (`> 0`) block ONLY agent
///   worktrees, because removing a whole worktree loses that work. For ordinary projects
///   only rebuildable artifacts are removed, so dirty git does not block (it is noted in
///   `reason`). Unknown (`None`) git facts never block.
///
/// Any non-live block gives status `Blocked`.
///
/// **Status (when nothing blocks).** A session exists and `inactive_days >=
/// cfg.inactive_days` -> `Inactive`; session exists but newer -> `Recent`; no session at
/// all -> `NoSessions` (selectable, never preselected).
///
/// **Preselection.** `preselected = status == Inactive && blocked_by.is_empty()`.
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
    let tracked: Vec<&Artifact> = cand
        .artifacts
        .iter()
        .filter(|a| {
            let rel = relative_to(&cand.root, &a.path).unwrap_or_else(|| a.path.clone());
            // An artifact equal to the root (e.g. a whole worktree dir) has no meaningful
            // "tracked" answer: the project's own files are tracked by definition.
            !rel.as_os_str().is_empty() && git.is_tracked(&cand.root, &rel)
        })
        .collect();
    let unverified: Vec<&Artifact> = cand.artifacts.iter().filter(|a| !a.verified).collect();

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
    if !tracked.is_empty() {
        blocked_by.push(BlockReason::TrackedArtifact);
        block_msgs.push(if tracked.len() == 1 {
            format!("Un {} è tracciato da git", labels(&tracked))
        } else {
            format!("{} cartelle ({}) sono tracciate da git", tracked.len(), labels(&tracked))
        });
    }
    if !unverified.is_empty() {
        blocked_by.push(BlockReason::UnverifiedArtifact);
        block_msgs.push(if unverified.len() == 1 {
            format!("{} non è stato verificato come cartella rigenerabile", labels(&unverified))
        } else {
            format!(
                "{} cartelle ({}) non verificate come rigenerabili",
                unverified.len(),
                labels(&unverified)
            )
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
    // Preselect only inactive projects with no block and a clean (or unknown) git tree.
    let preselected = status == ProjectStatus::Inactive
        && blocked_by.is_empty()
        && gs.dirty != Some(true);

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
        artifacts: cand.artifacts,
        reclaimable_bytes,
        preselected,
        is_agent_worktree,
        git: gs,
    }
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

/// Turn a user selection into a [`CleanPlan`].
///
/// Every id must exist in `reports` and must not be `InUse`, `Blocked` or `Unreachable`
/// (nor carry any `blocked_by` reason); otherwise the whole call fails with an Italian
/// `Error::Other` and nothing is planned. Duplicate ids are planned once.
///
/// * Ordinary project -> one `RemoveDir` item per artifact (`expected_bytes` =
///   `effective_bytes`).
/// * Agent worktree that is `Inactive` or `Recent`, known clean (`dirty == Some(false)`)
///   and with a known `main_worktree` different from itself -> ONE `WorktreeDir` item with
///   `GitWorktreeRemove { repo: main_worktree }` and `expected_bytes = reclaimable_bytes`
///   (so git deregisters the worktree instead of leaving a dangling entry). Anything less
///   certain falls back to removing only the artifacts.
///
/// Items with an identical path are emitted once. The plan id is unique per call
/// (timestamp + process-wide counter).
pub fn build_plan(
    reports: &[ProjectReport],
    selected_ids: &[String],
    mode: RemoveMode,
) -> Result<CleanPlan> {
    let mut items: Vec<PlanItem> = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for id in selected_ids {
        if !seen_ids.insert(id.as_str()) {
            continue;
        }
        let report = reports
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| Error::Other(format!("Progetto non trovato nell'analisi: {id}")))?;

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

        let whole_worktree = report.is_agent_worktree
            && matches!(report.status, ProjectStatus::Inactive | ProjectStatus::Recent)
            && report.git.dirty == Some(false);
        let main_repo = report
            .git
            .main_worktree
            .as_ref()
            .filter(|m| path_key(m) != report.id);

        if let (true, Some(repo)) = (whole_worktree, main_repo) {
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
            for a in &report.artifacts {
                if seen_paths.insert(path_key(&a.path)) {
                    items.push(PlanItem {
                        path: a.path.clone(),
                        kind: a.kind,
                        action: PlanAction::RemoveDir,
                        expected_bytes: a.size.effective_bytes(),
                        project_id: report.id.clone(),
                        reason: report.reason.clone(),
                    });
                }
            }
        }
    }

    let expected_bytes = items.iter().fold(0u64, |acc, i| acc.saturating_add(i.expected_bytes));
    Ok(CleanPlan { id: new_plan_id(), mode, items, expected_bytes })
}

#[cfg(test)]
mod tests;
