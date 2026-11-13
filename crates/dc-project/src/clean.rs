//! Selection, re-validation and removal: the only code of the targeted cleanup that deletes.
//!
//! [`select`] turns the flags into the keys of a [`ProjectScan`]; [`clean`] never trusts that
//! scan. Right before removing anything it runs the (light) analysis of the project again and
//! keeps only the chosen keys that are still listed, in the same category, unblocked and not
//! more suspect than before; the item that is removed is the *fresh* one. Folders and worktrees
//! go through one `CleanPlan` (`guard::validate`, links never followed, worktrees with
//! `git worktree remove` and never `--force`), loose files through `guard::validate_file`, and
//! archived sessions through `dc_app::revalidate_archived` and `dc_app::delete_archived`.
//!
//! Every string in the outcome is English. The engine's Italian reasons are never copied, and
//! the operating system's own error descriptions (localized) are reduced to `os error N`.

use crate::owned::{agent_slug, key_within};
use crate::search::is_link;
use crate::{Block, Category, Env, Item, Options, Project, ProjectScan, SCHEMA, SearchMode, Target, analyze_with};
use dc_cleaner::{Cancel, ParallelRemover, guard, remove_files_parallel, worker_count};
use dc_model::{Agent, ArtifactKind, CleanPlan, ItemOutcome, Level, PlanAction, PlanItem, RemoveMode, path_key};
use dc_mukade::ArchivedSession;
use dc_sessions::{HarnessCli, SessionOutcome, SystemCli, SystemTrash, Trash};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
            Block::LiveSession => "live agent session",
            Block::DirtyGit => "uncommitted changes",
            Block::UnpushedCommits => "commits that no other branch has",
            Block::Unreachable => "unreachable",
            Block::SessionLocked => "session locked",
            Block::Unknown => "unknown reason",
        })
    }
}

fn join_blocks(why: &[Block]) -> String {
    why.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")
}

// ───────────────────────────────── select ─────────────────────────────────

/// What the user asked for on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanRequest {
    /// `--builds`, `--worktrees`, `--logs`, `--sessions`.
    pub categories: Vec<Category>,
    /// `--include <path>`, any number.
    pub include: Vec<PathBuf>,
    pub mode: RemoveMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectError {
    /// `--include` of a path the analysis did not produce.
    NotInAnalysis(PathBuf),
    /// `--include` of an item that cannot be removed at all.
    Blocked { path: PathBuf, why: Vec<Block> },
    /// `Copies` or `Named` passed as a category.
    CategoryNeedsInclude(Category),
}

impl fmt::Display for SelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectError::NotInAnalysis(p) => write!(f, "not found in the analysis of this project: {}", p.display()),
            SelectError::Blocked { path, why } => {
                write!(f, "cannot be removed ({}): {}", join_blocks(why), path.display())
            }
            SelectError::CategoryNeedsInclude(c) => {
                write!(f, "'{c}' cannot be selected as a whole: name each folder with --include <path>")
            }
        }
    }
}

impl std::error::Error for SelectError {}

/// What was chosen: the keys of [`ProjectScan::items`], in the order of the listing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chosen {
    pub keys: Vec<String>,
    /// Sum of the chosen items' `bytes` (an unmeasured item counts 0), an item inside another
    /// chosen item counted once, with the outer one.
    pub bytes: u64,
    /// The chosen worktrees: they are removed by git for good, also in Recycle Bin mode.
    pub permanent_anyway: Vec<String>,
}

