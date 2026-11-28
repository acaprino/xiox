//! UI-less glue between the engine crates and Xiox: settings, the developer scan,
//! the clean plan and the removal of archived agent sessions. Ported from the old devclean Tauri
//! shell (`src-tauri/src/lib.rs`) without Tauri and without any external disk-size tool: folder
//! sizes come from the NTFS MFT (`xiox-disktree`) when the app runs as administrator, otherwise
//! from the walker.
//!
//! Safety: `execute_plan` and `delete_archived` never trust what the caller passes. Every plan
//! item must be a path the given scan found (an artifact, or the root of an agent worktree),
//! and every session must be one the scan listed as archived and removable; anything else
//! refuses the whole request before a single file is touched. Sessions are checked once more
//! against a fresh listing right before they go ([`revalidate_archived`]): an analysis can be
//! hours old.

pub use xiox_cleaner::{Cancel, Tick};
use xiox_model::{
    APP_DIR, Agent, AnalysisResult, CleanPlan, CleanReport, CleanResult, DEFAULT_INACTIVE_DAYS, DirIndex, EngineConfig, ItemOutcome,
    Level, Loaded, PlanAction, PlanItem, ProjectCandidate, ProjectLayout, ProjectReport, ProjectScanner, RemoveMode, ScanOptions,
    Selection, SessionRecord, SessionSource, Settings, normalize_path, path_key,
};
use xiox_mukade::{ArchivedSession, Descriptor, MukadeSessions};
use xiox_sessions::{HarnessCli, SessionOutcome, Trash};
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::cell::Cell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AGENTS: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi];

// ───────────────────────────────── settings ─────────────────────────────────

/// Milliseconds since the Unix epoch (0 if the clock is before it).
pub fn now_ms() -> i64 {
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
        all_drives: true,
        layout: ProjectLayout::Tree,
    }
}

/// `%APPDATA%\Xiox\settings.json`: where versions before `pref.json` ([`prefs`]) kept the Developers options, read
/// once when `pref.json` has no Developers section yet.
pub fn settings_path() -> PathBuf {
    settings_path_in(&env_dir("APPDATA", &["AppData", "Roaming"]))
}

/// [`settings_path`] under the given `%APPDATA%`.
pub fn settings_path_in(appdata: &Path) -> PathBuf {
    appdata.join(APP_DIR).join("settings.json")
}

/// `%LOCALAPPDATA%\Xiox\mukade`: mukade's session listing cache. Nothing else is written
/// there, and mukade never writes beside the executable or in the harnesses' stores.
pub fn cache_dir() -> PathBuf {
    cache_dir_in(&env_dir("LOCALAPPDATA", &["AppData", "Local"]))
}

/// [`cache_dir`] under the given `%LOCALAPPDATA%`.
pub fn cache_dir_in(local_appdata: &Path) -> PathBuf {
    local_appdata.join(APP_DIR).join("mukade")
}

/// The Developers section of `pref.json`; without one, the old `settings.json`; without that, the defaults.
pub fn load_settings() -> Settings {
    load_settings_from(&prefs::Store::beside_exe(), &settings_path())
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    prefs::Store::beside_exe().write(prefs::DEVELOPERS, s)
}

fn load_settings_from(store: &prefs::Store, legacy: &Path) -> Settings {
    store.read_over(prefs::DEVELOPERS, default_settings()).unwrap_or_else(|| read_settings(legacy))
}

/// Missing or unreadable file: the defaults.
fn read_settings(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_settings)
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
    /// When the scan started ([`now_ms`]); 0 = unknown, as old as it gets.
    pub scanned_at_ms: i64,
}

/// How long an analysis may be used to clean projects and worktrees.
pub const MAX_SCAN_AGE_MS: i64 = 30 * 60 * 1000;

/// Why a plan from an analysis older than [`MAX_SCAN_AGE_MS`] is refused.
pub const STALE_ANALYSIS: &str = "The analysis is too old: press “Analyze” again.";

