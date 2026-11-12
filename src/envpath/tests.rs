//! Test di `envpath`: solo il finto [`Fake`] (registro, dischi, backup,
//! messaggio a Windows in memoria). Nessun accesso al PC vero, tranne il test
//! `#[ignore]` in sola lettura in fondo.

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

fn env() -> Env {
    let mut m = HashMap::new();
    m.insert("systemroot".to_string(), r"C:\Windows".to_string());
    m.insert("windir".to_string(), r"C:\Windows".to_string());
    m.insert("userprofile".to_string(), r"C:\Users\me".to_string());
    m.insert("localappdata".to_string(), r"C:\Users\me\AppData\Local".to_string());
    Env(m)
}

fn sz(s: &str) -> PathValue {
    PathValue { kind: Kind::Sz, text: s.to_string() }
}

fn ex(s: &str) -> PathValue {
    PathValue { kind: Kind::ExpandSz, text: s.to_string() }
}

/// Registro, dischi, backup e messaggio finti. Le lettere in `fixed` sono
/// dischi fissi presenti; le cartelle in `missing` (minuscole) mancano davvero.
struct Fake {
    hklm: RefCell<Result<Option<PathValue>, String>>,
    hkcu: RefCell<Result<Option<PathValue>, String>>,
    fixed: &'static str,
    missing: RefCell<Vec<String>>,
    backup_err: Option<String>,
    write_err: Option<String>,
    broadcast_err: Option<String>,
    /// Un'altra app cambia il PATH mentre si fa il backup.
    change_on_backup: Option<(RegRoot, PathValue)>,
    log: RefCell<Vec<String>>,
}

impl Fake {
    fn new(hklm: Option<PathValue>, hkcu: Option<PathValue>) -> Fake {
        Fake {
            hklm: RefCell::new(Ok(hklm)),
            hkcu: RefCell::new(Ok(hkcu)),
            fixed: "CD",
            missing: RefCell::new(Vec::new()),
            backup_err: None,
            write_err: None,
            broadcast_err: None,
            change_on_backup: None,
            log: RefCell::new(Vec::new()),
        }
    }

    fn with_missing(self, paths: &[&str]) -> Fake {
        *self.missing.borrow_mut() = paths.iter().map(|p| p.to_lowercase()).collect();
        self
    }

    fn slot(&self, root: RegRoot) -> &RefCell<Result<Option<PathValue>, String>> {
        match root {
            RegRoot::Hklm => &self.hklm,
            RegRoot::Hkcu => &self.hkcu,
        }
    }

    fn set(&self, root: RegRoot, v: PathValue) {
        *self.slot(root).borrow_mut() = Ok(Some(v));
    }

