//! Voci native di Pulizia file: ciò che non è «cancella file o chiavi»
//! (Appunti, cache DNS, registri eventi, collegamenti non validi in
//! `sysclean`; i database dei browser in `browserdb`).
//!
//! Ogni voce ha un `analyze` che guarda e basta e un `clean` che agisce
//! **solo** su `NativeFound::items`, cioè su ciò che l'analisi dello stesso
//! job ha trovato. Nessuna UI qui: `main.rs` mostra le caselle (selezione
//! `native:<id>`, vedi `rules::native_key`) e lancia [`run_natives`] nel
//! thread del job.

use std::sync::atomic::{AtomicBool, Ordering};

use pc_rules::{Env, Guard};

/// Note tenute per una voce nel resoconto.
const MAX_NOTES: usize = 20;

/// Nota di una voce `needs_preview` pulita senza un'analisi prima.
pub const NO_PREVIEW: &str = "Esegui prima Analisi per vedere le voci da togliere";

/// Ciò che serve a una voce nativa: l'ambiente delle regole (cartelle fidate),
/// l'annullamento e il `Guard` unico della pulizia (lo usa chi cancella file).
pub struct NativeCtx<'a> {
    pub env: &'a Env,
    pub cancel: &'a AtomicBool,
    pub guard: &'a Guard,
}

/// Ciò che l'analisi ha trovato.
#[derive(Debug, Default, Clone)]
pub struct NativeFound {
    /// Testo per il resoconto: la prima riga nella tabella, tutte nell'elenco
    /// sotto la tabella.
    pub detail: Vec<String>,
    /// Elementi trovati (canali, collegamenti, 1 per Appunti e DNS).
    pub count: u64,
    /// Quanti di `count` sono file (il «N file» del titolo dell'analisi).
    pub files: u64,
    pub bytes: u64,
    /// Lasciati dall'analisi (protetti, nome non leggibile).
    pub skipped: u64,
    /// Su cosa può agire `clean` (canali, percorsi dei `.lnk`, `"clipboard"`…).
    pub items: Vec<String>,
}

/// Esito della pulizia di una voce.
#[derive(Debug, Default, Clone)]
pub struct NativeCleaned {
    pub freed: u64,
    pub removed: u64,
    pub failed: u64,
    pub notes: Vec<String>,
    /// Cosa si è tolto davvero (canali svuotati, collegamenti cancellati), per il resoconto.
    pub done: Vec<String>,
}

impl NativeCleaned {
    pub fn note(&mut self, text: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(text);
        }
    }
}

/// Una voce nativa. `clean` riceve il `NativeFound` di `analyze` dello stesso
/// job e non tocca altro.
#[derive(Clone, Copy)]
pub struct NativeItem {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub warning: Option<&'static str>,
    /// Testo calcolato al momento della conferma, scritto fra parentesi dopo
    /// `warning` (cookie: «lista: N domini»). Legge al più un file piccolo.
    pub warning_extra: Option<fn() -> String>,
    pub default_on: bool,
    pub needs_admin: bool,
    /// Processi (con o senza `.exe`) che bloccano la voce finché sono attivi.
    pub running: &'static [&'static str],
    /// Con «Pulizia», `clean` riceve il `NativeFound` dell'ultima «Analisi»
    /// mostrata all'utente (non un'analisi nuova, mai vista): senza
    /// anteprima la voce non fa nulla. Per le voci più rischiose (PATH).
    pub needs_preview: bool,
    pub analyze: fn(&NativeCtx) -> NativeFound,
    pub clean: fn(&NativeCtx, &NativeFound) -> NativeCleaned,
}

/// Tutte le voci native, nell'ordine del pannello.
pub fn all() -> Vec<NativeItem> {
    let mut v = crate::sysclean::items();
    v.extend(crate::browserdb::items());
    v.extend(crate::envpath::items());
    v.extend(crate::netcreds::items());
    v.extend(crate::winold::items());
    v.extend(crate::wipe::items());
    v
}

