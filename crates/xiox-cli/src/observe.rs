//! `xiox observe`: a long-running, read-only process for ONE project. It takes a snapshot, re-checks
//! at intervals and tells its caller what appeared since it started, one JSON object per stdout line.
//!
//! stdout carries only those lines; everything meant for a person goes to stderr. The loop waits on
//! a channel fed by a stdin-reader thread, so it sleeps between checks and ends when stdin closes or
//! a `stop` line arrives.

use crate::run::{EXIT_ERROR, scan_project, write_result};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use xiox_observer::{Event, Measured, Observer, Params, measure_bytes};
use xiox_project::{Category, Options, ProjectScan};

/// Folders are measured with these caps, for the baseline and for every later check alike, so that
/// growth is never the difference between two different measurers.
pub const MEASURE_MAX_ENTRIES: usize = 200_000;
pub const MEASURE_MAX_TIME: Duration = Duration::from_secs(10);
/// The longest one check spends measuring folders (the baseline is not bounded by it): once it
/// is over, the folders left are measured at the next check, the oldest measure first.
pub const MEASURE_BUDGET: Duration = Duration::from_secs(30);

/// Longest piece of a stdin line quoted back in an `error` event.
const ECHO_MAX_CHARS: usize = 80;

pub struct ObserveArgs {
    pub project: Option<PathBuf>,
    pub interval_secs: u64,
    pub grow_threshold: u64,
    pub worktree_threshold: usize,
    pub no_drives: bool,
    pub mcp: bool,
}

impl ObserveArgs {
    pub fn params(&self) -> Params {
        Params {
            grow_threshold: self.grow_threshold,
            worktree_threshold: self.worktree_threshold,
        }
    }
}

/// What the loop needs from the outside, so that tests can fake the disk and the clock.
pub struct Deps {
    /// A light analysis of the project (no weights).
    pub analyze: Box<dyn Fn() -> Result<ProjectScan, String> + Send + Sync>,
    /// The size of one folder, bounded (a lower bound when it hit a cap); `None` when it cannot
    /// be told.
    pub measure: Box<dyn Fn(&Path) -> Option<Measured> + Send + Sync>,
    /// The longest one check spends measuring folders.
    pub measure_budget: Duration,
    /// How many linked worktrees git lists for a project root; `None` when git fails. Asked only
    /// when a check lists none while the observer knew some, to tell a failure from a real removal.
    pub linked_worktrees: Box<dyn Fn(&Path) -> Option<usize> + Send + Sync>,
    pub now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    pub interval: Duration,
    /// How long one MCP `tools/call` waits for the baseline before telling the agent to ask again.
    pub baseline_wait: Duration,
}

/// The default of `Deps::baseline_wait`: well under the shortest client tool timeout, so that the
/// "still running, call again" answer reaches the client.
pub const BASELINE_WAIT: Duration = Duration::from_secs(30);

/// The options of every analysis of the observer, the baseline and each check alike. Never `admin`:
/// from an elevated terminal the search would read the whole MFT of every NTFS drive at each check
/// (hundreds of MB, seconds), which a process that checks at intervals, next to others, must not do.
pub fn observe_options(cmd: &ObserveArgs) -> Options {
    Options {
        drives: !cmd.no_drives,
        measure: false,
        admin: false,
    }
}

impl Deps {
    /// The real PC (or the test switches' fake world in debug builds). In debug builds
    /// `XIOX_OBSERVE_INTERVAL_MS` replaces the interval and bypasses its 60-second minimum.
    pub fn real(cmd: &ObserveArgs) -> Self {
        let path = cmd.project.clone().unwrap_or_else(|| PathBuf::from("."));
        let opts = observe_options(cmd);
        let mut interval = Duration::from_secs(cmd.interval_secs);
        if cfg!(debug_assertions) {
            if let Some(ms) = std::env::var("XIOX_OBSERVE_INTERVAL_MS")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
            {
                interval = Duration::from_millis(ms.max(1));
            }
        }
        Deps {
            analyze: Box::new(move || scan_project(&path, &opts).map_err(|e| e.to_string())),
            measure: Box::new(|p| measure_bytes(p, MEASURE_MAX_ENTRIES, MEASURE_MAX_TIME)),
            measure_budget: MEASURE_BUDGET,
            linked_worktrees: Box::new(xiox_project::linked_worktree_count),
            now_ms: Box::new(|| chrono::Utc::now().timestamp_millis()),
            interval,
            baseline_wait: BASELINE_WAIT,
        }
    }
}

/// The initial snapshot: a light analysis, then the weights of everything that has a folder and no
/// weight yet and of the build folders, then the baseline. Shared by both modes.
pub fn start(deps: &Deps, params: Params) -> Result<Observer, String> {
    let mut scan = (deps.analyze)()?;
    let mut partial = HashSet::new();
    for item in &mut scan.items {
        if let Some(path) = &item.path {
            if item.bytes.is_none() || item.category == Category::Builds {
                item.bytes = match (deps.measure)(path) {
                    Some(m) => {
                        if !m.complete {
                            partial.insert(item.key.clone());
                        }
                        Some(m.bytes)
                    }
                    None => None,
                };
            }
        }
    }
    Ok(Observer::new_measured(
        scan,
        params,
        (deps.now_ms)(),
        &partial,
    ))
}

/// Why a check's analysis cannot be trusted, when it cannot: git failed in a way the analysis turns
/// into "no worktrees" or "not a repository" instead of an error (a timeout, say), and feeding that
/// to the observer would report every worktree as removed and then as new again.
fn degraded(observer: &Observer, scan: &ProjectScan, deps: &Deps) -> Option<String> {
    let before = observer.project();
    let now = &scan.project;
    if now.root != before.root {
        return Some(format!(
            "the project root is now {} instead of {}",
            now.root.display(),
            before.root.display()
        ));
    }
    if now.is_repo != before.is_repo {
        return Some(format!(
            "git no longer gives the same answer for the project (repository: was {}, is {})",
            before.is_repo, now.is_repo
        ));
    }
    if now.worktrees.is_empty() && !before.worktrees.is_empty() {
        return match (deps.linked_worktrees)(&now.root) {
            Some(0) => None, // really all removed
            Some(_) => Some("git lists linked worktrees that the analysis did not see".to_string()),
            None => Some("git could not list the worktrees".to_string()),
        };
    }
    None
}

/// One check: a light analysis, the weights of `observer.keys_to_measure()` and of the folders it
/// has never seen (so that a new folder has its weight from its first `changed`), then the
/// observer's events. Shared by both modes. Measuring is bounded by `deps.measure_budget`: what is
/// left when it is over waits for the next check (the folders never measured first, then the
/// oldest measure). An analysis that looks degraded (see [`degraded`]) is an error, not a change.
pub fn check(observer: &mut Observer, deps: &Deps) -> Result<Vec<Event>, String> {
    let mut scan = (deps.analyze)()?;
    if let Some(reason) = degraded(observer, &scan, deps) {
        return Err(format!(
            "the analysis looks incomplete ({reason}); the last good report is kept"
        ));
    }
    let started = (deps.now_ms)();
    let known: HashSet<String> = observer
        .report(false)
        .items
        .into_iter()
        .map(|v| v.item.key)
        .collect();
    // Folders never seen first, then what the observer wants measured again, oldest first.
    let mut order: Vec<String> = Vec::new();
    let mut queued: HashSet<String> = HashSet::new();
    for item in &scan.items {
        if !known.contains(&item.key) && queued.insert(item.key.clone()) {
            order.push(item.key.clone());
        }
    }
    for key in observer.keys_to_measure(started) {
        if queued.insert(key.clone()) {
            order.push(key);
        }
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, item) in scan.items.iter().enumerate() {
        index.entry(item.key.clone()).or_insert(i);
    }
    let budget_ms = i64::try_from(deps.measure_budget.as_millis()).unwrap_or(i64::MAX);
    let mut partial: HashSet<String> = HashSet::new();
    for key in order {
        let Some(&i) = index.get(&key) else { continue };
        let item = &mut scan.items[i];
        let Some(path) = &item.path else { continue };
        if (deps.now_ms)().saturating_sub(started) >= budget_ms {
            break;
        }
        item.bytes = match (deps.measure)(path) {
            Some(m) => {
                if !m.complete {
                    partial.insert(key);
                }
                Some(m.bytes)
            }
            None => None,
        };
    }
    Ok(observer.update_measured(scan, (deps.now_ms)(), &partial))
}

