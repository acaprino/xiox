//! Session source backed by the `mukade` executable (<https://github.com/acaprino/mukade>, MIT),
//! used as an already-compiled program, never as source: `mukade list <harness>` prints one JSON
//! row per session and this crate reads them. One reader per harness (Claude Code, Codex CLI,
//! OpenCode, Pi) parses the harness's own store. Only the listings are used (`cwd`, last update,
//! archived, live): no session is ever read in full and nothing is written to the stores. What
//! each listing covers (mukade `docs/usage.md`, "The list output"):
//!
//! * Claude Code: the `projects/**/*.jsonl` transcripts, the sessions `history.jsonl` still names
//!   after their transcript was deleted, and the `sessions/<pid>.json` registrations whose process
//!   is verified running (`live`). mukade keeps a cache of what each file said, keyed by the
//!   file's path, size and mtime, under the folder given to [`MukadeSessions::with_cache_dir`]
//!   (passed as `MUKADE_DATA_DIR`).
//! * Codex: the rollouts under `sessions/` plus every `threads` row of the newest
//!   `state_<n>.sqlite` (the catalogue outlives the rollouts), subagent threads included.
//! * OpenCode: the selected `opencode.db` and the other `opencode*.db` beside it, `session_v2`
//!   and the legacy `session` table.
//! * Pi: `sessions/**/*.jsonl`; no update time is recorded, the file's mtime stands in.
//!
//! Which `mukade.exe` runs: `XIOX_MUKADE_EXE`, else `mukade.exe` beside the running executable,
//! else the prebuilt copy kept in this repository (`vendor/mukade/mukade.exe`).
//!
//! Agent-managed git worktrees (Codex `~/.codex/worktrees`, OpenCode's `worktree` table and
//! `project.sandboxes`) are not a mukade concern: they are discovered here, read-only.

mod bounded;
mod calls;
mod descriptors;
/// A synthetic Claude Code store, for the tests of this crate and, with the `test-support`
/// feature, of the crates that read it through Mukade.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use bounded::{BoundedError, Limits};
pub use descriptors::{parse_time_ms, Descriptor};
pub use calls::{cut_input, parse_stream, strip_verbatim, was_cut, CallReader, DumpError, Outcome, ToolCall, CUT_MARK, MAX_INPUT_CHARS};

use xiox_model::*;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Where each harness keeps its sessions (a folder, or OpenCode's database file). `None`: not
/// configured, nothing to list.
#[derive(Clone, Debug, Default)]
pub struct StoreRoots {
    pub claude_config_dir: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub opencode_db: Option<PathBuf>,
    pub pi_agent_dir: Option<PathBuf>,
}

/// One row of `mukade list` (keys `id`, `title`, `cwd`, `timestamp`, `location`, `archived`,
/// `unreadable`, `live`, and `parent`/`kind` for a session that has a parent).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, rename = "timestamp")]
    pub updated: Option<String>,
    /// Whether the source marked this session archived.
    #[serde(default)]
    pub archived: bool,
    /// The native file (or store record) the session lives in.
    #[serde(default, rename = "location")]
    pub path: String,
    /// The native parent link, when the harness records one (rows listed with `--children`).
    #[serde(default)]
    pub parent: Option<String>,
    /// `spawn` or `fork`: set whenever `parent` is.
    #[serde(default, rename = "kind")]
    pub parent_kind: Option<String>,
    #[serde(default)]
    pub unreadable: Option<String>,
    /// `Some(true)` only when the harness's process is verified running; `None` is "not known".
    #[serde(default)]
    pub live: Option<bool>,
}

pub struct MukadeSessions {
    agent: Agent,
    roots: StoreRoots,
    /// The native `opencode.exe` that `mukade dump` needs to read an OpenCode session.
    opencode_exe: Option<PathBuf>,
    /// mukade's data folder (its Claude Code listing cache). `None`: a folder under the system
    /// temp directory.
    cache_dir: Option<PathBuf>,
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn env_dir(var: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or(default)
}

const EXE_NAME: &str = if cfg!(windows) { "mukade.exe" } else { "mukade" };

/// The mukade executable: `XIOX_MUKADE_EXE`, else beside the running executable, else the
/// prebuilt copy in this repository. `None` when none of them exists.
pub fn mukade_exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("XIOX_MUKADE_EXE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(EXE_NAME)));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/mukade").join(EXE_NAME);
    beside.into_iter().chain([repo]).find(|p| p.is_file())
}

impl MukadeSessions {
    /// Standard store locations (environment variables of each harness first, then defaults).
    pub fn from_env(agent: Agent) -> Self {
        let h = home();
        let mut roots = StoreRoots::default();
        match agent {
            Agent::Claude => {
                roots.claude_config_dir = Some(env_dir("CLAUDE_CONFIG_DIR", h.join(".claude")))
            }
            Agent::Codex => roots.codex_home = Some(env_dir("CODEX_HOME", h.join(".codex"))),
            Agent::Opencode => {
                roots.opencode_db = Some(env_dir(
                    "OPENCODE_DB",
                    h.join(".local").join("share").join("opencode").join("opencode.db"),
                ))
            }
            Agent::Pi => {
                roots.pi_agent_dir = Some(env_dir("PI_CODING_AGENT_DIR", h.join(".pi").join("agent")))
            }
        }
        let exe = std::env::var_os("XIOX_OPENCODE_EXE").filter(|v| !v.is_empty()).map(PathBuf::from);
        Self::with_roots(agent, roots).with_opencode_exe(exe)
    }

    pub fn with_roots(agent: Agent, roots: StoreRoots) -> Self {
        Self { agent, roots, opencode_exe: None, cache_dir: None }
    }

