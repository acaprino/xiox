//! N7 «Vecchia installazione di Windows» (`docs/ccleaner-coverage.md` §2.6):
//! la cartella `%SystemDrive%\Windows.old` che un aggiornamento di Windows
//! lascia per poter tornare alla versione di prima.
//!
//! La cartella **non si cancella mai a mano** (è di TrustedInstaller, piena di
//! collegamenti fisici e giunzioni): la pulizia la affida al gestore
//! «Previous Installations» di Pulizia disco. Si scrive `StateFlags0099 = 2`
//! solo sotto quella sottochiave di `VolumeCaches`, si lancia
//! `cleanmgr.exe /sagerun:99` per percorso assoluto, si aspetta la fine e poi
//! si toglie **sempre** il valore. Se `StateFlags0099` c'è già sotto una
//! qualsiasi sottochiave (lo ha messo qualcun altro) non si procede.
//!
//! Ogni accesso al sistema passa da un trait ([`Files`], [`VolumeCaches`],
//! [`Cleanmgr`]): `analyze`/`clean` contengono tutta la logica e i test le
//! provano con implementazioni finte; le vere (`win::Real*`) le usa solo
//! [`items`].

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};

/// Id della voce (selezione `native:windows_old`).
pub const ID: &str = "windows_old";
pub const LABEL: &str = "Vecchia installazione di Windows";
pub const WARNING: &str =
    "Elimina i file della versione precedente di Windows: dopo non potrai più tornare indietro alla versione di prima.";
/// Nome della cartella nella radice del disco di sistema.
pub const FOLDER: &str = "Windows.old";
/// Chiave dei gestori di Pulizia disco (HKLM, vista a 64 bit).
pub const VOLUME_CACHES: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches";
/// L'unica sottochiave in cui si scrive.
pub const HANDLER: &str = "Previous Installations";
/// Valore letto da `cleanmgr /sagerun:99`.
pub const FLAG_NAME: &str = "StateFlags0099";
/// `2` = gestore scelto per questa esecuzione di `/sagerun`.
pub const FLAG_VALUE: u32 = 2;
pub const SAGERUN_ARG: &str = "/sagerun:99";
/// Tempo massimo per stimare la dimensione; oltre, «oltre N GB».
pub const SIZE_BUDGET: Duration = Duration::from_secs(10);
/// Oltre questo tempo si smette di aspettare Pulizia disco (che continua da sola).
pub const MAX_WAIT: Duration = Duration::from_secs(2 * 60 * 60);
/// Intervallo fra due controlli di `cleanmgr`.
pub const POLL_STEP: Duration = Duration::from_millis(500);

/// Cosa c'è a un percorso, senza seguire collegamenti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Missing,
    /// Cartella vera (non punto di reparse).
    Dir,
    /// Collegamento simbolico, giunzione o altro punto di reparse.
    Reparse,
    /// File o altro che non è una cartella.
    Other,
    /// Non si riesce a leggere (accesso negato, errore di I/O).
    Unreadable,
}

/// Una voce di una cartella.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: OsString,
    pub dir: bool,
    /// Punto di reparse: una cartella così non si esplora.
    pub reparse: bool,
    pub size: u64,
}

/// Il file system, in sola lettura.
pub trait Files {
    /// Radice del disco di sistema (`C:\`), dalla cartella di Windows
    /// (`GetSystemWindowsDirectoryW`), mai dalle variabili del processo.
    fn system_drive(&self) -> Option<PathBuf>;
    fn kind(&self, p: &Path) -> Kind;
    /// Il contenuto di `dir` (senza `.` e `..`), senza seguire collegamenti.
    fn list(&self, dir: &Path) -> Result<Vec<Entry>, String>;
}

/// I valori `StateFlags0099` dei gestori di Pulizia disco.
pub trait VolumeCaches {
    /// Le sottochiavi di [`VOLUME_CACHES`] che hanno un valore [`FLAG_NAME`]
    /// (di qualsiasi tipo). `Err` se una non si legge.
    fn flagged(&self) -> Result<Vec<String>, String>;
    /// Scrive [`FLAG_NAME`] = [`FLAG_VALUE`] (`REG_DWORD`) **solo** sotto
    /// [`HANDLER`], senza creare la chiave se manca.
    fn set_flag(&self) -> Result<(), String>;
    /// Toglie [`FLAG_NAME`] da [`HANDLER`]; `Ok` anche se non c'è.
    fn remove_flag(&self) -> Result<(), String>;
}

