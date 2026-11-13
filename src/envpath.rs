//! N5 «Percorso ambiente» (`docs/ccleaner-coverage.md` §2.6): toglie dalla
//! variabile `Path` dell'utente (`HKCU\Environment`) e di sistema
//! (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`) le
//! cartelle che mancano davvero e i doppioni.
//!
//! - **Espansione**: come Windows. Un valore `REG_SZ` non si espande mai (una
//!   `%VAR%` lì è testo letterale: la voce si tiene). In un `REG_EXPAND_SZ` le
//!   `%VAR%` prendono il valore vero ([`Expander`]: il blocco d'ambiente nuovo
//!   dell'utente, `CreateEnvironmentBlock`, che deve coincidere con quello del
//!   processo); una variabile sconosciuta, incerta o non definita lì = voce
//!   tenuta. Nel PATH di sistema si espandono solo le variabili che Windows
//!   definisce da sé ([`SYSTEM_VARS`]). Mai `xiox_rules::Env::for_rules`, che
//!   sostituisce i valori con quelli «sicuri» per cancellare.
//! - **Cosa si propone**: un doppione (stesso percorso risolto, senza
//!   maiuscole ASCII e senza `\` finali; si tiene la prima; le voci fra
//!   virgolette o con spazi intorno non sono mai doppioni, e una voce con una
//!   `%VAR%` non è mai doppione di una letterale né viceversa) o una cartella che manca
//!   davvero (`sysclean::truly_missing`: errore 2 o 3, nessun antenato punto
//!   di reparse) su un disco fisso presente (`GetDriveTypeW == DRIVE_FIXED`).
//!   Mai: variabili non espandibili, UNC e percorsi `\\?\`, relativi, con
//!   `..` o con virgolette in mezzo, dischi rimovibili, di rete o assenti, voci
//!   vuote, voci sotto `%SystemRoot%`. Il PATH di sistema non si svuota mai.
//! - **Account**: il PATH utente si tocca solo se il processo (elevato) gira
//!   come l'utente della sessione; altrimenti solo quello di sistema
//!   ([`crate::account`]). Senza diritti di amministratore il PATH di sistema
//!   si mostra ma non si propone ([`ADMIN_ONLY`]); quello utente sì (il backup
//!   usa allora una cartella di lavoro dell'utente, `regbackup::work_dir`).
//! - **Scrittura**: solo con le voci mostrate dall'ultima «Analisi»
//!   (`NativeItem::needs_preview`). Backup `.reg` delle chiavi da scrivere
//!   (`regbackup`, verificato: deve contenere `"Path"`; fallito = niente
//!   scrittura), poi si rilegge ogni valore: se uno è cambiato dall'analisi
//!   non si scrive nulla; altrimenti lo si riscrive con lo stesso tipo e le
//!   voci rimaste nell'ordine di prima (i `;;` vuoti spariscono); infine
//!   `WM_SETTINGCHANGE` a tutte le finestre.
//!
//! Ogni accesso al sistema passa dai trait [`PathSys`] ed [`Expander`]; i test
//! usano dei finti.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};

use xiox_rules::RegRoot;

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

/// Variabili che si espandono nel PATH di sistema: quelle che Windows
/// definisce da sé prima di comporlo. Le altre (anche di sistema, come
/// `%JAVA_HOME%`) dipendono dall'ordine in cui Windows le legge: la voce si
/// tiene.
pub const SYSTEM_VARS: &[&str] = &[
    "systemroot", "windir", "systemdrive", "programfiles", "programfiles(x86)", "programw6432", "programdata",
    "allusersprofile", "commonprogramfiles", "commonprogramfiles(x86)", "commonprogramw6432", "public",
];

const WARNING: &str = "Toglie dal PATH le cartelle che non esistono più e i doppioni. Una cartella creata più tardi \
                       (un SDK o uno strumento installato dopo, o che si ricrea quando si aggiorna) o su un disco la \
                       cui lettera ora è di un altro disco esce dal PATH: strumenti e build potrebbero non trovare i \
                       programmi. Pulizia toglie solo le voci mostrate dall'ultima Analisi. Backup .reg in \
                       Documenti\\PulisciPC.";

