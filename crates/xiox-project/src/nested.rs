//! Git repositories inside a folder: what a folder that is about to be removed would take with it.

use crate::search::is_link;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Most directory entries read before giving up: a folder bigger than this cannot be vouched for.
const MAX_ENTRIES: usize = 500_000;

/// Why the folder could not be looked through: what it holds cannot be vouched for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotChecked {
    /// More than [`MAX_ENTRIES`] entries.
    TooBig,
    /// A folder inside it could not be listed, or an entry could not be read.
    Unreadable,
}

/// Every git repository strictly below `dir` (a `.git` folder or file), found without following
/// links and without entering a repository once found; `dir`'s own `.git` is not looked at.
/// `Err` when more than [`MAX_ENTRIES`] entries had to be read, or when a folder or an entry
/// could not be read: what cannot be seen may be a repository.
pub(crate) fn nested_repos(dir: &Path) -> Result<Vec<PathBuf>, NotChecked> {
    let mut found = Vec::new();
    let mut read_entries = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let read = match fs::read_dir(&d) {
            Ok(read) => read,
            // Gone while it was walked: nothing left to take with the folder.
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Err(NotChecked::Unreadable),
        };
        for entry in read {
            let entry = entry.map_err(|_| NotChecked::Unreadable)?;
            read_entries += 1;
            if read_entries > MAX_ENTRIES {
                return Err(NotChecked::TooBig);
            }
            // `DirEntry::metadata` does not follow links.
            let meta = entry.metadata().map_err(|_| NotChecked::Unreadable)?;
            if !meta.is_dir() || is_link(&meta) {
                continue;
            }
            let path = entry.path();
            match path.join(".git").symlink_metadata() {
                Ok(_) => found.push(path),
                Err(e) if e.kind() == ErrorKind::NotFound => stack.push(path),
                Err(_) => return Err(NotChecked::Unreadable),
            }
        }
    }
    found.sort();
    Ok(found)
}
