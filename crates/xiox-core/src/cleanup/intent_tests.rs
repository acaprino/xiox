//! Intent tests of Cleanup: what `CleanupState` refuses, what it lets through and what the view shows.
//! The worker is a test function behind `CleanupWorkers`: nothing here reaches the engine, the disk,
//! the registry or the Recycle Bin, and `open_keep_list` is only ever called where it refuses (the real
//! one starts an editor).

use super::*;
use crate::native::NativeFound;
use crate::view::testing::{action, button, err, notifier, row};
use crate::view::*;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use xiox_rules::{Cleaner, Opt, Origin};

// ---------- worker seam ----------

/// What the recording worker was asked to do: (`also_clean`, categories, sorted preview keys).
static JOBS: Mutex<Vec<(bool, Vec<usize>, Vec<String>)>> = Mutex::new(Vec::new());
/// The tests that read `JOBS` run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn jobs() -> Vec<(bool, Vec<usize>, Vec<String>)> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Records the job; an analysis also reports the natives it was given (so a later cleanup has a
/// preview on screen), then ends.
fn record(job: CleanJob, out: Outbox<CleanMsg>) {
    let mut keys: Vec<String> = job.previews.keys().cloned().collect();
    keys.sort();
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push((job.also_clean, job.cats.clone(), keys));
    if !job.also_clean {
        for item in job.natives.iter().chain(&job.natives_last) {
            let found = NativeFound { count: 1, items: vec!["found".into()], ..Default::default() };
            out.send(CleanMsg::Native(rules::native_key(item), item.label.to_string(), found, None));
        }
    }
    out.send(CleanMsg::Done);
}

/// Ends at once, records nothing.
fn quiet(_: CleanJob, out: Outbox<CleanMsg>) {
    out.send(CleanMsg::Done);
}

/// Runs until «Cancel» raises the job's flags (gives up after five seconds).
fn hold(job: CleanJob, out: Outbox<CleanMsg>) {
    for _ in 0..500 {
        if job.files_cancel.is_cancelled() || job.cancel.load(SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    out.send(CleanMsg::Done);
}

const QUIET: CleanupWorkers = CleanupWorkers { job: quiet };
const HOLD: CleanupWorkers = CleanupWorkers { job: hold };

// ---------- states ----------

fn natives_for_test() -> Vec<native::NativeItem> {
    native::all().into_iter().filter(|i| i.id == "env_path" || i.id == "clipboard").collect()
}

pub(super) fn opt(id: &str, label: &str, description: &str) -> Opt {
    Opt {
        id: id.into(),
        label: label.into(),
        description: description.into(),
        warning: None,
        default_on: false,
        actions: vec![],
        excludes: vec![],
        unsupported: None,
    }
}

pub(super) fn cleaner(id: &str, label: &str, origin: Origin, group: &str, options: Vec<Opt>) -> Cleaner {
    Cleaner { id: id.into(), label: label.into(), group: group.into(), origin, description: String::new(), running: vec![], options }
}

/// One embedded cleaner (a warning option without description, an unsupported one with one) and three
/// Winapp2 ones in two sections.
fn rule_set() -> rules::RuleSet {
    let mut history = opt("history", "History", "");
    history.warning = Some("Forgets every command you typed.".into());
    let mut legacy = opt("legacy", "Legacy", "Old files.");
    legacy.unsupported = Some("needs the old engine".into());
    let both = |label: &str| vec![opt("logs", "Logs", ""), opt("temp", label, "")];
    rules::RuleSet {
        cleaners: vec![
            cleaner("t_tool", "Toolchain", Origin::CleanerMl("t"), rules::EMBEDDED_GROUP, vec![history, legacy]),
            cleaner("w_alpha", "Alpha App", Origin::Winapp2, "Utilities", vec![opt("cache", "Alpha App", "")]),
            cleaner("w_beta", "Beta Tool", Origin::Winapp2, "Utilities", both("Temp")),
            cleaner("w_gamma", "Gamma", Origin::Winapp2, "Browsers", vec![opt("cache", "Gamma", "")]),
        ],
        winapp2_version: Some("0.1".into()),
        status: "Winapp2 0.1 loaded".into(),
        warnings: vec!["Winapp2: 2 regole scartate".into()],
        env: xiox_rules::Env::for_rules(),
        sections: vec![("Browsers".into(), vec![3]), ("Utilities".into(), vec![1, 2])],
        groups: vec![(rules::EMBEDDED_GROUP.into(), vec![0])],
    }
}

pub(super) fn state_with(workers: CleanupWorkers) -> CleanupState {
    let mut st = CleanupState::with(natives_for_test(), vec![], workers);
    st.rules = Some(rule_set());
    st
}

fn state() -> CleanupState {
    state_with(QUIET)
}

/// A state whose jobs are recorded; the guard keeps the other recording tests out meanwhile.
fn recording() -> (CleanupState, MutexGuard<'static, ()>) {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    (state_with(CleanupWorkers { job: record }), guard)
}

/// Waits for the job to land (polling, like the pump does).
pub(super) fn finish(st: &mut CleanupState) {
    for _ in 0..500 {
        st.poll();
        if !st.busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the worker never answered");
}

/// An analysis on screen.
fn analyzed(workers: CleanupWorkers) -> CleanupState {
    let mut st = state_with(workers);
    st.analyze(notifier()).unwrap();
    finish(&mut st);
    st
}

/// The ids of the rows a category holds in the view: after its `Group` row, up to the next one.
fn rows_of<'a>(v: &'a SectionView, group: &str) -> Vec<&'a str> {
    let at = v.rows.iter().position(|r| r.kind == RowKind::Group && r.id == group).unwrap_or_else(|| panic!("no category {group} in the list"));
    v.rows[at + 1..].iter().take_while(|r| r.kind != RowKind::Group).map(|r| r.id.as_str()).collect()
}

/// The view as JSON and back: every view these tests look at has to read through the contract.
fn view(st: &CleanupState, admin: bool) -> SectionView {
    let v = st.view(admin);
    let back: SectionView = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
    assert_eq!(back, v, "the view does not round-trip");
    v
}

fn detail<'a>(r: &'a RowView, key: &str) -> Option<&'a str> {
    r.detail.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn blocker(rs: &mut rules::RuleSet) {
    rs.cleaners[1].running = vec!["blocker.exe".into()];
}

// ---------- selection ----------

#[test]
fn rows_are_keyed_and_unknown_or_blocked_rows_are_refused() {
    let mut st = state();
    assert!(st.clean_sel[0]);
    st.toggle("cat:0").unwrap();
    assert!(!st.clean_sel[0]);
    assert!(st.rule_sel.contains("native:clipboard"));
    st.toggle("native:clipboard").unwrap();
    assert!(!st.rule_sel.contains("native:clipboard"));
    st.toggle("t_tool/history").unwrap();
    assert!(st.rule_sel.contains("t_tool/history"));
    st.toggle("t_tool/history").unwrap();
    assert!(!st.rule_sel.contains("t_tool/history"));
    // The keys of the view are the keys of the selection.
    let v = view(&st, false);
    assert_eq!(rows_of(&v, "rules:Development & AI"), ["t_tool/history", "t_tool/legacy"]);
    assert_eq!(rows_of(&v, "rules:Windows"), ["native:clipboard", "native:env_path"]);

    // Unknown ids and headers change nothing.
    let before = (st.clean_sel.clone(), st.rule_sel.clone());
    for id in ["", "cat:99", "cat:x", "native:nope", "t_tool/nope", "nope/history", "section:Nope", "history"] {
        assert!(st.toggle(id).is_err(), "{id} must be refused");
    }
    // Blocked rows: an unsupported option, an option and a native whose program runs.
    let legacy = rule_set();
    let why = rules::blocked(&legacy.cleaners[0], &legacy.cleaners[0].options[1], &[]).unwrap();
    assert_eq!(err(st.toggle("t_tool/legacy")), why);
    let mut rs = rule_set();
    blocker(&mut rs);
    st.rules = Some(rs);
    st.processes = vec!["Blocker.EXE".into()];
    let why = rules::blocked(&st.rules.as_ref().unwrap().cleaners[1], &st.rules.as_ref().unwrap().cleaners[1].options[0], &st.processes).unwrap();
    assert_eq!(err(st.toggle("w_alpha/cache")), why);
    st.natives[0].running = &["blocker"];
    let why = rules::native_blocked(&st.natives[0], &st.processes).unwrap();
    assert_eq!(err(st.toggle("native:clipboard")), why);
    assert_eq!((st.clean_sel.clone(), st.rule_sel.clone()), before, "a refused toggle changes nothing");
    let v = view(&st, false);
    let r = row(&v, "native:clipboard");
    assert!(!r.enabled && r.muted && r.check == Check::Off);
    assert_eq!(r.why.as_deref(), Some(why.as_str()));
}

