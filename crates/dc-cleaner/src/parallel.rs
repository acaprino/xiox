//! Plan execution with a pool of worker threads, progress ticks and «Annulla».
//!
//! 1. In sequence, every item is validated (`guard::validate`) and measured: a refused item is
//!    `Skipped` and never reaches a worker. The workers only delete what was validated.
//! 2. A rayon pool of `workers` threads.
//! 3. Permanent `RemoveDir`: each folder is a task that deletes its own files and spawns a task
//!    for each subfolder; links are removed as links, never followed. Every folder, the item's
//!    own one included, is checked again right before it is listed (`fsops::open_dir`): one
//!    that has become a link since is not entered, only the link goes (an item: 0 bytes).
//!    When the last task of an item ends, its folders are removed deepest first. `Removed` only once the path is gone.
//! 4. Recycle Bin: one task per item, same pool.
//! 5. `GitWorktreeRemove`: in sequence, after the rest.
//! 6. «Annulla» ([`Cancel`]) is checked before every file: an interrupted item is
//!    `Skipped { "Annullato: liberati {n} byte" }`, an item never started `Skipped { "Annullato" }`.
//! 7. Progress: atomic counters, read by one thread that calls `tick` at most every 100 ms.

use crate::{failed, fsops, guard, locks, permanent_failure, skipped};
use dc_model::{CleanPlan, CleanResult, ItemOutcome, ItemResult, PlanAction, PlanItem, RemoveMode, Remover, path_key};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shortest interval between two calls of `tick`.
const TICK_EVERY: Duration = Duration::from_millis(100);

const CANCELLED: &str = "Annullato";

/// Worker threads for a cleanup: `min(available_parallelism, 8)`, at least 1.
pub fn worker_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 8)
}

/// «Annulla»: shared between the UI and the workers. Cloning shares the same flag.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(SeqCst)
    }
}

/// Progress of a running cleanup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_items: usize,
    pub total_items: usize,
}

#[derive(Default)]
struct Progress {
    done_bytes: AtomicU64,
    done_items: AtomicUsize,
    total_bytes: u64,
    total_items: usize,
}

impl Progress {
    fn snapshot(&self) -> Tick {
        Tick {
            done_bytes: self.done_bytes.load(SeqCst).min(self.total_bytes),
            total_bytes: self.total_bytes,
            done_items: self.done_items.load(SeqCst).min(self.total_items),
            total_items: self.total_items,
        }
    }
}

/// Runs `work` on this thread while one other thread calls `tick` every [`TICK_EVERY`], and
/// once more with the final counters when `work` has ended. A panicking `tick` only stops
/// the ticks.
fn with_ticker<R>(progress: &Progress, tick: &(dyn Fn(Tick) + Sync), work: impl FnOnce() -> R) -> R {
    let (stop, stopped) = channel::<()>();
    std::thread::scope(|s| {
        s.spawn(move || {
            let call = || catch_unwind(AssertUnwindSafe(|| tick(progress.snapshot()))).is_ok();
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(TICK_EVERY) {
                if !call() {
                    return;
                }
            }
            // The work is done: one last tick with the final counters.
            call();
        });
        let r = work();
        drop(stop);
        r
    })
}

fn pool(workers: usize) -> Result<rayon::ThreadPool, String> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers.max(1))
        .thread_name(|i| format!("pulisci-worker-{i}"))
        .build()
        .map_err(|e| format!("could not start the worker threads: {e}"))
}

// ───────────────────────────── preparation (sequential) ─────────────────────────────

enum Job {
    /// Permanent delete of a real folder, by the workers.
    Tree,
    /// Move to the Recycle Bin.
    Recycle,
    /// The item path is a link: only the link goes (0 bytes).
    Link,
    /// `git worktree remove`, in sequence after the rest.
    Worktree(PathBuf),
}

struct Prepared {
    idx: usize,
    /// Normalized (validated) path.
    path: PathBuf,
    job: Job,
    /// Measured size: what `Removed` reports.
    bytes: u64,
    files: u64,
    /// Folders below the item's own one.
    dirs: u64,
}

