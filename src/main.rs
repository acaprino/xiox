#![windows_subsystem = "windows"]

mod account;
mod browserdb;
mod cleaner;
mod dev;
mod envpath;
#[cfg(test)]
mod live_checks;
mod native;
mod netcreds;
mod regbackup;
mod registry;
mod rules;
mod services;
mod sys;
mod sysclean;
mod theme;
mod winold;
mod wipe;

use eframe::egui::{self, Align, Color32, FontId, Layout, Margin, RichText, Stroke, TextStyle, Ui, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use theme::*;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Cleaner,
    Registry,
    Services,
    Dev,
}

enum CleanMsg {
    Progress(String, f32),
    Analyzed(usize, cleaner::Found),
    Cleaned(usize, cleaner::Found, cleaner::Cleaned),
    /// Opzione dinamica: chiave `"<cleaner>/<opzione>"`, etichetta, raccolta e (in pulizia) esito.
    Rule(String, String, rules::RuleFound, Option<rules::RuleCleaned>),
    /// Voce nativa: chiave `"native:<id>"`, etichetta, analisi e (in pulizia) esito.
    Native(String, String, native::NativeFound, Option<native::NativeCleaned>),
    Done,
}

enum RegMsg {
    Progress(String, f32),
    Found(Vec<registry::Issue>),
    Done,
    /// `refused`: backup chiesto e non riuscito (il motivo); allora nessuna riparazione.
    Fixed { ok: usize, failed: usize, backup: Option<String>, refused: Option<String> },
}

enum SvcMsg {
    Loaded(Vec<services::Service>),
    Message(String),
}

/// Come è finita l'ultima analisi o pulizia di Pulizia file.
#[derive(Clone, Copy, PartialEq, Debug)]
enum CleanEnd {
    Done,
    Cancelled,
    /// Il thread di lavoro è morto (panic) prima di mandare `CleanMsg::Done`.
    Died,
}

/// Testo per una pulizia fermata da un errore interno (come in Sviluppatori).
const CLEAN_DIED: &str = "Pulizia interrotta da un errore interno: alcuni elementi potrebbero essere già stati rimossi. Premi «Analizza» per vedere cosa resta.";

/// Le voci scelte all'avvio: le categorie e (chiave, etichetta) delle voci native e delle
/// opzioni dinamiche, per elencare nel resoconto quelle che non hanno mai risposto.
#[derive(Default, Clone)]
struct Chosen {
    cats: Vec<usize>,
    others: Vec<(String, String)>,
}

#[derive(PartialEq)]
enum CleanView {
    Empty,
    Analyzed,
    Cleaned,
}

struct App {
    tab: Tab,
    admin: bool,
    /// Account del processo e della sessione, se diversi (calcolato una volta all'avvio).
    mismatch: Option<(String, String)>,
    frame: u64,
    started: Instant,
    screenshot: Option<PathBuf>,
    shot_sent: bool,
    mem: sys::Memory,
    mem_at: Instant,

    clean_sel: Vec<bool>,
    only_old: bool,
    clean_job: Option<Receiver<CleanMsg>>,
    clean_text: String,
    clean_progress: f32,
    found: Vec<Option<cleaner::Found>>,
    cleaned: Vec<Option<cleaner::Cleaned>>,
    clean_view: CleanView,
    /// «Annulla» della pulizia o dell'analisi in corso: lo stesso clic ferma le categorie
    /// (`dc_cleaner::Cancel`) e le voci native e dinamiche (`AtomicBool`).
    clean_cancel: Option<(dc_cleaner::Cancel, Arc<AtomicBool>)>,
    clean_started: Instant,
    /// Resoconto della pulizia, costruito quando tutte le voci hanno finito.
    clean_report: Option<dc_model::CleanReport>,
    /// Voci scelte all'avvio dell'ultima analisi o pulizia.
    clean_chosen: Chosen,
    /// Come è finita l'ultima analisi o pulizia (`None` = in corso o mai partita).
    clean_end: Option<CleanEnd>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    clean_saved: Option<(String, bool)>,
    confirm_clean: bool,
    /// Avvisi della finestra di conferma, calcolati una volta all'apertura
    /// (la lista dei cookie si legge lì, non a ogni fotogramma).
    confirm_warnings: Vec<String>,

    rules: Option<rules::RuleSet>,
    /// Aperto finché il thread delle regole (caricamento e aggiornamento di Winapp2) vive.
    rules_rx: Option<Receiver<rules::RulesMsg>>,
    /// Esito dell'aggiornamento di Winapp2 di questo avvio (`None` = in corso o non partito).
    winapp2_update: Option<pc_rules::updater::UpdateOutcome>,
    /// Finestra Informazioni aperta.
    about_open: bool,
    rule_sel: HashSet<String>,
    rule_found: Vec<(String, String, rules::RuleFound)>,
    rule_cleaned: Vec<(String, String, rules::RuleCleaned)>,
    rule_search: String,
    /// Voci native (Appunti, DNS…): selezione `native:<id>` in `rule_sel`.
    natives: Vec<native::NativeItem>,
    native_found: Vec<(String, String, native::NativeFound)>,
    native_cleaned: Vec<(String, String, native::NativeCleaned)>,
    /// Processi attivi: aggiornati a ogni «Analizza» e prima di ogni pulizia.
    processes: Vec<String>,

    reg_sel: Vec<bool>,
    issues: Vec<registry::Issue>,
    reg_job: Option<Receiver<RegMsg>>,
    reg_text: String,
    reg_progress: f32,
    reg_scanned: bool,
    reg_confirm: bool,
    reg_msg: String,

    services: Vec<services::Service>,
    svc_job: Option<Receiver<SvcMsg>>,
    svc_loaded: bool,
    svc_all: bool,
    svc_filter: String,
    svc_msg: String,
    svc_confirm: Option<u32>,

    dev: dev::DevState,
}

impl App {
    fn new(screenshot: Option<PathBuf>, tab: Tab, dev: dev::DevState, about_open: bool) -> Self {
        let n = cleaner::CATEGORIES.len();
        let natives = native::all();
        let mut app = App {
            tab,
            admin: sys::is_admin(),
            mismatch: sys::profile_mismatch(),
            frame: 0,
            started: Instant::now(),
            screenshot,
            shot_sent: false,
            mem: sys::memory(),
            mem_at: Instant::now(),
            clean_sel: cleaner::CATEGORIES.iter().map(|c| c.default_on).collect(),
            only_old: true,
            clean_job: None,
            clean_text: String::new(),
            clean_progress: 0.0,
            found: vec![None; n],
            cleaned: vec![None; n],
            clean_view: CleanView::Empty,
            clean_cancel: None,
            clean_started: Instant::now(),
            clean_report: None,
            clean_chosen: Chosen::default(),
            clean_end: None,
            clean_saved: None,
            confirm_clean: false,
            confirm_warnings: vec![],
            rules: None,
            rules_rx: None,
            winapp2_update: None,
            about_open,
            rule_sel: rules::native_defaults(&natives),
            rule_found: vec![],
            rule_cleaned: vec![],
            rule_search: String::new(),
            natives,
            native_found: vec![],
            native_cleaned: vec![],
            processes: rules::processes(),
            reg_sel: registry::CATEGORIES.iter().map(|c| c.default_on).collect(),
            issues: vec![],
            reg_job: None,
            reg_text: String::new(),
            reg_progress: 0.0,
            reg_scanned: false,
            reg_confirm: false,
            reg_msg: String::new(),
            services: vec![],
            svc_job: None,
            svc_loaded: false,
            svc_all: false,
            svc_filter: String::new(),
            svc_msg: String::new(),
            svc_confirm: None,
            dev,
        };
        if tab == Tab::Services {
            app.load_services();
        }
        app
    }

    // ---------- background jobs ----------

    /// Carica le regole esterne in un thread; finché non arrivano, i gruppi dinamici non ci sono.
    /// Poi, tranne con --screenshot, aggiorna Winapp2 in sottofondo (nessun `join`:
    /// l'uscita non lo aspetta) e ricarica le regole se è arrivata una copia nuova.
    fn start_rules(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.rules_rx = Some(rx);
        // Con --screenshot non si aggiorna Winapp2; solo nella build di debug le
        // regole usano l'ambiente del processo, così un profilo finto
        // (USERPROFILE, LOCALAPPDATA…) vale anche per loro.
        let fake_profile = self.screenshot.is_some();
        let from_process = rules::rules_env_from_process(fake_profile);
        let update = rules::should_update(fake_profile).then(rules::real_update);
        let ctx = ctx.clone();
        rules::start(
            pc_rules::updater::data_dir(),
            rules::own_folders,
            move || if from_process { pc_rules::Env::from_process() } else { pc_rules::Env::for_rules() },
            rules::load,
            update,
            move |m| {
                let _ = tx.send(m);
                ctx.request_repaint();
            },
        );
    }

    /// Opzioni dinamiche spuntate (cleaner e opzione clonati per il job).
    fn chosen_rules(&self) -> Vec<(pc_rules::Cleaner, pc_rules::Opt)> {
        let Some(rs) = &self.rules else { return vec![] };
        rs.cleaners
            .iter()
            .flat_map(|c| c.options.iter().map(move |o| (c, o)))
            .filter(|(c, o)| self.rule_sel.contains(&rules::key(c, o)))
            .map(|(c, o)| (c.clone(), o.clone()))
            .collect()
    }

