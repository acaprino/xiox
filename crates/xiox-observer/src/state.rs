//! The observer's memory: the baseline taken at the start, the items seen by the last check,
//! what changed since, and which thresholds are currently crossed. Pure: it never touches the
//! disk, the clock or git; the caller feeds it scans and the current time.

use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use xiox_project::{
    Category, Item, Note, Project, ProjectScan, SCHEMA, SearchMode, StaleBranch, is_recommended, key_within,
};

/// How many `--include` paths one `xiox clean` command carries before it is cut.
const CLEAN_COMMAND_MAX_ITEMS: usize = 25;

/// A folder whose last measure hit the caps is measured again at most this often (10 minutes):
/// walking a huge tree at every check, while agents compile, is the cost the caps exist to avoid.
pub const PARTIAL_BACK_OFF_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    /// Bytes of growth since the baseline that raise [`AttentionKind::Growth`].
    pub grow_threshold: u64,
    /// New worktrees since the baseline that raise [`AttentionKind::Worktrees`].
    pub worktree_threshold: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self { grow_threshold: 2 * 1024 * 1024 * 1024, worktree_threshold: 3 }
    }
}

/// An [`Item`] plus what only the observer knows.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    #[serde(flatten)]
    pub item: Item,
    /// Absent from the baseline (the first scan of the process).
    pub new: bool,
    /// When the observer first saw it; a baseline item: when the process started.
    pub first_seen_ms: i64,
    /// When `item.bytes` was last measured; `None` while it never was.
    pub measured_at_ms: Option<i64>,
    /// `item.bytes` is a LOWER BOUND: the last measure hit the entry or time cap before the end of
    /// the folder, so the real size is at least that. Such a folder is measured again only after
    /// [`PARTIAL_BACK_OFF_MS`].
    pub bytes_partial: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttentionKind {
    Growth,
    Worktrees,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Measured bytes per category name (`builds`, `worktrees`, ...); a category with nothing measured is absent.
    pub by_category: BTreeMap<String, u64>,
    pub items: usize,
    /// Measured bytes of the items `xiox clean` would offer as recommended.
    pub recommended_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// [`xiox_project::SCHEMA`].
    pub schema: u32,
    pub project: Project,
    pub items: Vec<ItemView>,
    pub notes: Vec<Note>,
    /// Stale local branches, from the latest scan (same field as `xiox analyze --json`).
    pub branches: Vec<StaleBranch>,
    pub search: SearchMode,
    /// When the latest scan was taken, and how long it took.
    pub scanned_at_ms: i64,
    pub elapsed_ms: u64,
    pub totals: Totals,
    /// Since the baseline, measured bytes only.
    pub growth_bytes: i64,
    /// Thresholds currently exceeded.
    pub attention: Vec<AttentionKind>,
    pub stale: bool,
    /// `xiox clean "<root>" --include "<p1>" ...` for the new recommended items `clean` accepts.
    pub clean_command: Option<String>,
    /// More such items than one command carries ([`CLEAN_COMMAND_MAX_ITEMS`]).
    pub clean_command_truncated: bool,
    pub started_at_ms: i64,
    pub checked_at_ms: i64,
}

/// What the observer tells its caller. The caller adds `schema` and `time` when it prints one.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Event {
    Started { project: Project, params: Params, totals: Totals },
    /// `removed` are item keys.
    Changed { added: Vec<ItemView>, removed: Vec<String>, growth_bytes: i64 },
    Attention { kind: AttentionKind, value: u64, threshold: u64, clean_command: Option<String> },
    Report(Report),
    Error { message: String },
}

pub struct Observer {
    params: Params,
    project: Project,
    notes: Vec<Note>,
    branches: Vec<StaleBranch>,
    search: SearchMode,
    scanned_at_ms: i64,
    elapsed_ms: u64,
    started_at_ms: i64,
    checked_at_ms: i64,
    /// Every key of the first scan, with its bytes at the baseline (`None` until first measured).
    baseline: HashMap<String, Option<u64>>,
    /// The items of the last check, in the order of the scan.
    items: Vec<ItemView>,
    growth_fired: bool,
    worktrees_fired: bool,
}

