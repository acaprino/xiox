//! Copies of the repository and folders named after the project, found outside it.
//!
//! Candidates come from the MFT index (elevated, NTFS drives: a name search with the four
//! `<name><separator>*` patterns, which `DirIndex::find_dirs_named` supports) and from a
//! bounded walk (the project's parent, the drive roots and the user's home). The walk also
//! lists every git repository it meets, whatever its name, so a clone called `zzz` is found by
//! identity; the index can only offer folders by name. Links are never followed, system
//! folders, `AppData`, the project and its worktrees are never entered, and a git repository is
//! tested as a candidate but never descended into.

use crate::owned::key_within;
use crate::{Block, Category, Env, Item, Level, Note, Options, Project, SearchMode, Target, Why};
use crate::gitref::git_may_read;
use crate::nested::nested_repos;
use crate::owned::live_session_in;
use xiox_model::{ArtifactKind, DirIndex, is_system_dir, normalize_path, path_key};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A project name shorter than this (in characters) is never searched by name (copies still are, by identity).
const MIN_NAME_CHARS: usize = 4;
/// How deep the walk goes below a drive root or the user's home.
const WALK_DEPTH: usize = 3;
const SEPARATORS: [char; 4] = ['-', '_', '.', ' '];

/// What the search found, how it looked, and what the user should know about it.
pub(crate) struct Found {
    pub items: Vec<Item>,
    pub mode: SearchMode,
    pub notes: Vec<Note>,
}

/// True when `folder` is `project` + one of `-` `_` `.` ` ` + at least one more character, case-insensitive.
/// A project name shorter than [`MIN_NAME_CHARS`] matches nothing (too common to attribute folders).
pub(crate) fn name_matches_project(project: &str, folder: &str) -> bool {
    if project.chars().count() < MIN_NAME_CHARS {
        return false;
    }
    let (p, f) = (project.to_lowercase(), folder.to_lowercase());
    f.strip_prefix(p.as_str()).is_some_and(|rest| {
        let mut chars = rest.chars();
        chars.next().is_some_and(|c| SEPARATORS.contains(&c)) && chars.next().is_some()
    })
}

/// Candidate folders outside the project: those named after it and every git repository, found
/// by walking `roots` (`(root, max_depth)`; depth 1 = the children of the root). The caller
/// tells copies and named folders apart with [`classify`].
pub(crate) fn walk_candidates(project: &Project, roots: &[(PathBuf, usize)]) -> Vec<PathBuf> {
    let own = Own::new(project);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (root, depth) in roots {
        let root = normalize_path(root);
        if own.holds(&path_key(&root)) {
            continue;
        }
        walk(&root, *depth, &own, &project.name, &mut seen, &mut out);
    }
    out
}

fn walk(dir: &Path, levels: usize, own: &Own, name: &str, seen: &mut HashSet<String>, out: &mut Vec<PathBuf>) {
    if levels == 0 {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    let parent_is_root = dir.parent().is_none();
    let mut children: Vec<(String, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        // `DirEntry::metadata` does not follow links: a link is never a directory here.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || is_link(&meta) {
            continue;
        }
        children.push((entry.file_name().to_string_lossy().into_owned(), entry.path()));
    }
    children.sort();
    for (child, path) in children {
        if is_system_dir(&child, parent_is_root) {
            continue;
        }
        let key = path_key(&path);
        if own.holds(&key) {
            continue;
        }
        let repo = is_git_repo(&path);
        if !own.inside(&key)
            && (repo || name_matches_project(name, &child))
            && !inside_other_repo(&path)
            && seen.insert(key)
        {
            out.push(path.clone());
        }
        if !repo {
            walk(&path, levels - 1, own, name, seen, out);
        }
    }
}

