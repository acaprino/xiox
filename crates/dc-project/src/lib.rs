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
mod logs;
mod owned;
mod resolve;
mod search;

#[cfg(test)]
mod tests;

pub use clean::{
    CleanOutcome, CleanRequest, Chosen, Outcome, SelectError, clean, recycle_shortfall, select,
};
pub use dc_git::normalize_remote;
pub use owned::key_within;
pub use resolve::resolve_project;

use dc_model::{Agent, ArtifactKind, Level};
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};

/// Version of the JSON shape of [`ProjectScan`].
pub const SCHEMA: u32 = 1;

/// What kind of thing an [`Item`] is. The order of the variants is the order of the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Builds,
    Worktrees,
    Copies,
    Named,
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
    Unknown,
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Category::Builds => "builds",
            Category::Worktrees => "worktrees",
            Category::Copies => "copies",
            Category::Named => "named",
            Category::Logs => "logs",
            Category::Sessions => "sessions",
        })
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Block::UnsavedWork => "has commits or changes the project does not have",
            Block::LiveSession => "live agent session",
            Block::DirtyGit => "uncommitted changes",
            Block::UnpushedCommits => "commits that no other branch has",
            Block::Unreachable => "unreachable",
            Block::SessionLocked => "session locked",
            Block::Unknown => "unknown reason",
        })
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
    /// Non-empty: cannot be removed at all.
    pub blocked: Vec<Block>,
    pub why: Why,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Main working tree of the repository, or the given folder when it is not one. Normalized.
    pub root: PathBuf,
    /// Last component of `root`.
    pub name: String,
    pub is_repo: bool,
    /// `remote.origin.url` as git has it.
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
}

impl Default for Options {
    fn default() -> Self {
        Self { drives: true, measure: true, admin: false }
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

/// Everything outside the project tree, injected (tests pass fakes).
pub(crate) struct Env<'a> {
    pub sessions: &'a [dc_app::Preloaded],
    pub archived: &'a [dc_mukade::ArchivedSession],
    /// Where `copies`/`named` are walked below the project's parent, to depth 3; `None` = every
    /// fixed drive and the user's home.
    pub search_roots: Option<Vec<PathBuf>>,
    pub now_ms: i64,
}

/// Analyze the project that holds `path` with the real sessions of this PC. Writes nothing
/// except the session listing cache the engine already writes (`dc_app::cache_dir()`).
pub fn analyze_project(path: &Path, opts: &Options) -> Result<ProjectScan, Error> {
    let cache = Some(dc_app::cache_dir());
    let (sessions, _) = dc_app::load_sessions(cache.clone());
    let (archived, _) = dc_app::load_archived_sessions(cache);
    let env = Env { sessions: &sessions, archived: &archived, search_roots: None, now_ms: dc_app::now_ms() };
    analyze_with(path, opts, &env)
}

/// [`analyze_project`] with `copies`/`named` searched only below `search_roots` (to depth 3)
/// instead of the fixed drives and the home folder. Used by tests and by debug builds of `xiox`.
/// Still reads the real sessions of this PC.
pub fn analyze_project_in(path: &Path, opts: &Options, search_roots: &[PathBuf]) -> Result<ProjectScan, Error> {
    let cache = Some(dc_app::cache_dir());
    let (sessions, _) = dc_app::load_sessions(cache.clone());
    let (archived, _) = dc_app::load_archived_sessions(cache);
    let env = Env {
        sessions: &sessions,
        archived: &archived,
        search_roots: Some(search_roots.to_vec()),
        now_ms: dc_app::now_ms(),
    };
    analyze_with(path, opts, &env)
}

/// [`analyze_project_in`] without reading any session of this PC (no `sessions` items, no live
/// session blocks, nothing written to the cache). For tests that run on fake trees.
pub fn analyze_project_without_sessions(
    path: &Path,
    opts: &Options,
    search_roots: &[PathBuf],
) -> Result<ProjectScan, Error> {
    let env = Env { sessions: &[], archived: &[], search_roots: Some(search_roots.to_vec()), now_ms: dc_app::now_ms() };
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
    owned::analyze_with(path, opts, env)
}
