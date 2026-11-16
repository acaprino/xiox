//! The history part of the analysis result: the evidence behind a folder and the report of what
//! was read. Fields are camelCase and values kebab-case, like the rest of the JSON.

use std::collections::BTreeMap;

use serde::Serialize;
use xiox_model::Agent;
use xiox_mukade::Outcome;

use crate::{Coverage, EvidenceRef, OpKind, PatchAction, Source, Strength};

/// What makes a session folder risky to remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    NoGit,
    WeakEvidence,
    PartialCoverage,
    RecentActivity,
}

/// Whether a session that touched the folder may still be running.
// shortcut: no source verifies "not active" (Mukade's `live: None` means "not known"); add the
// variant when one does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Activity {
    Active,
    Unknown,
}

/// How a tool call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallOutcome {
    Succeeded,
    Failed,
    Unknown,
}

impl From<Outcome> for CallOutcome {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Succeeded => CallOutcome::Succeeded,
            Outcome::Failed => CallOutcome::Failed,
            Outcome::Unknown => CallOutcome::Unknown,
        }
    }
}

/// Which part of a tool call showed the folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OperationSource {
    Tool { tool: String },
    Shell { op: OpKind, script: bool },
    Patch { action: PatchAction },
    Cd,
    Cwd,
}

impl From<&Source> for OperationSource {
    fn from(s: &Source) -> Self {
        match s {
            Source::Structured { tool, .. } => OperationSource::Tool { tool: tool.clone() },
            Source::Shell { kind, script } => OperationSource::Shell { op: *kind, script: *script },
            Source::Patch(action) => OperationSource::Patch { action: *action },
            Source::Cd => OperationSource::Cd,
            Source::Cwd => OperationSource::Cwd,
        }
    }
}

/// One piece of evidence, as a reference into the session: never the text of a command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub source: OperationSource,
    pub outcome: CallOutcome,
    pub strength: Option<Strength>,
    pub entry: Option<String>,
    pub call: Option<String>,
    pub time: Option<String>,
    pub cwd: Option<String>,
}

impl From<&EvidenceRef> for Operation {
    fn from(r: &EvidenceRef) -> Self {
        Operation {
            source: (&r.source).into(),
            outcome: r.outcome.into(),
            strength: r.strength,
            entry: r.entry.clone(),
            call: r.call.clone(),
            time: r.time.clone(),
            cwd: r.cwd.clone(),
        }
    }
}

/// What one session shows about one folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvidence {
    pub agent: Agent,
    pub harness: String,
    pub store: String,
    pub id: String,
    pub parent: Option<String>,
    pub last_activity_ms: Option<i64>,
    pub strength: Option<Strength>,
    /// The session's own listing names the folder as its working directory.
    pub listed_cwd: bool,
    pub operations: Vec<Operation>,
    /// Operations left out of `operations` to keep the result small.
    pub omitted: usize,
    pub failed: usize,
    /// The session has shell calls the parsers could not fully read.
    pub gaps: bool,
}

/// The evidence behind a session folder. `sessions` are distinct by `(store, harness, id)`, the
/// most recently active first, then those without a date, then by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub strength: Option<Strength>,
    pub last_activity_ms: Option<i64>,
    pub sessions: Vec<SessionEvidence>,
}

/// Why the calls of a session could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadFailure {
    NoReader,
    TimedOut,
    TooLarge,
    Failed,
}

/// Why a folder the history named is not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Exclusion {
    NotLocal,
    OutsideScope,
    Protected,
    InsideProject,
    OtherRepository,
    NotAProject,
    Missing,
    Link,
}

/// A cap that stopped the reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Limit {
    Sessions { max: usize, left: usize },
    Calls { max: usize },
    Duration { secs: u64, left: usize },
}

/// What one agent's store gave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReport {
    pub agent: Agent,
    pub listing_failed: bool,
    pub listed: usize,
    pub related: usize,
    pub read: usize,
    pub failed: BTreeMap<ReadFailure, usize>,
}

/// What the history analysis read and left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryReport {
    pub sources: Vec<SourceReport>,
    pub sessions_read: usize,
    pub calls: usize,
    pub shell: Coverage,
    pub excluded: BTreeMap<Exclusion, usize>,
    pub limits: Vec<Limit>,
}

impl HistoryReport {
    /// Something was not read: a cap was hit, a listing failed, or a session could not be read.
    pub fn partial(&self) -> bool {
        !self.limits.is_empty() || self.sources.iter().any(|s| s.listing_failed || !s.failed.is_empty())
    }
}
