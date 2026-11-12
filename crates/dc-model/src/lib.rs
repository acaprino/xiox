//! devclean shared contract.
//!
//! Every other crate depends ONLY on this one (plus third-party crates). Concrete
//! implementations are injected through the traits below, so each crate can be built
//! and tested in isolation. Do not add behaviour here beyond the small path helpers:
//! changes to this file are contract changes and go through the integrator.
//!
//! JSON shape (Tauri <-> React): structs are camelCase, enums are kebab-case strings
//! (unit variants) or `{ "type": "...", ... }` (data variants). `src/types.ts` mirrors this.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_INACTIVE_DAYS: u32 = 14;

// ───────────────────────────── errors / helpers ─────────────────────────────

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Other(String),
}

/// Result of a best-effort load: never fails as a whole, problems become warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded<T> {
    pub items: Vec<T>,
    pub warnings: Vec<String>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self { items: Vec::new(), warnings: Vec::new() }
    }
}

/// Strip `\\?\` / `\\?\UNC\` prefixes, use `\` separators (Windows), drop trailing
/// separators (keeps `C:\`). Does not touch the filesystem.
pub fn normalize_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let mut out = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    };
    if cfg!(windows) {
        out = out.replace('/', "\\");
    }
    let keep = if cfg!(windows) { 3 } else { 1 };
    while out.len() > keep && (out.ends_with('\\') || out.ends_with('/')) {
        out.pop();
    }
    PathBuf::from(out)
}

/// Comparison key: normalized + lowercase (Windows paths are case-insensitive).
/// Use this, never raw `==`, to compare paths coming from different sources.
pub fn path_key(p: &Path) -> String {
    normalize_path(p).to_string_lossy().to_lowercase()
}

// ─────────────────────────────── agents / sessions ──────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    Claude,
    Codex,
    Opencode,
    Pi,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub agent: Agent,
    pub session_id: Option<String>,
    /// Real working directory of the session (taken from `cwd`/`directory` fields,
    /// NEVER decoded from a folder name). Passed through `normalize_path`. May not exist.
    pub project_path: PathBuf,
    /// Unix epoch milliseconds of the last activity (last record timestamp; file mtime
    /// only as a documented fallback).
    pub last_activity_ms: i64,
    pub archived: bool,
    /// True only if the owning agent process is verified alive right now.
    pub live: bool,
}