#[test]
fn selection_is_frozen_while_busy_or_with_the_dialog_open() {
    let mut st = state_with(HOLD);
    st.analyze(notifier()).unwrap();
    assert!(st.busy());
    let before = (st.clean_sel.clone(), st.rule_sel.clone(), st.only_old);
    for (name, r) in [
        ("toggle", st.toggle("cat:0")),
        ("category box", st.toggle("builtin:Advanced")),
        ("select all", st.select_all(true)),
        ("only old", st.set_only_old(false)),
        ("analyze", st.analyze(notifier())),
        ("clean", st.request_clean()),
        ("confirm", st.confirm(notifier())),
    ] {
        assert_eq!(err(r), BUSY, "{name} while a job runs");
    }
    assert_eq!((st.clean_sel.clone(), st.rule_sel.clone(), st.only_old), before);
    let v = view(&st, false);
    assert!(!action(&v, "analyze").enabled && !action(&v, "clean").enabled);
    st.cancel_job().unwrap();
    finish(&mut st);

    // The confirmation describes the selection: it may not move under it.
    st.request_clean().unwrap();
    assert!(st.confirm_clean);
    for (name, r) in [
        ("toggle", st.toggle("cat:0")),
        ("category box", st.toggle("builtin:Advanced")),
        ("select all", st.select_all(false)),
        ("only old", st.set_only_old(false)),
        ("analyze", st.analyze(notifier())),
        ("clean", st.request_clean()),
    ] {
        assert_eq!(err(r), DIALOG_OPEN, "{name} with the confirmation open");
    }
    assert_eq!((st.clean_sel.clone(), st.rule_sel.clone(), st.only_old), before);
    let v = view(&st, false);
    assert!(v.options.iter().all(|o| !matches!(o, OptionView::Toggle { enabled: true, .. })), "the box is off while the dialog is open");
    st.close_confirm();
    st.toggle("cat:0").unwrap();

    // A report on screen freezes them too; Analyze is the way out (or releasing it).
    st.clean_view = CleanView::Cleaned;
    st.clean_report = Some(Default::default());
    assert_eq!(err(st.toggle("cat:0")), REPORT_OPEN);
    assert_eq!(err(st.toggle("builtin:Advanced")), REPORT_OPEN);
    assert_eq!(err(st.select_all(true)), REPORT_OPEN);
    assert_eq!(err(st.set_only_old(false)), REPORT_OPEN);
    st.release();
    assert!(st.clean_report.is_none() && st.clean_view == CleanView::Empty);
    st.toggle("cat:0").unwrap();
}

#[test]
fn analyze_needs_something_selected() {
    let (mut st, _serial) = recording();
    st.clean_sel = vec![false; cleaner::CATEGORIES.len()];
    st.rule_sel.clear();
    assert_eq!(err(st.analyze(notifier())), NOTHING);
    st.clean_view = CleanView::Analyzed;
    assert_eq!(err(st.request_clean()), NOTHING);
    let v = view(&st, false);
    assert!(!action(&v, "analyze").enabled && !action(&v, "clean").enabled);
    assert_eq!(action(&v, "clean").label, "Clean");
    // A selected row that is blocked does not count.
    st.rule_sel.insert("native:clipboard".into());
    st.natives[0].running = &["blocker"];
    st.processes = vec!["blocker.exe".into()];
    assert_eq!(err(st.analyze(notifier())), NOTHING);
    assert!(jobs().is_empty(), "nothing started");
    st.natives[0].running = &[];
    st.analyze(notifier()).unwrap();
    finish(&mut st);
    assert_eq!(jobs().len(), 1);
    assert!(!jobs()[0].0, "Analyze never cleans");
    // `Clean 1 item`: the one native.
    let v = view(&st, false);
    assert_eq!(action(&v, "clean").label, "Clean 1 item");
}

#[test]
fn cleanup_confirm_needs_the_open_dialog() {
    let (mut st, _serial) = recording();
    assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
    assert!(jobs().is_empty());
    // Clean works on the analysis on screen: there is none yet.
    assert_eq!(err(st.request_clean()), NO_ANALYSIS);
    assert!(!st.confirm_clean && jobs().is_empty());
    st.analyze(notifier()).unwrap();
    finish(&mut st);
    assert_eq!(jobs().len(), 1);
    // Opening and closing runs nothing; a closed dialog cannot be confirmed.
    st.request_clean().unwrap();
    st.close_confirm();
    assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
    assert_eq!(jobs().len(), 1);
    st.request_clean().unwrap();
    assert!(st.confirm_clean);
    st.confirm(notifier()).unwrap();
    assert!(!st.confirm_clean, "the dialog is closed");
    finish(&mut st);
    let all = jobs();
    assert_eq!(all.iter().filter(|j| j.0).count(), 1, "one cleaning job: {all:?}");
    assert!(all[1].0);
    assert!(all[1].1.iter().all(|i| st.clean_sel[*i]), "only the chosen categories");
    // A second Enter finds no dialog.
    assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
    assert_eq!(jobs().len(), 2);
}

#[test]
fn previews_only_from_the_analysis_on_screen() {
    let (mut st, _serial) = recording();
    st.rule_sel.insert("native:env_path".into());
    st.rule_sel.remove("native:clipboard");
    let shown = |st: &mut CleanupState| {
        st.native_found.push(("native:env_path".into(), "Environment PATH".into(), NativeFound { items: vec!["C:\\gone".into()], ..Default::default() }));
    };
    // Cleaning from the analysis on screen: the PATH preview goes with the job.
    st.clean_view = CleanView::Analyzed;
    shown(&mut st);
    st.start(true, notifier());
    finish(&mut st);
    assert_eq!(jobs().last().unwrap().2, ["native:env_path"]);
    // The same entry, but the screen is no longer an analysis: no preview.
    st.clean_view = CleanView::Cleaned;
    shown(&mut st);
    st.start(true, notifier());
    finish(&mut st);
    assert!(jobs().last().unwrap().2.is_empty());
    // An analysis never carries one.
    st.clean_view = CleanView::Analyzed;
    shown(&mut st);
    st.start(false, notifier());
    finish(&mut st);
    assert!(jobs().last().unwrap().2.is_empty());
    // End to end: Analyze shows the PATH entries, Clean through the dialog uses exactly those; a
    // new Analyze that replaced them leaves nothing to reuse once the cleaning is over.
    st.analyze(notifier()).unwrap();
    finish(&mut st);
    st.request_clean().unwrap();
    st.confirm(notifier()).unwrap();
    finish(&mut st);
    let last = jobs().pop().unwrap();
    assert!(last.0 && last.2 == ["native:env_path"], "{last:?}");
    assert_eq!(st.clean_view, CleanView::Cleaned);
    assert_eq!(err(st.request_clean()), NO_ANALYSIS, "a report is not an analysis to clean");
}

/// The warnings of an open confirmation describe the rules it was opened for: rules that load under
/// it close it (a new analysis is not needed, a new confirmation is).
#[test]
fn a_dialog_does_not_outlive_the_rules_it_was_opened_for() {
    let mut st = analyzed(QUIET);
    st.request_clean().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    st.rules_rx = Some(rx);
    tx.send(rules::RulesMsg::Updated(xiox_rules::updater::UpdateOutcome::UpToDate)).unwrap();
    st.poll();
    assert!(st.confirm_clean, "an update outcome changes no rule");
    tx.send(rules::RulesMsg::Loaded(rule_set())).unwrap();
    st.poll();
    assert!(!st.confirm_clean);
    assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
    st.request_clean().unwrap();
}

