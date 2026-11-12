//! N8 «Password di rete» (`docs/ccleaner-coverage.md` §2.6): le credenziali
//! di Windows di tipo dominio (unità di rete, cartelle condivise, Desktop
//! remoto) e gli account FTP salvati da Esplora file
//! (`HKCU\Software\Microsoft\Ftp\Accounts`).
//!
//! - Analisi: `CredEnumerateW(NULL, 0)`, solo `CRED_TYPE_DOMAIN_PASSWORD` e
//!   mai i nomi con i prefissi di [`EXCLUDED_PREFIXES`]; delle credenziali si
//!   leggono solo tipo, `TargetName` e `UserName`, mai `CredentialBlob`.
//!   Dell'FTP solo i nomi delle sottochiavi (server) e dei loro valori
//!   (utenti), mai i dati.
//! - Pulizia: solo gli elementi dell'analisi dello stesso job. Ogni
//!   credenziale si rilegge con `CredReadW` subito prima di `CredDeleteW` e
//!   si lascia se è sparita («già assente») o cambiata; le chiavi FTP passano
//!   da [`crate::rules::RuleRegistry`] (backup `.reg` fail-closed,
//!   `reg_allowed`, apertura senza seguire collegamenti).
//!
//! Ogni accesso al sistema passa da un trait ([`Credentials`],
//! [`FtpAccounts`], `RuleRegistry`): le funzioni [`analyze`] e [`clean`]
//! contengono tutta la logica e i test le provano solo con implementazioni
//! finte; quelle vere (`win::Real*`) le usa solo [`items`].

use std::sync::atomic::{AtomicBool, Ordering};

use pc_rules::{RegItem, RegRoot};
use windows_sys::Win32::Security::Credentials::CRED_TYPE_DOMAIN_PASSWORD;

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};
use crate::rules::RuleRegistry;

/// Identificativo della voce (selezione `native:network_passwords`).
pub const ID: &str = "network_passwords";
/// Chiave degli account FTP di Esplora file, sotto `HKCU`.
pub const FTP_ACCOUNTS: &str = r"Software\Microsoft\Ftp\Accounts";
/// Nomi di credenziale che non si toccano mai, anche se di tipo dominio
/// (confronto senza maiuscole): account Microsoft e credenziali generiche
/// «vecchio stile».
pub const EXCLUDED_PREFIXES: [&str; 4] = ["MicrosoftAccount:", "WindowsLive:", "virtualapp/didlogical", "LegacyGeneric:"];
/// Avviso della conferma (§2.6, brief Fase 3b).
pub const WARNING: &str = "Elimina le password salvate per unità di rete, condivisioni e Desktop remoto: dovrai \
     reinserirle. Le password dei browser e degli account Microsoft non si toccano.";

/// Una credenziale come la dà Windows, **senza** il segreto: tipo, nome di
/// destinazione e utente. `None` = testo non valido in UTF-16 (o nome
/// nullo): non si tocca.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCred {
    pub kind: u32,
    pub target: Option<String>,
    pub user: Option<String>,
}

/// Gestione credenziali di Windows dell'utente del processo.
pub trait Credentials {
    /// `CredEnumerateW(NULL, 0)`: tutte le credenziali (nessuna = vuoto).
    fn list(&self) -> Result<Vec<RawCred>, String>;
    /// `CredReadW(target, CRED_TYPE_DOMAIN_PASSWORD)`: `Ok(None)` se non c'è.
    fn read(&self, target: &str) -> Result<Option<RawCred>, String>;
    /// `CredDeleteW(target, CRED_TYPE_DOMAIN_PASSWORD, 0)`: `Ok(true)` se
    /// eliminata, `Ok(false)` se non c'era più.
    fn delete(&self, target: &str) -> Result<bool, String>;
}

/// Un account FTP di Esplora file: la sottochiave (server) e i nomi dei suoi
/// valori (utenti), mai i dati.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtpServer {
    pub name: String,
    pub users: Vec<String>,
}

/// Ciò che si legge di `HKCU\Software\Microsoft\Ftp\Accounts`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtpList {
    pub servers: Vec<FtpServer>,
    /// Sottochiavi lasciate (nome non valido, collegamento, non apribili).
    pub skipped: u64,
}

