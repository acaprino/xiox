//! Where a path came from: shell commands from agent sessions, parsed into the operations they
//! perform on disk. Deterministic parsers only; no model guesses paths.

mod command;
mod dialect;
mod lex;
mod op;
mod paths;

pub use command::{Arg, Node, Simple, parse};
pub use dialect::Shell;
pub use lex::{Token, lex};
pub use op::{Candidate, Completeness, Extraction, Flags, OpKind, RawOp};
pub use paths::{Env, PathError, Resolved, is_null_device, resolve, resolve_path, strip_verbatim};
