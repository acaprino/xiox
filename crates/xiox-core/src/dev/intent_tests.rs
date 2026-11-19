//! Intent tests: what `DevState` refuses, what it lets through, and what the view shows.
//! Workers are test functions behind `DevWorkers`: nothing here reaches the engine or the disk.
//! (The fixtures below copy the helpers of `tests.rs`, which are private to that module.)

use super::*;
use crate::view::testing::{action, button, err, notifier, row};
use crate::view::*;
use std::path::PathBuf;
use std::sync::Mutex;
use xiox_model::{Artifact, ArtifactKind, GitState, SizeInfo};

fn report(id: &str, status: ProjectStatus, preselected: bool, bytes: u64) -> ProjectReport {
    ProjectReport {
        id: id.into(),
        root: PathBuf::from(format!(r"D:\p\{id}")),
        name: id.into(),
        ecosystems: vec![],
        agents: vec![],
        last_activity_ms: None,
        last_activity_source: None,
        inactive_days: None,
        status,
        blocked_by: vec![],
        reason: String::new(),
        artifacts: vec![Artifact {
            id: xiox_model::path_key(&PathBuf::from(format!(r"D:\p\{id}\target"))),
            path: PathBuf::from(format!(r"D:\p\{id}\target")),
            kind: ArtifactKind::RustTarget,
            label: "Build Rust (target)".into(),
            size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
            verified: true,
            class: xiox_model::Class::Build,
            level: xiox_model::Level::Recommended,
            note: String::new(),
            ambiguous: false,
            preselected,
        }],
        reclaimable_bytes: bytes,
        preselected,
        is_agent_worktree: false,
        git: GitState::default(),
    }
}


fn tid(id: &str) -> String {
    xiox_model::path_key(&PathBuf::from(format!(r"D:\p\{id}\target")))
}

fn ids(list: &[&str]) -> HashSet<String> {
    list.iter().map(|id| tid(id)).collect()
}

fn session(id: &str, blocked: Option<&str>, file: bool, bytes: u64) -> ArchivedSession {
    ArchivedSession {
        agent: Agent::Codex,
        id: id.into(),
        title: Some(format!("  titolo\n{id}  ")),
        cwd: Some(PathBuf::from(r"D:\Projects\demo")),
        updated_ms: None,
        file: file.then(|| PathBuf::from(format!(r"C:\s\{id}.jsonl"))),
        bytes,
        blocked: blocked.map(str::to_string),
    }
}


/// Un progetto (più uno bloccato), un worktree d'agente e due sessioni.
pub(crate) fn three_sections() -> DevScan {
    let mut wt = report("w", ProjectStatus::Recent, false, 30);
    wt.is_agent_worktree = true;
    DevScan {
        projects: vec![report("a", ProjectStatus::Inactive, true, 100), report("b", ProjectStatus::Blocked, false, 50)],
        worktrees: vec![wt],
        archived: vec![session("1", None, false, 0), session("2", None, true, 4)],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    }
}


/// Un progetto inattivo con tre cartelle: target consigliato e preselezionato,
/// node_modules facoltativo, dist al livello Attenzione.
fn levels_project() -> ProjectReport {
    let mut r = report("web", ProjectStatus::Inactive, true, 100);
    let base = r.artifacts[0].clone();
    let mk = |dir: &str, label: &str, class, level, note: &str, bytes| {
        let path = PathBuf::from(format!(r"D:\p\web\{dir}"));
        Artifact {
            id: path_key(&path),
            path,
            label: label.into(),
            class,
            level,
            note: note.into(),
            preselected: false,
            size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
            ..base.clone()
        }
    };
    r.artifacts.push(mk(
        "node_modules",
        "Node dependencies (node_modules)",
        xiox_model::Class::Dependency,
        Level::Optional,
        "Download again with npm install (or pnpm, yarn, bun install)",
        50,
    ));
    r.artifacts.push(mk(
        "dist",
        "Build JavaScript (dist)",
        xiox_model::Class::Build,
        Level::Caution,
        "May contain sources: git does not ignore it",
        20,
    ));
    r.artifacts[1].kind = ArtifactKind::NodeModules;
    r.artifacts[2].kind = ArtifactKind::Catalog;
    r.reclaimable_bytes = 170;
    r
}

// ---------- worker seam ----------

fn scan_none(_: Settings, _: bool, _: Outbox<DevMsg>) {}

/// What the recording clean worker was asked to do: (mode, plan items).
static CALLS: Mutex<Vec<(RemoveMode, usize)>> = Mutex::new(Vec::new());
/// The tests that clear and read `CALLS` run one at a time.
static CALLS_SERIAL: Mutex<()> = Mutex::new(());

fn clean_recording(job: DevCleanJob, out: Outbox<DevMsg>) {
    CALLS.lock().unwrap().push((job.mode, job.plan.as_ref().map_or(0, |p| p.items.len())));
    out.send(DevMsg::Cleaned(DevCleaned::new()));
}

fn clean_quiet(_: DevCleanJob, out: Outbox<DevMsg>) {
    out.send(DevMsg::Cleaned(DevCleaned::new()));
}

