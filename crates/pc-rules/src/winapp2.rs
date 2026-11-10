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

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::expand::expand;
use crate::guard::protected;
use crate::model::{Action, Cleaner, Env, ExcludeRule, Opt, Origin, RegRoot, Search};

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
            "detect" => match split_root(value.split('|').next().unwrap_or("")) {
                Some((root, path)) => entry.detect.push(Detect::RegKey {
                    root,
                    path: path.to_string(),
                }),
                None => entry.skipped_rules += 1,
            },
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
                    Some((k, v)) => (k, Some(v.trim()).filter(|v| !v.is_empty())),
                    None => (value, None),
                };
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
    if w.version.is_none() {
        if let Some(v) = comment.strip_prefix("Version:") {
            let v = v.trim();
            if !v.is_empty() {
                w.version = Some(v.to_string());
            }
        }
    }
    if w.declared_entries.is_none() {
        if let Some(v) = comment.strip_prefix("# of entries:") {
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            w.declared_entries = digits.parse().ok();
        }
    }
}

/// `percorso|modelli|FLAG`. `None` se non ci sono modelli né flag (solo il
/// percorso): non si sa cosa cancellare, quindi non si inventa.
fn file_key(value: &str, unknown: &mut BTreeMap<String, u32>) -> Option<FileKey> {
    let mut parts = value.split('|');
    let path = parts.next().unwrap_or("").trim().trim_end_matches('\\');
    if path.is_empty() {
        return None;
    }
    let second = parts.next()?.trim();
    let third = parts.next().map(str::trim).unwrap_or("");
    // Caso raro: il secondo campo è già un flag.
    let (patterns_field, flags_field) = if is_flag(second) && third.is_empty() {
        ("", second)
    } else {
        (second, third)
    };
    let mut patterns: Vec<String> = patterns_field
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if patterns.is_empty() {
        patterns.push("*.*".to_string());
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
    for entry in &w.entries {
        if !detected(entry, env, reg_exists) {
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
            if expanded.iter().all(|p| protected(p, env).is_some()) {
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
            if !reg_path_allowed(&rk.path) {
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
            group: entry
                .section
                .clone()
                .unwrap_or_else(|| "Applicazioni".to_string()),
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

fn detected(entry: &Entry, env: &Env, reg_exists: &dyn Fn(RegRoot, &str) -> bool) -> bool {
    if entry.detect.is_empty() {
        return entry
            .file_keys
            .iter()
            .any(|fk| path_exists(&fk.path, env));
    }
    entry.detect.iter().any(|d| match d {
        Detect::RegKey { root, path } => reg_exists(*root, path),
        Detect::File(path) => path_exists(path, env),
    })
}

/// Solo `Software\<qualcosa>` (anche `Software\Classes\…`).
fn reg_path_allowed(path: &str) -> bool {
    let mut parts = path.split('\\').filter(|p| !p.is_empty());
    let first = parts.next();
    let second = parts.next();
    first.is_some_and(|f| f.eq_ignore_ascii_case("software"))
        && second.is_some()
        && !path.split('\\').any(|p| p == "..")
}

/// Esiste almeno uno dei percorsi ottenuti espandendo le variabili (con `*` e
/// `?` nei componenti).
fn path_exists(path: &str, env: &Env) -> bool {
    expand(path, env).iter().any(|p| exists_wild(p))
}

fn exists_wild(path: &Path) -> bool {
    let mut current: Vec<PathBuf> = vec![PathBuf::new()];
    for comp in path.components() {
        match comp {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.contains(['*', '?']) {
                    let mut next = Vec::new();
                    for dir in &current {
                        let Ok(rd) = std::fs::read_dir(dir) else { continue };
                        for e in rd.flatten() {
                            if wildcard_match(&name, &e.file_name().to_string_lossy()) {
                                next.push(e.path());
                            }
                        }
                    }
                    current = next;
                    if current.is_empty() {
                        return false;
                    }
                } else {
                    for p in &mut current {
                        p.push(&*name);
                    }
                }
            }
            other => {
                for p in &mut current {
                    p.push(other.as_os_str());
                }
            }
        }
    }
    current.iter().any(|p| p.exists())
}

/// `*` e `?`, senza badare alle maiuscole.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
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
        assert_eq!(beta.group, "Applicazioni");
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

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("Adobe Photoshop*", "ADOBE PHOTOSHOP 2026"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("a*b", "acd"));
        assert!(wildcard_match("*.log", "x.y.log"));
    }

    /// `PC_RULES_WINAPP2=<percorso> cargo test -p pc-rules parses_the_real_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn parses_the_real_file() {
        let path = std::env::var("PC_RULES_WINAPP2").expect("PC_RULES_WINAPP2 non impostata");
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
    }
}
