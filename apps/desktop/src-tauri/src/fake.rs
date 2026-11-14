//! The backend of the debug window on fixtures (`XIOX_UI_FAKE=1`, debug builds only): a Rust port of
//! the state machine in the header of `apps/desktop/src/mockmodel.ts`, served over
//! `apps/desktop/fixtures/views.json`. It never reaches the engine: no state of the engine, no
//! worker, no file, no system call. `apps/desktop/fixtures/intent-script.json` is replayed against it
//! by the tests, as the TypeScript mock replays it.
//!
//! A VIEW is a fixture state (`cleanup:analyzed`...). The model holds, per section, the name of the
//! current state, a revision counter and a few memories. Fixture states are fixed copies: a toggle
//! or an ack changes the view it was made in and nothing else.

use crate::backend::Backend;
use crate::error::CmdResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use xiox_core::notify::SectionId;
use xiox_core::pump::ViewSink;
use xiox_core::view::*;

/// The fixture file the page's mock serves too.
const FIXTURE: &str = include_str!("../../fixtures/views.json");

/// How often a running fake job moves (the real engine wakes the page when its worker speaks).
const TICK_MS: u64 = 200;
/// Ticks a fake job shows its fraction for before it lands (`TICKS` of the mock).
const TICKS: u32 = 3;

const SECTIONS: [SectionId; 4] = [SectionId::Cleanup, SectionId::Registry, SectionId::Services, SectionId::Developers];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub app: AppInfo,
    pub about: AboutView,
    pub views: BTreeMap<SectionId, BTreeMap<String, SectionView>>,
}

// ---------- the tables of the machine (mockmodel.ts header) ----------

fn initial(s: SectionId) -> &'static str {
    match s {
        SectionId::Cleanup | SectionId::Registry | SectionId::Developers => "idle",
        SectionId::Services => "loaded",
    }
}

/// Where `cancel` and `closeDialog` fall back when nothing was recorded.
fn home(s: SectionId) -> &'static str {
    match s {
        SectionId::Cleanup | SectionId::Registry => "analyzed",
        SectionId::Services => "loaded",
        SectionId::Developers => "projects",
    }
}

/// The states a dialog can be opened from.
fn analyzed(s: SectionId) -> &'static [&'static str] {
    match s {
        SectionId::Cleanup => &["analyzed", "interrupted", "browser", "applications"],
        SectionId::Registry => &["analyzed"],
        SectionId::Services => &["loaded"],
        SectionId::Developers => &["projects", "worktrees", "sessions"],
    }
}

/// The Cleanup categories that have a state of their own.
fn category_state(category: &str) -> Option<&'static str> {
    match category {
        "rules:Browser" => Some("browser"),
        "winapp2" => Some("applications"),
        _ => None,
    }
}

const CATEGORY_STATES: [&str; 2] = ["browser", "applications"];
const DEV_CATEGORIES: [&str; 3] = ["projects", "worktrees", "sessions"];

/// The state a busy state lands on after its ticks; `None`: the state is not busy.
fn lands(s: SectionId, state: &str) -> Option<&'static str> {
    match (s, state) {
        (SectionId::Cleanup, "analyzing") => Some("analyzed"),
        (SectionId::Cleanup, "cleaning") => Some("report"),
        (SectionId::Registry, "scanning") => Some("analyzed"),
        (SectionId::Registry, "fixing") => Some("fixed"),
        (SectionId::Services, "loading") => Some("loaded"),
        (SectionId::Developers, "analyzing") => Some("projects"),
        (SectionId::Developers, "cleaning") => Some("report"),
        _ => None,
    }
}

fn analyze_busy(s: SectionId) -> &'static str {
    match s {
        SectionId::Cleanup | SectionId::Developers => "analyzing",
        SectionId::Registry => "scanning",
        SectionId::Services => "loading",
    }
}

/// The intent that starts the section's analysis.
fn analyze_kind(s: SectionId) -> &'static str {
    match s {
        SectionId::Cleanup | SectionId::Developers => "analyze",
        SectionId::Registry => "scan",
        SectionId::Services => "refresh",
    }
}

/// Where a report is closed to (the section is released): `None` = Home.
fn empty(s: SectionId) -> Option<&'static str> {
    match s {
        SectionId::Cleanup | SectionId::Developers => Some("idle"),
        _ => None,
    }
}

