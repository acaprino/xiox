//! N5 «Percorso ambiente» (`docs/ccleaner-coverage.md` §2.6): toglie dalla
//! variabile `Path` dell'utente (`HKCU\Environment`) e di sistema
//! (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`) le
//! cartelle che mancano davvero e i doppioni.
//!
//! - **Cosa si propone**: una voce ripetuta (confronto dopo l'espansione di
//!   `%VAR%`, senza maiuscole e senza `\` finali; si tiene la prima) o una
//!   cartella che manca davvero (`sysclean::truly_missing`: errore 2 o 3,
//!   nessun antenato punto di reparse) su un disco fisso presente
//!   (`GetDriveTypeW == DRIVE_FIXED`). Mai: variabili non espandibili, UNC e
//!   percorsi `\\?\`, relativi, con `..` o con virgolette in mezzo, dischi
//!   rimovibili, di rete o assenti, voci vuote, voci sotto `%SystemRoot%`.
//!   I doppioni si cercano dentro la stessa chiave (vedi il report: si
//!   cancella meno). Il PATH di sistema non si svuota mai.
//! - **Scrittura**: backup `.reg` delle due chiavi (`regbackup`, fail-closed:
//!   backup fallito = niente scrittura), poi per ogni chiave si rilegge il
//!   valore e, solo se è identico a quello dell'analisi, lo si riscrive con lo
//!   stesso tipo (`REG_SZ`/`REG_EXPAND_SZ`) e le voci rimaste nell'ordine di
//!   prima (i `;;` vuoti spariscono); infine `WM_SETTINGCHANGE` a tutte le
//!   finestre.
//!
//! Ogni accesso al sistema passa dal trait [`PathSys`]; i test usano un finto.

use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};

use pc_rules::{Env, RegRoot};

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};
use crate::regbackup;

/// Chiave dell'ambiente dell'utente (sotto `HKCU`).
pub const HKCU_KEY: &str = "Environment";
/// Chiave dell'ambiente di sistema (sotto `HKLM`).
pub const HKLM_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
/// Nome del valore.
pub const VALUE: &str = "Path";
/// Le due chiavi, nell'ordine in cui Windows compone il PATH.
pub const ROOTS: [RegRoot; 2] = [RegRoot::Hklm, RegRoot::Hkcu];
/// Righe di dettaglio tenute nella tabella.
const MAX_DETAIL: usize = 50;

const WARNING: &str = "Toglie dal PATH le cartelle che non esistono più e i doppioni. Un programma che si aspetta una \
                       cartella su un disco non ancora collegato potrebbe non trovarla. Backup .reg in Documenti\\PulisciPC.";

/// Tipo del valore `Path` nel registro (si riscrive sempre con lo stesso).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sz,
    ExpandSz,
}

/// Il valore `Path` di una chiave: tipo e testo (senza lo zero finale).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathValue {
    pub kind: Kind,
    pub text: String,
}

/// Registro, dischi, backup e avviso a Windows.
pub trait PathSys {
    /// `Path` della chiave di `root`. `Ok(None)` se il valore (o la chiave)
    /// non c'è; `Err` se non si legge, se non è `REG_SZ`/`REG_EXPAND_SZ` o se
    /// il testo non è UTF-16 valido: allora la chiave non si tocca.
    fn read(&self, root: RegRoot) -> Result<Option<PathValue>, String>;
    /// Riscrive `Path` con il tipo e il testo dati.
    fn write(&self, root: RegRoot, value: &PathValue) -> Result<(), String>;
    /// Backup `.reg` delle due chiavi; `Err` = niente scrittura.
    fn backup(&self) -> Result<PathBuf, String>;
    /// `true` solo per un disco fisso presente (`GetDriveTypeW == DRIVE_FIXED`).
    fn volume_is_fixed(&self, p: &Path) -> bool;
    /// `true` solo se la cartella manca davvero (vedi `sysclean::truly_missing`).
    fn truly_missing(&self, p: &Path) -> bool;
    /// `WM_SETTINGCHANGE` con `"Environment"` a tutte le finestre.
    fn broadcast(&self) -> Result<(), String>;
}

