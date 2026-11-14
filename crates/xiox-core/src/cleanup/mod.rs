//! Stato di Pulizia file senza interfaccia: scelte, lavori in un thread, analisi, pulizia,
//! resoconto e opzioni dinamiche. Spostato da `src/main.rs` così com'era; al posto di
//! `egui::Context` i thread svegliano chi mostra la sezione con un [`SharedNotifier`].

use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::{cleaner, native, rules, sys};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod tests;

pub enum CleanMsg {
    Progress(String, f32),
    Analyzed(usize, cleaner::Found),
    Cleaned(usize, cleaner::Found, cleaner::Cleaned),
    /// Opzione dinamica: chiave `"<cleaner>/<opzione>"`, etichetta, raccolta e (in pulizia) esito.
    Rule(String, String, rules::RuleFound, Option<rules::RuleCleaned>),
    /// Voce nativa: chiave `"native:<id>"`, etichetta, analisi e (in pulizia) esito.
    Native(String, String, native::NativeFound, Option<native::NativeCleaned>),
    Done,
}

/// Come è finita l'ultima analisi o pulizia di Pulizia file.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CleanEnd {
    Done,
    Cancelled,
    /// Il thread di lavoro è morto (panic) prima di mandare `CleanMsg::Done`.
    Died,
}

/// Testo per una pulizia fermata da un errore interno (come in Sviluppatori).
pub const CLEAN_DIED: &str = "Pulizia interrotta da un errore interno: alcuni elementi potrebbero essere già stati rimossi. Premi «Analizza» per vedere cosa resta.";

/// Le voci scelte all'avvio: le categorie e (chiave, etichetta) delle voci native e delle
/// opzioni dinamiche, per elencare nel resoconto quelle che non hanno mai risposto.
#[derive(Default, Clone)]
pub struct Chosen {
    pub cats: Vec<usize>,
    pub others: Vec<(String, String)>,
}

#[derive(PartialEq)]
pub enum CleanView {
    Empty,
    Analyzed,
    Cleaned,
}

/// Lo stato di Pulizia file: tutto ciò che la sezione sa e i lavori in corso.
pub struct CleanupState {
    pub clean_sel: Vec<bool>,
    pub only_old: bool,
    pub clean_job: Option<Receiver<CleanMsg>>,
    pub clean_text: String,
    pub clean_progress: f32,
    pub found: Vec<Option<cleaner::Found>>,
    pub cleaned: Vec<Option<cleaner::Cleaned>>,
    pub clean_view: CleanView,
    /// «Annulla» della pulizia o dell'analisi in corso: lo stesso clic ferma le categorie
    /// (`xiox_cleaner::Cancel`) e le voci native e dinamiche (`AtomicBool`).
    pub clean_cancel: Option<(xiox_cleaner::Cancel, Arc<AtomicBool>)>,
    pub clean_started: Instant,
    /// Resoconto della pulizia, costruito quando tutte le voci hanno finito.
    pub clean_report: Option<xiox_model::CleanReport>,
    /// Voci scelte all'avvio dell'ultima analisi o pulizia.
    pub clean_chosen: Chosen,
    /// Come è finita l'ultima analisi o pulizia (`None` = in corso o mai partita).
    pub clean_end: Option<CleanEnd>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    pub clean_saved: Option<(String, bool)>,
    pub confirm_clean: bool,
    /// Avvisi della finestra di conferma, calcolati una volta all'apertura
    /// (la lista dei cookie si legge lì, non a ogni fotogramma).
    pub confirm_warnings: Vec<String>,

    pub rules: Option<rules::RuleSet>,
    /// Aperto finché il thread delle regole (caricamento e aggiornamento di Winapp2) vive.
    pub rules_rx: Option<Receiver<rules::RulesMsg>>,
    /// Esito dell'aggiornamento di Winapp2 di questo avvio (`None` = in corso o non partito).
    pub winapp2_update: Option<xiox_rules::updater::UpdateOutcome>,
    pub rule_sel: HashSet<String>,
    pub rule_found: Vec<(String, String, rules::RuleFound)>,
    pub rule_cleaned: Vec<(String, String, rules::RuleCleaned)>,
    pub rule_search: String,
    /// Voci native (Appunti, DNS…): selezione `native:<id>` in `rule_sel`.
    pub natives: Vec<native::NativeItem>,
    pub native_found: Vec<(String, String, native::NativeFound)>,
    pub native_cleaned: Vec<(String, String, native::NativeCleaned)>,
    /// Processi attivi: aggiornati a ogni «Analizza» e prima di ogni pulizia.
    pub processes: Vec<String>,
}