/// `cleanmgr.exe` di sistema.
pub trait Cleanmgr {
    /// `Ok` se l'eseguibile c'è (si controlla prima di scrivere nel registro).
    fn available(&self) -> Result<(), String>;
    /// Avvia `cleanmgr.exe /sagerun:99` per percorso assoluto.
    fn start(&self) -> Result<Box<dyn Child>, String>;
}

/// Un `cleanmgr` avviato. Non c'è modo di terminarlo: annullare vuol dire
/// solo smettere di aspettare.
pub trait Child {
    /// `Some(codice)` quando è finito.
    fn poll(&mut self) -> Result<Option<i32>, String>;
}

/// Dimensione stimata di una cartella.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    /// Fermata per il tempo: `bytes` è un minimo.
    pub timed_out: bool,
    /// Cartelle non leggibili: `bytes` è un minimo.
    pub unreadable: u64,
    pub cancelled: bool,
}

impl Size {
    /// La stima comprende tutto ciò che c'era da leggere.
    pub fn complete(&self) -> bool {
        !self.timed_out && self.unreadable == 0 && !self.cancelled
    }
}

/// Come è finita l'attesa di `cleanmgr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    Exited(i32),
    Cancelled,
    TimedOut,
}

// ---------------------------------------------------------------------------
// Voce.

/// La voce N7, spenta, con l'avviso della §2.6. Bloccata finché `cleanmgr`
/// è aperto (una Pulizia disco già in corso non si sovrappone).
pub fn items() -> Vec<NativeItem> {
    vec![NativeItem {
        id: ID,
        group: crate::sysclean::GROUP,
        label: LABEL,
        description: "Elimina la cartella Windows.old, cioè la copia della versione precedente di Windows che resta \
             dopo un aggiornamento. La elimina Pulizia disco di Windows (cleanmgr), mai una cancellazione a mano; la \
             dimensione mostrata è una stima. Pulizia agisce solo se l'ultima Analisi ha mostrato la cartella.",
        warning: Some(WARNING),
        warning_extra: None,
        default_on: false,
        needs_admin: true,
        running: &["cleanmgr.exe"],
        // Si elimina solo dopo aver visto cartella e dimensione nell'Analisi.
        needs_preview: true,
        analyze: real::analyze,
        clean: real::clean,
    }]
}

/// Collegamento fra la voce e le implementazioni vere.
mod real {
    use super::*;
    use std::time::Instant;

    pub fn analyze(ctx: &NativeCtx) -> NativeFound {
        let start = Instant::now();
        super::analyze(&win::RealFiles, &|| start.elapsed(), ctx.cancel)
    }

    pub fn clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        let start = Instant::now();
        let cleanmgr = win::RealCleanmgr { system32: win::system_dir() };
        let d = Deps {
            fs: &win::RealFiles,
            reg: &win::RealVolumeCaches,
            cleanmgr: &cleanmgr,
            clock: &|| start.elapsed(),
            sleep: &std::thread::sleep,
        };
        super::clean(&d, found, ctx.cancel)
    }
}

// ---------------------------------------------------------------------------
// Analisi.

/// `X:\Windows.old` se la radice del disco di sistema è `X:\` (lettera di
/// unità, niente UNC né `\\?\`); altrimenti `None`.
pub fn windows_old_path(fs: &dyn Files) -> Option<PathBuf> {
    let root = fs.system_drive()?;
    is_drive_root(&root).then(|| root.join(FOLDER))
}

