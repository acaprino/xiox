//! Entry point of the commands: arguments in, exit code out. Results go to `out`, everything
//! else (progress, notes, errors, the confirmation question) to `err`.

use crate::args::{self, Command};
use crate::{json, observe, render};
use xiox_model::RemoveMode;
use xiox_project::{Category, Chosen, CleanOutcome, CleanRequest, Error, Options, ProjectScan};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_PARTIAL: i32 = 3;

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match args::parse(args) {
        Err(msg) => {
            let _ = writeln!(err, "error: {msg}\n\n{}", args::USAGE);
            EXIT_USAGE
        }
        Ok(Command::Help) => code_of(write_result(out, err, &format!("{}\n", args::help()))),
        Ok(Command::Version) => code_of(write_result(out, err, &format!("xiox {}\n", env!("CARGO_PKG_VERSION")))),
        Ok(Command::Analyze { project, json, no_drives }) => analyze(project, json, no_drives, out, err),
        Ok(Command::Observe { project, interval_secs, grow_threshold, worktree_threshold, no_drives, mcp }) => {
            let cmd = observe::ObserveArgs { project, interval_secs, grow_threshold, worktree_threshold, no_drives, mcp };
            observe::run_observe(cmd, out, err)
        }
        Ok(Command::Clean { project, categories, include, permanent, yes, dry_run, json, no_drives }) => {
            let flags = CleanFlags { project, categories, include, permanent, yes, dry_run, json, no_drives };
            clean(&flags, out, err)
        }
    }
}

fn code_of(r: Result<(), i32>) -> i32 {
    r.err().unwrap_or(EXIT_OK)
}

/// Test switches, read by debug builds only (a release `xiox` ignores both variables):
/// `XIOX_SEARCH_ROOTS` (paths separated by `;`) replaces the fixed drives and the home folder
/// as the places searched for copies and name-matched folders; `XIOX_NO_SESSIONS=1` skips the
/// session listing of this PC (no cache written, no `sessions` items). Either one switches the
/// analysis (and the cleaning) to the test mode: roots default to none, sessions to the real ones.
struct TestSwitches {
    search_roots: Vec<PathBuf>,
    no_sessions: bool,
}

fn test_switches() -> Option<TestSwitches> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let roots = std::env::var_os("XIOX_SEARCH_ROOTS");
    let no_sessions = std::env::var_os("XIOX_NO_SESSIONS").is_some_and(|v| v == "1");
    if roots.is_none() && !no_sessions {
        return None;
    }
    let search_roots = roots
        .map(|r| r.to_string_lossy().split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default();
    Some(TestSwitches { search_roots, no_sessions })
}

/// The analysis of the real PC (or of the test switches' fake world in debug builds).
pub fn scan_project(path: &Path, opts: &Options) -> Result<ProjectScan, Error> {
    match test_switches() {
        Some(t) if t.no_sessions => xiox_project::analyze_project_without_sessions(path, opts, &t.search_roots),
        Some(t) => xiox_project::analyze_project_in(path, opts, &t.search_roots),
        None => xiox_project::analyze_project(path, opts),
    }
}

/// The removal, in the same world as [`scan_project`]: the test switches that fake the analysis
/// fake the cleaning too, so a debug build driven by them never reads a real session.
fn clean_project(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, dry_run: bool) -> CleanOutcome {
    match test_switches() {
        Some(t) if t.no_sessions => {
            xiox_project::clean_without_sessions(scan, chosen, mode, dry_run, &t.search_roots)
        }
        Some(t) => xiox_project::clean_in(scan, chosen, mode, dry_run, &t.search_roots),
        None => xiox_project::clean(scan, chosen, mode, dry_run),
    }
}

/// Analyze the project (the current folder by default), telling the person on `err`.
fn analyze_path(project: Option<PathBuf>, no_drives: bool, err: &mut dyn Write) -> Result<ProjectScan, i32> {
    let path = project.unwrap_or_else(|| PathBuf::from("."));
    let opts = Options { drives: !no_drives, measure: true, admin: xiox_project::is_elevated(), history: true };
    let _ = writeln!(err, "Analyzing {} ...", path.display());
    scan_project(&path, &opts).map_err(|e| {
        let _ = writeln!(err, "error: {e}");
        EXIT_ERROR
    })
}

