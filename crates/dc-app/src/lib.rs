//! UI-less glue between the devclean crates and PulisciPC: settings, the developer scan,
//! the clean plan and the removal of archived agent sessions. Ported from devclean's Tauri
//! shell (`src-tauri/src/lib.rs`) without Tauri and without WizTree: folder sizes come from
//! the NTFS MFT (`dc-disktree`) when the app runs as administrator, otherwise from the walker.
//!
//! Safety: `execute_plan` and `delete_archived` never trust what the caller passes. Every plan
//! item must be a path the given scan found (an artifact, or the root of an agent worktree),
//! and every session must be one the scan listed as archived and removable; anything else
//! refuses the whole request before a single file is touched.

use dc_model::{
    Agent, AnalysisResult, CleanPlan, CleanResult, DEFAULT_INACTIVE_DAYS, DirIndex, EngineConfig, Loaded,
    PlanAction, PlanItem, ProjectCandidate, ProjectReport, ProjectScanner, RemoveMode, Remover, ScanOptions,
    SessionRecord, SessionSource, Settings, normalize_path, path_key,
};
use dc_mukade::{ArchivedSession, MukadeSessions};
use dc_sessions::{HarnessCli, SessionOutcome, Trash};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const APP_DIR: &str = "PulisciPC";
const AGENTS: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi];

// ───────────────────────────────── settings ─────────────────────────────────

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The user's profile folder (`%USERPROFILE%`, else `$HOME`); `None` when neither is set.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

/// `%<var>%`, else `<home>\<fallback...>` (the temp folder stands in for a missing home).
fn env_dir(var: &str, fallback: &[&str]) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| {
        let base = home_dir().unwrap_or_else(std::env::temp_dir);
        fallback.iter().fold(base, |p, c| p.join(c))
    })
}

/// Conventional project folders under the home, in this order, kept only if they exist.
const HOME_PROJECT_DIRS: &[&[&str]] = &[
    &["source", "repos"],
    &["source"],
    &["repos"],
    &["Projects"],
    &["dev"],
    &["code"],
    &["Documents", "GitHub"],
];

/// Projects from the agents' sessions; the conventional project folders under the home that
/// exist are listed as extra folders, walked in full only when the user switches `scan_roots`
/// on (none found: the list is empty). Removal goes to the Recycle Bin.
pub fn default_settings() -> Settings {
    default_settings_for(home_dir().as_deref())
}

