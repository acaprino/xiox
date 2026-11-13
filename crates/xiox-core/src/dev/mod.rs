//! Voce «Sviluppatori»: Progetti, Worktree degli agenti e Sessioni archiviate.
//!
//! Analisi e pulizia girano in un thread e parlano alla UI con [`DevMsg`] su `mpsc`, come
//! `start_clean`/`poll` in `main.rs`: la UI non si blocca mai. L'ultima analisi ([`DevScan`])
//! è anche l'elenco di ciò che si può cancellare: `xiox_app` rifiuta tutto il resto.
//! I dati dell'analisi si liberano con [`DevState::release`] (uscendo dalla voce) o rianalizzando.
//!
//! Qui sta lo stato senza disegno; il disegno egui resta in `src/dev.rs` finché non lo sostituisce
//! la nuova interfaccia.

use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::sys;
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Projects,
    Worktrees,
    Sessions,
}

impl Section {
    /// Valore di `--dev-section`.
    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "projects" | "progetti" => Some(Section::Projects),
            "worktree" | "worktrees" => Some(Section::Worktrees),
            "sessions" | "sessioni" => Some(Section::Sessions),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Section::Projects => "Progetti",
            Section::Worktrees => "Worktree",
            Section::Sessions => "Sessioni archiviate",
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

/// Il dialogo di conferma: il piano esatto che verrà eseguito, solo per la sezione visibile.
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
    pub settings: Settings,
    pub settings_err: Option<String>,
    pub section: Section,
    pub view: View,
    pub scan: Option<Arc<DevScan>>,
    /// Le cartelle scelte nei progetti (`Artifact::id`).
    pub sel_artifacts: HashSet<String>,
    pub sel_worktrees: Vec<bool>,
    pub sel_sessions: Vec<bool>,
    pub cleaned: Option<DevCleaned>,
    pub error: Option<String>,
    pub job: Option<Receiver<DevMsg>>,
    pub text: String,
    pub progress: f32,
    pub confirm: Option<Confirm>,
    /// Perché «Avvia pulizia» non ha aperto la conferma (striscia gialla sopra la card).
    pub notice: Option<String>,
    pub root_sel: Option<usize>,
    pub auto_scan: bool,
    /// Falso con `--only-root`: le impostazioni restano in memoria.
    pub persist: bool,
    /// `--screenshot`: progetti aperti, nessun dialogo di sistema.
    pub screenshot: bool,
    /// Avanzamento e «Annulla» della pulizia in corso.
    pub tick: Option<Tick>,
    pub cancel: Option<Cancel>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    pub saved: Option<(String, bool)>,
    /// Solo build di debug: `--dev-demo`, per gli screenshot della conferma e del resoconto.
    #[cfg(debug_assertions)]
    pub demo: Option<Demo>,
    /// Fotogrammi dall'ultimo passo della dimostrazione (lo screenshot aspetta che il dialogo compaia).
    #[cfg(debug_assertions)]
    pub demo_frames: u32,
}

/// `--dev-demo` (solo build di debug, solo con `--only-root` su un albero finto): dopo l'analisi
/// sceglie tutte le cartelle e apre la conferma (`Confirm`), oppure pulisce in modo definitivo
/// (`Clean`), oppure pulisce e annulla subito (`Cancel`).
#[cfg(debug_assertions)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Demo {
    Confirm,
    Clean,
    Cancel,
}

#[cfg(debug_assertions)]
impl Demo {
    pub fn parse(s: &str) -> Option<Demo> {
        match s {
            "confirm" => Some(Demo::Confirm),
            "clean" => Some(Demo::Clean),
            "cancel" => Some(Demo::Cancel),
            _ => None,
        }
    }
}

/// Progetti e worktree non si puliscono da un'analisi più vecchia di `xiox_app::MAX_SCAN_AGE_MS`.
pub const STALE_SCAN: &str = xiox_app::STALE_ANALYSIS;
/// Il thread della pulizia è morto senza esito: qualcosa può essere già sparito.
pub const CLEANUP_DIED: &str = "Un errore interno ha fermato la pulizia: alcuni elementi potrebbero essere già stati rimossi. Premi «Analizza» per vedere cosa resta.";

