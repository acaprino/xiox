//! Targeted analysis of ONE project: everything on the PC that belongs to it.
//!
//! Given a path, [`resolve_project`] finds the project (the main working tree of the git
//! repository that holds it, or the folder itself) and [`analyze_project`] lists, by
//! [`Category`], the folders, worktrees and sessions that belong to it. Nothing here deletes.
//!
//! Every string a user can read is English and built from structured fields: the engine's
//! Italian texts (`ProjectReport::reason`, `Artifact::label`/`note`, `ArchivedSession::blocked`,
//! warnings) are never copied.

mod clean;
mod container;
mod hint;
mod history;
mod identity;
mod level;
mod logs;
mod nested;
mod owned;
mod provenance;
mod resolve;
mod search;

#[cfg(test)]
mod tests;

pub use clean::{
    CleanOutcome, CleanRequest, Chosen, Confirmation, Method, Outcome, OutcomeItem, SelectError, clean, clean_in,
    clean_without_sessions, method_of, recycle_shortfall, select,
};
pub use xiox_git::normalize_remote;
pub use hint::{is_recommended, is_top_level, too_shallow};
pub use history::{
    Activity, CallOutcome, ConflictReport, Exclusion, HistoryReport, Limit, Operation, OperationSource, Provenance, ReadFailure, Risk,
    SessionEvidence, SourceReport,
};
pub use owned::key_within;
pub use provenance::{
    Candidate, Completeness, Coverage, Env as PathEnv, EvidenceRef, Extraction, Flags, FolderEvidence, Ledger, OpKind,
    PatchAction, RawOp, Resolved, SessionKey, Shell, ShellCommand, Source, Strength, effective_cwd, extract,
    folder_of, resolve, session_ledgers, shell_command,
};
pub use resolve::{linked_worktree_count, resolve_project};

use xiox_model::{Agent, ArtifactKind};

pub use identity::FolderId;
pub use level::{DANGER_TEXT, Level};
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};

/// Version of the JSON shape of [`ProjectScan`].
pub const SCHEMA: u32 = 2;

/// What kind of thing an [`Item`] is. The order of the variants is the order of the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Builds,
    Worktrees,
    Copies,
    Named,
    /// A folder the session history shows an agent made: listed for review only.
    Scratch,
    Logs,
    Sessions,
}

/// Why an item cannot be removed at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Block {
    LiveSession,
    DirtyGit,
    UnpushedCommits,
    Unreachable,
    SessionLocked,
    /// A copy of the repository with commits or changes the project does not have.
    UnsavedWork,
    /// A copy, or a folder that is or holds a git repository, whose unsaved work git could not
    /// check (it cannot read the repository, or answered nothing).
    UnsavedWorkUnknown,
    /// Sessions of another project wrote in the folder too.
    OtherProjectWrites,
    /// The folder holds git repositories of its own.
    HoldsRepositories,
    /// The folder holds files git tracks.
    TrackedFiles,
    /// A worktree someone kept with `git worktree lock`.
    WorktreeLocked,
    Unknown,
}

impl Block {
    /// The English reason, in the one wording used everywhere (the listing, `--include` errors, the guide).
    pub fn reason(self) -> &'static str {
        match self {
            Block::LiveSession => "an agent session is open",
            Block::DirtyGit => "uncommitted changes",
            Block::UnpushedCommits => "commits not kept anywhere else",
            Block::Unreachable => "could not be checked",
            Block::SessionLocked => "session locked by its harness",
            Block::UnsavedWork => "has commits or changes the project does not have",
            Block::UnsavedWorkUnknown => "could not be checked for unsaved work",
            Block::OtherProjectWrites => "sessions of another project wrote here",
            Block::HoldsRepositories => "holds git repositories",
            Block::TrackedFiles => "holds files tracked by git",
            Block::WorktreeLocked => "locked with git worktree lock",
            Block::Unknown => "unknown reason",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Category::Builds => "builds",
            Category::Worktrees => "worktrees",
            Category::Copies => "copies",
            Category::Named => "named",
            Category::Scratch => "scratch",
            Category::Logs => "logs",
            Category::Sessions => "sessions",
        })
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// Why the item belongs to the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Why {
    /// A folder of the build/cache catalog; English label from `ArtifactKind`/`Class`, e.g. "Rust build".
    CatalogFolder { label_en: String },
    LinkedWorktree { branch: Option<String>, head: Option<String> },
    SameRepository { shared_root_commit: bool, same_origin: bool, has_own_work: Option<bool> },
    NameOnly,
    LogFile { ignored: bool },
    ArchivedSession { agent: Agent, title: Option<String> },
    /// A folder that agent sessions made or used, found in the session history.
    SessionFolder { strength: Strength, from_script: bool, activity: Activity },
}