/// Category flags take the `Recommended`, unblocked items (for `Sessions`: every unblocked one).
/// `include` adds any unblocked item of any level except sessions, by path. Nothing is chosen
/// when the request is refused.
pub fn select(scan: &ProjectScan, req: &CleanRequest) -> Result<Chosen, SelectError> {
    if let Some(c) = req.categories.iter().find(|c| matches!(c, Category::Copies | Category::Named)) {
        return Err(SelectError::CategoryNeedsInclude(*c));
    }
    let mut keys: HashSet<&str> = HashSet::new();
    for c in &req.categories {
        for i in scan.items.iter().filter(|i| i.category == *c && i.blocked.is_empty()) {
            if *c == Category::Sessions || i.level == Level::Recommended {
                keys.insert(&i.key);
            }
        }
    }
    for given in &req.include {
        let abs = std::path::absolute(given).unwrap_or_else(|_| given.clone());
        let key = path_key(&abs);
        let Some(i) = scan.items.iter().find(|i| i.path.is_some() && i.key == key) else {
            return Err(SelectError::NotInAnalysis(given.clone()));
        };
        if !i.blocked.is_empty() {
            return Err(SelectError::Blocked { path: given.clone(), why: i.blocked.clone() });
        }
        keys.insert(&i.key);
    }

    let chosen: Vec<&Item> = scan.items.iter().filter(|i| keys.contains(i.key.as_str())).collect();
    let nested = nested_keys(&chosen);
    Ok(Chosen {
        bytes: chosen
            .iter()
            .filter(|i| !nested.contains(&i.key))
            .fold(0u64, |sum, i| sum.saturating_add(i.bytes.unwrap_or(0))),
        permanent_anyway: chosen
            .iter()
            .filter(|i| matches!(i.target, Target::Worktree { .. }))
            .map(|i| i.key.clone())
            .collect(),
        keys: chosen.iter().map(|i| i.key.clone()).collect(),
    })
}

/// Keys of the items that sit inside another item of `items` (removing the outer one removes them).
fn nested_keys(items: &[&Item]) -> HashSet<String> {
    items
        .iter()
        .filter(|i| i.path.is_some())
        .filter(|i| items.iter().any(|o| o.path.is_some() && o.key != i.key && key_within(&i.key, &o.key)))
        .map(|i| i.key.clone())
        .collect()
}

/// The items of `scan` that `chosen` names, in the order of `chosen.keys`. A key the scan does
/// not list can only come from a hand-made [`Chosen`]: it names nothing and is left out.
fn chosen_items<'a>(scan: &'a ProjectScan, chosen: &Chosen) -> Vec<&'a Item> {
    chosen.keys.iter().filter_map(|k| scan.items.iter().find(|i| i.key == *k)).collect()
}

// ───────────────────────────────── outcome ─────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Outcome {
    /// Measured (or, in a dry run, expected) bytes.
    Removed { bytes_freed: u64 },
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanOutcome {
    /// [`SCHEMA`].
    pub schema: u32,
    pub mode: RemoveMode,
    pub dry_run: bool,
    /// In the order of `Chosen::keys`: key, path (`None` for sessions), category, what happened.
    pub items: Vec<(String, Option<PathBuf>, Category, Outcome)>,
    /// Sum of the `Removed` bytes, what went to the Recycle Bin included.
    pub bytes_freed: u64,
    /// The part of `bytes_freed` moved to the Recycle Bin: freed only when it is emptied.
    pub recycled_bytes: u64,
    pub elapsed_ms: u64,
}

impl CleanOutcome {
    /// At least one item was skipped or failed.
    pub fn partial(&self) -> bool {
        self.items.iter().any(|(.., o)| !matches!(o, Outcome::Removed { .. }))
    }
}

fn skipped(reason: impl Into<String>) -> Outcome {
    Outcome::Skipped { reason: reason.into() }
}

fn failed(error: impl Into<String>) -> Outcome {
    Outcome::Failed { error: error.into() }
}

// ──────────────────────────── English-only text ────────────────────────────

