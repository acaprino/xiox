//! The Cleanup section without a window: choices, the jobs in a thread, analysis, cleaning, the
//! report and the dynamic options. The threads wake whoever shows the section with a
//! [`SharedNotifier`].
//!
//! On top of the state sit the page's intents (`toggle`, `select_all`, `analyze`,
//! `request_clean`, `confirm`...): each refuses, with a sentence in English, whatever the page
//! must not do now, and a refusal changes nothing. The real work runs behind [`CleanupWorkers`],
//! so the tests go from «Continue» to the start without touching the engine.

use crate::notify::{Outbox, SectionId, SharedNotifier};
use crate::{browserdb, cleaner, native, rules, sys};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod intent_tests;
#[cfg(test)]
mod script_tests;
#[cfg(test)]
mod tests;
mod view;

// Why an intent is refused (English: the page shows them as they are). The first four are
// Developers' own, so both sections say the same thing.
pub use crate::dev::{BUSY, DIALOG_OPEN, NOTHING_TO_CANCEL, NO_DIALOG};
pub const NOTHING: &str = "Select something to analyze or clean first.";
/// «Clean» works on the analysis on screen: after a report, or with none, there is nothing to clean.
pub const NO_ANALYSIS: &str = "Analyze first: Clean works on the analysis on screen.";
pub const REPORT_OPEN: &str = "Close the report first.";
pub const NO_KEEP_LIST: &str = "There is no cookie keep list in this build.";
/// A category box or «select all» that has nothing to tick: whatever is in reach is blocked or at the
/// Caution level, and those are chosen one by one.
pub const NOTHING_TO_TICK: &str = "Nothing here can be ticked in one go: blocked and caution items are chosen one by one.";
/// The box of a category the search hides.
pub const CATEGORY_HIDDEN: &str = "That category is hidden by the search.";

/// Prefix of the nav ids: a group of the built-in categories, a group of the rules.
const BUILTIN: &str = "builtin:";
const RULES: &str = "rules:";
/// The nav id of the Winapp2 entries.
pub const WINAPP2: &str = "winapp2";

/// A category of the list: what the page's menu names and its box ticks.
pub(crate) struct Group {
    /// `builtin:<group>`, `rules:<group>` or `winapp2`: the id of its `NavItem` and of its `Group` row.
    pub id: String,
    pub name: String,
    pub members: Members,
}

/// What a category holds.
pub(crate) enum Members {
    /// Indexes into `cleaner::CATEGORIES`.
    Builtin(Vec<usize>),
    /// The native items (indexes into `natives`), then the cleaners that came with the program
    /// (indexes into the rule set's cleaners).
    Rules(Vec<usize>, Vec<usize>),
    /// The sections of Winapp2.
    Winapp2,
}

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

/// What a cleanup stopped by an internal error says (the same as in Developers).
pub const CLEAN_DIED: &str = "Cleanup stopped by an internal error: some items may already be gone. Press “Analyze” to see what is left.";

/// Le voci scelte all'avvio: le categorie e (chiave, etichetta) delle voci native e delle
/// opzioni dinamiche, per elencare nel resoconto quelle che non hanno mai risposto.
#[derive(Default, Clone)]
pub struct Chosen {
    pub cats: Vec<usize>,
    pub others: Vec<(String, String)>,
}

#[derive(PartialEq, Debug)]
pub enum CleanView {
    Empty,
    Analyzed,
    Cleaned,
}

/// Lo stato di Pulizia file: tutto ciò che la sezione sa e i lavori in corso.
pub struct CleanupState {
    pub(crate) clean_sel: Vec<bool>,
    only_old: bool,
    pub(crate) clean_job: Option<Receiver<CleanMsg>>,
    clean_text: String,
    clean_progress: f32,
    found: Vec<Option<cleaner::Found>>,
    cleaned: Vec<Option<cleaner::Cleaned>>,
    pub(crate) clean_view: CleanView,
    /// «Annulla» della pulizia o dell'analisi in corso: lo stesso clic ferma le categorie
    /// (`xiox_cleaner::Cancel`) e le voci native e dinamiche (`AtomicBool`).
    clean_cancel: Option<(xiox_cleaner::Cancel, Arc<AtomicBool>)>,
    clean_started: Instant,
    /// Resoconto della pulizia, costruito quando tutte le voci hanno finito.
    clean_report: Option<xiox_model::CleanReport>,
    /// Voci scelte all'avvio dell'ultima analisi o pulizia.
    clean_chosen: Chosen,
    /// Come è finita l'ultima analisi o pulizia (`None` = in corso o mai partita).
    pub(crate) clean_end: Option<CleanEnd>,
    /// Esito di «Salva come testo…»: (testo, riuscito).
    clean_saved: Option<(String, bool)>,
    pub(crate) confirm_clean: bool,
    /// Avvisi della finestra di conferma, calcolati una volta all'apertura
    /// (la lista dei cookie si legge lì, non a ogni fotogramma).
    confirm_warnings: Vec<String>,

