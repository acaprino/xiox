//! Categorie dinamiche di Pulizia file: i cleaner inglobati (CleanerML,
//! gruppi "Windows", "Browser" e "Sviluppo e AI", vedi
//! `xiox_rules::embedded`) e le voci di Winapp2 ("Applicazioni").
//!
//! Nessuna UI qui: la finestra mostra le caselle e lancia il job; questo modulo
//! carica le regole, raccoglie ciò che un'opzione toglierebbe e lo cancella
//! seguendo il contratto di `xiox_rules` (vedi `crates/xiox-rules/src/lib.rs`):
//! file e cartelle solo con `delete_checked_with` e un solo `Guard` per
//! pulizia; registro ricontrollato con `reg_allowed`, backup `.reg` prima,
//! chiavi aperte senza seguire i collegamenti.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use xiox_rules::model::PROTECT_TREE_PREFIX;
use xiox_rules::updater::UpdateOutcome;
use xiox_rules::{
    collect_ctl, present, within, reg_allowed, running, updater, winapp2, Action, Cleaner,
    CollectCtl, Env, Guard, Opt, Origin, RegItem, RegRoot,
};

use crate::native::NativeItem;
use crate::{regbackup, sys};

/// Titolo del gruppo dei cleaner inglobati.
pub const EMBEDDED_GROUP: &str = "Development & AI";
/// Titolo del gruppo delle voci di Winapp2.
pub const WINAPP2_GROUP: &str = "Applications";
/// Ordine dei gruppi dei cleaner inglobati nel pannello; un gruppo fuori
/// elenco va dopo questi, in ordine alfabetico. "Applicazioni" (Winapp2) viene
/// sempre per ultimo.
pub const GROUP_ORDER: [&str; 3] = ["Windows", "Browser", EMBEDDED_GROUP];
/// Attribuzione delle regole nella finestra Informazioni.
pub const ABOUT_RULES: &str = "The cleaning rules for applications come from Winapp2 \
     (https://github.com/MoscaDotTo/Winapp2), licence CC-BY-SA-4.0, downloaded at every start.";

/// Note tenute per un'opzione nel resoconto.
const MAX_NOTES: usize = 20;

/// Regole pronte per la scheda.
pub struct RuleSet {
    /// Prima gli inglobati presenti, poi le voci di Winapp2 rilevate.
    pub cleaners: Vec<Cleaner>,
    /// Versione della copia di Winapp2 caricata (finestra Informazioni, riga di stato).
    pub winapp2_version: Option<String>,
    /// Riga di stato del caricamento; con un esito dell'aggiornamento vale [`status_line`].
    pub status: String,
    pub warnings: Vec<String>,
    /// Ambiente con cui si sono lette le regole: lo stesso per analisi e pulizia.
    pub env: Env,
    /// Sezioni di Winapp2 (`Cleaner::group`, in ordine alfabetico) con gli
    /// indici delle voci in `cleaners`.
    pub sections: Vec<(String, Vec<usize>)>,
    /// Gruppi dei cleaner inglobati presenti (`Cleaner::group`, nell'ordine di
    /// [`GROUP_ORDER`]) con gli indici in `cleaners`.
    pub groups: Vec<(String, Vec<usize>)>,
}

pub enum RulesMsg {
    /// Regole caricate (all'avvio, e di nuovo dopo un aggiornamento riuscito).
    Loaded(RuleSet),
    /// Esito dell'aggiornamento di Winapp2 (uno per avvio).
    Updated(UpdateOutcome),
}

/// Ciò che un'opzione toglierebbe.
#[derive(Debug, Clone, Default)]
pub struct RuleFound {
    pub files: Vec<(PathBuf, u64)>,
    pub dirs_to_remove: Vec<PathBuf>,
    pub reg: Vec<RegItem>,
    pub bytes: u64,
    /// File più chiavi e valori di registro.
    pub count: u64,
    pub skipped: u64,
    pub skipped_reasons: Vec<String>,
    pub cancelled: bool,
}

/// Esito della pulizia di un'opzione.
#[derive(Debug, Clone, Default)]
pub struct RuleCleaned {
    pub freed: u64,
    /// File e voci di registro tolti.
    pub removed: u64,
    /// Di `removed`, quanti erano file (il resoconto conta solo questi tra i file eliminati).
    pub files: u64,
    /// File e voci di registro rimasti (in uso, rifiutati, backup non riuscito).
    pub failed: u64,
    pub backup: Option<PathBuf>,
    /// Note per il resoconto (opzione saltata, backup non riuscito…).
    pub notes: Vec<String>,
}

impl RuleCleaned {
    fn note(&mut self, text: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(text);
        }
    }
}

/// Registro usato dalla pulizia, sostituibile nei test.
pub trait RuleRegistry: Sync {
    /// Scrive un `.reg` con le chiavi esistenti di `items` (nessun file se non
    /// ne esiste nessuna) e dice quali voci esistevano. `Err` se anche una sola
    /// chiave esistente non si salva o non si sa se esista.
    fn backup(&self, items: &[RegItem]) -> Result<Backup, String>;
    /// Cancella una chiave (con le sottochiavi) o un valore: `Ok(true)` se
    /// eliminata, `Ok(false)` se non c'era più.
    fn delete(&self, item: &RegItem) -> Result<bool, String>;
}

/// Chiave di selezione e di risultato: `"<cleaner.id>/<opt.id>"`.
pub fn key(cleaner: &Cleaner, opt: &Opt) -> String {
    format!("{}/{}", cleaner.id, opt.id)
}

/// Lettura della copia di Winapp2: `Ok((testo, versione))` (vedi `updater::read_valid`).
pub type ReadWinapp2 = dyn Fn(&Path) -> Result<(String, String), String>;

/// Carica le regole dalla cartella dati (`updater::data_dir()`), con la copia
/// di Winapp2 letta da `updater::read_valid` e il registro vero (sola lettura).
pub fn load(dir: &Path, env: &Env) -> RuleSet {
    load_with(dir, env, &|d| updater::read_valid(d), &reg_exists)
}

/// Come [`load`], con lettura di Winapp2 e registro iniettati.
pub fn load_with(
    dir: &Path,
    env: &Env,
    read_winapp2: &ReadWinapp2,
    reg_exists: &dyn Fn(&RegItem) -> bool,
) -> RuleSet {
    let (all, mut warnings) = xiox_rules::embedded(env);
    let mut cleaners: Vec<Cleaner> = all.into_iter().filter(|c| present(c, env, reg_exists)).collect();
    let mut winapp: Vec<Cleaner> = Vec::new();
    let (status, version) = match read_winapp2(dir) {
        Ok((text, version)) => {
            let (w, parse_warnings) = winapp2::parse(&text);
            drop(text);
            warnings.extend(parse_warnings);
            let detect = |root: RegRoot, path: &str| {
                reg_exists(&RegItem::Key {
                    root,
                    path: path.to_string(),
                })
            };
            let (found, report) = winapp2::to_cleaners_with(&w, env, &detect);
            drop(w);
            let dropped = report.discarded_protected + report.discarded_missing_var;
            if dropped > 0 {
                warnings.push(format!(
                    "Winapp2: {dropped} rules discarded (protected paths or undefined variables)"
                ));
            }
            if report.skipped_excludes > 0 {
                warnings.push(format!(
                    "Winapp2: {} items not offered (unsupported exclusions)",
                    report.skipped_excludes
                ));
            }
            // Regola 9: vince l'inglobato presente (vedi `covered_by_embedded`).
            let before = found.len();
            winapp = found
                .into_iter()
                .filter(|w| !covered_by_embedded(w, &cleaners, env))
                .collect();
            let hidden = before - winapp.len();
            if hidden > 0 {
                let what = if hidden == 1 { "1 item".to_string() } else { format!("{hidden} items") };
                warnings.push(format!("Winapp2: {what} hidden because already covered by {EMBEDDED_GROUP}"));
            }
            (status_text(Winapp2Status::Loaded(&version)), Some(version))
        }
        // `read_valid` lo dice così quando il file non c'è ancora.
        Err(e) if e == "no local copy" => ("Winapp2 not downloaded yet".to_string(), None),
        Err(e) => (format!("Winapp2 not read: {}", &e), None),
    };
    let first = cleaners.len();
    let groups = embedded_groups(&cleaners);
    cleaners.extend(winapp);
    let mut sections: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, c) in cleaners.iter().enumerate().skip(first) {
        match sections.iter_mut().find(|(g, _)| *g == c.group) {
            Some((_, v)) => v.push(i),
            None => sections.push((c.group.clone(), vec![i])),
        }
    }
    sections.sort_by_key(|(g, _)| g.to_lowercase());
    RuleSet {
        cleaners,
        winapp2_version: version,
        status,
        warnings,
        env: env.clone(),
        sections,
        groups,
    }
}

/// Raggruppa i cleaner per `Cleaner::group` nell'ordine di [`GROUP_ORDER`]
/// (gli altri gruppi dopo, in ordine alfabetico); dentro un gruppo resta
/// l'ordine di `cleaners`.
pub fn embedded_groups(cleaners: &[Cleaner]) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, c) in cleaners.iter().enumerate() {
        match groups.iter_mut().find(|(g, _)| *g == c.group) {
            Some((_, v)) => v.push(i),
            None => groups.push((c.group.clone(), vec![i])),
        }
    }
    let rank = |g: &str| GROUP_ORDER.iter().position(|o| *o == g).unwrap_or(GROUP_ORDER.len());
    groups.sort_by_key(|(g, _)| (rank(g), g.to_lowercase()));
    groups
}

// ---------------------------------------------------------------------------
// Avvio: caricamento, aggiornamento di Winapp2, cartelle dell'app.

/// Da dove viene la riga di stato di Winapp2.
pub enum Winapp2Status<'a> {
    /// Copia valida caricata (versione), aggiornamento ancora in corso.
    Loaded(&'a str),
    /// Esito dell'aggiornamento.
    Outcome(&'a UpdateOutcome),
}

/// Testo della riga di stato sopra "Applicazioni".
pub fn status_text(s: Winapp2Status) -> String {
    match s {
        Winapp2Status::Loaded(version) => format!("Winapp2 {version} loaded"),
        Winapp2Status::Outcome(UpdateOutcome::Updated { version }) => format!("Winapp2 {version} updated"),
        Winapp2Status::Outcome(UpdateOutcome::UpToDate) => "Winapp2 already up to date".to_string(),
        Winapp2Status::Outcome(UpdateOutcome::KeptOld(why)) => format!("Winapp2 not updated: {why}"),
        Winapp2Status::Outcome(UpdateOutcome::NoCopyYet(_)) => "Winapp2 not downloaded yet".to_string(),
    }
}

/// Riga di stato mostrata e testo al passaggio del mouse (il motivo, se non
/// c'è ancora una copia). Finché il `RuleSet` ricaricato dopo `Updated` non
/// arriva, resta la riga del caricamento: la versione mostrata è sempre
/// quella delle regole in uso.
pub fn status_line(rs: &RuleSet, outcome: Option<&UpdateOutcome>) -> (String, Option<String>) {
    match outcome {
        None => (rs.status.clone(), None),
        Some(UpdateOutcome::Updated { version }) if rs.winapp2_version.as_deref() != Some(version.as_str()) => {
            (rs.status.clone(), None)
        }
        Some(o @ UpdateOutcome::NoCopyYet(why)) => (status_text(Winapp2Status::Outcome(o)), Some(why.clone())),
        Some(o) => (status_text(Winapp2Status::Outcome(o)), None),
    }
}

/// `false` in modalità prova (`--screenshot`): niente rete, niente scritture
/// in `%ProgramData%`.
pub fn should_update(screenshot: bool) -> bool {
    !screenshot
}

/// Avvia l'aggiornamento di Winapp2 nella cartella data; chiama la callback
/// una volta sola con l'esito (in produzione `updater::spawn_update`).
pub type UpdateSpawner = Box<dyn FnOnce(PathBuf, Box<dyn FnOnce(UpdateOutcome) + Send>) + Send>;

/// L'aggiornamento vero: `updater::spawn_update` (thread staccato, curl, ACL).
pub fn real_update() -> UpdateSpawner {
    Box::new(updater::spawn_update)
}

/// Carica le regole in un thread staccato (nessun `join`: l'uscita dal
/// programma non lo aspetta) e le manda con `notify`. Poi, se `update` c'è,
/// aggiorna Winapp2 e, sullo stesso thread, ricarica con lo stesso `Env`
/// quando l'esito è `Updated` (vedi [`after_update`]). L'`Env` riceve le
/// chiavi `protect-tree:` delle cartelle dell'app (`own`, in produzione
/// [`own_folders`]) prima di ogni caricamento.
pub fn start(
    dir: PathBuf,
    own: impl FnOnce(&Env) -> Vec<PathBuf> + Send + 'static,
    make_env: impl FnOnce() -> Env + Send + 'static,
    load: impl Fn(&Path, &Env) -> RuleSet + Send + 'static,
    update: Option<UpdateSpawner>,
    notify: impl Fn(RulesMsg) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut env = make_env();
        let own = own(&env);
        protect_own_folders(&mut env, &own);
        notify(RulesMsg::Loaded(load(&dir, &env)));
        let Some(spawn) = update else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(
            dir.clone(),
            Box::new(move |outcome| {
                let _ = tx.send(outcome);
            }),
        );
        // La callback si chiama sempre una volta; se il thread sparisce, `recv` fallisce.
        if let Ok(outcome) = rx.recv() {
            after_update(outcome, || load(&dir, &env), &notify);
        }
    });
}

/// Esito dell'aggiornamento → messaggi: sempre `Updated(esito)`; con
/// `Updated` anche le regole ricaricate (`reload`), negli altri casi si
/// tengono quelle già caricate.
pub fn after_update(outcome: UpdateOutcome, reload: impl FnOnce() -> RuleSet, notify: &dyn Fn(RulesMsg)) {
    let again = matches!(outcome, UpdateOutcome::Updated { .. });
    notify(RulesMsg::Updated(outcome));
    if again {
        notify(RulesMsg::Loaded(reload()));
    }
}