/// `s` with every operating-system error description (Rust prints `<localized text> (os error
/// N)`, in the language of Windows) cut down to `os error N`. Nothing else is touched.
pub(crate) fn strip_os_text(s: &str) -> String {
    const TAG: &str = "(os error ";
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find(TAG) {
        let head = &rest[..p];
        let after = &rest[p + TAG.len()..];
        let Some(end) = after.find(')') else { break };
        let code = &after[..end];
        if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        // The description starts after the last ": " before the tag (or at the start).
        let start = head.rfind(": ").map_or(0, |i| i + 2);
        out.push_str(&head[..start]);
        out.push_str("os error ");
        out.push_str(code);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// ` (os error N)` when `s` carries a code, else nothing.
fn os_code(s: &str) -> String {
    let t = strip_os_text(s);
    t.find("os error ")
        .map(|i| {
            let digits: String = t[i + "os error ".len()..].chars().take_while(char::is_ascii_digit).collect();
            format!(" (os error {digits})")
        })
        .unwrap_or_default()
}

fn io_code(e: &io::Error) -> String {
    e.raw_os_error().map(|c| format!(" (os error {c})")).unwrap_or_default()
}

/// A `Failed` text of the remover: English, with the OS text reduced and git's own output (in
/// the user's language) left out.
pub(crate) fn english_error(error: &str, locked_by: &[String]) -> String {
    let base = if error.starts_with("git worktree remove failed") {
        "git refused to remove the worktree (not forced; local changes are never discarded)".to_string()
    } else {
        strip_os_text(error)
    };
    if locked_by.is_empty() { base } else { format!("{base} (in use by: {})", locked_by.join(", ")) }
}

fn from_remover(o: &ItemOutcome) -> Outcome {
    match o {
        ItemOutcome::Removed { bytes_freed } => Outcome::Removed { bytes_freed: *bytes_freed },
        ItemOutcome::Skipped { reason } => skipped(strip_os_text(reason)),
        ItemOutcome::Failed { error, locked_by } => failed(english_error(error, locked_by)),
    }
}

// ───────────────────────────── the Recycle Bin ─────────────────────────────

/// Uppercase drive letter of `C:\…`; `None` for UNC and relative paths.
fn drive_letter(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

/// Bytes the item frees: what the analysis measured, else measured now.
fn expected_bytes(item: &Item) -> u64 {
    if let Some(b) = item.bytes {
        return b;
    }
    match (&item.target, &item.path) {
        (Target::File, Some(p)) => fs::symlink_metadata(p).map_or(0, |m| m.len()),
        (Target::Dir { .. } | Target::Worktree { .. }, Some(p)) => dc_cleaner::measure_dir(p).effective_bytes(),
        _ => 0,
    }
}

/// Volumes whose Recycle Bin cannot take the chosen items: `(drive, bytes moved, room)`. Empty
/// for `Permanent`. Only folders and files go to the bin: a worktree is removed by git, a
/// session is small, and an item inside another chosen one goes with it.
pub fn recycle_shortfall(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode) -> Vec<(char, u64, u64)> {
    shortfall_with(scan, chosen, mode, dc_cleaner::recycle_room)
}

/// [`recycle_shortfall`] with the room of each bin given (tests).
pub(crate) fn shortfall_with(
    scan: &ProjectScan,
    chosen: &Chosen,
    mode: RemoveMode,
    room: impl Fn(char) -> Option<u64>,
) -> Vec<(char, u64, u64)> {
    if mode == RemoveMode::Permanent {
        return Vec::new();
    }
    let items = chosen_items(scan, chosen);
    let nested = nested_keys(&items);
    let mut per_volume: BTreeMap<char, u64> = BTreeMap::new();
    for i in items.iter().filter(|i| !nested.contains(&i.key)) {
        if !matches!(i.target, Target::Dir { .. } | Target::File) {
            continue;
        }
        if let Some(letter) = i.path.as_deref().and_then(drive_letter) {
            let sum = per_volume.entry(letter).or_default();
            *sum = sum.saturating_add(expected_bytes(i));
        }
    }
    let needed: Vec<(char, u64)> = per_volume.into_iter().collect();
    dc_cleaner::shortfall(&needed, room)
}

// ───────────────────────────────── clean ─────────────────────────────────

/// What the removal needs from outside (tests pass fakes).
pub(crate) struct Io<'a> {
    pub cli: &'a dyn HarnessCli,
    /// The Recycle Bin, for loose files and sessions.
    pub trash: &'a dyn Trash,
    /// The archived sessions of a harness as they are now (`dc_app::fresh_archived`).
    pub fresh: &'a dyn Fn(Agent) -> Vec<ArchivedSession>,
}

/// Remove what `chosen` names from `scan` (see the module doc): re-validate, then delete. With
/// `dry_run` every check runs and nothing is touched; the outcomes say what would be freed.
/// Reads the real sessions of this PC, like the analysis does.
pub fn clean(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, dry_run: bool) -> CleanOutcome {
    let cache = Some(dc_app::cache_dir());
    let (sessions, _) = dc_app::load_sessions(cache.clone());
    let (archived, _) = dc_app::load_archived_sessions(cache);
    let env = Env { sessions: &sessions, archived: &archived, search_roots: None, now_ms: dc_app::now_ms() };
    let cli = SystemCli::locate();
    let io = Io { cli: &cli, trash: &SystemTrash, fresh: &dc_app::fresh_archived };
    clean_with(scan, chosen, mode, dry_run, &env, &io)
}

pub(crate) fn clean_with(
    scan: &ProjectScan,
    chosen: &Chosen,
    mode: RemoveMode,
    dry_run: bool,
    env: &Env,
    io: &Io,
) -> CleanOutcome {
    let started = Instant::now();
    let old = chosen_items(scan, chosen);
    let mut done: HashMap<String, Outcome> = HashMap::new();

    // The search of copies and named folders is only worth repeating when one was chosen.
    let wants_search = old.iter().any(|i| matches!(i.category, Category::Copies | Category::Named));
    let opts = Options {
        drives: wants_search && scan.search != SearchMode::ParentOnly,
        measure: false,
        admin: scan.search == SearchMode::MftIndex,
    };
    match analyze_with(&scan.project.root, &opts, env) {
        Ok(fresh) => {
            let run = Run { mode, dry_run, project: &fresh.project, env, io };
            run.remove(&old, &fresh, &mut done);
        }
        Err(e) => {
            for i in &old {
                done.insert(i.key.clone(), skipped(format!("the project could not be checked again: {e}")));
            }
        }
    }

    let mut items = Vec::with_capacity(old.len());
    let (mut bytes_freed, mut recycled_bytes) = (0u64, 0u64);
    for o in &old {
        let out = done.remove(&o.key).unwrap_or_else(|| skipped("it was not processed"));
        if let Outcome::Removed { bytes_freed: b } = &out {
            bytes_freed = bytes_freed.saturating_add(*b);
            if mode == RemoveMode::Recycle && !matches!(o.target, Target::Worktree { .. }) {
                recycled_bytes = recycled_bytes.saturating_add(*b);
            }
        }
        items.push((o.key.clone(), o.path.clone(), o.category, out));
    }
    CleanOutcome {
        schema: SCHEMA,
        mode,
        dry_run,
        items,
        bytes_freed,
        recycled_bytes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// A chosen item and its fresh counterpart.
struct Todo<'a> {
    old: &'a Item,
    fresh: &'a Item,
}

impl Todo<'_> {
    fn path(&self) -> &Path {
        self.fresh.path.as_deref().expect("a todo of the path rounds has a path")
    }

    /// What a dry run says it would free: as the user saw it in the analysis, else measured now.
    fn expected(&self) -> u64 {
        match self.old.bytes {
            Some(b) => b,
            None => expected_bytes(self.fresh),
        }
    }
}

struct Run<'a> {
    mode: RemoveMode,
    dry_run: bool,
    /// The project as it is now.
    project: &'a Project,
    env: &'a Env<'a>,
    io: &'a Io<'a>,
}

/// Why a chosen item is not removed after the fresh analysis; `Ok` is the fresh item.
fn recheck<'a>(old: &Item, fresh: Option<&'a Item>) -> Result<&'a Item, String> {
    let Some(f) = fresh else {
        return Err(if old.path.is_none() { "no longer archived" } else { "no longer there" }.to_string());
    };
    if f.category != old.category || std::mem::discriminant(&f.target) != std::mem::discriminant(&old.target) {
        return Err(format!("it is now listed as {}, not {}", f.category, old.category));
    }
    if !f.blocked.is_empty() {
        return Err(format!("now blocked: {}", join_blocks(&f.blocked)));
    }
    if f.level == Level::Caution && old.level != Level::Caution {
        return Err("now needs a manual review (it is no longer a recommended item)".to_string());
    }
    Ok(f)
}