/// Gli account FTP di Esplora file.
pub trait FtpAccounts {
    /// Sottochiavi di [`FTP_ACCOUNTS`] aperte senza seguire collegamenti;
    /// `Ok` vuoto se la chiave non c'è, `Err` se non si legge.
    fn servers(&self) -> Result<FtpList, String>;
}

// ---------------------------------------------------------------------------
// Voce.

/// La voce N8, spenta, con l'avviso della §2.6.
pub fn items() -> Vec<NativeItem> {
    vec![NativeItem {
        id: ID,
        group: "Windows",
        label: "Password di rete",
        description: "Elimina da Gestione credenziali di Windows le password salvate per unità di rete, cartelle \
             condivise e Desktop remoto, e gli account FTP salvati da Esplora file. Le credenziali generiche (app, \
             Git, browser) e gli account Microsoft non si toccano. Le credenziali eliminate non hanno copia; le \
             chiavi degli account FTP si salvano prima in un file .reg in Documenti\\PulisciPC.",
        warning: Some(WARNING),
        warning_extra: None,
        default_on: false,
        needs_admin: false,
        running: &[],
        analyze: real_analyze,
        clean: real_clean,
    }]
}

fn real_analyze(_: &NativeCtx) -> NativeFound {
    analyze(&win::RealCredentials, &win::RealFtp)
}

fn real_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
    clean(&win::RealCredentials, &crate::rules::RealRegistry, found, ctx.cancel)
}

// ---------------------------------------------------------------------------
// Logica (provata con i finti).

/// Righe di dettaglio per elemento tenute nel resoconto.
const MAX_DETAIL: usize = 50;
/// Utenti FTP mostrati per server.
const MAX_SHOWN_USERS: usize = 5;

/// `true` per una credenziale che la voce può proporre: tipo
/// `CRED_TYPE_DOMAIN_PASSWORD`, nome non vuoto, senza zeri e senza i
/// prefissi di [`EXCLUDED_PREFIXES`] (senza maiuscole).
pub fn is_network_password(kind: u32, target: &str) -> bool {
    kind == CRED_TYPE_DOMAIN_PASSWORD
        && !target.is_empty()
        && !target.contains(SEP)
        && !EXCLUDED_PREFIXES.iter().any(|p| target.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)))
}

/// `true` per un nome di sottochiave FTP che si può cancellare: un solo
/// componente (niente `\`, `.`, `..`, zeri), non vuoto, al più 255 caratteri.
pub fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.encode_utf16().count() <= 255
        && !name.contains(['\\', SEP])
        && name != "."
        && name != ".."
}

/// Percorso (sotto `HKCU`) della chiave di un account FTP.
fn ftp_key_path(server: &str) -> String {
    format!("{FTP_ACCOUNTS}\\{server}")
}

/// Credenziali di rete e account FTP. Guarda e basta: nessuna rilettura,
/// nessuna cancellazione.
pub fn analyze(creds: &dyn Credentials, ftp: &dyn FtpAccounts) -> NativeFound {
    let mut errors: Vec<String> = Vec::new();
    let mut skipped = 0u64;
    let mut found_creds: Vec<(String, String)> = Vec::new();
    match creds.list() {
        Ok(list) => {
            for c in list.into_iter().filter(|c| c.kind == CRED_TYPE_DOMAIN_PASSWORD) {
                match (c.target, c.user) {
                    (Some(target), Some(user)) if !user.contains(SEP) => {
                        let dup = found_creds.iter().any(|(t, _)| t.eq_ignore_ascii_case(&target));
                        if is_network_password(c.kind, &target) && !dup {
                            found_creds.push((target, user));
                        }
                    }
                    _ => skipped += 1,
                }
            }
        }
        Err(e) => errors.push(format!("Credenziali di Windows non leggibili: {e}")),
    }
    let mut servers: Vec<FtpServer> = Vec::new();
    match ftp.servers() {
        Ok(list) => {
            skipped += list.skipped;
            for s in list.servers {
                if valid_server_name(&s.name) && !servers.iter().any(|x| x.name.eq_ignore_ascii_case(&s.name)) {
                    servers.push(s);
                } else {
                    skipped += 1;
                }
            }
        }
        Err(e) => errors.push(format!("Account FTP non leggibili: {e}")),
    }
    let count = (found_creds.len() + servers.len()) as u64;
    if count == 0 && errors.is_empty() && skipped == 0 {
        return NativeFound::default();
    }
    let mut parts: Vec<String> = Vec::new();
    if !found_creds.is_empty() {
        parts.push(format!("{} password di rete", found_creds.len()));
    }
    if !servers.is_empty() {
        parts.push(format!("{} account FTP", servers.len()));
    }
    let mut detail = vec![if parts.is_empty() { "Nessuna password di rete da eliminare".to_string() } else { parts.join(", ") }];
    detail.extend(errors);
    if skipped > 0 {
        detail.push(format!("{skipped} lasciati: nome non leggibile o collegamento del registro"));
    }
    let lines = found_creds
        .iter()
        .map(|(t, u)| if u.is_empty() { format!("Rete: {t}") } else { format!("Rete: {t} (utente {u})") })
        .chain(servers.iter().map(|s| {
            let mut users = s.users.iter().take(MAX_SHOWN_USERS).cloned().collect::<Vec<_>>().join(", ");
            if s.users.len() > MAX_SHOWN_USERS {
                users.push_str(", …");
            }
            if users.is_empty() {
                format!("FTP: {}", s.name)
            } else {
                format!("FTP: {} (utenti: {users})", s.name)
            }
        }));
    detail.extend(lines.take(MAX_DETAIL));
    let items = found_creds.iter().map(|(t, u)| cred_item(t, u)).chain(servers.iter().map(|s| ftp_item(&s.name))).collect();
    NativeFound { detail, count, skipped, items, ..Default::default() }
}