// ---------- the view ----------

#[test]
fn cleanup_view_groups_levels_and_modes() {
    let mut st = state();
    let v = view(&st, false);
    let nav: Vec<(&str, &str, bool)> = v.nav.iter().map(|n| (n.id.as_str(), n.name.as_str(), n.active)).collect();
    assert_eq!(
        nav,
        [
            ("builtin:System", "System", false),
            ("builtin:Explorer", "Explorer", false),
            ("builtin:Advanced", "Advanced", false),
            ("rules:Windows", "Windows", false),
            ("rules:Development & AI", "Development & AI", false),
            ("winapp2", rules::WINAPP2_GROUP, false),
        ]
    );
    assert_eq!(v.nav[0].note.as_deref(), Some("6 items · 6 selected"));
    assert_eq!(v.nav[1].note.as_deref(), Some("1 item · 0 selected"));
    assert_eq!(v.nav[3].note.as_deref(), Some("2 items · 1 selected"));
    assert_eq!(v.nav[4].note.as_deref(), Some("2 items · 0 selected"));
    assert_eq!(v.nav[5].note.as_deref(), Some("4 items · 0 selected"));
    assert!(v.nav.iter().all(|n| n.bytes.is_none()), "no analysis yet");
    assert_eq!(v.columns.iter().map(|c| (c.title.as_str(), c.kind)).collect::<Vec<_>>(), [
        ("Item", ColumnKind::Name),
        ("Level", ColumnKind::Level),
        ("Files", ColumnKind::Cell),
        ("Size", ColumnKind::Size),
    ]);
    assert_eq!((v.section, v.rev, v.title.as_str()), (SectionId::Cleanup, 0, "Cleanup"));
    assert!(v.bulk && v.dialog.is_none() && v.report.is_none() && v.job.is_none());
    assert_eq!(v.search, Some(SearchView { placeholder: "Search all categories".into(), value: String::new() }));
    // The subtitle counts what the list shows: 6 + 1 + 2 + 2 + 2 + 4 rows, the ticked ones.
    let ticked = v.rows.iter().filter(|r| r.kind == RowKind::Item && r.check == Check::On).count();
    assert_eq!(v.subtitle, format!("17 items · {ticked} selected"));
    // Built-in rows: the Recycle Bin empties, a default-on category is recommended.
    let bin = row(&v, "cat:0");
    assert_eq!((bin.level, bin.check, bin.enabled), (Some(LevelView::Recommended), Check::On, true));
    assert_eq!(bin.name, cleaner::CATEGORIES[0].name);
    assert_eq!(bin.sub.as_deref(), Some(cleaner::CATEGORIES[0].help));
    assert_eq!(detail(bin, "mode"), Some("empties the Recycle Bin"));
    assert_eq!(detail(row(&v, "cat:1"), "mode"), Some("permanently"));
    assert_eq!(detail(bin, "why"), Some(cleaner::CATEGORIES[0].help));
    assert_eq!(detail(bin, "item"), Some(cleaner::CATEGORIES[0].name));
    assert!(bin.cells.is_empty() && bin.bytes.is_none(), "no figures before an analysis");
    // Default-off: optional. Needs administrator: said for a standard user, not for an administrator.
    let thumbs = row(&v, "cat:6");
    assert_eq!((thumbs.level, thumbs.check), (Some(LevelView::Optional), Check::Off));
    let why = "Needs administrator: some folders are skipped";
    assert_eq!(row(&view(&st, false), "cat:7").why.as_deref(), Some(why));
    assert_eq!(row(&view(&st, true), "cat:7").why, None);
    // Natives and options: caution for a warning, the blocked reason for an unsupported one.
    let (clip, path) = (row(&v, "native:clipboard"), row(&v, "native:env_path"));
    assert_eq!((clip.level, clip.check, clip.sub.as_deref()), (Some(LevelView::Recommended), Check::On, Some(st.natives[0].description)));
    assert_eq!((path.level, path.check, path.enabled), (Some(LevelView::Caution), Check::Off, true));
    assert_eq!(path.why.as_deref(), st.natives[1].warning);
    assert_eq!(detail(path, "warning"), st.natives[1].warning);
    assert_eq!(detail(path, "mode"), Some("permanently"));
    let (history, legacy) = (row(&v, "t_tool/history"), row(&v, "t_tool/legacy"));
    assert_eq!((history.name.as_str(), history.level, history.enabled), ("Toolchain – History", Some(LevelView::Caution), true));
    assert_eq!(history.sub, None, "neither the option nor its cleaner has a description");
    assert!(detail(history, "why").is_none(), "no description: no why");
    assert_eq!(detail(history, "warning"), Some("Forgets every command you typed."));
    let rs = rule_set();
    let blocked = rules::blocked(&rs.cleaners[0], &rs.cleaners[0].options[1], &[]).unwrap();
    assert_eq!((legacy.enabled, legacy.muted, legacy.check, legacy.level), (false, true, Check::Off, Some(LevelView::Optional)));
    assert_eq!(legacy.why.as_deref(), Some(blocked.as_str()));
    assert_eq!((legacy.sub.as_deref(), detail(legacy, "why")), (Some("Old files."), Some("Old files.")));
    // The box, the status line of Winapp2 (the rules are loaded), no cookie list without the item, the buttons.
    assert_eq!(v.options.len(), 2, "no cookie list: this build has no cookie item");
    assert!(matches!(&v.options[0], OptionView::Toggle { id, label, on: true, enabled: true, .. }
        if id == "onlyOld" && label == "Only delete Temp files older than 24 hours"));
    assert!(matches!(&v.options[1], OptionView::Text { .. }));
    assert_eq!((action(&v, "analyze").label.as_str(), action(&v, "analyze").key.as_deref(), action(&v, "analyze").enabled), ("Analyze", Some("a"), true));
    let clean = action(&v, "clean");
    assert_eq!((clean.label.as_str(), clean.key.as_deref(), clean.primary, clean.enabled), ("Clean 7 items", Some("Enter"), true, false));
    assert_eq!((v.status.badge.as_str(), v.status.text.as_str()), ("Ready", "Pick what to clean, then analyze"));
    // After an analysis: figures, sizes per group, the total in the status.
    st.clean_view = CleanView::Analyzed;
    st.found[0] = Some(cleaner::Found { count: 5, bytes: 1000, skipped_dirs: 2, ..Default::default() });
    st.found[7] = Some(cleaner::Found { count: 2, bytes: 24, ..Default::default() });
    st.native_found.push(("native:clipboard".into(), "Clipboard".into(), NativeFound { bytes: 0, detail: vec!["1 item".into(), "other".into()], ..Default::default() }));
    st.rule_found.push(("t_tool/history".into(), "Toolchain – History".into(), rules::RuleFound { count: 4, bytes: 77, skipped: 3, skipped_reasons: vec!["a".into(), "b".into()], ..Default::default() }));
    let v = view(&st, false);
    assert_eq!(v.nav.iter().map(|n| n.bytes).collect::<Vec<_>>(), [Some(1000), Some(0), Some(24), Some(0), Some(77), Some(0)]);
    assert_eq!(v.status.text, format!("{} to delete (11 files)", sys::fmt_size(1101)));
    let history = row(&v, "t_tool/history");
    assert_eq!((history.cells.as_slice(), history.bytes), (&["4".to_string()][..], Some(77)));
    assert_eq!(detail(history, "left"), Some("3 items left (protected or unreadable)"));
    assert_eq!(history.lines, ["a", "b"]);
    let clip = row(&v, "native:clipboard");
    assert_eq!((clip.cells.as_slice(), clip.lines.len()), (&["—".to_string()][..], 2), "not files: a dash, the lines are the detail");
    let bin = row(&v, "cat:0");
    assert_eq!((bin.cells.as_slice(), bin.bytes), (&["5".to_string()][..], Some(1000)));
    assert!(detail(bin, "skipped").is_some(), "folders were skipped without administrator rights");
    assert_eq!(detail(row(&view(&st, true), "cat:0"), "skipped"), None);
}

