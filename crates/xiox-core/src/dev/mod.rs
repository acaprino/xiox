//! The Developers section: Projects, agent Worktrees and archived Sessions.
//!
//! Analysis and cleaning run in a thread and talk to the UI with [`DevMsg`] over `mpsc`: the UI
//! never blocks. The last analysis ([`DevScan`]) is also the list of what can be deleted:
//! `xiox_app` refuses everything else. The analysis data is freed with [`DevState::release`]
//! (leaving the section) or by analysing again.
//!
//! This is the state without any drawing; the Tauri window draws it from `apps/desktop`.

use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::sys;
use crate::view::DevCategory;
use xiox_app::{Cancel, DevScan, Tick, ANALYSIS_FAILED};
use xiox_model::{
    path_key, Agent, Artifact, CleanPlan, CleanReport, ItemOutcome, Level, PlanAction, PlanItem, ProjectReport, ProjectStatus,
    RemoveMode, Selection, Settings,
};
use xiox_mukade::ArchivedSession;
use xiox_sessions::{SystemCli, SystemTrash};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod intent_tests;
/// `DevState::view`: the section as a `SectionView`.
mod view;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Projects,
    Worktrees,
    Sessions,
}

impl From<DevCategory> for Section {
    fn from(c: DevCategory) -> Section {
        match c {
            DevCategory::Projects => Section::Projects,
            DevCategory::Worktrees => Section::Worktrees,
            DevCategory::Sessions => Section::Sessions,
        }
    }
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Projects => "Projects",
            Section::Worktrees => "Worktrees",
            Section::Sessions => "Archived sessions",
        }
    }
}

pub enum DevMsg {
    Step(String, f32),
    /// Avanzamento della cancellazione di cartelle e worktree.
    Tick(Tick),
    Scanned(DevScan),
    Cleaned(DevCleaned),
}

/// Esito di una pulizia: il resoconto (cartelle, worktree o sessioni).
pub struct DevCleaned {
    pub report: CleanReport,
    /// `path_key` → «progetto · cartella», per le righe di saltati e non riusciti.
    pub labels: HashMap<String, String>,
    /// Richieste rifiutate per intero (nulla è stato toccato).
    pub errors: Vec<String>,
    /// Un errore interno ha fermato la pulizia a metà: `report` ha solo gli esiti raccolti prima.
    pub interrupted: bool,
}

impl DevCleaned {
    pub fn new() -> Self {
        DevCleaned { report: CleanReport::default(), labels: HashMap::new(), errors: Vec::new(), interrupted: false }
    }

    /// Nulla è stato tolto, saltato o tentato.
    pub fn is_empty(&self) -> bool {
        let r = &self.report;
        r.by_category.is_empty() && r.skipped.is_empty() && r.failed.is_empty() && r.files == 0 && r.dirs == 0
    }