/// Analizza (e con `also_clean` pulisce) le voci scelte, in sequenza, come
/// `rules::run_options`: i processi si rileggono prima dell'analisi e prima
/// della pulizia di ogni voce; una voce bloccata si salta e lo si scrive nel
/// resoconto; `clean` si chiama solo se l'analisi ha trovato qualcosa.
/// Per le voci con `needs_preview`, con «Pulizia» non si rifà l'analisi:
/// `clean` riceve ciò che `previews` dà (l'ultima «Analisi» vista) e, se non
/// c'è, la voce si salta con una nota.
pub fn run_natives(
    chosen: &[NativeItem],
    also_clean: bool,
    ctx: &NativeCtx,
    previews: &dyn Fn(&NativeItem) -> Option<NativeFound>,
    processes: &dyn Fn() -> Vec<String>,
    progress: &dyn Fn(usize, String),
    emit: &mut dyn FnMut(&NativeItem, NativeFound, Option<NativeCleaned>),
) {
    let verb = if also_clean { "Pulizia" } else { "Analisi" };
    for (k, item) in chosen.iter().enumerate() {
        if ctx.cancel.load(Ordering::Relaxed) {
            return;
        }
        progress(k, format!("{verb} – {}", item.label));
        if let Some(why) = crate::rules::native_blocked(item, &processes()) {
            let mut skipped = NativeCleaned::default();
            skipped.note(format!("Saltata: {why}"));
            emit(item, NativeFound::default(), also_clean.then_some(skipped));
            continue;
        }
        if also_clean && item.needs_preview {
            let Some(found) = previews(item) else {
                let mut skipped = NativeCleaned::default();
                skipped.note(NO_PREVIEW.to_string());
                emit(item, NativeFound::default(), Some(skipped));
                continue;
            };
            let done = if found.items.is_empty() {
                NativeCleaned::default()
            } else if ctx.cancel.load(Ordering::Relaxed) {
                let mut stopped = NativeCleaned::default();
                stopped.note("Pulizia interrotta".to_string());
                stopped
            } else {
                (item.clean)(ctx, &found)
            };
            emit(item, found, Some(done));
            continue;
        }
        let found = (item.analyze)(ctx);
        let done = also_clean.then(|| {
            if let Some(why) = crate::rules::native_blocked(item, &processes()) {
                let mut skipped = NativeCleaned::default();
                skipped.note(format!("Saltata: {why}"));
                return skipped;
            }
            if found.items.is_empty() {
                return NativeCleaned::default();
            }
            if ctx.cancel.load(Ordering::Relaxed) {
                let mut stopped = NativeCleaned::default();
                stopped.note("Pulizia interrotta".to_string());
                return stopped;
            }
            (item.clean)(ctx, &found)
        });
        emit(item, found, done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static SEEN: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
    }

    fn found_two(_: &NativeCtx) -> NativeFound {
        NativeFound { count: 2, items: vec!["a".into(), "b".into()], ..Default::default() }
    }

    fn found_none(_: &NativeCtx) -> NativeFound {
        NativeFound::default()
    }

    fn record(_: &NativeCtx, f: &NativeFound) -> NativeCleaned {
        SEEN.with(|s| s.borrow_mut().push(f.items.clone()));
        NativeCleaned { removed: f.items.len() as u64, ..Default::default() }
    }

    fn item(id: &'static str, running: &'static [&'static str], analyze: fn(&NativeCtx) -> NativeFound) -> NativeItem {
        NativeItem {
            id,
            group: "Windows",
            label: "Voce",
            description: "",
            warning: None,
            warning_extra: None,
            default_on: false,
            needs_admin: false,
            running,
            needs_preview: false,
            analyze,
            clean: record,
        }
    }

    fn run(chosen: &[NativeItem], also_clean: bool, procs: &[&str]) -> Vec<(String, NativeFound, Option<NativeCleaned>)> {
        run_with(chosen, also_clean, procs, None)
    }

    fn run_with(
        chosen: &[NativeItem],
        also_clean: bool,
        procs: &[&str],
        preview: Option<NativeFound>,
    ) -> Vec<(String, NativeFound, Option<NativeCleaned>)> {
        let env = Env::default();
        let guard = Guard::new(&env);
        let cancel = AtomicBool::new(false);
        let ctx = NativeCtx { env: &env, cancel: &cancel, guard: &guard };
        let procs: Vec<String> = procs.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        run_natives(chosen, also_clean, &ctx, &|_| preview.clone(), &|| procs.clone(), &|_, _| {}, &mut |i, f, c| out.push((i.id.to_string(), f, c)));
        out
    }

    #[test]
    fn clean_gets_exactly_what_the_same_job_found() {
        SEEN.with(|s| s.borrow_mut().clear());
        let out = run(&[item("x", &[], found_two)], true, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2.as_ref().unwrap().removed, 2);
        SEEN.with(|s| assert_eq!(*s.borrow(), vec![vec!["a".to_string(), "b".to_string()]]));
    }

    #[test]
    fn analysis_only_never_cleans_and_empty_findings_are_not_cleaned() {
        SEEN.with(|s| s.borrow_mut().clear());
        let out = run(&[item("x", &[], found_two)], false, &[]);
        assert!(out[0].2.is_none());
        let out = run(&[item("y", &[], found_none)], true, &[]);
        assert_eq!(out[0].2.as_ref().unwrap().removed, 0);
        SEEN.with(|s| assert!(s.borrow().is_empty()));
    }

    #[test]
    fn blocked_item_is_skipped_with_a_note() {
        SEEN.with(|s| s.borrow_mut().clear());
        let out = run(&[item("x", &["Safari.exe"], found_two)], true, &["safari.exe"]);
        assert_eq!(out[0].1.count, 0);
        let c = out[0].2.as_ref().unwrap();
        assert_eq!(c.removed, 0);
        assert!(c.notes[0].contains("Chiudi Safari.exe per pulire"), "{:?}", c.notes);
        SEEN.with(|s| assert!(s.borrow().is_empty()));
    }

    fn never_analyzed(_: &NativeCtx) -> NativeFound {
        panic!("una voce needs_preview non si analizza di nuovo con Pulizia")
    }

    #[test]
    fn preview_item_without_an_analysis_does_nothing() {
        SEEN.with(|s| s.borrow_mut().clear());
        let it = NativeItem { needs_preview: true, ..item("p", &[], never_analyzed) };
        let out = run_with(&[it], true, &[], None);
        let c = out[0].2.as_ref().unwrap();
        assert_eq!(c.removed, 0);
        assert_eq!(c.notes, vec![NO_PREVIEW.to_string()]);
        SEEN.with(|s| assert!(s.borrow().is_empty()));
    }

    #[test]
    fn preview_item_cleans_exactly_the_previewed_findings() {
        SEEN.with(|s| s.borrow_mut().clear());
        let it = NativeItem { needs_preview: true, ..item("p", &[], never_analyzed) };
        let seen = NativeFound { count: 1, items: vec!["visto".into()], ..Default::default() };
        let out = run_with(&[it], true, &[], Some(seen));
        assert_eq!(out[0].1.items, vec!["visto".to_string()]);
        assert_eq!(out[0].2.as_ref().unwrap().removed, 1);
        SEEN.with(|s| assert_eq!(*s.borrow(), vec![vec!["visto".to_string()]]));
    }
}