    /// Where mukade keeps its data (`MUKADE_DATA_DIR`): the Claude Code listing cache (what each
    /// session file said, keyed by the file's path, size and mtime, so only the files that
    /// changed are read next time) and the intent prompts it installs on first run. Never beside
    /// its executable.
    pub fn with_cache_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.cache_dir = dir;
        self
    }

    /// The native `opencode.exe` (`XIOX_OPENCODE_EXE` in [`Self::from_env`]).
    pub fn with_opencode_exe(mut self, exe: Option<PathBuf>) -> Self {
        self.opencode_exe = exe;
        self
    }

    pub(crate) fn harness(&self) -> &'static str {
        match self.agent {
            Agent::Claude => "claude_code",
            Agent::Codex => "codex",
            Agent::Opencode => "opencode",
            Agent::Pi => "pi",
        }
    }

    /// The variable mukade reads this agent's store from.
    fn store_env(&self) -> &'static str {
        match self.agent {
            Agent::Claude => "CLAUDE_CONFIG_DIR",
            Agent::Codex => "CODEX_HOME",
            Agent::Opencode => "OPENCODE_DB",
            Agent::Pi => "PI_CODING_AGENT_DIR",
        }
    }

    /// The store as configured (a folder, or OpenCode's database file). An agent that is not
    /// installed has none on disk.
    pub(crate) fn store_root(&self) -> Option<&Path> {
        match self.agent {
            Agent::Claude => self.roots.claude_config_dir.as_deref(),
            Agent::Codex => self.roots.codex_home.as_deref(),
            Agent::Opencode => self.roots.opencode_db.as_deref(),
            Agent::Pi => self.roots.pi_agent_dir.as_deref(),
        }
    }

    /// A `mukade` command with its environment ready: the data and intent folders (both inside
    /// ours: without `MUKADE_INTENT_DIR` mukade writes its default intent prompts beside its own
    /// executable), the harness's store variable set to `store`, no stdin and no console window.
    fn mukade_command(&self, store: &Path) -> anyhow::Result<Command> {
        let exe = mukade_exe().ok_or_else(|| {
            anyhow::anyhow!("{EXE_NAME} not found (put it beside the executable or set XIOX_MUKADE_EXE)")
        })?;
        let data = self
            .cache_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("xiox-mukade"));
        let data = std::path::absolute(&data).unwrap_or(data);
        let mut cmd = Command::new(exe);
        cmd.env(self.store_env(), store)
            .env("MUKADE_DATA_DIR", &data)
            .env("MUKADE_INTENT_DIR", data.join("intent-prompts"))
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        Ok(cmd)
    }

    /// `mukade list <harness>`: the root sessions, or with `children` every session the store
    /// holds (forks and subagent sessions included).
    pub(crate) fn run_list(&self, children: bool) -> anyhow::Result<Vec<SessionInfo>> {
        let store = self.store_root().ok_or_else(|| anyhow::anyhow!("no store configured"))?;
        let mut cmd = self.mukade_command(store)?;
        cmd.arg("list").arg(self.harness());
        if children {
            cmd.arg("--children");
        }
        let out = cmd
            .output()
            .map_err(|e| anyhow::anyhow!("{}: {e}", cmd.get_program().to_string_lossy()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("mukade list {}: {}", self.harness(), err.trim());
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str::<SessionInfo>(l).map_err(|e| {
                    anyhow::anyhow!("mukade list {}: unreadable row: {e}", self.harness())
                })
            })
            .collect()
    }

    fn list(&self) -> anyhow::Result<Vec<SessionInfo>> {
        match self.agent {
            // Claude Code: the root sessions only (subagent transcripts left out).
            Agent::Claude => self.run_list(false),
            _ => self.run_list(true),
        }
    }
}

pub(crate) fn file_mtime_ms(path: &str) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = m.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// Aggregate sessions to one record per normalized project path (newest activity wins).
pub fn aggregate(
    agent: Agent,
    infos: Vec<SessionInfo>,
    warnings: &mut Vec<String>,
) -> Vec<SessionRecord> {
    let mut by: HashMap<String, SessionRecord> = HashMap::new();
    let mut no_cwd = 0usize;
    let mut no_time = 0usize;
    for i in infos {
        let Some(cwd) = i.cwd.as_deref().filter(|c| !c.trim().is_empty()) else {
            no_cwd += 1;
            continue;
        };
        // Pi records no update time in its listing: fall back to the session file's mtime.
        let ms = i.updated.as_deref().and_then(descriptors::parse_time_ms).or_else(|| file_mtime_ms(&i.path));
        let Some(ms) = ms else {
            no_time += 1;
            continue;
        };
        let path = normalize_path(&PathBuf::from(cwd));
        let key = path_key(&path);
        let e = by.entry(key).or_insert_with(|| SessionRecord {
            agent,
            session_id: None,
            project_path: path,
            last_activity_ms: i64::MIN,
            archived: true,
            live: false,
        });
        e.archived &= i.archived;
        // `Some(true)` only when the harness's process is verified running (Claude Code's
        // `sessions/<pid>.json`); `None` is "not known", never "not running".
        e.live |= i.live == Some(true);
        if ms > e.last_activity_ms {
            e.last_activity_ms = ms;
            e.session_id = Some(i.id);
        }
    }
    if no_cwd > 0 {
        warnings.push(format!("{agent:?}: {no_cwd} sessions without a working folder ignored"));
    }
    if no_time > 0 {
        warnings.push(format!("{agent:?}: {no_time} sessions without a date ignored"));
    }
    by.into_values().collect()
}

impl SessionSource for MukadeSessions {
    fn agent(&self) -> Agent {
        self.agent
    }

    fn load(&self) -> Loaded<SessionRecord> {
        let mut out = Loaded::default();
        // A missing store is normal (agent not installed): stay quiet.
        if self.store_root().is_none_or(|p| !p.exists()) {
            return out;
        }
        match self.list() {
            Ok(infos) => out.items = aggregate(self.agent, infos, &mut out.warnings),
            Err(e) => out.warnings.push(format!("{:?}: {e:#}", self.agent)),
        }
        out
    }

    fn managed_worktrees(&self) -> Vec<PathBuf> {
        match self.agent {
            Agent::Codex => self.roots.codex_home.as_deref().map(codex_worktrees).unwrap_or_default(),
            Agent::Opencode => {
                self.roots.opencode_db.as_deref().map(opencode_worktrees).unwrap_or_default()
            }
            Agent::Claude => {
                if self.store_root().is_none_or(|p| !p.exists()) {
                    return Vec::new();
                }
                let cwds: Vec<PathBuf> = self
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|i| i.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()))
                    .map(PathBuf::from)
                    .collect();
                claude_worktrees(&cwds)
            }
            Agent::Pi => Vec::new(),
        }
    }
}

// ───────────────────────────── managed worktrees ────────────────────────────

/// Checkout roots under `<codex home>/worktrees`: `worktrees/<name>/<repo>` containing `.git`
/// (also `worktrees/<name>` itself when it is the checkout).
pub fn codex_worktrees(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(top) = std::fs::read_dir(home.join("worktrees")) else { return found };
    for name_dir in top.flatten() {
        let name_path = name_dir.path();
        if !name_path.is_dir() {
            continue;
        }
        if name_path.join(".git").exists() {
            found.push(normalize_path(&name_path));
            continue;
        }
        let Ok(inner) = std::fs::read_dir(&name_path) else { continue };
        for repo in inner.flatten() {
            let p = repo.path();
            if p.is_dir() && p.join(".git").exists() {
                found.push(normalize_path(&p));
            }
        }
    }
    dedupe_sorted(found)
}