/// Validation and measurement of one item: `Err` is its final outcome (refused, dry run).
fn prepare(mode: RemoveMode, idx: usize, item: &PlanItem, dry_run: bool) -> Result<Prepared, ItemOutcome> {
    let checked = guard::validate(item).map_err(skipped)?;
    let path = checked.path;

    if let PlanAction::GitWorktreeRemove { repo } = &item.action {
        if checked.is_link {
            return Err(skipped("worktree path is a link: refusing git worktree remove"));
        }
        crate::validate_repo(repo, &path).map_err(skipped)?;
    }

    // Links: nothing to measure (the target is never touched, so nothing is freed).
    let (size, dirs) = if checked.is_link { Default::default() } else { fsops::measure_counted(&path) };
    let bytes = size.effective_bytes();

    if dry_run {
        return Err(skipped(if checked.is_link {
            "dry-run: would free 0 bytes (path is a link: only the link would be removed)".to_string()
        } else {
            format!("dry-run: would free {bytes} bytes")
        }));
    }

    let job = if checked.is_link {
        Job::Link
    } else {
        match (&item.action, mode) {
            (PlanAction::RemoveDir, RemoveMode::Permanent) => Job::Tree,
            (PlanAction::RemoveDir, RemoveMode::Recycle) => Job::Recycle,
            (PlanAction::GitWorktreeRemove { repo }, _) => Job::Worktree(repo.clone()),
        }
    };
    Ok(Prepared { idx, path, job, bytes, files: size.file_count, dirs })
}

// ───────────────────────────── per-item state (workers) ─────────────────────────────

#[derive(Default)]
struct ItemState {
    /// Tasks of this item queued or running; the one that brings it to 0 finishes the item.
    pending: AtomicUsize,
    /// A worker began working on the item.
    started: AtomicBool,
    /// «Annulla» stopped a task of this item before it was done.
    interrupted: AtomicBool,
    /// Logical bytes of the files deleted so far.
    deleted_bytes: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    /// Mirror of `report.failures`, read without the lock (retry patience).
    failures: AtomicU64,
    report: Mutex<fsops::RemoveReport>,
    /// Folders seen, with their depth below the item (0 = the item itself).
    seen_dirs: Mutex<Vec<(usize, PathBuf)>>,
    /// Folders that must not be removed (something below them could not go), and so neither
    /// can their parents.
    blocked: Mutex<HashSet<PathBuf>>,
    outcome: Mutex<Option<ItemOutcome>>,
    /// Bytes this item added to `Progress::done_bytes`.
    progressed: AtomicU64,
    /// The item folder had become a link by the time it was listed: only the link went.
    root_was_link: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ItemState {
    fn patient(&self) -> bool {
        self.failures.load(SeqCst) == 0
    }

    /// `p` could not be removed (or listed); `holder` is the folder that therefore stays.
    fn fail(&self, p: &Path, e: io::Error, holder: Option<&Path>) {
        self.failures.fetch_add(1, SeqCst);
        lock(&self.report).fail(p, e);
        if let Some(h) = holder {
            lock(&self.blocked).insert(h.to_path_buf());
        }
    }

    fn set(&self, o: ItemOutcome) {
        *lock(&self.outcome) = Some(o);
    }
}

struct Ctx<'a> {
    cancel: &'a Cancel,
    progress: &'a Progress,
    any_cancelled: AtomicBool,
}

impl Ctx<'_> {
    fn cancelled(&self, st: &ItemState) -> bool {
        if self.cancel.is_cancelled() {
            st.interrupted.store(true, SeqCst);
            true
        } else {
            false
        }
    }

    fn add_bytes(&self, st: &ItemState, n: u64) {
        st.progressed.fetch_add(n, SeqCst);
        self.progress.done_bytes.fetch_add(n, SeqCst);
    }

    /// The outcome of an item «Annulla» stopped: `n` = bytes already deleted.
    fn cancelled_outcome(&self, started: bool, n: u64) -> ItemOutcome {
        self.any_cancelled.store(true, SeqCst);
        if started { skipped(format!("{CANCELLED}: liberati {n} byte")) } else { skipped(CANCELLED) }
    }

    fn item_done(&self) {
        self.progress.done_items.fetch_add(1, SeqCst);
    }
}

