//! Lettura di `Winapp2.ini` e conversione delle voci rilevate in `Cleaner`.
//!
//! Sintassi (vedi il file vero): intestazione di commenti con `; Version: N` e
//! `; # of entries: N`, poi sezioni `[Nome *]` con chiavi `Chiave<n>=valore`:
//! `LangSecRef`, `Section`, `Detect` (`ROOT\chiave`), `DetectFile`
//! (`percorso` o `cartella|file`, con `*`/`?`), `Default`, `Warning`,
//! `FileKey` (`percorso|modelli;separati|FLAG`), `RegKey` (`ROOT\chiave|valore?`),
//! `ExcludeKey` (`FILE|percorso|modello`, `PATH|…`, `REG|ROOT\chiave|valore?`).
//! I flag sono `RECURSE` e `REMOVESELF`. Le chiavi che non si conoscono si
//! ignorano e si contano negli avvisi.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::expand::expand;
use crate::guard::Guard;
use crate::model::{Action, Cleaner, Env, ExcludeRule, Opt, Origin, RegRoot, Search};
use crate::regguard::reg_allowed;
use crate::wildcard::wildcard_match;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winapp2 {
    pub version: Option<String>,
    pub declared_entries: Option<u32>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Nome come nel file (di solito con ` *` finale).
    pub name: String,
    pub section: Option<String>,
    pub lang_sec_ref: Option<u32>,
    pub detect: Vec<Detect>,
    /// `Default` del file (vero se manca). Nell'app ogni voce parte spenta.
    pub default_on: bool,
    pub warning: Option<String>,
    pub file_keys: Vec<FileKey>,
    pub reg_keys: Vec<RegKeyRule>,
    pub excludes: Vec<Exclude>,
    /// Regole lette ma non rappresentabili (radice di registro diversa da
    /// HKCU/HKLM, `FileKey` senza modelli): ignorate.
    pub skipped_rules: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detect {
    RegKey { root: RegRoot, path: String },
    /// Percorso (anche con `*`/`?`); `cartella|file` è già unito con `\`.
    File(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileKey {
    pub path: String,
    pub patterns: Vec<String>,
    pub recurse: bool,
    pub remove_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegKeyRule {
    pub root: RegRoot,
    pub path: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclude {
    /// `FILE`, `PATH` o `REG` (in maiuscolo).
    pub kind: String,
    pub path: String,
    pub pattern: Option<String>,
}

/// Cosa è successo alle regole durante la conversione.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Azioni scartate perché puntano a un percorso o a una chiave protetti.
    pub discarded_protected: u32,
    /// Azioni scartate perché usano una variabile d'ambiente non definita.
    pub discarded_missing_var: u32,
    /// Regole ignorate in lettura (`Entry::skipped_rules`) delle voci rilevate.
    pub skipped_unknown: u32,
    /// Voci con `ExcludeKey` di specie diversa da `FILE` (`PATH`, `REG`): non
    /// si offrono, perché non si saprebbero rispettare.
    pub skipped_excludes: u32,
}

const MAX_UNREADABLE_WARNINGS: usize = 50;

/// Legge il testo di `Winapp2.ini`. Non fallisce mai: le righe illeggibili e le
/// chiavi sconosciute finiscono negli avvisi.
pub fn parse(text: &str) -> (Winapp2, Vec<String>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut w = Winapp2 {
        version: None,
        declared_entries: None,
        entries: Vec::new(),
    };
    let mut warnings: Vec<String> = Vec::new();
    let mut unknown: BTreeMap<String, u32> = BTreeMap::new();
    let mut unreadable = 0usize;
    let mut cur: Option<Entry> = None;

    let mut bad = |warnings: &mut Vec<String>, n: usize, line: &str, why: &str| {
        unreadable += 1;
        if unreadable <= MAX_UNREADABLE_WARNINGS {
            warnings.push(format!("riga {n}: {why}: {line}"));
        }
    };

    for (idx, raw) in text.lines().enumerate() {
        let n = idx + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix(';') {
            if cur.is_none() {
                header(comment.trim(), &mut w);
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(e) = cur.take() {
                w.entries.push(e);
            }
            cur = Some(Entry {
                name: line[1..line.len() - 1].trim().to_string(),
                section: None,
                lang_sec_ref: None,
                detect: Vec::new(),
                default_on: true,
                warning: None,
                file_keys: Vec::new(),
                reg_keys: Vec::new(),
                excludes: Vec::new(),
                skipped_rules: 0,
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bad(&mut warnings, n, line, "riga illeggibile");
            continue;
        };
        let Some(entry) = cur.as_mut() else {
            bad(&mut warnings, n, line, "chiave fuori da una voce");
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let base = key.trim_end_matches(|c: char| c.is_ascii_digit());
        match base.to_ascii_lowercase().as_str() {
            "langsecref" => match value.parse::<u32>() {
                Ok(v) => entry.lang_sec_ref = Some(v),
                Err(_) => bad(&mut warnings, n, line, "LangSecRef non numerico"),
            },
            "section" => {
                if !value.is_empty() {
                    entry.section = Some(value.to_string());
                }
            }
            "default" => entry.default_on = !value.eq_ignore_ascii_case("false"),
            "warning" => {
                if !value.is_empty() {
                    entry.warning = Some(value.to_string());
                }
            }
            "detect" => {
                let target = value.split('|').next().unwrap_or("");
                if let Some(rest) = classes_root(target) {
                    // HKCR è la fusione di `Software\Classes` di HKCU e HKLM.
                    for root in [RegRoot::Hkcu, RegRoot::Hklm] {
                        entry.detect.push(Detect::RegKey {
                            root,
                            path: format!(r"Software\Classes\{rest}"),
                        });
                    }
                } else {
                    match split_root(target) {
                        Some((root, path)) => entry.detect.push(Detect::RegKey {
                            root,
                            path: path.to_string(),
                        }),
                        None => entry.skipped_rules += 1,
                    }
                }
            }
            "detectfile" => {
                if !value.is_empty() {
                    let joined = match value.split_once('|') {
                        Some((dir, file)) if !file.trim().is_empty() => {
                            format!("{}\\{}", dir.trim().trim_end_matches('\\'), file.trim())
                        }
                        Some((dir, _)) => dir.trim().to_string(),
                        None => value.to_string(),
                    };
                    entry.detect.push(Detect::File(joined));
                }
            }
            "filekey" => match file_key(value, &mut unknown) {
                Some(fk) => entry.file_keys.push(fk),
                None => entry.skipped_rules += 1,
            },
            "regkey" => {
                let (key_part, val_part) = match value.split_once('|') {
                    Some((k, v)) => (k, Some(v.trim())),
                    None => (value, None),
                };
                // `Chiave|` con il nome del valore vuoto non è «tutta la chiave»:
                // non si sa cosa cancellare, quindi si salta.
                if val_part == Some("") {
                    entry.skipped_rules += 1;
                    continue;
                }
                match split_root(key_part) {
                    Some((root, path)) => entry.reg_keys.push(RegKeyRule {
                        root,
                        path: path.to_string(),
                        value: val_part.map(str::to_string),
                    }),
                    None => entry.skipped_rules += 1,
                }
            }
            "excludekey" => {
                let mut parts = value.splitn(3, '|');
                let kind = parts.next().unwrap_or("").trim().to_ascii_uppercase();
                let path = parts.next().unwrap_or("").trim();
                let pattern = parts.next().map(str::trim).filter(|p| !p.is_empty());
                if kind.is_empty() || path.is_empty() {
                    bad(&mut warnings, n, line, "ExcludeKey illeggibile");
                } else {
                    entry.excludes.push(Exclude {
                        kind,
                        path: path.to_string(),
                        pattern: pattern.map(str::to_string),
                    });
                }
            }
            _ => *unknown.entry(base.to_string()).or_insert(0) += 1,
        }
    }
    if let Some(e) = cur.take() {
        w.entries.push(e);
    }

    if unreadable > MAX_UNREADABLE_WARNINGS {
        warnings.push(format!(
            "altre {} righe illeggibili non elencate",
            unreadable - MAX_UNREADABLE_WARNINGS
        ));
    }
    for (key, count) in &unknown {
        warnings.push(format!("chiave sconosciuta «{key}»: {count} volte, ignorata"));
    }
    let skipped: u32 = w.entries.iter().map(|e| e.skipped_rules).sum();
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} regole non rappresentabili (radice di registro diversa da HKCU/HKLM o FileKey senza modelli), ignorate"
        ));
    }
    (w, warnings)
}

fn header(comment: &str, w: &mut Winapp2) {
    if w.version.is_none()
        && let Some(v) = comment.strip_prefix("Version:")
    {
        let v = v.trim();
        if !v.is_empty() {
            w.version = Some(v.to_string());
        }
    }
    if w.declared_entries.is_none()
        && let Some(v) = comment.strip_prefix("# of entries:")
    {
        let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
        w.declared_entries = digits.parse().ok();
    }
}

/// `percorso|modelli|FLAG`. `None` se mancano i modelli (solo il percorso,
/// oppure un flag al loro posto): non si sa cosa cancellare, quindi non si
/// inventa.
fn file_key(value: &str, unknown: &mut BTreeMap<String, u32>) -> Option<FileKey> {
    let mut parts = value.split('|');
    let path = parts.next().unwrap_or("").trim().trim_end_matches('\\');
    if path.is_empty() {
        return None;
    }
    let patterns_field = parts.next()?.trim();
    let flags_field = parts.next().map(str::trim).unwrap_or("");
    // Un flag al posto dei modelli (`percorso|RECURSE`) o nessun modello
    // (`percorso||RECURSE`): non si inventa `*.*`.
    if is_flag(patterns_field) {
        return None;
    }
    let patterns: Vec<String> = patterns_field
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if patterns.is_empty() {
        return None;
    }
    let (mut recurse, mut remove_self) = (false, false);
    for flag in flags_field.split(['|', ';']).map(str::trim).filter(|f| !f.is_empty()) {
        match flag.to_ascii_uppercase().as_str() {
            "RECURSE" => recurse = true,
            "REMOVESELF" => remove_self = true,
            other => *unknown.entry(format!("flag {other}")).or_insert(0) += 1,
        }
    }
    Some(FileKey {
        path: path.to_string(),
        patterns,
        recurse,
        remove_self,
    })
}

fn is_flag(s: &str) -> bool {
    s.eq_ignore_ascii_case("RECURSE") || s.eq_ignore_ascii_case("REMOVESELF")
}

/// `HKCR\.foo` → `.foo`.
fn classes_root(s: &str) -> Option<&str> {
    let s = s.trim();
    let (root, rest) = s.split_once('\\')?;
    let rest = rest.trim().trim_matches('\\');
    (matches!(root.to_ascii_uppercase().as_str(), "HKCR" | "HKEY_CLASSES_ROOT") && !rest.is_empty())
        .then_some(rest)
}

/// `HKCU\Software\X` → (Hkcu, `Software\X`). Altre radici: `None`.
fn split_root(s: &str) -> Option<(RegRoot, &str)> {
    let s = s.trim();
    let (root, rest) = s.split_once('\\').unwrap_or((s, ""));
    let root = match root.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => RegRoot::Hkcu,
        "HKLM" | "HKEY_LOCAL_MACHINE" => RegRoot::Hklm,
        _ => return None,
    };
    let rest = rest.trim().trim_matches('\\');
    if rest.is_empty() {
        return None;
    }
    Some((root, rest))
}

/// Come `to_cleaners_with`, controllando i `Detect` di registro sul registro vero.
pub fn to_cleaners(w: &Winapp2, env: &Env) -> (Vec<Cleaner>, Report) {
    to_cleaners_with(w, env, &real_reg_exists)
}

#[cfg(windows)]
fn real_reg_exists(root: RegRoot, path: &str) -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    let hive = match root {
        RegRoot::Hkcu => HKEY_CURRENT_USER,
        RegRoot::Hklm => HKEY_LOCAL_MACHINE,
    };
    RegKey::predef(hive)
        .open_subkey_with_flags(path, KEY_READ)
        .is_ok()
}

#[cfg(not(windows))]
fn real_reg_exists(_root: RegRoot, _path: &str) -> bool {
    false
}

/// Tiene solo le voci rilevate e ne fa un `Cleaner` ciascuna, con una sola
/// opzione (tutta la voce è una casella) spenta di default.
///
/// Una voce è rilevata se almeno un `Detect` (registro, via `reg_exists`) o
/// `DetectFile` esiste; senza `Detect`, se esiste la cartella di almeno un
/// `FileKey`. Le azioni con percorso protetto, chiave di registro fuori da
/// `Software` o variabile mancante si scartano e si contano.
pub fn to_cleaners_with(
    w: &Winapp2,
    env: &Env,
    reg_exists: &dyn Fn(RegRoot, &str) -> bool,
) -> (Vec<Cleaner>, Report) {
    let mut report = Report::default();
    let mut out = Vec::new();
    let guard = Guard::new(env);
    let mut probe = Probe::default();
    for entry in &w.entries {
        if !detected(entry, env, reg_exists, &mut probe) {
            continue;
        }
        report.skipped_unknown += entry.skipped_rules;
        // Solo `FILE` si sa esprimere; con un'altra specie (`PATH`, `REG`)
        // cancellare senza rispettarla sarebbe peggio che non offrire la voce.
        if entry.excludes.iter().any(|x| x.kind != "FILE") {
            report.skipped_excludes += 1;
            continue;
        }
        let excludes: Vec<ExcludeRule> = entry
            .excludes
            .iter()
            .map(|x| ExcludeRule {
                path: x.path.clone(),
                pattern: x.pattern.clone(),
            })
            .collect();
        let mut actions = Vec::new();
        for fk in &entry.file_keys {
            let expanded = expand(&fk.path, env);
            if expanded.is_empty() {
                report.discarded_missing_var += 1;
                continue;
            }
            if expanded.iter().any(|p| unsafe_files_rule(p, fk, &guard)) {
                report.discarded_protected += 1;
                continue;
            }
            actions.push(Action::Files {
                path: fk.path.clone(),
                patterns: fk.patterns.clone(),
                // REMOVESELF toglie la cartella con tutto il contenuto.
                recurse: fk.recurse || fk.remove_self,
                remove_self: fk.remove_self,
                search: Search::Winapp2,
            });
        }
        for rk in &entry.reg_keys {
            if reg_allowed(rk.root, &rk.path, rk.value.as_deref()).is_err() {
                report.discarded_protected += 1;
                continue;
            }
            actions.push(match &rk.value {
                Some(value) => Action::RegValue {
                    root: rk.root,
                    path: rk.path.clone(),
                    value: value.clone(),
                },
                None => Action::RegKey {
                    root: rk.root,
                    path: rk.path.clone(),
                },
            });
        }
        if actions.is_empty() {
            continue;
        }
        let label = entry.name.strip_suffix(" *").unwrap_or(&entry.name).trim();
        out.push(Cleaner {
            id: format!("winapp2:{label}"),
            label: label.to_string(),
            group: group_of(entry),
            origin: Origin::Winapp2,
            description: String::new(),
            running: Vec::new(),
            options: vec![Opt {
                id: "tutto".to_string(),
                label: label.to_string(),
                description: String::new(),
                warning: entry.warning.clone(),
                // Nessuna voce preselezionata, qualunque sia `Default`.
                default_on: false,
                actions,
                excludes,
                unsupported: None,
            }],
        });
    }
    (out, report)
}

/// Gruppo in italiano: prima `LangSecRef` (le sezioni definite dalla tabella di
/// CONTRIBUTING.md di Winapp2), poi `Section` (`Games` → `Giochi`,
/// `<X> Web Browser` → `Browser: <X>`, il resto com'è), altrimenti
/// `Applicazioni`.
fn group_of(entry: &Entry) -> String {
    let by_ref = entry.lang_sec_ref.and_then(|r| {
        Some(match r {
            3021 => "Applicazioni",
            3022 => "Internet",
            3023 => "Multimedia",
            3024 => "Utilità",
            3025 => "Windows",
            3031 => "Microsoft Store",
            3001 => "Browser: Internet Explorer",
            3005 | 3006 => "Browser: Microsoft Edge",
            3026 => "Browser: Firefox",
            3027 => "Browser: Opera",
            3028 => "Browser: Safari",
            3029 => "Browser: Google Chrome",
            3030 => "Thunderbird",
            3033 => "Browser: Vivaldi",
            3034 => "Browser: Brave",
            _ => return None,
        })
    });
    if let Some(g) = by_ref {
        return g.to_string();
    }
    let Some(section) = entry.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return "Applicazioni".to_string();
    };
    if section.eq_ignore_ascii_case("games") {
        return "Giochi".to_string();
    }
    let lower = section.to_lowercase();
    if let Some(name) = lower.strip_suffix(" web browser").map(|_| &section[..section.len() - 12])
        && !name.trim().is_empty()
    {
        return format!("Browser: {}", name.trim());
    }
    section.to_string()
}

fn detected(
    entry: &Entry,
    env: &Env,
    reg_exists: &dyn Fn(RegRoot, &str) -> bool,
    probe: &mut Probe,
) -> bool {
    if entry.detect.is_empty() {
        return entry
            .file_keys
            .iter()
            .any(|fk| path_exists(&fk.path, env, probe));
    }
    entry.detect.iter().any(|d| match d {
        Detect::RegKey { root, path } => {
            reg_exists(*root, path)
                || (*root == RegRoot::Hklm
                    && wow6432_view(path).is_some_and(|alt| reg_exists(*root, &alt)))
        }
        Detect::File(path) => path_exists(path, env, probe),
    })
}

/// Percorso di `FileKey` (già espanso) che non si può offrire. Oltre a
/// `protected` sul percorso intero, un jolly che sta subito sotto una cartella
/// protetta o la radice di un disco (`%AppData%\Micro*`, `%UserProfile%\*\x`)
/// si risolve in cartelle che il controllo sul percorso con il jolly non vede:
/// si rifiuta se la regola prende tutto (modello `*`/`*.*`) o scende nelle
/// sottocartelle (`RECURSE`/`REMOVESELF`), salvo un ultimo componente con un
/// prefisso letterale di almeno `MIN_LITERAL_PREFIX` caratteri.
/// Caratteri letterali minimi prima del jolly in un ultimo componente sotto
/// una cartella protetta.
const MIN_LITERAL_PREFIX: usize = 3;

fn unsafe_files_rule(expanded: &Path, fk: &FileKey, guard: &Guard) -> bool {
    if guard.protected(expanded).is_some() {
        return true;
    }
    let text = expanded.to_string_lossy().replace('/', "\\");
    let comps: Vec<&str> = text.split('\\').filter(|c| !c.is_empty()).collect();
    let Some(first_wild) = comps.iter().position(|c| c.contains(['*', '?'])) else {
        return false;
    };
    let takes_all = fk.recurse
        || fk.remove_self
        || fk.patterns.iter().any(|p| matches!(p.as_str(), "*" | "*.*"));
    if !takes_all {
        return false;
    }
    if first_wild == 0 {
        return true;
    }
    // Il jolly è la cartella stessa da pulire (`Micro*`), o è un componente fatto
    // solo di jolly (`*`): si risolve in qualunque cartella sotto il prefisso.
    // Un jolly parziale con una coda fissa (`Radeon*\cache`) resta dentro la
    // cartella dell'applicazione.
    let wild = comps[first_wild];
    let wild_only = wild.chars().all(|c| matches!(c, '*' | '?' | '.'));
    if first_wild + 1 < comps.len() && !wild_only {
        return false;
    }
    // Ultimo componente con almeno `MIN_LITERAL_PREFIX` caratteri letterali
    // prima del jolly (`Discord*`, `CLR_v*`): resta nella cartella di una
    // applicazione; la raccolta rifiuta comunque ogni cartella concreta
    // protetta. `*`, `M*`, `Mi*` si rifiutano.
    let literal = wild.chars().take_while(|c| !matches!(c, '*' | '?')).count();
    if literal >= MIN_LITERAL_PREFIX {
        return false;
    }
    let prefix = comps[..first_wild].join("\\");
    guard.protected(Path::new(&prefix)).is_some()
}

/// `Software\X` → `Software\Wow6432Node\X` (la vista a 32 bit di HKLM).
fn wow6432_view(path: &str) -> Option<String> {
    let (first, rest) = path.split_once('\\')?;
    if !first.eq_ignore_ascii_case("software") {
        return None;
    }
    let second = rest.split('\\').next().unwrap_or("");
    if second.eq_ignore_ascii_case("wow6432node") {
        return None;
    }
    Some(format!(r"{first}\Wow6432Node\{rest}"))
}

/// Esiste almeno uno dei percorsi ottenuti espandendo le variabili (con `*` e
/// `?` nei componenti), cercato con `probe`.
fn path_exists(path: &str, env: &Env, probe: &mut Probe) -> bool {
    expand(path, env).iter().any(|p| probe.exists(p))
}

/// Componenti di jolly al più in un percorso di rilevazione.
const MAX_PROBE_WILDCARDS: usize = 6;
/// Voci di cartella esaminate al più per una rilevazione.
const MAX_PROBE_ENTRIES: usize = 5000;

/// `true` se un percorso di rilevazione (`DetectFile`, cartella di `FileKey`) si
/// può cercare sul disco: assoluto con lettera di unità (mai UNC, `\\?\`,
/// `\\.\`, volumi o percorsi relativi, che aprirebbero connessioni di rete o
/// dispositivi), senza `.`/`..`, flussi (`:`) o componenti con punti o spazi
/// finali, e al più `MAX_PROBE_WILDCARDS` componenti con jolly.
fn is_probe_safe(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('/', "\\");
    let b = text.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return false;
    }
    let mut wild = 0;
    for c in text[3..].split('\\').filter(|c| !c.is_empty()) {
        if c == "." || c == ".." || c.contains(':') || c.ends_with(['.', ' ']) {
            return false;
        }
        if c.contains(['*', '?']) {
            wild += 1;
        }
    }
    wild <= MAX_PROBE_WILDCARDS
}