/// Linked checkouts known to OpenCode: `worktree.directory` and `project.sandboxes` of the
/// selected database and of the other `opencode*.db` beside it, minus every project's main
/// worktree (the `worktree` table lists those too) and anything whose `.git` is not a file.
pub fn opencode_worktrees(selected: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut mains: HashSet<String> = HashSet::new();
    let clean = |s: Option<&str>| {
        s.map(str::trim).filter(|s| !s.is_empty()).map(|s| normalize_path(Path::new(s)))
    };
    for db in opencode_databases(selected) {
        let Ok(conn) = open_read_only(&db) else { continue };
        for row in text_columns(&conn, "project", &["worktree", "sandboxes"]) {
            mains.extend(clean(row[0].as_deref()).map(|p| path_key(&p)));
            let sandboxes = row[1]
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok());
            for item in sandboxes.into_iter().flatten() {
                candidates.extend(clean(item.as_str()));
            }
        }
        for row in text_columns(&conn, "worktree", &["directory"]) {
            candidates.extend(clean(row[0].as_deref()));
        }
    }
    let kept: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|p| !mains.contains(&path_key(p)) && p.join(".git").is_file())
        .collect();
    dedupe_sorted(kept)
}

/// The selected database first, then the other `opencode.db` / `opencode-<channel>.db` beside it
/// (OpenCode starts a new file for a new channel and leaves the old one behind).
fn opencode_databases(selected: &Path) -> Vec<PathBuf> {
    let mut files = vec![selected.to_path_buf()];
    if let Some(dir) = selected.parent()
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        let mut here: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_opencode_db_name(p))
            .collect();
        here.sort_by_key(|p| path_key(p));
        files.extend(here);
    }
    let mut seen = HashSet::new();
    files.retain(|p| p.is_file() && seen.insert(path_key(p)));
    files
}

fn is_opencode_db_name(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    n == "opencode.db" || (n.starts_with("opencode-") && n.ends_with(".db"))
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(2))?;
    Ok(conn)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Lower-case column names of `table`; empty if the table does not exist.
fn columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut st = conn.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
    let rows = st.query_map([], |r| r.get::<_, String>(1))?;
    let mut out = HashSet::new();
    for name in rows {
        out.insert(name?.to_lowercase());
    }
    Ok(out)
}

/// Text values of the `wanted` columns of `table` (aligned with `wanted`; `None` for a missing
/// column or a non-text value). Empty if the table or all columns are missing.
fn text_columns(conn: &Connection, table: &str, wanted: &[&str]) -> Vec<Vec<Option<String>>> {
    let Ok(cols) = columns(conn, table) else { return Vec::new() };
    let present: Vec<&str> =
        wanted.iter().copied().filter(|c| cols.contains(&c.to_lowercase())).collect();
    if present.is_empty() {
        return Vec::new();
    }
    let sel: Vec<String> = present.iter().map(|c| quote(c)).collect();
    let sql = format!("SELECT {} FROM {}", sel.join(", "), quote(table));
    let Ok(mut st) = conn.prepare(&sql) else { return Vec::new() };
    let Ok(mut rows) = st.query([]) else { return Vec::new() };
    let mut out = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        out.push(
            wanted
                .iter()
                .map(|c| {
                    let idx = present.iter().position(|p| p == c)?;
                    match row.get_ref(idx).ok()? {
                        rusqlite::types::ValueRef::Text(b) => {
                            Some(String::from_utf8_lossy(b).into_owned())
                        }
                        _ => None,
                    }
                })
                .collect(),
        );
    }
    out
}

fn dedupe_sorted(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out: Vec<PathBuf> = paths.into_iter().filter(|p| seen.insert(path_key(p))).collect();
    out.sort_by_key(|p| path_key(p));
    out
}

// ───────────────────────────── archived sessions ─────────────────────────────

/// An archived root session, with what removing it would take away. Nothing here deletes: the
/// caller removes `file` (never anything else) or, for OpenCode, goes through the CLI.
#[derive(Clone, Debug)]
pub struct ArchivedSession {
    pub agent: Agent,
    pub id: String,
    pub title: Option<String>,
    /// Working folder, normalized (no `\\?\` prefix).
    pub cwd: Option<PathBuf>,
    pub updated_ms: Option<i64>,
    /// The session's own transcript file, the only thing that may be removed for it.
    pub file: Option<PathBuf>,
    /// Size of `file`; for Codex, of every page of the thread ([`codex_thread_files`]); 0 when
    /// there is no file.
    pub bytes: u64,
    /// Why it cannot be removed (Italian); `None` means removable.
    pub blocked: Option<String>,
}

impl MukadeSessions {
    /// Archived root sessions of this agent. A missing store lists nothing, quietly.
    pub fn archived(&self) -> Loaded<ArchivedSession> {
        let mut out = Loaded::default();
        let Some(store) = self.store_root().filter(|p| p.exists()) else { return out };
        let listed = match self.agent {
            // One cached listing; the roots are the sessions with no parent link.
            // NOTE: Pi records no parent link, so its forks are all roots here.
            Agent::Claude => self.list().map(|all| {
                let roots = all.iter().filter(|i| i.parent.is_none()).cloned().collect();
                (roots, all)
            }),
            // NOTE: Codex `threads` rows may be paginated by the reader; a long catalogue could
            // be truncated and hide children from `all`.
            _ => self.run_list(false).and_then(|roots| Ok((roots, self.run_list(true)?))),
        };
        match listed {
            Ok((roots, all)) => out.items = select_archived(self.agent, store, roots, &all),
            Err(e) => out.warnings.push(format!("{:?}: {e:#}", self.agent)),
        }
        out
    }
}

/// Is `p` a file that can be a session's own transcript: an existing `.jsonl`, never Claude
/// Code's `history.jsonl` (shared by every session).
fn is_transcript_file(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.ends_with(".jsonl") && name != "history.jsonl" && p.is_file()
}

/// Is `p` inside `root`: no `..` component, and `root` is a prefix of `p` once both are
/// normalized (verbatim `\\?\` prefix, separators, case). Does not touch the filesystem.
fn within(root: &Path, p: &Path) -> bool {
    !p.components().any(|c| matches!(c, Component::ParentDir))
        && PathBuf::from(path_key(p)).starts_with(PathBuf::from(path_key(root)))
}

/// Where a Codex home keeps rollouts: live threads, then those `codex archive` moved.
const CODEX_ROLLOUT_DIRS: [&str; 2] = ["sessions", "archived_sessions"];

/// The `.jsonl` files in `dir` (and below it when `deep`); links are not followed.
fn jsonl_files(dir: &Path, deep: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let p = e.path();
        if t.is_dir() {
            if deep {
                jsonl_files(&p, true, out);
            }
        } else if t.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jsonl")) {
            out.push(p);
        }
    }
}

/// Is `name` a rollout of the Codex thread `id`: `rollout-<ts>-<id>.jsonl`, or a page of a
/// paginated thread, `rollout-<ts>-<id>_<page>.jsonl` (as mukade's `rollout_id` reads them).
fn is_codex_rollout_of(name: &str, id: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let Some(stem) = name.strip_suffix(".jsonl") else { return false };
    let thread = stem.rsplit_once('_').map_or(stem, |(t, _page)| t);
    !id.is_empty()
        && thread.starts_with("rollout-")
        && thread.ends_with(&format!("-{}", id.to_ascii_lowercase()))
}

