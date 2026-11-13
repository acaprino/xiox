//! Test di `envpath`: solo il finto [`Fake`] (registro, dischi, backup,
//! messaggio a Windows in memoria). Nessun accesso al PC vero, tranne il test
//! `#[ignore]` in sola lettura in fondo.

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// Ambiente finto: blocco nuovo dell'utente e processo uguali.
fn env() -> BlockExpander {
    let mut m = HashMap::new();
    m.insert("systemroot".to_string(), r"C:\Windows".to_string());
    m.insert("windir".to_string(), r"C:\Windows".to_string());
    m.insert("programfiles".to_string(), r"C:\Program Files".to_string());
    m.insert("userprofile".to_string(), r"C:\Users\me".to_string());
    m.insert("localappdata".to_string(), r"C:\Users\me\AppData\Local".to_string());
    m.insert("appdata".to_string(), r"C:\Users\me\AppData\Roaming".to_string());
    BlockExpander { block: m.clone(), process: m }
}

/// `env()` con `name` = `value` in entrambi gli ambienti.
fn env_with(name: &str, value: &str) -> BlockExpander {
    let mut e = env();
    e.block.insert(name.to_string(), value.to_string());
    e.process.insert(name.to_string(), value.to_string());
    e
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
    account: Account,
    /// Il processo ha i diritti di amministratore (può scrivere HKLM).
    admin: bool,
    /// Chiavi chieste all'ultimo backup.
    backed_up: RefCell<Vec<RegRoot>>,
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
            account: Account::Same(r"PC\me".to_string()),
            admin: true,
            backed_up: RefCell::new(Vec::new()),
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
    fn backup(&self, roots: &[RegRoot]) -> Result<PathBuf, String> {
        self.log.borrow_mut().push("backup".to_string());
        *self.backed_up.borrow_mut() = roots.to_vec();
        if let Some((root, v)) = &self.change_on_backup {
            self.set(*root, v.clone());
        }
        match &self.backup_err {
            Some(e) => Err(e.clone()),
            None => Ok(PathBuf::from(r"C:\Users\me\Documents\Xiox\backup_path.reg")),
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
    fn account(&self) -> Account {
        self.account.clone()
    }
    fn system_writable(&self) -> bool {
        self.admin
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
    let p = proposals(RegRoot::Hkcu, &ex(r"C:\Tools\bin;D:\gone"), &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Missing)]);
    assert_eq!(p[0].entry, r"D:\gone");
}

#[test]
fn duplicates_after_normalization_keep_the_first() {
    let fake = Fake::new(None, None);
    let text = r"C:\Users\me\bin;C:\Users\me\BIN\;C:\Tools;c:\users\ME\bin\\;C:\Tools;%USERPROFILE%\x;%userprofile%\X\";
    let p = proposals(RegRoot::Hkcu, &ex(text), &fake, &env()).unwrap();
    assert_eq!(
        indexes(&p),
        vec![(1, Reason::Duplicate), (3, Reason::Duplicate), (4, Reason::Duplicate), (6, Reason::Duplicate)]
    );
}

#[test]
fn a_variable_copy_and_a_literal_copy_are_both_kept() {
    // Se Windows espande `%USERPROFILE%` in un altro modo componendo il PATH,
    // togliere la copia letterale lascerebbe solo quella che non funziona:
    // nessuna delle due è un doppione, in qualunque ordine.
    let fake = Fake::new(None, None);
    for text in [r"%USERPROFILE%\bin;C:\Users\me\bin", r"C:\Users\me\bin;%USERPROFILE%\bin", r"C:\Users\me\bin\;%UserProfile%\BIN"] {
        let p = proposals(RegRoot::Hkcu, &ex(text), &fake, &env()).unwrap();
        assert!(p.is_empty(), "{text}: {p:?}");
    }
    // Una cartella che manca davvero si toglie comunque, ciascuna per conto suo.
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\gone"]);
    let p = proposals(RegRoot::Hkcu, &ex(r"%USERPROFILE%\gone;C:\Users\me\gone"), &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(0, Reason::Missing), (1, Reason::Missing)]);
}