    /// Come mostrare un percorso del resoconto: l'etichetta del piano, se c'è.
    pub fn label(&self, path: &Path) -> String {
        self.labels.get(&path_key(path)).cloned().unwrap_or_else(|| path.display().to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Empty,
    Scanned,
    Cleaned,
}

/// The confirmation dialog: the exact plan that will run, for the visible section only.
pub struct Confirm {
    /// L'analisi da cui è nato il piano: la pulizia gira su questa, non su `DevState::scan`.
    pub scan: Arc<DevScan>,
    pub section: Section,
    /// La scelta da cui è nato il piano (per rifarlo «Definitiva»).
    pub sel: Selection,
    pub plan: Option<CleanPlan>,
    /// Le cartelle Attenzione del piano: (elemento, avviso). Non vuoto → serve «Ho letto gli avvisi».
    pub cautions: Vec<(String, String)>,
    /// «Ho letto gli avvisi» spuntato.
    pub ack: bool,
    /// I Cestini che non bastano: (unità, byte da spostare, byte liberi nel Cestino).
    pub shortfall: Vec<(char, u64, u64)>,
    pub sessions: Vec<(Agent, String)>,
    /// (elemento, dimensione) per l'elenco nel dialogo.
    pub rows: Vec<(String, String)>,
    pub summary: Vec<String>,
    pub error: Option<String>,
    pub mode: RemoveMode,
    /// Quante righe della sezione visibile erano scelte all'apertura (per il pulsante di conferma).
    pub selected: usize,
    /// «Non tocca le altre sezioni: …», se nelle altre due c'è qualcosa di scelto.
    pub others: Option<String>,
}

pub struct DevState {
    settings: Settings,
    settings_err: Option<String>,
    section: Section,
    pub(crate) view: View,
    pub(crate) scan: Option<Arc<DevScan>>,
    /// Le cartelle scelte nei progetti (`Artifact::id`).
    sel_artifacts: HashSet<String>,
    sel_worktrees: Vec<bool>,
    sel_sessions: Vec<bool>,
    pub(crate) cleaned: Option<DevCleaned>,
    error: Option<String>,
    job: Option<Receiver<DevMsg>>,
    text: String,
    progress: f32,
    pub(crate) confirm: Option<Confirm>,
    /// Perché «Avvia pulizia» non ha aperto la conferma (striscia gialla sopra la card).
    notice: Option<String>,
    root_sel: Option<usize>,
    /// Falso con `--only-root`: le impostazioni restano in memoria.
    pub(crate) persist: bool,
    /// Avanzamento e «Annulla» della pulizia in corso.
    tick: Option<Tick>,
    cancel: Option<Cancel>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    saved: Option<(String, bool)>,
    /// Chi fa il lavoro vero: i due thread di analisi e di pulizia. I test ne mettono di finti.
    workers: DevWorkers,
}

/// Tutto ciò che serve a un thread di pulizia: la conferma già accettata, spogliata di ciò che
/// al thread non serve.
pub struct DevCleanJob {
    /// L'analisi da cui è nato il piano.
    pub scan: Arc<DevScan>,
    /// Cartelle e worktree da togliere; `None` se si puliscono solo sessioni.
    pub plan: Option<CleanPlan>,
    /// Le sessioni archiviate da cancellare.
    pub sessions: Vec<(Agent, String)>,
    pub mode: RemoveMode,
    /// `path_key` → «progetto · cartella», per il resoconto.
    pub plan_labels: HashMap<String, String>,
    pub session_labels: HashMap<(Agent, String), String>,
    /// Lo stesso flag di `DevState::cancel`: «Annulla» lo alza.
    pub cancel: Cancel,
}

/// I due lavori di `DevState`, come puntatori a funzione: quelli veri per default, funzioni finte
/// nei test (così `confirm` arriva fino all'avvio senza toccare il motore).
#[derive(Clone, Copy)]
pub struct DevWorkers {
    pub scan: fn(Settings, bool, Outbox<DevMsg>),
    pub clean: fn(DevCleanJob, Outbox<DevMsg>),
}

impl DevWorkers {
    pub const REAL: DevWorkers = DevWorkers { scan: real_scan, clean: real_clean };
}

/// Progetti e worktree non si puliscono da un'analisi più vecchia di `xiox_app::MAX_SCAN_AGE_MS`.
pub const STALE_SCAN: &str = xiox_app::STALE_ANALYSIS;
/// Il thread della pulizia è morto senza esito: qualcosa può essere già sparito.
pub const CLEANUP_DIED: &str = "An internal error stopped the cleanup: some items may already have been removed. Press “Analyze” to see what is left.";

// Why an intent is refused (English: the page shows them as they are).
/// Un lavoro è in corso: tutto resta fermo tranne «Annulla».
pub const BUSY: &str = "Wait for the running job to finish.";
/// La conferma è aperta: ciò che descrive non deve cambiare.
pub const DIALOG_OPEN: &str = "Close the confirmation first.";
pub const NO_DIALOG: &str = "No confirmation is open.";
pub const NOT_READY: &str = "Read the warnings and tick “I have read the warnings” first.";
pub const NO_ANALYSIS: &str = "Analyze first: there is nothing to choose from yet.";
pub const NOTHING_SELECTED: &str = "Nothing is selected in this list.";
pub const NOTHING_TO_CANCEL: &str = "Nothing can be cancelled right now.";
pub const NO_ROOT_SELECTED: &str = "Pick a folder in the list first.";
pub const NO_FOLDERS: &str = "This project has no folders to choose.";
pub const NOTHING_UNDER_HEADING: &str = "Nothing under this heading can be chosen.";
pub const NO_WARNINGS: &str = "This confirmation has no warnings to read.";
pub const NOTHING_TO_RUN: &str = "This confirmation has nothing to clean: close it and analyze again.";
pub const PERMANENT_NOT_OFFERED: &str = "Permanent deletion is not offered here: the Recycle Bin has room.";
pub const PERMANENT_ONLY: &str = "The Recycle Bin is too small: only permanent deletion is offered.";

impl DevState {
    pub fn new(section: Section) -> Self {
        Self::with_settings(xiox_app::load_settings(), section)
    }

    pub fn with_settings(settings: Settings, section: Section) -> Self {
        DevState {
            settings,
            settings_err: None,
            section,
            view: View::Empty,
            scan: None,
            sel_artifacts: HashSet::new(),
            sel_worktrees: Vec::new(),
            sel_sessions: Vec::new(),
            cleaned: None,
            error: None,
            job: None,
            text: String::new(),
            progress: 0.0,
            confirm: None,
            notice: None,
            root_sel: None,
            persist: true,
            tick: None,
            cancel: None,
            saved: None,
            workers: DevWorkers::REAL,
        }
    }

    /// `--only-root <dir>`: si cerca solo in `dir`, niente sessioni né altri dischi, e le
    /// impostazioni non si salvano.
    pub fn only_root(dir: PathBuf, section: Section) -> Self {
        let mut st = Self::with_settings(only_root_settings(xiox_app::default_settings(), dir), section);
        st.persist = false;
        st
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Libera l'analisi e l'esito (la voce torna vuota). Non fa nulla mentre un lavoro è in corso.
    pub fn release(&mut self) {
        if self.busy() {
            return;
        }
        self.drop_data();
        self.view = View::Empty;
        self.error = None;
    }

    pub fn drop_data(&mut self) {
        self.scan = None;
        self.sel_artifacts = HashSet::new();
        self.sel_worktrees = Vec::new();
        self.sel_sessions = Vec::new();
        self.cleaned = None;
        self.confirm = None;
        self.notice = None;
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.job else { return };
        let mut msgs = Vec::new();
        let mut gone = false;
        loop {
            match rx.try_recv() {
                Ok(m) => msgs.push(m),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    gone = true;
                    break;
                }
            }
        }
        let mut done = false;
        for m in msgs {
            match m {
                DevMsg::Step(t, p) => {
                    self.text = t;
                    self.progress = p;
                }
                DevMsg::Tick(t) => self.tick = Some(t),
                DevMsg::Scanned(scan) => {
                    self.set_scan(scan);
                    done = true;
                }
                DevMsg::Cleaned(c) => {
                    // Quello che c'era è cambiato: l'analisi non vale più.
                    self.drop_data();
                    self.cleaned = Some(c);
                    self.view = View::Cleaned;
                    done = true;
                }
            }
        }
        if gone && !done {
            // Una pulizia morta a metà può aver già tolto qualcosa: l'analisi non vale più e
            // «Avvia pulizia» resta spento fino a una nuova analisi.
            let cleaning = self.view == View::Cleaned;
            self.drop_data();
            let text = if cleaning { CLEANUP_DIED } else { "The job was interrupted by an internal error." };
            self.error = Some(text.into());
            self.view = View::Empty;
            done = true;
        }
        if done {
            self.job = None;
            self.tick = None;
            self.cancel = None;
        }
    }

    pub fn set_scan(&mut self, mut scan: DevScan) {
        self.notice = None;
        // Con `--only-root` mai nulla di vero: via sessioni e worktree fuori dall'albero finto.
        if !self.persist {
            if let [root] = self.settings.roots.as_slice() {
                keep_only_under(&mut scan, root);
            } else {
                // Non c'è un'unica radice (impossibile con `only_root`): nel dubbio non si tiene nulla.
                scan = DevScan { warnings: scan.warnings, backend: scan.backend, elapsed_ms: scan.elapsed_ms, scanned_at_ms: scan.scanned_at_ms, ..Default::default() };
            }
        }
        self.sel_artifacts = preselect_artifacts(&scan.projects);
        self.sel_worktrees = preselect_reports(&scan.worktrees);
        // Le sessioni le sceglie l'utente: dopo l'analisi nessuna è spuntata.
        self.sel_sessions = vec![false; scan.archived.len()];
        self.scan = Some(Arc::new(scan));
        self.view = View::Scanned;
    }

    pub fn start_scan(&mut self, notifier: SharedNotifier, admin: bool) {
        self.drop_data();
        self.error = None;
        self.view = View::Empty;
        self.text = "Starting the analysis…".into();
        self.progress = 0.0;
        let (tx, rx) = channel();
        self.job = Some(rx);
        let settings = self.settings.clone();
        let work = self.workers.scan;
        std::thread::spawn(move || work(settings, admin, Outbox::new(tx, notifier, SectionId::Developers)));
    }

    /// Quando «Avvia pulizia» è attivo: nessun lavoro in corso e qualcosa scelto nella sezione visibile.
    pub fn can_clean(&self) -> bool {
        !self.busy() && self.selection().0 > 0
    }

    /// (selezionati, totale, byte selezionati) della sezione visibile.
    pub fn selection(&self) -> (usize, usize, u64) {
        match &self.scan {
            Some(scan) => section_stats(scan, self.section, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions),
            None => (0, 0, 0),
        }
    }

    pub fn open_confirm(&mut self) {
        self.open_confirm_at(xiox_app::now_ms());
    }

    /// Cambia sezione; l'avviso legato al pulsante della sezione precedente sparisce.
    pub fn set_section(&mut self, s: Section) {
        self.section = s;
        self.notice = None;
    }

    /// «Pulisci» nel dialogo: l'età dell'analisi si ricontrolla a `now_ms` (il dialogo può
    /// essere rimasto aperto a lungo). Troppo vecchia per progetti e worktree: nessuna pulizia,
    /// avviso nella striscia gialla. `Some` = la conferma da passare a `start_clean`.
    pub fn accept_confirm(&mut self, c: Confirm, now_ms: i64) -> Option<Confirm> {
        if c.section != Section::Sessions && c.scan.is_stale(now_ms) {
            self.notice = Some(STALE_SCAN.into());
            return None;
        }
        Some(c)
    }

    pub fn open_confirm_at(&mut self, now_ms: i64) {
        self.open_confirm_with(now_ms, xiox_app::recycle_shortfall);
    }

    /// Costruisce il piano dalla selezione della sezione visibile (lo stesso che verrà
    /// eseguito) e apre la conferma. Le righe scelte nelle altre sezioni non ci entrano.
    /// Progetti e worktree: un'analisi vecchia (`DevScan::is_stale` a `now_ms`) non apre nulla
    /// e lo dice nella striscia gialla; le sessioni si ricontrollano comunque prima di cancellarle.
    /// `shortfall`: i Cestini che non bastano per il piano (`xiox_app::recycle_shortfall`).
    pub fn open_confirm_with(&mut self, now_ms: i64, shortfall: impl Fn(&CleanPlan) -> Vec<(char, u64, u64)>) {
        let Some(scan) = self.scan.clone() else { return };
        let mode = self.settings.remove_mode;
        let section = self.section;
        self.notice = None;
        if section != Section::Sessions && scan.is_stale(now_ms) {
            self.notice = Some(STALE_SCAN.into());
            return;
        }
        let action = section_action(&scan, section, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions);
        let sel = action.selection(&scan);
        let sessions: Vec<&ArchivedSession> = scan
            .archived
            .iter()
            .filter(|s| action.sessions.iter().any(|(a, id)| *a == s.agent && *id == s.id))
            .collect();
        let mut rows = Vec::new();
        let mut summary = Vec::new();
        let mut plan = None;
        let mut error = None;
        let mut cautions = Vec::new();
        let mut short = Vec::new();
        if !sel.artifacts.is_empty() || !sel.worktrees.is_empty() {
            match xiox_app::build_plan(&scan, &sel, mode) {
                Ok(p) => {
                    for it in &p.items {
                        rows.push((plan_label(&scan, it), sys::fmt_size(it.expected_bytes)));
                    }
                    if !p.items.is_empty() {
                        summary.push(format!(
                            "{} on disk: {}",
                            plural(p.items.len(), "folder", "folders"),
                            sys::fmt_size(p.expected_bytes)
                        ));
                    }
                    cautions = plan_cautions(&scan, &p);
                    short = shortfall(&p);
                    plan = Some(p);
                }
                Err(e) => error = Some(e),
            }
        }
        if !sessions.is_empty() {
            let bytes: u64 = sessions.iter().map(|s| s.bytes).sum();
            summary.push(format!(
                "{}: {}",
                plural(sessions.len(), "archived session", "archived sessions"),
                sys::fmt_size(bytes)
            ));
            for s in &sessions {
                rows.push((session_label(s), session_size(s)));
            }
        }
        let sessions = sessions.iter().map(|s| (s.agent, s.id.clone())).collect();
        let selected = section_stats(&scan, section, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions).0;
        let others = others_note(&scan, section, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions);
        self.confirm = Some(Confirm {
            scan,
            section,
            sel,
            plan,
            cautions,
            ack: false,
            shortfall: short,
            sessions,
            rows,
            summary,
            error,
            mode,
            selected,
            others,
        });
    }

    pub fn start_clean(&mut self, notifier: SharedNotifier, c: Confirm) {
        let plan_labels: HashMap<String, String> =
            c.plan.iter().flat_map(|p| &p.items).map(|i| (path_key(&i.path), plan_label(&c.scan, i))).collect();
        let picked: HashSet<(Agent, &str)> = c.sessions.iter().map(|(a, id)| (*a, id.as_str())).collect();
        let session_labels: HashMap<(Agent, String), String> = c
            .scan
            .archived
            .iter()
            .filter(|s| picked.contains(&(s.agent, s.id.as_str())))
            .map(|s| ((s.agent, s.id.clone()), session_label(s)))
            .collect();
        let Confirm { scan, plan, sessions, mode, .. } = c;
        let plan = plan.filter(|p| !p.items.is_empty());
        self.cleaned = None;
        self.error = None;
        self.saved = None;
        self.view = View::Cleaned;
        self.text = "Cleaning…".into();
        self.progress = 0.0;
        // La barra e «Annulla» ci sono da subito, prima del primo avanzamento.
        self.tick = plan.as_ref().map(|p| Tick {
            done_bytes: 0,
            total_bytes: p.expected_bytes,
            done_items: 0,
            total_items: p.items.len(),
        });
        let cancel = Cancel::default();
        self.cancel = plan.is_some().then(|| cancel.clone());
        let job = DevCleanJob { scan, plan, sessions, mode, plan_labels, session_labels, cancel };
        let (tx, rx) = channel();
        self.job = Some(rx);
        let work = self.workers.clean;
        std::thread::spawn(move || work(job, Outbox::new(tx, notifier, SectionId::Developers)));
    }

    /// «Annulla» durante la pulizia: i worker si fermano prima del file successivo.
    pub fn cancel_clean(&mut self) {
        if let Some(c) = &self.cancel {
            c.cancel();
        }
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.as_ref().is_some_and(Cancel::is_cancelled)
    }

    pub fn save(&mut self) {
        if self.persist {
            self.settings_err = xiox_app::save_settings(&self.settings).err();
        }
    }
}

/// The intents of the page. Every one of them refuses what the page must not do now, with a reason in English (`Err`): the caller (the Tauri command) turns it into a
/// `CmdError { kind: "refused" }`. A refused intent changes nothing.
impl DevState {
    /// The same state with other workers (tests; `DevWorkers::REAL` is the default).
    pub fn with_workers(mut self, w: DevWorkers) -> Self {
        self.workers = w;
        self
    }

    /// A job runs: nothing may change under it.
    fn idle(&self) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        Ok(())
    }

    /// Selection and settings: no job, and no confirmation whose text describes the selection.
    fn editable(&self) -> Result<(), String> {
        self.idle()?;
        if self.confirm.is_some() {
            return Err(DIALOG_OPEN.into());
        }
        Ok(())
    }

    /// The analysis on screen, for the intents that act on its rows.
    fn analysis(&self) -> Result<Arc<DevScan>, String> {
        match (&self.scan, self.view) {
            (Some(scan), View::Scanned) => Ok(scan.clone()),
            _ => Err(NO_ANALYSIS.into()),
        }
    }

    pub fn select_category(&mut self, c: DevCategory) -> Result<(), String> {
        self.editable()?;
        self.analysis()?;
        self.set_section(c.into());
        Ok(())
    }

    /// A row id of the contract: `p:` a project, `a:` one of its folders, `w:` a worktree,
    /// `s:<agent>:<id>` a session, `h:` a sub-heading (its box ticks the rows under it: the sessions
    /// of a project, the worktrees of a repository). The others (`o:` worktrees nobody manages, the
    /// heading over them) and anything unknown are not choosable. A row the table shows disabled is
    /// refused here too.
    pub fn toggle(&mut self, row: &str) -> Result<(), String> {
        self.editable()?;
        let scan = self.analysis()?;
        if let Some(id) = row.strip_prefix("p:") {
            let r = scan.projects.iter().find(|r| r.id == id).ok_or_else(|| unknown_row(row))?;
            if !status_allows(r) {
                return Err(locked_why(r));
            }
            if r.artifacts.is_empty() {
                return Err(NO_FOLDERS.into());
            }
            toggle_project(r, &mut self.sel_artifacts);
        } else if let Some(id) = row.strip_prefix("a:") {
            let (r, a) = scan
                .projects
                .iter()
                .flat_map(|r| r.artifacts.iter().map(move |a| (r, a)))
                .find(|(_, a)| a.id == id)
                .ok_or_else(|| unknown_row(row))?;
            if !status_allows(r) {
                return Err(locked_why(r));
            }
            let on = !self.sel_artifacts.contains(&a.id);
            set_artifact(&mut self.sel_artifacts, a, on);
        } else if let Some(id) = row.strip_prefix("w:") {
            let i = scan.worktrees.iter().position(|r| r.id == id).ok_or_else(|| unknown_row(row))?;
            let r = &scan.worktrees[i];
            if !selectable(r) {
                return Err(nothing_to_remove(r).map(str::to_string).unwrap_or_else(|| locked_why(r)));
            }
            let on = self.sel_worktrees.get(i).copied().unwrap_or(false);
            *self.sel_worktrees.get_mut(i).ok_or_else(|| unknown_row(row))? = !on;
        } else if row.starts_with("s:") {
            let i = session_index(&scan.archived, row).ok_or_else(|| unknown_row(row))?;
            if let Some(why) = &scan.archived[i].blocked {
                return Err(why.clone());
            }
            let on = self.sel_sessions.get(i).copied().unwrap_or(false);
            *self.sel_sessions.get_mut(i).ok_or_else(|| unknown_row(row))? = !on;
        } else if row.starts_with("h:") {
            let (ids, on) = crate::view::header_toggle(&self.rows(), row).ok_or_else(|| unknown_row(row))?;
            if ids.is_empty() {
                return Err(NOTHING_UNDER_HEADING.into());
            }
            // The rows under a heading are worktrees or sessions the list lets you tick.
            for id in &ids {
                if let Some(i) = id.strip_prefix("w:").and_then(|w| scan.worktrees.iter().position(|r| r.id == w)) {
                    self.sel_worktrees[i] = on;
                } else if let Some(i) = session_index(&scan.archived, id) {
                    self.sel_sessions[i] = on;
                }
            }
        } else {
            return Err(unknown_row(row));
        }
        Ok(())
    }

    /// «All» / «None» of the open list: the rows the table lets you tick.
    pub fn select_all(&mut self, on: bool) -> Result<(), String> {
        self.editable()?;
        let scan = self.analysis()?;
        match self.section {
            Section::Projects => select_all_artifacts(&scan.projects, &mut self.sel_artifacts, on),
            Section::Worktrees => select_reports(&scan.worktrees, &mut self.sel_worktrees, on),
            Section::Sessions => select_sessions(&scan.archived, &mut self.sel_sessions, on),
        }
        Ok(())
    }

    pub fn set_inactive_days(&mut self, days: u32) -> Result<(), String> {
        self.editable()?;
        self.settings.inactive_days = days.clamp(1, 365);
        self.save();
        Ok(())
    }

    pub fn set_remove_mode(&mut self, mode: RemoveMode) -> Result<(), String> {
        self.editable()?;
        self.settings.remove_mode = mode;
        self.save();
        Ok(())
    }

    pub fn set_scan_roots(&mut self, on: bool) -> Result<(), String> {
        self.editable()?;
        self.settings.scan_roots = on;
        self.save();
        Ok(())
    }

    /// The folder highlighted in the list of extra folders (`None`: none).
    pub fn select_root(&mut self, index: Option<usize>) -> Result<(), String> {
        self.editable()?;
        if index.is_some_and(|i| i >= self.settings.roots.len()) {
            return Err(NO_ROOT_SELECTED.into());
        }
        self.root_sel = index;
        Ok(())
    }

    /// A folder to analyze too (chosen in the picker, in Rust). One already there, by `path_key`,
    /// is ignored.
    pub fn add_root(&mut self, dir: PathBuf) -> Result<(), String> {
        self.editable()?;
        if !self.settings.roots.iter().any(|r| path_key(r) == path_key(&dir)) {
            self.settings.roots.push(dir);
            self.save();
        }
        Ok(())
    }

    /// Takes the highlighted folder out of the list.
    pub fn remove_root(&mut self) -> Result<(), String> {
        self.editable()?;
        match self.root_sel {
            Some(i) if i < self.settings.roots.len() => {
                self.settings.roots.remove(i);
                self.root_sel = None;
                self.save();
                Ok(())
            }
            _ => Err(NO_ROOT_SELECTED.into()),
        }
    }

    pub fn analyze(&mut self, notifier: SharedNotifier, admin: bool) -> Result<(), String> {
        self.editable()?;
        self.start_scan(notifier, admin);
        Ok(())
    }

    /// «Clean N …»: opens the confirmation for the visible list, or says why not. An analysis
    /// older than `xiox_app::MAX_SCAN_AGE_MS` opens nothing and puts [`STALE_SCAN`] in `notice`.
    pub fn request_clean(&mut self) -> Result<(), String> {
        self.editable()?;
        if !self.can_clean() {
            return Err(NOTHING_SELECTED.into());
        }
        self.open_confirm();
        Ok(())
    }

    /// The «I have read the warnings» box of the open confirmation.
    pub fn set_ack(&mut self, on: bool) -> Result<(), String> {
        self.idle()?;
        let c = self.confirm.as_mut().ok_or_else(|| NO_DIALOG.to_string())?;
        if c.cautions.is_empty() {
            return Err(NO_WARNINGS.into());
        }
        c.ack = on;
        Ok(())
    }

    /// The confirmation, if it can run now: open, ready (the warnings read, a plan that built),
    /// and `permanent` is the button it shows («Permanently» only when the Recycle Bin is too
    /// small, and then it is the only one). A confirmation that is not ready stays open.
    /// `Ok(None)`: the analysis is too old at `now_ms`; the dialog is closed and [`STALE_SCAN`]
    /// is on screen. `Some` goes to `start_clean`.
    pub fn take_ready_confirm(&mut self, permanent: bool, now_ms: i64) -> Result<Option<Confirm>, String> {
        self.idle()?;
        let c = self.confirm.as_mut().ok_or_else(|| NO_DIALOG.to_string())?;
        if c.error.is_some() || c.rows.is_empty() {
            return Err(NOTHING_TO_RUN.into());
        }
        if !confirm_ready(c) {
            return Err(NOT_READY.into());
        }
        let offered = !c.shortfall.is_empty();
        if permanent != offered {
            return Err(if offered { PERMANENT_ONLY } else { PERMANENT_NOT_OFFERED }.into());
        }
        if permanent {
            if let Err(e) = make_permanent(c) {
                c.error = Some(e.clone());
                return Err(e);
            }
        }
        let c = self.confirm.take().ok_or_else(|| NO_DIALOG.to_string())?;
        Ok(self.accept_confirm(c, now_ms))
    }

    /// «Clean» / «Permanently» in the confirmation: runs it, at the current time.
    pub fn confirm(&mut self, notifier: SharedNotifier, permanent: bool) -> Result<(), String> {
        if let Some(c) = self.take_ready_confirm(permanent, xiox_app::now_ms())? {
            self.start_clean(notifier, c);
        }
        Ok(())
    }

    /// «Cancel» in the confirmation (and Esc): nothing runs.
    pub fn close_confirm(&mut self) {
        self.confirm = None;
    }

    /// «Cancel» during a cleanup of folders or worktrees: the workers stop before the next item.
    /// The cleanup then ends with a report of what was done (`cancelled`), never with the old
    /// analysis back. An analysis, or a cleanup of sessions only, has nothing to stop.
    pub fn cancel(&mut self) -> Result<(), String> {
        if !self.busy() || self.cancel.is_none() {
            return Err(NOTHING_TO_CANCEL.into());
        }
        self.cancel_clean();
        Ok(())
    }

    /// The text of «Save as text…»: the report, when the cleanup is over and did something.
    pub fn report_text(&self) -> Option<String> {
        if self.busy() {
            return None;
        }
        self.cleaned.as_ref().filter(|c| !c.is_empty()).map(|c| c.report.to_text())
    }

    /// The outcome of «Save as text…» (the dialog is Rust's): what the report shows.
    pub fn set_saved(&mut self, text: String, ok: bool) {
        self.saved = Some((text, ok));
    }
}

fn unknown_row(row: &str) -> String {
    format!("“{row}” is not a row that can be chosen.")
}

/// The archived session of a row id `s:<agent>:<id>`.
fn session_index(archived: &[ArchivedSession], row: &str) -> Option<usize> {
    let (agent, id) = row.strip_prefix("s:")?.split_once(':')?;
    archived.iter().position(|s| session_agent_id(s.agent) == agent && s.id == id)
}

/// The agent as it appears in a row id: `claude` | `codex` | `opencode` | `pi`.
pub fn session_agent_id(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Opencode => "opencode",
        Agent::Pi => "pi",
    }
}