#[test]
fn cleanup_view_confirm_lists_the_warnings_of_the_open_dialog() {
    let mut st = analyzed(QUIET);
    st.rule_sel.insert("t_tool/history".into());
    st.rule_sel.insert("native:env_path".into());
    st.request_clean().unwrap();
    let v = view(&st, false);
    let d = v.dialog.as_ref().expect("the dialog is open");
    assert_eq!(d.id, "confirm");
    assert_eq!(d.title, "Start cleaning?");
    assert_eq!(d.text, ["The selected files will be deleted permanently. Files in use are left where they are."]);
    assert_eq!(d.warnings_title.as_deref(), Some("Warnings for the chosen options"));
    assert_eq!(d.warnings, st.confirm_warnings);
    assert!(d.warnings.iter().any(|w| w.starts_with("Toolchain – History: ")), "{:?}", d.warnings);
    assert!(d.warnings.iter().any(|w| w.starts_with(st.natives[1].label)), "{:?}", d.warnings);
    // The confirmation carries its own numbers (the page decides nothing from the action label) and says it deletes for good.
    let clean = v.actions.iter().find(|a| a.id == "clean").expect("the clean action");
    assert_eq!(d.summary, [clean.label.clone(), v.status.text.clone()]);
    assert!(d.summary[0].starts_with("Clean ") && d.summary[1].contains(" to delete ("), "{:?}", d.summary);
    assert!(d.permanent);
    assert!(d.rows.is_empty() && d.cautions.is_empty() && d.caution_title.is_none() && d.ack.is_none() && d.error.is_none());
    // Primary first, as the old window's dialog had them.
    assert_eq!(d.buttons.iter().map(|b| (b.id.as_str(), b.label.as_str(), b.primary, b.enabled)).collect::<Vec<_>>(), [
        ("continue", clean.label.as_str(), true, true),
        ("cancel", "Cancel", false, true),
    ]);
    assert!(button(d, "continue").enabled);
    // Behind it the section is frozen.
    assert!(!action(&v, "analyze").enabled && !action(&v, "clean").enabled);
    // The warnings are the ones of what is chosen now: nothing chosen with a warning, no title.
    st.close_confirm();
    assert!(view(&st, false).dialog.is_none());
    st.rule_sel.clear();
    st.request_clean().unwrap();
    let d = view(&st, false).dialog.unwrap();
    assert!(d.warnings.is_empty() && d.warnings_title.is_none());
}

#[test]
fn cleanup_view_report_and_items_after_cleaning() {
    let mut st = state();
    let (tx, rx) = std::sync::mpsc::channel();
    st.clean_job = Some(rx);
    st.clean_view = CleanView::Cleaned;
    st.clean_cancel = Some((xiox_cleaner::Cancel::default(), Arc::new(AtomicBool::new(false))));
    st.clean_chosen = Chosen { cats: vec![1, 3], others: vec![("native:clipboard".into(), "Clipboard".into()), ("t_tool/history".into(), "Toolchain – History".into())] };
    let locked = PathBuf::from(r"C:\t\a.tmp");
    tx.send(CleanMsg::Progress("Cleaning – Recycle Bin".into(), 0.5)).unwrap();
    let cat = cleaner::Cleaned { freed: 100, removed: 4, failed: 1, dirs: 2, failures: vec![(locked.clone(), "in uso".into())], cancelled: false };
    tx.send(CleanMsg::Cleaned(1, cleaner::Found::default(), cat)).unwrap();
    let nat = native::NativeCleaned { removed: 1, notes: vec!["a note".into()], done: vec!["Clipboard emptied".into()], ..Default::default() };
    tx.send(CleanMsg::Native("native:clipboard".into(), "Clipboard".into(), NativeFound::default(), Some(nat))).unwrap();
    let rule = rules::RuleCleaned { freed: 500, removed: 12, files: 10, failed: 2, backup: Some(PathBuf::from(r"C:\b\x.reg")), ..Default::default() };
    tx.send(CleanMsg::Rule("t_tool/history".into(), "Toolchain – History".into(), rules::RuleFound::default(), Some(rule))).unwrap();
    st.poll();
    // Running: the totals come through the status, the job through `job`, Cancel is offered.
    let v = view(&st, false);
    assert_eq!((v.status.badge.as_str(), v.status.text.clone()), ("Cleaning", format!("{} freed", sys::fmt_size(600))));
    let job = v.job.as_ref().unwrap();
    assert_eq!((job.label.as_str(), job.fraction, job.detail.as_deref()), ("Cleaning…", 0.5, Some("Cleaning – Recycle Bin")));
    assert!(action(&v, "cancel").enabled && !action(&v, "clean").enabled && !action(&v, "analyze").enabled);
    assert!(v.report.is_none() && v.rows.iter().any(|r| r.id == "cat:0"), "the list stays while it runs");
    assert!(!v.bulk && v.search.is_none(), "the box of the list and its search wait for the job");
    assert!(v.rows.iter().filter(|r| r.kind == RowKind::Group).all(|r| !r.enabled), "so do the boxes of the categories");
    assert_eq!(st.report_text(), None, "nothing to save while it runs");
    tx.send(CleanMsg::Done).unwrap();
    st.poll();
    let v = view(&st, false);
    let r = v.report.as_ref().expect("the report");
    assert_eq!((r.title.as_str(), r.cancelled, r.freed_bytes, r.files, r.dirs), ("Cleanup done", false, 600, 14, 2));
    assert_eq!(v.status.badge, "Cleanup done");
    assert_eq!(r.by_category, [("Toolchain – History".to_string(), 500), (cleaner::CATEGORIES[1].name.to_string(), 100)]);
    // The files that stayed name their category; a counted failure names the option; the items that
    // never answered are skipped.
    let named: Vec<(&str, &str, &str)> = r.failed.iter().map(|l| (l.item.as_str(), l.why.as_str(), l.path.as_str())).collect();
    assert_eq!(named[0], (cleaner::CATEGORIES[1].name, "in uso", r"C:\t\a.tmp"));
    assert_eq!((named[1].0, named[1].2), ("Toolchain – History", ""));
    let skipped: Vec<(&str, &str)> = r.skipped.iter().map(|l| (l.item.as_str(), l.path.as_str())).collect();
    assert_eq!(skipped, [(cleaner::CATEGORIES[3].name, "")]);
    assert_eq!(r.notes.iter().map(|n| n.tone).collect::<Vec<_>>(), [Tone::Warn]);
    let items: Vec<(&str, u64, u64, u64, Option<&str>, Vec<&str>)> =
        r.items.iter().map(|i| (i.name.as_str(), i.freed_bytes, i.removed, i.left, i.note.as_deref(), i.lines.iter().map(String::as_str).collect())).collect();
    assert_eq!(items, [
        (cleaner::CATEGORIES[1].name, 100, 4, 1, None, vec![]),
        ("Clipboard", 0, 1, 0, Some("a note"), vec!["Clipboard emptied", "a note"]),
        ("Toolchain – History", 500, 12, 2, Some(r"Registry backup: C:\b\x.reg"), vec![r"Registry backup: C:\b\x.reg"]),
    ]);
    assert!(r.can_save && r.saved.is_none());
    assert!(st.report_text().unwrap().contains("Cleanup report"));
    st.set_saved("Saved.".into(), true);
    assert_eq!(view(&st, false).report.unwrap().saved, Some(SavedView { text: "Saved.".into(), ok: true }));
    // After the report: nothing to clean, Analyze allowed; the figures of the analysis are gone.
    assert!(!action(&v, "clean").enabled && action(&v, "analyze").enabled && v.job.is_none());
    assert!(v.rows.iter().all(|r| r.cells.is_empty() && r.bytes.is_none()));

    // Cancelled: the same report, flagged, with no list left to choose from and Clean off.
    let mut st = state_with(HOLD);
    st.analyze(notifier()).unwrap();
    st.cancel_job().unwrap();
    finish(&mut st);
    assert_eq!(st.clean_end, Some(CleanEnd::Cancelled));
    st.clean_sel[1] = true;
    let (tx, rx) = std::sync::mpsc::channel();
    st.clean_job = Some(rx);
    st.clean_view = CleanView::Cleaned;
    st.clean_chosen = Chosen { cats: vec![1], others: vec![] };
    st.clean_cancel = Some((xiox_cleaner::Cancel::default(), Arc::new(AtomicBool::new(false))));
    st.cancel_job().unwrap();
    assert_eq!(view(&st, false).status.badge, "Cancelling");
    assert_eq!(view(&st, false).job.unwrap().label, "Cancelling…");
    let cancelling = view(&st, false);
    assert_eq!((action(&cancelling, "cancel").label.as_str(), action(&cancelling, "cancel").enabled), ("Cancelling…", false));
    tx.send(CleanMsg::Done).unwrap();
    st.poll();
    let v = view(&st, false);
    let r = v.report.as_ref().unwrap();
    assert_eq!((r.title.as_str(), r.cancelled, v.status.badge.as_str()), ("Cleanup cancelled", true, "Cleanup cancelled"));
    assert!(v.rows.is_empty() && v.empty.is_none(), "the report fills the place of the list");
    assert!(!action(&v, "clean").enabled && action(&v, "analyze").enabled);
    assert_eq!(r.notes[0].tone, Tone::Info);
    assert!(r.skipped.iter().any(|l| l.item == cleaner::CATEGORIES[1].name), "the category that never ran is skipped");

    // A worker that died: the report says so, and it is not called cancelled.
    let mut st = state();
    let (tx, rx) = std::sync::mpsc::channel::<CleanMsg>();
    st.clean_job = Some(rx);
    st.clean_view = CleanView::Cleaned;
    drop(tx);
    st.poll();
    let v = view(&st, false);
    assert_eq!(v.status.badge, "Cleanup stopped");
    assert!(v.report.as_ref().unwrap().notes.iter().any(|n| n.tone == Tone::Error));
}