/// Where `cancel` goes from a busy state: `Some(Some(state))`, `Some(None)` = that job cannot be
/// cancelled (refused), `None` = back to where the job started.
fn cancel_to(s: SectionId, state: &str) -> Option<Option<&'static str>> {
    match (s, state) {
        (SectionId::Cleanup, "analyzing") => Some(Some("interrupted")),
        (SectionId::Cleanup, "cleaning") => Some(Some("cancelled")),
        (SectionId::Developers, "analyzing") => Some(None),
        (SectionId::Developers, "cleaning") => Some(Some("cancelled")),
        _ => None,
    }
}

fn open_kind(s: SectionId) -> &'static str {
    match s {
        SectionId::Registry => "openFix",
        _ => "openConfirm",
    }
}

fn dialog_state(s: SectionId) -> &'static str {
    match s {
        SectionId::Registry => "backup",
        _ => "confirm",
    }
}

fn notice_text(action: &str) -> &'static str {
    match action {
        "manual" => "Selected services set to Manual.",
        "restore" => "Original startup types restored.",
        _ => "Selected services disabled.",
    }
}

/// What it takes to run: the dialog state, the dialog button and the busy state it starts
/// (`None`: Services applies the change at once).
struct Run {
    from: &'static str,
    button: &'static str,
    busy: Option<&'static str>,
}

fn run_rule(s: SectionId, kind: &str, intent: &Value) -> Option<Run> {
    match (s, kind) {
        (SectionId::Cleanup, "confirm") => Some(Run { from: "confirm", button: "continue", busy: Some("cleaning") }),
        (SectionId::Services, "confirm") => Some(Run { from: "confirm", button: "continue", busy: None }),
        (SectionId::Registry, "fix") => {
            let backup = intent["backup"].as_bool().unwrap_or(false);
            Some(Run { from: "backup", button: if backup { "backup" } else { "nobackup" }, busy: Some("fixing") })
        }
        (SectionId::Developers, "confirm") => Some(Run { from: "confirm", button: "clean", busy: Some("cleaning") }),
        (SectionId::Developers, "permanent") => Some(Run { from: "shortfall", button: "permanent", busy: Some("cleaning") }),
        _ => None,
    }
}

// ---------- the model ----------

pub struct FakeModel {
    app: AppInfo,
    about: AboutView,
    /// A working copy: toggles and acks change it, the fixture stays as loaded.
    work: BTreeMap<SectionId, BTreeMap<String, SectionView>>,
    state: BTreeMap<SectionId, String>,
    revs: BTreeMap<SectionId, u64>,
    /// Where `closeDialog` and `cancel` go back to; none = Home.
    back: BTreeMap<SectionId, String>,
    ticks: BTreeMap<SectionId, u32>,
    /// The Cleanup category chosen by `select`, applied to every view whose nav has it.
    active: BTreeMap<SectionId, String>,
    /// The Services action whose dialog is open (decides the notice text).
    pending_action: String,
}

impl FakeModel {
    pub fn new(f: Fixture) -> Result<FakeModel, String> {
        let mut state = BTreeMap::new();
        for s in SECTIONS {
            let name = initial(s);
            if !f.views.get(&s).is_some_and(|v| v.contains_key(name)) {
                return Err(format!("the fixture has no {s:?}:{name}"));
            }
            state.insert(s, name.to_string());
        }
        Ok(FakeModel {
            app: f.app,
            about: f.about,
            work: f.views,
            state,
            revs: SECTIONS.iter().map(|s| (*s, 0)).collect(),
            back: BTreeMap::new(),
            ticks: SECTIONS.iter().map(|s| (*s, 0)).collect(),
            active: BTreeMap::new(),
            pending_action: "disable".into(),
        })
    }

    /// The embedded fixture.
    pub fn load() -> Result<FakeModel, String> {
        let fixture: Fixture = serde_json::from_str(FIXTURE).map_err(|e| format!("the fixture does not read: {e}"))?;
        FakeModel::new(fixture)
    }

    pub fn app_info(&self) -> AppInfo {
        self.app.clone()
    }

    pub fn about(&self) -> AboutView {
        self.about.clone()
    }

    /// Opens a section: its current view (the Services notice does not outlive the section being left).
    pub fn show(&mut self, s: SectionId) -> SectionView {
        if s == SectionId::Services {
            self.clear_notice();
        }
        self.stamp(s)
    }

    pub fn view(&mut self, s: SectionId) -> SectionView {
        self.stamp(s)
    }

    /// The name of the fixture state a section is in.
    pub fn state_of(&self, s: SectionId) -> &str {
        &self.state[&s]
    }

