//! Entry point of the commands: arguments in, exit code out. Results go to `out`, everything
//! else (progress, notes, errors) to `err`.

use crate::args::{self, Command};
use crate::{json, render};
use dc_project::{Error, Options, ProjectScan};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
#[allow(dead_code)] // returned by `clean`
pub const EXIT_PARTIAL: i32 = 3;

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match args::parse(args) {
        Err(msg) => {
            let _ = writeln!(err, "error: {msg}\n\n{}", args::USAGE);
            EXIT_USAGE
        }
        Ok(Command::Help) => {
            let _ = writeln!(out, "{}", args::help());
            EXIT_OK
        }
        Ok(Command::Version) => {
            let _ = writeln!(out, "xiox {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Ok(Command::Analyze { project, json, no_drives }) => analyze(project, json, no_drives, out, err),
        Ok(Command::Clean { .. }) => {
            let _ = writeln!(err, "error: clean is not implemented yet");
            EXIT_USAGE
        }
    }
}

/// Test switches, read by debug builds only (a release `xiox` ignores both variables):
/// `XIOX_SEARCH_ROOTS` (paths separated by `;`) replaces the fixed drives and the home folder
/// as the places searched for copies and name-matched folders; `XIOX_NO_SESSIONS=1` skips the
/// session listing of this PC (no cache written, no `sessions` items). Either one switches the
/// analysis to the test mode: roots default to none, sessions to the real ones.
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

fn analyze(project: Option<PathBuf>, json: bool, no_drives: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let path = project.unwrap_or_else(|| PathBuf::from("."));
    let opts = Options { drives: !no_drives, measure: true, admin: dc_project::is_elevated() };
    let _ = writeln!(err, "Analyzing {} ...", path.display());
    let scan = match scan_project(&path, &opts) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            return EXIT_ERROR;
        }
    };
    if json {
        let _ = writeln!(out, "{}", json::scan_json(&scan));
    } else {
        let _ = write!(out, "{}", render::render_scan(&scan));
    }
    EXIT_OK
}