fn internal_error() -> ItemOutcome {
    failed("internal error while removing this item", vec![])
}

/// One folder of a permanent delete: its files, its links, then a task per subfolder.
fn tree_task<'s>(s: &rayon::Scope<'s>, ctx: &'s Ctx<'s>, job: &'s Prepared, st: &'s ItemState, dir: PathBuf, depth: usize) {
    let r = catch_unwind(AssertUnwindSafe(|| tree_dir(s, ctx, job, st, &dir, depth)));
    if r.is_err() {
        st.set(internal_error());
    }
    if st.pending.fetch_sub(1, SeqCst) == 1 {
        finish_tree(ctx, job, st);
    }
}

fn tree_dir<'s>(s: &rayon::Scope<'s>, ctx: &'s Ctx<'s>, job: &'s Prepared, st: &'s ItemState, dir: &Path, depth: usize) {
    if ctx.cancelled(st) {
        return;
    }
    st.started.store(true, SeqCst);
    // Every folder, the item's own one included, is checked right before it is listed: a
    // folder replaced by a link since it was validated (the item) or listed (a subfolder) is
    // never entered, only the link goes. On Windows the check is made on a handle that also
    // keeps the folder from being renamed or replaced until its files are deleted (`_pin`).
    let _pin = match fsops::open_dir(dir) {
        Ok(fsops::DirCheck::Dir(pin)) => pin,
        Ok(fsops::DirCheck::Link) => {
            match fsops::remove_link_entry(dir, st.patient()) {
                Ok(()) if depth == 0 => st.root_was_link.store(true, SeqCst),
                Ok(()) => {}
                Err(e) => st.fail(dir, e, dir.parent()),
            }
            return;
        }
        Ok(fsops::DirCheck::Reparse) => {
            let e = io::Error::other("reparse point that is not a link: not entered");
            return st.fail(dir, e, dir.parent());
        }
        Ok(fsops::DirCheck::NotDir) => {
            return st.fail(dir, io::Error::other("no longer a folder"), dir.parent());
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        // Not in `seen_dirs`: block the parent directly.
        Err(e) => return st.fail(dir, e, dir.parent()),
    };
    lock(&st.seen_dirs).push((depth, dir.to_path_buf()));
    let entries: Vec<_> = match fs::read_dir(dir) {
        // Collect first: do not mutate the directory while iterating it.
        Ok(rd) => rd.collect(),
        Err(e) => return st.fail(dir, e, Some(dir)),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                st.fail(dir, e, Some(dir));
                continue;
            }
        };
        let p = entry.path();
        match entry.file_type() {
            Err(e) => st.fail(&p, e, Some(dir)),
            Ok(ft) if ft.is_dir() && !ft.is_symlink() => {
                st.pending.fetch_add(1, SeqCst);
                s.spawn(move |s| tree_task(s, ctx, job, st, p, depth + 1));
            }
            Ok(ft) => {
                if ctx.cancelled(st) {
                    return;
                }
                // Links are removed but not counted as files (as in `fsops::measure`).
                let r = if ft.is_symlink() {
                    fsops::remove_link_entry(&p, st.patient())
                } else {
                    // Logical length: a hardlinked file counts in full here (the link count is
                    // not in the directory listing), so the «Annullato: liberati n» figure can
                    // overstate by the files that have other links. `Removed` reports the
                    // measured size, which accounts for hardlinks.
                    let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    fsops::with_retry(&p, st.patient(), || fs::remove_file(&p)).inspect(|()| {
                        st.deleted_bytes.fetch_add(len, SeqCst);
                        ctx.add_bytes(st, len);
                        st.files.fetch_add(1, SeqCst);
                    })
                };
                if let Err(e) = r {
                    st.fail(&p, e, Some(dir));
                }
            }
        }
    }
}

