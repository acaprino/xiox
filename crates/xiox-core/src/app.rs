//! The whole application behind the window: the three section states, the section on screen and
//! the revision counter, with one entry per thing the page can ask.
//!
//! `Core` is the only door between the page and the engine. The page sends typed intents (never a
//! path, never a command line); each section state decides what it accepts (`refused` with an
//! English reason otherwise) and `Core` only maps the intent to the state's method, polls the
//! section first (a worker that ended since the last view is seen BEFORE the intent is judged:
//! a confirmation never runs on stale state) and answers with the section's new view.
//!
//! `Core` is not thread-safe by itself: the Tauri crate keeps it behind a `Mutex`, and the view
//! pump (`pump.rs`) locks it to build the views the workers' wake-ups ask for.

use crate::dev::{self, DevState, NO_DIALOG};
use crate::notify::{SectionId, SharedNotifier};
use crate::regstate::RegistryState;
use crate::cleanup::{CleanView, CleanupState};
use crate::view::*;
use crate::{rules, sys};
use std::path::PathBuf;

/// How the application starts.
pub struct CoreOptions {
    /// The version shown in About.
    pub version: String,
    /// The process runs as administrator.
    pub admin: bool,
    /// `--only-root <dir>`: Developers looks only in this folder and keeps its settings in memory.
    pub only_root: Option<PathBuf>,
    /// Load the cleaning rules (Winapp2 and the embedded ones) in a thread at the start.
    pub load_rules: bool,
    /// Also update Winapp2 in the background (`rules::should_update`).
    pub update_rules: bool,
}

pub struct Core {
    pub cleanup: CleanupState,
    pub registry: RegistryState,
    pub dev: DevState,
    admin: bool,
    /// (account of the process, account of the Windows session) when they differ.
    mismatch: Option<(String, String)>,
    version: String,
    /// The section the page shows (leaving Developers frees its analysis).
    shown: SectionId,
    rev: u64,
    notifier: SharedNotifier,
}

impl Core {
    /// The application as it starts. With `load_rules` the rules thread starts here.
    pub fn new(o: CoreOptions, notifier: SharedNotifier) -> Core {
        let dev = match &o.only_root {
            Some(dir) => DevState::only_root(dir.clone(), dev::Section::Projects),
            None => DevState::new(dev::Section::Projects),
        };
        let (load, update) = (o.load_rules, o.update_rules);
        let mut core = Core::with_states(o, notifier.clone(), CleanupState::new(), RegistryState::new(), dev);
        core.mismatch = sys::profile_mismatch();
        if load {
            core.cleanup.start_rules(notifier, false, update);
        }
        core
    }

    /// The application over states made by the caller (tests: their workers never reach the engine).
    pub fn with_states(
        o: CoreOptions,
        notifier: SharedNotifier,
        cleanup: CleanupState,
        registry: RegistryState,
        dev: DevState,
    ) -> Core {
        Core {
            cleanup,
            registry,
            dev,
            admin: o.admin,
            mismatch: None,
            version: o.version,
            shown: SectionId::Cleanup,
            rev: 0,
            notifier,
        }
    }

    /// What the window needs before it draws anything. `fake`: the page runs on fixtures.
    pub fn app_info(&self, fake: bool) -> AppInfo {
        AppInfo {
            version: self.version.clone(),
            admin: self.admin,
            ram_total_bytes: sys::memory().total,
            profile_mismatch: self.mismatch.as_ref().map(|(p, s)| {
                format!("Xiox is cleaning the profile of “{p}”, not that of “{s}”, who opened this Windows session.")
            }),
            fake,
        }
    }

    /// The About window: the name and the version of the rules in use, and who they come from.
    pub fn about(&self) -> AboutView {
        let winapp2 = match self.cleanup.rules.as_ref() {
            None => "Winapp2: loading".to_string(),
            Some(rs) => match &rs.winapp2_version {
                Some(v) => format!("Winapp2: version {v}"),
                None => "Winapp2: not downloaded yet".to_string(),
            },
        };
        AboutView { version: self.version.clone(), winapp2, attribution: rules::ABOUT_RULES.to_string() }
    }

