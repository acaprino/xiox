//! What the session history says about folders: the shape of the result and the inputs the
//! analysis reads. The logic that fills the shape lives in the modules of the later stages.

mod folders;
mod mukade;
mod queue;
mod report;
pub(crate) mod risk;
mod scratch;
pub(crate) mod scope;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Instant;

use xiox_model::{path_key, Agent, Loaded};
use xiox_mukade::{strip_verbatim, Descriptor, ToolCall};

pub(crate) use folders::Folders;
pub(crate) use mukade::MukadeCalls;
pub(crate) use scratch::scratch_items;
#[cfg(test)]
pub(crate) use scratch::MAX_SCRATCH;
pub(crate) use queue::{Budget, BUDGET};
use queue::Queue;
use scope::{cwd_roots, local_dir, Scope};

use crate::search::Own;
use crate::{session_ledgers, Category, Coverage, Env, Item, Options, PathEnv, Project};

pub use report::{
    Activity, CallOutcome, ConflictReport, Exclusion, HistoryReport, Limit, Operation, OperationSource, Provenance, ReadFailure,
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
    budget: Budget,
}

impl<'a> History<'a> {
    /// `None` when the history is not asked for or there is no listing to read.
    pub fn start(project: &Project, opts: &Options, env: &Env<'a>) -> Option<History<'a>> {
        if !opts.history || env.history.listings.is_empty() {
            return None;
        }
        let sources = env
            .history
            .listings
            .iter()
            .map(|(agent, listing)| SourceReport {
                agent: *agent,
                listing_failed: !listing.warnings.is_empty(),
                listed: listing.items.len(),
                related: 0,
                read: 0,
                failed: BTreeMap::new(),
            })
            .collect();
        let report = HistoryReport::new(sources);
        Some(History { input: env.history, scope: Scope::of(project, opts, env), folders: Folders::default(), report, budget: BUDGET })
    }

    /// The project roots the listed working directories lead to, to be verified with the folders
    /// the search finds. Records each folder a listing names.
    pub fn listed_roots(&mut self, project: &Project) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for (_, listing) in self.input.listings {
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

    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Reads the calls of the project's sessions, newest first, within the budget, and folds
    /// what they show into the folders. A copy a session made is verified and, once it is an
    /// item, brings the sessions that worked in it into the queue.
    pub fn read(&mut self, project: &Project, items: &mut Vec<Item>, verify: &mut dyn FnMut(&[PathBuf]) -> Vec<Item>) {
        // No source, nothing to read: the report keeps what the listings said.
        let Some(source) = self.input.calls else { return };
        let roots: Vec<PathBuf> = std::iter::once(project.root.clone())
            .chain(project.worktrees.iter().cloned())
            .chain(items.iter().filter(|i| i.category == Category::Copies).filter_map(|i| i.path.clone()))
            .collect();
        let mut queue = Queue::new(self.input.listings, &roots);
        let mut checked: HashSet<String> = items.iter().map(|i| i.key.clone()).collect();
        let started = Instant::now();
        let mut attempts = 0;
        while queue.left() > 0 {
            if let Some(limit) = self.limit_reached(attempts, started, queue.left()) {
                self.report.limits.push(limit);
                break;
            }
            let Some(session) = queue.next() else { break };
            if queue.is_ambiguous(session) {
                self.count_failure(session, ReadFailure::AmbiguousId);
                continue;
            }
            attempts += 1;
            if self.read_session(source, session) {
                for item in self.new_copies(project, &mut checked, items, verify) {
                    if let (Category::Copies, Some(path)) = (item.category, &item.path) {
                        queue.add_root(path);
                    }
                    items.push(item);
                }
            }
        }
        for session in queue.related() {
            let row = self.row(session.agent);
            self.report.sources[row].related += 1;
        }
    }

    /// The first cap the reading has reached, with the related sessions it leaves.
    fn limit_reached(&self, attempts: usize, started: Instant, left: usize) -> Option<Limit> {
        let b = &self.budget;
        if attempts >= b.max_sessions {
            Some(Limit::Sessions { max: b.max_sessions, left })
        } else if self.report.calls >= b.max_calls {
            Some(Limit::Calls { max: b.max_calls })
        } else if started.elapsed() >= b.max_duration {
            Some(Limit::Duration { secs: b.max_duration.as_secs(), left })
        } else {
            None
        }
    }

    /// Reads one session and folds its calls into the folders; `false` when it could not be
    /// read. The calls are dropped before the next session is asked for.
    fn read_session(&mut self, source: &dyn CallSource, session: &Descriptor) -> bool {
        let row = self.row(session.agent);
        let read = if session.info.unreadable.is_some() { Err(ReadFailure::Failed) } else { source.calls(session) };
        let calls = match read {
            Ok(calls) => calls,
            Err(why) => {
                self.count_failure(session, why);
                return false;
            }
        };
        self.report.sources[row].read += 1;
        self.report.sessions_read += 1;
        self.report.calls += calls.len();
        for ledger in session_ledgers(&calls, &self.input.user.path_env()).values() {
            self.folders.add_ledger(session, ledger);
            add_coverage(&mut self.report.shell, ledger.coverage());
        }
        true
    }

    fn count_failure(&mut self, session: &Descriptor, why: ReadFailure) {
        let row = self.row(session.agent);
        *self.report.sources[row].failed.entry(why).or_default() += 1;
    }

    /// The copies among the folders the last session named, outside the project and not looked
    /// at before: the further context changes the sessions show.
    fn new_copies(
        &mut self,
        project: &Project,
        checked: &mut HashSet<String>,
        items: &[Item],
        verify: &mut dyn FnMut(&[PathBuf]) -> Vec<Item>,
    ) -> Vec<Item> {
        let own = Own::new(project);
        let mut dirs = Vec::new();
        for folder in self.folders.take_fresh().into_iter().filter(|f| !own.holds(&path_key(f))) {
            match local_dir(&folder.to_string_lossy()) {
                Ok(dir) => dirs.push(dir),
                Err(why) => *self.report.excluded.entry(why).or_default() += 1,
            }
        }
        let home = Some(self.input.user.home.as_path()).filter(|h| !h.as_os_str().is_empty());
        let roots = cwd_roots(project, &dirs, &self.scope, home, &mut self.report.excluded);
        let roots: Vec<PathBuf> = roots.into_iter().filter(|r| checked.insert(path_key(r))).collect();
        verify(&roots).into_iter().filter(|i| !items.iter().any(|known| known.key == i.key)).collect()
    }

    fn row(&self, agent: Agent) -> usize {
        self.report.sources.iter().position(|s| s.agent == agent).expect("every listed agent has a row")
    }

    /// The folders the sessions created or wrote in, as review candidates. `items` are what the
    /// analysis lists otherwise.
    pub fn scratch(&mut self, project: &Project, items: &[Item], env: &Env, opts: &Options) -> Vec<Item> {
        scratch_items(project, &self.folders, items, env, opts, &mut self.report)
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

/// Adds the shell coverage of one session to the total.
fn add_coverage(total: &mut Coverage, c: &Coverage) {
    total.shell_calls += c.shell_calls;
    total.unresolved += c.unresolved;
    total.looped += c.looped;
    total.script += c.script;
    total.truncated += c.truncated;
    total.exhausted += c.exhausted;
}
