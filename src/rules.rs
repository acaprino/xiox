//! Categorie dinamiche di Pulizia file: i cleaner inglobati (CleanerML,
//! gruppo "Sviluppo e AI") e le voci di Winapp2 ("Applicazioni").
//!
//! Nessuna UI qui: `main.rs` mostra le caselle e lancia il job; questo modulo
//! carica le regole, raccoglie ciò che un'opzione toglierebbe e lo cancella
//! seguendo il contratto di `pc_rules` (vedi `crates/pc-rules/src/lib.rs`):
//! file e cartelle solo con `delete_checked_with` e un solo `Guard` per
//! pulizia; registro ricontrollato con `reg_allowed`, backup `.reg` prima,
//! chiavi aperte senza seguire i collegamenti.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pc_rules::{
    collect_ctl, overlaps, present, reg_allowed, running, updater, winapp2, Action, Cleaner,
    CollectCtl, Env, Guard, Opt, Origin, RegItem, RegRoot,
};

/// Titolo del gruppo dei cleaner inglobati.
pub const EMBEDDED_GROUP: &str = "Sviluppo e AI";
/// Titolo del gruppo delle voci di Winapp2.
pub const WINAPP2_GROUP: &str = "Applicazioni";

/// Note tenute per un'opzione nel resoconto.
const MAX_NOTES: usize = 20;

/// Regole pronte per la scheda.
pub struct RuleSet {
    /// Prima gli inglobati presenti, poi le voci di Winapp2 rilevate.
    pub cleaners: Vec<Cleaner>,
    /// Per la pagina Informazioni (Task 8); la scheda mostra `status`.
    #[allow(dead_code)]
    pub winapp2_version: Option<String>,
    pub status: String,
    pub warnings: Vec<String>,
    /// Ambiente con cui si sono lette le regole: lo stesso per analisi e pulizia.
    pub env: Env,
    /// Sezioni di Winapp2 (`Cleaner::group`, in ordine alfabetico) con gli
    /// indici delle voci in `cleaners`.
    pub sections: Vec<(String, Vec<usize>)>,
}

pub enum RulesMsg {
    Loaded(RuleSet),
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
    /// Scrive un `.reg` con le chiavi di `items`. `Err` se anche una sola
    /// chiave esistente non si salva.
    fn backup(&self, items: &[RegItem]) -> Result<PathBuf, String>;
    /// Cancella una chiave (con le sottochiavi) o un valore.
    fn delete(&self, item: &RegItem) -> Result<(), String>;
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
    load_with(dir, env, &|d| updater::read_valid(d), &pc_rules::reg_exists_real)
}

/// Come [`load`], con lettura di Winapp2 e registro iniettati.
pub fn load_with(
    dir: &Path,
    env: &Env,
    read_winapp2: &ReadWinapp2,
    reg_exists: &dyn Fn(&RegItem) -> bool,
) -> RuleSet {
    let (all, mut warnings) = pc_rules::embedded(env);
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
                    "Winapp2: {dropped} regole scartate (percorsi protetti o variabili non definite)"
                ));
            }
            if report.skipped_excludes > 0 {
                warnings.push(format!(
                    "Winapp2: {} voci non offerte (esclusioni non supportate)",
                    report.skipped_excludes
                ));
            }
            // Regola 9: una voce che tocca le stesse cartelle di un inglobato
            // presente si nasconde; vince l'inglobato.
            let embedded_paths: Vec<&str> = cleaners.iter().flat_map(file_paths).collect();
            let before = found.len();
            winapp = found
                .into_iter()
                .filter(|w| {
                    !file_paths(w).any(|a| embedded_paths.iter().any(|b| overlaps(a, b, env)))
                })
                .collect();
            let hidden = before - winapp.len();
            if hidden > 0 {
                let what = if hidden == 1 { "1 voce".to_string() } else { format!("{hidden} voci") };
                warnings.push(format!("Winapp2: {what} nascoste perché già coperte da {EMBEDDED_GROUP}"));
            }
            (format!("Winapp2 {version} caricato"), Some(version))
        }
        // `read_valid` lo dice così quando il file non c'è ancora.
        Err(e) if e == "nessuna copia locale" => ("Winapp2 non ancora scaricato".to_string(), None),
        Err(e) => (format!("Winapp2 non letto: {e}"), None),
    };
    let first = cleaners.len();
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
    }
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
        return Some(format!("{s}: questa opzione non si può usare"));
    }
    let active = running(cleaner, processes);
    if active.is_empty() {
        None
    } else {
        Some(format!("Chiudi {} per pulire", active.join(", ")))
    }
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