/// Elimina ciò che l'analisi dello stesso job ha trovato (`found.items`) e
/// nient'altro:
/// - ogni credenziale si ricontrolla col filtro, si rilegge con `CredReadW` e
///   si cancella solo se c'è ancora con lo stesso tipo, nome e utente;
///   sparita = «Già assente», un errore = `failed` e si passa alla prossima;
/// - gli account FTP: `reg_allowed`, poi un solo backup `.reg` (se fallisce
///   nessuna chiave si tocca), poi si cancellano solo le chiavi che
///   esistevano al backup.
pub fn clean(creds: &dyn Credentials, reg: &dyn RuleRegistry, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let mut ftp: Vec<(&str, RegItem)> = Vec::new();
    for raw in &found.items {
        if cancel.load(Ordering::Relaxed) {
            out.note("Pulizia interrotta".to_string());
            return out;
        }
        match parse_item(raw) {
            Some(Item::Cred { target, user }) => clean_cred(creds, target, user, &mut out),
            Some(Item::Ftp(name)) => {
                let path = ftp_key_path(name);
                if !valid_server_name(name) {
                    out.failed += 1;
                    out.note(format!("Account FTP non ammesso, lasciato: {}", name.replace(SEP, " ")));
                } else if let Err(why) = pc_rules::reg_allowed(RegRoot::Hkcu, &path, None) {
                    out.failed += 1;
                    out.note(format!("Account FTP {name} non toccato ({why})"));
                } else {
                    ftp.push((name, RegItem::Key { root: RegRoot::Hkcu, path }));
                }
            }
            None => {
                out.failed += 1;
                out.note("Elemento non riconosciuto, lasciato".to_string());
            }
        }
    }
    if ftp.is_empty() {
        return out;
    }
    if cancel.load(Ordering::Relaxed) {
        out.note("Pulizia interrotta".to_string());
        return out;
    }
    let keys: Vec<RegItem> = ftp.iter().map(|(_, i)| i.clone()).collect();
    let backup = match reg.backup(&keys) {
        Ok(b) => b,
        Err(e) => {
            out.failed += ftp.len() as u64;
            out.note(format!("Backup del registro non riuscito ({e}): nessun account FTP eliminato"));
            return out;
        }
    };
    if let Some(p) = &backup.path {
        out.note(format!("Backup del registro: {}", p.display()));
    }
    for (i, (name, item)) in ftp.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            out.note("Pulizia interrotta".to_string());
            break;
        }
        // Una chiave che non c'era al backup non si cancella (potrebbe
        // nascere dopo, senza copia).
        if !backup.present.get(i).copied().unwrap_or(false) {
            out.note(format!("Già assente: account FTP {name}"));
            continue;
        }
        match reg.delete(item) {
            Ok(true) => {
                out.removed += 1;
                out.done.push(format!("Eliminato: account FTP {name}"));
            }
            Ok(false) => out.note(format!("Già assente: account FTP {name}")),
            Err(e) => {
                out.failed += 1;
                out.note(format!("Account FTP {name}: {e}"));
            }
        }
    }
    out
}

