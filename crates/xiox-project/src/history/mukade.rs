//! The real [`CallSource`]: the tool calls of a session, dumped by Mukade.

use std::cell::OnceCell;

use xiox_model::SessionSource;
use xiox_mukade::{CallReader, Descriptor, DumpError, MukadeSessions, ToolCall};

use super::{CallSource, ReadFailure};

/// Reads sessions through one [`CallReader`] per agent. A reader is made when the first session
/// of its agent is asked for, so the copy of OpenCode's database happens once per analysis, and
/// only when an OpenCode session is read.
pub(crate) struct MukadeCalls<'a> {
    agents: Vec<AgentReader<'a>>,
}

struct AgentReader<'a> {
    sessions: &'a MukadeSessions,
    /// `None` once the reader could not be made (OpenCode without its native exe, no store).
    reader: OnceCell<Option<CallReader<'a>>>,
}

impl<'a> MukadeCalls<'a> {
    /// `sessions` holds one source per agent.
    pub fn new(sessions: &'a [MukadeSessions]) -> MukadeCalls<'a> {
        let agents = sessions.iter().map(|sessions| AgentReader { sessions, reader: OnceCell::new() }).collect();
        MukadeCalls { agents }
    }
}

impl CallSource for MukadeCalls<'_> {
    fn calls(&self, session: &Descriptor) -> Result<Vec<ToolCall>, ReadFailure> {
        let row = self.agents.iter().find(|a| a.sessions.agent() == session.agent).ok_or(ReadFailure::NoReader)?;
        let reader = row.reader.get_or_init(|| row.sessions.call_reader().ok()).as_ref().ok_or(ReadFailure::NoReader)?;
        reader.dump(&session.info.id, session.info.cwd.as_deref()).map_err(failure)
    }
}

/// The error text is dropped on purpose: it may carry what the session said.
fn failure(error: DumpError) -> ReadFailure {
    match error {
        DumpError::TimedOut(_) => ReadFailure::TimedOut,
        DumpError::TooLarge(_) => ReadFailure::TooLarge,
        DumpError::NotConfigured(_) | DumpError::Spawn(_) | DumpError::Exited(_) => ReadFailure::Failed,
    }
}
