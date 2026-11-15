//! Where a path came from: shell commands from agent sessions, parsed into the operations they
//! perform on disk. Deterministic parsers only; no model guesses paths.

mod op;

pub use op::{Candidate, Completeness, Extraction, Flags, OpKind, RawOp};