    fn value(&self, root: RegRoot) -> Option<PathValue> {
        self.slot(root).borrow().clone().unwrap()
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn writes(&self) -> Vec<String> {
        self.log().into_iter().filter(|l| l.starts_with("write")).collect()
    }
}

impl PathSys for Fake {
    fn read(&self, root: RegRoot) -> Result<Option<PathValue>, String> {
        self.slot(root).borrow().clone()
    }
    fn write(&self, root: RegRoot, value: &PathValue) -> Result<(), String> {
        if let Some(e) = &self.write_err {
            return Err(e.clone());
        }
        self.log.borrow_mut().push(format!("write {} {:?} {}", regbackup::root_name(root), value.kind, value.text));
        self.set(root, value.clone());
        Ok(())
    }
    fn backup(&self) -> Result<PathBuf, String> {
        self.log.borrow_mut().push("backup".to_string());
        if let Some((root, v)) = &self.change_on_backup {
            self.set(*root, v.clone());
        }
        match &self.backup_err {
            Some(e) => Err(e.clone()),
            None => Ok(PathBuf::from(r"C:\Users\me\Documents\PulisciPC\backup_path.reg")),
        }
    }
    fn volume_is_fixed(&self, p: &Path) -> bool {
        let s = p.to_string_lossy();
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(d), Some(':')) => self.fixed.contains(d.to_ascii_uppercase()),
            _ => false,
        }
    }
    fn truly_missing(&self, p: &Path) -> bool {
        self.missing.borrow().contains(&p.to_string_lossy().to_lowercase())
    }
    fn broadcast(&self) -> Result<(), String> {
        self.log.borrow_mut().push("broadcast".to_string());
        match &self.broadcast_err {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn indexes(p: &[Proposal]) -> Vec<(usize, Reason)> {
    p.iter().map(|p| (p.index, p.reason)).collect()
}

// ---------------------------------------------------------------------------
// Divisione e dati del registro.

#[test]
fn split_respects_quotes_and_rejects_unbalanced_ones() {
    assert_eq!(split_entries(r#"a;"b;c";;d;"#).unwrap(), vec!["a", r#""b;c""#, "", "d", ""]);
    assert_eq!(split_entries("").unwrap(), vec![""]);
    assert!(split_entries(r#"a;"b"#).is_none());
}

#[test]
fn registry_text_is_decoded_strictly_and_encoded_with_one_terminator() {
    let enc = encode_reg_text("C:\\a;é");
    assert_eq!(enc.len(), ("C:\\a;é".encode_utf16().count() + 1) * 2);
    assert_eq!(&enc[enc.len() - 2..], &[0, 0]);
    assert_eq!(decode_reg_text(&enc).unwrap(), "C:\\a;é");
    // Più terminatori in fondo: si tolgono tutti.
    let mut two = enc.clone();
    two.extend_from_slice(&[0, 0]);
    assert_eq!(decode_reg_text(&two).unwrap(), "C:\\a;é");
    // Nessun terminatore: va bene lo stesso.
    assert_eq!(decode_reg_text(&enc[..enc.len() - 2]).unwrap(), "C:\\a;é");
    // Lunghezza dispari, zero in mezzo, UTF-16 non valido: non si tocca.
    assert!(decode_reg_text(&[0x41, 0, 0x42]).is_err());
    assert!(decode_reg_text(&[0x41, 0, 0, 0, 0x42, 0]).is_err());
    assert!(decode_reg_text(&[0x00, 0xD8, 0x41, 0]).is_err());
}

// ---------------------------------------------------------------------------
// Cosa si propone.

#[test]
fn missing_folder_on_a_fixed_volume_is_proposed() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone"]);
    let p = proposals(RegRoot::Hkcu, r"C:\Tools\bin;D:\gone", &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Missing)]);
    assert_eq!(p[0].entry, r"D:\gone");
}

#[test]
fn duplicates_after_expansion_and_normalization_keep_the_first() {
    let fake = Fake::new(None, None);
    let text = r"%USERPROFILE%\bin;C:\Users\me\BIN\;C:\Tools;c:\users\ME\bin\\;C:\Tools";
    let p = proposals(RegRoot::Hkcu, text, &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Duplicate), (3, Reason::Duplicate), (4, Reason::Duplicate)]);
}

#[test]
fn per_user_variables_in_the_system_path_are_never_judged() {
    // In HKLM `%USERPROFILE%` vale per ogni utente in un modo diverso: per un
    // altro account `C:\Users\me\.local\bin` non è un doppione, e
    // `%USERPROFILE%\gone` può esistere.
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\gone", r"C:\Users\me\AppData\Local\old"]);
    let text = r"%USERPROFILE%\.local\bin;C:\Users\me\.local\bin;%UserProfile%\gone;%LOCALAPPDATA%\old;C:\x";
    let p = proposals(RegRoot::Hklm, text, &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
    // Nel PATH dell'utente invece valgono solo per lui.
    let p = proposals(RegRoot::Hkcu, text, &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Duplicate), (2, Reason::Missing), (3, Reason::Missing)]);
}

