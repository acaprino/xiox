//! Where a path came from: shell commands from agent sessions, parsed into the operations they
//! perform on disk. Deterministic parsers only; no model guesses paths.

mod args;
mod braces;
mod budget;
mod call;
mod command;
mod dialect;
mod extract;
mod ledger;
mod lex;
mod op;
mod patch;
mod paths;
mod recognize;
mod script;

pub use args::posix;
pub use budget::Budget;
pub use command::Arg;
pub use call::{ShellCommand, shell_command};
pub use dialect::Shell;
pub use extract::extract;
pub use ledger::{
    Coverage, EvidenceRef, FolderEvidence, Ledger, SessionKey, Source, Strength, effective_cwd, folder_of,
    session_ledgers,
};
pub use lex::lex;
pub use op::{Candidate, Completeness, Extraction, Flags, OpKind, RawOp};
pub use patch::{PatchAction, patch_ops};
pub use paths::{Env, Resolved, resolve, resolve_path};
pub use script::{Lang, scan_script};

// Parser stages the unit tests reach into.
#[cfg(test)]
pub use {
    args::powershell,
    command::{Node, Simple, parse},
    lex::Token,
    paths::{PathError, is_null_device},
    recognize::{Context, Invocation, Registry},
};
