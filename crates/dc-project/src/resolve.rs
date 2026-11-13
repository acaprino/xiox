//! Which project a path belongs to.

use crate::{Error, Project};
use dc_git::WorktreeEntry;
use dc_model::normalize_path;
use std::path::{Path, PathBuf};

/// The project that holds `path`: the main working tree of its git repository (also when
/// `path` is a subfolder or a linked worktree), else `path` itself.
pub fn resolve_project(path: &Path) -> Result<Project, Error> {
    Ok(resolve_full(path)?.0)
}

/// [`resolve_project`] plus every entry of `git worktree list` (main included; empty when the
/// root is not a repository).
pub(crate) fn resolve_full(path: &Path) -> Result<(Project, Vec<WorktreeEntry>), Error> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let abs = normalize_path(&abs);
    if !abs.exists() {
        return Err(Error::PathNotFound(abs));
    }
    if !abs.is_dir() {
        return Err(Error::NotADirectory(abs));
    }
    let main = dc_git::repo_main_root(&abs).filter(|r| r.is_dir());
    let is_repo = main.is_some();
    let root = main.unwrap_or(abs);
    let entries = if is_repo { dc_git::list_worktrees(&root).unwrap_or_default() } else { Vec::new() };
    let worktrees: Vec<PathBuf> =
        entries.iter().filter(|e| !e.is_main && !e.prunable).map(|e| normalize_path(&e.path)).collect();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let origin = if is_repo { dc_git::origin_url(&root) } else { None };
    Ok((Project { root, name, is_repo, origin, worktrees }, entries))
}