/// `Some` when `candidate` is something to list: a copy of the project's repository, or a folder
/// named after the project. `project_roots`: the project's root commits; `project_origin`: its
/// `remote.origin.url`.
pub(crate) fn classify(
    project: &Project,
    project_roots: &[String],
    project_origin: Option<&str>,
    candidate: &Path,
) -> Option<(Category, Why)> {
    let own = Own::new(project);
    let key = path_key(candidate);
    if own.holds(&key) || own.inside(&key) || inside_other_repo(candidate) {
        return None;
    }
    if !fs::symlink_metadata(candidate).is_ok_and(|m| m.is_dir() && !is_link(&m)) {
        return None;
    }
    // A repository whose `.git` points somewhere git must not go is no copy: git is not asked.
    if is_git_repo(candidate) && git_may_read(candidate, home_dir().as_deref()) {
        let shared_root_commit = {
            let roots = xiox_git::root_commits(candidate);
            roots.iter().any(|r| project_roots.contains(r))
        };
        let same_origin = match (project_origin, xiox_git::origin_url(candidate)) {
            (Some(a), Some(b)) => xiox_git::normalize_remote(a) == xiox_git::normalize_remote(&b),
            _ => false,
        };
        if shared_root_commit || same_origin {
            let has_own_work = has_own_work(project, candidate);
            return Some((Category::Copies, Why::SameRepository { shared_root_commit, same_origin, has_own_work }));
        }
    }
    let name = candidate.file_name()?.to_string_lossy();
    name_matches_project(&project.name, &name).then_some((Category::Named, Why::NameOnly))
}

/// `Some(true)`: the copy is dirty, or HEAD, a local branch, a tag or a stash entry of it is a
/// commit that no ref of the project's repository reaches (a commit the project has only as a
/// dangling object, after a deleted branch or a rebase, is the copy's own work). `Some(false)`:
/// clean and every tip reachable. `None`: git cannot tell. Every git call goes through `xiox_git`
/// (timeout, clean environment).
fn has_own_work(project: &Project, copy: &Path) -> Option<bool> {
    let dirty = xiox_git::working_tree_dirty(copy);
    let missing = match xiox_git::work_tips(copy) {
        Some(tips) if !tips.is_empty() => xiox_git::tips_missing_from_refs(&project.root, &tips),
        _ => None,
    };
    match (dirty, missing) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// What a git repository that is not a copy of the project holds.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Clean, and every commit is in a remote-tracking branch.
    Safe,
    /// Uncommitted changes, or commits no remote has: its history exists only here.
    Work,
    /// git cannot read it (`dubious ownership`, a broken `.git`) or gave no answer.
    Unreadable,
}

fn independent_verdict(dir: &Path) -> Verdict {
    match xiox_git::working_tree_dirty(dir) {
        None => Verdict::Unreadable,
        Some(true) => Verdict::Work,
        Some(false) => match xiox_git::work_tips(dir).map(|tips| xiox_git::tips_not_pushed(dir, &tips)) {
            Some(Some(false)) => Verdict::Safe,
            Some(Some(true)) => Verdict::Work,
            _ => Verdict::Unreadable,
        },
    }
}

/// The blocks of a name-matched folder: it is, or holds, a git repository. The folder itself
/// being a repository with work of its own blocks it; a repository git cannot read, in it or
/// below it, blocks it (nothing can show it has no unsaved work). What is below a folder and
/// is not listed is checked again when the folder is removed (`clean::check_contents`).
/// With `repos_block`, any git repository inside the folder blocks it too: a session folder is
/// nobody's copy, so what it holds cannot be vouched for (a named folder's nested copies are
/// checked one by one at removal instead).
pub(crate) fn named_blocks(dir: &Path, repos_block: bool) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut unknown = false;
    let home = home_dir();
    // A repository git must not be asked about cannot be vouched for.
    let askable = |repo: &Path| git_may_read(repo, home.as_deref());
    if is_git_repo(dir) {
        match if askable(dir) { independent_verdict(dir) } else { Verdict::Unreadable } {
            Verdict::Safe => {}
            Verdict::Work => blocks.push(Block::UnsavedWork),
            Verdict::Unreadable => unknown = true,
        }
    }
    match nested_repos(dir) {
        Ok(repos) => {
            unknown |= repos.iter().any(|r| !askable(r) || xiox_git::working_tree_dirty(r).is_none());
            if repos_block && !repos.is_empty() {
                blocks.push(Block::HoldsRepositories);
            }
        }
        Err(_) => unknown = true,
    }
    if unknown {
        blocks.push(Block::UnsavedWorkUnknown);
    }
    blocks
}

