//! What the sessions say about folders, kept per folder and merged per session on demand.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use xiox_mukade::{parse_time_ms, Descriptor};
use xiox_model::path_key;

use super::{Operation, Provenance, SessionEvidence};
use crate::owned::key_within;
use crate::{Coverage, FolderEvidence, Ledger, Source, Strength};

/// Operations kept per session and folder; the rest is only counted.
pub(crate) const MAX_OPERATIONS_SHOWN: usize = 20;

/// How firmly the sessions tie a folder to their own work, and whether scripts alone show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Standing {
    pub strength: Strength,
    /// Every operation that gives `strength` comes from a script.
    pub from_script: bool,
}

impl Standing {
    /// The stronger of the two; at equal strength it is script-only only if both are.
    pub fn merge(self, other: Standing) -> Standing {
        use std::cmp::Ordering::*;
        match self.strength.cmp(&other.strength) {
            Greater => self,
            Less => other,
            Equal => Standing { from_script: self.from_script && other.from_script, ..self },
        }
    }

    fn of(folder: &FolderEvidence) -> Option<Standing> {
        let strength = folder.strength?;
        let from_script = folder
            .refs
            .iter()
            .filter(|r| r.strength == Some(strength))
            .all(|r| matches!(r.source, Source::Shell { script: true, .. }));
        Some(Standing { strength, from_script })
    }
}

/// A folder a session wrote in or created.
pub(crate) struct Marked<'a> {
    pub key: &'a str,
    pub path: &'a Path,
    pub standing: Standing,
}

/// What is known of one folder: how it was first spelled and what each session showed of it.
struct Entry {
    path: PathBuf,
    standing: Option<Standing>,
    sessions: Vec<SessionEvidence>,
}

/// The evidence of every session, keyed by the `path_key` of the folder it names.
#[derive(Default)]
pub(crate) struct Folders {
    by_folder: BTreeMap<String, Entry>,
    /// Folders a ledger named first, not yet handed to `take_fresh`.
    fresh: Vec<PathBuf>,
}

impl Folders {
    /// The session's own listing names `dir` as its working directory.
    pub fn add_listed(&mut self, dir: &Path, d: &Descriptor) {
        let evidence = SessionEvidence {
            agent: d.agent,
            harness: d.harness.clone(),
            store: d.store.clone(),
            id: d.info.id.clone(),
            parent: d.info.parent.clone(),
            last_activity_ms: d.updated_ms(),
            strength: Some(Strength::Used),
            listed_cwd: true,
            operations: Vec::new(),
            omitted: 0,
            failed: 0,
            gaps: false,
        };
        self.entry(dir).sessions.push(evidence);
    }

    /// What the session's ledger shows: one entry per folder, kept apart from the listed working directory.
    pub fn add_ledger(&mut self, d: &Descriptor, ledger: &Ledger) {
        let gaps = has_gaps(ledger.coverage());
        for folder in ledger.folders() {
            let path = Path::new(&folder.path);
            if !self.by_folder.contains_key(&path_key(path)) {
                self.fresh.push(path.to_path_buf());
            }
            let entry = self.entry(path);
            entry.standing = match (entry.standing, Standing::of(folder)) {
                (Some(a), Some(b)) => Some(a.merge(b)),
                (a, b) => a.or(b),
            };
            entry.sessions.push(session_evidence(d, folder, gaps));
        }
    }

    fn entry(&mut self, dir: &Path) -> &mut Entry {
        self.by_folder
            .entry(path_key(dir))
            .or_insert_with(|| Entry { path: dir.to_path_buf(), standing: None, sessions: Vec::new() })
    }

    /// The folders some session wrote in or created, parents before the folders inside them.
    pub fn marked(&self) -> impl Iterator<Item = Marked<'_>> {
        self.by_folder.iter().filter_map(|(key, e)| {
            let standing = e.standing.filter(|s| s.strength >= Strength::Written)?;
            Some(Marked { key, path: &e.path, standing })
        })
    }

    /// The folders ledgers named since the last call that no listing had named before.
    pub fn take_fresh(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.fresh)
    }

    /// The evidence of the folder `root_key` and of everything below it, one entry per session.
    pub fn provenance_within(&self, root_key: &str) -> Option<Provenance> {
        let mut merged: BTreeMap<(&str, &str, &str), SessionEvidence> = BTreeMap::new();
        let below = self.by_folder.iter().filter(|(k, _)| key_within(k, root_key)).flat_map(|(_, e)| &e.sessions);
        for e in below {
            match merged.get_mut(&(e.store.as_str(), e.harness.as_str(), e.id.as_str())) {
                Some(m) => merge(m, e),
                None => {
                    merged.insert((&e.store, &e.harness, &e.id), e.clone());
                }
            }
        }
        let mut sessions: Vec<SessionEvidence> = merged.into_values().collect();
        if sessions.is_empty() {
            return None;
        }
        for s in &mut sessions {
            s.last_activity_ms = observed_activity(s);
        }
        // Most recent first, the undated after them; the map order (store, harness, id) is stable.
        sessions.sort_by_key(|s| (s.last_activity_ms.is_none(), Reverse(s.last_activity_ms)));
        Some(Provenance {
            strength: sessions.iter().filter_map(|s| s.strength).max(),
            last_activity_ms: sessions.iter().filter_map(|s| s.last_activity_ms).max(),
            sessions,
        })
    }
}

/// Folds `other`, another folder's evidence of the same session, into `into`.
fn merge(into: &mut SessionEvidence, other: &SessionEvidence) {
    into.strength = into.strength.max(other.strength);
    into.listed_cwd |= other.listed_cwd;
    into.operations.extend(other.operations.iter().cloned());
    into.omitted += other.omitted;
    into.failed += other.failed;
    into.gaps |= other.gaps;
}

/// The latest of the listed update time and the times of the operations.
fn observed_activity(s: &SessionEvidence) -> Option<i64> {
    let ops = s.operations.iter().filter_map(|o| o.time.as_deref().and_then(parse_time_ms));
    ops.chain(s.last_activity_ms).max()
}

fn session_evidence(d: &Descriptor, folder: &FolderEvidence, gaps: bool) -> SessionEvidence {
    SessionEvidence {
        agent: d.agent,
        harness: d.harness.clone(),
        store: d.store.clone(),
        id: d.info.id.clone(),
        parent: d.info.parent.clone(),
        last_activity_ms: d.updated_ms(),
        strength: folder.strength,
        listed_cwd: false,
        operations: folder.refs.iter().take(MAX_OPERATIONS_SHOWN).map(Operation::from).collect(),
        omitted: folder.refs.len().saturating_sub(MAX_OPERATIONS_SHOWN),
        failed: folder.failed.len(),
        gaps,
    }
}

/// The session has shell calls the parsers could not fully read.
fn has_gaps(c: &Coverage) -> bool {
    c.unresolved + c.looped + c.script + c.truncated + c.exhausted > 0
}
