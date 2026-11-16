//! Which session folders the project may look at, and which project roots they lead to. Every
//! step before the last one is lexical: a folder outside the scope is counted, never opened.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use xiox_model::{normalize_path, path_key};
use xiox_mukade::strip_verbatim;
use xiox_scanner::{project_roots_from_sessions, Skip};

use super::Exclusion;
use crate::owned::key_within;
use crate::search::Own;
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
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for dir in dirs.iter().filter(|d| seen.insert(path_key(d))) {
        let key = path_key(dir);
        if !scope.contains(&key) {
            count(Exclusion::OutsideScope);
        } else if own.holds(&key) {
            count(Exclusion::InsideProject);
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