    /// The page opens a section. Leaving Developers frees its analysis (it can be large).
    pub fn show(&mut self, s: SectionId) -> SectionView {
        if self.shown == SectionId::Developers && s != SectionId::Developers {
            self.dev.release();
        }
        self.shown = s;
        self.view(s)
    }

    /// The section's view now: its workers are polled first, and it gets the next revision.
    pub fn view(&mut self, s: SectionId) -> SectionView {
        self.poll(s);
        let mut v = match s {
            SectionId::Cleanup => self.cleanup.view(self.admin),
            SectionId::Registry => self.registry.view(),
            SectionId::Developers => self.dev.view(self.admin),
        };
        self.rev += 1;
        v.rev = self.rev;
        v
    }

    fn poll(&mut self, s: SectionId) {
        match s {
            SectionId::Cleanup => self.cleanup.poll(),
            SectionId::Registry => self.registry.poll(),
            SectionId::Developers => self.dev.poll(),
        }
    }

    // ---------- intents (page -> Rust) ----------

    pub fn cleanup(&mut self, i: CleanupIntent) -> Result<SectionView, String> {
        self.poll(SectionId::Cleanup);
        let n = self.notifier.clone();
        let st = &mut self.cleanup;
        match i {
            CleanupIntent::Toggle { row } => st.toggle(&row),
            CleanupIntent::SelectAll { on } => st.select_all(on),
            CleanupIntent::SetOnlyOld { on } => st.set_only_old(on),
            CleanupIntent::Search { text } => {
                st.set_search(&text);
                Ok(())
            }
            CleanupIntent::Analyze => st.analyze(n),
            CleanupIntent::OpenConfirm => st.request_clean(),
            // `confirm` can return Ok(()) when nothing started (the analysis went stale and the
            // warning is on the view): the caller always gets the view and reads it.
            CleanupIntent::Confirm => st.confirm(n),
            // Over the confirmation «Cancel» closes it; over a report «Close» frees the report
            // (the analysis is gone with it); anything else has no dialog to close.
            CleanupIntent::CloseDialog if st.confirm_clean => {
                st.close_confirm();
                Ok(())
            }
            CleanupIntent::CloseDialog if st.clean_view == CleanView::Cleaned && !st.busy() => {
                st.release();
                Ok(())
            }
            CleanupIntent::CloseDialog => Err(NO_DIALOG.into()),
            CleanupIntent::Cancel => st.cancel_job(),
            CleanupIntent::OpenKeepList => st.open_keep_list(),
        }?;
        Ok(self.view(SectionId::Cleanup))
    }

    pub fn registry(&mut self, i: RegistryIntent) -> Result<SectionView, String> {
        self.poll(SectionId::Registry);
        let n = self.notifier.clone();
        let st = &mut self.registry;
        match i {
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
        }?;
        Ok(self.view(SectionId::Registry))
    }

    pub fn developers(&mut self, i: DevIntent) -> Result<SectionView, String> {
        self.poll(SectionId::Developers);
        let n = self.notifier.clone();
        let admin = self.admin;
        let st = &mut self.dev;
        match i {
            DevIntent::Select { category } => st.select_category(category),
            DevIntent::Toggle { row } => st.toggle(&row),
            DevIntent::SelectAll { on } => st.select_all(on),
            DevIntent::SetInactiveDays { days } => st.set_inactive_days(days),
            DevIntent::SetRemoveMode { mode } => st.set_remove_mode(mode.into()),
            DevIntent::SetScanRoots { on } => st.set_scan_roots(on),
            DevIntent::SelectRoot { index } => st.select_root(index),
            DevIntent::RemoveRoot => st.remove_root(),
            DevIntent::Analyze => st.analyze(n, admin),
            DevIntent::OpenConfirm => st.request_clean(),
            DevIntent::SetAck { on } => st.set_ack(on),
            DevIntent::Confirm => st.confirm(n, false),
            DevIntent::Permanent => st.confirm(n, true),
            DevIntent::CloseDialog if st.confirm.is_some() => {
                st.close_confirm();
                Ok(())
            }
            // Over a report (finished or cancelled) «Close» frees the section: the analysis is
            // not brought back.
            DevIntent::CloseDialog if st.view == dev::View::Cleaned && !st.busy() => {
                st.release();
                Ok(())
            }
            DevIntent::CloseDialog => Err(NO_DIALOG.into()),
            DevIntent::Cancel => st.cancel(),
        }?;
        Ok(self.view(SectionId::Developers))
    }