fn analyze(project: Option<PathBuf>, json: bool, no_drives: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let scan = match analyze_path(project, no_drives, err) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let text = if json {
        match json::scan_json(&scan) {
            Ok(v) => format!("{v}\n"),
            Err(e) => {
                let _ = writeln!(err, "error: cannot write the result as JSON: {e}");
                return EXIT_ERROR;
            }
        }
    } else {
        render::render_scan(&scan)
    };
    code_of(write_result(out, err, &text))
}

/// An I/O error in English: its kind (fixed English text) and the OS code. The operating system's own
/// description is in the language of Windows and is never printed.
fn io_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => format!("{} (os error {code})", e.kind()),
        None => e.kind().to_string(),
    }
}

/// Write a result to stdout. A closed pipe (`| head`) is not an error; any other failure is.
pub fn write_result(out: &mut dyn Write, err: &mut dyn Write, text: &str) -> Result<(), i32> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => {
            let _ = writeln!(err, "error: cannot write to the output: {}", io_text(&e));
            Err(EXIT_ERROR)
        }
    }
}

// ───────────────────────────────── clean ─────────────────────────────────

struct CleanFlags {
    project: Option<PathBuf>,
    categories: Vec<Category>,
    include: Vec<PathBuf>,
    permanent: bool,
    yes: bool,
    dry_run: bool,
    json: bool,
    no_drives: bool,
}

const NOTHING_SELECTED: &str =
    "Nothing selected. Name what to clean: --builds --worktrees --logs --sessions --include <path>";

/// Where the words of a run go. Text for the person: stdout, or stderr with `--json` (stdout
/// then carries the one JSON object and nothing else).
struct Sink<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    json: bool,
}

impl Sink<'_> {
    fn say(&mut self, text: &str) -> Result<(), i32> {
        if self.json {
            let _ = self.err.write_all(text.as_bytes());
            Ok(())
        } else {
            write_result(self.out, self.err, text)
        }
    }

    fn fail(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// The text for the person on stderr, whatever the mode.
    fn note(&mut self, text: &str) {
        let _ = self.err.write_all(text.as_bytes());
        let _ = self.err.flush();
    }

    /// End of a run that has a result: with `--json` the one object on stdout (`outcome` is
    /// `None` when nothing was run). Returns the exit code.
    fn conclude(&mut self, scan: &ProjectScan, outcome: Option<&CleanOutcome>, code: i32) -> i32 {
        if !self.json {
            return code;
        }
        match json::outcome_json(scan, outcome) {
            Ok(v) => match write_result(self.out, self.err, &format!("{v}\n")) {
                Ok(()) => code,
                Err(_) => write_failed_code(outcome),
            },
            Err(e) => {
                self.fail(&format!("error: cannot write the result as JSON: {e}"));
                write_failed_code(outcome)
            }
        }
    }
}

/// Exit code when the result could not be written. Nothing was removed (no outcome, or a dry
/// run): an error. Something was removed: 3 when part of it was not, so that a script still
/// learns the run was partial, else an error (the removal itself went well, the report did not).
fn write_failed_code(outcome: Option<&CleanOutcome>) -> i32 {
    match outcome {
        Some(o) if !o.dry_run && o.partial() => EXIT_PARTIAL,
        _ => EXIT_ERROR,
    }
}

/// `y` or `yes`, any case, around any whitespace. Anything else (an empty line included) is no.
fn is_yes(answer: &str) -> bool {
    let a = answer.trim();
    a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes")
}

/// Ask `Remove <n> items (<size>)? [y/N] ` on `err` and read one line from `input`. End of input
/// or a read error is a no.
fn confirm(err: &mut dyn Write, input: &mut dyn BufRead, chosen: &Chosen, mode: RemoveMode) -> bool {
    let n = chosen.keys.len();
    // Worktrees are removed by git for good, also when the rest goes to the Recycle Bin.
    let worktrees = if mode == RemoveMode::Recycle && !chosen.permanent_anyway.is_empty() {
        "; worktrees are removed by git for good, not to the Recycle Bin"
    } else {
        ""
    };
    let _ = write!(
        err,
        "Remove {n} item{} ({}{worktrees})? [y/N] ",
        if n == 1 { "" } else { "s" },
        render::fmt_size(chosen.bytes)
    );
    let _ = err.flush();
    let mut line = String::new();
    input.read_line(&mut line).is_ok() && is_yes(&line)
}