/// Somma delle dimensioni dei file sotto `root`, senza entrare nei punti di
/// reparse (giunzioni come `Documents and Settings`), fermandosi dopo
/// `budget` misurato con `clock` (tempo monotono). I file con collegamenti
/// fisici si contano una volta per nome: la stima può superare lo spazio
/// che si libera davvero.
pub fn measure(fs: &dyn Files, root: &Path, budget: Duration, clock: &dyn Fn() -> Duration, cancel: &AtomicBool) -> Size {
    let start = clock();
    let mut size = Size::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            size.cancelled = true;
            break;
        }
        if clock().saturating_sub(start) >= budget {
            size.timed_out = true;
            break;
        }
        let Ok(entries) = fs.list(&dir) else {
            size.unreadable += 1;
            continue;
        };
        for e in entries {
            if e.name == "." || e.name == ".." {
                continue;
            }
            if e.dir {
                if !e.reparse {
                    stack.push(dir.join(&e.name));
                }
            } else {
                size.files += 1;
                size.bytes = size.bytes.saturating_add(e.size);
            }
        }
    }
    size
}

/// Se `%SystemDrive%\Windows.old` è una cartella vera: un elemento (il suo
/// percorso) con la dimensione stimata in al più [`SIZE_BUDGET`]. Un
/// collegamento, un file o un percorso illeggibile non si propone mai.
pub fn analyze(fs: &dyn Files, clock: &dyn Fn() -> Duration, cancel: &AtomicBool) -> NativeFound {
    let Some(path) = windows_old_path(fs) else {
        return NativeFound::default();
    };
    let shown = path.display().to_string();
    match fs.kind(&path) {
        Kind::Missing => return NativeFound::default(),
        Kind::Dir => {}
        k => {
            return NativeFound {
                detail: vec![format!("{shown} {}: non si tocca", kind_text(k))],
                skipped: 1,
                ..Default::default()
            };
        }
    }
    let size = measure(fs, &path, SIZE_BUDGET, clock, cancel);
    if size.cancelled {
        return NativeFound::default();
    }
    NativeFound {
        detail: vec![
            format!("{shown}: {}", size_text(&size)),
            "Si elimina con Pulizia disco di Windows (cleanmgr), mai a mano; la dimensione è una stima.".to_string(),
        ],
        count: 1,
        files: 0,
        bytes: size.bytes,
        skipped: 0,
        items: vec![path.to_string_lossy().into_owned()],
    }
}

fn kind_text(k: Kind) -> &'static str {
    match k {
        Kind::Reparse => "è un collegamento (punto di reparse)",
        Kind::Other => "non è una cartella",
        Kind::Unreadable => "non si riesce a leggere",
        Kind::Missing => "non esiste",
        Kind::Dir => "è una cartella",
    }
}

fn size_text(s: &Size) -> String {
    let n = crate::sys::fmt_size(s.bytes);
    if s.timed_out {
        format!("oltre {n} (conteggio fermato dopo {} s)", SIZE_BUDGET.as_secs())
    } else if s.unreadable > 0 {
        let what = if s.unreadable == 1 {
            "1 cartella non leggibile".to_string()
        } else {
            format!("{} cartelle non leggibili", s.unreadable)
        };
        format!("almeno {n} ({what})")
    } else {
        format!("circa {n}")
    }
}

// ---------------------------------------------------------------------------
// Pulizia.

/// Aspetta la fine di `cleanmgr` controllando ogni [`POLL_STEP`]. Annullare
/// o superare `max` vuol dire solo smettere di aspettare: `cleanmgr` non si
/// termina mai (ha già letto i gestori scelti e può finire da solo).
pub fn wait_cleanmgr(child: &mut dyn Child, cancel: &AtomicBool, max: Duration, sleep: &dyn Fn(Duration)) -> Result<Waited, String> {
    let mut waited = Duration::ZERO;
    loop {
        if let Some(code) = child.poll()? {
            return Ok(Waited::Exited(code));
        }
        if cancel.load(Ordering::Relaxed) {
            return Ok(Waited::Cancelled);
        }
        if waited >= max {
            return Ok(Waited::TimedOut);
        }
        sleep(POLL_STEP);
        waited += POLL_STEP;
    }
}

/// I sistemi usati da [`clean`].
pub struct Deps<'a> {
    pub fs: &'a dyn Files,
    pub reg: &'a dyn VolumeCaches,
    pub cleanmgr: &'a dyn Cleanmgr,
    pub clock: &'a dyn Fn() -> Duration,
    pub sleep: &'a dyn Fn(Duration),
}