impl CleanupState {
    pub fn new() -> Self {
        let n = cleaner::CATEGORIES.len();
        let natives = native::all();
        CleanupState {
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
            rule_sel: rules::native_defaults(&natives),
            rule_found: vec![],
            rule_cleaned: vec![],
            rule_search: String::new(),
            natives,
            native_found: vec![],
            native_cleaned: vec![],
            processes: rules::processes(),
        }
    }

    /// Carica le regole esterne in un thread; finché non arrivano, i gruppi dinamici non ci sono.
    /// Poi, se `update`, aggiorna Winapp2 in sottofondo (nessun `join`:
    /// l'uscita non lo aspetta) e ricarica le regole se è arrivata una copia nuova.
    /// `from_process`: solo nella build di debug le regole usano l'ambiente del processo,
    /// così un profilo finto (USERPROFILE, LOCALAPPDATA…) vale anche per loro.
    pub fn start_rules(&mut self, notifier: SharedNotifier, from_process: bool, update: bool) {
        let (tx, rx) = channel();
        self.rules_rx = Some(rx);
        let update = update.then(rules::real_update);
        let outbox = Outbox::new(tx, notifier, SectionId::Cleanup);
        rules::start(
            xiox_rules::updater::data_dir(),
            rules::own_folders,
            move || if from_process { xiox_rules::Env::from_process() } else { xiox_rules::Env::for_rules() },
            rules::load,
            update,
            move |m| outbox.send(m),
        );
    }

    /// Opzioni dinamiche spuntate (cleaner e opzione clonati per il job).
    pub fn chosen_rules(&self) -> Vec<(xiox_rules::Cleaner, xiox_rules::Opt)> {
        let Some(rs) = &self.rules else { return vec![] };
        rs.cleaners
            .iter()
            .flat_map(|c| c.options.iter().map(move |o| (c, o)))
            .filter(|(c, o)| self.rule_sel.contains(&rules::key(c, o)))
            .map(|(c, o)| (c.clone(), o.clone()))
            .collect()
    }