    fn start_clean(&mut self, also_clean: bool, ctx: &egui::Context) {
        let cats: Vec<usize> = (0..self.clean_sel.len()).filter(|&i| self.clean_sel[i]).collect();
        self.processes = rules::processes();
        let dynamic = self.chosen_rules();
        let natives: Vec<native::NativeItem> = self.natives.iter().filter(|i| self.rule_sel.contains(&rules::native_key(i))).copied().collect();
        let env = self.rules.as_ref().map(|r| r.env.clone());
        self.clean_chosen = Chosen {
            cats: cats.clone(),
            others: natives
                .iter()
                .map(|i| (rules::native_key(i), i.label.to_string()))
                .chain(dynamic.iter().map(|(c, o)| (rules::key(c, o), rules::row_label(c, o))))
                .collect(),
        };
        self.clean_end = None;
        // «Pulisci spazio libero» gira dopo tutto il resto (anche le opzioni
        // dinamiche), perché sovrascriva anche ciò che le altre voci cancellano.
        let (natives, natives_last) = native::split_last(natives);
        let (tx, rx) = channel();
        self.clean_job = Some(rx);
        self.found = vec![None; self.clean_sel.len()];
        self.cleaned = vec![None; self.clean_sel.len()];
        self.rule_found.clear();
        self.rule_cleaned.clear();
        // Voci con anteprima obbligatoria (PATH): con «Pulizia» agiscono solo su
        // ciò che l'ultima «Analisi» ha mostrato, se la vista è ancora quella.
        let previews: std::collections::HashMap<String, native::NativeFound> =
            if also_clean && self.clean_view == CleanView::Analyzed {
                self.native_found.iter().map(|(k, _, f)| (k.clone(), f.clone())).collect()
            } else {
                std::collections::HashMap::new()
            };
        self.native_found.clear();
        self.native_cleaned.clear();
        self.clean_view = if also_clean { CleanView::Cleaned } else { CleanView::Analyzed };
        self.clean_report = None;
        self.clean_saved = None;
        self.clean_started = Instant::now();
        let files_cancel = dc_cleaner::Cancel::default();
        let cancel = Arc::new(AtomicBool::new(false));
        self.clean_cancel = Some((files_cancel.clone(), cancel.clone()));
        let only_old = self.only_old;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let verb = if also_clean { "Pulizia" } else { "Analisi" };
            let total = (cats.len() + natives.len() + dynamic.len() + natives_last.len()).max(1) as f32;
            for (k, &i) in cats.iter().enumerate() {
                let name = cleaner::CATEGORIES[i].name;
                if files_cancel.is_cancelled() || cancel.load(Ordering::SeqCst) {
                    // Annullata prima di iniziare: nel resoconto tra i saltati.
                    if also_clean {
                        let c = cleaner::Cleaned { cancelled: true, ..Default::default() };
                        let _ = tx.send(CleanMsg::Cleaned(i, cleaner::Found::default(), c));
                    }
                    continue;
                }
                let _ = tx.send(CleanMsg::Progress(format!("{verb} – {name}"), k as f32 / total));
                ctx.request_repaint();
                let f = cleaner::analyze(i, only_old);
                if also_clean {
                    // L'avanzamento dentro la categoria affina la barra.
                    let tick = |t: dc_cleaner::Tick| {
                        if t.total_items > 0 {
                            let part = t.done_items as f32 / t.total_items as f32;
                            let _ = tx.send(CleanMsg::Progress(format!("{verb} – {name}"), (k as f32 + part) / total));
                            ctx.request_repaint();
                        }
                    };
                    let c = cleaner::clean(i, &f, &tick, &files_cancel);
                    let _ = tx.send(CleanMsg::Cleaned(i, f, c));
                } else {
                    let _ = tx.send(CleanMsg::Analyzed(i, f));
                }
            }
            // Voci native e opzioni dinamiche: in sequenza, un solo Guard per tutta
            // la pulizia; i processi si rileggono prima dell'analisi e della pulizia
            // di ognuna. Regole non ancora caricate: l'ambiente fidato di sempre.
            let env = env.unwrap_or_else(pc_rules::Env::for_rules);
            let guard = pc_rules::Guard::new(&env);
            let processes = rules::processes;
            let nctx = native::NativeCtx { env: &env, cancel: &*cancel, guard: &guard };
            let nprogress = |k: usize, text: String| {
                let _ = tx.send(CleanMsg::Progress(text, (cats.len() + k) as f32 / total));
                ctx.request_repaint();
            };
            let previews = |item: &native::NativeItem| previews.get(&rules::native_key(item)).cloned();
            native::run_natives(&natives, also_clean, &nctx, &previews, &processes, &nprogress, &mut |item, f, done| {
                let _ = tx.send(CleanMsg::Native(rules::native_key(item), item.label.to_string(), f, done));
            });
            let run = rules::RunCtx {
                env: &env,
                guard: &guard,
                reg: &rules::RealRegistry,
                reg_exists: &rules::reg_exists,
                processes: &processes,
                cancel: &*cancel,
            };
            let progress = |k: usize, text: String| {
                let _ = tx.send(CleanMsg::Progress(text, (cats.len() + natives.len() + k) as f32 / total));
                ctx.request_repaint();
            };
            rules::run_options(&dynamic, also_clean, &run, &progress, &mut |key, label, f, done| {
                let _ = tx.send(CleanMsg::Rule(key, label, f, done));
            });
            let lprogress = |k: usize, text: String| {
                let _ = tx.send(CleanMsg::Progress(text, (cats.len() + natives.len() + dynamic.len() + k) as f32 / total));
                ctx.request_repaint();
            };
            native::run_natives(&natives_last, also_clean, &nctx, &previews, &processes, &lprogress, &mut |item, f, done| {
                let _ = tx.send(CleanMsg::Native(rules::native_key(item), item.label.to_string(), f, done));
            });
            let _ = tx.send(CleanMsg::Done);
            ctx.request_repaint();
        });
    }

    /// Fine dell'analisi o della pulizia (`died`: il thread è morto senza `Done`): dopo una
    /// pulizia, il resoconto di tutte le voci, anche di quelle che non hanno mai risposto.
    fn finish_clean(&mut self, died: bool) {
        self.clean_job = None;
        let cancelled = self.clean_cancel.take().is_some_and(|(f, a)| f.is_cancelled() || a.load(Ordering::SeqCst));
        let end = if died {
            CleanEnd::Died
        } else if cancelled {
            CleanEnd::Cancelled
        } else {
            CleanEnd::Done
        };
        self.clean_end = Some(end);
        if self.clean_view == CleanView::Cleaned {
            let elapsed = self.clean_started.elapsed().as_millis() as u64;
            self.clean_report = Some(clean_report(
                &self.cleaned,
                &self.native_cleaned,
                &self.rule_cleaned,
                &self.clean_chosen,
                elapsed,
                end,
            ));
        }
    }

    /// «Annulla»: ferma le categorie, le voci native e le opzioni dinamiche.
    fn cancel_clean(&self) {
        if let Some((files, other)) = &self.clean_cancel {
            files.cancel();
            other.store(true, Ordering::SeqCst);
        }
    }

    fn clean_cancelling(&self) -> bool {
        self.clean_cancel.as_ref().is_some_and(|(f, a)| f.is_cancelled() || a.load(Ordering::SeqCst))
    }

    /// Solo build di debug, `--clean-demo` con `--screenshot`: un resoconto inventato per
    /// vedere la vista. Non legge né tocca alcun file.
    #[cfg(debug_assertions)]
    fn clean_demo(&mut self) {
        let user = PathBuf::from(r"C:\Users\demo\AppData\Local\Temp");
        self.clean_sel = vec![false; self.clean_sel.len()];
        for (i, freed, removed) in [(1usize, 734_003_200u64, 1_204u64), (3, 52_428_800, 2), (6, 18_874_368, 9)] {
            self.clean_sel[i] = true;
            self.found[i] = Some(cleaner::Found { bytes: freed, count: removed, ..Default::default() });
            let mut c = cleaner::Cleaned { freed, removed, dirs: if i == 1 { 37 } else { 0 }, ..Default::default() };
            if i == 1 {
                c.failed = 2;
                c.failures = vec![
                    (user.join("setup.log"), "Il file è in uso (in uso da: setup.exe)".into()),
                    (user.join("~DF1A2B.tmp"), "Accesso negato. (os error 5)".into()),
                ];
            }
            self.cleaned[i] = Some(c);
        }
        self.clean_sel[8] = true;
        self.cleaned[8] = Some(cleaner::Cleaned { cancelled: true, ..Default::default() });
        let cats: Vec<usize> = (0..self.clean_sel.len()).filter(|&i| self.clean_sel[i]).collect();
        let native = native::NativeCleaned { freed: 0, removed: 1, done: vec!["Appunti svuotati".into()], ..Default::default() };
        self.native_cleaned.push(("native:clipboard".into(), "Appunti".into(), native));
        let rule = rules::RuleCleaned {
            freed: 157_286_400,
            removed: 3_310,
            files: 3_310,
            failed: 1,
            notes: vec!["1 file in uso".into()],
            ..Default::default()
        };
        self.rule_cleaned.push(("demo/cache".into(), "Google Chrome – Cache".into(), rule));
        let mut skipped = rules::RuleCleaned::default();
        skipped.notes.push("Saltata: Firefox in esecuzione".into());
        self.rule_cleaned.push(("demo/ff".into(), "Firefox – Cache".into(), skipped));
        self.clean_chosen = Chosen {
            cats,
            others: vec![
                ("native:clipboard".into(), "Appunti".into()),
                ("demo/cache".into(), "Google Chrome – Cache".into()),
                ("demo/ff".into(), "Firefox – Cache".into()),
                ("demo/edge".into(), "Microsoft Edge – Cache".into()),
            ],
        };
        self.clean_view = CleanView::Cleaned;
        self.clean_end = Some(CleanEnd::Cancelled);
        self.clean_report = Some(clean_report(
            &self.cleaned,
            &self.native_cleaned,
            &self.rule_cleaned,
            &self.clean_chosen,
            8_400,
            CleanEnd::Cancelled,
        ));
    }

    fn start_reg_scan(&mut self, ctx: &egui::Context) {
        let cats: Vec<usize> = (0..self.reg_sel.len()).filter(|&i| self.reg_sel[i]).collect();
        let (tx, rx) = channel();
        self.reg_job = Some(rx);
        self.issues.clear();
        self.reg_msg.clear();
        self.reg_scanned = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let total = cats.len().max(1) as f32;
            for (k, &i) in cats.iter().enumerate() {
                let name = registry::CATEGORIES[i].name;
                let _ = tx.send(RegMsg::Progress(format!("Controllo: {name}"), k as f32 / total));
                ctx.request_repaint();
                let found = registry::scan(i, &|_| {});
                let _ = tx.send(RegMsg::Found(found));
            }
            let _ = tx.send(RegMsg::Done);
            ctx.request_repaint();
        });
    }

    fn start_reg_fix(&mut self, with_backup: bool, ctx: &egui::Context) {
        let chosen: Vec<registry::Issue> = self.issues.iter().filter(|i| i.selected).cloned().collect();
        let (tx, rx) = channel();
        self.reg_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if with_backup {
                let _ = tx.send(RegMsg::Progress("Salvataggio del backup…".into(), 0.0));
            }
            let total = chosen.len().max(1) as f32;
            // Backup chiesto e non riuscito: non si ripara nulla.
            let out = registry::fix_all(&chosen, with_backup, &registry::backup, &mut registry::apply, &mut |k| {
                if k % 20 == 0 {
                    let _ = tx.send(RegMsg::Progress("Riparazione in corso…".into(), k as f32 / total));
                    ctx.request_repaint();
                }
            });
            let backup = out.backup.map(|p| p.to_string_lossy().to_string());
            let _ = tx.send(RegMsg::Fixed { ok: out.ok, failed: out.failed, backup, refused: out.refused });
            ctx.request_repaint();
        });
    }

    fn load_services(&mut self) {
        let (tx, rx) = channel();
        self.svc_job = Some(rx);
        self.svc_loaded = true;
        std::thread::spawn(move || {
            let _ = tx.send(SvcMsg::Loaded(services::load()));
        });
    }

    fn apply_services(&mut self, start: Option<u32>) {
        let chosen: Vec<services::Service> = self.services.iter().filter(|s| s.selected && !s.protected).cloned().collect();
        let (tx, rx) = channel();
        self.svc_job = Some(rx);
        std::thread::spawn(move || send_svc(tx, chosen, start));
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.rules_rx {
            let mut ended = false;
            loop {
                match rx.try_recv() {
                    Ok(rules::RulesMsg::Loaded(set)) => self.rules = Some(set),
                    Ok(rules::RulesMsg::Updated(outcome)) => self.winapp2_update = Some(outcome),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
            if ended {
                self.rules_rx = None;
            }
        }
        if let Some(rx) = &self.clean_job {
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
                    CleanMsg::Progress(t, p) => {
                        self.clean_text = t;
                        self.clean_progress = p;
                    }
                    CleanMsg::Analyzed(i, f) => self.found[i] = Some(f),
                    CleanMsg::Cleaned(i, f, c) => {
                        self.found[i] = Some(f);
                        self.cleaned[i] = Some(c);
                    }
                    CleanMsg::Rule(key, label, f, c) => {
                        if let Some(c) = c {
                            self.rule_cleaned.push((key.clone(), label.clone(), c));
                        }
                        self.rule_found.push((key, label, f));
                    }
                    CleanMsg::Native(key, label, f, c) => {
                        if let Some(c) = c {
                            self.native_cleaned.push((key.clone(), label.clone(), c));
                        }
                        self.native_found.push((key, label, f));
                    }
                    CleanMsg::Done => done = true,
                }
            }
            // Thread morto senza `Done` (panic): il resoconto con quello che è arrivato.
            if done || gone {
                self.finish_clean(!done);
            }
        }
        if let Some(rx) = &self.reg_job {
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
                        self.reg_text = t;
                        self.reg_progress = p;
                    }
                    RegMsg::Found(v) => self.issues.extend(v),
                    RegMsg::Done => done = true,
                    RegMsg::Fixed { refused: Some(why), .. } => {
                        self.reg_msg = format!("Backup del registro non riuscito ({why}): nessuna riparazione eseguita.");
                        done = true;
                    }
                    RegMsg::Fixed { ok, failed, backup, refused: None } => {
                        self.reg_msg = format!("Riparati {ok} problemi.");
                        if failed > 0 {
                            self.reg_msg.push_str(&format!(" {failed} non riparati (servono i diritti di amministratore)."));
                        }
                        if let Some(b) = backup {
                            self.reg_msg.push_str(&format!("\nBackup salvato in: {b}"));
                        }
                        if failed == 0 {
                            self.issues.retain(|i| !i.selected);
                        }
                        done = true;
                    }
                }
            }
            if gone && !done {
                self.reg_msg = "Il lavoro si è interrotto per un errore interno.".into();
                done = true;
            }
            if done {
                self.reg_job = None;
            }
        }
        if let Some(rx) = &self.svc_job {
            let mut reload = false;
            let mut done = false;
            while let Ok(m) = rx.try_recv() {
                match m {
                    SvcMsg::Loaded(v) => {
                        self.services = v;
                        done = true;
                    }
                    SvcMsg::Message(t) => {
                        self.svc_msg = t;
                        reload = true;
                    }
                }
            }
            if done {
                self.svc_job = None;
            }
            if reload {
                self.svc_job = None;
                self.load_services();
            }
        }
        self.dev.poll();
    }

    // ---------- drawing ----------

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(0, 14)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let (r, p) = ui.allocate_painter(Vec2::splat(34.0), egui::Sense::hover());
                    let c = r.rect.center();
                    p.circle_filled(c, 16.0, ACCENT);
                    p.circle_filled(c, 9.0, SIDEBAR);
                    p.circle_filled(c + Vec2::new(6.0, -6.0), 4.0, GREEN);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("PulisciPC").color(Color32::WHITE).size(18.0).strong());
                        ui.label(RichText::new("Pulizia leggera").color(Color32::from_gray(150)).size(12.0));
                    });
                });
                ui.add_space(22.0);
                for (tab, icon, label) in [
                    (Tab::Cleaner, "🗑", "Pulizia file"),
                    (Tab::Registry, "🔧", "Registro"),
                    (Tab::Services, "⚙", "Servizi e RAM"),
                    (Tab::Dev, "💻", "Sviluppatori"),
                ] {
                    let sel = self.tab == tab;
                    let fill = if sel { SIDEBAR_SEL } else { SIDEBAR };
                    let text = RichText::new(format!("  {icon}   {label}")).size(15.0).color(if sel { Color32::WHITE } else { Color32::from_gray(190) });
                    let resp = ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(212.0, 46.0)));
                    if sel {
                        ui.painter().rect_filled(egui::Rect::from_min_size(resp.rect.min, Vec2::new(4.0, resp.rect.height())), 0.0, ACCENT);
                    }
                    if resp.clicked() {
                        if self.tab == Tab::Dev && tab != Tab::Dev {
                            // Uscendo dalla voce si liberano i dati dell'analisi (se nessun lavoro è in corso).
                            self.dev.release();
                        }
                        self.tab = tab;
                        if tab == Tab::Services && !self.svc_loaded {
                            self.load_services();
                        }
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let ram = format!("RAM: {} GB", (self.mem.total as f64 / 1073741824.0).round());
                        ui.label(RichText::new(ram).color(Color32::from_gray(140)).size(12.0));
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        let (t, c) = if self.admin { ("Amministratore", GREEN) } else { ("Utente standard", Color32::from_rgb(0xe0, 0xa0, 0x30)) };
                        let (r, p) = ui.allocate_painter(Vec2::new(10.0, 14.0), egui::Sense::hover());
                        p.circle_filled(r.rect.center(), 4.0, c);
                        ui.label(RichText::new(t).color(c).size(12.0));
                    });
                    ui.add_space(8.0);
                    // Stessa forma delle voci in alto; layout dall'alto per avere il testo a sinistra come loro.
                    ui.allocate_ui_with_layout(Vec2::new(212.0, 46.0), Layout::top_down(Align::Min), |ui| {
                        let open = self.about_open;
                        let text = RichText::new("  ℹ   Informazioni").size(15.0).color(if open { Color32::WHITE } else { Color32::from_gray(190) });
                        let fill = if open { SIDEBAR_SEL } else { SIDEBAR };
                        if ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(212.0, 46.0))).clicked() {
                            self.about_open = true;
                        }
                    });
                });
            });
    }

    /// Finestra Informazioni: nome, versione, Winapp2 caricato e attribuzione.
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let winapp2 = match self.rules.as_ref() {
            None => "Winapp2: caricamento in corso".to_string(),
            Some(rs) => match &rs.winapp2_version {
                Some(v) => format!("Winapp2: versione {v}"),
                None => "Winapp2: non ancora scaricato".to_string(),
            },
        };
        let mut close = false;
        let resp = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("PulisciPC").size(18.0).strong());
            ui.add_space(6.0);
            ui.label(format!("Versione {}", env!("CARGO_PKG_VERSION")));
            ui.label(winapp2);
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(rules::ABOUT_RULES).color(MUTED)).wrap());
            ui.add_space(12.0);
            if secondary(ui, "Chiudi", true).clicked() {
                close = true;
            }
        });
        if close || resp.should_close() {
            self.about_open = false;
        }
    }

    /// Avviso quando l'app gira con un account diverso da quello della sessione (profilo sbagliato).
    fn profile_strip(&self, ui: &mut Ui) {
        let Some((p, s)) = &self.mismatch else { return };
        egui::Frame::new().fill(WARN_BG).corner_radius(6).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            let text = format!("⚠  Stai pulendo il profilo di «{p}», non quello di «{s}» che ha aperto la sessione.");
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
        });
        ui.add_space(8.0);
    }

    fn cleaner_tab(&mut self, ui: &mut Ui) {
        let busy = self.clean_job.is_some();
        egui::SidePanel::left("clean_opts")
            .exact_width(270.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let mut group = "";
                    for (i, c) in cleaner::CATEGORIES.iter().enumerate() {
                        if c.group != group {
                            group = c.group;
                            ui.add_space(6.0);
                            ui.label(RichText::new(group).strong().color(ACCENT));
                            ui.separator();
                        }
                        ui.add_enabled_ui(!busy, |ui| {
                            let label = if c.needs_admin && !self.admin { format!("{}  🛡", c.name) } else { c.name.to_string() };
                            ui.checkbox(&mut self.clean_sel[i], label).on_hover_text(c.help);
                        });
                    }
                    let natives = Natives { items: &self.natives, admin: self.admin };
                    match &self.rules {
                        Some(rs) => {
                            let status = rules::status_line(rs, self.winapp2_update.as_ref());
                            rules_panel(ui, rs, natives, status, &mut self.rule_sel, &mut self.rule_search, &self.processes, busy);
                        }
                        // Regole non ancora caricate: le voci native ci sono già.
                        None => group_panel(ui, None, natives, &rules::panel_groups(&[], natives.items), &mut self.rule_sel, &self.processes, busy),
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Opzioni").strong().color(ACCENT));
                    ui.separator();
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.checkbox(&mut self.only_old, format!("Elimina dalle cartelle Temp solo i file più vecchi di {} ore", cleaner::TEMP_MIN_AGE_HOURS));
                    });
                });
            });
        egui::TopBottomPanel::bottom("clean_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let any = self.clean_sel.iter().any(|&b| b)
                        || !rules::active_natives(&self.natives, &self.rule_sel, &self.processes).is_empty()
                        || self.rules.as_ref().is_some_and(|rs| !rules::active_choices(&rs.cleaners, &self.rule_sel, &self.processes).is_empty());
                    if primary(ui, "Avvia pulizia", !busy && any).clicked() {
                        self.confirm_warnings = self.rule_warnings();
                        self.confirm_clean = true;
                    }
                    if secondary(ui, "Analizza", !busy && any).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_clean(false, &ctx);
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.clean_text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.clean_progress).fill(ACCENT).show_percentage());
                    ui.add_space(6.0);
                    let cancelling = self.clean_cancelling();
                    if secondary(ui, if cancelling { "Annullamento…" } else { "Annulla" }, !cancelling).clicked() {
                        self.cancel_clean();
                    }
                    ui.add_space(10.0);
                }
                match self.clean_view {
                    CleanView::Empty => {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Pulizia dei file temporanei").size(22.0).strong().color(TEXT));
                            ui.add_space(6.0);
                            ui.label(RichText::new("Scegli a sinistra cosa pulire, poi premi «Analizza» per vedere quanto spazio puoi liberare.").color(MUTED));
                        });
                    }
                    CleanView::Analyzed => self.analysis_table(ui),
                    CleanView::Cleaned => self.cleaned_view(ui),
                }
            });
        });
        if self.confirm_clean {
            let mut close = false;
            let warnings = self.confirm_warnings.clone();
            egui::Modal::new(egui::Id::new("confirm_clean")).show(ui.ctx(), |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Avviare la pulizia?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("I file selezionati verranno eliminati definitivamente. I file in uso vengono lasciati dove sono.");
                if !warnings.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Avvisi delle opzioni scelte").strong());
                    let list = |ui: &mut Ui| {
                        for w in &warnings {
                            ui.label(RichText::new(w).color(RED));
                        }
                    };
                    if warnings.len() > 4 {
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, list);
                    } else {
                        list(ui);
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_clean(true, &ctx);
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.confirm_clean = false;
            }
        }
    }

    /// "<voce>: <avviso>" per ogni voce nativa e "<cleaner> – <opzione>: <avviso>"
    /// per ogni opzione dinamica spuntata, non bloccata, con un avviso.
    fn rule_warnings(&self) -> Vec<String> {
        // Solo le scelte che si pulirebbero davvero (non quelle bloccate).
        let active = rules::active_natives(&self.natives, &self.rule_sel, &self.processes);
        let mut out = rules::native_warnings(&active);
        // Le voci con anteprima usano l'ultima «Analisi» solo se la vista è ancora quella (vedi `start_clean`).
        let analyzed = self.clean_view == CleanView::Analyzed;
        let shown = |i: &native::NativeItem| {
            let key = rules::native_key(i);
            analyzed.then(|| self.native_found.iter().find(|(k, _, _)| *k == key).map(|(_, _, f)| f.items.len() as u64)).flatten()
        };
        out.extend(rules::preview_notes(&active, &shown));
        if let Some(rs) = &self.rules {
            out.extend(rules::choice_warnings(&rules::active_choices(&rs.cleaners, &self.rule_sel, &self.processes)));
        }
        out
    }

    fn analysis_table(&self, ui: &mut Ui) {
        let total: u64 = self.found.iter().flatten().map(|f| f.bytes).sum::<u64>()
            + self.native_found.iter().map(|r| r.2.bytes).sum::<u64>()
            + self.rule_found.iter().map(|r| r.2.bytes).sum::<u64>();
        let files: u64 = self.found.iter().flatten().map(|f| f.count).sum::<u64>()
            + self.native_found.iter().map(|r| r.2.files).sum::<u64>()
            + self.rule_found.iter().map(|r| r.2.count).sum::<u64>();
        let (title, note) = analysis_title(self.clean_job.is_some(), self.clean_end);
        ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
        ui.label(RichText::new(format!("{} da eliminare ({} file)", sys::fmt_size(total), files)).size(15.0).color(ACCENT));
        if let Some(note) = note {
            ui.label(RichText::new(note).color(RED));
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("an").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Elemento").strong());
                ui.label(RichText::new("Dimensione").strong());
                ui.label(RichText::new("File").strong());
                ui.label("");
                ui.end_row();
                for (i, f) in self.found.iter().enumerate() {
                    if let Some(f) = f {
                        ui.label(cleaner::CATEGORIES[i].name);
                        ui.label(sys::fmt_size(f.bytes));
                        ui.label(f.count.to_string());
                        if f.skipped_dirs > 0 && !self.admin {
                            ui.label(RichText::new("🛡 alcune cartelle richiedono l'amministratore").color(MUTED).size(12.0));
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                }
                for (_, label, f) in &self.native_found {
                    ui.label(label);
                    ui.label(sys::fmt_size(f.bytes));
                    // Canali, DNS e Appunti non sono file: il numero sta nel dettaglio.
                    ui.label(if f.files > 0 { f.files.to_string() } else { "—".to_string() });
                    note_cell(ui, &f.detail);
                    ui.end_row();
                }
                for (_, label, f) in &self.rule_found {
                    ui.label(label);
                    ui.label(sys::fmt_size(f.bytes));
                    ui.label(f.count.to_string());
                    if f.skipped > 0 {
                        ui.label(RichText::new(format!("{} elementi lasciati (protetti o illeggibili)", f.skipped)).color(MUTED).size(12.0))
                            .on_hover_text(f.skipped_reasons.join("\n"));
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
            for (key, label, f) in &self.native_found {
                native_list(ui, key, label, &f.detail);
            }
        });
    }

    fn cleaned_view(&mut self, ui: &mut Ui) {
        let busy = self.clean_job.is_some();
        let report = if busy { None } else { self.clean_report.as_ref() };
        let Some(r) = report else {
            // Durante la pulizia: il totale e le voci finite, come sempre.
            let freed: u64 = self.cleaned.iter().flatten().map(|c| c.freed).sum::<u64>()
                + self.native_cleaned.iter().map(|r| r.2.freed).sum::<u64>()
                + self.rule_cleaned.iter().map(|r| r.2.freed).sum::<u64>();
            let title = if busy { "Pulizia in corso…" } else { "Pulizia completata" };
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            ui.label(RichText::new(format!("Liberati {}", sys::fmt_size(freed))).size(15.0).color(GREEN));
            ui.add_space(10.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.cleaned_detail(ui));
            return;
        };
        let title = if r.cancelled { "Pulizia annullata" } else { "Pulizia completata" };
        let mut save = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                save = secondary(ui, "Salva come testo…", true).clicked();
            });
        });
        // Con --screenshot nessun dialogo di sistema.
        if save && self.screenshot.is_none() {
            if let Some(out) = dev::save_report_text(r) {
                self.clean_saved = Some(out);
            }
        }
        if let Some((text, ok)) = &self.clean_saved {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if *ok { GREEN } else { RED })).wrap());
        }
        let Some(r) = &self.clean_report else { return };
        dev::report_view(ui, r, &|p| p.display().to_string(), "clean_report", |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Dettaglio per voce").strong().color(TEXT));
            self.cleaned_detail(ui);
        });
    }

    /// Una riga per categoria, voce nativa e opzione dinamica: liberati, eliminati, lasciati, note.
    fn cleaned_detail(&self, ui: &mut Ui) {
        egui::Grid::new("cl").striped(true).min_col_width(80.0).spacing([24.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Elemento").strong());
            ui.label(RichText::new("Liberati").strong());
            ui.label(RichText::new("Eliminati").strong());
            ui.label(RichText::new("Lasciati (in uso)").strong());
            ui.label("");
            ui.end_row();
            for (i, c) in self.cleaned.iter().enumerate() {
                if let Some(c) = c {
                    ui.label(cleaner::CATEGORIES[i].name);
                    ui.label(sys::fmt_size(c.freed));
                    ui.label(c.removed.to_string());
                    ui.label(c.failed.to_string());
                    ui.label("");
                    ui.end_row();
                }
            }
            for (_, label, c) in &self.native_cleaned {
                ui.label(label);
                ui.label(sys::fmt_size(c.freed));
                ui.label(c.removed.to_string());
                ui.label(c.failed.to_string());
                note_cell(ui, &c.notes);
                ui.end_row();
            }
            for (_, label, c) in &self.rule_cleaned {
                ui.label(label);
                ui.label(sys::fmt_size(c.freed));
                ui.label(c.removed.to_string());
                ui.label(c.failed.to_string());
                let mut notes = c.notes.clone();
                if let Some(b) = &c.backup {
                    notes.push(format!("Backup del registro: {}", b.display()));
                }
                match notes.first() {
                    Some(first) => {
                        ui.label(RichText::new(first).color(MUTED).size(12.0)).on_hover_text(notes.join("\n"));
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
        for (key, label, c) in &self.native_cleaned {
            let lines: Vec<String> = c.done.iter().chain(&c.notes).cloned().collect();
            native_list(ui, key, label, &lines);
        }
    }

    fn registry_tab(&mut self, ui: &mut Ui) {
        let busy = self.reg_job.is_some();
        egui::SidePanel::left("reg_opts")
            .exact_width(270.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin::same(14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show_inside(ui, |ui| {
                ui.label(RichText::new("Registro").strong().color(ACCENT));
                ui.separator();
                ui.add_enabled_ui(!busy, |ui| {
                    for (i, c) in registry::CATEGORIES.iter().enumerate() {
                        ui.checkbox(&mut self.reg_sel[i], c.name);
                    }
                });
                ui.add_space(14.0);
                ui.label(RichText::new("Prima di riparare puoi salvare un backup: con un doppio clic sul file .reg si torna indietro.").size(12.0).color(MUTED));
            });
        egui::TopBottomPanel::bottom("reg_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let sel = self.issues.iter().filter(|i| i.selected).count();
                    if primary(ui, &format!("Ripara selezionati ({sel})…"), !busy && sel > 0).clicked() {
                        self.reg_confirm = true;
                    }
                    if secondary(ui, "Cerca problemi", !busy && self.reg_sel.iter().any(|&b| b)).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_scan(&ctx);
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            card(ui, |ui| {
                if busy {
                    ui.label(RichText::new(&self.reg_text).size(16.0).color(TEXT));
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(self.reg_progress).fill(ACCENT).show_percentage());
                    ui.add_space(10.0);
                }
                if !self.reg_msg.is_empty() {
                    ui.label(RichText::new(&self.reg_msg).color(GREEN));
                    ui.add_space(8.0);
                }
                if !self.reg_scanned {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Pulizia del registro").size(22.0).strong().color(TEXT));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Premi «Cerca problemi» per trovare voci che puntano a file o programmi che non esistono più.").color(MUTED));
                    });
                    return;
                }
                if self.issues.is_empty() && !busy {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("✔  Nessun problema trovato").size(18.0).color(GREEN)));
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} problemi trovati", self.issues.len())).size(18.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Nessuno").clicked() {
                            self.issues.iter_mut().for_each(|i| i.selected = false);
                        }
                        if ui.small_button("Tutti").clicked() {
                            self.issues.iter_mut().for_each(|i| i.selected = true);
                        }
                    });
                });
                ui.add_space(6.0);
                let w = ui.available_width();
                let (wp, wd) = (210.0, ((w - 270.0) * 0.45).max(120.0));
                let wk = (w - 30.0 - wp - wd - 20.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    cell(ui, wp, RichText::new("Problema").strong());
                    cell(ui, wd, RichText::new("Dati").strong());
                    cell(ui, wk, RichText::new("Chiave di registro").strong());
                });
                ui.separator();
                let n = self.issues.len();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, n, |ui, range| {
                    for i in range {
                        let it = &mut self.issues[i];
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut it.selected, "");
                            cell(ui, wp, &it.problem);
                            cell(ui, wd, &it.data);
                            cell(ui, wk, it.fix.key_text());
                        });
                    }
                });
            });
        });
        if self.reg_confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("reg_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Salvare un backup del registro?").size(18.0).strong());
                ui.add_space(6.0);
                ui.label("Consigliato: prima delle modifiche viene salvata una copia (.reg) nella cartella Documenti\\PulisciPC.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Sì, salva e ripara", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_fix(true, &ctx);
                        close = true;
                    }
                    if secondary(ui, "Ripara senza backup", true).clicked() {
                        let ctx = ui.ctx().clone();
                        self.start_reg_fix(false, &ctx);
                        close = true;
                    }
                    if ui.button("Annulla").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.reg_confirm = false;
            }
        }
    }

    fn services_tab(&mut self, ui: &mut Ui) {
        if self.mem_at.elapsed() > Duration::from_secs(2) {
            self.mem = sys::memory();
            self.mem_at = Instant::now();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        let busy = self.svc_job.is_some();
        egui::TopBottomPanel::bottom("svc_buttons")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(16, 12)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let sel: Vec<&services::Service> = self.services.iter().filter(|s| s.selected && !s.protected).collect();
                    let kb: u64 = sel.iter().map(|s| s.freeable_kb()).sum();
                    ui.label(RichText::new(format!("Selezionati: {}  ·  RAM liberabile (stima): {}", sel.len(), sys::fmt_size(kb * 1024))).color(TEXT));
                    let n = sel.len();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, "Disattiva selezionati", !busy && n > 0).clicked() {
                            self.svc_confirm = Some(services::START_DISABLED);
                        }
                        if secondary(ui, "Imposta su Manuale", !busy && n > 0).clicked() {
                            self.svc_confirm = Some(services::START_MANUAL);
                        }
                        let saved = services::read_backup().len();
                        if secondary(ui, &format!("Ripristina originali ({saved})"), !busy && saved > 0).clicked() {
                            self.svc_confirm = Some(0);
                        }
                    });
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(16))).show_inside(ui, |ui| {
            self.profile_strip(ui);
            // RAM card
            card(ui, |ui| {
                let used = self.mem.total.saturating_sub(self.mem.available);
                let frac = if self.mem.total > 0 { used as f32 / self.mem.total as f32 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Memoria RAM in uso").size(16.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} di {} ({:.0}%)", sys::fmt_size(used), sys::fmt_size(self.mem.total), frac * 100.0)).color(MUTED));
                    });
                });
                ui.add_space(4.0);
                let color = if frac > 0.85 { Color32::from_rgb(0xd9, 0x48, 0x3b) } else if frac > 0.65 { Color32::from_rgb(0xe0, 0xa0, 0x30) } else { GREEN };
                let bar_w = ui.max_rect().width();
                ui.add(egui::ProgressBar::new(frac).fill(color).desired_height(14.0).desired_width(bar_w));
            });
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.svc_all, false, "Consigliati da spegnere");
                    ui.selectable_value(&mut self.svc_all, true, "Tutti i servizi");
                    ui.add_space(12.0);
                    ui.add(egui::TextEdit::singleline(&mut self.svc_filter).hint_text("Cerca…").desired_width(180.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!busy, egui::Button::new("⟳ Aggiorna")).clicked() {
                            self.load_services();
                        }
                    });
                });
                if !self.svc_msg.is_empty() {
                    ui.label(RichText::new(&self.svc_msg).color(GREEN));
                }
                ui.add_space(6.0);
                if busy && self.services.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Lettura dei servizi…");
                    });
                    return;
                }
                let filter = self.svc_filter.to_lowercase();
                let rows: Vec<usize> = (0..self.services.len())
                    .filter(|&i| {
                        let s = &self.services[i];
                        (self.svc_all || s.suggestion.is_some())
                            && (filter.is_empty() || s.display.to_lowercase().contains(&filter) || s.name.to_lowercase().contains(&filter))
                    })
                    .collect();
                let w = ui.available_width();
                let (wn, ws, wa, wm) = (((w - 60.0) * 0.30).max(160.0), 110.0, 150.0, 120.0);
                let wd = (w - 40.0 - wn - ws - wa - wm - 30.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    cell(ui, wn, RichText::new("Servizio").strong());
                    cell(ui, ws, RichText::new("Stato").strong());
                    cell(ui, wa, RichText::new("Avvio").strong());
                    cell(ui, wm, RichText::new("Memoria").strong());
                    cell(ui, wd, RichText::new("Perché spegnerlo").strong());
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, rows.len(), |ui, range| {
                    for &i in &rows[range] {
                        let s = &mut self.services[i];
                        ui.horizontal(|ui| {
                            if s.protected {
                                ui.add_enabled(false, egui::Checkbox::new(&mut false, "")).on_disabled_hover_text("Servizio necessario a Windows: protetto");
                            } else {
                                ui.checkbox(&mut s.selected, "");
                            }
                            cell(ui, wn, RichText::new(&s.display).color(TEXT)).on_hover_text(&s.name);
                            let (st, sc) = if s.running { ("In esecuzione", GREEN) } else { ("Fermo", MUTED) };
                            cell(ui, ws, RichText::new(st).color(sc));
                            let ac = if s.start == services::START_DISABLED { Color32::from_rgb(0xd9, 0x48, 0x3b) } else { TEXT };
                            cell(ui, wa, RichText::new(s.start_text()).color(ac));
                            let mem = if !s.running {
                                "–".to_string()
                            } else if s.shared > 1 {
                                format!("{} (condivisa)", sys::fmt_size(s.mem_kb * 1024))
                            } else {
                                sys::fmt_size(s.mem_kb * 1024)
                            };
                            cell(ui, wm, mem);
                            let note = if s.protected { "🔒 Necessario a Windows" } else { s.suggestion.unwrap_or("") };
                            cell(ui, wd, RichText::new(note).color(MUTED));
                        });
                    }
                });
            });
        });
        if let Some(action) = self.svc_confirm {
            let mut close = false;
            egui::Modal::new(egui::Id::new("svc_confirm")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                let (title, body) = match action {
                    0 => ("Ripristinare i servizi?", "Tutti i servizi cambiati da PulisciPC tornano come erano prima."),
                    services::START_MANUAL => ("Impostare su Manuale?", "I servizi si fermano subito e Windows li avvierà solo quando servono."),
                    _ => ("Disattivare i servizi?", "I servizi si fermano subito e non partiranno più. Potrai sempre usare «Ripristina originali»."),
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(6.0);
                ui.label(body);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Continua", true).clicked() {
                        if action == 0 {
                            self.apply_services(None);
                        } else {
                            self.apply_services(Some(action));
                        }
                        close = true;
                    }
                    if secondary(ui, "Annulla", true).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.svc_confirm = None;
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        ctx.request_repaint();
        let busy = self.clean_job.is_some()
            || self.reg_job.is_some()
            || self.svc_job.is_some()
            || self.dev.busy()
            || self.dev.demo_pending()
            || self.rules_rx.is_some();
        // L'analisi Sviluppatori dura circa un minuto senza amministratore: la si aspetta fino a 5 minuti.
        let dev_wait = self.dev.busy() && self.started.elapsed() < Duration::from_secs(300);
        if self.frame >= 120 && (!busy || self.frame >= 900) && !dev_wait && !self.shot_sent {
            self.shot_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
                .collect()
        });
        if let Some(img) = shots.first() {
            let [w, h] = img.size;
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = buf.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn send_svc(tx: Sender<SvcMsg>, chosen: Vec<services::Service>, start: Option<u32>) {
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
    let _ = tx.send(SvcMsg::Message(msg));
}

/// Gruppi dinamici del pannello sinistro di Pulizia file: gli inglobati per
/// gruppo ("Windows", "Browser", "Sviluppo e AI", `rules::GROUP_ORDER`; un
/// `CollapsingHeader` per cleaner) e "Applicazioni" (Winapp2, uno per sezione,
/// con «Cerca»). Il corpo di un `CollapsingHeader` si costruisce solo se aperto.
/// `status`: riga di stato di Winapp2 e motivo al passaggio del mouse (`rules::status_line`).
#[allow(clippy::too_many_arguments)]
fn rules_panel(ui: &mut Ui, rs: &rules::RuleSet, natives: Natives, status: (String, Option<String>), sel: &mut HashSet<String>, search: &mut String, processes: &[String], busy: bool) {
    group_panel(ui, Some(rs), natives, &rules::panel_groups(&rs.groups, natives.items), sel, processes, busy);
    ui.add_space(6.0);
    ui.label(RichText::new(rules::WINAPP2_GROUP).strong().color(ACCENT));
    ui.separator();
    let (text, why) = status;
    let hover: Vec<&str> = why.as_deref().into_iter().chain(rs.warnings.iter().map(String::as_str)).collect();
    let status = ui.label(RichText::new(text).color(MUTED).size(12.0));
    if !hover.is_empty() {
        status.on_hover_text(hover.join("\n"));
    }
    if rs.sections.is_empty() {
        return;
    }
    ui.add(egui::TextEdit::singleline(search).hint_text("Cerca").desired_width(f32::INFINITY));
    let needle = search.trim().to_lowercase();
    let hit = |text: &str| text.to_lowercase().contains(&needle);
    for (section, idx) in &rs.sections {
        let whole = needle.is_empty() || hit(section);
        let items: Vec<&pc_rules::Cleaner> = idx
            .iter()
            .map(|&i| &rs.cleaners[i])
            .filter(|c| whole || hit(&c.label) || c.options.iter().any(|o| hit(&o.label)))
            .collect();
        if items.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(format!("{section} ({})", items.len()))
            .id_salt(("winapp2", section))
            .default_open(false)
            .show(ui, |ui| {
                for c in items {
                    for o in &c.options {
                        rule_checkbox(ui, c, o, &c.label, sel, processes, busy);
                    }
                }
            });
    }
}

/// Le voci native e se l'app è amministratore (per il 🛡).
#[derive(Clone, Copy)]
struct Natives<'a> {
    items: &'a [native::NativeItem],
    admin: bool,
}

/// I gruppi dei cleaner inglobati (`rules::panel_groups`): titolo, poi le voci
/// native del gruppo come caselle, poi un `CollapsingHeader` per cleaner XML.
fn group_panel(ui: &mut Ui, rs: Option<&rules::RuleSet>, natives: Natives, groups: &[(String, Vec<usize>, Vec<usize>)], sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    for (group, nat, idx) in groups {
        ui.add_space(6.0);
        ui.label(RichText::new(group).strong().color(ACCENT));
        ui.separator();
        for item in nat.iter().map(|&n| &natives.items[n]) {
            native_checkbox(ui, item, natives.admin, sel, processes, busy);
        }
        let Some(rs) = rs else { continue };
        for c in idx.iter().map(|&i| &rs.cleaners[i]) {
            let header = egui::CollapsingHeader::new(&c.label).id_salt(("rule", &c.id)).default_open(false).show(ui, |ui| {
                for o in &c.options {
                    rule_checkbox(ui, c, o, &o.label, sel, processes, busy);
                }
            });
            if !c.description.is_empty() {
                header.header_response.on_hover_text(&c.description);
            }
        }
    }
}

/// Casella di una voce nativa: come le categorie statiche (🛡 se serve
/// l'amministratore), disattivata se un suo processo è attivo, avviso al
/// passaggio del mouse.
fn native_checkbox(ui: &mut Ui, item: &native::NativeItem, admin: bool, sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    let key = rules::native_key(item);
    let label = if item.needs_admin && !admin { format!("{}  🛡", item.label) } else { item.label.to_string() };
    if let Some(why) = rules::native_blocked(item, processes) {
        let mut off = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(&why);
        ui.label(RichText::new(why).color(MUTED).size(12.0));
        return;
    }
    let mut help = item.description.to_string();
    if let Some(w) = item.warning {
        help.push_str("\n\nAttenzione: ");
        help.push_str(w);
    }
    let mut on = sel.contains(&key);
    let r = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, label)).on_hover_text(help);
    if r.changed() {
        if on {
            sel.insert(key);
        } else {
            sel.remove(&key);
        }
    }
    if item.id == browserdb::COOKIES_ID {
        ui.add_space(2.0);
        let hint = "Apre Documenti\\PulisciPC\\cookie_da_conservare.txt nel Blocco note (lo crea se manca). \
                    Un dominio per riga: i suoi cookie restano.";
        if secondary(ui, "Modifica lista cookie", true).on_hover_text(hint).clicked() {
            open_keep_list();
        }
        ui.add_space(4.0);
    }
}