/// Toglie `StateFlags0099` all'uscita di scena se nessuno l'ha già fatto con
/// [`FlagGuard::finish`]: anche su errore, ritorno anticipato o panico.
struct FlagGuard<'a> {
    reg: &'a dyn VolumeCaches,
    armed: bool,
}

impl FlagGuard<'_> {
    fn finish(mut self) -> Result<(), String> {
        self.armed = false;
        self.reg.remove_flag()
    }
}

impl Drop for FlagGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.reg.remove_flag();
        }
    }
}

/// Pulisce solo se l'analisi dello stesso job ha trovato **proprio**
/// `%SystemDrive%\Windows.old` e se è ancora una cartella vera. Prima di
/// scrivere: `cleanmgr` deve esserci e nessuna sottochiave di
/// `VolumeCaches` deve avere già `StateFlags0099` (altrimenti non si procede
/// e non si tocca il valore altrui). Il valore si toglie sempre, anche se la
/// scrittura, l'avvio o l'attesa falliscono.
pub fn clean(d: &Deps, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let Some(path) = windows_old_path(d.fs) else {
        return out;
    };
    let expected = path.to_string_lossy();
    if !found.items.iter().any(|i| i.eq_ignore_ascii_case(&expected)) || cancel.load(Ordering::Relaxed) {
        return out;
    }
    let shown = path.display().to_string();
    match d.fs.kind(&path) {
        Kind::Dir => {}
        Kind::Missing => {
            out.note(format!("{shown}: già assente"));
            return out;
        }
        k => {
            out.failed = 1;
            out.note(format!("{shown} {}: non si tocca", kind_text(k)));
            return out;
        }
    }
    if let Err(e) = d.cleanmgr.available() {
        out.failed = 1;
        out.note(format!("Pulizia disco non disponibile: {e}"));
        return out;
    }
    match d.reg.flagged() {
        Err(e) => {
            out.failed = 1;
            out.note(format!("Impostazioni di Pulizia disco non leggibili ({e}): non si procede"));
            return out;
        }
        Ok(keys) if !keys.is_empty() => {
            out.failed = 1;
            out.note(format!(
                "{FLAG_NAME} è già impostato da un altro programma in: {}. Non si procede, per non avviare pulizie \
                 che non hai scelto.",
                keys.join(", ")
            ));
            return out;
        }
        Ok(_) => {}
    }
    if cancel.load(Ordering::Relaxed) {
        return out;
    }
    let flag = FlagGuard { reg: d.reg, armed: true };
    let ran = d
        .reg
        .set_flag()
        .map_err(|e| format!("{FLAG_NAME} non scritto: {e}"))
        .and_then(|()| d.cleanmgr.start().map_err(|e| format!("cleanmgr.exe non avviato: {e}")))
        .and_then(|mut child| {
            wait_cleanmgr(child.as_mut(), cancel, MAX_WAIT, d.sleep).map_err(|e| format!("attesa di cleanmgr.exe interrotta: {e}"))
        });
    let removed = flag.finish();
    match ran {
        Err(e) => {
            out.failed = 1;
            out.note(e);
        }
        Ok(Waited::Cancelled) => out.note(
            "Annullato: si è smesso di aspettare Pulizia disco (cleanmgr), che può continuare da sola fino alla fine."
                .to_string(),
        ),
        Ok(Waited::TimedOut) => out.note(format!(
            "Pulizia disco non ha finito entro {} ore: si è smesso di aspettare (può continuare da sola).",
            MAX_WAIT.as_secs() / 3600
        )),
        Ok(Waited::Exited(code)) => after_exit(d, &path, found, code, cancel, &mut out),
    }
    if let Err(e) = removed {
        out.failed = 1;
        out.note(format!(
            "{FLAG_NAME} non tolto da HKLM\\{VOLUME_CACHES}\\{HANDLER}: {e}. Toglilo a mano con l'Editor del Registro."
        ));
    }
    out
}