/// What removing the item means.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Target {
    Dir { kind: ArtifactKind },
    File,
    /// `git worktree remove` run from `repo`.
    Worktree { repo: PathBuf },
    Session { agent: Agent, id: String },
    /// A folder inside the project or one of its worktrees; `repo` is that repository.
    Subtree { repo: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// `path_key(path)`; for sessions `session:<agent>:<id>`.
    pub key: String,
    pub category: Category,
    /// `None` for sessions.
    pub path: Option<PathBuf>,
    /// `None` when not measured (light mode).
    pub bytes: Option<u64>,
    pub level: Level,
    /// What makes the item risky to remove, in the order of [`Risk`], without repeats.
    pub risks: Vec<Risk>,
    /// The folder as it was when analysed; checked again before removal.
    #[serde(skip)]
    pub identity: Option<FolderId>,
    /// Non-empty: cannot be removed at all.
    pub blocked: Vec<Block>,
    pub why: Why,
    pub target: Target,
    /// The session evidence behind a `scratch` item, and behind the `named`, `copies` and
    /// `worktrees` folders the sessions speak about; `None` otherwise.
    pub provenance: Option<Provenance>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Main working tree of the repository, or the given folder when it is not one. Normalized.
    pub root: PathBuf,
    /// Last component of `root`.
    pub name: String,
    pub is_repo: bool,
    /// `remote.origin.url` without the credentials of its authority (`user:token@`).
    pub origin: Option<String>,
    /// Linked worktrees whose folder exists (main excluded). Normalized.
    pub worktrees: Vec<PathBuf>,
}

/// How `copies` and `named` were searched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchMode {
    MftIndex,
    BoundedWalk,
    ParentOnly,
}

/// Something the user should know about the listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Note {
    NotARepository,
    GitMissing,
    NameTooShort,
    WalkMayMissDeepFolders,
    /// A registered worktree whose folder is gone: information only.
    PrunableWorktree { path: PathBuf },
    /// The engine reported at least one warning (its text is not English, so it is not copied).
    EngineWarning,
    /// The session listing of this PC failed: live agent sessions may be missed and archived
    /// sessions may be missing.
    SessionsNotRead,
    /// Git could not list the files of the project (or of a worktree): no log files are listed from it.
    LogsNotChecked,
    /// The folder is a drive root, the user profile (or a folder above it), or holds several
    /// projects: the category flags of `clean` refuse it.
    ContainerRoot,
    /// The session history was read only in part: a cap was hit or a store could not be read.
    HistoryPartial,
    /// The session history could not be read at all, so `scratch` is empty.
    HistoryNotRead,
}