impl DevScan {
    /// Too old to plan a cleanup from: more than [`MAX_SCAN_AGE_MS`] before `now_ms`, or of
    /// unknown age. A clock that went backwards counts as fresh.
    pub fn is_stale(&self, now_ms: i64) -> bool {
        self.scanned_at_ms <= 0 || now_ms.saturating_sub(self.scanned_at_ms) > MAX_SCAN_AGE_MS
    }
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

/// All four agents through the mukade executable: listings only, read-only. A source
/// that panics becomes a warning. The managed worktrees are kept only if they exist
/// (`xiox_mukade::claude_worktrees` names the worktree a session folder points into without
/// checking it is still there).
pub fn load_sessions(cache_dir: Option<PathBuf>) -> (Vec<Preloaded>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = Vec::new();
    for agent in AGENTS {
        let src = MukadeSessions::from_env(agent).with_cache_dir(cache_dir.clone());
        let loaded = catch_unwind(AssertUnwindSafe(|| src.load())).unwrap_or_else(|_| Loaded {
            items: Vec::new(),
            warnings: vec![format!("{agent:?}: internal error while reading the sessions, ignored")],
        });
        let worktrees = catch_unwind(AssertUnwindSafe(|| src.managed_worktrees())).unwrap_or_else(|_| {
            warnings.push(format!("{agent:?}: could not read the managed worktrees"));
            Vec::new()
        });
        let worktrees = worktrees.into_iter().filter(|p| p.is_dir()).collect();
        out.push(Preloaded { agent, loaded, worktrees });
    }
    (out, warnings)
}

/// Every session of every harness, children included, one listing per agent in [`AGENTS`] order.
/// A source that panics becomes a warning.
pub fn load_descriptors(cache_dir: Option<PathBuf>) -> Vec<(Agent, Loaded<Descriptor>)> {
    AGENTS
        .into_iter()
        .map(|agent| {
            let src = MukadeSessions::from_env(agent).with_cache_dir(cache_dir.clone());
            (agent, guarded_listing(agent, || src.descriptors()))
        })
        .collect()
}

/// `list` with a panic turned into an empty listing and a warning.
fn guarded_listing(agent: Agent, list: impl FnOnce() -> Loaded<Descriptor>) -> Loaded<Descriptor> {
    catch_unwind(AssertUnwindSafe(list)).unwrap_or_else(|_| Loaded {
        items: Vec::new(),
        warnings: vec![format!("{agent:?}: internal error while listing the sessions, ignored")],
    })
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
                .push(format!("{agent:?}: internal error while reading the archived sessions, ignored")),
        }
    }
    out
}

/// Archived root sessions of every harness, read as [`run_scan`] reads them, and the warnings.
pub fn load_archived_sessions(cache_dir: Option<PathBuf>) -> (Vec<ArchivedSession>, Vec<String>) {
    let l = load_archived(cache_dir);
    (l.items, l.warnings)
}