fn default_settings_for(home: Option<&Path>) -> Settings {
    let roots = home
        .map(|h| {
            HOME_PROJECT_DIRS
                .iter()
                .map(|parts| parts.iter().fold(h.to_path_buf(), |p, c| p.join(c)))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    Settings {
        session_projects: true,
        roots,
        scan_roots: false,
        inactive_days: DEFAULT_INACTIVE_DAYS,
        remove_mode: RemoveMode::Recycle,
    }
}

/// `%APPDATA%\PulisciPC\settings.json`.
pub fn settings_path() -> PathBuf {
    env_dir("APPDATA", &["AppData", "Roaming"]).join(APP_DIR).join("settings.json")
}

/// `%LOCALAPPDATA%\PulisciPC\mukade`: mukade's session listing cache. Nothing else is written
/// there, and mukade never writes beside the executable or in the harnesses' stores.
pub fn cache_dir() -> PathBuf {
    env_dir("LOCALAPPDATA", &["AppData", "Local"]).join(APP_DIR).join("mukade")
}

/// Missing or unreadable file: the defaults.
pub fn load_settings() -> Settings {
    read_settings(&settings_path())
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    write_settings(&settings_path(), s)
}

fn read_settings(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_settings)
}

fn write_settings(path: &Path, s: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

// ─────────────────────────────────── scan ───────────────────────────────────

/// Result of one developer scan. It is also the allow-list of `execute_plan` and
/// `delete_archived`: keep the last one and pass it back.
#[derive(Clone, Debug, Default)]
pub struct DevScan {
    /// Ordinary projects: neither an agent worktree nor a linked git worktree.
    pub projects: Vec<ProjectReport>,
    /// Worktrees the agents created (selectable: the whole worktree can go).
    pub worktrees: Vec<ProjectReport>,
    /// Linked git worktrees no agent manages: shown for information, never cleaned.
    pub other_worktrees: Vec<ProjectReport>,
    /// Archived root sessions of every harness, removable or not (`blocked`).
    pub archived: Vec<ArchivedSession>,
    pub warnings: Vec<String>,
    /// `"mft"` when the MFT index was used, `"walker"` otherwise.
    pub backend: String,
    pub elapsed_ms: u64,
}

/// One agent's sessions, read once per scan: they choose the project roots AND feed the engine.
pub struct Preloaded {
    pub agent: Agent,
    pub loaded: Loaded<SessionRecord>,
    /// Existing folders only.
    pub worktrees: Vec<PathBuf>,
}

impl SessionSource for Preloaded {
    fn agent(&self) -> Agent {
        self.agent
    }
    fn load(&self) -> Loaded<SessionRecord> {
        self.loaded.clone()
    }
    fn managed_worktrees(&self) -> Vec<PathBuf> {
        self.worktrees.clone()
    }
}

/// All four agents through the embedded mukade library: listings only, read-only. A source
/// that panics becomes a warning. The managed worktrees are kept only if they exist
/// (`dc_mukade::claude_worktrees` names the worktree a session folder points into without
/// checking it is still there).
pub fn load_sessions(cache_dir: Option<PathBuf>) -> (Vec<Preloaded>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = Vec::new();
    for agent in AGENTS {
        let src = MukadeSessions::from_env(agent).with_cache_dir(cache_dir.clone());
        let loaded = catch_unwind(AssertUnwindSafe(|| src.load())).unwrap_or_else(|_| Loaded {
            items: Vec::new(),
            warnings: vec![format!("{agent:?}: errore interno nella lettura delle sessioni, ignorate")],
        });
        let worktrees = catch_unwind(AssertUnwindSafe(|| src.managed_worktrees())).unwrap_or_else(|_| {
            warnings.push(format!("{agent:?}: impossibile leggere i worktree gestiti"));
            Vec::new()
        });
        let worktrees = worktrees.into_iter().filter(|p| p.is_dir()).collect();
        out.push(Preloaded { agent, loaded, worktrees });
    }
    (out, warnings)
}

/// Archived root sessions of every harness. A source that panics becomes a warning.
fn load_archived(cache_dir: Option<PathBuf>) -> Loaded<ArchivedSession> {
    let mut out = Loaded::default();
    for agent in AGENTS {
        let src = MukadeSessions::from_env(agent).with_cache_dir(cache_dir.clone());
        match catch_unwind(AssertUnwindSafe(|| src.archived())) {
            Ok(l) => {
                out.items.extend(l.items);
                out.warnings.extend(l.warnings);
            }
            Err(_) => out
                .warnings
                .push(format!("{agent:?}: errore interno nella lettura delle sessioni archiviate, ignorate")),
        }
    }
    out
}

/// The roots one scan walks, and where they came from.
pub struct ScanRoots {
    /// Outer roots only, sorted by `path_key`: a root inside another one is walked as part of it.
    pub roots: Vec<PathBuf>,
    /// What the session working directories gave (roots and skipped folders with the reason).
    pub from_sessions: dc_scanner::SessionRoots,
    /// Folders holding many projects, for the engine's session matching
    /// (`EngineConfig::container_roots`): the home and every folder in `settings.roots`.
    pub container_roots: Vec<PathBuf>,
}

/// Both arguments are `path_key`s; component-aware (`c:\ab` is not within `c:\a`).
fn key_within(child: &str, parent: &str) -> bool {
    let sep = std::path::MAIN_SEPARATOR;
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| parent.ends_with(sep) || rest.starts_with(sep))
}

/// Projects come from the agents' sessions (`settings.session_projects`, see
/// `dc_scanner::project_roots_from_sessions`), plus the agents' managed worktrees that exist,
/// plus the folders in `settings.roots` when `settings.scan_roots` is on.
pub fn plan_roots(settings: &Settings, sessions: &[Preloaded], home: Option<&Path>) -> ScanRoots {
    let user: Vec<PathBuf> = if settings.scan_roots {
        settings.roots.iter().map(|r| normalize_path(r)).filter(|r| r.is_dir()).collect()
    } else {
        Vec::new()
    };
    let session_dirs: Vec<PathBuf> = if settings.session_projects {
        sessions.iter().flat_map(|s| s.loaded.items.iter().map(|r| r.project_path.clone())).collect()
    } else {
        Vec::new()
    };
    let from_sessions = dc_scanner::project_roots_from_sessions(&session_dirs, home);
    let worktrees = sessions.iter().flat_map(|s| s.worktrees.iter()).filter(|p| p.is_dir());

    // Outer roots first (shorter key), then drop every root inside one already kept.
    let mut all: Vec<PathBuf> =
        user.into_iter().chain(from_sessions.roots.iter().chain(worktrees).map(|p| normalize_path(p))).collect();
    all.sort_by_key(|p| path_key(p).len());
    let mut kept: Vec<(String, PathBuf)> = Vec::new();
    for p in all {
        let k = path_key(&p);
        if !kept.iter().any(|(outer, _)| key_within(&k, outer)) {
            kept.push((k, p));
        }
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0));

    let mut container_roots: Vec<PathBuf> = home.map(normalize_path).into_iter().collect();
    container_roots.extend(settings.roots.iter().map(|r| normalize_path(r)));
    ScanRoots { roots: kept.into_iter().map(|(_, p)| p).collect(), from_sessions, container_roots }
}

