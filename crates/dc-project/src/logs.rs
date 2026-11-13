//! Log and temporary files of a project: `*.log`, `*.tmp`, `*.bak`, `*.dmp` of at least 1 MiB
//! that git does not track, inside the project root and its linked worktrees.

use crate::owned::key_within;
use crate::{Category, Item, Note, Project, Target, Why};
use dc_model::{Level, normalize_path, path_key};
use std::path::{Path, PathBuf};

/// Smallest file that is listed.
pub(crate) const MIN_BYTES: u64 = 1 << 20;

const EXTENSIONS: [&str; 4] = ["log", "tmp", "bak", "dmp"];

fn wanted_extension(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Log/temporary files of the project root and each existing worktree. `build_dirs` are the
/// folders already listed under `builds`: nothing inside them is listed twice. Two git calls per
/// root (ignored, then untracked and not ignored); a root that is not a repository adds nothing.
/// A root git cannot list adds one `Note::LogsNotChecked`: "no logs found" is then not a verdict.
pub(crate) fn log_items(project: &Project, build_dirs: &[PathBuf], notes: &mut Vec<Note>) -> Vec<Item> {
    if !project.is_repo {
        return Vec::new();
    }
    let builds: Vec<String> = build_dirs.iter().map(|d| path_key(d)).collect();
    let mut out: Vec<Item> = Vec::new();
    for root in std::iter::once(&project.root).chain(project.worktrees.iter()) {
        for (ignored, level) in [(true, Level::Recommended), (false, Level::Caution)] {
            let Some(files) = dc_git::untracked_files(root, ignored) else {
                if !notes.contains(&Note::LogsNotChecked) {
                    notes.push(Note::LogsNotChecked);
                }
                continue;
            };
            for rel in files.into_iter().filter(|f| wanted_extension(f)) {
                let path = root.join(&rel);
                let key = path_key(&path);
                if builds.iter().any(|b| key_within(&key, b)) || out.iter().any(|i| i.key == key) {
                    continue;
                }
                // `symlink_metadata`: a link (to a file or a folder) is never listed.
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                if !meta.file_type().is_file() || meta.len() < MIN_BYTES {
                    continue;
                }
                out.push(Item {
                    key,
                    category: Category::Logs,
                    path: Some(normalize_path(&path)),
                    bytes: Some(meta.len()),
                    level,
                    blocked: Vec::new(),
                    why: Why::LogFile { ignored },
                    target: Target::File,
                });
            }
        }
    }
    out
}