/// Esito dopo la fine di `cleanmgr`: cartella sparita = eliminata (si libera
/// la stima dell'analisi); ancora presente = errore, con lo spazio liberato
/// solo se la nuova stima è completa (mai più di quanto si sa).
fn after_exit(d: &Deps, path: &Path, found: &NativeFound, code: i32, cancel: &AtomicBool, out: &mut NativeCleaned) {
    let shown = path.display().to_string();
    let kind = d.fs.kind(path);
    if kind == Kind::Missing {
        out.removed = 1;
        out.freed = found.bytes;
        out.done.push(format!("{shown} (eliminata da Pulizia disco)"));
        return;
    }
    let mut left = String::new();
    if kind == Kind::Dir {
        let after = measure(d.fs, path, SIZE_BUDGET, d.clock, cancel);
        if after.complete() {
            out.freed = found.bytes.saturating_sub(after.bytes);
        }
        left = format!(" ({})", size_text(&after));
    }
    let code = if code != 0 { format!(" con codice {code}") } else { String::new() };
    out.failed = 1;
    out.note(format!(
        "Pulizia disco è terminata{code} ma {shown} è ancora presente{left}: riprova dopo un riavvio oppure da \
         Impostazioni > Sistema > Archiviazione > File temporanei."
    ));
}

/// `true` solo per `X:\` (lettera di unità e radice, nient'altro).
fn is_drive_root(p: &Path) -> bool {
    let mut c = p.components();
    matches!(c.next(), Some(Component::Prefix(pre)) if matches!(pre.kind(), Prefix::Disk(_)))
        && matches!(c.next(), Some(Component::RootDir))
        && c.next().is_none()
}

// ---------------------------------------------------------------------------
// Implementazioni vere (Win32). I test usano solo `RealFiles` su una
// cartella temporanea; registro e `cleanmgr` mai.