    /// Applies one intent by the transition table and returns the section's new view. An intent that
    /// is refused or does not apply is IGNORED: same state, new rev. `intent` is the JSON the page
    /// sends (the typed intents of the contract, serialized).
    pub fn intent(&mut self, s: SectionId, intent: &Value) -> SectionView {
        self.apply(s, intent);
        self.stamp(s)
    }

    /// What a worker would push on its own: one view for every section whose job moved.
    pub fn tick(&mut self) -> Vec<SectionView> {
        let mut pushed = Vec::new();
        for s in SECTIONS {
            let Some(landing) = lands(s, &self.state[&s]) else { continue };
            let ticks = self.ticks.entry(s).or_insert(0);
            *ticks += 1;
            if *ticks > TICKS {
                self.go_to(s, landing);
            }
            pushed.push(self.stamp(s));
        }
        pushed
    }

    /// Jumps a section to one of its fixture states (the replay's lever, as the mock's `go`).
    #[cfg(test)]
    pub fn go(&mut self, s: SectionId, state: &str) -> SectionView {
        self.go_to(s, state);
        self.back.remove(&s);
        self.stamp(s)
    }

    /// The text `Save as text…` would write: the report on screen, when it can be saved.
    pub fn report_text(&self, s: SectionId) -> Option<String> {
        let report = self.current(s).report.as_ref().filter(|r| r.can_save)?;
        Some(format!("{}\nSample report for the window on fixtures.\n", report.title))
    }

    /// `Save as text…` came to `text`: the report on screen says so (the working copy keeps it until
    /// the report goes).
    pub fn set_saved(&mut self, s: SectionId, text: String, ok: bool) -> SectionView {
        if let Some(report) = self.current_mut(s).report.as_mut().filter(|r| r.can_save) {
            report.saved = Some(SavedView { text, ok });
        }
        self.stamp(s)
    }

    fn current(&self, s: SectionId) -> &SectionView {
        &self.work[&s][&self.state[&s]]
    }

    fn current_mut(&mut self, s: SectionId) -> &mut SectionView {
        let name = self.state[&s].clone();
        self.work.get_mut(&s).and_then(|v| v.get_mut(&name)).expect("the current state is in the fixture")
    }

    fn stamp(&mut self, s: SectionId) -> SectionView {
        let rev = self.revs.entry(s).or_insert(0);
        *rev += 1;
        let rev = *rev;
        let state = self.state[&s].clone();
        let mut view = self.current(s).clone();
        view.rev = rev;
        if let Some(active) = self.active.get(&s) {
            if !(s == SectionId::Cleanup && CATEGORY_STATES.contains(&state.as_str())) && view.nav.iter().any(|n| &n.id == active) {
                for n in view.nav.iter_mut() {
                    n.active = &n.id == active;
                }
            }
        }
        if let Some(job) = view.job.as_mut().filter(|_| lands(s, &state).is_some()) {
            job.fraction = self.ticks[&s].min(TICKS) as f32 / TICKS as f32;
        }
        view
    }

    fn is_busy(&self, s: SectionId) -> bool {
        lands(s, &self.state[&s]).is_some()
    }

    /// A job runs, a dialog is open or a report is on screen: selection and new jobs are refused.
    fn frozen(&self, s: SectionId) -> bool {
        let view = self.current(s);
        self.is_busy(s) || view.dialog.is_some() || view.report.is_some()
    }

    fn go_to(&mut self, s: SectionId, state: &str) {
        assert!(self.work[&s].contains_key(state), "no state {state} for {s:?}");
        self.state.insert(s, state.to_string());
        self.ticks.insert(s, 0);
        // Arriving on a report: it starts unsaved.
        if let Some(report) = self.current_mut(s).report.as_mut() {
            report.saved = None;
        }
    }

    fn clear_notice(&mut self) {
        if let Some(loaded) = self.work.get_mut(&SectionId::Services).and_then(|v| v.get_mut(home(SectionId::Services))) {
            loaded.notices.clear();
        }
    }

    fn action_enabled(&self, s: SectionId, id: &str) -> bool {
        self.current(s).actions.iter().any(|a| a.id == id && a.enabled)
    }

