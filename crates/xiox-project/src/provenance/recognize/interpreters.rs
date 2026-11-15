//! Language interpreters with source on the command line: `python -c`, `node -e`, a heredoc, or a
//! script file the same command line just wrote. The source is scanned for calls with literal paths.

use super::packages::{self, Scaffold};
use super::{Context, Invocation, Recognizer, path_op, text_args, words};
use crate::provenance::{Lang, OpKind, RawOp, posix, scan_script};

pub(super) struct Interpreters;

/// Every name that runs source: one with a heredoc is a script Xiox does not interpret.
const SCRIPT_RUNNERS: [&str; 14] =
    ["python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php", "bash", "sh", "zsh", "pwsh", "powershell"];
const PYTHON_SOURCE: [&str; 1] = ["-c"];
const JAVASCRIPT_SOURCE: [&str; 4] = ["-e", "--eval", "-p", "--print"];

pub(super) fn runs_scripts(name: &str) -> bool {
    SCRIPT_RUNNERS.contains(&name)
}

impl Recognizer for Interpreters {
    fn names(&self) -> &'static [&'static str] {
        &["python", "python3", "py", "node", "deno", "bun"]
    }

    fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp> {
        let words = words(inv.args);
        match inv.name.as_str() {
            "python" | "python3" | "py" => python(inv, &words, ctx),
            // `bun create` and `bun init` scaffold a project like the package managers do.
            "bun" if matches!(words.first(), Some(&("create" | "init"))) => packages::create_init(inv.args, Scaffold::Bun),
            _ => run_source(inv, &words, &JAVASCRIPT_SOURCE, Lang::JavaScript, ctx),
        }
    }
}

fn python(inv: &Invocation, words: &[&str], ctx: &mut Context) -> Vec<RawOp> {
    if let Some(at) = words.iter().position(|word| *word == "-m") {
        if matches!(words.get(at + 1), Some(&("venv" | "virtualenv"))) {
            let dirs = posix(&text_args(&words[at + 2..]), &["--prompt"]).positional;
            return dirs.into_iter().map(|dir| path_op(OpKind::Create, None, Some(dir))).collect();
        }
    }
    run_source(inv, words, &PYTHON_SOURCE, Lang::Python, ctx)
}

/// The operations of the source `inv` runs; reading it marks the call as a script.
fn run_source(inv: &Invocation, words: &[&str], inline: &[&str], lang: Lang, ctx: &mut Context) -> Vec<RawOp> {
    let Some(code) = source(inv, words, inline, ctx) else { return Vec::new() };
    ctx.flags.script = true;
    let scan = scan_script(&code, lang);
    ctx.unresolved += scan.unresolved;
    scan.ops
}

/// An inline string, else the heredoc, else a script file the command line wrote earlier.
fn source(inv: &Invocation, words: &[&str], inline: &[&str], ctx: &Context) -> Option<String> {
    let flagged = words.iter().position(|word| inline.contains(word)).and_then(|at| words.get(at + 1));
    flagged
        .map(|code| code.to_string())
        .or_else(|| inv.heredoc.map(str::to_owned))
        .or_else(|| words.iter().find_map(|word| ctx.scripts.get(*word).cloned()))
}
