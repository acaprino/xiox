//! Git repositories inside a folder: what a folder that is about to be removed would take with it.

use crate::search::{is_git_repo, is_link};
use std::fs;
use std::path::{Path, PathBuf};

/// Most directory entries read before giving up: a folder bigger than this cannot be vouched for.
const MAX_ENTRIES: usize = 500_000;

/// The folder is too big to look through.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TooBig;

/// Every git repository strictly below `dir` (a `.git` folder or file), found without following
/// links and without entering a repository once found; `dir`'s own `.git` is not looked at.
/// `Err` when more than [`MAX_ENTRIES`] entries had to be read.
pub(crate) fn nested_repos(dir: &Path) -> Result<Vec<PathBuf>, TooBig> {
    let mut found = Vec::new();
    let mut read_entries = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            read_entries += 1;
            if read_entries > MAX_ENTRIES {
                return Err(TooBig);
            }
            // `DirEntry::metadata` does not follow links.
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_dir() || is_link(&meta) {
                continue;
            }
            let path = entry.path();
            if is_git_repo(&path) {
                found.push(path);
            } else {
                stack.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}