/// One AI-agent session store (Claude Code, Codex, OpenCode).
pub trait SessionSource: Send + Sync {
    fn agent(&self) -> Agent;
    /// Read-only. A missing store is `Loaded::default()`, not an error. Must not panic.
    fn load(&self) -> Loaded<SessionRecord>;
    /// Directories this agent created as managed git worktrees (e.g. `~/.codex/worktrees/*`).
    fn managed_worktrees(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

// ───────────────────────────── scanning / artifacts ─────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeInfo {
    pub logical_bytes: u64,
    /// On-disk (cluster-rounded, hardlink-deduplicated) size when known.
    pub allocated_bytes: Option<u64>,
    pub file_count: u64,
}

impl SizeInfo {
    /// Bytes that deleting would actually free.
    pub fn effective_bytes(&self) -> u64 {
        self.allocated_bytes.unwrap_or(self.logical_bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    RustTarget,
    NodeModules,
    PythonVenv,
    PythonCache,
    Gradle,
    Maven,
    DotNet,
    CMake,
    /// A whole agent-managed git worktree directory.
    WorktreeDir,
    /// An entry of the folder catalog (the catalog says what it is through `label`/`class`).
    Catalog,
    Other,
}

/// What an artifact folder holds (C B D L P of the catalog spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Cache,
    Build,
    Dependency,
    Log,
    Personal,
}

impl Class {
    /// Level before any per-folder check: Cache, Build, Log -> Recommended; Dependency ->
    /// Optional; Personal -> Caution.
    pub fn base_level(self) -> Level {
        match self {
            Class::Cache | Class::Build | Class::Log => Level::Recommended,
            Class::Dependency => Level::Optional,
            Class::Personal => Level::Caution,
        }
    }
}

/// How strongly the app proposes removing a folder. Only `Recommended` folders are ever
/// preselected; `Caution` ones carry a warning (`Artifact::note`) the user confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Recommended,
    Optional,
    Caution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Stable id = `path_key(path)`.
    pub id: String,
    pub path: PathBuf,
    pub kind: ArtifactKind,
    /// Human label, e.g. "Rust target/".
    pub label: String,
    pub size: SizeInfo,
    /// Identity check passed (CACHEDIR.TAG / .rustc_info.json, .modules.yaml, pyvenv.cfg ...).
    /// Unverified artifacts are `Caution` and never preselected.
    pub verified: bool,
    pub class: Class,
    /// Set by the engine (`class.base_level()` until then).
    pub level: Level,
    /// Warning shown in the UI (Italian); "" when none.
    pub note: String,
    /// The name can also be a source folder (the † names of the catalog): `Caution` unless git
    /// ignores it.
    pub ambiguous: bool,
    /// Set by the engine.
    pub preselected: bool,
}

/// What the user ticked: artifact ids (`Artifact::id`) and agent-worktree report ids
/// (`ProjectReport::id`, removed as a whole when possible).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub artifacts: Vec<String>,
    pub worktrees: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCandidate {
    pub root: PathBuf,
    pub name: String,
    /// e.g. ["rust", "node"]
    pub ecosystems: Vec<String>,
    pub artifacts: Vec<Artifact>,
    /// Newest mtime (epoch ms) of source files, EXCLUDING artifacts and `.git`.
    pub source_mtime_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    /// Descend into dot-directories such as `.claude/worktrees` (default true).
    pub include_dot_dirs: bool,
    pub max_depth: Option<usize>,
    /// Paths (compared with `path_key`) never entered.
    pub ignore: Vec<PathBuf>,
    /// Artifacts smaller than this are dropped.
    pub min_artifact_bytes: u64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            include_dot_dirs: true,
            max_depth: None,
            ignore: Vec::new(),
            min_artifact_bytes: 1 << 20,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IndexedDir {
    pub path: PathBuf,
    pub size: SizeInfo,
}

/// Pre-built directory index. Lets the scanner find artifact
/// directories without walking the disk.
pub trait DirIndex: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn size_of(&self, dir: &Path) -> Option<SizeInfo>;
    /// Indexed directories under any of `roots` whose last path component matches one of
    /// `names` ([`name_matches`]: a name, or a pattern with one leading or trailing `*`).
    fn find_dirs_named(&self, names: &[&str], roots: &[PathBuf]) -> Vec<IndexedDir>;
}

/// System locations never searched for projects: `$Recycle.Bin`, `System Volume Information`
/// and `AppData` anywhere (programs' data, never a project of the user, and huge); `Windows`,
/// `ProgramData` and `Program Files*` directly under a drive root (`parent_is_root`), so that a
/// Flutter `windows/` folder deeper down is NOT skipped. Shared by the scanner's walk and the
/// MFT index search, which must agree.
pub fn is_system_dir(name: &str, parent_is_root: bool) -> bool {
    let is = |s: &str| name.eq_ignore_ascii_case(s);
    if is("$recycle.bin") || is("system volume information") || is("appdata") {
        return true;
    }
    const PF: &str = "program files";
    parent_is_root
        && (is("windows")
            || is("programdata")
            || name.get(..PF.len()).is_some_and(|h| h.eq_ignore_ascii_case(PF)))
}

/// Does folder `name` match `pattern`, ASCII case-insensitively? `pattern` is a plain name, a
/// prefix (`checkpoint-*`) or a suffix (`*.egg-info`); the `*` stands for at least one
/// character. A lone `*` matches nothing.
pub fn name_matches(pattern: &str, name: &str) -> bool {
    let fixed_ok = |fixed: &str, at_start: bool| {
        name.len() > fixed.len() && {
            let cut = if at_start { fixed.len() } else { name.len() - fixed.len() };
            name.is_char_boundary(cut)
                && (if at_start { &name[..cut] } else { &name[cut..] }).eq_ignore_ascii_case(fixed)
        }
    };
    if let Some(p) = pattern.strip_suffix('*') {
        return !p.is_empty() && !p.contains('*') && fixed_ok(p, true);
    }
    if let Some(s) = pattern.strip_prefix('*') {
        return !s.is_empty() && !s.contains('*') && fixed_ok(s, false);
    }
    name.eq_ignore_ascii_case(pattern)
}

pub trait ProjectScanner: Send + Sync {
    /// `index = None` -> walk the filesystem. `Some` -> discover artifact dirs through the
    /// index, then validate markers/identity on disk. Never deletes or writes anything.
    fn scan(
        &self,
        roots: &[PathBuf],
        opts: &ScanOptions,
        index: Option<&dyn DirIndex>,
    ) -> Loaded<ProjectCandidate>;
}

// ──────────────────────────────────── git ───────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    pub is_repo: bool,
    /// Uncommitted changes (tracked modified/staged/untracked-not-ignored). None = could not tell.
    pub dirty: Option<bool>,
    /// Commits not on any remote branch. None = could not tell.
    pub unpushed_commits: Option<u32>,
    pub has_upstream: Option<bool>,
    pub branch: Option<String>,
    /// `.git` is a file pointing to a main repo (git worktree checkout).
    pub is_linked_worktree: bool,
    pub main_worktree: Option<PathBuf>,
    /// Author date of HEAD, epoch ms.
    pub last_commit_ms: Option<i64>,
}

