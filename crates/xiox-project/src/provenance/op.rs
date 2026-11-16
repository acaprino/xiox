//! Value types of the extractor: what a command does to disk, before and after resolution.

use serde::Serialize;

/// What an operation does to its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpKind {
    Create,
    Copy,
    Move,
    Clone,
    Worktree,
    Write,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OpKind::Create => "create",
            OpKind::Copy => "copy",
            OpKind::Move => "move",
            OpKind::Clone => "clone",
            OpKind::Worktree => "worktree",
            OpKind::Write => "write",
        }
    }
}

/// One step read from a command, with paths exactly as written (not yet resolved).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawOp {
    /// An operation on disk. `base` is the directory the paths are relative to when the command
    /// itself named one (`git -C`, `cwd=` in a script); `script` marks a path read from an inline script.
    Path { kind: OpKind, src: Option<String>, dst: Option<String>, base: Option<String>, script: bool },
    /// `cd` or `pushd`: context for the paths that follow, never a candidate.
    Cd { dst: Option<String>, push: bool },
    /// `popd`: back to the directory before the last `pushd`.
    PopCd,
}

/// Why an extraction may be partial or approximate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub looped: bool,
    pub script: bool,
    pub truncated: bool,
    pub exhausted: bool,
}

/// Whether the command text the extractor saw was the whole command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Whole,
    Truncated,
}

/// What the extractor read from one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extraction {
    pub ops: Vec<RawOp>,
    pub flags: Flags,
    /// What the extractor could not read: non-literal arguments in scripts, operations beyond the
    /// budget, the last command of a truncated text. Paths the resolver cannot resolve are added later.
    pub unresolved: usize,
}

/// A resolved, absolute operation: a path the command created, copied, moved or wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: OpKind,
    pub src: Option<String>,
    pub dst: String,
    pub script: bool,
}