/// A cleanup that runs until the shared `Cancel` flag of its job is raised, then answers like the
/// engine does: with a report marked cancelled. (Gives up after five seconds.)
fn clean_until_cancelled(job: DevCleanJob, out: Outbox<DevMsg>) {
    for _ in 0..500 {
        if job.cancel.is_cancelled() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut done = DevCleaned::new();
    done.report.cancelled = job.cancel.is_cancelled();
    done.report.files = 3;
    done.report.freed_bytes = 12;
    out.send(DevMsg::Cleaned(done));
}

const TEST_WORKERS: DevWorkers = DevWorkers { scan: scan_none, clean: clean_recording };

/// A state whose workers never reach the engine. Tests that change settings use `persist = false`.
fn state() -> DevState {
    DevState::with_settings(xiox_app::default_settings(), Section::Projects).with_workers(TEST_WORKERS)
}

/// `set_scan` empties a scan when `persist` is false and there is no single root, so the scan goes
/// in first and the state stops saving afterwards: no test here can write the real settings file.
fn with_scan(scan: DevScan) -> DevState {
    let mut st = state();
    st.set_scan(scan);
    st.persist = false;
    st
}

fn scanned() -> DevState {
    with_scan(three_sections())
}

fn levels_state() -> DevState {
    with_scan(DevScan { projects: vec![levels_project()], scanned_at_ms: xiox_app::now_ms(), ..Default::default() })
}

fn dist_id() -> String {
    tid("web").replace("target", "dist")
}

/// Waits for the job to land (polling, like the pump does).
fn finish(st: &mut DevState) {
    for _ in 0..500 {
        st.poll();
        if !st.busy() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the worker never answered");
}

// ---------- selection ----------

#[test]
fn toggles_follow_the_tables() {
    let mut scan = three_sections();
    // A blocked session next to the free ones: Claude's «1» (the codex «1» is free).
    scan.archived.push(ArchivedSession { agent: Agent::Claude, ..session("1", Some("session in use"), true, 9) });
    let mut st = with_scan(scan);
    assert_eq!(st.sel_artifacts, ids(&["a"]), "the engine preselects the free project's folder");

    // A project the table greys out (blocked): neither its box nor its folders move.
    assert_eq!(err(st.toggle("p:b")), locked_why(&st.scan.as_ref().unwrap().projects[1]));
    assert!(st.toggle(&format!("a:{}", tid("b"))).is_err());
    assert_eq!(st.sel_artifacts, ids(&["a"]));

    // A free project: «all» goes to none and back; so does one of its folders.
    st.toggle("p:a").unwrap();
    assert!(st.sel_artifacts.is_empty());
    st.toggle("p:a").unwrap();
    assert_eq!(st.sel_artifacts, ids(&["a"]));
    st.toggle(&format!("a:{}", tid("a"))).unwrap();
    assert!(st.sel_artifacts.is_empty());
    st.toggle(&format!("a:{}", tid("a"))).unwrap();
    assert_eq!(st.sel_artifacts, ids(&["a"]));

    // A worktree that can be removed.
    assert_eq!(st.sel_worktrees, vec![false]);
    st.toggle("w:w").unwrap();
    assert_eq!(st.sel_worktrees, vec![true]);
    st.toggle("w:w").unwrap();
    assert_eq!(st.sel_worktrees, vec![false]);

    // Sessions: the blocked one is refused with its own reason, the free ones flip.
    assert_eq!(err(st.toggle("s:claude:1")), "session in use");
    assert_eq!(st.sel_sessions, vec![false, false, false]);
    st.toggle("s:codex:2").unwrap();
    assert_eq!(st.sel_sessions, vec![false, true, false]);

    // Not rows of a table: the «other worktrees» (information only), the header, the unknown.
    for row in ["o:x", "h:others", "x:1", "p:nope", "a:nope", "w:nope", "s:codex:nope", "s:nobody:1", "s:", "p:", "", "a"] {
        assert!(st.toggle(row).is_err(), "{row} must be refused");
    }
    assert_eq!(st.sel_artifacts, ids(&["a"]));
    assert_eq!(st.sel_sessions, vec![false, true, false]);
}

#[test]
fn a_heading_ticks_the_free_rows_under_it() {
    let mut scan = three_sections();
    scan.archived.push(ArchivedSession { agent: Agent::Claude, ..session("1", Some("session in use"), true, 9) });
    scan.archived.push(ArchivedSession { cwd: None, ..session("3", None, true, 2) });
    let mut st = with_scan(scan);
    st.select_category(DevCategory::Sessions).unwrap();
    let v = st.view(false);
    let demo = r"h:project:D:\Projects\demo";
    assert_eq!(
        v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [demo, "s:codex:1", "s:codex:2", "s:claude:1", "h:project:none", "s:codex:3"],
        "the sessions under their project, those with none last"
    );
    assert_eq!((row(&v, demo).name.as_str(), row(&v, demo).sub.as_deref(), row(&v, demo).check), ("demo (3 sessions)", Some(r"D:\Projects\demo"), Check::Off));
    assert_eq!(row(&v, "h:project:none").name, "No project (1 session)");
    st.toggle(demo).unwrap();
    assert_eq!(st.sel_sessions, vec![true, true, false, false], "the session in use stays, the other project does not move");
    assert_eq!(row(&st.view(false), demo).check, Check::On);
    st.toggle(demo).unwrap();
    assert_eq!(st.sel_sessions, vec![false; 4]);

    // The worktrees under their repository; a heading of another list is not there to click.
    st.select_category(DevCategory::Worktrees).unwrap();
    assert_eq!(row(&st.view(false), "h:repo:none").name, "No main repository (1 worktree)");
    st.toggle("h:repo:none").unwrap();
    assert_eq!(st.sel_worktrees, vec![true]);
    assert_eq!(err(st.toggle(demo)), unknown_row(demo));
}

#[test]
fn projects_sit_in_a_tree_of_their_folders_whose_boxes_tick_the_free_ones_inside() {
    let at = |id: &str, root: &str, status, pre| ProjectReport { root: PathBuf::from(root), ..report(id, status, pre, 10) };
    let scan = DevScan {
        projects: vec![
            at("a", r"D:\Projects\a", ProjectStatus::Inactive, true),
            at("b", r"D:\Projects\clients\b", ProjectStatus::Inactive, false),
            at("c", r"D:\Projects\clients\c", ProjectStatus::Blocked, false),
            // Another spelling of the same folder.
            at("d", r"d:\projects\d", ProjectStatus::Inactive, false),
            at("e", r"C:\code\e", ProjectStatus::Inactive, false),
            // A drive's root has no folder above it.
            at("r", r"D:\", ProjectStatus::Inactive, false),
        ],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    };
    let mut st = with_scan(scan);
    let v = st.view(false);
    let (top, clients, code) = (r"h:folder:D:\Projects", r"h:folder:D:\Projects\clients", r"h:folder:C:\code");
    let tree: Vec<(&str, Option<&str>)> = v.rows.iter().filter(|r| r.kind != RowKind::Child).map(|r| (r.id.as_str(), r.parent.as_deref())).collect();
    assert_eq!(
        tree,
        [
            ("p:r", None),
            (top, None),
            (clients, Some(top)),
            ("p:b", Some(clients)),
            ("p:c", Some(clients)),
            ("p:a", Some(top)),
            ("p:d", Some(top)),
            (code, None),
            ("p:e", Some(code)),
        ],
        "D:\\ and its one folder are one row; the folders before the projects beside them"
    );
    let heading = |v: &SectionView, id: &str| {
        let r = row(v, id);
        (r.name.clone(), r.sub.clone(), r.check)
    };
    assert_eq!(heading(&v, top), (r"D:\Projects".to_string(), None, Check::Mixed));
    assert_eq!(heading(&v, clients), ("clients".to_string(), None, Check::Off), "the blocked project is not in its box");
    assert_eq!(row(&v, &format!("a:{}", tid("b"))).parent.as_deref(), Some("p:b"), "a project's folders to clean stay under it");

    st.toggle(top).unwrap();
    assert_eq!(st.sel_artifacts, ids(&["a", "b", "d"]), "every free project inside it, the inner folder's too");
    assert_eq!(heading(&st.view(false), top).2, Check::On);
    st.toggle(top).unwrap();
    assert!(st.sel_artifacts.is_empty());
    st.toggle(clients).unwrap();
    assert_eq!(st.sel_artifacts, ids(&["b"]));
    assert_eq!(heading(&st.view(false), top).2, Check::Mixed);
}

#[test]
fn a_worktree_that_cannot_be_removed_cannot_be_ticked() {
    let mut dirty = report("d", ProjectStatus::Inactive, true, 40);
    dirty.is_agent_worktree = true;
    dirty.artifacts.clear();
    dirty.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
    dirty.git.dirty = Some(true);
    let mut in_use = report("u", ProjectStatus::InUse, false, 5);
    in_use.is_agent_worktree = true;
    let mut st = with_scan(DevScan { worktrees: vec![dirty, in_use], scanned_at_ms: xiox_app::now_ms(), ..Default::default() });
    assert_eq!(err(st.toggle("w:d")), "Unsaved changes: the worktree stays");
    assert_eq!(err(st.toggle("w:u")), locked_why(&st.scan.as_ref().unwrap().worktrees[1]));
    assert_eq!(st.sel_worktrees, vec![false, false]);
    // «All» skips them too.
    st.select_category(DevCategory::Worktrees).unwrap();
    st.select_all(true).unwrap();
    assert_eq!(st.sel_worktrees, vec![false, false]);
    assert!(!st.can_clean());
}

#[test]
fn select_all_and_category_need_an_analysis_and_act_on_the_visible_list() {
    let mut st = state();
    assert_eq!(err(st.select_all(true)), NO_ANALYSIS);
    assert_eq!(err(st.toggle("p:a")), NO_ANALYSIS);
    assert_eq!(err(st.select_category(DevCategory::Sessions)), NO_ANALYSIS);

    let mut st = scanned();
    st.select_all(false).unwrap();
    assert!(st.sel_artifacts.is_empty());
    st.select_all(true).unwrap();
    assert_eq!(st.sel_artifacts, ids(&["a"]), "the blocked project's folders never go in");
    assert_eq!(st.sel_worktrees, vec![false], "only the visible list changes");
    st.select_category(DevCategory::Worktrees).unwrap();
    assert_eq!(st.section, Section::Worktrees);
    st.select_all(true).unwrap();
    assert_eq!(st.sel_worktrees, vec![true]);
    st.select_category(DevCategory::Sessions).unwrap();
    st.select_all(true).unwrap();
    assert_eq!(st.sel_sessions, vec![true, true]);
    st.select_all(false).unwrap();
    assert_eq!(st.sel_sessions, vec![false, false]);
}

#[test]
fn nothing_changes_while_busy_or_with_the_dialog_open() {
    let mut st = scanned();
    let snapshot = |st: &DevState| {
        (
            st.sel_artifacts.clone(),
            st.sel_worktrees.clone(),
            st.sel_sessions.clone(),
            st.section,
            st.settings.inactive_days,
            st.settings.remove_mode,
            st.settings.scan_roots,
            st.settings.roots.clone(),
            st.root_sel,
        )
    };
    let before = snapshot(&st);
    let dir = PathBuf::from(r"D:\somewhere\new");

    // A running job: every intent but `cancel` and `close_confirm` says BUSY.
    let (_tx, rx) = std::sync::mpsc::channel::<DevMsg>();
    st.job = Some(rx);
    let scan_roots = st.settings.scan_roots;
    let results = vec![
        ("select_category", st.select_category(DevCategory::Sessions)),
        ("toggle", st.toggle("p:a")),
        ("select_all", st.select_all(true)),
        ("set_inactive_days", st.set_inactive_days(10)),
        ("set_remove_mode", st.set_remove_mode(RemoveMode::Permanent)),
        ("set_scan_roots", st.set_scan_roots(!scan_roots)),
        ("select_root", st.select_root(Some(0))),
        ("add_root", st.add_root(dir.clone())),
        ("remove_root", st.remove_root()),
        ("analyze", st.analyze(notifier(), false)),
        ("request_clean", st.request_clean()),
        ("set_ack", st.set_ack(true)),
        ("take_ready_confirm", st.take_ready_confirm(false, xiox_app::now_ms()).map(|_| ())),
        ("confirm", st.confirm(notifier(), false)),
    ];
    for (name, r) in results {
        assert_eq!(err(r), BUSY, "{name} while a job runs");
    }
    assert_eq!(snapshot(&st), before);
    assert!(st.confirm.is_none() && st.busy());
    // `cancel` is allowed while busy: here nothing can be cancelled (an analysis), and it says so.
    assert_eq!(err(st.cancel()), NOTHING_TO_CANCEL);
    st.job = None;

    // A confirmation on screen: the warnings it shows describe THIS selection, so it is frozen.
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    assert!(st.confirm.is_some());
    let results = vec![
        ("select_category", st.select_category(DevCategory::Sessions)),
        ("toggle", st.toggle("p:a")),
        ("select_all", st.select_all(true)),
        ("set_inactive_days", st.set_inactive_days(10)),
        ("set_remove_mode", st.set_remove_mode(RemoveMode::Permanent)),
        ("set_scan_roots", st.set_scan_roots(!scan_roots)),
        ("select_root", st.select_root(Some(0))),
        ("add_root", st.add_root(dir.clone())),
        ("remove_root", st.remove_root()),
        ("analyze", st.analyze(notifier(), false)),
        ("request_clean", st.request_clean()),
    ];
    for (name, r) in results {
        assert_eq!(err(r), DIALOG_OPEN, "{name} with the confirmation open");
    }
    assert_eq!(snapshot(&st), before);
    assert!(st.confirm.is_some() && !st.busy(), "the dialog is untouched");

    // Closing it gives the selection back.
    st.close_confirm();
    assert!(st.confirm.is_none());
    st.toggle("p:a").unwrap();
    assert!(st.sel_artifacts.is_empty());
}

// ---------- settings ----------

#[test]
fn settings_intents_clamp_dedupe_and_stay_in_memory_with_only_root() {
    // The real settings file (`%APPDATA%\Xiox`) is never looked at here: what keeps it untouched is
    // `persist`, which `only_root` turns off.
    let tmp = tempfile::tempdir().unwrap();
    let mut st = DevState::only_root(tmp.path().to_path_buf(), Section::Projects).with_workers(TEST_WORKERS);
    assert!(!st.persist, "an --only-root state keeps its settings in memory");
    st.set_inactive_days(0).unwrap();
    assert_eq!(st.settings.inactive_days, 1);
    st.set_inactive_days(999).unwrap();
    assert_eq!(st.settings.inactive_days, 365);
    st.set_inactive_days(45).unwrap();
    assert_eq!(st.settings.inactive_days, 45);
    st.set_remove_mode(RemoveMode::Permanent).unwrap();
    assert_eq!(st.settings.remove_mode, RemoveMode::Permanent);
    st.set_scan_roots(false).unwrap();
    assert!(!st.settings.scan_roots);

    let extra = PathBuf::from(r"D:\Extra\Folder");
    st.add_root(extra.clone()).unwrap();
    st.add_root(extra.clone()).unwrap();
    st.add_root(PathBuf::from(r"d:\extra\folder")).unwrap();
    assert_eq!(st.settings.roots, vec![tmp.path().to_path_buf(), extra.clone()], "a duplicate (even by case) is ignored");

    assert!(st.select_root(Some(7)).is_err(), "no such folder in the list");
    assert_eq!(err(st.remove_root()), NO_ROOT_SELECTED);
    st.select_root(Some(1)).unwrap();
    assert_eq!(st.root_sel, Some(1));
    st.remove_root().unwrap();
    assert_eq!(st.settings.roots, vec![tmp.path().to_path_buf()]);
    assert_eq!(st.root_sel, None);
    st.select_root(Some(0)).unwrap();
    st.select_root(None).unwrap();
    assert_eq!(st.root_sel, None);

    // `persist` is false: nothing was written and nothing failed to write.
    assert!(st.settings_err.is_none());
    assert!(!st.persist);
}

// ---------- confirmation ----------

#[test]
fn confirm_needs_an_open_ready_dialog() {
    // No dialog: nothing runs, whatever the selection says.
    let mut st = scanned();
    assert_eq!(err(st.confirm(notifier(), false)), NO_DIALOG);
    assert_eq!(err(st.take_ready_confirm(false, xiox_app::now_ms())), NO_DIALOG);
    assert_eq!(err(st.set_ack(true)), NO_DIALOG);
    assert!(!st.busy());

    // A Caution folder in the plan: the warnings have to be read first.
    let mut st = levels_state();
    st.sel_artifacts.insert(dist_id());
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    assert_eq!(err(st.confirm(notifier(), false)), NOT_READY);
    assert_eq!(err(st.take_ready_confirm(false, xiox_app::now_ms())), NOT_READY);
    assert!(st.confirm.is_some(), "the dialog stays open");
    assert!(!st.busy());
    // «Permanently» is not the button of this dialog (no shortfall): refused even when ready.
    st.set_ack(true).unwrap();
    assert_eq!(err(st.take_ready_confirm(true, xiox_app::now_ms())), PERMANENT_NOT_OFFERED);
    assert!(st.confirm.is_some());
    // The warnings read: the plan comes out, once, and the dialog is gone.
    let c = st.take_ready_confirm(false, xiox_app::now_ms()).unwrap().expect("a ready confirmation");
    assert_eq!(c.plan.as_ref().unwrap().items.len(), 2);
    assert!(st.confirm.is_none());
    assert_eq!(err(st.confirm(notifier(), false)), NO_DIALOG, "a second Enter does nothing");

    // The Recycle Bin is too small: only «Permanently» is offered.
    let _serial = CALLS_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    CALLS.lock().unwrap().clear();
    let mut st = levels_state();
    st.settings.remove_mode = RemoveMode::Recycle;
    st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
    assert_eq!(err(st.confirm(notifier(), false)), PERMANENT_ONLY);
    assert!(st.confirm.is_some() && !st.busy());
    assert!(CALLS.lock().unwrap().is_empty());
    st.confirm(notifier(), true).unwrap();
    assert!(st.busy() && st.confirm.is_none());
    finish(&mut st);
    assert_eq!(*CALLS.lock().unwrap(), vec![(RemoveMode::Permanent, 1)], "the worker got the permanent plan");
    assert_eq!(st.view, View::Cleaned);
}

/// The Recycle Bin is too small AND a Caution folder is in the plan: «Permanently» is the only
/// button, and it still needs the warnings read. The refusal comes BEFORE the plan is switched to
/// permanent: a dialog that is refused is left exactly as it was (still offering the shortfall,
/// still in Recycle mode), never silently downgraded to a permanent deletion by a button that did
/// nothing.
#[test]
fn permanently_with_unread_warnings_is_refused_before_the_plan_turns_permanent() {
    let _serial = CALLS_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    CALLS.lock().unwrap().clear();
    let mut st = levels_state();
    st.settings.remove_mode = RemoveMode::Recycle;
    st.sel_artifacts.insert(dist_id());
    st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
    let c = st.confirm.as_ref().expect("the dialog is open");
    assert!(!c.shortfall.is_empty() && !c.cautions.is_empty() && !c.ack, "shortfall + caution + warnings not read");
    assert_eq!(c.mode, RemoveMode::Recycle);

    assert_eq!(err(st.confirm(notifier(), true)), NOT_READY);
    assert_eq!(err(st.take_ready_confirm(true, xiox_app::now_ms())), NOT_READY);
    let c = st.confirm.as_ref().expect("the dialog stays open");
    assert!(!c.shortfall.is_empty(), "the shortfall is still offered: the plan was not made permanent");
    assert_eq!(c.mode, RemoveMode::Recycle);
    assert!(!st.busy());
    assert!(CALLS.lock().unwrap().is_empty(), "nothing ran");
    let d = st.view(false).dialog.expect("the dialog is still drawn");
    assert_eq!(d.buttons.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["permanent", "cancel"]);
    assert!(!button(&d, "permanent").enabled, "the button is off until the warnings are read");

    // The other button is not this dialog's either, and it is refused for the same first reason.
    assert_eq!(err(st.take_ready_confirm(false, xiox_app::now_ms())), NOT_READY);

    // The warnings read: now «Permanently» runs, as a permanent plan.
    st.set_ack(true).unwrap();
    assert_eq!(err(st.take_ready_confirm(false, xiox_app::now_ms())), PERMANENT_ONLY);
    st.confirm(notifier(), true).unwrap();
    assert!(st.busy() && st.confirm.is_none());
    finish(&mut st);
    assert_eq!(*CALLS.lock().unwrap(), vec![(RemoveMode::Permanent, 2)]);
}

#[test]
fn a_plan_that_did_not_build_is_not_run() {
    // The dialog carries an error: not ready, and the reason is not the warnings'.
    let mut st = levels_state();
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    st.confirm.as_mut().unwrap().error = Some("Plan refused: test".into());
    assert_eq!(err(st.take_ready_confirm(false, xiox_app::now_ms())), NOTHING_TO_RUN);
    assert!(st.confirm.is_some());
    let v = st.view(false);
    assert!(!button(v.dialog.as_ref().unwrap(), "clean").enabled);
    assert_eq!(v.dialog.as_ref().unwrap().error.as_deref(), Some("Plan refused: test"));
}

#[test]
fn confirm_after_half_an_hour_only_warns() {
    let mut st = scanned();
    let t = st.scan.as_ref().unwrap().scanned_at_ms;
    st.open_confirm_with(t + 1000, |_| vec![]);
    assert!(st.confirm.is_some());
    let late = t + 31 * 60 * 1000;
    assert!(late - t > xiox_app::MAX_SCAN_AGE_MS);
    assert!(matches!(st.take_ready_confirm(false, late), Ok(None)), "an old analysis does not clean");
    assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
    assert!(!st.busy());
    assert!(st.confirm.is_none(), "the dialog is closed, the warning is on screen");
    let v = st.view(false);
    assert!(v.notices.iter().any(|n| n.tone == Tone::Warn && n.text == STALE_SCAN));

    // Just inside the limit it goes.
    let mut st = scanned();
    let t = st.scan.as_ref().unwrap().scanned_at_ms;
    st.open_confirm_with(t, |_| vec![]);
    assert!(matches!(st.take_ready_confirm(false, t + xiox_app::MAX_SCAN_AGE_MS), Ok(Some(_))));
    assert!(st.notice.is_none());
}

#[test]
fn request_clean_opens_the_dialog_or_says_why_not() {
    let mut st = scanned();
    st.select_all(false).unwrap();
    assert_eq!(err(st.request_clean()), NOTHING_SELECTED);
    assert!(st.confirm.is_none());
    st.toggle("p:a").unwrap();
    st.request_clean().unwrap();
    assert!(st.confirm.is_some());

    // An analysis older than 30 minutes opens nothing and says so (the old window's strip, as a notice).
    let mut old = three_sections();
    old.scanned_at_ms = xiox_app::now_ms() - 31 * 60 * 1000;
    let mut st = with_scan(old);
    st.request_clean().unwrap();
    assert!(st.confirm.is_none());
    assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
}

// ---------- cancel ----------

#[test]
fn cancel_stops_a_running_cleanup_and_never_leaves_the_old_analysis_to_clean() {
    let mut st = scanned();
    assert_eq!(err(st.cancel()), NOTHING_TO_CANCEL, "idle");

    // A real cleanup through the seam, held by a worker that waits for the cancel flag.
    st.workers = DevWorkers { scan: scan_none, clean: clean_until_cancelled };
    st.settings.remove_mode = RemoveMode::Recycle;
    st.request_clean().unwrap();
    st.confirm(notifier(), false).unwrap();
    assert!(st.busy());
    let v = st.view(false);
    assert_eq!(v.status.badge, "Cleaning");
    assert_eq!(v.job.as_ref().unwrap().label, "Cleaning…");
    assert_eq!(action(&v, "cancel").label, "Cancel");
    assert!(action(&v, "cancel").enabled);
    assert!(!action(&v, "clean").enabled && !action(&v, "analyze").enabled);
    assert_eq!(err(st.toggle("p:a")), BUSY);

    st.cancel().unwrap();
    st.cancel().unwrap();
    assert!(st.cancelling());
    let v = st.view(false);
    assert_eq!(v.status.badge, "Cancelling");
    assert_eq!(v.job.as_ref().unwrap().label, "Cancelling…");
    assert_eq!(action(&v, "cancel").label, "Cancelling…");
    assert!(!action(&v, "cancel").enabled);

    // The worker saw the flag and answered: a report, not the old analysis.
    finish(&mut st);
    assert!(st.cancel.is_none() && st.scan.is_none());
    assert!(!st.can_clean(), "no Clean button over an analysis a partial cleanup has changed");
    assert_eq!(err(st.request_clean()), NOTHING_SELECTED);
    let v = st.view(false);
    assert_eq!(v.status.badge, "Cleanup cancelled");
    let report = v.report.as_ref().expect("the report of the partial cleanup");
    assert!(report.cancelled);
    assert!(report.notes.iter().any(|n| n.text == "Cancelled: the items not yet started stayed where they were."));
    assert!(!action(&v, "clean").enabled && action(&v, "analyze").enabled);
    assert!(v.actions.iter().all(|a| a.id != "cancel"));
}

#[test]
fn a_cleanup_without_a_plan_has_nothing_to_cancel() {
    // Sessions have no cancellable engine call: no Cancel button, and the intent says so.
    let mut st = scanned();
    st.workers = DevWorkers { scan: scan_none, clean: clean_quiet };
    st.select_category(DevCategory::Sessions).unwrap();
    st.select_all(true).unwrap();
    st.request_clean().unwrap();
    st.confirm(notifier(), false).unwrap();
    assert!(st.busy() && st.cancel.is_none());
    assert_eq!(err(st.cancel()), NOTHING_TO_CANCEL);
    assert!(st.view(false).actions.iter().all(|a| a.id != "cancel"));
    finish(&mut st);
}

// ---------- view ----------

#[test]
fn dev_view_lists_sections_rows_and_the_clean_button() {
    let st = scanned();
    let v = st.view(false);
    assert_eq!(v.section, SectionId::Developers);
    assert_eq!(v.rev, 0, "Core numbers the revisions");
    assert_eq!(v.status.badge, "Analysis done");
    assert!(v.status.text.starts_with(&format!("{} to free · 1 of 2 selected · Analysis in ", sys::fmt_size(100))), "{}", v.status.text);
    assert!(v.status.text.ends_with("folders measured one by one"), "{}", v.status.text);
    assert!(v.job.is_none());

    // The three lists, the open one marked, the picks and totals of each.
    let nav: Vec<(&str, bool, Option<&str>, Option<u64>)> =
        v.nav.iter().map(|n| (n.id.as_str(), n.active, n.note.as_deref(), n.bytes)).collect();
    assert_eq!(
        nav,
        vec![
            ("projects", true, Some("1 folder selected"), Some(150)),
            ("worktrees", false, None, Some(30)),
            ("sessions", false, None, Some(4)),
        ]
    );
    assert_eq!(v.nav[0].name, Section::Projects.name());

    // Projects: under the folder they sit in, each with its folders to clean under it; the blocked one greyed out with
    // its reason. The folder's box is the free project's.
    let ids: Vec<String> = v.rows.iter().map(|r| r.id.clone()).collect();
    let folder = r"h:folder:D:\p";
    assert_eq!(ids, vec![folder.to_string(), "p:a".to_string(), format!("a:{}", tid("a")), "p:b".to_string(), format!("a:{}", tid("b"))]);
    let f = row(&v, folder);
    assert_eq!((f.kind, f.name.as_str(), f.parent.as_deref(), f.check, f.enabled), (RowKind::Header, r"D:\p", None, Check::On, true));
    let a = row(&v, "p:a");
    assert_eq!((a.kind, a.check, a.enabled, a.muted), (RowKind::Item, Check::On, true, false));
    assert_eq!(a.bytes, Some(100));
    assert!(a.parent.as_deref() == Some(folder) && a.why.is_none());
    let child = row(&v, &format!("a:{}", tid("a")));
    assert_eq!((child.kind, child.parent.as_deref(), child.check, child.enabled), (RowKind::Child, Some("p:a"), Check::On, true));
    assert_eq!(child.level, Some(LevelView::Recommended));
    assert_eq!(child.name, "Build Rust (target)");
    let keys: Vec<&str> = child.detail.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["item", "folder", "size", "level", "mode"]);
    assert_eq!(child.detail[0].1, "a · Build Rust (target)");
    assert_eq!(child.detail[4].1, "to the Recycle Bin");
    let b = row(&v, "p:b");
    let why = locked_why(&st.scan.as_ref().unwrap().projects[1]);
    assert_eq!((b.check, b.enabled, b.muted, b.why.as_deref()), (Check::Off, false, true, Some(why.as_str())));
    let bchild = row(&v, &format!("a:{}", tid("b")));
    assert_eq!((bchild.check, bchild.enabled, bchild.muted), (Check::Off, false, true));
    assert_eq!(bchild.parent.as_deref(), Some("p:b"));

    // The columns of the open list and the buttons.
    let cols: Vec<ColumnKind> = v.columns.iter().map(|c| c.kind).collect();
    assert_eq!(cols, vec![ColumnKind::Name, ColumnKind::Cell, ColumnKind::Level, ColumnKind::Size]);
    let clean = action(&v, "clean");
    assert_eq!(clean.label, clean_button(Section::Projects, 1));
    assert!(clean.enabled && clean.primary);
    assert_eq!(clean.key.as_deref(), Some("Enter"));
    let analyze = action(&v, "analyze");
    assert!(analyze.enabled);
    assert_eq!(analyze.key.as_deref(), Some("a"));
    assert!(v.actions.iter().all(|a| a.id != "cancel"), "nothing to cancel");
    assert!(v.bulk);
    assert!(v.empty.is_none() && v.dialog.is_none() && v.report.is_none());
    let footer = format!("Selected in {}: {} · {}", Section::Projects.name(), sys::fmt_size(100), "1 folder");
    assert_eq!(v.footer.as_deref(), Some(footer.as_str()));
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Number { id, value, min: 1, max: 365, enabled: true, .. } if id == "inactiveDays" && *value == st.settings.inactive_days)));

    // The other two lists, from the same state.
    let mut st = st;
    st.select_category(DevCategory::Worktrees).unwrap();
    let v = st.view(false);
    assert_eq!(v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["h:repo:none", "w:w"], "the worktree under its repository");
    let w = row(&v, "w:w");
    assert_eq!((w.check, w.enabled), (Check::Off, true));
    assert!(w.detail.iter().any(|(k, val)| k == "mode" && val == "permanently (git)"));
    assert_eq!(action(&v, "clean").label, clean_button(Section::Worktrees, 0));
    assert!(!action(&v, "clean").enabled, "nothing ticked in this list");
    assert!(v.nav[1].active && !v.nav[0].active);
    st.select_category(DevCategory::Sessions).unwrap();
    let v = st.view(false);
    let s1 = row(&v, "s:codex:1");
    assert_eq!((s1.bytes, s1.size_text.as_deref()), (None, Some("database row only")), "no file: the text, not a size");
    let s2 = row(&v, "s:codex:2");
    assert_eq!((s2.bytes, s2.size_text.as_deref()), (Some(4), None));
    assert_eq!(s2.cells.len(), 3, "harness, project folder, date");
    assert_eq!(s2.cells[0], "Codex");
    assert_eq!(s2.cells[1], "demo");
    assert_eq!(s2.name, "titolo 2");
}