/// Voce di un elenco di cartella: nome e se è una cartella vera (non un
/// collegamento: non si scende nei collegamenti).
type Listing = Rc<Vec<(String, bool)>>;

/// Ricerca sul disco per la rilevazione delle voci. Gli elenchi delle cartelle
/// restano in memoria per tutta la conversione (decine di voci cercano in
/// `%LocalAppData%\Microsoft\Edge*`); `None` = cartella illeggibile o troppo
/// grande.
#[derive(Default)]
struct Probe {
    listings: HashMap<String, Option<Listing>>,
    /// Cartelle lette davvero (per le prove).
    reads: usize,
}

impl Probe {
    fn list(&mut self, dir: &Path) -> Option<Listing> {
        let key = dir.to_string_lossy().to_lowercase();
        if let Some(l) = self.listings.get(&key) {
            return l.clone();
        }
        self.reads += 1;
        let listing = std::fs::read_dir(dir).ok().and_then(|rd| {
            let mut v = Vec::new();
            for e in rd.flatten() {
                if v.len() >= MAX_PROBE_ENTRIES {
                    return None;
                }
                let is_dir = e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
                v.push((e.file_name().to_string_lossy().into_owned(), is_dir));
            }
            Some(Rc::new(v))
        });
        self.listings.insert(key, listing.clone());
        listing
    }