/// The last task of a permanent delete: remove the folders deepest first, then decide the
/// outcome.
fn finish_tree(ctx: &Ctx<'_>, job: &Prepared, st: &ItemState) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if lock(&st.outcome).is_some() {
            return; // a task panicked
        }
        if !st.interrupted.load(SeqCst) {
            remove_dirs(st);
        }
        let rep = std::mem::take(&mut *lock(&st.report));
        let outcome = if rep.first.is_some() {
            permanent_failure(rep).err().unwrap_or_else(internal_error)
        } else if st.interrupted.load(SeqCst) {
            let n = st.deleted_bytes.load(SeqCst).min(job.bytes);
            ctx.cancelled_outcome(st.started.load(SeqCst), n)
        } else if fsops::is_gone(&job.path) {
            ctx.add_bytes(st, job.bytes.saturating_sub(st.progressed.load(SeqCst)));
            // Only a link went: its target was never touched, nothing was freed.
            let bytes_freed = if st.root_was_link.load(SeqCst) { 0 } else { job.bytes };
            ItemOutcome::Removed { bytes_freed }
        } else {
            failed("removal reported success but the path still exists", vec![])
        };
        st.set(outcome);
    }));
    if r.is_err() {
        st.set(internal_error());
    }
    ctx.item_done();
}

/// Removes the item's folders deepest first, in sequence on this worker (removing an empty
/// folder is cheap; no nested parallel work on a pool thread's stack).
fn remove_dirs(st: &ItemState) {
    let mut dirs = std::mem::take(&mut *lock(&st.seen_dirs));
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, d) in dirs {
        if lock(&st.blocked).contains(&d) {
            if let Some(parent) = d.parent() {
                lock(&st.blocked).insert(parent.to_path_buf());
            }
            continue;
        }
        match fsops::with_retry(&d, st.patient(), || fs::remove_dir(&d)) {
            Ok(()) => {
                st.dirs.fetch_add(1, SeqCst);
            }
            Err(e) => st.fail(&d, e, d.parent()),
        }
    }
}

/// Recycle Bin, or the item path is a link: one task for the whole item.
fn single_task(ctx: &Ctx<'_>, job: &Prepared, st: &ItemState) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if ctx.cancel.is_cancelled() {
            return ctx.cancelled_outcome(false, 0);
        }
        let r = match job.job {
            Job::Link => crate::remove_link_item(&job.path),
            _ => crate::remove_recycle(&job.path),
        };
        match r {
            Ok(()) if fsops::is_gone(&job.path) => {
                if !matches!(job.job, Job::Link) {
                    st.files.store(job.files, SeqCst);
                    st.dirs.store(job.dirs + 1, SeqCst);
                }
                ctx.add_bytes(st, job.bytes);
                ItemOutcome::Removed { bytes_freed: job.bytes }
            }
            Ok(()) => failed("removal reported success but the path still exists", vec![]),
            Err(f) => f,
        }
    }));
    st.set(r.unwrap_or_else(|_| internal_error()));
    ctx.item_done();
}

fn worktree_job(ctx: &Ctx<'_>, job: &Prepared, repo: &Path, st: &ItemState) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if ctx.cancel.is_cancelled() {
            return ctx.cancelled_outcome(false, 0);
        }
        match crate::remove_git_worktree(repo, &job.path) {
            Ok(()) if fsops::is_gone(&job.path) => {
                st.files.store(job.files, SeqCst);
                st.dirs.store(job.dirs + 1, SeqCst);
                ctx.add_bytes(st, job.bytes);
                ItemOutcome::Removed { bytes_freed: job.bytes }
            }
            Ok(()) => failed("removal reported success but the path still exists", vec![]),
            Err(f) => f,
        }
    }));
    st.set(r.unwrap_or_else(|_| internal_error()));
    ctx.item_done();
}

// ───────────────────────────── the remover ─────────────────────────────

/// The production remover: `workers` threads (see [`worker_count`]), progress and «Annulla».
/// Same guarantees as [`crate::DefaultRemover`], which delegates here.
#[derive(Clone, Copy, Debug)]
pub struct ParallelRemover {
    pub workers: usize,
}