impl Observer {
    /// Everything in `first` is the baseline: never `new`.
    pub fn new(first: ProjectScan, params: Params, now_ms: i64) -> Self {
        Self::new_measured(first, params, now_ms, &HashSet::new())
    }

    /// Like [`Observer::new`]; `partial` are the keys whose `bytes` are a lower bound (the measure
    /// hit a cap).
    pub fn new_measured(first: ProjectScan, params: Params, now_ms: i64, partial: &HashSet<String>) -> Self {
        let mut baseline = HashMap::new();
        let mut items = Vec::new();
        for item in first.items {
            if baseline.contains_key(&item.key) {
                continue; // a repeated key keeps its first item
            }
            baseline.insert(item.key.clone(), item.bytes);
            items.push(ItemView {
                measured_at_ms: item.bytes.map(|_| now_ms),
                bytes_partial: item.bytes.is_some() && partial.contains(&item.key),
                item,
                new: false,
                first_seen_ms: now_ms,
            });
        }
        Self {
            params,
            project: first.project,
            notes: first.notes,
            branches: first.branches,
            search: first.search,
            scanned_at_ms: first.scanned_at_ms,
            elapsed_ms: first.elapsed_ms,
            started_at_ms: now_ms,
            checked_at_ms: now_ms,
            baseline,
            items,
            growth_fired: false,
            worktrees_fired: false,
        }
    }

    /// The project as the last scan the observer accepted saw it.
    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn started(&self) -> Event {
        Event::Started { project: self.project.clone(), params: self.params, totals: self.totals() }
    }

    /// Feed the result of a re-check. `scan.items[*].bytes` are the measured values (`None` = not
    /// measured: the last measurement of the same item is kept). Returns `Changed` (only if
    /// something was added or removed) and then any `Attention` events.
    pub fn update(&mut self, scan: ProjectScan, now_ms: i64) -> Vec<Event> {
        self.update_measured(scan, now_ms, &HashSet::new())
    }

    /// Like [`Observer::update`]; `partial` are the keys measured in this scan whose `bytes` are a
    /// lower bound (the measure hit a cap). An item not measured keeps its last bytes and their flag.
    pub fn update_measured(&mut self, scan: ProjectScan, now_ms: i64, partial: &HashSet<String>) -> Vec<Event> {
        let previous = std::mem::take(&mut self.items);
        let previous_order: Vec<String> = previous.iter().map(|v| v.item.key.clone()).collect();
        let mut previous: HashMap<String, ItemView> =
            previous.into_iter().map(|v| (v.item.key.clone(), v)).collect();

        let mut seen: HashSet<String> = HashSet::new();
        let mut added: Vec<ItemView> = Vec::new();
        let mut items: Vec<ItemView> = Vec::with_capacity(scan.items.len());
        for mut item in scan.items {
            if !seen.insert(item.key.clone()) {
                continue; // a repeated key keeps its first item
            }
            let before = previous.remove(&item.key);
            // The same folder listed in another category than at the last check (a worktree seen
            // while git was still creating it comes first as a copy): told again, in its new one.
            let recategorized = before.as_ref().is_some_and(|b| b.item.category != item.category);
            let in_baseline = self.baseline.contains_key(&item.key);
            let mut bytes_partial = false;
            let measured_at_ms = match (item.bytes, &before) {
                (Some(_), _) => {
                    bytes_partial = partial.contains(&item.key);
                    Some(now_ms)
                }
                (None, Some(b)) if b.item.bytes.is_some() => {
                    item.bytes = b.item.bytes;
                    bytes_partial = b.bytes_partial;
                    b.measured_at_ms
                }
                _ => None,
            };
            if in_baseline && item.bytes.is_some() {
                // A baseline folder that was not measured at the start is compared with its first measure.
                if let Some(slot) = self.baseline.get_mut(&item.key) {
                    slot.get_or_insert(item.bytes.unwrap_or(0));
                }
            }
            let first_seen_ms = match (&before, in_baseline) {
                (_, true) => self.started_at_ms,
                (Some(b), false) => b.first_seen_ms,
                (None, false) => now_ms,
            };
            let view = ItemView { item, new: !in_baseline, first_seen_ms, measured_at_ms, bytes_partial };
            if before.is_none() || recategorized {
                added.push(view.clone());
            }
            items.push(view);
        }
        let removed: Vec<String> = previous_order.into_iter().filter(|k| previous.contains_key(k)).collect();

        self.project = scan.project;
        self.notes = scan.notes;
        self.branches = scan.branches;
        self.search = scan.search;
        self.scanned_at_ms = scan.scanned_at_ms;
        self.elapsed_ms = scan.elapsed_ms;
        self.items = items;
        self.checked_at_ms = now_ms;

        let mut events = Vec::new();
        if !added.is_empty() || !removed.is_empty() {
            events.push(Event::Changed { added, removed, growth_bytes: self.growth_bytes() });
        }

        let growth = self.growth();
        let threshold = self.params.grow_threshold;
        if growth > threshold {
            if !self.growth_fired {
                self.growth_fired = true;
                events.push(Event::Attention {
                    kind: AttentionKind::Growth,
                    value: growth,
                    threshold,
                    clean_command: self.clean_command().0,
                });
            }
        } else {
            self.growth_fired = false;
        }

        let worktrees = self.new_worktrees();
        if self.worktrees_exceeded() {
            if !self.worktrees_fired {
                self.worktrees_fired = true;
                events.push(Event::Attention {
                    kind: AttentionKind::Worktrees,
                    value: worktrees as u64,
                    threshold: self.params.worktree_threshold as u64,
                    clean_command: self.clean_command().0,
                });
            }
        } else {
            self.worktrees_fired = false;
        }
        events
    }

