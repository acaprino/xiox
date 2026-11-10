//! Voce «Sviluppatori»: Progetti, Worktree degli agenti e Sessioni archiviate.
//!
//! Analisi e pulizia girano in un thread e parlano alla UI con [`DevMsg`] su `mpsc`, come
//! `start_clean`/`poll` in `main.rs`: la UI non si blocca mai. L'ultima analisi ([`DevScan`])
//! è anche l'elenco di ciò che si può cancellare: `dc_app` rifiuta tutto il resto.
//! I dati dell'analisi si liberano con [`DevState::release`] (uscendo dalla voce) o rianalizzando.

use crate::sys;
use crate::theme::*;
use dc_app::{DevScan, ANALYSIS_FAILED};
use dc_model::{path_key, Agent, CleanPlan, ItemOutcome, PlanAction, PlanItem, ProjectReport, ProjectStatus, RemoveMode, Settings};
use dc_mukade::ArchivedSession;
use dc_sessions::{SystemCli, SystemTrash};
use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Stroke, Ui, Vec2};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

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
    Scanned(DevScan),
    Cleaned(DevCleaned),
}

/// Esito di una pulizia: una riga per elemento (cartella, worktree o sessione).
pub struct DevCleaned {
    pub rows: Vec<(String, ItemOutcome)>,
    pub freed: u64,
    /// Richieste rifiutate per intero (nulla è stato toccato).
    pub errors: Vec<String>,
    pub mode: RemoveMode,
    /// Un errore interno ha fermato la pulizia a metà: `rows` ha solo gli esiti raccolti prima.
    pub interrupted: bool,
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
    plan: Option<CleanPlan>,
    sessions: Vec<(Agent, String)>,
    /// (elemento, dimensione) per l'elenco nel dialogo.
    rows: Vec<(String, String)>,
    summary: Vec<String>,
    error: Option<String>,
    mode: RemoveMode,
}

pub struct DevState {
    settings: Settings,
    settings_err: Option<String>,
    section: Section,
    view: View,
    scan: Option<Arc<DevScan>>,
    sel_projects: Vec<bool>,
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
}

/// Progetti e worktree non si puliscono da un'analisi più vecchia di `dc_app::MAX_SCAN_AGE_MS`.
const STALE_SCAN: &str = dc_app::STALE_ANALYSIS;
/// Il thread della pulizia è morto senza esito: qualcosa può essere già sparito.
const CLEANUP_DIED: &str = "Un errore interno ha fermato la pulizia: alcuni elementi potrebbero essere già stati rimossi. Premi «Analizza» per vedere cosa resta.";

impl DevState {
    /// `analyze`: avvia l'analisi la prima volta che la voce viene mostrata (`--analyze`).
    pub fn new(section: Section, analyze: bool) -> Self {
        Self::with_settings(dc_app::load_settings(), section, analyze)
    }