/// How long the panic hook waits for its message to be written before the panicking thread goes on.
const PANIC_REPORT_WAIT: Duration = Duration::from_millis(250);

/// The name of the thread that takes the baseline of the MCP server: a panic there does not end
/// the server, the next `tools/call` takes the baseline again.
pub(crate) const MCP_BASELINE_THREAD: &str = "xiox-mcp-baseline";

/// What the panic hook writes on stderr for a panic of `thread` (`info` is the hook's own text).
pub(crate) fn panic_report(thread: Option<&str>, info: &str) -> String {
    if thread == Some(MCP_BASELINE_THREAD) {
        format!(
            "error: the first analysis stopped on an internal error; the next call retries it: {info}\n"
        )
    } else {
        format!("error: xiox observe stopped on an internal error: {info}\n")
    }
}

/// A panic is told on stderr only (the default hook does the same, this one says what it is) and
/// the process ends with a non-zero code. The hook must never block its thread: it runs before the
/// unwinding, so a thread stuck in it never drops what it owns (a channel's sender, say) and the
/// threads waiting on that thread wait for ever. The message is therefore written by a helper
/// thread, and the hook gives up after `PANIC_REPORT_WAIT` if somebody else holds the stderr lock.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let message = panic_report(thread.name(), &info.to_string());
        let (done_tx, done_rx) = mpsc::channel();
        let helper = {
            let message = message.clone();
            std::thread::Builder::new()
                .name("xiox-panic-report".into())
                .spawn(move || {
                    let _ = std::io::stderr().write_all(message.as_bytes());
                    let _ = done_tx.send(());
                })
        };
        match helper {
            Ok(_) => {
                let _ = done_rx.recv_timeout(PANIC_REPORT_WAIT);
            }
            // No thread to spare: the message matters more than the risk of a held lock.
            Err(_) => {
                let _ = std::io::stderr().write_all(message.as_bytes());
            }
        }
    }));
}

/// The command of the real PC: real stdin, the real clock.
pub fn run_observe(cmd: ObserveArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    install_panic_hook();
    let deps = Deps::real(&cmd);
    if cmd.mcp {
        return crate::mcp::serve(
            cmd.params(),
            std::io::stdin().lock(),
            out,
            err,
            std::sync::Arc::new(deps),
        );
    }
    observe(
        cmd,
        std::io::BufReader::new(std::io::stdin()),
        out,
        err,
        &deps,
    )
}

/// An output that remembers a closed pipe: `write_result` treats it as success (the person just ran
/// `| head`), but for a long-running process it means the caller is gone.
pub(crate) struct Tap<'a> {
    inner: &'a mut dyn Write,
    pub(crate) pipe_closed: bool,
}

impl<'a> Tap<'a> {
    pub(crate) fn new(inner: &'a mut dyn Write) -> Self {
        Tap {
            inner,
            pipe_closed: false,
        }
    }
}

impl Write for Tap<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner
            .write(buf)
            .inspect_err(|e| self.pipe_closed |= e.kind() == std::io::ErrorKind::BrokenPipe)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner
            .flush()
            .inspect_err(|e| self.pipe_closed |= e.kind() == std::io::ErrorKind::BrokenPipe)
    }
}

/// The longest stdin line either mode accepts (without its line end). A longer one is dropped
/// (never kept in memory) and reported, so a caller cannot grow the process without bound.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// One read of a capped line.
pub(crate) enum LineRead {
    /// End of input, nothing more to read.
    Eof,
    /// A line, without its `\n` (a `\r` before it is still there).
    Line(Vec<u8>),
    /// The line was longer than the cap: it has been read to its end and thrown away.
    TooLong,
}

/// Reads one line of at most `max` bytes. The excess of a longer line is consumed and dropped as it
/// arrives, so memory stays at `max` bytes whatever the caller sends.
pub(crate) fn read_capped_line(r: &mut impl BufRead, max: usize) -> std::io::Result<LineRead> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut any = false;
    loop {
        let chunk = match r.fill_buf() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if chunk.is_empty() {
            break;
        }
        any = true;
        let end = chunk.iter().position(|&b| b == b'\n');
        let piece = &chunk[..end.unwrap_or(chunk.len())];
        if !too_long {
            if line.len() + piece.len() > max {
                too_long = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(piece);
            }
        }
        let used = piece.len() + usize::from(end.is_some());
        r.consume(used);
        if end.is_some() {
            break;
        }
    }
    Ok(match (any, too_long) {
        (false, _) => LineRead::Eof,
        (true, true) => LineRead::TooLong,
        (true, false) => LineRead::Line(line),
    })
}