#[test]
fn cancel_and_release_follow_the_job() {
    let mut st = state_with(HOLD);
    assert_eq!(err(st.cancel_job()), NOTHING_TO_CANCEL);
    st.analyze(notifier()).unwrap();
    let v = view(&st, false);
    assert_eq!((v.status.badge.as_str(), v.job.as_ref().unwrap().label.as_str()), ("Analyzing", "Analyzing…"));
    assert!(action(&v, "cancel").enabled && action(&v, "cancel").key.as_deref() == Some("Esc"));
    st.cancel_job().unwrap();
    assert_eq!(view(&st, false).status.badge, "Cancelling");
    finish(&mut st);
    // The analysis that was cut short says so, as an error notice.
    let v = view(&st, false);
    assert_eq!(st.clean_end, Some(CleanEnd::Cancelled));
    assert_eq!(v.status.badge, analysis_title(false, st.clean_end).0);
    assert_eq!(v.notices.len(), 1);
    assert_eq!((v.notices[0].tone, v.notices[0].text.as_str()), (Tone::Error, analysis_title(false, st.clean_end).1.unwrap()));
    assert!(v.actions.iter().all(|a| a.id != "cancel"));
    // Nothing to release while a job runs; after it the section is empty, the choices stay.
    let mut st = state_with(HOLD);
    st.clean_sel[2] = false;
    st.analyze(notifier()).unwrap();
    st.release();
    assert!(st.busy() && st.clean_view == CleanView::Analyzed);
    st.cancel_job().unwrap();
    finish(&mut st);
    st.release();
    assert_eq!((st.clean_view == CleanView::Empty, st.clean_end, st.found.iter().flatten().count()), (true, None, 0));
    assert!(!st.clean_sel[2]);
}

#[test]
fn keep_list_belongs_to_the_cookie_item() {
    let mut st = state();
    assert_eq!(err(st.open_keep_list()), NO_KEEP_LIST, "refused without the item, before anything starts");
    let mut cookies = st.natives[0];
    cookies.id = browserdb::COOKIES_ID;
    cookies.group = "Browser";
    st.natives.push(cookies);
    let ids = |v: &SectionView| v.options.iter().filter_map(|o| match o {
        OptionView::Button { id, label, enabled, hint } => Some((id.clone(), label.clone(), *enabled, hint.is_some())),
        _ => None,
    }).collect::<Vec<_>>();
    // The button is in the view whatever part of the list is on screen: no category is open any more.
    assert_eq!(ids(&view(&st, false)), [("keepList".to_string(), "Edit cookie keep list".to_string(), true, true)]);
    st.set_search("zzz");
    assert_eq!(ids(&view(&st, false)), [("keepList".to_string(), "Edit cookie keep list".to_string(), true, true)], "a search does not take it away");
}

// ---------- one list, every category ----------

/// The ids of the built-in categories of a group, as the list keys them.
fn builtin_ids(group: &str) -> Vec<String> {
    (0..cleaner::CATEGORIES.len()).filter(|&i| cleaner::CATEGORIES[i].group == group).map(|i| format!("cat:{i}")).collect()
}

#[test]
fn every_category_is_in_the_list_led_by_a_group_row() {
    let st = state();
    let v = view(&st, false);
    // The menu and the list name the same categories in the same order; the first row is a category.
    let groups: Vec<(&str, &str)> = v.rows.iter().filter(|r| r.kind == RowKind::Group).map(|r| (r.id.as_str(), r.name.as_str())).collect();
    assert_eq!(groups, v.nav.iter().map(|n| (n.id.as_str(), n.name.as_str())).collect::<Vec<_>>());
    assert_eq!(groups.len(), 6);
    assert_eq!(v.rows[0].id, "builtin:System");
    assert!(v.nav.iter().all(|n| !n.active), "no category is open: the page follows the scroll");
    assert_eq!(rows_of(&v, "builtin:System"), builtin_ids("System"));
    assert_eq!(rows_of(&v, "builtin:Explorer"), builtin_ids("Explorer"));
    assert_eq!(rows_of(&v, "builtin:Advanced"), builtin_ids("Advanced"));
    assert_eq!(rows_of(&v, "winapp2"), ["section:Browsers", "w_gamma/cache", "section:Utilities", "w_alpha/cache", "w_beta/logs", "w_beta/temp"]);
    // The line of a category carries no description, level or detail: it is a heading with a box.
    let g = row(&v, "builtin:System");
    assert_eq!((g.kind, g.parent.as_deref(), g.sub.as_deref(), g.level, g.muted), (RowKind::Group, None, None, None, false));
    assert!(g.detail.is_empty() && g.lines.is_empty() && g.size_text.is_none());
    // Two groups side by side keep their own rows: the same list holds them all.
    assert!(v.rows.iter().any(|r| r.id == "cat:0") && v.rows.iter().any(|r| r.id == "native:clipboard") && v.rows.iter().any(|r| r.id == "w_beta/temp"));
    // Rules not loaded: the groups of the natives are there, Winapp2 is not.
    let mut st = state();
    st.rules = None;
    let v = view(&st, false);
    assert_eq!(v.rows.iter().filter(|r| r.kind == RowKind::Group).map(|r| r.id.as_str()).collect::<Vec<_>>(), ["builtin:System", "builtin:Explorer", "builtin:Advanced", "rules:Windows"]);
}

