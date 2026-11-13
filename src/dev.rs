//! Voce «Sviluppatori»: Progetti, Worktree degli agenti e Sessioni archiviate.
//!
//! Analisi e pulizia girano in un thread e parlano alla UI con [`DevMsg`] su `mpsc`, come
//! `start_clean`/`poll` in `main.rs`: la UI non si blocca mai. L'ultima analisi ([`DevScan`])
//! è anche l'elenco di ciò che si può cancellare: `xiox_app` rifiuta tutto il resto.
//! I dati dell'analisi si liberano con [`DevState::release`] (uscendo dalla voce) o rianalizzando.

use crate::sys;
use crate::theme::*;
use xiox_app::{Cancel, DevScan, Tick, ANALYSIS_FAILED};
use xiox_model::{
    path_key, Agent, Artifact, CleanPlan, CleanReport, ItemOutcome, Level, PlanAction, PlanItem, ProjectReport, ProjectStatus,
    RemoveMode, Selection, Settings,
};
use xiox_mukade::ArchivedSession;
use xiox_sessions::{SystemCli, SystemTrash};
use eframe::egui::collapsing_header::{paint_default_icon, CollapsingState};
use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Stroke, Ui, Vec2};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

const ROW_H: f32 = 22.0;
const CHECK_W: f32 = 22.0;
/// Spazio lasciato alla barra di scorrimento a destra delle tabelle.
const SCROLLBAR: f32 = 12.0;

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

    fn name(self) -> &'static str {
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
    fn new() -> Self {
        DevCleaned { report: CleanReport::default(), labels: HashMap::new(), errors: Vec::new(), interrupted: false }
    }

    /// Nulla è stato tolto, saltato o tentato.
    fn is_empty(&self) -> bool {
        let r = &self.report;
        r.by_category.is_empty() && r.skipped.is_empty() && r.failed.is_empty() && r.files == 0 && r.dirs == 0
    }

    /// Come mostrare un percorso del resoconto: l'etichetta del piano, se c'è.
    fn label(&self, path: &Path) -> String {
        self.labels.get(&path_key(path)).cloned().unwrap_or_else(|| path.display().to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum View {
    Empty,
    Scanned,
    Cleaned,
}

/// Il dialogo di conferma: il piano esatto che verrà eseguito, solo per la sezione visibile.
struct Confirm {
    /// L'analisi da cui è nato il piano: la pulizia gira su questa, non su `DevState::scan`.
    scan: Arc<DevScan>,
    section: Section,
    /// La scelta da cui è nato il piano (per rifarlo «Definitiva»).
    sel: Selection,
    plan: Option<CleanPlan>,
    /// Le cartelle Attenzione del piano: (elemento, avviso). Non vuoto → serve «Ho letto gli avvisi».
    cautions: Vec<(String, String)>,
    /// «Ho letto gli avvisi» spuntato.
    ack: bool,
    /// I Cestini che non bastano: (unità, byte da spostare, byte liberi nel Cestino).
    shortfall: Vec<(char, u64, u64)>,
    sessions: Vec<(Agent, String)>,
    /// (elemento, dimensione) per l'elenco nel dialogo.
    rows: Vec<(String, String)>,
    summary: Vec<String>,
    error: Option<String>,
    mode: RemoveMode,
    /// Quante righe della sezione visibile erano scelte all'apertura (per il pulsante di conferma).
    selected: usize,
    /// «Non tocca le altre sezioni: …», se nelle altre due c'è qualcosa di scelto.
    others: Option<String>,
}

pub struct DevState {
    settings: Settings,
    settings_err: Option<String>,
    section: Section,
    view: View,
    scan: Option<Arc<DevScan>>,
    /// Le cartelle scelte nei progetti (`Artifact::id`).
    sel_artifacts: HashSet<String>,
    sel_worktrees: Vec<bool>,
    sel_sessions: Vec<bool>,
    cleaned: Option<DevCleaned>,
    error: Option<String>,
    job: Option<Receiver<DevMsg>>,
    text: String,
    progress: f32,
    confirm: Option<Confirm>,
    /// Perché «Avvia pulizia» non ha aperto la conferma (striscia gialla sopra la card).
    notice: Option<String>,
    root_sel: Option<usize>,
    auto_scan: bool,
    /// Falso con `--only-root`: le impostazioni restano in memoria.
    persist: bool,
    /// `--screenshot`: progetti aperti, nessun dialogo di sistema.
    screenshot: bool,
    /// Avanzamento e «Annulla» della pulizia in corso.
    tick: Option<Tick>,
    cancel: Option<Cancel>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    saved: Option<(String, bool)>,
    /// Solo build di debug: `--dev-demo`, per gli screenshot della conferma e del resoconto.
    #[cfg(debug_assertions)]
    demo: Option<Demo>,
    /// Fotogrammi dall'ultimo passo della dimostrazione (lo screenshot aspetta che il dialogo compaia).
    #[cfg(debug_assertions)]
    demo_frames: u32,
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
const STALE_SCAN: &str = xiox_app::STALE_ANALYSIS;
/// Il thread della pulizia è morto senza esito: qualcosa può essere già sparito.
const CLEANUP_DIED: &str = "Un errore interno ha fermato la pulizia: alcuni elementi potrebbero essere già stati rimossi. Premi «Analizza» per vedere cosa resta.";

impl DevState {
    /// `analyze`: avvia l'analisi la prima volta che la voce viene mostrata (`--analyze`).
    pub fn new(section: Section, analyze: bool) -> Self {
        Self::with_settings(xiox_app::load_settings(), section, analyze)
    }

    fn with_settings(settings: Settings, section: Section, analyze: bool) -> Self {
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

    fn drop_data(&mut self) {
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

    fn set_scan(&mut self, mut scan: DevScan) {
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

    fn start_scan(&mut self, ctx: &egui::Context, admin: bool) {
        self.drop_data();
        self.error = None;
        self.view = View::Empty;
        self.text = "Avvio dell'analisi…".into();
        self.progress = 0.0;
        let (tx, rx) = channel();
        self.job = Some(rx);
        let settings = self.settings.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let step = |t: &str, p: f32| {
                let _ = tx.send(DevMsg::Step(t.to_string(), p));
                ctx.request_repaint();
            };
            let scan = xiox_app::run_scan(&settings, admin, &step);
            let _ = tx.send(DevMsg::Scanned(scan));
            ctx.request_repaint();
        });
    }

    /// Quando «Avvia pulizia» è attivo: nessun lavoro in corso e qualcosa scelto nella sezione visibile.
    fn can_clean(&self) -> bool {
        !self.busy() && self.selection().0 > 0
    }

    /// (selezionati, totale, byte selezionati) della sezione visibile.
    fn selection(&self) -> (usize, usize, u64) {
        match &self.scan {
            Some(scan) => section_stats(scan, self.section, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions),
            None => (0, 0, 0),
        }
    }

    fn open_confirm(&mut self) {
        self.open_confirm_at(xiox_app::now_ms());
    }

    /// Cambia sezione; l'avviso legato al pulsante della sezione precedente sparisce.
    fn set_section(&mut self, s: Section) {
        self.section = s;
        self.notice = None;
    }

    /// «Pulisci» nel dialogo: l'età dell'analisi si ricontrolla a `now_ms` (il dialogo può
    /// essere rimasto aperto a lungo). Troppo vecchia per progetti e worktree: nessuna pulizia,
    /// avviso nella striscia gialla. `Some` = la conferma da passare a `start_clean`.
    fn accept_confirm(&mut self, c: Confirm, now_ms: i64) -> Option<Confirm> {
        if c.section != Section::Sessions && c.scan.is_stale(now_ms) {
            self.notice = Some(STALE_SCAN.into());
            return None;
        }
        Some(c)
    }

    fn open_confirm_at(&mut self, now_ms: i64) {
        self.open_confirm_with(now_ms, xiox_app::recycle_shortfall);
    }

    /// Costruisce il piano dalla selezione della sezione visibile (lo stesso che verrà
    /// eseguito) e apre la conferma. Le righe scelte nelle altre sezioni non ci entrano.
    /// Progetti e worktree: un'analisi vecchia (`DevScan::is_stale` a `now_ms`) non apre nulla
    /// e lo dice nella striscia gialla; le sessioni si ricontrollano comunque prima di cancellarle.
    /// `shortfall`: i Cestini che non bastano per il piano (`xiox_app::recycle_shortfall`).
    fn open_confirm_with(&mut self, now_ms: i64, shortfall: impl Fn(&CleanPlan) -> Vec<(char, u64, u64)>) {
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

    fn start_clean(&mut self, ctx: &egui::Context, c: Confirm) {
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
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let step = |t: &str, p: f32| {
                let _ = tx.send(DevMsg::Step(t.to_string(), p));
                ctx.request_repaint();
            };
            let tick = |t: Tick| {
                let _ = tx.send(DevMsg::Tick(t));
                ctx.request_repaint();
            };
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
            let _ = tx.send(DevMsg::Cleaned(out));
            ctx.request_repaint();
        });
    }

    /// «Annulla» durante la pulizia: i worker si fermano prima del file successivo.
    fn cancel_clean(&mut self) {
        if let Some(c) = &self.cancel {
            c.cancel();
        }
    }

    fn cancelling(&self) -> bool {
        self.cancel.as_ref().is_some_and(Cancel::is_cancelled)
    }

    fn save(&mut self) {
        if self.persist {
            self.settings_err = xiox_app::save_settings(&self.settings).err();
        }
    }

    /// «Salva come testo…»: il resoconto in un file scelto dall'utente.
    fn save_report(&mut self) {
        let Some(c) = &self.cleaned else { return };
        if self.screenshot {
            return;
        }
        if let Some(out) = save_report_text(&c.report) {
            self.saved = Some(out);
        }
    }

    /// Un passo di `--dev-demo` (solo build di debug, solo `--only-root` su un albero finto).
    #[cfg(debug_assertions)]
    fn run_demo(&mut self, ctx: &egui::Context) {
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
            self.start_clean(ctx, c);
            if demo == Demo::Cancel {
                self.cancel_clean();
            }
        }
    }

    // ---------- drawing ----------

    fn options(&mut self, ui: &mut Ui, busy: bool) {
        group(ui, "Sezione", 6.0);
        let counts = self.scan.as_ref().map(|s| [s.projects.len(), s.worktrees.len(), s.archived.len()]);
        let all = [Section::Projects, Section::Worktrees, Section::Sessions];
        // Le spunte di ogni sezione, anche di quelle non visibili.
        let picked = all.map(|s| match &self.scan {
            Some(scan) => section_stats(scan, s, &self.sel_artifacts, &self.sel_worktrees, &self.sel_sessions).0,
            None => 0,
        });
        for (k, s) in all.into_iter().enumerate() {
            let text = section_label(s, counts.map(|c| c[k]));
            let on = self.section == s;
            let btn = egui::Button::new(RichText::new(text).color(if on { ACCENT } else { TEXT }).strong())
                .fill(if on { SEL_BG } else { CARD })
                .stroke(Stroke::NONE)
                .min_size(Vec2::new(ui.available_width(), 28.0));
            if ui.add(btn).clicked() {
                self.set_section(s);
            }
            if picked[k] > 0 {
                ui.label(RichText::new(selected_text(s, picked[k])).size(12.0).color(MUTED));
            }
        }

        let mut changed = false;
        group(ui, "Opzioni", 12.0);
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                ui.label("Giorni di inattività");
                let r = ui.add(egui::DragValue::new(&mut self.settings.inactive_days).range(1..=365));
                if (r.changed() && !r.dragged()) || r.drag_stopped() {
                    changed = true;
                }
            });
        });

        group(ui, "Eliminazione", 12.0);
        ui.add_enabled_ui(!busy, |ui| {
            changed |= ui.radio_value(&mut self.settings.remove_mode, RemoveMode::Recycle, "Nel Cestino (recuperabile)").changed();
            changed |= ui.radio_value(&mut self.settings.remove_mode, RemoveMode::Permanent, "Definitiva").changed();
        });
        ui.add(
            egui::Label::new(
                RichText::new("Il Cestino non libera spazio finché non lo svuoti. «Cestino» in Pulizia file lo svuota.")
                    .size(12.0)
                    .color(MUTED),
            )
            .wrap(),
        );

        group(ui, "Cartelle aggiuntive", 12.0);
        ui.add_enabled_ui(!busy, |ui| {
            changed |= ui.checkbox(&mut self.settings.scan_roots, "Analizza anche queste cartelle").changed();
            ui.add_space(2.0);
            for (i, root) in self.settings.roots.iter().enumerate() {
                let text = root.display().to_string();
                let on = self.root_sel == Some(i);
                let r = ui
                    .add(egui::Button::selectable(on, RichText::new(&text).color(if on { Color32::WHITE } else { TEXT })).truncate())
                    .on_hover_text(&text);
                if r.clicked() {
                    self.root_sel = if on { None } else { Some(i) };
                }
            }
            if self.settings.roots.is_empty() {
                ui.label(RichText::new("Nessuna cartella.").color(MUTED));
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("Aggiungi cartella…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title("Cartella da analizzare").pick_folder() {
                        if !self.settings.roots.iter().any(|r| path_key(r) == path_key(&dir)) {
                            self.settings.roots.push(dir);
                            changed = true;
                        }
                    }
                }
                let can = self.root_sel.is_some_and(|i| i < self.settings.roots.len());
                if ui.add_enabled(can, egui::Button::new("Togli")).clicked() {
                    if let Some(i) = self.root_sel.take() {
                        self.settings.roots.remove(i);
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.save();
        }
        if let Some(e) = &self.settings_err {
            ui.add(egui::Label::new(RichText::new(format!("Impostazioni non salvate: {e}")).size(12.0).color(RED)).wrap());
        }
    }

    fn body(&mut self, ui: &mut Ui, busy: bool) {
        if busy {
            match self.tick {
                Some(t) => {
                    let text = if self.cancelling() { "Annullamento in corso…" } else { "Pulizia in corso…" };
                    ui.label(RichText::new(text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(tick_fraction(&t)).fill(ACCENT).text(tick_text(&t)));
                    ui.add_space(8.0);
                    if secondary(ui, "Annulla", !self.cancelling()).clicked() {
                        self.cancel_clean();
                    }
                    ui.add_space(10.0);
                }
                None => {
                    ui.label(RichText::new(&self.text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.progress).fill(ACCENT).show_percentage());
                    ui.add_space(10.0);
                }
            }
        }
        match self.view {
            View::Empty if busy => {
                ui.label(RichText::new("L'analisi può durare un minuto: intanto puoi usare le altre voci.").color(MUTED));
            }
            View::Empty => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Pulizia per sviluppatori").size(22.0).strong().color(TEXT));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Premi «Analizza»: i progetti vengono dalle sessioni di Claude Code, Codex, OpenCode e Pi.")
                            .color(MUTED),
                    );
                    if let Some(e) = &self.error {
                        ui.add_space(8.0);
                        ui.label(RichText::new(e).color(RED));
                    }
                });
            }
            View::Scanned => self.scanned_view(ui),
            View::Cleaned if busy && self.tick.is_some() => {}
            View::Cleaned => self.cleaned_view(ui, busy),
        }
    }

    fn scanned_view(&mut self, ui: &mut Ui) {
        let Some(scan) = self.scan.clone() else { return };
        let (selected, total, bytes) = self.selection();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Analisi completata").size(20.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let none = ui.small_button("Nessuno").clicked();
                let all = ui.small_button("Tutti").clicked();
                if none || all {
                    match self.section {
                        Section::Projects => select_all_artifacts(&scan.projects, &mut self.sel_artifacts, all),
                        Section::Worktrees => select_reports(&scan.worktrees, &mut self.sel_worktrees, all),
                        Section::Sessions => select_sessions(&scan.archived, &mut self.sel_sessions, all),
                    }
                }
            });
        });
        ui.label(
            RichText::new(format!("{} da liberare · {selected} di {total} selezionati", sys::fmt_size(bytes)))
                .size(15.0)
                .color(ACCENT),
        );
        let how = if scan.backend == "mft" { "dimensioni dalla MFT del disco" } else { "cartelle misurate una per una" };
        ui.label(RichText::new(format!("{} · {how}", fmt_elapsed(scan.elapsed_ms))).size(12.0).color(MUTED));
        match self.section {
            Section::Projects => {
                ui.label(
                    RichText::new("Ogni cartella ha la sua casella: «Consigliato» è proposto nei progetti inattivi, le altre le scegli tu.")
                        .size(12.0)
                        .color(MUTED),
                );
            }
            Section::Worktrees => {
                ui.label(
                    RichText::new("I worktree si rimuovono con git e sempre in modo definitivo; quelli con modifiche non salvate restano.")
                        .size(12.0)
                        .color(MUTED),
                );
            }
            Section::Sessions => {}
        }
        ui.add_space(8.0);
        match self.section {
            Section::Projects => {
                projects_table(ui, &scan.projects, &mut self.sel_artifacts, analysis_failed(&scan), self.screenshot)
            }
            Section::Worktrees => {
                worktrees_table(ui, &scan.worktrees, &scan.other_worktrees, &mut self.sel_worktrees, analysis_failed(&scan))
            }
            Section::Sessions => sessions_table(ui, &scan.archived, &mut self.sel_sessions),
        }
    }

    fn cleaned_view(&mut self, ui: &mut Ui, busy: bool) {
        let title = cleaned_title(self.cleaned.as_ref(), busy);
        let can_save = !busy && self.cleaned.as_ref().is_some_and(|c| !c.is_empty());
        let mut save = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            if can_save {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    save = secondary(ui, "Salva come testo…", true).clicked();
                });
            }
        });
        if save {
            self.save_report();
        }
        if let Some((text, ok)) = &self.saved {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *ok { GREEN } else { RED })).wrap());
        }
        let Some(c) = &self.cleaned else { return };
        if c.interrupted {
            let text = "Un errore interno ha fermato la pulizia: altri elementi potrebbero essere stati toccati senza comparire qui. Premi «Analizza» per vedere cosa resta.";
            ui.add(egui::Label::new(RichText::new(text).color(RED)).wrap());
        }
        if c.is_empty() {
            if c.errors.is_empty() && !c.interrupted {
                ui.add_space(10.0);
                ui.label(RichText::new("Nessun elemento da pulire.").color(MUTED));
            }
            return;
        }
        report_view(ui, &c.report, &|p| c.label(p), "dev_cleaned", |_| {});
    }
}

