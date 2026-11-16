//! What the sessions say about folders, kept per folder and merged per session on demand.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use xiox_mukade::{parse_time_ms, Descriptor};
use xiox_model::path_key;

use super::{Provenance, SessionEvidence};
use crate::owned::key_within;
use crate::Strength;

/// The evidence of every session, keyed by the `path_key` of the folder it names.
#[derive(Default)]
pub(crate) struct Folders {
    by_folder: BTreeMap<String, Vec<SessionEvidence>>,
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
        self.by_folder.entry(path_key(dir)).or_default().push(evidence);
    }

    /// The evidence of the folder `root_key` and of everything below it, one entry per session.
    pub fn provenance_within(&self, root_key: &str) -> Option<Provenance> {
        let mut merged: BTreeMap<(&str, &str, &str), SessionEvidence> = BTreeMap::new();
        let below = self.by_folder.iter().filter(|(k, _)| key_within(k, root_key)).flat_map(|(_, v)| v);
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