    fn exists(&mut self, path: &Path) -> bool {
        if !is_probe_safe(path) {
            return false;
        }
        let text = path.to_string_lossy().replace('/', "\\");
        let comps: Vec<&str> = text[3..].split('\\').filter(|c| !c.is_empty()).collect();
        let mut current = vec![PathBuf::from(&text[..3])];
        let mut budget = MAX_PROBE_ENTRIES;
        for (i, name) in comps.iter().enumerate() {
            let last = i + 1 == comps.len();
            if !name.contains(['*', '?']) {
                for p in &mut current {
                    p.push(name);
                }
                continue;
            }
            let mut next = Vec::new();
            for dir in &current {
                let Some(listing) = self.list(dir) else { continue };
                if listing.len() > budget {
                    return false;
                }
                budget -= listing.len();
                for (entry, is_dir) in listing.iter() {
                    if (last || *is_dir) && wildcard_match(name, entry) {
                        next.push(dir.join(entry));
                    }
                }
            }
            current = next;
            if current.is_empty() {
                return false;
            }
        }
        current.iter().any(|p| std::fs::symlink_metadata(p).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    const SAMPLE: &str = include_str!("../tests/fixtures/winapp2_sample.ini");

    fn entry<'a>(w: &'a Winapp2, name: &str) -> &'a Entry {
        w.entries
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("voce «{name}» assente"))
    }

    /// Ambiente finto sotto `root`; crea le cartelle di base.
    fn env_for(root: &Path) -> Env {
        let mut m = std::collections::HashMap::new();
        for (k, rel) in [
            ("userprofile", "Users/u"),
            ("appdata", "Users/u/AppData/Roaming"),
            ("localappdata", "Users/u/AppData/Local"),
            ("programdata", "ProgramData"),
            ("windir", "Windows"),
        ] {
            let p = root.join(rel);
            fs::create_dir_all(&p).unwrap();
            m.insert(k.to_string(), p.to_string_lossy().replace('/', "\\"));
        }
        // `C:` del disco della cartella temporanea: `%SystemDrive%\*` è la radice.
        let drive: String = root.to_string_lossy().chars().take(2).collect();
        m.insert("systemdrive".to_string(), drive);
        Env(m)
    }

    fn mk(env: &Env, var: &str, rel: &str) {
        let base = env.get(var).unwrap();
        fs::create_dir_all(Path::new(base).join(rel)).unwrap();
    }

    fn ids(cleaners: &[Cleaner]) -> Vec<&str> {
        cleaners.iter().map(|c| c.id.as_str()).collect()
    }

    fn no_registry(_: RegRoot, _: &str) -> bool {
        false
    }

    #[test]
    fn reads_version_and_declared_entries() {
        let (w, _) = parse(SAMPLE);
        assert_eq!(w.version.as_deref(), Some("260915"));
        assert_eq!(w.declared_entries, Some(4068));
        assert_eq!(w.entries.len(), 12);
    }

    #[test]
    fn reads_section_lang_ref_warning_and_default() {
        let (w, _) = parse(SAMPLE);
        let a = entry(&w, "Alpha Cache *");
        assert_eq!(a.section.as_deref(), Some("Alpha Suite"));
        assert_eq!(a.lang_sec_ref, Some(3025));
        assert!(a.warning.is_none());
        let e = entry(&w, "Epsilon Warn *");
        assert!(e.warning.as_deref().unwrap().contains("impostazioni"));
        assert!(!e.default_on);
        assert!(entry(&w, "Iota Machine *").default_on);
    }

    #[test]
    fn file_key_flags_and_patterns() {
        let (w, _) = parse(SAMPLE);
        let a = entry(&w, "Alpha Cache *");
        assert_eq!(a.file_keys.len(), 2);
        assert_eq!(a.file_keys[0].path, r"%LocalAppData%\Alpha\Cache");
        assert_eq!(a.file_keys[0].patterns, vec!["*.log", "*.tmp"]);
        assert!(a.file_keys[0].recurse);
        assert!(!a.file_keys[0].remove_self);
        assert_eq!(a.file_keys[1].patterns, vec!["*.*"]);
        assert!(!a.file_keys[1].recurse);
        let b = entry(&w, "Beta Temp *");
        assert!(b.file_keys[0].remove_self);
        assert!(!b.file_keys[0].recurse);
    }

    #[test]
    fn reg_key_with_and_without_value() {
        let (w, _) = parse(SAMPLE);
        let g = entry(&w, "Gamma Registry *");
        assert_eq!(
            g.detect,
            vec![Detect::RegKey {
                root: RegRoot::Hkcu,
                path: r"Software\Gamma".into()
            }]
        );
        assert_eq!(g.reg_keys.len(), 2);
        assert_eq!(g.reg_keys[0].root, RegRoot::Hkcu);
        assert_eq!(g.reg_keys[0].path, r"Software\Gamma\MRU");
        assert_eq!(g.reg_keys[0].value, None);
        assert_eq!(g.reg_keys[1].value.as_deref(), Some("LastFile"));
        let i = entry(&w, "Iota Machine *");
        assert_eq!(i.reg_keys[0].root, RegRoot::Hklm);
    }

    #[test]
    fn exclude_key_is_read() {
        let (w, _) = parse(SAMPLE);
        let d = entry(&w, "Delta Keep *");
        assert_eq!(
            d.excludes,
            vec![Exclude {
                kind: "FILE".into(),
                path: r"%AppData%\Delta\Cache".into(),
                pattern: Some("keep.dat".into()),
            }]
        );
    }

    #[test]
    fn detect_file_joins_folder_and_file() {
        let (w, _) = parse(SAMPLE);
        assert_eq!(
            entry(&w, "Mu Plain").detect,
            vec![Detect::File(r"%AppData%\Mu\mu.exe".into())]
        );
    }

    #[test]
    fn unknown_keys_are_counted_not_fatal() {
        let (w, warnings) = parse(SAMPLE);
        assert_eq!(w.entries.len(), 12);
        let future = warnings.iter().find(|m| m.contains("FutureKey")).unwrap();
        assert!(future.contains("2 volte"), "{future}");
        assert!(warnings.iter().any(|m| m.contains("SpecialDetect")));
        // La voce con chiavi sconosciute si legge comunque.
        assert_eq!(entry(&w, "Eta Unknown *").file_keys.len(), 1);
    }

    #[test]
    fn unreadable_lines_and_unsupported_roots_are_reported() {
        let (w, warnings) = parse("[A *]\nsenza uguale\nFileKey1=%AppData%\\A\nRegKey1=HKU\\x\\y\n");
        assert_eq!(w.entries.len(), 1);
        assert!(warnings.iter().any(|m| m.contains("illeggibile")));
        assert_eq!(w.entries[0].skipped_rules, 2);
        assert!(w.entries[0].file_keys.is_empty() && w.entries[0].reg_keys.is_empty());
    }

    #[test]
    fn bom_and_crlf_are_accepted() {
        let text = format!(
            "\u{feff}{}",
            "; Version: 1\r\n; # of entries: 1\r\n[A *]\r\nDetectFile=%AppData%\\A\r\nFileKey1=%AppData%\\A|*.tmp|RECURSE\r\n"
        );
        let (w, warnings) = parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(w.version.as_deref(), Some("1"));
        assert_eq!(w.entries[0].file_keys[0].patterns, vec!["*.tmp"]);
        assert_eq!(w.entries[0].detect, vec![Detect::File(r"%AppData%\A".into())]);
    }

    #[test]
    fn entry_without_any_installed_target_is_not_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let (w, _) = parse(SAMPLE);
        let (cleaners, _) = to_cleaners_with(&w, &env, &no_registry);
        assert!(cleaners.is_empty(), "{:?}", ids(&cleaners));

        // Solo Alpha è installata.
        mk(&env, "localappdata", "Alpha");
        let (cleaners, _) = to_cleaners_with(&w, &env, &no_registry);
        assert_eq!(ids(&cleaners), vec!["winapp2:Alpha Cache"]);
    }

    #[test]
    fn entry_without_detect_uses_the_filekey_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let (w, _) = parse(SAMPLE);
        let (cleaners, _) = to_cleaners_with(&w, &env, &no_registry);
        assert!(!ids(&cleaners).contains(&"winapp2:Zeta No Detect"));
        mk(&env, "appdata", r"Zeta\Cache");
        let (cleaners, _) = to_cleaners_with(&w, &env, &no_registry);
        assert_eq!(ids(&cleaners), vec!["winapp2:Zeta No Detect"]);
    }

    #[test]
    fn registry_detect_goes_through_the_injected_check() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let (w, _) = parse(SAMPLE);
        let only_gamma = |root: RegRoot, path: &str| root == RegRoot::Hkcu && path == r"Software\Gamma";
        let (cleaners, _) = to_cleaners_with(&w, &env, &only_gamma);
        assert_eq!(ids(&cleaners), vec!["winapp2:Gamma Registry"]);
        let c = &cleaners[0];
        assert_eq!(c.group, "Gamma Browser");
        assert_eq!(
            c.options[0].actions,
            vec![
                Action::RegKey {
                    root: RegRoot::Hkcu,
                    path: r"Software\Gamma\MRU".into()
                },
                Action::RegValue {
                    root: RegRoot::Hkcu,
                    path: r"Software\Gamma\Settings".into(),
                    value: "LastFile".into()
                },
            ]
        );
    }

    #[test]
    fn detect_file_supports_wildcards_and_folder_file_pairs() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let text = "[W *]\nDetectFile=%AppData%\\Adobe Photoshop*\nFileKey1=%AppData%\\W|*.*\n\
                    [M *]\nDetectFile=%AppData%\\Mu|mu.exe\nFileKey1=%AppData%\\M|*.*\n";
        let (w, _) = parse(text);
        let (c, _) = to_cleaners_with(&w, &env, &no_registry);
        assert!(c.is_empty());
        mk(&env, "appdata", "adobe photoshop 2026");
        fs::create_dir_all(Path::new(env.get("appdata").unwrap()).join("Mu")).unwrap();
        let (c, _) = to_cleaners_with(&w, &env, &no_registry);
        assert_eq!(ids(&c), vec!["winapp2:W"]);
        fs::write(Path::new(env.get("appdata").unwrap()).join("Mu/MU.EXE"), b"x").unwrap();
        let (c, _) = to_cleaners_with(&w, &env, &no_registry);
        assert_eq!(ids(&c), vec!["winapp2:W", "winapp2:M"]);
    }

    #[test]
    fn rule_expanding_to_a_protected_path_is_discarded_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        mk(&env, "userprofile", ".theta");
        let (w, _) = parse(SAMPLE);
        let (cleaners, report) = to_cleaners_with(&w, &env, &no_registry);
        assert!(!ids(&cleaners).contains(&"winapp2:Theta Profile"));
        assert_eq!(report.discarded_protected, 1);
    }

    #[test]
    fn rule_with_undefined_variable_is_discarded_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        mk(&env, "appdata", "Kappa");
        let (w, _) = parse(SAMPLE);
        let (cleaners, report) = to_cleaners_with(&w, &env, &no_registry);
        assert_eq!(report.discarded_missing_var, 1);
        // L'altra regola della voce resta.
        let k = cleaners.iter().find(|c| c.id == "winapp2:Kappa Missing Var").unwrap();
        assert_eq!(k.options[0].actions.len(), 1);
    }

    #[test]
    fn registry_rules_outside_software_are_discarded_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let text = "[R *]\nDetect=HKCU\\Software\\R\nRegKey1=HKCU\\Control Panel\\R\nRegKey2=HKLM\\Software\nRegKey3=HKLM\\System\\R|v\nRegKey4=HKCU\\Software\\Classes\\R\n";
        let (w, _) = parse(text);
        let (c, report) = to_cleaners_with(&w, &env, &|_, _| true);
        // Control Panel, System e `HKLM\Software` preso intero.
        assert_eq!(report.discarded_protected, 3);
        assert_eq!(
            c[0].options[0].actions,
            vec![Action::RegKey {
                root: RegRoot::Hkcu,
                path: r"Software\Classes\R".into()
            }]
        );
    }

    #[test]
    fn file_exclude_keys_go_on_the_option_and_other_kinds_are_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        mk(&env, "appdata", "Delta");
        mk(&env, "appdata", "Pi");
        let (w, _) = parse(&format!(
            "{SAMPLE}\n[Pi Path *]\nDetectFile=%AppData%\\Pi\nFileKey1=%AppData%\\Pi|*\nExcludeKey1=PATH|%AppData%\\Pi\\Keep\n"
        ));
        let (cleaners, report) = to_cleaners_with(&w, &env, &no_registry);
        let delta = cleaners.iter().find(|c| c.id == "winapp2:Delta Keep").unwrap();
        assert_eq!(
            delta.options[0].excludes,
            vec![ExcludeRule {
                path: r"%AppData%\Delta\Cache".into(),
                pattern: Some("keep.dat".into()),
            }]
        );
        assert!(!ids(&cleaners).contains(&"winapp2:Pi Path"));
        assert_eq!(report.skipped_excludes, 1);
    }

    #[test]
    fn remove_self_implies_recurse_and_labels_groups_and_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        mk(&env, "appdata", "Beta");
        mk(&env, "appdata", "Epsilon");
        mk(&env, "appdata", "Mu");
        fs::write(Path::new(env.get("appdata").unwrap()).join("Mu/mu.exe"), b"x").unwrap();
        let (w, _) = parse(SAMPLE);
        let (cleaners, _) = to_cleaners_with(&w, &env, &no_registry);
        let by = |id: &str| cleaners.iter().find(|c| c.id == id).unwrap();

        let beta = by("winapp2:Beta Temp");
        assert_eq!(beta.label, "Beta Temp");
        // LangSecRef=3026 è la sezione Firefox.
        assert_eq!(beta.group, "Browser: Firefox");
        assert_eq!(beta.origin, Origin::Winapp2);
        match &beta.options[0].actions[0] {
            Action::Files { recurse, remove_self, search, path, patterns } => {
                assert!(*recurse && *remove_self);
                assert_eq!(*search, Search::Winapp2);
                assert_eq!(path, r"%AppData%\Beta\Temp");
                assert_eq!(patterns, &vec!["*.*".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        assert!(by("winapp2:Epsilon Warn").options[0].warning.is_some());
        // Nome senza ` *` finale: resta com'è.
        assert_eq!(by("winapp2:Mu Plain").label, "Mu Plain");
    }

    #[test]
    fn everything_is_off_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        for (var, rel) in [
            ("localappdata", "Alpha"),
            ("appdata", "Beta"),
            ("appdata", "Epsilon"),
            ("appdata", r"Zeta\Cache"),
            ("appdata", "Eta"),
            ("programdata", "Iota"),
        ] {
            mk(&env, var, rel);
        }
        let (w, _) = parse(SAMPLE);
        assert!(entry(&w, "Iota Machine *").default_on);
        let (cleaners, _) = to_cleaners_with(&w, &env, &|_, _| true);
        assert!(cleaners.len() >= 6, "{:?}", ids(&cleaners));
        for c in &cleaners {
            assert_eq!(c.options.len(), 1);
            assert!(!c.options[0].default_on, "{} parte accesa", c.id);
            assert!(c.options[0].unsupported.is_none());
        }
    }

    fn one_entry(body: &str) -> Winapp2 {
        let text = format!("[P *]\nDetect=HKCU\\Software\\P\n{body}");
        parse(&text).0
    }

    #[test]
    fn wildcard_rules_under_protected_folders_are_discarded_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let w = one_entry(
            "FileKey1=%UserProfile%\\*|*|RECURSE\n\
             FileKey2=%LocalAppData%\\*|*.*|RECURSE\n\
             FileKey3=%SystemDrive%\\*|*|REMOVESELF\n\
             FileKey4=%AppData%\\*|*|RECURSE\n\
             FileKey5=%ProgramData%\\*|*|RECURSE\n\
             FileKey6=%WinDir%\\*|*|RECURSE\n\
             FileKey7=%AppData%\\Micro*|*|RECURSE\n\
             FileKey8=%UserProfile%\\*\\Cache|*|RECURSE\n\
             FileKey9=%NoSuchVar%\\*|*|RECURSE\n\
             FileKey10=%LocalAppData%\\Foo\\*|*.log\n\
             FileKey11=%AppData%\\Foo\\*|*|RECURSE\n\
             FileKey12=%ProgramData%\\Foo*\\Logs|*.log\n\
             FileKey13=%LocalAppData%\\Radeon*\\cache|*|RECURSE\n\
             FileKey14=%AppData%\\*WebStorage\\Logs|*|RECURSE\n",
        );
        let (c, report) = to_cleaners_with(&w, &env, &|_, _| true);
        assert_eq!(report.discarded_protected, 8, "{report:?}");
        assert_eq!(report.discarded_missing_var, 1);
        let paths: Vec<&str> = c[0].options[0]
            .actions
            .iter()
            .map(|a| match a {
                Action::Files { path, .. } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            vec![
                r"%LocalAppData%\Foo\*",
                r"%AppData%\Foo\*",
                r"%ProgramData%\Foo*\Logs",
                r"%LocalAppData%\Radeon*\cache",
                r"%AppData%\*WebStorage\Logs"
            ]
        );
    }

    /// N3: un ultimo componente con almeno 3 caratteri letterali prima del
    /// jolly (`Discord*`) resta: la raccolta controlla le cartelle concrete.
    #[test]
    fn partial_wildcard_last_component_with_a_literal_prefix_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let w = one_entry(
            "FileKey1=%AppData%\\Discord*|*|RECURSE\n\
             FileKey2=%LocalAppData%\\TempOffice*|*\n\
             FileKey3=%AppData%\\JDownloader*|*.log|RECURSE\n\
             FileKey4=%AppData%\\Mi*|*|RECURSE\n\
             FileKey5=%AppData%\\*|*|RECURSE\n\
             FileKey6=%AppData%\\M?*|*|RECURSE\n\
             FileKey7=%AppData%\\Microsoft*|*|RECURSE\n",
        );
        let (c, report) = to_cleaners_with(&w, &env, &|_, _| true);
        let paths: Vec<&str> = c[0].options[0]
            .actions
            .iter()
            .map(|a| match a {
                Action::Files { path, .. } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            vec![r"%AppData%\Discord*", r"%LocalAppData%\TempOffice*", r"%AppData%\JDownloader*"]
        );
        // `Mi*`, `*`, `M?*` (meno di 3 lettere) e `Microsoft*` (può essere la
        // cartella protetta `%AppData%\Microsoft`, la rifiuta il guard).
        assert_eq!(report.discarded_protected, 4, "{report:?}");
    }

    fn reg_actions(body: &str) -> (Vec<Action>, Report) {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let w = one_entry(body);
        let (c, report) = to_cleaners_with(&w, &env, &|_, _| true);
        (
            c.first().map(|c| c.options[0].actions.clone()).unwrap_or_default(),
            report,
        )
    }

    #[test]
    fn whole_key_deletes_of_system_registry_branches_are_discarded() {
        let (actions, report) = reg_actions(
            "RegKey1=HKLM\\Software\\Classes\n\
             RegKey2=HKCU\\Software\\Microsoft\n\
             RegKey3=HKLM\\Software\\Policies\n\
             RegKey4=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\n\
             RegKey5=HKLM\\Software\\Wow6432Node\n\
             RegKey6=HKLM\\Software\\Wow6432Node\\Classes\n\
             RegKey7=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\n\
             RegKey8=HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon\n\
             RegKey9=HKCU\\Software\\VOS\n\
             RegKey10=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RunMRU\n\
             RegKey11=HKLM\\Software\\Classes\\.mpf\n\
             RegKey12=HKLM\\Software\\Wow6432Node\\VOS\n",
        );
        assert_eq!(report.discarded_protected, 8, "{report:?}");
        let paths: Vec<&str> = actions
            .iter()
            .map(|a| match a {
                Action::RegKey { path, .. } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            vec![
                r"Software\VOS",
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU",
                r"Software\Classes\.mpf",
                r"Software\Wow6432Node\VOS"
            ]
        );
    }

    #[test]
    fn value_deletes_inside_critical_keys_are_discarded() {
        let (actions, report) = reg_actions(
            "RegKey1=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run|Foo\n\
             RegKey2=HKCU\\Software|Foo\n\
             RegKey3=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer|ScreenshotIndex\n",
        );
        assert_eq!(
            actions,
            vec![Action::RegValue {
                root: RegRoot::Hkcu,
                path: r"Software\Microsoft\Windows\CurrentVersion\Explorer".into(),
                value: "ScreenshotIndex".into(),
            }]
        );
        assert_eq!(report.discarded_protected, 2);
    }

    /// Le chiavi provate dalla revisione: la conversione le scarta tutte e
    /// tiene le chiavi profonde che Winapp2 usa davvero.
    #[test]
    fn review_registry_probes_are_discarded_at_conversion() {
        let (actions, report) = reg_actions(
            "RegKey1=HKLM\\Software\\Microsoft\n\
             RegKey2=HKLM\\Software\\Microsoft\\Windows NT\\CurrentVersion\n\
             RegKey3=HKLM\\Software\\Policies\\Google\n\
             RegKey4=HKLM\\Software\\Classes\\exefile\n\
             RegKey5=HKLM\\Software\\Classes\\*\n\
             RegKey6=HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\n\
             RegKey7=HKLM\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon|Userinit\n\
             RegKey8=HKLM\\Software\\Classes\\exefile\\shell\\open\\command\n\
             RegKey9=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer\\Run|x\n\
             RegKey10=HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RecentDocs\n\
             RegKey11=HKLM\\Software\\Classes\\.mpf\n\
             RegKey12=HKLM\\Software\\Microsoft\\Tracing\n\
             RegKey13=HKCU\\Software\\Microsoft\\Internet Explorer\\TypedURLs\n",
        );
        assert_eq!(report.discarded_protected, 9, "{report:?}");
        let paths: Vec<&str> = actions
            .iter()
            .map(|a| match a {
                Action::RegKey { path, .. } | Action::RegValue { path, .. } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            vec![
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs",
                r"Software\Classes\.mpf",
                r"Software\Microsoft\Tracing",
                r"Software\Microsoft\Internet Explorer\TypedURLs",
            ]
        );
    }

    #[test]
    fn empty_registry_value_name_is_skipped_not_a_whole_key_delete() {
        let (w, warnings) = parse(
            "[P *]\nDetect=HKCU\\Software\\P\nRegKey1=HKCU\\Software\\P\\Y|\nRegKey2=HKCU\\Software\\P\\Z\n",
        );
        assert_eq!(w.entries[0].reg_keys.len(), 1);
        assert_eq!(w.entries[0].reg_keys[0].path, r"Software\P\Z");
        assert_eq!(w.entries[0].skipped_rules, 1);
        assert!(warnings.iter().any(|m| m.contains("non rappresentabili")));
    }

    #[test]
    fn file_key_with_a_flag_instead_of_patterns_is_skipped_and_counted() {
        let (w, _) = parse(
            "[P *]\nFileKey1=%AppData%\\P\\a|RECURSE\nFileKey2=%AppData%\\P\\b|REMOVESELF\n\
             FileKey3=%AppData%\\P\\c||RECURSE\nFileKey4=%AppData%\\P\\d|*.log|RECURSE\n",
        );
        assert_eq!(w.entries[0].file_keys.len(), 1);
        assert_eq!(w.entries[0].file_keys[0].path, r"%AppData%\P\d");
        assert_eq!(w.entries[0].skipped_rules, 3);
    }

    #[test]
    fn hkcr_detect_checks_software_classes_in_both_hives() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let (w, _) = parse("[C *]\nDetect=HKCR\\.foo\nFileKey1=%AppData%\\C|*.tmp\n");
        assert_eq!(w.entries[0].skipped_rules, 0);
        let hkcu = |r: RegRoot, p: &str| r == RegRoot::Hkcu && p == r"Software\Classes\.foo";
        let hklm = |r: RegRoot, p: &str| r == RegRoot::Hklm && p == r"Software\Classes\.foo";
        assert_eq!(to_cleaners_with(&w, &env, &hkcu).0.len(), 1);
        assert_eq!(to_cleaners_with(&w, &env, &hklm).0.len(), 1);
        assert!(to_cleaners_with(&w, &env, &no_registry).0.is_empty());
    }

    #[test]
    fn hklm_detect_also_checks_the_wow6432node_view() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let (w, _) =
            parse("[W *]\nDetect=HKLM\\Software\\Vendor\\App\nFileKey1=%AppData%\\W|*.tmp\n");
        let wow = |r: RegRoot, p: &str| r == RegRoot::Hklm && p == r"Software\Wow6432Node\Vendor\App";
        assert_eq!(to_cleaners_with(&w, &env, &wow).0.len(), 1);
        // HKCU non ha la vista Wow6432Node.
        let (w, _) =
            parse("[W *]\nDetect=HKCU\\Software\\Vendor\\App\nFileKey1=%AppData%\\W|*.tmp\n");
        let wow = |_: RegRoot, p: &str| p == r"Software\Wow6432Node\Vendor\App";
        assert!(to_cleaners_with(&w, &env, &wow).0.is_empty());
    }

    /// Legge il vero ambiente del processo (solo lettura) per verificare che
    /// `from_process` aggiunga le cartelle note.
    #[cfg(windows)]
    #[test]
    fn from_process_adds_known_folders() {
        let env = Env::from_process();
        for name in ["locallowappdata", "documents", "commonappdata"] {
            assert!(
                env.get(name).is_some_and(|v| v.len() > 3),
                "{name} manca: {:?}",
                env.get(name)
            );
        }
        assert_eq!(env.get("commonappdata"), env.get("programdata"));
    }

    #[test]
    fn probe_safety_rules() {
        for ok in [r"C:\Users\u\AppData\Local\Microsoft\Edge*", r"D:\x\*\*\*\*\*\*", "c:/a/b"] {
            assert!(is_probe_safe(Path::new(ok)), "{ok}");
        }
        for bad in [
            r"\\host\share\x",
            r"\\?\C:\x",
            r"\\.\PhysicalDrive0",
            r"\\?\Volume{0}\x",
            r"\??\C:\x",
            r"C:x",
            r"x\y",
            r"C:\a\..\b",
            r"C:\a\.\b",
            r"C:\a.\b",
            r"C:\a \b",
            r"C:\a\b:stream",
            r"C:\*\*\*\*\*\*\*",
        ] {
            assert!(!is_probe_safe(Path::new(bad)), "{bad}");
        }
    }

    /// I3: `DetectFile` verso UNC, `\\?\`, `..` non si cerca nemmeno se il
    /// file esiste; lo stesso file con un percorso normale sì.
    #[test]
    fn unsafe_detect_paths_are_never_probed() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_for(tmp.path());
        let real = tmp.path().join("present.txt");
        fs::write(&real, "x").unwrap();
        let shown = real.to_string_lossy().replace('/', "\\");
        let parent = tmp.path().file_name().unwrap().to_string_lossy().into_owned();
        let detect = |d: &str| {
            let (w, _) = parse(&format!("[D *]\nDetectFile={d}\nFileKey1=%AppData%\\D|*.tmp\n"));
            !to_cleaners_with(&w, &env, &no_registry).0.is_empty()
        };
        assert!(detect(&shown));
        assert!(!detect(&format!(r"\\?\{shown}")));
        assert!(!detect(&format!(r"{}\..\{parent}\present.txt", tmp.path().display())));
        assert!(!detect(r"\\xiox-rules-no-such-host.invalid\share\x"));
        assert!(!detect(r"C:\*\*\*\*\*\*\*\x"));
    }

    #[test]
    fn huge_folders_stop_the_probe() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big");
        fs::create_dir_all(&big).unwrap();
        for i in 0..=MAX_PROBE_ENTRIES {
            fs::write(big.join(format!("f{i}")), b"").unwrap();
        }
        let mut probe = Probe::default();
        assert!(!probe.exists(&big.join("f1*")), "oltre il limite di voci");
        // Senza jolly non si elenca nulla.
        assert!(probe.exists(&big.join("f1")));
    }

    #[test]
    fn folder_listings_are_cached_across_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("Local").join("Microsoft");
        fs::create_dir_all(local.join("Edge Beta").join("User Data")).unwrap();
        let mut probe = Probe::default();
        for _ in 0..40 {
            assert!(probe.exists(&local.join("Edge*").join("User Data")));
            assert!(!probe.exists(&local.join("Edge*").join("Nope")));
        }
        assert_eq!(probe.reads, 1);
    }

    #[test]
    fn groups_come_from_lang_sec_ref_then_section() {
        let (w, _) = parse(
            "[A *]\nLangSecRef=3021\n[B *]\nLangSecRef=3022\n[C *]\nLangSecRef=3023\n\
             [D *]\nLangSecRef=3024\n[E *]\nLangSecRef=3025\n[F *]\nSection=Games\n\
             [G *]\nSection=Opera GX Web Browser\n[H *]\nSection=Genspark Web browser\n\
             [I *]\nSection=Adobe\n[J *]\n[K *]\nLangSecRef=3029\n[L *]\nLangSecRef=9999\nSection=Games\n\
             [M *]\nLangSecRef=3024\nSection=Games\n",
        );
        let got: Vec<String> = w.entries.iter().map(group_of).collect();
        assert_eq!(
            got,
            vec![
                "Applicazioni",
                "Internet",
                "Multimedia",
                "Utilità",
                "Windows",
                "Giochi",
                "Browser: Opera GX",
                "Browser: Genspark",
                "Adobe",
                "Applicazioni",
                "Browser: Google Chrome",
                "Giochi",
                "Utilità",
            ]
        );
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("Adobe Photoshop*", "ADOBE PHOTOSHOP 2026"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("a*b", "acd"));
        assert!(wildcard_match("*.log", "x.y.log"));
    }

    /// Regola di registro precedente (`reg_path_allowed`), per misurare sul file
    /// vero cosa scarta in più `regguard`.
    /// Rami di `Software` (con `Wow6432Node` tolto) che non si cancellano interi.
    const REG_BRANCHES: &[&str] = &["classes", "microsoft", "policies"];

    /// Chiavi di registro che una regola può cancellare: `Software\<qualcosa>`.
    /// Per le chiavi intere (`whole_key`) anche: mai `Software`, `Software\Classes`,
    /// `Software\Microsoft`, `Software\Policies` e `Wow6432Node` interi, mai le
    /// chiavi contenitore di Windows (`Microsoft\Windows`, `Windows NT`,
    /// `…\CurrentVersion` e le sue figlie dirette: `Run`, `RunOnce`, `Policies`,
    /// `Uninstall`, `Explorer`, `Winlogon`…). I valori (`|Nome`) seguono la regola
    /// semplice.
    fn previous_reg_path_allowed(path: &str, whole_key: bool) -> bool {
        let mut comps: Vec<String> = path
            .split('\\')
            .filter(|p| !p.is_empty())
            .map(str::to_lowercase)
            .collect();
        if comps.iter().any(|c| c == "..") || comps.first().map(String::as_str) != Some("software") {
            return false;
        }
        if comps.len() < 2 {
            return false;
        }
        if !whole_key {
            return true;
        }
        // La vista a 32 bit ha le stesse regole della principale.
        if comps[1] == "wow6432node" {
            comps.remove(1);
            if comps.len() < 2 {
                return false;
            }
        }
        if comps.len() == 2 && REG_BRANCHES.contains(&comps[1].as_str()) {
            return false;
        }
        if comps[1] == "microsoft" {
            let rest: Vec<&str> = comps[2..].iter().map(String::as_str).collect();
            match rest.as_slice() {
                ["windows"] | ["windows nt"] => return false,
                ["windows" | "windows nt", "currentversion"]
                | ["windows" | "windows nt", "currentversion", _] => return false,
                _ => {}
            }
        }
        true
    }

    /// `XIOX_RULES_WINAPP2=<percorso> cargo test -p xiox-rules parses_the_real_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn parses_the_real_file() {
        let path = std::env::var("XIOX_RULES_WINAPP2").expect("XIOX_RULES_WINAPP2 non impostata");
        let text = std::fs::read_to_string(path).unwrap();
        let t0 = std::time::Instant::now();
        let (w, warnings) = parse(&text);
        let took = t0.elapsed();
        let unknown = warnings.iter().filter(|m| m.contains("chiave sconosciuta")).count();
        eprintln!(
            "voci: {} (dichiarate {:?}), versione {:?}, avvisi: {}, chiavi sconosciute: {}, tempo di lettura: {:?}",
            w.entries.len(),
            w.declared_entries,
            w.version,
            warnings.len(),
            unknown,
            took
        );
        for m in warnings.iter().take(10) {
            eprintln!("  avviso: {m}");
        }
        assert!(w.version.as_deref().unwrap_or("0") >= "260915");
        let n = w.entries.len() as f64;
        assert!((n - 4068.0).abs() / 4068.0 <= 0.01, "voci: {n}");
        let declared = f64::from(w.declared_entries.expect("intestazione senza numero di voci"));
        assert!((declared - n).abs() / declared <= 0.01, "dichiarate {declared}, lette {n}");

        // Conversione con un ambiente finto vuoto: nulla è rilevato, ma non deve andare in panico.
        let (c, _) = to_cleaners_with(&w, &Env::default(), &|_, _| false);
        eprintln!("rilevate con ambiente vuoto: {}", c.len());

        // Quante regole e voci toglie ciascuna regola di sicurezza (indipendente
        // dalla rilevazione; variabili dell'ambiente vero, sola lettura).
        let env = Env::from_process();
        let guard = Guard::new(&env);
        let (mut fk_total, mut fk_dropped) = (0u32, 0u32);
        let (mut wild_extra, mut reg_new, mut dropped_wild, mut dropped_reg) = (0, 0, 0, 0);
        let (mut entries_with_rules, mut dropped_any) = (0, 0);
        // Registro: regola precedente contro `regguard`, con l'elenco di ciò che
        // `regguard` scarta in più, per motivo.
        let mut reg_dropped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        let (mut reg_total, mut reg_prev_refused, mut reg_now_refused) = (0, 0, 0);
        for e in &w.entries {
            let (mut old_ok, mut new_ok, mut only_wild, mut only_reg) = (0, 0, 0, 0);
            for fk in &e.file_keys {
                let ex = expand(&fk.path, &env);
                if ex.is_empty() {
                    continue;
                }
                let old = !ex.iter().all(|p| guard.protected(p).is_some());
                let new = !ex.iter().any(|p| unsafe_files_rule(p, fk, &guard));
                old_ok += old as u32;
                new_ok += new as u32;
                fk_total += 1;
                fk_dropped += u32::from(!new);
                if old && !new {
                    wild_extra += 1;
                    only_wild += 1;
                }
            }
            for rk in &e.reg_keys {
                let old = previous_reg_path_allowed(&rk.path, rk.value.is_none());
                let verdict = reg_allowed(rk.root, &rk.path, rk.value.as_deref());
                let new = verdict.is_ok();
                reg_total += 1;
                reg_prev_refused += u32::from(!old);
                reg_now_refused += u32::from(!new);
                if let Err(why) = verdict {
                    let root = match rk.root {
                        RegRoot::Hkcu => "HKCU",
                        RegRoot::Hklm => "HKLM",
                    };
                    let value = rk.value.as_ref().map(|v| format!("|{v}")).unwrap_or_default();
                    let mark = if old { "  [nuova]" } else { "" };
                    reg_dropped
                        .entry(why)
                        .or_default()
                        .push(format!("{root}\\{}{value}{mark}", rk.path));
                }
                old_ok += old as u32;
                new_ok += new as u32;
                if old && !new {
                    reg_new += 1;
                    only_reg += 1;
                }
            }
            if old_ok > 0 {
                entries_with_rules += 1;
            }
            if old_ok > 0 && new_ok == 0 {
                dropped_any += 1;
                dropped_wild += (only_wild > 0) as u32;
                dropped_reg += (only_reg > 0) as u32;
            }
        }
        eprintln!(
            "nuove regole: FileKey con jolly sotto cartella protetta scartate {wild_extra}, RegKey intere di sistema scartate {reg_new}; voci con azioni prima {entries_with_rules}, voci che restano senza azioni per il nuovo controllo {dropped_any} (di cui con file {dropped_wild}, con registro {dropped_reg}); regole ignorate in lettura {}",
            w.entries.iter().map(|e| e.skipped_rules).sum::<u32>()
        );
        eprintln!("FileKey con variabili definite: {fk_total}, scartate dalle regole di sicurezza: {fk_dropped}");
        let lines = text
            .lines()
            .filter(|l| l.trim_start().to_ascii_lowercase().starts_with("regkey"))
            .count();
        eprintln!(
            "registro: righe RegKey {lines}, lette (HKCU/HKLM) {reg_total}, rifiutate dalla regola precedente {reg_prev_refused}, rifiutate da regguard {reg_now_refused} (per motivo qui sotto; «[nuova]» = la regola precedente la teneva)"
        );
        for (why, keys) in &reg_dropped {
            eprintln!("  {why}: {}", keys.len());
            for k in keys {
                eprintln!("    {k}");
            }
        }
    }
}