/// Il resoconto di una pulizia (Sviluppatori e Pulizia file): spazio liberato, conteggi,
/// durata, tabella per categoria, saltati e non riusciti. `label` dice come mostrare un
/// percorso dei saltati e dei non riusciti; `more` aggiunge righe in fondo, nello stesso
/// riquadro scorrevole.
pub fn report_view(ui: &mut Ui, r: &CleanReport, label: &dyn Fn(&Path) -> String, id_salt: &str, more: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(format!("Spazio liberato: {}", sys::fmt_size(r.freed_bytes))).size(15.0).color(GREEN));
    if r.recycled_bytes > 0 {
        ui.label(RichText::new(recycled_text(r.recycled_bytes)).color(TEXT));
    }
    ui.label(
        RichText::new(format!(
            "File eliminati: {} · Cartelle eliminate: {} · Durata: {}",
            r.files,
            r.dirs,
            fmt_duration(r.elapsed_ms)
        ))
        .size(12.0)
        .color(MUTED),
    );
    if r.cancelled {
        ui.label(RichText::new("Annullata: le voci non ancora iniziate sono rimaste dov'erano.").color(MUTED));
    }
    ui.add_space(10.0);
    let gap = ui.spacing().item_spacing.x;
    egui::ScrollArea::vertical().id_salt(id_salt).auto_shrink([false, false]).show(ui, |ui| {
        let total = ui.available_width() - SCROLLBAR;
        if !r.by_category.is_empty() {
            let w = col_widths(total, &[0.0, 120.0], gap);
            header(ui, &w, &["Categoria", "Spazio"]);
            for (i, (label, bytes)) in r.by_category.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], RichText::new(label).color(TEXT));
                    crate::cell(ui, w[1], RichText::new(sys::fmt_size(*bytes)).color(TEXT));
                });
            }
        }
        for (title, list, color) in [("Saltati", &r.skipped, MUTED), ("Non riusciti", &r.failed, RED)] {
            if list.is_empty() {
                continue;
            }
            ui.add_space(12.0);
            ui.label(RichText::new(format!("{title} ({})", list.len())).strong().color(TEXT));
            let w = col_widths(total, &[0.0, 300.0], gap);
            header(ui, &w, &["Elemento", "Motivo"]);
            for (i, (path, why)) in list.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], RichText::new(label(path)).color(TEXT)).on_hover_text(path.display().to_string());
                    crate::cell(ui, w[1], RichText::new(why).color(color)).on_hover_text(why);
                });
            }
        }
        more(ui);
    });
}

/// «Salva come testo…»: il resoconto in un file scelto dall'utente. `None` se l'utente
/// chiude il dialogo; altrimenti (messaggio, riuscito).
pub fn save_report_text(r: &CleanReport) -> Option<(String, bool)> {
    let path = rfd::FileDialog::new()
        .set_title("Salva il resoconto")
        .set_file_name("Resoconto Xiox.txt")
        .add_filter("Testo", &["txt"])
        .save_file()?;
    Some(match std::fs::write(&path, r.to_text()) {
        Ok(()) => (format!("Resoconto salvato in {}", path.display()), true),
        Err(e) => (format!("Resoconto non salvato: {e}"), false),
    })
}

/// La voce «Sviluppatori»: pannello opzioni, pulsanti in basso, card centrale, conferma.
pub fn dev_tab(state: &mut DevState, ui: &mut Ui, admin: bool) {
    if state.auto_scan && !state.busy() {
        state.auto_scan = false;
        let ctx = ui.ctx().clone();
        state.start_scan(&ctx, admin);
    }
    #[cfg(debug_assertions)]
    {
        let ctx = ui.ctx().clone();
        state.run_demo(&ctx);
    }
    let busy = state.busy();
    egui::SidePanel::left("dev_opts")
        .exact_width(270.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| state.options(ui, busy));
        });
    egui::TopBottomPanel::bottom("dev_buttons")
        .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let (selected, _, bytes) = state.selection();
                if state.scan.is_some() {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("Selezionati in {}: {}", state.section.name(), sys::fmt_size(bytes))).color(TEXT));
                        ui.label(RichText::new(selection_text(state.section, selected)).size(12.0).color(MUTED));
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary(ui, &clean_button(state.section, selected), state.can_clean()).clicked() {
                        state.open_confirm();
                    }
                    if secondary(ui, "Analizza", !busy).clicked() {
                        let ctx = ui.ctx().clone();
                        state.start_scan(&ctx, admin);
                    }
                });
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
        match (&state.view, &state.scan, &state.cleaned) {
            (View::Scanned, Some(scan), _) => {
                if let Some(n) = &state.notice {
                    notice_strip(ui, n);
                }
                warn_strip(ui, &scan.warnings)
            }
            // Un piano o una scelta di sessioni rifiutati: niente è stato toccato e non si
            // riprova da soli, serve una nuova analisi.
            (View::Cleaned, _, Some(c)) if !c.errors.is_empty() => error_strip(ui, &c.errors),
            _ => {}
        }
        card(ui, |ui| state.body(ui, busy));
    });
    confirm_dialog(state, ui.ctx());
}

