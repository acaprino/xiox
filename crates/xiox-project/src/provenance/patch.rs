//! File actions of a Codex `apply_patch` text. Headers are the only lines that start with `*** `
//! at column 0; content lines start with `+`, `-`, a space or `@@`, so they never collide.

use serde::Serialize;

use super::Completeness;

/// What a patch section does to a file. `MoveFrom` is an `Update File` that has a `Move to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatchAction {
    Add,
    Update,
    Delete,
    MoveFrom,
    MoveTo,
}

/// Header prefix and the action it announces, in the order they are tried.
const HEADERS: [(&str, PatchAction); 4] = [
    ("*** Add File:", PatchAction::Add),
    ("*** Update File:", PatchAction::Update),
    ("*** Delete File:", PatchAction::Delete),
    ("*** Move to:", PatchAction::MoveTo),
];

/// The file actions of `text`, paths as written. A truncated text may end in the middle of a
/// header, so its last line is dropped unless a newline closed it.
pub fn patch_ops(text: &str, completeness: Completeness) -> Vec<(PatchAction, String)> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // `split` leaves an empty tail after a final newline, otherwise the unfinished line.
    let tail = lines.pop().unwrap_or("");
    if completeness == Completeness::Whole {
        lines.push(tail);
    }

    let mut ops: Vec<(PatchAction, String)> = Vec::new();
    for line in lines {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some((action, rest)) = HEADERS
            .iter()
            .find_map(|(prefix, action)| line.strip_prefix(prefix).map(|rest| (*action, rest)))
        else {
            continue;
        };
        let path = rest.trim();
        if path.is_empty() {
            continue;
        }
        if action == PatchAction::MoveTo
            && let Some((last, _)) = ops.last_mut().filter(|(last, _)| *last == PatchAction::Update)
        {
            *last = PatchAction::MoveFrom;
        }
        ops.push((action, path.to_string()));
    }
    ops
}
