//! `xiox observe`: a long-running, read-only process for ONE project. It takes a snapshot, re-checks
//! at intervals and tells its caller what appeared since it started, one JSON object per stdout line.
//!
//! stdout carries only those lines; everything meant for a person goes to stderr. The loop waits on
//! a channel fed by a stdin-reader thread, so it sleeps between checks and ends when stdin closes or
//! a `stop` line arrives.

use crate::run::{EXIT_ERROR, scan_project, write_result};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use xiox_observer::{Event, Observer, Params, measure_bytes};
use xiox_project::{Category, Options, ProjectScan};

/// Folders are measured with these caps, for the baseline and for every later check alike, so that
/// growth is never the difference between two different measurers.
pub const MEASURE_MAX_ENTRIES: usize = 200_000;
pub const MEASURE_MAX_TIME: Duration = Duration::from_secs(10);

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
        Params { grow_threshold: self.grow_threshold, worktree_threshold: self.worktree_threshold }
    }
}

/// What the loop needs from the outside, so that tests can fake the disk and the clock.
pub struct Deps {
    /// A light analysis of the project (no weights).
    pub analyze: Box<dyn Fn() -> Result<ProjectScan, String>>,
    /// The size of one folder, bounded; `None` when it cannot be told.
    pub measure: Box<dyn Fn(&Path) -> Option<u64>>,
    pub now_ms: Box<dyn Fn() -> i64>,
    pub interval: Duration,
}

impl Deps {
    /// The real PC (or the test switches' fake world in debug builds). In debug builds
    /// `XIOX_OBSERVE_INTERVAL_MS` replaces the interval and bypasses its 60-second minimum.
    pub fn real(cmd: &ObserveArgs) -> Self {
        let path = cmd.project.clone().unwrap_or_else(|| PathBuf::from("."));
        let opts = Options { drives: !cmd.no_drives, measure: false, admin: xiox_project::is_elevated() };
        let mut interval = Duration::from_secs(cmd.interval_secs);
        if cfg!(debug_assertions) {
            if let Some(ms) = std::env::var("XIOX_OBSERVE_INTERVAL_MS").ok().and_then(|v| v.trim().parse::<u64>().ok()) {
                interval = Duration::from_millis(ms.max(1));
            }
        }
        Deps {
            analyze: Box::new(move || scan_project(&path, &opts).map_err(|e| e.to_string())),
            measure: Box::new(|p| measure_bytes(p, MEASURE_MAX_ENTRIES, MEASURE_MAX_TIME)),
            now_ms: Box::new(|| chrono::Utc::now().timestamp_millis()),
            interval,
        }
    }
}

/// The initial snapshot: a light analysis, then the weights of everything that has a folder and no
/// weight yet and of the build folders, then the baseline. Shared by both modes.
pub fn start(deps: &Deps, params: Params) -> Result<Observer, String> {
    let mut scan = (deps.analyze)()?;
    for item in &mut scan.items {
        if let Some(path) = &item.path {
            if item.bytes.is_none() || item.category == Category::Builds {
                item.bytes = (deps.measure)(path);
            }
        }
    }
    Ok(Observer::new(scan, params, (deps.now_ms)()))
}

/// One check: a light analysis, the weights of `observer.keys_to_measure()` and of the folders it
/// has never seen (so that a new folder has its weight from its first `changed`), then the
/// observer's events. Shared by both modes.
pub fn check(observer: &mut Observer, deps: &Deps) -> Result<Vec<Event>, String> {
    let mut scan = (deps.analyze)()?;
    let mut measure: HashSet<String> = observer.keys_to_measure().into_iter().collect();
    let known: HashSet<String> = observer.report(false).items.into_iter().map(|v| v.item.key).collect();
    measure.extend(scan.items.iter().filter(|i| !known.contains(&i.key)).map(|i| i.key.clone()));
    for item in &mut scan.items {
        if measure.contains(&item.key) {
            if let Some(path) = &item.path {
                item.bytes = (deps.measure)(path);
            }
        }
    }
    Ok(observer.update(scan, (deps.now_ms)()))
}

