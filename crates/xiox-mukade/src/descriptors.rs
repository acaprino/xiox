//! Sessions as Mukade lists them, before any aggregation per folder.
//!
//! [`MukadeSessions::list`] folds sessions into one record per project folder and drops Claude
//! Code's subagents. A consumer that needs each session (its provenance, its parent, its last
//! activity) reads [`Descriptor`]s instead.

use crate::{file_mtime_ms, MukadeSessions, SessionInfo};
use xiox_model::{Agent, Loaded};

/// One session of a store, with the names its tool calls carry.
#[derive(Clone, Debug)]
pub struct Descriptor {
    pub agent: Agent,
    /// Mukade's harness name, as in `ToolCall.harness`.
    pub harness: String,
    /// The configured store, as in `ToolCall.store`.
    pub store: String,
    pub info: SessionInfo,
}

impl Descriptor {
    /// The listed update time, else the session file's modification time. For recency and
    /// ordering only, never for provenance.
    pub fn updated_ms(&self) -> Option<i64> {
        self.info.updated.as_deref().and_then(parse_time_ms).or_else(|| file_mtime_ms(&self.info.path))
    }
}

/// Epoch milliseconds of an RFC 3339 time or of a number of seconds or milliseconds.
pub fn parse_time_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    let n: i64 = s.trim().parse().ok()?;
    // Heuristic: seconds vs milliseconds.
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

impl MukadeSessions {
    /// Every session of the store, forks and subagents included. A missing store lists nothing,
    /// quietly; a failed `mukade list` is one warning.
    pub fn descriptors(&self) -> Loaded<Descriptor> {
        let mut out = Loaded::default();
        let Some(store) = self.store_root().filter(|p| p.exists()) else { return out };
        match self.run_list(true) {
            Ok(infos) => {
                let store = store.to_string_lossy();
                out.items = infos
                    .into_iter()
                    .map(|info| Descriptor {
                        agent: self.agent,
                        harness: self.harness().to_string(),
                        store: store.to_string(),
                        info,
                    })
                    .collect();
            }
            Err(e) => out.warnings.push(format!("{:?}: {e:#}", self.agent)),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_claude_store, write_claude_subagent};
    use crate::StoreRoots;
    use std::path::Path;

    const ROOT_ID: &str = "11111111-2222-3333-4444-555555555555";

    fn claude(store: &Path, data: &Path) -> MukadeSessions {
        MukadeSessions::with_roots(
            Agent::Claude,
            StoreRoots { claude_config_dir: Some(store.to_path_buf()), ..Default::default() },
        )
        .with_cache_dir(Some(data.to_path_buf()))
    }

    #[test]
    fn descriptors_list_children_with_their_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("claude");
        write_claude_store(&store, ROOT_ID);
        write_claude_subagent(&store, ROOT_ID, "abc");
        let listed = claude(&store, &tmp.path().join("data")).descriptors();
        assert!(listed.warnings.is_empty(), "{:?}", listed.warnings);
        assert_eq!(listed.items.len(), 2, "{:?}", listed.items);
        let child = listed.items.iter().find(|d| d.info.parent.is_some()).expect("a child");
        assert_eq!(child.info.parent.as_deref(), Some(ROOT_ID));
        for d in &listed.items {
            assert_eq!(d.agent, Agent::Claude);
            assert_eq!(d.harness, "claude_code");
            assert_eq!(d.store, store.to_string_lossy());
        }
    }

    #[test]
    fn descriptor_store_matches_the_tool_call_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("claude");
        write_claude_store(&store, ROOT_ID);
        let sessions = claude(&store, &tmp.path().join("data"));
        let listed = sessions.descriptors();
        let calls = sessions.call_reader().unwrap().dump(ROOT_ID, None).unwrap();
        assert_eq!(listed.items[0].store, calls[0].store);
        assert_eq!(listed.items[0].harness, calls[0].harness);
    }

    #[test]
    fn missing_store_lists_nothing_quietly() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("nothing-here");
        let sessions = MukadeSessions::with_roots(
            Agent::Claude,
            StoreRoots { claude_config_dir: Some(gone), ..Default::default() },
        );
        let listed = sessions.descriptors();
        assert!(listed.items.is_empty());
        assert!(listed.warnings.is_empty(), "{:?}", listed.warnings);
    }

    #[test]
    fn updated_ms_reads_rfc3339_and_falls_back_to_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("s.jsonl");
        std::fs::write(&file, "{}").unwrap();
        let mtime = file_mtime_ms(file.to_str().unwrap()).unwrap();
        let at = |updated: Option<&str>| Descriptor {
            agent: Agent::Pi,
            harness: "pi".into(),
            store: String::new(),
            info: SessionInfo {
                updated: updated.map(String::from),
                path: file.to_string_lossy().into_owned(),
                ..Default::default()
            },
        };
        let cases = [
            (Some("2026-11-01T10:00:00.000Z"), Some(1_793_527_200_000)),
            (Some("not a time"), Some(mtime)),
            (None, Some(mtime)),
        ];
        for (updated, want) in cases {
            assert_eq!(at(updated).updated_ms(), want, "{updated:?}");
        }
    }
}