    /// Keys of items whose bytes should be measured at the next check, oldest measure first
    /// (never measured before all): every new item (it may still be growing: a worktree being
    /// checked out, a build in progress) and the `builds` folders of the baseline (their growth is
    /// what is watched). Only items with a folder to measure; the caller bounds each measure and
    /// the whole check. An item whose last measure hit the caps is left out until
    /// [`PARTIAL_BACK_OFF_MS`] have passed since it.
    pub fn keys_to_measure(&self, now_ms: i64) -> Vec<String> {
        let mut due: Vec<&ItemView> = self
            .items
            .iter()
            .filter(|v| v.item.path.is_some())
            .filter(|v| v.new || v.item.category == Category::Builds)
            .filter(|v| {
                !(v.bytes_partial && v.measured_at_ms.is_some_and(|at| now_ms.saturating_sub(at) < PARTIAL_BACK_OFF_MS))
            })
            .collect();
        due.sort_by_key(|v| v.measured_at_ms); // stable: `None` first, the scan order among equals
        due.into_iter().map(|v| v.item.key.clone()).collect()
    }

    pub fn report(&self, stale: bool) -> Report {
        let (clean_command, clean_command_truncated) = self.clean_command();
        let mut attention = Vec::new();
        if self.growth() > self.params.grow_threshold {
            attention.push(AttentionKind::Growth);
        }
        if self.worktrees_exceeded() {
            attention.push(AttentionKind::Worktrees);
        }
        Report {
            schema: SCHEMA,
            project: self.project.clone(),
            items: self.items.clone(),
            notes: self.notes.clone(),
            branches: self.branches.clone(),
            search: self.search,
            scanned_at_ms: self.scanned_at_ms,
            elapsed_ms: self.elapsed_ms,
            totals: self.totals(),
            growth_bytes: self.growth_bytes(),
            attention,
            stale,
            clean_command,
            clean_command_truncated,
            started_at_ms: self.started_at_ms,
            checked_at_ms: self.checked_at_ms,
        }
    }

    /// What one item adds to the growth: a new item all its measured bytes; a baseline item what
    /// it grew by since the baseline (shrinking counts 0). An item not measured (yet) counts 0.
    fn growth_of(&self, v: &ItemView) -> u64 {
        match (self.baseline.get(&v.item.key), v.item.bytes) {
            (None, Some(now)) => now,
            (Some(Some(base)), Some(now)) => now.saturating_sub(*base),
            _ => 0,
        }
    }

