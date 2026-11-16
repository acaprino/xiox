//! How an item goes away: the method its target and the removal mode give.

use serde::Serialize;
use xiox_model::RemoveMode;

use crate::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    RecycleBin,
    Permanent,
    GitWorktreeRemove,
    HarnessSession,
}

impl Method {
    /// The English description, in the one wording used by the summary and the guide.
    pub fn describe(self) -> &'static str {
        match self {
            Method::RecycleBin => "to the Recycle Bin",
            Method::Permanent => "deleted for good",
            Method::GitWorktreeRemove => "removed by git for good, not to the Recycle Bin",
            Method::HarnessSession => "removed with the session tools of its harness",
        }
    }
}

/// Worktrees go through `git worktree remove` and sessions through their harness in every mode;
/// everything else follows `mode`.
pub fn method_of(target: &Target, mode: RemoveMode) -> Method {
    match (target, mode) {
        (Target::Worktree { .. }, _) => Method::GitWorktreeRemove,
        (Target::Session { .. }, _) => Method::HarnessSession,
        (_, RemoveMode::Recycle) => Method::RecycleBin,
        (_, RemoveMode::Permanent) => Method::Permanent,
    }
}
