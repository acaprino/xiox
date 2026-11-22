//! The Registry section without a window: its state, the two jobs (scan, fix) behind a worker
//! seam, the intents the page can send, and the `SectionView` the page draws.
//!
//! The intents refuse what the page must not do now: registry choices and the scan while a job
//! runs or while the backup question is open (the question describes the selection), a fix without
//! the question, a question without a selected issue. A refused intent changes nothing.

use crate::dev::{BUSY, DIALOG_OPEN, NOTHING_SELECTED, NO_DIALOG};
use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::registry;
use crate::sys::{fmt_count, plural};
use crate::view::*;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// What the registry workers tell the state.
pub enum RegMsg {
    Progress(String, f32),
    Found(Vec<registry::Issue>),
    Done,
    /// `refused`: backup chiesto e non riuscito (il motivo); allora nessuna riparazione.
    Fixed { ok: usize, failed: usize, backup: Option<String>, refused: Option<String> },
}

/// The two jobs of the state as plain function pointers: the real ones by default, fake ones in
/// tests (so `scan` and `fix` reach the start without ever touching the registry).
#[derive(Clone, Copy)]
pub struct RegistryWorkers {
    /// The categories to look through (indices into `registry::CATEGORIES`).
    pub scan: fn(Vec<usize>, Outbox<RegMsg>),
    /// The issues to fix, and whether to save a `.reg` backup first.
    pub fix: fn(Vec<registry::Issue>, bool, Outbox<RegMsg>),
}

impl RegistryWorkers {
    pub const REAL: RegistryWorkers = RegistryWorkers { scan: real_scan, fix: real_fix };
}

// Why an intent is refused (English: the page shows them as they are). The refusals every section
// shares (`BUSY`, `DIALOG_OPEN`, `NO_DIALOG`, `NOTHING_SELECTED`) live in `dev`.
pub const NO_CATEGORY: &str = "Tick at least one category to look through.";
pub const NO_ISSUES: &str = "There are no issues to choose from: find issues first.";
pub const UNKNOWN_ISSUE: &str = "That issue is not in the list.";
pub const UNKNOWN_CATEGORY: &str = "That category does not exist.";
pub const FIX_DIED: &str = "The worker stopped: some issues may already have been fixed. Scan again to see what is left.";

/// The id of the sub-heading of a kind of issue: `t:<index into registry::CATEGORIES>`. Its box ticks the kind's issues.
const KIND_ROW: &str = "t:";

/// The issue of a row id `i:<index>`.
fn issue_index(row: &str) -> Option<usize> {
    row.strip_prefix("i:")?.parse().ok()
}

pub struct RegistryState {
    /// One tick per `registry::CATEGORIES` entry.
    sel: Vec<bool>,
    issues: Vec<registry::Issue>,
    job: Option<Receiver<RegMsg>>,
    text: String,
    progress: f32,
    scanned: bool,
    /// The backup question is open.
    pub(crate) confirm: bool,
    msg: String,
    /// The running job is a fix (not a scan).
    fixing: bool,
    /// How `msg` is shown: a result (ok), a partial one (warn) or a failure (a refused backup, a
    /// dead worker: error).
    msg_tone: Tone,
    workers: RegistryWorkers,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryState {
    pub fn new() -> Self {
        RegistryState {
            sel: registry::CATEGORIES.iter().map(|c| c.default_on).collect(),
            issues: vec![],
            job: None,
            text: String::new(),
            progress: 0.0,
            scanned: false,
            confirm: false,
            msg: String::new(),
            fixing: false,
            msg_tone: Tone::Ok,
            workers: RegistryWorkers::REAL,
        }
    }

    /// The same state with other workers (tests; `RegistryWorkers::REAL` is the default).
    pub fn with_workers(mut self, w: RegistryWorkers) -> Self {
        self.workers = w;
        self
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn selected(&self) -> usize {
        self.issues.iter().filter(|i| i.selected).count()
    }

    fn any_category(&self) -> bool {
        self.sel.iter().any(|&b| b)
    }

    // ---------- the jobs ----------

    pub fn start_scan(&mut self, n: SharedNotifier) {
        let cats: Vec<usize> = (0..self.sel.len()).filter(|&i| self.sel[i]).collect();
        let (tx, rx) = channel();
        self.job = Some(rx);
        self.fixing = false;
        self.msg_tone = Tone::Ok;
        self.text.clear();
        self.progress = 0.0;
        self.issues.clear();
        self.msg.clear();
        self.scanned = true;
        let work = self.workers.scan;
        std::thread::spawn(move || work(cats, Outbox::new(tx, n, SectionId::Registry)));
    }

    pub fn start_fix(&mut self, with_backup: bool, n: SharedNotifier) {
        let chosen: Vec<registry::Issue> = self.issues.iter().filter(|i| i.selected).cloned().collect();
        let (tx, rx) = channel();
        self.job = Some(rx);
        self.fixing = true;
        self.msg_tone = Tone::Ok;
        self.text.clear();
        self.progress = 0.0;
        let work = self.workers.fix;
        std::thread::spawn(move || work(chosen, with_backup, Outbox::new(tx, n, SectionId::Registry)));
    }

    /// Drains what the worker has said so far. A worker that ends without a result (it died)
    /// ends the job with a message, never a spinner that does not stop.
    pub fn poll(&mut self) {
        let Some(rx) = &self.job else { return };
        let mut done = false;
        let mut gone = false;
        loop {
            let m = match rx.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    gone = true;
                    break;
                }
            };
            match m {
                RegMsg::Progress(t, p) => {
                    self.text = t;
                    self.progress = p;
                }
                RegMsg::Found(v) => self.issues.extend(v),
                RegMsg::Done => done = true,
                RegMsg::Fixed { refused: Some(why), .. } => {
                    self.msg = format!("Registry backup failed ({why}): no repair was done.");
                    self.msg_tone = Tone::Error;
                    done = true;
                }
                RegMsg::Fixed { ok, failed, backup, refused: None } => {
                    self.msg = format!("Fixed {}.", plural(ok, "issue", "issues"));
                    if failed > 0 {
                        self.msg.push_str(&format!(" {} not fixed (administrator rights are needed).", fmt_count(failed as u64)));
                    }
                    if let Some(b) = backup {
                        self.msg.push_str(&format!("\nBackup saved in: {b}"));
                    }
                    if failed == 0 {
                        self.issues.retain(|i| !i.selected);
                    }
                    self.msg_tone = if failed > 0 { Tone::Warn } else { Tone::Ok };
                    done = true;
                }
            }
        }
        if gone && !done {
            // A fix that died may have repaired some issues: the list on screen can be stale.
            self.msg = if self.fixing { FIX_DIED } else { JOB_DIED }.into();
            self.msg_tone = Tone::Error;
            done = true;
        }
        if done {
            self.job = None;
            self.fixing = false;
        }
    }