#[test]
fn a_category_line_says_what_its_box_ticks() {
    let mut st = state();
    let v = view(&st, false);
    // System is all default-on, Explorer all off.
    let system = row(&v, "builtin:System");
    assert_eq!((system.check, system.enabled), (Check::On, true));
    assert_eq!(row(&v, "builtin:Explorer").check, Check::Off);
    // Windows holds a ticked native and a Caution one: its box looks at the first only, and says so.
    let windows = row(&v, "rules:Windows");
    assert_eq!((windows.check, windows.enabled), (Check::On, true));
    assert_eq!(windows.why.as_deref(), Some("Caution items are not ticked by this box: tick them one by one."));
    assert_eq!(row(&v, "builtin:System").why, None, "no Caution row, nothing to say");
    // Ticking the Caution row by hand moves nothing on the box; unticking the other makes it Off.
    st.toggle("native:env_path").unwrap();
    assert_eq!(row(&view(&st, false), "rules:Windows").check, Check::On);
    st.toggle("native:clipboard").unwrap();
    assert_eq!(row(&view(&st, false), "rules:Windows").check, Check::Off);
    // Some of them: Mixed.
    st.toggle("cat:0").unwrap();
    assert_eq!(row(&view(&st, false), "builtin:System").check, Check::Mixed);
    // A category with nothing the box can tick (a Caution row and an unsupported one): no box.
    let dev = row(&v, "rules:Development & AI");
    assert_eq!((dev.check, dev.enabled), (Check::None, false));
    assert!(dev.why.is_some());
    // Winapp2's sub-headings have a box over their own rows (nothing of theirs is ticked here); so does its category.
    assert_eq!(row(&v, "section:Browsers").check, Check::Off);
    assert_eq!(row(&v, "winapp2").check, Check::Off);

    // Figures appear once the analysis has answered there: files and bytes of the rows.
    let mut st = state();
    assert!(row(&view(&st, false), "builtin:System").cells.is_empty() && row(&view(&st, false), "builtin:System").bytes.is_none());
    st.clean_view = CleanView::Analyzed;
    st.found[0] = Some(cleaner::Found { count: 5, bytes: 1000, ..Default::default() });
    st.found[1] = Some(cleaner::Found { count: 2_000, bytes: 24, ..Default::default() });
    let v = view(&st, false);
    let system = row(&v, "builtin:System");
    assert_eq!((system.cells.as_slice(), system.bytes), (&["2,005".to_string()][..], Some(1024)));
    let explorer = row(&v, "builtin:Explorer");
    assert!(explorer.cells.is_empty() && explorer.bytes.is_none(), "nothing answered there yet");
    // A cleaning drops the figures: the report that follows is not an analysis.
    st.clean_view = CleanView::Cleaned;
    assert!(row(&view(&st, false), "builtin:System").bytes.is_none());
}

#[test]
fn a_category_box_ticks_what_it_can_and_never_a_caution_or_blocked_row() {
    let mut st = state();
    // Windows: the clipboard is on, the PATH item (Caution) is off. The box is On, so a click clears the ticked ones.
    st.toggle("rules:Windows").unwrap();
    assert!(!st.rule_sel.contains("native:clipboard") && !st.rule_sel.contains("native:env_path"));
    // Off: a click ticks the one it can, never the Caution row.
    st.toggle("rules:Windows").unwrap();
    assert!(st.rule_sel.contains("native:clipboard"));
    assert!(!st.rule_sel.contains("native:env_path"), "a Caution row is ticked one by one");
    // A Caution row ticked by hand stays when the box clears the others.
    st.toggle("native:env_path").unwrap();
    st.toggle("rules:Windows").unwrap();
    assert!(st.rule_sel.contains("native:env_path") && !st.rule_sel.contains("native:clipboard"));
    // Mixed: a click ticks them all.
    st.toggle("cat:0").unwrap();
    assert_eq!(row(&view(&st, false), "builtin:System").check, Check::Mixed);
    st.toggle("builtin:System").unwrap();
    assert!(builtin_ids("System").iter().all(|id| st.clean_sel[id[4..].parse::<usize>().unwrap()]));
    assert_eq!(row(&view(&st, false), "builtin:System").check, Check::On);
    st.toggle("builtin:System").unwrap();
    assert!(builtin_ids("System").iter().all(|id| !st.clean_sel[id[4..].parse::<usize>().unwrap()]));
    // The other categories did not move.
    assert!(st.rule_sel.contains("native:env_path"));

    // Nothing in Development & AI can be ticked in one go (Caution and unsupported): the box is refused.
    assert_eq!(err(st.toggle("rules:Development & AI")), NOTHING_TO_TICK);
    assert!(!st.rule_sel.contains("t_tool/history") && !st.rule_sel.contains("t_tool/legacy"));

    // A blocked option (its program runs) is left alone by the Winapp2 box.
    let mut rs = rule_set();
    blocker(&mut rs);
    st.rules = Some(rs);
    st.processes = vec!["blocker.exe".into()];
    st.toggle("winapp2").unwrap();
    assert_eq!(
        ["w_gamma/cache", "w_beta/logs", "w_beta/temp"].map(|k| st.rule_sel.contains(k)),
        [true, true, true],
        "the options that can run are ticked"
    );
    assert!(!st.rule_sel.contains("w_alpha/cache"), "a blocked option is not");
    let v = view(&st, false);
    assert_eq!((row(&v, "winapp2").check, row(&v, "w_alpha/cache").check, row(&v, "w_alpha/cache").enabled), (Check::On, Check::Off, false));

    // A category the search hides cannot be ticked, and an unknown id is still unknown.
    st.set_search("zzz");
    assert_eq!(err(st.toggle("builtin:System")), CATEGORY_HIDDEN);
    assert!(st.toggle("builtin:Nope").is_err());
}

#[test]
fn select_all_ticks_every_row_it_can_and_none_clears_them() {
    let mut st = state();
    st.select_all(true).unwrap();
    assert!(st.clean_sel.iter().all(|&b| b), "every built-in category");
    for key in ["native:clipboard", "w_alpha/cache", "w_beta/logs", "w_beta/temp", "w_gamma/cache"] {
        assert!(st.rule_sel.contains(key), "{key} is ticked");
    }
    for key in ["native:env_path", "t_tool/history"] {
        assert!(!st.rule_sel.contains(key), "{key} is Caution: never ticked in one go");
    }
    assert!(!st.rule_sel.contains("t_tool/legacy"), "an unsupported option is not either");
    let v = view(&st, false);
    assert!(v.bulk);
    for id in ["builtin:System", "builtin:Explorer", "builtin:Advanced", "rules:Windows", "winapp2"] {
        assert_eq!(row(&v, id).check, Check::On, "{id}");
    }
    assert_eq!(row(&v, "rules:Development & AI").check, Check::None);
    assert_eq!(v.subtitle, format!("17 items · {} selected", v.rows.iter().filter(|r| r.kind == RowKind::Item && r.check == Check::On).count()));

    // A Caution row ticked by hand survives «None»; everything else it could tick goes.
    st.toggle("native:env_path").unwrap();
    st.select_all(false).unwrap();
    assert!(st.clean_sel.iter().all(|&b| !b));
    assert_eq!(st.rule_sel.iter().map(String::as_str).collect::<Vec<_>>(), ["native:env_path"]);
    let v = view(&st, false);
    assert!(["builtin:System", "builtin:Explorer", "builtin:Advanced", "rules:Windows", "winapp2"].iter().all(|id| row(&v, id).check == Check::Off));

    // Only what the list shows: the search keeps the rest as it was.
    st.set_search("beta");
    st.select_all(true).unwrap();
    assert_eq!(st.rule_sel.iter().cloned().collect::<std::collections::BTreeSet<_>>(), ["native:env_path", "w_beta/logs", "w_beta/temp"].map(String::from).into());
    assert!(st.clean_sel.iter().all(|&b| !b));
    // Nothing shown, or nothing that can be ticked: refused, and nothing changes.
    st.set_search("zzz");
    let before = (st.clean_sel.clone(), st.rule_sel.clone());
    assert_eq!(err(st.select_all(true)), NOTHING_TO_TICK);
    assert_eq!((st.clean_sel.clone(), st.rule_sel.clone()), before);
    assert!(!view(&st, false).bulk);
    st.set_search("toolchain");
    assert_eq!(err(st.select_all(true)), NOTHING_TO_TICK, "Caution and unsupported only");
    assert!(!view(&st, false).bulk);
}