// ───────────────────────────────── helpers ─────────────────────────────────

/// The project and its worktrees, as `path_key`s.
pub(crate) struct Own {
    all: Vec<String>,
}

impl Own {
    pub(crate) fn new(project: &Project) -> Own {
        let mut all = vec![path_key(&project.root)];
        all.extend(project.worktrees.iter().map(|w| path_key(w)));
        Own { all }
    }

    /// `key` is the project or a worktree, or inside one.
    pub(crate) fn holds(&self, key: &str) -> bool {
        self.all.iter().any(|o| key_within(key, o))
    }

    /// `key` contains the project or a worktree (removing it would remove them).
    pub(crate) fn inside(&self, key: &str) -> bool {
        self.all.iter().any(|o| key_within(o, key))
    }
}

/// Symlink, junction or any other reparse point.
pub(crate) fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// A `.git` folder or file (a linked worktree or submodule has a file).
pub(crate) fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// Some ancestor of `path` (not `path` itself) is a git repository. Like git, the folders in
/// `GIT_CEILING_DIRECTORIES` and above them are not looked at.
pub(crate) fn inside_other_repo(path: &Path) -> bool {
    let ceilings: Vec<String> = std::env::var_os("GIT_CEILING_DIRECTORIES")
        .map(|v| std::env::split_paths(&v).map(|p| path_key(&p)).collect())
        .unwrap_or_default();
    for a in path.ancestors().skip(1) {
        if ceilings.contains(&path_key(a)) {
            return false;
        }
        if is_git_repo(a) {
            return true;
        }
    }
    false
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

// ───────────────────────────────── the index ────────────────────────────────

/// Folders of `index` under `roots` named after the project. `find_dirs_named` takes a name or
/// one `prefix*`, so one pattern per separator; the name rule is applied again on the result.
/// The index already leaves out links and system folders; the project's own folders and the
/// folders that contain them are dropped here. Touches no disk.
pub(crate) fn mft_hits(index: &dyn DirIndex, project: &Project, roots: &[PathBuf]) -> Vec<PathBuf> {
    let lower = project.name.to_lowercase();
    let patterns: Vec<String> = SEPARATORS.iter().map(|s| format!("{lower}{s}*")).collect();
    let names: Vec<&str> = patterns.iter().map(String::as_str).collect();
    let own = Own::new(project);
    index
        .find_dirs_named(&names, roots)
        .into_iter()
        .map(|d| normalize_path(&d.path))
        .filter(|p| p.file_name().is_some_and(|n| name_matches_project(&project.name, &n.to_string_lossy())))
        .filter(|p| {
            let k = path_key(p);
            !own.holds(&k) && !own.inside(&k)
        })
        .collect()
}

/// Index of the NTFS fixed drives; a failure or a panic is "no index".
fn read_index(drives: &[xiox_disktree::FixedDrive]) -> Option<xiox_disktree::DiskIndex> {
    let roots: Vec<PathBuf> = drives.iter().filter(|d| d.ntfs).map(|d| d.root()).collect();
    if roots.is_empty() {
        return None;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        xiox_disktree::build_for_roots(&roots, true, &xiox_disktree::Progress::default()).0
    }))
    .ok()
    .flatten()
}

// ───────────────────────────────── the search ───────────────────────────────

/// The items among `dirs`: classified by identity or name, with their blocks and size. A folder
/// that is neither is left out.
pub(crate) fn verify_dirs(project: &Project, opts: &Options, env: &Env, dirs: &[PathBuf]) -> Vec<Item> {
    if dirs.is_empty() {
        return Vec::new();
    }
    let roots: Vec<String> = if project.is_repo { xiox_git::root_commits(&project.root) } else { Vec::new() };
    dirs.iter()
        .filter_map(|dir| {
            let (category, why) = classify(project, &roots, project.origin.as_deref(), dir)?;
            Some(item_for(opts, env, dir, category, why))
        })
        .collect()
}