impl DevState {
    /// `analyze`: avvia l'analisi la prima volta che la voce viene mostrata (`--analyze`).
    pub fn new(section: Section, analyze: bool) -> Self {
        Self::with_settings(xiox_app::load_settings(), section, analyze)
    }

    pub fn with_settings(settings: Settings, section: Section, analyze: bool) -> Self {
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
            auto_scan: analyze,
            persist: true,
            screenshot: false,
            tick: None,
            cancel: None,
            saved: None,
            #[cfg(debug_assertions)]
            demo: None,
            #[cfg(debug_assertions)]
            demo_frames: 0,
        }
    }

    /// `--only-root <dir>`: si cerca solo in `dir`, niente sessioni né altri dischi, e le
    /// impostazioni non si salvano.
    pub fn only_root(dir: PathBuf, section: Section, analyze: bool) -> Self {
        let mut st = Self::with_settings(only_root_settings(xiox_app::default_settings(), dir), section, analyze);
        st.persist = false;
        st
    }

    /// `--screenshot`: i progetti si mostrano aperti e «Salva come testo…» non apre dialoghi.
    pub fn set_screenshot(&mut self) {
        self.screenshot = true;
    }

    /// Solo build di debug e solo con `--only-root` su un albero finto (mai sui progetti veri):
    /// la dimostrazione cancella in modo definitivo. `Err`: il motivo del rifiuto.
    #[cfg(debug_assertions)]
    pub fn set_demo(&mut self, demo: Demo) -> Result<(), String> {
        demo_allowed(self.persist, &self.settings.roots)?;
        self.demo = Some(demo);
        Ok(())
    }

    /// Lo screenshot aspetta: la dimostrazione non ha ancora finito o il dialogo sta comparendo.
    pub fn demo_pending(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.demo.is_some() || (self.demo_frames > 0 && self.demo_frames < 30)
        }
        #[cfg(not(debug_assertions))]
        false
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
            let text = if cleaning { CLEANUP_DIED } else { "Il lavoro si è interrotto per un errore interno." };
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
        self.text = "Avvio dell'analisi…".into();
        self.progress = 0.0;
        let (tx, rx) = channel();
        self.job = Some(rx);
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            let outbox = Outbox::new(tx, notifier, SectionId::Developers);
            let step = |t: &str, p: f32| outbox.send(DevMsg::Step(t.to_string(), p));
            let scan = xiox_app::run_scan(&settings, admin, &step);
            outbox.send(DevMsg::Scanned(scan));
        });
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
                            "{} su disco: {}",
                            plural(p.items.len(), "cartella", "cartelle"),
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
                plural(sessions.len(), "sessione archiviata", "sessioni archiviate"),
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
        self.text = "Pulizia in corso…".into();
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
        let (tx, rx) = channel();
        self.job = Some(rx);
        std::thread::spawn(move || {
            let outbox = Outbox::new(tx, notifier, SectionId::Developers);
            let step = |t: &str, p: f32| outbox.send(DevMsg::Step(t.to_string(), p));
            let tick = |t: Tick| outbox.send(DevMsg::Tick(t));
            let mut out = DevCleaned::new();
            out.labels = plan_labels;
            guarded(&mut out, |out| {
                if let Some(plan) = &plan {
                    step("Eliminazione di cartelle e worktree…", 0.1);
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
                    step("Controllo delle sessioni archiviate…", 0.5);
                    match xiox_app::revalidate_archived(&scan, &sessions, &xiox_app::fresh_archived) {
                        Ok(checked) => {
                            for o in checked.skipped {
                                add_session_outcome(&mut out.report, &label(o.agent, &o.id), &o.outcome, mode);
                            }
                            if !checked.keep.is_empty() {
                                step("Cancellazione delle sessioni archiviate…", 0.6);
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
        });
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

    /// Un passo di `--dev-demo` (solo build di debug, solo `--only-root` su un albero finto).
    #[cfg(debug_assertions)]
    pub fn run_demo(&mut self, notifier: SharedNotifier) {
        if self.demo_frames > 0 {
            self.demo_frames += 1;
        }
        let Some(demo) = self.demo else { return };
        if self.busy() || self.view != View::Scanned {
            return;
        }
        let Some(scan) = self.scan.clone() else { return };
        self.demo = None;
        // Le radici possono essere cambiate dal pannello dopo l'avvio: si ricontrolla.
        if let Err(e) = demo_allowed(self.persist, &self.settings.roots) {
            self.notice = Some(e);
            return;
        }
        self.demo_frames = 1;
        self.section = Section::Projects;
        select_all_artifacts(&scan.projects, &mut self.sel_artifacts, true);
        if demo != Demo::Confirm {
            self.settings.remove_mode = RemoveMode::Permanent;
        }
        self.open_confirm();
        if demo == Demo::Confirm {
            return;
        }
        if let Some(mut c) = self.confirm.take() {
            c.ack = true;
            self.start_clean(notifier, c);
            if demo == Demo::Cancel {
                self.cancel_clean();
            }
        }
    }
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
    if n == 1 {
        "1 cartella al livello Attenzione:".into()
    } else {
        format!("{n} cartelle al livello Attenzione:")
    }
}

/// L'avviso sul Cestino troppo piccolo, una frase per unità e poi la domanda.
pub fn shortfall_text(short: &[(char, u64, u64)]) -> String {
    let mut parts: Vec<String> = short
        .iter()
        .map(|(d, need, room)| {
            format!("Il Cestino di {d}: ha {} liberi, la pulizia ne sposta {}.", sys::fmt_size(*room), sys::fmt_size(*need))
        })
        .collect();
    parts.push("Eliminare definitivamente questa volta?".into());
    parts.join(" ")
}

pub fn confirm_title(section: Section) -> &'static str {
    match section {
        Section::Projects => "Pulire le cartelle selezionate?",
        Section::Worktrees => "Rimuovere i worktree selezionati?",
        Section::Sessions => "Cancellare le sessioni archiviate selezionate?",
    }
}

pub fn confirm_text(section: Section, mode: RemoveMode) -> &'static str {
    match (section, mode) {
        (Section::Sessions, RemoveMode::Recycle) => "Una copia viene messa nel Cestino prima della cancellazione, quando il file esiste.",
        (Section::Sessions, RemoveMode::Permanent) => "Le sessioni vengono cancellate definitivamente.",
        (Section::Worktrees, RemoveMode::Recycle) => {
            "I worktree vengono rimossi definitivamente con git, anche con «Nel Cestino»; le cartelle di build dentro un worktree vanno nel Cestino."
        }
        (_, RemoveMode::Recycle) => "Gli elementi selezionati vanno nel Cestino.",
        (_, RemoveMode::Permanent) => "Gli elementi selezionati verranno eliminati definitivamente.",
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
            "Il progetto è in uso da un agente: le sue cartelle si puliscono quando la sessione è chiusa.".into()
        }
        ProjectStatus::Unreachable => format!("Il progetto non è raggiungibile e non si pulisce: {}", r.reason),
        _ => format!("Il progetto è bloccato e non si pulisce: {}", r.reason),
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
        (true, Some(true)) => "Modifiche non salvate: il worktree resta",
        (true, None) => "Stato di git non leggibile: il worktree resta",
        _ => "Nulla da togliere",
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
        Section::Projects => plural(n, "cartella", "cartelle"),
        Section::Worktrees => plural(n, "worktree", "worktree"),
        Section::Sessions => plural(n, "sessione", "sessioni"),
    }
}

/// Cosa c'è di scelto in una sezione, con il nome della cosa: «3 cartelle selezionate».
pub fn selected_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "cartella selezionata", "cartelle selezionate"),
        Section::Worktrees => plural(n, "worktree selezionato", "worktree selezionati"),
        Section::Sessions => plural(n, "sessione selezionata", "sessioni selezionate"),
    }
}