fn confirm_dialog(state: &mut DevState, ctx: &egui::Context) {
    let Some(c) = &mut state.confirm else { return };
    let mut go = false;
    let mut permanent = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("dev_confirm")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(RichText::new(confirm_title(c.section)).size(18.0).strong());
        ui.add_space(6.0);
        ui.add(egui::Label::new(confirm_text(c.section, shown_mode(c))).wrap());
        if let Some(o) = &c.others {
            ui.add_space(4.0);
            ui.add(egui::Label::new(o.as_str()).wrap());
        }
        ui.add_space(8.0);
        if let Some(e) = &c.error {
            ui.add(egui::Label::new(RichText::new(e).color(RED)).wrap());
        }
        for s in &c.summary {
            ui.label(RichText::new(s).color(ACCENT));
        }
        if !c.rows.is_empty() {
            ui.add_space(4.0);
            let w = col_widths(ui.available_width() - SCROLLBAR, &[0.0, 90.0], ui.spacing().item_spacing.x);
            egui::ScrollArea::vertical()
                .id_salt("dev_confirm_list")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, c.rows.len(), |ui, range| {
                    for i in range {
                        let (label, size) = &c.rows[i];
                        striped_row(ui, i, |ui| {
                            crate::cell(ui, w[0], label.as_str());
                            crate::cell(ui, w[1], RichText::new(size).color(MUTED));
                        });
                    }
                });
        }
        if !c.cautions.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new(caution_title(c.cautions.len())).strong().color(RED));
            egui::ScrollArea::vertical().id_salt("dev_confirm_cautions").max_height(150.0).auto_shrink([false, true]).show(
                ui,
                |ui| {
                    for (label, note) in &c.cautions {
                        ui.label(RichText::new(label).color(TEXT));
                        ui.add(egui::Label::new(RichText::new(note).size(12.0).color(MUTED)).wrap());
                        ui.add_space(2.0);
                    }
                },
            );
            ui.checkbox(&mut c.ack, "Ho letto gli avvisi");
        }
        if !c.shortfall.is_empty() {
            ui.add_space(10.0);
            ui.add(egui::Label::new(RichText::new(shortfall_text(&c.shortfall)).strong().color(TEXT)).wrap());
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = confirm_ready(c);
            if c.shortfall.is_empty() {
                go = primary(ui, &confirm_button(c), ready).clicked();
            } else {
                permanent = primary(ui, "Definitiva", ready).clicked();
            }
            close = secondary(ui, "Annulla", true).clicked();
        });
    });
    if permanent {
        if let Some(c) = state.confirm.as_mut() {
            if let Err(e) = make_permanent(c) {
                c.error = Some(e);
                return;
            }
        }
        go = true;
    }
    if go {
        if let Some(c) = state.confirm.take().and_then(|c| state.accept_confirm(c, xiox_app::now_ms())) {
            state.start_clean(ctx, c);
        }
    } else if close {
        state.confirm = None;
    }
}

/// Il modo che il pulsante applica: con il Cestino troppo piccolo c'è solo «Definitiva».
fn shown_mode(c: &Confirm) -> RemoveMode {
    if c.shortfall.is_empty() {
        c.mode
    } else {
        RemoveMode::Permanent
    }
}

/// «Definitiva» dopo l'avviso sul Cestino: lo stesso piano, rifatto in modo definitivo.
fn make_permanent(c: &mut Confirm) -> Result<(), String> {
    let plan = xiox_app::build_plan(&c.scan, &c.sel, RemoveMode::Permanent)?;
    c.plan = Some(plan);
    c.mode = RemoveMode::Permanent;
    c.shortfall.clear();
    Ok(())
}

/// «Pulisci» (o «Definitiva») si può premere: un piano valido e non vuoto, e gli avvisi letti
/// quando ci sono cartelle al livello Attenzione.
fn confirm_ready(c: &Confirm) -> bool {
    c.error.is_none() && !c.rows.is_empty() && (c.cautions.is_empty() || c.ack)
}

fn caution_title(n: usize) -> String {
    if n == 1 {
        "1 cartella al livello Attenzione:".into()
    } else {
        format!("{n} cartelle al livello Attenzione:")
    }
}

/// L'avviso sul Cestino troppo piccolo, una frase per unità e poi la domanda.
fn shortfall_text(short: &[(char, u64, u64)]) -> String {
    let mut parts: Vec<String> = short
        .iter()
        .map(|(d, need, room)| {
            format!("Il Cestino di {d}: ha {} liberi, la pulizia ne sposta {}.", sys::fmt_size(*room), sys::fmt_size(*need))
        })
        .collect();
    parts.push("Eliminare definitivamente questa volta?".into());
    parts.join(" ")
}

fn confirm_title(section: Section) -> &'static str {
    match section {
        Section::Projects => "Pulire le cartelle selezionate?",
        Section::Worktrees => "Rimuovere i worktree selezionati?",
        Section::Sessions => "Cancellare le sessioni archiviate selezionate?",
    }
}

fn confirm_text(section: Section, mode: RemoveMode) -> &'static str {
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

fn confirm_button(c: &Confirm) -> String {
    clean_button(c.section, c.selected)
}

// ---------- tables ----------

/// Colonne dei progetti: freccia, casella, nome e stato (cartella e avviso), ultimo uso
/// (livello), dimensione.
const PROJECT_COLS: [f32; 5] = [CHECK_W, CHECK_W, 0.0, 120.0, 80.0];

/// Ogni progetto è una riga con la casella a tre stati e si apre sulle sue cartelle, ognuna con
/// la sua casella, il livello, la dimensione e l'avviso. Nulla è nascosto: le cartelle dei
/// progetti in uso o bloccati restano visibili, con la casella spenta.
fn projects_table(ui: &mut Ui, reports: &[ProjectReport], sel: &mut HashSet<String>, failed: bool, open_all: bool) {
    if reports.is_empty() {
        empty_note(ui, empty_text(Section::Projects, failed));
        return;
    }
    let w = col_widths(ui.available_width() - SCROLLBAR, &PROJECT_COLS, ui.spacing().item_spacing.x);
    header(ui, &w, &["", "", "Progetto · cartella", "Ultimo uso · livello", "Dimensione"]);
    egui::ScrollArea::vertical().id_salt("dev_projects").auto_shrink([false, false]).show(ui, |ui| {
        let mut row = 0;
        for r in reports {
            let free = status_allows(r);
            let locked = (!free).then(|| locked_why(r));
            let locked = locked.as_deref();
            let color = if free { TEXT } else { MUTED };
            let mut open = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("dev_project", &r.id)), open_all);
            if open_all {
                open.set_open(true);
            }
            striped_row(ui, row, |ui| {
                ui.allocate_ui_with_layout(Vec2::new(w[0], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(w[0]);
                    ui.set_max_width(w[0]);
                    if !r.artifacts.is_empty() {
                        open.show_toggle_button(ui, paint_default_icon);
                    }
                });
                let tri = tri_state(r, sel);
                let mut all = tri == Tri::All;
                let check = ui
                    .allocate_ui_with_layout(Vec2::new(w[1], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                        ui.set_min_width(w[1]);
                        ui.set_max_width(w[1]);
                        ui.add_enabled(
                            free && !r.artifacts.is_empty(),
                            egui::Checkbox::without_text(&mut all).indeterminate(tri == Tri::Some),
                        )
                    })
                    .inner;
                if check.clicked() {
                    toggle_project(r, sel);
                }
                if let Some(why) = locked {
                    check.on_disabled_hover_text(why);
                }
                ui.allocate_ui_with_layout(Vec2::new(w[2], ROW_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(w[2]);
                    ui.set_max_width(w[2]);
                    ui.add(egui::Label::new(RichText::new(&r.name).strong().color(color)))
                        .on_hover_text(r.root.display().to_string());
                    // Il motivo intero, a capo se serve: la riga si alza.
                    ui.add(egui::Label::new(RichText::new(&r.reason).size(12.0).color(MUTED)).wrap());
                });
                crate::cell(ui, w[3], RichText::new(r.last_activity_ms.map_or_else(|| "—".to_string(), fmt_date)).color(color));
                crate::cell(ui, w[4], RichText::new(sys::fmt_size(r.reclaimable_bytes)).color(color));
            });
            row += 1;
            open.show_body_unindented(ui, |ui| {
                for a in &r.artifacts {
                    striped_row(ui, row, |ui| artifact_cells(ui, &w, a, sel, locked));
                    row += 1;
                }
            });
        }
    });
}

/// Una cartella di un progetto: casella, etichetta con l'avviso sotto, livello, dimensione.
/// `locked`: perché il progetto non si pulisce (casella spenta, spiegata al passaggio).
fn artifact_cells(ui: &mut Ui, w: &[f32], a: &Artifact, sel: &mut HashSet<String>, locked: Option<&str>) {
    let free = locked.is_none();
    crate::cell(ui, w[0], "");
    let mut on = sel.contains(&a.id);
    let check = check_cell(ui, w[1], &mut on, free);
    if check.changed() {
        set_artifact(sel, a, on);
    }
    if let Some(why) = locked {
        check.on_disabled_hover_text(why);
    }
    let color = if free { TEXT } else { MUTED };
    ui.allocate_ui_with_layout(Vec2::new(w[2], ROW_H), Layout::top_down(Align::Min), |ui| {
        ui.set_min_width(w[2]);
        ui.set_max_width(w[2]);
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add(egui::Label::new(RichText::new(&a.label).color(color)).truncate()).on_hover_text(a.path.display().to_string());
        if !a.note.is_empty() {
            // L'avviso intero («Si riscarica con…», «Potrebbe contenere sorgenti…»), a capo se serve.
            ui.add(egui::Label::new(RichText::new(&a.note).size(12.0).color(MUTED)).wrap());
        }
    });
    let (level, level_color) = level_text(a.level);
    crate::cell(ui, w[3], RichText::new(level).color(if free { level_color } else { MUTED }));
    crate::cell(ui, w[4], RichText::new(sys::fmt_size(a.size.effective_bytes())).color(color));
}

fn worktrees_table(ui: &mut Ui, reports: &[ProjectReport], others: &[ProjectReport], sel: &mut [bool], failed: bool) {
    let w = col_widths(ui.available_width() - SCROLLBAR, &[CHECK_W, 0.0, 110.0, 150.0, 86.0], ui.spacing().item_spacing.x);
    let titles = ["", "Worktree", "Repository", "Stato", "Dimensione"];
    if reports.is_empty() {
        empty_note(ui, empty_text(Section::Worktrees, failed));
    } else {
        header(ui, &w, &titles);
    }
    egui::ScrollArea::vertical().id_salt("dev_worktrees").auto_shrink([false, false]).show(ui, |ui| {
        for (i, r) in reports.iter().enumerate() {
            let on = selectable(r);
            let why = nothing_to_remove(r);
            striped_row(ui, i, |ui| {
                let check = check_cell(ui, w[0], &mut sel[i], on);
                if let Some(why) = why {
                    check.on_disabled_hover_text(why);
                }
                worktree_cells(ui, &w, r, if on { TEXT } else { MUTED }, why);
            });
        }
        if !others.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Altri worktree (creati a mano, non si rimuovono da qui)").color(MUTED));
            ui.add_space(2.0);
            if reports.is_empty() {
                header(ui, &w, &titles);
            }
            for (i, r) in others.iter().enumerate() {
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], "");
                    worktree_cells(ui, &w, r, MUTED, None);
                });
            }
        }
    });
}

/// `why`: perché la riga non ha nulla da togliere, al posto dello stato e in grigio come le
/// righe bloccate.
fn worktree_cells(ui: &mut Ui, w: &[f32], r: &ProjectReport, color: Color32, why: Option<&str>) {
    crate::cell(ui, w[1], RichText::new(&r.name).color(color)).on_hover_text(r.root.display().to_string());
    let repo = r.git.main_worktree.as_deref();
    let resp = crate::cell(ui, w[2], RichText::new(repo.map_or_else(|| "—".to_string(), folder_name)).color(color));
    if let Some(p) = repo {
        resp.on_hover_text(p.display().to_string());
    }
    match why {
        Some(why) => crate::cell(ui, w[3], RichText::new(why).size(12.0).color(MUTED)),
        None => crate::cell(ui, w[3], RichText::new(&r.reason).color(color)),
    };
    crate::cell(ui, w[4], RichText::new(sys::fmt_size(r.reclaimable_bytes)).color(color));
}