/// Apre la lista dei cookie da conservare in un thread a parte (crearla e
/// avviare l'editor non deve fermare la UI); un errore si mostra in una finestra.
fn open_keep_list() {
    std::thread::spawn(|| {
        if let Err(e) = browserdb::open_keep_list() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Lista dei cookie da conservare")
                .set_description(format!("Non è stato possibile aprire la lista: {e}"))
                .show();
        }
    });
}

/// Elenco completo di una voce nativa sotto la tabella (collegamenti trovati,
/// canali svuotati, note), aperto: si legge prima di «Avvia pulizia».
fn native_list(ui: &mut Ui, key: &str, label: &str, lines: &[String]) {
    if lines.len() < 2 {
        return;
    }
    ui.add_space(8.0);
    egui::CollapsingHeader::new(format!("{label}: elenco ({} righe)", lines.len()))
        .id_salt(("native_list", key))
        .default_open(true)
        .show(ui, |ui| {
            for l in lines {
                ui.label(RichText::new(l).color(MUTED).size(12.0));
            }
        });
}

/// Ultima colonna delle tabelle: la prima riga, le altre al passaggio del mouse.
fn note_cell(ui: &mut Ui, lines: &[String]) {
    match lines.first() {
        Some(first) => {
            ui.label(RichText::new(first).color(MUTED).size(12.0)).on_hover_text(lines.join("\n"));
        }
        None => {
            ui.label("");
        }
    }
}