impl Run<'_> {
    fn remove(&self, old: &[&Item], fresh: &ProjectScan, done: &mut HashMap<String, Outcome>) {
        let by_key: HashMap<&str, &Item> = fresh.items.iter().map(|i| (i.key.as_str(), i)).collect();
        let mut paths: Vec<Todo> = Vec::new();
        let mut sessions: Vec<Todo> = Vec::new();
        for o in old {
            match recheck(o, by_key.get(o.key.as_str()).copied()) {
                Err(reason) => {
                    done.insert(o.key.clone(), skipped(reason));
                }
                Ok(f) if f.path.is_some() => paths.push(Todo { old: o, fresh: f }),
                Ok(f) => sessions.push(Todo { old: o, fresh: f }),
            }
        }
        self.remove_paths(&paths, done);
        self.remove_sessions(&sessions, done);
    }

    /// Folders, worktrees and files, outer items first: an item inside another chosen one is
    /// `Skipped` once the outer one is gone, and still tried when the outer one was not removed.
    fn remove_paths(&self, todos: &[Todo], done: &mut HashMap<String, Outcome>) {
        let mut pending: Vec<usize> = (0..todos.len()).collect();
        while !pending.is_empty() {
            let mut still = Vec::new();
            for &i in &pending {
                let key = &todos[i].fresh.key;
                let outer = todos.iter().find(|o| {
                    o.fresh.key != *key
                        && key_within(key, &o.fresh.key)
                        && matches!(done.get(&o.fresh.key), Some(Outcome::Removed { .. }))
                });
                match outer {
                    Some(o) => {
                        done.insert(key.clone(), skipped(format!("already removed with {}", o.path().display())));
                    }
                    None => still.push(i),
                }
            }
            pending = still;
            let (ready, wait): (Vec<usize>, Vec<usize>) = pending.iter().copied().partition(|&i| {
                !pending.iter().any(|&j| j != i && key_within(&todos[i].fresh.key, &todos[j].fresh.key))
            });
            if ready.is_empty() {
                // Cannot happen (containment is a strict order); never guess when it does.
                for &i in &wait {
                    done.insert(todos[i].fresh.key.clone(), failed("internal error: items could not be ordered"));
                }
                return;
            }
            let round: Vec<&Todo> = ready.iter().map(|&i| &todos[i]).collect();
            self.round(&round, done);
            pending = wait;
        }
    }

    fn round(&self, todos: &[&Todo], done: &mut HashMap<String, Outcome>) {
        let (dirs, files): (Vec<&Todo>, Vec<&Todo>) =
            todos.iter().partition(|t| matches!(t.fresh.target, Target::Dir { .. } | Target::Worktree { .. }));
        self.remove_dirs(&dirs, done);
        self.remove_files(&files, done);
    }

    // ── folders and worktrees ──

    fn remove_dirs(&self, todos: &[&Todo], done: &mut HashMap<String, Outcome>) {
        let mut checked: Vec<(&Todo, PlanItem)> = Vec::new();
        for t in todos {
            match check_dir(t.fresh, t.old.bytes.unwrap_or(0), self.project) {
                Ok(pi) => checked.push((t, pi)),
                Err(reason) => {
                    done.insert(t.fresh.key.clone(), skipped(reason));
                }
            }
        }
        if checked.is_empty() {
            return;
        }
        if self.dry_run {
            for (t, _) in &checked {
                done.insert(t.fresh.key.clone(), Outcome::Removed { bytes_freed: t.expected() });
            }
            return;
        }
        let items: Vec<PlanItem> = checked.iter().map(|(_, pi)| pi.clone()).collect();
        let plan = CleanPlan {
            id: "xiox-clean".to_string(),
            mode: self.mode,
            expected_bytes: items.iter().fold(0u64, |s, i| s.saturating_add(i.expected_bytes)),
            items,
        };
        let res = ParallelRemover { workers: worker_count() }.execute_with(&plan, false, &|_| {}, &Cancel::default());
        for (t, pi) in &checked {
            let key = path_key(&pi.path);
            let out = res
                .items
                .iter()
                .find(|r| path_key(&r.path) == key)
                .map(|r| from_remover(&r.outcome))
                .unwrap_or_else(|| failed("the removal did not report on this item"));
            done.insert(t.fresh.key.clone(), out);
        }
    }

    // ── loose files ──

    fn remove_files(&self, todos: &[&Todo], done: &mut HashMap<String, Outcome>) {
        let mut valid: Vec<(&Todo, PathBuf, u64)> = Vec::new();
        for t in todos {
            let path = t.path();
            let md = match fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    done.insert(t.fresh.key.clone(), skipped("no longer there"));
                    continue;
                }
                Err(e) => {
                    done.insert(t.fresh.key.clone(), skipped(format!("cannot access it{}", io_code(&e))));
                    continue;
                }
            };
            match guard::validate_file(path) {
                Ok(p) => valid.push((t, p, md.len())),
                Err(reason) => {
                    done.insert(t.fresh.key.clone(), skipped(strip_os_text(&reason)));
                }
            }
        }
        if valid.is_empty() {
            return;
        }
        if self.dry_run {
            for (t, _, len) in &valid {
                done.insert(t.fresh.key.clone(), Outcome::Removed { bytes_freed: *len });
            }
            return;
        }
        match self.mode {
            RemoveMode::Permanent => {
                let files: Vec<(PathBuf, u64)> = valid.iter().map(|(_, p, len)| (p.clone(), *len)).collect();
                let (_, _, failures, _) = remove_files_parallel(&files, worker_count(), &|_| {}, &Cancel::default());
                for (t, p, len) in &valid {
                    let k = path_key(p);
                    let out = if let Some((_, e)) = failures.iter().find(|(fp, _)| path_key(fp) == k) {
                        failed(format!("could not delete the file{}", os_code(e)))
                    } else if gone(p) {
                        Outcome::Removed { bytes_freed: *len }
                    } else {
                        failed("the file is still there after the removal")
                    };
                    done.insert(t.fresh.key.clone(), out);
                }
            }
            RemoveMode::Recycle => {
                for (t, p, len) in &valid {
                    let out = match self.io.trash.to_bin(p) {
                        Ok(()) if gone(p) => Outcome::Removed { bytes_freed: *len },
                        Ok(()) => failed("the file is still there after moving it to the Recycle Bin"),
                        Err(e) => failed(format!("could not move the file to the Recycle Bin{}", os_code(&e))),
                    };
                    done.insert(t.fresh.key.clone(), out);
                }
            }
        }
    }

    // ── sessions ──

    fn remove_sessions(&self, todos: &[Todo], done: &mut HashMap<String, Outcome>) {
        if todos.is_empty() {
            return;
        }
        // The allow-list of the engine: the archived sessions as the analysis just read them.
        let dev = dc_app::DevScan { archived: self.env.archived.to_vec(), ..Default::default() };
        let picks: Vec<(Agent, String)> = todos
            .iter()
            .filter_map(|t| match &t.fresh.target {
                Target::Session { agent, id } => Some((*agent, id.clone())),
                _ => None,
            })
            .collect();
        let key_of = |agent: Agent, id: &str| format!("session:{}:{}", agent_slug(agent), id);
        let checked = match dc_app::revalidate_archived(&dev, &picks, self.io.fresh) {
            Ok(c) => c,
            Err(_) => {
                for (agent, id) in &picks {
                    done.insert(key_of(*agent, id), failed("the session could not be checked and was left alone"));
                }
                return;
            }
        };
        for s in &checked.skipped {
            done.insert(key_of(s.agent, &s.id), skipped("no longer archived, or in use now"));
        }
        if checked.keep.is_empty() {
            return;
        }
        if self.dry_run {
            for (agent, id) in &checked.keep {
                let key = key_of(*agent, id);
                let bytes = todos.iter().find(|t| t.fresh.key == key).map_or(0, |t| t.expected());
                done.insert(key, Outcome::Removed { bytes_freed: bytes });
            }
            return;
        }
        match dc_app::delete_archived(&dev, &checked.keep, self.mode, self.io.cli, self.io.trash) {
            Ok(outcomes) => {
                for o in &outcomes {
                    done.insert(key_of(o.agent, &o.id), from_session(o));
                }
            }
            Err(_) => {
                for (agent, id) in &checked.keep {
                    done.insert(key_of(*agent, id), failed("the sessions were refused and left alone"));
                }
            }
        }
    }
}