fn sessions_table(ui: &mut Ui, sessions: &[ArchivedSession], sel: &mut [bool]) {
    if sessions.is_empty() {
        empty_note(ui, empty_text(Section::Sessions, false));
        return;
    }
    let w = col_widths(ui.available_width() - SCROLLBAR, &[CHECK_W, 76.0, 0.0, 80.0, 74.0, 116.0], ui.spacing().item_spacing.x);
    header(ui, &w, &["", "Harness", "Titolo", "Progetto", "Data", "Dimensione"]);
    egui::ScrollArea::vertical().id_salt("dev_sessions").auto_shrink([false, false]).show_rows(ui, ROW_H, sessions.len(), |ui, range| {
        for i in range {
            let s = &sessions[i];
            let on = s.blocked.is_none();
            let color = if on { TEXT } else { MUTED };
            striped_row(ui, i, |ui| {
                let check = check_cell(ui, w[0], &mut sel[i], on);
                if let Some(why) = &s.blocked {
                    check.on_disabled_hover_text(why);
                }
                crate::cell(ui, w[1], RichText::new(agent_label(s.agent)).color(color));
                crate::cell(ui, w[2], RichText::new(session_title(s)).color(color));
                let project = crate::cell(ui, w[3], RichText::new(s.cwd.as_deref().map_or_else(|| "—".to_string(), folder_name)).color(color));
                if let Some(cwd) = &s.cwd {
                    project.on_hover_text(cwd.display().to_string());
                }
                crate::cell(ui, w[4], RichText::new(s.updated_ms.map_or_else(|| "—".to_string(), fmt_date)).color(color));
                match &s.blocked {
                    Some(why) => crate::cell(ui, w[5], RichText::new(why).size(12.0).color(MUTED)),
                    None if s.file.is_none() => crate::cell(ui, w[5], RichText::new(session_size(s)).size(12.0).color(MUTED)),
                    None => crate::cell(ui, w[5], RichText::new(session_size(s)).color(TEXT)),
                };
            });
        }
    });
}

fn group(ui: &mut Ui, title: &str, space: f32) {
    ui.add_space(space);
    ui.label(RichText::new(title).strong().color(ACCENT));
    ui.separator();
}

fn header(ui: &mut Ui, w: &[f32], titles: &[&str]) {
    ui.horizontal(|ui| {
        for (w, t) in w.iter().zip(titles) {
            crate::cell(ui, *w, RichText::new(*t).strong());
        }
    });
    ui.separator();
}

fn empty_note(ui: &mut Ui, text: &str) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| ui.label(RichText::new(text).size(16.0).color(MUTED)));
    ui.add_space(10.0);
}

/// Una riga di tabella alta [`ROW_H`]; le righe dispari hanno lo sfondo [`STRIPE`].
fn striped_row(ui: &mut Ui, i: usize, add: impl FnOnce(&mut Ui)) {
    let bg = ui.painter().add(egui::Shape::Noop);
    let r = ui
        .horizontal(|ui| {
            ui.set_min_height(ROW_H);
            add(ui)
        })
        .response
        .rect;
    if i % 2 == 1 {
        let half = ui.spacing().item_spacing.y / 2.0;
        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), (r.top() - half)..=(r.bottom() + half));
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 3.0, STRIPE));
    }
}

fn check_cell(ui: &mut Ui, w: f32, value: &mut bool, enabled: bool) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, ROW_H), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add_enabled(enabled, egui::Checkbox::without_text(value))
    })
    .inner
}

/// Gli avvisi dell'analisi, il fallimento dell'analisi dei progetti per primo e in evidenza.
fn warn_strip(ui: &mut Ui, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    let list = ordered_warnings(warnings);
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for w in list.iter().take(2) {
            if w.as_str() == ANALYSIS_FAILED {
                let text = format!("⚠  {w} I progetti e i worktree non sono stati analizzati: premi «Analizza» per riprovare.");
                ui.add(egui::Label::new(RichText::new(text).strong().color(TEXT)).wrap());
            } else {
                ui.add(egui::Label::new(RichText::new(format!("⚠  {w}")).color(TEXT)).truncate());
            }
        }
        if list.len() > 2 {
            let rest: Vec<&str> = list[2..].iter().map(|w| w.as_str()).collect();
            ui.label(RichText::new(format!("e altri {} avvisi", list.len() - 2)).size(12.0).color(MUTED))
                .on_hover_text(rest.join("\n"));
        }
    });
    ui.add_space(8.0);
}

/// Un avviso sul pulsante appena premuto (per esempio l'analisi troppo vecchia), niente dialogo.
fn notice_strip(ui: &mut Ui, text: &str) {
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(format!("⚠  {text}")).strong().color(TEXT)).wrap());
    });
    ui.add_space(8.0);
}

/// Richieste rifiutate da `xiox_app` (per esempio «Piano rifiutato: …»): per intero, mai troncate.
fn error_strip(ui: &mut Ui, errors: &[String]) {
    egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for e in errors {
            ui.add(egui::Label::new(RichText::new(format!("⚠  {e}")).color(TEXT)).wrap());
        }
        ui.label(RichText::new("Quella parte non è stata toccata. Premi «Analizza» per rifare l'analisi prima di riprovare.").strong().color(TEXT));
    });
    ui.add_space(8.0);
}

// ---------- pure helpers ----------

/// Lo stato e i blocchi che `xiox_engine::build_plan` lascia passare: non in uso, non bloccato.
/// Perché le caselle di un progetto che [`status_allows`] ferma sono spente.
fn locked_why(r: &ProjectReport) -> String {
    match r.status {
        ProjectStatus::InUse => {
            "Il progetto è in uso da un agente: le sue cartelle si puliscono quando la sessione è chiusa.".into()
        }
        ProjectStatus::Unreachable => format!("Il progetto non è raggiungibile e non si pulisce: {}", r.reason),
        _ => format!("Il progetto è bloccato e non si pulisce: {}", r.reason),
    }
}

fn status_allows(r: &ProjectReport) -> bool {
    matches!(r.status, ProjectStatus::Inactive | ProjectStatus::Recent | ProjectStatus::NoSessions)
        && r.blocked_by.is_empty()
}

/// Quello che `xiox_engine::build_plan` accetta E da cui ricava almeno un elemento
/// (`xiox_app::plans_something`, la stessa regola): un worktree con modifiche non salvate, o di
/// stato git ignoto, senza artefatti darebbe un piano vuoto.
fn selectable(r: &ProjectReport) -> bool {
    status_allows(r) && xiox_app::plans_something(r)
}

/// Perché una riga che lo stato lascerebbe passare non ha nulla da togliere; `None` altrimenti
/// (allora il `reason` del report dice già tutto).
fn nothing_to_remove(r: &ProjectReport) -> Option<&'static str> {
    if !status_allows(r) || xiox_app::plans_something(r) {
        return None;
    }
    Some(match (r.is_agent_worktree, r.git.dirty) {
        (true, Some(true)) => "Modifiche non salvate: il worktree resta",
        (true, None) => "Stato di git non leggibile: il worktree resta",
        _ => "Nulla da togliere",
    })
}

fn preselect_reports(reports: &[ProjectReport]) -> Vec<bool> {
    reports.iter().map(|r| r.preselected && selectable(r)).collect()
}

fn select_reports(reports: &[ProjectReport], sel: &mut [bool], all: bool) {
    for (s, r) in sel.iter_mut().zip(reports) {
        *s = all && selectable(r);
    }
}

fn select_sessions(sessions: &[ArchivedSession], sel: &mut [bool], all: bool) {
    for (s, x) in sel.iter_mut().zip(sessions) {
        *s = all && x.blocked.is_none();
    }
}

fn chosen_reports<'a>(reports: &'a [ProjectReport], sel: &'a [bool]) -> impl Iterator<Item = &'a ProjectReport> {
    reports.iter().zip(sel).filter(|(r, s)| **s && selectable(r)).map(|(r, _)| r)
}

fn chosen_sessions<'a>(sessions: &'a [ArchivedSession], sel: &'a [bool]) -> impl Iterator<Item = &'a ArchivedSession> {
    sessions.iter().zip(sel).filter(|(s, on)| **on && s.blocked.is_none()).map(|(s, _)| s)
}

/// (selezionati, totale, byte selezionati) di una tabella di progetti o worktree.
fn report_stats(reports: &[ProjectReport], sel: &[bool]) -> (usize, usize, u64) {
    let chosen: Vec<&ProjectReport> = chosen_reports(reports, sel).collect();
    (chosen.len(), reports.len(), chosen.iter().map(|r| r.reclaimable_bytes).sum())
}

fn session_stats(sessions: &[ArchivedSession], sel: &[bool]) -> (usize, usize, u64) {
    let chosen: Vec<&ArchivedSession> = chosen_sessions(sessions, sel).collect();
    (chosen.len(), sessions.len(), chosen.iter().map(|s| s.bytes).sum())
}

/// Esegue la pulizia; un panico non perde gli esiti già raccolti in `out` e la segna interrotta.
fn guarded(out: &mut DevCleaned, work: impl FnOnce(&mut DevCleaned)) {
    if catch_unwind(AssertUnwindSafe(|| work(&mut *out))).is_err() {
        out.interrupted = true;
    }
}

/// (selezionati, totale, byte selezionati) di una sola sezione; nei progetti si contano le cartelle.
fn section_stats(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> (usize, usize, u64) {
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
struct Action {
    artifacts: Vec<String>,
    worktrees: Vec<String>,
    sessions: Vec<(Agent, String)>,
}

impl Action {
    /// La scelta per `xiox_app::build_plan`: le cartelle come sono; ogni worktree d'agente intero,
    /// o le sue cartelle consigliate (la regola di `xiox_app::selection_for_projects`).
    fn selection(&self, scan: &DevScan) -> Selection {
        let mut sel = if self.worktrees.is_empty() {
            Selection::default()
        } else {
            xiox_app::selection_for_projects(scan, &self.worktrees)
        };
        sel.artifacts.extend(self.artifacts.iter().cloned());
        sel
    }
}

fn section_action(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> Action {
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
fn selection_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "cartella", "cartelle"),
        Section::Worktrees => plural(n, "worktree", "worktree"),
        Section::Sessions => plural(n, "sessione", "sessioni"),
    }
}

/// Cosa c'è di scelto in una sezione, con il nome della cosa: «3 cartelle selezionate».
fn selected_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "cartella selezionata", "cartelle selezionate"),
        Section::Worktrees => plural(n, "worktree selezionato", "worktree selezionati"),
        Section::Sessions => plural(n, "sessione selezionata", "sessioni selezionate"),
    }
}

