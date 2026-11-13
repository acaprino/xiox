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
use crate::{Category, Env, Item, Note, Options, Project, SearchMode, Target, Why};
use dc_git::GitCli;
use dc_model::{ArtifactKind, DirIndex, GitProbe, Level, is_system_dir, normalize_path, path_key};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    if is_git_repo(candidate) {
        let shared_root_commit = {
            let roots = dc_git::root_commits(candidate);
            roots.iter().any(|r| project_roots.contains(r))
        };
        let same_origin = match (project_origin, dc_git::origin_url(candidate)) {
            (Some(a), Some(b)) => dc_git::normalize_remote(a) == dc_git::normalize_remote(&b),
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

/// `Some(true)`: the copy is dirty, or HEAD or any local branch tip of it is a commit the
/// project's repository does not have. `Some(false)`: clean and every tip is contained.
/// `None`: git cannot tell.
fn has_own_work(project: &Project, copy: &Path) -> Option<bool> {
    let dirty = GitCli.state(copy).dirty;
    let mut tips: Vec<String> = git_head(copy).into_iter().collect();
    let refs = git_command(copy).args(["for-each-ref", "--format=%(objectname)", "refs/heads"]).output().ok();
    let listed = refs.as_ref().is_some_and(|o| o.status.success());
    if let Some(o) = refs {
        tips.extend(String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
    }
    tips.sort();
    tips.dedup();
    let mut unknown = !listed || tips.is_empty();
    let mut missing = false;
    for tip in &tips {
        match git_exit(&project.root, &["cat-file", "-e", tip]) {
            Some(false) => missing = true,
            Some(true) => {}
            None => unknown = true,
        }
    }
    match (dirty, missing) {
        (Some(true), _) | (_, true) => Some(true),
        (Some(false), false) if !unknown => Some(false),
        _ => None,
    }
}

fn git_command(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).arg("--no-optional-locks").env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

fn git_head(dir: &Path) -> Option<String> {
    let o = git_command(dir).args(["rev-parse", "--verify", "-q", "HEAD"]).output().ok()?;
    let sha = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !sha.is_empty()).then_some(sha)
}

/// `Some(true)` on exit 0, `Some(false)` on exit 1, `None` when git did not run or failed otherwise.
fn git_exit(dir: &Path, args: &[&str]) -> Option<bool> {
    match git_command(dir).args(args).output().ok()?.status.code()? {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

// ───────────────────────────────── helpers ─────────────────────────────────

/// The project and its worktrees, as `path_key`s.
struct Own {
    all: Vec<String>,
}

impl Own {
    fn new(project: &Project) -> Own {
        let mut all = vec![path_key(&project.root)];
        all.extend(project.worktrees.iter().map(|w| path_key(w)));
        Own { all }
    }

    /// `key` is the project or a worktree, or inside one.
    fn holds(&self, key: &str) -> bool {
        self.all.iter().any(|o| key_within(key, o))
    }

    /// `key` contains the project or a worktree (removing it would remove them).
    fn inside(&self, key: &str) -> bool {
        self.all.iter().any(|o| key_within(o, key))
    }
}

/// Symlink, junction or any other reparse point.
fn is_link(meta: &fs::Metadata) -> bool {
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
fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// Some ancestor of `path` (not `path` itself) is a git repository. Like git, the folders in
/// `GIT_CEILING_DIRECTORIES` and above them are not looked at.
fn inside_other_repo(path: &Path) -> bool {
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

fn home_dir() -> Option<PathBuf> {
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
fn read_index(drives: &[dc_disktree::FixedDrive]) -> Option<dc_disktree::DiskIndex> {
    let roots: Vec<PathBuf> = drives.iter().filter(|d| d.ntfs).map(|d| d.root()).collect();
    if roots.is_empty() {
        return None;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dc_disktree::build_for_roots(&roots, true, &dc_disktree::Progress::default()).0
    }))
    .ok()
    .flatten()
}

// ───────────────────────────────── the search ───────────────────────────────

/// Find the copies and the name-matched folders of `project`.
pub(crate) fn search(project: &Project, opts: &Options, env: &Env) -> Found {
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
                let drives = dc_disktree::fixed_drives();
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
    let mut seen = HashSet::new();
    candidates.retain(|c| seen.insert(path_key(c)));

    let roots: Vec<String> = if project.is_repo { dc_git::root_commits(&project.root) } else { Vec::new() };
    let items = candidates
        .into_iter()
        .filter_map(|c| {
            let (category, why) = classify(project, &roots, project.origin.as_deref(), &c)?;
            Some(Item {
                key: path_key(&c),
                category,
                bytes: opts.measure.then(|| dc_cleaner::measure_dir(&c).effective_bytes()),
                level: Level::Caution,
                blocked: Vec::new(),
                why,
                // `ArtifactKind::ProjectFolder` arrives with the cleaning plan (Task 5).
                target: Target::Dir { kind: ArtifactKind::Other },
                path: Some(c),
            })
        })
        .collect();
    Found { items, mode, notes }
}