/// Il thread dell'analisi (il lavoro vero di `DevWorkers::REAL`). `outbox` avvisa la UI a ogni
/// messaggio e, alla fine o in caso di panico, chiudendosi.
pub fn real_scan(settings: Settings, admin: bool, outbox: Outbox<DevMsg>) {
    let step = |t: &str, p: f32| outbox.send(DevMsg::Step(t.to_string(), p));
    let scan = xiox_app::run_scan(&settings, admin, &step);
    outbox.send(DevMsg::Scanned(scan));
}

/// Il thread della pulizia (il lavoro vero di `DevWorkers::REAL`): cartelle e worktree con
/// `xiox_app::execute_plan`, poi le sessioni riverificate e cancellate. Un panico non perde gli
/// esiti già raccolti (`guarded`).
pub fn real_clean(job: DevCleanJob, outbox: Outbox<DevMsg>) {
    let DevCleanJob { scan, plan, sessions, mode, plan_labels, session_labels, cancel } = job;
    let step = |t: &str, p: f32| outbox.send(DevMsg::Step(t.to_string(), p));
    let tick = |t: Tick| outbox.send(DevMsg::Tick(t));
    let mut out = DevCleaned::new();
    out.labels = plan_labels;
    guarded(&mut out, |out| {
        if let Some(plan) = &plan {
            step("Deleting folders and worktrees…", 0.1);
            match xiox_app::execute_plan(&scan, plan, &tick, &cancel) {
                Ok(res) => merge_report(&mut out.report, xiox_app::report(&scan, plan, &res)),
                Err(e) => out.errors.push(e),
            }
        }
        if !sessions.is_empty() {
            let started = Instant::now();
            let label = |agent: Agent, id: &str| {
                session_labels
                    .get(&(agent, id.to_string()))
                    .cloned()
                    .unwrap_or_else(|| format!("{} · {id}", agent_label(agent)))
            };
            // L'analisi può avere ore: si cancella solo ciò che è ANCORA archiviato e libero.
            step("Checking the archived sessions…", 0.5);
            match xiox_app::revalidate_archived(&scan, &sessions, &xiox_app::fresh_archived) {
                Ok(checked) => {
                    for o in checked.skipped {
                        add_session_outcome(&mut out.report, &label(o.agent, &o.id), &o.outcome, mode);
                    }
                    if !checked.keep.is_empty() {
                        step("Deleting the archived sessions…", 0.6);
                        let cli = SystemCli::locate();
                        match xiox_app::delete_archived(&scan, &checked.keep, mode, &cli, &SystemTrash) {
                            Ok(done) => {
                                for o in done {
                                    add_session_outcome(&mut out.report, &label(o.agent, &o.id), &o.outcome, mode);
                                }
                            }
                            Err(e) => out.errors.push(e),
                        }
                    }
                }
                Err(e) => out.errors.push(e),
            }
            out.report.elapsed_ms = out.report.elapsed_ms.saturating_add(started.elapsed().as_millis() as u64);
        }
    });
    drop(scan);
    outbox.send(DevMsg::Cleaned(out));
}

