//! The Services & RAM section without a window: its state, the two jobs (read the list, change
//! start types) behind a worker seam, the intents the page can send, and the `SectionView` the page
//! draws.
//!
//! Moved out of `src/main.rs` (the egui window): `SvcMsg`, `send_svc`, the `svc_*` fields,
//! `load_services`, `apply_services` and the services half of `poll`. The strings of the moved code
//! are Italian until Task 18 translates them at the source; the sentences the view adds are English.
//!
//! The intents refuse what the egui window had disabled: a protected row cannot be ticked, an action
//! needs a ticked service (or a saved backup, for «Restore originals»), a confirmation needs its
//! question open, nothing changes while a job runs or the question is open. A refused intent
//! changes nothing. Services Xiox never touches (`protected`) are filtered out of every change,
//! whatever the page ticked.

use crate::dev::{plural, BUSY, DIALOG_OPEN, NOTHING_SELECTED, NO_DIALOG};
use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::view::*;
use crate::{services, sys};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// What the services workers tell the state.
pub enum SvcMsg {
    Loaded(Vec<services::Service>),
    Message(String),
}

/// The four calls of the state as plain function pointers: the real ones by default, fake ones in
/// tests (so `confirm` reaches the start without ever touching a service or the machine).
#[derive(Clone, Copy)]
pub struct ServicesWorkers {
    /// Reads every service.
    pub load: fn(Outbox<SvcMsg>),
    /// The services to change (never a protected one), and the start type: `None` puts the saved
    /// originals back.
    pub apply: fn(Vec<services::Service>, Option<u32>, Outbox<SvcMsg>),
    /// How many services have a saved original start type (what «Restore originals» would restore).
    pub saved: fn() -> usize,
    /// The machine's memory, for the meter.
    pub memory: fn() -> sys::Memory,
}

impl ServicesWorkers {
    pub const REAL: ServicesWorkers = ServicesWorkers { load: real_load, apply: send_svc, saved: real_saved, memory: sys::memory };
}

// Why an intent is refused (English: the page shows them as they are). The refusals every section
// shares (`BUSY`, `DIALOG_OPEN`, `NO_DIALOG`, `NOTHING_SELECTED`) live in `dev`.
pub const PROTECTED: &str = "Needed by Windows: protected";
pub const UNKNOWN_SERVICE: &str = "That service is not in the list.";
pub const NOT_SHOWN: &str = "That service is hidden by the filter.";
pub const NOTHING_SAVED: &str = "No original startup types are saved: there is nothing to restore.";
pub const UNKNOWN_ACTION: &str = "That action is not offered.";

pub struct ServicesState {
    pub services: Vec<services::Service>,
    pub job: Option<Receiver<SvcMsg>>,
    pub loaded: bool,
    /// `true`: every service; `false`: only the ones Xiox suggests turning off.
    pub all: bool,
    pub filter: String,
    pub msg: String,
    /// The question that is open: the start type it would apply (`0`: restore the originals).
    pub confirm: Option<u32>,
    /// `msg` reports a failure (a dead worker), not a result.
    msg_error: bool,
    workers: ServicesWorkers,
}

impl Default for ServicesState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServicesState {
    pub fn new() -> Self {
        ServicesState {
            services: vec![],
            job: None,
            loaded: false,
            all: false,
            filter: String::new(),
            msg: String::new(),
            confirm: None,
            msg_error: false,
            workers: ServicesWorkers::REAL,
        }
    }

    /// The same state with other workers (tests; `ServicesWorkers::REAL` is the default).
    pub fn with_workers(mut self, w: ServicesWorkers) -> Self {
        self.workers = w;
        self
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The services an action would change: ticked and not protected.
    fn chosen(&self) -> Vec<&services::Service> {
        self.services.iter().filter(|s| s.selected && !s.protected).collect()
    }

    // ---------- the jobs ----------

    pub fn load(&mut self, n: SharedNotifier) {
        let (tx, rx) = channel();
        self.job = Some(rx);
        self.loaded = true;
        let work = self.workers.load;
        std::thread::spawn(move || work(Outbox::new(tx, n, SectionId::Services)));
    }

    /// Changes the ticked, not protected services to `start` (`None`: puts the saved originals back).
    pub fn apply(&mut self, start: Option<u32>, n: SharedNotifier) {
        let chosen: Vec<services::Service> = self.chosen().into_iter().cloned().collect();
        let (tx, rx) = channel();
        self.job = Some(rx);
        let work = self.workers.apply;
        std::thread::spawn(move || work(chosen, start, Outbox::new(tx, n, SectionId::Services)));
    }

    /// Drains what the worker has said so far. A message (the result of a change) reloads the list
    /// with `n`. A worker that ends without a result (it died) ends the job with a message, never a
    /// spinner that does not stop.
    pub fn poll(&mut self, n: SharedNotifier) {
        let Some(rx) = &self.job else { return };
        let mut reload = false;
        let mut done = false;
        let mut gone = false;
        loop {
            match rx.try_recv() {
                Ok(SvcMsg::Loaded(v)) => {
                    self.services = v;
                    done = true;
                }
                Ok(SvcMsg::Message(t)) => {
                    self.msg = t;
                    self.msg_error = false;
                    reload = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    gone = true;
                    break;
                }
            }
        }
        if done {
            self.job = None;
        }
        if reload {
            self.job = None;
            self.load(n);
        } else if gone && !done {
            self.job = None;
            self.msg = JOB_DIED.into();
            self.msg_error = true;
        }
    }

    // ---------- intents (page -> Rust) ----------

    /// The egui filter: the Recommended list or all of them, and the search text (lower case) in the
    /// display name or in the service name.
    pub fn visible(&self) -> Vec<usize> {
        let filter = self.filter.to_lowercase();
        (0..self.services.len())
            .filter(|&i| {
                let s = &self.services[i];
                (self.all || s.suggestion.is_some())
                    && (filter.is_empty() || s.display.to_lowercase().contains(&filter) || s.name.to_lowercase().contains(&filter))
            })
            .collect()
    }

    /// Recommended (`false`) or All (`true`). Only changes what is shown.
    pub fn set_all(&mut self, all: bool) {
        self.all = all;
    }

    /// The search box. Only changes what is shown.
    pub fn set_search(&mut self, text: &str) {
        self.filter = text.to_string();
    }

    /// Nothing changes under a running job or under the question (it describes the selection).
    fn editable(&self) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        if self.confirm.is_some() {
            return Err(DIALOG_OPEN.into());
        }
        Ok(())
    }

