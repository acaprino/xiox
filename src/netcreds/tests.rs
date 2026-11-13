//! Test di N8 con soli finti: niente Gestione credenziali né registro veri
//! (tranne `live_netcreds_analysis_read_only`, `#[ignore]`, di sola lettura).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use xiox_rules::{RegItem, RegRoot};
use windows_sys::Win32::Security::Credentials::{
    CRED_TYPE_DOMAIN_CERTIFICATE, CRED_TYPE_DOMAIN_EXTENDED, CRED_TYPE_DOMAIN_PASSWORD,
    CRED_TYPE_DOMAIN_VISIBLE_PASSWORD, CRED_TYPE_GENERIC, CRED_TYPE_GENERIC_CERTIFICATE,
};

use super::*;
use crate::account::Account;
use crate::native::{NativeCleaned, NativeFound};
use crate::rules::{Backup, RuleRegistry};

/// Segreto di ogni credenziale finta: non deve mai comparire nel resoconto.
const SECRET: &str = "S3greto-che-non-si-vede";

// ---------------------------------------------------------------------------
// Finti.

#[derive(Clone)]
struct Cred {
    kind: u32,
    target: Option<String>,
    user: Option<String>,
    /// Tenuto solo dal finto: il trait non ha modo di restituirlo.
    #[allow(dead_code)]
    secret: String,
}

fn cred(kind: u32, target: &str, user: &str) -> Cred {
    Cred { kind, target: Some(target.into()), user: Some(user.into()), secret: SECRET.into() }
}

fn raw(c: &Cred) -> RawCred {
    RawCred { kind: c.kind, target: c.target.clone(), user: c.user.clone() }
}

#[derive(Default)]
struct FakeCreds {
    store: RefCell<Vec<Cred>>,
    list_err: Option<String>,
    read_err: Vec<String>,
    delete_err: Vec<String>,
    reads: RefCell<Vec<String>>,
    deleted: RefCell<Vec<String>>,
}

impl FakeCreds {
    fn with(creds: Vec<Cred>) -> Self {
        FakeCreds { store: RefCell::new(creds), ..Default::default() }
    }
    fn domain(&self, target: &str) -> Option<usize> {
        self.store.borrow().iter().position(|c| {
            c.kind == CRED_TYPE_DOMAIN_PASSWORD && c.target.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(target))
        })
    }
}