#[test]
fn only_ascii_case_differences_make_duplicates() {
    // Il segno Kelvin U+212A diventa `k` con `to_lowercase`, ma NTFS lo tiene
    // diverso da `K`: due cartelle diverse, tenute tutte e due.
    let fake = Fake::new(None, None);
    let p = proposals(RegRoot::Hkcu, &ex("C:\\Tools\\K;C:\\Tools\\\u{212A}"), &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
    let p = proposals(RegRoot::Hkcu, &ex("C:\\Tools\\\u{212A};C:\\tools\\k"), &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
    // Altre lettere non ASCII: uguali solo se scritte uguali.
    let p = proposals(RegRoot::Hkcu, &ex("C:\\Città;C:\\CITTÀ;C:\\città"), &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(2, Reason::Duplicate)]);
}

#[test]
fn per_user_variables_in_the_system_path_are_never_judged() {
    // In HKLM `%USERPROFILE%` vale per ogni utente in un modo diverso: per un
    // altro account `C:\Users\me\.local\bin` non è un doppione, e
    // `%USERPROFILE%\gone` può esistere.
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\gone", r"C:\Users\me\AppData\Local\old"]);
    let text = r"%USERPROFILE%\.local\bin;C:\Users\me\.local\bin;%UserProfile%\gone;%LOCALAPPDATA%\old;C:\x";
    let p = proposals(RegRoot::Hklm, &ex(text), &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
    // Nel PATH dell'utente invece valgono solo per lui.
    let p = proposals(RegRoot::Hkcu, &ex(text), &fake, &env()).unwrap();
    // (La seconda non è un doppione della prima: variabile contro letterale.)
    assert_eq!(indexes(&p), vec![(2, Reason::Missing), (3, Reason::Missing)]);
}

#[test]
fn a_drive_root_is_not_the_same_as_a_bare_drive() {
    let fake = Fake::new(None, None);
    let p = proposals(RegRoot::Hkcu, &ex(r"D:\;D:;D:\"), &fake, &env()).unwrap();
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
    let p = proposals(RegRoot::Hkcu, &ex(&text), &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn a_folder_that_is_there_or_not_judgeable_is_kept() {
    // C:\Tools non è fra le cartelle mancanti del finto: c'è, o l'errore non
    // è 2/3, o c'è un punto di reparse (lo decide `truly_missing`).
    let fake = Fake::new(None, None);
    let p = proposals(RegRoot::Hkcu, &ex(r"C:\Tools;D:\data\bin"), &fake, &env()).unwrap();
    assert!(p.is_empty());
}

#[test]
fn quoted_entries_are_judged_without_quotes_and_never_split() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone", r"D:\x", r"y"]);
    let p = proposals(RegRoot::Hkcu, &ex(r#""D:\gone";"D:\x;y";D:\gone"#), &fake, &env()).unwrap();
    // Mancano entrambe; la seconda non è un «doppione» della prima.
    assert_eq!(indexes(&p), vec![(0, Reason::Missing), (2, Reason::Missing)]);
}

#[test]
fn a_quoted_copy_and_a_plain_copy_are_both_kept() {
    // `"C:\x"` funziona solo in cmd.exe, `C:\x` ovunque: nessuna delle due è
    // un doppione, in qualunque ordine. Anche con spazi intorno.
    let fake = Fake::new(None, None);
    for text in [r#""C:\x";C:\x"#, r#"C:\x;"C:\x""#, r"C:\x; C:\x", r" C:\x;C:\x"] {
        let p = proposals(RegRoot::Hkcu, &ex(text), &fake, &env()).unwrap();
        assert!(p.is_empty(), "{text}: {p:?}");
    }
}

#[test]
fn redirected_appdata_is_expanded_to_its_real_place_and_kept() {
    // AppData reindirizzata su un server: `%APPDATA%\npm` è UNC, non si
    // giudica, anche se la cartella locale nel profilo manca.
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\AppData\Roaming\npm"]);
    let e = env_with("appdata", r"\\server\profili\me\AppData\Roaming");
    let p = proposals(RegRoot::Hkcu, &ex(r"%APPDATA%\npm;C:\Users\me\AppData\Roaming\npm"), &fake, &e).unwrap();
    // La seconda manca davvero (ed è un'altra cartella): solo lei.
    assert_eq!(indexes(&p), vec![(1, Reason::Missing)]);
    // AppData su D:, dove la cartella c'è: niente da togliere.
    let e = env_with("appdata", r"D:\Profili\me\AppData\Roaming");
    let p = proposals(RegRoot::Hkcu, &ex(r"%APPDATA%\npm"), &fake, &e).unwrap();
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn reg_sz_values_are_literal_and_never_expanded() {
    // In un REG_SZ Windows non espande `%USERPROFILE%`: le due voci sono
    // diverse e si tengono entrambe (la prima non funziona, la seconda sì).
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\gone"]);
    let p = proposals(RegRoot::Hkcu, &sz(r"%USERPROFILE%\bin;C:\Users\me\bin;%USERPROFILE%\gone"), &fake, &env()).unwrap();
    assert!(p.is_empty(), "{p:?}");
    // Lo stesso testo in un REG_EXPAND_SZ: la cartella mancante si toglie;
    // variabile e letterale restano tutte e due (mai doppioni fra loro).
    let p = proposals(RegRoot::Hkcu, &ex(r"%USERPROFILE%\bin;C:\Users\me\bin;%USERPROFILE%\gone"), &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(2, Reason::Missing)]);
    // Nella pulizia vera, dall'analisi.
    let fake = Fake::new(None, Some(sz(r"%USERPROFILE%\bin;C:\Users\me\bin"))).with_missing(&[]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 0, "{:?}", found.detail);
}

#[test]
fn unknown_pseudo_or_uncertain_variables_keep_the_entry() {
    let fake = Fake::new(None, None).with_missing(&[r"C:\Users\me\Documents\x", r"D:\old\bin", r"D:\cargo\bin"]);
    // `%DOCUMENTS%` non esiste in Windows: testo letterale. La seconda voce
    // (quella che funziona) non è un doppione della prima.
    let p = proposals(RegRoot::Hkcu, &ex(r"%DOCUMENTS%\x;C:\Users\me\Documents\x"), &fake, &env()).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Missing)]);
    // Variabile cambiata dopo l'avvio: il processo dice D:\old, il blocco
    // nuovo D:\cargo. Non si sa quale vale: si tiene.
    let mut e = env();
    e.block.insert("cargo_home".into(), r"D:\cargo".into());
    e.process.insert("cargo_home".into(), r"D:\old".into());
    assert_eq!(e.var("CARGO_HOME"), None);
    let p = proposals(RegRoot::Hkcu, &ex(r"%CARGO_HOME%\bin"), &fake, &e).unwrap();
    assert!(p.is_empty(), "{p:?}");
    // Solo nel processo o solo nel blocco: si tiene.
    let mut e = env();
    e.process.insert("only_proc".into(), r"D:\old".into());
    e.block.insert("only_block".into(), r"D:\old".into());
    let p = proposals(RegRoot::Hkcu, &ex(r"%ONLY_PROC%\bin;%ONLY_BLOCK%\bin"), &fake, &e).unwrap();
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn the_system_path_expands_only_windows_own_variables() {
    let fake = Fake::new(None, None).with_missing(&[r"C:\Program Files\gone", r"D:\jdk\bin"]);
    let e = env_with("java_home", r"D:\jdk");
    let p = proposals(RegRoot::Hklm, &ex(r"C:\a;%ProgramFiles%\gone;%JAVA_HOME%\bin"), &fake, &e).unwrap();
    assert_eq!(indexes(&p), vec![(1, Reason::Missing)]);
    // Nel PATH dell'utente sì.
    let p = proposals(RegRoot::Hkcu, &ex(r"%JAVA_HOME%\bin"), &fake, &e).unwrap();
    assert_eq!(indexes(&p), vec![(0, Reason::Missing)]);
}

#[test]
fn environment_block_is_parsed_with_lowercase_names() {
    let text = "=C:=C:\\\0Path=C:\\a;C:\\b\0APPDATA=\\\\srv\\x\0weird\0\0";
    let words: Vec<u16> = text.encode_utf16().collect();
    let m = parse_env_block(&words);
    assert_eq!(m.get("path").map(String::as_str), Some(r"C:\a;C:\b"));
    assert_eq!(m.get("appdata").map(String::as_str), Some(r"\\srv\x"));
    assert_eq!(m.len(), 2, "{m:?}");
}

#[test]
fn unbalanced_quotes_leave_the_whole_value_alone() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone"]);
    assert!(proposals(RegRoot::Hkcu, &ex(r#"D:\gone;"C:\a"#), &fake, &env()).is_err());
}

#[test]
fn without_a_known_windows_folder_nothing_is_proposed() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone"]);
    let mut e = env();
    e.block.remove("systemroot");
    e.block.remove("windir");
    assert!(proposals(RegRoot::Hkcu, &ex(r"D:\gone"), &fake, &e).is_err());
}

#[test]
fn the_system_path_is_never_emptied() {
    let fake = Fake::new(None, None).with_missing(&[r"D:\gone", r"D:\old"]);
    assert!(proposals(RegRoot::Hklm, &ex(r"D:\gone;;D:\old;"), &fake, &env()).is_err());
    // Il PATH dell'utente invece sì: resta vuoto, Windows lo accetta.
    let p = proposals(RegRoot::Hkcu, &ex(r"D:\gone;;D:\old;"), &fake, &env()).unwrap();
    assert_eq!(p.len(), 2);
}

// ---------------------------------------------------------------------------
// Analisi.

#[test]
fn analysis_reads_both_keys_and_reports_entries_without_writing() {
    let fake = Fake::new(Some(sz(r"C:\Windows\system32;D:\gone")), Some(ex(r"C:\Users\me\bin;C:\Users\me\bin\")))
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
        Some(ex(r"%USERPROFILE%\bin;;D:\old;%userprofile%\bin;C:\B")),
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
fn a_path_changed_since_the_analysis_stops_the_whole_item() {
    let fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\gone", r"D:\old"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 2);
    fake.set(RegRoot::Hkcu, ex(r"D:\old;C:\a;C:\new"));
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 0);
    assert_eq!(out.failed, 2);
    assert!(out.notes.iter().any(|n| n.contains("PATH cambiato dopo l'analisi")), "{:?}", out.notes);
    assert!(fake.log().is_empty(), "niente backup né scrittura: {:?}", fake.log());
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"D:\old;C:\a;C:\new")));
    assert_eq!(fake.value(RegRoot::Hklm), Some(sz(r"C:\Windows;D:\gone")));
}

#[test]
fn clean_removes_only_the_previewed_entries() {
    // L'analisi ha mostrato solo D:\gone; nel frattempo manca anche C:\b.
    let fake = Fake::new(None, Some(ex(r"D:\gone;C:\a;C:\b"))).with_missing(&[r"D:\gone"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 1);
    fake.missing.borrow_mut().push(r"c:\b".into());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 1, "{:?}", out.notes);
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"C:\a;C:\b")));
}

#[test]
fn clean_without_a_preview_writes_nothing() {
    let fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(ex(r"D:\old"))).with_missing(&[r"D:\gone", r"D:\old"]);
    let out = clean_path(&fake, &env(), &NativeFound::default(), &no_cancel());
    assert_eq!((out.removed, out.failed), (0, 0));
    assert!(fake.log().is_empty(), "{:?}", fake.log());
    // E la voce dice di fare prima l'analisi (`run_natives`).
    let item = items()[0];
    assert!(item.needs_preview);
    let env = xiox_rules::Env::default();
    let guard = xiox_rules::Guard::new(&env);
    let cancel = no_cancel();
    let ctx = NativeCtx { env: &env, cancel: &cancel, guard: &guard };
    let mut got = Vec::new();
    crate::native::run_natives(&[item], true, &ctx, &|_| None, &Vec::new, &|_, _| {}, &mut |_, f, c| got.push((f, c)));
    assert_eq!(got.len(), 1);
    let c = got[0].1.as_ref().unwrap();
    assert_eq!(c.notes, vec!["Esegui prima Analisi per vedere le voci da togliere".to_string()]);
    assert_eq!(c.removed, 0);
}

#[test]
fn another_account_skips_the_user_path_but_not_the_system_one() {
    let mut fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(ex(r"D:\old"))).with_missing(&[r"D:\gone", r"D:\old"]);
    fake.account = Account::Other(r"PC\admin".into());
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 1);
    assert!(found.detail.iter().any(|d| d.contains("PATH utente di un altro account: non toccato")), "{:?}", found.detail);
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 1);
    assert_eq!(*fake.backed_up.borrow(), vec![RegRoot::Hklm]);
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"D:\old")));
    // Items di HKCU di un'analisi fatta con l'account giusto, pulizia con
    // l'account sbagliato (o non verificato): HKCU non si tocca.
    for acct in [Account::Other(r"PC\admin".into()), Account::Unknown("errore 5".into())] {
        let mut fake = Fake::new(None, Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\old"]);
        let found = analyze_path(&fake, &env());
        assert_eq!(found.count, 1);
        fake.account = acct;
        let out = clean_path(&fake, &env(), &found, &no_cancel());
        assert_eq!((out.removed, out.failed), (0, 1));
        assert!(fake.writes().is_empty());
        assert!(out.notes.iter().any(|n| n.contains("PATH utente")), "{:?}", out.notes);
    }
}

