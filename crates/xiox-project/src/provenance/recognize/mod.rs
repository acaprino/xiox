//! Recognizers: one strategy per command family, looked up by command name. Each reads the
//! operations a single simple command performs; the pipeline (Task 8) walks the statements.

mod archives;
mod directories;
mod git;
mod interpreters;
mod location;
mod output_flags;
mod packages;
mod shells;
mod transfer;
mod writers;

use std::collections::HashMap;
use std::sync::OnceLock;

use super::args::{PsParams, powershell};
use super::budget::Budget;
use super::command::Arg;
use super::dialect::Shell;
use super::op::{Flags, OpKind, RawOp};

use archives::Archives;
use directories::Directories;
use git::Git;
use interpreters::Interpreters;
use location::Location;
use packages::Packages;
use shells::Shells;
use transfer::Transfer;
use writers::Writers;

/// A simple command as a recognizer sees it.
pub struct Invocation<'a> {
    /// Basename of the first word, lowercase, without `.exe`.
    pub name: String,
    pub args: &'a [Arg],
    pub shell: Shell,
    pub heredoc: Option<&'a str>,
}

/// What recognizers record besides operations.
#[derive(Debug, Default)]
pub struct Context {
    pub flags: Flags,
    pub unresolved: usize,
    /// Script bodies written by earlier commands, by file name.
    pub scripts: HashMap<String, String>,
    /// What the whole extraction may still spend, shared with nested shells.
    pub budget: Budget,
}

/// Reads the operations of one family of commands.
pub trait Recognizer: Sync {
    /// The command names this family owns, lowercase.
    fn names(&self) -> &'static [&'static str];
    fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp>;
}

/// Commands that repeat their body: the paths they touch are not literal.
const LOOPS: [&str; 4] = ["for", "foreach", "foreach-object", "%"];

/// Command names mapped to the recognizer that owns them.
pub struct Registry {
    by_name: HashMap<&'static str, &'static dyn Recognizer>,
}

impl Registry {
    /// Every recognizer family, each owning its names exclusively.
    pub fn families() -> &'static [&'static dyn Recognizer] {
        &[&Location, &Directories, &Transfer, &Archives, &Writers, &Git, &Packages, &Shells, &Interpreters]
    }

    pub fn standard() -> &'static Registry {
        static STANDARD: OnceLock<Registry> = OnceLock::new();
        STANDARD.get_or_init(|| {
            let mut by_name = HashMap::new();
            for family in Self::families() {
                for name in family.names() {
                    let previous = by_name.insert(*name, *family);
                    debug_assert!(previous.is_none(), "command name `{name}` has two recognizers");
                }
            }
            Registry { by_name }
        })
    }

    /// The operations of `inv`: a loop only raises its flag; a command that yields nothing may
    /// still name an output directory through a tool flag.
    pub fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp> {
        if LOOPS.contains(&inv.name.as_str()) {
            ctx.flags.looped = true;
            return Vec::new();
        }
        // A script read from a heredoc: Xiox does not interpret it, even when the tool is unknown.
        if inv.heredoc.is_some() && interpreters::runs_scripts(&inv.name) {
            ctx.flags.script = true;
        }
        let ops = match self.by_name.get(inv.name.as_str()) {
            Some(family) => family.recognize(inv, ctx),
            None => Vec::new(),
        };
        if ops.is_empty() { output_flags::output_directories(inv.args) } else { ops }
    }
}

/// The text arguments, without PowerShell array commas.
fn words(args: &[Arg]) -> Vec<&str> {
    args.iter().filter_map(Arg::text).collect()
}

/// Plain words as arguments, for the parsers that take them.
fn text_args(words: &[&str]) -> Vec<Arg> {
    words.iter().map(|word| Arg::Text((*word).to_owned())).collect()
}

fn path_op(kind: OpKind, src: Option<String>, dst: Option<String>) -> RawOp {
    RawOp::Path { kind, src, dst, base: None, script: false }
}

fn write_op(dst: impl Into<String>) -> RawOp {
    path_op(OpKind::Write, None, Some(dst.into()))
}

/// The path parameter names every cmdlet that takes a path shares.
const PATH_PARAMS: [(&str, &str); 4] =
    [("path", "path"), ("literalpath", "path"), ("lp", "path"), ("pspath", "path")];

/// PowerShell parsing of `args` with the shared path parameters plus `extra`.
fn powershell_with_path(args: &[Arg], extra: &[(&str, &'static str)]) -> PsParams {
    let params: Vec<_> = PATH_PARAMS.iter().chain(extra).copied().collect();
    powershell(args, &params)
}

/// The values of the `path` parameter, or else those of the first positional argument.
fn ps_paths(params: &PsParams) -> Vec<String> {
    match params.named.get("path") {
        Some(paths) => paths.clone(),
        None => params.positional.first().cloned().unwrap_or_default(),
    }
}
