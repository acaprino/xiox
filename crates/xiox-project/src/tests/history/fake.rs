//! Fake session listings for the history tests: descriptors and the input that carries them.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{json, Value};
use xiox_model::{Agent, Loaded};
use xiox_mukade::{Descriptor, SessionInfo, ToolCall};

use crate::history::{CallSource, HistoryInput, ReadFailure, UserDirs};

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

/// A tool call of the session `s1` in the store `store`; `canonical` is Mukade's tool name.
pub fn call(canonical: &str, tool: &str, input: Value, cwd: Option<&str>) -> ToolCall {
    ToolCall {
        harness: "test".into(),
        store: "store".into(),
        session: "s1".into(),
        parent: None,
        entry: Some("e1".into()),
        call: Some("c1".into()),
        tool: Some(tool.into()),
        canonical: Some(canonical.into()),
        input,
        cwd: cwd.map(Into::into),
        time: Some("2026-11-15T10:00:00Z".into()),
        error: None,
        exit: None,
        result_head: Some("ok".into()),
    }
}

pub fn bash(command: &str, cwd: &str) -> ToolCall {
    call("bash", "Bash", json!({ "command": command }), Some(cwd))
}

/// A session reader over canned answers, keyed by `(store, id)`; it remembers the order it was
/// asked in. A session with no answer has no calls.
#[derive(Default)]
pub struct FakeCalls {
    answers: HashMap<(String, String), Result<Vec<ToolCall>, ReadFailure>>,
    asked: RefCell<Vec<String>>,
}

impl FakeCalls {
    pub fn answer(mut self, store: &str, id: &str, answer: Result<Vec<ToolCall>, ReadFailure>) -> Self {
        self.answers.insert((store.into(), id.into()), answer);
        self
    }

    /// The ids of the sessions read so far, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }
}

impl CallSource for FakeCalls {
    fn calls(&self, session: &Descriptor) -> Result<Vec<ToolCall>, ReadFailure> {
        self.asked.borrow_mut().push(session.info.id.clone());
        self.answers.get(&(session.store.clone(), session.info.id.clone())).cloned().unwrap_or(Ok(Vec::new()))
    }
}
