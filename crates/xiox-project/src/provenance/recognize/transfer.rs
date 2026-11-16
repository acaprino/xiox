//! `cp`, `mv`, `Copy-Item`, `Move-Item`, `robocopy` and `xcopy`: one source/destination pair per
//! source.

use super::{Context, Invocation, Recognizer, path_op, powershell_with_path, words};
use crate::provenance::{OpKind, RawOp, Shell, posix};

pub(super) struct Transfer;

impl Recognizer for Transfer {
    fn names(&self) -> &'static [&'static str] {
        &["cp", "copy", "copy-item", "cpi", "mv", "move", "move-item", "mi", "robocopy", "xcopy"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        let kind = match inv.name.as_str() {
            "cp" | "copy" | "copy-item" | "cpi" | "robocopy" | "xcopy" => OpKind::Copy,
            _ => OpKind::Move,
        };
        let (sources, dst) = match grammar(inv) {
            Shell::PowerShell => powershell_pair(inv),
            Shell::Cmd => cmd_pair(inv),
            Shell::Bash => posix_pair(inv),
        };
        match dst {
            Some(dst) => sources.into_iter().map(|src| path_op(kind, Some(src), Some(dst.clone()))).collect(),
            None => Vec::new(),
        }
    }
}

/// The argument grammar of the command: the cmdlets and the Windows tools carry their own,
/// whatever shell runs them.
fn grammar(inv: &Invocation) -> Shell {
    match inv.name.as_str() {
        "copy-item" | "move-item" => Shell::PowerShell,
        "robocopy" | "xcopy" => Shell::Cmd,
        _ => inv.shell,
    }
}

type Pair = (Vec<String>, Option<String>);

fn powershell_pair(inv: &Invocation) -> Pair {
    let params = powershell_with_path(
        inv.args,
        &[("destination", "dst"), ("filter", "x"), ("include", "x"), ("exclude", "x")],
    );
    let mut positional = params.positional.into_iter();
    let sources = match params.named.get("path") {
        Some(paths) => paths.clone(),
        None => positional.next().unwrap_or_default(),
    };
    let dst = match params.named.get("dst") {
        Some(dst) => dst.first().cloned(),
        None => positional.next().and_then(|dst| dst.into_iter().next()),
    };
    (sources, dst)
}

/// `source destination [masks] /flags`: only the first two paths count.
fn cmd_pair(inv: &Invocation) -> Pair {
    let mut paths = words(inv.args).into_iter().filter(|word| !word.starts_with('/')).map(str::to_owned);
    let source = paths.next();
    (source.into_iter().collect(), paths.next())
}

/// `-t DIR sources...`, else the last word is the destination.
fn posix_pair(inv: &Invocation) -> Pair {
    let parsed = posix(inv.args, &["-t", "--target-directory", "-S", "--suffix"]);
    let target = ["-t", "--target-directory"]
        .iter()
        .find_map(|key| parsed.options.get(*key).cloned().flatten());
    let mut positional = parsed.positional;
    match target {
        Some(target) => (positional, Some(target)),
        None if positional.len() > 1 => {
            let dst = positional.pop();
            (positional, dst)
        }
        None => (Vec::new(), None),
    }
}