/// The roots one scan walks, and where they came from.
pub struct ScanRoots {
    /// Outer roots only, sorted by `path_key`: a root inside another one is walked as part of it.
    pub roots: Vec<PathBuf>,
    /// What the session working directories gave (roots and skipped folders with the reason).
    pub from_sessions: xiox_scanner::SessionRoots,
    /// Folders holding many projects, for the engine's session matching
    /// (`EngineConfig::container_roots`): the home, every folder in `settings.roots` and the
    /// drive roots searched whole.
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
/// `xiox_scanner::project_roots_from_sessions`), plus the agents' managed worktrees that exist,
/// plus the folders in `settings.roots` when `settings.scan_roots` is on, plus `drives`: drive
/// roots (`X:\`) searched whole. A root inside another one is dropped, so every folder on a
/// drive in `drives` is searched as part of that drive.
pub fn plan_roots(settings: &Settings, sessions: &[Preloaded], home: Option<&Path>, drives: &[PathBuf]) -> ScanRoots {
    let drives: Vec<PathBuf> = drives.iter().map(|d| normalize_path(d)).filter(|d| d.is_dir()).collect();
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
    let from_sessions = xiox_scanner::project_roots_from_sessions(&session_dirs, home);
    let worktrees = sessions.iter().flat_map(|s| s.worktrees.iter()).filter(|p| p.is_dir());

    // Outer roots first (shorter key), then drop every root inside one already kept.
    let mut all: Vec<PathBuf> = drives
        .iter()
        .cloned()
        .chain(user)
        .chain(from_sessions.roots.iter().chain(worktrees).map(|p| normalize_path(p)))
        .collect();
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
    container_roots.extend(drives);
    ScanRoots { roots: kept.into_iter().map(|(_, p)| p).collect(), from_sessions, container_roots }
}

/// Hands the index only the roots it covers. `xiox_disktree::build_for_roots` leaves out the
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

/// The bar between «Analyzing projects…» (45%) and «Archived sessions…» (90%): the share of the
/// projects probed so far. Never leaves that range, so the bar only moves forward.
pub fn analysis_fraction(done: usize, total: usize) -> f32 {
    const FROM: f32 = 0.45;
    const TO: f32 = 0.9;
    if total == 0 {
        return FROM;
    }
    FROM + (TO - FROM) * (done.min(total) as f32 / total as f32)
}

/// The label of the bar while the projects are probed: how many are done, once there is a count.
pub fn analysis_text(done: usize, total: usize) -> String {
    if total == 0 {
        "Analyzing projects…".to_string()
    } else {
        format!("Analyzing projects… {done} of {total}")
    }
}

/// The least time between two progress reports of the project probe.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

const FAST_READ_FAILED: &str = "Fast disk read failed: using the normal scan.";
pub const ANALYSIS_FAILED: &str = "Internal error while analyzing the projects.";

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
/// `settings.all_drives`: every fixed drive is searched whole ([`xiox_disktree::fixed_drives`]):
/// the NTFS ones through their MFT, the others with the walker. An NTFS drive whose MFT cannot
/// be read (or any NTFS drive without `admin`) falls back to the roots of [`plan_roots`] on it.
///
/// `admin`: the MFT index (`xiox-disktree`) is built for ALL the roots and handed to the scanner,
/// which takes the artifact sizes from it (roots it does not cover are walked). Without `admin`
/// there is no index and the `xiox-scanner` walker does everything. The four steps are always
/// reported. A panic while building the index falls back to the walker; a panic in the analysis
/// gives no reports and a warning (the archived sessions are still listed). Nothing is written
/// except mukade's listing cache in [`cache_dir`].
pub fn run_scan(settings: &Settings, admin: bool, step: &dyn Fn(&str, f32)) -> DevScan {
    let started = Instant::now();
    let scanned_at_ms = now_ms();
    let cache = Some(cache_dir());

    step("Reading sessions…", 0.05);
    let (sessions, mut warnings) = load_sessions(cache.clone());
    let home = home_dir();
    let fixed = if settings.all_drives { xiox_disktree::fixed_drives() } else { Vec::new() };
    // The roots without the drives: on an NTFS drive whose MFT cannot be read, they are what
    // gets searched.
    let base = plan_roots(settings, &sessions, home.as_deref(), &[]);

    // Always reported, so the UI shows the same four steps; without admin nothing is read here.
    step("Fast disk read…", 0.25);
    let drive_keys: Vec<String> = fixed.iter().map(|d| path_key(&d.root())).collect();
    let mut index_roots: Vec<PathBuf> = fixed.iter().filter(|d| d.ntfs).map(|d| d.root()).collect();
    // A root on a fixed drive is read with its drive (NTFS) or walked with it (not NTFS).
    index_roots.extend(
        base.roots.iter().filter(|r| !drive_keys.iter().any(|k| key_within(&path_key(r), k))).cloned(),
    );
    let index = if admin && !index_roots.is_empty() {
        guarded_index(
            || xiox_disktree::build_for_roots(&index_roots, true, &xiox_disktree::Progress::default()),
            &mut warnings,
        )
    } else {
        None
    };
    // Searched whole: the NTFS drives the index covers, and every drive that is not NTFS.
    let drives: Vec<PathBuf> = fixed
        .iter()
        .filter(|d| !d.ntfs || index.as_ref().is_some_and(|ix| ix.size_of(&d.root()).is_some()))
        .map(|d| d.root())
        .collect();
    let plan = if drives.is_empty() { base } else { plan_roots(settings, &sessions, home.as_deref(), &drives) };
    if plan.roots.is_empty() {
        warnings.push(if settings.session_projects {
            "No project found in the agents' sessions: add folders to analyze in the settings.".into()
        } else {
            "No folders to analyze.".into()
        });
    }
    let backend = index.as_ref().map_or("walker", |i| i.backend_name()).to_string();

    step("Analyzing projects…", 0.45);
    let cfg = EngineConfig {
        roots: plan.roots.clone(),
        container_roots: plan.container_roots.clone(),
        inactive_days: settings.inactive_days,
        scan: ScanOptions::default(),
        now_ms: now_ms(),
    };
    let sources: Vec<&dyn SessionSource> = sessions.iter().map(|s| s as &dyn SessionSource).collect();
    let scanner = SplitScanner { inner: &xiox_scanner::DefaultScanner };
    // The slow part: a few `git` processes per project. It says how far it is, so the bar moves
    // from 45% to 90% instead of sitting still for minutes.
    let reported = Cell::new(None::<Instant>);
    let analysis = guarded_analysis(|| {
        xiox_engine::analyze_with_progress(
            &cfg,
            &scanner,
            index.as_ref().map(|i| i as &dyn DirIndex),
            &sources,
            &xiox_git::GitCli,
            &|done, total| {
                // Thousands of projects must not flood the window with updates: a few per second.
                let due = done == 0 || reported.get().is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
                if due {
                    reported.set(Some(Instant::now()));
                    step(&analysis_text(done, total), analysis_fraction(done, total));
                }
            },
        )
    });
    drop(index);
    warnings.extend(analysis.warnings);
    let (projects, worktrees, other_worktrees) = split_reports(analysis.reports);

    step("Archived sessions…", 0.9);
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
        scanned_at_ms,
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

/// Whether [`selection_for_projects`] gives this report at least one item: the whole agent
/// worktree, or one of its `Recommended` artifacts. Status and blocks are not looked at.
pub fn plans_something(report: &ProjectReport) -> bool {
    xiox_engine::whole_worktree_repo(report).is_some()
        || report.artifacts.iter().any(|a| a.level == Level::Recommended)
}

/// The per-project tick of the Developers list as the engine's selection (used by `xiox_core::dev`).
///
/// Report ids (one checkbox per project/worktree) -> the engine's [`Selection`]: an agent
/// worktree removed whole goes in `worktrees`; any other report gives only its `Recommended`
/// artifacts, so a per-project tick never plans a git-tracked, unverified or dependency folder
/// (those need a pick of their own). An id that is not a project or agent worktree of `scan` is
/// kept as an artifact id, so that [`build_plan`] refuses it.
pub fn selection_for_projects(scan: &DevScan, project_ids: &[String]) -> Selection {
    let mut sel = Selection::default();
    for id in project_ids {
        match selectable(scan).find(|r| &r.id == id) {
            None => sel.artifacts.push(id.clone()),
            Some(r) if xiox_engine::whole_worktree_repo(r).is_some() => sel.worktrees.push(id.clone()),
            Some(r) => sel
                .artifacts
                .extend(r.artifacts.iter().filter(|a| a.level == Level::Recommended).map(|a| a.id.clone())),
        }
    }
    sel
}

/// [`xiox_engine::build_plan`] over the projects and agent worktrees of `scan`: the user picks
/// folders by [`Artifact::id`] (any level) and agent worktrees to remove whole by project id.
pub fn build_plan(scan: &DevScan, sel: &Selection, mode: RemoveMode) -> Result<CleanPlan, String> {
    xiox_engine::build_plan(&selectable_reports(scan), sel, mode).map_err(|e| e.to_string())
}

/// Runs the plan for real (no dry run) with [`xiox_cleaner::ParallelRemover`] and
/// [`xiox_cleaner::worker_count`] workers, after [`checked_plan`]: only items the scan itself
/// would plan are executed. An analysis older than [`MAX_SCAN_AGE_MS`] is refused
/// ([`STALE_ANALYSIS`]) before anything is touched. `tick` reports progress (at most every
/// 100 ms, from one thread); `cancel` stops before the next file.
pub fn execute_plan(
    scan: &DevScan,
    plan: &CleanPlan,
    tick: &(dyn Fn(Tick) + Sync),
    cancel: &Cancel,
) -> Result<CleanResult, String> {
    execute_plan_at(scan, plan, now_ms(), tick, cancel)
}

/// [`execute_plan`] with the current time given: the age check uses `now_ms`.
pub fn execute_plan_at(
    scan: &DevScan,
    plan: &CleanPlan,
    now_ms: i64,
    tick: &(dyn Fn(Tick) + Sync),
    cancel: &Cancel,
) -> Result<CleanResult, String> {
    let checked = runnable_plan(scan, plan, now_ms)?;
    let remover = xiox_cleaner::ParallelRemover { workers: xiox_cleaner::worker_count() };
    Ok(remover.execute_with(&checked, false, tick, cancel))
}

/// The age check, then [`checked_plan`].
fn runnable_plan(scan: &DevScan, plan: &CleanPlan, now_ms: i64) -> Result<CleanPlan, String> {
    if scan.is_stale(now_ms) {
        return Err(STALE_ANALYSIS.into());
    }
    checked_plan(scan, plan)
}

/// [`execute_plan_at`] through any [`Remover`] (tests).
#[cfg(test)]
fn execute_plan_with(
    scan: &DevScan,
    plan: &CleanPlan,
    now_ms: i64,
    remover: &dyn xiox_model::Remover,
) -> Result<CleanResult, String> {
    Ok(remover.execute(&runnable_plan(scan, plan, now_ms)?, false))
}

/// The volumes whose Recycle Bin cannot take what `plan` would move there:
/// `(drive letter, bytes the plan moves, free room in the bin)`, from
/// [`xiox_cleaner::recycle_room`]. Empty for a `Permanent` plan, and for volumes whose room is
/// unknown. The UI asks to delete permanently instead of letting Windows decide per file.
pub fn recycle_shortfall(plan: &CleanPlan) -> Vec<(char, u64, u64)> {
    recycle_shortfall_with(plan, xiox_cleaner::recycle_room)
}

/// [`recycle_shortfall`] with the room of each bin given (tests).
fn recycle_shortfall_with(plan: &CleanPlan, room: impl Fn(char) -> Option<u64>) -> Vec<(char, u64, u64)> {
    if plan.mode == RemoveMode::Permanent {
        return Vec::new();
    }
    let mut per_volume: std::collections::BTreeMap<char, u64> = std::collections::BTreeMap::new();
    // A worktree removal is always permanent (git deletes it): only `RemoveDir` uses the Bin.
    for item in plan.items.iter().filter(|i| matches!(i.action, PlanAction::RemoveDir)) {
        if let Some(letter) = drive_letter(&item.path) {
            let sum = per_volume.entry(letter).or_default();
            *sum = sum.saturating_add(item.expected_bytes);
        }
    }
    let needed: Vec<(char, u64)> = per_volume.into_iter().collect();
    xiox_cleaner::shortfall(&needed, room)
}

/// Uppercase drive letter of `C:\…` or `\\?\C:\…`; `None` for UNC and relative paths.
fn drive_letter(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

/// Category of the agent worktrees removed whole, in [`report`].
pub const WORKTREE_CATEGORY: &str = "Agent worktrees";

/// The final summary of an [`execute_plan`] run. Category = the artifact's `label` (an agent
/// worktree: [`WORKTREE_CATEGORY`]), largest first. `recycled_bytes` and `freed_bytes` come
/// from the remover's totals (`res.recycled_bytes`, the rest of `res.bytes_freed`).
pub fn report(scan: &DevScan, plan: &CleanPlan, res: &CleanResult) -> CleanReport {
    let labels: HashMap<String, &str> = selectable(scan)
        .flat_map(|r| r.artifacts.iter().map(|a| (path_key(&a.path), a.label.as_str())))
        .collect();
    let worktrees: HashSet<String> = scan.worktrees.iter().map(|r| path_key(&r.root)).collect();
    let actions: HashMap<String, &PlanAction> = plan.items.iter().map(|i| (path_key(&i.path), &i.action)).collect();

    // The remover knows what went to the Recycle Bin: its totals are the source of truth.
    let mut out = CleanReport {
        recycled_bytes: res.recycled_bytes,
        freed_bytes: res.bytes_freed.saturating_sub(res.recycled_bytes),
        files: res.files_removed,
        dirs: res.dirs_removed,
        elapsed_ms: res.elapsed_ms,
        cancelled: res.cancelled,
        ..Default::default()
    };
    let mut by_category: Vec<(String, u64)> = Vec::new();
    for it in &res.items {
        let key = path_key(&it.path);
        match &it.outcome {
            ItemOutcome::Removed { bytes_freed } => {
                let action = actions.get(&key).copied();
                let label = match (action, labels.get(&key)) {
                    (Some(PlanAction::GitWorktreeRemove { .. }), _) => WORKTREE_CATEGORY.to_string(),
                    (_, Some(l)) => (*l).to_string(),
                    _ if worktrees.contains(&key) => WORKTREE_CATEGORY.to_string(),
                    _ => it.path.display().to_string(),
                };
                match by_category.iter_mut().find(|(l, _)| *l == label) {
                    Some((_, b)) => *b += bytes_freed,
                    None => by_category.push((label, *bytes_freed)),
                }
            }
            ItemOutcome::Skipped { reason } => out.skipped.push((it.path.clone(), reason.clone())),
            ItemOutcome::Failed { error, locked_by } => {
                let why = if locked_by.is_empty() {
                    error.clone()
                } else {
                    format!("{error} (in use by: {})", locked_by.join(", "))
                };
                out.failed.push((it.path.clone(), why));
            }
        }
    }
    // Largest first; equal sizes by label, so the order is stable.
    by_category.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.by_category = by_category;
    out
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
/// 2. the [`Selection`] is rebuilt from the items (`RemoveDir` -> the id of the artifact at that
///    path; `GitWorktreeRemove` -> its `project_id`), the canonical plan is rebuilt with
///    [`build_plan`] (so an `InUse` or blocked project is refused), and every item must match a
///    canonical one on `path_key(path)`, `project_id`, `kind` and `action` (for
///    `GitWorktreeRemove`, `repo` compared with `path_key`).
///
/// The result holds the matching canonical items, once each, in the caller's order (a subset of
/// the canonical plan is fine, and so is any folder of any level: picking it is the consent),
/// with the caller's plan id and mode.
fn checked_plan(scan: &DevScan, plan: &CleanPlan) -> Result<CleanPlan, String> {
    let artifact_ids: HashMap<String, &str> = selectable(scan)
        .flat_map(|r| r.artifacts.iter().map(|a| (path_key(&a.path), a.id.as_str())))
        .collect();
    let allowed: HashSet<String> =
        artifact_ids.keys().cloned().chain(scan.worktrees.iter().map(|r| path_key(&r.root))).collect();
    if let Some(bad) = plan.items.iter().find(|i| !allowed.contains(&path_key(&i.path))) {
        return Err(format!("Path not in the latest analysis, refused: {}", bad.path.display()));
    }

    let mut sel = Selection::default();
    for i in &plan.items {
        match i.action {
            // A path that is no artifact (a worktree root as a plain delete) selects nothing: it
            // then matches no canonical item below.
            PlanAction::RemoveDir => {
                if let Some(id) = artifact_ids.get(&path_key(&i.path)) {
                    sel.artifacts.push((*id).to_string());
                }
            }
            PlanAction::GitWorktreeRemove { .. } => sel.worktrees.push(i.project_id.clone()),
        }
    }
    let canonical = build_plan(scan, &sel, plan.mode).map_err(|e| format!("Plan refused: {e}"))?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<PlanItem> = Vec::new();
    for i in &plan.items {
        let key = path_key(&i.path);
        let Some(c) = canonical
            .items
            .iter()
            .find(|c| {
                path_key(&c.path) == key
                    && c.project_id == i.project_id
                    && c.kind == i.kind
                    && same_action(&c.action, &i.action)
            })
        else {
            return Err(format!(
                "Plan refused: the item does not match the latest analysis: {}",
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

/// Why a picked session is left out by [`revalidate_archived`] (shown as «Saltato (…)»).
pub const NO_LONGER_ARCHIVED: &str = "no longer archived or in use";

/// The picks that can still go, and those dropped (each with a `Skipped` outcome).
#[derive(Debug, Default)]
pub struct Revalidated {
    pub keep: Vec<(Agent, String)>,
    pub skipped: Vec<SessionOutcome>,
}

/// The archived sessions of `agent` as they are now: a fresh listing, not the analysis.
/// A listing that panics gives none (so every pick of that harness is skipped).
pub fn fresh_archived(agent: Agent) -> Vec<ArchivedSession> {
    let src = MukadeSessions::from_env(agent).with_cache_dir(Some(cache_dir()));
    catch_unwind(AssertUnwindSafe(|| src.archived().items)).unwrap_or_default()
}

/// Checks the picks against the live state right before [`delete_archived`]: the analysis may be
/// hours old and a session may have been reopened, unarchived or deleted since. `fresh(agent)`
/// lists that harness's archived sessions now ([`fresh_archived`]; injectable for tests) and is
/// called once per harness picked. A pick is kept only if the fresh listing still has exactly
/// one session with its id, archived, with `blocked == None` and the same file as in `scan`;
/// the others are skipped with [`NO_LONGER_ARCHIVED`] and must never reach `delete_archived`.
/// A pick `scan` never listed refuses the whole request, as in `delete_archived`.
pub fn revalidate_archived(
    scan: &DevScan,
    picked: &[(Agent, String)],
    fresh: &dyn Fn(Agent) -> Vec<ArchivedSession>,
) -> Result<Revalidated, String> {
    let mut now: Vec<(Agent, Vec<ArchivedSession>)> = Vec::new();
    let mut seen = HashSet::new();
    let mut out = Revalidated::default();
    for (agent, id) in picked {
        if !seen.insert((*agent, id.as_str())) {
            continue;
        }
        let Some(old) = scan.archived.iter().find(|s| s.agent == *agent && s.id == *id) else {
            return Err(format!("Session not in the latest analysis, refused: {agent:?} {id}"));
        };
        if !now.iter().any(|(a, _)| a == agent) {
            now.push((*agent, fresh(*agent)));
        }
        let listed = &now.iter().find(|(a, _)| a == agent).expect("just listed").1;
        let mut same = listed.iter().filter(|s| s.agent == *agent && s.id == *id);
        let still_fine = match (same.next(), same.next()) {
            (Some(s), None) => {
                s.blocked.is_none() && s.file.as_deref().map(path_key) == old.file.as_deref().map(path_key)
            }
            _ => false,
        };
        if still_fine {
            out.keep.push((*agent, id.clone()));
        } else {
            out.skipped.push(SessionOutcome {
                agent: *agent,
                id: id.clone(),
                outcome: ItemOutcome::Skipped { reason: NO_LONGER_ARCHIVED.into() },
            });
        }
    }
    Ok(out)
}

/// Removes the picked archived sessions (`xiox_sessions::delete_sessions`). Every pick must be a
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
            return Err(format!("Session not in the latest analysis, refused: {agent:?} {id}"));
        };
        if found.next().is_some() {
            return Err(format!(
                "Session listed more than once in the latest analysis, refused for safety: {agent:?} {id}"
            ));
        }
        if let Some(why) = &s.blocked {
            return Err(format!("Session cannot be deleted, refused ({why}): {agent:?} {id}"));
        }
        items.push(s.clone());
    }
    Ok(xiox_sessions::delete_sessions(&items, mode, cli, trash))
}

pub mod prefs;

#[cfg(test)]
mod tests;