    pub(crate) rules: Option<rules::RuleSet>,
    /// Aperto finché il thread delle regole (caricamento e aggiornamento di Winapp2) vive.
    rules_rx: Option<Receiver<rules::RulesMsg>>,
    /// Esito dell'aggiornamento di Winapp2 di questo avvio (`None` = in corso o non partito).
    pub winapp2_update: Option<xiox_rules::updater::UpdateOutcome>,
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
    /// Il lavoro del thread di analisi e pulizia (quello vero per default).
    workers: CleanupWorkers,
}

/// Tutto ciò che il thread di analisi e pulizia riceve: la scelta fatta all'avvio e i segnali di
/// «Annulla». Il thread non legge più lo stato.
pub struct CleanJob {
    pub cats: Vec<usize>,
    pub natives: Vec<native::NativeItem>,
    pub natives_last: Vec<native::NativeItem>,
    pub dynamic: Vec<(xiox_rules::Cleaner, xiox_rules::Opt)>,
    pub env: Option<xiox_rules::Env>,
    pub previews: HashMap<String, native::NativeFound>,
    pub only_old: bool,
    pub also_clean: bool,
    pub files_cancel: xiox_cleaner::Cancel,
    pub cancel: Arc<AtomicBool>,
}

/// Il lavoro di `CleanupState` come puntatore a funzione: quello vero per default, una funzione
/// finta nei test (così `confirm` arriva fino all'avvio senza toccare il motore).
#[derive(Clone, Copy)]
pub struct CleanupWorkers {
    pub job: fn(CleanJob, Outbox<CleanMsg>),
}

impl CleanupWorkers {
    pub const REAL: CleanupWorkers = CleanupWorkers { job: real_job };
}

impl Default for CleanupState {
    fn default() -> Self {
        Self::new()
    }
}

impl CleanupState {
    pub fn new() -> Self {
        Self::with(native::all(), rules::processes(), CleanupWorkers::REAL)
    }

