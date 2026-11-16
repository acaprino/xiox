//! Which listed sessions belong to the project, and in which order they are read.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use xiox_model::{path_key, Agent, Loaded};
use xiox_mukade::Descriptor;

use super::scope::local_dir;
use crate::owned::key_within;

/// How much of the history one analysis may read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    pub max_sessions: usize,
    pub max_calls: usize,
    /// Checked before each `dump`; a `dump` already running is bounded by its own timeout.
    pub max_duration: Duration,
}

// shortcut: starting values, to be tuned with the `session_probe` example on real stores.
pub(crate) const BUDGET: Budget =
    Budget { max_sessions: 250, max_calls: 250_000, max_duration: Duration::from_secs(240) };

/// A session is told apart by its store and harness as well as its id.
type Key<'a> = (&'a str, &'a str, &'a str);

fn key_of(d: &Descriptor) -> Key<'_> {
    (&d.store, &d.harness, &d.info.id)
}

struct Entry<'a> {
    session: &'a Descriptor,
    /// `path_key` of the working directory, when the listing gives a local one.
    cwd: Option<String>,
    /// Position of the parent session in the queue, when it is listed in the same store.
    parent: Option<usize>,
    related: bool,
    taken: bool,
}

/// The sessions of the project, newest first. A session is related when its working directory is
/// in a root (the project, a worktree, a verified copy) or its parent is related.
pub(crate) struct Queue<'a> {
    entries: Vec<Entry<'a>>,
    roots: Vec<String>,
}

impl<'a> Queue<'a> {
    pub fn new(listings: &'a [(Agent, Loaded<Descriptor>)], roots: &[PathBuf]) -> Queue<'a> {
        let mut unique: HashMap<Key<'a>, &'a Descriptor> = HashMap::new();
        for session in listings.iter().flat_map(|(_, l)| &l.items) {
            unique.entry(key_of(session)).or_insert(session);
        }
        let mut sessions: Vec<&'a Descriptor> = unique.into_values().collect();
        // Most recent first; the undated and the ties by (store, harness, id).
        sessions.sort_by_cached_key(|&s| (Reverse(s.updated_ms()), key_of(s)));
        let position: HashMap<Key<'a>, usize> = sessions.iter().enumerate().map(|(i, s)| (key_of(s), i)).collect();
        let entries = sessions
            .iter()
            .map(|&session| Entry {
                session,
                cwd: session.info.cwd.as_deref().and_then(|c| local_dir(c).ok()).map(|d| path_key(&d)),
                parent: session
                    .info
                    .parent
                    .as_deref()
                    .and_then(|p| position.get(&(session.store.as_str(), session.harness.as_str(), p)).copied()),
                related: false,
                taken: false,
            })
            .collect();
        let mut queue = Queue { entries, roots: roots.iter().map(|r| path_key(r)).collect() };
        queue.relate();
        queue
    }

    /// A new root (a copy a session made): the sessions that worked in it are now related.
    pub fn add_root(&mut self, root: &Path) {
        self.roots.push(path_key(root));
        self.relate();
    }

    /// The most recent related session not read yet.
    pub fn next(&mut self) -> Option<&'a Descriptor> {
        let entry = self.entries.iter_mut().find(|e| e.related && !e.taken)?;
        entry.taken = true;
        Some(entry.session)
    }

    /// The related sessions not read yet.
    pub fn left(&self) -> usize {
        self.entries.iter().filter(|e| e.related && !e.taken).count()
    }

    /// Every related session, read or not.
    pub fn related(&self) -> impl Iterator<Item = &'a Descriptor> + '_ {
        self.entries.iter().filter(|e| e.related).map(|e| e.session)
    }

    /// Marks the sessions in a root, then their descendants, until nothing changes.
    fn relate(&mut self) {
        for e in &mut self.entries {
            e.related |= e.cwd.as_deref().is_some_and(|c| self.roots.iter().any(|r| key_within(c, r)));
        }
        loop {
            let related: Vec<bool> = self.entries.iter().map(|e| e.related).collect();
            let mut changed = false;
            for e in self.entries.iter_mut().filter(|e| !e.related) {
                if e.parent.is_some_and(|p| related[p]) {
                    e.related = true;
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }
}