    // ---------- intents (page -> Rust) ----------

    /// Choices and the scan: no job, and no question whose text describes the selection.
    fn editable(&self) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        if self.confirm {
            return Err(DIALOG_OPEN.into());
        }
        Ok(())
    }

    pub fn toggle_category(&mut self, i: usize) -> Result<(), String> {
        self.editable()?;
        match self.sel.get_mut(i) {
            Some(b) => {
                *b = !*b;
                Ok(())
            }
            None => Err(UNKNOWN_CATEGORY.into()),
        }
    }

    /// `row` is `i:<index>` (an issue) or `t:<category>` (the box of a kind of issue), as in the view.
    pub fn toggle(&mut self, row: &str) -> Result<(), String> {
        self.editable()?;
        if row.starts_with(KIND_ROW) {
            let (ids, on) = header_toggle(&self.rows(), row).ok_or_else(|| UNKNOWN_CATEGORY.to_string())?;
            for i in ids.iter().filter_map(|id| issue_index(id)) {
                self.issues[i].selected = on;
            }
            return Ok(());
        }
        let i = issue_index(row).filter(|&i| i < self.issues.len()).ok_or_else(|| UNKNOWN_ISSUE.to_string())?;
        self.issues[i].selected = !self.issues[i].selected;
        Ok(())
    }

    pub fn select_all(&mut self, on: bool) -> Result<(), String> {
        self.editable()?;
        if self.issues.is_empty() {
            return Err(NO_ISSUES.into());
        }
        self.issues.iter_mut().for_each(|i| i.selected = on);
        Ok(())
    }

    pub fn scan(&mut self, n: SharedNotifier) -> Result<(), String> {
        self.editable()?;
        if !self.any_category() {
            return Err(NO_CATEGORY.into());
        }
        self.start_scan(n);
        Ok(())
    }

    /// Opens the backup question: it needs an issue selected.
    pub fn open_fix(&mut self) -> Result<(), String> {
        self.editable()?;
        if self.selected() == 0 {
            return Err(NOTHING_SELECTED.into());
        }
        self.confirm = true;
        Ok(())
    }

    /// The answer to the backup question (`backup`: save a `.reg` copy first). The question closes
    /// and the fix starts; without the question open there is nothing to answer.
    pub fn fix(&mut self, backup: bool, n: SharedNotifier) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        if !self.confirm {
            return Err(NO_DIALOG.into());
        }
        if self.selected() == 0 {
            return Err(NOTHING_SELECTED.into());
        }
        self.confirm = false;
        self.start_fix(backup, n);
        Ok(())
    }

    pub fn close_confirm(&mut self) {
        self.confirm = false;
    }

    // ---------- the view ----------

    pub fn view(&self) -> SectionView {
        let busy = self.busy();
        let selected = self.selected();
        let rows = self.rows();
        SectionView {
            section: SectionId::Registry,
            rev: 0,
            job: self.job_view(),
            status: self.status_view(selected),
            nav: Vec::new(),
            options: self.options_view(),
            title: "Registry problems".into(),
            subtitle: self.subtitle(),
            columns: vec![
                ColumnView { title: "Issue".into(), kind: ColumnKind::Name },
                ColumnView { title: "Registry key".into(), kind: ColumnKind::Cell },
            ],
            bulk: !self.issues.is_empty() && !busy,
            empty: self.empty_view(rows.is_empty()),
            rows,
            notices: self.notices_view(),
            actions: vec![
                ActionView {
                    id: "scan".into(),
                    label: "Find issues".into(),
                    key: Some("a".into()),
                    enabled: !busy && !self.confirm && self.any_category(),
                    primary: false,
                },
                ActionView {
                    id: "fix".into(),
                    label: format!("Fix selected ({})…", fmt_count(selected as u64)),
                    key: Some("Enter".into()),
                    enabled: !busy && !self.confirm && selected > 0,
                    primary: true,
                },
            ],
            search: None,
            dialog: self.dialog_view(),
            report: None,
            footer: None,
        }
    }

    fn job_view(&self) -> Option<JobView> {
        if !self.busy() {
            return None;
        }
        let label = if !self.text.is_empty() {
            self.text.clone()
        } else if self.fixing {
            "Fixing…".into()
        } else {
            "Scanning the registry…".into()
        };
        Some(JobView { label, fraction: self.progress, detail: None })
    }

    fn status_view(&self, selected: usize) -> StatusView {
        let (badge, text) = if self.busy() {
            (if self.fixing { "Fixing" } else { "Scanning" }.to_string(), String::new())
        } else if !self.scanned {
            ("Ready".to_string(), "No scan yet".to_string())
        } else {
            ("Scan done".to_string(), format!("{} · {} selected", issue_count(self.issues.len()), fmt_count(selected as u64)))
        };
        StatusView { badge, text }
    }

    fn subtitle(&self) -> String {
        if !self.scanned {
            "No scan yet".into()
        } else {
            issue_count(self.issues.len())
        }
    }

    fn options_view(&self) -> Vec<OptionView> {
        let enabled = !self.busy();
        let mut v: Vec<OptionView> = registry::CATEGORIES
            .iter()
            .enumerate()
            .map(|(i, c)| OptionView::Toggle {
                id: format!("cat:{i}"),
                label: c.name.into(),
                on: self.sel.get(i).copied().unwrap_or(false),
                enabled,
                hint: None,
            })
            .collect();
        v.push(OptionView::Text {
            text: "Before fixing you can save a backup: double-click the .reg file to go back.".into(),
            tone: Tone::Info,
            hint: None,
        });
        v
    }

    /// The issues found, under a sub-heading per kind of issue (in the order of `registry::CATEGORIES`, with how many
    /// it holds) whose box ticks the kind's issues.
    fn rows(&self) -> Vec<RowView> {
        let mut rows = Vec::new();
        for (c, kind) in registry::CATEGORIES.iter().enumerate() {
            let found: Vec<(usize, &registry::Issue)> = self.issues.iter().enumerate().filter(|(_, it)| it.category == c).collect();
            if found.is_empty() {
                continue;
            }
            rows.push(RowView { name: format!("{} ({})", kind.name, fmt_count(found.len() as u64)), ..blank_row(format!("{KIND_ROW}{c}"), RowKind::Header) });
            rows.extend(found.into_iter().map(|(i, it)| RowView {
                check: if it.selected { Check::On } else { Check::Off },
                name: it.problem.clone(),
                sub: Some(it.data.clone()),
                cells: vec![it.fix.key_text()],
                detail: vec![
                    ("item".into(), it.problem.clone()),
                    ("key".into(), it.fix.key_text()),
                    ("data".into(), it.data.clone()),
                ],
                ..blank_row(format!("i:{i}"), RowKind::Item)
            }));
        }
        box_headers(&mut rows, self.busy());
        rows
    }

    fn empty_view(&self, no_rows: bool) -> Option<EmptyView> {
        if !no_rows {
            return None;
        }
        Some(if self.busy() {
            EmptyView { title: "Working…".into(), text: "This takes a moment.".into() }
        } else if !self.scanned {
            EmptyView {
                title: "Registry cleanup".into(),
                text: "Press “Find issues” to find entries that point to files or programs that no longer exist.".into(),
            }
        } else {
            EmptyView { title: "No issues found".into(), text: "Nothing in the chosen categories needs fixing.".into() }
        })
    }

    /// `msg` is the result of the last job; its first line is the notice, the rest hides behind it.
    fn notices_view(&self) -> Vec<NoticeView> {
        if self.msg.is_empty() {
            return Vec::new();
        }
        let mut lines = self.msg.lines().map(str::to_string);
        let text = lines.next().unwrap_or_default();
        vec![NoticeView { tone: self.msg_tone, text, more: lines.collect() }]
    }

    fn dialog_view(&self) -> Option<DialogView> {
        if !self.confirm {
            return None;
        }
        let button = |id: &str, label: &str, primary: bool| ButtonView { id: id.into(), label: label.into(), enabled: true, primary };
        // The buttons say what they do and to how many issues, not "Yes".
        let issues = plural(self.selected(), "issue", "issues");
        Some(DialogView {
            id: "backup".into(),
            title: "Save a registry backup?".into(),
            text: vec!["Recommended: before the changes a copy (.reg) is saved in the Documents\\Xiox folder.".into()],
            summary: Vec::new(),
            permanent: false,
            rows: Vec::new(),
            warnings_title: None,
            warnings: Vec::new(),
            caution_title: None,
            cautions: Vec::new(),
            ack: None,
            error: None,
            buttons: vec![
                button("backup", &format!("Back up and fix {issues}"), true),
                button("nobackup", &format!("Fix {issues} without a backup"), false),
                button("cancel", "Cancel", false),
            ],
        })
    }
}