    /// `row` is `svc:<name>` as in the view. A protected row, one the filter hides, an unknown one: refused.
    pub fn toggle(&mut self, row: &str) -> Result<(), String> {
        self.editable()?;
        let name = row.strip_prefix("svc:").ok_or_else(|| UNKNOWN_SERVICE.to_string())?;
        let i = self.services.iter().position(|s| s.name == name).ok_or_else(|| UNKNOWN_SERVICE.to_string())?;
        if self.services[i].protected {
            return Err(PROTECTED.into());
        }
        if !self.visible().contains(&i) {
            return Err(NOT_SHOWN.into());
        }
        self.services[i].selected = !self.services[i].selected;
        Ok(())
    }

    /// «Refresh»: reads the list again. The result of the last change goes off the screen.
    pub fn refresh(&mut self, n: SharedNotifier) -> Result<(), String> {
        self.editable()?;
        self.msg.clear();
        self.load(n);
        Ok(())
    }

    /// Opens the question for `action`. Restore needs a saved original; the others a ticked,
    /// not protected service.
    pub fn open_confirm(&mut self, action: ServiceAction) -> Result<(), String> {
        self.editable()?;
        let start = self.check_action(action)?;
        self.msg.clear();
        self.confirm = Some(start);
        Ok(())
    }

    /// `Ok(the start type the question would apply)` when the action has what it needs.
    fn check_action(&self, action: ServiceAction) -> Result<u32, String> {
        match action {
            ServiceAction::Restore => {
                if (self.workers.saved)() == 0 {
                    return Err(NOTHING_SAVED.into());
                }
                Ok(0)
            }
            ServiceAction::Disable | ServiceAction::Manual => {
                if self.chosen().is_empty() {
                    return Err(NOTHING_SELECTED.into());
                }
                Ok(if action == ServiceAction::Manual { services::START_MANUAL } else { services::START_DISABLED })
            }
        }
    }

    /// «Continue» in the question: the question closes and the change starts. The selection is
    /// checked again, as when the question opened.
    pub fn confirm(&mut self, n: SharedNotifier) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        let Some(start) = self.confirm else { return Err(NO_DIALOG.into()) };
        let action = action_of(start).ok_or_else(|| UNKNOWN_ACTION.to_string())?;
        self.check_action(action)?;
        self.confirm = None;
        self.apply(if start == 0 { None } else { Some(start) }, n);
        Ok(())
    }

    pub fn close_confirm(&mut self) {
        self.confirm = None;
    }

    /// The section is shown again: the result of the last change goes off the screen (`Core::show`).
    pub fn clear_message(&mut self) {
        self.msg.clear();
        self.msg_error = false;
    }

    // ---------- the view ----------

    pub fn view(&self) -> SectionView {
        let busy = self.busy();
        let visible = self.visible();
        let chosen = self.chosen();
        let freeable_kb: u64 = chosen.iter().map(|s| s.freeable_kb()).sum();
        let saved = (self.workers.saved)();
        let first_load = busy && self.services.is_empty();
        let action = |id: &str, label: String, key: Option<&str>, enabled: bool, primary: bool| ActionView {
            id: id.into(),
            label,
            key: key.map(str::to_string),
            enabled,
            primary,
        };
        let free = !busy && self.confirm.is_none();
        let can_change = free && !chosen.is_empty();
        SectionView {
            section: SectionId::Services,
            rev: 0,
            job: self.job_view(),
            status: self.status_view(),
            nav: Vec::new(),
            options: self.options_view(),
            title: "Services & RAM".into(),
            subtitle: format!("{} of {} services shown", visible.len(), self.services.len()),
            columns: vec![
                ColumnView { title: "Service".into(), kind: ColumnKind::Name },
                ColumnView { title: "State".into(), kind: ColumnKind::Cell },
                ColumnView { title: "Startup".into(), kind: ColumnKind::Cell },
                ColumnView { title: "Memory".into(), kind: ColumnKind::Cell },
            ],
            rows: visible.iter().map(|&i| service_row(&self.services[i])).collect(),
            bulk: false,
            empty: first_load.then(|| EmptyView { title: "Reading services…".into(), text: "This takes a moment.".into() }),
            notices: self.notices_view(),
            actions: vec![
                action("refresh", "Refresh".into(), Some("a"), free, false),
                action("manual", "Set to Manual".into(), None, can_change, false),
                action("restore", format!("Restore originals ({saved})"), None, free && saved > 0, false),
                action("disable", "Disable".into(), Some("Enter"), can_change, true),
            ],
            search: Some(SearchView { placeholder: "Search…".into(), value: self.filter.clone() }),
            dialog: self.dialog_view(),
            report: None,
            footer: Some(format!("Selected: {} · Freeable RAM (estimate): {}", chosen.len(), sys::fmt_size(freeable_kb * 1024))),
        }
    }

    fn job_view(&self) -> Option<JobView> {
        if !self.busy() {
            return None;
        }
        let label = if self.services.is_empty() { "Reading services…" } else { "Working on the services…" };
        Some(JobView { label: label.into(), fraction: 0.0, detail: None })
    }

    fn status_view(&self) -> StatusView {
        let (badge, text) = if self.busy() {
            ("Working".to_string(), String::new())
        } else if !self.loaded {
            ("Ready".to_string(), "Services not read yet".to_string())
        } else {
            let recommended = self.services.iter().filter(|s| s.suggestion.is_some()).count();
            ("Loaded".to_string(), format!("{} services read · {recommended} recommended to turn off", self.services.len()))
        };
        StatusView { badge, text }
    }

    fn options_view(&self) -> Vec<OptionView> {
        let mem = (self.workers.memory)();
        let used = mem.total.saturating_sub(mem.available);
        let frac = if mem.total > 0 { used as f32 / mem.total as f32 } else { 0.0 };
        let tone = if frac > 0.85 {
            Tone::Error
        } else if frac > 0.65 {
            Tone::Warn
        } else {
            Tone::Ok
        };
        vec![
            OptionView::Meter { id: "ram".into(), label: "RAM in use".into(), used_bytes: used, total_bytes: mem.total, tone },
            OptionView::Choice {
                id: "filter".into(),
                label: "Show".into(),
                value: if self.all { "all" } else { "recommended" }.into(),
                choices: vec![
                    ChoiceItem { id: "recommended".into(), label: "Recommended".into() },
                    ChoiceItem { id: "all".into(), label: "All".into() },
                ],
                enabled: true,
            },
        ]
    }

    fn notices_view(&self) -> Vec<NoticeView> {
        if self.msg.is_empty() {
            return Vec::new();
        }
        vec![NoticeView { tone: if self.msg_error { Tone::Error } else { Tone::Ok }, text: self.msg.clone(), more: Vec::new() }]
    }

    /// The question. Disable and Set to Manual name what they change: the ticked, not protected
    /// services, hidden ones included (they are changed too), one row each with its startup type now.
    /// Restore puts back every saved original, whatever is ticked: its question lists nothing.
    fn dialog_view(&self) -> Option<DialogView> {
        let start = self.confirm?;
        let action = action_of(start)?;
        let chosen = self.chosen();
        let count = plural(chosen.len(), "service", "services");
        let (title, text) = match action {
            ServiceAction::Restore => ("Restore the services?".to_string(), "Every service Xiox changed goes back to how it was."),
            ServiceAction::Manual => (
                format!("Set {count} to Manual?"),
                "The services stop now and Windows starts them only when needed.",
            ),
            ServiceAction::Disable => (
                format!("Disable {count}?"),
                "The services stop now and will not start again. You can always use “Restore originals”.",
            ),
        };
        let rows = match action {
            ServiceAction::Restore => Vec::new(),
            _ => chosen.iter().map(|s| (s.display.clone(), s.start_text().to_string())).collect(),
        };
        Some(DialogView {
            id: "confirm".into(),
            title,
            text: vec![text.into()],
            summary: Vec::new(),
            rows,
            warnings_title: None,
            warnings: Vec::new(),
            caution_title: None,
            cautions: Vec::new(),
            ack: None,
            error: None,
            buttons: vec![
                ButtonView { id: "continue".into(), label: "Continue".into(), enabled: true, primary: true },
                ButtonView { id: "cancel".into(), label: "Cancel".into(), enabled: true, primary: false },
            ],
        })
    }
}