/// Il pulsante che pulisce la sezione visibile: il verbo della sezione e, se c'è una scelta, quanto.
pub fn clean_button(section: Section, n: usize) -> String {
    let verb = match section {
        Section::Projects => "Pulisci",
        Section::Worktrees => "Rimuovi",
        Section::Sessions => "Cancella",
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
        [a] => Some(format!("Non tocca le altre sezioni: {a}.")),
        [a, b] => Some(format!("Non tocca le altre sezioni: {a} e {b}.")),
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
        Level::Recommended => "Consigliato",
        Level::Optional => "Facoltativo",
        Level::Caution => "Attenzione",
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
                let note = if a.note.is_empty() { "Controlla che cosa contiene prima di cancellarla." } else { a.note.as_str() };
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
pub const SESSIONS_CATEGORY: &str = "Sessioni archiviate";

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
            into.failed.push((PathBuf::from(label), format!("{error} (in uso da: {})", locked_by.join(", "))))
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
        _ if busy => "Pulizia in corso…",
        Some(c) if c.interrupted => "Pulizia interrotta",
        Some(c) if c.is_empty() && !c.errors.is_empty() => "Pulizia non eseguita",
        Some(c) if c.report.cancelled => "Pulizia annullata",
        _ => "Pulizia completata",
    }
}