/// A local branch that looks stale: listed, never deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleBranch {
    pub name: String,
    pub merged: bool,
    pub worktree_missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScan {
    /// [`SCHEMA`].
    pub schema: u32,
    pub project: Project,
    /// Sorted by category order, then bytes descending.
    pub items: Vec<Item>,
    pub branches: Vec<StaleBranch>,
    pub notes: Vec<Note>,
    /// What the session history gave; `None` when it was not asked for or had no source.
    pub history: Option<HistoryReport>,
    pub search: SearchMode,
    pub scanned_at_ms: i64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Search every fixed drive (`false`: only the project's parent).
    pub drives: bool,
    /// Measure folder sizes (`false`: light mode, `bytes` is `None` except for sessions).
    pub measure: bool,
    /// The process is elevated (the MFT index may be read).
    pub admin: bool,
    /// Read the session history for folders the project does not list otherwise.
    pub history: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { drives: true, measure: true, admin: false, history: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "path", rename_all = "kebab-case")]
pub enum Error {
    PathNotFound(PathBuf),
    NotADirectory(PathBuf),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::PathNotFound(p) => write!(f, "path not found: {}", p.display()),
            Error::NotADirectory(p) => write!(f, "not a directory: {}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

/// Whether the session listings of this PC could be read. A failed live listing blinds the
/// live-session block, so `clean` leaves worktrees, copies and named folders alone then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionHealth {
    pub live_failed: bool,
    pub archived_failed: bool,
}

/// The health of the listings that [`xiox_app::load_sessions`] and [`xiox_app::load_archived_sessions`]
/// returned. A listing that returned nothing and complained failed; one that returned sessions
/// and complained about some it left out (no working folder, no date) did not.
pub(crate) fn session_health(
    sessions: &[xiox_app::Preloaded],
    loader_warnings: &[String],
    archived: &[xiox_mukade::ArchivedSession],
    archived_warnings: &[String],
) -> SessionHealth {
    SessionHealth {
        live_failed: !loader_warnings.is_empty()
            || sessions.iter().any(|p| p.loaded.items.is_empty() && !p.loaded.warnings.is_empty()),
        archived_failed: archived.is_empty() && !archived_warnings.is_empty(),
    }
}

/// Everything outside the project tree, injected (tests pass fakes).
pub(crate) struct Env<'a> {
    pub sessions: &'a [xiox_app::Preloaded],
    pub archived: &'a [xiox_mukade::ArchivedSession],
    /// Whether the listings above could be read (tests pass the default: all fine).
    pub health: SessionHealth,
    /// Where `copies`/`named` are walked below the project's parent, to depth 3; `None` = every
    /// fixed drive and the user's home.
    pub search_roots: Option<Vec<PathBuf>>,
    pub now_ms: i64,
    pub history: history::HistoryInput<'a>,
}

/// Analyze the project that holds `path` with the real sessions of this PC, history included
/// unless `opts.history` is off. Writes nothing except the session listing cache the engine
/// already writes (`xiox_app::cache_dir()`); an OpenCode database is read from a temporary copy.
pub fn analyze_project(path: &Path, opts: &Options) -> Result<ProjectScan, Error> {
    with_real_env(xiox_app::cache_dir(), opts.history, |env| analyze_with(path, opts, env))
}

/// [`analyze_project`] for the probe: no folder sizes, a session cache under the temporary
/// folder instead of Xiox's own, and a history budget of `(sessions, calls, seconds)` (`None`:
/// the one `analyze_project` uses, see [`history_budget`]).
pub fn history_probe(path: &Path, budget: Option<(usize, usize, u64)>) -> Result<ProjectScan, Error> {
    let budget = budget.map_or(history::BUDGET, |(max_sessions, max_calls, secs)| history::Budget {
        max_sessions,
        max_calls,
        max_duration: std::time::Duration::from_secs(secs),
    });
    let cache = std::env::temp_dir().join("xiox-session-probe");
    with_real_env(cache, true, |env| probe_with(path, budget, env))
}

/// The history budget `analyze_project` reads within, as `(sessions, calls, seconds)`.
pub fn history_budget() -> (usize, usize, u64) {
    let b = history::BUDGET;
    (b.max_sessions, b.max_calls, b.max_duration.as_secs())
}

/// The sessions listing of every harness; without the history there is nothing to list, so no
/// store is asked and nothing is cached.
fn real_listings(history: bool, cache: Option<PathBuf>) -> Vec<(Agent, xiox_model::Loaded<xiox_mukade::Descriptor>)> {
    if history { xiox_app::load_descriptors(cache) } else { Vec::new() }
}

/// The real sessions and history sources of this PC, listed through the cache in `cache`.
pub(crate) fn with_real_env<R>(cache: PathBuf, history: bool, run: impl FnOnce(&Env) -> R) -> R {
    let cache = Some(cache);
    let (sessions, loader_warnings) = xiox_app::load_sessions(cache.clone());
    let (archived, archived_warnings) = xiox_app::load_archived_sessions(cache.clone());
    let health = session_health(&sessions, &loader_warnings, &archived, &archived_warnings);
    let listings = real_listings(history, cache.clone());
    let sources: Vec<xiox_mukade::MukadeSessions> = listings
        .iter()
        .map(|(agent, _)| xiox_mukade::MukadeSessions::from_env(*agent).with_cache_dir(cache.clone()))
        .collect();
    let calls = history::MukadeCalls::new(&sources);
    let user = history::UserDirs::from_env();
    let env = Env {
        sessions: &sessions,
        archived: &archived,
        health,
        search_roots: None,
        now_ms: xiox_app::now_ms(),
        history: history::HistoryInput { listings: &listings, calls: Some(&calls), user: &user },
    };
    run(&env)
}

/// [`analyze_project`] with `copies`/`named` searched only below `search_roots` (to depth 3)
/// instead of the fixed drives and the home folder. Used by tests and by debug builds of `xiox`.
/// Reads the sessions, and with `opts.history` their history, from the stores the environment
/// names, as [`analyze_project`] and [`clean::clean_in`] do.
pub fn analyze_project_in(path: &Path, opts: &Options, search_roots: &[PathBuf]) -> Result<ProjectScan, Error> {
    with_real_env(xiox_app::cache_dir(), opts.history, |real| {
        let env = Env {
            sessions: real.sessions,
            archived: real.archived,
            health: real.health,
            search_roots: Some(search_roots.to_vec()),
            now_ms: real.now_ms,
            history: real.history,
        };
        analyze_with(path, opts, &env)
    })
}

/// [`analyze_project_in`] without reading any session of this PC (no `sessions` items, no live
/// session blocks, no history, nothing written to the cache). For tests that run on fake trees.
pub fn analyze_project_without_sessions(
    path: &Path,
    opts: &Options,
    search_roots: &[PathBuf],
) -> Result<ProjectScan, Error> {
    let env = Env {
        sessions: &[],
        archived: &[],
        health: SessionHealth::default(),
        search_roots: Some(search_roots.to_vec()),
        now_ms: xiox_app::now_ms(),
        history: history::HistoryInput::none(),
    };
    analyze_with(path, opts, &env)
}

/// True when the process runs with an elevated token (an administrator that approved UAC).
/// Same check as the window app's `sys::is_admin`.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: no arguments, no pointers; returns a BOOL.
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// [`analyze_project`] with everything outside the project tree injected.
pub(crate) fn analyze_with(path: &Path, opts: &Options, env: &Env) -> Result<ProjectScan, Error> {
    owned::analyze_with(path, opts, env, history::BUDGET, &[])
}

/// [`analyze_with`] that also verifies `known`, folders found some other way than the search.
pub(crate) fn analyze_known(path: &Path, opts: &Options, env: &Env, known: &[PathBuf]) -> Result<ProjectScan, Error> {
    owned::analyze_with(path, opts, env, history::BUDGET, known)
}

/// The probe's analysis: light mode, history on, the given budget.
pub(crate) fn probe_with(path: &Path, budget: history::Budget, env: &Env) -> Result<ProjectScan, Error> {
    let opts = Options { measure: false, ..Options::default() };
    owned::analyze_with(path, &opts, env, budget, &[])
}