/// Casella di un'opzione dinamica: disattivata se non supportata (spiegazione al
/// passaggio del mouse) o se un suo processo è attivo ("Chiudi … per pulire").
fn rule_checkbox(ui: &mut Ui, c: &pc_rules::Cleaner, o: &pc_rules::Opt, label: &str, sel: &mut HashSet<String>, processes: &[String], busy: bool) {
    let key = rules::key(c, o);
    if let Some(why) = rules::blocked(c, o, processes) {
        let mut off = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(&why);
        if o.unsupported.is_none() {
            ui.label(RichText::new(why).color(MUTED).size(12.0));
        }
        return;
    }
    let mut on = sel.contains(&key);
    let mut help = o.description.clone();
    if let Some(w) = &o.warning {
        if !help.is_empty() {
            help.push_str("\n\n");
        }
        help.push_str("Attenzione: ");
        help.push_str(w);
    }
    let mut r = ui.add_enabled(!busy, egui::Checkbox::new(&mut on, label));
    if !help.is_empty() {
        r = r.on_hover_text(help);
    }
    if r.changed() {
        if on {
            sel.insert(key);
        } else {
            sel.remove(&key);
        }
    }
}

/// Titolo dell'analisi e, se non è arrivata in fondo, la nota che lo dice.
fn analysis_title(busy: bool, end: Option<CleanEnd>) -> (&'static str, Option<&'static str>) {
    match (busy, end) {
        (true, _) => ("Analisi in corso…", None),
        (false, Some(CleanEnd::Cancelled)) => {
            ("Analisi annullata", Some("Analisi annullata: le voci mancanti non sono state analizzate."))
        }
        (false, Some(CleanEnd::Died)) => (
            "Analisi interrotta",
            Some("Un errore interno ha fermato l'analisi: le voci mancanti non sono state analizzate."),
        ),
        (false, _) => ("Analisi completata", None),
    }
}