/// Il modo che il pulsante applica: con il Cestino troppo piccolo c'è solo «Definitiva».
pub fn shown_mode(c: &Confirm) -> RemoveMode {
    if c.shortfall.is_empty() {
        c.mode
    } else {
        RemoveMode::Permanent
    }
}

/// «Definitiva» dopo l'avviso sul Cestino: lo stesso piano, rifatto in modo definitivo.
pub fn make_permanent(c: &mut Confirm) -> Result<(), String> {
    let plan = xiox_app::build_plan(&c.scan, &c.sel, RemoveMode::Permanent)?;
    c.plan = Some(plan);
    c.mode = RemoveMode::Permanent;
    c.shortfall.clear();
    Ok(())
}

/// «Pulisci» (o «Definitiva») si può premere: un piano valido e non vuoto, e gli avvisi letti
/// quando ci sono cartelle al livello Attenzione.
pub fn confirm_ready(c: &Confirm) -> bool {
    c.error.is_none() && !c.rows.is_empty() && (c.cautions.is_empty() || c.ack)
}

pub fn caution_title(n: usize) -> String {
    format!("{} at the Caution level:", plural(n, "folder", "folders"))
}

/// L'avviso sul Cestino troppo piccolo, una frase per unità e poi la domanda.
pub fn shortfall_text(short: &[(char, u64, u64)]) -> String {
    let mut parts: Vec<String> = short
        .iter()
        .map(|(d, need, room)| {
            format!("The Recycle Bin of {d}: has {} free, the cleanup moves {}.", sys::fmt_size(*room), sys::fmt_size(*need))
        })
        .collect();
    parts.push("Delete permanently this time?".into());
    parts.join(" ")
}