#[test]
fn the_search_filters_every_category_and_leaves_the_menu_alone() {
    let mut st = state();
    // By name: only the categories that have a matching row are in the list, each with its matching rows.
    st.set_search("toolchain");
    let v = view(&st, false);
    assert_eq!(v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["rules:Development & AI", "t_tool/history", "t_tool/legacy"]);
    assert_eq!(v.nav.len(), 6, "the menu still lists every category");
    assert_eq!(v.subtitle, "2 items · 0 selected");
    assert!(v.empty.is_none());
    // By description (any case, trimmed): the unsupported option is described as «Old files.».
    st.set_search("  OLD FILES ");
    let v = view(&st, false);
    assert_eq!(v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["rules:Development & AI", "t_tool/legacy"]);
    // A built-in category is found by its name too.
    st.set_search(cleaner::CATEGORIES[0].name);
    let v = view(&st, false);
    assert_eq!(v.rows.first().map(|r| r.id.as_str()), Some("builtin:System"));
    assert!(v.rows.iter().any(|r| r.id == "cat:0"));
    // No filter: everything again.
    st.set_search("");
    assert_eq!(view(&st, false).rows.iter().filter(|r| r.kind == RowKind::Group).count(), 6);
    // The boxes follow the rows that are shown: the box of a category with some rows hidden reads its shown ones.
    st.toggle("cat:0").unwrap();
    st.set_search(cleaner::CATEGORIES[0].name);
    let v = view(&st, false);
    assert_eq!(row(&v, "builtin:System").check, Check::Off, "the only shown row is off");
}

#[test]
fn the_list_boxes_and_the_search_wait_while_a_job_runs_or_a_report_shows() {
    let mut st = state_with(HOLD);
    st.analyze(notifier()).unwrap();
    let v = view(&st, false);
    assert!(!v.bulk && v.search.is_none());
    assert!(v.rows.iter().filter(|r| r.kind == RowKind::Group).all(|r| !r.enabled));
    st.cancel_job().unwrap();
    finish(&mut st);
    let v = view(&st, false);
    assert!(v.bulk && v.search.is_some(), "back after the job");
    assert!(v.rows.iter().filter(|r| r.kind == RowKind::Group && r.check != Check::None).all(|r| r.enabled));
    // A report on screen replaces the list: no box, no search.
    st.clean_view = CleanView::Cleaned;
    st.clean_report = Some(Default::default());
    let v = view(&st, false);
    assert!(!v.bulk && v.search.is_none() && v.report.is_some());
}

// ---------- Winapp2 ----------

#[test]
fn winapp2_search_matches_section_cleaner_or_option() {
    let rs = rule_set();
    let m = |needle: &str| winapp2_matches(&rs, needle);
    let all = vec![("Browsers".to_string(), vec![3]), ("Utilities".to_string(), vec![1, 2])];
    assert_eq!(m(""), all);
    assert_eq!(m("   "), all, "blank is no filter");
    assert_eq!(m("UTIL"), [("Utilities".to_string(), vec![1, 2])], "a section name takes the whole section");
    assert_eq!(m("gamma"), [("Browsers".to_string(), vec![3])], "a cleaner name");
    assert_eq!(m(" Logs "), [("Utilities".to_string(), vec![2])], "an option label, any case, trimmed");
    assert!(m("zzz").is_empty());

    // In the view: the Winapp2 category is the last of the list, a header per section with the number
    // of cleaners, its options after it, and the status line of Winapp2 with its reasons.
    let mut st = state();
    st.winapp2_update = Some(xiox_rules::updater::UpdateOutcome::NoCopyYet("rete assente".into()));
    let v = view(&st, false);
    assert_eq!(v.title, "Cleanup");
    assert_eq!(v.search, Some(SearchView { placeholder: "Search all categories".into(), value: String::new() }));
    let last = v.rows.iter().position(|r| r.id == "winapp2").expect("the Winapp2 category");
    let shown: Vec<(&str, RowKind, &str)> = v.rows[last..].iter().map(|r| (r.id.as_str(), r.kind, r.name.as_str())).collect();
    assert_eq!(shown, [
        ("winapp2", RowKind::Group, rules::WINAPP2_GROUP),
        ("section:Browsers", RowKind::Header, "Browsers (1)"),
        ("w_gamma/cache", RowKind::Item, "Gamma"),
        ("section:Utilities", RowKind::Header, "Utilities (2)"),
        ("w_alpha/cache", RowKind::Item, "Alpha App"),
        ("w_beta/logs", RowKind::Item, "Beta Tool – Logs"),
        ("w_beta/temp", RowKind::Item, "Beta Tool – Temp"),
    ]);
    assert_ne!(row(&v, "section:Utilities").check, Check::None, "a section of options that can be ticked has a box");
    let (text, why) = rules::status_line(st.rules.as_ref().unwrap(), st.winapp2_update.as_ref());
    let status = v.options.iter().find_map(|o| match o {
        OptionView::Text { text, hint, .. } => Some((text.clone(), hint.clone())),
        _ => None,
    });
    assert_eq!(status, Some((text, Some(format!("{}\nWinapp2: 2 regole scartate", why.unwrap())))));
    // The search keeps the Winapp2 sections as before (here: a cleaner's label), now across every category.
    st.set_search("beta");
    let v = view(&st, false);
    assert_eq!(v.search.as_ref().unwrap().value, "beta");
    assert_eq!(v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["winapp2", "section:Utilities", "w_beta/logs", "w_beta/temp"]);
    assert_eq!(row(&v, "section:Utilities").name, "Utilities (1)");
    // A section's box ticks the rows the search shows, and clears them once they are all ticked; a section the search
    // hides cannot be ticked.
    let alpha = st.rule_sel.contains("w_alpha/cache");
    st.toggle("section:Utilities").unwrap();
    assert!(st.rule_sel.contains("w_beta/logs") && st.rule_sel.contains("w_beta/temp"));
    assert_eq!(st.rule_sel.contains("w_alpha/cache"), alpha, "hidden by the search: left as it was");
    assert_eq!(row(&view(&st, false), "section:Utilities").check, Check::On);
    st.toggle("section:Utilities").unwrap();
    assert!(!st.rule_sel.contains("w_beta/logs") && !st.rule_sel.contains("w_beta/temp"));
    assert_eq!(row(&view(&st, false), "section:Utilities").check, Check::Off);
    assert_eq!(err(st.toggle("section:Browsers")), CATEGORY_HIDDEN);
    // Rows are choosable one by one too, and a search that finds nothing says so.
    st.toggle("w_beta/logs").unwrap();
    assert_eq!(row(&view(&st, false), "w_beta/logs").check, Check::On);
    st.set_search("zzz");
    let v = view(&st, false);
    assert!(v.rows.is_empty() && !v.bulk);
    assert_eq!(v.empty, Some(EmptyView { title: "No matches".into(), text: "Nothing matches “zzz”.".into() }));
    assert_eq!(v.nav.len(), 6, "the menu is not filtered");
    // Rules not loaded yet: no Winapp2 entry (and no status line), the natives are there already.
    st.set_search("");
    st.rules = None;
    let v = view(&st, false);
    assert_eq!(v.nav.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["builtin:System", "builtin:Explorer", "builtin:Advanced", "rules:Windows"]);
    assert!(v.rows.iter().all(|r| r.id != "winapp2") && v.options.iter().all(|o| !matches!(o, OptionView::Text { .. })));
}

// ---------- «Save as text…» ----------

/// The outcome of a save lands on the report that was written and on no other: if the report on
/// screen changed while the dialog was open, the person gets the current view and no notice. The
/// comparison and the write are one call, so they happen under one lock.
#[test]
fn a_saved_notice_lands_only_on_the_report_that_was_written() {
    use crate::app::{Core, CoreOptions};
    let mut st = state();
    st.clean_view = CleanView::Cleaned;
    st.clean_report = Some(Default::default());
    let tmp = tempfile::tempdir().unwrap();
    let options = CoreOptions { version: "t".into(), admin: false, only_root: None, load_rules: false, update_rules: false };
    let dev = crate::dev::DevState::only_root(tmp.path().to_path_buf(), crate::dev::Section::Projects);
    let mut core = Core::with_states(
        options,
        std::sync::Arc::new(crate::notify::NoNotifier),
        st,
        crate::regstate::RegistryState::new(),
        dev,
    );
    let written = core.report_text(SectionId::Cleanup).expect("a report on screen can be saved");

    let v = core.set_saved_if(SectionId::Cleanup, "the text of another report", "Saved to x".into(), true);
    assert!(v.report.expect("the report is still shown").saved.is_none(), "a notice for another report is dropped");
    let v = core.set_saved_if(SectionId::Cleanup, &written, "Saved to y".into(), true);
    assert_eq!(v.report.unwrap().saved, Some(SavedView { text: "Saved to y".into(), ok: true }));
    // No report in the section: nothing to put a notice on.
    let v = core.set_saved_if(SectionId::Registry, &written, "Saved to z".into(), true);
    assert!(v.report.is_none());
}