/// Perché si propone di togliere una voce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Duplicate,
    Missing,
}

/// Una voce del PATH da togliere: posizione fra le voci separate da `;`
/// (vuote comprese), testo così come è scritto, motivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub index: usize,
    pub entry: String,
    pub reason: Reason,
}

// ---------------------------------------------------------------------------
// Voce.

pub fn items() -> Vec<NativeItem> {
    vec![NativeItem {
        id: "env_path",
        group: "Windows",
        label: "Percorso ambiente",
        description: "Toglie dalla variabile PATH, dell'utente e di sistema, le cartelle che non esistono più su un \
                      disco fisso e quelle scritte due volte. Le cartelle di Windows, di rete, su dischi rimovibili \
                      o scollegati e quelle con variabili sconosciute non si toccano mai. Prima di scrivere si salva \
                      un backup .reg delle due chiavi.",
        warning: Some(WARNING),
        warning_extra: None,
        default_on: false,
        needs_admin: true,
        running: &[],
        analyze: real_analyze,
        clean: real_clean,
    }]
}

fn real_analyze(ctx: &NativeCtx) -> NativeFound {
    analyze_path(&win::RealPath, ctx.env)
}

fn real_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
    clean_path(&win::RealPath, ctx.env, found, ctx.cancel)
}

/// Nome della chiave per il resoconto.
pub fn root_label(root: RegRoot) -> &'static str {
    match root {
        RegRoot::Hklm => "Sistema",
        RegRoot::Hkcu => "Utente",
    }
}

// ---------------------------------------------------------------------------
// Testo del PATH e del registro.

/// Voci di un PATH separate da `;` fuori dalle virgolette (vuote comprese).
/// `None` se le virgolette non si chiudono: allora il valore non si tocca.
pub fn split_entries(text: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if quoted {
        return None;
    }
    out.push(&text[start..]);
    Some(out)
}

/// Testo di un valore `REG_SZ`/`REG_EXPAND_SZ`: UTF-16 little endian, gli
/// zeri finali tolti. Lunghezza dispari, zero in mezzo o UTF-16 non valido
/// danno `Err` (riscrivendo si perderebbe qualcosa).
pub fn decode_reg_text(data: &[u8]) -> Result<String, String> {
    if !data.len().is_multiple_of(2) {
        return Err("valore con lunghezza dispari".into());
    }
    let mut words: Vec<u16> = data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    while words.last() == Some(&0) {
        words.pop();
    }
    if words.contains(&0) {
        return Err("valore con un carattere nullo in mezzo".into());
    }
    String::from_utf16(&words).map_err(|_| "valore con testo UTF-16 non valido".to_string())
}