/// La nota di una voce nativa o dinamica che dice che non è stata pulita del tutto
/// (saltata, interrotta, non supportata), se c'è.
fn stop_note(notes: &[String]) -> Option<&String> {
    notes.iter().find(|n| n.starts_with("Saltata") || n.contains("interrotta") || n.starts_with("Opzione non supportata"))
}

/// Il resoconto di Pulizia file: categorie (nome di `CATEGORIES`), voci native e opzioni
/// dinamiche (la loro etichetta). Le categorie elencano i file rimasti uno per uno; le voci
/// native e dinamiche, che contano soltanto, compaiono con il numero e la prima nota; quelle
/// saltate o interrotte tra i saltati con il motivo. Le voci di `chosen` che non hanno mai
/// risposto (annullate prima di partire, o thread morto) sono anch'esse tra i saltate.
///
/// «File eliminati»: i file delle categorie (gli elementi del Cestino sono file nel Cestino,
/// quindi contano) e i soli file delle opzioni dinamiche (`RuleCleaned::files`, non le voci
/// di registro). Le voci native no: tolgono canali, righe di database, Appunti…, non file.
fn clean_report(
    cleaned: &[Option<cleaner::Cleaned>],
    natives: &[(String, String, native::NativeCleaned)],
    rules: &[(String, String, rules::RuleCleaned)],
    chosen: &Chosen,
    elapsed_ms: u64,
    end: CleanEnd,
) -> dc_model::CleanReport {
    let mut r = dc_model::CleanReport { elapsed_ms, cancelled: end != CleanEnd::Done, ..Default::default() };
    if end == CleanEnd::Died {
        r.failed.push((PathBuf::from("Pulizia"), CLEAN_DIED.to_string()));
    }
    let never = if end == CleanEnd::Died { "Non eseguito: pulizia interrotta da un errore interno" } else { "Annullato" };
    let counted = |n: u64, notes: &[String]| match notes.first() {
        Some(first) => format!("{n} elementi lasciati: {first}"),
        None => format!("{n} elementi lasciati (in uso o rifiutati)"),
    };
    for (i, c) in cleaned.iter().enumerate() {
        let Some(c) = c else { continue };
        let name = cleaner::CATEGORIES[i].name;
        r.freed_bytes += c.freed;
        r.files += c.removed;
        r.dirs += c.dirs;
        if c.freed > 0 {
            r.by_category.push((name.to_string(), c.freed));
        }
        r.failed.extend(c.failures.iter().cloned());
        // Fallimenti contati ma senza percorso (non dovrebbe accadere): una riga per la categoria.
        let unnamed = c.failed.saturating_sub(c.failures.len() as u64);
        if unnamed > 0 {
            r.failed.push((PathBuf::from(name), counted(unnamed, &[])));
        }
        if c.cancelled {
            let why = if c.removed == 0 && c.failed == 0 {
                "Annullato".to_string()
            } else {
                format!("Annullato: liberati {}", sys::fmt_size(c.freed))
            };
            r.skipped.push((PathBuf::from(name), why));
        }
    }
    for &i in &chosen.cats {
        if cleaned.get(i).is_none_or(|c| c.is_none()) {
            r.skipped.push((PathBuf::from(cleaner::CATEGORIES[i].name), never.to_string()));
        }
    }
    let others = natives
        .iter()
        .map(|(_, label, c)| (label, c.freed, 0, c.failed, &c.notes))
        .chain(rules.iter().map(|(_, label, c)| (label, c.freed, c.files, c.failed, &c.notes)));
    for (label, freed, files, failed, notes) in others {
        r.freed_bytes += freed;
        r.files += files;
        if freed > 0 {
            r.by_category.push((label.clone(), freed));
        }
        if failed > 0 {
            r.failed.push((PathBuf::from(label), counted(failed, notes)));
        }
        if let Some(why) = stop_note(notes) {
            let why = if freed > 0 { format!("{why}: liberati {}", sys::fmt_size(freed)) } else { why.clone() };
            r.skipped.push((PathBuf::from(label), why));
        }
    }
    for (key, label) in &chosen.others {
        let reported = natives.iter().any(|(k, _, _)| k == key) || rules.iter().any(|(k, _, _)| k == key);
        if !reported {
            r.skipped.push((PathBuf::from(label), never.to_string()));
        }
    }
    r.by_category.sort_by(|a, b| b.1.cmp(&a.1));
    r
}

