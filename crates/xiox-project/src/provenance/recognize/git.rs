//! `git`: clones, worktrees and new repositories, with the directory `-C` named.

use super::{Context, Invocation, Recognizer, text_args, words};
use crate::provenance::{OpKind, RawOp, posix};

pub(super) struct Git;

/// Clone options that take a value, so the value is not read as the source or the directory.
const CLONE_VALUES: [&str; 21] = [
    "-b",
    "--branch",
    "--depth",
    "-o",
    "--origin",
    "-u",
    "--upload-pack",
    "--reference",
    "--reference-if-able",
    "--separate-git-dir",
    "-c",
    "--config",
    "--filter",
    "-j",
    "--jobs",
    "--template",
    "--server-option",
    "--shallow-since",
    "--shallow-exclude",
    "--bundle-uri",
    "--revision",
];
const WORKTREE_VALUES: [&str; 3] = ["-b", "-B", "--reason"];
const INIT_VALUES: [&str; 5] = ["--template", "--separate-git-dir", "-b", "--initial-branch", "--object-format"];
/// Global options that take a value, other than `-C`.
const GLOBAL_VALUES: [&str; 4] = ["-c", "--git-dir", "--work-tree", "--namespace"];

impl Recognizer for Git {
    fn names(&self) -> &'static [&'static str] {
        &["git"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        let words = words(inv.args);
        let (base, at) = global_options(&words);
        let Some(&subcommand) = words.get(at) else { return Vec::new() };
        let rest = &words[at + 1..];
        if rest.iter().any(|word| matches!(*word, "--help" | "-h")) {
            return Vec::new();
        }
        let op = |kind, src: Option<&str>, dst: Option<&str>| RawOp::Path {
            kind,
            src: src.map(str::to_owned),
            dst: dst.map(str::to_owned),
            base: base.map(str::to_owned),
            script: false,
        };
        match subcommand {
            "clone" => {
                let pos = posix(&text_args(rest), &CLONE_VALUES).positional;
                pos.first().map(|src| op(OpKind::Clone, Some(src), pos.get(1).map(String::as_str))).into_iter().collect()
            }
            "worktree" if rest.first() == Some(&"add") => {
                let pos = posix(&text_args(&rest[1..]), &WORKTREE_VALUES).positional;
                pos.first().map(|dst| op(OpKind::Worktree, None, Some(dst))).into_iter().collect()
            }
            "init" => {
                let pos = posix(&text_args(rest), &INIT_VALUES).positional;
                pos.first().map(|dst| op(OpKind::Create, None, Some(dst))).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }
}

/// The directory `-C` names and the index of the subcommand, after the global options.
fn global_options<'a>(words: &[&'a str]) -> (Option<&'a str>, usize) {
    let mut base = None;
    let mut at = 0;
    while at < words.len() && words[at].starts_with('-') {
        match words[at] {
            "-C" if at + 1 < words.len() => {
                base = Some(words[at + 1]);
                at += 2;
            }
            option if GLOBAL_VALUES.contains(&option) && at + 1 < words.len() => at += 2,
            _ => at += 1,
        }
    }
    (base, at)
}