#[test]
fn dev_view_without_a_scan_and_while_scanning() {
    let mut st = state();
    st.persist = false;
    let v = st.view(false);
    assert_eq!(v.status.badge, "Ready");
    assert!(v.rows.is_empty() && !v.bulk && v.footer.is_none() && v.report.is_none());
    assert!(v.nav.iter().all(|n| n.bytes.is_none() && n.note.is_none()));
    let e = v.empty.as_ref().unwrap();
    assert_eq!(e.title, "Developer cleanup");
    assert_eq!(e.text, "Press “Analyze”: the projects come from the Claude Code, Codex, OpenCode and Pi sessions.");
    assert!(action(&v, "analyze").enabled && !action(&v, "clean").enabled);
    assert_eq!(action(&v, "clean").label, clean_button(Section::Projects, 0));
    assert_eq!(v.options.len(), 5, "days, deletion, the Recycle Bin note, scan roots, folders: no error text");

    // A failure to start leaves the reason on screen.
    st.error = Some("The job was interrupted by an internal error.".into());
    let v = st.view(false);
    assert!(v.notices.iter().any(|n| n.tone == Tone::Error && n.text.starts_with("The job was interrupted")));

    // Scanning: a job without a tick shows the worker's own text and progress; options are locked.
    st.error = None;
    st.analyze(notifier(), false).unwrap();
    assert!(st.busy());
    let v = st.view(false);
    assert_eq!(v.status.badge, "Analyzing");
    let job = v.job.as_ref().unwrap();
    assert_eq!((job.label.as_str(), job.fraction, job.detail.as_deref()), ("Starting the analysis…", 0.0, None));
    let e = v.empty.as_ref().unwrap();
    assert_eq!(e.title, "Analyzing");
    assert_eq!(e.text, "The analysis can take a minute: meanwhile you can use the other sections.");
    assert!(!action(&v, "analyze").enabled && !v.bulk);
    assert!(v.options.iter().all(|o| match o {
        OptionView::Number { enabled, .. }
        | OptionView::Choice { enabled, .. }
        | OptionView::Toggle { enabled, .. }
        | OptionView::List { enabled, .. } => !*enabled,
        _ => true,
    }));
    // The scan worker of the seam answered nothing and ended: reported, not a spinner.
    finish(&mut st);
    assert!(st.error.is_some());
}

