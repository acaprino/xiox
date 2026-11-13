//! Filesystem helpers: link-safe directory listing, identity checks, skip rules, sizing.

use crate::markers::Identity;
use xiox_model::{SizeInfo, normalize_path, path_key};
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct FileEntry {
    pub name: String,
    pub mtime_ms: Option<i64>,
}

pub struct SubDir {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Listing {
    pub files: Vec<FileEntry>,
    pub dirs: Vec<SubDir>,
}

#[derive(PartialEq, Eq)]
enum Kind {
    Dir,
    File,
    /// symlink, junction or other reparse-point directory: never followed.
    Link,
}

#[cfg(windows)]
fn is_reparse_attr(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
#[cfg(not(windows))]
fn is_reparse_attr(_meta: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn attr_compressed_or_sparse(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & (0x800 | 0x200) != 0
}
#[cfg(not(windows))]
fn attr_compressed_or_sparse(_meta: &Metadata) -> bool {
    false
}

fn classify(meta: &Metadata) -> Kind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        return Kind::Link;
    }
    if ft.is_dir() {
        // Junctions are reported as symlinks by std; this also catches other reparse points.
        return if is_reparse_attr(meta) { Kind::Link } else { Kind::Dir };
    }
    Kind::File
}

pub fn mtime_ms(meta: &Metadata) -> Option<i64> {
    let d = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(d.as_millis() as i64)
}

/// List one directory without following links. Link/reparse directories are omitted.
pub fn list_dir(dir: &Path) -> std::io::Result<Listing> {
    let mut l = Listing::default();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        match classify(&meta) {
            Kind::Dir => l.dirs.push(SubDir { name, path: entry.path() }),
            Kind::File => l.files.push(FileEntry { name, mtime_ms: mtime_ms(&meta) }),
            Kind::Link => {}
        }
    }
    Ok(l)
}

// ───────────────────────────── identity ─────────────────────────────

fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).symlink_metadata().is_ok()
}

pub fn identity_ok(identity: Identity, dir: &Path) -> bool {
    match identity {
        Identity::ParentMarker => true,
        Identity::RustTarget => exists(dir, "CACHEDIR.TAG") || exists(dir, ".rustc_info.json"),
        Identity::NodeModules => {
            exists(dir, ".modules.yaml")
                || exists(dir, ".package-lock.json")
                || exists(dir, ".yarn-integrity")
                || exists(dir, ".pnpm")
        }
        Identity::PythonVenv => exists(dir, "pyvenv.cfg"),
        Identity::MesonBuild => is_real_dir(&dir.join("meson-private")) && exists(dir, "build.ninja"),
    }
}

/// A real directory: not a file, not a link, junction or other reparse point.
pub fn is_real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| classify(&m) == Kind::Dir)
}

/// Does `dir` carry any strong identity file (it is certainly generated output)?
pub fn has_any_identity(dir: &Path) -> bool {
    identity_ok(Identity::RustTarget, dir)
        || identity_ok(Identity::NodeModules, dir)
        || identity_ok(Identity::PythonVenv, dir)
}

// ───────────────────────────── skip rules ─────────────────────────────

pub fn ignore_set(ignore: &[PathBuf]) -> HashSet<String> {
    ignore.iter().map(|p| path_key(p)).collect()
}

// ───────────────────────────── sizing ─────────────────────────────

const CLUSTER: u64 = 4096;

fn allocated_for(path: &Path, meta: &Metadata) -> u64 {
    let len = meta.len();
    if attr_compressed_or_sparse(meta)
        && let Ok(real) = filesize::file_real_size_fast(path, meta)
    {
        return real.div_ceil(CLUSTER) * CLUSTER;
    }
    len.div_ceil(CLUSTER) * CLUSTER
}

const MAX_SIZE_WARNINGS: usize = 5;

/// Logical + allocated size and file count of one artifact directory (single walk, links not
/// followed). Hardlinks are NOT deduplicated (would need a file handle per file).
pub fn dir_size(root: &Path) -> (SizeInfo, Vec<String>) {
    let mut info = SizeInfo::default();
    let mut allocated = 0u64;
    let mut warnings = Vec::new();
    let mut dropped = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                if warnings.len() < MAX_SIZE_WARNINGS {
                    warnings.push(format!("cannot read {}: {e}", normalize_path(&dir).display()));
                } else {
                    dropped += 1;
                }
                continue;
            }
        };
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            match classify(&meta) {
                Kind::Dir => stack.push(entry.path()),
                Kind::File => {
                    info.logical_bytes += meta.len();
                    info.file_count += 1;
                    allocated += allocated_for(&entry.path(), &meta);
                }
                Kind::Link => {}
            }
        }
    }
    if dropped > 0 {
        warnings.push(format!("{dropped} more unreadable directories under {}", normalize_path(root).display()));
    }
    info.allocated_bytes = Some(allocated);
    (info, warnings)
}