    fn with_settings(settings: Settings, section: Section, analyze: bool) -> Self {
        DevState {
            settings,
            settings_err: None,
            section,
            view: View::Empty,
            scan: None,
            sel_projects: Vec::new(),
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
        }
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
        self.sel_projects = Vec::new();
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
        }
    }

    fn set_scan(&mut self, scan: DevScan) {
        self.notice = None;
        self.sel_projects = preselect_reports(&scan.projects);
        self.sel_worktrees = preselect_reports(&scan.worktrees);
        self.sel_sessions = preselect_sessions(&scan.archived);
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
            let scan = dc_app::run_scan(&settings, admin, &step);
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
            Some(scan) => section_stats(scan, self.section, &self.sel_projects, &self.sel_worktrees, &self.sel_sessions),
            None => (0, 0, 0),
        }
    }

    fn open_confirm(&mut self) {
        self.open_confirm_at(dc_app::now_ms());
    }

    /// Cambia sezione; l'avviso legato al pulsante della sezione precedente sparisce.
    fn set_section(&mut self, s: Section) {
        self.section = s;
        self.notice = None;
    }

    /// «Continua» nel dialogo: l'età dell'analisi si ricontrolla a `now_ms` (il dialogo può
    /// essere rimasto aperto a lungo). Troppo vecchia per progetti e worktree: nessuna pulizia,
    /// avviso nella striscia gialla. `Some` = la conferma da passare a `start_clean`.
    fn accept_confirm(&mut self, c: Confirm, now_ms: i64) -> Option<Confirm> {
        if c.section != Section::Sessions && c.scan.is_stale(now_ms) {
            self.notice = Some(STALE_SCAN.into());
            return None;
        }
        Some(c)
    }

    /// Costruisce il piano dalla selezione della sezione visibile (lo stesso che verrà
    /// eseguito) e apre la conferma. Le righe scelte nelle altre sezioni non ci entrano.
    /// Progetti e worktree: un'analisi vecchia (`DevScan::is_stale` a `now_ms`) non apre nulla
    /// e lo dice nella striscia gialla; le sessioni si ricontrollano comunque prima di cancellarle.
    fn open_confirm_at(&mut self, now_ms: i64) {
        let Some(scan) = self.scan.clone() else { return };
        let mode = self.settings.remove_mode;
        let section = self.section;
        self.notice = None;
        if section != Section::Sessions && scan.is_stale(now_ms) {
            self.notice = Some(STALE_SCAN.into());
            return;
        }
        let action = section_action(&scan, section, &self.sel_projects, &self.sel_worktrees, &self.sel_sessions);
        let ids = action.ids;
        let sessions: Vec<&ArchivedSession> = scan
            .archived
            .iter()
            .filter(|s| action.sessions.iter().any(|(a, id)| *a == s.agent && *id == s.id))
            .collect();
        let mut rows = Vec::new();
        let mut summary = Vec::new();
        let mut plan = None;
        let mut error = None;
        if !ids.is_empty() {
            match dc_app::build_plan(&scan, &ids, mode) {
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
        self.confirm = Some(Confirm { scan, section, plan, sessions, rows, summary, error, mode });
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
        self.cleaned = None;
        self.error = None;
        self.view = View::Cleaned;
        self.text = "Pulizia in corso…".into();
        self.progress = 0.0;
        let (tx, rx) = channel();
        self.job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let step = |t: &str, p: f32| {
                let _ = tx.send(DevMsg::Step(t.to_string(), p));
                ctx.request_repaint();
            };
            let mut out = DevCleaned { rows: Vec::new(), freed: 0, errors: Vec::new(), mode, interrupted: false };
            guarded(&mut out, |out| {
                if let Some(plan) = plan.filter(|p| !p.items.is_empty()) {
                    step("Eliminazione di cartelle e worktree…", 0.1);
                    match dc_app::execute_plan(&scan, &plan) {
                        Ok(res) => {
                            out.freed = out.freed.saturating_add(res.bytes_freed);
                            for it in res.items {
                                let label = plan_labels
                                    .get(&path_key(&it.path))
                                    .cloned()
                                    .unwrap_or_else(|| it.path.display().to_string());
                                out.rows.push((label, it.outcome));
                            }
                        }
                        Err(e) => out.errors.push(e),
                    }
                }
                if !sessions.is_empty() {
                    let label = |agent: Agent, id: &str| {
                        session_labels
                            .get(&(agent, id.to_string()))
                            .cloned()
                            .unwrap_or_else(|| format!("{} · {id}", agent_label(agent)))
                    };
                    // L'analisi può avere ore: si cancella solo ciò che è ANCORA archiviato e libero.
                    step("Controllo delle sessioni archiviate…", 0.5);
                    match dc_app::revalidate_archived(&scan, &sessions, &dc_app::fresh_archived) {
                        Ok(checked) => {
                            for o in checked.skipped {
                                out.rows.push((label(o.agent, &o.id), o.outcome));
                            }
                            if !checked.keep.is_empty() {
                                step("Cancellazione delle sessioni archiviate…", 0.6);
                                let cli = SystemCli::locate();
                                match dc_app::delete_archived(&scan, &checked.keep, mode, &cli, &SystemTrash) {
                                    Ok(done) => {
                                        for o in done {
                                            if let ItemOutcome::Removed { bytes_freed } = &o.outcome {
                                                out.freed = out.freed.saturating_add(*bytes_freed);
                                            }
                                            out.rows.push((label(o.agent, &o.id), o.outcome));
                                        }
                                    }
                                    Err(e) => out.errors.push(e),
                                }
                            }
                        }
                        Err(e) => out.errors.push(e),
                    }
                }
            });
            drop(scan);
            let _ = tx.send(DevMsg::Cleaned(out));
            ctx.request_repaint();
        });
    }

    fn save(&mut self) {
        self.settings_err = dc_app::save_settings(&self.settings).err();
    }

    // ---------- drawing ----------

    fn options(&mut self, ui: &mut Ui, busy: bool) {
        group(ui, "Sezione", 6.0);
        let counts = self.scan.as_ref().map(|s| [s.projects.len(), s.worktrees.len(), s.archived.len()]);
        for (k, s) in [Section::Projects, Section::Worktrees, Section::Sessions].into_iter().enumerate() {
            let text = section_label(s, counts.map(|c| c[k]));
            let on = self.section == s;
            let btn = egui::Button::new(RichText::new(text).color(if on { ACCENT } else { TEXT }).strong())
                .fill(if on { SEL_BG } else { CARD })
                .stroke(Stroke::NONE)
                .min_size(Vec2::new(ui.available_width(), 28.0));
            if ui.add(btn).clicked() {
                self.set_section(s);
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
            ui.label(RichText::new(&self.text).size(16.0).color(TEXT));
            ui.add_space(6.0);
            ui.add(egui::ProgressBar::new(self.progress).fill(ACCENT).show_percentage());
            ui.add_space(10.0);
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
                        Section::Projects => select_reports(&scan.projects, &mut self.sel_projects, all),
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
        if self.section == Section::Worktrees {
            ui.label(
                RichText::new("I worktree si rimuovono con git e sempre in modo definitivo; quelli con modifiche non salvate restano.")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        ui.add_space(8.0);
        match self.section {
            Section::Projects => projects_table(ui, &scan.projects, &mut self.sel_projects, analysis_failed(&scan)),
            Section::Worktrees => {
                worktrees_table(ui, &scan.worktrees, &scan.other_worktrees, &mut self.sel_worktrees, analysis_failed(&scan))
            }
            Section::Sessions => sessions_table(ui, &scan.archived, &mut self.sel_sessions),
        }
    }

    fn cleaned_view(&self, ui: &mut Ui, busy: bool) {
        let title = match &self.cleaned {
            _ if busy => "Pulizia in corso…",
            Some(c) if c.interrupted => "Pulizia interrotta",
            Some(c) if c.rows.is_empty() && !c.errors.is_empty() => "Pulizia non eseguita",
            _ => "Pulizia completata",
        };
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        let Some(c) = &self.cleaned else { return };
        if c.interrupted {
            let text = "Un errore interno ha fermato la pulizia: altri elementi potrebbero essere stati toccati senza comparire qui. Premi «Analizza» per vedere cosa resta.";
            ui.add(egui::Label::new(RichText::new(text).color(RED)).wrap());
        }
        if c.rows.is_empty() {
            if c.errors.is_empty() && !c.interrupted {
                ui.add_space(10.0);
                ui.label(RichText::new("Nessun elemento da pulire.").color(MUTED));
            }
            return;
        }
        ui.label(RichText::new(format!("Liberati {}", sys::fmt_size(c.freed))).size(15.0).color(GREEN));
        if c.mode == RemoveMode::Recycle {
            ui.label(RichText::new("Quello che è andato nel Cestino libera spazio quando lo svuoti.").size(12.0).color(MUTED));
        }
        ui.add_space(10.0);
        let w = col_widths(ui.available_width() - SCROLLBAR, &[0.0, 240.0], ui.spacing().item_spacing.x);
        header(ui, &w, &["Elemento", "Esito"]);
        egui::ScrollArea::vertical().id_salt("dev_cleaned").auto_shrink([false, false]).show_rows(ui, ROW_H, c.rows.len(), |ui, range| {
            for i in range {
                let (label, outcome) = &c.rows[i];
                let (text, color) = outcome_text(outcome);
                striped_row(ui, i, |ui| {
                    crate::cell(ui, w[0], RichText::new(label).color(TEXT));
                    crate::cell(ui, w[1], RichText::new(text).color(color));
                });
            }
        });
    }
}

/// La voce «Sviluppatori»: pannello opzioni, pulsanti in basso, card centrale, conferma.
pub fn dev_tab(state: &mut DevState, ui: &mut Ui, admin: bool) {
    if state.auto_scan && !state.busy() {
        state.auto_scan = false;
        let ctx = ui.ctx().clone();
        state.start_scan(&ctx, admin);
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
                        ui.label(RichText::new(format!("Selezionati: {}", sys::fmt_size(bytes))).color(TEXT));
                        ui.label(RichText::new(selection_text(state.section, selected)).size(12.0).color(MUTED));
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary(ui, "Avvia pulizia", state.can_clean()).clicked() {
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
    let Some(c) = &state.confirm else { return };
    let mut go = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("dev_confirm")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(confirm_title(c.section)).size(18.0).strong());
        ui.add_space(6.0);
        ui.add(egui::Label::new(confirm_text(c.section, c.mode)).wrap());
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
                .max_height(220.0)
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
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if primary(ui, &confirm_button(c), c.error.is_none() && !c.rows.is_empty()).clicked() {
                go = true;
            }
            if secondary(ui, "Annulla", true).clicked() {
                close = true;
            }
        });
    });
    if go {
        if let Some(c) = state.confirm.take().and_then(|c| state.accept_confirm(c, dc_app::now_ms())) {
            state.start_clean(ctx, c);
        }
    } else if close {
        state.confirm = None;
    }
}