/// How a thread is started: replaceable so that a failed spawn can be tested.
pub(crate) type Spawn = fn(Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()>;

fn spawn_named(job: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("xiox-observe-stdin".into())
        .spawn(job)
        .map(|_| ())
}

/// `Instant::now() + interval` that cannot panic: an interval too large for an `Instant` waits
/// for the longest accepted one instead.
fn deadline_after(interval: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(interval)
        .or_else(|| now.checked_add(Duration::from_secs(crate::args::MAX_INTERVAL_SECS)))
        .unwrap_or(now)
}

/// How many stdin lines wait for the loop at most: the reader thread (and, behind it, the pipe of
/// a caller that writes faster than the observer answers) waits when the queue is full.
const INPUT_QUEUE: usize = 16;

/// What the stdin-reader thread tells the loop.
enum Input {
    Line(String),
    /// A line over `MAX_LINE_BYTES`, already dropped.
    TooLong,
    Closed,
    /// Reading stdin failed.
    Failed(String),
}

/// Starts the reader thread. `Err` is the sentence for the person when the thread cannot start.
fn spawn_reader(
    mut stdin: impl BufRead + Send + 'static,
    spawn: Spawn,
) -> Result<mpsc::Receiver<Input>, String> {
    let (tx, rx) = mpsc::sync_channel(INPUT_QUEUE);
    let job = Box::new(move || {
        loop {
            let message = match read_capped_line(&mut stdin, MAX_LINE_BYTES) {
                Ok(LineRead::Eof) => Input::Closed,
                Ok(LineRead::Line(bytes)) => {
                    Input::Line(String::from_utf8_lossy(&bytes).trim().to_string())
                }
                Ok(LineRead::TooLong) => Input::TooLong,
                Err(e) => Input::Failed(e.to_string()),
            };
            let last = matches!(message, Input::Closed | Input::Failed(_));
            if tx.send(message).is_err() || last {
                return;
            }
        }
    });
    spawn(job).map_err(|e| format!("cannot start the stdin reader thread: {e}"))?;
    Ok(rx)
}

/// After a `report` line: the `report` lines that already wait are folded into it (one check
/// answers them all; each still gets its `report` event). Returns how many were folded and the
/// first waiting message that is not a `report`, which the loop handles next.
fn fold_queued_reports(input: &mpsc::Receiver<Input>) -> (usize, Option<Input>) {
    let mut folded = 0;
    loop {
        match input.try_recv() {
            Ok(Input::Line(line)) if line == "report" => folded += 1,
            Ok(other) => return (folded, Some(other)),
            Err(_) => return (folded, None),
        }
    }
}

/// A JSON value as one line with its line end. serde_json leaves U+2028 and U+2029 raw inside
/// strings; some line splitters (Python `splitlines()`) break lines on them, so they are written
/// as escapes: the line parses back to the same value.
pub(crate) fn json_line(value: &Value) -> String {
    let mut line = value
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    line.push('\n');
    line
}

/// `{"schema":1,"event":"<kebab>","time":"<RFC 3339 UTC>", …event fields}` as one line. A `report`
/// carries its own `schema`: it is not added twice.
fn event_line(event: &Event, time_ms: i64) -> Result<String, String> {
    let value =
        serde_json::to_value(event).map_err(|e| format!("cannot write an event as JSON: {e}"))?;
    let Value::Object(fields) = value else {
        return Err("cannot write an event as JSON: not an object".to_string());
    };
    let time = chrono::DateTime::from_timestamp_millis(time_ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string());
    let mut line = Map::new();
    line.insert("schema".into(), Value::from(xiox_project::SCHEMA));
    for (key, value) in fields {
        if key == "event" {
            line.insert(key, value);
            line.insert("time".into(), Value::String(time.clone()));
        } else if key != "schema" {
            line.insert(key, value);
        }
    }
    Ok(json_line(&Value::Object(line)))
}

fn echo(line: &str) -> String {
    let mut shown: String = line.chars().take(ECHO_MAX_CHARS).collect();
    if line.chars().count() > ECHO_MAX_CHARS {
        shown.push_str("...");
    }
    shown
}

/// How the loop writes: one JSON line at a time through `write_result`, and the way out when it
/// cannot (`Err(code)`: the caller left, 0, or the write failed, 1).
struct Emitter<'a, 'b> {
    out: Tap<'a>,
    err: &'b mut dyn Write,
    now_ms: &'b dyn Fn() -> i64,
}

impl Emitter<'_, '_> {
    fn emit(&mut self, event: &Event) -> Result<(), i32> {
        let line = match event_line(event, (self.now_ms)()) {
            Ok(l) => l,
            Err(message) => {
                // The event cannot be written (a path JSON cannot carry): say so instead.
                let _ = writeln!(self.err, "error: {message}");
                match event_line(&Event::Error { message }, (self.now_ms)()) {
                    Ok(l) => l,
                    Err(_) => return Ok(()),
                }
            }
        };
        self.write(&line)
    }

    fn write(&mut self, line: &str) -> Result<(), i32> {
        write_result(&mut self.out, self.err, line)?;
        if self.out.pipe_closed { Err(0) } else { Ok(()) }
    }

    /// An `error` event and the same words for the person.
    fn error(&mut self, message: String) -> Result<(), i32> {
        let _ = writeln!(self.err, "error: {message}");
        self.emit(&Event::Error { message })
    }
}

/// The loop. Returns the exit code: 0 on `stop`, end of input or a closed pipe; 1 when the first
/// analysis fails or stdout cannot be written.
pub fn observe(
    cmd: ObserveArgs,
    stdin: impl BufRead + Send + 'static,
    out: &mut dyn Write,
    err: &mut dyn Write,
    deps: &Deps,
) -> i32 {
    observe_with(cmd, stdin, out, err, deps, spawn_named)
}

pub(crate) fn observe_with(
    cmd: ObserveArgs,
    stdin: impl BufRead + Send + 'static,
    out: &mut dyn Write,
    err: &mut dyn Write,
    deps: &Deps,
    spawn: Spawn,
) -> i32 {
    let mut observer = match start(deps, cmd.params()) {
        Ok(o) => o,
        Err(message) => {
            let _ = writeln!(err, "error: {message}");
            return EXIT_ERROR;
        }
    };
    // Before the first line: if the reader cannot start there is nothing on stdout at all.
    let input = match spawn_reader(stdin, spawn) {
        Ok(rx) => rx,
        Err(message) => {
            let _ = writeln!(err, "error: {message}");
            return EXIT_ERROR;
        }
    };
    // `started` is the first line; if it cannot be written there is nothing to observe for.
    let started = observer.started();
    let label = match &started {
        Event::Started { project, .. } => project.root.display().to_string(),
        _ => String::new(),
    };
    let first = match event_line(&started, (deps.now_ms)()) {
        Ok(l) => l,
        Err(message) => {
            let _ = writeln!(err, "error: {message}");
            return EXIT_ERROR;
        }
    };
    let mut emitter = Emitter {
        out: Tap::new(out),
        err,
        now_ms: &*deps.now_ms,
    };
    if let Err(code) = emitter.write(&first) {
        return code;
    }
    let _ = writeln!(
        emitter.err,
        "Observing {} every {}s (stdin: report, stop). Nothing is removed.",
        label,
        deps.interval.as_secs_f64().round()
    );

    let mut deadline = deadline_after(deps.interval);
    // After a failed check the report is the last good one, marked stale.
    let mut last_check_failed = false;
    // The message that followed folded `report` lines: handled before waiting for more.
    let mut pending: Option<Input> = None;
    loop {
        let message = match pending.take() {
            Some(m) => Some(m),
            None => {
                let wait = deadline.saturating_duration_since(Instant::now());
                match input.recv_timeout(wait) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => {
                        // The reader always says `Closed` or `Failed` before it ends: it died.
                        let _ =
                            writeln!(emitter.err, "error: the stdin reader stopped unexpectedly");
                        return EXIT_ERROR;
                    }
                }
            }
        };
        let checked = match &message {
            None => true,
            Some(Input::Line(line)) => line == "report",
            Some(Input::Closed | Input::Failed(_) | Input::TooLong) => false,
        };
        let result = match message {
            None => run_check(&mut observer, deps, &mut emitter, &mut last_check_failed),
            Some(Input::Closed) => return 0,
            Some(Input::Failed(message)) => {
                let _ = writeln!(emitter.err, "error: cannot read stdin: {message}");
                return EXIT_ERROR;
            }
            Some(Input::TooLong) => emitter.error(format!(
                "stdin line too long (over {MAX_LINE_BYTES} bytes): dropped"
            )),
            Some(Input::Line(line)) => match line.as_str() {
                "" => continue,
                "stop" => return 0,
                "report" => {
                    let (folded, next) = fold_queued_reports(&input);
                    pending = next;
                    run_check(&mut observer, deps, &mut emitter, &mut last_check_failed).and_then(
                        |()| {
                            for _ in 0..=folded {
                                emitter.emit(&Event::Report(observer.report(last_check_failed)))?;
                            }
                            Ok(())
                        },
                    )
                }
                other => emitter.error(format!(
                    "unknown command: {} (expected report or stop)",
                    echo(other)
                )),
            },
        };
        if let Err(code) = result {
            return code;
        }
        if checked {
            deadline = deadline_after(deps.interval);
        }
    }
}

/// One check, its events written (or the `error` event when it fails).
fn run_check(
    observer: &mut Observer,
    deps: &Deps,
    emitter: &mut Emitter,
    failed: &mut bool,
) -> Result<(), i32> {
    match check(observer, deps) {
        Ok(events) => {
            *failed = false;
            for event in &events {
                emitter.emit(event)?;
            }
            Ok(())
        }
        Err(message) => {
            *failed = true;
            emitter.error(format!("check failed: {message}"))
        }
    }
}

/// Fake worlds shared by the tests of both modes.
#[cfg(test)]
pub(crate) mod fakes {
    use super::Deps;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::time::Duration;
    use xiox_model::{Agent, ArtifactKind, Level};
    use xiox_observer::Measured;
    use xiox_project::{Category, Item, Project, ProjectScan, SCHEMA, SearchMode, Target, Why};