/// The question's start type back to its action (`0`: restore, Manual, Disabled). Any other value
/// is not an action the page offers: `None`, never read as Disable.
fn action_of(start: u32) -> Option<ServiceAction> {
    match start {
        0 => Some(ServiceAction::Restore),
        services::START_MANUAL => Some(ServiceAction::Manual),
        services::START_DISABLED => Some(ServiceAction::Disable),
        _ => None,
    }
}

fn service_row(s: &services::Service) -> RowView {
    let state = if s.running { "Running" } else { "Stopped" };
    let memory = if !s.running {
        "–".to_string()
    } else if s.shared > 1 {
        format!("{} (shared)", sys::fmt_size(s.mem_kb * 1024))
    } else {
        sys::fmt_size(s.mem_kb * 1024)
    };
    let mut detail = vec![
        ("item".to_string(), s.display.clone()),
        ("service".to_string(), s.name.clone()),
        ("startup".to_string(), s.start_text().to_string()),
        ("state".to_string(), state.to_string()),
    ];
    if let Some(why) = s.suggestion {
        detail.push(("why turn it off".into(), why.into()));
    }
    RowView {
        check: if s.selected && !s.protected { Check::On } else { Check::Off },
        enabled: !s.protected,
        name: s.display.clone(),
        sub: Some(s.name.clone()),
        cells: vec![state.to_string(), s.start_text().to_string(), memory],
        muted: s.protected,
        why: if s.protected { Some(PROTECTED.to_string()) } else { s.suggestion.map(str::to_string) },
        detail,
        ..blank_row(format!("svc:{}", s.name), RowKind::Item)
    }
}

// ---------- the real jobs ----------

fn real_load(out: Outbox<SvcMsg>) {
    out.send(SvcMsg::Loaded(services::load()));
}

fn real_saved() -> usize {
    services::read_backup().len()
}