/// Session texts are Italian: only the verdict and an OS error code are kept.
fn from_session(o: &SessionOutcome) -> Outcome {
    match &o.outcome {
        ItemOutcome::Removed { bytes_freed } => Outcome::Removed { bytes_freed: *bytes_freed },
        ItemOutcome::Skipped { .. } => skipped("the session is locked or no longer archived"),
        ItemOutcome::Failed { error, .. } => failed(format!("could not delete the session{}", os_code(error))),
    }
}

fn gone(p: &Path) -> bool {
    fs::symlink_metadata(p).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
}

// ──────────────────────────── the checks of a folder ────────────────────────────

/// The project root, any worktree, or a folder inside or around them: a `ProjectFolder` that
/// overlaps them must never be removed (it would take the project, or a worktree, with it, or
/// be part of it). Keys are `path_key`s.
pub(crate) fn overlaps_project(project: &Project, key: &str) -> bool {
    std::iter::once(&project.root).chain(project.worktrees.iter()).map(|p| path_key(p)).any(|own| {
        key_within(key, &own) || key_within(&own, key)
    })
}

/// A worktree that is, or holds, the project's root cannot be removed.
fn holds_root(project: &Project, key: &str) -> bool {
    key_within(&path_key(&project.root), key)
}

/// The checks of a folder or worktree right before it goes, and its plan item. `Err` is the
/// English reason it is skipped. Nothing is trusted from the analysis but the path: the item
/// must be there now, a real folder (never a link of any kind), outside the project when it is
/// a copy or a named folder, and accepted by `guard::validate`.
pub(crate) fn check_dir(fresh: &Item, expected_bytes: u64, project: &Project) -> Result<PlanItem, String> {
    let Some(path) = fresh.path.as_deref() else { return Err("not a folder".to_string()) };
    let md = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err("no longer there".to_string()),
        Err(e) => return Err(format!("cannot access it{}", io_code(&e))),
    };
    if is_link(&md) {
        return Err("now a link: not followed".to_string());
    }
    if !md.is_dir() {
        return Err("no longer a folder".to_string());
    }
    let key = &fresh.key;
    let (kind, action) = match &fresh.target {
        Target::Dir { kind } => {
            if *kind == ArtifactKind::ProjectFolder && overlaps_project(project, key) {
                return Err("it overlaps the project or one of its worktrees".to_string());
            }
            (*kind, PlanAction::RemoveDir)
        }
        Target::Worktree { .. } => {
            if holds_root(project, key) {
                return Err("it is, or contains, the project's main folder".to_string());
            }
            // The repository is the project as it is now, not what the analysis said.
            (ArtifactKind::WorktreeDir, PlanAction::GitWorktreeRemove { repo: project.root.clone() })
        }
        _ => return Err("not a folder".to_string()),
    };
    let item = PlanItem {
        path: dc_model::normalize_path(path),
        kind,
        action,
        expected_bytes,
        project_id: project.name.clone(),
        reason: "chosen with xiox clean".to_string(),
    };
    guard::validate(&item).map_err(|r| strip_os_text(&r))?;
    Ok(item)
}
