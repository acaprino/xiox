//! Di chi è il processo rispetto all'utente della sessione interattiva.
//!
//! Il manifest di release chiede `requireAdministrator`: un utente standard
//! che approva UAC con le credenziali di un altro amministratore fa girare
//! Xiox come quell'account, e allora `HKCU`, Gestione credenziali e le
//! cartelle del profilo sono **sue**, non di chi sta davanti al PC. Le voci
//! che toccano dati dell'utente (`envpath` per il PATH utente, `netcreds`)
//! chiedono [`current`] e, se non è [`Account::Same`], non toccano nulla di
//! quell'utente.

/// L'account del processo rispetto all'utente della sessione interattiva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    /// Lo stesso utente (nome dell'account).
    Same(String),
    /// Un altro account (UAC approvato con le credenziali di un altro
    /// amministratore): nome dell'account del processo.
    Other(String),
    /// Non verificato (motivo).
    Unknown(String),
}

/// L'account vero del processo: SID del token confrontato con quello
/// dell'utente della sessione (`WTSQuerySessionInformationW` per utente e
/// dominio, poi `LookupAccountNameW` ed `EqualSid`). Un errore = `Unknown`.
pub fn current() -> Account {
    win::account().unwrap_or_else(Account::Unknown)
}

/// SID dell'utente del token del processo in forma di testo (`S-1-5-21-…`),
/// per `icacls`.
pub fn user_sid_string() -> Result<String, String> {
    win::user_sid_string()
}

mod win {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, LookupAccountNameW, LookupAccountSidW, TokenUser, SID_NAME_USE, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::System::RemoteDesktop::{
        WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName, WTS_CURRENT_SERVER_HANDLE,
        WTS_CURRENT_SESSION, WTS_INFO_CLASS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::Account;

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Token del processo, chiuso all'uscita.
    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: handle aperto da `OpenProcessToken`, chiuso una volta sola.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// SID dell'utente del token (una `TOKEN_USER` in un buffer allineato).
    fn token_user() -> Result<Vec<u64>, String> {
        let mut h: HANDLE = null_mut();
        // SAFETY: pseudo-handle del processo; `h` è un puntatore valido.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut h) } == 0 {
            return Err(format!("token del processo non aperto: {}", last_error()));
        }
        let t = Token(h);
        let mut len = 0u32;
        // SAFETY: prima chiamata solo per la lunghezza.
        unsafe { GetTokenInformation(t.0, TokenUser, null_mut(), 0, &mut len) };
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        // SAFETY: buffer di almeno `len` byte.
        if unsafe { GetTokenInformation(t.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } == 0 {
            return Err(format!("utente del processo non letto: {}", last_error()));
        }
        Ok(buf)
    }

    fn sid_of(token_user: &[u64]) -> *mut core::ffi::c_void {
        // SAFETY: il buffer contiene una TOKEN_USER valida (da `token_user`).
        unsafe { (*(token_user.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    pub fn user_sid_string() -> Result<String, String> {
        let user = token_user()?;
        let mut p: *mut u16 = null_mut();
        // SAFETY: SID valido; `p` riceve una stringa da liberare con `LocalFree`.
        if unsafe { ConvertSidToStringSidW(sid_of(&user), &mut p) } == 0 || p.is_null() {
            return Err(format!("SID dell'utente non convertito: {}", last_error()));
        }
        // SAFETY: stringa terminata da zero restituita dal sistema.
        let n = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        // SAFETY: `n` caratteri validi.
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) });
        // SAFETY: memoria di `ConvertSidToStringSidW`, liberata una volta.
        unsafe { LocalFree(p.cast()) };
        if s.starts_with("S-1-") && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S') {
            Ok(s)
        } else {
            Err(format!("SID dell'utente non valido: {s}"))
        }
    }

    fn wts_string(class: WTS_INFO_CLASS) -> Result<String, String> {
        let mut p: *mut u16 = null_mut();
        let mut bytes = 0u32;
        // SAFETY: sessione corrente del server locale; `p` allocata dal
        // sistema e liberata con `WTSFreeMemory`.
        let ok = unsafe { WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, class, &mut p, &mut bytes) };
        if ok == 0 || p.is_null() {
            return Err(format!("utente della sessione non letto: {}", last_error()));
        }
        // SAFETY: stringa terminata da zero restituita dal sistema.
        let n = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        // SAFETY: `n` caratteri validi.
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) });
        // SAFETY: memoria di `WTSQuerySessionInformationW`, liberata una volta.
        unsafe { WTSFreeMemory(p.cast()) };
        Ok(s)
    }

    /// `DOMINIO\utente` del SID.
    fn sid_name(sid: *mut core::ffi::c_void) -> Option<String> {
        let (mut name, mut domain) = ([0u16; 257], [0u16; 257]);
        let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
        let mut use_: SID_NAME_USE = 0;
        // SAFETY: SID valido, buffer e lunghezze coerenti.
        let ok = unsafe {
            LookupAccountSidW(null(), sid, name.as_mut_ptr(), &mut nl, domain.as_mut_ptr(), &mut dl, &mut use_)
        };
        (ok != 0).then(|| {
            format!("{}\\{}", String::from_utf16_lossy(&domain[..dl as usize]), String::from_utf16_lossy(&name[..nl as usize]))
        })
    }

    /// SID di un account per nome (`DOMINIO\utente`).
    fn name_sid(account: &str) -> Result<Vec<u64>, String> {
        let w = wide(account);
        let mut sid = vec![0u64; 16]; // 128 byte: più del SID più lungo (68)
        let mut sl = (sid.len() * 8) as u32;
        let mut domain = [0u16; 257];
        let mut dl = domain.len() as u32;
        let mut use_: SID_NAME_USE = 0;
        // SAFETY: nome terminato da zero, buffer e lunghezze coerenti.
        let ok = unsafe {
            LookupAccountNameW(null(), w.as_ptr(), sid.as_mut_ptr().cast(), &mut sl, domain.as_mut_ptr(), &mut dl, &mut use_)
        };
        if ok == 0 {
            return Err(format!("account della sessione {account} non trovato: {}", last_error()));
        }
        Ok(sid)
    }

    pub fn account() -> Result<Account, String> {
        let user = token_user()?;
        let me = sid_of(&user);
        let name = sid_name(me).unwrap_or_else(|| "account sconosciuto".to_string());
        let (u, d) = (wts_string(WTSUserName)?, wts_string(WTSDomainName)?);
        if u.is_empty() {
            return Err("nessun utente nella sessione".into());
        }
        let session = name_sid(&if d.is_empty() { u } else { format!("{d}\\{u}") })?;
        // SAFETY: due SID validi.
        let same = unsafe { EqualSid(me, session.as_ptr() as *mut _) } != 0;
        Ok(if same { Account::Same(name) } else { Account::Other(name) })
    }
}