#[test]
fn a_drive_root_is_not_the_same_as_a_bare_drive() {
    let fake = Fake::new(None, None);
    let p = proposals(RegRoot::Hkcu, r"D:\;D:;D:\", &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(2, Reason::Duplicate)]);
}

#[test]
fn risky_entries_are_never_proposed_even_if_missing_or_repeated() {
    let entries = [
        r"%NOPE%\gone",                // variabile non espandibile
        r"C:\100%\gone",               // `%` aperto
        r"\\server\share\gone",        // UNC
        r"\\server\share\gone",        // UNC ripetuta
        r"E:\gone",                    // disco non fisso (rimovibile, rete)
        r"Z:\gone",                    // disco assente
        r"C:\Windows\gone",            // sotto %SystemRoot%
        r"%SystemRoot%\System32\gone", // sotto %SystemRoot%
        r"C:\Windows\System32",
        r"%SystemRoot%\system32", // doppione, ma sotto %SystemRoot%
        r"relative\gone",         // relativa
        r"\\?\C:\gone",           // percorso letterale
        r"C:\a\..\gone",          // con `..`
        r"C:gone",                // relativa al disco
        r#"C:\a"b"c\gone"#,       // virgolette in mezzo
        "  ",                     // solo spazi
    ];
    let missing: Vec<String> = entries.iter().map(|e| e.trim().to_string()).collect();
    let mut missing: Vec<&str> = missing.iter().map(String::as_str).collect();
    missing.extend([r"C:\Windows\System32\gone", r"\\?\C:\gone", r"C:\gone"]);
    let fake = Fake::new(None, None).with_missing(&missing);
    let text = entries.join(";");
    let p = proposals(RegRoot::Hkcu, &text, &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn a_folder_that_is_there_or_not_judgeable_is_kept() {
    // C:\Tools non è fra le cartelle mancanti del finto: c'è, o l'errore non
    // è 2/3, o c'è un punto di reparse (lo decide `truly_missing`).
    let fake = Fake::new(None, None);
    let p = proposals(RegRoot::Hkcu, r"C:\Tools;D:\data\bin", &fake, &env()).unwrap();
    assert!(p.is_empty());
}

#[test]
fn quoted_entries_are_judged_without_quotes_and_never_split() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone", r"D:\x", r"y"]);
    let p = proposals(RegRoot::Hkcu, r#""D:\gone";"D:\x;y";D:\gone"#, &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(0, Reason::Missing), (2, Reason::Duplicate)]);
}

#[test]
fn unbalanced_quotes_leave_the_whole_value_alone() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone"]);
    assert!(proposals(RegRoot::Hkcu, r#"D:\gone;"C:\a"#, &fake, &env()).is_err());
}

#[test]
fn without_a_known_windows_folder_nothing_is_proposed() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone"]);
    let mut e = env();
    e.0.remove("systemroot");
    e.0.remove("windir");
    assert!(proposals(RegRoot::Hkcu, r"D:\gone", &fake, &e).is_err());
}

#[test]
fn the_system_path_is_never_emptied() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone", r"D:\old"]);
    assert!(proposals(RegRoot::Hklm, r"D:\gone;;D:\old;", &fake, &env()).is_err());
    // Il PATH dell'utente invece sì: resta vuoto, Windows lo accetta.
    let p = proposals(RegRoot::Hkcu, r"D:\gone;;D:\old;", &fake, &env()).unwrap();
    assert_eq!(p.len(), 2);
}

// ---------------------------------------------------------------------------
// Analisi.

#[test]
fn analysis_reads_both_keys_and_reports_entries_without_writing() {
    let fake = Fake::new(Some(sz(r"C:\Windows\system32;D:\gone")), Some(ex(r"%USERPROFILE%\bin;C:\Users\me\bin")))
        .with_missing(&[r"D:\gone"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 2);
    assert_eq!(found.files, 0);
    assert_eq!(found.items.len(), 2);
    assert!(found.detail[0].contains("2 cartelle"), "{:?}", found.detail);
    let all = found.detail.join("\n");
    assert!(all.contains(r"Sistema: D:\gone") && all.contains("non esiste più"), "{all}");
    assert!(all.contains(r"Utente: C:\Users\me\bin") && all.contains("doppione"), "{all}");
    assert!(fake.log().is_empty(), "l'analisi non scrive: {:?}", fake.log());
}

#[test]
fn analysis_notes_unreadable_keys_and_goes_on() {
    let fake = Fake::new(None, Some(sz(r"D:\gone"))).with_missing(&[r"D:\gone"]);
    *fake.hklm.borrow_mut() = Err("accesso negato".to_string());
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 1);
    assert!(found.detail.iter().any(|d| d.contains("Sistema") && d.contains("accesso negato")), "{:?}", found.detail);
}

#[test]
fn a_missing_value_or_a_clean_path_gives_nothing() {
    let fake = Fake::new(Some(sz(r"C:\Windows\system32")), None);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
}

// ---------------------------------------------------------------------------
// Pulizia.

#[test]
fn clean_backs_up_then_rewrites_with_same_type_and_order_then_broadcasts() {
    let fake = Fake::new(
        Some(sz(r"C:\Windows\system32;D:\gone;C:\Tools;")),
        Some(ex(r"%USERPROFILE%\bin;;D:\old;C:\Users\me\bin;C:\B")),
    )
    .with_missing(&[r"D:\gone", r"D:\old"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 3);
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 3, "{:?}", out.notes);
    assert_eq!(out.failed, 0, "{:?}", out.notes);
    assert_eq!(fake.value(RegRoot::Hklm), Some(sz(r"C:\Windows\system32;C:\Tools;")));
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"%USERPROFILE%\bin;C:\B")));
    let log = fake.log();
    assert_eq!(log.first().map(String::as_str), Some("backup"), "{log:?}");
    assert_eq!(log.last().map(String::as_str), Some("broadcast"), "{log:?}");
    assert_eq!(log.iter().filter(|l| *l == "broadcast").count(), 1);
    assert_eq!(fake.writes().len(), 2);
    assert!(out.notes.iter().any(|n| n.contains("backup_path.reg")), "{:?}", out.notes);
    assert_eq!(out.done.len(), 3);
}