/// Cartelle dell'app da proteggere con tutto il contenuto: `%ProgramData%\Xiox`
/// (le regole) e `Documenti\Xiox` (backup del registro e dei servizi):
/// sia `%USERPROFILE%\Documents\Xiox`, quella di `sys::data_dir`, sia
/// `<documents>\Xiox` dell'`Env` (Documenti spostati, per esempio in OneDrive).
/// Si proteggono anche le cartelle con il nome di prima (`PulisciPC`): finché una
/// vecchia installazione non è stata rinominata (o se la rinomina non è riuscita)
/// ci stanno ancora i backup dell'utente.
pub fn own_folders(env: &Env) -> Vec<PathBuf> {
    let app_data = updater::data_dir().parent().map(Path::to_path_buf).unwrap_or_else(updater::data_dir);
    own_folders_with(&app_data, std::env::var("USERPROFILE").ok().as_deref(), env)
}

/// [`own_folders`] con le cartelle passate da fuori (nei test, finte).
pub fn own_folders_with(app_data: &Path, profile: Option<&str>, env: &Env) -> Vec<PathBuf> {
    let mut out = vec![app_data.to_path_buf()];
    // Stessa cartella dati con l'altro nome (`Xiox` <-> `PulisciPC`), accanto alla prima.
    let known_name = app_data.file_name().is_some_and(|n| {
        n.eq_ignore_ascii_case(xiox_model::legacy::APP_DIR) || n.eq_ignore_ascii_case(xiox_model::legacy::LEGACY_APP_DIR)
    });
    if let (true, Some(parent)) = (known_name, app_data.parent()) {
        out.push(parent.join(xiox_model::legacy::APP_DIR));
        out.push(parent.join(xiox_model::legacy::LEGACY_APP_DIR));
    }
    let mut under = |base: &Path| {
        out.push(base.join(xiox_model::legacy::APP_DIR));
        out.push(base.join(xiox_model::legacy::LEGACY_APP_DIR));
    };
    if let Some(p) = profile.filter(|p| !p.trim().is_empty()) {
        under(&Path::new(p).join("Documents"));
    }
    if let Some(docs) = env.get("documents") {
        under(Path::new(docs));
    }
    out.sort();
    out.dedup();
    out
}

/// Aggiunge all'`Env` una chiave `protect-tree:` per cartella: il `Guard`
/// (analisi e pulizia) le rifiuta con tutto il contenuto.
pub fn protect_own_folders(env: &mut Env, dirs: &[PathBuf]) {
    for (i, d) in dirs.iter().enumerate() {
        env.0.insert(format!("{PROTECT_TREE_PREFIX}xiox{i}"), d.to_string_lossy().into_owned());
    }
}

/// Regola 9: una voce di Winapp2 si nasconde solo se è **tutta** coperta da
/// cleaner inglobati presenti: ha almeno un'azione su file, nessuna azione di
/// registro (un inglobato non le copre), e ogni azione su file coincide con,
/// o sta dentro, un'azione su file di un inglobato ([`xiox_rules::within`], a
/// senso unico: un antenato non basta).
pub fn covered_by_embedded(entry: &Cleaner, embedded: &[Cleaner], env: &Env) -> bool {
    let actions: Vec<&Action> = entry.options.iter().flat_map(|o| &o.actions).collect();
    let outer: Vec<&str> = embedded.iter().filter(|c| is_embedded(c)).flat_map(file_paths).collect();
    !actions.is_empty()
        && actions.iter().all(|a| match a {
            Action::Files { path, .. } => outer.iter().any(|o| within(path, o, env)),
            _ => false,
        })
}

/// Percorsi (non espansi) delle azioni su file di un cleaner.
fn file_paths(c: &Cleaner) -> impl Iterator<Item = &str> {
    c.options.iter().flat_map(|o| &o.actions).filter_map(|a| match a {
        Action::Files { path, .. } => Some(path.as_str()),
        _ => None,
    })
}

/// Nome di un'opzione nelle tabelle e negli avvisi: "<cleaner> – <opzione>",
/// solo il nome se coincidono (le voci di Winapp2 hanno una sola opzione).
pub fn row_label(cleaner: &Cleaner, opt: &Opt) -> String {
    if cleaner.label == opt.label {
        cleaner.label.clone()
    } else {
        format!("{} – {}", cleaner.label, opt.label)
    }
}

/// `true` se il cleaner viene da un XML inglobato.
pub fn is_embedded(c: &Cleaner) -> bool {
    matches!(c.origin, Origin::CleanerMl(_))
}

/// Perché l'opzione non si può scegliere né pulire adesso (`None` = si può):
/// non supportata, oppure un processo di `running` è attivo.
pub fn blocked(cleaner: &Cleaner, opt: &Opt, processes: &[String]) -> Option<String> {
    if let Some(why) = &opt.unsupported {
        let mut s = why.clone();
        if let Some(first) = s.get(..1).map(str::to_uppercase) {
            s.replace_range(..1, &first);
        }
        return Some(format!("{s}: this option cannot be used"));
    }
    let active = running(cleaner, processes);
    if active.is_empty() {
        None
    } else {
        Some(format!("Close {} to clean", active.join(", ")))
    }
}

// ---------------------------------------------------------------------------
// Voci native (`crate::native`): stesse caselle, stessa selezione, stessi avvisi.

/// Prefisso delle chiavi di selezione delle voci native.
pub const NATIVE_PREFIX: &str = "native:";

/// Chiave di selezione e di risultato di una voce nativa: `"native:<id>"`.
pub fn native_key(item: &NativeItem) -> String {
    format!("{NATIVE_PREFIX}{}", item.id)
}

/// Le voci native accese per default (selezione iniziale del pannello).
pub fn native_defaults(items: &[NativeItem]) -> HashSet<String> {
    items.iter().filter(|i| i.default_on).map(native_key).collect()
}

/// Come [`blocked`] per una voce nativa: `Some("Chiudi … per pulire")` se un
/// processo di `running` è attivo (nomi senza maiuscole, `.exe` facoltativo).
pub fn native_blocked(item: &NativeItem, processes: &[String]) -> Option<String> {
    let hits = running_among(item.running, processes);
    (!hits.is_empty()).then(|| format!("Close {} to clean", hits.join(", ")))
}

/// I nomi di `wanted` che sono fra i `processes` attivi, confrontati senza
/// maiuscole e con `.exe` facoltativo (come `xiox_rules::running`).
pub fn running_among<'a>(wanted: &[&'a str], processes: &[String]) -> Vec<&'a str> {
    let norm = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
    };
    let active: Vec<String> = processes.iter().map(|p| norm(p)).collect();
    wanted
        .iter()
        .copied()
        .filter(|r| {
            let n = norm(r);
            !n.is_empty() && active.contains(&n)
        })
        .collect()
}

/// Voci native spuntate e non bloccate (abilitano «Avvia pulizia»).
pub fn active_natives<'a>(items: &'a [NativeItem], sel: &HashSet<String>, processes: &[String]) -> Vec<&'a NativeItem> {
    items
        .iter()
        .filter(|i| sel.contains(&native_key(i)) && native_blocked(i, processes).is_none())
        .collect()
}

/// "<voce>: <avviso>" per ogni voce nativa con un avviso, con fra parentesi
/// il testo di `warning_extra` (calcolato adesso: «lista: N domini»).
pub fn native_warnings(items: &[&NativeItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| {
            let w = i.warning?;
            Some(match i.warning_extra {
                Some(extra) => format!("{}: {w} ({})", i.label, extra()),
                None => format!("{}: {w}", i.label),
            })
        })
        .collect()
}

/// Una riga per ogni voce con `needs_preview` scelta: con «Pulizia» agisce
/// solo su ciò che l'ultima Analisi ha mostrato. `shown(item)` dà quanti
/// elementi quell'Analisi ha mostrato, `None` se non ce n'è una ancora
/// valida (allora la voce non farà nulla, e la conferma lo dice).
pub fn preview_notes(items: &[&NativeItem], shown: &dyn Fn(&NativeItem) -> Option<u64>) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.needs_preview)
        .map(|i| match shown(i) {
            Some(0) => format!("{}: the last Analysis showed nothing to remove, nothing is touched", i.label),
            Some(n) => {
                let what = if n == 1 { "1 item".to_string() } else { format!("{n} items") };
                format!("{}: only what the last Analysis showed is touched ({what})", i.label)
            }
            None => format!("{}: no Analysis to use, the item will do nothing. {}", i.label, crate::native::NO_PREVIEW),
        })
        .collect()
}

/// Gruppi del pannello: i gruppi dei cleaner inglobati (`groups`) con, in
/// ciascuno, le voci native dello stesso `group` (indici in `natives`), che
/// vengono **prima** dei cleaner XML. Un gruppo che ha solo voci native si
/// inserisce al suo posto di [`GROUP_ORDER`]; fuori elenco, in ordine alfabetico.
pub fn panel_groups(groups: &[(String, Vec<usize>)], natives: &[NativeItem]) -> Vec<(String, Vec<usize>, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>, Vec<usize>)> =
        groups.iter().map(|(g, idx)| (g.clone(), Vec::new(), idx.clone())).collect();
    for (n, item) in natives.iter().enumerate() {
        match out.iter_mut().find(|(g, _, _)| g == item.group) {
            Some((_, v, _)) => v.push(n),
            None => out.push((item.group.to_string(), vec![n], Vec::new())),
        }
    }
    let rank = |g: &str| GROUP_ORDER.iter().position(|o| *o == g).unwrap_or(GROUP_ORDER.len());
    // Stesso ordine di `embedded_groups`: rango di `GROUP_ORDER`, poi il nome.
    out.sort_by_key(|(g, _, _)| (rank(g), g.to_lowercase()));
    out
}

/// Raccoglie ciò che l'opzione toglierebbe. Un'opzione non supportata non
/// raccoglie nulla.
pub fn analyze_option(
    opt: &Opt,
    env: &Env,
    reg_exists: &dyn Fn(&RegItem) -> bool,
    ctl: &CollectCtl,
) -> RuleFound {
    if opt.unsupported.is_some() {
        return RuleFound::default();
    }
    let c = collect_ctl(opt, env, &[], reg_exists, ctl);
    RuleFound {
        count: (c.files.len() + c.reg.len()) as u64,
        files: c.files,
        dirs_to_remove: c.dirs_to_remove,
        reg: c.reg,
        bytes: c.bytes,
        skipped: c.skipped,
        skipped_reasons: c.skipped_reasons,
        cancelled: c.cancelled,
    }
}

/// Opzioni spuntate che si possono davvero pulire adesso (né non supportate
/// né bloccate da un processo): abilitano «Avvia pulizia» e danno gli avvisi
/// della conferma.
pub fn active_choices<'a>(
    cleaners: &'a [Cleaner],
    sel: &HashSet<String>,
    processes: &[String],
) -> Vec<(&'a Cleaner, &'a Opt)> {
    cleaners
        .iter()
        .flat_map(|c| c.options.iter().map(move |o| (c, o)))
        .filter(|(c, o)| sel.contains(&key(c, o)) && blocked(c, o, processes).is_none())
        .collect()
}

/// "<cleaner> – <opzione>: <avviso>" per ogni scelta con un avviso.
pub fn choice_warnings(choices: &[(&Cleaner, &Opt)]) -> Vec<String> {
    choices
        .iter()
        .filter_map(|(c, o)| o.warning.as_ref().map(|w| format!("{}: {w}", row_label(c, o))))
        .collect()
}

/// `true` se le regole si leggono con le variabili del processo invece che
/// con le cartelle note: solo con `--screenshot` e solo nella build di debug
/// (il profilo finto delle prove); in release `--screenshot` usa
/// `Env::for_rules` come il resto.
pub fn rules_env_from_process(screenshot: bool) -> bool {
    cfg!(debug_assertions) && screenshot
}

/// Pulisce ciò che `analyze_option` ha raccolto: i file, poi le cartelle
/// svuotate (nell'ordine dato), poi il registro (backup `.reg` prima; se il
/// backup non riesce nessuna voce di registro si cancella; si cancellano solo
/// le voci che il backup ha trovato). Un file o una cartella che non sta
/// sotto una cartella di base delle azioni di `opt` (espanse con `env`) non
/// si tocca. Non conosce la UI né i processi: chi chiama usa [`clean_checked`].
pub fn clean_option(
    found: &RuleFound,
    opt: &Opt,
    env: &Env,
    guard: &Guard,
    reg: &dyn RuleRegistry,
    cancel: &AtomicBool,
) -> RuleCleaned {
    let mut out = RuleCleaned::default();
    if opt.unsupported.is_some() {
        out.note("Option not supported: not cleaned".to_string());
        return out;
    }
    if found.cancelled {
        out.note("Analysis interrupted: nothing cleaned".to_string());
        return out;
    }
    let bases = rule_bases(opt, env);
    for (path, size) in &found.files {
        if cancel.load(Ordering::Relaxed) {
            out.note("Cleanup interrupted".to_string());
            return out;
        }
        if !under_any(path, &bases) {
            out.failed += 1;
            out.note(format!("Not touched, outside the rule's folders: {}", path.display()));
            continue;
        }
        match delete_path(path, guard) {
            Ok(()) => {
                out.removed += 1;
                out.files += 1;
                out.freed += size;
            }
            Err(e) => {
                out.failed += 1;
                out.note(e);
            }
        }
    }
    for dir in &found.dirs_to_remove {
        if cancel.load(Ordering::Relaxed) {
            out.note("Cleanup interrupted".to_string());
            return out;
        }
        if !under_any(dir, &bases) {
            out.note(format!("Not touched, outside the rule's folders: {}", dir.display()));
            continue;
        }
        // Una cartella che non si è svuotata (file in uso) resta: non è un errore.
        let _ = delete_path(dir, guard);
    }
    let mut items: Vec<RegItem> = Vec::new();
    for item in &found.reg {
        let (root, path, value) = reg_parts(item);
        match reg_allowed(root, path, value) {
            Ok(()) => items.push(item.clone()),
            Err(why) => {
                out.failed += 1;
                out.note(format!("Registry not touched: {} ({why})", reg_text(item)));
            }
        }
    }
    if items.is_empty() || cancel.load(Ordering::Relaxed) {
        return out;
    }
    let backup = match reg.backup(&items) {
        Ok(b) => b,
        Err(e) => {
            out.failed += items.len() as u64;
            out.note(format!(
                "Registry backup failed ({e}): no registry item deleted"
            ));
            return out;
        }
    };
    out.backup = backup.path;
    for (i, item) in items.iter().enumerate() {
        // Una voce che non c'era al backup non si cancella (potrebbe nascere
        // dopo, senza copia).
        if !backup.present.get(i).copied().unwrap_or(false) {
            out.note(format!("Already gone: {}", reg_text(item)));
            continue;
        }
        match reg.delete(item) {
            Ok(true) => out.removed += 1,
            Ok(false) => out.note(format!("Already gone: {}", reg_text(item))),
            Err(e) => {
                out.failed += 1;
                out.note(format!("{}: {e}", reg_text(item)));
            }
        }
    }
    out
}