#[test]
fn dev_view_dialog_shows_cautions_ack_and_shortfall() {
    let mut st = levels_state();
    st.sel_artifacts.insert(dist_id());
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    let v = st.view(false);
    let d = v.dialog.as_ref().expect("the confirmation");
    assert_eq!(d.id, "confirm");
    assert_eq!(d.title, confirm_title(Section::Projects));
    assert_eq!(d.text, vec![confirm_text(Section::Projects, st.settings.remove_mode).to_string()]);
    assert_eq!(d.caution_title.as_deref(), Some(caution_title(1).as_str()));
    assert_eq!(d.cautions.len(), 1);
    assert_eq!(d.ack, Some(false));
    assert_eq!(d.rows.len(), 2);
    assert_eq!(d.permanent, st.settings.remove_mode == RemoveMode::Permanent, "the flag follows the mode the button acts in");
    assert_eq!(d.buttons.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["clean", "cancel"]);
    assert!(!button(d, "clean").enabled && button(d, "clean").primary);
    assert_eq!(button(d, "clean").label, clean_button(Section::Projects, 2));
    assert!(button(d, "cancel").enabled && !button(d, "cancel").primary);
    assert!(d.error.is_none());
    st.set_ack(true).unwrap();
    let v = st.view(false);
    let d = v.dialog.as_ref().unwrap();
    assert_eq!(d.ack, Some(true));
    assert!(button(d, "clean").enabled);
    st.set_ack(false).unwrap();
    assert!(!button(st.view(false).dialog.as_ref().unwrap(), "clean").enabled, "taking the tick back disables it again");

    // Only target in the plan: no cautions, no ack box, ready at once.
    let mut st = levels_state();
    st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
    let v = st.view(false);
    let d = v.dialog.as_ref().unwrap();
    assert!(d.ack.is_none() && d.caution_title.is_none() && d.cautions.is_empty());
    assert!(button(d, "clean").enabled);
    assert_eq!(err(st.set_ack(true)), NO_WARNINGS);

    // The Recycle Bin is too small: «Permanently» and «Cancel», and the question in the text.
    let mut st = levels_state();
    st.settings.remove_mode = RemoveMode::Recycle;
    st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
    let v = st.view(false);
    let d = v.dialog.as_ref().unwrap();
    assert_eq!(d.buttons.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["permanent", "cancel"]);
    assert_eq!(button(d, "permanent").label, "Permanently");
    assert!(d.permanent, "the way out of a full Recycle Bin deletes for good");
    assert!(button(d, "permanent").enabled && button(d, "permanent").primary);
    assert!(d.text.last().unwrap().ends_with("Delete permanently this time?"));
    assert_eq!(d.text[0], confirm_text(Section::Projects, RemoveMode::Permanent), "the text describes what the button does");
}