/// Nota delle voci del PATH di sistema quando il processo non è elevato.
pub const ADMIN_ONLY: &str = "solo con diritti di amministratore: non verrà modificato";

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

pub use crate::account::Account;

/// Registro, dischi, account, backup e avviso a Windows.
pub trait PathSys {
    /// `Path` della chiave di `root`. `Ok(None)` se il valore (o la chiave)
    /// non c'è; `Err` se non si legge, se non è `REG_SZ`/`REG_EXPAND_SZ` o se
    /// il testo non è UTF-16 valido: allora la chiave non si tocca.
    fn read(&self, root: RegRoot) -> Result<Option<PathValue>, String>;
    /// Riscrive `Path` con il tipo e il testo dati.
    fn write(&self, root: RegRoot, value: &PathValue) -> Result<(), String>;
    /// Backup `.reg` delle chiavi di `roots`, verificato; `Err` = niente scrittura.
    fn backup(&self, roots: &[RegRoot]) -> Result<PathBuf, String>;
    /// `true` solo per un disco fisso presente (`GetDriveTypeW == DRIVE_FIXED`).
    fn volume_is_fixed(&self, p: &Path) -> bool;
    /// `true` solo se la cartella manca davvero (vedi `sysclean::truly_missing`).
    fn truly_missing(&self, p: &Path) -> bool;
    /// `WM_SETTINGCHANGE` con `"Environment"` a tutte le finestre.
    fn broadcast(&self) -> Result<(), String>;
    /// Di chi è `HKCU` rispetto all'utente della sessione.
    fn account(&self) -> Account;
    /// `true` se il processo può scrivere il PATH di sistema (`HKLM`): gira
    /// con i diritti di amministratore.
    fn system_writable(&self) -> bool;
}

/// Il valore di una variabile d'ambiente come Windows lo dà ai programmi
/// nuovi dell'utente.
pub trait Expander {
    /// `Some` solo se il valore è certo; `None` (variabile sconosciuta o
    /// incerta) = la voce si tiene.
    fn var(&self, name: &str) -> Option<String>;
}

/// [`Expander`] su due ambienti: il blocco nuovo dell'utente
/// (`CreateEnvironmentBlock`) e quello del processo. Una variabile vale solo
/// se c'è in entrambi con lo stesso valore (altrimenti è cambiata dopo
/// l'avvio, o il processo ha un ambiente diverso da quello dell'utente).
/// Nomi in minuscolo.
#[derive(Debug, Clone, Default)]
pub struct BlockExpander {
    pub block: HashMap<String, String>,
    pub process: HashMap<String, String>,
}

impl Expander for BlockExpander {
    fn var(&self, name: &str) -> Option<String> {
        let n = name.to_lowercase();
        let b = self.block.get(&n)?;
        (self.process.get(&n) == Some(b)).then(|| b.clone())
    }
}

/// Le variabili di un blocco d'ambiente (`NOME=valore\0…\0\0`, UTF-16), con
/// i nomi in minuscolo. Le voci senza nome (`=C:=C:\`) o non UTF-16 valide
/// si saltano (quelle variabili restano sconosciute: la voce si tiene).
pub fn parse_env_block(words: &[u16]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in words.split(|w| *w == 0) {
        if entry.is_empty() {
            break;
        }
        let Ok(s) = String::from_utf16(entry) else { continue };
        let Some(pos) = s.char_indices().skip(1).find(|(_, c)| *c == '=').map(|(i, _)| i) else { continue };
        let name = &s[..pos];
        if name.starts_with('=') {
            continue;
        }
        out.insert(name.to_lowercase(), s[pos + 1..].to_string());
    }
    out
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
                      o scollegati e quelle con variabili sconosciute non si toccano mai. Pulizia toglie solo le \
                      voci mostrate dall'ultima Analisi; prima di scrivere si salva un backup .reg.",
        warning: Some(WARNING),
        warning_extra: None,
        default_on: false,
        needs_admin: true,
        running: &[],
        needs_preview: true,
        analyze: real_analyze,
        clean: real_clean,
    }]
}