/// What `clean_run` takes from outside (tests pass fakes): the answer to the question, the
/// removal and the Recycle Bin check.
struct Deps<'a> {
    stdin_is_terminal: bool,
    stdin: &'a mut dyn BufRead,
    remove: &'a dyn Fn(&ProjectScan, &Chosen, RemoveMode, bool) -> CleanOutcome,
    shortfall: &'a dyn Fn(&ProjectScan, &Chosen, RemoveMode) -> Vec<(char, u64, u64)>,
}

fn clean(f: &CleanFlags, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let scan = match analyze_path(f.project.clone(), f.no_drives, err) {
        Ok(scan) => scan,
        Err(code) => return code,
    };
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut deps = Deps {
        stdin_is_terminal: stdin.is_terminal(),
        stdin: &mut lock,
        remove: &clean_project,
        shortfall: &xiox_project::recycle_shortfall,
    };
    let mut sink = Sink { out, err, json: f.json };
    match clean_run(f, &mut sink, scan, &mut deps) {
        Ok(code) | Err(code) => code,
    }
}

/// The steps of `clean` after the analysis, in order; `Err` is a failure to write something
/// before anything was removed (already reported).
///
/// Streams: with `--json` every text goes to stderr and stdout carries the one object. Without
/// it, a run that will ask the question (no `--yes`, no `--dry-run`) writes the summary, the
/// question and `Cancelled.` to stderr, so a redirected stdout never hides what is about to be
/// removed; every other text (listing, `Nothing to clean.`, summary of a `--yes` or `--dry-run`
/// run, the outcome) goes to stdout. Errors always go to stderr.
fn clean_run(f: &CleanFlags, s: &mut Sink, scan: ProjectScan, deps: &mut Deps) -> Result<i32, i32> {
    // A path JSON cannot carry must stop the run before anything is removed, not after.
    if f.json {
        if let Err(e) = json::scan_json(&scan) {
            s.fail(&format!("error: cannot write the result as JSON: {e}"));
            return Ok(EXIT_ERROR);
        }
    }
    // 1. Nothing named: the listing, as `analyze` prints it.
    if f.categories.is_empty() && f.include.is_empty() {
        if !f.json {
            s.say(&render::render_scan(&scan))?;
        }
        s.say(&format!("{NOTHING_SELECTED}\n"))?;
        return Ok(s.conclude(&scan, None, EXIT_OK));
    }

    // 2. Select. A refusal removes nothing.
    let mode = if f.permanent { RemoveMode::Permanent } else { RemoveMode::Recycle };
    let request = CleanRequest { categories: f.categories.clone(), include: f.include.clone(), mode };
    let chosen = match xiox_project::select(&scan, &request) {
        Ok(c) => c,
        Err(e) => {
            s.fail(&format!("error: {e}"));
            return Ok(EXIT_USAGE);
        }
    };
    if chosen.keys.is_empty() {
        s.say("Nothing to clean.\n")?;
        return Ok(s.conclude(&scan, None, EXIT_OK));
    }

    // 3. Summary, and the Recycle Bin check.
    let asks = !f.yes && !f.dry_run && !f.json;
    let shortfall = (deps.shortfall)(&scan, &chosen, mode);
    let summary = render::render_summary(&scan, &chosen, mode, &shortfall);
    if asks {
        s.note(&summary);
    } else {
        s.say(&summary)?;
    }
    if !shortfall.is_empty() {
        s.fail("error: not enough room in the Recycle Bin: nothing was removed");
        return Ok(EXIT_ERROR);
    }

    // 4. Dry run: every check, no removal.
    if f.dry_run {
        let outcome = (deps.remove)(&scan, &chosen, mode, true);
        s.say("Dry run: nothing was removed.\n")?;
        s.say(&render::render_outcome(&outcome))?;
        return Ok(s.conclude(&scan, Some(&outcome), EXIT_OK));
    }

    // 5. Confirmation.
    if !f.yes {
        if f.json {
            s.fail("error: --json needs --yes to remove anything");
            return Ok(EXIT_USAGE);
        }
        if !deps.stdin_is_terminal {
            s.fail("error: not a terminal: pass --yes to remove without asking");
            return Ok(EXIT_USAGE);
        }
        if !confirm(s.err, deps.stdin, &chosen, mode) {
            s.note("Cancelled.\n");
            return Ok(EXIT_OK);
        }
    }

    // 6. Remove, and say what happened. From here on something may be gone: a report that
    // cannot be written to stdout goes to stderr, and the exit code still tells partial from not.
    let outcome = (deps.remove)(&scan, &chosen, mode, false);
    let report = render::render_outcome(&outcome);
    if s.say(&report).is_err() {
        s.note(&report);
        return Ok(write_failed_code(Some(&outcome)));
    }
    let code = if outcome.partial() { EXIT_PARTIAL } else { EXIT_OK };
    Ok(s.conclude(&scan, Some(&outcome), code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiox_project::Chosen;

    #[test]
    fn only_y_and_yes_in_any_case_confirm() {
        for yes in ["y", "Y", "yes", "YES", "Yes", " y \r\n", "yes\n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "N", "no", "yess", "ye", "yes please", "sure", "1", "ok"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    fn chosen(n: usize, bytes: u64) -> Chosen {
        Chosen { keys: (0..n).map(|i| format!("k{i}")).collect(), bytes, permanent_anyway: Vec::new() }
    }

    #[test]
    fn the_question_names_the_count_and_the_size_and_waits_for_one_line() {
        let mut err = Vec::new();
        assert!(confirm(&mut err, &mut "yes\n".as_bytes(), &chosen(3, 6 * 1024 * 1024 * 1024), RemoveMode::Recycle));
        assert_eq!(String::from_utf8(err).unwrap(), "Remove 3 items (6.0 GB)? [y/N] ");
        let mut err = Vec::new();
        assert!(!confirm(&mut err, &mut "n\n".as_bytes(), &chosen(1, 512), RemoveMode::Recycle));
        assert_eq!(String::from_utf8(err).unwrap(), "Remove 1 item (512 B)? [y/N] ");
        // End of input is a no, never a yes.
        assert!(!confirm(&mut Vec::new(), &mut "".as_bytes(), &chosen(1, 1), RemoveMode::Recycle));
    }

    #[test]
    fn the_question_says_worktrees_go_for_good_when_the_rest_goes_to_the_bin() {
        let mut c = chosen(2, 2048);
        c.permanent_anyway = vec!["k0".to_string()];
        let mut err = Vec::new();
        assert!(confirm(&mut err, &mut "y\n".as_bytes(), &c, RemoveMode::Recycle));
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Remove 2 items (2 KB; worktrees are removed by git for good, not to the Recycle Bin)? [y/N] "
        );
        // Everything is permanent already: nothing to add.
        let mut err = Vec::new();
        assert!(confirm(&mut err, &mut "y\n".as_bytes(), &c, RemoveMode::Permanent));
        assert_eq!(String::from_utf8(err).unwrap(), "Remove 2 items (2 KB)? [y/N] ");
    }

    #[test]
    fn an_io_error_is_printed_with_its_kind_and_code_never_the_os_description() {
        let os = std::io::Error::from_raw_os_error(5);
        assert_eq!(io_text(&os), format!("{} (os error 5)", os.kind()));
        assert!(io_text(&os).is_ascii());
        assert_eq!(io_text(&std::io::Error::other("disk full")), std::io::Error::other("x").kind().to_string());
    }

    // ── clean_run with fakes: no analysis, no removal, no terminal ──

    use crate::testdata::{GB, build, scan};
    use xiox_project::Outcome;
    use std::cell::Cell;

    /// A stdout that takes `ok` writes and then fails, as a closed handle or a full disk does.
    struct FlakyOut {
        ok: usize,
    }
    impl Write for FlakyOut {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.ok == 0 {
                return Err(std::io::Error::other("disk full"));
            }
            self.ok -= 1;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn flags(yes: bool, dry_run: bool, json: bool, permanent: bool) -> CleanFlags {
        CleanFlags {
            project: None,
            categories: vec![Category::Builds],
            include: Vec::new(),
            permanent,
            yes,
            dry_run,
            json,
            no_drives: true,
        }
    }

    struct Ran {
        code: i32,
        out: String,
        err: String,
        removals: usize,
        dry_removals: usize,
    }

    /// Run `clean_run` on a one-build scan. `answer`: `Some(text)` is a terminal that types it,
    /// `None` is a stdin that is not a terminal. `partial`: the removal leaves the item behind.
    fn run_clean(
        f: &CleanFlags,
        answer: Option<&str>,
        shortfall: Vec<(char, u64, u64)>,
        partial: bool,
        out: &mut dyn Write,
    ) -> Ran {
        let removals = Cell::new(0);
        let dry = Cell::new(0);
        let remove = |s: &ProjectScan, c: &Chosen, mode: RemoveMode, dry_run: bool| {
            if dry_run {
                dry.set(dry.get() + 1);
            } else {
                removals.set(removals.get() + 1);
            }
            let items = c
                .keys
                .iter()
                .map(|k| {
                    let it = s.items.iter().find(|i| i.key == *k).unwrap();
                    let o = if partial {
                        Outcome::Failed { error: "in use".into() }
                    } else {
                        Outcome::Removed { bytes_freed: it.bytes.unwrap_or(0) }
                    };
                    (k.clone(), it.path.clone(), it.category, o)
                })
                .collect();
            CleanOutcome {
                schema: 1,
                mode,
                dry_run,
                items,
                bytes_freed: if partial { 0 } else { c.bytes },
                recycled_bytes: 0,
                elapsed_ms: 1,
            }
        };
        let room = |_: &ProjectScan, _: &Chosen, _: RemoveMode| shortfall.clone();
        let mut input = answer.unwrap_or("").as_bytes();
        let mut deps = Deps { stdin_is_terminal: answer.is_some(), stdin: &mut input, remove: &remove, shortfall: &room };
        let mut err = Vec::new();
        let code = {
            let mut sink = Sink { out, err: &mut err, json: f.json };
            match clean_run(f, &mut sink, scan(vec![build()]), &mut deps) {
                Ok(c) | Err(c) => c,
            }
        };
        Ran {
            code,
            out: String::new(),
            err: String::from_utf8(err).unwrap(),
            removals: removals.get(),
            dry_removals: dry.get(),
        }
    }

    fn run_to_vec(f: &CleanFlags, answer: Option<&str>, shortfall: Vec<(char, u64, u64)>, partial: bool) -> Ran {
        let mut out = Vec::new();
        let mut r = run_clean(f, answer, shortfall, partial, &mut out);
        r.out = String::from_utf8(out).unwrap();
        r
    }

    const SUMMARY: &str = "Total 6.0 GB in 1 item, permanently.";

    #[test]
    fn when_the_question_is_asked_the_summary_and_the_question_are_on_stderr_and_stdout_stays_clean() {
        // `n`: cancelled, nothing removed, nothing on stdout (a redirect cannot hide the summary).
        let r = run_to_vec(&flags(false, false, false, true), Some("n\n"), vec![], false);
        assert_eq!(r.code, EXIT_OK);
        assert_eq!(r.removals, 0);
        assert_eq!(r.out, "");
        assert!(r.err.contains(SUMMARY), "{}", r.err);
        assert!(r.err.find(SUMMARY) < r.err.find("Remove 1 item (6.0 GB)? [y/N] "), "{}", r.err);
        assert!(r.err.contains("Cancelled."), "{}", r.err);

        // `y`: removed; the outcome (not the summary) is on stdout.
        let r = run_to_vec(&flags(false, false, false, true), Some("Y\n"), vec![], false);
        assert_eq!((r.code, r.removals), (EXIT_OK, 1));
        assert!(r.err.contains(SUMMARY) && r.err.contains("[y/N]"), "{}", r.err);
        assert!(r.out.contains("removed") && r.out.contains("Freed 6.0 GB"), "{}", r.out);
        assert!(!r.out.contains("Total 6.0 GB"), "{}", r.out);

        // No terminal: the summary is still shown (on stderr), then the refusal; exit 2, nothing removed.
        let r = run_to_vec(&flags(false, false, false, true), None, vec![], false);
        assert_eq!((r.code, r.removals), (EXIT_USAGE, 0));
        assert_eq!(r.out, "");
        assert!(r.err.contains(SUMMARY) && r.err.contains("not a terminal: pass --yes"), "{}", r.err);
    }

    #[test]
    fn without_a_question_the_summary_stays_on_stdout() {
        let r = run_to_vec(&flags(true, false, false, true), None, vec![], false);
        assert_eq!((r.code, r.removals), (EXIT_OK, 1));
        assert!(r.out.contains(SUMMARY) && r.out.contains("Freed 6.0 GB"), "{}", r.out);
        let r = run_to_vec(&flags(false, true, false, true), None, vec![], false);
        assert_eq!((r.code, r.removals, r.dry_removals), (EXIT_OK, 0, 1));
        assert!(r.out.contains(SUMMARY) && r.out.contains("Dry run: nothing was removed."), "{}", r.out);
    }

    #[test]
    fn a_partial_removal_exits_3_and_an_eof_or_a_wrong_answer_removes_nothing() {
        let r = run_to_vec(&flags(true, false, false, true), None, vec![], true);
        assert_eq!(r.code, EXIT_PARTIAL);
        assert!(r.out.contains("failed") && r.out.contains("in use"), "{}", r.out);
        // A terminal that hits end of input, a typo and an empty line are all a no.
        for answer in ["", "yep\n", "\n"] {
            let r = run_to_vec(&flags(false, false, false, true), Some(answer), vec![], false);
            assert_eq!((r.code, r.removals), (EXIT_OK, 0), "{answer:?}");
        }
    }

    #[test]
    fn a_recycle_bin_that_is_too_small_exits_1_and_removes_nothing() {
        let short = vec![('D', 6 * GB, 1024)];
        for f in [flags(true, false, false, false), flags(false, false, false, false)] {
            let r = run_to_vec(&f, Some("y\n"), short.clone(), false);
            assert_eq!((r.code, r.removals), (EXIT_ERROR, 0));
            let all = format!("{}{}", r.out, r.err);
            assert!(all.contains("Recycle Bin on D: is too small (6.0 GB to move, 1 KB free)"), "{all}");
        }
    }

    #[test]
    fn a_report_that_cannot_be_written_after_the_removal_reaches_stderr_and_the_exit_code_stays_honest() {
        // The removal went well, the report did not: an error, and the outcome is on stderr.
        let r = run_clean(&flags(true, false, false, true), None, vec![], false, &mut FlakyOut { ok: 1 });
        assert_eq!((r.code, r.removals), (EXIT_ERROR, 1));
        assert!(r.err.contains("removed") && r.err.contains("Freed 6.0 GB"), "{}", r.err);
        // Part of it was not removed: still 3, and the person sees which.
        let r = run_clean(&flags(true, false, false, true), None, vec![], true, &mut FlakyOut { ok: 1 });
        assert_eq!((r.code, r.removals), (EXIT_PARTIAL, 1));
        assert!(r.err.contains("failed") && r.err.contains("in use"), "{}", r.err);
        // With --json the outcome text is on stderr already; a failing stdout changes only the code.
        let r = run_clean(&flags(true, false, true, true), None, vec![], true, &mut FlakyOut { ok: 0 });
        assert_eq!((r.code, r.removals), (EXIT_PARTIAL, 1));
        assert!(r.err.contains("failed"), "{}", r.err);
        let r = run_clean(&flags(true, false, true, true), None, vec![], false, &mut FlakyOut { ok: 0 });
        assert_eq!(r.code, EXIT_ERROR);
    }

    #[test]
    fn a_failing_stdout_before_any_removal_is_an_error_and_removes_nothing() {
        // `--yes`: the summary is the first thing written; if it fails nothing has been removed.
        let r = run_clean(&flags(true, false, false, true), None, vec![], false, &mut FlakyOut { ok: 0 });
        assert_eq!(r.code, EXIT_ERROR);
        assert_eq!(r.removals, 0);
    }

    #[test]
    fn write_failed_code_is_3_only_for_a_partial_removal() {
        assert_eq!(write_failed_code(None), EXIT_ERROR);
    }
}