#[test]
fn dev_view_report_after_cleaning_and_save_text() {
    let mut st = scanned();
    let (tx, rx) = std::sync::mpsc::channel::<DevMsg>();
    st.job = Some(rx);
    st.view = View::Cleaned;
    let mut out = DevCleaned::new();
    out.report.freed_bytes = 10;
    out.report.files = 2;
    out.report.dirs = 1;
    out.report.elapsed_ms = 1500;
    out.report.by_category = vec![("Progetti".into(), 10)];
    out.report.skipped = vec![(PathBuf::from(r"D:\p\a\target"), "in uso".into())];
    out.labels.insert(xiox_model::path_key(Path::new(r"D:\p\a\target")), "a · Build Rust (target)".into());
    out.errors = vec!["Plan refused: test".into()];
    tx.send(DevMsg::Cleaned(out)).unwrap();
    st.poll();
    assert!(!st.busy() && st.scan.is_none());

    let v = st.view(false);
    assert_eq!(v.status.badge, "Cleanup done");
    assert_eq!(v.status.text, format!("Space freed: {}", sys::fmt_size(10)));
    assert!(v.rows.is_empty() && !v.bulk && v.empty.is_none() && v.footer.is_none());
    let r = v.report.as_ref().expect("the report");
    assert_eq!((r.freed_bytes, r.recycled_bytes, r.files, r.dirs, r.elapsed_ms), (10, 0, 2, 1, 1500));
    assert!(r.can_save && !r.cancelled && r.saved.is_none() && r.items.is_empty());
    assert_eq!(r.by_category, vec![("Progetti".to_string(), 10)]);
    assert_eq!(r.skipped, vec![ReportLine { item: "a · Build Rust (target)".into(), why: "in uso".into(), path: r"D:\p\a\target".into() }]);
    assert!(r.notes.is_empty());
    // A refused request is an error with what to do next.
    assert!(v.notices.iter().any(|n| n.tone == Tone::Error && n.text == "Plan refused: test"));
    assert!(v.notices.iter().any(|n| n.text == "That part was not touched. Press “Analyze” to analyze again before retrying."));
    assert!(!action(&v, "clean").enabled, "nothing to clean from a report");

    let text = st.report_text().expect("a non-empty result");
    assert!(text.starts_with("Cleanup report"), "{text}");
    st.set_saved("x".into(), true);
    assert_eq!(st.view(false).report.unwrap().saved, Some(SavedView { text: "x".into(), ok: true }));
    st.set_saved("no".into(), false);
    assert_eq!(st.view(false).report.unwrap().saved, Some(SavedView { text: "no".into(), ok: false }));
}