    fn apply(&mut self, s: SectionId, intent: &Value) {
        let here = self.state[&s].clone();
        let kind = intent["kind"].as_str().unwrap_or("");
        if kind == analyze_kind(s) {
            if self.is_busy(s) || self.current(s).dialog.is_some() {
                return; // a report does not stop it
            }
            self.back.insert(s, here);
            if s == SectionId::Services {
                self.clear_notice();
            }
            self.go_to(s, analyze_busy(s));
        } else if kind == "select" && (s == SectionId::Cleanup || s == SectionId::Developers) {
            if self.frozen(s) {
                return;
            }
            let category = intent["category"].as_str().unwrap_or("");
            if s == SectionId::Developers {
                if DEV_CATEGORIES.contains(&here.as_str()) && DEV_CATEGORIES.contains(&category) {
                    self.go_to(s, category);
                }
            } else if self.current(s).nav.iter().any(|n| n.id == category) {
                match category_state(category) {
                    Some(own) => {
                        if analyzed(SectionId::Cleanup).contains(&here.as_str()) {
                            self.go_to(s, own);
                        }
                    }
                    None => {
                        self.active.insert(s, category.to_string());
                        if CATEGORY_STATES.contains(&here.as_str()) {
                            self.go_to(s, home(SectionId::Cleanup));
                        }
                    }
                }
            }
        } else if kind == "toggle" {
            if !self.frozen(s) {
                self.toggle(s, intent["row"].as_str().unwrap_or(""));
            }
        } else if kind == open_kind(s) {
            let action = match s {
                SectionId::Services => intent["action"].as_str().unwrap_or("").to_string(),
                SectionId::Registry => "fix".to_string(),
                _ => "clean".to_string(),
            };
            if !analyzed(s).contains(&here.as_str()) || !self.action_enabled(s, &action) {
                return;
            }
            self.back.insert(s, here);
            if s == SectionId::Services {
                self.clear_notice();
                self.pending_action = action;
            }
            self.go_to(s, dialog_state(s));
            self.reset_ack(s);
        } else if kind == "setAck" && s == SectionId::Developers {
            self.set_ack(intent["on"].as_bool().unwrap_or(false));
        } else if run_rule(s, kind, intent).is_some() {
            self.run(s, kind, intent);
        } else if kind == "closeDialog" {
            let view = self.current(s);
            if view.dialog.is_none() && view.report.is_none() {
                return;
            }
            if view.report.is_some() {
                // A report is closed by releasing the section: the analysis is gone, the section is empty.
                self.back.remove(&s);
                self.go_to(s, empty(s).unwrap_or(home(s)));
                return;
            }
            let to = self.back.get(&s).cloned().unwrap_or_else(|| home(s).to_string());
            self.go_to(s, &to);
        } else if kind == "cancel" {
            if !self.is_busy(s) {
                return;
            }
            match cancel_to(s, &here) {
                Some(None) => {} // a scan cannot be cancelled
                Some(Some(to)) => {
                    self.back.remove(&s); // the analysis is dropped: nothing to go back to
                    self.go_to(s, to);
                }
                None => {
                    let to = self.back.get(&s).cloned().unwrap_or_else(|| home(s).to_string());
                    self.go_to(s, &to);
                }
            }
        }
    }

    fn toggle(&mut self, s: SectionId, id: &str) {
        let Some(row) = self.current_mut(s).rows.iter_mut().find(|r| r.id == id) else { return };
        if !row.enabled || row.check == Check::None {
            return;
        }
        row.check = if row.check == Check::On { Check::Off } else { Check::On };
    }

    fn set_ack(&mut self, on: bool) {
        let Some(dialog) = self.current_mut(SectionId::Developers).dialog.as_mut() else { return };
        if dialog.ack.is_none() {
            return;
        }
        dialog.ack = Some(on);
        if let Some(primary) = dialog.buttons.iter_mut().find(|b| b.primary) {
            primary.enabled = on;
        }
    }

    /// A dialog that opens again must not start pre-checked.
    fn reset_ack(&mut self, s: SectionId) {
        let Some(dialog) = self.current_mut(s).dialog.as_mut() else { return };
        if dialog.ack.is_none() {
            return;
        }
        dialog.ack = Some(false);
        if let Some(primary) = dialog.buttons.iter_mut().find(|b| b.primary) {
            primary.enabled = false;
        }
    }

    fn run(&mut self, s: SectionId, kind: &str, intent: &Value) {
        let Some(rule) = run_rule(s, kind, intent) else { return };
        if self.state[&s] != rule.from {
            return;
        }
        let allowed = self.current(s).dialog.as_ref().is_some_and(|d| {
            d.buttons.iter().any(|b| b.id == rule.button && b.enabled) && d.ack != Some(false)
        });
        if !allowed {
            return;
        }
        match rule.busy {
            None => {
                // Services: the change is applied at once and the list comes back with a notice.
                let text = notice_text(&self.pending_action).to_string();
                if let Some(loaded) = self.work.get_mut(&s).and_then(|v| v.get_mut(home(s))) {
                    loaded.notices = vec![NoticeView { tone: Tone::Ok, text, more: Vec::new() }];
                }
                self.go_to(s, home(s));
            }
            // `back` stays what it was when the dialog opened; a cancelled clean forgets it.
            Some(busy) => self.go_to(s, busy),
        }
    }
}