/// A panic is told on stderr only (the default hook does the same, this one says what it is) and
/// the process ends with a non-zero code.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let _ = writeln!(std::io::stderr(), "error: xiox observe stopped on an internal error: {info}");
    }));
}

/// The command of the real PC: real stdin, the real clock.
pub fn run_observe(cmd: ObserveArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if cmd.mcp {
        let _ = writeln!(err, "error: --mcp is not available yet");
        return EXIT_ERROR;
    }
    install_panic_hook();
    let deps = Deps::real(&cmd);
    observe(cmd, std::io::BufReader::new(std::io::stdin()), out, err, &deps)
}

/// An output that remembers a closed pipe: `write_result` treats it as success (the person just ran
/// `| head`), but for a long-running process it means the caller is gone.
struct Tap<'a> {
    inner: &'a mut dyn Write,
    pipe_closed: bool,
}

impl Write for Tap<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf).inspect_err(|e| self.pipe_closed |= e.kind() == std::io::ErrorKind::BrokenPipe)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush().inspect_err(|e| self.pipe_closed |= e.kind() == std::io::ErrorKind::BrokenPipe)
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
    std::thread::Builder::new().name("xiox-observe-stdin".into()).spawn(job).map(|_| ())
}

/// `Instant::now() + interval` that cannot panic: an interval too large for an `Instant` waits
/// for the longest accepted one instead.
fn deadline_after(interval: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(interval)
        .or_else(|| now.checked_add(Duration::from_secs(crate::args::MAX_INTERVAL_SECS)))
        .unwrap_or(now)
}

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
fn spawn_reader(mut stdin: impl BufRead + Send + 'static, spawn: Spawn) -> Result<mpsc::Receiver<Input>, String> {
    let (tx, rx) = mpsc::channel();
    let job = Box::new(move || {
        loop {
            let message = match read_capped_line(&mut stdin, MAX_LINE_BYTES) {
                Ok(LineRead::Eof) => Input::Closed,
                Ok(LineRead::Line(bytes)) => Input::Line(String::from_utf8_lossy(&bytes).trim().to_string()),
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

/// `{"schema":1,"event":"<kebab>","time":"<RFC 3339 UTC>", …event fields}` as one line. A `report`
/// carries its own `schema`: it is not added twice.
fn event_line(event: &Event, time_ms: i64) -> Result<String, String> {
    let value = serde_json::to_value(event).map_err(|e| format!("cannot write an event as JSON: {e}"))?;
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
    Ok(format!("{}\n", Value::Object(line)))
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
    let mut emitter = Emitter { out: Tap { inner: out, pipe_closed: false }, err, now_ms: &*deps.now_ms };
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
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        let message = match input.recv_timeout(wait) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                // The reader always says `Closed` or `Failed` before it ends: it died.
                let _ = writeln!(emitter.err, "error: the stdin reader stopped unexpectedly");
                return EXIT_ERROR;
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
            Some(Input::TooLong) => emitter.error(format!("stdin line too long (over {MAX_LINE_BYTES} bytes): dropped")),
            Some(Input::Line(line)) => match line.as_str() {
                "" => continue,
                "stop" => return 0,
                "report" => run_check(&mut observer, deps, &mut emitter, &mut last_check_failed)
                    .and_then(|()| emitter.emit(&Event::Report(observer.report(last_check_failed)))),
                other => emitter.error(format!("unknown command: {} (expected report or stop)", echo(other))),
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
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;
    use xiox_model::{ArtifactKind, Level};
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
            why: Why::CatalogFolder { label_en: "Rust build".into() },
            target: Target::Dir { kind: ArtifactKind::Catalog },
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
            why: Why::LinkedWorktree { branch: None, head: None },
            target: Target::Worktree { repo: PathBuf::from(r"D:\Projects\mukade") },
        }
    }

    /// Fake dependencies: `analyze` returns the script's results in order and then repeats the
    /// last one; `hook(n)` runs at the start of the `n`th call (0 = the initial analysis).
    /// Every measured folder is 10 MB. The clock advances one second per call.
    pub(crate) fn deps(script: Vec<Result<ProjectScan, String>>, hook: impl Fn(usize) + 'static) -> Deps {
        let calls = Cell::new(0usize);
        let clock = Rc::new(Cell::new(1_700_000_000_000i64));
        Deps {
            analyze: Box::new(move || {
                let n = calls.get();
                calls.set(n + 1);
                hook(n);
                script[n.min(script.len() - 1)].clone()
            }),
            measure: Box::new(|_| Some(10 * MB)),
            now_ms: Box::new(move || {
                clock.set(clock.get() + 1000);
                clock.get()
            }),
            interval: Duration::from_millis(15),
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
    use std::sync::mpsc;

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
        (tx, std::io::BufReader::new(Gated { rx, pending: Vec::new() }))
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
            self.lines().iter().map(|v| v["event"].as_str().unwrap().to_string()).collect()
        }
    }

    fn run_with(stdin: impl BufRead + Send + 'static, d: &Deps) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = observe(args(), stdin, &mut out, &mut err, d);
        Run { code, stdout: String::from_utf8(out).unwrap(), stderr: String::from_utf8(err).unwrap() }
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
            vec![Ok(scan(vec![build()])), Ok(scan(vec![build(), worktree("mukade-wt")]))],
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
            let time = v["time"].as_str().unwrap_or_else(|| panic!("no time in {v}"));
            assert!(time.ends_with('Z'), "UTC: {time}");
            chrono::DateTime::parse_from_rfc3339(time).unwrap_or_else(|e| panic!("{time}: {e}"));
            let event = v["event"].as_str().unwrap();
            assert!(["started", "changed", "attention", "report", "error"].contains(&event), "{event}");
        }
        // Started carries the project, the parameters and the totals.
        assert_eq!(lines[0]["project"]["name"], "mukade");
        assert_eq!(lines[0]["params"]["worktreeThreshold"], 3);
        assert!(lines[0]["totals"].is_object());
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
            vec![Ok(scan(vec![build()])), Ok(scan(vec![build(), worktree("mukade-wt")]))],
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

        let report = lines.iter().find(|v| v["event"] == "report").expect("a report line");
        assert_eq!(report["stale"], false);
        let items = report["items"].as_array().unwrap();
        let by_cat = |c: &str| items.iter().find(|i| i["category"] == c).unwrap();
        assert_eq!(by_cat("worktrees")["new"], true);
        assert_eq!(by_cat("builds")["new"], false);
        // `schema` is written once on a report line (the report carries its own).
        let line = r.stdout.lines().find(|l| l.contains("\"event\":\"report\"")).unwrap();
        assert_eq!(line.matches("\"schema\"").count(), 1, "{line}");
        // Changed comes before the report.
        let events = r.events();
        let (c, p) = (events.iter().position(|e| e == "changed"), events.iter().position(|e| e == "report"));
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
        let r = run_ticking(vec![Ok(scan(vec![build()])), Err("disk went away".into())], 3, "report\nstop\n");
        assert_eq!(r.code, 0, "{}", r.stderr);
        let lines = r.lines();
        let errors: Vec<&Value> = lines.iter().filter(|v| v["event"] == "error").collect();
        assert!(errors.len() >= 2, "{}", r.stdout);
        assert!(errors[0]["message"].as_str().unwrap().contains("disk went away"), "{}", errors[0]);
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
        let r = run_text("frobnicate now\nreport\nstop\n", vec![Ok(scan(vec![build()]))]);
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
            vec![Ok(scan(vec![build()])), Ok(scan(vec![build(), worktree("a")])), Err("boom".into())],
            4,
            "what\nreport\nstop\n",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert!(!r.stdout.is_empty());
        for line in r.stdout.lines() {
            let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("stdout line {line:?}: {e}"));
            assert!(v.is_object() && v["schema"] == 1 && v["event"].is_string() && v["time"].is_string(), "{line}");
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
        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(3600));
        // Never ends by itself: the failing write must end it.
        let (_tx, stdin) = gated();
        let mut err = Vec::new();
        let code = observe(args(), stdin, &mut Failing(std::io::ErrorKind::BrokenPipe), &mut err, &d);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));

        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(3600));
        let (_tx, stdin) = gated();
        let mut err = Vec::new();
        let code = observe(args(), stdin, &mut Failing(std::io::ErrorKind::PermissionDenied), &mut err, &d);
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&err).contains("cannot write to the output"));
    }

    #[test]
    fn check_measures_new_and_known_builds_and_returns_the_observers_events() {
        let d = deps(
            vec![Ok(scan(vec![build()])), Ok(scan(vec![build(), worktree("w")]))],
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
        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(u64::MAX));
        let r = run_with(Cursor::new(b"report
stop
".to_vec()), &d);
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
        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(3600));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = observe_with(args(), Cursor::new(b"stop
".to_vec()), &mut out, &mut err, &d, failing_spawn);
        let err = String::from_utf8(err).unwrap();
        assert_eq!(code, 1, "{err}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        let errors: Vec<&str> = err.lines().filter(|l| l.starts_with("error:")).collect();
        assert_eq!(errors.len(), 1, "{err}");
        assert!(errors[0].contains("cannot start the stdin reader thread") && errors[0].contains("no threads left"), "{err}");
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
        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(3600));
        let r = run_with(std::io::BufReader::new(BadStdin { panic: true }), &d);
        assert_eq!(r.code, 1, "{}", r.stderr);
        let errors: Vec<&str> = r.stderr.lines().filter(|l| l.starts_with("error:")).collect();
        assert_eq!(errors.len(), 1, "{}", r.stderr);
        assert!(errors[0].contains("stdin reader stopped"), "{}", r.stderr);
        assert_eq!(r.events(), ["started"], "{}", r.stdout);
    }

    #[test]
    fn a_stdin_read_error_is_reported_on_stderr_and_ends_the_loop_with_1() {
        let d = with_interval(deps(vec![Ok(scan(vec![build()]))], |_| {}), Duration::from_secs(3600));
        let r = run_with(std::io::BufReader::new(BadStdin { panic: false }), &d);
        assert_eq!(r.code, 1, "{}", r.stderr);
        assert!(r.stderr.contains("error: cannot read stdin: bad pipe"), "{}", r.stderr);
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
            [Some("abc\r".into()), Some("".into()), None, Some("five5".into()), Some("last".into())]
        );
        // A too-long last line without a newline is dropped too, then end of input.
        let mut r = Cursor::new(b"abcdefgh".to_vec());
        assert!(matches!(read_capped_line(&mut r, 5).unwrap(), LineRead::TooLong));
        assert!(matches!(read_capped_line(&mut r, 5).unwrap(), LineRead::Eof));
    }

    #[test]
    fn read_capped_line_swallows_a_huge_line_in_small_pieces() {
        // 20 MB without a newline through a 4 KiB buffer: the result is one TooLong, then the
        // next line is read normally.
        let huge = std::io::repeat(b'x').take(20 * 1024 * 1024).chain(Cursor::new(b"
ok
".to_vec()));
        let mut r = std::io::BufReader::with_capacity(4096, huge);
        assert!(matches!(read_capped_line(&mut r, MAX_LINE_BYTES).unwrap(), LineRead::TooLong));
        match read_capped_line(&mut r, MAX_LINE_BYTES).unwrap() {
            LineRead::Line(b) => assert_eq!(b, b"ok"),
            _ => panic!("expected the next line"),
        }
    }

    #[test]
    fn an_overlong_stdin_line_is_dropped_with_an_error_event_and_the_loop_goes_on() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let r = run_text(&format!("{long}
report
stop
"), vec![Ok(scan(vec![build()]))]);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.events(), ["started", "error", "report"], "{}", r.stdout);
        let message = r.lines()[1]["message"].as_str().unwrap().to_string();
        assert!(message.contains("too long") && message.len() < 300, "{message}");
        // A line of exactly the cap is read (and is just an unknown command).
        let edge = "y".repeat(MAX_LINE_BYTES);
        let r = run_text(&format!("{edge}
stop
"), vec![Ok(scan(vec![build()]))]);
        assert!(r.lines()[1]["message"].as_str().unwrap().contains("unknown command"), "{}", r.stdout);
    }
}