/// Cartelle di base delle azioni su file di `opt` (percorsi espansi fino al
/// primo componente con jolly), come componenti confrontabili.
fn rule_bases(opt: &Opt, env: &Env) -> Vec<Vec<String>> {
    opt.actions
        .iter()
        .filter_map(|a| match a {
            Action::Files { path, .. } => Some(path),
            _ => None,
        })
        .flat_map(|p| xiox_rules::expand(p, env))
        .map(|p| {
            comps(&p)
                .into_iter()
                .take_while(|c| !c.contains(['*', '?']))
                .collect::<Vec<_>>()
        })
        .filter(|b| b.len() >= 2)
        .collect()
}

/// `path` coincide con una delle basi o sta sotto.
fn under_any(path: &Path, bases: &[Vec<String>]) -> bool {
    let c = comps(path);
    bases.iter().any(|b| c.starts_with(b))
}

/// Componenti in minuscolo, senza `\\?\`, vuoti e `.`.
fn comps(path: &Path) -> Vec<String> {
    let s = path.to_string_lossy().replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.split('\\')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_lowercase)
        .collect()
}

/// Come [`clean_option`], ma prima ricontrolla [`blocked`] con l'elenco dei
/// processi preso subito prima della pulizia: un'opzione bloccata si salta e
/// lo si scrive nel resoconto.
#[allow(clippy::too_many_arguments)]
pub fn clean_checked(
    cleaner: &Cleaner,
    opt: &Opt,
    found: &RuleFound,
    processes: &[String],
    env: &Env,
    guard: &Guard,
    reg: &dyn RuleRegistry,
    cancel: &AtomicBool,
) -> RuleCleaned {
    if let Some(why) = blocked(cleaner, opt, processes) {
        let mut out = RuleCleaned::default();
        out.note(format!("Skipped: {why}"));
        return out;
    }
    clean_option(found, opt, env, guard, reg, cancel)
}

/// Ciò che serve al ciclo delle opzioni dinamiche (tutto iniettabile).
pub struct RunCtx<'a> {
    pub env: &'a Env,
    /// Un solo `Guard` per tutta la pulizia.
    pub guard: &'a Guard,
    pub reg: &'a dyn RuleRegistry,
    pub reg_exists: &'a (dyn Fn(&RegItem) -> bool + Sync),
    /// Elenco dei processi attivi, riletto prima di ogni analisi e di ogni pulizia.
    pub processes: &'a (dyn Fn() -> Vec<String> + Sync),
    pub cancel: &'a AtomicBool,
}

/// Analizza (e con `also_clean` pulisce) le opzioni scelte, in sequenza.
/// I processi si rileggono subito prima dell'analisi e subito prima della
/// pulizia di **ogni** opzione. `progress(indice, testo)` riceve l'avanzamento;
/// `emit(chiave, etichetta, raccolta, esito)` una riga per opzione.
pub fn run_options(
    chosen: &[(Cleaner, Opt)],
    also_clean: bool,
    run: &RunCtx,
    progress: &(dyn Fn(usize, String) + Sync),
    emit: &mut dyn FnMut(String, String, RuleFound, Option<RuleCleaned>),
) {
    let verb = if also_clean { "Cleaning" } else { "Analyzing" };
    for (k, (c, o)) in chosen.iter().enumerate() {
        if run.cancel.load(Ordering::Relaxed) {
            return;
        }
        let label = row_label(c, o);
        progress(k, format!("{verb} – {label}"));
        let key = key(c, o);
        if let Some(why) = blocked(c, o, &(run.processes)()) {
            let mut skipped = RuleCleaned::default();
            skipped.note(format!("Skipped: {why}"));
            emit(key, label, RuleFound::default(), also_clean.then_some(skipped));
            continue;
        }
        let tick = |n: u64| progress(k, format!("{verb} – {label} ({n} items)"));
        let ctl = CollectCtl { cancel: run.cancel, tick: &tick };
        let found = analyze_option(o, run.env, run.reg_exists, &ctl);
        let done = also_clean.then(|| {
            let now = (run.processes)();
            clean_checked(c, o, &found, &now, run.env, run.guard, run.reg, run.cancel)
        });
        emit(key, label, found, done);
    }
}

/// L'unica via con cui questo modulo cancella file e cartelle.
fn delete_path(path: &Path, guard: &Guard) -> Result<(), String> {
    xiox_rules::delete_checked_with(path, guard).map(|_| ())
}

fn reg_parts(item: &RegItem) -> (RegRoot, &str, Option<&str>) {
    match item {
        RegItem::Key { root, path } => (*root, path.as_str(), None),
        RegItem::Value { root, path, value } => (*root, path.as_str(), Some(value.as_str())),
    }
}

fn root_name(root: RegRoot) -> &'static str {
    match root {
        RegRoot::Hkcu => "HKCU",
        RegRoot::Hklm => "HKLM",
    }
}

/// `HKCU\Software\X` o `HKCU\Software\X|valore`, per il resoconto.
pub fn reg_text(item: &RegItem) -> String {
    let (root, path, value) = reg_parts(item);
    match value {
        Some(v) => format!("{}\\{path}|{v}", root_name(root)),
        None => format!("{}\\{path}", root_name(root)),
    }
}

// ---------------------------------------------------------------------------
// Backup del registro.

/// Esito di un backup: il file scritto (`None` se nessuna chiave esisteva) e,
/// per ogni voce passata, se esisteva (solo quelle si cancellano).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: Option<PathBuf>,
    pub present: Vec<bool>,
}

/// Accesso al registro per il backup, sostituibile nei test.
pub trait RegBackend {
    /// `Ok(true)` se la voce (chiave, o valore della chiave) esiste; `Err` se
    /// non si sa (accesso negato, collegamento): allora il backup fallisce.
    fn exists(&self, item: &RegItem) -> Result<bool, String>;
    /// Esporta la chiave `root\path` in `out` (formato di `reg export`, UTF-16).
    fn export(&self, root: RegRoot, path: &str, out: &Path) -> Result<(), String>;
}

/// Scrive in `dest` un `.reg` con tutte le chiavi esistenti di `items`, con
/// l'esportazione sicura di [`regbackup::write_backup`] (cartella di lavoro
/// `work`, destinazione che non sia un collegamento, nomi `<stem>_N.reg` mai
/// sovrascritti, `Err` se una chiave non si esporta).
///
/// - Una voce che non si sa se esista fa fallire tutto (nessuna cancellazione).
/// - Nessuna voce esistente: nessun file (`path: None`).
pub fn backup_to(
    dest: &Path,
    stem: &str,
    work: &regbackup::WorkDir,
    be: &dyn RegBackend,
    items: &[RegItem],
) -> Result<Backup, String> {
    let present = items
        .iter()
        .map(|i| be.exists(i).map_err(|e| format!("{}: {e}", reg_text(i))))
        .collect::<Result<Vec<bool>, String>>()?;
    let keys: Vec<(RegRoot, String)> = items
        .iter()
        .zip(&present)
        .filter(|(_, p)| **p)
        .map(|(i, _)| {
            let (root, path, _) = reg_parts(i);
            (root, path.to_string())
        })
        .collect();
    let export = |root: RegRoot, path: &str, out: &Path| be.export(root, path, out);
    let path = regbackup::write_backup(dest, stem, work, &export, &keys)?;
    Ok(Backup { path, present })
}

/// Il registro vero.
pub struct RealRegistry;

impl RuleRegistry for RealRegistry {
    fn backup(&self, items: &[RegItem]) -> Result<Backup, String> {
        let stem = format!("backup_regole_{}", sys::stamp());
        backup_to(&sys::data_dir(), &stem, &regbackup::admin_work_dir, &win::RealBackend, items)
    }
    fn delete(&self, item: &RegItem) -> Result<bool, String> {
        win::delete(item)
    }
}

/// Backup sul registro vero (come [`RealRegistry`]) ma nella cartella `dest`
/// invece che in `Documenti\Xiox`, con la cartella di lavoro adatta al
/// processo (`regbackup::work_dir`). Per i backup con segreti (`netcreds`).
pub fn real_backup_in(dest: &Path, stem: &str, items: &[RegItem]) -> Result<Backup, String> {
    backup_to(dest, stem, &regbackup::work_dir, &win::RealBackend, items)
}

/// Esistenza di una chiave o di un valore con la stessa apertura della
/// pulizia (`REG_OPTION_OPEN_LINK`, vista a 64 bit, solo il collegamento di
/// `HKCU\Software\Classes` seguito): analisi e pulizia vedono lo stesso
/// registro. Un errore conta come «non c'è» (la voce non si raccoglie).
pub fn reg_exists(item: &RegItem) -> bool {
    win::exists(item).unwrap_or(false)
}

/// Nomi degli eseguibili in esecuzione (`claude.exe`, …), via ToolHelp.
pub fn processes() -> Vec<String> {
    win::processes()
}

/// `true` se, aprendo `comps` (i componenti fin qui, l'ultimo è un
/// collegamento) sotto `root`, il collegamento si può seguire: solo
/// `HKCU\Software\Classes` verso esattamente `\REGISTRY\USER\<sid>_Classes`,
/// con `sid` quello del token del processo.
pub fn classes_link_ok(root: RegRoot, comps: &[&str], target: &str, sid: &str) -> bool {
    root == RegRoot::Hkcu
        && !sid.is_empty()
        && comps.len() == 2
        && comps[0].eq_ignore_ascii_case("software")
        && comps[1].eq_ignore_ascii_case("classes")
        && target.eq_ignore_ascii_case(&format!(r"\REGISTRY\USER\{sid}_Classes"))
}

/// Padre e nome di una chiave dai componenti normalizzati (`None` se non ha
/// almeno due componenti): niente `RegDeleteTreeW(padre, "")`.
pub fn split_parent(path: &str) -> Option<(String, String)> {
    let comps: Vec<&str> = path.split('\\').filter(|c| !c.is_empty()).collect();
    let (name, parent) = comps.split_last()?;
    if parent.is_empty() {
        return None;
    }
    Some((parent.join("\\"), (*name).to_string()))
}

/// Esito di `RegEnumKeyExW` nell'esame di un albero: `None` = continua,
/// `Some(false)` = fine dell'elenco, `Some(true)` = errore (si rifiuta).
pub fn enum_verdict(rc: u32) -> Option<bool> {
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    match rc {
        ERROR_SUCCESS => None,
        ERROR_NO_MORE_ITEMS => Some(false),
        _ => Some(true),
    }
}

mod win {
    use std::path::Path;
    use std::sync::OnceLock;

    use xiox_rules::{RegItem, RegRoot};
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY,
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_READ,
        KEY_SET_VALUE, KEY_WOW64_64KEY, REG_LINK, REG_OPTION_OPEN_LINK,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{classes_link_ok, enum_verdict, reg_parts, reg_text, split_parent, RegBackend};