#[test]
fn without_admin_the_system_path_is_shown_but_never_proposed() {
    let mut fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), Some(ex(r"D:\old;C:\a"))).with_missing(&[r"D:\gone", r"D:\old"]);
    fake.admin = false;
    let found = analyze_path(&fake, &env());
    // Solo la voce dell'utente si conta e va a `clean`.
    assert_eq!(found.count, 1, "{:?}", found.detail);
    assert_eq!(found.items.len(), 1);
    assert!(found.items[0].starts_with("HKCU"), "{:?}", found.items);
    assert!(found.detail[0].contains("1 cartella"), "{:?}", found.detail);
    let line = found.detail.iter().find(|d| d.starts_with("Sistema:")).expect("riga di sistema");
    assert!(line.contains(r"D:\gone") && line.contains(ADMIN_ONLY), "{line}");
    assert_eq!(ADMIN_ONLY, "solo con diritti di amministratore: non verrà modificato");
    // La pulizia scrive solo HKCU, con il backup solo di HKCU.
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!((out.removed, out.failed), (1, 0), "{:?}", out.notes);
    assert_eq!(*fake.backed_up.borrow(), vec![RegRoot::Hkcu]);
    assert_eq!(fake.value(RegRoot::Hklm), Some(sz(r"C:\Windows;D:\gone")));
    assert_eq!(fake.value(RegRoot::Hkcu), Some(ex(r"C:\a")));

    // Solo voci di sistema: nessuna da contare, ma si vedono.
    let mut fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), None).with_missing(&[r"D:\gone"]);
    fake.admin = false;
    let found = analyze_path(&fake, &env());
    assert_eq!((found.count, found.items.len()), (0, 0));
    assert!(found.detail[0].contains("senza diritti di amministratore"), "{:?}", found.detail);
    assert!(found.detail.iter().any(|d| d.contains(ADMIN_ONLY)), "{:?}", found.detail);

    // Elementi di HKLM (da un'analisi elevata) con un processo non elevato: rifiutati.
    let fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), None).with_missing(&[r"D:\gone"]);
    let found = analyze_path(&fake, &env());
    assert_eq!(found.count, 1);
    let mut fake = fake;
    fake.admin = false;
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!((out.removed, out.failed), (0, 1));
    assert!(fake.log().is_empty(), "{:?}", fake.log());
    assert!(out.notes.iter().any(|n| n.contains(ADMIN_ONLY)), "{:?}", out.notes);
}