// ---------- the backend ----------

pub struct FakeBackend {
    model: Mutex<FakeModel>,
    sink: Arc<dyn ViewSink>,
}

impl FakeBackend {
    /// The fake backend, with its ticker: every 200 ms the running fake jobs move and the views are
    /// pushed to `sink`. The ticker stops when the backend is dropped.
    pub fn start(sink: Arc<dyn ViewSink>) -> Result<Arc<FakeBackend>, String> {
        let backend = Arc::new(FakeBackend { model: Mutex::new(FakeModel::load()?), sink });
        let weak: Weak<FakeBackend> = Arc::downgrade(&backend);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(TICK_MS));
            let Some(b) = weak.upgrade() else { break };
            let views = b.model().tick();
            for v in views {
                b.sink.send(&v);
            }
        });
        Ok(backend)
    }

    fn model(&self) -> MutexGuard<'_, FakeModel> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn intent(&self, s: SectionId, i: &impl Serialize) -> CmdResult<SectionView> {
        let json = serde_json::to_value(i).expect("an intent serializes");
        Ok(self.model().intent(s, &json))
    }
}

impl Backend for FakeBackend {
    fn app_info(&self) -> AppInfo {
        self.model().app_info()
    }

    fn about(&self) -> AboutView {
        self.model().about()
    }

    fn show(&self, s: SectionId) -> SectionView {
        self.model().show(s)
    }

    fn view(&self, s: SectionId) -> SectionView {
        self.model().view(s)
    }

    fn cleanup(&self, i: CleanupIntent) -> CmdResult<SectionView> {
        self.intent(SectionId::Cleanup, &i)
    }

    fn registry(&self, i: RegistryIntent) -> CmdResult<SectionView> {
        self.intent(SectionId::Registry, &i)
    }

    fn services(&self, i: ServicesIntent) -> CmdResult<SectionView> {
        self.intent(SectionId::Services, &i)
    }

    fn developers(&self, i: DevIntent) -> CmdResult<SectionView> {
        self.intent(SectionId::Developers, &i)
    }

    /// The fixtures have no extra folder to add: the view stays as it is.
    fn add_root(&self, _dir: PathBuf) -> CmdResult<SectionView> {
        Ok(self.model().view(SectionId::Developers))
    }

    fn report_text(&self, s: SectionId) -> Option<String> {
        self.model().report_text(s)
    }

    fn set_saved(&self, s: SectionId, text: String, ok: bool) -> SectionView {
        self.model().set_saved(s, text, ok)
    }