    pub(crate) const MB: u64 = 1024 * 1024;

    pub(crate) fn project() -> Project {
        Project {
            root: PathBuf::from(r"D:\Projects\mukade"),
            name: "mukade".into(),
            is_repo: true,
            origin: None,
            worktrees: vec![],
        }
    }

    pub(crate) fn scan(items: Vec<Item>) -> ProjectScan {
        ProjectScan {
            schema: SCHEMA,
            project: project(),
            items,
            branches: vec![],
            notes: vec![],
            search: SearchMode::BoundedWalk,
            scanned_at_ms: 0,
            elapsed_ms: 0,
        }
    }

    pub(crate) fn build() -> Item {
        let p = PathBuf::from(r"D:\Projects\mukade\target");
        Item {
            key: xiox_model::path_key(&p),
            category: Category::Builds,
            path: Some(p),
            bytes: None,
            level: Level::Recommended,
            blocked: vec![],
            why: Why::CatalogFolder {
                label_en: "Rust build".into(),
            },
            target: Target::Dir {
                kind: ArtifactKind::Catalog,
            },
        }
    }

    /// An archived agent session: no folder, no size.
    pub(crate) fn session(n: usize, title: &str) -> Item {
        let id = format!("s{n}");
        Item {
            key: format!("session:claude:{id}"),
            category: Category::Sessions,
            path: None,
            bytes: None,
            level: Level::Optional,
            blocked: vec![],
            why: Why::ArchivedSession {
                agent: Agent::Claude,
                title: Some(title.to_string()),
            },
            target: Target::Session {
                agent: Agent::Claude,
                id,
            },
        }
    }

    pub(crate) fn worktree(name: &str) -> Item {
        let p = PathBuf::from(format!(r"D:\Projects\{name}"));
        Item {
            key: xiox_model::path_key(&p),
            category: Category::Worktrees,
            path: Some(p),
            bytes: None,
            level: Level::Optional,
            blocked: vec![],
            why: Why::LinkedWorktree {
                branch: None,
                head: None,
            },
            target: Target::Worktree {
                repo: PathBuf::from(r"D:\Projects\mukade"),
            },
        }
    }