/// Pulisce ciò che `analyze_option` ha raccolto: i file, poi le cartelle
/// svuotate (nell'ordine dato), poi il registro (backup `.reg` prima; se il
/// backup non riesce nessuna voce di registro si cancella). Non conosce la
/// UI né i processi: chi chiama usa [`clean_checked`].
pub fn clean_option(
    found: &RuleFound,
    opt: &Opt,
    guard: &Guard,
    reg: &dyn RuleRegistry,
    cancel: &AtomicBool,
) -> RuleCleaned {
    let mut out = RuleCleaned::default();
    if opt.unsupported.is_some() {
        out.note("Opzione non supportata: non pulita".to_string());
        return out;
    }
    if found.cancelled {
        out.note("Analisi interrotta: niente pulito".to_string());
        return out;
    }
    for (path, size) in &found.files {
        if cancel.load(Ordering::Relaxed) {
            out.note("Pulizia interrotta".to_string());
            return out;
        }
        match delete_path(path, guard) {
            Ok(()) => {
                out.removed += 1;
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
            out.note("Pulizia interrotta".to_string());
            return out;
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
                out.note(format!("Registro non toccato: {} ({why})", reg_text(item)));
            }
        }
    }
    if items.is_empty() || cancel.load(Ordering::Relaxed) {
        return out;
    }
    match reg.backup(&items) {
        Ok(path) => out.backup = Some(path),
        Err(e) => {
            out.failed += items.len() as u64;
            out.note(format!(
                "Backup del registro non riuscito ({e}): nessuna voce di registro eliminata"
            ));
            return out;
        }
    }
    for item in &items {
        match reg.delete(item) {
            Ok(()) => out.removed += 1,
            Err(e) => {
                out.failed += 1;
                out.note(format!("{}: {e}", reg_text(item)));
            }
        }
    }
    out
}

/// Come [`clean_option`], ma prima ricontrolla [`blocked`] con l'elenco dei
/// processi preso subito prima della pulizia: un'opzione bloccata si salta e
/// lo si scrive nel resoconto.
pub fn clean_checked(
    cleaner: &Cleaner,
    opt: &Opt,
    found: &RuleFound,
    processes: &[String],
    guard: &Guard,
    reg: &dyn RuleRegistry,
    cancel: &AtomicBool,
) -> RuleCleaned {
    if let Some(why) = blocked(cleaner, opt, processes) {
        let mut out = RuleCleaned::default();
        out.note(format!("Saltata: {why}"));
        return out;
    }
    clean_option(found, opt, guard, reg, cancel)
}