pub fn confirm_title(section: Section) -> &'static str {
    match section {
        Section::Projects => "Clean the selected folders?",
        Section::Worktrees => "Remove the selected worktrees?",
        Section::Sessions => "Delete the selected archived sessions?",
    }
}

pub fn confirm_text(section: Section, mode: RemoveMode) -> &'static str {
    match (section, mode) {
        (Section::Sessions, RemoveMode::Recycle) => "A copy is put in the Recycle Bin before deletion, when the file exists.",
        (Section::Sessions, RemoveMode::Permanent) => "The sessions are deleted permanently.",
        (Section::Worktrees, RemoveMode::Recycle) => {
            "Worktrees are removed permanently with git, even with “To the Recycle Bin”; the build folders inside a worktree go to the Recycle Bin."
        }
        (_, RemoveMode::Recycle) => "The selected items go to the Recycle Bin.",
        (_, RemoveMode::Permanent) => "The selected items will be deleted permanently.",
    }
}

pub fn confirm_button(c: &Confirm) -> String {
    clean_button(c.section, c.selected)
}


/// Lo stato e i blocchi che `xiox_engine::build_plan` lascia passare: non in uso, non bloccato.
/// Perché le caselle di un progetto che [`status_allows`] ferma sono spente.
pub fn locked_why(r: &ProjectReport) -> String {
    match r.status {
        ProjectStatus::InUse => {
            "The project is in use by an agent: its folders are cleaned when the session is closed.".into()
        }
        ProjectStatus::Unreachable => format!("The project is not reachable and is not cleaned: {}", r.reason),
        _ => format!("The project is locked and is not cleaned: {}", r.reason),
    }
}