    /// Lo stato con altre voci native, altri processi attivi e un altro lavoro (i test).
    pub fn with(natives: Vec<native::NativeItem>, processes: Vec<String>, workers: CleanupWorkers) -> Self {
        let n = cleaner::CATEGORIES.len();
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
            processes,
            workers,
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
        // «Clean free space» gira dopo tutto il resto (anche le opzioni
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
        // L'avanzamento riparte da zero: la barra non mostra quello del lavoro di prima.
        self.clean_progress = 0.0;
        self.clean_text.clear();
        let job = CleanJob { cats, natives, natives_last, dynamic, env, previews, only_old, also_clean, files_cancel, cancel };
        let work = self.workers.job;
        std::thread::spawn(move || work(job, Outbox::new(tx, notifier, SectionId::Cleanup)));
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
                    Ok(rules::RulesMsg::Loaded(set)) => {
                        self.rules = Some(set);
                        // The warnings of an open confirmation describe the rules it was opened
                        // for: new ones close it, and «Clean» opens it again with theirs.
                        self.confirm_clean = false;
                    }
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

/// What the page asks of Cleanup (`CleanupIntent`). Each intent refuses, with a reason in English
/// (`Err`), whatever the page must not do now: the caller (the Tauri command) turns
/// it into a `CmdError { kind: "refused" }`. A refused intent changes nothing.
impl CleanupState {
    /// A job runs: nothing may change under it.
    fn idle(&self) -> Result<(), String> {
        if self.busy() {
            return Err(BUSY.into());
        }
        Ok(())
    }

    /// No job and no confirmation, whose warnings describe the choice.
    fn unlocked(&self) -> Result<(), String> {
        self.idle()?;
        if self.confirm_clean {
            return Err(DIALOG_OPEN.into());
        }
        Ok(())
    }

    /// Choices and settings: unlocked, and no report on screen (it is closed first, or replaced
    /// by a new analysis).
    fn editable(&self) -> Result<(), String> {
        self.unlocked()?;
        if self.clean_view == CleanView::Cleaned {
            return Err(REPORT_OPEN.into());
        }
        Ok(())
    }

    /// The categories of the list, in the order the page shows them (and its menu lists them): a
    /// group of the built-in categories (`builtin:<group>`), a group of the rules (`rules:<group>`: the
    /// natives first, then the cleaners that came with the program), and Winapp2 when it has sections.
    pub(crate) fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for (i, c) in cleaner::CATEGORIES.iter().enumerate() {
            let id = format!("{BUILTIN}{}", c.group);
            match groups.iter_mut().find(|g| g.id == id) {
                Some(Group { members: Members::Builtin(idx), .. }) => idx.push(i),
                _ => groups.push(Group { id, name: c.group.to_string(), members: Members::Builtin(vec![i]) }),
            }
        }
        groups.extend(
            self.rule_groups().into_iter().map(|(g, nat, idx)| Group { id: format!("{RULES}{g}"), name: g, members: Members::Rules(nat, idx) }),
        );
        if self.rules.as_ref().is_some_and(|rs| !rs.sections.is_empty()) {
            groups.push(Group { id: WINAPP2.into(), name: rules::WINAPP2_GROUP.into(), members: Members::Winapp2 });
        }
        groups
    }

    /// The groups of the rules panel: (group, natives, cleaners) as `rules::panel_groups` has them.
    pub fn rule_groups(&self) -> Vec<(String, Vec<usize>, Vec<usize>)> {
        rules::panel_groups(self.rules.as_ref().map_or(&[][..], |rs| &rs.groups[..]), &self.natives)
    }

    /// A row id of the contract: `cat:<index>` a built-in category, `native:<id>` a native item,
    /// `<cleaner>/<option>` a rule, the id of a category of the list (`builtin:<group>`,
    /// `rules:<group>`, `winapp2`: its box) or of a section of Winapp2 (`section:<name>`: its box).
    /// Anything unknown is not choosable, and neither is a row the list shows disabled (an
    /// unsupported option, a program that runs).
    pub fn toggle(&mut self, row: &str) -> Result<(), String> {
        self.editable()?;
        if self.groups().iter().any(|g| g.id == row) {
            return self.toggle_group(row);
        }
        if row.starts_with(view::SECTION_ROW) {
            return self.toggle_section(row);
        }
        let unknown = || format!("“{row}” is not a row that can be chosen.");
        if let Some(n) = row.strip_prefix("cat:") {
            let i = n.parse::<usize>().ok().filter(|i| i.to_string() == n && *i < self.clean_sel.len()).ok_or_else(unknown)?;
            self.clean_sel[i] = !self.clean_sel[i];
            return Ok(());
        }
        let blocked = if row.starts_with(rules::NATIVE_PREFIX) {
            let item = self.natives.iter().find(|i| rules::native_key(i) == row).ok_or_else(unknown)?;
            rules::native_blocked(item, &self.processes)
        } else {
            let rs = self.rules.as_ref().ok_or_else(unknown)?;
            let (c, o) = rs
                .cleaners
                .iter()
                .flat_map(|c| c.options.iter().map(move |o| (c, o)))
                .find(|(c, o)| rules::key(c, o) == row)
                .ok_or_else(unknown)?;
            rules::blocked(c, o, &self.processes)
        };
        if let Some(why) = blocked {
            return Err(why);
        }
        if !self.rule_sel.remove(row) {
            self.rule_sel.insert(row.to_string());
        }
        Ok(())
    }

    /// The box of a category: when every row it can tick is ticked they all go off, otherwise they all
    /// go on. It acts on what the list shows (the search may hide some rows) and never on a row that
    /// is blocked or at the Caution level.
    fn toggle_group(&mut self, id: &str) -> Result<(), String> {
        let shown = self.shown_groups(false);
        let group = shown.iter().find(|g| g.id == id).ok_or_else(|| CATEGORY_HIDDEN.to_string())?;
        let ticks: Vec<&view::Entry> = group.entries.iter().filter(|e| view::eligible(&e.row)).collect();
        if ticks.is_empty() {
            return Err(NOTHING_TO_TICK.into());
        }
        let on = !ticks.iter().all(|e| e.row.check == crate::view::Check::On);
        let ids: Vec<String> = ticks.iter().map(|e| e.row.id.clone()).collect();
        for row in &ids {
            self.set_chosen(row, on);
        }
        Ok(())
    }

    /// The box of a section of Winapp2: the same rule as a category's box, over the section's rows the list shows.
    fn toggle_section(&mut self, id: &str) -> Result<(), String> {
        let rows = self.list_rows(self.shown_groups(false), false);
        let (ids, on) = crate::view::header_toggle(&rows, id).ok_or_else(|| CATEGORY_HIDDEN.to_string())?;
        if ids.is_empty() {
            return Err(NOTHING_TO_TICK.into());
        }
        for row in &ids {
            self.set_chosen(row, on);
        }
        Ok(())
    }

    /// «Select all» / «None» of the list: every row it shows that can be ticked in one go, on or off.
    pub fn select_all(&mut self, on: bool) -> Result<(), String> {
        self.editable()?;
        let ids: Vec<String> =
            self.shown_groups(false).into_iter().flat_map(|g| g.entries).filter(|e| view::eligible(&e.row)).map(|e| e.row.id).collect();
        if ids.is_empty() {
            return Err(NOTHING_TO_TICK.into());
        }
        for row in &ids {
            self.set_chosen(row, on);
        }
        Ok(())
    }

    /// Sets the choice of a row of the list (`cat:<index>`, a native or an option key) without the
    /// checks of `toggle`: the ids come from rows the list shows ticked-able.
    fn set_chosen(&mut self, row: &str, on: bool) {
        if let Some(i) = row.strip_prefix("cat:").and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < self.clean_sel.len()) {
            self.clean_sel[i] = on;
        } else if on {
            self.rule_sel.insert(row.to_string());
        } else {
            self.rule_sel.remove(row);
        }
    }