#[test]
fn dev_view_report_notes_for_an_empty_or_interrupted_cleanup() {
    let landed = |mutate: &dyn Fn(&mut DevCleaned)| {
        let mut st = scanned();
        let (tx, rx) = std::sync::mpsc::channel::<DevMsg>();
        st.job = Some(rx);
        st.view = View::Cleaned;
        let mut out = DevCleaned::new();
        mutate(&mut out);
        tx.send(DevMsg::Cleaned(out)).unwrap();
        st.poll();
        st
    };
    // Nothing done, nothing refused: say so, and there is nothing to save.
    let st = landed(&|_| {});
    let v = st.view(false);
    let r = v.report.unwrap();
    assert_eq!(r.notes, vec![NoticeView { tone: Tone::Info, text: "Nothing to clean.".into(), more: vec![] }]);
    assert!(!r.can_save);
    assert!(st.report_text().is_none());
    // Interrupted: the notice, not «Nothing to clean».
    let st = landed(&|o| o.interrupted = true);
    let v = st.view(false);
    assert!(v.notices.iter().any(|n| n.tone == Tone::Error && n.text.starts_with("An internal error stopped the cleanup")));
    assert!(v.report.unwrap().notes.is_empty());
    assert_eq!(v.status.badge, "Cleanup interrupted");
}

