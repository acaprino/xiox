//! Bounded size of a folder: a check of the observer must never run for long or eat memory,
//! whatever the tree it is pointed at.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `FILE_ATTRIBUTE_REPARSE_POINT`: junctions, symlinks and every other reparse point.
const REPARSE_POINT: u32 = 0x400;

fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = REPARSE_POINT;
        false
    }
}

/// The size of a folder as far as a bounded walk got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The bytes counted. When `complete` is false it is a LOWER BOUND of the real size.
    pub bytes: u64,
    /// The walk reached the end of the folder (within the caps).
    pub complete: bool,
}

/// Bounded recursive size of `path`: never follows links or reparse points (a link counts as
/// nothing: removing it frees nothing of its target); stops after `max_entries` entries (files
/// and folders below `path`) or `max_time`; then it returns what it had counted so far with
/// `complete: false` (a lower bound: a huge folder is not blind, it is only measured in part).
/// `None` only when the root cannot be read. A folder below the root that cannot be read is skipped
/// (it under-counts, it does not fail the measure). Iterative: a deep tree cannot overflow the
/// stack, and only the folders still to visit are kept in memory.
pub fn measure_bytes(path: &Path, max_entries: usize, max_time: Duration) -> Option<Measured> {
    let start = Instant::now();
    let whole = |bytes: u64| {
        Some(Measured {
            bytes,
            complete: true,
        })
    };
    let partial = |bytes: u64| {
        Some(Measured {
            bytes,
            complete: false,
        })
    };
    let root = fs::symlink_metadata(path).ok()?;
    if is_link(&root) {
        return whole(0);
    }
    if !root.is_dir() {
        return whole(root.len());
    }
    let mut total: u64 = 0;
    let mut entries: usize = 0;
    let mut pending: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if start.elapsed() >= max_time {
            return partial(total);
        }
        let read = match fs::read_dir(&dir) {
            Ok(read) => read,
            // The root itself unreadable: no measure at all; deeper: skip it.
            Err(_) if dir == path => return None,
            Err(_) => continue,
        };
        for entry in read {
            let Ok(entry) = entry else { continue };
            entries += 1;
            if entries > max_entries || start.elapsed() >= max_time {
                return partial(total);
            }
            // `DirEntry::metadata` does not follow links.
            let Ok(meta) = entry.metadata() else { continue };
            if is_link(&meta) {
                continue;
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    whole(total)
}