#[test]
fn double_semicolons_go_only_with_a_real_change() {
    let fake = Fake::new(None, Some(ex(r"C:\a;;C:\b;;"))).with_missing(&[]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 0, "i `;;` non contano come voci");
    let fake = Fake::new(None, Some(ex(r";C:\a;;D:\gone;;C:\b;;"))).with_missing(&[r"D:\gone"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 1);
    clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"C:\a;C:\b;")));
}

#[test]
fn backup_failure_means_no_write_at_all() {
    let mut fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(sz(r"D:\old"))).with_missing(&[r"D:\gone", r"D:\old"]);
    fake.backup_err = Some("reg.exe terminato con codice 1".to_string());
    let found = analyze_path(&fake, &env());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 0);
    assert_eq!(out.failed, 2);
    assert!(fake.writes().is_empty());
    assert!(!fake.log().contains(&"broadcast".to_string()));
    assert!(out.notes.iter().any(|n| n.contains("Backup") && n.contains("codice 1")), "{:?}", out.notes);
    assert_eq!(fake.value(RegRoot::Hklm), Some(sz(r"C:\Windows;D:\gone")));
}

#[test]
fn a_path_changed_since_the_analysis_is_not_written() {
    let fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\gone", r"D:\old"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 2);
    fake.set(RegRoot::Hkcu, ex(r"D:\old;C:\a;C:\new"));
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 1);
    assert_eq!(out.failed, 1);
    assert!(out.notes.iter().any(|n| n.contains("PATH cambiato, rifai l'analisi")), "{:?}", out.notes);
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"D:\old;C:\a;C:\new")));
    assert_eq!(fake.value(RegRoot::Hklm), Some(sz(r"C:\Windows")));
}