mod win {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_LOCAL_MACHINE, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_DWORD, REG_LINK,
        REG_OPTION_OPEN_LINK,
    };
    use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetSystemWindowsDirectoryW};

    use super::{Child, Cleanmgr, Entry, Files, Kind, VolumeCaches, FLAG_NAME, FLAG_VALUE, HANDLER, SAGERUN_ARG, VOLUME_CACHES};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn os_error(code: u32) -> String {
        std::io::Error::from_raw_os_error(code as i32).to_string()
    }

    /// `%SystemRoot%\System32` da `GetSystemDirectoryW` (mai dal `PATH`).
    pub fn system_dir() -> Option<PathBuf> {
        let mut buf = [0u16; 520];
        // SAFETY: `buf` ha `buf.len()` u16 scrivibili e la dimensione passata è quella.
        let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        (n > 0 && n < buf.len()).then(|| PathBuf::from(OsString::from_wide(&buf[..n])))
    }

    // ----- File system (sola lettura) -----

    pub struct RealFiles;

    /// Ricerca aperta da `FindFirstFileExW`, chiusa all'uscita.
    struct Find(HANDLE);

    impl Drop for Find {
        fn drop(&mut self) {
            // SAFETY: handle valido di `FindFirstFileExW`, chiuso una volta sola.
            unsafe { FindClose(self.0) };
        }
    }

    /// `<dir>\*` in UTF-16 terminato da zero, con `\\?\` davanti ai percorsi
    /// `X:\…` (Windows.old contiene percorsi oltre `MAX_PATH`).
    fn search_pattern(dir: &Path) -> Vec<u16> {
        let mut c = dir.components();
        let plain_disk = matches!(c.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
            && matches!(c.next(), Some(Component::RootDir));
        let mut w: Vec<u16> = Vec::new();
        if plain_disk {
            w.extend(r"\\?\".encode_utf16());
        }
        w.extend(dir.as_os_str().encode_wide());
        if w.last() != Some(&(b'\\' as u16)) {
            w.push(b'\\' as u16);
        }
        w.push(b'*' as u16);
        w.push(0);
        w
    }

    impl Files for RealFiles {
        fn system_drive(&self) -> Option<PathBuf> {
            let mut buf = [0u16; 520];
            // SAFETY: `buf` ha `buf.len()` u16 scrivibili e la dimensione passata è quella.
            let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 || n >= buf.len() {
                return None;
            }
            let windir = PathBuf::from(OsString::from_wide(&buf[..n]));
            match windir.components().next() {
                Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)) => {
                    let mut root = p.as_os_str().to_os_string();
                    root.push("\\");
                    Some(PathBuf::from(root))
                }
                _ => None,
            }
        }

        fn kind(&self, p: &Path) -> Kind {
            use std::os::windows::fs::MetadataExt;
            match std::fs::symlink_metadata(p) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Kind::Missing,
                Err(_) => Kind::Unreadable,
                Ok(md) if md.file_type().is_symlink() || md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
                    Kind::Reparse
                }
                Ok(md) if md.is_dir() => Kind::Dir,
                Ok(_) => Kind::Other,
            }
        }

        fn list(&self, dir: &Path) -> Result<Vec<Entry>, String> {
            let pattern = search_pattern(dir);
            let mut data = WIN32_FIND_DATAW::default();
            // SAFETY: `pattern` termina con 0; con `FindExInfoBasic` la funzione
            // scrive un `WIN32_FIND_DATAW` intero in `data`; nessun filtro. La
            // ricerca si chiude con `Find` su ogni percorso.
            let h = unsafe {
                FindFirstFileExW(
                    pattern.as_ptr(),
                    FindExInfoBasic,
                    (&mut data as *mut WIN32_FIND_DATAW).cast::<c_void>(),
                    FindExSearchNameMatch,
                    null(),
                    FIND_FIRST_EX_LARGE_FETCH,
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let find = Find(h);
            let mut out = Vec::new();
            loop {
                let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
                let name = &data.cFileName[..len];
                if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                    let attrs = data.dwFileAttributes;
                    out.push(Entry {
                        name: OsString::from_wide(name),
                        dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
                        reparse: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
                    });
                }
                // SAFETY: ricerca valida (`find`), `data` scrivibile.
                if unsafe { FindNextFileW(find.0, &mut data) } == 0 {
                    // SAFETY: nessun argomento; subito dopo la chiamata fallita.
                    let err = unsafe { GetLastError() };
                    if err == ERROR_NO_MORE_FILES {
                        break;
                    }
                    return Err(os_error(err));
                }
            }
            Ok(out)
        }
    }

    // ----- Registro -----

    pub struct RealVolumeCaches;

    /// Chiave aperta, chiusa all'uscita.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: chiave aperta da `RegOpenKeyExW`, chiusa una volta sola.
            unsafe { RegCloseKey(self.0) };
        }
    }

    /// Apre `path` sotto `parent` nella vista a 64 bit (`options` = 0 o
    /// `REG_OPTION_OPEN_LINK`). Mai `RegCreateKeyExW`: nessuna chiave nuova.
    fn open(parent: HKEY, path: &str, options: u32, access: u32) -> Result<Key, u32> {
        let w = wide(path);
        let mut out: HKEY = null_mut();
        // SAFETY: `w` termina con 0, `out` è un puntatore valido.
        let rc = unsafe { RegOpenKeyExW(parent, w.as_ptr(), options, access | KEY_WOW64_64KEY, &mut out) };
        if rc == ERROR_SUCCESS {
            Ok(Key(out))
        } else {
            Err(rc)
        }
    }

    /// `Ok(true)` se il valore `name` (terminato da zero) esiste, di qualsiasi tipo.
    fn value_exists(key: &Key, name: &[u16]) -> Result<bool, u32> {
        // SAFETY: chiave aperta, nome terminato da zero; nessun dato richiesto.
        let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), null_mut(), null_mut(), null_mut()) };
        match rc {
            ERROR_SUCCESS => Ok(true),
            ERROR_FILE_NOT_FOUND => Ok(false),
            rc => Err(rc),
        }
    }

    /// La chiave (aperta con `REG_OPTION_OPEN_LINK`) è un collegamento del registro.
    fn is_link(key: &Key) -> bool {
        let name = wide("SymbolicLinkValue");
        let mut ty = 0u32;
        // SAFETY: chiave aperta, nome terminato da zero; solo il tipo.
        let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty, null_mut(), null_mut()) };
        rc == ERROR_SUCCESS && ty == REG_LINK
    }

    fn handler_path() -> String {
        format!("{VOLUME_CACHES}\\{HANDLER}")
    }

    impl VolumeCaches for RealVolumeCaches {
        fn flagged(&self) -> Result<Vec<String>, String> {
            let root = open(HKEY_LOCAL_MACHINE, VOLUME_CACHES, 0, KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE)
                .map_err(|rc| format!("HKLM\\{VOLUME_CACHES}: {}", os_error(rc)))?;
            let flag = wide(FLAG_NAME);
            let mut out = Vec::new();
            for i in 0u32.. {
                // Nomi di chiave: al più 255 caratteri.
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                // SAFETY: `name` ha `len` u16 scrivibili; classe e data non richieste.
                let rc = unsafe {
                    RegEnumKeyExW(root.0, i, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut())
                };
                if rc == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if rc != ERROR_SUCCESS {
                    return Err(format!("sottochiavi di VolumeCaches: {}", os_error(rc)));
                }
                let sub = String::from_utf16_lossy(&name[..len as usize]);
                let key = open(root.0, &sub, 0, KEY_QUERY_VALUE).map_err(|rc| format!("{sub}: {}", os_error(rc)))?;
                match value_exists(&key, &flag) {
                    Ok(true) => out.push(sub),
                    Ok(false) => {}
                    Err(rc) => return Err(format!("{sub}\\{FLAG_NAME}: {}", os_error(rc))),
                }
            }
            Ok(out)
        }

        fn set_flag(&self) -> Result<(), String> {
            let path = handler_path();
            let key = open(HKEY_LOCAL_MACHINE, &path, REG_OPTION_OPEN_LINK, KEY_SET_VALUE | KEY_QUERY_VALUE).map_err(|rc| {
                if rc == ERROR_FILE_NOT_FOUND {
                    format!("gestore «{HANDLER}» di Pulizia disco assente")
                } else {
                    format!("HKLM\\{path}: {}", os_error(rc))
                }
            })?;
            if is_link(&key) {
                return Err(format!("HKLM\\{path} è un collegamento del registro: non si scrive"));
            }
            let name = wide(FLAG_NAME);
            let data = FLAG_VALUE.to_le_bytes();
            // SAFETY: chiave aperta in scrittura, nome terminato da zero, 4 byte di DWORD.
            let rc = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, data.as_ptr(), data.len() as u32) };
            if rc == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(os_error(rc))
            }
        }

        fn remove_flag(&self) -> Result<(), String> {
            let path = handler_path();
            let key = match open(HKEY_LOCAL_MACHINE, &path, REG_OPTION_OPEN_LINK, KEY_SET_VALUE) {
                Ok(k) => k,
                Err(ERROR_FILE_NOT_FOUND) => return Ok(()),
                Err(rc) => return Err(format!("HKLM\\{path}: {}", os_error(rc))),
            };
            let name = wide(FLAG_NAME);
            // SAFETY: chiave aperta in scrittura, nome terminato da zero.
            let rc = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(os_error(rc))
            }
        }
    }

    // ----- cleanmgr -----

    /// `cleanmgr.exe` della cartella di sistema, per percorso assoluto.
    pub struct RealCleanmgr {
        pub system32: Option<PathBuf>,
    }

    impl RealCleanmgr {
        fn exe(&self) -> Result<PathBuf, String> {
            self.system32
                .as_ref()
                .map(|d| d.join("cleanmgr.exe"))
                .filter(|p| p.is_file())
                .ok_or_else(|| "cleanmgr.exe non trovato nella cartella di sistema".to_string())
        }
    }

    impl Cleanmgr for RealCleanmgr {
        fn available(&self) -> Result<(), String> {
            self.exe().map(|_| ())
        }

        fn start(&self) -> Result<Box<dyn Child>, String> {
            let exe = self.exe()?;
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg(SAGERUN_ARG)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(dir) = &self.system32 {
                cmd.current_dir(dir);
            }
            let child = cmd.spawn().map_err(|e| e.to_string())?;
            Ok(Box::new(RealChild(child)))
        }
    }

    /// Il `cleanmgr` avviato. `std::process::Child` non termina il processo
    /// quando esce di scena: smettere di aspettare non lo uccide.
    struct RealChild(std::process::Child);

    impl Child for RealChild {
        fn poll(&mut self) -> Result<Option<i32>, String> {
            self.0.try_wait().map(|s| s.map(|st| st.code().unwrap_or(-1))).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests;
