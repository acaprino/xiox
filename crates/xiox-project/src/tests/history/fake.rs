//! Fake session listings for the history tests: descriptors and the input that carries them.

use xiox_model::{Agent, Loaded};
use xiox_mukade::{Descriptor, SessionInfo};

use crate::history::{CallSource, HistoryInput, UserDirs};

/// A listed session; `updated` is an RFC 3339 time, `parent` the native parent link.
pub fn descriptor(agent: Agent, store: &str, id: &str, cwd: &str, updated: &str, parent: Option<&str>) -> Descriptor {
    Descriptor {
        agent,
        harness: crate::owned::agent_slug(agent).into(),
        store: store.into(),
        info: SessionInfo {
            id: id.into(),
            cwd: Some(cwd.into()),
            updated: Some(updated.into()),
            parent: parent.map(String::from),
            parent_kind: parent.map(|_| "spawn".into()),
            ..SessionInfo::default()
        },
    }
}

/// One listing per agent, as the real input has them.
pub fn listing(agent: Agent, items: Vec<Descriptor>) -> (Agent, Loaded<Descriptor>) {
    (agent, Loaded { items, warnings: Vec::new() })
}

pub fn input<'a>(
    listings: &'a [(Agent, Loaded<Descriptor>)],
    calls: Option<&'a dyn CallSource>,
    user: &'a UserDirs,
) -> HistoryInput<'a> {
    HistoryInput { listings, calls, user }
}
