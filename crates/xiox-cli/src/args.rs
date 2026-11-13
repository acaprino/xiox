//! Command line of `xiox`: a small hand-written parser, no dependency.

use dc_project::Category;
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
    Help,
    Version,
}

pub const USAGE: &str = "\
Usage:
  xiox analyze [<project>] [--json] [--no-drives]
  xiox clean   [<project>] [--builds] [--worktrees] [--logs] [--sessions]
               [--include <path>]... [--permanent] [--yes] [--dry-run] [--json] [--no-drives]
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
  -h, --help       show this help
  -V, --version    show the version

Exit codes: 0 done, 1 error, 2 wrong usage, 3 partly cleaned (an item failed or was skipped)."
    )
}

#[derive(Clone, Copy)]
enum Which {
    Analyze,
    Clean,
}

const NO_COPIES_FLAG: &str = "--copies/--named do not exist: name each folder with --include <path>";
const INCLUDE_NEEDS_PATH: &str = "--include needs a path: --include <path>";

/// Parse the arguments after the program name. `Err` is a usage message (exit code 2).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter();
    let first = it.next().ok_or_else(|| "no command given (expected analyze or clean)".to_string())?;
    let (which, name) = match first.as_str() {
        "-h" | "--help" => return Ok(Command::Help),
        "-V" | "--version" => return Ok(Command::Version),
        "analyze" => (Which::Analyze, "analyze"),
        "clean" => (Which::Clean, "clean"),
        other => return Err(format!("unknown command: {other} (expected analyze or clean)")),
    };

    let mut project: Option<PathBuf> = None;
    let mut categories: Vec<Category> = Vec::new();
    let mut include: Vec<PathBuf> = Vec::new();
    let (mut permanent, mut yes, mut dry_run, mut json, mut no_drives) = (false, false, false, false, false);
    let only_clean = |flag: &str| -> Result<(), String> {
        match which {
            Which::Clean => Ok(()),
            Which::Analyze => Err(format!("{flag} is only valid for clean")),
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
        if inline.is_some() && flag != "--include" {
            return Err(format!("{flag} does not take a value"));
        }
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--json" => json = true,
            "--no-drives" => no_drives = true,
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
    }
}