pub fn recycled_text(bytes: u64) -> String {
    format!("{} nel Cestino: si liberano svuotandolo", sys::fmt_size(bytes))
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

/// «1,2 GB di 3,4 GB · 2 di 5 voci».
pub fn tick_text(t: &Tick) -> String {
    format!(
        "{} di {} · {} di {} {}",
        sys::fmt_size(t.done_bytes),
        sys::fmt_size(t.total_bytes),
        t.done_items,
        t.total_items,
        if t.total_items == 1 { "voce" } else { "voci" }
    )
}

/// Durata del resoconto: «850 ms», «12 s», «2 min 05 s».
pub fn fmt_duration(ms: u64) -> String {
    let s = ms / 1000;
    if ms < 1000 {
        format!("{ms} ms")
    } else if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {:02} s", s / 60, s % 60)
    }
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

/// File che segna la radice di un albero finto per `--dev-demo`.
#[cfg(debug_assertions)]
pub const DEMO_MARKER: &str = ".xiox-demo";

/// `--dev-demo` cancella in modo definitivo: vale solo con `--only-root` (`!persist`) e una sola
/// radice dentro la cartella temporanea di sistema o con [`DEMO_MARKER`] alla radice.
#[cfg(debug_assertions)]
pub fn demo_allowed(persist: bool, roots: &[PathBuf]) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("Dimostrazione rifiutata: {why}"));
    let [root] = roots else {
        return refuse("serve --only-root con una sola cartella.");
    };
    if persist {
        return refuse("serve --only-root.");
    }
    let Ok(root) = std::fs::canonicalize(root) else {
        return refuse("la cartella di --only-root non esiste.");
    };
    let in_temp = std::fs::canonicalize(std::env::temp_dir()).is_ok_and(|t| root.starts_with(&t) && root != t);
    if in_temp || root.join(DEMO_MARKER).is_file() {
        Ok(())
    } else {
        refuse(&format!(
            "{} non è un albero finto (né nella cartella temporanea né con il file {DEMO_MARKER}).",
            root.display()
        ))
    }
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
        Section::Projects | Section::Worktrees if failed => "Analisi dei progetti non riuscita: premi «Analizza» per riprovare.",
        Section::Projects => "Nessun progetto trovato.",
        Section::Worktrees => "Nessun worktree creato da un agente.",
        Section::Sessions => "Nessuna sessione archiviata.",
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn section_label(s: Section, count: Option<usize>) -> String {
    match count {
        Some(n) => format!("{} ({n})", s.name()),
        None => s.name().to_string(),
    }
}

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
        "(senza titolo)".into()
    } else {
        t
    }
}

pub fn session_label(s: &ArchivedSession) -> String {
    format!("{} · {}", agent_label(s.agent), session_title(s))
}

pub fn session_size(s: &ArchivedSession) -> String {
    if s.file.is_none() {
        "solo riga nel database".into()
    } else {
        sys::fmt_size(s.bytes)
    }
}

/// «progetto · Build Rust (target)» per una cartella, «progetto · worktree intero» per un worktree.
pub fn plan_label(scan: &DevScan, item: &PlanItem) -> String {
    let report = scan.projects.iter().chain(&scan.worktrees).find(|r| r.id == item.project_id);
    let name = report.map_or_else(|| folder_name(&item.path), |r| r.name.clone());
    match item.action {
        PlanAction::GitWorktreeRemove { .. } => format!("{name} · worktree intero"),
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
        .map(|d| d.with_timezone(&chrono::Local).format("%d/%m/%Y").to_string())
        .unwrap_or_else(|| "—".into())
}

pub fn fmt_elapsed(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    if s < 60 {
        format!("Analisi in {s} s")
    } else {
        format!("Analisi in {} min {} s", s / 60, s % 60)
    }
}