/// Una credenziale: filtro, rilettura, cancellazione.
fn clean_cred(creds: &dyn Credentials, target: &str, user: &str, out: &mut NativeCleaned) {
    if !is_network_password(CRED_TYPE_DOMAIN_PASSWORD, target) {
        out.failed += 1;
        out.note(format!("Credenziale non ammessa, lasciata: {}", target.replace(SEP, " ")));
        return;
    }
    match creds.read(target) {
        Ok(None) => {
            out.note(format!("Già assente: {target}"));
            return;
        }
        Err(e) => {
            out.failed += 1;
            out.note(format!("{target}: non si rilegge ({e}), lasciata"));
            return;
        }
        Ok(Some(c)) => {
            let same = c.kind == CRED_TYPE_DOMAIN_PASSWORD
                && c.target.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(target))
                && c.user.as_deref() == Some(user);
            if !same {
                out.note(format!("{target}: cambiata dopo l'analisi, lasciata (rifai l'analisi)"));
                return;
            }
        }
    }
    match creds.delete(target) {
        Ok(true) => {
            out.removed += 1;
            out.done.push(format!("Eliminata: {target}"));
        }
        Ok(false) => out.note(format!("Già assente: {target}")),
        Err(e) => {
            out.failed += 1;
            out.note(format!("{target}: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Elementi di `NativeFound::items`.

/// Separatore dei campi di un elemento: lo zero non può stare in un testo
/// Win32 (nomi di credenziale, utenti, sottochiavi).
const SEP: char = '\0';

/// Elemento di una credenziale: nome e utente visti dall'analisi.
pub fn cred_item(target: &str, user: &str) -> String {
    format!("cred{SEP}{target}{SEP}{user}")
}

/// Elemento di un account FTP: nome della sottochiave.
pub fn ftp_item(server: &str) -> String {
    format!("ftp{SEP}{server}")
}

/// Un elemento letto da `NativeFound::items`.
#[derive(Debug, PartialEq, Eq)]
pub enum Item<'a> {
    Cred { target: &'a str, user: &'a str },
    Ftp(&'a str),
}

/// `None` per un elemento che non viene da [`cred_item`] o [`ftp_item`].
pub fn parse_item(s: &str) -> Option<Item<'_>> {
    let mut parts = s.splitn(3, SEP);
    match (parts.next()?, parts.next(), parts.next()) {
        ("cred", Some(target), Some(user)) => Some(Item::Cred { target, user }),
        ("ftp", Some(server), None) => Some(Item::Ftp(server)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Implementazioni vere (Win32). Mai chiamate dai test normali.

mod win {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CredEnumerateW, CredFree, CredReadW, CREDENTIALW, CRED_TYPE_DOMAIN_PASSWORD};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_WOW64_64KEY, REG_LINK, REG_OPTION_OPEN_LINK,
    };

    use super::{Credentials, FtpAccounts, FtpList, FtpServer, RawCred, FTP_ACCOUNTS};

    /// Lunghezza massima letta di un testo di una credenziale (i nomi veri
    /// sono al più `CRED_MAX_DOMAIN_TARGET_NAME_LENGTH` = 337 caratteri).
    const MAX_TEXT: usize = 32_767;
    /// Sottochiavi FTP lette al più.
    const MAX_SERVERS: usize = 1000;
    /// Nomi di utente letti al più per ogni server FTP (solo per il resoconto).
    const MAX_USERS: usize = 20;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn os_error(code: u32) -> String {
        std::io::Error::from_raw_os_error(code as i32).to_string()
    }

    /// Testo UTF-16 terminato da zero; `None` se nullo, troppo lungo o non
    /// valido (un nome storpiato non si passerebbe a `CredDeleteW`).
    fn text(p: *const u16) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` è una stringa terminata da zero di una CREDENTIALW
        // restituita da Windows; si legge fino allo zero, al più MAX_TEXT u16.
        let n = (0..=MAX_TEXT).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        if n > MAX_TEXT {
            return None;
        }
        // SAFETY: `n` u16 leggibili a partire da `p` (contati sopra).
        String::from_utf16(unsafe { std::slice::from_raw_parts(p, n) }).ok()
    }

    /// Tipo, nome e utente di una credenziale. Si leggono **solo** questi tre
    /// campi: mai `CredentialBlob` né `CredentialBlobSize`.
    fn info(p: *const CREDENTIALW) -> RawCred {
        // SAFETY: `p` punta a una CREDENTIALW valida di CredEnumerateW o
        // CredReadW, ancora da liberare; si copiano solo tre campi.
        let (kind, target, user) = unsafe { ((*p).Type, (*p).TargetName, (*p).UserName) };
        RawCred { kind, target: text(target), user: if user.is_null() { Some(String::new()) } else { text(user) } }
    }

    /// Buffer di CredEnumerateW/CredReadW, liberato con `CredFree` all'uscita
    /// (anche in caso di panico).
    struct CredBuf(*const c_void);

    impl Drop for CredBuf {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: buffer allocato da CredEnumerateW/CredReadW, liberato una volta sola.
                unsafe { CredFree(self.0) };
            }
        }
    }

    /// Gestione credenziali dell'utente del processo.
    pub struct RealCredentials;

    impl Credentials for RealCredentials {
        fn list(&self) -> Result<Vec<RawCred>, String> {
            let mut count = 0u32;
            let mut arr: *mut *mut CREDENTIALW = null_mut();
            // SAFETY: filtro nullo e flag 0 (tutte le credenziali dell'utente,
            // con i nomi così come sono); `count` e `arr` ricevono il risultato.
            let ok = unsafe { CredEnumerateW(null(), 0, &mut count, &mut arr) } != 0;
            if !ok {
                // SAFETY: nessun argomento.
                let e = unsafe { GetLastError() };
                return if e == ERROR_NOT_FOUND { Ok(Vec::new()) } else { Err(os_error(e)) };
            }
            let _buf = CredBuf(arr as *const c_void);
            if arr.is_null() {
                return Ok(Vec::new());
            }
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                // SAFETY: `arr` ha `count` puntatori (CredEnumerateW riuscita).
                let p = unsafe { *arr.add(i) };
                if !p.is_null() {
                    out.push(info(p));
                }
            }
            Ok(out)
        }

        fn read(&self, target: &str) -> Result<Option<RawCred>, String> {
            let w = wide(target);
            let mut p: *mut CREDENTIALW = null_mut();
            // SAFETY: nome terminato da zero, tipo dominio, flag 0; `p` riceve
            // un buffer liberato da `CredBuf`.
            let ok = unsafe { CredReadW(w.as_ptr(), CRED_TYPE_DOMAIN_PASSWORD, 0, &mut p) } != 0;
            if !ok {
                // SAFETY: nessun argomento.
                let e = unsafe { GetLastError() };
                return if e == ERROR_NOT_FOUND { Ok(None) } else { Err(os_error(e)) };
            }
            let _buf = CredBuf(p as *const c_void);
            if p.is_null() {
                return Err("CredReadW senza risultato".into());
            }
            Ok(Some(info(p)))
        }

        fn delete(&self, target: &str) -> Result<bool, String> {
            let w = wide(target);
            // SAFETY: nome terminato da zero, solo il tipo dominio, flag 0 (riservato).
            if unsafe { CredDeleteW(w.as_ptr(), CRED_TYPE_DOMAIN_PASSWORD, 0) } != 0 {
                return Ok(true);
            }
            // SAFETY: nessun argomento.
            let e = unsafe { GetLastError() };
            if e == ERROR_NOT_FOUND {
                Ok(false)
            } else {
                Err(os_error(e))
            }
        }
    }

    // ----- Account FTP -----

    /// Chiave aperta, chiusa all'uscita.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: chiave aperta da `RegOpenKeyExW`, chiusa una volta sola.
            unsafe { RegCloseKey(self.0) };
        }
    }

    /// Apre `name` sotto `parent` **senza seguire** un collegamento
    /// (`REG_OPTION_OPEN_LINK`), nella vista a 64 bit (come `rules`).
    fn open_one(parent: HKEY, name: &[u16], access: u32) -> Result<Key, u32> {
        let mut w = name.to_vec();
        w.push(0);
        let mut out: HKEY = null_mut();
        // SAFETY: `w` termina con 0, `out` è un puntatore valido.
        let rc = unsafe { RegOpenKeyExW(parent, w.as_ptr(), REG_OPTION_OPEN_LINK, access | KEY_WOW64_64KEY, &mut out) };
        if rc == ERROR_SUCCESS {
            Ok(Key(out))
        } else {
            Err(rc)
        }
    }

    /// `true` se la chiave (aperta con `REG_OPTION_OPEN_LINK`) è un
    /// collegamento del registro; `Err` se non si sa.
    fn is_link(key: &Key) -> Result<bool, String> {
        let name = wide("SymbolicLinkValue");
        let mut ty = 0u32;
        // SAFETY: solo il tipo, nessun buffer.
        let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty, null_mut(), null_mut()) };
        match rc {
            ERROR_FILE_NOT_FOUND => Ok(false),
            ERROR_SUCCESS => Ok(ty == REG_LINK),
            rc => Err(format!("collegamento illeggibile (errore {rc})")),
        }
    }

    /// `HKCU\Software\Microsoft\Ftp\Accounts`, un componente alla volta, mai
    /// attraverso un collegamento. `Ok(None)` se non c'è.
    fn open_accounts() -> Result<Option<Key>, String> {
        let mut cur: Option<Key> = None;
        for comp in FTP_ACCOUNTS.split('\\') {
            let parent = cur.as_ref().map(|k| k.0).unwrap_or(HKEY_CURRENT_USER);
            let name: Vec<u16> = comp.encode_utf16().collect();
            let key = match open_one(parent, &name, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS) {
                Ok(k) => k,
                Err(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Err(rc) => return Err(format!("{comp}: non si apre (errore {rc})")),
            };
            if is_link(&key)? {
                return Err(format!("{comp}: collegamento del registro, non si segue"));
            }
            cur = Some(key);
        }
        Ok(cur)
    }

    /// Nomi delle sottochiavi (UTF-16 grezzo), al più `max`.
    fn subkeys(key: &Key, max: usize) -> Result<Vec<Vec<u16>>, String> {
        let mut out = Vec::new();
        for index in 0..max as u32 {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            // SAFETY: buffer e lunghezza validi; classe e data non richieste.
            let rc = unsafe {
                RegEnumKeyExW(key.0, index, buf.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut())
            };
            match rc {
                ERROR_SUCCESS => out.push(buf[..len as usize].to_vec()),
                ERROR_NO_MORE_ITEMS => break,
                rc => return Err(format!("elenco degli account non leggibile (errore {rc})")),
            }
        }
        Ok(out)
    }

    /// Nomi dei valori (mai i dati), al più `max`; quelli illeggibili si saltano.
    fn value_names(key: &Key, max: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u16; 16_384];
        for index in 0u32.. {
            if out.len() >= max {
                break;
            }
            let mut len = buf.len() as u32;
            // SAFETY: buffer del nome e lunghezza validi; tipo e dati non
            // richiesti (puntatori nulli): il valore non si legge.
            let rc = unsafe {
                RegEnumValueW(key.0, index, buf.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut())
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            if let Ok(name) = String::from_utf16(&buf[..len as usize]) {
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
        out
    }

    /// Gli account FTP di Esplora file dell'utente del processo.
    pub struct RealFtp;

    impl FtpAccounts for RealFtp {
        fn servers(&self) -> Result<FtpList, String> {
            let mut list = FtpList::default();
            let Some(accounts) = open_accounts()? else {
                return Ok(list);
            };
            for raw in subkeys(&accounts, MAX_SERVERS)? {
                let Ok(name) = String::from_utf16(&raw) else {
                    list.skipped += 1;
                    continue;
                };
                let child = match open_one(accounts.0, &raw, KEY_QUERY_VALUE) {
                    Ok(k) => k,
                    Err(_) => {
                        list.skipped += 1;
                        continue;
                    }
                };
                if !matches!(is_link(&child), Ok(false)) {
                    list.skipped += 1;
                    continue;
                }
                list.servers.push(FtpServer { name, users: value_names(&child, MAX_USERS) });
            }
            Ok(list)
        }
    }
}

#[cfg(test)]
mod tests;