impl Remover for ParallelRemover {
    fn execute(&self, plan: &CleanPlan, dry_run: bool) -> CleanResult {
        self.execute_with(plan, dry_run, &|_| {}, &Cancel::default())
    }
}

impl ParallelRemover {
    /// Executes `plan`; `tick` is called from one thread at most every 100 ms while the
    /// workers run; `cancel` stops before the next file.
    pub fn execute_with(&self, plan: &CleanPlan, dry_run: bool, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> CleanResult {
        self.execute_hooked(plan, dry_run, tick, cancel, &|| {})
    }

    /// [`Self::execute_with`]; `before_work` runs after validation and measurement, right
    /// before the workers start (tests use it to change the disk in between).
    pub(crate) fn execute_hooked(
        &self,
        plan: &CleanPlan,
        dry_run: bool,
        tick: &(dyn Fn(Tick) + Sync),
        cancel: &Cancel,
        before_work: &dyn Fn(),
    ) -> CleanResult {
        let start = Instant::now();
        let mut result = CleanResult { dry_run, ..Default::default() };
        let mut outcomes: Vec<Option<ItemOutcome>> = vec![None; plan.items.len()];
        let mut jobs: Vec<Prepared> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // 1. Validation and measurement, in sequence: nothing refused reaches a worker.
        for (idx, item) in plan.items.iter().enumerate() {
            if cancel.is_cancelled() {
                outcomes[idx] = Some(skipped(CANCELLED));
                result.cancelled = true;
                continue;
            }
            if !seen.insert(path_key(&item.path)) {
                outcomes[idx] = Some(skipped("duplicate of an earlier plan item"));
                continue;
            }
            match catch_unwind(AssertUnwindSafe(|| prepare(plan.mode, idx, item, dry_run))) {
                Ok(Ok(job)) => jobs.push(job),
                Ok(Err(outcome)) => outcomes[idx] = Some(outcome),
                Err(_) => outcomes[idx] = Some(internal_error()),
            }
        }

        let progress = Progress {
            total_bytes: jobs.iter().map(|j| j.bytes).sum(),
            total_items: jobs.len(),
            ..Default::default()
        };
        let ctx = Ctx { cancel, progress: &progress, any_cancelled: AtomicBool::new(false) };
        let states: Vec<ItemState> = jobs.iter().map(|_| ItemState::default()).collect();
        before_work();

        if !jobs.is_empty() {
            match pool(self.workers) {
                Err(e) => {
                    for j in &jobs {
                        outcomes[j.idx] = Some(failed(e.clone(), vec![]));
                    }
                }
                Ok(pool) => with_ticker(&progress, tick, || {
                    let ctx = &ctx;
                    // 2-4. Folders (task per folder), Recycle Bin and link items (task per item).
                    pool.scope(|s| {
                        for (job, st) in jobs.iter().zip(&states) {
                            match job.job {
                                Job::Tree => {
                                    st.pending.store(1, SeqCst);
                                    let dir = fsops::verbatim(&job.path);
                                    s.spawn(move |s| tree_task(s, ctx, job, st, dir, 0));
                                }
                                Job::Recycle | Job::Link => s.spawn(move |_| single_task(ctx, job, st)),
                                Job::Worktree(_) => {}
                            }
                        }
                    });
                    // 5. git worktree remove, in sequence.
                    for (job, st) in jobs.iter().zip(&states) {
                        if let Job::Worktree(repo) = &job.job {
                            worktree_job(ctx, job, repo, st);
                        }
                    }
                }),
            }
        }

        for (job, st) in jobs.iter().zip(&states) {
            result.files_removed += st.files.load(SeqCst);
            result.dirs_removed += st.dirs.load(SeqCst);
            let o = lock(&st.outcome).take();
            if let (Some(ItemOutcome::Removed { bytes_freed }), Job::Recycle) = (&o, &job.job) {
                result.recycled_bytes += bytes_freed;
            }
            if outcomes[job.idx].is_none() {
                outcomes[job.idx] = Some(o.unwrap_or_else(internal_error));
            }
        }
        for (item, outcome) in plan.items.iter().zip(outcomes) {
            let outcome = outcome.unwrap_or_else(internal_error);
            if let ItemOutcome::Removed { bytes_freed } = &outcome {
                result.bytes_freed += bytes_freed;
            }
            result.items.push(ItemResult { path: item.path.clone(), outcome });
        }
        result.cancelled |= ctx.any_cancelled.load(SeqCst);
        result.elapsed_ms = start.elapsed().as_millis() as u64;
        result
    }
}

// ───────────────────────────── loose files (Pulizia file) ─────────────────────────────

/// Removes one file (a link: only the link). `Ok(false)` = it was already gone.
fn remove_one_file(p: &Path, patient: bool) -> Result<bool, String> {
    if !p.is_absolute() {
        return Err("percorso non assoluto".into());
    }
    let v = fsops::verbatim(p);
    let r = match fs::symlink_metadata(&v) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => Err(e),
        Ok(m) if m.file_type().is_symlink() => fsops::remove_link_entry(&v, patient),
        Ok(m) if m.is_dir() => return Err("è una cartella: non viene eliminata".into()),
        Ok(_) => fsops::with_retry(&v, patient, || fs::remove_file(&v)),
    };
    r.map(|()| true).map_err(|e| {
        let locked_by =
            if matches!(e.raw_os_error(), Some(5 | 32 | 33)) { locks::locking_processes(p) } else { vec![] };
        if locked_by.is_empty() { e.to_string() } else { format!("{e} (in uso da: {})", locked_by.join(", ")) }
    })
}

