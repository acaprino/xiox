//! `mkdir` and `New-Item`: directories created, files written.

use super::{Context, Invocation, Recognizer, powershell_with_path, ps_paths, words, write_op};
use crate::provenance::{OpKind, RawOp, Shell, posix};

pub(super) struct Directories;

impl Recognizer for Directories {
    fn names(&self) -> &'static [&'static str] {
        &["mkdir", "md", "new-item", "ni"]
    }

    fn recognize(&self, inv: &Invocation, _ctx: &mut Context) -> Vec<RawOp> {
        let is_mkdir = matches!(inv.name.as_str(), "mkdir" | "md");
        match inv.shell {
            Shell::PowerShell => new_item(inv, if is_mkdir { "directory" } else { "file" }),
            _ if !is_mkdir => new_item(inv, "file"),
            Shell::Cmd => words(inv.args)
                .into_iter()
                .filter(|word| !word.starts_with('/'))
                .map(create_op)
                .collect(),
            Shell::Bash => posix(inv.args, &["-m", "--mode"]).positional.into_iter().map(create_op).collect(),
        }
    }
}

fn create_op(dst: impl Into<String>) -> RawOp {
    RawOp::Path { kind: OpKind::Create, src: None, dst: Some(dst.into()), base: None, script: false }
}

/// `New-Item`: `-Name` goes under each path (the current directory without one); the item type
/// decides between a directory, a file and something that leaves nothing on disk.
fn new_item(inv: &Invocation, default_type: &str) -> Vec<RawOp> {
    let params = powershell_with_path(
        inv.args,
        &[("name", "name"), ("itemtype", "itemtype"), ("type", "itemtype"), ("it", "itemtype"), ("value", "value")],
    );
    let mut paths = ps_paths(&params);
    if let Some(name) = params.named.get("name").and_then(|names| names.first()) {
        if paths.is_empty() {
            paths.push(".".into());
        }
        paths = paths.into_iter().map(|path| format!("{path}\\{name}")).collect();
    }
    let item_type = params.named.get("itemtype").and_then(|types| types.first());
    let item_type = item_type.map_or_else(|| default_type.to_owned(), |item| item.to_lowercase());
    if item_type.starts_with("dir") {
        paths.into_iter().map(create_op).collect()
    } else if item_type.starts_with("file") {
        paths.into_iter().map(write_op).collect()
    } else {
        Vec::new()
    }
}