/// `text` in UTF-16 little endian con un solo zero finale.
pub fn encode_reg_text(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// Il testo con le voci `remove` tolte, le altre nell'ordine di prima, senza
/// voci vuote; il `;` finale resta se c'era.
fn rebuild(entries: &[&str], remove: &[usize]) -> String {
    let kept: Vec<&str> = entries
        .iter()
        .enumerate()
        .filter(|(i, e)| !e.is_empty() && !remove.contains(i))
        .map(|(_, e)| *e)
        .collect();
    let mut text = kept.join(";");
    if entries.len() > 1 && entries.last() == Some(&"") && !kept.is_empty() {
        text.push(';');
    }
    text
}

// ---------------------------------------------------------------------------
// Cosa si propone.

/// Le voci da togliere dal valore `text` della chiave di `root`. `Err` (con
/// il motivo) quando il valore intero non si tocca: virgolette non chiuse,
/// cartella di Windows sconosciuta, PATH di sistema che resterebbe vuoto.
pub fn proposals(root: RegRoot, text: &str, sys: &dyn PathSys, env: &Env) -> Result<Vec<Proposal>, String> {
    let entries = split_entries(text).ok_or("virgolette non chiuse, non si tocca")?;
    let windows: Vec<PathBuf> = ["systemroot", "windir"]
        .iter()
        .filter_map(|v| env.get(v))
        .map(PathBuf::from)
        .filter(|p| disk_path(p))
        .collect();
    if windows.is_empty() {
        return Err("cartella di Windows sconosciuta, non si tocca".into());
    }
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (index, raw) in entries.iter().enumerate() {
        // Nel PATH di sistema una variabile per utente vale per ogni account
        // in modo diverso: la voce non si giudica e non fa da «prima» di un
        // doppione.
        let per_user = root == RegRoot::Hklm && uses_per_user_var(raw);
        let expanded = if per_user { None } else { expanded_entry(raw, env) };
        let mut duplicate = false;
        if let Some(e) = &expanded {
            let key = norm_key(e);
            if seen.contains(&key) {
                duplicate = true;
            } else {
                seen.push(key);
            }
        }
        let Some(p) = expanded.as_deref().and_then(|e| judgeable(e, &windows)) else { continue };
        if !sys.volume_is_fixed(&p) {
            continue;
        }
        let reason = if duplicate {
            Reason::Duplicate
        } else if sys.truly_missing(&p) {
            Reason::Missing
        } else {
            continue;
        };
        out.push(Proposal { index, entry: raw.to_string(), reason });
    }
    if root == RegRoot::Hklm && !out.is_empty() && entries.iter().filter(|e| !e.is_empty()).count() == out.len() {
        return Err("il PATH di sistema resterebbe vuoto, non si tocca".into());
    }
    Ok(out)
}

/// Variabili che valgono per utente (come in `sysclean` per i collegamenti
/// comuni).
const PER_USER_VARS: &[&str] = &[
    "userprofile", "appdata", "localappdata", "locallowappdata", "homepath", "homedrive", "homeshare",
    "temp", "tmp", "onedrive", "onedriveconsumer", "onedrivecommercial", "username", "userdomain",
];

fn uses_per_user_var(raw: &str) -> bool {
    let low = raw.to_lowercase();
    PER_USER_VARS.iter().any(|v| low.contains(&format!("%{v}%")))
}

/// La voce senza spazi intorno, senza le virgolette che la racchiudono e con
/// le `%VAR%` espanse. `None` se è vuota, ha virgolette in mezzo, una
/// variabile sconosciuta o un `%` aperto, o se l'espansione dà `;`, `%` o `"`.
fn expanded_entry(raw: &str, env: &Env) -> Option<String> {
    let t = raw.trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    if t.is_empty() || t.contains('"') {
        return None;
    }
    let e = expand_vars(t, env)?;
    (!e.contains([';', '%', '"'])).then_some(e)
}

/// `%Var%` sostituite con l'ambiente; `None` se una variabile non c'è o un
/// `%` resta aperto.
fn expand_vars(s: &str, env: &Env) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let j = after.find('%')?;
        let name = &after[..j];
        if name.is_empty() {
            return None;
        }
        out.push_str(env.get(name)?);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Chiave di confronto dei doppioni: minuscole, senza `\` finali (ma `C:\`
/// resta `c:\`, diverso da `C:`).
fn norm_key(e: &str) -> String {
    let mut k = e.to_lowercase();
    while k.ends_with('\\') && !(k.len() == 3 && k.as_bytes()[1] == b':') {
        k.pop();
    }
    k
}

/// `C:\…`: lettera di unità e radice, non `\\?\`, non UNC, non relativo.
fn disk_path(p: &Path) -> bool {
    let mut c = p.components();
    matches!(c.next(), Some(Component::Prefix(pre)) if matches!(pre.kind(), Prefix::Disk(_)))
        && matches!(c.next(), Some(Component::RootDir))
}

/// Il percorso da giudicare, se la voce espansa `e` si può proporre: `C:\…`,
/// senza `..`, senza componenti che finiscono con `.` o spazio (Windows li
/// toglierebbe e il confronto con `%SystemRoot%` sbaglierebbe), non dentro
/// (né uguale a) una delle cartelle di Windows `windows`.
fn judgeable(e: &str, windows: &[PathBuf]) -> Option<PathBuf> {
    let p = Path::new(e);
    if !disk_path(p) {
        return None;
    }
    for c in p.components() {
        match c {
            Component::ParentDir => return None,
            Component::Normal(n) => {
                let n = n.to_string_lossy();
                if n.ends_with('.') || n.ends_with(' ') {
                    return None;
                }
            }
            _ => {}
        }
    }
    let pc = lower_comps(p);
    if windows.iter().any(|w| {
        let wc = lower_comps(w);
        wc.len() >= 2 && pc.starts_with(&wc)
    }) {
        return None;
    }
    Some(p.to_path_buf())
}

fn lower_comps(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Radice del disco (`C:\`) per `GetDriveTypeW`.
fn drive_root(p: &Path) -> Option<String> {
    match p.components().next()? {
        Component::Prefix(pre) => match pre.kind() {
            Prefix::Disk(d) => Some(format!("{}:\\", (d as char).to_ascii_uppercase())),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Elementi di `NativeFound::items`.

/// Un elemento: chiave, tipo e testo del valore letto dall'analisi, posizione
/// e testo della voce. Formato `ROOT\tTIPO\tINDICE\tLUNGHEZZA\t<voce><valore>`
/// (la lunghezza in byte della voce separa voce e valore, che possono
/// contenere qualunque carattere).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    root: RegRoot,
    original: PathValue,
    index: usize,
    entry: String,
}

fn kind_tag(k: Kind) -> &'static str {
    match k {
        Kind::Sz => "sz",
        Kind::ExpandSz => "expand",
    }
}

fn encode_item(root: RegRoot, original: &PathValue, p: &Proposal) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}{}",
        regbackup::root_name(root),
        kind_tag(original.kind),
        p.index,
        p.entry.len(),
        p.entry,
        original.text
    )
}

fn parse_item(s: &str) -> Option<Item> {
    let mut it = s.splitn(5, '\t');
    let root = match it.next()? {
        "HKLM" => RegRoot::Hklm,
        "HKCU" => RegRoot::Hkcu,
        _ => return None,
    };
    let kind = match it.next()? {
        "sz" => Kind::Sz,
        "expand" => Kind::ExpandSz,
        _ => return None,
    };
    let index: usize = it.next()?.parse().ok()?;
    let len: usize = it.next()?.parse().ok()?;
    let rest = it.next()?;
    let (entry, text) = (rest.get(..len)?, rest.get(len..)?);
    Some(Item { root, original: PathValue { kind, text: text.to_string() }, index, entry: entry.to_string() })
}

// ---------------------------------------------------------------------------
// Analisi e pulizia.

fn reason_text(r: Reason) -> &'static str {
    match r {
        Reason::Missing => "non esiste più",
        Reason::Duplicate => "doppione",
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Legge le due chiavi e dice cosa si toglierebbe. Non scrive nulla.
pub fn analyze_path(sys: &dyn PathSys, env: &Env) -> NativeFound {
    let mut found = NativeFound::default();
    let mut lines: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for root in ROOTS {
        let label = root_label(root);
        let value = match sys.read(root) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                notes.push(format!("{label}: PATH non letto, non si tocca ({e})"));
                continue;
            }
        };
        let ps = match proposals(root, &value.text, sys, env) {
            Ok(ps) => ps,
            Err(e) => {
                notes.push(format!("{label}: {e}"));
                continue;
            }
        };
        for p in &ps {
            found.count += 1;
            if lines.len() < MAX_DETAIL {
                lines.push(format!("{label}: {} ({})", p.entry, reason_text(p.reason)));
            }
            found.items.push(encode_item(root, &value, p));
        }
        let entries = split_entries(&value.text).unwrap_or_default();
        if !ps.is_empty() && entries.iter().rev().skip(1).any(|e| e.is_empty()) {
            notes.push(format!("{label}: si tolgono anche i separatori «;;» vuoti"));
        }
    }
    if found.count > 0 {
        found.detail.push(plural(found.count, "cartella da togliere dal PATH", "cartelle da togliere dal PATH"));
        found.detail.extend(lines);
        if found.count as usize > MAX_DETAIL {
            found.detail.push(format!("… e altre {}", found.count as usize - MAX_DETAIL));
        }
    }
    found.detail.extend(notes);
    found
}

/// Ciò che si scrive in una chiave.
struct Plan {
    root: RegRoot,
    original: PathValue,
    wanted: Vec<(usize, String)>,
    /// Elementi della stessa chiave con valori diversi: non si tocca.
    bad: bool,
}

fn same_value(read: Result<Option<PathValue>, String>, original: &PathValue) -> bool {
    matches!(read, Ok(Some(v)) if v == *original)
}

/// Toglie le voci di `found.items`, ciascuna solo se è ancora da togliere nel
/// valore attuale, che deve essere identico a quello dell'analisi. Backup
/// prima (fallito = niente scrittura), riletto subito prima di scrivere,
/// avviso a Windows dopo.
pub fn clean_path(sys: &dyn PathSys, env: &Env, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let mut plans: Vec<Plan> = Vec::new();
    for s in &found.items {
        let Some(it) = parse_item(s) else {
            out.failed += 1;
            out.note("Elemento del PATH non valido, lasciato".into());
            continue;
        };
        match plans.iter_mut().find(|p| p.root == it.root) {
            Some(p) => {
                p.bad |= p.original != it.original;
                p.wanted.push((it.index, it.entry));
            }
            None => plans.push(Plan { root: it.root, original: it.original, wanted: vec![(it.index, it.entry)], bad: false }),
        }
    }

    // Cosa resta da togliere, sul valore riletto adesso.
    let mut ready: Vec<(RegRoot, PathValue, PathValue, Vec<String>)> = Vec::new();
    for plan in plans {
        if cancel.load(Ordering::Relaxed) {
            return out;
        }
        let label = root_label(plan.root);
        let n = plan.wanted.len() as u64;
        if plan.bad {
            out.failed += n;
            out.note(format!("{label}: elementi non coerenti, PATH lasciato"));
            continue;
        }
        if !same_value(sys.read(plan.root), &plan.original) {
            out.failed += n;
            out.note(format!("{label}: PATH cambiato, rifai l'analisi"));
            continue;
        }
        let current = match proposals(plan.root, &plan.original.text, sys, env) {
            Ok(c) => c,
            Err(e) => {
                out.failed += n;
                out.note(format!("{label}: {e}"));
                continue;
            }
        };
        let mut remove: Vec<usize> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for (index, entry) in &plan.wanted {
            if current.iter().any(|p| p.index == *index && p.entry == *entry) {
                if !remove.contains(index) {
                    remove.push(*index);
                    removed.push(entry.clone());
                }
            } else {
                out.note(format!("{label}: {entry} non è più da togliere, lasciata"));
            }
        }
        if remove.is_empty() {
            continue;
        }
        let entries = split_entries(&plan.original.text).unwrap_or_default();
        let new = PathValue { kind: plan.original.kind, text: rebuild(&entries, &remove) };
        ready.push((plan.root, plan.original, new, removed));
    }
    if ready.is_empty() || cancel.load(Ordering::Relaxed) {
        return out;
    }

    match sys.backup() {
        Ok(path) => out.note(format!("Backup del PATH: {}", path.display())),
        Err(e) => {
            out.failed += ready.iter().map(|r| r.3.len() as u64).sum::<u64>();
            out.note(format!("Backup .reg non riuscito, il PATH non si tocca: {e}"));
            return out;
        }
    }

    let mut wrote = false;
    for (root, original, new, removed) in ready {
        let label = root_label(root);
        if cancel.load(Ordering::Relaxed) {
            out.note(format!("{label}: annullato, PATH lasciato"));
            continue;
        }
        if !same_value(sys.read(root), &original) {
            out.failed += removed.len() as u64;
            out.note(format!("{label}: PATH cambiato, rifai l'analisi"));
            continue;
        }
        match sys.write(root, &new) {
            Ok(()) => {
                wrote = true;
                out.removed += removed.len() as u64;
                out.done.extend(removed.iter().map(|e| format!("{label}: {e}")));
            }
            Err(e) => {
                out.failed += removed.len() as u64;
                out.note(format!("{label}: PATH non scritto ({e})"));
            }
        }
    }
    if wrote {
        if let Err(e) = sys.broadcast() {
            out.note(format!(
                "Avviso a Windows non riuscito ({e}): i programmi già aperti vedranno il nuovo PATH quando ripartono"
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Implementazione vera (Win32). Mai chiamata dai test, tranne quello
// `#[ignore]` in sola lettura.

mod win {
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};

    use pc_rules::RegRoot;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_OPTION_OPEN_LINK, REG_SZ,
    };
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    use super::{decode_reg_text, drive_root, encode_reg_text, Kind, PathSys, PathValue, HKCU_KEY, HKLM_KEY, VALUE};

    /// Tentativi di lettura se il valore cambia fra la domanda della
    /// lunghezza e la lettura.
    const READ_TRIES: usize = 3;
    /// Attesa massima della risposta di ogni finestra a `WM_SETTINGCHANGE`.
    const BROADCAST_TIMEOUT_MS: u32 = 5000;

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

    fn open(parent: HKEY, name: &str, options: u32, access: u32) -> Result<Key, u32> {
        let w = wide(name);
        let mut out: HKEY = null_mut();
        // SAFETY: `w` termina con 0, `out` è un puntatore valido; la chiave
        // aperta la chiude `Key`.
        let rc = unsafe { RegOpenKeyExW(parent, w.as_ptr(), options, access | KEY_WOW64_64KEY, &mut out) };
        if rc == ERROR_SUCCESS { Ok(Key(out)) } else { Err(rc) }
    }

    /// La chiave `Environment` di `root`, aperta **senza seguire** un
    /// collegamento su `Environment` stessa (`REG_OPTION_OPEN_LINK`): se lo
    /// fosse, non avrebbe un valore `Path` e non si toccherebbe nulla. Per
    /// HKLM il percorso fino a `Session Manager` si apre normalmente, perché
    /// `CurrentControlSet` è un collegamento di sistema da seguire.
    fn open_env(root: RegRoot, access: u32) -> Result<Key, u32> {
        match root {
            RegRoot::Hkcu => open(HKEY_CURRENT_USER, HKCU_KEY, REG_OPTION_OPEN_LINK, access),
            RegRoot::Hklm => {
                let (parent, leaf) = HKLM_KEY.rsplit_once('\\').ok_or(ERROR_FILE_NOT_FOUND)?;
                let parent = open(HKEY_LOCAL_MACHINE, parent, 0, KEY_QUERY_VALUE)?;
                open(parent.0, leaf, REG_OPTION_OPEN_LINK, access)
            }
        }
    }

    pub struct RealPath;

    impl PathSys for RealPath {
        fn read(&self, root: RegRoot) -> Result<Option<PathValue>, String> {
            let key = match open_env(root, KEY_QUERY_VALUE) {
                Ok(k) => k,
                Err(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Err(rc) => return Err(format!("chiave non aperta (errore {rc})")),
            };
            let name = wide(VALUE);
            for _ in 0..READ_TRIES {
                let mut ty = 0u32;
                let mut len = 0u32;
                // SAFETY: chiave aperta, nome terminato da zero; prima solo
                // tipo e lunghezza (nessun buffer).
                let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty, null_mut(), &mut len) };
                if rc == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                if rc != ERROR_SUCCESS {
                    return Err(format!("valore Path non letto (errore {rc})"));
                }
                let kind = match ty {
                    REG_SZ => Kind::Sz,
                    REG_EXPAND_SZ => Kind::ExpandSz,
                    other => return Err(format!("valore Path di tipo {other}, non gestito")),
                };
                let mut buf = vec![0u8; len as usize];
                let mut got = len;
                let mut ty2 = 0u32;
                // SAFETY: `buf` ha `got` byte scrivibili e la dimensione
                // passata è quella.
                let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty2, buf.as_mut_ptr(), &mut got) };
                if rc == ERROR_MORE_DATA || (rc == ERROR_SUCCESS && ty2 != ty) {
                    continue; // cambiato fra le due letture
                }
                if rc != ERROR_SUCCESS {
                    return Err(format!("valore Path non letto (errore {rc})"));
                }
                buf.truncate(got as usize);
                return decode_reg_text(&buf).map(|text| Some(PathValue { kind, text }));
            }
            Err("valore Path che cambia di continuo".into())
        }

        fn write(&self, root: RegRoot, value: &PathValue) -> Result<(), String> {
            let key = open_env(root, KEY_QUERY_VALUE | KEY_SET_VALUE).map_err(|rc| format!("chiave non aperta (errore {rc})"))?;
            let data = encode_reg_text(&value.text);
            let len = u32::try_from(data.len()).map_err(|_| "PATH troppo lungo".to_string())?;
            let ty = match value.kind {
                Kind::Sz => REG_SZ,
                Kind::ExpandSz => REG_EXPAND_SZ,
            };
            let name = wide(VALUE);
            // SAFETY: chiave aperta con `KEY_SET_VALUE`, nome terminato da
            // zero, `data` ha `len` byte leggibili (testo con lo zero finale).
            let rc = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, ty, data.as_ptr(), len) };
            if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!("errore {rc}")) }
        }

        fn backup(&self) -> Result<PathBuf, String> {
            let stem = format!("backup_path_{}", crate::sys::stamp());
            let keys = [(RegRoot::Hkcu, HKCU_KEY.to_string()), (RegRoot::Hklm, HKLM_KEY.to_string())];
            crate::regbackup::write_backup(
                &crate::sys::data_dir(),
                &stem,
                &crate::regbackup::admin_work_dir,
                &crate::regbackup::reg_exe_export,
                &keys,
            )?
            .ok_or_else(|| "nessuna chiave esportata".to_string())
        }

        fn volume_is_fixed(&self, p: &Path) -> bool {
            let Some(root) = drive_root(p) else { return false };
            let w = wide(&root);
            // SAFETY: radice `X:\` terminata da zero; si chiede solo il tipo.
            unsafe { GetDriveTypeW(w.as_ptr()) == DRIVE_FIXED }
        }

        fn truly_missing(&self, p: &Path) -> bool {
            crate::sysclean::truly_missing(p)
        }

        fn broadcast(&self) -> Result<(), String> {
            let w = wide("Environment");
            let mut result: usize = 0;
            // SAFETY: `w` (terminata da zero) vive fino al ritorno della
            // chiamata, che è sincrona; `result` è un puntatore valido.
            let rc = unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0,
                    w.as_ptr() as isize,
                    SMTO_ABORTIFHUNG,
                    BROADCAST_TIMEOUT_MS,
                    &mut result,
                )
            };
            if rc == 0 { Err(std::io::Error::last_os_error().to_string()) } else { Ok(()) }
        }
    }
}

#[cfg(test)]
mod tests;