/// Hands the index only the roots it covers. `dc_disktree::build_for_roots` leaves out the
/// roots on a volume it could not read, and `size_of` is `None` for a reparse-point folder
/// (junction, OneDrive): the scanner looks for artifacts ONLY through the index when it gets
/// one, so those roots go to the walker (`index = None`) instead.
struct SplitScanner<'a> {
    inner: &'a dyn ProjectScanner,
}

impl ProjectScanner for SplitScanner<'_> {
    fn scan(&self, roots: &[PathBuf], opts: &ScanOptions, index: Option<&dyn DirIndex>) -> Loaded<ProjectCandidate> {
        let (indexed, walked): (Vec<PathBuf>, Vec<PathBuf>) =
            roots.iter().cloned().partition(|r| index.is_some_and(|ix| ix.size_of(r).is_some()));
        let mut out = Loaded::default();
        for (part, ix) in [(indexed, index), (walked, None)] {
            if part.is_empty() {
                continue;
            }
            let l = self.inner.scan(&part, opts, ix);
            out.items.extend(l.items);
            out.warnings.extend(l.warnings);
        }
        out
    }
}

const FAST_READ_FAILED: &str = "Lettura veloce del disco non riuscita: uso la scansione normale.";
const ANALYSIS_FAILED: &str = "Errore interno durante l'analisi dei progetti.";

/// Runs the MFT index build; a panic becomes a warning and no index (the walker does it all).
fn guarded_index<I>(build: impl FnOnce() -> (Option<I>, Vec<String>), warnings: &mut Vec<String>) -> Option<I> {
    match catch_unwind(AssertUnwindSafe(build)) {
        Ok((index, w)) => {
            warnings.extend(w);
            index
        }
        Err(_) => {
            warnings.push(FAST_READ_FAILED.into());
            None
        }
    }
}

/// Runs the analysis; a panic becomes an empty result with a warning instead of unwinding
/// into the caller (the UI thread).
fn guarded_analysis(run: impl FnOnce() -> AnalysisResult) -> AnalysisResult {
    catch_unwind(AssertUnwindSafe(run))
        .unwrap_or_else(|_| AnalysisResult { reports: Vec::new(), warnings: vec![ANALYSIS_FAILED.into()] })
}

/// `(projects, agent worktrees, other linked worktrees)`, in the input order.
pub fn split_reports(reports: Vec<ProjectReport>) -> (Vec<ProjectReport>, Vec<ProjectReport>, Vec<ProjectReport>) {
    let (mut projects, mut worktrees, mut others) = (Vec::new(), Vec::new(), Vec::new());
    for r in reports {
        if r.is_agent_worktree {
            worktrees.push(r);
        } else if r.git.is_linked_worktree {
            others.push(r);
        } else {
            projects.push(r);
        }
    }
    (projects, worktrees, others)
}

