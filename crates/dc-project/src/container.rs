//! Folders that hold many projects. A project is the unit of `xiox clean`: pointed at a drive
//! root, the user profile or a folder with several repositories, the category flags would take
//! the builds and the archived sessions of everything under it, so they are refused there.

use crate::Project;
use crate::owned::key_within;
use crate::search::{home_dir, is_git_repo, is_link};
use dc_model::{is_system_dir, path_key};
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

/// How deep below the folder repositories are looked for (1 = its children).
const MAX_DEPTH: usize = 3;
/// Most folders read while counting.
const MAX_DIRS: usize = 2000;

/// The rule. A folder is a container when it is a drive root, or the user profile, or a folder
/// above it (whether or not it is a repository: a dotfiles repository at `~` is still the whole
/// profile), or when it is not a repository and holds more than one project: several git
/// repositories (`repos_below`) or several engine project roots (`engine_roots`). A repository
/// that holds other repositories (vendored clones) is one project.
pub(crate) fn is_container(
    root: &Path,
    is_repo: bool,
    home: Option<&Path>,
    engine_roots: usize,
    repos_below: usize,
) -> bool {
    if root.parent().is_none() {
        return true;
    }
    if home.is_some_and(|h| key_within(&path_key(h), &path_key(root))) {
        return true;
    }
    !is_repo && (engine_roots > 1 || repos_below > 1)
}

/// [`is_container`] for `project`, counting repositories only when the cheaper rules did not
/// decide it.
pub(crate) fn container_root(project: &Project, engine_roots: usize) -> bool {
    let home = home_dir();
    if is_container(&project.root, project.is_repo, home.as_deref(), engine_roots, 0) {
        return true;
    }
    !project.is_repo && is_container(&project.root, false, home.as_deref(), engine_roots, repos_below(&project.root, 2))
}

/// How many git repositories lie below `root` (to [`MAX_DEPTH`] levels, [`MAX_DIRS`] folders, no
/// link followed, no repository entered), counting stops at `stop_at`.
pub(crate) fn repos_below(root: &Path, stop_at: usize) -> usize {
    let mut found = 0;
    let mut visited = 0;
    let mut queue: VecDeque<(std::path::PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= MAX_DEPTH {
            continue;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        let parent_is_root = dir.parent().is_none();
        for entry in read.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_dir() || is_link(&meta) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || is_system_dir(&name, parent_is_root) {
                continue;
            }
            visited += 1;
            if visited > MAX_DIRS {
                return found;
            }
            let path = entry.path();
            if is_git_repo(&path) {
                found += 1;
                if found >= stop_at {
                    return found;
                }
            } else {
                queue.push_back((path, depth + 1));
            }
        }
    }
    found
}