fn cell(ui: &mut Ui, w: f32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.poll();
        if self.clean_job.is_some() || self.reg_job.is_some() || self.svc_job.is_some() || self.dev.busy() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        self.sidebar(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ctx, |ui| match self.tab {
            Tab::Cleaner => self.cleaner_tab(ui),
            Tab::Registry => self.registry_tab(ui),
            Tab::Services => self.services_tab(ui),
            Tab::Dev => {
                if self.mismatch.is_some() {
                    egui::TopBottomPanel::top("dev_profile")
                        .frame(egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 0 }))
                        .show_inside(ui, |ui| self.profile_strip(ui));
                }
                dev::dev_tab(&mut self.dev, ui, self.admin)
            }
        });
        self.about_window(ctx);
        self.handle_screenshot(ctx);
    }

    /// Con --screenshot il puntatore non c'è: niente evidenziazioni né suggerimenti dove
    /// capita che il cursore stia (sotto Xvfb sta al centro della finestra).
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.screenshot.is_some() {
            raw_input.events.retain(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_)));
            raw_input.events.push(egui::Event::PointerGone);
        }
    }
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xe8, 0xf0, 0xfd);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xf7, 0xf8, 0xfa);
    v.extreme_bg_color = Color32::from_rgb(0xe9, 0xeb, 0xef);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(175));
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(12.0, 5.0);
        s.spacing.interact_size.y = 22.0;
    });
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut screenshot = None;
    let mut tab = Tab::Cleaner;
    let mut analyze = false;
    let mut about = false;
    let mut section = dev::Section::Projects;
    let mut only_root: Option<PathBuf> = None;
    #[cfg(debug_assertions)]
    let mut demo = None;
    #[cfg(debug_assertions)]
    let mut clean_demo = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" if i + 1 < args.len() => {
                screenshot = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--tab" if i + 1 < args.len() => {
                tab = match args[i + 1].as_str() {
                    "registry" | "registro" => Tab::Registry,
                    "services" | "servizi" => Tab::Services,
                    "dev" | "sviluppatori" => Tab::Dev,
                    _ => Tab::Cleaner,
                };
                i += 1;
            }
            "--analyze" => analyze = true,
            // Come --tab: apre una vista all'avvio (per gli screenshot della finestra Informazioni).
            "--about" => about = true,
            "--dev-section" if i + 1 < args.len() => {
                section = dev::Section::parse(&args[i + 1]).unwrap_or(section);
                i += 1;
            }
            // Verifiche su un albero finto: si cerca solo lì, impostazioni in memoria.
            "--only-root" if i + 1 < args.len() => {
                only_root = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            // Solo build di debug e solo con --only-root su un albero finto: conferma, pulizia o
            // annulla da soli. Nelle build di rilascio non esiste.
            #[cfg(debug_assertions)]
            "--dev-demo" if i + 1 < args.len() => {
                demo = dev::Demo::parse(&args[i + 1]);
                i += 1;
            }
            // Solo build di debug e solo con --screenshot: un resoconto di Pulizia file inventato
            // (nessun file letto o toccato). Nelle build di rilascio non esiste.
            #[cfg(debug_assertions)]
            "--clean-demo" => clean_demo = true,
            _ => {}
        }
        i += 1;
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PulisciPC")
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([900.0, 560.0]);
    if screenshot.is_some() {
        viewport = viewport.with_active(false).with_window_level(egui::WindowLevel::AlwaysOnBottom);
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    // Cartelle di «Pulisci spazio libero» rimaste da uno spegnimento brusco:
    // via subito, in un thread a parte (mai negli screenshot né su un albero finto).
    if screenshot.is_none() && only_root.is_none() {
        wipe::remove_leftovers_in_background();
    }
    eframe::run_native(
        "PulisciPC",
        options,
        Box::new(move |cc| {
            style(&cc.egui_ctx);
            let mut dev = match only_root {
                Some(dir) => dev::DevState::only_root(dir, section, analyze),
                None => dev::DevState::new(section, analyze),
            };
            if screenshot.is_some() {
                dev.set_screenshot();
            }
            #[cfg(debug_assertions)]
            if let Some(d) = demo {
                if let Err(e) = dev.set_demo(d) {
                    eprintln!("{e}");
                    return Err(e.into());
                }
            }
            #[cfg(debug_assertions)]
            let clean_demo = clean_demo && screenshot.is_some();
            let mut app = App::new(screenshot, tab, dev, about);
            #[cfg(debug_assertions)]
            if clean_demo {
                app.clean_demo();
            }
            app.start_rules(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Il resoconto somma categorie, voci native e opzioni dinamiche, le ordina per spazio e
    /// riporta i file rimasti, le voci con fallimenti contati e le categorie annullate.
    #[test]
    fn clean_report_merges_categories_natives_and_rules() {
        let mut cleaned = vec![None; cleaner::CATEGORIES.len()];
        let locked = PathBuf::from(r"C:\t\a.tmp");
        cleaned[1] = Some(cleaner::Cleaned {
            freed: 100,
            removed: 4,
            failed: 1,
            dirs: 2,
            failures: vec![(locked.clone(), "in uso".into())],
            cancelled: false,
        });
        cleaned[3] = Some(cleaner::Cleaned { cancelled: true, ..Default::default() });
        let natives = vec![(
            "native:x".to_string(),
            "Appunti".to_string(),
            native::NativeCleaned { freed: 0, removed: 1, ..Default::default() },
        )];
        let rules = vec![(
            "c/o".to_string(),
            "Chrome – Cache".to_string(),
            rules::RuleCleaned {
                freed: 500,
                removed: 12,
                files: 10,
                failed: 2,
                notes: vec!["2 file in uso".into()],
                ..Default::default()
            },
        )];
        let r = clean_report(&cleaned, &natives, &rules, &Chosen::default(), 1200, CleanEnd::Cancelled);
        assert_eq!(r.freed_bytes, 600);
        // 4 file della categoria e i 10 file della regola: non gli Appunti, non le voci di registro.
        assert_eq!(r.files, 14);
        assert_eq!(r.dirs, 2);
        assert_eq!(r.elapsed_ms, 1200);
        assert!(r.cancelled);
        assert_eq!(
            r.by_category,
            vec![("Chrome – Cache".to_string(), 500), (cleaner::CATEGORIES[1].name.to_string(), 100)]
        );
        assert_eq!(r.failed.len(), 2);
        assert_eq!(r.failed[0], (locked, "in uso".to_string()));
        assert_eq!(r.failed[1].0, PathBuf::from("Chrome – Cache"));
        assert!(r.failed[1].1.contains("2 elementi lasciati: 2 file in uso"));
        assert_eq!(r.skipped, vec![(PathBuf::from(cleaner::CATEGORIES[3].name), "Annullato".to_string())]);
        assert!(r.to_text().contains("Spazio liberato"));
    }

    /// Voci native e dinamiche saltate o interrotte senza fallimenti, e voci scelte che non
    /// hanno mai risposto (annullate prima di partire): tutte tra i saltati, anche nel testo.
    #[test]
    fn clean_report_lists_skipped_interrupted_and_never_started_items() {
        let mut cleaned = vec![None; cleaner::CATEGORIES.len()];
        cleaned[1] = Some(cleaner::Cleaned { freed: 10, removed: 1, ..Default::default() });
        let mut chrome = native::NativeCleaned::default();
        chrome.note("Saltata: Chrome in esecuzione".into());
        let natives = vec![("native:chrome".to_string(), "Chrome – Cronologia".to_string(), chrome)];
        let rules = vec![(
            "c/o".to_string(),
            "Firefox – Cache".to_string(),
            rules::RuleCleaned { freed: 2048, removed: 3, files: 3, notes: vec!["Pulizia interrotta".into()], ..Default::default() },
        )];
        let chosen = Chosen {
            cats: vec![1, 2],
            others: vec![
                ("native:chrome".into(), "Chrome – Cronologia".into()),
                ("c/o".into(), "Firefox – Cache".into()),
                ("native:dns".into(), "Cache DNS".into()),
                ("x/y".into(), "Edge – Cache".into()),
            ],
        };
        let r = clean_report(&cleaned, &natives, &rules, &chosen, 10, CleanEnd::Cancelled);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        let skipped: Vec<(String, String)> =
            r.skipped.iter().map(|(p, w)| (p.display().to_string(), w.clone())).collect();
        assert_eq!(
            skipped,
            vec![
                (cleaner::CATEGORIES[2].name.to_string(), "Annullato".to_string()),
                ("Chrome – Cronologia".to_string(), "Saltata: Chrome in esecuzione".to_string()),
                ("Firefox – Cache".to_string(), "Pulizia interrotta: liberati 2 KB".to_string()),
                ("Cache DNS".to_string(), "Annullato".to_string()),
                ("Edge – Cache".to_string(), "Annullato".to_string()),
            ]
        );
        let text = r.to_text();
        assert!(text.contains("Saltati (5):"), "{text}");
        assert!(text.contains("Chrome – Cronologia: Saltata: Chrome in esecuzione"), "{text}");
        assert!(text.contains("Edge – Cache: Annullato"), "{text}");
    }

    fn test_app() -> (tempfile::TempDir, App) {
        let t = tempfile::tempdir().unwrap();
        let dev = dev::DevState::only_root(t.path().to_path_buf(), dev::Section::Projects, false);
        let app = App::new(None, Tab::Cleaner, dev, false);
        (t, app)
    }

    /// Thread della pulizia morto (panic) prima di `Done`: il job si chiude, il resoconto
    /// si costruisce con quello che è arrivato e dice dell'errore; le voci mai arrivate
    /// sono tra i saltati.
    #[test]
    fn dead_cleaning_thread_still_builds_the_report() {
        let (_t, mut app) = test_app();
        let (tx, rx) = channel();
        app.clean_job = Some(rx);
        app.clean_view = CleanView::Cleaned;
        app.clean_cancel = Some((dc_cleaner::Cancel::default(), Arc::new(AtomicBool::new(false))));
        app.clean_chosen = Chosen { cats: vec![1, 3], others: vec![("native:dns".into(), "Cache DNS".into())] };
        let c = cleaner::Cleaned { freed: 100, removed: 2, ..Default::default() };
        tx.send(CleanMsg::Cleaned(1, cleaner::Found::default(), c)).unwrap();
        app.poll();
        assert!(app.clean_job.is_some(), "il thread è vivo: si aspetta");
        drop(tx);
        app.poll();
        assert!(app.clean_job.is_none());
        assert!(app.clean_cancel.is_none());
        assert_eq!(app.clean_end, Some(CleanEnd::Died));
        let r = app.clean_report.as_ref().expect("resoconto");
        assert!(r.cancelled);
        assert_eq!((r.freed_bytes, r.files), (100, 2));
        assert_eq!(r.failed, vec![(PathBuf::from("Pulizia"), CLEAN_DIED.to_string())]);
        let never = "Non eseguito: pulizia interrotta da un errore interno".to_string();
        assert_eq!(
            r.skipped,
            vec![(PathBuf::from(cleaner::CATEGORIES[3].name), never.clone()), (PathBuf::from("Cache DNS"), never)]
        );
        assert!(r.to_text().contains("Pulizia interrotta da un errore interno"));
    }

    /// Analisi: thread morto o «Annulla» chiudono il job e il titolo dice che non è completa.
    #[test]
    fn dead_or_cancelled_analysis_is_marked_interrupted() {
        let (_t, mut app) = test_app();
        let (tx, rx) = channel::<CleanMsg>();
        app.clean_job = Some(rx);
        app.clean_view = CleanView::Analyzed;
        drop(tx);
        app.poll();
        assert!(app.clean_job.is_none());
        assert!(app.clean_report.is_none());
        assert_eq!(app.clean_end, Some(CleanEnd::Died));
        assert_eq!(analysis_title(false, app.clean_end).0, "Analisi interrotta");

        let (tx, rx) = channel();
        app.clean_job = Some(rx);
        let cancel = dc_cleaner::Cancel::default();
        cancel.cancel();
        app.clean_cancel = Some((cancel, Arc::new(AtomicBool::new(false))));
        tx.send(CleanMsg::Done).unwrap();
        app.poll();
        assert!(app.clean_job.is_none());
        assert_eq!(app.clean_end, Some(CleanEnd::Cancelled));
        let (title, note) = analysis_title(false, app.clean_end);
        assert_eq!(title, "Analisi annullata");
        assert!(note.unwrap().contains("le voci mancanti non sono state analizzate"));
        assert_eq!(analysis_title(false, Some(CleanEnd::Done)), ("Analisi completata", None));
        assert_eq!(analysis_title(true, None).0, "Analisi in corso…");
    }
}
