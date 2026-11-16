//! Sessions of other projects near the candidates: who else worked where this project's
//! sessions left a folder.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use xiox_model::{path_key, Agent, Loaded};
use xiox_mukade::Descriptor;
use xiox_scanner::is_broad_dir;

use super::scope::local_dir;
use crate::owned::key_within;

/// `(store, harness, id)` of a session.
pub(crate) type SessionId = (String, String, String);

pub(crate) fn session_id(d: &Descriptor) -> SessionId {
    (d.store.clone(), d.harness.clone(), d.info.id.clone())
}

/// The sessions outside `related` whose listed working folder is near one of `candidates`
/// (`(key, path)`), with the indexes of those candidates: the most recent first, then by
/// `(store, harness, id)`. Near means the candidate itself, a folder inside it, or a folder
/// above it that is not a drive root, the home or a broad folder.
pub(crate) fn writers_near<'a>(
    listings: &'a [(Agent, Loaded<Descriptor>)],
    related: &HashSet<SessionId>,
    candidates: &[(String, PathBuf)],
    home: Option<&Path>,
) -> Vec<(&'a Descriptor, Vec<usize>)> {
    let mut near: Vec<(&'a Descriptor, Vec<usize>)> = listings
        .iter()
        .flat_map(|(_, l)| &l.items)
        .filter(|d| !related.contains(&session_id(d)))
        .filter_map(|d| {
            let cwd = local_dir(d.info.cwd.as_deref()?).ok()?;
            let at = path_key(&cwd);
            let wide = cwd.parent().is_none()
                || home.is_some_and(|h| path_key(h) == at)
                || is_broad_dir(&cwd, home);
            let hits: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, (key, _))| key_within(&at, key) || (!wide && key_within(key, &at)))
                .map(|(i, _)| i)
                .collect();
            (!hits.is_empty()).then_some((d, hits))
        })
        .collect();
    near.sort_by_cached_key(|(d, _)| (Reverse(d.updated_ms()), session_id(d)));
    near
}