/// Il pulsante che pulisce la sezione visibile: il verbo della sezione e, se c'è una scelta, quanto.
fn clean_button(section: Section, n: usize) -> String {
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
fn others_note(scan: &DevScan, section: Section, sa: &HashSet<String>, sw: &[bool], ss: &[bool]) -> Option<String> {
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
enum Tri {
    All,
    None,
    Some,
}

fn tri_state(r: &ProjectReport, sel: &HashSet<String>) -> Tri {
    let on = r.artifacts.iter().filter(|a| sel.contains(&a.id)).count();
    match on {
        0 => Tri::None,
        n if n == r.artifacts.len() => Tri::All,
        _ => Tri::Some,
    }
}

/// Clic sulla casella del progetto: da «tutte» a nessuna, da «nessuna» o «alcune» a tutte.
/// Un progetto in uso o bloccato non cambia.
fn toggle_project(r: &ProjectReport, sel: &mut HashSet<String>) {
    if !status_allows(r) {
        return;
    }
    let all = tri_state(r, sel) != Tri::All;
    for a in &r.artifacts {
        set_artifact(sel, a, all);
    }
}

fn set_artifact(sel: &mut HashSet<String>, a: &Artifact, on: bool) {
    if on {
        sel.insert(a.id.clone());
    } else {
        sel.remove(&a.id);
    }
}

/// La preselezione del motore (`Artifact::preselected`), mai nei progetti in uso o bloccati.
fn preselect_artifacts(reports: &[ProjectReport]) -> HashSet<String> {
    reports
        .iter()
        .filter(|r| status_allows(r))
        .flat_map(|r| r.artifacts.iter().filter(|a| a.preselected).map(|a| a.id.clone()))
        .collect()
}

/// «Tutti» e «Nessuno» nei progetti: ogni cartella dei progetti liberi, qualunque livello.
fn select_all_artifacts(reports: &[ProjectReport], sel: &mut HashSet<String>, all: bool) {
    sel.clear();
    if all {
        sel.extend(reports.iter().filter(|r| status_allows(r)).flat_map(|r| r.artifacts.iter().map(|a| a.id.clone())));
    }
}

/// Le cartelle scelte dei progetti liberi.
fn chosen_artifacts<'a>(
    reports: &'a [ProjectReport],
    sel: &'a HashSet<String>,
) -> impl Iterator<Item = (&'a ProjectReport, &'a Artifact)> {
    reports
        .iter()
        .filter(|r| status_allows(r))
        .flat_map(move |r| r.artifacts.iter().filter(|a| sel.contains(&a.id)).map(move |a| (r, a)))
}

/// (cartelle scelte, cartelle in tutto, byte scelti).
fn artifact_stats(reports: &[ProjectReport], sel: &HashSet<String>) -> (usize, usize, u64) {
    let total = reports.iter().map(|r| r.artifacts.len()).sum();
    let (n, bytes) = chosen_artifacts(reports, sel)
        .fold((0, 0u64), |(n, b), (_, a)| (n + 1, b.saturating_add(a.size.effective_bytes())));
    (n, total, bytes)
}

fn level_text(level: Level) -> (&'static str, Color32) {
    match level {
        Level::Recommended => ("Consigliato", GREEN),
        Level::Optional => ("Facoltativo", ACCENT),
        Level::Caution => ("Attenzione", RED),
    }
}

/// Le cartelle Attenzione di un piano, con il loro avviso: la conferma le elenca.
fn plan_cautions(scan: &DevScan, plan: &CleanPlan) -> Vec<(String, String)> {
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
fn merge_report(into: &mut CleanReport, r: CleanReport) {
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
const SESSIONS_CATEGORY: &str = "Sessioni archiviate";

/// L'esito di una sessione nel resoconto: il file conta tra i file eliminati, i byte tra
/// liberati o nel Cestino secondo il modo.
fn add_session_outcome(into: &mut CleanReport, label: &str, outcome: &ItemOutcome, mode: RemoveMode) {
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
fn add_category(into: &mut CleanReport, label: &str, bytes: u64) {
    match into.by_category.iter_mut().find(|(l, _)| l == label) {
        Some((_, b)) => *b = b.saturating_add(bytes),
        None => into.by_category.push((label.to_string(), bytes)),
    }
    into.by_category.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

fn cleaned_title(c: Option<&DevCleaned>, busy: bool) -> &'static str {
    match c {
        _ if busy => "Pulizia in corso…",
        Some(c) if c.interrupted => "Pulizia interrotta",
        Some(c) if c.is_empty() && !c.errors.is_empty() => "Pulizia non eseguita",
        Some(c) if c.report.cancelled => "Pulizia annullata",
        _ => "Pulizia completata",
    }
}

fn recycled_text(bytes: u64) -> String {
    format!("{} nel Cestino: si liberano svuotandolo", sys::fmt_size(bytes))
}

/// Avanzamento della barra: a byte, o a voci quando i byte non si conoscono.
fn tick_fraction(t: &Tick) -> f32 {
    if t.total_bytes > 0 {
        (t.done_bytes as f64 / t.total_bytes as f64).clamp(0.0, 1.0) as f32
    } else if t.total_items > 0 {
        (t.done_items as f32 / t.total_items as f32).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// «1,2 GB di 3,4 GB · 2 di 5 voci».
fn tick_text(t: &Tick) -> String {
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
fn fmt_duration(ms: u64) -> String {
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
fn only_root_settings(mut s: Settings, dir: PathBuf) -> Settings {
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
fn keep_only_under(scan: &mut DevScan, root: &Path) {
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
fn demo_allowed(persist: bool, roots: &[PathBuf]) -> Result<(), String> {
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

fn analysis_failed(scan: &DevScan) -> bool {
    scan.warnings.iter().any(|w| w == ANALYSIS_FAILED)
}

/// Il fallimento dell'analisi prima degli altri avvisi.
fn ordered_warnings(warnings: &[String]) -> Vec<&String> {
    let (mut first, rest): (Vec<&String>, Vec<&String>) = warnings.iter().partition(|w| w.as_str() == ANALYSIS_FAILED);
    first.extend(rest);
    first
}

/// Cosa dire in una sezione vuota: dopo un'analisi fallita le liste vuote non significano «niente».
fn empty_text(section: Section, failed: bool) -> &'static str {
    match section {
        Section::Projects | Section::Worktrees if failed => "Analisi dei progetti non riuscita: premi «Analizza» per riprovare.",
        Section::Projects => "Nessun progetto trovato.",
        Section::Worktrees => "Nessun worktree creato da un agente.",
        Section::Sessions => "Nessuna sessione archiviata.",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn section_label(s: Section, count: Option<usize>) -> String {
    match count {
        Some(n) => format!("{} ({n})", s.name()),
        None => s.name().to_string(),
    }
}

/// Larghezze delle colonne: la colonna `0.0` prende quello che resta (almeno 80 px).
fn col_widths(total: f32, cols: &[f32], gap: f32) -> Vec<f32> {
    let fixed: f32 = cols.iter().sum();
    let gaps = gap * cols.len().saturating_sub(1) as f32;
    let flex = (total - fixed - gaps).max(80.0);
    cols.iter().map(|&w| if w == 0.0 { flex } else { w }).collect()
}

fn agent_label(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
        Agent::Opencode => "OpenCode",
        Agent::Pi => "Pi",
    }
}

fn session_title(s: &ArchivedSession) -> String {
    let t = s.title.as_deref().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        "(senza titolo)".into()
    } else {
        t
    }
}

fn session_label(s: &ArchivedSession) -> String {
    format!("{} · {}", agent_label(s.agent), session_title(s))
}

fn session_size(s: &ArchivedSession) -> String {
    if s.file.is_none() {
        "solo riga nel database".into()
    } else {
        sys::fmt_size(s.bytes)
    }
}

/// «progetto · Build Rust (target)» per una cartella, «progetto · worktree intero» per un worktree.
fn plan_label(scan: &DevScan, item: &PlanItem) -> String {
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

fn folder_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Data locale `gg/mm/aaaa` da millisecondi epoch.
fn fmt_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%d/%m/%Y").to_string())
        .unwrap_or_else(|| "—".into())
}

fn fmt_elapsed(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    if s < 60 {
        format!("Analisi in {s} s")
    } else {
        format!("Analisi in {} min {} s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiox_model::{Artifact, ArtifactKind, BlockReason, GitState, SizeInfo};
    use std::path::PathBuf;

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

    /// L'id della cartella `target` del progetto `id` di [`report`].
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

    #[test]
    fn only_free_projects_are_selectable() {
        assert!(selectable(&report("a", ProjectStatus::Inactive, true, 1)));
        assert!(selectable(&report("b", ProjectStatus::Recent, false, 1)));
        assert!(selectable(&report("c", ProjectStatus::NoSessions, false, 1)));
        for st in [ProjectStatus::InUse, ProjectStatus::Blocked, ProjectStatus::Unreachable] {
            assert!(!selectable(&report("x", st, false, 1)), "{st:?}");
        }
        let mut blocked = report("d", ProjectStatus::Recent, false, 1);
        blocked.blocked_by.push(BlockReason::DirtyGit);
        assert!(!selectable(&blocked));
        for st in [ProjectStatus::InUse, ProjectStatus::Blocked, ProjectStatus::Unreachable] {
            let mut r = report("x", st, false, 1);
            r.reason = "modifiche non salvate".into();
            let why = locked_why(&r);
            assert!(why.starts_with("Il progetto "), "{why}");
            assert_eq!(why.contains("modifiche non salvate"), st != ProjectStatus::InUse, "{why}");
        }
        let mut empty = report("e", ProjectStatus::Inactive, false, 0);
        empty.artifacts.clear();
        assert!(!selectable(&empty), "un progetto senza artefatti non ha nulla da togliere");
        assert_eq!(nothing_to_remove(&empty), Some("Nulla da togliere"));
        empty.is_agent_worktree = true;
        empty.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
        empty.git.dirty = Some(false);
        assert!(selectable(&empty), "un worktree pulito si toglie intero");
        assert_eq!(nothing_to_remove(&empty), None);
    }

    #[test]
    fn a_worktree_with_nothing_to_remove_is_greyed_out_with_the_reason() {
        let mut dirty = report("d", ProjectStatus::Inactive, true, 40);
        dirty.is_agent_worktree = true;
        dirty.artifacts.clear();
        dirty.git.main_worktree = Some(PathBuf::from(r"D:\repo"));
        dirty.git.dirty = Some(true);
        let mut unknown = dirty.clone();
        unknown.id = "u".into();
        unknown.git.dirty = None;
        let mut clean = dirty.clone();
        clean.id = "c".into();
        clean.git.dirty = Some(false);
        // Lo stesso giudizio di xiox_engine::build_plan: piano vuoto per i primi due.
        for (r, want) in [(&dirty, false), (&unknown, false), (&clean, true)] {
            let plan = dc_engine_plan_len(r);
            assert_eq!(selectable(r), want, "{}", r.id);
            assert_eq!(plan > 0, want, "{}", r.id);
        }
        assert_eq!(nothing_to_remove(&dirty), Some("Modifiche non salvate: il worktree resta"));
        assert_eq!(nothing_to_remove(&unknown), Some("Stato di git non leggibile: il worktree resta"));
        // Un worktree in uso lo dice già il suo stato.
        let mut busy = dirty.clone();
        busy.status = ProjectStatus::InUse;
        assert_eq!(nothing_to_remove(&busy), None);

        let rs = vec![dirty, unknown, clean];
        assert_eq!(preselect_reports(&rs), vec![false, false, true]);
        let mut sel = vec![false; 3];
        select_reports(&rs, &mut sel, true);
        assert_eq!(sel, vec![false, false, true]);
        assert_eq!(report_stats(&rs, &[true, true, true]), (1, 3, 40), "i byte dei worktree che restano non contano");
    }

    fn dc_engine_plan_len(r: &ProjectReport) -> usize {
        let scan = DevScan { worktrees: vec![r.clone()], ..Default::default() };
        let sel = xiox_app::selection_for_projects(&scan, &[r.id.clone()]);
        xiox_app::build_plan(&scan, &sel, RemoveMode::Recycle).unwrap().items.len()
    }

    #[test]
    fn preselection_follows_the_engine_but_never_a_blocked_row() {
        let mut odd = report("c", ProjectStatus::Blocked, true, 5);
        odd.preselected = true;
        let rs = vec![report("a", ProjectStatus::Inactive, true, 1), report("b", ProjectStatus::Recent, false, 2), odd];
        assert_eq!(preselect_reports(&rs), vec![true, false, false]);
    }

    #[test]
    fn sessions_start_unticked_and_blocked_never_chosen() {
        let ss = vec![session("1", None, false, 0), session("2", Some("sessione in uso"), true, 10), session("3", None, true, 7)];
        let mut st = state();
        st.set_scan(DevScan { archived: ss.clone(), ..Default::default() });
        assert_eq!(st.sel_sessions, vec![false, false, false], "dopo l'analisi nessuna sessione è spuntata");
        st.set_section(Section::Sessions);
        assert!(!st.can_clean(), "nulla scelto: «Avvia pulizia» resta spento");
        assert_eq!(st.selection(), (0, 3, 0));
        // «Tutti» spunta le non bloccate.
        select_sessions(&ss, &mut st.sel_sessions, true);
        assert_eq!(st.sel_sessions, vec![true, false, true]);
        assert!(st.can_clean());
        // Anche se la casella risultasse spuntata, una sessione bloccata non viene mai scelta.
        let sel = vec![true, true, false];
        let ids: Vec<&str> = chosen_sessions(&ss, &sel).map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["1"]);
        assert_eq!(session_stats(&ss, &[true, true, true]), (2, 3, 7));
    }

    #[test]
    fn select_all_and_none_skip_what_cannot_go() {
        let rs = vec![report("a", ProjectStatus::Inactive, true, 1), report("b", ProjectStatus::InUse, false, 2)];
        let mut sel = vec![false, false];
        select_reports(&rs, &mut sel, true);
        assert_eq!(sel, vec![true, false]);
        select_reports(&rs, &mut sel, false);
        assert_eq!(sel, vec![false, false]);
        let ss = vec![session("1", Some("x"), true, 1), session("2", None, true, 1)];
        let mut ssel = vec![false, false];
        select_sessions(&ss, &mut ssel, true);
        assert_eq!(ssel, vec![false, true]);
    }

    /// Un progetto (più uno bloccato), un worktree d'agente e due sessioni.
    fn three_sections() -> DevScan {
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

    #[test]
    fn an_old_analysis_does_not_clean_projects_or_worktrees() {
        let at = 1_800_000_000_000;
        let mut st = state();
        st.set_scan(DevScan { scanned_at_ms: at, ..three_sections() });
        st.sel_worktrees = vec![true];
        let late = at + xiox_app::MAX_SCAN_AGE_MS + 1;
        for section in [Section::Projects, Section::Worktrees] {
            st.section = section;
            st.open_confirm_at(late);
            assert!(st.confirm.is_none(), "{section:?}: nessun dialogo");
            assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
        }
        assert_eq!(STALE_SCAN, "L'analisi è troppo vecchia: premi «Analizza» di nuovo.");

        // Entro la mezz'ora la conferma si apre e l'avviso sparisce.
        st.section = Section::Projects;
        st.open_confirm_at(at + xiox_app::MAX_SCAN_AGE_MS);
        assert!(st.confirm.take().is_some());
        assert!(st.notice.is_none());

        // Le sessioni si ricontrollano dal vivo prima di cancellarle: l'età non le ferma.
        st.section = Section::Sessions;
        st.open_confirm_at(late);
        assert!(st.confirm.take().is_some());
        assert!(st.notice.is_none());

        // Un'analisi di età ignota vale come vecchia; una nuova analisi toglie l'avviso.
        st.set_scan(DevScan { scanned_at_ms: 0, ..three_sections() });
        st.section = Section::Worktrees;
        st.sel_worktrees = vec![true];
        st.open_confirm_at(at);
        assert!(st.confirm.is_none() && st.notice.is_some());
        st.set_scan(three_sections());
        assert!(st.notice.is_none());
    }

    #[test]
    fn a_dialog_left_open_past_half_an_hour_does_not_clean() {
        let t = 1_800_000_000_000;
        let mut st = state();
        st.set_scan(DevScan { scanned_at_ms: t, ..three_sections() });
        st.sel_worktrees = vec![true];
        for section in [Section::Projects, Section::Worktrees] {
            st.set_section(section);
            st.open_confirm_at(t);
            let c = st.confirm.take().expect("aperta in tempo");
            assert!(st.accept_confirm(c, t + 31 * 60 * 1000).is_none(), "{section:?}");
            assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
            assert!(!st.busy(), "nessuna pulizia avviata");
            // Cambiare sezione toglie l'avviso.
            st.set_section(Section::Sessions);
            assert!(st.notice.is_none());
        }
        st.set_section(Section::Projects);
        st.open_confirm_at(t);
        let c = st.confirm.take().unwrap();
        assert!(st.accept_confirm(c, t + 29 * 60 * 1000).is_some());
        // Le sessioni si ricontrollano dal vivo: l'età non le ferma.
        st.set_section(Section::Sessions);
        st.open_confirm_at(t);
        let c = st.confirm.take().unwrap();
        assert!(st.accept_confirm(c, t + 31 * 60 * 1000).is_some());
        assert!(st.notice.is_none());
    }

    #[test]
    fn stats_count_only_the_selectable_rows_of_one_section() {
        let scan = three_sections();
        let (sp, sw, ss) = (ids(&["a", "b"]), [true], [true, true]);
        // La cartella del progetto bloccato non conta anche se risultasse spuntata.
        assert_eq!(section_stats(&scan, Section::Projects, &sp, &sw, &ss), (1, 2, 100));
        assert_eq!(section_stats(&scan, Section::Worktrees, &sp, &sw, &ss), (1, 1, 30));
        assert_eq!(section_stats(&scan, Section::Sessions, &sp, &sw, &ss), (2, 2, 4));
        assert_eq!(section_stats(&scan, Section::Projects, &ids(&["b"]), &sw, &ss), (0, 2, 0));
        assert_eq!(selection_text(Section::Projects, 1), "1 cartella");
        assert_eq!(selection_text(Section::Projects, 2), "2 cartelle");
        assert_eq!(selection_text(Section::Worktrees, 3), "3 worktree");
        assert_eq!(selection_text(Section::Sessions, 665), "665 sessioni");
    }

    #[test]
    fn the_action_list_holds_only_the_visible_section() {
        let scan = three_sections();
        let (sp, sw, ss) = (ids(&["a"]), [true], [true, true]);
        let p = section_action(&scan, Section::Projects, &sp, &sw, &ss);
        assert_eq!(p, Action { artifacts: vec![tid("a")], ..Default::default() });
        let w = section_action(&scan, Section::Worktrees, &sp, &sw, &ss);
        assert_eq!(w, Action { worktrees: vec!["w".into()], ..Default::default() });
        let s = section_action(&scan, Section::Sessions, &sp, &sw, &ss);
        assert_eq!(s, Action { sessions: vec![(Agent::Codex, "1".into()), (Agent::Codex, "2".into())], ..Default::default() });
        let sel = p.selection(&scan);
        assert_eq!((sel.artifacts, sel.worktrees), (vec![tid("a")], vec![]));
        // Nulla scelto nella sezione visibile: niente da fare, qualunque cosa ci sia nelle altre.
        assert_eq!(section_action(&scan, Section::Worktrees, &sp, &[false], &ss), Action::default());
        assert_eq!(section_action(&scan, Section::Sessions, &sp, &sw, &[false, false]), Action::default());
    }

    #[test]
    fn the_confirmation_names_the_section_and_lists_only_its_items() {
        let mut st = state();
        st.set_scan(three_sections());
        assert_eq!((&st.sel_artifacts, &st.sel_worktrees, &st.sel_sessions), (&ids(&["a"]), &vec![false], &vec![false, false]));
        st.sel_worktrees = vec![true];
        st.sel_sessions = vec![true, true];

        st.section = Section::Sessions;
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert!(c.plan.is_none(), "nessun progetto né worktree nella conferma delle sessioni");
        assert_eq!(c.sessions.len(), 2);
        assert!(c.rows.iter().all(|(label, _)| label.starts_with("Codex · ")), "{:?}", c.rows);
        assert_eq!(confirm_title(c.section), "Cancellare le sessioni archiviate selezionate?");
        assert_eq!(confirm_button(&c), "Cancella 2 sessioni");

        st.section = Section::Projects;
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert!(c.sessions.is_empty(), "nessuna sessione nella conferma dei progetti");
        let plan = c.plan.as_ref().unwrap();
        assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "a"), "{:?}", plan.items);
        assert_eq!(c.rows.len(), plan.items.len());
        assert_eq!(confirm_title(c.section), "Pulire le cartelle selezionate?");
        assert_eq!(confirm_button(&c), "Pulisci 1 cartella");

        st.section = Section::Worktrees;
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert!(c.sessions.is_empty());
        let plan = c.plan.as_ref().unwrap();
        assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "w"), "{:?}", plan.items);
        assert_eq!(confirm_title(c.section), "Rimuovere i worktree selezionati?");
    }

    #[test]
    fn confirmation_texts_follow_section_and_mode() {
        assert_eq!(
            confirm_text(Section::Sessions, RemoveMode::Recycle),
            "Una copia viene messa nel Cestino prima della cancellazione, quando il file esiste."
        );
        assert_eq!(confirm_text(Section::Sessions, RemoveMode::Permanent), "Le sessioni vengono cancellate definitivamente.");
        assert_eq!(
            confirm_text(Section::Worktrees, RemoveMode::Recycle),
            "I worktree vengono rimossi definitivamente con git, anche con «Nel Cestino»; le cartelle di build dentro un worktree vanno nel Cestino."
        );
        assert_eq!(confirm_text(Section::Worktrees, RemoveMode::Permanent), "Gli elementi selezionati verranno eliminati definitivamente.");
        assert_eq!(
            confirm_text(Section::Projects, RemoveMode::Recycle),
            "Gli elementi selezionati vanno nel Cestino."
        );
        assert_eq!(confirm_text(Section::Projects, RemoveMode::Permanent), "Gli elementi selezionati verranno eliminati definitivamente.");
        let one = Confirm {
            scan: Arc::new(DevScan::default()),
            section: Section::Sessions,
            sel: Selection::default(),
            plan: None,
            cautions: vec![],
            ack: false,
            shortfall: vec![],
            sessions: vec![(Agent::Pi, "x".into())],
            rows: vec![],
            summary: vec![],
            error: None,
            mode: RemoveMode::Recycle,
            selected: 1,
            others: None,
        };
        assert_eq!(confirm_button(&one), "Cancella 1 sessione");
        let three = Confirm { section: Section::Projects, sessions: vec![], selected: 3, ..one };
        assert_eq!(confirm_button(&three), "Pulisci 3 cartelle");
    }

    #[test]
    fn selected_text_and_clean_button_name_the_section() {
        assert_eq!(selected_text(Section::Projects, 1), "1 cartella selezionata");
        assert_eq!(selected_text(Section::Projects, 3), "3 cartelle selezionate");
        assert_eq!(selected_text(Section::Worktrees, 1), "1 worktree selezionato");
        assert_eq!(selected_text(Section::Worktrees, 2), "2 worktree selezionati");
        assert_eq!(selected_text(Section::Sessions, 1), "1 sessione selezionata");
        assert_eq!(selected_text(Section::Sessions, 5), "5 sessioni selezionate");

        assert_eq!(clean_button(Section::Projects, 0), "Pulisci");
        assert_eq!(clean_button(Section::Worktrees, 0), "Rimuovi");
        assert_eq!(clean_button(Section::Sessions, 0), "Cancella");
        assert_eq!(clean_button(Section::Projects, 1), "Pulisci 1 cartella");
        assert_eq!(clean_button(Section::Projects, 3), "Pulisci 3 cartelle");
        assert_eq!(clean_button(Section::Worktrees, 1), "Rimuovi 1 worktree");
        assert_eq!(clean_button(Section::Worktrees, 2), "Rimuovi 2 worktree");
        assert_eq!(clean_button(Section::Sessions, 1), "Cancella 1 sessione");
        assert_eq!(clean_button(Section::Sessions, 5), "Cancella 5 sessioni");
    }

    #[test]
    fn the_confirmation_says_what_it_leaves_out() {
        let mut st = state();
        st.set_scan(three_sections());
        st.sel_worktrees = vec![true];
        assert_eq!((st.sel_artifacts.len(), st.sel_sessions.clone()), (1, vec![false, false]));
        st.sel_sessions = vec![true, true];

        // Progetti: le righe scelte in Worktree e Sessioni restano fuori dal piano e sono nominate.
        st.set_section(Section::Projects);
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 worktree selezionato e 2 sessioni selezionate."));
        assert_eq!(c.selected, 1);
        assert_eq!(confirm_button(&c), "Pulisci 1 cartella");
        let plan = c.plan.as_ref().unwrap();
        assert!(!plan.items.is_empty() && plan.items.iter().all(|i| i.project_id == "a"), "{:?}", plan.items);
        assert!(c.sessions.is_empty());

        // Sessioni: Progetti e Worktree, nell'ordine.
        st.set_section(Section::Sessions);
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 cartella selezionata e 1 worktree selezionato."));
        assert_eq!(confirm_button(&c), "Cancella 2 sessioni");

        // Una sola altra sezione con qualcosa di scelto: una sola voce.
        st.set_section(Section::Worktrees);
        st.sel_sessions = vec![false, false];
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert_eq!(c.others.as_deref(), Some("Non tocca le altre sezioni: 1 cartella selezionata."));
        assert_eq!(confirm_button(&c), "Rimuovi 1 worktree");

        // Le altre sezioni vuote: nessuna nota.
        st.sel_artifacts.clear();
        st.open_confirm();
        let c = st.confirm.take().unwrap();
        assert_eq!(c.others, None);
    }

    #[test]
    fn labels() {
        assert_eq!(agent_label(Agent::Opencode), "OpenCode");
        assert_eq!(agent_label(Agent::Claude), "Claude Code");
        let s = session("abc", None, false, 0);
        assert_eq!(session_title(&s), "titolo abc");
        assert_eq!(session_label(&s), "Codex · titolo abc");
        assert_eq!(session_size(&s), "solo riga nel database");
        assert_eq!(session_size(&session("x", None, true, 2048)), "2 KB");
        let mut untitled = s.clone();
        untitled.title = Some("   ".into());
        assert_eq!(session_title(&untitled), "(senza titolo)");
        assert_eq!(section_label(Section::Sessions, Some(664)), "Sessioni archiviate (664)");
        assert_eq!(section_label(Section::Worktrees, None), "Worktree");
        assert_eq!(plural(1, "cartella", "cartelle"), "1 cartella");
        assert_eq!(plural(0, "cartella", "cartelle"), "0 cartelle");
        assert_eq!(fmt_elapsed(58_400), "Analisi in 58 s");
        assert_eq!(fmt_elapsed(125_000), "Analisi in 2 min 5 s");
    }

    #[test]
    fn plan_labels_name_the_project_and_the_artifact() {
        let mut wt = report("wt", ProjectStatus::Inactive, true, 9);
        wt.is_agent_worktree = true;
        let scan = DevScan { projects: vec![report("app", ProjectStatus::Inactive, true, 1)], worktrees: vec![wt], ..Default::default() };
        let item = |path: &str, id: &str, action| PlanItem {
            path: PathBuf::from(path),
            kind: ArtifactKind::RustTarget,
            action,
            expected_bytes: 0,
            project_id: id.into(),
            reason: String::new(),
        };
        assert_eq!(plan_label(&scan, &item(r"D:\p\app\target", "app", PlanAction::RemoveDir)), "app · Build Rust (target)");
        let remove_wt = PlanAction::GitWorktreeRemove { repo: PathBuf::from(r"D:\repo") };
        assert_eq!(plan_label(&scan, &item(r"D:\p\wt", "wt", remove_wt)), "wt · worktree intero");
        assert_eq!(plan_label(&scan, &item(r"D:\x\node_modules", "?", PlanAction::RemoveDir)), "node_modules · node_modules");
    }

    #[test]
    fn dates_are_day_month_year() {
        // Mezzogiorno UTC: lo stesso giorno in ogni fuso tra -11 e +11.
        assert_eq!(fmt_date(1_762_776_000_000), "10/11/2025");
        assert_eq!(fmt_date(1_709_208_000_000), "29/02/2024");
    }

    #[test]
    fn a_failed_analysis_is_not_an_empty_one() {
        let ok = DevScan { warnings: vec!["Sessioni di Pi illeggibili".into()], ..Default::default() };
        assert!(!analysis_failed(&ok));
        assert_eq!(empty_text(Section::Projects, false), "Nessun progetto trovato.");
        let failed = DevScan { warnings: vec!["altro".into(), ANALYSIS_FAILED.into()], ..Default::default() };
        assert!(analysis_failed(&failed));
        assert_eq!(ordered_warnings(&failed.warnings), vec![ANALYSIS_FAILED, "altro"]);
        for s in [Section::Projects, Section::Worktrees] {
            assert!(empty_text(s, true).contains("non riuscita"), "{s:?}");
        }
        assert_eq!(empty_text(Section::Sessions, true), "Nessuna sessione archiviata.");
    }

    #[test]
    fn column_widths_give_the_rest_to_the_flexible_column() {
        assert_eq!(col_widths(500.0, &[20.0, 0.0, 100.0], 8.0), vec![20.0, 364.0, 100.0]);
        assert_eq!(col_widths(100.0, &[20.0, 0.0, 100.0], 8.0), vec![20.0, 80.0, 100.0]);
    }

    #[test]
    fn sections_from_the_command_line() {
        assert_eq!(Section::parse("projects"), Some(Section::Projects));
        assert_eq!(Section::parse("worktree"), Some(Section::Worktrees));
        assert_eq!(Section::parse("sessions"), Some(Section::Sessions));
        assert_eq!(Section::parse("boh"), None);
    }

    fn state() -> DevState {
        DevState::with_settings(xiox_app::default_settings(), Section::Projects, false)
    }

    #[test]
    fn scan_result_preselects_and_release_frees_it() {
        let mut st = state();
        let (tx, rx) = channel();
        st.job = Some(rx);
        tx.send(DevMsg::Step("Lettura delle sessioni…".into(), 0.05)).unwrap();
        st.poll();
        assert!(st.busy());
        assert_eq!(st.text, "Lettura delle sessioni…");

        // Mentre il lavoro è in corso release() non tocca nulla.
        st.release();
        assert!(st.busy());

        let scan = DevScan {
            projects: vec![report("a", ProjectStatus::Inactive, true, 10), report("b", ProjectStatus::Recent, false, 5)],
            archived: vec![session("1", None, false, 0), session("2", Some("sessione in uso"), true, 3)],
            ..Default::default()
        };
        tx.send(DevMsg::Scanned(scan)).unwrap();
        st.poll();
        assert!(!st.busy());
        assert_eq!(st.view, View::Scanned);
        assert_eq!(st.sel_artifacts, ids(&["a"]));
        assert_eq!(st.sel_sessions, vec![false, false], "le sessioni partono non spuntate");
        assert_eq!(st.selection(), (1, 2, 10));

        st.release();
        assert!(st.scan.is_none());
        assert_eq!(st.view, View::Empty);
        assert!(st.sel_artifacts.is_empty() && st.sel_sessions.is_empty());
    }

    #[test]
    fn a_dead_job_is_reported_not_left_spinning() {
        let mut st = state();
        let (tx, rx) = channel::<DevMsg>();
        st.job = Some(rx);
        drop(tx);
        st.poll();
        assert!(!st.busy());
        assert_eq!(st.error.as_deref(), Some("Il lavoro si è interrotto per un errore interno."));
        assert_eq!(st.view, View::Empty);
    }

    #[test]
    fn a_dead_cleanup_discards_the_scan_and_disables_the_button() {
        let mut st = state();
        st.set_scan(three_sections());
        assert!(st.can_clean());
        // Come dopo «Continua»: vista dell'esito, lavoro in corso, poi il thread muore senza esito.
        st.view = View::Cleaned;
        let (tx, rx) = channel::<DevMsg>();
        st.job = Some(rx);
        assert!(!st.can_clean());
        drop(tx);
        st.poll();
        assert!(!st.busy());
        assert!(st.scan.is_none());
        assert!(st.sel_artifacts.is_empty() && st.sel_worktrees.is_empty() && st.sel_sessions.is_empty());
        assert!(!st.can_clean());
        assert_eq!(st.error.as_deref(), Some(CLEANUP_DIED));
        assert!(CLEANUP_DIED.contains("potrebbero essere già stati rimossi"));
        assert_eq!(st.view, View::Empty);
    }

    #[test]
    fn a_panic_keeps_the_outcomes_collected_so_far() {
        let mut out = DevCleaned::new();
        guarded(&mut out, |o| {
            add_session_outcome(&mut o.report, "Codex · a", &ItemOutcome::Removed { bytes_freed: 7 }, RemoveMode::Permanent);
        });
        assert!(!out.interrupted);
        guarded(&mut out, |o| {
            add_session_outcome(&mut o.report, "Codex · b", &ItemOutcome::Skipped { reason: "in uso".into() }, RemoveMode::Permanent);
            panic!("prova");
        });
        assert!(out.interrupted);
        assert_eq!(out.report.skipped.len(), 1);
        assert_eq!(out.report.freed_bytes, 7);
        assert_eq!(cleaned_title(Some(&out), false), "Pulizia interrotta");
    }

    #[test]
    fn the_confirmation_uses_the_chosen_mode_and_keeps_its_scan() {
        let mut st = state();
        st.set_scan(three_sections());
        for mode in [RemoveMode::Permanent, RemoveMode::Recycle] {
            st.settings.remove_mode = mode;
            st.open_confirm();
            let c = st.confirm.take().unwrap();
            assert_eq!(c.mode, mode);
            assert!(Arc::ptr_eq(&c.scan, st.scan.as_ref().unwrap()));
        }
        // Una nuova analisi arrivata dopo l'apertura non cambia la conferma già aperta.
        st.open_confirm();
        let old = st.scan.clone().unwrap();
        st.set_scan(three_sections());
        let c = st.confirm.as_ref().unwrap();
        assert!(Arc::ptr_eq(&c.scan, &old) && !Arc::ptr_eq(&c.scan, st.scan.as_ref().unwrap()));
    }

    #[test]
    fn cleaning_result_drops_the_scan() {
        let mut st = state();
        st.set_scan(DevScan { projects: vec![report("a", ProjectStatus::Inactive, true, 10)], ..Default::default() });
        let (tx, rx) = channel();
        st.job = Some(rx);
        st.tick = Some(Tick { done_bytes: 0, total_bytes: 10, done_items: 0, total_items: 1 });
        st.cancel = Some(Cancel::default());
        tx.send(DevMsg::Tick(Tick { done_bytes: 5, total_bytes: 10, done_items: 0, total_items: 1 })).unwrap();
        st.poll();
        assert_eq!(st.tick.map(|t| t.done_bytes), Some(5));
        let mut c = DevCleaned::new();
        c.report.freed_bytes = 10;
        c.report.by_category = vec![("Build Rust (target)".into(), 10)];
        tx.send(DevMsg::Cleaned(c)).unwrap();
        st.poll();
        assert!(!st.busy());
        assert!(st.tick.is_none() && st.cancel.is_none());
        assert!(st.scan.is_none());
        assert_eq!(st.view, View::Cleaned);
        assert_eq!(st.cleaned.as_ref().map(|c| c.report.freed_bytes), Some(10));
        assert_eq!(st.selection(), (0, 0, 0));
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
            "Dipendenze Node (node_modules)",
            xiox_model::Class::Dependency,
            Level::Optional,
            "Si riscarica con npm install (o pnpm, yarn, bun install)",
            50,
        ));
        r.artifacts.push(mk(
            "dist",
            "Build JavaScript (dist)",
            xiox_model::Class::Build,
            Level::Caution,
            "Potrebbe contenere sorgenti: git non la ignora",
            20,
        ));
        r.artifacts[1].kind = ArtifactKind::NodeModules;
        r.artifacts[2].kind = ArtifactKind::Catalog;
        r.reclaimable_bytes = 170;
        r
    }

    #[test]
    fn the_project_checkbox_has_three_states() {
        let r = levels_project();
        let mut sel = preselect_artifacts(std::slice::from_ref(&r));
        assert_eq!(sel, ids(&["web"]), "solo target è preselezionato");
        assert_eq!(tri_state(&r, &sel), Tri::Some);
        // «alcune» → tutte
        toggle_project(&r, &mut sel);
        assert_eq!(tri_state(&r, &sel), Tri::All);
        assert_eq!(sel.len(), 3);
        // «tutte» → nessuna
        toggle_project(&r, &mut sel);
        assert_eq!(tri_state(&r, &sel), Tri::None);
        assert!(sel.is_empty());
        // «nessuna» → tutte
        toggle_project(&r, &mut sel);
        assert_eq!(tri_state(&r, &sel), Tri::All);

        // Un progetto in uso non cambia e non conta, ma le sue cartelle restano elencate.
        let mut busy = levels_project();
        busy.status = ProjectStatus::InUse;
        let mut none = HashSet::new();
        toggle_project(&busy, &mut none);
        assert!(none.is_empty());
        assert!(preselect_artifacts(std::slice::from_ref(&busy)).is_empty());
        assert_eq!(artifact_stats(std::slice::from_ref(&busy), &sel), (0, 3, 0));
    }

    #[test]
    fn the_total_follows_each_folder() {
        let rs = vec![levels_project()];
        let mut sel = preselect_artifacts(&rs);
        assert_eq!(artifact_stats(&rs, &sel), (1, 3, 100));
        set_artifact(&mut sel, &rs[0].artifacts[1], true);
        assert_eq!(artifact_stats(&rs, &sel), (2, 3, 150));
        set_artifact(&mut sel, &rs[0].artifacts[0], false);
        assert_eq!(artifact_stats(&rs, &sel), (1, 3, 50));
        select_all_artifacts(&rs, &mut sel, true);
        assert_eq!(artifact_stats(&rs, &sel), (3, 3, 170));
        select_all_artifacts(&rs, &mut sel, false);
        assert_eq!(artifact_stats(&rs, &sel), (0, 3, 0));
    }

    #[test]
    fn levels_have_their_words_and_colors() {
        assert_eq!(level_text(Level::Recommended), ("Consigliato", GREEN));
        assert_eq!(level_text(Level::Optional), ("Facoltativo", ACCENT));
        assert_eq!(level_text(Level::Caution), ("Attenzione", RED));
    }

    fn levels_state() -> DevState {
        let mut st = state();
        st.set_scan(DevScan { projects: vec![levels_project()], scanned_at_ms: xiox_app::now_ms(), ..Default::default() });
        st
    }

    #[test]
    fn a_caution_folder_needs_the_warnings_read() {
        let mut st = levels_state();
        // Solo target: niente avvisi, «Pulisci» subito.
        st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
        let c = st.confirm.take().unwrap();
        assert!(c.cautions.is_empty());
        assert!(confirm_ready(&c));
        assert_eq!(c.plan.as_ref().unwrap().items.len(), 1, "solo la cartella scelta");

        // Con dist: elencata con il suo avviso, «Pulisci» spento finché non si legge.
        st.sel_artifacts.insert(tid("web").replace("target", "dist"));
        st.open_confirm_with(xiox_app::now_ms(), |_| vec![]);
        let mut c = st.confirm.take().unwrap();
        assert_eq!(c.plan.as_ref().unwrap().items.len(), 2);
        assert_eq!(c.cautions, vec![("web · Build JavaScript (dist)".to_string(), "Potrebbe contenere sorgenti: git non la ignora".to_string())]);
        assert!(!confirm_ready(&c));
        c.ack = true;
        assert!(confirm_ready(&c));
        assert_eq!(caution_title(1), "1 cartella al livello Attenzione:");
        assert_eq!(caution_title(2), "2 cartelle al livello Attenzione:");
    }

    #[test]
    fn a_small_recycle_bin_offers_permanent_deletion() {
        let mut st = levels_state();
        st.settings.remove_mode = RemoveMode::Recycle;
        st.open_confirm_with(xiox_app::now_ms(), |p| vec![('D', p.expected_bytes, 0)]);
        let mut c = st.confirm.take().unwrap();
        assert_eq!(c.shortfall, vec![('D', 100, 0)]);
        assert_eq!(
            shortfall_text(&c.shortfall),
            "Il Cestino di D: ha 0 byte liberi, la pulizia ne sposta 100 byte. Eliminare definitivamente questa volta?"
        );
        // Il pulsante è «Definitiva»: il testo non parla del Cestino.
        assert_eq!(c.mode, RemoveMode::Recycle);
        assert_eq!(shown_mode(&c), RemoveMode::Permanent);
        assert_eq!(confirm_text(c.section, shown_mode(&c)), "Gli elementi selezionati verranno eliminati definitivamente.");
        make_permanent(&mut c).unwrap();
        assert_eq!(c.mode, RemoveMode::Permanent);
        assert_eq!(c.plan.as_ref().unwrap().mode, RemoveMode::Permanent);
        assert_eq!(c.plan.as_ref().unwrap().items.len(), 1, "lo stesso piano");
        assert!(c.shortfall.is_empty());
        let two = [('C', 10 << 30, 2 << 30), ('E', 3 << 20, 0)];
        assert_eq!(
            shortfall_text(&two),
            "Il Cestino di C: ha 2,00 GB liberi, la pulizia ne sposta 10,00 GB. Il Cestino di E: ha 0 byte liberi, la pulizia ne sposta 3,0 MB. Eliminare definitivamente questa volta?"
        );
    }

    #[test]
    fn the_report_adds_up_folders_and_sessions() {
        let mut out = DevCleaned::new();
        assert!(out.is_empty());
        assert_eq!(cleaned_title(Some(&out), false), "Pulizia completata");
        merge_report(
            &mut out.report,
            CleanReport {
                freed_bytes: 100,
                files: 4,
                dirs: 2,
                elapsed_ms: 1500,
                by_category: vec![("Build Rust (target)".into(), 100)],
                skipped: vec![(PathBuf::from(r"D:\p\web\dist"), "Annullato".into())],
                cancelled: true,
                ..Default::default()
            },
        );
        out.labels.insert(path_key(Path::new(r"D:\p\web\dist")), "web · Build JavaScript (dist)".into());
        add_session_outcome(&mut out.report, "Codex · x", &ItemOutcome::Removed { bytes_freed: 300 }, RemoveMode::Recycle);
        let f = ItemOutcome::Failed { error: "bloccato".into(), locked_by: vec!["code.exe".into()] };
        add_session_outcome(&mut out.report, "Codex · y", &f, RemoveMode::Recycle);
        let r = &out.report;
        assert_eq!((r.freed_bytes, r.recycled_bytes, r.files, r.dirs), (100, 300, 5, 2));
        assert_eq!(r.by_category, vec![(SESSIONS_CATEGORY.to_string(), 300), ("Build Rust (target)".to_string(), 100)]);
        assert_eq!(r.failed, vec![(PathBuf::from("Codex · y"), "bloccato (in uso da: code.exe)".to_string())]);
        assert_eq!(out.label(Path::new(r"D:\p\web\dist")), "web · Build JavaScript (dist)");
        assert_eq!(cleaned_title(Some(&out), false), "Pulizia annullata");
        assert_eq!(cleaned_title(Some(&out), true), "Pulizia in corso…");
        assert_eq!(recycled_text(300), "300 byte nel Cestino: si liberano svuotandolo");
        let text = r.to_text();
        assert!(text.contains("Spazio liberato") && text.contains("Saltati (1)") && text.contains("Non riusciti (1)"), "{text}");

        let mut refused = DevCleaned::new();
        refused.errors.push("Piano rifiutato".into());
        assert_eq!(cleaned_title(Some(&refused), false), "Pulizia non eseguita");
    }

    #[test]
    fn progress_text_and_bar() {
        let t = Tick { done_bytes: 512, total_bytes: 2048, done_items: 1, total_items: 4 };
        assert_eq!(tick_text(&t), "512 byte di 2 KB · 1 di 4 voci");
        assert_eq!(tick_fraction(&t), 0.25);
        let unknown = Tick { done_bytes: 0, total_bytes: 0, done_items: 1, total_items: 2 };
        assert_eq!(tick_fraction(&unknown), 0.5);
        assert_eq!(tick_text(&Tick { total_items: 1, ..unknown }), "0 byte di 0 byte · 1 di 1 voce");
        assert_eq!(fmt_duration(850), "850 ms");
        assert_eq!(fmt_duration(12_300), "12 s");
        assert_eq!(fmt_duration(125_000), "2 min 05 s");
    }

    #[test]
    fn cancel_stops_the_running_cleanup() {
        let mut st = state();
        assert!(!st.cancelling());
        st.cancel = Some(Cancel::default());
        let shared = st.cancel.clone().unwrap();
        st.cancel_clean();
        assert!(st.cancelling() && shared.is_cancelled(), "lo stesso Cancel dei worker");
    }

    /// Un report con la radice indicata.
    fn report_at(id: &str, root: &str) -> ProjectReport {
        let mut r = report(id, ProjectStatus::Inactive, true, 1);
        r.root = PathBuf::from(root);
        r
    }

    /// Un'analisi con roba dentro `D:\t\albero` e fuori (anche un fratello `albero-x`) e due sessioni.
    fn mixed_scan() -> DevScan {
        let mut wt_in = report_at("wi", r"D:\t\albero\wt");
        wt_in.is_agent_worktree = true;
        let mut wt_out = report_at("wo", r"D:\Projects\vero-wt");
        wt_out.is_agent_worktree = true;
        DevScan {
            projects: vec![report_at("pi", r"D:\t\albero\p1"), report_at("po", r"D:\Projects\vero"), report_at("px", r"D:\t\albero-x\p2")],
            worktrees: vec![wt_in, wt_out],
            other_worktrees: vec![report_at("oi", r"D:\t\albero\o1"), report_at("oo", r"D:\altrove\o2")],
            archived: vec![session("1", None, true, 4), session("2", None, true, 5)],
            scanned_at_ms: xiox_app::now_ms(),
            ..Default::default()
        }
    }

    fn report_ids(rs: &[ProjectReport]) -> Vec<&str> {
        rs.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn keep_only_under_drops_everything_outside_the_root() {
        let mut scan = mixed_scan();
        keep_only_under(&mut scan, Path::new(r"D:\t\albero"));
        assert_eq!(report_ids(&scan.projects), vec!["pi"], "il fratello albero-x non sta sotto albero");
        assert_eq!(report_ids(&scan.worktrees), vec!["wi"]);
        assert_eq!(report_ids(&scan.other_worktrees), vec!["oi"]);
        assert!(scan.archived.is_empty(), "le sessioni non hanno una cartella nell'albero finto");

        // La radice stessa resta; maiuscole e barre non cambiano il giudizio.
        let mut scan = DevScan { projects: vec![report_at("r", r"D:\t\albero"), report_at("c", "d:/T/ALBERO/sub")], ..Default::default() };
        keep_only_under(&mut scan, Path::new(r"D:\t\albero"));
        assert_eq!(report_ids(&scan.projects), vec!["r", "c"]);
    }

    #[test]
    fn only_root_never_keeps_real_sessions_or_worktrees() {
        let mut fake = DevState::only_root(PathBuf::from(r"D:\t\albero"), Section::Sessions, false);
        fake.set_scan(mixed_scan());
        let scan = fake.scan.clone().unwrap();
        assert!(scan.archived.is_empty() && fake.sel_sessions.is_empty());
        assert_eq!(report_ids(&scan.worktrees), vec!["wi"], "solo il worktree dentro l'albero finto");
        assert_eq!(fake.sel_worktrees.len(), 1);
        assert_eq!(report_ids(&scan.projects), vec!["pi"]);
        assert_eq!(report_ids(&scan.other_worktrees), vec!["oi"]);

        // Uno stato normale tiene tutto.
        let mut normal = state();
        normal.set_scan(mixed_scan());
        let scan = normal.scan.clone().unwrap();
        assert_eq!(scan.archived.len(), 2);
        assert_eq!(scan.worktrees.len(), 2);
        assert_eq!(scan.projects.len(), 3);
        assert_eq!(scan.other_worktrees.len(), 2);
    }

    #[test]
    fn only_root_keeps_everything_in_memory() {
        let dir = PathBuf::from(r"Z:\albero\finto");
        let s = only_root_settings(xiox_app::default_settings(), dir.clone());
        assert!(!s.session_projects && s.scan_roots && !s.all_drives);
        assert_eq!(s.roots, vec![dir.clone()]);
        let mut st = DevState::only_root(dir, Section::Projects, false);
        assert!(!st.persist);
        st.settings.inactive_days = 3;
        st.save();
        assert!(st.settings_err.is_none());
    }

    /// `--dev-demo` cancella per davvero: solo con `--only-root` su un albero finto.
    #[cfg(debug_assertions)]
    #[test]
    fn demo_runs_only_on_a_fake_tree() {
        // Fuori dalla cartella temporanea: serve il file segnale.
        let base = std::env::current_dir().unwrap().join("target");
        let real = tempfile::Builder::new().prefix("demo-vero-").tempdir_in(&base).unwrap();
        let mut st = DevState::only_root(real.path().to_path_buf(), Section::Projects, false);
        let e = st.set_demo(Demo::Clean).unwrap_err();
        assert!(e.contains(DEMO_MARKER), "{e}");
        assert!(st.demo.is_none());
        std::fs::write(real.path().join(DEMO_MARKER), "").unwrap();
        st.set_demo(Demo::Clean).unwrap();
        assert_eq!(st.demo, Some(Demo::Clean));

        // Dentro la cartella temporanea basta --only-root, ma non la cartella temporanea stessa.
        let tmp = tempfile::tempdir().unwrap();
        let mut st = DevState::only_root(tmp.path().to_path_buf(), Section::Projects, false);
        st.set_demo(Demo::Confirm).unwrap();
        let mut st = DevState::only_root(std::env::temp_dir(), Section::Projects, false);
        assert!(st.set_demo(Demo::Confirm).is_err());
        let mut st = DevState::only_root(real.path().join("manca"), Section::Projects, false);
        assert!(st.set_demo(Demo::Confirm).is_err(), "cartella inesistente");

        // Senza --only-root (le radici vere dell'utente) mai, nemmeno con il segnale.
        let mut normal = state();
        normal.settings.roots = vec![real.path().to_path_buf()];
        assert!(normal.set_demo(Demo::Clean).is_err());
        assert!(normal.demo.is_none());
        // Le radici cambiate dopo l'avvio: il passo della dimostrazione si ferma.
        assert!(demo_allowed(false, &[real.path().to_path_buf(), tmp.path().to_path_buf()]).is_err());
    }
}
