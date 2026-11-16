//! Which session folders the project may look at, and which project roots they lead to. Every
//! step before the last one is lexical: a folder outside the scope is counted, never opened.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use xiox_model::{normalize_path, path_key};
use xiox_mukade::strip_verbatim;
use xiox_scanner::{project_roots_from_sessions, Skip};

use super::Exclusion;
use crate::owned::key_within;
use crate::search::{is_link, Own};
use crate::{Env, Options, Project};

/// Where `--no-drives` and the search roots let the analysis reach, as `path_key`s.
pub(crate) enum Scope {
    Everywhere,
    Within(Vec<String>),
}

impl Scope {
    /// The same reach as the search for copies: the project's parent, and the drives (or the
    /// injected search roots) unless `--no-drives`.
    pub fn of(project: &Project, opts: &Options, env: &Env) -> Scope {
        if opts.drives && env.search_roots.is_none() {
            return Scope::Everywhere;
        }
        let mut keys: Vec<String> = project.root.parent().map(path_key).into_iter().collect();
        if opts.drives {
            keys.extend(env.search_roots.iter().flatten().map(|r| path_key(r)));
        }
        Scope::Within(keys)
    }

    pub fn contains(&self, key: &str) -> bool {
        match self {
            Scope::Everywhere => true,
            Scope::Within(roots) => roots.iter().any(|r| key_within(key, r)),
        }
    }
}

/// A listing's working directory as a local absolute folder, from the text alone. Git Bash forms
/// (`/d/x`) need the context of a command to resolve, which a listing does not have.
pub(crate) fn local_dir(raw: &str) -> Result<PathBuf, Exclusion> {
    let plain = strip_verbatim(raw).replace('/', "\\");
    let bytes = plain.as_bytes();
    let absolute = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !absolute || plain.split('\\').any(|c| c == "..") {
        return Err(Exclusion::NotLocal);
    }
    Ok(normalize_path(Path::new(&plain)))
}

/// The drives that are fixed local disks, asked of the system once and only when needed.
#[derive(Default)]
pub(crate) struct FixedDrives(OnceCell<Vec<char>>);

impl FixedDrives {
    pub fn holds(&self, path: &Path) -> bool {
        let letter = path.to_string_lossy().chars().next().map(|c| c.to_ascii_uppercase());
        let drives = self.0.get_or_init(|| xiox_disktree::fixed_drives().iter().map(|d| d.letter).collect());
        letter.is_some_and(|l| drives.contains(&l))
    }
}

/// Looks at the folder and every ancestor from the top down without following a link: the first
/// link, or the first one that cannot be read, is why the folder is left out.
pub(crate) fn unlinked_chain(path: &Path) -> Result<(), Exclusion> {
    let mut chain: Vec<&Path> = path.ancestors().filter(|a| a.parent().is_some()).collect();
    chain.reverse();
    for dir in chain {
        match dir.symlink_metadata() {
            Ok(meta) if is_link(&meta) => return Err(Exclusion::Link),
            Ok(_) => {}
            Err(_) => return Err(Exclusion::Missing),
        }
    }
    Ok(())
}

/// The project roots the session directories `dirs` lead to, other than the project itself.
/// Each directory is counted once in `excluded` when it, or the root it leads to, is left out.
pub(crate) fn cwd_roots(
    project: &Project,
    dirs: &[PathBuf],
    scope: &Scope,
    home: Option<&Path>,
    excluded: &mut BTreeMap<Exclusion, usize>,
) -> Vec<PathBuf> {
    let own = Own::new(project);
    let mut count = |why: Exclusion| *excluded.entry(why).or_default() += 1;
    let drives = FixedDrives::default();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for dir in dirs.iter().filter(|d| seen.insert(path_key(d))) {
        let key = path_key(dir);
        if !scope.contains(&key) {
            count(Exclusion::OutsideScope);
        } else if own.holds(&key) {
            count(Exclusion::InsideProject);
        } else if !drives.holds(dir) {
            count(Exclusion::NotLocal);
        } else if let Err(why) = unlinked_chain(dir) {
            count(why);
        } else {
            candidates.push(dir.clone());
        }
    }

    let found = project_roots_from_sessions(&candidates, home);
    for (_, skip) in &found.skipped {
        count(match skip {
            Skip::Missing => Exclusion::Missing,
            Skip::Broad | Skip::Cloud => Exclusion::Protected,
            Skip::NotAProject => Exclusion::NotAProject,
        });
    }
    found
        .roots
        .into_iter()
        .filter(|root| {
            let key = path_key(root);
            // A repository above the session folder can lie beyond the scope: never climb out.
            let kept = scope.contains(&key) && !own.holds(&key);
            if !kept {
                count(if own.holds(&key) { Exclusion::InsideProject } else { Exclusion::OutsideScope });
            }
            kept
        })
        .collect()
}
