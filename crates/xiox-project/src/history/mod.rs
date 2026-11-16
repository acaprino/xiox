//! What the session history says about folders: the shape of the result and the inputs the
//! analysis reads. The logic that fills the shape lives in the modules of the later stages.

mod folders;
mod report;
pub(crate) mod scope;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use xiox_model::{Agent, Loaded};
use xiox_mukade::{strip_verbatim, Descriptor, ToolCall};

use folders::Folders;
use scope::{cwd_roots, local_dir, Scope};

use crate::{Category, Coverage, Env, Item, Options, PathEnv, Project};

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

/// One run of the history analysis over a project: what the listings name, and the report of
/// what was read and left out.
pub(crate) struct History<'a> {
    input: HistoryInput<'a>,
    scope: Scope,
    folders: Folders,
    report: HistoryReport,
}

impl<'a> History<'a> {
    /// `None` when the history is not asked for or there is no listing to read.
    pub fn start(project: &Project, opts: &Options, env: &Env<'a>) -> Option<History<'a>> {
        if !opts.history || env.history.listings.is_empty() {
            return None;
        }
        let report = HistoryReport {
            sources: Vec::new(),
            sessions_read: 0,
            calls: 0,
            shell: Coverage::default(),
            excluded: BTreeMap::new(),
            limits: Vec::new(),
        };
        Some(History { input: env.history, scope: Scope::of(project, opts, env), folders: Folders::default(), report })
    }

    /// The project roots the listed working directories lead to, to be verified with the folders
    /// the search finds. Records each listing in `sources` and each folder it names.
    pub fn listed_roots(&mut self, project: &Project, _opts: &Options, _env: &Env) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for (agent, listing) in self.input.listings {
            self.report.sources.push(SourceReport {
                agent: *agent,
                listing_failed: !listing.warnings.is_empty(),
                listed: listing.items.len(),
                // shortcut: set by the stage that reads the related sessions' calls.
                related: 0,
                read: 0,
                failed: BTreeMap::new(),
            });
            for session in &listing.items {
                let Some(cwd) = session.info.cwd.as_deref() else { continue };
                match local_dir(cwd) {
                    Ok(dir) => {
                        self.folders.add_listed(&dir, session);
                        dirs.push(dir);
                    }
                    Err(why) => *self.report.excluded.entry(why).or_default() += 1,
                }
            }
        }
        let home = Some(self.input.user.home.as_path()).filter(|h| !h.as_os_str().is_empty());
        cwd_roots(project, &dirs, &self.scope, home, &mut self.report.excluded)
    }

    /// The provenance of the items the sessions can speak about.
    pub fn attach(&self, items: &mut [Item]) {
        for item in items.iter_mut().filter(|i| matches!(i.category, Category::Copies | Category::Named | Category::Scratch)) {
            item.provenance = self.folders.provenance_within(&item.key);
        }
    }

    pub fn into_report(self) -> HistoryReport {
        self.report
    }
}