fn real_analyze(_: &NativeCtx) -> NativeFound {
    match win::real_expander() {
        Ok(exp) => analyze_path(&win::RealPath, &exp),
        Err(e) => NativeFound { detail: vec![no_env_note(&e)], ..Default::default() },
    }
}

fn real_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
    match win::real_expander() {
        Ok(exp) => clean_path(&win::RealPath, &exp, found, ctx.cancel),
        Err(e) => {
            let mut out = NativeCleaned { failed: found.items.len() as u64, ..Default::default() };
            out.note(no_env_note(&e));
            out
        }
    }
}

fn no_env_note(e: &str) -> String {
    format!("Ambiente dell'utente non letto ({e}): il PATH non si tocca")
}

/// Nome della chiave per il resoconto.
pub fn root_label(root: RegRoot) -> &'static str {
    match root {
        RegRoot::Hklm => "Sistema",
        RegRoot::Hkcu => "Utente",
    }
}

/// `Some(nota)` se il PATH utente non si tocca con questo account.
fn hkcu_refusal(account: &Account) -> Option<String> {
    match account {
        Account::Same(_) => None,
        Account::Other(name) => {
            Some(format!("Utente: PATH utente di un altro account: non toccato (il programma gira come {name})"))
        }
        Account::Unknown(e) => Some(format!("Utente: account della sessione non verificato ({e}), PATH utente non toccato")),
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

/// Le voci da togliere dal valore `value` della chiave di `root`. `Err` (con
/// il motivo) quando il valore intero non si tocca: virgolette non chiuse,
/// cartella di Windows sconosciuta, PATH di sistema che resterebbe vuoto.
pub fn proposals(root: RegRoot, value: &PathValue, sys: &dyn PathSys, exp: &dyn Expander) -> Result<Vec<Proposal>, String> {
    let entries = split_entries(&value.text).ok_or("virgolette non chiuse, non si tocca")?;
    let windows: Vec<PathBuf> = ["systemroot", "windir"]
        .iter()
        .filter_map(|v| exp.var(v))
        .map(PathBuf::from)
        .filter(|p| disk_path(p))
        .collect();
    if windows.is_empty() {
        return Err("cartella di Windows sconosciuta, non si tocca".into());
    }
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (index, raw) in entries.iter().enumerate() {
        let resolved = resolve(root, value.kind, raw, exp);
        let mut duplicate = false;
        // Solo le voci scritte in modo semplice fanno da doppione (e da
        // «prima»): `"C:\x"` funziona solo in cmd.exe, `C:\x` ovunque.
        if let Some(r) = resolved.as_ref().filter(|r| r.plain) {
            let key = dup_key(raw, r);
            if seen.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
                duplicate = true;
            } else {
                seen.push(key);
            }
        }
        let Some(p) = resolved.as_ref().and_then(|r| judgeable(&r.path, &windows)) else { continue };
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

/// Il percorso che Windows dà davvero a una voce.
struct Resolved {
    path: String,
    /// Né virgolette né spazi intorno: può essere un doppione.
    plain: bool,
    /// Scritta con almeno una `%VAR%` espansa.
    from_var: bool,
}

/// La voce senza spazi intorno, senza le virgolette che la racchiudono e,
/// solo in un `REG_EXPAND_SZ`, con le `%VAR%` espanse. `None` se è vuota, ha
/// virgolette in mezzo, una `%` in un `REG_SZ` (Windows non la espande: testo
/// letterale), una variabile sconosciuta o incerta, un `%` aperto, o se
/// l'espansione dà `;`, `%` o `"`.
fn resolve(root: RegRoot, kind: Kind, raw: &str, exp: &dyn Expander) -> Option<Resolved> {
    let t = raw.trim();
    let inner = t.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
    let plain = inner.is_none() && t.len() == raw.len();
    let t = inner.unwrap_or(t);
    if t.is_empty() || t.contains('"') {
        return None;
    }
    let path = match kind {
        Kind::Sz if t.contains('%') => return None,
        Kind::Sz => t.to_string(),
        Kind::ExpandSz => expand_vars(root, t, exp)?,
    };
    let from_var = t.contains('%');
    (!path.contains([';', '%', '"'])).then_some(Resolved { path, plain, from_var })
}

/// `%Var%` sostituite con il valore vero; `None` se una variabile non c'è, è
/// incerta, nel PATH di sistema non è fra [`SYSTEM_VARS`], o se un `%` resta
/// aperto.
fn expand_vars(root: RegRoot, s: &str, exp: &dyn Expander) -> Option<String> {
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
        if root == RegRoot::Hklm && !SYSTEM_VARS.contains(&name.to_lowercase().as_str()) {
            return None;
        }
        out.push_str(&exp.var(name)?);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Chiave di confronto dei doppioni (da confrontare con
/// `eq_ignore_ascii_case`: solo le maiuscole ASCII contano come uguali; un
/// carattere non ASCII diverso, come il segno Kelvin U+212A contro `K`, che
/// NTFS tiene distinti, fa due voci diverse, tenute tutte e due).
///
/// Una voce con una `%VAR%` si confronta solo con le voci scritte con lo
/// stesso testo (prefisso `v:`), mai con una letterale (prefisso `l:`): se
/// Windows, componendo il PATH, espande la variabile in un altro modo (ordine
/// delle variabili utente), togliere la copia letterale lascerebbe solo
/// quella che non funziona. Così una coppia «variabile contro letterale» resta
/// intera; la cartella che manca davvero si toglie comunque, voce per voce.
fn dup_key(raw: &str, r: &Resolved) -> String {
    if r.from_var {
        format!("v:{}", trim_slashes(raw.trim()))
    } else {
        format!("l:{}", trim_slashes(&r.path))
    }
}

/// Senza `\` finali (ma `C:\` resta `C:\`, diverso da `C:`).
fn trim_slashes(e: &str) -> &str {
    let mut k = e;
    while k.ends_with('\\') && !(k.len() == 3 && k.as_bytes()[1] == b':') {
        k = &k[..k.len() - 1];
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

/// Legge le due chiavi e dice cosa si toglierebbe (tutte le voci, una per
/// riga: la pulizia toglierà solo queste). Non scrive nulla.
pub fn analyze_path(sys: &dyn PathSys, exp: &dyn Expander) -> NativeFound {
    let mut found = NativeFound::default();
    let mut lines: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut admin_only: Vec<String> = Vec::new();
    let account = sys.account();
    let system_writable = sys.system_writable();
    for root in ROOTS {
        let label = root_label(root);
        if root == RegRoot::Hkcu {
            if let Some(why) = hkcu_refusal(&account) {
                notes.push(why);
                continue;
            }
        }
        let value = match sys.read(root) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                notes.push(format!("{label}: PATH non letto, non si tocca ({e})"));
                continue;
            }
        };
        let ps = match proposals(root, &value, sys, exp) {
            Ok(ps) => ps,
            Err(e) => {
                notes.push(format!("{label}: {e}"));
                continue;
            }
        };
        // Senza diritti di amministratore il PATH di sistema non si scrive:
        // le sue voci si mostrano, ma non si contano e non vanno a `clean`.
        if root == RegRoot::Hklm && !system_writable {
            admin_only.extend(ps.iter().map(|p| {
                format!("{label}: {} ({}) – {ADMIN_ONLY}", p.entry, reason_text(p.reason))
            }));
            continue;
        }
        for p in &ps {
            found.count += 1;
            lines.push(format!("{label}: {} ({})", p.entry, reason_text(p.reason)));
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
    } else if !admin_only.is_empty() {
        found.detail.push("Nessuna cartella da togliere senza diritti di amministratore".to_string());
    }
    found.detail.extend(admin_only.iter().cloned());
    if found.count > 0 || !admin_only.is_empty() {
        if let Account::Same(name) = &account {
            found.detail.push(format!("Account: {name}"));
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
}

fn same_value(read: Result<Option<PathValue>, String>, original: &PathValue) -> bool {
    matches!(read, Ok(Some(v)) if v == *original)
}

/// Toglie le voci di `found.items` (quelle mostrate dall'ultima analisi),
/// ciascuna solo se è ancora da togliere. Ogni valore deve essere identico a
/// quello dell'analisi: se uno è cambiato, o un elemento non è valido, non
/// si scrive nulla. Backup prima (fallito = niente scrittura), riletto subito
/// prima di scrivere, avviso a Windows dopo.
pub fn clean_path(sys: &dyn PathSys, exp: &dyn Expander, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let total = found.items.len() as u64;
    let mut plans: Vec<Plan> = Vec::new();
    for s in &found.items {
        let Some(it) = parse_item(s) else {
            out.failed += total;
            out.note("Elemento del PATH non valido: non si tocca nulla, rifai l'analisi".into());
            return out;
        };
        match plans.iter_mut().find(|p| p.root == it.root) {
            Some(p) if p.original != it.original => {
                out.failed += total;
                out.note("Elementi del PATH non coerenti: non si tocca nulla, rifai l'analisi".into());
                return out;
            }
            Some(p) => p.wanted.push((it.index, it.entry)),
            None => plans.push(Plan { root: it.root, original: it.original, wanted: vec![(it.index, it.entry)] }),
        }
    }

    let account = sys.account();
    if let Some(p) = plans.iter().find(|p| p.root == RegRoot::Hkcu) {
        if let Some(why) = hkcu_refusal(&account) {
            out.failed += p.wanted.len() as u64;
            out.note(why);
            plans.retain(|p| p.root != RegRoot::Hkcu);
        }
    }
    if let Some(p) = plans.iter().find(|p| p.root == RegRoot::Hklm) {
        if !sys.system_writable() {
            out.failed += p.wanted.len() as u64;
            out.note(format!("Sistema: {ADMIN_ONLY}"));
            plans.retain(|p| p.root != RegRoot::Hklm);
        }
    }
    // Un valore cambiato dall'analisi: l'utente non ha visto quello attuale.
    if let Some(p) = plans.iter().find(|p| !same_value(sys.read(p.root), &p.original)) {
        out.failed += plans.iter().map(|p| p.wanted.len() as u64).sum::<u64>();
        out.note(format!("{}: PATH cambiato dopo l'analisi, non si tocca nulla: rifai l'analisi", root_label(p.root)));
        return out;
    }

    // Cosa resta da togliere fra le voci viste, sul valore attuale.
    let mut ready: Vec<(RegRoot, PathValue, PathValue, Vec<String>)> = Vec::new();
    for plan in plans {
        if cancel.load(Ordering::Relaxed) {
            return out;
        }
        let label = root_label(plan.root);
        let current = match proposals(plan.root, &plan.original, sys, exp) {
            Ok(c) => c,
            Err(e) => {
                out.failed += plan.wanted.len() as u64;
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

    let pending = |ready: &[(RegRoot, PathValue, PathValue, Vec<String>)]| ready.iter().map(|r| r.3.len() as u64).sum::<u64>();
    let roots: Vec<RegRoot> = ready.iter().map(|r| r.0).collect();
    let who = match &account {
        Account::Same(name) => format!(" (account {name})"),
        _ => String::new(),
    };
    match sys.backup(&roots) {
        Ok(path) => out.note(format!("Backup del PATH{who}: {}", path.display())),
        Err(e) => {
            out.failed += pending(&ready);
            out.note(format!("Backup .reg non riuscito, il PATH non si tocca: {e}"));
            return out;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        out.note("Annullato, PATH lasciato".into());
        return out;
    }
    // Riletti tutti subito prima di scrivere: un cambio durante il backup
    // ferma tutto.
    if let Some(r) = ready.iter().find(|r| !same_value(sys.read(r.0), &r.1)) {
        out.failed += pending(&ready);
        out.note(format!("{}: PATH cambiato dopo l'analisi, non si tocca nulla: rifai l'analisi", root_label(r.0)));
        return out;
    }

    let mut wrote = false;
    for (root, _, new, removed) in ready {
        let label = root_label(root);
        // TOCTOU inevitabile: fra la rilettura qui sopra e `RegSetValueExW`
        // un altro programma può ancora cambiare `Path`, e la sua modifica
        // andrebbe persa. Il registro non ha un «confronta e scrivi»
        // atomico; la finestra è di pochi microsecondi e il backup appena
        // fatto ha il valore di prima.
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

/// `Ok` se il testo di un `.reg` esportato ha, per ogni chiave di `roots`, la
/// sua sezione con il valore `"Path"`.
pub fn backup_covers(text: &str, roots: &[RegRoot]) -> Result<(), String> {
    for root in roots {
        let header = match root {
            RegRoot::Hkcu => format!("[HKEY_CURRENT_USER\\{HKCU_KEY}]"),
            RegRoot::Hklm => format!("[HKEY_LOCAL_MACHINE\\{HKLM_KEY}]"),
        }
        .to_lowercase();
        let mut inside = false;
        let mut ok = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line.to_lowercase() == header;
            } else if inside && line.to_lowercase().starts_with("\"path\"=") {
                ok = true;
                break;
            }
        }
        if !ok {
            return Err(format!("il backup non contiene \"Path\" della chiave {}", root_label(*root)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Implementazione vera (Win32). Mai chiamata dai test, tranne quello
// `#[ignore]` in sola lettura.

mod win {
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};

    use xiox_rules::RegRoot;
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

    use super::{
        backup_covers, decode_reg_text, drive_root, encode_reg_text, parse_env_block, Account, BlockExpander, Kind,
        PathSys, PathValue, HKCU_KEY, HKLM_KEY, VALUE,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY};
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// Token del processo, chiuso all'uscita.
    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: handle aperto da `OpenProcessToken`, chiuso una volta sola.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn token(access: u32) -> Result<Token, String> {
        let mut h: HANDLE = null_mut();
        // SAFETY: pseudo-handle del processo; `h` è un puntatore valido.
        if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) } == 0 {
            return Err(format!("token del processo non aperto: {}", last_error()));
        }
        Ok(Token(h))
    }

    /// L'ambiente che Windows darebbe a un programma nuovo dell'utente del
    /// processo (`CreateEnvironmentBlock`, senza ereditare quello attuale),
    /// confrontato con quello del processo.
    pub fn real_expander() -> Result<BlockExpander, String> {
        let t = token(TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE)?;
        let mut env: *mut core::ffi::c_void = null_mut();
        // SAFETY: token valido con i diritti richiesti; `env` è un puntatore
        // valido, il blocco si libera con `DestroyEnvironmentBlock`.
        if unsafe { CreateEnvironmentBlock(&mut env, t.0, 0) } == 0 || env.is_null() {
            return Err(format!("blocco d'ambiente non creato: {}", last_error()));
        }
        let p = env as *const u16;
        let mut n = 0usize;
        // SAFETY: il blocco finisce con due zeri consecutivi; si legge fino a lì.
        unsafe {
            while !(*p.add(n) == 0 && *p.add(n + 1) == 0) {
                n += 1;
            }
        }
        // SAFETY: `n + 2` caratteri letti sopra.
        let block = parse_env_block(unsafe { std::slice::from_raw_parts(p, n + 2) });
        // SAFETY: blocco creato da `CreateEnvironmentBlock`, liberato una volta.
        unsafe { DestroyEnvironmentBlock(env) };
        let process = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?.to_lowercase(), v.into_string().ok()?)))
            .collect();
        Ok(BlockExpander { block, process })
    }

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

        fn backup(&self, roots: &[RegRoot]) -> Result<PathBuf, String> {
            let stem = format!("backup_path_{}", crate::sys::stamp());
            let keys: Vec<(RegRoot, String)> = roots
                .iter()
                .map(|r| match r {
                    RegRoot::Hkcu => (RegRoot::Hkcu, HKCU_KEY.to_string()),
                    RegRoot::Hklm => (RegRoot::Hklm, HKLM_KEY.to_string()),
                })
                .collect();
            let path = crate::regbackup::write_backup(
                &crate::sys::data_dir(),
                &stem,
                &crate::regbackup::work_dir,
                &crate::regbackup::reg_exe_export,
                &keys,
            )?
            .ok_or_else(|| "nessuna chiave esportata".to_string())?;
            // Il file scritto (e già su disco: `write_backup` fa `sync_all`)
            // deve contenere `"Path"` di ogni chiave.
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            backup_covers(&String::from_utf16_lossy(&words), roots).map_err(|e| format!("{e} ({})", path.display()))?;
            Ok(path)
        }

        fn account(&self) -> Account {
            crate::account::current()
        }

        fn system_writable(&self) -> bool {
            crate::sys::is_admin()
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