#[test]
fn a_change_during_the_backup_is_caught_by_the_reread_before_writing() {
    let mut fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    fake.change_on_backup = Some((RegRoot::Hkcu, ex(r"D:\old;C:\a;C:\new")));
    let found = analyze_path(&fake, &env());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 0);
    assert!(fake.writes().is_empty());
    assert!(!fake.log().contains(&"broadcast".to_string()));
    assert!(out.notes.iter().any(|n| n.contains("PATH cambiato, rifai l'analisi")), "{:?}", out.notes);
}

#[test]
fn a_folder_that_came_back_is_kept_and_nothing_else_is_written() {
    let fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    let found = analyze_path(&fake, &env());
    fake.missing.borrow_mut().clear(); // disco ricollegato, cartella tornata
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 0);
    assert!(fake.log().is_empty(), "niente backup né scrittura: {:?}", fake.log());
    assert!(out.notes.iter().any(|n| n.contains(r"D:\old") && n.contains("lasciata")), "{:?}", out.notes);
}

#[test]
fn items_not_from_the_analysis_are_refused() {
    let fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    let found = analyze_path(&fake, &env());
    let mut forged = found.clone();
    forged.items = vec![
        "spazzatura".to_string(),
        // Indice giusto, voce diversa.
        found.items[0].replacen(r"D:\old", r"C:\a\", 1),
    ];
    let out = clean_path(&fake, &env(), &forged, &no_cancel());
    assert_eq!(out.removed, 0);
    assert!(out.failed >= 1);
    assert!(fake.writes().is_empty());
}

#[test]
fn a_write_error_is_reported_and_no_broadcast_without_writes() {
    let mut fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    fake.write_err = Some("accesso negato".to_string());
    let found = analyze_path(&fake, &env());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 0);
    assert_eq!(out.failed, 1);
    assert!(out.notes.iter().any(|n| n.contains("accesso negato")), "{:?}", out.notes);
    assert!(!fake.log().contains(&"broadcast".to_string()));
}

#[test]
fn a_failed_broadcast_is_only_a_note() {
    let mut fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    fake.broadcast_err = Some("timeout".to_string());
    let found = analyze_path(&fake, &env());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 1);
    assert!(out.notes.iter().any(|n| n.contains("timeout")), "{:?}", out.notes);
}

#[test]
fn cancel_before_writing_writes_nothing() {
    let fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
    let found = analyze_path(&fake, &env());
    let cancel = AtomicBool::new(true);
    let out = clean_path(&fake, &env(), &found, &cancel);
    assert_eq!(out.removed, 0);
    assert!(fake.writes().is_empty());
}

// ---------------------------------------------------------------------------
// Voce.

#[test]
fn item_is_off_by_default_with_the_agreed_warning() {
    let items = items();
    assert_eq!(items.len(), 1);
    let i = &items[0];
    assert_eq!(i.id, "env_path");
    assert_eq!(i.label, "Percorso ambiente");
    assert_eq!(i.group, "Windows");
    assert!(!i.default_on && i.needs_admin);
    assert_eq!(
        i.warning,
        Some(
            "Toglie dal PATH le cartelle che non esistono più e i doppioni. Un programma che si aspetta una \
             cartella su un disco non ancora collegato potrebbe non trovarla. Backup .reg in Documenti\\PulisciPC."
        )
    );
    let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap());
    assert!(!text.to_lowercase().contains("bleachbit") && !text.contains("AGG -"));
}

/// Sola lettura sul PC vero: stampa cosa l'analisi proporrebbe (solo
/// percorsi di cartelle, nessun segreto). Non scrive, non fa backup.
#[test]
#[ignore]
fn live_envpath_analysis_read_only() {
    let env = Env::for_rules();
    for root in ROOTS {
        match win::RealPath.read(root) {
            Ok(Some(v)) => println!("{}: {:?}, {} caratteri", root_label(root), v.kind, v.text.chars().count()),
            Ok(None) => println!("{}: nessun valore Path", root_label(root)),
            Err(e) => println!("{}: {e}", root_label(root)),
        }
    }
    let found = analyze_path(&win::RealPath, &env);
    println!("count={} items={}", found.count, found.items.len());
    for line in &found.detail {
        println!("  {line}");
    }
}