/// L'unica via con cui questo modulo cancella file e cartelle.
fn delete_path(path: &Path, guard: &Guard) -> Result<(), String> {
    pc_rules::delete_checked_with(path, guard).map(|_| ())
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

/// Il registro vero.
pub struct RealRegistry;

impl RuleRegistry for RealRegistry {
    fn backup(&self, items: &[RegItem]) -> Result<PathBuf, String> {
        win::backup(items)
    }
    fn delete(&self, item: &RegItem) -> Result<(), String> {
        win::delete(item)
    }
}

/// Nomi degli eseguibili in esecuzione (`claude.exe`, …), via ToolHelp.
pub fn processes() -> Vec<String> {
    win::processes()
}

mod win {
    use std::path::PathBuf;

    use pc_rules::{RegItem, RegRoot};
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY,
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE,
        KEY_WOW64_64KEY, REG_LINK, REG_OPTION_OPEN_LINK,
    };

    use super::{reg_parts, reg_text, root_name};
    use crate::sys;

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

    /// `true` se la chiave (aperta con `REG_OPTION_OPEN_LINK`) è un
    /// collegamento simbolico del registro.
    fn is_link(key: &Key) -> bool {
        let name = wide("SymbolicLinkValue");
        let mut ty = 0u32;
        // SAFETY: si chiede solo il tipo, nessun buffer di dati.
        let rc = unsafe {
            RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), std::ptr::null_mut())
        };
        rc == ERROR_SUCCESS && ty == REG_LINK
    }

    fn hive(root: RegRoot) -> HKEY {
        match root {
            RegRoot::Hkcu => HKEY_CURRENT_USER,
            RegRoot::Hklm => HKEY_LOCAL_MACHINE,
        }
    }

    /// Apre `path` un componente alla volta, ognuno senza seguire collegamenti
    /// e rifiutando un componente che è un collegamento. L'ultimo si apre con
    /// `last_access`. `Ok(None)` se la chiave non esiste.
    fn open_chain(root: RegRoot, path: &str, last_access: u32) -> Result<Option<Key>, String> {
        let comps: Vec<&str> = path.split('\\').filter(|c| !c.is_empty()).collect();
        if comps.is_empty() {
            return Err("percorso di registro vuoto".into());
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
                Err(rc) => return Err(format!("non si apre (errore {rc})")),
            };
            if is_link(&key) {
                return Err("collegamento del registro: non si segue".into());
            }
            cur = Some(key);
        }
        Ok(cur)
    }

    pub fn key_exists(root: RegRoot, path: &str) -> bool {
        matches!(open_chain(root, path, KEY_READ), Ok(Some(_)))
    }

    /// `true` se sotto `key` c'è un collegamento, una sottochiave che non si
    /// apre o più sottochiavi del budget.
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
            if rc != ERROR_SUCCESS {
                return false;
            }
            index += 1;
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            match open_one(key.0, &name, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS) {
                Ok(child) => {
                    if is_link(&child) || tree_has_link(&child, budget) {
                        return true;
                    }
                }
                Err(_) => return true,
            }
        }
    }

    pub fn delete(item: &RegItem) -> Result<(), String> {
        let (root, path, value) = reg_parts(item);
        // Contratto di pc-rules: si ricontrolla subito prima di cancellare.
        pc_rules::reg_allowed(root, path, value).map_err(|why| format!("{} ({why})", reg_text(item)))?;
        match value {
            Some(v) => {
                let Some(key) = open_chain(root, path, KEY_SET_VALUE)? else {
                    return Ok(());
                };
                let w = wide(v);
                // SAFETY: chiave aperta con KEY_SET_VALUE, nome terminato da 0.
                let rc = unsafe { RegDeleteValueW(key.0, w.as_ptr()) };
                if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("valore non eliminato (errore {rc})"))
                }
            }
            None => {
                let (parent_path, name) = path.rsplit_once('\\').ok_or("chiave senza padre")?;
                let Some(parent) = open_chain(root, parent_path, KEY_READ | KEY_SET_VALUE | DELETE)? else {
                    return Ok(());
                };
                let target = match open_one(parent.0, name, KEY_READ) {
                    Ok(k) => k,
                    Err(ERROR_FILE_NOT_FOUND) => return Ok(()),
                    Err(rc) => return Err(format!("non si apre (errore {rc})")),
                };
                let mut budget = TREE_BUDGET;
                if is_link(&target) || tree_has_link(&target, &mut budget) {
                    return Err("contiene un collegamento del registro o è troppo grande: non toccata".into());
                }
                drop(target);
                let w = wide(name);
                // SAFETY: padre aperto con DELETE e lettura, nome terminato da 0.
                let rc = unsafe { RegDeleteTreeW(parent.0, w.as_ptr()) };
                if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("chiave non eliminata (errore {rc})"))
                }
            }
        }
    }

    /// Un `.reg` con tutte le chiavi toccate (`reg export`, vista a 64 bit),
    /// nella stessa cartella dei backup del Registro. Una chiave che esiste e
    /// non si esporta fa fallire tutto.
    pub fn backup(items: &[RegItem]) -> Result<PathBuf, String> {
        let mut keys: Vec<(RegRoot, String)> = items
            .iter()
            .map(|i| {
                let (root, path, _) = reg_parts(i);
                (root, path.to_string())
            })
            .collect();
        keys.sort_by_key(|(r, p)| (root_name(*r), p.to_lowercase()));
        keys.dedup_by(|a, b| a.0 == b.0 && a.1.eq_ignore_ascii_case(&b.1));
        let dir = sys::data_dir();
        let out = dir.join(format!("backup_regole_{}.reg", sys::stamp()));
        let tmp = dir.join("export_regole_tmp.reg");
        let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
        for (root, path) in keys {
            if !key_exists(root, &path) {
                continue; // niente da salvare e niente da cancellare
            }
            let _ = std::fs::remove_file(&tmp);
            let full = format!("{}\\{}", root_name(root), path);
            let st = sys::hidden_cmd("reg").args(["export", &full]).arg(&tmp).args(["/y", "/reg:64"]).status();
            if !matches!(st, Ok(s) if s.success()) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("{full} non esportata"));
            }
            let bytes = std::fs::read(&tmp).map_err(|e| format!("{full}: {e}"))?;
            let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let s = String::from_utf16_lossy(&words);
            let s = s.trim_start_matches('\u{feff}');
            let body = s.split_once('\n').map(|x| x.1).unwrap_or("");
            text.push_str(body);
        }
        let _ = std::fs::remove_file(&tmp);
        let mut data = vec![0xFF, 0xFE];
        for w in text.encode_utf16() {
            data.extend_from_slice(&w.to_le_bytes());
        }
        std::fs::write(&out, data).map_err(|e| e.to_string())?;
        Ok(out)
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
    use pc_rules::{Action, Origin, RegRoot, Search};
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
        Err("nessuna copia locale".to_string())
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

    /// Registro finto: registra le chiamate, il backup può fallire.
    #[derive(Default)]
    struct FakeReg {
        log: Mutex<Vec<String>>,
        fail_backup: bool,
    }

    fn item_text(item: &RegItem) -> String {
        match item {
            RegItem::Key { path, .. } => path.clone(),
            RegItem::Value { path, value, .. } => format!("{path}|{value}"),
        }
    }

    impl RuleRegistry for FakeReg {
        fn backup(&self, items: &[RegItem]) -> Result<PathBuf, String> {
            self.log.lock().unwrap().push(format!("backup:{}", items.len()));
            if self.fail_backup {
                Err("reg export non riuscito".into())
            } else {
                Ok(PathBuf::from(r"C:\finto\backup.reg"))
            }
        }
        fn delete(&self, item: &RegItem) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("delete:{}", item_text(item)));
            Ok(())
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
        assert_eq!(rs.status, "Winapp2 non ancora scaricato");
        assert_eq!(rs.winapp2_version, None);
        // Cargo installato: compare solo lui.
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        let rs = load_with(&t.path().join("rules"), &env, &no_copy, &never);
        assert_eq!(ids(&rs), vec!["bbag_cargo"]);
        assert!(rs.cleaners.iter().all(|c| c.group == EMBEDDED_GROUP));
        assert!(rs.sections.is_empty());
    }

    #[test]
    fn winapp2_entry_covering_an_embedded_tool_is_hidden() {
        let t = tempfile::tempdir().unwrap();
        let env = fake_env(t.path());
        write(&t.path().join(r"profile\.cargo\registry\cache\index\a.crate"), "x");
        write(&t.path().join(r"profile\.otherapp\cache\b.tmp"), "x");
        let ini = "; Version: 260915\r\n\
                   [Cargo Registry *]\r\nSection=Development\r\nFileKey1=%UserProfile%\\.cargo\\registry|*.*|RECURSE\r\n\r\n\
                   [Other App *]\r\nSection=Utilities\r\nFileKey1=%UserProfile%\\.otherapp\\cache|*.*|RECURSE\r\n";
        let read = |_: &Path| Ok((ini.to_string(), "260915".to_string()));
        let rs = load_with(&t.path().join("rules"), &env, &read, &never);
        assert_eq!(ids(&rs), vec!["bbag_cargo", "winapp2:Other App"]);
        assert_eq!(rs.status, "Winapp2 260915 caricato");
        assert_eq!(rs.winapp2_version.as_deref(), Some("260915"));
        assert_eq!(rs.sections, vec![("Utilities".to_string(), vec![1])]);
        assert!(rs.warnings.iter().any(|w| w.contains("1 voce")), "{:?}", rs.warnings);
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
        let done = clean_option(&found, &o, &guard, &reg, &AtomicBool::new(false));
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
        let done = clean_option(&found, &o, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(
            *reg.log.lock().unwrap(),
            vec!["backup:2", r"delete:Software\FakeApp\Recent", r"delete:Software\FakeApp|LastFile"]
        );
        assert_eq!((done.removed, done.failed), (2, 0));
        assert_eq!(done.backup, Some(PathBuf::from(r"C:\finto\backup.reg")));
        // Backup non riuscito: nessuna cancellazione, e lo si dice.
        let reg = FakeReg { fail_backup: true, ..Default::default() };
        let done = clean_option(&found, &o, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(*reg.log.lock().unwrap(), vec!["backup:2"]);
        assert_eq!((done.removed, done.failed), (0, 2));
        assert!(done.notes.iter().any(|n| n.contains("Backup del registro non riuscito")), "{:?}", done.notes);
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
            Some("Chiudi claude.exe per pulire")
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
            &Guard::new(&env),
            &reg,
            &AtomicBool::new(false),
        );
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        assert!(done.notes.iter().any(|n| n.contains("Chiudi claude.exe")), "{:?}", done.notes);
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
        let done = clean_option(&found, &o, &guard, &reg, &AtomicBool::new(false));
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        let done = clean_checked(&c, &o, &found, &[], &guard, &reg, &AtomicBool::new(false));
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
        let done = clean_option(&found, &opt(vec![]), &Guard::new(&env), &reg, &AtomicBool::new(false));
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
        let done = clean_option(&found, &o, &Guard::new(&env), &reg, &AtomicBool::new(false));
        assert_eq!(done.removed, 0);
        assert!(dir.join("a.tmp").exists());
        assert!(reg.log.lock().unwrap().is_empty());
    }

    /// Sola lettura del registro vero: si apre una chiave che c'è sempre,
    /// componente per componente, senza seguire collegamenti.
    #[test]
    fn real_registry_opens_keys_without_following_links() {
        assert!(win::key_exists(RegRoot::Hklm, r"Software\Microsoft\Windows NT\CurrentVersion"));
        assert!(!win::key_exists(RegRoot::Hkcu, r"Software\pulisci-pc-no-such-key"));
    }
}