    fn is_fake(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model() -> FakeModel {
        FakeModel::load().expect("the embedded fixture loads")
    }

    fn intent(m: &mut FakeModel, s: SectionId, v: Value) -> SectionView {
        m.intent(s, &v)
    }

    /// Every state of the table of `mockmodel.ts`.
    const STATES: [(SectionId, &[&str]); 4] = [
        (
            SectionId::Cleanup,
            &["idle", "analyzing", "analyzed", "interrupted", "browser", "applications", "confirm", "cleaning", "report", "cancelled"],
        ),
        (SectionId::Registry, &["idle", "scanning", "analyzed", "backup", "fixing", "fixed"]),
        (SectionId::Services, &["loading", "loaded", "confirm"]),
        (
            SectionId::Developers,
            &["idle", "analyzing", "projects", "worktrees", "sessions", "confirm", "shortfall", "cleaning", "report", "cancelled"],
        ),
    ];

    /// Where two JSON values first differ, as a path (the message of a failed round trip).
    fn first_difference(a: &Value, b: &Value, at: &str) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, v) in x {
                    match y.get(k) {
                        Some(w) => {
                            if let Some(d) = first_difference(v, w, &format!("{at}/{k}")) {
                                return Some(d);
                            }
                        }
                        None => return Some(format!("{at}/{k} is missing")),
                    }
                }
                y.keys().find(|k| !x.contains_key(*k)).map(|k| format!("{at}/{k} is new"))
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    return Some(format!("{at}: {} items vs {}", x.len(), y.len()));
                }
                x.iter().zip(y).enumerate().find_map(|(i, (v, w))| first_difference(v, w, &format!("{at}/{i}")))
            }
            (Value::Number(x), Value::Number(y)) if !(x.is_f64() && y.is_f64()) && !(x.is_u64() && y.is_u64()) && !(x.is_i64() && y.is_i64()) => {
                // JavaScript writes `0` and `1` where Rust writes `0.0` and `1.0` (a fraction): the same
                // number. Integers (bytes, counts) never go through this: they are equal exactly.
                let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
                ((x - y).abs() > 1e-6).then(|| format!("{at}: {x} vs {y}"))
            }
            _ if a == b => None,
            _ => Some(format!("{at}: {a} vs {b}")),
        }
    }

    #[test]
    fn fixture_round_trips_and_holds_every_state() {
        let original: Value = serde_json::from_str(FIXTURE).unwrap();
        let fixture: Fixture = serde_json::from_value(original.clone()).expect("the fixture reads through the contract");
        let written = serde_json::to_value(&fixture).unwrap();
        let difference = first_difference(&original, &written, "");
        assert!(difference.is_none(), "the typed fixture writes back what it read; first difference: {}", difference.unwrap_or_default());
        for (section, names) in STATES {
            for name in names {
                assert!(fixture.views[&section].contains_key(*name), "{section:?}:{name} is in the fixture");
            }
            for (name, view) in &fixture.views[&section] {
                assert_eq!(view.section, section, "{section:?}:{name}");
            }
        }
        assert!(fixture.app.fake, "the fixture says the window runs on fixtures");
    }

    #[test]
    fn fake_analyze_ticks_then_lands_on_analyzed() {
        let mut m = model();
        let started = intent(&mut m, SectionId::Cleanup, json!({ "kind": "analyze" }));
        assert_eq!(m.state_of(SectionId::Cleanup), "analyzing");
        assert_eq!(started.job.as_ref().map(|j| j.fraction), Some(0.0));
        for third in [1.0_f32 / 3.0, 2.0 / 3.0, 1.0] {
            let pushed = m.tick();
            assert_eq!(pushed.iter().map(|v| v.section).collect::<Vec<_>>(), vec![SectionId::Cleanup]);
            let fraction = pushed[0].job.as_ref().expect("still busy").fraction;
            assert!((fraction - third).abs() < 1e-6, "{fraction} vs {third}");
            assert_eq!(m.state_of(SectionId::Cleanup), "analyzing");
        }
        let landed = m.tick();
        assert_eq!(m.state_of(SectionId::Cleanup), "analyzed");
        assert!(landed[0].job.is_none());
        assert!(m.tick().is_empty(), "nothing runs: nothing is pushed");
    }

    #[test]
    fn fake_cancel_returns_to_the_state_before_the_job() {
        let mut m = model();
        // A job with no state of its own to land on goes back to where it started (the page never sends a
        // `cancel` for Registry or Services: the intent types have none; the table still holds it).
        intent(&mut m, SectionId::Registry, json!({ "kind": "scan" }));
        assert_eq!(m.state_of(SectionId::Registry), "scanning");
        intent(&mut m, SectionId::Registry, json!({ "kind": "cancel" }));
        assert_eq!(m.state_of(SectionId::Registry), "idle");
        // A cleaning ends in the cancelled report, an analysis keeps what it had, a scan cannot stop.
        m.go(SectionId::Developers, "cleaning");
        intent(&mut m, SectionId::Developers, json!({ "kind": "cancel" }));
        assert_eq!(m.state_of(SectionId::Developers), "cancelled");
        intent(&mut m, SectionId::Cleanup, json!({ "kind": "analyze" }));
        intent(&mut m, SectionId::Cleanup, json!({ "kind": "cancel" }));
        assert_eq!(m.state_of(SectionId::Cleanup), "interrupted");
        intent(&mut m, SectionId::Developers, json!({ "kind": "closeDialog" }));
        intent(&mut m, SectionId::Developers, json!({ "kind": "analyze" }));
        intent(&mut m, SectionId::Developers, json!({ "kind": "cancel" }));
        assert_eq!(m.state_of(SectionId::Developers), "analyzing", "a Developers scan is refused");
        // Nothing busy: nothing to cancel.
        m.go(SectionId::Cleanup, "analyzed");
        intent(&mut m, SectionId::Cleanup, json!({ "kind": "cancel" }));
        assert_eq!(m.state_of(SectionId::Cleanup), "analyzed");
    }

    /// The safety rules of the table, on a fixture whose flags are loosened: the MODEL refuses, not
    /// the fixture's buttons (Review Focus 1).
    #[test]
    fn fake_confirm_needs_its_dialog_its_enabled_button_and_the_ack() {
        let mut m = model();
        // No dialog: nothing runs.
        m.go(SectionId::Cleanup, "analyzed");
        intent(&mut m, SectionId::Cleanup, json!({ "kind": "confirm" }));
        assert_eq!(m.state_of(SectionId::Cleanup), "analyzed");
        // The Developers confirmation stays while its ack box is off, even with the button forced on.
        m.go(SectionId::Developers, "projects");
        intent(&mut m, SectionId::Developers, json!({ "kind": "openConfirm" }));
        assert_eq!(m.state_of(SectionId::Developers), "confirm");
        m.current_mut(SectionId::Developers).dialog.as_mut().unwrap().buttons.iter_mut().for_each(|b| b.enabled = true);
        intent(&mut m, SectionId::Developers, json!({ "kind": "confirm" }));
        assert_eq!(m.state_of(SectionId::Developers), "confirm", "the ack box is off");
        intent(&mut m, SectionId::Developers, json!({ "kind": "permanent" }));
        assert_eq!(m.state_of(SectionId::Developers), "confirm", "`permanent` belongs to the shortfall dialog");
        intent(&mut m, SectionId::Developers, json!({ "kind": "setAck", "on": true }));
        intent(&mut m, SectionId::Developers, json!({ "kind": "confirm" }));
        assert_eq!(m.state_of(SectionId::Developers), "cleaning");
        // A selection is frozen while it runs; the frozen section answers a toggle with the same view.
        let before = m.view(SectionId::Developers);
        let after = intent(&mut m, SectionId::Developers, json!({ "kind": "toggle", "row": "p:1" }));
        assert_eq!(before.rows, after.rows);
    }

    #[test]
    fn fake_save_puts_the_outcome_on_the_report_and_only_there() {
        let mut m = model();
        assert_eq!(m.report_text(SectionId::Cleanup), None);
        m.go(SectionId::Cleanup, "report");
        assert!(m.report_text(SectionId::Cleanup).is_some());
        let v = m.set_saved(SectionId::Cleanup, "Report saved.".into(), true);
        assert_eq!(v.report.unwrap().saved, Some(SavedView { text: "Report saved.".into(), ok: true }));
        // Not a report: unchanged.
        m.go(SectionId::Services, "loaded");
        assert_eq!(m.report_text(SectionId::Services), None);
        assert!(m.set_saved(SectionId::Services, "x".into(), true).report.is_none());
    }

    /// The fake is a stand-in: nothing in this file may name the engine.
    #[test]
    fn fake_never_reaches_the_engine() {
        let source = include_str!("fake.rs");
        for forbidden in [["xiox_core::", "cleaner"].concat(), ["xiox_core::dev::", "DevState"].concat(), ["Co", "re::"].concat()] {
            assert!(!source.contains(&forbidden), "fake.rs names `{forbidden}`");
        }
    }

    // ---- The shared script: the same file the TypeScript mock replays.

    fn expect_matches(view: &SectionView, state: &str, want: &Value, where_: &str) {
        let obj = want.as_object().expect("expect is an object");
        for (key, v) in obj {
            match key.as_str() {
                "state" => assert_eq!(state, v.as_str().unwrap(), "{where_}: state"),
                "rev" => assert_eq!(view.rev, v.as_u64().unwrap(), "{where_}: rev"),
                "fraction" => match v.as_f64() {
                    None => assert!(view.job.is_none(), "{where_}: no job"),
                    Some(f) => {
                        let got = view.job.as_ref().unwrap_or_else(|| panic!("{where_}: a job")).fraction;
                        assert!((f64::from(got) - f).abs() < 1e-6, "{where_}: fraction {got}");
                    }
                },
                "checks" => {
                    for (id, check) in v.as_object().unwrap() {
                        let row = view.rows.iter().find(|r| &r.id == id).unwrap_or_else(|| panic!("{where_}: row {id}"));
                        assert_eq!(&serde_json::to_value(row.check).unwrap(), check, "{where_}: check {id}");
                    }
                }
                "active" => {
                    let active: Vec<&str> = view.nav.iter().filter(|n| n.active).map(|n| n.id.as_str()).collect();
                    assert_eq!(active, vec![v.as_str().unwrap()], "{where_}: active");
                }
                "ack" => assert_eq!(view.dialog.as_ref().and_then(|d| d.ack), v.as_bool(), "{where_}: ack"),
                "primaryEnabled" => {
                    let primary = view.dialog.as_ref().and_then(|d| d.buttons.iter().find(|b| b.primary)).map(|b| b.enabled);
                    assert_eq!(primary, v.as_bool(), "{where_}: primary");
                }
                "notices" => {
                    let texts: Vec<&str> = view.notices.iter().map(|n| n.text.as_str()).collect();
                    let want: Vec<&str> = v.as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
                    assert_eq!(texts, want, "{where_}: notices");
                }
                "dialog" => assert_eq!(view.dialog.is_some(), v.as_bool().unwrap(), "{where_}: dialog"),
                "report" => assert_eq!(view.report.is_some(), v.as_bool().unwrap(), "{where_}: report"),
                "saved" => assert_eq!(view.report.as_ref().is_some_and(|r| r.saved.is_some()), v.as_bool().unwrap(), "{where_}: saved"),
                other => panic!("{where_}: the script has an expectation this replay does not know: {other}"),
            }
        }
    }

    fn section_of(v: &Value) -> SectionId {
        serde_json::from_value(v.clone()).expect("a section of the contract")
    }

    /// An intent of the script has to be one the contract has: the page can send it.
    fn check_typed(section: SectionId, intent: &Value) {
        let ok = match section {
            SectionId::Cleanup => serde_json::from_value::<CleanupIntent>(intent.clone()).is_ok(),
            SectionId::Registry => serde_json::from_value::<RegistryIntent>(intent.clone()).is_ok(),
            SectionId::Services => serde_json::from_value::<ServicesIntent>(intent.clone()).is_ok(),
            SectionId::Developers => serde_json::from_value::<DevIntent>(intent.clone()).is_ok(),
        };
        assert!(ok, "{section:?}: `{intent}` is not an intent of the contract");
    }

    #[test]
    fn fake_replays_the_shared_script() {
        let script: Value = serde_json::from_str(include_str!("../../fixtures/intent-script.json")).unwrap();
        let mut m = model();
        let steps = script["steps"].as_array().unwrap();
        assert!(steps.len() > 100, "the script is short");
        let mut checked = 0;
        for (index, step) in steps.iter().enumerate() {
            let where_ = format!("step {} ({})", index + 1, step["note"].as_str().unwrap_or(""));
            let op = step["op"].as_str().unwrap();
            if op == "tick" {
                let pushed = m.tick();
                let want = step["pushed"].as_array().unwrap();
                assert_eq!(
                    pushed.iter().map(|v| v.section).collect::<Vec<_>>(),
                    want.iter().map(|p| section_of(&p["section"])).collect::<Vec<_>>(),
                    "{where_}: sections pushed"
                );
                for (view, w) in pushed.iter().zip(want) {
                    let mut expect = w.clone();
                    expect.as_object_mut().unwrap().remove("section");
                    expect_matches(view, m.state_of(view.section), &expect, &where_);
                    checked += 1;
                }
                continue;
            }
            let section = section_of(&step["section"]);
            let view = match op {
                "go" => m.go(section, step["state"].as_str().unwrap()),
                "show" => m.show(section),
                "view" => m.view(section),
                // `Save as text…`: the dialog is Rust's; the model is told it went well.
                "save" => m.set_saved(section, "Report saved.".into(), true),
                "intent" => {
                    check_typed(section, &step["intent"]);
                    m.intent(section, &step["intent"])
                }
                other => panic!("{where_}: unknown op {other}"),
            };
            expect_matches(&view, m.state_of(section), &step["expect"], &where_);
            checked += 1;
        }
        assert!(checked > 100, "{checked} views checked");
    }

    /// The tauri commands go through the trait: the fake answers the same typed intents.
    #[test]
    fn the_fake_backend_answers_typed_intents() {
        struct Nowhere;
        impl ViewSink for Nowhere {
            fn send(&self, _: &SectionView) {}
        }
        let b = FakeBackend::start(Arc::new(Nowhere)).unwrap();
        assert!(b.is_fake());
        let v = b.cleanup(CleanupIntent::Analyze).unwrap();
        assert_eq!((v.section, v.job.is_some()), (SectionId::Cleanup, true));
        assert!(b.view(SectionId::Cleanup).rev > v.rev);
        assert_eq!(b.report_text(SectionId::Cleanup), None);
        assert!(b.app_info().fake);
    }
}
