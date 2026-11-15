//! Project scaffolders: `cargo new`, `npm create`, `uv init`, `dotnet new` and the like. Each
//! creates the directory its command line names.

use super::{Context, Invocation, Recognizer, path_op, text_args, words};
use crate::provenance::{Arg, OpKind, RawOp, posix};

pub(super) struct Packages;

impl Recognizer for Packages {
    fn names(&self) -> &'static [&'static str] {
        &["cargo", "npm", "pnpm", "yarn", "npx", "pnpx", "bunx", "uv", "dotnet", "virtualenv"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        match inv.name.as_str() {
            "cargo" => cargo(inv.args),
            "npm" | "pnpm" | "yarn" => create_init(inv.args, Scaffold::Package),
            "npx" | "pnpx" | "bunx" => create_package(inv.args),
            "uv" => uv(inv.args),
            "dotnet" => dotnet(inv.args),
            _ => virtualenv(inv.args),
        }
    }
}

fn create(dst: impl Into<String>) -> RawOp {
    path_op(OpKind::Create, None, Some(dst.into()))
}

/// The words up to `--`: what follows belongs to the template, not to the tool.
fn before_separator<'a>(words: &[&'a str]) -> Vec<&'a str> {
    words.iter().copied().take_while(|word| *word != "--").collect()
}

fn cargo(args: &[Arg]) -> Vec<RawOp> {
    let words = words(args);
    match words.first() {
        Some(&("new" | "init")) => {
            let values = ["--name", "--vcs", "--edition", "--registry", "--color", "--config", "-Z"];
            posix(&text_args(&words[1..]), &values).positional.into_iter().take(1).map(create).collect()
        }
        _ => Vec::new(),
    }
}

/// Where the directory sits among the positional arguments after `create` or `init`.
#[derive(Clone, Copy)]
pub(super) enum Scaffold {
    /// `npm`, `pnpm`, `yarn`: `create|init <package> <dir>`.
    Package,
    /// `bun`: `create <template> <dir>`, but `init <dir>` names the directory alone. `bun` reaches
    /// this through the interpreters, which own the name.
    Bun,
}

impl Scaffold {
    fn dir_index(self, subcommand: &str) -> usize {
        match (self, subcommand) {
            (Scaffold::Bun, "init") => 0,
            _ => 1,
        }
    }
}

pub(super) fn create_init(args: &[Arg], tool: Scaffold) -> Vec<RawOp> {
    let words = before_separator(&words(args));
    match words.first() {
        Some(&subcommand @ ("create" | "init")) => {
            let pos = posix(&text_args(&words[1..]), &["--scope", "-w", "--workspace"]).positional;
            pos.into_iter().nth(tool.dir_index(subcommand)).map(create).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// `npx create-<name> <dir>`, with an optional scope.
fn create_package(args: &[Arg]) -> Vec<RawOp> {
    let words = before_separator(&words(args));
    let pos = posix(&text_args(&words), &["-p", "--package"]).positional;
    match pos.first() {
        Some(package) if is_create_package(package) => pos.into_iter().nth(1).map(create).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn is_create_package(package: &str) -> bool {
    let name = match package.strip_prefix('@') {
        Some(scoped) => {
            scoped.split_once('/').filter(|(scope, _)| !scope.is_empty()).map_or(package, |(_, name)| name)
        }
        None => package,
    };
    name.starts_with("create-")
}

fn uv(args: &[Arg]) -> Vec<RawOp> {
    let words = words(args);
    match words.first() {
        Some(&"init") => {
            let values = ["--name", "--python", "-p", "--build-backend", "--vcs", "--author-from"];
            posix(&text_args(&words[1..]), &values).positional.into_iter().take(1).map(create).collect()
        }
        Some(&"venv") => {
            let pos = posix(&text_args(&words[1..]), &["--python", "-p", "--prompt"]).positional;
            vec![create(pos.into_iter().next().unwrap_or_else(|| ".venv".into()))]
        }
        _ => Vec::new(),
    }
}

/// `dotnet new`: the output directory, else the project name.
fn dotnet(args: &[Arg]) -> Vec<RawOp> {
    let words = words(args);
    if words.first() != Some(&"new") {
        return Vec::new();
    }
    let values = ["-o", "--output", "-n", "--name", "-f", "--framework", "-lang", "--language"];
    let options = posix(&text_args(&words[1..]), &values).options;
    ["-o", "--output", "-n", "--name"]
        .iter()
        .find_map(|key| options.get(*key).cloned().flatten().filter(|dst| !dst.is_empty()))
        .map(create)
        .into_iter()
        .collect()
}

fn virtualenv(args: &[Arg]) -> Vec<RawOp> {
    posix(args, &["-p", "--python", "--prompt"]).positional.into_iter().map(create).collect()
}