    pub fn start(&mut self, also_clean: bool, notifier: SharedNotifier) {
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
        let files_cancel = xiox_cleaner::Cancel::default();
        let cancel = Arc::new(AtomicBool::new(false));
        self.clean_cancel = Some((files_cancel.clone(), cancel.clone()));
        let only_old = self.only_old;
        std::thread::spawn(move || {
            let outbox = Outbox::new(tx, notifier, SectionId::Cleanup);
            let verb = if also_clean { "Pulizia" } else { "Analisi" };
            let total = (cats.len() + natives.len() + dynamic.len() + natives_last.len()).max(1) as f32;
            for (k, &i) in cats.iter().enumerate() {
                let name = cleaner::CATEGORIES[i].name;
                if files_cancel.is_cancelled() || cancel.load(Ordering::SeqCst) {
                    // Annullata prima di iniziare: nel resoconto tra i saltati.
                    if also_clean {
                        let c = cleaner::Cleaned { cancelled: true, ..Default::default() };
                        outbox.send(CleanMsg::Cleaned(i, cleaner::Found::default(), c));
                    }
                    continue;
                }
                outbox.send(CleanMsg::Progress(format!("{verb} – {name}"), k as f32 / total));
                let f = cleaner::analyze(i, only_old);
                if also_clean {
                    // L'avanzamento dentro la categoria affina la barra.
                    let tick = |t: xiox_cleaner::Tick| {
                        if t.total_items > 0 {
                            let part = t.done_items as f32 / t.total_items as f32;
                            outbox.send(CleanMsg::Progress(format!("{verb} – {name}"), (k as f32 + part) / total));
                        }
                    };
                    let c = cleaner::clean(i, &f, &tick, &files_cancel);
                    outbox.send(CleanMsg::Cleaned(i, f, c));
                } else {
                    outbox.send(CleanMsg::Analyzed(i, f));
                }
            }
            // Voci native e opzioni dinamiche: in sequenza, un solo Guard per tutta
            // la pulizia; i processi si rileggono prima dell'analisi e della pulizia
            // di ognuna. Regole non ancora caricate: l'ambiente fidato di sempre.
            let env = env.unwrap_or_else(xiox_rules::Env::for_rules);
            let guard = xiox_rules::Guard::new(&env);
            let processes = rules::processes;
            let nctx = native::NativeCtx { env: &env, cancel: &*cancel, guard: &guard };
            let nprogress = |k: usize, text: String| {
                outbox.send(CleanMsg::Progress(text, (cats.len() + k) as f32 / total));
            };
            let previews = |item: &native::NativeItem| previews.get(&rules::native_key(item)).cloned();
            native::run_natives(&natives, also_clean, &nctx, &previews, &processes, &nprogress, &mut |item, f, done| {
                outbox.send(CleanMsg::Native(rules::native_key(item), item.label.to_string(), f, done));
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
                outbox.send(CleanMsg::Progress(text, (cats.len() + natives.len() + k) as f32 / total));
            };
            rules::run_options(&dynamic, also_clean, &run, &progress, &mut |key, label, f, done| {
                outbox.send(CleanMsg::Rule(key, label, f, done));
            });
            let lprogress = |k: usize, text: String| {
                outbox.send(CleanMsg::Progress(text, (cats.len() + natives.len() + dynamic.len() + k) as f32 / total));
            };
            native::run_natives(&natives_last, also_clean, &nctx, &previews, &processes, &lprogress, &mut |item, f, done| {
                outbox.send(CleanMsg::Native(rules::native_key(item), item.label.to_string(), f, done));
            });
            outbox.send(CleanMsg::Done);
        });
    }

    /// Fine dell'analisi o della pulizia (`died`: il thread è morto senza `Done`): dopo una
    /// pulizia, il resoconto di tutte le voci, anche di quelle che non hanno mai risposto.
    pub fn finish(&mut self, died: bool) {
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
    pub fn cancel(&self) {
        if let Some((files, other)) = &self.clean_cancel {
            files.cancel();
            other.store(true, Ordering::SeqCst);
        }
    }

    pub fn cancelling(&self) -> bool {
        self.clean_cancel.as_ref().is_some_and(|(f, a)| f.is_cancelled() || a.load(Ordering::SeqCst))
    }

    /// Un lavoro di pulizia o di analisi è in corso.
    pub fn busy(&self) -> bool {
        self.clean_job.is_some()
    }

    /// Almeno una voce da pulire è scelta (e non bloccata): abilita «Analizza» e «Avvia pulizia».
    pub fn any_selected(&self) -> bool {
        self.clean_sel.iter().any(|&b| b)
            || !rules::active_natives(&self.natives, &self.rule_sel, &self.processes).is_empty()
            || self.rules.as_ref().is_some_and(|rs| !rules::active_choices(&rs.cleaners, &self.rule_sel, &self.processes).is_empty())
    }

    /// "<voce>: <avviso>" per ogni voce nativa e "<cleaner> – <opzione>: <avviso>"
    /// per ogni opzione dinamica spuntata, non bloccata, con un avviso.
    pub fn rule_warnings(&self) -> Vec<String> {
        // Solo le scelte che si pulirebbero davvero (non quelle bloccate).
        let active = rules::active_natives(&self.natives, &self.rule_sel, &self.processes);
        let mut out = rules::native_warnings(&active);
        // Le voci con anteprima usano l'ultima «Analisi» solo se la vista è ancora quella (vedi `start`).
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

    /// Raccoglie ciò che i thread di lavoro hanno mandato (le regole e la pulizia).
    pub fn poll(&mut self) {
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
                self.finish(!done);
            }
        }
    }
}

/// Titolo dell'analisi e, se non è arrivata in fondo, la nota che lo dice.
pub fn analysis_title(busy: bool, end: Option<CleanEnd>) -> (&'static str, Option<&'static str>) {
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
pub fn stop_note(notes: &[String]) -> Option<&String> {
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
pub fn clean_report(
    cleaned: &[Option<cleaner::Cleaned>],
    natives: &[(String, String, native::NativeCleaned)],
    rules: &[(String, String, rules::RuleCleaned)],
    chosen: &Chosen,
    elapsed_ms: u64,
    end: CleanEnd,
) -> xiox_model::CleanReport {
    let mut r = xiox_model::CleanReport { elapsed_ms, cancelled: end != CleanEnd::Done, ..Default::default() };
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