pub fn status_allows(r: &ProjectReport) -> bool {
    matches!(r.status, ProjectStatus::Inactive | ProjectStatus::Recent | ProjectStatus::NoSessions)
        && r.blocked_by.is_empty()
}

/// Quello che `xiox_engine::build_plan` accetta E da cui ricava almeno un elemento
/// (`xiox_app::plans_something`, la stessa regola): un worktree con modifiche non salvate, o di
/// stato git ignoto, senza artefatti darebbe un piano vuoto.
pub fn selectable(r: &ProjectReport) -> bool {
    status_allows(r) && xiox_app::plans_something(r)
}

/// Perché una riga che lo stato lascerebbe passare non ha nulla da togliere; `None` altrimenti
/// (allora il `reason` del report dice già tutto).
pub fn nothing_to_remove(r: &ProjectReport) -> Option<&'static str> {
    if !status_allows(r) || xiox_app::plans_something(r) {
        return None;
    }
    Some(match (r.is_agent_worktree, r.git.dirty) {
        (true, Some(true)) => "Unsaved changes: the worktree stays",
        (true, None) => "Git status not readable: the worktree stays",
        _ => "Nothing to remove",
    })
}

pub fn preselect_reports(reports: &[ProjectReport]) -> Vec<bool> {
    reports.iter().map(|r| r.preselected && selectable(r)).collect()
}

pub fn select_reports(reports: &[ProjectReport], sel: &mut [bool], all: bool) {
    for (s, r) in sel.iter_mut().zip(reports) {
        *s = all && selectable(r);
    }
}

pub fn select_sessions(sessions: &[ArchivedSession], sel: &mut [bool], all: bool) {
    for (s, x) in sel.iter_mut().zip(sessions) {
        *s = all && x.blocked.is_none();
    }
}

pub fn chosen_reports<'a>(reports: &'a [ProjectReport], sel: &'a [bool]) -> impl Iterator<Item = &'a ProjectReport> {
    reports.iter().zip(sel).filter(|(r, s)| **s && selectable(r)).map(|(r, _)| r)
}

pub fn chosen_sessions<'a>(sessions: &'a [ArchivedSession], sel: &'a [bool]) -> impl Iterator<Item = &'a ArchivedSession> {
    sessions.iter().zip(sel).filter(|(s, on)| **on && s.blocked.is_none()).map(|(s, _)| s)
}

/// (selezionati, totale, byte selezionati) di una tabella di progetti o worktree.
pub fn report_stats(reports: &[ProjectReport], sel: &[bool]) -> (usize, usize, u64) {
    let chosen: Vec<&ProjectReport> = chosen_reports(reports, sel).collect();
    (chosen.len(), reports.len(), chosen.iter().map(|r| r.reclaimable_bytes).sum())
}

pub fn session_stats(sessions: &[ArchivedSession], sel: &[bool]) -> (usize, usize, u64) {
    let chosen: Vec<&ArchivedSession> = chosen_sessions(sessions, sel).collect();
    (chosen.len(), sessions.len(), chosen.iter().map(|s| s.bytes).sum())
}

/// Esegue la pulizia; un panico non perde gli esiti già raccolti in `out` e la segna interrotta.
pub fn guarded(out: &mut DevCleaned, work: impl FnOnce(&mut DevCleaned)) {
    if catch_unwind(AssertUnwindSafe(|| work(&mut *out))).is_err() {
        out.interrupted = true;
    }
}

/// (selezionati, totale, byte selezionati) di una sola sezione; nei progetti si contano le cartelle.
pub fn section_stats(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> (usize, usize, u64) {
    match section {
        Section::Projects => artifact_stats(&scan.projects, sa),
        Section::Worktrees => report_stats(&scan.worktrees, sw),
        Section::Sessions => session_stats(&scan.archived, ss),
    }
}

/// Cosa farebbe «Avvia pulizia» nella sezione visibile: le cartelle dei progetti
/// (`Artifact::id`) e i worktree da passare a `build_plan`, le sessioni da passare a
/// `delete_archived`. Mai righe di un'altra sezione.
#[derive(Debug, Default, PartialEq)]
pub struct Action {
    pub artifacts: Vec<String>,
    pub worktrees: Vec<String>,
    pub sessions: Vec<(Agent, String)>,
}

impl Action {
    /// La scelta per `xiox_app::build_plan`: le cartelle come sono; ogni worktree d'agente intero,
    /// o le sue cartelle consigliate (la regola di `xiox_app::selection_for_projects`).
    pub fn selection(&self, scan: &DevScan) -> Selection {
        let mut sel = if self.worktrees.is_empty() {
            Selection::default()
        } else {
            xiox_app::selection_for_projects(scan, &self.worktrees)
        };
        sel.artifacts.extend(self.artifacts.iter().cloned());
        sel
    }
}

pub fn section_action(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> Action {
    match section {
        Section::Projects => Action {
            artifacts: chosen_artifacts(&scan.projects, sa).map(|(_, a)| a.id.clone()).collect(),
            ..Default::default()
        },
        Section::Worktrees => Action {
            worktrees: chosen_reports(&scan.worktrees, sw).map(|r| r.id.clone()).collect(),
            ..Default::default()
        },
        Section::Sessions => Action {
            sessions: chosen_sessions(&scan.archived, ss).map(|s| (s.agent, s.id.clone())).collect(),
            ..Default::default()
        },
    }
}

/// Seconda riga della barra in basso: il conteggio della sola sezione visibile.
pub fn selection_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "folder", "folders"),
        Section::Worktrees => plural(n, "worktree", "worktrees"),
        Section::Sessions => plural(n, "session", "sessions"),
    }
}

