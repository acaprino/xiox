//! The git repositories of the PC, found once, and what makes two of them the same repository.
//!
//! One identity per repository (its root commits and its normalized `origin`), computed once for the whole PC instead of
//! once per project and candidate: the desktop asks git about each repository a fixed number of times, then groups the
//! repositories that are copies of each other. A repository git must not read ([`git_may_read`]) never enters.

use crate::gitref::{git_home, git_may_read};
use crate::search::{is_git_repo, is_link};
use rayon::prelude::*;
use xiox_model::{is_system_dir, normalize_path, path_key};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// What a repository is, for telling copies apart: its root (first) commits and its `origin`, normalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoIdentity {
    pub root: PathBuf,
    pub root_commits: Vec<String>,
    /// `remote.origin.url` through [`xiox_git::normalize_remote`].
    pub origin: Option<String>,
}

/// The repositories of the PC in `path_key` order, and the groups of those that are the same repository.
#[derive(Clone, Debug, Default)]
pub struct RepoIndex {
    pub repos: Vec<RepoIdentity>,
    /// Indexes into `repos`, each group of at least two, in `path_key` order; the groups in the order of their first member.
    pub(crate) groups: Vec<Vec<usize>>,
}

impl RepoIndex {
    /// The identities of `roots` that are repositories git may read (doubles dropped by `path_key`), asked of git in
    /// parallel, and their groups.
    pub fn build(roots: &[PathBuf]) -> RepoIndex {
        let home = git_home();
        let mut seen = HashSet::new();
        let mut dirs: Vec<PathBuf> = roots
            .iter()
            .map(|r| normalize_path(r))
            .filter(|r| seen.insert(path_key(r)))
            .filter(|r| is_git_repo(r) && git_may_read(r, home.as_deref()))
            .collect();
        dirs.sort_by_key(|d| path_key(d));
        let repos: Vec<RepoIdentity> = dirs
            .into_par_iter()
            .map(|root| RepoIdentity {
                root_commits: xiox_git::root_commits(&root),
                origin: xiox_git::origin_url(&root).map(|o| xiox_git::normalize_remote(&o)),
                root,
            })
            .collect();
        let ids: Vec<(Vec<String>, Option<String>)> = repos.iter().map(|r| (r.root_commits.clone(), r.origin.clone())).collect();
        RepoIndex { groups: groups(&ids), repos }
    }

    /// The group that holds the repository at `root` (any spelling); `None` when it is the only one of its kind.
    pub fn group_of(&self, root: &Path) -> Option<&[usize]> {
        let key = path_key(root);
        self.groups.iter().find(|g| g.iter().any(|&i| path_key(&self.repos[i].root) == key)).map(Vec::as_slice)
    }

    /// Every group, as indexes into `repos`.
    pub fn groups(&self) -> &[Vec<usize>] {
        &self.groups
    }

    /// The member of `group` the others are copies of: the one with the most recent `activity`; on a tie, or with no
    /// activity at all, the first by `path_key` (the group is in that order).
    pub fn reference(&self, group: &[usize], activity: &dyn Fn(&Path) -> Option<i64>) -> usize {
        let mut best = group[0];
        let mut best_at = activity(&self.repos[best].root);
        for &i in &group[1..] {
            let at = activity(&self.repos[i].root);
            if at > best_at {
                best = i;
                best_at = at;
            }
        }
        best
    }
}

/// The groups of `ids` (root commits, origin): two are joined by a root commit in common or by the same origin, and
/// joined ones join whatever each of them joins. Only groups of two or more, each in index order, in the order of their
/// first index.
pub(crate) fn groups(ids: &[(Vec<String>, Option<String>)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut at = i;
        while parent[at] != root {
            let next = parent[at];
            parent[at] = root;
            at = next;
        }
        root
    }
    // The first repository that showed each commit or origin: a later one with the same joins it.
    let mut first: HashMap<String, usize> = HashMap::new();
    for (i, (commits, origin)) in ids.iter().enumerate() {
        let marks = commits.iter().filter(|c| !c.is_empty()).map(|c| format!("c:{c}"));
        let marks = marks.chain(origin.iter().filter(|o| !o.is_empty()).map(|o| format!("o:{o}")));
        for mark in marks {
            match first.get(&mark) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    // The smaller index is the root: a group's root is its first member.
                    parent[a.max(b)] = a.min(b);
                }
                None => {
                    first.insert(mark, i);
                }
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..ids.len() {
        let root = find(&mut parent, i);
        by_root.entry(root).or_default().push(i);
    }
    by_root.into_values().filter(|g| g.len() >= 2).collect()
}

/// The git repositories under `roots` (`(root, depth)`; depth 1 = the root's children), found by walking: no link is
/// followed, system folders and `AppData` are skipped, and a repository is listed but never entered. A root that is a
/// repository is listed itself. In `path_key` order, without doubles.
pub fn repositories_under(roots: &[(PathBuf, usize)]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (root, depth) in roots {
        let root = normalize_path(root);
        if is_git_repo(&root) {
            if seen.insert(path_key(&root)) {
                out.push(root);
            }
            continue;
        }
        walk(&root, *depth, &mut seen, &mut out);
    }
    out.sort_by_key(|p| path_key(p));
    out
}

fn walk(dir: &Path, levels: usize, seen: &mut HashSet<String>, out: &mut Vec<PathBuf>) {
    if levels == 0 {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    let parent_is_root = dir.parent().is_none();
    for entry in read.flatten() {
        // `DirEntry::metadata` does not follow links: a link is never a directory here.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || is_link(&meta) || is_system_dir(&entry.file_name().to_string_lossy(), parent_is_root) {
            continue;
        }
        let path = entry.path();
        if is_git_repo(&path) {
            if seen.insert(path_key(&path)) {
                out.push(normalize_path(&path));
            }
        } else {
            walk(&path, levels - 1, seen, out);
        }
    }
}