pub trait GitProbe: Send + Sync {
    fn state(&self, root: &Path) -> GitState;
    /// True if any file under `rel` (relative to `root`) is tracked by git.
    fn is_tracked(&self, root: &Path, rel: &Path) -> bool;
    /// `Some(true)` ignored by git, `Some(false)` not ignored, `None` = not a repository or git did not answer.
    fn is_ignored(&self, root: &Path, rel: &Path) -> Option<bool>;
}

// ────────────────────────────── analysis (engine) ───────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectStatus {
    /// No activity for >= inactive_days. Its `Recommended` folders are preselected.
    Inactive,
    /// Activity within inactive_days. Selectable, not preselected.
    Recent,
    /// No session data at all; only file/git age known. Selectable; its `Recommended` folders
    /// are preselected when that age is >= inactive_days.
    NoSessions,
    /// A live agent process is using it. Not selectable.
    InUse,
    /// Cannot be removed (see `blocked_by`). Not selectable.
    Blocked,
    /// Root path not reachable (offline drive, permissions). Never treated as deleted.
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockReason {
    LiveSession,
    DirtyGit,
    UnpushedCommits,
    /// No longer emitted by the engine (a tracked folder is `Level::Caution`).
    TrackedArtifact,
    /// No longer emitted by the engine (an unverified folder is `Level::Caution`).
    UnverifiedArtifact,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivitySource {
    Claude,
    Codex,
    Opencode,
    Pi,
    Git,
    Files,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReport {
    /// Stable id = `path_key(root)`.
    pub id: String,
    pub root: PathBuf,
    pub name: String,
    pub ecosystems: Vec<String>,
    pub agents: Vec<Agent>,
    pub last_activity_ms: Option<i64>,
    pub last_activity_source: Option<ActivitySource>,
    pub inactive_days: Option<u32>,
    pub status: ProjectStatus,
    pub blocked_by: Vec<BlockReason>,
    /// Plain-language explanation shown in the UI (Italian), e.g. "Inattivo da 94 giorni".
    pub reason: String,
    pub artifacts: Vec<Artifact>,
    pub reclaimable_bytes: u64,
    pub preselected: bool,
    pub is_agent_worktree: bool,
    pub git: GitState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    pub roots: Vec<PathBuf>,
    /// Folders that hold many projects (the home, the folders the user asked to walk in full).
    /// A session started in one of them, or above one, never credits the projects below it.
    /// The scan roots themselves are not used for this: a root found from a session is often
    /// a workspace whose session belongs to the projects below it.
    #[serde(default)]
    pub container_roots: Vec<PathBuf>,
    pub inactive_days: u32,
    pub scan: ScanOptions,
    /// Reference "now" in epoch ms (injected for deterministic tests).
    pub now_ms: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub reports: Vec<ProjectReport>,
    pub warnings: Vec<String>,
}

// ───────────────────────────── plan / execute ───────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoveMode {
    /// Windows Recycle Bin. NOTE: does not free disk space until the bin is emptied.
    Recycle,
    Permanent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum PlanAction {
    RemoveDir,
    /// `git -C <repo> worktree remove` then prune, instead of a raw delete.
    GitWorktreeRemove { repo: PathBuf },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub action: PlanAction,
    pub expected_bytes: u64,
    pub project_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanPlan {
    pub id: String,
    pub mode: RemoveMode,
    pub items: Vec<PlanItem>,
    pub expected_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum ItemOutcome {
    /// `bytes_freed` is measured, not assumed.
    Removed { bytes_freed: u64 },
    Skipped { reason: String },
    Failed { error: String, locked_by: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub path: PathBuf,
    pub outcome: ItemOutcome,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub items: Vec<ItemResult>,
    /// Sum of measured `Removed.bytes_freed`.
    pub bytes_freed: u64,
    pub dry_run: bool,
}

pub trait Remover: Send + Sync {
    /// `dry_run = true` runs every check and reports what would happen without touching disk.
    fn execute(&self, plan: &CleanPlan, dry_run: bool) -> CleanResult;
}

// ───────────────────────────── app-level (UI) types ─────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Find the projects from the working directories of the agents' sessions (default on).
    #[serde(default = "yes")]
    pub session_projects: bool,
    /// Extra folders, walked in full only when `scan_roots` is on (projects never opened with
    /// an agent).
    pub roots: Vec<PathBuf>,
    /// Walk every folder in `roots` too. Default off: a settings file written before this field
    /// existed keeps its folder list, but only the session projects are scanned.
    #[serde(default)]
    pub scan_roots: bool,
    pub inactive_days: u32,
    pub remove_mode: RemoveMode,
    /// Look for projects on every fixed drive (MFT for NTFS, walker for the others), not only
    /// in the folders above. Default on; `--only-root` turns it off.
    #[serde(default = "yes")]
    pub all_drives: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub reports: Vec<ProjectReport>,
    pub warnings: Vec<String>,
    /// Name of the scan backend that produced the result
    pub backend: String,
    pub elapsed_ms: u64,
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_file_keeps_roots_but_scans_sessions_only() {
        let s: Settings = serde_json::from_str(
            r#"{"roots":["D:\\Projects"],"inactiveDays":14,"removeMode":"permanent"}"#,
        )
        .unwrap();
        assert!(s.session_projects);
        assert!(!s.scan_roots);
        assert!(s.all_drives, "an old settings file searches every fixed drive");
        assert_eq!(s.roots, vec![PathBuf::from(r"D:\Projects")]);
    }

    #[test]
    fn system_dirs() {
        for n in ["Program Files", "Program Files (x86)", "PROGRAMDATA", "Windows", "AppData", "$Recycle.Bin", "System Volume Information"] {
            assert!(is_system_dir(n, true), "{n} at the drive root");
        }
        for n in ["Program Files", "Program Files (x86)", "ProgramData", "Windows", "windows"] {
            assert!(!is_system_dir(n, false), "{n} below the drive root");
        }
        assert!(is_system_dir("appdata", false), "AppData at any level");
        assert!(is_system_dir("$RECYCLE.BIN", false));
        assert!(!is_system_dir("Programs", true));
        assert!(!is_system_dir("Program", true));
        assert!(!is_system_dir("Projects", true));
    }

    #[test]
    fn name_patterns() {
        assert!(name_matches("target", "TARGET"));
        assert!(!name_matches("target", "targets"));
        assert!(name_matches("checkpoint-*", "Checkpoint-500"));
        assert!(!name_matches("checkpoint-*", "checkpoint-"));
        assert!(!name_matches("checkpoint-*", "checkpointless"));
        assert!(name_matches("*.egg-info", "pkg.EGG-INFO"));
        assert!(!name_matches("*.egg-info", ".egg-info"));
        assert!(!name_matches("*", "anything"));
        assert!(!name_matches("*a*", "bab"));
        assert!(!name_matches("*.egg-info", "è.egg-inf"));
    }

    #[test]
    fn old_devclean_settings_with_wiztree_fields_still_load() {
        let json = r#"{"sessionProjects":true,"roots":["D:\\Projects"],"scanRoots":false,
                       "inactiveDays":30,"useWiztree":true,"wiztreePath":"C:\\x\\WizTree64.exe",
                       "removeMode":"permanent"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.inactive_days, 30);
        assert_eq!(s.remove_mode, RemoveMode::Permanent);
        assert_eq!(s.roots, vec![PathBuf::from(r"D:\Projects")]);
    }

    #[test]
    fn normalize_strips_verbatim_prefix_and_trailing_sep() {
        assert_eq!(normalize_path(Path::new(r"\\?\C:\Users\alfio\")), PathBuf::from(r"C:\Users\alfio"));
        assert_eq!(normalize_path(Path::new(r"\\?\UNC\srv\share\x")), PathBuf::from(r"\\srv\share\x"));
        assert_eq!(normalize_path(Path::new(r"C:\")), PathBuf::from(r"C:\"));
    }

    #[test]
    fn key_is_case_insensitive_and_prefix_agnostic() {
        assert_eq!(path_key(Path::new(r"\\?\C:\Users\Alfio")), path_key(Path::new(r"c:\users\alfio\")));
    }

    #[test]
    fn effective_bytes_prefers_allocated() {
        let s = SizeInfo { logical_bytes: 10, allocated_bytes: Some(4096), file_count: 1 };
        assert_eq!(s.effective_bytes(), 4096);
        assert_eq!(SizeInfo { logical_bytes: 10, ..Default::default() }.effective_bytes(), 10);
    }

    #[test]
    fn json_shape_is_camel_and_kebab() {
        let a = ItemOutcome::Removed { bytes_freed: 5 };
        assert_eq!(serde_json::to_string(&a).unwrap(), r#"{"type":"removed","bytesFreed":5}"#);
        assert_eq!(serde_json::to_string(&ProjectStatus::InUse).unwrap(), r#""in-use""#);
        assert_eq!(serde_json::to_string(&Level::Recommended).unwrap(), r#""recommended""#);
        assert_eq!(serde_json::to_string(&Class::Dependency).unwrap(), r#""dependency""#);
    }

    #[test]
    fn class_base_levels() {
        for c in [Class::Cache, Class::Build, Class::Log] {
            assert_eq!(c.base_level(), Level::Recommended, "{c:?}");
        }
        assert_eq!(Class::Dependency.base_level(), Level::Optional);
        assert_eq!(Class::Personal.base_level(), Level::Caution);
    }
}