#[test]
fn backup_covers_only_the_keys_to_write_and_names_the_account() {
    // HKCU\Environment senza Path (o assente): il backup è solo di HKLM.
    let fake = Fake::new(Some(sz(r"C:\Windows;D:\gone")), None).with_missing(&[r"D:\gone"]);
    let found = analyze_path(&fake, &env());
    let out = clean_path(&fake, &env(), &found, &no_cancel());
    assert_eq!(out.removed, 1);
    assert_eq!(*fake.backed_up.borrow(), vec![RegRoot::Hklm]);
    assert!(out.notes.iter().any(|n| n.contains(r"account PC\me") && n.contains("backup_path.reg")), "{:?}", out.notes);
}

#[test]
fn backup_text_must_hold_path_for_each_key() {
    let both = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Environment]\r\n\"TEMP\"=\"x\"\r\n\"Path\"=hex(2):43,00\r\n\r\n\
                [HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment]\r\n\"PATH\"=\"C:\\\\a\"\r\n";
    assert!(backup_covers(both, &[RegRoot::Hkcu, RegRoot::Hklm]).is_ok());
    let user_only = "[HKEY_CURRENT_USER\\Environment]\r\n\"Path\"=\"C:\\\\a\"\r\n[HKEY_LOCAL_MACHINE\\SYSTEM\\x]\r\n\"Path\"=\"y\"\r\n";
    assert!(backup_covers(user_only, &[RegRoot::Hkcu]).is_ok());
    assert!(backup_covers(user_only, &[RegRoot::Hklm]).is_err());
    // `Path` sotto un'altra sezione o assente: no.
    let wrong = "[HKEY_CURRENT_USER\\Environment]\r\n\"TEMP\"=\"x\"\r\n[HKEY_CURRENT_USER\\Other]\r\n\"Path\"=\"y\"\r\n";
    assert!(backup_covers(wrong, &[RegRoot::Hkcu]).is_err());
    assert!(backup_covers("", &[RegRoot::Hkcu]).is_err());
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
    assert!(out.notes.iter().any(|n| n.contains("PATH cambiato dopo l'analisi")), "{:?}", out.notes);
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
    assert!(i.needs_preview);
    let w = i.warning.unwrap();
    for part in ["installato dopo", "lettera", "strumenti e build", "solo le voci mostrate dall'ultima Analisi", r"Documenti\Xiox"] {
        assert!(w.contains(part), "{part}: {w}");
    }
    let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap());
    assert!(!text.to_lowercase().contains("bleachbit") && !text.contains("AGG -"));
}

/// Sola lettura sul PC vero: stampa cosa l'analisi proporrebbe (solo
/// percorsi di cartelle, nessun segreto). Non scrive, non fa backup.
#[test]
#[ignore]
fn live_envpath_analysis_read_only() {
    let env = win::real_expander().expect("blocco d'ambiente");
    println!("account: {:?}", win::RealPath.account());
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