/// Cosa c'è di scelto in una sezione, con il nome della cosa: «3 cartelle selezionate».
pub fn selected_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "folder selected", "folders selected"),
        Section::Worktrees => plural(n, "worktree selected", "worktrees selected"),
        Section::Sessions => plural(n, "session selected", "sessions selected"),
    }
}

/// Il pulsante che pulisce la sezione visibile: il verbo della sezione e, se c'è una scelta, quanto.
pub fn clean_button(section: Section, n: usize) -> String {
    let verb = match section {
        Section::Projects => "Clean",
        Section::Worktrees => "Remove",
        Section::Sessions => "Delete",
    };
    if n == 0 {
        verb.into()
    } else {
        format!("{verb} {}", selection_text(section, n))
    }
}

/// La riga del dialogo che dice cosa resta fuori: le righe scelte nelle altre sezioni.
pub fn others_note(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> Option<String> {
    let parts: Vec<String> = [Section::Projects, Section::Worktrees, Section::Sessions]
        .into_iter()
        .filter(|s| *s != section)
        .filter_map(|s| {
            let n = section_stats(scan, s, sa, sw, ss).0;
            (n > 0).then(|| selected_text(s, n))
        })
        .collect();
    match parts.as_slice() {
        [] => None,
        [a] => Some(format!("Does not touch the other sections: {a}.")),
        [a, b] => Some(format!("Does not touch the other sections: {a} and {b}.")),
        _ => None,
    }
}

// ---------- cartelle dei progetti ----------

/// La casella di un progetto: tutte le sue cartelle scelte, nessuna o alcune.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tri {
    All,
    None,
    Some,
}

pub fn tri_state(r: &ProjectReport, sel: &HashSet<String>) -> Tri {
    let on = r.artifacts.iter().filter(|a| sel.contains(&a.id)).count();
    match on {
        0 => Tri::None,
        n if n == r.artifacts.len() => Tri::All,
        _ => Tri::Some,
    }
}

/// Clic sulla casella del progetto: da «tutte» a nessuna, da «nessuna» o «alcune» a tutte.
/// Un progetto in uso o bloccato non cambia.
pub fn toggle_project(r: &ProjectReport, sel: &mut HashSet<String>) {
    if !status_allows(r) {
        return;
    }
    let all = tri_state(r, sel) != Tri::All;
    for a in &r.artifacts {
        set_artifact(sel, a, all);
    }
}

pub fn set_artifact(sel: &mut HashSet<String>, a: &Artifact, on: bool) {
    if on {
        sel.insert(a.id.clone());
    } else {
        sel.remove(&a.id);
    }
}

/// La preselezione del motore (`Artifact::preselected`), mai nei progetti in uso o bloccati.
pub fn preselect_artifacts(reports: &[ProjectReport]) -> HashSet<String> {
    reports
        .iter()
        .filter(|r| status_allows(r))
        .flat_map(|r| r.artifacts.iter().filter(|a| a.preselected).map(|a| a.id.clone()))
        .collect()
}

/// «Tutti» e «Nessuno» nei progetti: ogni cartella dei progetti liberi, qualunque livello.
pub fn select_all_artifacts(reports: &[ProjectReport], sel: &mut HashSet<String>, all: bool) {
    sel.clear();
    if all {
        sel.extend(reports.iter().filter(|r| status_allows(r)).flat_map(|r| r.artifacts.iter().map(|a| a.id.clone())));
    }
}

/// Le cartelle scelte dei progetti liberi.
pub fn chosen_artifacts<'a>(
    reports: &'a [ProjectReport],
    sel: &'a HashSet<String>,
) -> impl Iterator<Item = (&'a ProjectReport, &'a Artifact)> {
    reports
        .iter()
        .filter(|r| status_allows(r))
        .flat_map(move |r| r.artifacts.iter().filter(|a| sel.contains(&a.id)).map(move |a| (r, a)))
}

/// (cartelle scelte, cartelle in tutto, byte scelti).
pub fn artifact_stats(reports: &[ProjectReport], sel: &HashSet<String>) -> (usize, usize, u64) {
    let total = reports.iter().map(|r| r.artifacts.len()).sum();
    let (n, bytes) = chosen_artifacts(reports, sel)
        .fold((0, 0u64), |(n, b), (_, a)| (n + 1, b.saturating_add(a.size.effective_bytes())));
    (n, total, bytes)
}

pub fn level_word(level: Level) -> &'static str {
    match level {
        Level::Recommended => "Recommended",
        Level::Optional => "Optional",
        Level::Caution => "Caution",
    }
}

/// Le cartelle Attenzione di un piano, con il loro avviso: la conferma le elenca.
pub fn plan_cautions(scan: &DevScan, plan: &CleanPlan) -> Vec<(String, String)> {
    let artifacts: HashMap<String, &Artifact> =
        scan.projects.iter().chain(&scan.worktrees).flat_map(|r| &r.artifacts).map(|a| (path_key(&a.path), a)).collect();
    plan.items
        .iter()
        .filter(|i| matches!(i.action, PlanAction::RemoveDir))
        .filter_map(|i| {
            let a = artifacts.get(&path_key(&i.path))?;
            (a.level == Level::Caution).then(|| {
                let note = if a.note.is_empty() { "Check what it contains before deleting it." } else { a.note.as_str() };
                (plan_label(scan, i), note.to_string())
            })
        })
        .collect()
}

// ---------- resoconto ----------

/// Aggiunge il resoconto di `execute_plan` a quello della pulizia.
pub fn merge_report(into: &mut CleanReport, r: CleanReport) {
    into.freed_bytes = into.freed_bytes.saturating_add(r.freed_bytes);
    into.recycled_bytes = into.recycled_bytes.saturating_add(r.recycled_bytes);
    into.files = into.files.saturating_add(r.files);
    into.dirs = into.dirs.saturating_add(r.dirs);
    into.elapsed_ms = into.elapsed_ms.saturating_add(r.elapsed_ms);
    into.cancelled |= r.cancelled;
    for (label, bytes) in r.by_category {
        add_category(into, &label, bytes);
    }
    into.skipped.extend(r.skipped);
    into.failed.extend(r.failed);
}