/// The developer scan. `step(text, fraction)` reports progress in Italian.
///
/// `admin`: the MFT index (`dc-disktree`) is built for ALL the roots and handed to the scanner,
/// which takes the artifact sizes from it (roots it does not cover are walked). Without `admin`
/// there is no index and the `dc-scanner` walker does everything. The four steps are always
/// reported. A panic while building the index falls back to the walker; a panic in the analysis
/// gives no reports and a warning (the archived sessions are still listed). Nothing is written
/// except mukade's listing cache in [`cache_dir`].
pub fn run_scan(settings: &Settings, admin: bool, step: &dyn Fn(&str, f32)) -> DevScan {
    let started = Instant::now();
    let cache = Some(cache_dir());

    step("Lettura delle sessioni…", 0.05);
    let (sessions, mut warnings) = load_sessions(cache.clone());
    let home = home_dir();
    let plan = plan_roots(settings, &sessions, home.as_deref());
    if plan.roots.is_empty() {
        warnings.push(if settings.session_projects {
            "Nessun progetto trovato nelle sessioni degli agenti: aggiungi delle cartelle da analizzare nelle impostazioni.".into()
        } else {
            "Nessuna cartella da analizzare.".into()
        });
    }

    // Always reported, so the UI shows the same four steps; without admin nothing is read here.
    step("Lettura veloce del disco…", 0.25);
    let index = if admin && !plan.roots.is_empty() {
        guarded_index(
            || dc_disktree::build_for_roots(&plan.roots, true, &dc_disktree::Progress::default()),
            &mut warnings,
        )
    } else {
        None
    };
    let backend = index.as_ref().map_or("walker", |i| i.backend_name()).to_string();

    step("Analisi dei progetti…", 0.45);
    let cfg = EngineConfig {
        roots: plan.roots.clone(),
        container_roots: plan.container_roots.clone(),
        inactive_days: settings.inactive_days,
        scan: ScanOptions::default(),
        now_ms: now_ms(),
    };
    let sources: Vec<&dyn SessionSource> = sessions.iter().map(|s| s as &dyn SessionSource).collect();
    let scanner = SplitScanner { inner: &dc_scanner::DefaultScanner };
    let analysis = guarded_analysis(|| {
        dc_engine::analyze(&cfg, &scanner, index.as_ref().map(|i| i as &dyn DirIndex), &sources, &dc_git::GitCli)
    });
    drop(index);
    warnings.extend(analysis.warnings);
    let (projects, worktrees, other_worktrees) = split_reports(analysis.reports);

    step("Sessioni archiviate…", 0.9);
    let archived = load_archived(cache);
    warnings.extend(archived.warnings);

    DevScan {
        projects,
        worktrees,
        other_worktrees,
        archived: archived.items,
        warnings,
        backend,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

// ─────────────────────────────── plan / clean ───────────────────────────────

/// Projects and agent worktrees only: the other worktrees cannot be selected.
fn selectable(scan: &DevScan) -> impl Iterator<Item = &ProjectReport> {
    scan.projects.iter().chain(&scan.worktrees)
}

fn selectable_reports(scan: &DevScan) -> Vec<ProjectReport> {
    selectable(scan).cloned().collect()
}

pub fn build_plan(scan: &DevScan, selected_ids: &[String], mode: RemoveMode) -> Result<CleanPlan, String> {
    dc_engine::build_plan(&selectable_reports(scan), selected_ids, mode).map_err(|e| e.to_string())
}

/// Runs the plan for real (no dry run) through `dc_cleaner::DefaultRemover`, after
/// [`checked_plan`]: only items the scan itself would plan are executed.
pub fn execute_plan(scan: &DevScan, plan: &CleanPlan) -> Result<CleanResult, String> {
    let checked = checked_plan(scan, plan)?;
    Ok(dc_cleaner::DefaultRemover.execute(&checked, false))
}

fn same_action(a: &PlanAction, b: &PlanAction) -> bool {
    match (a, b) {
        (PlanAction::RemoveDir, PlanAction::RemoveDir) => true,
        (PlanAction::GitWorktreeRemove { repo: x }, PlanAction::GitWorktreeRemove { repo: y }) => {
            path_key(x) == path_key(y)
        }
        _ => false,
    }
}

/// The plan to execute, never the caller's items themselves:
///
/// 1. allow-list: every path must be an artifact of a project or agent worktree of `scan`, or
///    the root of an agent worktree (an ordinary project's root, anything of the other
///    worktrees and any path the scan never saw are refused);
/// 2. the canonical plan is rebuilt with `dc_engine::build_plan` from the scan's reports for the
///    distinct `project_id`s of the items (so an `InUse` or blocked project is refused), and
///    every item must match a canonical one on `path_key(path)`, `kind` and `action` (for
///    `GitWorktreeRemove`, `repo` compared with `path_key`).
///
/// The result holds the matching canonical items, once each, in the caller's order (a subset of
/// the canonical plan is fine), with the caller's plan id and mode.
fn checked_plan(scan: &DevScan, plan: &CleanPlan) -> Result<CleanPlan, String> {
    let allowed: HashSet<String> = selectable(scan)
        .flat_map(|r| r.artifacts.iter().map(|a| path_key(&a.path)))
        .chain(scan.worktrees.iter().map(|r| path_key(&r.root)))
        .collect();
    if let Some(bad) = plan.items.iter().find(|i| !allowed.contains(&path_key(&i.path))) {
        return Err(format!("Percorso non presente nell'ultima analisi, rifiutato: {}", bad.path.display()));
    }

    let mut ids: Vec<String> = Vec::new();
    for i in &plan.items {
        if !ids.contains(&i.project_id) {
            ids.push(i.project_id.clone());
        }
    }
    let canonical = dc_engine::build_plan(&selectable_reports(scan), &ids, plan.mode)
        .map_err(|e| format!("Piano rifiutato: {e}"))?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<PlanItem> = Vec::new();
    for i in &plan.items {
        let key = path_key(&i.path);
        let Some(c) = canonical
            .items
            .iter()
            .find(|c| path_key(&c.path) == key && c.kind == i.kind && same_action(&c.action, &i.action))
        else {
            return Err(format!(
                "Piano rifiutato: l'elemento non corrisponde all'ultima analisi: {}",
                i.path.display()
            ));
        };
        if seen.insert(key) {
            items.push(c.clone());
        }
    }
    let expected_bytes = items.iter().fold(0u64, |acc, i| acc.saturating_add(i.expected_bytes));
    Ok(CleanPlan { id: plan.id.clone(), mode: plan.mode, items, expected_bytes })
}

/// Removes the picked archived sessions (`dc_sessions::delete_sessions`). Every pick must be a
/// session `scan` listed for that harness with `blocked == None`; otherwise nothing is removed.
/// A session picked twice is removed once; one the scan lists twice is refused (ambiguous).
pub fn delete_archived(
    scan: &DevScan,
    picked: &[(Agent, String)],
    mode: RemoveMode,
    cli: &dyn HarnessCli,
    trash: &dyn Trash,
) -> Result<Vec<SessionOutcome>, String> {
    let mut seen = HashSet::new();
    let mut items: Vec<ArchivedSession> = Vec::new();
    for (agent, id) in picked {
        if !seen.insert((*agent, id.as_str())) {
            continue;
        }
        let mut found = scan.archived.iter().filter(|s| s.agent == *agent && s.id == *id);
        let Some(s) = found.next() else {
            return Err(format!("Sessione non presente nell'ultima analisi, rifiutata: {agent:?} {id}"));
        };
        if found.next().is_some() {
            return Err(format!(
                "Sessione presente più volte nell'ultima analisi, rifiutata per sicurezza: {agent:?} {id}"
            ));
        }
        if let Some(why) = &s.blocked {
            return Err(format!("Sessione non cancellabile, rifiutata ({why}): {agent:?} {id}"));
        }
        items.push(s.clone());
    }
    Ok(dc_sessions::delete_sessions(&items, mode, cli, trash))
}

#[cfg(test)]
mod tests;