impl Credentials for FakeCreds {
    fn list(&self) -> Result<Vec<RawCred>, String> {
        match &self.list_err {
            Some(e) => Err(e.clone()),
            None => Ok(self.store.borrow().iter().map(raw).collect()),
        }
    }
    fn read(&self, target: &str) -> Result<Option<RawCred>, String> {
        self.reads.borrow_mut().push(target.to_string());
        if self.read_err.iter().any(|t| t == target) {
            return Err("errore di lettura".into());
        }
        Ok(self.domain(target).map(|i| raw(&self.store.borrow()[i])))
    }
    fn delete(&self, target: &str) -> Result<bool, String> {
        if self.delete_err.iter().any(|t| t == target) {
            return Err("Accesso negato".into());
        }
        match self.domain(target) {
            Some(i) => {
                self.store.borrow_mut().remove(i);
                self.deleted.borrow_mut().push(target.to_string());
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

struct FakeFtp(Result<FtpList, String>);

impl FtpAccounts for FakeFtp {
    fn servers(&self) -> Result<FtpList, String> {
        self.0.clone()
    }
}

fn no_ftp() -> FakeFtp {
    FakeFtp(Ok(FtpList::default()))
}

fn ftp(servers: &[(&str, &[&str])]) -> FakeFtp {
    FakeFtp(Ok(FtpList {
        servers: servers
            .iter()
            .map(|(n, u)| FtpServer { name: n.to_string(), users: u.iter().map(|s| s.to_string()).collect() })
            .collect(),
        skipped: 0,
    }))
}

#[derive(Default)]
struct FakeReg {
    /// Percorsi (sotto HKCU) che esistono.
    present: Mutex<Vec<String>>,
    backup_err: Option<String>,
    delete_err: Vec<String>,
    backed_up: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
}

impl FakeReg {
    fn with(paths: &[&str]) -> Self {
        FakeReg { present: Mutex::new(paths.iter().map(|p| p.to_string()).collect()), ..Default::default() }
    }
}

fn key_path(item: &RegItem) -> String {
    match item {
        RegItem::Key { root: RegRoot::Hkcu, path } => path.clone(),
        other => panic!("solo chiavi HKCU: {other:?}"),
    }
}

impl RuleRegistry for FakeReg {
    fn backup(&self, items: &[RegItem]) -> Result<Backup, String> {
        if let Some(e) = &self.backup_err {
            return Err(e.clone());
        }
        let present = self.present.lock().unwrap();
        let flags: Vec<bool> = items.iter().map(|i| present.iter().any(|p| p.eq_ignore_ascii_case(&key_path(i)))).collect();
        self.backed_up.lock().unwrap().extend(items.iter().zip(&flags).filter(|(_, f)| **f).map(|(i, _)| key_path(i)));
        Ok(Backup { path: Some(PathBuf::from(r"C:\Users\x\AppData\Local\PulisciPC\backup\backup_password_ftp_1.reg")), present: flags })
    }
    fn delete(&self, item: &RegItem) -> Result<bool, String> {
        let path = key_path(item);
        if self.delete_err.contains(&path) {
            return Err("chiave non eliminata (errore 5)".into());
        }
        let mut present = self.present.lock().unwrap();
        match present.iter().position(|p| p.eq_ignore_ascii_case(&path)) {
            Some(i) => {
                present.remove(i);
                self.deleted.lock().unwrap().push(path);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

fn ftp_key(server: &str) -> String {
    format!(r"Software\Microsoft\Ftp\Accounts\{server}")
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn me() -> Account {
    Account::Same(r"PC\mario".into())
}

/// Analisi con l'account della sessione.
fn scan(creds: &FakeCreds, f: &FakeFtp) -> NativeFound {
    analyze(creds, f, &me())
}

/// Pulizia con l'account della sessione.
fn wipe(creds: &FakeCreds, f: &FakeFtp, reg: &FakeReg, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    clean(creds, f, reg, &me(), found, cancel)
}

/// Tutto il testo che l'utente può vedere.
fn all_text(found: &NativeFound, cleaned: Option<&NativeCleaned>) -> String {
    let mut t = found.detail.join("\n");
    if let Some(c) = cleaned {
        t.push('\n');
        t.push_str(&c.notes.join("\n"));
        t.push('\n');
        t.push_str(&c.done.join("\n"));
    }
    t
}

fn sample() -> Vec<Cred> {
    vec![
        cred(CRED_TYPE_DOMAIN_PASSWORD, "nas.casa.lan", r"NAS\mario"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "TERMSRV/server01", r"UFFICIO\mario"),
        cred(CRED_TYPE_GENERIC, "git:https://github.com", "mario"),
        cred(CRED_TYPE_GENERIC, "LegacyGeneric:target=foo", "mario"),
        cred(CRED_TYPE_DOMAIN_CERTIFICATE, "smartcard.lan", "mario"),
        cred(CRED_TYPE_DOMAIN_VISIBLE_PASSWORD, "passport.net", "mario@outlook.com"),
        cred(CRED_TYPE_GENERIC_CERTIFICATE, "cert.lan", "mario"),
        cred(CRED_TYPE_DOMAIN_EXTENDED, "ext.lan", "mario"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "MicrosoftAccount:user=mario@outlook.com", "mario@outlook.com"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "microsoftaccount:user=altro@outlook.com", "altro@outlook.com"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "WindowsLive:target=virtualapp/didlogical", "02abc"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "virtualapp/didlogical", "02abc"),
        cred(CRED_TYPE_DOMAIN_PASSWORD, "LegacyGeneric:target=bar", "mario"),
    ]
}

// ---------------------------------------------------------------------------
// Filtro.

#[test]
fn filter_keeps_only_domain_passwords_without_excluded_prefixes() {
    assert!(is_network_password(CRED_TYPE_DOMAIN_PASSWORD, "nas.casa.lan"));
    assert!(is_network_password(CRED_TYPE_DOMAIN_PASSWORD, "TERMSRV/server01"));
    for kind in [
        CRED_TYPE_GENERIC,
        CRED_TYPE_DOMAIN_CERTIFICATE,
        CRED_TYPE_DOMAIN_VISIBLE_PASSWORD,
        CRED_TYPE_GENERIC_CERTIFICATE,
        CRED_TYPE_DOMAIN_EXTENDED,
    ] {
        assert!(!is_network_password(kind, "nas.casa.lan"), "tipo {kind} accettato");
    }
    for t in [
        "MicrosoftAccount:user=a@b.c",
        "MICROSOFTACCOUNT:user=a@b.c",
        "WindowsLive:target=virtualapp/didlogical",
        "windowslive:x",
        "virtualapp/didlogical",
        "VirtualApp/DidLogical:x",
        "LegacyGeneric:target=x",
        "",
        "con\0zero",
    ] {
        assert!(!is_network_password(CRED_TYPE_DOMAIN_PASSWORD, t), "{t:?} accettato");
    }
}

#[test]
fn analysis_lists_only_network_passwords_with_target_and_user() {
    let creds = FakeCreds::with(sample());
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    assert_eq!(found.count, 2, "{:?}", found.detail);
    assert_eq!(found.items.len(), 2);
    assert_eq!((found.files, found.bytes), (0, 0));
    assert_eq!(found.detail[0], "2 password di rete");
    let text = all_text(&found, None);
    assert!(text.contains("nas.casa.lan") && text.contains(r"NAS\mario"), "{text}");
    assert!(text.contains("TERMSRV/server01") && text.contains(r"UFFICIO\mario"), "{text}");
    for never in ["github", "MicrosoftAccount", "microsoftaccount", "WindowsLive", "didlogical", "LegacyGeneric", "smartcard", "passport", "cert.lan", "ext.lan"] {
        assert!(!text.contains(never), "{never} nel resoconto: {text}");
    }
    // L'analisi guarda e basta.
    assert!(creds.deleted.borrow().is_empty() && creds.reads.borrow().is_empty());
}

#[test]
fn no_secret_ever_reaches_the_report() {
    let creds = FakeCreds::with(sample());
    let fs = ftp(&[("ftp.example.com", &["mario"])]);
    let found = scan(&creds, &fs);
    assert!(!all_text(&found, None).contains(SECRET));
    assert!(!found.items.iter().any(|i| i.contains(SECRET)));
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com")]);
    let cleaned = wipe(&creds, &fs, &reg, &found, &no_cancel());
    assert!(!all_text(&found, Some(&cleaned)).contains(SECRET));
}

#[test]
fn unreadable_names_are_skipped_and_counted() {
    let mut list = vec![cred(CRED_TYPE_DOMAIN_PASSWORD, "nas.casa.lan", "mario")];
    list.push(Cred { kind: CRED_TYPE_DOMAIN_PASSWORD, target: None, user: Some("x".into()), secret: SECRET.into() });
    list.push(Cred { kind: CRED_TYPE_DOMAIN_PASSWORD, target: Some("srv".into()), user: None, secret: SECRET.into() });
    // Un nome illeggibile di un altro tipo non conta: non è della voce.
    list.push(Cred { kind: CRED_TYPE_GENERIC, target: None, user: None, secret: SECRET.into() });
    let fs = no_ftp();
    let found = scan(&FakeCreds::with(list), &fs);
    assert_eq!(found.count, 1);
    assert_eq!(found.skipped, 2);
}

#[test]
fn nothing_found_means_empty_result() {
    let fs = no_ftp();
    let found = scan(&FakeCreds::with(vec![cred(CRED_TYPE_GENERIC, "git:x", "m")]), &fs);
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty() && found.detail.is_empty());
}

#[test]
fn enumeration_errors_are_reported_and_ftp_still_listed() {
    let creds = FakeCreds { list_err: Some("errore 1312".into()), ..Default::default() };
    let fs = ftp(&[("ftp.example.com", &[])]);
    let found = scan(&creds, &fs);
    assert_eq!(found.count, 1);
    let text = all_text(&found, None);
    assert!(text.contains("Credenziali di Windows non leggibili: errore 1312"), "{text}");
    let fs = FakeFtp(Err("collegamento del registro".into()));
    let found = scan(&FakeCreds::with(sample()), &fs);
    assert_eq!(found.count, 2);
    assert!(all_text(&found, None).contains("Account FTP non leggibili: collegamento del registro"));
}

// ---------------------------------------------------------------------------
// Pulizia delle credenziali.

#[test]
fn clean_rereads_then_deletes_only_what_the_analysis_found() {
    let creds = FakeCreds::with(sample());
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    // Nata dopo l'analisi: non si tocca.
    creds.store.borrow_mut().push(cred(CRED_TYPE_DOMAIN_PASSWORD, "nuovo.lan", "mario"));
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (2, 0), "{:?}", cleaned.notes);
    assert_eq!(*creds.reads.borrow(), vec!["nas.casa.lan", "TERMSRV/server01"]);
    assert_eq!(*creds.deleted.borrow(), vec!["nas.casa.lan", "TERMSRV/server01"]);
    assert!(creds.domain("nuovo.lan").is_some());
    assert_eq!(creds.store.borrow().len(), sample().len() - 2 + 1);
    assert!(cleaned.done.iter().any(|d| d.contains("nas.casa.lan")), "{:?}", cleaned.done);
}

#[test]
fn credential_gone_between_analysis_and_clean_is_already_absent() {
    let creds = FakeCreds::with(sample());
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    creds.store.borrow_mut().retain(|c| c.target.as_deref() != Some("nas.casa.lan"));
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 0));
    assert!(cleaned.notes.iter().any(|n| n.contains("Già assente") && n.contains("nas.casa.lan")), "{:?}", cleaned.notes);
    assert_eq!(*creds.deleted.borrow(), vec!["TERMSRV/server01"]);
}

#[test]
fn delete_error_counts_as_failed_and_the_others_go_on() {
    let mut creds = FakeCreds::with(sample());
    creds.delete_err = vec!["nas.casa.lan".into()];
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 1));
    assert!(cleaned.notes.iter().any(|n| n.contains("nas.casa.lan") && n.contains("Accesso negato")), "{:?}", cleaned.notes);
    assert_eq!(*creds.deleted.borrow(), vec!["TERMSRV/server01"]);
}

#[test]
fn reread_error_leaves_the_credential() {
    let mut creds = FakeCreds::with(sample());
    creds.read_err = vec!["nas.casa.lan".into()];
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 1));
    assert_eq!(*creds.deleted.borrow(), vec!["TERMSRV/server01"]);
}

#[test]
fn credential_changed_after_analysis_is_left() {
    let creds = FakeCreds::with(sample());
    let fs = no_ftp();
    let found = scan(&creds, &fs);
    // Salvata di nuovo con un altro utente: non è quella che l'utente ha visto.
    for c in creds.store.borrow_mut().iter_mut() {
        if c.target.as_deref() == Some("nas.casa.lan") {
            c.user = Some(r"NAS\admin".into());
        }
    }
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!(cleaned.removed, 1);
    assert!(cleaned.notes.iter().any(|n| n.contains("nas.casa.lan") && n.contains("cambiata")), "{:?}", cleaned.notes);
    assert_eq!(*creds.deleted.borrow(), vec!["TERMSRV/server01"]);
}

#[test]
fn forged_items_outside_the_filter_are_never_deleted() {
    let creds = FakeCreds::with(sample());
    let found = NativeFound {
        count: 3,
        items: vec![
            cred_item("MicrosoftAccount:user=mario@outlook.com", "mario@outlook.com"),
            cred_item("virtualapp/didlogical", "02abc"),
            "qualcos'altro".to_string(),
        ],
        ..Default::default()
    };
    let fs = no_ftp();
    let cleaned = wipe(&creds, &fs, &FakeReg::default(), &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (0, 3), "{:?}", cleaned.notes);
    assert!(creds.deleted.borrow().is_empty());
    assert!(creds.reads.borrow().is_empty(), "nemmeno riletta");
}

#[test]
fn cancel_stops_before_any_delete() {
    let creds = FakeCreds::with(sample());
    let fs = ftp(&[("ftp.example.com", &[])]);
    let found = scan(&creds, &fs);
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com")]);
    let cleaned = wipe(&creds, &fs, &reg, &found, &AtomicBool::new(true));
    assert_eq!(cleaned.removed, 0);
    assert!(creds.deleted.borrow().is_empty());
    assert!(reg.deleted.lock().unwrap().is_empty() && reg.backed_up.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Account FTP.

#[test]
fn ftp_analysis_shows_server_and_user_names_only() {
    let fs = ftp(&[("ftp.example.com", &["mario", "anna"]), ("192.168.1.5", &[])]);
    let found = scan(&FakeCreds::default(), &fs);
    assert_eq!(found.count, 2);
    assert_eq!(found.detail[0], "2 account FTP");
    let text = all_text(&found, None);
    assert!(text.contains("ftp.example.com") && text.contains("mario") && text.contains("anna"), "{text}");
    assert!(text.contains("192.168.1.5"), "{text}");
}

#[test]
fn summary_line_counts_both_kinds() {
    let fs = ftp(&[("ftp.example.com", &[])]);
    let found = scan(&FakeCreds::with(sample()), &fs);
    assert_eq!(found.detail[0], "2 password di rete, 1 account FTP");
    let fs = no_ftp();
    let found = scan(&FakeCreds::with(vec![cred(CRED_TYPE_DOMAIN_PASSWORD, "a", "b")]), &fs);
    assert_eq!(found.detail[0], "1 password di rete");
}

#[test]
fn invalid_ftp_names_are_skipped() {
    let mut list = FtpList::default();
    for n in ["ok.example.com", "", ".", "..", r"a\b", "con\0zero"] {
        list.servers.push(FtpServer { name: n.into(), users: vec![] });
    }
    list.skipped = 1;
    let fs = FakeFtp(Ok(list));
    let found = scan(&FakeCreds::default(), &fs);
    assert_eq!(found.count, 1);
    assert_eq!(found.skipped, 6);
    assert!(valid_server_name("ok.example.com") && valid_server_name("ftp.x"));
    assert!(!valid_server_name(&"a".repeat(256)));
}

#[test]
fn ftp_keys_are_backed_up_then_deleted() {
    let creds = FakeCreds::default();
    let fs = ftp(&[("ftp.example.com", &["mario"]), ("old.example.com", &[])]);
    let found = scan(&creds, &fs);
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com"), &ftp_key("old.example.com"), &ftp_key("altro.example.com")]);
    let cleaned = wipe(&creds, &fs, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (2, 0), "{:?}", cleaned.notes);
    assert_eq!(*reg.backed_up.lock().unwrap(), vec![ftp_key("ftp.example.com"), ftp_key("old.example.com")]);
    assert_eq!(*reg.deleted.lock().unwrap(), vec![ftp_key("ftp.example.com"), ftp_key("old.example.com")]);
    // Solo ciò che l'analisi ha trovato: la chiave nata dopo resta.
    assert_eq!(*reg.present.lock().unwrap(), vec![ftp_key("altro.example.com")]);
    let note = cleaned.notes.iter().find(|n| n.contains("backup_password_ftp_1.reg")).expect("nota del backup");
    assert!(note.contains(r"AppData\Local\PulisciPC\backup") && !note.contains("Documents"), "{note}");
    assert!(note.contains("password FTP cifrate") && note.contains("eliminalo quando tutto funziona"), "{note}");
}

#[test]
fn failed_backup_deletes_no_ftp_key_but_credentials_still_go() {
    let creds = FakeCreds::with(sample());
    let fs = ftp(&[("ftp.example.com", &[])]);
    let found = scan(&creds, &fs);
    let mut reg = FakeReg::with(&[&ftp_key("ftp.example.com")]);
    reg.backup_err = Some("reg.exe terminato con codice 1".into());
    let cleaned = wipe(&creds, &fs, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (2, 1), "{:?}", cleaned.notes);
    assert!(reg.deleted.lock().unwrap().is_empty());
    let note = cleaned.notes.iter().find(|n| n.contains("Backup degli account FTP non riuscito")).expect("nota");
    assert!(note.contains("nessun account FTP eliminato") && note.contains("password di rete eliminate: 2"), "{note}");
    assert_eq!(*creds.deleted.borrow(), vec!["nas.casa.lan", "TERMSRV/server01"]);
}

#[test]
fn ftp_key_gone_before_backup_is_not_deleted() {
    let creds = FakeCreds::default();
    let fs = ftp(&[("ftp.example.com", &[]), ("old.example.com", &[])]);
    let found = scan(&creds, &fs);
    let mut reg = FakeReg::with(&[&ftp_key("ftp.example.com"), &ftp_key("old.example.com")]);
    reg.present.get_mut().unwrap().retain(|p| !p.ends_with("old.example.com"));
    reg.delete_err = vec![];
    let cleaned = wipe(&creds, &fs, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 0));
    assert!(cleaned.notes.iter().any(|n| n.contains("Già assente") && n.contains("old.example.com")), "{:?}", cleaned.notes);
}

#[test]
fn ftp_delete_error_counts_as_failed_and_the_others_go_on() {
    let creds = FakeCreds::default();
    let fs = ftp(&[("ftp.example.com", &[]), ("old.example.com", &[])]);
    let found = scan(&creds, &fs);
    let mut reg = FakeReg::with(&[&ftp_key("ftp.example.com"), &ftp_key("old.example.com")]);
    reg.delete_err = vec![ftp_key("ftp.example.com")];
    let cleaned = wipe(&creds, &fs, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 1));
    assert_eq!(*reg.deleted.lock().unwrap(), vec![ftp_key("old.example.com")]);
}

#[test]
fn forged_ftp_items_never_reach_the_registry() {
    let found = NativeFound {
        count: 2,
        items: vec![ftp_item("..", &[]), ftp_item(r"..\..\Run", &[])],
        ..Default::default()
    };
    let reg = FakeReg::default();
    let fs = ftp(&[("..", &[])]);
    let cleaned = wipe(&FakeCreds::default(), &fs, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (0, 2));
    assert!(reg.backed_up.lock().unwrap().is_empty() && reg.deleted.lock().unwrap().is_empty());
}

#[test]
fn ftp_account_changed_after_the_analysis_is_left() {
    let creds = FakeCreds::default();
    let fs = ftp(&[("ftp.example.com", &["mario"]), ("old.example.com", &["anna"])]);
    let found = scan(&creds, &fs);
    // Dopo l'analisi: un utente in più su un server, l'altro uguale.
    let now = ftp(&[("ftp.example.com", &["mario", "nuovo"]), ("OLD.example.com", &["anna"])]);
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com"), &ftp_key("old.example.com")]);
    let cleaned = wipe(&creds, &now, &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (1, 0), "{:?}", cleaned.notes);
    assert_eq!(*reg.deleted.lock().unwrap(), vec![ftp_key("old.example.com")]);
    assert!(cleaned.notes.iter().any(|n| n.contains("ftp.example.com") && n.contains("cambiato")), "{:?}", cleaned.notes);
    // Sparito dall'elenco: già assente, nessun backup né cancellazione.
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com")]);
    let cleaned = wipe(&creds, &no_ftp(), &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (0, 0));
    assert!(reg.backed_up.lock().unwrap().is_empty() && reg.deleted.lock().unwrap().is_empty());
    // Elenco non rileggibile: nessun account FTP toccato.
    let cleaned = wipe(&creds, &FakeFtp(Err("errore 5".into())), &reg, &found, &no_cancel());
    assert_eq!((cleaned.removed, cleaned.failed), (0, 2));
    assert!(reg.deleted.lock().unwrap().is_empty());
}

#[test]
fn ftp_items_carry_the_users_that_were_shown() {
    let users = vec!["mario".to_string(), "anna".to_string()];
    let item = ftp_item("ftp.example.com", &users);
    assert_eq!(parse_item(&item), Some(Item::Ftp { server: "ftp.example.com", users: vec!["mario", "anna"] }));
    assert_eq!(parse_item(&ftp_item("x", &[])), Some(Item::Ftp { server: "x", users: vec![] }));
    // Il vecchio formato senza utenti non vale più.
    assert_eq!(parse_item("ftp\0x"), None);
}

#[test]
fn another_account_is_neither_listed_nor_touched() {
    let creds = FakeCreds::with(sample());
    let fs = ftp(&[("ftp.example.com", &[])]);
    let reg = FakeReg::with(&[&ftp_key("ftp.example.com")]);
    let seen = scan(&creds, &fs);
    for acct in [Account::Other(r"PC\admin".into()), Account::Unknown("errore 5".into())] {
        let found = analyze(&creds, &fs, &acct);
        assert_eq!(found.count, 0);
        assert!(found.items.is_empty());
        assert_eq!(found.detail.len(), 1, "{:?}", found.detail);
        assert!(found.detail[0].contains("non toccate"), "{:?}", found.detail);
        // Anche con un'anteprima (fatta prima con l'account giusto).
        let cleaned = clean(&creds, &fs, &reg, &acct, &seen, &no_cancel());
        assert_eq!((cleaned.removed, cleaned.failed), (0, 3), "{:?}", cleaned.notes);
        assert!(cleaned.notes[0].contains("non toccate"), "{:?}", cleaned.notes);
        assert!(creds.reads.borrow().is_empty() && creds.deleted.borrow().is_empty());
        assert!(reg.backed_up.lock().unwrap().is_empty() && reg.deleted.lock().unwrap().is_empty());
    }
    let found = analyze(&creds, &fs, &Account::Other(r"PC\admin".into()));
    assert_eq!(found.detail[0], r"Credenziali di un altro account: non toccate (il programma gira come PC\admin)");
}

#[test]
fn every_found_item_is_listed_in_the_analysis() {
    // La pulizia agisce su ciò che si è visto: nessun limite alle righe.
    let list: Vec<Cred> = (0..80).map(|n| cred(CRED_TYPE_DOMAIN_PASSWORD, &format!("srv{n}.lan"), "mario")).collect();
    let found = scan(&FakeCreds::with(list), &no_ftp());
    assert_eq!(found.items.len(), 80);
    assert_eq!(found.detail.iter().filter(|l| l.starts_with("Rete:")).count(), 80);
}

// ---------------------------------------------------------------------------
// Voce.

#[test]
fn item_is_off_with_the_warning_and_no_admin() {
    let items = items();
    assert_eq!(items.len(), 1);
    let i = &items[0];
    assert_eq!(i.id, ID);
    assert_eq!(i.group, "Windows");
    assert_eq!(i.label, "Password di rete");
    assert!(!i.default_on && !i.needs_admin);
    assert!(i.needs_preview, "si elimina solo ciò che l'Analisi ha mostrato");
    let w = i.warning.unwrap();
    for part in ["unità di rete", "Desktop remoto", "account FTP", "Outlook", "Exchange", "ultima Analisi", "account Microsoft"] {
        assert!(w.contains(part), "{part} manca: {w}");
    }
    assert!(i.warning_extra.is_none() && i.running.is_empty());
    let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap());
    assert!(!text.to_lowercase().contains("bleachbit") && !text.contains("AGG -"), "{text}");
    assert!(i.description.contains(".reg") && i.description.contains(r"AppData\Local\PulisciPC\backup"), "{}", i.description);
    assert!(!i.description.contains("Documenti"), "{}", i.description);
    assert!(crate::native::all().iter().any(|n| n.id == ID));
}