    /// A folder for Developers to analyze too. The folder comes from the picker the command opens in
    /// Rust; the page never names one.
    pub fn add_root(&mut self, dir: PathBuf) -> Result<SectionView, String> {
        self.poll(SectionId::Developers);
        self.dev.add_root(dir)?;
        Ok(self.view(SectionId::Developers))
    }

    // ---------- reports ----------

    /// The text «Save as text…» writes: the report of the section, when there is one to save.
    pub fn report_text(&self, s: SectionId) -> Option<String> {
        match s {
            SectionId::Cleanup => self.cleanup.report_text(),
            SectionId::Developers => self.dev.report_text(),
            SectionId::Registry => None,
        }
    }

    /// What the save dialog and the write came to; the report on screen says it.
    pub fn set_saved(&mut self, s: SectionId, text: String, ok: bool) -> SectionView {
        match s {
            SectionId::Cleanup => self.cleanup.set_saved(text, ok),
            SectionId::Developers => self.dev.set_saved(text, ok),
            SectionId::Registry => {}
        }
        self.view(s)
    }

    /// [`Core::set_saved`] only if the report on screen is still `serialized`, the one that was
    /// written: the page may have closed it or started another job while the dialog was open.
    /// Compared and applied under the caller's one lock; otherwise the current view, no notice.
    pub fn set_saved_if(&mut self, s: SectionId, serialized: &str, text: String, ok: bool) -> SectionView {
        if self.report_text(s).as_deref() == Some(serialized) {
            self.set_saved(s, text, ok)
        } else {
            self.view(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::{CleanEnd, CleanJob, CleanMsg, CleanupWorkers};
    use crate::dev::{DevCleanJob, DevMsg, DevWorkers};
    use crate::notify::{NoNotifier, Outbox};
    use crate::view::testing::wait;
    use crate::view::RemoveModeView;
    use std::sync::Arc;

    fn options() -> CoreOptions {
        CoreOptions { version: "9.9.9".into(), admin: false, only_root: None, load_rules: false, update_rules: false }
    }

    // Workers that never reach the engine.
    fn job_quiet(_: CleanJob, out: Outbox<CleanMsg>) {
        out.send(CleanMsg::Done);
    }
    fn scan_never(_: xiox_model::Settings, _: bool, _: Outbox<DevMsg>) {}
    fn dev_clean_never(_: DevCleanJob, _: Outbox<DevMsg>) {}

    fn dev_state() -> DevState {
        let mut dev = DevState::with_settings(xiox_app::default_settings(), dev::Section::Projects)
            .with_workers(DevWorkers { scan: scan_never, clean: dev_clean_never });
        dev.persist = false; // a test never writes the real settings file
        dev
    }

    fn core() -> Core {
        Core::with_states(
            options(),
            Arc::new(NoNotifier),
            CleanupState::with(Vec::new(), Vec::new(), CleanupWorkers { job: job_quiet }),
            RegistryState::new(),
            dev_state(),
        )
    }

    #[test]
    fn leaving_developers_releases_the_analysis() {
        let mut c = core();
        c.show(SectionId::Developers);
        c.dev.set_scan(xiox_app::DevScan::default());
        assert!(c.dev.scan.is_some() && c.dev.view == dev::View::Scanned);
        // Another section: the analysis goes; the same section again: it stays.
        c.show(SectionId::Developers);
        assert!(c.dev.scan.is_some());
        c.show(SectionId::Registry);
        assert!(c.dev.scan.is_none() && c.dev.view == dev::View::Empty);
        // Showing Developers again frees nothing.
        c.show(SectionId::Developers);
        c.dev.set_scan(xiox_app::DevScan::default());
        c.show(SectionId::Developers);
        assert!(c.dev.scan.is_some());
    }

    #[test]
    fn views_carry_increasing_revs() {
        let mut c = core();
        let mut last = 0;
        for s in [SectionId::Cleanup, SectionId::Cleanup, SectionId::Registry, SectionId::Developers, SectionId::Cleanup] {
            let v = c.view(s);
            assert_eq!(v.section, s);
            assert!(v.rev > last, "rev {} after {last}", v.rev);
            last = v.rev;
        }
        let shown = c.show(SectionId::Registry);
        assert!(shown.rev > last);
        let answered = c.registry(RegistryIntent::ToggleCategory { index: 0 }).unwrap();
        assert!(answered.rev > shown.rev, "an intent's answer is a new revision too");
    }

    #[test]
    fn about_names_winapp2_and_the_licence() {
        let mut c = core();
        assert_eq!(c.about().winapp2, "Winapp2: loading");
        let rs = |v: Option<&str>| rules::RuleSet {
            cleaners: Vec::new(),
            winapp2_version: v.map(str::to_string),
            status: String::new(),
            warnings: Vec::new(),
            env: xiox_rules::Env::for_rules(),
            sections: Vec::new(),
            groups: Vec::new(),
        };
        c.cleanup.rules = Some(rs(None));
        assert_eq!(c.about().winapp2, "Winapp2: not downloaded yet");
        c.cleanup.rules = Some(rs(Some("260915")));
        let about = c.about();
        assert_eq!(about.winapp2, "Winapp2: version 260915");
        assert_eq!(about.version, "9.9.9");
        assert!(about.attribution.contains("Winapp2") && about.attribution.contains("CC-BY-SA-4.0"), "{}", about.attribution);
        // Builder-vs-fixture: the window the page draws from the fixture must say what this says (the
        // licence line is mandatory; a fixture that lied about it once ended up in a screenshot).
        let doc = crate::drift::document();
        assert_eq!(crate::drift::norm(&about.winapp2), crate::drift::norm(doc["about"]["winapp2"].as_str().unwrap()), "About: the Winapp2 line");
        assert_eq!(about.attribution, doc["about"]["attribution"].as_str().unwrap(), "About: the attribution");
        // The window's own facts.
        let info = c.app_info(true);
        assert_eq!((info.version.as_str(), info.admin, info.fake, info.profile_mismatch.clone()), ("9.9.9", false, true, None));
        assert!(info.ram_total_bytes > 0);
        c.mismatch = Some(("Admin".into(), "alfio".into()));
        assert_eq!(
            c.app_info(false).profile_mismatch.as_deref(),
            Some("Xiox is cleaning the profile of “Admin”, not that of “alfio”, who opened this Windows session.")
        );
    }

    #[test]
    fn refusals_are_english() {
        let mut c = core();
        let e = c.developers(DevIntent::Confirm).unwrap_err();
        assert_eq!(e, NO_DIALOG);
        let e = c.cleanup(CleanupIntent::Toggle { row: "x".into() }).unwrap_err();
        assert!(!e.contains('«') && !e.contains('»') && e.contains('x'), "{e}");
        for e in [
            c.cleanup(CleanupIntent::Confirm).unwrap_err(),
            c.cleanup(CleanupIntent::CloseDialog).unwrap_err(),
            c.registry(RegistryIntent::Fix { backup: true }).unwrap_err(),
            c.registry(RegistryIntent::CloseDialog).unwrap_err(),
            c.developers(DevIntent::CloseDialog).unwrap_err(),
            c.developers(DevIntent::Permanent).unwrap_err(),
        ] {
            assert!(!e.is_empty() && !e.contains('«'), "{e}");
        }
    }

    /// An intent no state would have allowed changes nothing: the answer is an error, and the
    /// views are those of before.
    #[test]
    fn a_refused_intent_changes_nothing() {
        let mut c = core();
        let before = (c.view(SectionId::Cleanup).rows.len(), c.cleanup.clean_sel.clone());
        assert!(c.cleanup(CleanupIntent::Confirm).is_err());
        assert!(c.cleanup(CleanupIntent::OpenConfirm).is_err(), "no analysis on screen");
        assert!(c.cleanup(CleanupIntent::Toggle { row: "native:nothing".into() }).is_err());
        assert_eq!((c.view(SectionId::Cleanup).rows.len(), c.cleanup.clean_sel.clone()), before);
        assert!(c.cleanup.clean_job.is_none() && !c.cleanup.confirm_clean);
    }

    /// Over a confirmation `closeDialog` closes it, over a report it frees it, and with neither it is
    /// refused: no other way to leave a report, and never a start.
    #[test]
    fn close_dialog_closes_the_confirmation_frees_a_report_and_otherwise_refuses() {
        let mut c = core();
        c.cleanup.clean_sel = (0..c.cleanup.clean_sel.len()).map(|i| i == 0).collect();
        assert!(c.cleanup(CleanupIntent::CloseDialog).is_err());
        c.cleanup(CleanupIntent::Analyze).unwrap();
        wait(|| c.view(SectionId::Cleanup).job.is_none());
        assert!(c.cleanup(CleanupIntent::CloseDialog).is_err(), "an analysis is not a dialog");
        // The confirmation: closing it starts nothing.
        let v = c.cleanup(CleanupIntent::OpenConfirm).unwrap();
        assert!(v.dialog.is_some());
        let v = c.cleanup(CleanupIntent::CloseDialog).unwrap();
        assert!(v.dialog.is_none() && v.job.is_none() && v.report.is_none());
        // Confirm again, then the report goes with «Close» and the analysis with it.
        c.cleanup(CleanupIntent::OpenConfirm).unwrap();
        c.cleanup(CleanupIntent::Confirm).unwrap();
        wait(|| c.view(SectionId::Cleanup).job.is_none());
        assert_eq!(c.cleanup.clean_view, CleanView::Cleaned);
        assert_eq!(c.cleanup.clean_end, Some(CleanEnd::Done));
        let v = c.cleanup(CleanupIntent::CloseDialog).unwrap();
        assert!(v.report.is_none() && v.dialog.is_none());
        assert_eq!(c.cleanup.clean_view, CleanView::Empty);
        assert!(c.cleanup(CleanupIntent::CloseDialog).is_err(), "nothing left to close");
        assert!(c.cleanup(CleanupIntent::Confirm).is_err(), "and no confirmation to confirm");
    }

    /// A confirmation whose analysis went stale while the dialog was open does not clean: the state
    /// closes it and puts the warning on screen, and `Core` still answers with the view (`Ok`), never
    /// with an error: the page reads the warning from the view.
    #[test]
    fn a_stale_confirm_returns_the_view_with_the_warning_and_starts_nothing() {
        let mut c = core();
        let now = xiox_app::now_ms();
        let analysed_at = now - (xiox_app::MAX_SCAN_AGE_MS as i64) - 60_000;
        let mut scan = crate::dev::intent_tests::three_sections();
        scan.scanned_at_ms = analysed_at;
        c.dev.persist = true; // `set_scan` empties a scan when it does not persist; the state stops saving after
        c.dev.set_scan(scan);
        c.dev.persist = false;
        // The dialog opened when the analysis was fresh and has been left open since.
        c.dev.open_confirm_with(analysed_at + 1000, |_| Vec::new());
        assert!(c.dev.confirm.is_some(), "the dialog is open");
        let v = c.developers(DevIntent::Confirm).expect("nothing started, and that is not an error");
        assert!(v.dialog.is_none() && v.job.is_none() && v.report.is_none(), "no dialog, no job, no report");
        assert!(v.notices.iter().any(|n| n.tone == Tone::Warn && n.text == dev::STALE_SCAN), "the warning is on the view: {:?}", v.notices);
        assert!(!c.dev.busy() && c.dev.cleaned.is_none(), "nothing ran");
    }

    /// A worker that ended since the last view is seen BEFORE the intent is judged: `OpenConfirm` right
    /// after an analysis landed must not be refused as busy, and a dead job must not be confirmed.
    #[test]
    fn an_intent_sees_what_the_workers_did_since_the_last_view() {
        let mut c = core();
        c.cleanup.clean_sel = (0..c.cleanup.clean_sel.len()).map(|i| i == 0).collect();
        // Start through the state (no view, so nobody polls) and let the quiet worker end.
        c.cleanup.analyze(Arc::new(NoNotifier)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(c.cleanup.busy(), "nobody has polled: the state still thinks the job runs");
        let v = c.cleanup(CleanupIntent::OpenConfirm).expect("the intent is judged on the state the workers left");
        assert!(v.dialog.is_some());
    }

    #[test]
    fn nothing_can_be_saved_where_there_is_no_report() {
        let mut c = core();
        for s in [SectionId::Cleanup, SectionId::Registry, SectionId::Developers] {
            assert_eq!(c.report_text(s), None, "{s:?}");
        }
        // The outcome of a save is shown by the section that has a report; the others ignore it.
        let v = c.set_saved(SectionId::Registry, "Report saved to x".into(), true);
        assert!(v.report.is_none());
    }

    /// Polls the Developers view until `done` holds (the real engine runs in worker threads).
    fn developers_until(c: &mut Core, what: &str, done: impl Fn(&SectionView) -> bool) -> SectionView {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let v = c.view(SectionId::Developers);
            if done(&v) {
                return v;
            }
            assert!(std::time::Instant::now() < until, "within 120 s: {what}");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// The real engine on a temp tree, end to end: analyze, choose the build folder, confirm, clean.
    /// It reads the session stores read-only (the scan looks at them; `only_root` then keeps only what
    /// is under the temp tree) and deletes only inside its own temp tree, for good: never the real
    /// Recycle Bin. `cargo test -p xiox-core app::developers_clean_a_temp_project_end_to_end -- --ignored`
    #[test]
    #[ignore = "real engine on a temp tree; reads the session stores read-only"]
    fn developers_clean_a_temp_project_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let project = root.join("proj");
        let target = project.join("target");
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]
name = \"proj\"
version = \"0.1.0\"
").unwrap();
        std::fs::write(target.join("debug").join("blob.bin"), vec![7u8; 1 << 20]).unwrap();

        let mut c = Core::new(
            CoreOptions { version: "t".into(), admin: false, only_root: Some(root.clone()), load_rules: false, update_rules: false },
            Arc::new(NoNotifier),
        );
        // For good, whatever the Recycle Bin holds: nothing of this test goes into the real one.
        c.developers(DevIntent::SetRemoveMode { mode: RemoveModeView::Permanent }).unwrap();
        let build_row = |v: &SectionView| {
            v.rows.iter().find(|r| r.id.starts_with("a:") && r.detail.iter().any(|(k, folder)| k == "folder" && folder.ends_with("target"))).cloned()
        };
        c.developers(DevIntent::Analyze).unwrap();
        let mut v = developers_until(&mut c, "the analysis ends", |v| v.job.is_none());
        if build_row(&v).is_none() {
            // The project is not recognised without a repository: make one and look again.
            let git = std::process::Command::new("git").arg("init").arg("-q").arg(&project).status();
            assert!(git.is_ok_and(|s| s.success()), "git init on the temp project");
            c.developers(DevIntent::Analyze).unwrap();
            v = developers_until(&mut c, "the second analysis ends", |v| v.job.is_none());
        }
        let row = build_row(&v).unwrap_or_else(|| panic!("the build folder is listed: {:?}", v.rows.iter().map(|r| &r.id).collect::<Vec<_>>()));
        if row.check != Check::On {
            c.developers(DevIntent::Toggle { row: row.id.clone() }).unwrap();
        }
        let v = c.developers(DevIntent::OpenConfirm).expect("Clean opens its confirmation");
        let dialog = v.dialog.expect("the confirmation is open");
        if dialog.ack.is_some() {
            c.developers(DevIntent::SetAck { on: true }).unwrap();
        }
        c.developers(DevIntent::Confirm).expect("the confirmation starts the clean");
        let v = developers_until(&mut c, "a report", |v| v.job.is_none() && v.report.is_some());
        let report = v.report.unwrap();
        assert!(report.freed_bytes >= 1 << 20, "freed {} bytes", report.freed_bytes);
        assert!(!target.exists(), "the build folder is gone");
        assert!(project.join("Cargo.toml").exists(), "the project stays");
    }
}