fn confirm_title(section: Section) -> &'static str {
    match section {
        Section::Projects => "Pulire i progetti selezionati?",
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
    match c.section {
        Section::Sessions => format!("Cancella {}", plural(c.sessions.len(), "sessione", "sessioni")),
        Section::Projects | Section::Worktrees => "Continua".into(),
    }
}

// ---------- tables ----------

fn projects_table(ui: &mut Ui, reports: &[ProjectReport], sel: &mut [bool], failed: bool) {
    if reports.is_empty() {
        empty_note(ui, empty_text(Section::Projects, failed));
        return;
    }
    let w = col_widths(ui.available_width() - SCROLLBAR, &[CHECK_W, 140.0, 0.0, 80.0, 80.0], ui.spacing().item_spacing.x);
    header(ui, &w, &["", "Progetto", "Stato", "Ultimo uso", "Da liberare"]);
    egui::ScrollArea::vertical().id_salt("dev_projects").auto_shrink([false, false]).show_rows(ui, ROW_H, reports.len(), |ui, range| {
        for i in range {
            let r = &reports[i];
            let on = selectable(r);
            let color = if on { TEXT } else { MUTED };
            striped_row(ui, i, |ui| {
                check_cell(ui, w[0], &mut sel[i], on);
                crate::cell(ui, w[1], RichText::new(&r.name).color(color)).on_hover_text(r.root.display().to_string());
                crate::cell(ui, w[2], RichText::new(&r.reason).color(color));
                crate::cell(ui, w[3], RichText::new(r.last_activity_ms.map_or_else(|| "—".to_string(), fmt_date)).color(color));
                crate::cell(ui, w[4], RichText::new(sys::fmt_size(r.reclaimable_bytes)).color(color));
            });
        }
    });
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

/// Richieste rifiutate da `dc_app` (per esempio «Piano rifiutato: …»): per intero, mai troncate.
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

/// Lo stato e i blocchi che `dc_engine::build_plan` lascia passare: non in uso, non bloccato.
fn status_allows(r: &ProjectReport) -> bool {
    matches!(r.status, ProjectStatus::Inactive | ProjectStatus::Recent | ProjectStatus::NoSessions)
        && r.blocked_by.is_empty()
}

/// Quello che `dc_engine::build_plan` accetta E da cui ricava almeno un elemento
/// (`dc_app::plans_something`, la stessa regola): un worktree con modifiche non salvate, o di
/// stato git ignoto, senza artefatti darebbe un piano vuoto.
fn selectable(r: &ProjectReport) -> bool {
    status_allows(r) && dc_app::plans_something(r)
}

/// Perché una riga che lo stato lascerebbe passare non ha nulla da togliere; `None` altrimenti
/// (allora il `reason` del report dice già tutto).
fn nothing_to_remove(r: &ProjectReport) -> Option<&'static str> {
    if !status_allows(r) || dc_app::plans_something(r) {
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

/// Scelta di piano: tutte le sessioni archiviate non bloccate.
fn preselect_sessions(sessions: &[ArchivedSession]) -> Vec<bool> {
    sessions.iter().map(|s| s.blocked.is_none()).collect()
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

/// (selezionati, totale, byte selezionati) di una sola sezione.
fn section_stats(scan: &DevScan, section: Section, sp: &[bool], sw: &[bool], ss: &[bool]) -> (usize, usize, u64) {
    match section {
        Section::Projects => report_stats(&scan.projects, sp),
        Section::Worktrees => report_stats(&scan.worktrees, sw),
        Section::Sessions => session_stats(&scan.archived, ss),
    }
}

/// Cosa farebbe «Avvia pulizia» nella sezione visibile: i progetti o worktree da passare a
/// `build_plan`, le sessioni da passare a `delete_archived`. Mai righe di un'altra sezione.
#[derive(Debug, Default, PartialEq)]
struct Action {
    ids: Vec<String>,
    sessions: Vec<(Agent, String)>,
}

fn section_action(scan: &DevScan, section: Section, sp: &[bool], sw: &[bool], ss: &[bool]) -> Action {
    let ids = |reports: &[ProjectReport], sel: &[bool]| chosen_reports(reports, sel).map(|r| r.id.clone()).collect::<Vec<_>>();
    match section {
        Section::Projects => Action { ids: ids(&scan.projects, sp), sessions: vec![] },
        Section::Worktrees => Action { ids: ids(&scan.worktrees, sw), sessions: vec![] },
        Section::Sessions => Action {
            ids: vec![],
            sessions: chosen_sessions(&scan.archived, ss).map(|s| (s.agent, s.id.clone())).collect(),
        },
    }
}

/// Seconda riga della barra in basso: il conteggio della sola sezione visibile.
fn selection_text(section: Section, n: usize) -> String {
    match section {
        Section::Projects => plural(n, "progetto", "progetti"),
        Section::Worktrees => plural(n, "worktree", "worktree"),
        Section::Sessions => plural(n, "sessione", "sessioni"),
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

/// «progetto · Rust target/» per una cartella, «progetto · worktree intero» per un worktree.
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

fn outcome_text(o: &ItemOutcome) -> (String, Color32) {
    match o {
        ItemOutcome::Removed { bytes_freed: 0 } => ("Rimosso".into(), GREEN),
        ItemOutcome::Removed { bytes_freed } => (format!("Rimosso · {}", sys::fmt_size(*bytes_freed)), GREEN),
        ItemOutcome::Skipped { reason } => (format!("Saltato ({reason})"), MUTED),
        ItemOutcome::Failed { error, locked_by } if locked_by.is_empty() => (format!("Non riuscito ({error})"), RED),
        ItemOutcome::Failed { error, locked_by } => (format!("Non riuscito ({error}; in uso da {})", locked_by.join(", ")), RED),
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
    use dc_model::{Artifact, ArtifactKind, BlockReason, GitState, SizeInfo};
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
                path: PathBuf::from(format!(r"D:\p\{id}\target")),
                kind: ArtifactKind::RustTarget,
                label: "Rust target/".into(),
                size: SizeInfo { logical_bytes: bytes, allocated_bytes: None, file_count: 1 },
                verified: true,
            }],
            reclaimable_bytes: bytes,
            preselected,
            is_agent_worktree: false,
            git: GitState::default(),
        }
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
        // Lo stesso giudizio di dc_engine::build_plan: piano vuoto per i primi due.
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
        dc_app::build_plan(&scan, &[r.id.clone()], RemoveMode::Recycle).unwrap().items.len()
    }

    #[test]
    fn preselection_follows_the_engine_but_never_a_blocked_row() {
        let mut odd = report("c", ProjectStatus::Blocked, true, 5);
        odd.preselected = true;
        let rs = vec![report("a", ProjectStatus::Inactive, true, 1), report("b", ProjectStatus::Recent, false, 2), odd];
        assert_eq!(preselect_reports(&rs), vec![true, false, false]);
    }

    #[test]
    fn sessions_preselected_unless_blocked() {
        let ss = vec![session("1", None, false, 0), session("2", Some("sessione in uso"), true, 10), session("3", None, true, 7)];
        assert_eq!(preselect_sessions(&ss), vec![true, false, true]);
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
            scanned_at_ms: dc_app::now_ms(),
            ..Default::default()
        }
    }

    #[test]
    fn an_old_analysis_does_not_clean_projects_or_worktrees() {
        let at = 1_800_000_000_000;
        let mut st = state();
        st.set_scan(DevScan { scanned_at_ms: at, ..three_sections() });
        st.sel_worktrees = vec![true];
        let late = at + dc_app::MAX_SCAN_AGE_MS + 1;
        for section in [Section::Projects, Section::Worktrees] {
            st.section = section;
            st.open_confirm_at(late);
            assert!(st.confirm.is_none(), "{section:?}: nessun dialogo");
            assert_eq!(st.notice.as_deref(), Some(STALE_SCAN));
        }
        assert_eq!(STALE_SCAN, "L'analisi è troppo vecchia: premi «Analizza» di nuovo.");

        // Entro la mezz'ora la conferma si apre e l'avviso sparisce.
        st.section = Section::Projects;
        st.open_confirm_at(at + dc_app::MAX_SCAN_AGE_MS);
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
        let (sp, sw, ss) = ([true, true], [true], [true, true]);
        assert_eq!(section_stats(&scan, Section::Projects, &sp, &sw, &ss), (1, 2, 100));
        assert_eq!(section_stats(&scan, Section::Worktrees, &sp, &sw, &ss), (1, 1, 30));
        assert_eq!(section_stats(&scan, Section::Sessions, &sp, &sw, &ss), (2, 2, 4));
        assert_eq!(section_stats(&scan, Section::Projects, &[false, true], &sw, &ss), (0, 2, 0));
        assert_eq!(selection_text(Section::Projects, 1), "1 progetto");
        assert_eq!(selection_text(Section::Worktrees, 3), "3 worktree");
        assert_eq!(selection_text(Section::Sessions, 665), "665 sessioni");
    }

    #[test]
    fn the_action_list_holds_only_the_visible_section() {
        let scan = three_sections();
        let (sp, sw, ss) = ([true, false], [true], [true, true]);
        let p = section_action(&scan, Section::Projects, &sp, &sw, &ss);
        assert_eq!(p, Action { ids: vec!["a".into()], sessions: vec![] });
        let w = section_action(&scan, Section::Worktrees, &sp, &sw, &ss);
        assert_eq!(w, Action { ids: vec!["w".into()], sessions: vec![] });
        let s = section_action(&scan, Section::Sessions, &sp, &sw, &ss);
        assert_eq!(s, Action { ids: vec![], sessions: vec![(Agent::Codex, "1".into()), (Agent::Codex, "2".into())] });
        // Nulla scelto nella sezione visibile: niente da fare, qualunque cosa ci sia nelle altre.
        assert_eq!(section_action(&scan, Section::Worktrees, &sp, &[false], &ss), Action::default());
        assert_eq!(section_action(&scan, Section::Sessions, &sp, &sw, &[false, false]), Action::default());
    }

    #[test]
    fn the_confirmation_names_the_section_and_lists_only_its_items() {
        let mut st = state();
        st.set_scan(three_sections());
        assert_eq!((&st.sel_projects, &st.sel_worktrees, &st.sel_sessions), (&vec![true, false], &vec![false], &vec![true, true]));
        st.sel_worktrees = vec![true];

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
        assert_eq!(confirm_title(c.section), "Pulire i progetti selezionati?");
        assert_eq!(confirm_button(&c), "Continua");

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
            plan: None,
            sessions: vec![(Agent::Pi, "x".into())],
            rows: vec![],
            summary: vec![],
            error: None,
            mode: RemoveMode::Recycle,
        };
        assert_eq!(confirm_button(&one), "Cancella 1 sessione");
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
    fn outcome_texts() {
        assert_eq!(outcome_text(&ItemOutcome::Removed { bytes_freed: 0 }).0, "Rimosso");
        assert_eq!(outcome_text(&ItemOutcome::Removed { bytes_freed: 1024 }).0, "Rimosso · 1 KB");
        assert_eq!(outcome_text(&ItemOutcome::Skipped { reason: "in uso".into() }).0, "Saltato (in uso)");
        let f = ItemOutcome::Failed { error: "accesso negato".into(), locked_by: vec![] };
        assert_eq!(outcome_text(&f), ("Non riuscito (accesso negato)".into(), RED));
        let f = ItemOutcome::Failed { error: "bloccato".into(), locked_by: vec!["code.exe".into()] };
        assert_eq!(outcome_text(&f).0, "Non riuscito (bloccato; in uso da code.exe)");
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
        assert_eq!(plan_label(&scan, &item(r"D:\p\app\target", "app", PlanAction::RemoveDir)), "app · Rust target/");
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
        DevState::with_settings(dc_app::default_settings(), Section::Projects, false)
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
        assert_eq!(st.sel_projects, vec![true, false]);
        assert_eq!(st.sel_sessions, vec![true, false]);
        assert_eq!(st.selection(), (1, 2, 10));

        st.release();
        assert!(st.scan.is_none());
        assert_eq!(st.view, View::Empty);
        assert!(st.sel_projects.is_empty() && st.sel_sessions.is_empty());
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
        assert!(st.sel_projects.is_empty() && st.sel_worktrees.is_empty() && st.sel_sessions.is_empty());
        assert!(!st.can_clean());
        assert_eq!(st.error.as_deref(), Some(CLEANUP_DIED));
        assert!(CLEANUP_DIED.contains("potrebbero essere già stati rimossi"));
        assert_eq!(st.view, View::Empty);
    }

    #[test]
    fn a_panic_keeps_the_outcomes_collected_so_far() {
        let mut out = DevCleaned { rows: vec![], freed: 0, errors: vec![], mode: RemoveMode::Recycle, interrupted: false };
        guarded(&mut out, |o| {
            o.rows.push(("a · Rust target/".into(), ItemOutcome::Removed { bytes_freed: 7 }));
            o.freed = 7;
        });
        assert!(!out.interrupted);
        guarded(&mut out, |o| {
            o.rows.push(("b · node_modules/".into(), ItemOutcome::Removed { bytes_freed: 3 }));
            panic!("prova");
        });
        assert!(out.interrupted);
        assert_eq!(out.rows.len(), 2);
        assert_eq!(out.freed, 7);
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
        let rows = vec![("a · Rust target/".to_string(), ItemOutcome::Removed { bytes_freed: 10 })];
        tx.send(DevMsg::Cleaned(DevCleaned { rows, freed: 10, errors: vec![], mode: RemoveMode::Recycle, interrupted: false })).unwrap();
        st.poll();
        assert!(!st.busy());
        assert!(st.scan.is_none());
        assert_eq!(st.view, View::Cleaned);
        assert_eq!(st.cleaned.as_ref().map(|c| c.freed), Some(10));
        assert_eq!(st.selection(), (0, 0, 0));
    }
}