/// Il thread dei cambiamenti (il lavoro vero di `ServicesWorkers::REAL`): `start` `None` rimette gli
/// originali, altrimenti imposta quel tipo di avvio ai servizi scelti. Dice com'è andata e basta:
/// è lo stato a rileggere l'elenco.
pub fn send_svc(chosen: Vec<services::Service>, start: Option<u32>, out: Outbox<SvcMsg>) {
    let msg = match start {
        None => {
            let (ok, bad) = services::restore_all();
            if bad > 0 {
                format!("Ripristinati {ok} servizi, {bad} non riusciti.")
            } else {
                format!("Ripristinati {ok} servizi.")
            }
        }
        Some(s) => {
            let mut ok = 0;
            let mut bad = 0;
            for svc in &chosen {
                if services::set_start(svc, s) {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
            let what = if s == services::START_DISABLED { "disattivati" } else { "impostati su Manuale" };
            if bad > 0 {
                format!("{ok} servizi {what}, {bad} non riusciti (servono i diritti di amministratore).")
            } else {
                format!("{ok} servizi {what}.")
            }
        }
    };
    out.send(SvcMsg::Message(msg));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::testing::{action, button, err, notifier, row, wait};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Mutex;

    fn svc(name: &str, display: &str, suggestion: Option<&'static str>, protected: bool, selected: bool) -> services::Service {
        services::Service {
            name: name.into(),
            display: display.into(),
            start: services::START_AUTO,
            delayed: false,
            running: true,
            pid: 100,
            mem_kb: 20_480,
            shared: 1,
            suggestion,
            protected,
            selected,
        }
    }

    /// Fax and DiagTrack are suggested, Spooler is not, RpcSs is protected (and, as a bad page
    /// could have left it, ticked).
    fn list() -> Vec<services::Service> {
        vec![
            svc("Fax", "Fax", Some("Needs a fax modem"), false, true),
            svc("DiagTrack", "Connected User Experiences and Telemetry", Some("Sends usage data"), false, false),
            svc("Spooler", "Print Spooler", None, false, true),
            svc("RpcSs", "Remote Procedure Call", None, true, true),
        ]
    }

    // Workers that must never run: the test reaches the intent's refusal first.
    fn never_load(_: Outbox<SvcMsg>) {
        panic!("the load worker must not run");
    }
    fn never_apply(_: Vec<services::Service>, _: Option<u32>, _: Outbox<SvcMsg>) {
        panic!("the apply worker must not run");
    }
    fn saved_none() -> usize {
        0
    }
    fn memory_half() -> sys::Memory {
        sys::Memory { total: 100 << 30, available: 50 << 30 }
    }
    const NEVER: ServicesWorkers = ServicesWorkers { load: never_load, apply: never_apply, saved: saved_none, memory: memory_half };

    fn state() -> ServicesState {
        let mut st = ServicesState::new().with_workers(NEVER);
        st.services = list();
        st.loaded = true;
        st
    }

    fn drain(st: &mut ServicesState) {
        wait(|| {
            st.poll(notifier());
            !st.busy()
        });
    }

    // ---- protected_services_cannot_be_ticked ----

    #[test]
    fn protected_services_cannot_be_ticked() {
        let mut st = state();
        st.set_all(true);
        assert_eq!(err(st.toggle("svc:RpcSs")), PROTECTED);
        assert_eq!(err(st.toggle("svc:Nope")), UNKNOWN_SERVICE);
        assert_eq!(err(st.toggle("Fax")), UNKNOWN_SERVICE);
        let v = st.view();
        let r = row(&v, "svc:RpcSs");
        assert!(!r.enabled && r.muted);
        assert_eq!(r.check, Check::Off, "a protected row never shows a tick, whatever the data says");
        assert_eq!(r.why.as_deref(), Some("Needed by Windows: protected"));
        // A normal row toggles both ways; one the filter hides is refused.
        st.toggle("svc:Fax").unwrap();
        assert!(!st.services[0].selected);
        st.toggle("svc:Fax").unwrap();
        assert!(st.services[0].selected);
        st.set_all(false);
        assert_eq!(err(st.toggle("svc:Spooler")), NOT_SHOWN);
    }

    #[test]
    fn services_toggles_are_refused_while_busy_or_with_the_dialog_open() {
        let mut st = state();
        st.open_confirm(ServiceAction::Disable).unwrap();
        assert_eq!(err(st.toggle("svc:DiagTrack")), DIALOG_OPEN);
        assert_eq!(err(st.refresh(notifier())), DIALOG_OPEN);
        assert_eq!(err(st.open_confirm(ServiceAction::Manual)), DIALOG_OPEN);
        assert!(!st.services[1].selected);
        st.close_confirm();
        st.job = Some(std::sync::mpsc::channel().1);
        assert_eq!(err(st.toggle("svc:DiagTrack")), BUSY);
        assert_eq!(err(st.refresh(notifier())), BUSY);
        assert_eq!(err(st.open_confirm(ServiceAction::Disable)), BUSY);
        assert_eq!(err(st.confirm(notifier())), BUSY);
    }

    // ---- fix_and_apply_need_their_dialog (services half) ----

    static APPLIED: Mutex<Vec<(Vec<String>, Option<u32>)>> = Mutex::new(Vec::new());
    fn recording_apply(chosen: Vec<services::Service>, start: Option<u32>, out: Outbox<SvcMsg>) {
        APPLIED.lock().unwrap().push((chosen.iter().map(|s| s.name.clone()).collect(), start));
        out.send(SvcMsg::Message("done".into()));
    }
    fn plain_load(out: Outbox<SvcMsg>) {
        out.send(SvcMsg::Loaded(list()));
    }

    #[test]
    fn fix_and_apply_need_their_dialog() {
        let mut st = state().with_workers(ServicesWorkers { load: plain_load, apply: recording_apply, ..NEVER });
        // No question open: nothing to continue, nothing recorded.
        assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
        assert!(APPLIED.lock().unwrap().is_empty());
        assert!(!st.busy());
        // A protected row that is ticked does not count: only protected rows ticked = nothing selected.
        st.services.iter_mut().for_each(|s| s.selected = s.protected);
        assert_eq!(err(st.open_confirm(ServiceAction::Disable)), NOTHING_SELECTED);
        assert_eq!(err(st.open_confirm(ServiceAction::Manual)), NOTHING_SELECTED);
        assert!(st.confirm.is_none());
        // Ticked: Fax and Spooler, plus the protected RpcSs.
        st.services.iter_mut().for_each(|s| s.selected = s.name != "DiagTrack");
        st.open_confirm(ServiceAction::Manual).unwrap();
        assert_eq!(st.confirm, Some(services::START_MANUAL));
        st.confirm(notifier()).unwrap();
        assert!(st.confirm.is_none(), "continuing closes the question");
        wait(|| !APPLIED.lock().unwrap().is_empty());
        {
            let done = APPLIED.lock().unwrap();
            assert_eq!(done.len(), 1);
            assert_eq!(done[0], (vec!["Fax".to_string(), "Spooler".to_string()], Some(services::START_MANUAL)), "RpcSs is protected: never changed");
        }
        drain(&mut st);
        assert_eq!(st.msg, "done");
        // Again without a question.
        assert_eq!(err(st.confirm(notifier())), NO_DIALOG);
        assert_eq!(APPLIED.lock().unwrap().len(), 1);
    }

    // ---- each action sends its own start type to the worker ----

    static SENT: Mutex<Vec<Option<u32>>> = Mutex::new(Vec::new());
    fn start_recording_apply(_: Vec<services::Service>, start: Option<u32>, out: Outbox<SvcMsg>) {
        SENT.lock().unwrap().push(start);
        out.send(SvcMsg::Message("done".into()));
    }
    fn saved_one() -> usize {
        1
    }

    /// The question says what it will do (title) and the worker does exactly that: Disable sends
    /// `Some(START_DISABLED)`, Set to Manual `Some(START_MANUAL)`, Restore `None` (the marker for
    /// "put the saved originals back"). A wrong start type under a right title would be a wrong
    /// change to the system under a confirmation that says something else.
    #[test]
    fn each_action_sends_its_own_start_type_to_the_worker() {
        for (action_kind, want) in [
            (ServiceAction::Disable, Some(services::START_DISABLED)),
            (ServiceAction::Manual, Some(services::START_MANUAL)),
            (ServiceAction::Restore, None),
        ] {
            let mut st = state().with_workers(ServicesWorkers { load: plain_load, apply: start_recording_apply, saved: saved_one, ..NEVER });
            st.open_confirm(action_kind).unwrap();
            st.confirm(notifier()).unwrap();
            wait(|| !SENT.lock().unwrap().is_empty());
            assert_eq!(SENT.lock().unwrap().drain(..).collect::<Vec<_>>(), vec![want], "{action_kind:?}");
            drain(&mut st);
        }
    }

    /// A start type the page could not have produced (`confirm` is a public field of the egui
    /// adapter) is refused: it is never read as Disable, and no question is drawn for it.
    #[test]
    fn an_unknown_start_type_in_the_question_is_refused() {
        let mut st = state().with_workers(ServicesWorkers { apply: start_recording_apply, ..NEVER });
        st.confirm = Some(services::START_AUTO);
        assert!(st.view().dialog.is_none(), "no question text for a start type nobody offers");
        assert_eq!(err(st.confirm(notifier())), UNKNOWN_ACTION);
        assert!(st.confirm.is_none() || st.confirm == Some(services::START_AUTO));
        assert!(!st.busy(), "nothing started");
        assert!(SENT.lock().unwrap().is_empty(), "nothing reached the worker");
    }

    // ---- restore_needs_a_saved_backup ----

    static SAVED: AtomicUsize = AtomicUsize::new(0);
    fn saved_now() -> usize {
        SAVED.load(SeqCst)
    }
    static RESTORED: Mutex<Vec<Option<u32>>> = Mutex::new(Vec::new());
    fn restoring_apply(_: Vec<services::Service>, start: Option<u32>, out: Outbox<SvcMsg>) {
        RESTORED.lock().unwrap().push(start);
        out.send(SvcMsg::Message("restored".into()));
    }

    #[test]
    fn restore_needs_a_saved_backup() {
        let mut st = state().with_workers(ServicesWorkers { apply: restoring_apply, load: plain_load, saved: saved_now, ..NEVER });
        st.services.iter_mut().for_each(|s| s.selected = false);
        SAVED.store(0, SeqCst);
        let v = st.view();
        assert_eq!(action(&v, "restore").label, "Restore originals (0)");
        assert!(!action(&v, "restore").enabled);
        assert_eq!(err(st.open_confirm(ServiceAction::Restore)), NOTHING_SAVED);
        assert!(st.confirm.is_none());
        // With a saved backup it needs no ticked service.
        SAVED.store(3, SeqCst);
        let v = st.view();
        assert_eq!(action(&v, "restore").label, "Restore originals (3)");
        assert!(action(&v, "restore").enabled);
        assert!(!action(&v, "disable").enabled && !action(&v, "manual").enabled, "no ticked service: the others stay off");
        st.open_confirm(ServiceAction::Restore).unwrap();
        assert_eq!(st.confirm, Some(0));
        // The backup went away while the question was open (another window restored): refused at the end.
        SAVED.store(0, SeqCst);
        assert_eq!(err(st.confirm(notifier())), NOTHING_SAVED);
        assert!(RESTORED.lock().unwrap().is_empty());
        SAVED.store(3, SeqCst);
        st.confirm(notifier()).unwrap();
        drain(&mut st);
        assert_eq!(*RESTORED.lock().unwrap(), vec![None], "restore applies `None`: the saved originals");
        SAVED.store(0, SeqCst);
    }

    // ---- the_filter_shows_recommended_or_all_and_searches_both_names ----

    #[test]
    fn the_filter_shows_recommended_or_all_and_searches_both_names() {
        let mut st = state();
        assert_eq!(st.visible(), vec![0, 1], "Recommended: only the suggested ones");
        st.set_all(true);
        assert_eq!(st.visible(), vec![0, 1, 2, 3]);
        st.set_search("PRINT");
        assert_eq!(st.visible(), vec![2], "the display name, any case");
        st.set_search("spool");
        assert_eq!(st.visible(), vec![2], "the service name too");
        st.set_search("diagtrack");
        assert_eq!(st.visible(), vec![1]);
        st.set_search("telemetry");
        assert_eq!(st.visible(), vec![1], "display name of a service whose name does not contain it");
        st.set_all(false);
        st.set_search("print");
        assert!(st.visible().is_empty(), "the filter and the search both apply");
        // The view lists what is visible.
        st.set_all(true);
        st.set_search("");
        let v = st.view();
        assert_eq!(v.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["svc:Fax", "svc:DiagTrack", "svc:Spooler", "svc:RpcSs"]);
        assert_eq!(v.search.as_ref().unwrap().placeholder, "Search…");
        st.set_search("fa");
        assert_eq!(st.view().search.unwrap().value, "fa");
    }

    // ---- a_message_reloads_the_list ----

    static LOADS: AtomicUsize = AtomicUsize::new(0);
    fn counting_load(out: Outbox<SvcMsg>) {
        LOADS.fetch_add(1, SeqCst);
        out.send(SvcMsg::Loaded(list()));
    }
    fn message_apply(_: Vec<services::Service>, _: Option<u32>, out: Outbox<SvcMsg>) {
        out.send(SvcMsg::Message("done".into()));
    }

    #[test]
    fn a_message_reloads_the_list() {
        let before = LOADS.load(SeqCst);
        let mut st = state().with_workers(ServicesWorkers { load: counting_load, apply: message_apply, ..NEVER });
        st.open_confirm(ServiceAction::Disable).unwrap();
        st.confirm(notifier()).unwrap();
        // The apply worker answers with a message: the state keeps it and reads the list again.
        wait(|| {
            st.poll(notifier());
            LOADS.load(SeqCst) > before
        });
        drain(&mut st);
        assert_eq!(LOADS.load(SeqCst), before + 1, "exactly one reload");
        assert_eq!(st.msg, "done");
        assert_eq!(st.services.len(), 4, "the list is the one the load worker returned");
        assert!(st.services[0].selected, "…so the ticks are the fresh ones");
    }

    fn dying_load(_out: Outbox<SvcMsg>) {}

    #[test]
    fn a_dead_worker_ends_the_job_with_an_error() {
        let mut st = ServicesState::new().with_workers(ServicesWorkers { load: dying_load, ..NEVER });
        st.load(notifier());
        drain(&mut st);
        let v = st.view();
        assert!(v.job.is_none());
        assert_eq!(v.notices[0].tone, Tone::Error);
        assert!(action(&v, "refresh").enabled, "the user can read again");
    }

    // ---- services_view_meter_footer_and_dialog_texts ----

    fn memory_of(used_percent: u64) -> sys::Memory {
        sys::Memory { total: 100 << 30, available: (100 - used_percent) << 30 }
    }
    fn memory_ok() -> sys::Memory {
        memory_of(65)
    }
    fn memory_warn() -> sys::Memory {
        memory_of(66)
    }
    fn memory_hot() -> sys::Memory {
        memory_of(86)
    }
    fn memory_edge() -> sys::Memory {
        memory_of(85)
    }

    fn tone_for(memory: fn() -> sys::Memory) -> Tone {
        let st = state().with_workers(ServicesWorkers { memory, ..NEVER });
        match &st.view().options[0] {
            OptionView::Meter { id, label, used_bytes, total_bytes, tone } => {
                assert_eq!((id.as_str(), label.as_str()), ("ram", "RAM in use"));
                let m = memory();
                assert_eq!((*used_bytes, *total_bytes), (m.total - m.available, m.total));
                *tone
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn services_view_meter_footer_and_dialog_texts() {
        // The meter: warn above 65 %, error above 85 %, ok otherwise (the edges stay on the lower tone).
        assert_eq!(tone_for(memory_ok), Tone::Ok);
        assert_eq!(tone_for(memory_warn), Tone::Warn);
        assert_eq!(tone_for(memory_edge), Tone::Warn);
        assert_eq!(tone_for(memory_hot), Tone::Error);

        let mut st = state();
        st.services[1].running = false;
        st.services[0].shared = 3;
        let v = st.view();
        assert_eq!(v.section, SectionId::Services);
        assert!(v.nav.is_empty() && !v.bulk && v.empty.is_none() && v.report.is_none() && v.dialog.is_none());
        match &v.options[1] {
            OptionView::Choice { id, value, choices, enabled, .. } => {
                assert_eq!((id.as_str(), value.as_str(), *enabled), ("filter", "recommended", true));
                assert_eq!(choices.iter().map(|c| (c.id.as_str(), c.label.as_str())).collect::<Vec<_>>(), vec![("recommended", "Recommended"), ("all", "All")]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(v.columns.iter().map(|c| (c.title.as_str(), c.kind)).collect::<Vec<_>>(), vec![
            ("Service", ColumnKind::Name),
            ("State", ColumnKind::Cell),
            ("Startup", ColumnKind::Cell),
            ("Memory", ColumnKind::Cell),
        ]);
        // Rows: Fax is running with a shared process, DiagTrack is stopped.
        let fax = row(&v, "svc:Fax");
        assert_eq!((fax.name.as_str(), fax.sub.as_deref()), ("Fax", Some("Fax")));
        assert_eq!(fax.cells, vec!["Running".to_string(), services::Service::start_text(&st.services[0]).to_string(), format!("{} (shared)", sys::fmt_size(20_480 * 1024))]);
        assert_eq!(fax.why.as_deref(), Some("Needs a fax modem"));
        assert!(fax.enabled && !fax.muted);
        assert!(fax.detail.contains(&("why turn it off".to_string(), "Needs a fax modem".to_string())));
        let diag = row(&v, "svc:DiagTrack");
        assert_eq!(diag.cells[0], "Stopped");
        assert_eq!(diag.cells[2], "–");
        // Footer: Fax (shared process: nothing freeable) and Spooler are ticked; RpcSs is protected.
        assert_eq!(v.footer.as_deref(), Some(format!("Selected: 2 · Freeable RAM (estimate): {}", sys::fmt_size(20_480 * 1024)).as_str()));
        st.services[0].shared = 1;
        assert_eq!(st.view().footer.unwrap(), format!("Selected: 2 · Freeable RAM (estimate): {}", sys::fmt_size(2 * 20_480 * 1024)));
        // Actions.
        assert_eq!(v.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["refresh", "manual", "restore", "disable"]);
        assert_eq!(action(&v, "refresh").label, "Refresh");
        assert_eq!(action(&v, "refresh").key.as_deref(), Some("a"));
        assert_eq!(action(&v, "manual").label, "Set to Manual");
        assert!(action(&v, "disable").primary && action(&v, "disable").key.as_deref() == Some("Enter"));
        assert!(action(&v, "disable").enabled && action(&v, "manual").enabled);

        // The three questions. Fax and Spooler are ticked and RpcSs (protected, ticked by a bad page) is
        // not counted; Spooler is hidden by the Recommended filter and is still changed, so it is listed.
        assert!(!st.visible().contains(&2), "Spooler is hidden by the filter");
        let two = vec![("Fax".to_string(), "Automatico".to_string()), ("Print Spooler".to_string(), "Automatico".to_string())];
        for (action_kind, title, text, rows) in [
            (ServiceAction::Restore, "Restore the services?", "Every service Xiox changed goes back to how it was.", Vec::new()),
            (ServiceAction::Manual, "Set 2 services to Manual?", "The services stop now and Windows starts them only when needed.", two.clone()),
            (ServiceAction::Disable, "Disable 2 services?", "The services stop now and will not start again. You can always use “Restore originals”.", two),
        ] {
            let mut st = state().with_workers(ServicesWorkers { saved: saved_two, ..NEVER });
            st.open_confirm(action_kind).unwrap();
            let v = st.view();
            let d = v.dialog.as_ref().unwrap();
            assert_eq!((d.id.as_str(), d.title.as_str()), ("confirm", title));
            assert_eq!(d.text, vec![text.to_string()]);
            assert_eq!(d.rows, rows, "{title}: the services it changes, the hidden one included, never the protected one");
            assert_eq!(d.buttons.iter().map(|b| (b.id.as_str(), b.label.as_str(), b.primary)).collect::<Vec<_>>(), vec![("continue", "Continue", true), ("cancel", "Cancel", false)]);
            assert!(button(d, "continue").enabled && d.ack.is_none());
            assert!(!action(&v, "disable").enabled, "the question is open: the buttons are off");
        }
        // One service: singular.
        let mut st = state().with_workers(NEVER);
        st.services.iter_mut().for_each(|s| s.selected = s.name == "Fax");
        st.open_confirm(ServiceAction::Disable).unwrap();
        assert_eq!(st.view().dialog.unwrap().title, "Disable 1 service?");

        // First load: nothing to list yet.
        let mut st = ServicesState::new().with_workers(ServicesWorkers { load: held_load, ..NEVER });
        st.load(notifier());
        let v = st.view();
        assert_eq!(v.empty.as_ref().unwrap().title, "Reading services…");
        assert!(v.job.is_some() && !action(&v, "refresh").enabled);
        HOLD.store(1, SeqCst);
        drain(&mut st);
        assert!(st.view().job.is_none());

        // The result of a change is a notice.
        st.msg = "2 servizi disattivati.".into();
        let v = st.view();
        assert_eq!((v.notices[0].tone, v.notices[0].text.as_str()), (Tone::Ok, "2 servizi disattivati."));
    }

    fn saved_two() -> usize {
        2
    }
    static HOLD: AtomicUsize = AtomicUsize::new(0);
    fn held_load(out: Outbox<SvcMsg>) {
        while HOLD.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.send(SvcMsg::Loaded(Vec::new()));
    }

    /// Every Services state of the shared fixture reads through the Rust types (no missing key,
    /// no unknown one) and round-trips.
    #[test]
    fn the_services_states_of_the_shared_fixture_read_through_the_contract() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/views.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let states = doc["views"]["services"].as_object().unwrap();
        assert_eq!(states.len(), 3, "loading, loaded, confirm");
        for (name, view) in states {
            let v: SectionView = serde_json::from_value(view.clone()).unwrap_or_else(|e| panic!("services:{name}: {e}"));
            assert_eq!(v.section, SectionId::Services);
            let again: SectionView = serde_json::from_value(serde_json::to_value(&v).unwrap()).unwrap();
            assert_eq!(again, v, "services:{name} does not round-trip");
        }
    }

    // ---- the replay of the shared script ----

    static LOAD_GATE: AtomicUsize = AtomicUsize::new(0);
    static SCRIPT_SAVED: AtomicUsize = AtomicUsize::new(0);

    /// The fixture's `loaded` list: Fax, DiagTrack and MapsBroker ticked, WSearch and SysMain not.
    fn script_list() -> Vec<services::Service> {
        [("Fax", true), ("DiagTrack", true), ("MapsBroker", true), ("WSearch", false), ("SysMain", false)]
            .into_iter()
            .map(|(n, on)| svc(n, n, Some("why"), false, on))
            .collect()
    }

    fn script_load(out: Outbox<SvcMsg>) {
        while LOAD_GATE.load(SeqCst) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        out.send(SvcMsg::Loaded(script_list()));
    }

    /// The start types the replay's changes were sent with, in order.
    static SCRIPT_SENT: Mutex<Vec<Option<u32>>> = Mutex::new(Vec::new());

    fn script_apply(_: Vec<services::Service>, start: Option<u32>, out: Outbox<SvcMsg>) {
        SCRIPT_SENT.lock().unwrap().push(start);
        out.send(SvcMsg::Message("2 servizi impostati su Manuale.".into()));
    }

    fn script_saved() -> usize {
        SCRIPT_SAVED.load(SeqCst)
    }

    const SCRIPT: ServicesWorkers = ServicesWorkers { load: script_load, apply: script_apply, saved: script_saved, memory: memory_half };

    fn shape(st: &ServicesState) -> &'static str {
        if st.busy() {
            "loading"
        } else if st.confirm.is_some() {
            "confirm"
        } else {
            "loaded"
        }
    }

    fn apply(st: &mut ServicesState, intent: ServicesIntent) -> Result<(), String> {
        let n = notifier();
        match intent {
            ServicesIntent::SetAll { all } => {
                st.set_all(all);
                Ok(())
            }
            ServicesIntent::Search { text } => {
                st.set_search(&text);
                Ok(())
            }
            ServicesIntent::Toggle { row } => st.toggle(&row),
            ServicesIntent::Refresh => st.refresh(n),
            ServicesIntent::OpenConfirm { action } => st.open_confirm(action),
            ServicesIntent::Confirm => st.confirm(n),
            ServicesIntent::CloseDialog => {
                if st.confirm.is_none() {
                    return Err(NO_DIALOG.into());
                }
                st.close_confirm();
                Ok(())
            }
        }
    }

    /// Replays the Services steps of `apps/desktop/fixtures/intent-script.json` against the real
    /// state, through the worker seam. The mock answers a confirm with the `loaded` state and an
    /// English notice at once; Rust runs the change, then reads the list again, so the replay waits
    /// for the job to end. Where the mock and the engine differ, the step is re-synchronised and
    /// counted: exactly TWO (below). When the script changes, re-count.
    ///   * `show` "the notice goes when the section is shown again": `ServicesState` has no `show`;
    ///     `Core::show` calls `clear_message()` and so does the replay (counted).
    ///   * the notice TEXT ("Selected services set to Manual."): the moved worker writes Italian
    ///     until Task 18, so the replay compares the notice COUNT and tone, not the text.
    #[test]
    fn replays_the_services_steps_of_the_shared_script() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/fixtures/intent-script.json");
        let script: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut st = ServicesState::new().with_workers(SCRIPT);
        st.services = script_list();
        st.loaded = true;
        let (mut compared, mut landed, mut shows, mut texts) = (0, 0, 0, 0);
        // The action the open question was opened with, and the start types the worker was given.
        let (mut opened, mut sent) = (None, Vec::new());
        for step in script["steps"].as_array().unwrap() {
            let note = step["note"].as_str().unwrap_or("");
            match step["op"].as_str().unwrap() {
                "show" if step["section"] == "services" => {
                    let expect = step["expect"]["notices"].as_array().unwrap();
                    if expect.is_empty() && !st.msg.is_empty() {
                        st.clear_message(); // what `Core::show` does when the section is shown again
                        shows += 1;
                    }
                    assert_eq!(st.view().notices.len(), expect.len(), "{note}");
                }
                "tick" => {
                    for pushed in step["pushed"].as_array().unwrap().iter().filter(|p| p["section"] == "services") {
                        match (shape(&st), pushed["state"].as_str().unwrap()) {
                            ("loading", "loaded") => {
                                LOAD_GATE.store(1, SeqCst);
                                drain(&mut st);
                                LOAD_GATE.store(0, SeqCst);
                                assert_eq!(shape(&st), "loaded", "{note}");
                                landed += 1;
                            }
                            (now, want) => assert_eq!(now, want, "{note}"),
                        }
                    }
                }
                "intent" if step["section"] == "services" => {
                    let intent: ServicesIntent = serde_json::from_value(step["intent"].clone()).unwrap();
                    let expect = step["expect"]["state"].as_str().unwrap();
                    let result = apply(&mut st, intent.clone());
                    if let (ServicesIntent::OpenConfirm { action }, Ok(())) = (&intent, &result) {
                        opened = Some(*action);
                    }
                    let restore = matches!(intent, ServicesIntent::OpenConfirm { action: ServiceAction::Restore });
                    assert_eq!(result.is_ok(), !restore, "{note}: {intent:?}");
                    if matches!(intent, ServicesIntent::Confirm) {
                        // Change, then read again (the reload waits at the gate): let it through.
                        wait(|| {
                            st.poll(notifier());
                            !st.msg.is_empty()
                        });
                        LOAD_GATE.store(1, SeqCst);
                        drain(&mut st);
                        LOAD_GATE.store(0, SeqCst);
                        let want = match opened.take().expect("a confirm follows an open question") {
                            ServiceAction::Disable => Some(services::START_DISABLED),
                            ServiceAction::Manual => Some(services::START_MANUAL),
                            ServiceAction::Restore => None,
                        };
                        sent.push(want);
                    }
                    assert_eq!(shape(&st), expect, "{note}: {intent:?}");
                    let v = st.view();
                    if let Some(notices) = step["expect"]["notices"].as_array() {
                        assert_eq!(v.notices.len(), notices.len(), "{note}: notices");
                        for n in &v.notices {
                            assert_eq!(n.tone, Tone::Ok, "{note}");
                            texts += 1;
                        }
                    }
                    if let Some(dialog) = step["expect"]["dialog"].as_bool() {
                        assert_eq!(v.dialog.is_some(), dialog, "{note}: dialog");
                    }
                    compared += 1;
                }
                _ => {}
            }
        }
        assert_eq!(*SCRIPT_SENT.lock().unwrap(), sent, "each change reached the worker with the start type of its action");
        assert_eq!(sent, vec![Some(services::START_MANUAL), Some(services::START_DISABLED), Some(services::START_DISABLED)]);
        assert!(compared >= 9, "{compared} services intents compared");
        assert_eq!((landed, shows, texts), (1, 1, 3), "jobs landed, notices cleared by `show`, notices compared without their text");
    }
}