/// The Codex home a rollout lies in: the folder holding the nearest `sessions` or
/// `archived_sessions` folder above the file.
fn codex_home_of(file: &Path) -> Option<&Path> {
    file.ancestors()
        .skip(1)
        .find(|a| {
            a.file_name()
                .is_some_and(|n| CODEX_ROLLOUT_DIRS.iter().any(|d| n.eq_ignore_ascii_case(d)))
        })
        .and_then(Path::parent)
}

/// `file` plus the other rollouts among `candidates` that belong to the thread `id`, sorted,
/// each once.
fn codex_pages_in(candidates: &[PathBuf], file: &Path, id: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = candidates
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| is_codex_rollout_of(&n.to_string_lossy(), id)))
        .cloned()
        .collect();
    if file.is_file() {
        out.push(file.to_path_buf());
    }
    out.sort_by_key(|p| path_key(p));
    out.dedup_by(|a, b| path_key(a) == path_key(b));
    out
}

/// Every file of the Codex thread `id` whose rollout is `file`. Codex 0.165 may keep one thread
/// over several files (`history_mode: "paginated"`): the latest page is the thread's rollout,
/// the earlier ones (`rollout-<ts>-<id>.jsonl`, `rollout-<ts>-<id>_<page>.jsonl`) can sit in
/// other day folders and in `archived_sessions/`, and `codex delete` removes them all. Looks
/// under both rollout folders of the home `file` lies in, else beside `file`. `file` itself is
/// always in the list when it exists.
pub fn codex_thread_files(file: &Path, id: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    match codex_home_of(file) {
        Some(home) => {
            for d in CODEX_ROLLOUT_DIRS {
                jsonl_files(&home.join(d), true, &mut candidates);
            }
        }
        None => {
            if let Some(dir) = file.parent() {
                jsonl_files(dir, false, &mut candidates);
            }
        }
    }
    codex_pages_in(&candidates, file, id)
}

/// Every session below `root` (children, their children, ...) through the parent links in
/// `children` (parent id -> sessions naming it). Each session is visited once, so a cycle in the
/// links ends the walk instead of looping.
fn descendants<'a>(root: &str, children: &HashMap<&str, Vec<&'a SessionInfo>>) -> Vec<&'a SessionInfo> {
    let mut seen: HashSet<String> = HashSet::from([root.to_string()]);
    let mut todo = vec![root.to_string()];
    let mut out = Vec::new();
    while let Some(id) = todo.pop() {
        for c in children.get(id.as_str()).into_iter().flatten() {
            if seen.insert(c.id.clone()) {
                out.push(*c);
                todo.push(c.id.clone());
            }
        }
    }
    out
}

/// The archived roots of `roots`, each with the file that is its own and the reason it cannot be
/// removed, if any. `all` is every session the store lists (children included).
pub fn select_archived(
    agent: Agent,
    store_root: &Path,
    roots: Vec<SessionInfo>,
    all: &[SessionInfo],
) -> Vec<ArchivedSession> {
    let store_key = path_key(store_root);
    // Codex: every rollout of the store, read once, to find the pages of paginated threads.
    let mut codex_rollouts = Vec::new();
    if agent == Agent::Codex {
        for d in CODEX_ROLLOUT_DIRS {
            jsonl_files(&store_root.join(d), true, &mut codex_rollouts);
        }
    }
    let mut children: HashMap<&str, Vec<&SessionInfo>> = HashMap::new();
    for c in all {
        if let Some(p) = c.parent.as_deref() {
            children.entry(p).or_default().push(c);
        }
    }
    roots
        .into_iter()
        .filter(|i| i.archived)
        .map(|i| {
            let path = PathBuf::from(&i.path);
            // The file is later deleted: it must be a transcript lying under the folders where
            // the agent keeps them, whatever the listing says (`history.jsonl` and
            // `sessions/<pid>.json` are never one).
            let own = |dirs: &[&str]| {
                is_transcript_file(&path)
                    && dirs.iter().any(|d| within(&store_root.join(d), &path))
            };
            let file = match agent {
                Agent::Claude => own(&["projects"]),
                Agent::Codex => own(&["sessions", "archived_sessions"]),
                Agent::Pi => own(&["sessions"]),
                // Deleted through the CLI, never as a file.
                Agent::Opencode => false,
            }
            .then(|| path.clone());
            let pages = match (&file, agent) {
                (Some(f), Agent::Codex) => codex_pages_in(&codex_rollouts, f, &i.id),
                (Some(f), _) => vec![f.clone()],
                (None, _) => Vec::new(),
            };
            let bytes = pages
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok())
                .fold(0u64, |acc, m| acc.saturating_add(m.len()));
            let updated_ms = i.updated.as_deref().and_then(descriptors::parse_time_ms).or_else(|| {
                file.as_ref().and_then(|f| file_mtime_ms(&f.to_string_lossy()))
            });
            // Every descendant counts, not only the children: the harness deletes the whole
            // subtree with the root (`opencode session delete`).
            let has_open_child = descendants(&i.id, &children)
                .iter()
                .any(|c| !c.archived || c.live == Some(true));
            let blocked = if i.live == Some(true) {
                Some("session in use".to_string())
            } else if has_open_child {
                Some("has a child session that is not archived or is in use".to_string())
            } else if agent == Agent::Opencode && path_key(&path) != store_key {
                Some("the session is in another OpenCode database".to_string())
            } else if agent == Agent::Codex && file.is_none() && is_transcript_file(&path) {
                // A real rollout outside `sessions/` and `archived_sessions/`: `codex delete`
                // would remove it with no copy in the Recycle Bin. A row whose rollout is gone
                // (or that names the database) has no file and stays deletable.
                Some("session file outside the store".to_string())
            } else if matches!(agent, Agent::Claude | Agent::Pi) && file.is_none() {
                // Claude Code and Pi sessions ARE their transcript: without it nothing may be
                // removed. Codex and OpenCode go through their CLI, which deletes a session
                // without a file too (a Codex `threads` row whose rollout is gone,
                // `codex delete --force <id>`; OpenCode never has one).
                Some("no session file".to_string())
            } else {
                None
            };
            ArchivedSession {
                agent,
                id: i.id,
                title: i.title,
                cwd: i
                    .cwd
                    .as_deref()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(|c| normalize_path(Path::new(c))),
                updated_ms,
                file,
                bytes,
                blocked,
            }
        })
        .collect()
}