// ---------- counts the user reads ----------

/// Every file count that reaches the page is grouped with commas (`18,204`) and the word agrees
/// with it (`1 file`, `0`, `2 items`): in the list cell, the detail box, the status line, the
/// nav note, the subtitle and the Clean button. Sizes keep their own format.
#[test]
fn cleanup_counts_are_grouped_and_agree_with_their_word() {
    let mut st = state();
    st.clean_view = CleanView::Analyzed;
    st.found[0] = Some(cleaner::Found { count: 18_204, bytes: 1_536_000_000, ..Default::default() });
    st.found[1] = Some(cleaner::Found { count: 1, bytes: 10, ..Default::default() });
    let v = view(&st, false);
    let bin = row(&v, "cat:0");
    assert_eq!(bin.cells, ["18,204".to_string()]);
    assert_eq!(detail(bin, "files"), Some("18,204"));
    assert_eq!(detail(bin, "size"), Some(sys::fmt_size(1_536_000_000).as_str()));
    assert_eq!(v.status.text, format!("{} to delete (18,205 files)", sys::fmt_size(1_536_000_010)));

    // One file: the singular.
    let mut one = state();
    one.clean_view = CleanView::Analyzed;
    one.found[0] = Some(cleaner::Found { count: 1, bytes: 5, ..Default::default() });
    assert_eq!(view(&one, false).status.text, format!("{} to delete (1 file)", sys::fmt_size(5)));
    // Nothing found: zero files, plural.
    let mut none = state();
    none.clean_view = CleanView::Analyzed;
    none.found[0] = Some(cleaner::Found::default());
    assert_eq!(view(&none, false).status.text, format!("{} to delete (0 files)", sys::fmt_size(0)));

    // The words of the nav note and the subtitle.
    let v = view(&state(), false);
    let items = v.rows.iter().filter(|r| r.kind == RowKind::Item).count();
    let ticked = v.rows.iter().filter(|r| r.kind == RowKind::Item && r.check == Check::On).count();
    assert_eq!(v.subtitle, format!("{} · {ticked} selected", sys::plural(items, "item", "items")));
    let one_row = sys::plural(1usize, "item", "items");
    assert_eq!(one_row, "1 item");
    assert!(v.nav.iter().all(|n| n.note.as_deref().is_some_and(|t| t.contains(" selected"))));
}

// ---------- choices kept between starts ----------

use xiox_app::prefs::{Store, CLEANUP};

fn category(name: &str) -> usize {
    cleaner::CATEGORIES.iter().position(|c| c.name == name).expect("a built-in category")
}

fn defaults() -> CleanupState {
    CleanupState::with(natives_for_test(), vec![], QUIET)
}

/// What pref.json brings back, over the defaults: categories by name (an unknown one is ignored), items by key, and a file
/// or a section that is missing, broken or empty leaves the defaults as they are.
#[test]
fn a_saved_file_is_applied_over_the_defaults() {
    let thumbs = category("Thumbnail cache");
    let bin = category("Recycle Bin");
    struct Case {
        name: &'static str,
        /// `None`: no file at all.
        json: Option<&'static str>,
        cats: Vec<(usize, bool)>,
        items: Vec<(&'static str, bool)>,
        only_old: bool,
    }
    let cases = [
        Case { name: "missing", json: None, cats: vec![], items: vec![], only_old: true },
        Case { name: "broken", json: Some("{ not json"), cats: vec![], items: vec![], only_old: true },
        Case { name: "no cleanup section", json: Some(r#"{"developers": {}}"#), cats: vec![], items: vec![], only_old: true },
        Case { name: "empty section", json: Some(r#"{"cleanup": {}}"#), cats: vec![], items: vec![], only_old: true },
        Case {
            name: "by name, unknown ignored",
            json: Some(r#"{"cleanup": {"categories": {"Thumbnail cache": true, "Recycle Bin": false, "A category that left": true}}}"#),
            cats: vec![(thumbs, true), (bin, false)],
            items: vec![],
            only_old: true,
        },
        Case {
            name: "items on and off, only_old",
            json: Some(r#"{"cleanup": {"items": {"w_alpha/cache": true, "native:env_path": false}, "onlyOld": false}}"#),
            cats: vec![],
            items: vec![("w_alpha/cache", true), ("native:env_path", false)],
            only_old: false,
        },
    ];
    for case in cases {
        let name = case.name;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pref.json");
        if let Some(json) = case.json {
            std::fs::write(&path, json).unwrap();
        }
        let st = defaults().persisted(Store::at(path));
        let mut sel = defaults().clean_sel;
        for (i, on) in case.cats {
            sel[i] = on;
        }
        assert_eq!(st.clean_sel, sel, "{name}: categories");
        for (key, on) in case.items {
            assert_eq!(st.rule_sel.contains(key), on, "{name}: {key}");
        }
        assert_eq!(st.only_old, case.only_old, "{name}: only_old");
    }
}

/// Only the differences from the defaults are written: a category or a native a later version adds
/// is not in the file, so it starts at its own default.
#[test]
fn only_what_differs_from_the_defaults_is_written() {
    let st = defaults();
    assert_eq!(prefs::Prefs::capture(&st.clean_sel, &st.rule_sel, &st.natives, st.only_old), prefs::Prefs::default());
}

/// Every intent that changes a choice writes it, and the next start (whose rules have not loaded yet)
/// gets the same choices back; a change made before the rules load keeps the options it cannot see,
/// and the other sections of pref.json stay as they were.
#[test]
fn choices_come_back_at_the_next_start() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("pref.json");
    std::fs::write(&path, r#"{"windowTheme": "dark"}"#).unwrap();
    let mut st = state().persisted(Store::at(path.clone()));
    let thumbs = category("Thumbnail cache");
    st.toggle(&format!("cat:{thumbs}")).unwrap();
    st.toggle("native:clipboard").unwrap();
    st.toggle("w_alpha/cache").unwrap();
    st.set_only_old(false).unwrap();
    assert!(st.store_err.is_none());

    let mut next = defaults().persisted(Store::at(path.clone()));
    assert!(next.rules.is_none(), "Winapp2 is not loaded yet");
    assert_eq!(next.clean_sel, st.clean_sel);
    assert_eq!(next.rule_sel, st.rule_sel);
    assert!(!next.only_old);

    next.toggle(&format!("cat:{thumbs}")).unwrap();
    let saved: prefs::Prefs = Store::at(path.clone()).read(CLEANUP).unwrap();
    assert_eq!(saved.items.get("w_alpha/cache"), Some(&true), "an option of rules not loaded yet stays chosen");
    assert!(!saved.categories.contains_key("Thumbnail cache"), "back to its default: nothing to keep");
    assert_eq!(Store::at(path).read::<String>("windowTheme").as_deref(), Some("dark"), "another section is untouched");
}

/// A refused intent writes nothing, and a state made without a store never writes.
#[test]
fn a_refused_intent_or_a_state_without_a_store_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("pref.json");
    let mut st = state().persisted(Store::at(path.clone()));
    st.confirm_clean = true;
    assert!(st.toggle("w_alpha/cache").is_err());
    assert!(!path.exists());

    let mut memory = state();
    memory.toggle("w_alpha/cache").unwrap();
    assert!(memory.store.is_none() && memory.store_err.is_none());
}

/// A file that cannot be written says so under the options; the choice is still made.
#[test]
fn a_write_that_fails_is_shown_under_the_options() {
    let tmp = tempfile::tempdir().unwrap();
    // A folder where the file should be: it cannot be read nor written.
    let mut st = state().persisted(Store::at(tmp.path().to_path_buf()));
    st.toggle("w_alpha/cache").unwrap();
    assert!(st.rule_sel.contains("w_alpha/cache"));
    let v = st.view(false);
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Text { text, tone: Tone::Error, .. } if text.starts_with("Choices not saved: "))));
}
