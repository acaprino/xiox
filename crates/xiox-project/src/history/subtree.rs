//! A session folder inside the project or one of its worktrees: what git knows about the subtree
//! decides whether it can go, never the guard of name-matched folders outside the project.

use std::path::Path;

use crate::Block;

/// The blocks git gives the subtree `dir` of `repo`: tracked files, or no answer.
pub(crate) fn tracked_blocks(repo: &Path, dir: &Path) -> Vec<Block> {
    match xiox_git::tracked_under(repo, dir) {
        Some(true) => vec![Block::TrackedFiles],
        Some(false) => Vec::new(),
        None => vec![Block::UnsavedWorkUnknown],
    }
}
