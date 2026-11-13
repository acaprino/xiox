//! Command line of `xiox`: a small hand-written parser, no dependency.

use crate::size::parse_size;
use xiox_project::Category;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Analyze {
        project: Option<PathBuf>,
        json: bool,
        no_drives: bool,
    },
    Clean {
        project: Option<PathBuf>,
        categories: Vec<Category>,
        include: Vec<PathBuf>,
        permanent: bool,
        yes: bool,
        dry_run: bool,
        json: bool,
        no_drives: bool,
    },
    Observe {
        project: Option<PathBuf>,
        interval_secs: u64,
        grow_threshold: u64,
        worktree_threshold: usize,
        no_drives: bool,
        mcp: bool,
    },
    Help,
    Version,
}

pub const USAGE: &str = "\
Usage:
  xiox analyze [<project>] [--json] [--no-drives]
  xiox clean   [<project>] [--builds] [--worktrees] [--logs] [--sessions]
               [--include <path>]... [--permanent] [--yes] [--dry-run] [--json] [--no-drives]
  xiox observe [<project>] [--interval <seconds>] [--grow-threshold <size>]
               [--worktree-threshold <n>] [--no-drives] [--mcp]
  xiox --help | --version";

pub fn help() -> String {
    format!(
        "\
xiox: find and remove everything on this PC that belongs to one project.

{USAGE}

<project> is any folder of the project (default: the current folder). A git
repository is resolved to its main working tree.

Commands:
  analyze   List build folders, worktrees, copies of the repository, folders with
            the project's name, log files and archived agent sessions, with their
            sizes. Removes nothing.
  clean     Remove what the flags name, after showing a summary and asking for
            confirmation. Without a category flag or --include it removes nothing
            and prints the listing like analyze.
  observe   Watch one project: take a snapshot, check again at intervals and tell what
            appeared since (one JSON object per line on stdout; with --mcp, a read-only
            MCP server on stdio). Reads `report` and `stop` lines on stdin and ends when
            stdin closes. Removes nothing.

Flags:
  --builds         clean: the recommended build folders
  --worktrees      clean: the recommended linked worktrees (removed by git, for good)
  --logs           clean: the recommended log files
  --sessions       clean: every archived agent session that is not blocked
  --include <path> clean: one more item of the listing, of any level (repeatable).
                   Copies and name-matched folders can only be taken this way.
  --permanent      clean: delete for good instead of moving to the Recycle Bin
  --yes            clean: do not ask for confirmation
  --dry-run        clean: do everything except deleting, and say what would be freed
  --json           print a single JSON object (schema 1) on stdout; messages go to stderr
  --no-drives      search only next to the project, not on every fixed drive
  --interval <s>   observe: seconds between checks (default 600, from 60 to 2592000)
  --grow-threshold <size>
                   observe: growth since the start that raises `attention`
                   (default 2GB; units B, KB, MB, GB, TB, binary: 1.5GB, 500MiB)
  --worktree-threshold <n>
                   observe: new worktrees since the start that raise `attention` (default 3)
  --mcp            observe: speak MCP on stdin/stdout instead of JSON lines
  -h, --help       show this help
  -V, --version    show the version

Exit codes: 0 done, 1 error, 2 wrong usage, 3 partly cleaned (an item failed or was skipped)."
    )
}

#[derive(Clone, Copy)]
enum Which {
    Analyze,
    Clean,
    Observe,
}

/// `observe` defaults and limits.
const DEFAULT_INTERVAL_SECS: u64 = 600;
const MIN_INTERVAL_SECS: u64 = 60;
/// The longest accepted interval (30 days): a larger one is a mistake, and `Instant` arithmetic
/// must never be handed a near-`u64::MAX` duration.
pub const MAX_INTERVAL_SECS: u64 = 30 * 24 * 60 * 60;
const DEFAULT_GROW_THRESHOLD: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_WORKTREE_THRESHOLD: usize = 3;

const NO_COPIES_FLAG: &str = "--copies/--named do not exist: name each folder with --include <path>";
const INCLUDE_NEEDS_PATH: &str = "--include needs a path: --include <path>";

/// The flags that take a value: `--flag value` or `--flag=value`.
fn takes_value(flag: &str) -> bool {
    matches!(flag, "--include" | "--interval" | "--grow-threshold" | "--worktree-threshold")
}