#[test]
fn dev_view_notices_order_the_warnings_and_keep_the_failure_first() {
    let mut scan = three_sections();
    scan.warnings = vec!["uno".into(), xiox_app::ANALYSIS_FAILED.to_string(), "tre".into(), "quattro".into()];
    let mut st = with_scan(scan);
    st.notice = Some("un avviso".into());
    let v = st.view(false);
    assert_eq!(v.notices.len(), 3);
    assert_eq!((v.notices[0].tone, v.notices[0].text.as_str()), (Tone::Warn, "un avviso"));
    let failed = format!("{} The projects and worktrees were not analyzed: press “Analyze” to try again.", xiox_app::ANALYSIS_FAILED);
    assert_eq!((v.notices[1].tone, v.notices[1].text.as_str(), v.notices[1].more.len()), (Tone::Warn, failed.as_str(), 0));
    assert_eq!((v.notices[2].tone, v.notices[2].text.as_str()), (Tone::Warn, "uno"));
    assert_eq!(v.notices[2].more, vec!["tre".to_string(), "quattro".to_string()], "past two, the rest hide in `more`");
    // A failed analysis explains an empty list.
    let mut st = with_scan(DevScan { warnings: vec![xiox_app::ANALYSIS_FAILED.to_string()], scanned_at_ms: xiox_app::now_ms(), ..Default::default() });
    let v = st.view(false);
    assert_eq!(v.empty.as_ref().unwrap().text, empty_text(Section::Projects, true));
    st.select_category(DevCategory::Sessions).unwrap();
    assert_eq!(st.view(false).empty.as_ref().unwrap().text, empty_text(Section::Sessions, false));
}

#[test]
fn dev_view_options_report_the_settings_and_the_save_error() {
    let mut st = scanned();
    st.settings.inactive_days = 45;
    st.settings.remove_mode = RemoveMode::Permanent;
    st.settings.roots = vec![PathBuf::from(r"D:\Work")];
    st.settings.scan_roots = true;
    st.root_sel = Some(0);
    st.settings_err = Some("access denied".into());
    let v = st.view(false);
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Number { value: 45, .. })));
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Choice { id, value, choices, .. } if id == "removeMode" && value == "permanent" && choices.len() == 2)));
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Toggle { id, on: true, .. } if id == "scanRoots")));
    assert!(v.options.iter().any(|o| matches!(o, OptionView::List { id, items, selected: Some(0), .. } if id == "roots" && items == &vec![r"D:\Work".to_string()])));
    assert!(v.options.iter().any(|o| matches!(o, OptionView::Text { text, tone: Tone::Error, .. } if text == "Settings not saved: access denied")));
    let detail_mode = |st: &DevState| {
        let v = st.view(false);
        row(&v, &format!("a:{}", tid("a"))).detail.iter().find(|(k, _)| k == "mode").map(|(_, m)| m.clone())
    };
    assert_eq!(detail_mode(&st).as_deref(), Some("permanently"));
    st.settings.remove_mode = RemoveMode::Recycle;
    assert_eq!(detail_mode(&st).as_deref(), Some("to the Recycle Bin"));
}

// ---------- the shared script ----------

/// Builder-vs-fixture drift (see `crate::drift`): the static strings of the view against the fixture state `name`.
fn drift_check(doc: &serde_json::Value, st: &DevState, name: &str, out: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>) {
    // The notices are the scan's warnings (an agent whose sessions could not be read): data, not text.
    let fixture = crate::drift::fixture_view(doc, "developers", name);
    // The job label is the worker's progress text, a cleanup of nothing (the replay's) has a report of its
    // own, and the fixture's dialogs and jobs are on Projects: a replay that is on another list is not it.
    let opts = crate::drift::Opts { notices: false, job: false, report: false, ..crate::drift::Opts::all() };
    let same_list = fixture["title"] == serde_json::to_value(st.view(false).title).unwrap();
    if !same_list && !["projects", "worktrees", "sessions"].contains(&name) {
        seen.insert(name.to_string());
        return;
    }
    out.extend(crate::drift::diff(&format!("developers:{name}"), &st.view(false), &fixture, opts));
    out.extend(crate::drift::view_problems(&format!("real developers:{name}"), &serde_json::to_value(st.view(false)).unwrap()));
    seen.insert(name.to_string());
}

/// The scan the replay runs on: the ids the script names (`p:1`, `w:3`, `h:others`...).
fn script_scan() -> DevScan {
    let mut p1 = report("1", ProjectStatus::Inactive, true, 100);
    // A second folder, not ticked: the project box is «mixed».
    let mut second = p1.artifacts[0].clone();
    second.id = xiox_model::path_key(Path::new(r"D:\p\1\node_modules"));
    second.path = PathBuf::from(r"D:\p\1\node_modules");
    second.preselected = false;
    p1.artifacts.push(second);
    // A second project with a Caution folder, ticked: the dialog has an ack box and some plan is
    // left even when `p:1` is switched off.
    let mut p2 = report("2", ProjectStatus::Inactive, true, 50);
    p2.artifacts[0].level = xiox_model::Level::Caution;
    // The folder the fixture's first two projects sit in: the script ticks its box.
    p1.root = PathBuf::from(r"D:\Projects\acme-api");
    p2.root = PathBuf::from(r"D:\Projects\old-prototype");
    let wt = |id: &str, pre: bool| {
        let mut r = report(id, ProjectStatus::Inactive, pre, 10);
        // Its own folders: the ids repeat the projects', the paths must not (the plan finds
        // an artifact by its path).
        r.root = PathBuf::from(format!(r"D:\w\{id}"));
        let target = PathBuf::from(format!(r"D:\w\{id}\target"));
        r.artifacts[0].id = xiox_model::path_key(&target);
        r.artifacts[0].path = target;
        r.is_agent_worktree = true;
        r.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
        r.git.dirty = Some(false);
        r
    };
    let mut dirty = wt("3", false);
    dirty.artifacts.clear();
    dirty.git.dirty = Some(true);
    DevScan {
        projects: vec![p1, p2],
        worktrees: vec![wt("1", true), wt("2", true), dirty],
        other_worktrees: vec![report("1", ProjectStatus::Inactive, false, 5)],
        archived: vec![session("1", None, true, 7), session("2", None, true, 8)],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    }
}

/// The script's name for what a state looks like.
fn shape(st: &DevState) -> &'static str {
    if st.busy() {
        return if st.view == View::Cleaned { "cleaning" } else { "analyzing" };
    }
    if let Some(c) = &st.confirm {
        return if c.shortfall.is_empty() { "confirm" } else { "shortfall" };
    }
    match (st.view, st.section) {
        (View::Empty, _) => "idle",
        (View::Cleaned, _) if st.cleaned.as_ref().is_some_and(|c| c.report.cancelled) => "cancelled",
        (View::Cleaned, _) => "report",
        (View::Scanned, Section::Projects) => "projects",
        (View::Scanned, Section::Worktrees) => "worktrees",
        (View::Scanned, Section::Sessions) => "sessions",
    }
}

/// The scan worker of the replay: lands the script's own scan (the ids it names).
fn scan_script(_: Settings, _: bool, out: Outbox<DevMsg>) {
    out.send(DevMsg::Scanned(script_scan()));
}

