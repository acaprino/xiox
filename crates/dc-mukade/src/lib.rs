//! Session source backed by the `mukade` library (<https://github.com/acaprino/mukade>, MIT),
//! vendored under `vendor/mukade` and compiled into the app: one reader per harness (Claude Code,
//! Codex CLI, OpenCode, Pi) that parses the harness's own store. Only the listings are used
//! (`cwd`, last update, archived, live): no session is ever read in full and nothing is written to
//! the stores. What each listing covers (mukade `docs/usage.md`, "The list output"):
//!
//! * Claude Code: the `projects/**/*.jsonl` transcripts, the sessions `history.jsonl` still names
//!   after their transcript was deleted, and the `sessions/<pid>.json` registrations whose process
//!   is verified running (`live`). The listing keeps a cache of what each file said, keyed by the
//!   file's path, size and mtime, in the folder given to [`MukadeSessions::with_cache_dir`].
//! * Codex: the rollouts under `sessions/` plus every `threads` row of the newest
//!   `state_<n>.sqlite` (the catalogue outlives the rollouts), subagent threads included.
//! * OpenCode: the selected `opencode.db` and the other `opencode*.db` beside it, `session_v2`
//!   and the legacy `session` table.
//! * Pi: `sessions/**/*.jsonl`; no update time is recorded, the file's mtime stands in.
//!
//! Agent-managed git worktrees (Codex `~/.codex/worktrees`, OpenCode's `worktree` table and
//! `project.sandboxes`) are not a mukade concern: they are discovered here, read-only.

use dc_model::*;
use mukade::model::HarnessId;
use mukade::reader::{self, SessionInfo};
use mukade::roots::StoreRoots;
use rusqlite::{Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct MukadeSessions {
    agent: Agent,
    roots: StoreRoots,
    /// Folder for mukade's listing cache (Claude Code only). `None`: no cache.
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
        Self::with_roots(agent, roots)
    }

    pub fn with_roots(agent: Agent, roots: StoreRoots) -> Self {
        Self { agent, roots, cache_dir: None }
    }

    /// Where mukade may keep its listing cache: what each Claude Code session file said, keyed by
    /// the file's path, size and mtime, so only the files that changed are read next time (a cold
    /// listing of a few hundred files takes seconds, a cached one tens of milliseconds). `None`:
    /// no cache, every file is read on every scan. Nothing else is ever written.
    pub fn with_cache_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.cache_dir = dir;
        self
    }

    fn harness(&self) -> HarnessId {
        match self.agent {
            Agent::Claude => HarnessId::ClaudeCode,
            Agent::Codex => HarnessId::Codex,
            Agent::Opencode => HarnessId::OpenCode,
            Agent::Pi => HarnessId::Pi,
        }
    }

    /// The store as configured (a folder, or OpenCode's database file). An agent that is not
    /// installed has none on disk.
    fn store_root(&self) -> Option<&Path> {
        match self.agent {
            Agent::Claude => self.roots.claude_config_dir.as_deref(),
            Agent::Codex => self.roots.codex_home.as_deref(),
            Agent::Opencode => self.roots.opencode_db.as_deref(),
            Agent::Pi => self.roots.pi_agent_dir.as_deref(),
        }
    }

    fn list(&self) -> anyhow::Result<Vec<SessionInfo>> {
        match self.agent {
            // Same as `Reader::list_all`, with our cache folder instead of mukade's own
            // (`mukade-data/cache` beside the executable).
            Agent::Claude => reader::claude_code::list_cached(&self.roots, self.cache_dir.as_deref()),
            _ => reader::for_harness(self.harness())?.list_all(&self.roots),
        }
    }
}

fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    let n: i64 = s.trim().parse().ok()?;
    // Heuristic: seconds vs milliseconds.
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

fn file_mtime_ms(path: &str) -> Option<i64> {
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
        let ms = i.updated.as_deref().and_then(parse_ms).or_else(|| file_mtime_ms(&i.path));
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
        warnings.push(format!("{agent:?}: {no_cwd} sessioni senza cartella di lavoro ignorate"));
    }
    if no_time > 0 {
        warnings.push(format!("{agent:?}: {no_time} sessioni senza data ignorate"));
    }
    by.into_values().collect()
}

/// A listing error that only says the store is not there (agent not installed): stay quiet.
fn is_missing_store(e: &anyhow::Error) -> bool {
    let root = e.root_cause();
    root.downcast_ref::<reader::Lookup>()
        .is_some_and(|l| matches!(l, reader::Lookup::NoStore(_)))
        || root
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
        || root.downcast_ref::<rusqlite::Error>().is_some_and(|db| {
            matches!(db, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::CannotOpen)
        })
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
            Err(e) if is_missing_store(&e) => {}
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
            Agent::Claude | Agent::Pi => Vec::new(),
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

    /// Real stores on the developer machine: `cargo test -p dc-mukade -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real agent stores"]
    fn real_stores_smoke() {
        let cache = std::env::temp_dir().join("devclean-dc-mukade-smoke-cache");
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