/// Deletes loose files with `workers` threads: `(bytes, files removed, failed, not attempted)`,
/// the bytes being the sizes given with the files. A file already gone counts nowhere; «Annulla»
/// stops before the next file (the files left are neither removed nor failed: they are the
/// last number, so `0` means every file was tried even if «Annulla» came at the end). Folders are refused
/// and links are removed as links, never followed.
///
/// **No `guard` here**: the paths are not checked against the protected locations; the
/// caller must have validated every file (Pulizia file, Task 11) before calling this.
pub fn remove_files_parallel(
    files: &[(PathBuf, u64)],
    workers: usize,
    tick: &(dyn Fn(Tick) + Sync),
    cancel: &Cancel,
) -> (u64, u64, Vec<(PathBuf, String)>, u64) {
    let pool = match pool(workers) {
        Ok(p) => p,
        Err(e) => return (0, 0, files.iter().map(|(p, _)| (p.clone(), e.clone())).collect(), 0),
    };
    let progress =
        Progress { total_bytes: files.iter().map(|(_, n)| n).sum(), total_items: files.len(), ..Default::default() };
    let (bytes, count, failures) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let untried = AtomicU64::new(0);
    let failed: Mutex<Vec<(usize, PathBuf, String)>> = Mutex::new(Vec::new());
    with_ticker(&progress, tick, || {
        pool.install(|| {
            files.par_iter().enumerate().for_each(|(i, (p, len))| {
                if cancel.is_cancelled() {
                    untried.fetch_add(1, SeqCst);
                    return;
                }
                let patient = failures.load(SeqCst) == 0;
                let r = catch_unwind(AssertUnwindSafe(|| remove_one_file(p, patient)))
                    .unwrap_or_else(|_| Err("errore interno".into()));
                match r {
                    Ok(true) => {
                        bytes.fetch_add(*len, SeqCst);
                        count.fetch_add(1, SeqCst);
                    }
                    Ok(false) => {}
                    Err(e) => {
                        failures.fetch_add(1, SeqCst);
                        lock(&failed).push((i, p.clone(), e));
                    }
                }
                progress.done_bytes.fetch_add(*len, SeqCst);
                progress.done_items.fetch_add(1, SeqCst);
            });
        })
    });
    let mut failed = failed.into_inner().unwrap_or_else(|e| e.into_inner());
    failed.sort_by_key(|(i, _, _)| *i);
    let failed = failed.into_iter().map(|(_, p, e)| (p, e)).collect();
    (bytes.into_inner(), count.into_inner(), failed, untried.into_inner())
}