    /// Diritto standard `DELETE` (serve a `RegDeleteTreeW` sulla chiave padre).
    const DELETE: u32 = 0x0001_0000;
    /// Sottochiavi esaminate al più, cercando collegamenti, prima di
    /// cancellare un albero: oltre, la chiave non si tocca.
    const TREE_BUDGET: usize = 20_000;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Chiave aperta, chiusa all'uscita.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: chiave aperta da `RegOpenKeyExW`, chiusa una volta sola.
            unsafe { RegCloseKey(self.0) };
        }
    }

    /// Apre `name` sotto `parent` **senza seguire** un collegamento
    /// (`REG_OPTION_OPEN_LINK`), nella vista a 64 bit.
    fn open_one(parent: HKEY, name: &str, access: u32) -> Result<Key, u32> {
        let w = wide(name);
        let mut out: HKEY = std::ptr::null_mut();
        // SAFETY: `w` termina con 0, `out` è un puntatore valido.
        let rc = unsafe { RegOpenKeyExW(parent, w.as_ptr(), REG_OPTION_OPEN_LINK, access | KEY_WOW64_64KEY, &mut out) };
        if rc == ERROR_SUCCESS {
            Ok(Key(out))
        } else {
            Err(rc)
        }
    }

    /// Destinazione del collegamento (`SymbolicLinkValue` di tipo `REG_LINK`)
    /// se la chiave, aperta con `REG_OPTION_OPEN_LINK`, è un collegamento.
    /// `Err` se il valore c'è ma non si legge.
    fn link_target(key: &Key) -> Result<Option<String>, String> {
        let name = wide("SymbolicLinkValue");
        let mut ty = 0u32;
        let mut len = 0u32;
        // SAFETY: prima solo tipo e lunghezza.
        let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), &mut len) };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("unreadable link (error {rc})"));
        }
        if ty != REG_LINK {
            return Ok(None);
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: buffer della lunghezza appena letta.
        let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut ty, buf.as_mut_ptr(), &mut len) };
        if rc != ERROR_SUCCESS || ty != REG_LINK {
            return Err(format!("unreadable link (error {rc})"));
        }
        let words: Vec<u16> = buf[..len as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let s = String::from_utf16_lossy(&words);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }

    fn hive(root: RegRoot) -> HKEY {
        match root {
            RegRoot::Hkcu => HKEY_CURRENT_USER,
            RegRoot::Hklm => HKEY_LOCAL_MACHINE,
        }
    }

    /// SID (stringa) dell'utente del token del processo; vuoto se non si legge.
    fn user_sid() -> &'static str {
        static SID: OnceLock<String> = OnceLock::new();
        SID.get_or_init(|| read_user_sid().unwrap_or_default())
    }

    fn read_user_sid() -> Option<String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: pseudo-handle del processo; `token` valido.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut len = 0u32;
        // SAFETY: prima chiamata per la lunghezza.
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
        // Allineato a 8 byte per la struttura TOKEN_USER.
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        // SAFETY: buffer di almeno `len` byte.
        let ok = unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } != 0;
        // SAFETY: handle aperto sopra, chiuso una volta.
        unsafe { CloseHandle(token) };
        if !ok {
            return None;
        }
        // SAFETY: il buffer contiene una TOKEN_USER valida.
        let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let mut s: *mut u16 = std::ptr::null_mut();
        // SAFETY: SID del token; `s` allocata dal sistema e liberata con LocalFree.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut s) } == 0 || s.is_null() {
            return None;
        }
        // SAFETY: stringa terminata da 0 restituita dal sistema.
        let len = (0..).take_while(|&i| unsafe { *s.add(i) } != 0).count();
        // SAFETY: `len` caratteri validi.
        let out = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) });
        // SAFETY: memoria allocata da ConvertSidToStringSidW.
        unsafe { LocalFree(s.cast()) };
        Some(out)
    }

    /// Apre `path` un componente alla volta, ognuno senza seguire collegamenti.
    /// Un componente che è un collegamento si rifiuta, tranne
    /// `HKCU\Software\Classes` verso `\REGISTRY\USER\<SID del processo>_Classes`
    /// (vedi [`classes_link_ok`]): lì si continua da `HKEY_USERS\<SID>_Classes`,
    /// aperta allo stesso modo. L'ultimo componente si apre con `last_access`.
    /// `Ok(None)` se la chiave non esiste.
    fn open_chain(root: RegRoot, path: &str, last_access: u32) -> Result<Option<Key>, String> {
        let comps: Vec<&str> = path.split('\\').filter(|c| !c.is_empty()).collect();
        if comps.is_empty() {
            return Err("empty registry path".into());
        }
        let mut cur: Option<Key> = None;
        for (i, comp) in comps.iter().enumerate() {
            let parent = cur.as_ref().map(|k| k.0).unwrap_or_else(|| hive(root));
            let access = if i + 1 == comps.len() {
                last_access | KEY_QUERY_VALUE
            } else {
                KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS
            };
            let key = match open_one(parent, comp, access) {
                Ok(k) => k,
                Err(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Err(rc) => return Err(format!("cannot be opened (error {rc})")),
            };
            let key = match link_target(&key)? {
                None => key,
                Some(target) if classes_link_ok(root, &comps[..=i], &target, user_sid()) => {
                    drop(key);
                    let classes = format!("{}_Classes", user_sid());
                    let k = match open_one(HKEY_USERS, &classes, access) {
                        Ok(k) => k,
                        Err(ERROR_FILE_NOT_FOUND) => return Ok(None),
                        Err(rc) => return Err(format!("cannot be opened (error {rc})")),
                    };
                    if link_target(&k)?.is_some() {
                        return Err("registry link: not followed".into());
                    }
                    k
                }
                Some(_) => return Err("registry link: not followed".into()),
            };
            cur = Some(key);
        }
        Ok(cur)
    }

    #[cfg(test)]
    pub fn key_exists(root: RegRoot, path: &str) -> bool {
        matches!(open_chain(root, path, KEY_READ), Ok(Some(_)))
    }

    /// Esistenza di una chiave o di un valore con [`open_chain`]; `Err` se non
    /// si sa (accesso negato, collegamento non ammesso).
    pub fn exists(item: &RegItem) -> Result<bool, String> {
        let (root, path, value) = reg_parts(item);
        let Some(key) = open_chain(root, path, KEY_READ)? else {
            return Ok(false);
        };
        let Some(v) = value else { return Ok(true) };
        let w = wide(v);
        let mut ty = 0u32;
        // SAFETY: solo il tipo, nessun buffer.
        let rc = unsafe { RegQueryValueExW(key.0, w.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), std::ptr::null_mut()) };
        match rc {
            ERROR_SUCCESS => Ok(true),
            ERROR_FILE_NOT_FOUND => Ok(false),
            rc => Err(format!("unreadable value (error {rc})")),
        }
    }

    /// `true` se sotto `key` c'è un collegamento, una sottochiave che non si
    /// apre, un errore di elenco o più sottochiavi del budget.
    fn tree_has_link(key: &Key, budget: &mut usize) -> bool {
        let mut index = 0u32;
        loop {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            // SAFETY: buffer e lunghezza validi; classe e data non richieste.
            let rc = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if let Some(verdict) = enum_verdict(rc) {
                return verdict;
            }
            index += 1;
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            match open_one(key.0, &name, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS) {
                Ok(child) => {
                    if !matches!(link_target(&child), Ok(None)) || tree_has_link(&child, budget) {
                        return true;
                    }
                }
                Err(_) => return true,
            }
        }
    }

    /// Cancella una chiave (con l'albero) o un valore. `Ok(true)` se eliminata,
    /// `Ok(false)` se non c'era più.
    pub fn delete(item: &RegItem) -> Result<bool, String> {
        let (root, path, value) = reg_parts(item);
        // Contratto di xiox-rules: si ricontrolla subito prima di cancellare.
        xiox_rules::reg_allowed(root, path, value).map_err(|why| format!("{} ({why})", reg_text(item)))?;
        match value {
            Some(v) => {
                let Some(key) = open_chain(root, path, KEY_SET_VALUE)? else {
                    return Ok(false);
                };
                let w = wide(v);
                // SAFETY: chiave aperta con KEY_SET_VALUE, nome terminato da 0.
                let rc = unsafe { RegDeleteValueW(key.0, w.as_ptr()) };
                match rc {
                    ERROR_SUCCESS => Ok(true),
                    ERROR_FILE_NOT_FOUND => Ok(false),
                    rc => Err(format!("value not deleted (error {rc})")),
                }
            }
            None => {
                let (parent_path, name) = split_parent(path).ok_or("key without a parent")?;
                let Some(parent) = open_chain(root, &parent_path, KEY_READ | KEY_SET_VALUE | DELETE)? else {
                    return Ok(false);
                };
                let target = match open_one(parent.0, &name, KEY_READ) {
                    Ok(k) => k,
                    Err(ERROR_FILE_NOT_FOUND) => return Ok(false),
                    Err(rc) => return Err(format!("cannot be opened (error {rc})")),
                };
                let mut budget = TREE_BUDGET;
                if !matches!(link_target(&target), Ok(None)) || tree_has_link(&target, &mut budget) {
                    return Err("contains a registry link or is too large: not touched".into());
                }
                drop(target);
                let w = wide(&name);
                // SAFETY: padre aperto con DELETE e lettura, nome non vuoto terminato da 0.
                let rc = unsafe { RegDeleteTreeW(parent.0, w.as_ptr()) };
                match rc {
                    ERROR_SUCCESS => Ok(true),
                    ERROR_FILE_NOT_FOUND => Ok(false),
                    rc => Err(format!("key not deleted (error {rc})")),
                }
            }
        }
    }

    /// Il registro vero per il backup: esistenza con [`open_chain`] (la
    /// stessa apertura della pulizia), esportazione con il `reg.exe` di sistema
    /// (percorso assoluto), vista a 64 bit.
    pub struct RealBackend;

    impl RegBackend for RealBackend {
        fn exists(&self, item: &RegItem) -> Result<bool, String> {
            exists(item)
        }
        fn export(&self, root: RegRoot, path: &str, out: &Path) -> Result<(), String> {
            crate::regbackup::reg_exe_export(root, path, out)
        }
    }

    pub fn processes() -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: istantanea dei processi, chiusa sotto.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        // SAFETY: struttura C semplice, azzerata, con la dimensione impostata.
        let mut pe: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        // SAFETY: `snap` valido, `pe` inizializzata.
        let mut ok = unsafe { Process32FirstW(snap, &mut pe) } != 0;
        while ok {
            let len = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
            out.push(String::from_utf16_lossy(&pe.szExeFile[..len]));
            // SAFETY: come sopra.
            ok = unsafe { Process32NextW(snap, &mut pe) } != 0;
        }
        // SAFETY: handle dell'istantanea, chiuso una volta sola.
        unsafe { CloseHandle(snap) };
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiox_rules::{Action, Origin, RegRoot, Search};
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;

    /// Profilo finto dentro `root`: nessuna variabile del processo.
    fn fake_env(root: &Path) -> Env {
        let p = root.join("profile");
        let s = |x: &Path| x.to_string_lossy().into_owned();
        Env(HashMap::from([
            ("userprofile".to_string(), s(&p)),
            ("localappdata".to_string(), s(&p.join("AppData").join("Local"))),
            ("appdata".to_string(), s(&p.join("AppData").join("Roaming"))),
            ("temp".to_string(), s(&p.join("AppData").join("Local").join("Temp"))),
            ("username".to_string(), "u".to_string()),
            ("programdata".to_string(), s(&root.join("ProgramData"))),
        ]))
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn never(_: &RegItem) -> bool {
        false
    }

    fn no_copy(_: &Path) -> Result<(String, String), String> {
        Err("no local copy".to_string())
    }

    fn opt(actions: Vec<Action>) -> Opt {
        Opt {
            id: "o".into(),
            label: "Opzione".into(),
            description: String::new(),
            warning: None,
            default_on: false,
            actions,
            excludes: vec![],
            unsupported: None,
        }
    }

    fn cleaner(running: &[&str], options: Vec<Opt>) -> Cleaner {
        Cleaner {
            id: "c".into(),
            label: "Strumento".into(),
            group: EMBEDDED_GROUP.into(),
            origin: Origin::CleanerMl("c"),
            description: String::new(),
            running: running.iter().map(|s| s.to_string()).collect(),
            options,
        }
    }

    fn files_rule(dir: &Path, patterns: &[&str]) -> Action {
        Action::Files {
            path: dir.to_string_lossy().into_owned(),
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            recurse: true,
            remove_self: false,
            search: Search::Winapp2,
        }
    }

    fn ctl_run(opt: &Opt, env: &Env) -> RuleFound {
        let cancel = AtomicBool::new(false);
        let tick = |_: u64| {};
        analyze_option(opt, env, &never, &CollectCtl { cancel: &cancel, tick: &tick })
    }

    /// Registro finto: registra le chiamate, il backup può fallire; `absent`
    /// sono le voci che non esistono al backup, `gone` quelle che spariscono
    /// fra il backup e la cancellazione (`delete` → `Ok(false)`).
    #[derive(Default)]
    struct FakeReg {
        log: Mutex<Vec<String>>,
        fail_backup: bool,
        absent: Vec<String>,
        gone: Vec<String>,
    }

    fn item_text(item: &RegItem) -> String {
        match item {
            RegItem::Key { path, .. } => path.clone(),
            RegItem::Value { path, value, .. } => format!("{path}|{value}"),
        }
    }

    impl RuleRegistry for FakeReg {
        fn backup(&self, items: &[RegItem]) -> Result<Backup, String> {
            self.log.lock().unwrap().push(format!("backup:{}", items.len()));
            if self.fail_backup {
                return Err("reg export non riuscito".into());
            }
            let present: Vec<bool> = items.iter().map(|i| !self.absent.contains(&item_text(i))).collect();
            let path = present.iter().any(|&p| p).then(|| PathBuf::from(r"C:\finto\backup.reg"));
            Ok(Backup { path, present })
        }
        fn delete(&self, item: &RegItem) -> Result<bool, String> {
            self.log.lock().unwrap().push(format!("delete:{}", item_text(item)));
            Ok(!self.gone.contains(&item_text(item)))
        }
    }

    fn ids(rs: &RuleSet) -> Vec<String> {
        rs.cleaners.iter().map(|c| c.id.clone()).collect()
    }

    #[test]
    fn embedded_cleaner_shows_only_when_installed_files_exist() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        // Profilo vuoto (con la sua Temp): nessuno strumento.
        fs::create_dir_all(t.path().join("profile").join("AppData").join("Local").join("Temp")).unwrap();
        let rs = load_with(&t.path().join("rules"), &env, &no_copy, &never);
        assert!(rs.cleaners.is_empty(), "{:?}", ids(&rs));
        assert_eq!(rs.status, "Winapp2 not downloaded yet");
        assert_eq!(rs.winapp2_version, None);
        // Cargo installato: compare solo lui.
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        let rs = load_with(&t.path().join("rules"), &env, &no_copy, &never);
        assert_eq!(ids(&rs), vec!["bbag_cargo"]);
        assert!(rs.cleaners.iter().all(|c| c.group == EMBEDDED_GROUP));
        assert!(rs.sections.is_empty());
    }

    #[test]
    fn embedded_groups_follow_windows_browser_development_order() {
        let mk = |id: &str, group: &str| {
            let mut c = cleaner(&[], vec![]);
            c.id = id.into();
            c.group = group.into();
            c
        };
        let cleaners = vec![
            mk("bbag_cargo", EMBEDDED_GROUP),
            mk("safari", "Browser"),
            mk("altro", "Zeta"),
            mk("windows_extra", "Windows"),
            mk("uv", EMBEDDED_GROUP),
        ];
        let groups = embedded_groups(&cleaners);
        let names: Vec<&str> = groups.iter().map(|(g, _)| g.as_str()).collect();
        assert_eq!(names, vec!["Windows", "Browser", EMBEDDED_GROUP, "Zeta"]);
        assert_eq!(groups[0].1, vec![3]);
        assert_eq!(groups[1].1, vec![1]);
        assert_eq!(groups[2].1, vec![0, 4]);
        // "Applicazioni" non è un gruppo degli inglobati: viene dopo, nel pannello.
        assert!(!GROUP_ORDER.contains(&WINAPP2_GROUP));
    }

    #[test]
    fn real_embedded_cleaners_group_windows_and_safari_first() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        // Safari e Internet Explorer presenti nel profilo finto, più Cargo.
        write(&t.path().join(r"profile\AppData\Local\Apple Computer\Safari\Cache.db"), "x");
        write(&t.path().join(r"profile\AppData\Local\Microsoft\Windows\INetCache\IE\a.gif"), "x");
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        let rs = load_with(&t.path().join("rules"), &env, &no_copy, &never);
        let names: Vec<(&str, Vec<&str>)> = rs
            .groups
            .iter()
            .map(|(g, idx)| (g.as_str(), idx.iter().map(|&i| rs.cleaners[i].id.as_str()).collect()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Windows", vec!["windows_extra"]),
                ("Browser", vec!["safari"]),
                (EMBEDDED_GROUP, vec!["bbag_cargo"]),
            ]
        );
    }

    #[test]
    fn winapp2_entry_covering_an_embedded_tool_is_hidden() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        write(&t.path().join(r"profile\.otherapp\cache\b.tmp"), "x");
        let ini = "; Version: 260915\r\n\
                   [Cargo Registry *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry\\cache|*.*|RECURSE\r\n\r\n\
                   [Other App *]\r\nSection=Utilities\r\nFileKey1=%UserProfile%\\.otherapp\\cache|*.*|RECURSE\r\n";
        let read = |_: &Path| Ok((ini.to_string(), "260915".to_string()));
        let rs = load_with(&t.path().join("rules"), &env, &read, &never);
        assert_eq!(ids(&rs), vec!["bbag_cargo", "winapp2:Other App"]);
        assert_eq!(rs.status, "Winapp2 260915 loaded");
        assert_eq!(rs.winapp2_version.as_deref(), Some("260915"));
        assert_eq!(rs.sections, vec![("Utilities".to_string(), vec![1])]);
        assert!(rs.warnings.iter().any(|w| w.contains("1 item")), "{:?}", rs.warnings);
    }

    #[test]
    fn analyze_then_clean_removes_only_collected_files() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let app = t.path().join(r"profile\.app");
        write(&app.join(r"cache\a.tmp"), "aaaa");
        write(&app.join(r"cache\sub\b.tmp"), "bb");
        write(&app.join(r"cache\keep.txt"), "keep");
        write(&app.join("other.tmp"), "outside");
        let o = opt(vec![files_rule(&app.join("cache"), &["*.tmp"])]);
        let found = ctl_run(&o, &env);
        assert_eq!(found.count, 2);
        assert_eq!(found.bytes, 6);
        let guard = Guard::new(&env);
        let reg = FakeReg::default();
        let done = clean_option(&found, &o, &env, &guard, &reg, &AtomicBool::new(false));
        assert_eq!((done.removed, done.failed, done.freed), (2, 0, 6), "{:?}", done.notes);
        assert!(!app.join(r"cache\a.tmp").exists());
        assert!(!app.join(r"cache\sub\b.tmp").exists());
        assert!(app.join(r"cache\keep.txt").exists());
        assert!(app.join("other.tmp").exists());
        assert!(reg.log.lock().unwrap().is_empty());
    }

    fn reg_found(items: Vec<RegItem>) -> RuleFound {
        RuleFound {
            count: items.len() as u64,
            reg: items,
            ..Default::default()
        }
    }

    #[test]
    fn registry_rules_make_a_backup_before_deleting() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let guard = Guard::new(&env);
        let items = vec![
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp\Recent".into() },
            RegItem::Value { root: RegRoot::Hkcu, path: r"Software\FakeApp".into(), value: "LastFile".into() },
        ];
        let found = reg_found(items);
        let o = opt(vec![]);
        let reg = FakeReg::default();
        let done = clean_option(&found, &o, &env, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(
            *reg.log.lock().unwrap(),
            vec!["backup:2", r"delete:Software\FakeApp\Recent", r"delete:Software\FakeApp|LastFile"]
        );
        assert_eq!((done.removed, done.failed), (2, 0));
        assert_eq!(done.backup, Some(PathBuf::from(r"C:\finto\backup.reg")));
        // Backup non riuscito: nessuna cancellazione, e lo si dice.
        let reg = FakeReg { fail_backup: true, ..Default::default() };
        let done = clean_option(&found, &o, &env, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(*reg.log.lock().unwrap(), vec!["backup:2"]);
        assert_eq!((done.removed, done.failed), (0, 2));
        assert!(done.notes.iter().any(|n| n.contains("Registry backup failed")), "{:?}", done.notes);
    }

    #[test]
    fn option_with_active_process_cannot_be_selected() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join(r"profile\.tool\cache");
        write(&dir.join("a.tmp"), "x");
        let o = opt(vec![files_rule(&dir, &["*"])]);
        let c = cleaner(&["claude.exe"], vec![o.clone()]);
        assert_eq!(blocked(&c, &o, &["explorer.exe".into()]), None);
        assert_eq!(
            blocked(&c, &o, &["Claude.EXE".into()]).as_deref(),
            Some("Close claude.exe to clean")
        );
        // Spuntata prima, il processo parte prima della pulizia: si salta.
        let found = ctl_run(&o, &env);
        assert_eq!(found.count, 1);
        let reg = FakeReg::default();
        let done = clean_checked(
            &c,
            &o,
            &found,
            &["claude.exe".into()],
            &env,
            &Guard::new(&env),
            &reg,
            &AtomicBool::new(false),
        );
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        assert!(done.notes.iter().any(|n| n.contains("Close claude.exe")), "{:?}", done.notes);
    }

    #[test]
    fn unsupported_option_is_never_cleaned() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join(r"profile\.tool\cache");
        write(&dir.join("a.tmp"), "x");
        let mut o = opt(vec![files_rule(&dir, &["*"])]);
        o.unsupported = Some("non supportato: deep".into());
        let c = cleaner(&[], vec![o.clone()]);
        assert!(blocked(&c, &o, &[]).is_some());
        // L'analisi non raccoglie nulla.
        assert_eq!(ctl_run(&o, &env).count, 0);
        // Anche con un `RuleFound` che contiene il file, la pulizia non lo tocca.
        let found = RuleFound {
            files: vec![(dir.join("a.tmp"), 1)],
            bytes: 1,
            count: 1,
            ..Default::default()
        };
        let reg = FakeReg::default();
        let guard = Guard::new(&env);
        let done = clean_option(&found, &o, &env, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        let done = clean_checked(&c, &o, &found, &[], &env, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
    }

    #[test]
    fn registry_item_refused_by_reg_allowed_is_skipped_at_delete_time() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let found = reg_found(vec![
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\Microsoft\Windows\CurrentVersion\Run".into() },
            RegItem::Key { root: RegRoot::Hklm, path: r"System\CurrentControlSet\Services\X".into() },
            RegItem::Value { root: RegRoot::Hklm, path: r"Software\Microsoft\Windows NT\CurrentVersion".into(), value: "ProductName".into() },
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp".into() },
        ]);
        let reg = FakeReg::default();
        let done = clean_option(&found, &opt(vec![]), &env, &Guard::new(&env), &reg, &AtomicBool::new(false));
        assert_eq!(*reg.log.lock().unwrap(), vec!["backup:1", r"delete:Software\FakeApp"]);
        assert_eq!((done.removed, done.failed), (1, 3));
    }

    #[test]
    fn cancelled_collection_is_not_cleaned() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join(r"profile\.tool\cache");
        write(&dir.join("a.tmp"), "x");
        let o = opt(vec![files_rule(&dir, &["*"])]);
        let cancel = AtomicBool::new(true);
        let tick = |_: u64| {};
        let found = analyze_option(&o, &env, &never, &CollectCtl { cancel: &cancel, tick: &tick });
        assert!(found.cancelled);
        // Anche un `RuleFound` annullato con dentro il file non si pulisce.
        let found = RuleFound {
            files: vec![(dir.join("a.tmp"), 1)],
            count: 1,
            cancelled: true,
            reg: vec![RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp".into() }],
            ..Default::default()
        };
        let reg = FakeReg::default();
        let done = clean_option(&found, &o, &env, &Guard::new(&env), &reg, &AtomicBool::new(false));
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        assert!(reg.log.lock().unwrap().is_empty());
    }

    // ---------- correzioni della revisione del Task 7 ----------

    /// I3: una voce si nasconde solo se OGNI sua azione su file sta dentro
    /// (o coincide con) un'azione di un inglobato presente.
    #[test]
    fn winapp2_entry_wider_than_an_embedded_tool_stays() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        write(&t.path().join(r"profile\.claude\cache\x.txt"), "x");
        write(&t.path().join(r"profile\AppData\Local\Temp\a.tmp"), "x");
        write(&t.path().join(r"profile\.otherapp\cache\b.tmp"), "x");
        let ini = "; Version: 260915\r\n\
                   [Cargo Whole *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry|*.*|RECURSE\r\n\r\n\
                   [Windows Temp *]\r\nSection=Windows\r\nFileKey1=%Temp%|*.tmp\r\n\r\n\
                   [Mixed *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry\\cache|*.*|RECURSE\r\nFileKey2=%UserProfile%\\.otherapp\\cache|*.*|RECURSE\r\n\r\n\
                   [Cargo Cache *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry\\cache|*.*|RECURSE\r\nFileKey2=%UserProfile%\\.cargo\\registry\\src\\x|*.*|RECURSE\r\n";
        let read = |_: &Path| Ok((ini.to_string(), "260915".to_string()));
        let rs = load_with(&t.path().join("rules"), &env, &read, &never);
        let mut winapp: Vec<String> = ids(&rs).into_iter().filter(|i| i.starts_with("winapp2:")).collect();
        winapp.sort();
        assert_eq!(winapp, vec!["winapp2:Cargo Whole", "winapp2:Mixed", "winapp2:Windows Temp"]);
        assert!(rs.warnings.iter().any(|w| w.contains("1 item")), "{:?}", rs.warnings);
    }

    /// Una voce con chiavi di registro non si nasconde: l'inglobato non le copre.
    #[test]
    fn winapp2_entry_with_registry_rules_is_not_hidden() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        let ini = "; Version: 260915\r\n\
                   [Cargo Reg *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry\\cache|*.*|RECURSE\r\nRegKey1=HKCU\\Software\\CargoGui\\Recent\r\n";
        let read = |_: &Path| Ok((ini.to_string(), "260915".to_string()));
        let rs = load_with(&t.path().join("rules"), &env, &read, &never);
        assert!(ids(&rs).contains(&"winapp2:Cargo Reg".to_string()), "{:?}", ids(&rs));
    }

    /// Valore di `XIOX_<nome>`; se manca, quello del vecchio nome `PULISCI_<nome>`.
    fn first_env(get: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
        get(&format!("XIOX_{name}")).or_else(|| get(&format!("PULISCI_{name}")))
    }

    /// Il Winapp2 per le prove sul file vero: `XIOX_WINAPP2`, altrimenti `PULISCI_WINAPP2`.
    fn winapp2_from_env() -> Option<String> {
        first_env(&|n| std::env::var(n).ok().filter(|v| !v.is_empty()), "WINAPP2")
    }

    #[test]
    fn winapp2_env_var_reads_the_new_name_first_then_the_old_one() {
        let both = |n: &str| match n {
            "XIOX_WINAPP2" => Some("nuovo".to_string()),
            "PULISCI_WINAPP2" => Some("vecchio".to_string()),
            _ => None,
        };
        assert_eq!(first_env(&both, "WINAPP2").as_deref(), Some("nuovo"));
        let only_old = |n: &str| (n == "PULISCI_WINAPP2").then(|| "vecchio".to_string());
        assert_eq!(first_env(&only_old, "WINAPP2").as_deref(), Some("vecchio"));
        assert_eq!(first_env(&|_| None, "WINAPP2"), None);
    }

    /// Misura sul Winapp2 vero (sola lettura): quali voci nasconde la regola 9
    /// con tutti gli inglobati presenti e ogni `Detect` considerato vero.
    /// `XIOX_WINAPP2=<percorso> cargo test -p xiox-core real_winapp2_rule_9 -- --ignored --nocapture`
    /// (anche il vecchio nome `PULISCI_WINAPP2` vale, se `XIOX_WINAPP2` non c'è).
    #[test]
    #[ignore]
    fn real_winapp2_rule_9_keeps_broad_entries() {
        let path = winapp2_from_env().expect("XIOX_WINAPP2 non impostata");
        let text = fs::read_to_string(path).unwrap();
        let env = Env(HashMap::from([
            ("userprofile".to_string(), r"C:\Users\u".to_string()),
            ("localappdata".to_string(), r"C:\Users\u\AppData\Local".to_string()),
            ("appdata".to_string(), r"C:\Users\u\AppData\Roaming".to_string()),
            ("temp".to_string(), r"C:\Users\u\AppData\Local\Temp".to_string()),
            ("tmp".to_string(), r"C:\Users\u\AppData\Local\Temp".to_string()),
            ("username".to_string(), "u".to_string()),
            ("programdata".to_string(), r"C:\ProgramData".to_string()),
            ("windir".to_string(), r"C:\Windows".to_string()),
            ("systemroot".to_string(), r"C:\Windows".to_string()),
            ("systemdrive".to_string(), "C:".to_string()),
            ("programfiles".to_string(), r"C:\Program Files".to_string()),
            ("programfiles(x86)".to_string(), r"C:\Program Files (x86)".to_string()),
            ("commonappdata".to_string(), r"C:\ProgramData".to_string()),
            ("locallowappdata".to_string(), r"C:\Users\u\AppData\LocalLow".to_string()),
            ("documents".to_string(), r"C:\Users\u\Documents".to_string()),
            ("public".to_string(), r"C:\Users\Public".to_string()),
        ]));
        let (mut w, _) = winapp2::parse(&text);
        for e in &mut w.entries {
            e.detect = vec![winapp2::Detect::RegKey { root: RegRoot::Hkcu, path: "Software".into() }];
        }
        let (entries, _) = winapp2::to_cleaners_with(&w, &env, &|_, _| true);
        let (embedded, _) = xiox_rules::embedded(&env);
        let hidden: Vec<&str> = entries
            .iter()
            .filter(|c| covered_by_embedded(c, &embedded, &env))
            .map(|c| c.label.as_str())
            .collect();
        eprintln!("voci: {}, nascoste: {} {:?}", entries.len(), hidden.len(), hidden);
        for name in [
            "Windows Temporary Files",
            "Windows Error Reporting",
            "AMD",
            "Universal Windows Platform",
            "Android Studio",
            "Microsoft Visual Studio",
        ] {
            assert!(!hidden.contains(&name), "{name} nascosta");
        }
    }

    /// I2: una voce che non c'era più non conta come eliminata.
    #[test]
    fn absent_registry_items_are_not_counted_as_removed() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let found = reg_found(vec![
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp\A".into() },
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp\B".into() },
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp\C".into() },
        ]);
        let reg = FakeReg {
            absent: vec![r"Software\FakeApp\B".into()],
            gone: vec![r"Software\FakeApp\C".into()],
            ..Default::default()
        };
        let done = clean_option(&found, &opt(vec![]), &env, &Guard::new(&env), &reg, &AtomicBool::new(false));
        // B non c'era al backup: non si prova nemmeno a cancellarla.
        assert_eq!(
            *reg.log.lock().unwrap(),
            vec!["backup:3", r"delete:Software\FakeApp\A", r"delete:Software\FakeApp\C"]
        );
        assert_eq!((done.removed, done.failed), (1, 0), "{:?}", done.notes);
        assert_eq!(done.notes.iter().filter(|n| n.starts_with("Already gone")).count(), 2, "{:?}", done.notes);
    }

    /// M10: nessuna chiave esistente → niente file di backup, niente nota.
    #[test]
    fn no_existing_key_means_no_backup_and_no_note() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let found = reg_found(vec![RegItem::Key { root: RegRoot::Hkcu, path: r"Software\FakeApp\A".into() }]);
        let reg = FakeReg { absent: vec![r"Software\FakeApp\A".into()], ..Default::default() };
        let done = clean_option(&found, &opt(vec![]), &env, &Guard::new(&env), &reg, &AtomicBool::new(false));
        assert_eq!(done.backup, None);
        assert!(done.notes.iter().all(|n| !n.contains("ackup")), "{:?}", done.notes);
        assert_eq!(*reg.log.lock().unwrap(), vec!["backup:1"]);
    }

    /// M5: un file fuori dalle cartelle delle azioni dell'opzione non si tocca,
    /// anche se un `RuleFound` costruito a mano lo contiene.
    #[test]
    fn clean_refuses_files_outside_the_rule_folders() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join(r"profile\.tool\cache");
        write(&dir.join("a.tmp"), "x");
        let outside = t.path().join(r"profile\Documents\tesi.docx");
        write(&outside, "important");
        let sibling = t.path().join(r"profile\.tool\cache-old\b.tmp");
        write(&sibling, "x");
        let o = opt(vec![files_rule(&dir, &["*"])]);
        let found = RuleFound {
            files: vec![(dir.join("a.tmp"), 1), (outside.clone(), 9), (sibling.clone(), 1)],
            dirs_to_remove: vec![t.path().join(r"profile\Documents")],
            count: 3,
            bytes: 11,
            ..Default::default()
        };
        let reg = FakeReg::default();
        let done = clean_option(&found, &o, &env, &Guard::new(&env), &reg, &AtomicBool::new(false));
        assert!(!dir.join("a.tmp").exists());
        assert!(outside.exists());
        assert!(sibling.exists());
        assert!(t.path().join(r"profile\Documents").exists());
        assert_eq!((done.removed, done.failed), (1, 2), "{:?}", done.notes);
        assert!(done.notes.iter().any(|n| n.contains("outside the rule's folders")), "{:?}", done.notes);
    }

    /// I4: la lista dei processi si rilegge subito prima dell'analisi e subito
    /// prima della pulizia di ogni opzione.
    #[test]
    fn processes_are_read_again_before_each_option() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let a_dir = t.path().join(r"profile\.a\cache");
        let b_dir = t.path().join(r"profile\.b\cache");
        write(&a_dir.join("a.tmp"), "x");
        write(&b_dir.join("b.tmp"), "x");
        let mut a = cleaner(&["a.exe"], vec![opt(vec![files_rule(&a_dir, &["*"])])]);
        a.id = "a".into();
        let mut b = cleaner(&["b.exe"], vec![opt(vec![files_rule(&b_dir, &["*"])])]);
        b.id = "b".into();
        let chosen = vec![(a.clone(), a.options[0].clone()), (b.clone(), b.options[0].clone())];
        // b.exe parte dopo l'analisi di B, prima della sua pulizia (4ª lettura).
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let procs = || {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= 4 { vec!["b.exe".to_string()] } else { vec!["explorer.exe".to_string()] }
        };
        let reg = FakeReg::default();
        let guard = Guard::new(&env);
        let cancel = AtomicBool::new(false);
        let run = RunCtx { env: &env, guard: &guard, reg: &reg, reg_exists: &never, processes: &procs, cancel: &cancel };
        let mut rows = Vec::new();
        run_options(&chosen, true, &run, &|_, _| {}, &mut |key, _, f, c| rows.push((key, f, c)));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(!a_dir.join("a.tmp").exists());
        assert!(b_dir.join("b.tmp").exists());
        assert_eq!(rows.len(), 2);
        let b_done = rows[1].2.as_ref().unwrap();
        assert_eq!(b_done.removed, 0);
        assert!(b_done.notes.iter().any(|n| n.contains("Close b.exe")), "{:?}", b_done.notes);
        // Solo analisi: una lettura per opzione.
        calls.store(0, Ordering::SeqCst);
        let mut rows = Vec::new();
        run_options(&chosen, false, &run, &|_, _| {}, &mut |key, _, f, c| rows.push((key, f, c)));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(rows.iter().all(|r| r.2.is_none()));
    }

    /// M9: le opzioni bloccate non abilitano «Avvia pulizia» e non mostrano avvisi.
    #[test]
    fn blocked_choices_do_not_enable_cleaning_or_warn() {
        let mut free = opt(vec![]);
        free.id = "free".into();
        free.warning = Some("perdi la cronologia".into());
        let mut busy = cleaner(&["claude.exe"], vec![free.clone()]);
        busy.id = "busy".into();
        let mut ok = cleaner(&[], vec![free.clone()]);
        ok.id = "ok".into();
        ok.label = "Altro".into();
        let cleaners = vec![busy.clone(), ok.clone()];
        let running = vec!["claude.exe".to_string()];
        let only_busy: std::collections::HashSet<String> = [key(&busy, &free)].into();
        assert!(active_choices(&cleaners, &only_busy, &running).is_empty());
        assert!(choice_warnings(&active_choices(&cleaners, &only_busy, &running)).is_empty());
        let both: std::collections::HashSet<String> = [key(&busy, &free), key(&ok, &free)].into();
        let active = active_choices(&cleaners, &both, &running);
        assert_eq!(active.len(), 1);
        assert_eq!(choice_warnings(&active), vec!["Altro – Opzione: perdi la cronologia"]);
    }

    /// M6: l'ambiente del processo per le regole solo con --screenshot e solo
    /// nella build di debug.
    #[test]
    fn process_env_only_for_debug_screenshots() {
        assert!(!rules_env_from_process(false));
        assert_eq!(rules_env_from_process(true), cfg!(debug_assertions));
    }

    // ---------- backup del registro (C1, I5, M2, M10) ----------

    /// Registro finto per `backup_to`: `exists` dalla lista, `export` scrive un
    /// `.reg` UTF-16 come `reg.exe`.
    struct FakeBackend {
        existing: Vec<&'static str>,
        broken: Vec<&'static str>,
        exports: Mutex<Vec<PathBuf>>,
    }

    impl FakeBackend {
        fn new(existing: Vec<&'static str>) -> FakeBackend {
            FakeBackend { existing, broken: vec![], exports: Mutex::new(vec![]) }
        }
    }

    impl RegBackend for FakeBackend {
        fn exists(&self, item: &RegItem) -> Result<bool, String> {
            let p = item_text(item);
            if self.broken.iter().any(|b| *b == p) {
                return Err("accesso negato".into());
            }
            Ok(self.existing.iter().any(|e| *e == p))
        }
        fn export(&self, root: RegRoot, path: &str, out: &Path) -> Result<(), String> {
            let text = format!(
                "Windows Registry Editor Version 5.00\r\n\r\n[{}\\{path}]\r\n\"x\"=\"1\"\r\n",
                root_name(root)
            );
            let mut data = vec![0xFF, 0xFE];
            for w in text.encode_utf16() {
                data.extend_from_slice(&w.to_le_bytes());
            }
            fs::write(out, data).map_err(|e| e.to_string())?;
            self.exports.lock().unwrap().push(out.to_path_buf());
            Ok(())
        }
    }

    fn key_item(path: &str) -> RegItem {
        RegItem::Key { root: RegRoot::Hkcu, path: path.into() }
    }

    /// Cartella di lavoro finta (in produzione: solo amministratori, nome casuale).
    fn work_in(root: &Path) -> impl Fn() -> Result<PathBuf, String> + '_ {
        move || {
            let d = root.join(format!("work{}", std::process::id()));
            let mut n = 0;
            loop {
                let cand = d.with_extension(n.to_string());
                if fs::create_dir(&cand).is_ok() {
                    return Ok(cand);
                }
                n += 1;
            }
        }
    }

    #[test]
    fn two_backups_in_the_same_second_get_two_files() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("Documenti");
        fs::create_dir_all(&dest).unwrap();
        let work_root = t.path().join("admin");
        fs::create_dir_all(&work_root).unwrap();
        let be = FakeBackend::new(vec![r"Software\FakeApp\A", r"Software\FakeApp\B"]);
        let work = work_in(&work_root);
        let first = backup_to(&dest, "backup_regole_20261111_120000", &work, &be, &[key_item(r"Software\FakeApp\A")]).unwrap();
        let second = backup_to(&dest, "backup_regole_20261111_120000", &work, &be, &[key_item(r"Software\FakeApp\B")]).unwrap();
        let (p1, p2) = (first.path.unwrap(), second.path.unwrap());
        assert_ne!(p1, p2);
        assert!(p1.exists() && p2.exists());
        assert_eq!(p2.file_name().unwrap(), "backup_regole_20261111_120000_2.reg");
        let text = |p: &Path| {
            let b = fs::read(p).unwrap();
            let w: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            String::from_utf16(&w).unwrap()
        };
        assert!(text(&p1).contains(r"[HKCU\Software\FakeApp\A]"));
        assert!(text(&p2).contains(r"[HKCU\Software\FakeApp\B]"));
        // I file temporanei e le cartelle di lavoro non restano.
        assert_eq!(fs::read_dir(&work_root).unwrap().count(), 0);
        assert!(be.exports.lock().unwrap().iter().all(|p| !p.exists()));
    }

    #[test]
    fn backup_fails_closed_without_an_admin_work_folder() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("Documenti");
        fs::create_dir_all(&dest).unwrap();
        let be = FakeBackend::new(vec![r"Software\FakeApp\A"]);
        let no_work = || Err::<PathBuf, String>("cartella di lavoro non protetta".into());
        let r = backup_to(&dest, "b", &no_work, &be, &[key_item(r"Software\FakeApp\A")]);
        assert!(r.is_err());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        assert!(be.exports.lock().unwrap().is_empty());
    }

    /// M2: una chiave che non si apre per errore non è «assente»: il backup fallisce.
    #[test]
    fn a_key_that_cannot_be_opened_fails_the_backup() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("Documenti");
        fs::create_dir_all(&dest).unwrap();
        let mut be = FakeBackend::new(vec![r"Software\FakeApp\A"]);
        be.broken = vec![r"Software\FakeApp\B"];
        let work = work_in(t.path());
        let r = backup_to(&dest, "b", &work, &be, &[key_item(r"Software\FakeApp\A"), key_item(r"Software\FakeApp\B")]);
        assert!(r.is_err(), "{r:?}");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    /// M10 in `backup_to`: nessuna chiave esistente → nessun file.
    #[test]
    fn backup_of_absent_keys_writes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("Documenti");
        fs::create_dir_all(&dest).unwrap();
        let be = FakeBackend::new(vec![]);
        let work = work_in(t.path());
        let b = backup_to(&dest, "b", &work, &be, &[key_item(r"Software\FakeApp\A")]).unwrap();
        assert_eq!(b.path, None);
        assert_eq!(b.present, vec![false]);
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    /// I5: la cartella di destinazione che è un collegamento (giunzione) si rifiuta.
    #[test]
    fn backup_refuses_a_destination_that_is_a_link() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("altrove");
        fs::create_dir_all(&target).unwrap();
        let dest = t.path().join("Documenti");
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&dest)
            .arg(&target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("giunzione non creata: test saltato");
            return;
        }
        let be = FakeBackend::new(vec![r"Software\FakeApp\A"]);
        let work = work_in(t.path());
        let r = backup_to(&dest, "b", &work, &be, &[key_item(r"Software\FakeApp\A")]);
        assert!(r.is_err(), "{r:?}");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    // ---------- registro vero: logica pura e sola lettura ----------

    /// I1: solo il collegamento `HKCU\Software\Classes` → `\REGISTRY\USER\<SID>_Classes`
    /// del SID del processo si segue.
    #[test]
    fn only_the_user_classes_link_is_followed() {
        let sid = "S-1-5-21-1-2-3-1001";
        let target = r"\REGISTRY\USER\S-1-5-21-1-2-3-1001_Classes";
        assert!(classes_link_ok(RegRoot::Hkcu, &["Software", "Classes"], target, sid));
        assert!(classes_link_ok(RegRoot::Hkcu, &["SOFTWARE", "classes"], &target.to_lowercase(), sid));
        assert!(!classes_link_ok(RegRoot::Hklm, &["Software", "Classes"], target, sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Classes", "X"], target, sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Other"], target, sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Classes"], r"\REGISTRY\USER\S-1-5-18_Classes", sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Classes"], r"\REGISTRY\MACHINE\SOFTWARE\Classes", sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Classes"], &format!("{target}\\x"), sid));
        assert!(!classes_link_ok(RegRoot::Hkcu, &["Software", "Classes"], target, ""));
    }

    /// M4: padre e nome dai componenti normalizzati.
    #[test]
    fn registry_parent_and_name_come_from_normalized_components() {
        assert_eq!(split_parent(r"Software\Foo\"), Some((r"Software".to_string(), "Foo".to_string())));
        assert_eq!(split_parent(r"\Software\\Foo\Bar"), Some((r"Software\Foo".to_string(), "Bar".to_string())));
        assert_eq!(split_parent(r"Software\"), None);
        assert_eq!(split_parent(""), None);
    }

    /// M1: solo la fine dell'elenco ferma l'esame dell'albero senza rifiuto.
    #[test]
    fn subkey_enumeration_errors_refuse_the_tree() {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_KEY_DELETED, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
        assert_eq!(enum_verdict(ERROR_SUCCESS), None);
        assert_eq!(enum_verdict(ERROR_NO_MORE_ITEMS), Some(false));
        for rc in [ERROR_ACCESS_DENIED, ERROR_KEY_DELETED, ERROR_MORE_DATA] {
            assert_eq!(enum_verdict(rc), Some(true));
        }
    }

    /// Sola lettura del registro vero: `HKCU\Software\Classes` (collegamento
    /// verso `HKU\<SID>_Classes`) si apre, come le sue sottochiavi; analisi e
    /// pulizia usano la stessa apertura.
    #[test]
    fn real_registry_opens_user_classes_through_its_link() {
        assert!(win::key_exists(RegRoot::Hkcu, r"Software\Classes"));
        assert!(win::key_exists(RegRoot::Hkcu, r"Software\Classes\Local Settings"));
        assert!(reg_exists(&RegItem::Key { root: RegRoot::Hkcu, path: r"Software\Classes\Local Settings".into() }));
        assert!(reg_exists(&RegItem::Value {
            root: RegRoot::Hklm,
            path: r"Software\Microsoft\Windows NT\CurrentVersion".into(),
            value: "ProductName".into(),
        }));
        assert!(!reg_exists(&RegItem::Value {
            root: RegRoot::Hklm,
            path: r"Software\Microsoft\Windows NT\CurrentVersion".into(),
            value: "xiox-no-such-value".into(),
        }));
    }

    // ---------- aggiornamento di Winapp2 e cartelle dell'app (Task 8) ----------

    use xiox_rules::updater::{Fetch, Fetched, Protect, UpdateOutcome};
    use std::cell::RefCell;
    use std::sync::mpsc::channel;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Winapp2 finto con una voce sola, per `load_with`.
    fn ini_reader(version: &'static str) -> impl Fn(&Path) -> Result<(String, String), String> {
        move |_: &Path| {
            let ini = format!(
                "; Version: {version}\r\n[Other App *]\r\nSection=Utilities\r\nFileKey1=%UserProfile%\\.otherapp\\cache|*.*|RECURSE\r\n"
            );
            Ok((ini, version.to_string()))
        }
    }

    /// Rete finta lenta: dorme e poi fallisce (nessuna scrittura dopo il sonno).
    struct SlowFetch(Duration);

    impl Fetch for SlowFetch {
        fn get(&self, _: &str, _: Option<&str>, _: Duration) -> Result<Fetched, String> {
            std::thread::sleep(self.0);
            Err("rete finta".into())
        }
    }

    /// Protezione finta: crea le cartelle, nessuna ACL vera.
    struct NoAcl;

    impl Protect for NoAcl {
        fn secure(&self, dir: &Path) -> Result<(), String> {
            fs::create_dir_all(dir).map_err(|e| e.to_string())
        }
        fn check(&self, _: &Path) -> Result<(), String> {
            Ok(())
        }
        fn check_file(&self, _: &fs::File) -> Result<(), String> {
            Ok(())
        }
        fn quarantine(&self, _: &Path) -> Result<(), String> {
            Err("non nei test".into())
        }
    }

    #[test]
    fn status_text_for_each_outcome() {
        let text = |o: UpdateOutcome| status_text(Winapp2Status::Outcome(&o));
        assert_eq!(text(UpdateOutcome::Updated { version: "261001".into() }), "Winapp2 261001 updated");
        assert_eq!(text(UpdateOutcome::UpToDate), "Winapp2 already up to date");
        assert_eq!(text(UpdateOutcome::KeptOld("timed out".into())), "Winapp2 not updated: timed out");
        assert_eq!(text(UpdateOutcome::NoCopyYet("timed out".into())), "Winapp2 not downloaded yet");
        assert_eq!(status_text(Winapp2Status::Loaded("260915")), "Winapp2 260915 loaded");
    }

    #[test]
    fn status_line_follows_load_and_update() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let old = load_with(&t.path().join("rules"), &env, &ini_reader("260915"), &never);
        let none = load_with(&t.path().join("rules"), &env, &no_copy, &never);
        // Aggiornamento in corso: la copia caricata.
        assert_eq!(status_line(&old, None), ("Winapp2 260915 loaded".to_string(), None));
        assert_eq!(status_line(&none, None).0, "Winapp2 not downloaded yet");
        // Scaricato ma non ancora ricaricato: resta "caricato" finché non arriva il nuovo RuleSet.
        let updated = UpdateOutcome::Updated { version: "261001".into() };
        assert_eq!(status_line(&old, Some(&updated)).0, "Winapp2 260915 loaded");
        let new = load_with(&t.path().join("rules"), &env, &ini_reader("261001"), &never);
        assert_eq!(status_line(&new, Some(&updated)).0, "Winapp2 261001 updated");
        assert_eq!(status_line(&old, Some(&UpdateOutcome::UpToDate)).0, "Winapp2 already up to date");
        // Nessuna copia: il motivo al passaggio del mouse.
        let (text, hover) = status_line(&none, Some(&UpdateOutcome::NoCopyYet("timed out".into())));
        assert_eq!(text, "Winapp2 not downloaded yet");
        assert_eq!(hover.as_deref(), Some("timed out"));
    }

    #[test]
    fn about_text_is_the_winapp2_attribution() {
        assert_eq!(
            ABOUT_RULES,
            "The cleaning rules for applications come from Winapp2 (https://github.com/MoscaDotTo/Winapp2), licence CC-BY-SA-4.0, downloaded at every start."
        );
        assert!(!ABOUT_RULES.to_lowercase().contains("bleachbit"));
    }

    #[test]
    fn no_update_in_screenshot_mode() {
        assert!(!should_update(true));
        assert!(should_update(false));
    }

    #[test]
    fn update_outcome_updated_triggers_a_reload_message() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join("rules");
        let msgs: RefCell<Vec<RulesMsg>> = RefCell::new(vec![]);
        let notify = |m: RulesMsg| msgs.borrow_mut().push(m);
        after_update(
            UpdateOutcome::Updated { version: "261001".into() },
            || load_with(&dir, &env, &ini_reader("261001"), &never),
            &notify,
        );
        {
            let got = msgs.borrow();
            assert_eq!(got.len(), 2);
            assert!(matches!(&got[0], RulesMsg::Updated(UpdateOutcome::Updated { version }) if version == "261001"));
            assert!(matches!(&got[1], RulesMsg::Loaded(rs) if rs.winapp2_version.as_deref() == Some("261001")));
        }
        // Gli altri esiti: solo il messaggio, nessun ricaricamento.
        for o in [
            UpdateOutcome::UpToDate,
            UpdateOutcome::KeptOld("timed out".into()),
            UpdateOutcome::NoCopyYet("timed out".into()),
        ] {
            msgs.borrow_mut().clear();
            after_update(o.clone(), || panic!("nessun ricaricamento per {o:?}"), &notify);
            let got = msgs.borrow();
            assert_eq!(got.len(), 1);
            assert!(matches!(&got[0], RulesMsg::Updated(x) if *x == o));
        }
    }

    #[test]
    fn start_reloads_with_the_same_env_after_an_update() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let own = t.path().join(r"ProgramData\Xiox");
        let seen: Arc<std::sync::Mutex<Vec<Env>>> = Arc::default();
        let seen_in = Arc::clone(&seen);
        let (tx, rx) = channel();
        let update: UpdateSpawner = Box::new(|_dir, done| done(UpdateOutcome::Updated { version: "261001".into() }));
        let own_in = own.clone();
        start(
            t.path().join("rules"),
            move |_: &Env| vec![own_in],
            move || env,
            move |d: &Path, e: &Env| {
                seen_in.lock().unwrap().push(e.clone());
                load_with(d, e, &ini_reader("261001"), &never)
            },
            Some(update),
            move |m| {
                let _ = tx.send(m);
            },
        );
        let wait = || rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(matches!(wait(), RulesMsg::Loaded(_)));
        assert!(matches!(wait(), RulesMsg::Updated(UpdateOutcome::Updated { .. })));
        assert!(matches!(wait(), RulesMsg::Loaded(_)));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1]);
        assert!(Guard::new(&seen[0]).protected(&own.join("winapp2").join("Winapp2.ini")).is_some());
    }

    #[test]
    fn closing_does_not_wait_for_the_update() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        let dir = t.path().join(r"ProgramData\Xiox\winapp2");
        let (tx, rx) = channel();
        let begun = Instant::now();
        let update: UpdateSpawner = Box::new(|dir, done| {
            xiox_rules::updater::spawn_update_with(
                dir,
                Arc::new(SlowFetch(Duration::from_secs(5))),
                Arc::new(NoAcl),
                "https://example.invalid/Winapp2.ini".into(),
                done,
            )
        });
        start(
            dir,
            |_: &Env| vec![],
            move || env,
            |d: &Path, e: &Env| load_with(d, e, &no_copy, &never),
            Some(update),
            move |m| {
                let _ = tx.send(m);
            },
        );
        assert!(begun.elapsed() < Duration::from_secs(1), "start ha aspettato: {:?}", begun.elapsed());
        // Le regole arrivano senza aspettare la rete; l'esito no.
        let first = rx.recv_timeout(Duration::from_secs(4)).unwrap();
        assert!(matches!(first, RulesMsg::Loaded(_)));
        assert!(rx.try_recv().is_err());
        assert!(begun.elapsed() < Duration::from_secs(5));
        // Fine del test senza `join`: il thread dell'aggiornamento resta staccato.
    }

    #[test]
    fn own_data_folders_are_protected() {
        let t = tempfile::tempdir().unwrap();
        let profile = t.path().join("profile");
        let mut base = fake_env(t.path());
        base.0.insert("documents".into(), profile.join("Documents").to_string_lossy().into_owned());
        let app_data = t.path().join(r"ProgramData\Xiox");
        let backups = profile.join(r"Documents\Xiox");
        // Le cartelle con il nome di prima (rinomina non ancora fatta o non riuscita) sono protette uguale.
        let old_app_data = t.path().join(r"ProgramData\PulisciPC");
        let old_backups = profile.join(r"Documents\PulisciPC");
        let files = [
            app_data.join(r"winapp2\Winapp2.ini"),
            app_data.join(r"winapp2\etag.txt"),
            app_data.join(r"winapp2\tmp\xiox-curl-1\body.bin"),
            backups.join("backup_regole_20261110_120000.reg"),
            backups.join("servizi_originali.txt"),
            old_app_data.join(r"winapp2\Winapp2.ini"),
            old_backups.join("backup_registro_20261001_090000.reg"),
        ];
        for f in &files {
            write(f, "dati");
        }
        let mut o = opt(vec![
            files_rule(&app_data, &["*.*"]),
            files_rule(&app_data.join("winapp2"), &["*.*"]),
            files_rule(&app_data.join(r"winapp2\tmp"), &["*"]),
            files_rule(&backups, &["*.reg", "*.txt"]),
            files_rule(&old_app_data.join("winapp2"), &["*.*"]),
            files_rule(&old_backups, &["*.reg", "*.txt"]),
        ]);
        o.actions.push(Action::Files {
            path: app_data.join("winapp2").to_string_lossy().into_owned(),
            patterns: vec!["Winapp2.ini".into()],
            recurse: false,
            remove_self: true,
            search: Search::Winapp2,
        });
        // Senza la protezione l'opzione finta raccoglierebbe i file.
        assert!(ctl_run(&o, &base).count > 0);

        let mut env = base.clone();
        let own = own_folders_with(&app_data, Some(&profile.to_string_lossy()), &env);
        assert!(own.contains(&app_data) && own.contains(&backups), "{own:?}");
        assert!(own.contains(&old_app_data) && own.contains(&old_backups), "{own:?}");
        protect_own_folders(&mut env, &own);
        let found = ctl_run(&o, &env);
        assert_eq!((found.count, found.dirs_to_remove.len()), (0, 0), "{found:?}");
        // Anche un `RuleFound` costruito a mano con quei file non li cancella.
        let forged = RuleFound {
            files: files.iter().map(|f| (f.clone(), 4)).collect(),
            dirs_to_remove: vec![
                app_data.join(r"winapp2\tmp"),
                app_data.join("winapp2"),
                app_data.clone(),
                backups.clone(),
                old_app_data.clone(),
                old_backups.clone(),
            ],
            count: files.len() as u64,
            ..Default::default()
        };
        let done = clean_option(&forged, &o, &env, &Guard::new(&env), &FakeReg::default(), &AtomicBool::new(false));
        assert_eq!((done.removed, done.failed), (0, files.len() as u64), "{:?}", done.notes);
        for f in &files {
            assert!(f.exists(), "cancellato: {}", f.display());
        }
    }

    /// Sola lettura del registro vero: si apre una chiave che c'è sempre,
    /// componente per componente, senza seguire collegamenti.
    #[test]
    fn real_registry_opens_keys_without_following_links() {
        assert!(win::key_exists(RegRoot::Hklm, r"Software\Microsoft\Windows NT\CurrentVersion"));
        assert!(!win::key_exists(RegRoot::Hkcu, r"Software\xiox-no-such-key"));
    }

    // ---------- prova del registro vero da amministratore (ignorata) ----------

    /// Chiavi di prova e backup creati da [`real_registry_round_trip_as_admin`]:
    /// all'uscita (anche per un panico) si tolgono le chiavi con il nome del
    /// test e i `.reg` restituiti dal test che contengono quel nome.
    #[cfg(windows)]
    struct VerifyCleanup {
        marker: String,
        keys: Vec<(RegRoot, String)>,
        backups: Vec<PathBuf>,
    }

    #[cfg(windows)]
    impl Drop for VerifyCleanup {
        fn drop(&mut self) {
            use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_WOW64_64KEY};
            for (root, path) in &self.keys {
                // Solo le chiavi del test: il nome contiene il marcatore unico.
                let Some((parent, name)) = split_parent(path) else { continue };
                if !name.starts_with(&self.marker) {
                    continue;
                }
                let hive = winreg::RegKey::predef(match root {
                    RegRoot::Hkcu => HKEY_CURRENT_USER,
                    RegRoot::Hklm => HKEY_LOCAL_MACHINE,
                });
                if let Ok(p) = hive.open_subkey_with_flags(&parent, KEY_ALL_ACCESS | KEY_WOW64_64KEY) {
                    let _ = p.delete_subkey_all(&name);
                }
            }
            for b in &self.backups {
                if backup_text(b).is_some_and(|t| t.contains(&self.marker)) {
                    let _ = fs::remove_file(b);
                }
            }
        }
    }

    /// Testo di un `.reg` UTF-16 (con BOM).
    #[cfg(windows)]
    fn backup_text(path: &Path) -> Option<String> {
        let b = fs::read(path).ok()?;
        let w: Vec<u16> = b.get(2..)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        Some(String::from_utf16_lossy(&w))
    }

    /// Crea `root\path` (vista a 64 bit; `Software\Wow6432Node\…` per la 32 bit)
    /// con i valori dati.
    #[cfg(windows)]
    fn create_key(root: RegRoot, path: &str, values: &[(&str, &str)]) {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_WOW64_64KEY};
        let hive = winreg::RegKey::predef(match root {
            RegRoot::Hkcu => HKEY_CURRENT_USER,
            RegRoot::Hklm => HKEY_LOCAL_MACHINE,
        });
        let (k, _) = hive
            .create_subkey_with_flags(path, KEY_ALL_ACCESS | KEY_WOW64_64KEY)
            .unwrap_or_else(|e| panic!("chiave di prova non creata {path}: {e}"));
        for (n, v) in values {
            k.set_value(n, &v.to_string()).unwrap();
        }
    }

    #[cfg(windows)]
    fn analyze_real(o: &Opt, env: &Env) -> RuleFound {
        let cancel = AtomicBool::new(false);
        let tick = |_: u64| {};
        analyze_option(o, env, &reg_exists, &CollectCtl { cancel: &cancel, tick: &tick })
    }

    #[cfg(windows)]
    fn clean_real(found: &RuleFound, o: &Opt, env: &Env, cleanup: &mut VerifyCleanup) -> RuleCleaned {
        let done = clean_option(found, o, env, &Guard::new(env), &RealRegistry, &AtomicBool::new(false));
        if let Some(b) = &done.backup {
            cleanup.backups.push(b.clone());
        }
        done
    }

    /// Prova sul registro vero, da un terminale da amministratore:
    /// `cargo test -p xiox-core real_registry_round_trip_as_admin -- --ignored --nocapture`.
    /// Usa solo chiavi create dal test (`Xiox_Verify_<pid>_<nanos>`) e le
    /// toglie all'uscita, anche dopo un panico. Prova: backup con `reg.exe` nella
    /// cartella solo per amministratori, cancellazione vera di chiavi e valori,
    /// `HKCU\Software\Classes` (collegamento), vista a 32 bit di HKLM, due backup
    /// nello stesso secondo, rifiuto di una chiave protetta.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn real_registry_round_trip_as_admin() {
        assert!(sys::is_admin(), "serve un terminale da amministratore: il test non ha fatto nulla");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let marker = format!("Xiox_Verify_{}_{nanos}", std::process::id());
        let env = Env::default();
        let key_a = format!(r"Software\{marker}_A");
        let key_b = format!(r"Software\{marker}_B");
        let key_c = format!(r"Software\Classes\{marker}_C");
        let key_d64 = format!(r"Software\{marker}_D");
        let key_d32 = format!(r"Software\Wow6432Node\{marker}_D");
        let key_e1 = format!(r"Software\{marker}_E1");
        let key_e2 = format!(r"Software\{marker}_E2");
        let mut cleanup = VerifyCleanup {
            marker: marker.clone(),
            keys: vec![
                (RegRoot::Hkcu, key_a.clone()),
                (RegRoot::Hkcu, key_b.clone()),
                (RegRoot::Hkcu, key_c.clone()),
                (RegRoot::Hklm, key_d32.clone()),
                (RegRoot::Hklm, key_d64.clone()),
                (RegRoot::Hkcu, key_e1.clone()),
                (RegRoot::Hkcu, key_e2.clone()),
            ],
            backups: vec![],
        };
        let key_rule = |root, path: &str| opt(vec![Action::RegKey { root, path: path.to_string() }]);
        let key_exists_now = |root, path: &str| reg_exists(&RegItem::Key { root, path: path.to_string() });

        // (a) chiave con una sottochiave e due valori.
        create_key(RegRoot::Hkcu, &key_a, &[("Uno", "1"), ("Due", "2")]);
        create_key(RegRoot::Hkcu, &format!(r"{key_a}\Sotto"), &[("Tre", "3")]);
        let o = key_rule(RegRoot::Hkcu, &key_a);
        let found = analyze_real(&o, &env);
        assert_eq!(found.reg, vec![RegItem::Key { root: RegRoot::Hkcu, path: key_a.clone() }], "(a) analisi");
        let done = clean_real(&found, &o, &env, &mut cleanup);
        eprintln!("(a) {done:?}");
        assert_eq!((done.removed, done.failed), (1, 0), "(a) {:?}", done.notes);
        let bk = done.backup.clone().expect("(a) nessun backup");
        assert!(bk.starts_with(sys::data_dir()), "(a) backup fuori da Documenti\\Xiox: {}", bk.display());
        assert!(backup_text(&bk).is_some_and(|t| t.contains(&format!("{marker}_A"))), "(a) backup senza la chiave");
        assert!(!key_exists_now(RegRoot::Hkcu, &key_a), "(a) la chiave c'è ancora");

        // (b) un solo valore: sparisce lui, il resto resta.
        create_key(RegRoot::Hkcu, &key_b, &[("Via", "x"), ("Resta", "y")]);
        let o = opt(vec![Action::RegValue { root: RegRoot::Hkcu, path: key_b.clone(), value: "Via".into() }]);
        let found = analyze_real(&o, &env);
        assert_eq!(found.reg.len(), 1, "(b) analisi");
        let done = clean_real(&found, &o, &env, &mut cleanup);
        eprintln!("(b) {done:?}");
        assert_eq!((done.removed, done.failed), (1, 0), "(b) {:?}", done.notes);
        let value = |name: &str| RegItem::Value { root: RegRoot::Hkcu, path: key_b.clone(), value: name.into() };
        assert!(!reg_exists(&value("Via")), "(b) il valore c'è ancora");
        assert!(reg_exists(&value("Resta")), "(b) l'altro valore è sparito");
        assert!(key_exists_now(RegRoot::Hkcu, &key_b), "(b) la chiave è sparita");

        // (c) sotto HKCU\Software\Classes (collegamento verso HKU\<SID>_Classes).
        create_key(RegRoot::Hkcu, &key_c, &[("Mru", "z")]);
        let o = key_rule(RegRoot::Hkcu, &key_c);
        let found = analyze_real(&o, &env);
        assert_eq!(found.reg.len(), 1, "(c) analisi: {found:?}");
        let done = clean_real(&found, &o, &env, &mut cleanup);
        eprintln!("(c) {done:?}");
        assert_eq!((done.removed, done.failed), (1, 0), "(c) {:?}", done.notes);
        let bk = done.backup.clone().expect("(c) nessun backup");
        assert!(backup_text(&bk).is_some_and(|t| t.contains(&format!("{marker}_C"))), "(c) backup senza la chiave");
        assert!(!key_exists_now(RegRoot::Hkcu, &key_c), "(c) la chiave c'è ancora");

        // (d) HKLM solo nella vista a 32 bit, regola scritta senza Wow6432Node.
        create_key(RegRoot::Hklm, &key_d32, &[("Recent", "r")]);
        assert!(!key_exists_now(RegRoot::Hklm, &key_d64), "(d) la chiave esiste anche a 64 bit");
        let o = key_rule(RegRoot::Hklm, &key_d64);
        let found = analyze_real(&o, &env);
        assert_eq!(found.reg, vec![RegItem::Key { root: RegRoot::Hklm, path: key_d32.clone() }], "(d) analisi");
        let done = clean_real(&found, &o, &env, &mut cleanup);
        eprintln!("(d) {done:?}");
        assert_eq!((done.removed, done.failed), (1, 0), "(d) {:?}", done.notes);
        let bk = done.backup.clone().expect("(d) nessun backup");
        assert!(backup_text(&bk).is_some_and(|t| t.contains(&format!("{marker}_D"))), "(d) backup senza la chiave");
        assert!(!key_exists_now(RegRoot::Hklm, &key_d32), "(d) la chiave a 32 bit c'è ancora");
        // Seconda pulizia della stessa raccolta: niente da togliere.
        let again = clean_real(&found, &o, &env, &mut cleanup);
        eprintln!("(d) di nuovo {again:?}");
        assert_eq!((again.removed, again.failed), (0, 0), "(d) di nuovo {:?}", again.notes);
        assert!(again.notes.iter().any(|n| n.starts_with("Already gone")), "(d) di nuovo {:?}", again.notes);
        assert_eq!(again.backup, None, "(d) backup di una chiave assente");

        // (e) due pulizie di fila: due backup diversi.
        create_key(RegRoot::Hkcu, &key_e1, &[("x", "1")]);
        create_key(RegRoot::Hkcu, &key_e2, &[("x", "2")]);
        let (o1, o2) = (key_rule(RegRoot::Hkcu, &key_e1), key_rule(RegRoot::Hkcu, &key_e2));
        let (f1, f2) = (analyze_real(&o1, &env), analyze_real(&o2, &env));
        let d1 = clean_real(&f1, &o1, &env, &mut cleanup);
        let d2 = clean_real(&f2, &o2, &env, &mut cleanup);
        eprintln!("(e) {:?} {:?}", d1.backup, d2.backup);
        let (b1, b2) = (d1.backup.clone().expect("(e) primo"), d2.backup.clone().expect("(e) secondo"));
        assert_ne!(b1, b2, "(e) stesso file di backup");
        assert!(backup_text(&b1).is_some_and(|t| t.contains(&format!("{marker}_E1"))), "(e) primo backup");
        assert!(backup_text(&b2).is_some_and(|t| t.contains(&format!("{marker}_E2"))), "(e) secondo backup");
        assert_eq!((d1.removed, d2.removed), (1, 1));

        // (f) chiave protetta: solo rifiuto. Prima di tutto `reg_allowed` deve
        // rifiutarla, altrimenti il test si ferma qui senza chiamare nulla.
        let protected_path = r"Software\Microsoft\Windows\CurrentVersion\Run";
        assert!(
            reg_allowed(RegRoot::Hklm, protected_path, None).is_err(),
            "(f) reg_allowed non rifiuta {protected_path}: test fermato senza cancellare"
        );
        assert!(key_exists_now(RegRoot::Hklm, protected_path), "(f) la chiave protetta non c'è su questo PC");
        let o = key_rule(RegRoot::Hklm, protected_path);
        let found = analyze_real(&o, &env);
        assert!(found.reg.is_empty() && found.skipped == 1, "(f) analisi: {found:?}");
        // Anche un `RuleFound` costruito a mano: `clean_option` la rifiuta prima del backup.
        let forged = RuleFound {
            reg: vec![RegItem::Key { root: RegRoot::Hklm, path: protected_path.into() }],
            count: 1,
            ..Default::default()
        };
        let done = clean_real(&forged, &o, &env, &mut cleanup);
        eprintln!("(f) {done:?}");
        assert_eq!((done.removed, done.failed), (0, 1), "(f) {:?}", done.notes);
        assert_eq!(done.backup, None);
        assert!(key_exists_now(RegRoot::Hklm, protected_path), "(f) la chiave protetta è sparita");
        drop(cleanup);
    }

    // ----- voci native -----

    fn native_none(_: &crate::native::NativeCtx) -> crate::native::NativeFound {
        crate::native::NativeFound::default()
    }

    fn native_nothing(_: &crate::native::NativeCtx, _: &crate::native::NativeFound) -> crate::native::NativeCleaned {
        crate::native::NativeCleaned::default()
    }

    fn native(id: &'static str, group: &'static str, running: &'static [&'static str]) -> NativeItem {
        NativeItem {
            id,
            group,
            label: "Voce nativa",
            description: "",
            warning: Some("attenzione"),
            warning_extra: None,
            default_on: id == "acceso",
            needs_admin: false,
            running,
            needs_preview: false,
            analyze: native_none,
            clean: native_nothing,
        }
    }

    #[test]
    fn native_item_is_blocked_while_its_process_runs() {
        let item = native("db", "Browser", &["Firefox.exe", "thunderbird"]);
        assert_eq!(native_blocked(&item, &["explorer.exe".to_string()]), None);
        assert_eq!(
            native_blocked(&item, &["firefox.EXE".to_string()]),
            Some("Close Firefox.exe to clean".to_string())
        );
        assert_eq!(
            native_blocked(&item, &["Thunderbird.exe".to_string(), "firefox.exe".to_string()]),
            Some("Close Firefox.exe, thunderbird to clean".to_string())
        );
        // Bloccata = non abilita «Avvia pulizia» e non dà avvisi.
        let items = [item, native("acceso", "Windows", &[])];
        let sel: HashSet<String> = items.iter().map(native_key).collect();
        let procs = ["firefox.exe".to_string()];
        let active = active_natives(&items, &sel, &procs);
        assert_eq!(active.iter().map(|i| i.id).collect::<Vec<_>>(), vec!["acceso"]);
        assert_eq!(native_warnings(&active), vec!["Voce nativa: attenzione".to_string()]);
        assert_eq!(native_key(&items[1]), "native:acceso");
        assert_eq!(native_defaults(&items), HashSet::from(["native:acceso".to_string()]));
        // Testo calcolato al momento della conferma (cookie: «lista: N domini»).
        let with_extra = NativeItem { warning_extra: Some(|| "lista: 0 domini".to_string()), ..items[1] };
        assert_eq!(native_warnings(&[&with_extra]), vec!["Voce nativa: attenzione (lista: 0 domini)".to_string()]);
        assert_eq!(running_among(&["Chrome.exe", "msedge"], &["MSEDGE.EXE".to_string()]), vec!["msedge"]);
    }

    #[test]
    fn confirm_says_what_preview_items_will_touch() {
        let plain = native("a", "Windows", &[]);
        let seen = NativeItem { needs_preview: true, label: "Vista", ..native("b", "Windows", &[]) };
        let never = NativeItem { needs_preview: true, label: "Mai vista", ..native("c", "Windows", &[]) };
        let empty = NativeItem { needs_preview: true, label: "Vuota", ..native("d", "Windows", &[]) };
        let shown = |i: &NativeItem| match i.id {
            "b" => Some(3),
            "d" => Some(0),
            _ => None,
        };
        let notes = preview_notes(&[&plain, &seen, &never, &empty], &shown);
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].starts_with("Vista:") && notes[0].contains("3 items"), "{notes:?}");
        assert!(notes[1].starts_with("Mai vista:") && notes[1].contains(crate::native::NO_PREVIEW), "{notes:?}");
        assert!(notes[2].starts_with("Vuota:") && notes[2].contains("nothing is touched"), "{notes:?}");
    }

    #[test]
    fn native_items_join_their_group_before_the_xml_cleaners() {
        let natives = [native("a", "Windows", &[]), native("b", "Windows", &[])];
        let groups = vec![("Windows".to_string(), vec![0]), ("Browser".to_string(), vec![1]), (EMBEDDED_GROUP.to_string(), vec![2])];
        let got = panel_groups(&groups, &natives);
        assert_eq!(got[0], ("Windows".to_string(), vec![0, 1], vec![0]));
        assert_eq!(got.len(), 3);
        // Nessun cleaner XML di Windows presente: il gruppo c'è lo stesso, primo.
        let groups = vec![("Browser".to_string(), vec![0]), (EMBEDDED_GROUP.to_string(), vec![1])];
        let got = panel_groups(&groups, &natives);
        let names: Vec<&str> = got.iter().map(|g| g.0.as_str()).collect();
        assert_eq!(names, vec!["Windows", "Browser", EMBEDDED_GROUP]);
        assert_eq!(got[0], ("Windows".to_string(), vec![0, 1], vec![]));
        // Senza regole caricate.
        assert_eq!(panel_groups(&[], &natives), vec![("Windows".to_string(), vec![0, 1], vec![])]);
    }
}