    /// Fake dependencies: `analyze` returns the script's results in order and then repeats the
    /// last one; `hook(n)` runs at the start of the `n`th call (0 = the initial analysis).
    /// Every measured folder is 10 MB. The clock advances one second per call.
    pub(crate) fn deps(
        script: Vec<Result<ProjectScan, String>>,
        hook: impl Fn(usize) + Send + Sync + 'static,
    ) -> Deps {
        let calls = AtomicUsize::new(0);
        let clock = Arc::new(AtomicI64::new(1_700_000_000_000i64));
        Deps {
            analyze: Box::new(move || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                hook(n);
                script[n.min(script.len() - 1)].clone()
            }),
            measure: Box::new(|_| {
                Some(Measured {
                    bytes: 10 * MB,
                    complete: true,
                })
            }),
            // Far over what the fake clock (one second per reading) can reach in a test.
            measure_budget: Duration::from_secs(1_000_000),
            linked_worktrees: Box::new(|_| Some(0)),
            now_ms: Box::new(move || clock.fetch_add(1000, Ordering::SeqCst) + 1000),
            interval: Duration::from_millis(15),
            baseline_wait: Duration::from_secs(5),
        }
    }

    /// Serializes the tests that replace the process-wide panic hook, and puts the previous hook
    /// back when dropped.
    pub(crate) struct PanicHook {
        previous: Option<Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>>,
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    static PANIC_HOOK_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Installs the project's own panic hook (`install_panic_hook`) for one test.
    pub(crate) fn project_panic_hook() -> PanicHook {
        let serial = PANIC_HOOK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::panic::take_hook();
        super::install_panic_hook();
        PanicHook {
            previous: Some(previous),
            _serial: serial,
        }
    }

    impl Drop for PanicHook {
        fn drop(&mut self) {
            // `set_hook` aborts the process when called from a panicking thread (a failed assert).
            if let Some(previous) = self.previous.take() {
                if !std::thread::panicking() {
                    std::panic::set_hook(previous);
                }
            }
        }
    }

    pub(crate) fn with_interval(mut d: Deps, interval: Duration) -> Deps {
        d.interval = interval;
        d
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use serde_json::Value;
    use std::io::{Cursor, Read};
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use xiox_project::Item;

    fn args() -> ObserveArgs {
        ObserveArgs {
            project: None,
            interval_secs: 600,
            grow_threshold: 2 * 1024 * MB,
            worktree_threshold: 3,
            no_drives: true,
            mcp: false,
        }
    }

    /// A stdin that blocks until the test sends bytes; closing the sender is end of input.
    struct Gated {
        rx: mpsc::Receiver<Vec<u8>>,
        pending: Vec<u8>,
    }

    impl Read for Gated {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pending.is_empty() {
                match self.rx.recv() {
                    Ok(bytes) => self.pending = bytes,
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.pending.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            Ok(n)
        }
    }

    fn gated() -> (mpsc::Sender<Vec<u8>>, std::io::BufReader<Gated>) {
        let (tx, rx) = mpsc::channel();
        (
            tx,
            std::io::BufReader::new(Gated {
                rx,
                pending: Vec::new(),
            }),
        )
    }

    struct Run {
        code: i32,
        stdout: String,
        stderr: String,
    }

    impl Run {
        fn lines(&self) -> Vec<Value> {
            self.stdout
                .lines()
                .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {l:?}: {e}")))
                .collect()
        }

        fn events(&self) -> Vec<String> {
            self.lines()
                .iter()
                .map(|v| v["event"].as_str().unwrap().to_string())
                .collect()
        }
    }

    fn run_with(stdin: impl BufRead + Send + 'static, d: &Deps) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = observe(args(), stdin, &mut out, &mut err, d);
        Run {
            code,
            stdout: String::from_utf8(out).unwrap(),
            stderr: String::from_utf8(err).unwrap(),
        }
    }

    fn run_text(input: &str, script: Vec<Result<ProjectScan, String>>) -> Run {
        // A long interval: only stdin drives the loop.
        let d = with_interval(deps(script, |_| {}), Duration::from_secs(3600));
        run_with(Cursor::new(input.as_bytes().to_vec()), &d)
    }

    /// Runs with ticks every 15 ms; `send_at(n)` bytes are sent to stdin when the `n`th analysis starts.
    fn run_ticking(script: Vec<Result<ProjectScan, String>>, n: usize, send: &'static str) -> Run {
        let (tx, stdin) = gated();
        let d = deps(script, move |call| {
            if call == n {
                tx.send(send.as_bytes().to_vec()).unwrap();
            }
        });
        run_with(stdin, &d)
    }

    #[test]
    fn started_is_the_first_line_and_all_lines_are_valid_json_with_schema_time_event() {
        let r = run_ticking(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("mukade-wt")])),
            ],
            2,
            "report\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        let lines = r.lines();
        assert!(lines.len() >= 3, "{lines:?}");
        assert_eq!(lines[0]["event"], "started");
        for v in &lines {
            assert!(v.is_object(), "{v}");
            assert_eq!(v["schema"], 1, "{v}");
            let time = v["time"]
                .as_str()
                .unwrap_or_else(|| panic!("no time in {v}"));
            assert!(time.ends_with('Z'), "UTC: {time}");
            chrono::DateTime::parse_from_rfc3339(time).unwrap_or_else(|e| panic!("{time}: {e}"));
            let event = v["event"].as_str().unwrap();
            assert!(
                ["started", "changed", "attention", "report", "error"].contains(&event),
                "{event}"
            );
        }
        // Started carries the project, the parameters and the totals.
        assert_eq!(lines[0]["project"]["name"], "mukade");
        assert_eq!(lines[0]["params"]["worktreeThreshold"], 3);
        assert!(lines[0]["totals"].is_object());
    }

    #[test]
    fn line_separators_are_escaped_on_every_json_line() {
        let mut wt = worktree("mukade-wt");
        wt.why = xiox_project::Why::LinkedWorktree {
            branch: Some("a\u{2028}b\u{2029}c".into()),
            head: None,
        };
        let r = run_ticking(
            vec![Ok(scan(vec![build()])), Ok(scan(vec![build(), wt]))],
            2,
            "report\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert!(
            !r.stdout.contains('\u{2028}') && !r.stdout.contains('\u{2029}'),
            "{}",
            r.stdout
        );
        assert!(r.stdout.contains("a\\u2028b\\u2029c"), "{}", r.stdout);
        // Every line still parses, and the string comes back intact.
        assert!(r.stdout.lines().any(|l| {
            serde_json::from_str::<Value>(l)
                .unwrap()
                .to_string()
                .contains("a\u{2028}b\u{2029}c")
        }));
    }

    #[test]
    fn a_panicking_stdin_reader_ends_the_loop_even_when_another_thread_holds_the_stderr_lock() {
        // The real binary used to hold `stderr().lock()` for its whole life: the panic hook of the
        // reader thread then waited for that lock before unwinding, the channel never closed and
        // the loop waited for ever.
        let _hook = project_panic_hook();
        let _stderr = std::io::stderr().lock();
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        let (tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            let r = run_with(std::io::BufReader::new(BadStdin { panic: true }), &d);
            let _ = tx.send((r.code, r.stderr));
        });
        let (code, stderr) = done
            .recv_timeout(Duration::from_secs(10))
            .expect("the loop must end, not wait for the dead reader for ever");
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("stdin reader stopped"), "{stderr}");
    }

    #[test]
    fn the_baseline_has_weights_before_started() {
        // The first analysis is light (no bytes); the baseline measures the build folder first.
        let r = run_text("stop\n", vec![Ok(scan(vec![build()]))]);
        assert_eq!(r.code, 0);
        let started = &r.lines()[0];
        assert_eq!(started["totals"]["byCategory"]["builds"], 10 * MB);
    }

    #[test]
    fn nothing_changes_gives_no_changed_event() {
        let r = run_ticking(vec![Ok(scan(vec![build()]))], 3, "stop\n");
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.events(), ["started"], "{}", r.stdout);
    }

    #[test]
    fn a_new_worktree_appears_in_a_changed_event_and_a_report_marks_it_new() {
        let r = run_ticking(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("mukade-wt")])),
            ],
            2,
            "report\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        let lines = r.lines();
        let changed: Vec<&Value> = lines.iter().filter(|v| v["event"] == "changed").collect();
        assert_eq!(changed.len(), 1, "{}", r.stdout);
        let added = changed[0]["added"].as_array().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["category"], "worktrees");
        assert_eq!(added[0]["new"], true);
        assert!(added[0]["firstSeenMs"].is_i64());
        // The new item is weighed when it is first seen.
        assert_eq!(added[0]["bytes"], 10 * MB);
        assert_eq!(changed[0]["removed"].as_array().unwrap().len(), 0);

        let report = lines
            .iter()
            .find(|v| v["event"] == "report")
            .expect("a report line");
        assert_eq!(report["stale"], false);
        let items = report["items"].as_array().unwrap();
        let by_cat = |c: &str| items.iter().find(|i| i["category"] == c).unwrap();
        assert_eq!(by_cat("worktrees")["new"], true);
        assert_eq!(by_cat("builds")["new"], false);
        // `schema` is written once on a report line (the report carries its own).
        let line = r
            .stdout
            .lines()
            .find(|l| l.contains("\"event\":\"report\""))
            .unwrap();
        assert_eq!(line.matches("\"schema\"").count(), 1, "{line}");
        // Changed comes before the report.
        let events = r.events();
        let (c, p) = (
            events.iter().position(|e| e == "changed"),
            events.iter().position(|e| e == "report"),
        );
        assert!(c < p, "{events:?}");
    }

    #[test]
    fn report_command_on_stdin_emits_a_report_event() {
        let r = run_text("report\nstop\n", vec![Ok(scan(vec![build()]))]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.events(), ["started", "report"], "{}", r.stdout);
        let report = &r.lines()[1];
        assert_eq!(report["stale"], false);
        assert_eq!(report["project"]["name"], "mukade");
        assert_eq!(report["items"][0]["category"], "builds");
        assert_eq!(report["items"][0]["new"], false);
        assert_eq!(report["growthBytes"], 0);
        assert!(report["checkedAtMs"].is_i64());
        // One `schema` key on the line.
        let line = r.stdout.lines().nth(1).unwrap();
        assert_eq!(line.matches("\"schema\"").count(), 1, "{line}");
        // `report` with surrounding blanks and a Windows line end is the same command.
        let r = run_text("  report \r\nstop\r\n", vec![Ok(scan(vec![build()]))]);
        assert_eq!(r.events(), ["started", "report"], "{}", r.stdout);
    }

    #[test]
    fn stop_and_eof_exit_zero_with_no_extra_output() {
        for input in ["stop\n", "", "stop", "\n\n  \nstop\nreport\n"] {
            let r = run_text(input, vec![Ok(scan(vec![build()]))]);
            assert_eq!(r.code, 0, "{input:?}: {}", r.stderr);
            assert_eq!(r.events(), ["started"], "{input:?}: {}", r.stdout);
        }
    }

    #[test]
    fn failed_check_emits_error_and_keeps_going_and_report_is_stale() {
        // After the first analysis every check fails; the loop goes on and answers the report with
        // the last good result, marked stale.
        let r = run_ticking(
            vec![Ok(scan(vec![build()])), Err("disk went away".into())],
            3,
            "report\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        let lines = r.lines();
        let errors: Vec<&Value> = lines.iter().filter(|v| v["event"] == "error").collect();
        assert!(errors.len() >= 2, "{}", r.stdout);
        assert!(
            errors[0]["message"]
                .as_str()
                .unwrap()
                .contains("disk went away"),
            "{}",
            errors[0]
        );
        let last = lines.last().unwrap();
        assert_eq!(last["event"], "report", "{}", r.stdout);
        assert_eq!(last["stale"], true);
        assert_eq!(last["items"][0]["category"], "builds");
        // The error is told to the person too.
        assert!(r.stderr.contains("disk went away"), "{}", r.stderr);
    }

    #[test]
    fn first_analysis_failure_exits_1_with_empty_stdout() {
        let r = run_text("report\n", vec![Err("path not found: D:\\nope".into())]);
        assert_eq!(r.code, 1);
        assert!(r.stdout.is_empty(), "{}", r.stdout);
        assert!(r.stderr.contains("path not found"), "{}", r.stderr);
    }

    #[test]
    fn unknown_stdin_line_is_an_error_event() {
        let r = run_text(
            "frobnicate now\nreport\nstop\n",
            vec![Ok(scan(vec![build()]))],
        );
        assert_eq!(r.code, 0);
        assert_eq!(r.events(), ["started", "error", "report"], "{}", r.stdout);
        let message = r.lines()[1]["message"].as_str().unwrap().to_string();
        assert!(message.contains("unknown command"), "{message}");
        assert!(message.contains("frobnicate now"), "{message}");
        // A long garbage line is not echoed whole.
        let long = "x".repeat(5000);
        let r = run_text(&format!("{long}\nstop\n"), vec![Ok(scan(vec![build()]))]);
        let message = r.lines()[1]["message"].as_str().unwrap().to_string();
        assert!(message.len() < 300, "{}", message.len());
    }

    #[test]
    fn nothing_but_json_on_stdout() {
        let r = run_ticking(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("a")])),
                Err("boom".into()),
            ],
            4,
            "what\nreport\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert!(!r.stdout.is_empty());
        for line in r.stdout.lines() {
            let v: Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("stdout line {line:?}: {e}"));
            assert!(
                v.is_object()
                    && v["schema"] == 1
                    && v["event"].is_string()
                    && v["time"].is_string(),
                "{line}"
            );
        }
        assert!(r.stdout.ends_with('\n'));
        // The human text is on stderr.
        assert!(r.stderr.contains("Observing"), "{}", r.stderr);
        assert!(r.stderr.contains("boom"), "{}", r.stderr);
        // And no JSON line is repeated there.
        assert!(!r.stderr.contains("\"event\""), "{}", r.stderr);
    }

    /// An output that fails every write with `kind`.
    struct Failing(std::io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(self.0))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(self.0))
        }
    }

    #[test]
    fn a_closed_pipe_ends_the_loop_with_0_and_another_write_error_with_1() {
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        // Never ends by itself: the failing write must end it.
        let (_tx, stdin) = gated();
        let mut err = Vec::new();
        let code = observe(
            args(),
            stdin,
            &mut Failing(std::io::ErrorKind::BrokenPipe),
            &mut err,
            &d,
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));

        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        let (_tx, stdin) = gated();
        let mut err = Vec::new();
        let code = observe(
            args(),
            stdin,
            &mut Failing(std::io::ErrorKind::PermissionDenied),
            &mut err,
            &d,
        );
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&err).contains("cannot write to the output"));
    }

    #[test]
    fn check_measures_new_and_known_builds_and_returns_the_observers_events() {
        let d = deps(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("w")])),
            ],
            |_| {},
        );
        let mut observer = start(&d, Params::default()).unwrap();
        let events = check(&mut observer, &d).unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        match &events[0] {
            Event::Changed { added, .. } => {
                assert_eq!(added.len(), 1);
                assert_eq!(added[0].item.bytes, Some(10 * MB));
            }
            other => panic!("{other:?}"),
        }
        let d = deps(vec![Ok(scan(vec![build()])), Err("nope".into())], |_| {});
        let mut observer = start(&d, Params::default()).unwrap();
        assert_eq!(check(&mut observer, &d).unwrap_err(), "nope");
        let d = deps(vec![Err("first".into())], |_| {});
        assert_eq!(start(&d, Params::default()).err().as_deref(), Some("first"));
    }

    // ---- robustness (review of task 2) ----

    #[test]
    fn a_huge_interval_does_not_panic() {
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(u64::MAX),
        );
        let r = run_with(Cursor::new(b"report\nstop\n".to_vec()), &d);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.events(), ["started", "report"], "{}", r.stdout);
        // The helper itself never overflows either.
        let _ = deadline_after(Duration::MAX);
    }

    fn failing_spawn(_job: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
        Err(std::io::Error::other("no threads left"))
    }

    #[test]
    fn a_failed_reader_spawn_exits_1_with_one_error_line_and_nothing_on_stdout() {
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = observe_with(
            args(),
            Cursor::new(b"stop\n".to_vec()),
            &mut out,
            &mut err,
            &d,
            failing_spawn,
        );
        let err = String::from_utf8(err).unwrap();
        assert_eq!(code, 1, "{err}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        let errors: Vec<&str> = err.lines().filter(|l| l.starts_with("error:")).collect();
        assert_eq!(errors.len(), 1, "{err}");
        assert!(
            errors[0].contains("cannot start the stdin reader thread")
                && errors[0].contains("no threads left"),
            "{err}"
        );
    }

    /// A stdin whose reads panic (the reader thread dies without saying anything) or fail.
    struct BadStdin {
        panic: bool,
    }

    impl Read for BadStdin {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if self.panic {
                panic!("reader boom");
            }
            Err(std::io::Error::other("bad pipe"))
        }
    }

    #[test]
    fn a_dead_reader_thread_is_reported_on_stderr_and_ends_the_loop_with_1() {
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        let r = run_with(std::io::BufReader::new(BadStdin { panic: true }), &d);
        assert_eq!(r.code, 1, "{}", r.stderr);
        let errors: Vec<&str> = r
            .stderr
            .lines()
            .filter(|l| l.starts_with("error:"))
            .collect();
        assert_eq!(errors.len(), 1, "{}", r.stderr);
        assert!(errors[0].contains("stdin reader stopped"), "{}", r.stderr);
        assert_eq!(r.events(), ["started"], "{}", r.stdout);
    }

    #[test]
    fn a_stdin_read_error_is_reported_on_stderr_and_ends_the_loop_with_1() {
        let d = with_interval(
            deps(vec![Ok(scan(vec![build()]))], |_| {}),
            Duration::from_secs(3600),
        );
        let r = run_with(std::io::BufReader::new(BadStdin { panic: false }), &d);
        assert_eq!(r.code, 1, "{}", r.stderr);
        assert!(
            r.stderr.contains("error: cannot read stdin: bad pipe"),
            "{}",
            r.stderr
        );
    }

    #[test]
    fn read_capped_line_splits_lines_drops_the_excess_and_keeps_the_cap_exact() {
        let mut r = Cursor::new(b"abc\r\n\nxyzxyz\nfive5\nlast".to_vec());
        let mut got = Vec::new();
        loop {
            match read_capped_line(&mut r, 5).unwrap() {
                LineRead::Eof => break,
                LineRead::Line(b) => got.push(Some(String::from_utf8(b).unwrap())),
                LineRead::TooLong => got.push(None),
            }
        }
        // `abc\r` is 4 bytes, `xyzxyz` is 6 (too long), `five5` is exactly 5 (kept).
        assert_eq!(
            got,
            [
                Some("abc\r".into()),
                Some("".into()),
                None,
                Some("five5".into()),
                Some("last".into())
            ]
        );
        // A too-long last line without a newline is dropped too, then end of input.
        let mut r = Cursor::new(b"abcdefgh".to_vec());
        assert!(matches!(
            read_capped_line(&mut r, 5).unwrap(),
            LineRead::TooLong
        ));
        assert!(matches!(
            read_capped_line(&mut r, 5).unwrap(),
            LineRead::Eof
        ));
    }

    #[test]
    fn read_capped_line_swallows_a_huge_line_in_small_pieces() {
        // 20 MB without a newline through a 4 KiB buffer: the result is one TooLong, then the
        // next line is read normally.
        let huge = std::io::repeat(b'x')
            .take(20 * 1024 * 1024)
            .chain(Cursor::new(b"\nok\n".to_vec()));
        let mut r = std::io::BufReader::with_capacity(4096, huge);
        assert!(matches!(
            read_capped_line(&mut r, MAX_LINE_BYTES).unwrap(),
            LineRead::TooLong
        ));
        match read_capped_line(&mut r, MAX_LINE_BYTES).unwrap() {
            LineRead::Line(b) => assert_eq!(b, b"ok"),
            _ => panic!("expected the next line"),
        }
    }

    #[test]
    fn an_overlong_stdin_line_is_dropped_with_an_error_event_and_the_loop_goes_on() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let r = run_text(
            &format!(
                "{long}
report
stop
"
            ),
            vec![Ok(scan(vec![build()]))],
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.events(), ["started", "error", "report"], "{}", r.stdout);
        let message = r.lines()[1]["message"].as_str().unwrap().to_string();
        assert!(
            message.contains("too long") && message.len() < 300,
            "{message}"
        );
        // A line of exactly the cap is read (and is just an unknown command).
        let edge = "y".repeat(MAX_LINE_BYTES);
        let r = run_text(
            &format!(
                "{edge}
stop
"
            ),
            vec![Ok(scan(vec![build()]))],
        );
        assert!(
            r.lines()[1]["message"]
                .as_str()
                .unwrap()
                .contains("unknown command"),
            "{}",
            r.stdout
        );
    }

    // ---- final review ----

    const GB: u64 = 1024 * MB;

    fn complete(bytes: u64) -> Option<Measured> {
        Some(Measured {
            bytes,
            complete: true,
        })
    }

    #[test]
    fn the_analysis_of_the_observer_never_uses_the_admin_search() {
        // From an elevated terminal the MFT of every NTFS drive would be read at each check.
        let mut a = args();
        a.no_drives = false;
        let o = observe_options(&a);
        assert!(!o.admin, "{o:?}");
        assert!(o.drives && !o.measure, "{o:?}");
        a.no_drives = true;
        assert!(!observe_options(&a).drives);
    }

    #[test]
    fn a_check_stops_measuring_when_its_budget_is_over_and_the_rest_waits_for_the_next_one() {
        let measured: Arc<Mutex<Vec<String>>> = Arc::default();
        let new: Vec<Item> = ["w1", "w2", "w3", "w4", "w5"]
            .iter()
            .map(|n| worktree(n))
            .collect();
        let mut with_new = vec![build()];
        with_new.extend(new);
        let mut d = deps(vec![Ok(scan(vec![build()])), Ok(scan(with_new))], |_| {});
        d.measure = Box::new({
            let measured = Arc::clone(&measured);
            move |p| {
                measured.lock().unwrap().push(p.display().to_string());
                complete(MB)
            }
        });
        // The fake clock moves one second per reading: 2.5 s leave room for two measures.
        d.measure_budget = Duration::from_millis(2500);
        let mut observer = start(&d, Params::default()).unwrap();
        let tail = || -> Vec<String> {
            let mut m = measured.lock().unwrap();
            let names = m
                .iter()
                .map(|p| p.rsplit('\\').next().unwrap().to_string())
                .collect();
            m.clear();
            names
        };
        let _ = tail(); // the baseline's own measure
        check(&mut observer, &d).unwrap();
        // Folders never seen first, in the order of the scan.
        assert_eq!(tail(), ["w1", "w2"]);
        let unmeasured = |o: &Observer| {
            o.report(false)
                .items
                .iter()
                .filter(|i| i.item.category == Category::Worktrees && i.item.bytes.is_none())
                .count()
        };
        assert_eq!(unmeasured(&observer), 3);
        // Then the ones never measured, before any measured again.
        check(&mut observer, &d).unwrap();
        assert_eq!(tail(), ["w3", "w4"]);
        // Then the last one never measured, and the oldest measure of the others: the build folder.
        check(&mut observer, &d).unwrap();
        assert_eq!(tail(), ["w5", "target"]);
        assert_eq!(unmeasured(&observer), 0);
    }

    #[test]
    fn a_folder_over_the_caps_counts_as_a_lower_bound_and_is_not_walked_again_at_every_check() {
        let clock = Arc::new(AtomicI64::new(1_700_000_000_000));
        let big_walks = Arc::new(AtomicUsize::new(0));
        let mut d = deps(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("big")])),
            ],
            |_| {},
        );
        d.now_ms = Box::new({
            let clock = Arc::clone(&clock);
            move || clock.load(Ordering::SeqCst)
        });
        d.measure = Box::new({
            let big_walks = Arc::clone(&big_walks);
            move |p| {
                if p.ends_with("big") {
                    big_walks.fetch_add(1, Ordering::SeqCst);
                    Some(Measured {
                        bytes: 3 * GB,
                        complete: false,
                    })
                } else {
                    complete(10 * MB)
                }
            }
        });
        let params = Params {
            grow_threshold: 2 * GB,
            worktree_threshold: 99,
        };
        let mut observer = start(&d, params).unwrap();
        clock.fetch_add(1000, Ordering::SeqCst);
        let events = check(&mut observer, &d).unwrap();
        // The part that was walked already tells the growth, and the attention fires.
        assert!(
            events.iter().any(|e| matches!(e,
                Event::Attention { kind: xiox_observer::AttentionKind::Growth, value, .. } if *value == 3 * GB)),
            "{events:?}"
        );
        let report = observer.report(false);
        let big = report
            .items
            .iter()
            .find(|i| i.item.category == Category::Worktrees)
            .unwrap();
        assert_eq!(big.item.bytes, Some(3 * GB));
        assert!(big.bytes_partial);
        assert!(
            !report
                .items
                .iter()
                .find(|i| i.item.category == Category::Builds)
                .unwrap()
                .bytes_partial
        );
        assert_eq!(big_walks.load(Ordering::SeqCst), 1);
        // A check every minute for nine minutes: the capped folder is left alone, the others are not.
        for _ in 0..9 {
            clock.fetch_add(60_000, Ordering::SeqCst);
            check(&mut observer, &d).unwrap();
        }
        assert_eq!(big_walks.load(Ordering::SeqCst), 1, "backed off");
        // Past the ten minutes it is measured again.
        clock.fetch_add(120_000, Ordering::SeqCst);
        check(&mut observer, &d).unwrap();
        assert_eq!(big_walks.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_baseline_flags_a_build_folder_that_hit_the_caps() {
        let mut d = deps(vec![Ok(scan(vec![build()]))], |_| {});
        d.measure = Box::new(|_| {
            Some(Measured {
                bytes: 5 * MB,
                complete: false,
            })
        });
        let observer = start(&d, Params::default()).unwrap();
        let item = &observer.report(false).items[0];
        assert_eq!(item.item.bytes, Some(5 * MB));
        assert!(item.bytes_partial && !item.new);
    }

    /// The project as a check sees it when git answered: three worktrees at the start.
    fn with_project(is_repo: bool, root: &str, linked: &[&str], items: Vec<Item>) -> ProjectScan {
        let mut s = scan(items);
        s.project.is_repo = is_repo;
        s.project.root = PathBuf::from(root);
        s.project.worktrees = linked
            .iter()
            .map(|n| PathBuf::from(format!(r"D:\Projects\{n}")))
            .collect();
        s
    }

    fn degraded_world(
        second: ProjectScan,
        linked: impl Fn(&Path) -> Option<usize> + Send + Sync + 'static,
    ) -> (Deps, Observer) {
        let first = with_project(
            true,
            r"D:\Projects\mukade",
            &["w1", "w2"],
            vec![build(), worktree("w1"), worktree("w2")],
        );
        let mut d = deps(vec![Ok(first), Ok(second)], |_| {});
        d.linked_worktrees = Box::new(linked);
        let observer = start(&d, Params::default()).unwrap();
        (d, observer)
    }

    #[test]
    fn a_check_whose_git_answers_look_degraded_is_an_error_and_never_a_change() {
        // Git timed out: no worktrees listed, and git still fails when asked again.
        let empty = || with_project(true, r"D:\Projects\mukade", &[], vec![build()]);
        let (d, mut observer) = degraded_world(empty(), |_| None);
        let message = check(&mut observer, &d).unwrap_err();
        assert!(
            message.contains("could not list the worktrees"),
            "{message}"
        );
        // Nothing was fed to the observer: the worktrees are still there, not removed, and the
        // next healthy check says nothing changed.
        assert_eq!(observer.report(false).items.len(), 3);
        let healthy = with_project(
            true,
            r"D:\Projects\mukade",
            &["w1", "w2"],
            vec![build(), worktree("w1"), worktree("w2")],
        );
        let mut d2 = deps(vec![Ok(healthy)], |_| {});
        d2.linked_worktrees = Box::new(|_| None);
        assert!(check(&mut observer, &d2).unwrap().is_empty());

        // Git answers the second time: the first answer was incomplete.
        let (d, mut observer) = degraded_world(empty(), |_| Some(2));
        let message = check(&mut observer, &d).unwrap_err();
        assert!(message.contains("lists linked worktrees"), "{message}");

        // Git answers "none": they were really all removed, and that is a change.
        let (d, mut observer) = degraded_world(empty(), |_| Some(0));
        let events = check(&mut observer, &d).unwrap();
        match &events[0] {
            Event::Changed { removed, .. } => assert_eq!(removed.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_check_with_another_root_or_no_longer_a_repository_is_an_error() {
        let items = || vec![build(), worktree("w1"), worktree("w2")];
        let not_a_repo = with_project(false, r"D:\Projects\mukade", &[], items());
        let (d, mut observer) = degraded_world(not_a_repo, |_| Some(0));
        let message = check(&mut observer, &d).unwrap_err();
        assert!(message.contains("repository"), "{message}");
        let elsewhere = with_project(true, r"D:\Projects\mukade\crates", &["w1", "w2"], items());
        let (d, mut observer) = degraded_world(elsewhere, |_| Some(0));
        let message = check(&mut observer, &d).unwrap_err();
        assert!(message.contains(r"D:\Projects\mukade\crates"), "{message}");
        // Through the loop it is an `error` event and the report is stale, never a `changed`.
        let first = with_project(
            true,
            r"D:\Projects\mukade",
            &["w1"],
            vec![build(), worktree("w1")],
        );
        let broken = with_project(true, r"D:\Projects\mukade", &[], vec![build()]);
        let mut d = with_interval(
            deps(vec![Ok(first), Ok(broken)], |_| {}),
            Duration::from_secs(3600),
        );
        d.linked_worktrees = Box::new(|_| None);
        let r = run_with(Cursor::new(b"report\nstop\n".to_vec()), &d);
        assert_eq!(r.events(), ["started", "error", "report"], "{}", r.stdout);
        let lines = r.lines();
        assert!(
            lines[1]["message"]
                .as_str()
                .unwrap()
                .starts_with("check failed:")
        );
        assert_eq!(lines[2]["stale"], true);
        assert_eq!(lines[2]["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn the_panic_message_of_the_baseline_worker_says_the_next_call_retries() {
        let on_worker = panic_report(Some(MCP_BASELINE_THREAD), "boom at x.rs:1");
        assert!(
            on_worker.contains(
                "the first analysis stopped on an internal error; the next call retries it"
            ),
            "{on_worker}"
        );
        assert!(
            on_worker.contains("boom at x.rs:1") && !on_worker.contains("xiox observe stopped"),
            "{on_worker}"
        );
        for other in [Some("xiox-observe-stdin"), Some("main"), None] {
            let message = panic_report(other, "boom");
            assert!(
                message.starts_with("error: xiox observe stopped on an internal error: boom"),
                "{message}"
            );
        }
    }

    #[test]
    fn report_lines_already_waiting_are_folded_into_one_check() {
        let (tx, rx) = mpsc::sync_channel(8);
        let line = |l: &str| Input::Line(l.to_string());
        // The first `report` line is the one the loop already took off the queue.
        for l in ["report", "stop", "report"] {
            tx.send(line(l)).unwrap();
        }
        let (folded, next) = fold_queued_reports(&rx);
        assert_eq!(
            folded, 1,
            "the report line that waits joins the one being answered"
        );
        assert!(matches!(next, Some(Input::Line(ref l)) if l == "stop"));
        // What follows the first other line stays queued, in order.
        assert!(matches!(rx.try_recv(), Ok(Input::Line(ref l)) if l == "report"));
        // Nothing waiting: nothing folded.
        let (folded, next) = fold_queued_reports(&rx);
        assert_eq!((folded, next.is_none()), (0, true));
    }

    #[test]
    fn every_report_line_gets_its_report_even_when_they_are_folded() {
        let r = run_text(
            "report\nreport\nreport\nstop\n",
            vec![Ok(scan(vec![build()]))],
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(
            r.events(),
            ["started", "report", "report", "report"],
            "{}",
            r.stdout
        );
    }

    #[test]
    fn the_stdin_queue_is_bounded() {
        let mut lines = String::new();
        for _ in 0..1000 {
            lines.push_str("report\n");
        }
        let rx = spawn_reader(Cursor::new(lines.into_bytes()), spawn_named).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // Nobody reads: the reader thread waits instead of queuing every line.
        let queued = rx.try_iter().count();
        assert!(queued <= INPUT_QUEUE + 1, "{queued} lines were queued");
    }

    /// The code of `source` with comments, string literals and char literals blanked out.
    fn code_of(source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '/' if chars.get(i + 1) == Some(&'/') => {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                }
                '"' => {
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        i += if chars[i] == '\\' { 2 } else { 1 };
                    }
                    i += 1;
                    out.push(' ');
                }
                '\'' if chars.get(i + 2) == Some(&'\'') => {
                    i += 3;
                    out.push(' ');
                }
                '\'' if chars.get(i + 1) == Some(&'\\') && chars.get(i + 3) == Some(&'\'') => {
                    i += 4;
                    out.push(' ');
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        out
    }

    /// What `source` names that removes or writes (comments and strings do not count).
    fn offences(source: &str) -> Vec<String> {
        // Words that are about something else: the `cleanCommand` the observer only PRINTS, and
        // `remove`/`removed` of in-memory maps and of the `removed` field of a `changed` event.
        let allowed = [
            "clean_command",
            "clean_command_truncated",
            "clean_command_max_items",
            "remove",
            "removed",
        ];
        let code = code_of(source);
        let mut found = Vec::new();
        for forbidden in [
            "fs::write",
            "File::create",
            "OpenOptions",
            "create_dir",
            "fs::rename",
            "fs::copy",
            "is_elevated",
        ] {
            if code.contains(forbidden) {
                found.push(forbidden.to_string());
            }
        }
        for ident in code.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            let lower = ident.to_ascii_lowercase();
            if allowed.contains(&lower.as_str()) {
                continue;
            }
            if ["clean", "remove", "delete", "trash", "unlink"]
                .iter()
                .any(|word| lower.contains(word))
            {
                found.push(ident.to_string());
            }
        }
        found
    }

    #[test]
    fn the_read_only_scan_catches_what_it_is_meant_to_catch() {
        for bad in [
            "std::fs::remove_dir_all(path)",
            "fs::write(p, b)",
            "let f = File::create(p);",
            "OpenOptions::new()",
            "move_to_trash(p)",
            "xiox_project::clean(&scan)",
            "std::fs::remove_file(p)",
        ] {
            assert!(!offences(bad).is_empty(), "{bad}");
        }
        // Comments, strings and the allowed words are not code that removes anything.
        let fine = "// remove everything
let m = \"xiox clean --delete\"; map.remove(&key); let c = '\"'; clean_command";
        assert_eq!(offences(fine), Vec::<String>::new());
    }

    #[test]
    fn the_observer_sources_never_name_anything_that_removes_or_writes() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let sources = [
            "src/observe.rs",
            "src/mcp.rs",
            "../xiox-observer/src/lib.rs",
            "../xiox-observer/src/measure.rs",
            "../xiox-observer/src/state.rs",
        ];
        for path in sources {
            let text = std::fs::read_to_string(format!("{manifest}/{path}")).unwrap();
            // Everything after the first test module is test code.
            let code = text.split("#[cfg(test)]").next().unwrap();
            assert!(code.len() > 300, "{path}: the scan reads the code");
            assert_eq!(offences(code), Vec::<String>::new(), "{path}");
        }
    }
}