// ---------------------------------------------------------------------------
// Dal vivo, sola lettura.

/// Elenca le credenziali (solo nome e tipo) e le sottochiavi FTP vere e
/// stampa i soli nomi di destinazione filtrati: mai utenti, mai segreti, mai
/// `clean`. Rilegge ogni credenziale proposta con `CredReadW` (lettura) per
/// verificare il tipo.
#[test]
#[ignore]
fn live_netcreds_analysis_read_only() {
    let found = analyze(&win::RealCredentials, &win::RealFtp, &crate::account::current());
    println!("elementi: {}, lasciati: {}", found.count, found.skipped);
    for line in &found.detail {
        if !line.starts_with("Rete:") && !line.starts_with("FTP:") {
            println!("{line}");
        }
    }
    for item in &found.items {
        match parse_item(item) {
            Some(Item::Cred { target, .. }) => {
                assert!(is_network_password(CRED_TYPE_DOMAIN_PASSWORD, target), "{target}");
                let again = win::RealCredentials.read(target).expect("CredReadW");
                assert!(again.as_ref().is_some_and(|c| c.kind == CRED_TYPE_DOMAIN_PASSWORD), "{target}");
                println!("credenziale: {target}");
            }
            Some(Item::Ftp { server, .. }) => println!("FTP: {server}"),
            None => panic!("elemento non riconosciuto"),
        }
    }
}