fn issue_count(n: usize) -> String {
    format!("{} found", plural(n, "issue", "issues"))
}

// ---------- the real jobs ----------

/// Il thread dell'analisi (il lavoro vero di `RegistryWorkers::REAL`): una categoria alla volta.
pub fn real_scan(cats: Vec<usize>, out: Outbox<RegMsg>) {
    let total = cats.len().max(1) as f32;
    for (k, &i) in cats.iter().enumerate() {
        let name = registry::CATEGORIES[i].name;
        out.send(RegMsg::Progress(format!("Checking: {name}"), k as f32 / total));
        let found = registry::scan(i, &|_| {});
        out.send(RegMsg::Found(found));
    }
    out.send(RegMsg::Done);
}

/// Il thread della riparazione (il lavoro vero di `RegistryWorkers::REAL`). Backup chiesto e non
/// riuscito: non si ripara nulla.
pub fn real_fix(chosen: Vec<registry::Issue>, with_backup: bool, out: Outbox<RegMsg>) {
    if with_backup {
        out.send(RegMsg::Progress("Saving the backup…".into(), 0.0));
    }
    let total = chosen.len().max(1) as f32;
    let res = registry::fix_all(&chosen, with_backup, &registry::backup, &mut registry::apply, &mut |k| {
        if k % 20 == 0 {
            out.send(RegMsg::Progress("Fixing…".into(), k as f32 / total));
        }
    });
    let backup = res.backup.map(|p| p.to_string_lossy().to_string());
    out.send(RegMsg::Fixed { ok: res.ok, failed: res.failed, backup, refused: res.refused });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Fix, Issue, Root};
    use crate::view::testing::{action, button, err, notifier, row, wait};
    use std::sync::atomic::{AtomicU8, Ordering::SeqCst};
    use std::sync::Mutex;

    fn issue(n: usize, selected: bool) -> Issue {
        Issue {
            category: 0,
            problem: format!("Missing shared DLL {n}"),
            data: format!("C:\\gone\\{n}.dll"),
            fix: Fix::DeleteValue { root: Root::Hklm, path: "SOFTWARE\\Acme\\SharedDLLs".into(), value: format!("v{n}") },
            selected,
        }
    }

    // Workers that must never run: the test reaches the intent's refusal first.
    fn never_scan(_: Vec<usize>, _: Outbox<RegMsg>) {
        panic!("the scan worker must not run");
    }
    fn never_fix(_: Vec<registry::Issue>, _: bool, _: Outbox<RegMsg>) {
        panic!("the fix worker must not run");
    }
    const NEVER: RegistryWorkers = RegistryWorkers { scan: never_scan, fix: never_fix };

    /// A scan that has been done: three issues, two of them ticked. No job ever ran.
    fn scanned() -> RegistryState {
        let mut st = RegistryState::new().with_workers(NEVER);
        st.issues = vec![issue(0, true), issue(1, true), issue(2, false)];
        st.scanned = true;
        st
    }

    fn drain(st: &mut RegistryState) {
        wait(|| {
            st.poll();
            !st.busy()
        });
    }

    // ---- registry_toggles_are_refused_while_busy_or_with_the_dialog_open ----

    static HOLD: AtomicU8 = AtomicU8::new(0);
    fn held_scan(_: Vec<usize>, out: Outbox<RegMsg>) {
        while HOLD.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.send(RegMsg::Found(vec![issue(9, true)]));
        out.send(RegMsg::Done);
    }

    #[test]
    fn registry_toggles_are_refused_while_busy_or_with_the_dialog_open() {
        let mut st = scanned();
        st.open_fix().unwrap();
        let before: Vec<bool> = st.issues.iter().map(|i| i.selected).collect();
        let cats = st.sel.clone();
        for r in [st.toggle("i:0"), st.toggle_category(0), st.select_all(false), st.scan(notifier())] {
            assert_eq!(err(r), DIALOG_OPEN);
        }
        assert_eq!(st.issues.iter().map(|i| i.selected).collect::<Vec<_>>(), before);
        assert_eq!(st.sel, cats);
        assert!(!st.busy(), "a refused scan starts nothing");
        // A running job freezes the same things.
        st.close_confirm();
        let mut st = st.with_workers(RegistryWorkers { scan: held_scan, fix: never_fix });
        st.scan(notifier()).unwrap();
        assert!(st.busy());
        for r in [st.toggle("i:0"), st.toggle_category(0), st.select_all(true), st.scan(notifier()), st.open_fix()] {
            assert_eq!(err(r), BUSY);
        }
        assert_eq!(err(st.fix(true, notifier())), BUSY);
        HOLD.store(1, SeqCst);
        drain(&mut st);
        assert_eq!(st.issues.len(), 1, "the scan found what the worker said");
        // Idle again: the same intents work.
        st.toggle("i:0").unwrap();
        assert!(!st.issues[0].selected);
        st.toggle_category(0).unwrap();
        assert_eq!(err(st.toggle("i:7")), UNKNOWN_ISSUE);
        assert_eq!(err(st.toggle("cat:0")), UNKNOWN_ISSUE);
        assert_eq!(err(st.toggle_category(99)), UNKNOWN_CATEGORY);
        st.select_all(true).unwrap();
        assert!(st.issues.iter().all(|i| i.selected));
    }

    #[test]
    fn scan_needs_a_category() {
        let mut st = RegistryState::new().with_workers(NEVER);
        st.sel.iter_mut().for_each(|b| *b = false);
        assert_eq!(err(st.scan(notifier())), NO_CATEGORY);
        assert!(!st.busy() && !st.scanned, "nothing started, nothing marked as scanned");
        assert!(!action(&st.view(), "scan").enabled);
        st.toggle_category(1).unwrap();
        assert!(action(&st.view(), "scan").enabled);
    }

    #[test]
    fn select_all_needs_issues() {
        let mut st = RegistryState::new().with_workers(NEVER);
        assert_eq!(err(st.select_all(true)), NO_ISSUES);
    }

    // ---- fix_and_apply_need_their_dialog (registry half) ----

    static FIX_BACKUP: AtomicU8 = AtomicU8::new(0); // 0 not run, 1 without backup, 2 with backup
    static FIXED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn recording_fix(chosen: Vec<registry::Issue>, with_backup: bool, out: Outbox<RegMsg>) {
        FIX_BACKUP.store(if with_backup { 2 } else { 1 }, SeqCst);
        FIXED.lock().unwrap().extend(chosen.iter().map(|i| i.problem.clone()));
        out.send(RegMsg::Fixed { ok: chosen.len(), failed: 0, backup: None, refused: None });
    }

    #[test]
    fn fix_and_apply_need_their_dialog() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: recording_fix });
        // No question open: nothing to answer, nothing recorded.
        assert_eq!(err(st.fix(true, notifier())), NO_DIALOG);
        assert_eq!(FIX_BACKUP.load(SeqCst), 0);
        assert!(!st.busy());
        // The question needs a ticked issue.
        st.select_all(false).unwrap();
        assert_eq!(err(st.open_fix()), NOTHING_SELECTED);
        assert!(!st.confirm);
        st.toggle("i:1").unwrap();
        st.open_fix().unwrap();
        assert!(st.confirm);
        assert!(!action(&st.view(), "fix").enabled, "the question is open: the button is off");
        st.fix(true, notifier()).unwrap();
        assert!(!st.confirm, "answering closes the question");
        drain(&mut st);
        assert_eq!(FIX_BACKUP.load(SeqCst), 2, "the fix worker was asked for a backup");
        assert_eq!(*FIXED.lock().unwrap(), vec!["Missing shared DLL 1".to_string()], "only the ticked issue");
        assert_eq!(st.issues.len(), 2, "a fixed issue leaves the list");
        // Answering again needs a new question.
        assert_eq!(err(st.fix(false, notifier())), NO_DIALOG);
        assert_eq!(FIX_BACKUP.load(SeqCst), 2);
    }

    static NOBACKUP: AtomicU8 = AtomicU8::new(0);
    fn nobackup_fix(chosen: Vec<registry::Issue>, with_backup: bool, out: Outbox<RegMsg>) {
        NOBACKUP.store(if with_backup { 2 } else { 1 }, SeqCst);
        out.send(RegMsg::Fixed { ok: chosen.len(), failed: 0, backup: None, refused: None });
    }

    #[test]
    fn fix_without_backup_is_the_other_button() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: nobackup_fix });
        st.open_fix().unwrap();
        st.fix(false, notifier()).unwrap();
        drain(&mut st);
        assert_eq!(NOBACKUP.load(SeqCst), 1);
    }

    fn refusing_fix(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        out.send(RegMsg::Fixed { ok: 0, failed: 0, backup: None, refused: Some("disk full".into()) });
    }

    #[test]
    fn a_refused_backup_fixes_nothing_and_says_why() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: refusing_fix });
        st.open_fix().unwrap();
        st.fix(true, notifier()).unwrap();
        drain(&mut st);
        assert_eq!(st.issues.len(), 3, "nothing was fixed: every issue is still there");
        assert_eq!(st.issues.iter().filter(|i| i.selected).count(), 2, "and still ticked");
        let v = st.view();
        assert_eq!(v.notices.len(), 1);
        assert_eq!(v.notices[0].tone, Tone::Error);
        assert!(v.notices[0].text.contains("disk full"), "{}", v.notices[0].text);
        assert!(action(&v, "fix").enabled, "the user can try again");
    }

    fn dying_fix(_: Vec<registry::Issue>, _: bool, _out: Outbox<RegMsg>) {}

    #[test]
    fn a_dead_worker_ends_the_job_with_an_error() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: dying_fix });
        st.open_fix().unwrap();
        st.fix(false, notifier()).unwrap();
        drain(&mut st);
        let v = st.view();
        assert!(v.job.is_none());
        assert_eq!(v.notices[0].tone, Tone::Error);
        assert_eq!(st.issues.len(), 3);
    }

    /// A fix worker that dies may have repaired some issues already: its own message says so, and a
    /// dead scan keeps the general one.
    #[test]
    fn a_dead_fix_says_some_issues_may_already_be_fixed_and_a_dead_scan_does_not() {
        fn dying_scan(_: Vec<usize>, _out: Outbox<RegMsg>) {}
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: dying_fix });
        st.open_fix().unwrap();
        st.fix(false, notifier()).unwrap();
        drain(&mut st);
        let n = &st.view().notices[0];
        assert_eq!(n.text, "The worker stopped: some issues may already have been fixed. Scan again to see what is left.");
        assert_eq!(n.tone, Tone::Error);

        let mut st = RegistryState::new().with_workers(RegistryWorkers { scan: dying_scan, fix: never_fix });
        st.scan(notifier()).unwrap();
        drain(&mut st);
        let n = &st.view().notices[0];
        assert_eq!((n.text.as_str(), n.tone), (JOB_DIED, Tone::Error));
    }

    fn fixed_one(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        out.send(RegMsg::Fixed { ok: 1, failed: 0, backup: None, refused: None });
    }
    fn fixed_some(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        out.send(RegMsg::Fixed { ok: 1234, failed: 1, backup: Some(r"C:\Users\u\Documents\Xiox\registry-backup-1.reg".into()), refused: None });
    }

    /// The result of a fix: the word agrees with the count (grouped), a partial result is a warning
    /// (not green) and a clean one is ok.
    #[test]
    fn the_result_of_a_fix_agrees_with_its_count_and_is_a_warning_when_some_failed() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: fixed_one });
        st.open_fix().unwrap();
        st.fix(false, notifier()).unwrap();
        drain(&mut st);
        let n = st.view().notices.remove(0);
        assert_eq!((n.text.as_str(), n.tone), ("Fixed 1 issue.", Tone::Ok));

        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: fixed_some });
        st.open_fix().unwrap();
        st.fix(true, notifier()).unwrap();
        drain(&mut st);
        let n = st.view().notices.remove(0);
        assert_eq!(n.text, "Fixed 1,234 issues. 1 not fixed (administrator rights are needed).");
        assert_eq!(n.tone, Tone::Warn, "some failed: not green");
        assert_eq!(n.more, vec![r"Backup saved in: C:\Users\u\Documents\Xiox\registry-backup-1.reg".to_string()]);
        assert_eq!(st.issues.len(), 3, "a failure keeps the list: the user sees what is left");
        // A new scan or fix starts from ok again.
        st.start_scan(notifier());
        assert_eq!(st.msg_tone, Tone::Ok);
        drain(&mut st);
    }

    #[test]
    fn issue_counts_agree_with_their_word_and_are_grouped() {
        assert_eq!(issue_count(0), "0 issues found");
        assert_eq!(issue_count(1), "1 issue found");
        assert_eq!(issue_count(2), "2 issues found");
        assert_eq!(issue_count(18_204), "18,204 issues found");
    }

    // ---- registry_view_lists_issues_and_the_backup_dialog ----

    #[test]
    fn registry_view_lists_issues_and_the_backup_dialog() {
        let mut st = RegistryState::new().with_workers(NEVER);
        // Before a scan.
        let v = st.view();
        assert_eq!(v.section, SectionId::Registry);
        assert!(v.nav.is_empty() && v.rows.is_empty() && !v.bulk);
        let e = v.empty.as_ref().unwrap();
        assert_eq!(e.title, "Registry cleanup");
        assert_eq!(e.text, "Press “Find issues” to find entries that point to files or programs that no longer exist.");
        assert_eq!(v.options.len(), registry::CATEGORIES.len() + 1);
        for (i, c) in registry::CATEGORIES.iter().enumerate() {
            match &v.options[i] {
                OptionView::Toggle { id, label, on, enabled, .. } => {
                    assert_eq!((id.clone(), label.as_str(), *on, *enabled), (format!("cat:{i}"), c.name, c.default_on, true));
                }
                other => panic!("{other:?}"),
            }
        }
        match v.options.last().unwrap() {
            OptionView::Text { text, .. } => assert_eq!(text, "Before fixing you can save a backup: double-click the .reg file to go back."),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            v.columns.iter().map(|c| (c.title.as_str(), c.kind)).collect::<Vec<_>>(),
            vec![("Issue", ColumnKind::Name), ("Registry key", ColumnKind::Cell)]
        );
        assert_eq!(action(&v, "scan").label, "Find issues");
        assert_eq!(action(&v, "scan").key.as_deref(), Some("a"));
        assert_eq!(action(&v, "fix").label, "Fix selected (0)…");
        assert!(!action(&v, "fix").enabled && action(&v, "fix").primary);
        assert!(v.dialog.is_none() && v.notices.is_empty());

        // After an empty scan.
        st.scanned = true;
        assert_eq!(st.view().empty.unwrap().title, "No issues found");

        // After a scan with issues.
        let mut st = scanned();
        let v = st.view();
        assert!(v.bulk && v.empty.is_none());
        assert_eq!(v.rows.iter().map(|r| (r.id.as_str(), r.kind)).collect::<Vec<_>>(), [
            ("t:0", RowKind::Header), ("i:0", RowKind::Item), ("i:1", RowKind::Item), ("i:2", RowKind::Item),
        ]);
        assert_eq!((row(&v, "t:0").name.as_str(), row(&v, "t:0").check), ("Missing shared DLLs (3)", Check::Mixed));
        let r = row(&v, "i:0");
        assert_eq!((r.name.as_str(), r.sub.as_deref()), ("Missing shared DLL 0", Some("C:\\gone\\0.dll")));
        assert_eq!(r.cells, vec!["HKLM\\SOFTWARE\\Acme\\SharedDLLs".to_string()]);
        assert_eq!(r.check, Check::On);
        assert_eq!(row(&v, "i:2").check, Check::Off);
        assert_eq!(action(&v, "fix").label, "Fix selected (2)…");
        assert!(action(&v, "fix").enabled);

        // The backup question.
        st.open_fix().unwrap();
        let v = st.view();
        let d = v.dialog.as_ref().unwrap();
        assert_eq!(d.id, "backup");
        assert_eq!(d.title, "Save a registry backup?");
        assert_eq!(d.text, vec!["Recommended: before the changes a copy (.reg) is saved in the Documents\\Xiox folder.".to_string()]);
        let issues = plural(st.selected(), "issue", "issues");
        let (backup, without) = (format!("Back up and fix {issues}"), format!("Fix {issues} without a backup"));
        assert_eq!(
            d.buttons.iter().map(|b| (b.id.as_str(), b.label.as_str(), b.primary)).collect::<Vec<_>>(),
            vec![("backup", backup.as_str(), true), ("nobackup", without.as_str(), false), ("cancel", "Cancel", false)]
        );
        assert!(button(d, "backup").enabled && d.ack.is_none());
        st.close_confirm();
        assert!(st.view().dialog.is_none());
    }

    #[test]
    fn issues_sit_under_their_kind_whose_box_ticks_them_all() {
        let mut st = scanned();
        st.issues.push(Issue { category: 1, ..issue(3, false) });
        st.issues.insert(0, Issue { category: 1, ..issue(4, true) });
        let v = st.view();
        assert_eq!(
            v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["t:0", "i:1", "i:2", "i:3", "t:1", "i:0", "i:4"],
            "the kinds in the order of the categories, each with its issues in the order found"
        );
        assert_eq!(row(&v, "t:1").name, format!("{} (2)", registry::CATEGORIES[1].name));
        // Some ticked: all go on; all ticked: all go off; the other kind does not move.
        st.toggle("t:0").unwrap();
        assert!((1..4).all(|i| st.issues[i].selected));
        assert_eq!(row(&st.view(), "t:0").check, Check::On);
        st.toggle("t:0").unwrap();
        assert!((1..4).all(|i| !st.issues[i].selected));
        assert_eq!((st.issues[0].selected, st.issues[4].selected), (true, false));
        assert_eq!(err(st.toggle("t:7")), UNKNOWN_CATEGORY, "a kind with no issue has no box");
        st.open_fix().ok();
        assert!(st.toggle("t:1").is_err(), "refused with the backup question open");
    }

    static HOLD2: AtomicU8 = AtomicU8::new(0);
    fn progress_scan(_: Vec<usize>, out: Outbox<RegMsg>) {
        out.send(RegMsg::Progress("Checking: x".into(), 0.5));
        while HOLD2.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.send(RegMsg::Done);
    }

    #[test]
    fn a_scan_shows_its_progress_and_freezes_the_options() {
        let mut st = RegistryState::new().with_workers(RegistryWorkers { scan: progress_scan, fix: never_fix });
        st.scan(notifier()).unwrap();
        wait(|| {
            st.poll();
            st.progress > 0.0
        });
        let v = st.view();
        let j = v.job.as_ref().unwrap();
        assert_eq!((j.label.as_str(), j.fraction), ("Checking: x", 0.5));
        assert!(v.options.iter().all(|o| !matches!(o, OptionView::Toggle { enabled: true, .. })));
        assert!(!action(&v, "scan").enabled && !action(&v, "fix").enabled);
        assert_eq!(v.status.badge, "Scanning");
        HOLD2.store(1, SeqCst);
        drain(&mut st);
        assert!(st.view().job.is_none());
    }

    static HOLD3: AtomicU8 = AtomicU8::new(0);
    fn gated_fix(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        while HOLD3.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.send(RegMsg::Fixed { ok: 0, failed: 0, backup: None, refused: None });
    }

    /// A job starts from zero: the label and the bar of the scan before it are not shown again.
    #[test]
    fn a_new_job_does_not_inherit_the_last_progress() {
        let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix: gated_fix });
        st.text = "Checking: old".into();
        st.progress = 0.9;
        st.open_fix().unwrap();
        st.fix(false, notifier()).unwrap();
        let j = st.view().job.unwrap();
        assert_eq!((j.label.as_str(), j.fraction), ("Fixing…", 0.0));
        HOLD3.store(1, SeqCst);
        drain(&mut st);
        let mut st = st.with_workers(RegistryWorkers { scan: progress_scan_after_reset, fix: never_fix });
        st.text = "Fixing…".into();
        st.progress = 0.8;
        st.scan(notifier()).unwrap();
        assert_eq!((st.text.as_str(), st.progress), ("", 0.0), "a scan starts from zero too");
        HOLD4.store(1, SeqCst);
        drain(&mut st);
    }

    static HOLD4: AtomicU8 = AtomicU8::new(0);
    fn progress_scan_after_reset(_: Vec<usize>, out: Outbox<RegMsg>) {
        while HOLD4.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.send(RegMsg::Done);
    }

    /// Builder-vs-fixture drift (see `crate::drift`): the static strings of the view against the fixture state `name`.
    fn drift_check(doc: &serde_json::Value, st: &RegistryState, name: &str, out: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>) {
        out.extend(crate::drift::diff(&format!("registry:{name}"), &st.view(), &crate::drift::fixture_view(doc, "registry", name), crate::drift::Opts::all()));
        out.extend(crate::drift::view_problems(&format!("real registry:{name}"), &serde_json::to_value(st.view()).unwrap()));
        seen.insert(name.to_string());
    }

    fn fix_one_failed(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        out.send(RegMsg::Fixed { ok: 1, failed: 1, backup: Some(r"C:\Users\<you>\Documents\Xiox\registry-backup-20261114_103000.reg".into()), refused: None });
    }
    fn fix_all_fixed(_: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        out.send(RegMsg::Fixed { ok: 2, failed: 0, backup: Some(r"C:\Users\<you>\Documents\Xiox\registry-backup-20261114_103000.reg".into()), refused: None });
    }

    /// The `fixed` and `partial` fixture states, built by the real state with a fix that ends so:
    /// the notice text, its tone and the backup line are the builder's, and the fixture shows the same.
    #[test]
    fn the_fixed_and_partial_states_are_what_the_builder_says() {
        let doc = crate::drift::document();
        let (mut drift, mut seen) = (Vec::new(), Default::default());
        for (name, fix) in [("partial", fix_one_failed as fn(Vec<registry::Issue>, bool, Outbox<RegMsg>)), ("fixed", fix_all_fixed)] {
            let mut st = scanned().with_workers(RegistryWorkers { scan: never_scan, fix });
            st.open_fix().unwrap();
            st.fix(true, notifier()).unwrap();
            drain(&mut st);
            drift_check(&doc, &st, name, &mut drift, &mut seen);
            let n = &st.view().notices[0];
            assert_eq!(n.tone, if name == "partial" { Tone::Warn } else { Tone::Ok }, "{name}");
            assert!(n.more[0].starts_with("Backup saved in: "), "{name}");
        }
        assert!(drift.is_empty(), "{}", drift.join("\n"));
    }

    /// Every Registry state of the shared fixture reads through the Rust types (no missing key,
    /// no unknown one) and round-trips.
    #[test]
    fn the_registry_states_of_the_shared_fixture_read_through_the_contract() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/views.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let states = doc["views"]["registry"].as_object().unwrap();
        assert_eq!(states.len(), 7, "idle, scanning, analyzed, backup, fixing, fixed, partial");
        for (name, view) in states {
            let v: SectionView = serde_json::from_value(view.clone()).unwrap_or_else(|e| panic!("registry:{name}: {e}"));
            assert_eq!(v.section, SectionId::Registry);
            let again: SectionView = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
            assert_eq!(again, v, "registry:{name} does not round-trip");
        }
    }

    // ---- the replay of the shared script ----

    static SCAN_GATE: AtomicU8 = AtomicU8::new(0);
    static FIX_GATE: AtomicU8 = AtomicU8::new(0);

    /// The forty issues of the fixture's `analyzed` state: two ticked, the rest not.
    /// The fixture's 40 issues: the kinds in turn (as the fixture lists them), the first two ticked.
    fn script_issues() -> Vec<Issue> {
        (0..40).map(|n| Issue { category: n % 11, ..issue(n, n < 2) }).collect()
    }

    fn script_scan(_: Vec<usize>, out: Outbox<RegMsg>) {
        while SCAN_GATE.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        out.send(RegMsg::Found(script_issues()));
        out.send(RegMsg::Done);
    }

    fn script_fix(chosen: Vec<registry::Issue>, _: bool, out: Outbox<RegMsg>) {
        while FIX_GATE.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        out.send(RegMsg::Fixed { ok: chosen.len(), failed: 0, backup: None, refused: None });
    }

    const SCRIPT: RegistryWorkers = RegistryWorkers { scan: script_scan, fix: script_fix };

    /// The fixture state a state corresponds to: what the script's `expect.state` says.
    fn shape(st: &RegistryState) -> &'static str {
        match (st.busy(), st.confirm, st.scanned, st.fixing) {
            (true, _, _, true) => "fixing",
            (true, _, _, false) => "scanning",
            (false, true, _, _) => "backup",
            (false, false, false, _) => "idle",
            (false, false, true, _) if !st.msg.is_empty() => "fixed",
            _ => "analyzed",
        }
    }

    fn script_state(name: &str) -> RegistryState {
        let mut st = RegistryState::new().with_workers(SCRIPT);
        if name == "analyzed" {
            st.issues = script_issues();
            st.scanned = true;
        }
        st
    }

    fn apply(st: &mut RegistryState, intent: RegistryIntent) -> Result<(), String> {
        let n = notifier();
        match intent {
            RegistryIntent::ToggleCategory { index } => st.toggle_category(index),
            RegistryIntent::Toggle { row } => st.toggle(&row),
            RegistryIntent::SelectAll { on } => st.select_all(on),
            RegistryIntent::Scan => st.scan(n),
            RegistryIntent::OpenFix => st.open_fix(),
            RegistryIntent::Fix { backup } => st.fix(backup, n),
            RegistryIntent::CloseDialog => {
                if !st.confirm {
                    return Err(NO_DIALOG.into());
                }
                st.close_confirm();
                Ok(())
            }
        }
    }

    fn finish(st: &mut RegistryState, gate: &AtomicU8) {
        gate.store(1, SeqCst);
        drain(st);
        gate.store(0, SeqCst);
    }

    /// Replays the Registry steps of `apps/desktop/fixtures/intent-script.json` against the real
    /// state, through the worker seam (the workers wait at a gate and answer when the script's
    /// `tick` says the job lands). No step is re-synchronised: the script (since the mock ticks any
    /// entry, the engine has no protected registry ones) and Rust agree on every state compared.
    #[test]
    fn replays_the_registry_steps_of_the_shared_script() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/intent-script.json");
        let script: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut st = script_state("idle");
        let doc = crate::drift::document();
        let (mut drift, mut seen) = (Vec::<String>::new(), std::collections::BTreeSet::<String>::new());
        drift_check(&doc, &st, "idle", &mut drift, &mut seen);
        let (mut compared, mut landed) = (0, 0);
        for step in script["steps"].as_array().unwrap() {
            let note = step["note"].as_str().unwrap_or("");
            match step["op"].as_str().unwrap() {
                "go" if step["section"] == "registry" => {
                    st = script_state(step["state"].as_str().unwrap());
                    if shape(&st) == step["state"].as_str().unwrap() {
                        drift_check(&doc, &st, shape(&st), &mut drift, &mut seen);
                    }
                }
                "tick" => {
                    for pushed in step["pushed"].as_array().unwrap().iter().filter(|p| p["section"] == "registry") {
                        match (shape(&st), pushed["state"].as_str().unwrap()) {
                            ("scanning", "analyzed") => {
                                finish(&mut st, &SCAN_GATE);
                                assert_eq!(shape(&st), "analyzed", "{note}");
                                landed += 1;
                            }
                            ("fixing", "fixed") => {
                                finish(&mut st, &FIX_GATE);
                                assert_eq!(shape(&st), "fixed", "{note}");
                                landed += 1;
                            }
                            // A tick that only moves the fraction: the job goes on.
                            (now, want) => assert_eq!(now, want, "{note}"),
                        }
                    }
                }
                "intent" if step["section"] == "registry" => {
                    let intent: RegistryIntent = serde_json::from_value(step["intent"].clone()).unwrap();
                    let expect = step["expect"]["state"].as_str().unwrap();
                    let result = apply(&mut st, intent.clone());
                    assert_eq!(result.is_ok(), note != "nothing to fix before a scan", "{note}: {intent:?}");
                    let v = st.view();
                    if let Some(checks) = step["expect"]["checks"].as_object() {
                        for (id, want) in checks {
                            let got = serde_json::to_value(row(&v, id).check).unwrap();
                            assert_eq!(&got, want, "{note}: check of {id}");
                        }
                    }
                    assert_eq!(shape(&st), expect, "{note}: {intent:?}");
                    assert_eq!(v.dialog.is_some(), expect == "backup", "{note}: dialog");
                    drift_check(&doc, &st, expect, &mut drift, &mut seen);
                    compared += 1;
                }
                _ => {}
            }
        }
        assert!(crate::drift::unknown(&drift).is_empty(), "the views the builders make differ from the fixture:\n{}", crate::drift::unknown(&drift).join("\n"));
        for want in ["idle", "scanning", "analyzed", "backup", "fixing"] {
            assert!(seen.contains(want), "registry:{want} was compared with the fixture (compared: {seen:?})");
        }
        assert!(compared >= 9, "{compared} registry intents compared");
        assert_eq!(landed, 2, "jobs landed: the scan and the fix");
    }
}