pub(crate) fn item_for(opts: &Options, env: &Env, dir: &Path, category: Category, why: Why) -> Item {
    // What cannot be removed at all: a copy with work the project lacks (or whose work cannot be
    // checked), a repository git cannot read, a live agent session inside.
    let mut blocked = match &why {
        Why::SameRepository { has_own_work: Some(true), .. } => vec![Block::UnsavedWork],
        Why::SameRepository { has_own_work: None, .. } => vec![Block::UnsavedWorkUnknown],
        Why::NameOnly => named_blocks(dir, false),
        Why::SessionFolder { .. } => named_blocks(dir, true),
        _ => Vec::new(),
    };
    if live_session_in(dir, env) {
        blocked.push(Block::LiveSession);
    }
    Item {
        key: path_key(dir),
        category,
        bytes: opts.measure.then(|| xiox_cleaner::measure_dir(dir).effective_bytes()),
        level: Level::Caution,
        risks: Vec::new(),
        identity: crate::identity::folder_id(dir),
        blocked,
        why,
        target: Target::Dir { kind: ArtifactKind::ProjectFolder },
        path: Some(dir.to_path_buf()),
        provenance: None,
    }
}

/// Find the copies and the name-matched folders of `project`. `extra` are folders found some
/// other way (the working directories of sessions); they are verified like the walk's.
pub(crate) fn search(project: &Project, opts: &Options, env: &Env, extra: &[PathBuf]) -> Found {
    // A short name is not searched by name: no `named` items and no index (the index only
    // offers names). Copies are still found by identity, by the walk.
    let by_name = project.name.chars().count() >= MIN_NAME_CHARS;
    let mut notes = Vec::new();
    if !by_name {
        notes.push(Note::NameTooShort);
    }

    let mut walk: Vec<(PathBuf, usize)> = Vec::new();
    if let Some(parent) = project.root.parent() {
        walk.push((parent.to_path_buf(), 1));
    }
    let mut mode = SearchMode::ParentOnly;
    let mut hits: Vec<PathBuf> = Vec::new();
    let mut deep_walk = false;
    if opts.drives {
        mode = SearchMode::BoundedWalk;
        let deep: Vec<PathBuf> = match &env.search_roots {
            Some(roots) => roots.clone(),
            None => {
                let drives = xiox_disktree::fixed_drives();
                let mut covered: Vec<PathBuf> = Vec::new();
                if opts.admin
                    && by_name
                    && let Some(index) = read_index(&drives)
                {
                    covered = drives.iter().map(|d| d.root()).filter(|r| index.size_of(r).is_some()).collect();
                    if !covered.is_empty() {
                        mode = SearchMode::MftIndex;
                        hits = mft_hits(&index, project, &covered)
                            .into_iter()
                            .filter(|p| !inside_other_repo(p))
                            .collect();
                    }
                }
                let covered_keys: Vec<String> = covered.iter().map(|c| path_key(c)).collect();
                let uncovered = |p: &Path| {
                    let k = path_key(p);
                    !covered_keys.iter().any(|c| key_within(&k, c))
                };
                let mut deep: Vec<PathBuf> = drives.iter().map(|d| d.root()).filter(|r| uncovered(r)).collect();
                deep.extend(home_dir().filter(|h| uncovered(h)));
                deep
            }
        };
        deep_walk = !deep.is_empty();
        walk.extend(deep.into_iter().map(|r| (r, WALK_DEPTH)));
    }

    if mode == SearchMode::BoundedWalk || deep_walk {
        notes.push(Note::WalkMayMissDeepFolders);
    }

    let mut candidates = walk_candidates(project, &walk);
    candidates.extend(hits);
    candidates.extend(extra.iter().cloned());
    let mut seen = HashSet::new();
    candidates.retain(|c| seen.insert(path_key(c)));

    let items = verify_dirs(project, opts, env, &candidates);
    Found { items, mode, notes }
}
