//! What the session history says about folders: the shape of the result and the inputs the
//! analysis reads. The logic that fills the shape lives in the modules of the later stages.

mod report;

use std::path::PathBuf;
use std::sync::LazyLock;

use xiox_model::{Agent, Loaded};
use xiox_mukade::{strip_verbatim, Descriptor, ToolCall};

use crate::PathEnv;

pub use report::{
    Activity, CallOutcome, Exclusion, HistoryReport, Limit, Operation, OperationSource, Provenance, ReadFailure,
    Risk, SessionEvidence, SourceReport,
};

/// The user directories a command may name through `~` and `/tmp`.
#[derive(Clone, Debug, Default)]
pub(crate) struct UserDirs {
    pub home: PathBuf,
    pub temp: PathBuf,
}

impl UserDirs {
    /// The profile folder and the temp folder of this process, without a `\\?\` prefix.
    pub fn from_env() -> UserDirs {
        let plain = |p: PathBuf| PathBuf::from(strip_verbatim(&p.to_string_lossy()).into_owned());
        UserDirs {
            home: plain(std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default()),
            temp: plain(std::env::temp_dir()),
        }
    }

    pub fn path_env(&self) -> PathEnv {
        PathEnv { home: self.home.to_string_lossy().into_owned(), temp: self.temp.to_string_lossy().into_owned() }
    }
}

/// Reads the tool calls of one session; the real reader asks Mukade, tests pass fakes.
pub(crate) trait CallSource {
    fn calls(&self, session: &Descriptor) -> Result<Vec<ToolCall>, ReadFailure>;
}

/// Everything the history analysis needs from outside the project tree.
#[derive(Clone, Copy)]
pub(crate) struct HistoryInput<'a> {
    pub listings: &'a [(Agent, Loaded<Descriptor>)],
    pub calls: Option<&'a dyn CallSource>,
    pub user: &'a UserDirs,
}

static NO_USER: LazyLock<UserDirs> = LazyLock::new(UserDirs::default);

impl HistoryInput<'static> {
    /// No listing and no source: the history is not read.
    pub fn none() -> HistoryInput<'static> {
        HistoryInput { listings: &[], calls: None, user: &NO_USER }
    }
}