    pub fn set_only_old(&mut self, on: bool) -> Result<(), String> {
        self.editable()?;
        self.only_old = on;
        Ok(())
    }

    /// The search box: it filters every category of the list and only changes what is shown, so it is
    /// never refused.
    pub fn set_search(&mut self, text: &str) {
        self.rule_search = text.to_string();
    }

    /// «Analyze»: what is chosen, analysed, nothing deleted. A report on screen is replaced.
    pub fn analyze(&mut self, notifier: SharedNotifier) -> Result<(), String> {
        self.unlocked()?;
        if !self.any_selected() {
            return Err(NOTHING.into());
        }
        self.start(false, notifier);
        Ok(())
    }

    /// «Clean N items»: opens the confirmation, with the warnings of what is chosen, for the
    /// analysis on screen. With none (nothing analysed yet, or a report is showing) there is nothing
    /// to clean from: that is the contract of the page, stricter than the old button (which cleaned
    /// without an analysis); the PATH preview needs an analysis on screen anyway.
    pub fn request_clean(&mut self) -> Result<(), String> {
        self.unlocked()?;
        if self.clean_view != CleanView::Analyzed {
            return Err(NO_ANALYSIS.into());
        }
        if !self.any_selected() {
            return Err(NOTHING.into());
        }
        self.confirm_warnings = self.rule_warnings();
        self.confirm_clean = true;
        Ok(())
    }

    /// «Continue» in the confirmation: closes it and cleans. Only from the open dialog.
    pub fn confirm(&mut self, notifier: SharedNotifier) -> Result<(), String> {
        self.idle()?;
        if !self.confirm_clean {
            return Err(NO_DIALOG.into());
        }
        self.confirm_clean = false;
        if !self.any_selected() {
            return Err(NOTHING.into());
        }
        self.start(true, notifier);
        Ok(())
    }

    /// «Cancel» in the confirmation (and Esc): nothing runs.
    pub fn close_confirm(&mut self) {
        self.confirm_clean = false;
    }

    /// «Cancel» during an analysis or a cleaning: the workers stop before the next item. A
    /// cleaning then ends with a report of what was done (`cancelled`); an analysis with what it had.
    pub fn cancel_job(&mut self) -> Result<(), String> {
        if !self.busy() || self.clean_cancel.is_none() {
            return Err(NOTHING_TO_CANCEL.into());
        }
        self.cancel();
        Ok(())
    }

    /// Closing a report (the page's «Close» over it): the analysis and the report go, the choices
    /// stay. Nothing happens while a job runs.
    pub fn release(&mut self) {
        if self.busy() {
            return;
        }
        let n = self.clean_sel.len();
        self.found = vec![None; n];
        self.cleaned = vec![None; n];
        self.rule_found.clear();
        self.rule_cleaned.clear();
        self.native_found.clear();
        self.native_cleaned.clear();
        self.clean_view = CleanView::Empty;
        self.clean_report = None;
        self.clean_saved = None;
        self.clean_chosen = Chosen::default();
        self.clean_end = None;
        self.confirm_clean = false;
    }

    /// Opens the cookie keep list in the editor (the real one starts it: call it off the UI thread).
    /// Refused when this list has no cookie item, as the page has no button for it then.
    pub fn open_keep_list(&self) -> Result<(), String> {
        if !self.natives.iter().any(|i| i.id == browserdb::COOKIES_ID) {
            return Err(NO_KEEP_LIST.into());
        }
        browserdb::open_keep_list().map_err(|e| format!("Could not open the cookie keep list: {e}"))
    }