/// The value of `--flag value` or `--flag=value`; a following `--flag` is not a value.
fn flag_value<'a>(
    flag: &str,
    inline: Option<String>,
    it: &mut impl Iterator<Item = &'a String>,
    what: &str,
) -> Result<String, String> {
    match inline {
        Some(v) if !v.is_empty() => Ok(v),
        Some(_) => Err(format!("{flag} needs {what}: {flag} <value>")),
        None => match it.next() {
            Some(v) if !v.starts_with("--") => Ok(v.clone()),
            _ => Err(format!("{flag} needs {what}: {flag} <value>")),
        },
    }
}

/// Parse the arguments after the program name. `Err` is a usage message (exit code 2).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter();
    let first = it.next().ok_or_else(|| "no command given (expected analyze, clean or observe)".to_string())?;
    let (which, name) = match first.as_str() {
        "-h" | "--help" => return Ok(Command::Help),
        "-V" | "--version" => return Ok(Command::Version),
        "analyze" => (Which::Analyze, "analyze"),
        "clean" => (Which::Clean, "clean"),
        "observe" => (Which::Observe, "observe"),
        other => return Err(format!("unknown command: {other} (expected analyze, clean or observe)")),
    };

    let mut project: Option<PathBuf> = None;
    let mut categories: Vec<Category> = Vec::new();
    let mut include: Vec<PathBuf> = Vec::new();
    let (mut permanent, mut yes, mut dry_run, mut json, mut no_drives) = (false, false, false, false, false);
    let mut interval_secs = DEFAULT_INTERVAL_SECS;
    let mut grow_threshold = DEFAULT_GROW_THRESHOLD;
    let mut worktree_threshold = DEFAULT_WORKTREE_THRESHOLD;
    let mut mcp = false;
    let only_clean = |flag: &str| -> Result<(), String> {
        match which {
            Which::Clean => Ok(()),
            Which::Analyze | Which::Observe => Err(format!("{flag} is only valid for clean")),
        }
    };
    let only_observe = |flag: &str| -> Result<(), String> {
        match which {
            Which::Observe => Ok(()),
            Which::Analyze | Which::Clean => Err(format!("{flag} is only valid for observe")),
        }
    };
    let add = |c: Category, categories: &mut Vec<Category>| {
        if !categories.contains(&c) {
            categories.push(c);
        }
    };

    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if matches!(flag, "--copies" | "--named") {
            return Err(NO_COPIES_FLAG.to_string());
        }
        if inline.is_some() && !takes_value(flag) {
            return Err(format!("{flag} does not take a value"));
        }
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--json" => {
                if matches!(which, Which::Observe) {
                    return Err("--json is not valid for observe (it always prints JSON lines)".to_string());
                }
                json = true;
            }
            "--no-drives" => no_drives = true,
            "--mcp" => {
                only_observe(flag)?;
                mcp = true;
            }
            "--interval" => {
                only_observe(flag)?;
                let value = flag_value(flag, inline, &mut it, "a number of seconds")?;
                interval_secs = value
                    .parse::<u64>()
                    .map_err(|_| format!("--interval needs a whole number of seconds, not \"{value}\""))?;
                if interval_secs < MIN_INTERVAL_SECS {
                    return Err(format!("--interval must be at least {MIN_INTERVAL_SECS} seconds, not {interval_secs}"));
                }
                if interval_secs > MAX_INTERVAL_SECS {
                    return Err(format!("--interval must be at most {MAX_INTERVAL_SECS} seconds (30 days), not {interval_secs}"));
                }
            }
            "--grow-threshold" => {
                only_observe(flag)?;
                let value = flag_value(flag, inline, &mut it, "a size")?;
                grow_threshold = parse_size(&value).map_err(|e| format!("--grow-threshold: {e}"))?;
            }
            "--worktree-threshold" => {
                only_observe(flag)?;
                let value = flag_value(flag, inline, &mut it, "a number")?;
                worktree_threshold = match value.parse::<usize>() {
                    Ok(n) if n >= 1 => n,
                    _ => return Err(format!("--worktree-threshold needs a whole number of at least 1, not \"{value}\"")),
                };
            }
            "--builds" => {
                only_clean(flag)?;
                add(Category::Builds, &mut categories);
            }
            "--worktrees" => {
                only_clean(flag)?;
                add(Category::Worktrees, &mut categories);
            }
            "--logs" => {
                only_clean(flag)?;
                add(Category::Logs, &mut categories);
            }
            "--sessions" => {
                only_clean(flag)?;
                add(Category::Sessions, &mut categories);
            }
            "--permanent" => {
                only_clean(flag)?;
                permanent = true;
            }
            "--yes" => {
                only_clean(flag)?;
                yes = true;
            }
            "--dry-run" => {
                only_clean(flag)?;
                dry_run = true;
            }
            "--include" => {
                only_clean(flag)?;
                let value = match inline {
                    Some(v) => v,
                    None => match it.next() {
                        Some(v) if !v.starts_with("--") => v.clone(),
                        _ => return Err(INCLUDE_NEEDS_PATH.to_string()),
                    },
                };
                if value.is_empty() {
                    return Err(INCLUDE_NEEDS_PATH.to_string());
                }
                include.push(PathBuf::from(value));
            }
            f if f.starts_with('-') && f.len() > 1 => return Err(format!("unknown flag: {f}")),
            _ => {
                if project.is_some() {
                    return Err(format!("unexpected extra argument: {arg} ({name} takes one project)"));
                }
                project = Some(PathBuf::from(arg));
            }
        }
    }

    Ok(match which {
        Which::Observe => Command::Observe { project, interval_secs, grow_threshold, worktree_threshold, no_drives, mcp },
        Which::Analyze => Command::Analyze { project, json, no_drives },
        Which::Clean => Command::Clean { project, categories, include, permanent, yes, dry_run, json, no_drives },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn analyze_defaults() {
        assert_eq!(p(&["analyze"]), Ok(Command::Analyze { project: None, json: false, no_drives: false }));
        assert_eq!(
            p(&["analyze", "D:\\x", "--json", "--no-drives"]),
            Ok(Command::Analyze { project: Some(PathBuf::from("D:\\x")), json: true, no_drives: true })
        );
    }

    #[test]
    fn clean_with_everything() {
        let got = p(&[
            "clean",
            "D:\\proj",
            "--builds",
            "--worktrees",
            "--logs",
            "--sessions",
            "--include",
            "D:\\a",
            "--include=D:\\b",
            "--permanent",
            "--yes",
            "--dry-run",
            "--json",
            "--no-drives",
        ]);
        assert_eq!(
            got,
            Ok(Command::Clean {
                project: Some(PathBuf::from("D:\\proj")),
                categories: vec![Category::Builds, Category::Worktrees, Category::Logs, Category::Sessions],
                include: vec![PathBuf::from("D:\\a"), PathBuf::from("D:\\b")],
                permanent: true,
                yes: true,
                dry_run: true,
                json: true,
                no_drives: true,
            })
        );
        // Flags may come before the project; a repeated category flag counts once.
        let got = p(&["clean", "--builds", "--builds", "D:\\proj"]);
        assert_eq!(
            got,
            Ok(Command::Clean {
                project: Some(PathBuf::from("D:\\proj")),
                categories: vec![Category::Builds],
                include: vec![],
                permanent: false,
                yes: false,
                dry_run: false,
                json: false,
                no_drives: false,
            })
        );
    }

    #[test]
    fn help_and_version_are_recognised_anywhere() {
        assert_eq!(p(&["--help"]), Ok(Command::Help));
        assert_eq!(p(&["-h"]), Ok(Command::Help));
        assert_eq!(p(&["analyze", "--help"]), Ok(Command::Help));
        assert_eq!(p(&["--version"]), Ok(Command::Version));
        assert_eq!(p(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn copies_flag_explains_include() {
        for f in ["--copies", "--named", "--copies=x", "--named=x"] {
            let e = p(&["clean", f]).unwrap_err();
            assert!(e.contains("--include <path>"), "{e}");
            assert!(e.contains("do not exist"), "{e}");
        }
    }

    #[test]
    fn unknown_flag_missing_value_second_positional_and_no_command_are_usage_errors() {
        assert!(p(&["analyze", "--nope"]).unwrap_err().contains("unknown flag: --nope"));
        assert!(p(&["clean", "--include"]).unwrap_err().contains("--include <path>"));
        assert!(p(&["clean", "--include", "--yes"]).unwrap_err().contains("--include <path>"));
        assert!(p(&["analyze", "a", "b"]).unwrap_err().contains("unexpected extra argument: b"));
        assert!(p(&[]).unwrap_err().contains("no command"));
        assert!(p(&["frobnicate"]).unwrap_err().contains("unknown command: frobnicate"));
        // Clean-only flags are refused by analyze; `--json=1` has no value to take.
        assert!(p(&["analyze", "--builds"]).unwrap_err().contains("only valid for clean"));
        assert!(p(&["analyze", "--json=1"]).unwrap_err().contains("does not take a value"));
        // Observe-only flags are refused by analyze and clean.
        assert!(p(&["analyze", "--interval", "120"]).unwrap_err().contains("only valid for observe"));
        assert!(p(&["clean", "--mcp"]).unwrap_err().contains("only valid for observe"));
        assert!(p(&["observe", "--json"]).unwrap_err().contains("not valid for observe"));
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn observe_defaults() {
        assert_eq!(
            p(&["observe"]),
            Ok(Command::Observe {
                project: None,
                interval_secs: 600,
                grow_threshold: 2 * GIB,
                worktree_threshold: 3,
                no_drives: false,
                mcp: false,
            })
        );
    }

    #[test]
    fn observe_all_flags() {
        let want = Command::Observe {
            project: Some(PathBuf::from("D:\\proj")),
            interval_secs: 120,
            grow_threshold: 500 * 1024 * 1024,
            worktree_threshold: 5,
            no_drives: true,
            mcp: true,
        };
        let got = p(&[
            "observe",
            "D:\\proj",
            "--interval",
            "120",
            "--grow-threshold",
            "500MB",
            "--worktree-threshold",
            "5",
            "--no-drives",
            "--mcp",
        ]);
        assert_eq!(got, Ok(want.clone()));
        // `--flag=value` and flags before the project.
        let got = p(&[
            "observe",
            "--mcp",
            "--interval=120",
            "--grow-threshold=500MB",
            "--worktree-threshold=5",
            "--no-drives",
            "D:\\proj",
        ]);
        assert_eq!(got, Ok(want));
        // The minimum itself is accepted.
        assert!(matches!(p(&["observe", "--interval", "60"]), Ok(Command::Observe { interval_secs: 60, .. })));
    }

    #[test]
    fn observe_interval_below_60_is_usage_error() {
        for v in ["59", "1", "0", "-5", "abc", "1.5", ""] {
            let e = p(&["observe", "--interval", v]).unwrap_err();
            assert!(e.contains("--interval"), "{v}: {e}");
        }
        assert!(p(&["observe", "--interval", "59"]).unwrap_err().contains("at least 60"));
        assert!(p(&["observe", "--interval"]).unwrap_err().contains("--interval"));
        assert!(p(&["observe", "--interval", "--mcp"]).unwrap_err().contains("--interval"));
    }

    #[test]
    fn observe_interval_above_30_days_is_usage_error() {
        assert!(matches!(
            p(&["observe", "--interval", "2592000"]),
            Ok(Command::Observe { interval_secs: 2_592_000, .. })
        ));
        for v in ["2592001", "18446744073709551615", "9999999999999"] {
            let e = p(&["observe", "--interval", v]).unwrap_err();
            assert!(e.contains("--interval") && e.contains("at most"), "{v}: {e}");
        }
        assert!(p(&["observe", "--interval=2592001"]).unwrap_err().contains("at most"));
        // A number that does not even fit in u64 is a usage error too.
        assert!(p(&["observe", "--interval", "18446744073709551616"]).unwrap_err().contains("--interval"));
    }

    #[test]
    fn observe_bad_size_and_zero_worktree_threshold_are_usage_errors() {
        for v in ["0", "banana", "-1GB", "", "2XB"] {
            let e = p(&["observe", "--grow-threshold", v]).unwrap_err();
            assert!(e.contains("--grow-threshold"), "{v}: {e}");
        }
        for v in ["0", "x", "-1", "1.5", ""] {
            let e = p(&["observe", "--worktree-threshold", v]).unwrap_err();
            assert!(e.contains("--worktree-threshold"), "{v}: {e}");
        }
        assert!(p(&["observe", "--grow-threshold"]).unwrap_err().contains("--grow-threshold"));
        assert!(p(&["observe", "--worktree-threshold"]).unwrap_err().contains("--worktree-threshold"));
    }

    #[test]
    fn observe_accepts_project_path_and_rejects_second_positional() {
        match p(&["observe", "D:\\proj"]) {
            Ok(Command::Observe { project, .. }) => assert_eq!(project, Some(PathBuf::from("D:\\proj"))),
            other => panic!("{other:?}"),
        }
        let e = p(&["observe", "a", "b"]).unwrap_err();
        assert!(e.contains("unexpected extra argument: b"), "{e}");
        assert!(p(&["observe", "--nope"]).unwrap_err().contains("unknown flag: --nope"));
        assert!(p(&["observe", "--help"]) == Ok(Command::Help));
    }
}
