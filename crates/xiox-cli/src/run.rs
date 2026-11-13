//! Entry point of the commands: arguments in, exit code out. Results go to `out`, everything
//! else (progress, notes, errors, the confirmation question) to `err`.

use crate::args::{self, Command};
use crate::{json, render};
use dc_model::RemoveMode;
use dc_project::{Category, Chosen, CleanOutcome, CleanRequest, Error, Options, ProjectScan};
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
        Some(t) if t.no_sessions => dc_project::analyze_project_without_sessions(path, opts, &t.search_roots),
        Some(t) => dc_project::analyze_project_in(path, opts, &t.search_roots),
        None => dc_project::analyze_project(path, opts),
    }
}

/// The removal, in the same world as [`scan_project`]: the test switches that fake the analysis
/// fake the cleaning too, so a debug build driven by them never reads a real session.
fn clean_project(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, dry_run: bool) -> CleanOutcome {
    match test_switches() {
        Some(t) if t.no_sessions => {
            dc_project::clean_without_sessions(scan, chosen, mode, dry_run, &t.search_roots)
        }
        Some(t) => dc_project::clean_in(scan, chosen, mode, dry_run, &t.search_roots),
        None => dc_project::clean(scan, chosen, mode, dry_run),
    }
}

/// Analyze the project (the current folder by default), telling the person on `err`.
fn analyze_path(project: Option<PathBuf>, no_drives: bool, err: &mut dyn Write) -> Result<ProjectScan, i32> {
    let path = project.unwrap_or_else(|| PathBuf::from("."));
    let opts = Options { drives: !no_drives, measure: true, admin: dc_project::is_elevated() };
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

/// Write a result to stdout. A closed pipe (`| head`) is not an error; any other failure is.
pub fn write_result(out: &mut dyn Write, err: &mut dyn Write, text: &str) -> Result<(), i32> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => {
            let _ = writeln!(err, "error: cannot write to the output: {e}");
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

    /// End of a run that has a result: with `--json` the one object on stdout (`outcome` is
    /// `None` when nothing was run). Returns the exit code.
    fn conclude(&mut self, scan: &ProjectScan, outcome: Option<&CleanOutcome>, code: i32) -> i32 {
        if !self.json {
            return code;
        }
        match json::outcome_json(scan, outcome) {
            Ok(v) => match write_result(self.out, self.err, &format!("{v}\n")) {
                Ok(()) => code,
                Err(c) => c,
            },
            Err(e) => {
                self.fail(&format!("error: cannot write the result as JSON: {e}"));
                if let Some(o) = outcome {
                    // Something may have been removed: say what, in plain text.
                    let _ = self.err.write_all(render::render_outcome(o).as_bytes());
                }
                EXIT_ERROR
            }
        }
    }
}

/// `y` or `yes`, any case, around any whitespace. Anything else (an empty line included) is no.
fn is_yes(answer: &str) -> bool {
    let a = answer.trim();
    a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes")
}

/// Ask `Remove <n> items (<size>)? [y/N] ` on `err` and read one line from `input`. End of input
/// or a read error is a no.
fn confirm(err: &mut dyn Write, input: &mut dyn BufRead, chosen: &Chosen) -> bool {
    let n = chosen.keys.len();
    let _ = write!(
        err,
        "Remove {n} item{} ({})? [y/N] ",
        if n == 1 { "" } else { "s" },
        render::fmt_size(chosen.bytes)
    );
    let _ = err.flush();
    let mut line = String::new();
    input.read_line(&mut line).is_ok() && is_yes(&line)
}

fn clean(f: &CleanFlags, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut sink = Sink { out, err, json: f.json };
    match clean_run(f, &mut sink) {
        Ok(code) | Err(code) => code,
    }
}

/// The steps of `clean`, in order; `Err` is a failure to write the result (already reported).
fn clean_run(f: &CleanFlags, s: &mut Sink) -> Result<i32, i32> {
    // 1. Analyze.
    let scan = match analyze_path(f.project.clone(), f.no_drives, s.err) {
        Ok(scan) => scan,
        Err(code) => return Ok(code),
    };
    // A path JSON cannot carry must stop the run before anything is removed, not after.
    if f.json {
        if let Err(e) = json::scan_json(&scan) {
            s.fail(&format!("error: cannot write the result as JSON: {e}"));
            return Ok(EXIT_ERROR);
        }
    }
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
    let chosen = match dc_project::select(&scan, &request) {
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
    let shortfall = dc_project::recycle_shortfall(&scan, &chosen, mode);
    s.say(&render::render_summary(&scan, &chosen, mode, &shortfall))?;
    if !shortfall.is_empty() {
        s.fail("error: not enough room in the Recycle Bin: nothing was removed");
        return Ok(EXIT_ERROR);
    }

    // 4. Dry run: every check, no removal.
    if f.dry_run {
        let outcome = clean_project(&scan, &chosen, mode, true);
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
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            s.fail("error: not a terminal: pass --yes to remove without asking");
            return Ok(EXIT_USAGE);
        }
        if !confirm(s.err, &mut stdin.lock(), &chosen) {
            s.say("Cancelled.\n")?;
            return Ok(s.conclude(&scan, None, EXIT_OK));
        }
    }

    // 6. Remove, and say what happened.
    let outcome = clean_project(&scan, &chosen, mode, false);
    let code = if outcome.partial() { EXIT_PARTIAL } else { EXIT_OK };
    s.say(&render::render_outcome(&outcome))?;
    Ok(s.conclude(&scan, Some(&outcome), code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_project::Chosen;

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
        assert!(confirm(&mut err, &mut "yes\n".as_bytes(), &chosen(3, 6 * 1024 * 1024 * 1024)));
        assert_eq!(String::from_utf8(err).unwrap(), "Remove 3 items (6.0 GB)? [y/N] ");
        let mut err = Vec::new();
        assert!(!confirm(&mut err, &mut "n\n".as_bytes(), &chosen(1, 512)));
        assert_eq!(String::from_utf8(err).unwrap(), "Remove 1 item (512 B)? [y/N] ");
        // End of input is a no, never a yes.
        assert!(!confirm(&mut Vec::new(), &mut "".as_bytes(), &chosen(1, 1)));
    }
}