/// Claude Code worktrees (`<repo>/.claude/worktrees/<name>`) the sessions' working folders point
/// into or sit above: the folder itself or a subfolder of a worktree gives that worktree; a
/// project folder gives each `.claude/worktrees/<name>` that holds a `.git`.
///
/// The first case (a cwd inside `.claude/worktrees/<name>`) does NOT check that the folder exists
/// or is a worktree: the caller must verify existence and `.git` before planning any deletion.
pub fn claude_worktrees(cwds: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for cwd in cwds {
        let cwd = normalize_path(cwd);
        let parts: Vec<_> = cwd.components().collect();
        let inside = parts.windows(3).position(|w| {
            let n = |c: &std::path::Component| c.as_os_str().to_string_lossy().to_lowercase();
            n(&w[0]) == ".claude" && n(&w[1]) == "worktrees"
        });
        if let Some(at) = inside {
            found.push(parts[..at + 3].iter().collect::<PathBuf>());
            continue;
        }
        let Ok(rd) = std::fs::read_dir(cwd.join(".claude").join("worktrees")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join(".git").exists() {
                found.push(normalize_path(&p));
            }
        }
    }
    dedupe_sorted(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, cwd: Option<&str>, updated: Option<&str>, archived: bool) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            cwd: cwd.map(Into::into),
            updated: updated.map(Into::into),
            archived,
            ..Default::default()
        }
    }

    fn find<'a>(v: &'a [SessionRecord], key: &str) -> &'a SessionRecord {
        v.iter().find(|r| path_key(&r.project_path) == key).expect(key)
    }

    #[test]
    fn aggregates_per_path_newest_wins_and_skips_incomplete() {
        let mut w = Vec::new();
        let v = aggregate(
            Agent::Pi,
            vec![
                info("a", Some(r"C:\dev\x"), Some("2026-11-01T10:00:00Z"), true),
                info("b", Some(r"c:\DEV\x\"), Some("2026-11-03T10:00:00Z"), false),
                info("c", None, Some("2026-11-03T10:00:00Z"), false),
                info("d", Some(r"C:\dev\y"), None, false),
                info("e", Some(r"C:\dev\z"), Some("1762000000"), false),
            ],
            &mut w,
        );
        assert_eq!(v.len(), 2);
        let x = find(&v, r"c:\dev\x");
        assert_eq!(x.session_id.as_deref(), Some("b"));
        assert!(!x.archived);
        let z = find(&v, r"c:\dev\z");
        assert_eq!(z.last_activity_ms, 1_762_000_000_000);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn live_is_only_a_verified_running_process() {
        let mut a = info("a", Some(r"C:\dev\x"), Some("2026-11-01T10:00:00Z"), false);
        a.live = Some(true);
        let mut b = info("b", Some(r"C:\dev\x"), Some("2026-11-02T10:00:00Z"), false);
        b.live = Some(false);
        let c = info("c", Some(r"C:\dev\y"), Some("2026-11-02T10:00:00Z"), false);
        assert_eq!(c.live, None, "not recorded is not false");
        let v = aggregate(Agent::Claude, vec![a, b, c], &mut Vec::new());
        assert!(find(&v, r"c:\dev\x").live, "one running session makes the project live");
        assert!(!find(&v, r"c:\dev\y").live);
    }

    #[test]
    fn parse_ms_handles_rfc3339_seconds_and_millis() {
        use descriptors::parse_time_ms as parse_ms;
        assert_eq!(parse_ms("2026-11-01T00:00:00Z"), Some(1_793_491_200_000));
        assert_eq!(parse_ms("1793491200"), Some(1_793_491_200_000));
        assert_eq!(parse_ms("1793491200000"), Some(1_793_491_200_000));
        assert_eq!(parse_ms("boh"), None);
    }

    fn roots_under(none: &Path) -> StoreRoots {
        StoreRoots {
            claude_config_dir: Some(none.join("claude")),
            codex_home: Some(none.join("codex")),
            opencode_db: Some(none.join("opencode").join("opencode.db")),
            pi_agent_dir: Some(none.join("pi")),
        }
    }

    #[test]
    fn missing_stores_are_empty_without_warnings() {
        let td = tempfile::tempdir().unwrap();
        let none = td.path().join("nothing-here");
        for a in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
            let s = MukadeSessions::with_roots(a, roots_under(&none));
            let l = s.load();
            assert!(l.items.is_empty(), "{a:?}");
            assert!(l.warnings.is_empty(), "{a:?}: {:?}", l.warnings);
            assert!(s.managed_worktrees().is_empty(), "{a:?}");
        }
    }

    #[test]
    fn empty_stores_are_empty_without_warnings() {
        let td = tempfile::tempdir().unwrap();
        let roots = roots_under(td.path());
        for dir in ["claude", "codex", "opencode", "pi"] {
            std::fs::create_dir_all(td.path().join(dir)).unwrap();
        }
        // An OpenCode database with its tables and no rows (one without the tables is schema
        // drift, and is reported).
        let c = Connection::open(roots.opencode_db.as_ref().unwrap()).unwrap();
        c.execute_batch(OC_PROJECT).unwrap();
        c.execute_batch(OC_SESSION_V2).unwrap();
        c.execute_batch(OC_WORKTREE).unwrap();
        drop(c);
        for a in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
            let l = MukadeSessions::with_roots(a, roots.clone())
                .with_cache_dir(Some(td.path().join("cache")))
                .load();
            assert!(l.items.is_empty(), "{a:?}");
            assert!(l.warnings.is_empty(), "{a:?}: {:?}", l.warnings);
        }
    }

    #[test]
    fn codex_managed_worktrees_layout() {
        let td = tempfile::tempdir().unwrap();
        let wt = td.path().join("worktrees");
        // worktrees/<name>/<repo>/.git (file) -> checkout root is <repo>
        let a = wt.join("fix-a").join("repo-a");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join(".git"), "gitdir: x").unwrap();
        std::fs::write(wt.join("fix-a").join(".codex-worktree-name"), "").unwrap();
        // marker only
        std::fs::create_dir_all(wt.join("empty")).unwrap();
        std::fs::write(wt.join("empty").join(".codex-worktree-name"), "").unwrap();
        // leftover dir without .git is not a checkout
        std::fs::create_dir_all(wt.join("left").join("repo-l").join(".cache")).unwrap();
        // flat layout: the name dir is itself the checkout
        let flat = wt.join("flat");
        std::fs::create_dir_all(flat.join(".git")).unwrap();
        // a stray file is ignored
        std::fs::write(wt.join("note.txt"), "x").unwrap();

        let roots = StoreRoots { codex_home: Some(td.path().to_path_buf()), ..Default::default() };
        let got = MukadeSessions::with_roots(Agent::Codex, roots).managed_worktrees();
        let keys: Vec<String> = got.iter().map(|p| path_key(p)).collect();
        assert_eq!(keys, vec![path_key(&a), path_key(&flat)]);
    }

    // Real shapes, copied from sqlite_master of a live OpenCode database (read-only).
    const OC_PROJECT: &str = "CREATE TABLE `project` (
        `id` text PRIMARY KEY, `worktree` text NOT NULL, `vcs` text, `name` text,
        `time_created` integer NOT NULL, `time_updated` integer NOT NULL,
        `sandboxes` text NOT NULL, `time_active` integer DEFAULT 0 NOT NULL, `time_archived` integer)";
    // OpenCode 2.2.x shape: mukade selects `parent_id` unconditionally (an older table without
    // it is reported as a warning, not listed).
    const OC_SESSION_V2: &str = "CREATE TABLE `session_v2` (
        `id` text PRIMARY KEY, `project_id` text NOT NULL, `slug` text NOT NULL,
        `directory` text NOT NULL, `title` text, `version` text NOT NULL,
        `time_created` integer NOT NULL, `time_updated` integer NOT NULL,
        `time_compacting` integer, `time_archived` integer, `parent_id` text, `fork_session_id` text)";
    const OC_WORKTREE: &str = "CREATE TABLE `worktree` (
        `project_id` text NOT NULL, `directory` text NOT NULL, `strategy` text,
        `time_created` integer NOT NULL, CONSTRAINT `worktree_pk` PRIMARY KEY(`project_id`, `directory`))";

    #[test]
    fn opencode_managed_worktrees_excludes_main_checkouts() {
        use rusqlite::params;
        let td = tempfile::tempdir().unwrap();
        let main = td.path().join("main-repo");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        let linked = td.path().join("linked-wt");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join(".git"), "gitdir: x").unwrap();
        let sandbox = td.path().join("sandbox-wt");
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::write(sandbox.join(".git"), "gitdir: y").unwrap();
        let plain = td.path().join("plain-dir-with-git-folder");
        std::fs::create_dir_all(plain.join(".git")).unwrap();
        let missing = td.path().join("gone");

        // The sandbox is known only to a channel database beside the selected one.
        let data = td.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let s = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let selected = data.join("opencode.db");
        {
            let c = Connection::open(&selected).unwrap();
            c.execute_batch(OC_PROJECT).unwrap();
            c.execute_batch(OC_WORKTREE).unwrap();
            c.execute(
                "INSERT INTO project (id, worktree, time_created, time_updated, sandboxes) VALUES ('p1', ?1, 1, 1, '[]')",
                params![s(&main)],
            )
            .unwrap();
            for d in [&main, &linked, &plain, &missing] {
                c.execute(
                    "INSERT INTO worktree (project_id, directory, time_created) VALUES ('p1', ?1, 1)",
                    params![s(d)],
                )
                .unwrap();
            }
        }
        {
            let c = Connection::open(data.join("opencode-old.db")).unwrap();
            c.execute_batch(OC_PROJECT).unwrap();
            c.execute(
                "INSERT INTO project (id, worktree, time_created, time_updated, sandboxes) VALUES ('p1', ?1, 1, 1, ?2)",
                params![s(&main), serde_json::to_string(&vec![s(&sandbox)]).unwrap()],
            )
            .unwrap();
        }
        // Not an OpenCode database name: never opened.
        std::fs::write(data.join("notes.db"), b"not sqlite").unwrap();

        let roots = StoreRoots { opencode_db: Some(selected), ..Default::default() };
        let got = MukadeSessions::with_roots(Agent::Opencode, roots).managed_worktrees();
        let mut want = vec![path_key(&linked), path_key(&sandbox)];
        want.sort();
        let keys: Vec<String> = got.iter().map(|p| path_key(p)).collect();
        assert_eq!(keys, want);
    }

    // ─────────────────────────── archived sessions ───────────────────────────

    fn ainfo(id: &str, path: &Path, archived: bool) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            title: Some(format!("titolo {id}")),
            cwd: Some(r"C:\dev\x".into()),
            updated: Some("2026-11-01T10:00:00Z".into()),
            archived,
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    fn touch(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn only_archived_roots_are_listed() {
        let td = tempfile::tempdir().unwrap();
        let f = |n: &str| {
            let p = td.path().join("sessions").join(format!("{n}.jsonl"));
            touch(&p, "x");
            p
        };
        let roots = vec![
            ainfo("a", &f("a"), false),
            ainfo("b", &f("b"), true),
            ainfo("c", &f("c"), false),
        ];
        let got = select_archived(Agent::Codex, td.path(), roots.clone(), &roots);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "b");
        assert_eq!(got[0].agent, Agent::Codex);
    }

    #[test]
    fn live_archived_session_is_blocked() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("sessions").join("a.jsonl");
        touch(&p, "x");
        let mut a = ainfo("a", &p, true);
        a.live = Some(true);
        let got = select_archived(Agent::Codex, td.path(), vec![a.clone()], &[a]);
        assert_eq!(got[0].blocked.as_deref(), Some("session in use"));
    }

    #[test]
    fn archived_parent_with_active_child_is_blocked() {
        let td = tempfile::tempdir().unwrap();
        let pp = td.path().join("sessions").join("p.jsonl");
        let pc = td.path().join("sessions").join("c.jsonl");
        touch(&pp, "x");
        touch(&pc, "x");
        let parent = ainfo("p", &pp, true);
        let mut child = ainfo("c", &pc, false);
        child.parent = Some("p".into());
        let all = vec![parent.clone(), child.clone()];
        let got = select_archived(Agent::Codex, td.path(), vec![parent.clone()], &all);
        assert!(got[0].blocked.as_deref().unwrap().contains("child session"), "{:?}", got[0].blocked);

        child.archived = true;
        let all = vec![parent.clone(), child];
        let got = select_archived(Agent::Codex, td.path(), vec![parent], &all);
        assert_eq!(got[0].blocked, None);
    }

    #[test]
    fn an_open_grandchild_blocks_the_archived_root() {
        let td = tempfile::tempdir().unwrap();
        let f = |n: &str| {
            let p = td.path().join("sessions").join(format!("{n}.jsonl"));
            touch(&p, "x");
            p
        };
        let root = ainfo("r", &f("r"), true);
        let mut child = ainfo("c", &f("c"), true);
        child.parent = Some("r".into());
        let mut grandchild = ainfo("g", &f("g"), false);
        grandchild.parent = Some("c".into());
        let all = vec![root.clone(), child.clone(), grandchild.clone()];
        let got = select_archived(Agent::Codex, td.path(), vec![root.clone()], &all);
        assert!(got[0].blocked.as_deref().unwrap().contains("child session"), "{:?}", got[0].blocked);

        // Archived but live, two levels down: still blocked.
        grandchild.archived = true;
        grandchild.live = Some(true);
        let all = vec![root.clone(), child.clone(), grandchild.clone()];
        let got = select_archived(Agent::Codex, td.path(), vec![root.clone()], &all);
        assert!(got[0].blocked.as_deref().unwrap().contains("child session"), "{:?}", got[0].blocked);

        grandchild.live = None;
        let all = vec![root.clone(), child, grandchild];
        let got = select_archived(Agent::Codex, td.path(), vec![root], &all);
        assert_eq!(got[0].blocked, None);
    }

    #[test]
    fn a_cycle_in_the_parent_links_ends_the_walk() {
        let td = tempfile::tempdir().unwrap();
        let f = |n: &str| {
            let p = td.path().join("sessions").join(format!("{n}.jsonl"));
            touch(&p, "x");
            p
        };
        let root = ainfo("r", &f("r"), true);
        let mut a = ainfo("a", &f("a"), true);
        a.parent = Some("r".into());
        let mut b = ainfo("b", &f("b"), true);
        b.parent = Some("a".into());
        // `a` and `b` name each other through `r`'s subtree: r -> a -> b -> a ...
        let mut back = ainfo("a", &f("a2"), true);
        back.parent = Some("b".into());
        let mut self_loop = ainfo("r", &f("r2"), true);
        self_loop.parent = Some("r".into());
        let all = vec![root.clone(), a, b, back, self_loop];
        let got = select_archived(Agent::Codex, td.path(), vec![root.clone()], &all);
        assert_eq!(got[0].blocked, None);

        let mut open = ainfo("o", &f("o"), false);
        open.parent = Some("b".into());
        let mut all = all;
        all.push(open);
        let got = select_archived(Agent::Codex, td.path(), vec![root], &all);
        assert!(got[0].blocked.as_deref().unwrap().contains("child session"), "{:?}", got[0].blocked);
    }

    #[test]
    fn codex_file_is_the_rollout_when_it_exists() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("sessions").join("rollout-a.jsonl");
        touch(&p, "12345");
        let a = ainfo("a", &p, true);
        let got = select_archived(Agent::Codex, td.path(), vec![a.clone()], &[a]);
        assert_eq!(got[0].file.as_deref(), Some(p.as_path()));
        assert_eq!(got[0].bytes, 5);
        assert_eq!(got[0].blocked, None);

        let gone = td.path().join("sessions").join("rollout-gone.jsonl");
        let b = ainfo("b", &gone, true);
        // Only a database row left: still deletable through the CLI.
        let got = select_archived(Agent::Codex, td.path(), vec![b.clone()], &[b]);
        assert_eq!(got[0].file, None);
        assert_eq!(got[0].bytes, 0);
        assert_eq!(got[0].blocked, None);
    }

    #[test]
    fn a_paginated_codex_thread_counts_every_page() {
        let td = tempfile::tempdir().unwrap();
        let home = td.path();
        let id = "019a0000-1111-2222-3333-444455556666";
        let day = |d: &str| home.join("sessions").join("2026").join("11").join(d);
        let first = day("09").join(format!("rollout-2026-11-09T10-00-00-{id}.jsonl"));
        let latest =
            day("10").join(format!("rollout-2026-11-10T09-00-00-{id}_019a0000-aaaa-bbbb-cccc-ddddeeeeffff.jsonl"));
        let moved = home.join("archived_sessions").join(format!("rollout-2026-11-08T08-00-00-{id}.jsonl"));
        let other = day("10").join("rollout-2026-11-10T09-00-00-019a0000-9999-2222-3333-444455556666.jsonl");
        touch(&first, "12345");
        touch(&latest, "123");
        touch(&moved, "1");
        touch(&other, "1234567");

        let mut want = vec![first.clone(), latest.clone(), moved.clone()];
        want.sort_by_key(|p| path_key(p));
        assert_eq!(codex_thread_files(&latest, id), want);

        let a = ainfo(id, &latest, true);
        let got = select_archived(Agent::Codex, home, vec![a.clone()], &[a]);
        assert_eq!(got[0].file.as_deref(), Some(latest.as_path()));
        assert_eq!(got[0].bytes, 9, "the three pages, not the other thread");

        // A single-file thread is just its rollout.
        assert_eq!(codex_thread_files(&other, "019a0000-9999-2222-3333-444455556666"), vec![other.clone()]);
        // Outside a Codex home: only the folder of the file is looked at.
        let loose = td.path().join("loose").join(format!("rollout-x-{id}.jsonl"));
        let beside = td.path().join("loose").join(format!("rollout-y-{id}_p.jsonl"));
        touch(&loose, "1");
        touch(&beside, "1");
        assert_eq!(codex_thread_files(&loose, id).len(), 2);
    }

    #[test]
    fn a_file_session_without_its_file_is_blocked_a_cli_session_is_not() {
        let td = tempfile::tempdir().unwrap();
        let gone = td.path().join("sessions").join("gone.jsonl");
        for agent in [Agent::Claude, Agent::Pi] {
            let a = ainfo("a", &gone, true);
            let got = select_archived(agent, td.path(), vec![a.clone()], &[a]);
            assert_eq!(got[0].file, None, "{agent:?}");
            assert_eq!(got[0].blocked.as_deref(), Some("no session file"), "{agent:?}");
        }
        // A Codex `threads` row whose rollout is gone: `codex delete` still removes it.
        let a = ainfo("a", &gone, true);
        let got = select_archived(Agent::Codex, td.path(), vec![a.clone()], &[a]);
        assert_eq!((got[0].file.as_deref(), got[0].bytes, got[0].blocked.as_deref()), (None, 0, None));
        let a = ainfo("a", td.path(), true);
        let got = select_archived(Agent::Opencode, td.path(), vec![a.clone()], &[a]);
        assert_eq!(got[0].file, None);
        assert_eq!(got[0].blocked, None);
    }

    #[test]
    fn claude_never_targets_history_or_pid_files() {
        let td = tempfile::tempdir().unwrap();
        let store = td.path();
        let history = store.join("history.jsonl");
        let pid = store.join("sessions").join("123.json");
        let own = store.join("projects").join("x").join("abc.jsonl");
        for p in [&history, &pid, &own] {
            touch(p, "{}");
        }
        for p in [&history, &pid] {
            let a = ainfo("a", p, true);
            let got = select_archived(Agent::Claude, store, vec![a.clone()], &[a]);
            assert_eq!(got[0].file, None, "{p:?}");
            assert_eq!(got[0].blocked.as_deref(), Some("no session file"), "{p:?}");
        }
        let a = ainfo("a", &own, true);
        let got = select_archived(Agent::Claude, store, vec![a.clone()], &[a]);
        assert_eq!(got[0].file.as_deref(), Some(own.as_path()));
        assert_eq!(got[0].blocked, None);
    }

    #[test]
    fn opencode_in_another_database_is_blocked() {
        let td = tempfile::tempdir().unwrap();
        let selected = td.path().join("opencode.db");
        let other = td.path().join("opencode-old.db");
        touch(&selected, "x");
        touch(&other, "x");
        let here = ainfo("a", &selected, true);
        let there = ainfo("b", &other, true);
        let all = vec![here.clone(), there.clone()];
        let got = select_archived(Agent::Opencode, &selected, all.clone(), &all);
        let a = got.iter().find(|s| s.id == "a").unwrap();
        let b = got.iter().find(|s| s.id == "b").unwrap();
        assert_eq!(a.file, None);
        assert_eq!(a.blocked, None);
        assert_eq!(b.file, None);
        assert!(b.blocked.as_deref().unwrap().contains("another OpenCode database"), "{:?}", b.blocked);
    }

    #[test]
    fn verbatim_prefixed_cwd_is_normalized() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("sessions").join("a.jsonl");
        touch(&p, "x");
        let mut a = ainfo("a", &p, true);
        a.cwd = Some(r"\\?\C:\Users\x\.codex\worktrees\a\daodan".into());
        let got = select_archived(Agent::Codex, td.path(), vec![a.clone()], &[a]);
        assert_eq!(got[0].cwd, Some(PathBuf::from(r"C:\Users\x\.codex\worktrees\a\daodan")));
    }

    #[test]
    fn missing_store_lists_nothing_quietly() {
        let td = tempfile::tempdir().unwrap();
        let none = td.path().join("nothing-here");
        let l = MukadeSessions::with_roots(Agent::Pi, roots_under(&none)).archived();
        assert!(l.items.is_empty());
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn claude_worktrees_from_cwds() {
        let td = tempfile::tempdir().unwrap();
        let repo = td.path().join("repo");
        let w1 = repo.join(".claude").join("worktrees").join("w1");
        let repo2 = td.path().join("repo2");
        let w2 = repo2.join(".claude").join("worktrees").join("w2");
        touch(&w2.join(".git"), "gitdir: x");
        let got = claude_worktrees(&[
            w1.join("sub"),
            w1.clone(),
            repo2.clone(),
            repo2.clone(),
            td.path().join("elsewhere"),
        ]);
        let keys: Vec<String> = got.iter().map(|p| path_key(p)).collect();
        let mut want = vec![path_key(&w1), path_key(&w2)];
        want.sort();
        assert_eq!(keys, want);
    }

    #[test]
    fn files_outside_the_agents_session_folders_are_not_targeted() {
        let td = tempfile::tempdir().unwrap();
        let store = td.path().join("store");
        let elsewhere = td.path().join("elsewhere");
        let one = |agent: Agent, p: &Path| {
            let a = ainfo("a", p, true);
            select_archived(agent, &store, vec![a.clone()], &[a]).remove(0)
        };

        // Codex: outside the store, and inside it but not under sessions/ or archived_sessions/.
        let out = elsewhere.join("rollout.jsonl");
        touch(&out, "123");
        let other = store.join("other").join("rollout.jsonl");
        touch(&other, "123");
        for p in [&out, &other] {
            let s = one(Agent::Codex, p);
            assert_eq!((s.file, s.bytes), (None, 0), "{p:?}");
            assert!(s.blocked.as_deref().unwrap_or("").contains("outside the store"), "{p:?}: {:?}", s.blocked);
        }
        // Codex rows with no rollout on disk (gone, or naming the database): still deletable.
        let db = store.join("state_5.sqlite");
        touch(&db, "x");
        for p in [store.join("sessions").join("rollout-gone.jsonl"), elsewhere.join("gone.jsonl"), db] {
            let s = one(Agent::Codex, &p);
            assert_eq!((s.file, s.bytes, s.blocked), (None, 0, None), "{p:?}");
        }
        let arch = store.join("archived_sessions").join("rollout.jsonl");
        touch(&arch, "123");
        assert_eq!(one(Agent::Codex, &arch).file.as_deref(), Some(arch.as_path()));

        // Pi: outside sessions/.
        let pi_out = store.join("auth").join("x.jsonl");
        touch(&pi_out, "1");
        let s = one(Agent::Pi, &pi_out);
        assert_eq!((s.file, s.bytes, s.blocked.as_deref()), (None, 0, Some("no session file")));
        let pi_in = store.join("sessions").join("--c--x").join("x.jsonl");
        touch(&pi_in, "1");
        assert_eq!(one(Agent::Pi, &pi_in).file.as_deref(), Some(pi_in.as_path()));

        // Claude: a `..` that climbs out of projects/ is refused even though the file exists.
        let victim = store.join("secret.jsonl");
        touch(&victim, "1");
        let sneaky = store.join("projects").join("..").join("secret.jsonl");
        let s = one(Agent::Claude, &sneaky);
        assert_eq!((s.file, s.bytes), (None, 0));
        assert_eq!(s.blocked.as_deref(), Some("no session file"));

        // Claude: a .jsonl under sessions/ (outside projects/): the projects guard alone.
        let reg = store.join("sessions").join("x.jsonl");
        touch(&reg, "1");
        let s = one(Agent::Claude, &reg);
        assert_eq!(s.file, None);
        assert_eq!(s.blocked.as_deref(), Some("no session file"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_variants_still_count_as_inside() {
        let td = tempfile::tempdir().unwrap();
        let store = td.path().join("Store");
        let real = store.join("sessions").join("Roll.jsonl");
        touch(&real, "123");
        let raw = real.to_string_lossy().into_owned();
        let variants = [
            raw.replace('\\', "/"),
            raw.to_uppercase(),
            format!(r"\\?\{raw}"),
        ];
        for v in variants {
            let a = ainfo("a", Path::new(&v), true);
            let got = select_archived(Agent::Codex, &store, vec![a.clone()], &[a]);
            assert_eq!(got[0].file.as_deref(), Some(Path::new(&v)), "{v}");
            assert_eq!(got[0].bytes, 3, "{v}");
        }
        // The store itself may be given in another spelling too.
        let store_alt = PathBuf::from(store.to_string_lossy().replace('\\', "/").to_lowercase());
        let a = ainfo("a", &real, true);
        let got = select_archived(Agent::Pi, &store_alt, vec![a.clone()], &[a]);
        assert_eq!(got[0].file.as_deref(), Some(real.as_path()));
    }

    /// Real stores on the developer machine: `cargo test -p xiox-mukade -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real agent stores"]
    fn real_stores_smoke() {
        let cache = std::env::temp_dir().join("xiox-mukade-smoke-cache");
        for a in [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi] {
            let s = MukadeSessions::from_env(a).with_cache_dir(Some(cache.clone()));
            for run in 1..=2 {
                let t = std::time::Instant::now();
                let l = s.load();
                println!(
                    "{a:?} run {run}: {} projects, {} live, {} warnings, {} worktrees, {} ms",
                    l.items.len(),
                    l.items.iter().filter(|r| r.live).count(),
                    l.warnings.len(),
                    s.managed_worktrees().len(),
                    t.elapsed().as_millis()
                );
                for w in l.warnings.iter().take(3) {
                    println!("  warn: {}", w.chars().take(160).collect::<String>());
                }
            }
        }
    }
}