    /// Growth since the baseline, measured bytes only. An item inside another item that already
    /// contributes (same kind: both new or both baseline) is counted with it, once.
    fn growth(&self) -> u64 {
        self.items
            .iter()
            .filter(|v| {
                !(v.item.path.is_some()
                    && self.items.iter().any(|o| {
                        o.item.path.is_some()
                            && o.new == v.new
                            && o.item.key != v.item.key
                            && key_within(&v.item.key, &o.item.key)
                            && self.growth_of(o) > 0
                    }))
            })
            .map(|v| self.growth_of(v))
            .fold(0u64, u64::saturating_add)
    }

    fn growth_bytes(&self) -> i64 {
        i64::try_from(self.growth()).unwrap_or(i64::MAX)
    }

    fn new_worktrees(&self) -> usize {
        self.items.iter().filter(|v| v.new && v.item.category == Category::Worktrees).count()
    }

    fn worktrees_exceeded(&self) -> bool {
        self.new_worktrees() > self.params.worktree_threshold
    }

    /// Per category the measured bytes, an item inside another of its category counted once;
    /// `recommended_bytes` likewise over the recommended items (a worktree and the build inside
    /// it are freed together).
    fn totals(&self) -> Totals {
        let measured: Vec<&Item> = self.items.iter().map(|v| &v.item).filter(|i| i.bytes.is_some()).collect();
        let mut totals = Totals { items: self.items.len(), ..Totals::default() };
        for category in measured.iter().map(|i| i.category).collect::<HashSet<_>>() {
            let of: Vec<&Item> = measured.iter().copied().filter(|i| i.category == category).collect();
            totals.by_category.insert(category.to_string(), sum_outer(&of));
        }
        let recommended: Vec<&Item> = measured.iter().copied().filter(|i| is_recommended(i)).collect();
        totals.recommended_bytes = sum_outer(&recommended);
        totals
    }

    /// The command that would remove the new recommended items `xiox clean` accepts by path
    /// (not blocked, not directly under a drive root, with a path: sessions cannot be named, and
    /// not holding a blocked item: `clean` refuses the whole command for it), and whether more than
    /// [`CLEAN_COMMAND_MAX_ITEMS`] had to be left out.
    fn clean_command(&self) -> (Option<String>, bool) {
        let holds_blocked = |outer: &Item| {
            self.items.iter().any(|j| {
                j.item.path.is_some()
                    && j.item.key != outer.key
                    && !j.item.blocked.is_empty()
                    && key_within(&j.item.key, &outer.key)
            })
        };
        let paths: Vec<String> = self
            .items
            .iter()
            .filter(|v| v.new && is_recommended(&v.item) && !holds_blocked(&v.item))
            .filter_map(|v| v.item.path.as_ref())
            .map(|p| p.display().to_string())
            .collect();
        if paths.is_empty() {
            return (None, false);
        }
        let truncated = paths.len() > CLEAN_COMMAND_MAX_ITEMS;
        let mut command = format!("xiox clean {}", quote_arg(&self.project.root.display().to_string()));
        for p in paths.iter().take(CLEAN_COMMAND_MAX_ITEMS) {
            command.push_str(" --include ");
            command.push_str(&quote_arg(p));
        }
        (Some(command), truncated)
    }
}

/// Sum of the measured bytes of `items`, an item inside another one counted once (with the outer one).
fn sum_outer(items: &[&Item]) -> u64 {
    items
        .iter()
        .filter(|i| {
            !(i.path.is_some()
                && items.iter().any(|o| o.path.is_some() && o.key != i.key && key_within(&i.key, &o.key)))
        })
        .fold(0u64, |sum, i| sum.saturating_add(i.bytes.unwrap_or(0)))
}

/// `arg` in double quotes for the Windows command-line parser: a run of backslashes before a
/// closing or embedded quote is doubled (`D:\` is written `"D:\\"`), an embedded quote is escaped.
fn quote_arg(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}