/// Raised by the replay when the script's cleanup is to finish by itself (a `tick` to `report`).
static FINISH_CLEAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The cleanup worker of the replay: runs until the replay lets it finish (a complete report) or
/// the job's `Cancel` flag is raised (a report marked cancelled, as the engine answers).
fn clean_script(job: DevCleanJob, out: Outbox<DevMsg>) {
    use std::sync::atomic::Ordering::SeqCst;
    for _ in 0..500 {
        if job.cancel.is_cancelled() || FINISH_CLEAN.load(SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut done = DevCleaned::new();
    done.report.cancelled = job.cancel.is_cancelled();
    out.send(DevMsg::Cleaned(done));
}

/// The state the script calls `name` (`go`).
fn script_state(name: &str) -> DevState {
    let mut st = DevState::with_settings(xiox_app::default_settings(), Section::Projects)
        .with_workers(DevWorkers { scan: scan_script, clean: clean_script });
    if name == "idle" {
        st.persist = false;
        return st;
    }
    st.set_scan(script_scan());
    st.persist = false;
    match name {
        "worktrees" => st.set_section(Section::Worktrees),
        "sessions" => st.set_section(Section::Sessions),
        "confirm" => st.open_confirm_with(xiox_app::now_ms(), |_| vec![]),
        "shortfall" => {
            st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
            // The script's shortfall dialog has no warnings to read: this one's are read.
            st.confirm.as_mut().unwrap().ack = true;
        }
        _ => {}
    }
    st
}

/// What the page sends for each intent, and what `DevState` is asked to do with it.
fn apply(st: &mut DevState, intent: DevIntent) -> Result<(), String> {
    match intent {
        DevIntent::Select { category } => st.select_category(category),
        DevIntent::Toggle { row } => st.toggle(&row),
        DevIntent::SelectAll { on } => st.select_all(on),
        DevIntent::SetInactiveDays { days } => st.set_inactive_days(days),
        DevIntent::SetRemoveMode { mode } => st.set_remove_mode(mode.into()),
        DevIntent::SetScanRoots { on } => st.set_scan_roots(on),
        DevIntent::SelectRoot { index } => st.select_root(index),
        DevIntent::RemoveRoot => st.remove_root(),
        DevIntent::Analyze => st.analyze(notifier(), false),
        DevIntent::OpenConfirm => st.request_clean(),
        DevIntent::SetAck { on } => st.set_ack(on),
        DevIntent::Confirm => st.confirm(notifier(), false),
        DevIntent::Permanent => st.confirm(notifier(), true),
        DevIntent::CloseDialog if st.confirm.is_some() => {
            st.close_confirm();
            Ok(())
        }
        // Over a report (finished or cancelled) the page's «Close» is `release()`: the section
        // goes back to its empty state and the analysis is not brought back.
        DevIntent::CloseDialog if st.view == View::Cleaned && !st.busy() => {
            st.release();
            Ok(())
        }
        DevIntent::CloseDialog => Err(NO_DIALOG.into()),
        DevIntent::Cancel => st.cancel(),
    }
}

/// Replays the Developers steps of `apps/desktop/fixtures/intent-script.json` against the real
/// state, through the worker seam. The script is the spec for the transitions and none of them is
/// bent to fit: a `tick` is replayed by letting the worker answer (the scan lands, the cleanup
/// ends), a `cancel` of a cleanup raises the job's flag and waits for the cancelled report, and a
/// refused intent (`cancel` of a scan, `select` over a report) must leave the shape unchanged.
/// The ticks of the script that only move a fraction are not replayed.
#[test]
fn replays_the_developers_steps_of_the_shared_script() {
    use std::sync::atomic::Ordering::SeqCst;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/intent-script.json");
    let script: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut st = script_state("idle");
    let doc = crate::drift::document();
    let (mut drift, mut seen) = (Vec::<String>::new(), std::collections::BTreeSet::<String>::new());
    drift_check(&doc, &st, "idle", &mut drift, &mut seen);
    // Compared intents; scans that landed; cleanups that finished by themselves; cleanups cancelled.
    let (mut compared, mut landed, mut finished, mut cancelled) = (0, 0, 0, 0);
    for step in script["steps"].as_array().unwrap() {
        let note = step["note"].as_str().unwrap_or("");
        match step["op"].as_str().unwrap() {
            "go" if step["section"] == "developers" => {
                st = script_state(step["state"].as_str().unwrap());
                if shape(&st) == step["state"].as_str().unwrap() {
                    drift_check(&doc, &st, shape(&st), &mut drift, &mut seen);
                }
            }
            "tick" => {
                for pushed in step["pushed"].as_array().unwrap().iter().filter(|p| p["section"] == "developers") {
                    match (shape(&st), pushed["state"].as_str().unwrap()) {
                        // The scan ends: its answer is polled in, like the pump does.
                        ("analyzing", "projects" | "worktrees" | "sessions") => {
                            // `set_scan` drops what an in-memory state may not keep: this one has no tree.
                            st.persist = true;
                            finish(&mut st);
                            st.persist = false;
                            assert_eq!(shape(&st), "projects", "{note}: the scan lands on projects");
                            landed += 1;
                        }
                        ("cleaning", "report") => {
                            FINISH_CLEAN.store(true, SeqCst);
                            finish(&mut st);
                            FINISH_CLEAN.store(false, SeqCst);
                            assert_eq!(shape(&st), "report", "{note}");
                            drift_check(&doc, &st, "report", &mut drift, &mut seen);
                            finished += 1;
                        }
                        _ => {}
                    }
                }
            }
            "intent" if step["section"] == "developers" => {
                let intent: DevIntent = serde_json::from_value(step["intent"].clone()).unwrap();
                let expect = step["expect"]["state"].as_str().unwrap();
                let was = shape(&st);
                let result = apply(&mut st, intent.clone());
                if was == "cleaning" && matches!(intent, DevIntent::Cancel) && result.is_ok() {
                    // The flag is up: the worker answers with the cancelled report.
                    finish(&mut st);
                    cancelled += 1;
                }
                assert_eq!(shape(&st), expect, "{note}: {intent:?}");
                let v = st.view(false);
                drift_check(&doc, &st, expect, &mut drift, &mut seen);
                if let Some(checks) = step["expect"]["checks"].as_object() {
                    for (id, want) in checks {
                        let got = serde_json::to_value(row(&v, id).check).unwrap();
                        assert_eq!(&got, want, "{note}: check of {id}");
                    }
                }
                if let Some(ack) = step["expect"]["ack"].as_bool() {
                    assert_eq!(v.dialog.as_ref().unwrap().ack, Some(ack), "{note}: ack");
                }
                if let Some(on) = step["expect"]["primaryEnabled"].as_bool() {
                    let d = v.dialog.as_ref().unwrap();
                    assert_eq!(d.buttons.iter().find(|b| b.primary).unwrap().enabled, on, "{note}: primary button");
                }
                if let Some(active) = step["expect"]["active"].as_str() {
                    assert!(v.nav.iter().any(|n| n.id == active && n.active), "{note}: active list");
                }
                if let Some(report) = step["expect"]["report"].as_bool() {
                    assert_eq!(v.report.is_some(), report, "{note}: report");
                }
                compared += 1;
            }
            _ => {}
        }
    }
    assert!(crate::drift::unknown(&drift).is_empty(), "the views the builders make differ from the fixture:\n{}", crate::drift::unknown(&drift).join("\n"));
    for want in ["idle", "analyzing", "projects", "worktrees", "sessions", "confirm", "shortfall", "cleaning", "report", "cancelled"] {
        assert!(seen.contains(want), "developers:{want} was compared with the fixture (compared: {seen:?})");
    }
    assert!(compared >= 20, "most of the script's intents were compared: {compared}");
    assert_eq!((landed, finished, cancelled), (2, 1, 1), "scans landed, cleanups finished, cleanups cancelled");
}