/// Categoria delle sessioni archiviate nel resoconto.
pub const SESSIONS_CATEGORY: &str = "Archived sessions";

/// L'esito di una sessione nel resoconto: il file conta tra i file eliminati, i byte tra
/// liberati o nel Cestino secondo il modo.
pub fn add_session_outcome(into: &mut CleanReport, label: &str, outcome: &ItemOutcome, mode: RemoveMode) {
    match outcome {
        ItemOutcome::Removed { bytes_freed } => {
            if *bytes_freed > 0 {
                into.files = into.files.saturating_add(1);
            }
            match mode {
                RemoveMode::Recycle => into.recycled_bytes = into.recycled_bytes.saturating_add(*bytes_freed),
                RemoveMode::Permanent => into.freed_bytes = into.freed_bytes.saturating_add(*bytes_freed),
            }
            add_category(into, SESSIONS_CATEGORY, *bytes_freed);
        }
        ItemOutcome::Skipped { reason } => into.skipped.push((PathBuf::from(label), reason.clone())),
        ItemOutcome::Failed { error, locked_by } if locked_by.is_empty() => into.failed.push((PathBuf::from(label), error.clone())),
        ItemOutcome::Failed { error, locked_by } => {
            into.failed.push((PathBuf::from(label), format!("{error} (in use by: {})", locked_by.join(", "))))
        }
    }
}

/// Somma `bytes` alla categoria `label`; l'elenco resta dal più grande.
pub fn add_category(into: &mut CleanReport, label: &str, bytes: u64) {
    match into.by_category.iter_mut().find(|(l, _)| l == label) {
        Some((_, b)) => *b = b.saturating_add(bytes),
        None => into.by_category.push((label.to_string(), bytes)),
    }
    into.by_category.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

pub fn cleaned_title(c: Option<&DevCleaned>, busy: bool) -> &'static str {
    match c {
        _ if busy => "Cleaning…",
        Some(c) if c.interrupted => "Cleanup interrupted",
        Some(c) if c.is_empty() && !c.errors.is_empty() => "Cleanup not run",
        Some(c) if c.report.cancelled => "Cleanup cancelled",
        _ => "Cleanup done",
    }
}

/// Avanzamento della barra: a byte, o a voci quando i byte non si conoscono.
pub fn tick_fraction(t: &Tick) -> f32 {
    if t.total_bytes > 0 {
        (t.done_bytes as f64 / t.total_bytes as f64).clamp(0.0, 1.0) as f32
    } else if t.total_items > 0 {
        (t.done_items as f32 / t.total_items as f32).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// «1.2 GB di 3.4 GB · 2 di 5 voci».
pub fn tick_text(t: &Tick) -> String {
    format!(
        "{} of {} · {} of {} {}",
        sys::fmt_size(t.done_bytes),
        sys::fmt_size(t.total_bytes),
        sys::fmt_count(t.done_items as u64),
        sys::fmt_count(t.total_items as u64),
        if t.total_items == 1 { "item" } else { "items" }
    )
}

/// `--only-root`: solo la cartella data, niente sessioni né altri dischi.
pub fn only_root_settings(mut s: Settings, dir: PathBuf) -> Settings {
    s.session_projects = false;
    s.roots = vec![dir];
    s.scan_roots = true;
    s.all_drives = false;
    s
}

/// `--only-root`: l'analisi legge comunque gli archivi veri delle sessioni e può trovare worktree
/// veri. Si tiene solo ciò che sta dentro l'albero finto: niente sessioni (non hanno una cartella
/// lì dentro) e solo i report la cui cartella sta sotto `root`, confrontata per componenti
/// (`albero-x` non sta sotto `albero`) e con la chiave di percorso del progetto.
pub fn keep_only_under(scan: &mut DevScan, root: &Path) {
    let root = PathBuf::from(xiox_model::path_key(root));
    let inside = |r: &ProjectReport| PathBuf::from(xiox_model::path_key(&r.root)).starts_with(&root);
    scan.archived.clear();
    scan.projects.retain(inside);
    scan.worktrees.retain(inside);
    scan.other_worktrees.retain(inside);
}

pub fn analysis_failed(scan: &DevScan) -> bool {
    scan.warnings.iter().any(|w| w == ANALYSIS_FAILED)
}

/// Il fallimento dell'analisi prima degli altri avvisi.
pub fn ordered_warnings(warnings: &[String]) -> Vec<&String> {
    let (mut first, rest): (Vec<&String>, Vec<&String>) = warnings.iter().partition(|w| w.as_str() == ANALYSIS_FAILED);
    first.extend(rest);
    first
}

/// Cosa dire in una sezione vuota: dopo un'analisi fallita le liste vuote non significano «niente».
pub fn empty_text(section: Section, failed: bool) -> &'static str {
    match section {
        Section::Projects | Section::Worktrees if failed => "Analysis of the projects failed: press “Analyze” to try again.",
        Section::Projects => "No project found.",
        Section::Worktrees => "No worktree created by an agent.",
        Section::Sessions => "No archived session.",
    }
}

pub use crate::sys::plural;

pub fn agent_label(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
        Agent::Opencode => "OpenCode",
        Agent::Pi => "Pi",
    }
}

pub fn session_title(s: &ArchivedSession) -> String {
    let t = s.title.as_deref().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        "(untitled)".into()
    } else {
        t
    }
}

pub fn session_label(s: &ArchivedSession) -> String {
    format!("{} · {}", agent_label(s.agent), session_title(s))
}

pub fn session_size(s: &ArchivedSession) -> String {
    if s.file.is_none() {
        "database row only".into()
    } else {
        sys::fmt_size(s.bytes)
    }
}

/// «progetto · Build Rust (target)» per una cartella, «progetto · worktree intero» per un worktree.
pub fn plan_label(scan: &DevScan, item: &PlanItem) -> String {
    let report = scan.projects.iter().chain(&scan.worktrees).find(|r| r.id == item.project_id);
    let name = report.map_or_else(|| folder_name(&item.path), |r| r.name.clone());
    match item.action {
        PlanAction::GitWorktreeRemove { .. } => format!("{name} · whole worktree"),
        PlanAction::RemoveDir => {
            let key = path_key(&item.path);
            let what = report
                .and_then(|r| r.artifacts.iter().find(|a| path_key(&a.path) == key))
                .map_or_else(|| folder_name(&item.path), |a| a.label.clone());
            format!("{name} · {what}")
        }
    }
}

pub fn folder_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Data locale `gg/mm/aaaa` da millisecondi epoch.
pub fn fmt_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".into())
}

pub fn fmt_elapsed(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    if s < 60 {
        format!("Analysis in {s} s")
    } else {
        format!("Analysis in {} min {} s", s / 60, s % 60)
    }
}