    /// The text of «Save as text…»: the report, once the cleaning is over.
    pub fn report_text(&self) -> Option<String> {
        if self.busy() {
            return None;
        }
        self.clean_report.as_ref().map(|r| r.to_text())
    }

    /// The outcome of «Save as text…» (the dialog is Rust's): what the report shows.
    pub fn set_saved(&mut self, text: String, ok: bool) {
        self.clean_saved = Some((text, ok));
    }
}

/// The Winapp2 filter of the rules panel: the sections (with the indexes of their cleaners in
/// `rs.cleaners`) that match `needle` (trimmed, any case). A section whose name matches keeps all its
/// cleaners; otherwise a cleaner stays when its label or one of its options' labels matches. A
/// cleaner that stays shows all its options. Sections left empty are dropped.
pub fn winapp2_matches(rs: &rules::RuleSet, needle: &str) -> Vec<(String, Vec<usize>)> {
    let needle = needle.trim().to_lowercase();
    let hit = |text: &str| text.to_lowercase().contains(&needle);
    rs.sections
        .iter()
        .filter_map(|(section, idx)| {
            let whole = needle.is_empty() || hit(section);
            let kept: Vec<usize> = idx
                .iter()
                .copied()
                .filter(|&i| {
                    let c = &rs.cleaners[i];
                    whole || hit(&c.label) || c.options.iter().any(|o| hit(&o.label))
                })
                .collect();
            (!kept.is_empty()).then(|| (section.clone(), kept))
        })
        .collect()
}

/// Il thread di analisi e pulizia (il lavoro vero di [`CleanupWorkers::REAL`]): le categorie, poi le
/// voci native, le opzioni dinamiche e per ultime le voci che girano dopo tutto il resto. `outbox`
/// avvisa la UI a ogni messaggio e, se il thread muore, alla fine.
fn real_job(job: CleanJob, outbox: Outbox<CleanMsg>) {
    let CleanJob { cats, natives, natives_last, dynamic, env, previews, only_old, also_clean, files_cancel, cancel } = job;
    let verb = if also_clean { "Cleaning" } else { "Analyzing" };
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
    let nctx = native::NativeCtx { env: &env, cancel: &cancel, guard: &guard };
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
        cancel: &cancel,
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
}

/// Titolo dell'analisi e, se non è arrivata in fondo, la nota che lo dice.
pub fn analysis_title(busy: bool, end: Option<CleanEnd>) -> (&'static str, Option<&'static str>) {
    match (busy, end) {
        (true, _) => ("Analysis in progress…", None),
        (false, Some(CleanEnd::Cancelled)) => {
            ("Analysis cancelled", Some("Analysis cancelled: the missing items were not analyzed."))
        }
        (false, Some(CleanEnd::Died)) => (
            "Analysis interrupted",
            Some("An internal error stopped the analysis: the missing items were not analyzed."),
        ),
        (false, _) => ("Analysis done", None),
    }
}

/// La nota di una voce nativa o dinamica che dice che non è stata pulita del tutto
/// (saltata, interrotta, non supportata), se c'è.
pub fn stop_note(notes: &[String]) -> Option<&String> {
    notes.iter().find(|n| n.starts_with("Skipped") || n.contains("interrupted") || n.starts_with("Option not supported"))
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
        r.failed.push((PathBuf::from("Cleanup"), CLEAN_DIED.to_string()));
    }
    let never = if end == CleanEnd::Died { "Not run: cleanup stopped by an internal error" } else { "Cancelled" };
    let counted = |n: u64, notes: &[String]| match notes.first() {
        Some(first) => format!("{} left: {first}", sys::plural(n, "item", "items")),
        None => format!("{} left (in use or refused)", sys::plural(n, "item", "items")),
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
                "Cancelled".to_string()
            } else {
                format!("Cancelled: {} freed", sys::fmt_size(c.freed))
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
            let why = if freed > 0 { format!("{why}: {} freed", sys::fmt_size(freed)) } else { why.clone() };
            r.skipped.push((PathBuf::from(label), why));
        }
    }
    for (key, label) in &chosen.others {
        let reported = natives.iter().any(|(k, _, _)| k == key) || rules.iter().any(|(k, _, _)| k == key);
        if !reported {
            r.skipped.push((PathBuf::from(label), never.to_string()));
        }
    }
    r.by_category.sort_by_key(|a| std::cmp::Reverse(a.1));
    r
}
