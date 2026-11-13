//! N6 «Pulisci spazio libero» (tabella di copertura,
//! `docs/ccleaner-coverage.md` §2.6): sovrascrive con zeri lo spazio libero
//! del volume di sistema perché i file già cancellati non si possano
//! recuperare.
//!
//! Solo su un disco fisso NTFS con testine (HDD): su un SSD non serve
//! (Windows usa TRIM) e si rifiuta, come quando il tipo non si determina o lo
//! spazio libero è sotto [`MIN_FREE`]. I file di zeri vanno in una cartella
//! nascosta `<volume>\Xiox_wipe_<pid>_<nanos>` (`PulisciPC_wipe_…` è il nome di
//! prima e si riconosce ancora) e ci si ferma quando
//! restano [`RESERVE`] liberi (riserva per Windows, il file di paging e le
//! altre app: CCleaner invece riempie il disco). La cartella si cancella
//! sempre: alla fine, su errore, su annullamento e anche su panico (guard con
//! `Drop`). In più ogni file di riempimento è aperto con
//! `FILE_FLAG_DELETE_ON_CLOSE`: se il processo muore, Windows lo cancella da
//! sé alla chiusura dell'handle. Una cartella rimasta (mancanza di corrente,
//! schermata blu) la toglie [`cleanup_leftovers`]: all'avvio dell'app in un
//! thread a parte ([`remove_leftovers_in_background`]) e all'inizio di ogni
//! pulizia di questa voce. L'analisi la segnala soltanto (sola lettura).
//!
//! Portata dichiarata nella voce: solo lo spazio libero del volume di sistema.
//! Nessuna fase sulla MFT (i file piccolissimi che stavano dentro la MFT
//! restano) e nessun altro volume. Nemmeno le copie shadow (punti di
//! ripristino) si toccano.
//!
//! Ogni accesso al disco passa dal trait [`Volume`]: la logica sta in
//! [`analyze_wipe`]/[`clean_wipe`] e i test la provano con un volume finto.
//! L'implementazione vera (`win::RealVolume`) la usa solo [`items`].

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use xiox_rules::Guard;

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};
use crate::sys::fmt_size;

/// Gruppo del pannello.
pub const GROUP: &str = "Windows";
/// Sotto questo spazio libero il volume non si pulisce.
pub const MIN_FREE: u64 = 2 << 30;
/// Spazio lasciato libero durante la scrittura.
pub const RESERVE: u64 = 1 << 30;
/// Un blocco di scrittura; fra un blocco e l'altro si rileggono annullamento
/// e spazio libero.
pub const BLOCK: u64 = 64 << 20;
/// Margine sopra la riserva: ci si ferma un blocco prima, così anche se
/// un'altra app consuma spazio durante l'ultima scrittura (fino a un blocco)
/// i liberi non scendono sotto [`RESERVE`].
pub const MARGIN: u64 = BLOCK;
/// Ogni scrittura è un multiplo di questo (multiplo di ogni dimensione di
/// settore: lo chiede `FILE_FLAG_NO_BUFFERING`).
pub const ALIGN: u64 = 1 << 20;
/// Dimensione massima di un file di riempimento.
pub const FILE_SIZE: u64 = 1 << 30;
/// Prefisso della cartella di lavoro nella radice del volume.
pub const DIR_PREFIX: &str = "Xiox_wipe_";
/// Prefisso di prima del cambio di nome: le cartelle rimaste con questo nome si riconoscono e si tolgono lo stesso.
pub const LEGACY_DIR_PREFIX: &str = "PulisciPC_wipe_";
/// Voci lette al più in una cartella di lavoro rimasta.
const MAX_ENTRIES: usize = 100_000;

/// Tipo di disco sotto il volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    /// Con penalità di ricerca (testine): la sovrascrittura serve.
    Hdd,
    /// Senza penalità di ricerca: TRIM, la sovrascrittura non serve.
    Ssd,
}

/// Un volume. I nomi passati sono sempre semplici (niente separatori) e
/// relativi alla radice.
pub trait Volume {
    /// Radice, `C:\`.
    fn root(&self) -> &Path;
    /// `true` solo per un disco fisso con file system NTFS.
    fn is_fixed_ntfs(&self) -> bool;
    /// HDD o SSD; `Err` se non si determina.
    fn kind(&self) -> Result<DiskKind, String>;
    /// Byte liberi utilizzabili dal processo (il minore fra quota e totale).
    fn free_bytes(&self) -> Result<u64, String>;
    /// Cartelle vere (non collegamenti) nella radice con il nome di una
    /// cartella di lavoro.
    fn leftover_dirs(&self) -> Result<Vec<String>, String>;
    /// Crea la cartella nascosta `name` nella radice; errore se esiste già.
    fn create_dir(&self, name: &str) -> Result<(), String>;
    /// Crea il file nuovo `file` in `dir` per gli zeri (`CREATE_NEW`, niente
    /// cache, cancellato da Windows alla chiusura).
    fn create_fill(&self, dir: &str, file: &str) -> Result<Box<dyn Fill + '_>, String>;
    /// Cancella i file di riempimento rimasti in `name` e poi la cartella;
    /// byte dei file cancellati. Non tocca altro.
    fn remove_dir(&self, name: &str) -> Result<u64, String>;
}

/// Un file di riempimento aperto. Chiuderlo (drop) lo cancella.
pub trait Fill {
    /// Aggiunge `len` byte di zeri (multiplo di [`ALIGN`]).
    fn write_zeros(&mut self, len: u64) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// Voce.

pub const DESCRIPTION: &str = "Sovrascrive con zeri lo spazio libero del solo disco di sistema (C:) perché i file già \
     cancellati non si possano recuperare. Copertura parziale: non si toccano gli altri dischi, né la MFT (i file \
     piccolissimi cancellati possono restare), né le copie shadow dei punti di ripristino, e circa 1 GB resta non \
     sovrascritto (la riserva). Solo su dischi rigidi (HDD) NTFS con almeno 2 GB liberi; sugli SSD si rifiuta, perché \
     Windows usa TRIM. Alcuni controller RAID o dischi virtuali si dichiarano HDD anche su SSD. Alla fine i file di \
     riempimento si cancellano.";

pub const WARNING: &str = "Sovrascrive lo spazio libero del solo disco di sistema (niente MFT, niente altri dischi) \
     perché i file cancellati non si possano recuperare. Può richiedere ore; durante la scrittura il disco resta quasi \
     pieno (si lascia almeno 1 GB libero) e gli altri programmi possono non riuscire a scrivere: chiudili prima. I file \
     cancellati dopo l'ultimo punto di ripristino possono restare nelle copie shadow, e con poco spazio libero Windows \
     può eliminare i punti di ripristino. Su SSD la voce si rifiuta.";

pub fn items() -> Vec<NativeItem> {
    vec![NativeItem {
        id: "wipe_free_space",
        group: GROUP,
        label: "Pulisci spazio libero",
        description: DESCRIPTION,
        warning: Some(WARNING),
        warning_extra: None,
        default_on: false,
        needs_admin: true,
        running: &[],
        needs_preview: false,
        analyze: real::analyze,
        clean: real::clean,
    }]
}

/// Collegamento fra la voce e il volume vero (in v1 solo quello di sistema).
mod real {
    use super::*;

    pub fn analyze(ctx: &NativeCtx) -> NativeFound {
        match win::RealVolume::system(ctx.guard) {
            Some(vol) => analyze_wipe(&vol),
            None => NativeFound {
                detail: vec!["Disco di sistema non determinato: non si pulisce".to_string()],
                ..Default::default()
            },
        }
    }

    pub fn clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        match win::RealVolume::system(ctx.guard) {
            Some(vol) => clean_wipe(&vol, found, ctx.cancel),
            None => NativeCleaned::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Nomi.

/// Nome della cartella di lavoro.
pub fn wipe_dir_name(pid: u32, nanos: u128) -> String {
    format!("{DIR_PREFIX}{pid}_{nanos}")
}

/// Il prefisso (nuovo o vecchio) con cui comincia il nome di una cartella di lavoro.
fn wipe_prefix(name: &str) -> Option<&'static str> {
    [DIR_PREFIX, LEGACY_DIR_PREFIX].into_iter().find(|p| name.starts_with(p))
}

/// `Xiox_wipe_<cifre>_<cifre>` (o il vecchio `PulisciPC_wipe_<cifre>_<cifre>`) e
/// nient'altro (maiuscole comprese).
pub fn is_wipe_dir_name(name: &str) -> bool {
    let Some(prefix) = wipe_prefix(name) else { return false };
    let rest = &name[prefix.len()..];
    let mut parts = rest.split('_');
    let digits = |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    digits(parts.next()) && digits(parts.next()) && parts.next().is_none()
}

/// Nome del file di riempimento numero `n` (da 1).
pub fn fill_name(n: usize) -> String {
    format!("fill_{n:05}.bin")
}

/// `fill_` + da 5 a 9 cifre + `.bin`.
pub fn is_fill_name(name: &str) -> bool {
    name.strip_prefix("fill_")
        .and_then(|r| r.strip_suffix(".bin"))
        .is_some_and(|d| (5..=9).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))
}

fn new_dir_name() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    wipe_dir_name(std::process::id(), nanos)
}

// ---------------------------------------------------------------------------
// Analisi.

/// Volume adatto: tipo e spazio libero letti adesso.
struct Suitable {
    free: u64,
}

fn root_text(vol: &dyn Volume) -> String {
    vol.root().to_string_lossy().into_owned()
}

/// Il volume si può pulire? `Err` con il motivo, scritto per il resoconto.
fn suitability(vol: &dyn Volume) -> Result<Suitable, String> {
    let root = root_text(vol);
    if !vol.is_fixed_ntfs() {
        return Err(format!("{root} non è un disco fisso NTFS: non si pulisce"));
    }
    let kind = vol.kind();
    let free = vol.free_bytes();
    let free_text = free.as_ref().map(|f| format!(", {} liberi", fmt_size(*f))).unwrap_or_default();
    match kind {
        Err(e) => {
            return Err(format!("{root}: tipo di disco non determinato ({e}){free_text}: per prudenza non si pulisce"));
        }
        Ok(DiskKind::Ssd) => return Err(format!("{root}: SSD{free_text}. Non serve su SSD: Windows usa TRIM")),
        Ok(DiskKind::Hdd) => {}
    }
    match free {
        Err(e) => Err(format!("{root}: spazio libero non leggibile ({e}): non si pulisce")),
        Ok(f) if f < MIN_FREE => Err(format!(
            "{root}: disco rigido (HDD) con {} liberi: sotto 2 GB non si pulisce",
            fmt_size(f)
        )),
        Ok(f) => Ok(Suitable { free: f }),
    }
}

/// Il pid nel nome di una cartella di lavoro.
fn wipe_dir_pid(name: &str) -> Option<u32> {
    if !is_wipe_dir_name(name) {
        return None;
    }
    name[wipe_prefix(name)?.len()..].split('_').next()?.parse().ok()
}

/// Esito di [`cleanup_leftovers`].
#[derive(Debug, Default)]
pub struct Leftovers {
    pub freed: u64,
    pub notes: Vec<String>,
    pub failed: u64,
}

/// Toglie le cartelle di lavoro rimaste da un'esecuzione interrotta (solo
/// quelle con il nome esatto, attraverso [`Volume::remove_dir`], che cancella
/// solo file `fill_*.bin` veri e non segue collegamenti). `skip_pid`: le
/// cartelle con questo pid restano (all'avvio: quelle del processo stesso,
/// che potrebbe averne appena creata una). Una cartella di un'altra istanza
/// in corso non si cancella: i suoi file sono aperti senza condivisione.
pub fn cleanup_leftovers(vol: &dyn Volume, skip_pid: Option<u32>) -> Leftovers {
    let root = root_text(vol);
    let mut out = Leftovers::default();
    let names = match vol.leftover_dirs() {
        Ok(n) => n,
        Err(e) => {
            out.failed = 1;
            out.notes.push(format!("Cartelle rimaste in {root} non leggibili: {e}"));
            return out;
        }
    };
    for name in names.iter().filter(|n| is_wipe_dir_name(n)) {
        if skip_pid.is_some() && wipe_dir_pid(name) == skip_pid {
            continue;
        }
        match vol.remove_dir(name) {
            Ok(bytes) => {
                out.freed += bytes;
                out.notes.push(format!(
                    "Cancellata la cartella rimasta da una pulizia interrotta: {root}{name} ({} liberati)",
                    fmt_size(bytes)
                ));
            }
            Err(e) => {
                out.failed += 1;
                out.notes.push(format!("Cartella rimasta {root}{name} non cancellata: {e}"));
            }
        }
    }
    out
}

/// All'avvio dell'app, in un thread a parte: toglie dal volume di sistema le
/// cartelle rimaste da una pulizia interrotta (spegnimento brusco), così il
/// disco non resta quasi pieno finché l'utente non rianalizza questa voce.
/// Mai quelle del processo stesso. L'esito va solo sullo standard error.
pub fn remove_leftovers_in_background() {
    std::thread::spawn(|| {
        let guard = Guard::new(&xiox_rules::Env::for_rules());
        let Some(vol) = win::RealVolume::system(&guard) else { return };
        // Nessuna cartella: niente da fare (il caso normale).
        if !vol.leftover_dirs().is_ok_and(|d| d.iter().any(|n| is_wipe_dir_name(n))) {
            return;
        }
        for n in cleanup_leftovers(&vol, Some(std::process::id())).notes {
            eprintln!("{n}");
        }
    });
}

/// Prefisso degli elementi «cartella rimasta» in `NativeFound::items`.
pub const LEFTOVER_ITEM: &str = "leftover:";

/// Guarda tipo di disco e spazio libero ed elenca le cartelle rimaste, **in
/// sola lettura**. Elemento `<radice>` solo se il volume è adatto; un
/// elemento `leftover:<nome>` per ogni cartella rimasta (la toglie la
/// pulizia, oppure il prossimo avvio). Non scrive e non cancella nulla.
pub fn analyze_wipe(vol: &dyn Volume) -> NativeFound {
    let root = root_text(vol);
    let mut leftovers = Vec::new();
    let mut notes = Vec::new();
    match vol.leftover_dirs() {
        Ok(names) => {
            for name in names.into_iter().filter(|n| is_wipe_dir_name(n)) {
                notes.push(format!(
                    "Trovata una cartella rimasta da una pulizia interrotta: {root}{name}. Verrà tolta (con Pulizia \
                     o al prossimo avvio di Xiox)"
                ));
                leftovers.push(format!("{LEFTOVER_ITEM}{name}"));
            }
        }
        Err(e) => notes.push(format!("Cartelle rimaste in {root} non leggibili: {e}")),
    }
    let mut found = match suitability(vol) {
        Ok(s) => NativeFound {
            detail: vec![format!(
                "{}: disco rigido (HDD) NTFS con {} liberi; se ne sovrascrivono circa {} (resta almeno 1 GB libero; \
                 niente MFT né altri dischi)",
                root_text(vol),
                fmt_size(s.free),
                fmt_size(s.free.saturating_sub(RESERVE + MARGIN))
            )],
            count: 1,
            items: vec![root_text(vol)],
            ..Default::default()
        },
        Err(why) => NativeFound { detail: vec![why], ..Default::default() },
    };
    found.detail.extend(notes);
    found.count += leftovers.len() as u64;
    found.items.extend(leftovers);
    found
}

// ---------------------------------------------------------------------------
// Pulizia.

/// Cancella la cartella di lavoro quando esce di scena, anche per un
/// panico; [`DirGuard::finish`] la cancella subito e ne dà l'esito.
struct DirGuard<'a> {
    vol: &'a dyn Volume,
    name: String,
    done: bool,
}

impl DirGuard<'_> {
    fn finish(mut self) -> Result<u64, String> {
        self.done = true;
        self.vol.remove_dir(&self.name)
    }
}

impl Drop for DirGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            let _ = self.vol.remove_dir(&self.name);
        }
    }
}

/// Perché la scrittura si è fermata.
enum Stop {
    /// Raggiunta la riserva (o lo spazio misurato all'inizio).
    Full,
    Cancelled,
    Failed(String),
}

/// Prima toglie le cartelle rimaste da una pulizia interrotta (rilette
/// adesso), poi scrive gli zeri e cancella sempre la cartella. Scrive solo se
/// l'analisi dello stesso job ha trovato questo volume adatto, e ricontrolla
/// tipo e spazio prima di cominciare.
pub fn clean_wipe(vol: &dyn Volume, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let root = root_text(vol);
    if found.items.is_empty() || cancel.load(Ordering::Relaxed) {
        return out;
    }
    let left = cleanup_leftovers(vol, None);
    out.freed += left.freed;
    out.failed += left.failed;
    for n in left.notes {
        out.note(n);
    }
    if !found.items.contains(&root) || cancel.load(Ordering::Relaxed) {
        return out;
    }
    let start = match suitability(vol) {
        Ok(s) => s,
        Err(why) => {
            out.note(why);
            return out;
        }
    };
    // Mai più dello spazio misurato adesso, meno la riserva: se lo spazio
    // libero non calasse scrivendo, non si scriverebbe all'infinito.
    let budget = start.free.saturating_sub(RESERVE + MARGIN);
    let name = new_dir_name();
    if let Err(e) = vol.create_dir(&name) {
        out.failed += 1;
        out.note(format!("Cartella di lavoro {root}{name} non creata: {e}"));
        return out;
    }
    let dir = DirGuard { vol, name: name.clone(), done: false };
    // Dichiarati dopo `dir`: in caso di panico si chiudono prima che la
    // cartella si cancelli.
    let mut files: Vec<Box<dyn Fill + '_>> = Vec::new();
    let mut written: u64 = 0;
    let mut in_file = FILE_SIZE;
    let stop = loop {
        if cancel.load(Ordering::Relaxed) {
            break Stop::Cancelled;
        }
        let free = match vol.free_bytes() {
            Ok(f) => f,
            Err(e) => break Stop::Failed(format!("spazio libero non leggibile: {e}")),
        };
        // Ci si ferma con [`MARGIN`] sopra la riserva: un'altra app che scrive
        // durante questo blocco non fa scendere i liberi sotto [`RESERVE`].
        let room = free.saturating_sub(RESERVE + MARGIN).min(budget - written);
        let mut n = room.min(BLOCK) / ALIGN * ALIGN;
        if n == 0 {
            break Stop::Full;
        }
        if in_file >= FILE_SIZE {
            match vol.create_fill(&name, &fill_name(files.len() + 1)) {
                Ok(f) => files.push(f),
                Err(e) => break Stop::Failed(format!("file di riempimento non creato: {e}")),
            }
            in_file = 0;
        }
        n = n.min(FILE_SIZE - in_file);
        let Some(file) = files.last_mut() else { break Stop::Failed("nessun file aperto".into()) };
        if let Err(e) = file.write_zeros(n) {
            break Stop::Failed(e);
        }
        written += n;
        in_file += n;
    };
    drop(files);
    let removal = dir.finish();
    let amount = fmt_size(written);
    match stop {
        Stop::Full => {
            out.removed = 1;
            out.note(format!(
                "Sovrascritti {amount} di spazio libero su {root} (lasciato almeno 1 GB libero; MFT, altri dischi e \
                 copie shadow non toccati)"
            ));
            out.done.push(format!("{root}: {amount} di spazio libero sovrascritti"));
        }
        Stop::Cancelled => out.note(format!("Annullata dopo {amount} su {root}: file di riempimento cancellati")),
        Stop::Failed(e) => {
            out.failed += 1;
            out.note(format!("Scrittura interrotta dopo {amount} su {root}: {e}"));
        }
    }
    if let Err(e) = removal {
        out.failed += 1;
        out.note(format!(
            "La cartella {root}{name} non si è cancellata ({e}): si riprova al prossimo avvio di Xiox o alla \
             prossima pulizia di questa voce"
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// File system vero, senza Win32 (provato nei test su cartelle temporanee).

fn is_reparse(md: &fs::Metadata) -> bool {
    // FILE_ATTRIBUTE_REPARSE_POINT
    md.file_attributes() & 0x400 != 0
}

/// Le cartelle vere di `root` con il nome di una cartella di lavoro.
pub fn list_wipe_dirs_at(root: &Path) -> Result<Vec<String>, String> {
    let rd = fs::read_dir(root).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
        if !is_wipe_dir_name(&name) {
            continue;
        }
        if let Ok(md) = fs::symlink_metadata(e.path()) {
            if md.is_dir() && !is_reparse(&md) {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Cancella i file `fill_*.bin` (file veri, non collegamenti) della cartella
/// di lavoro `root\name` e poi la cartella, con `delete_checked_with` (stesso
/// handle per controllo e cancellazione). Qualunque altra cosa dentro resta,
/// e allora resta anche la cartella. Una cartella che è un collegamento o
/// una giunzione non si apre.
pub fn remove_wipe_dir_at(root: &Path, name: &str, guard: &Guard) -> Result<u64, String> {
    if !is_wipe_dir_name(name) {
        return Err(format!("nome non di una cartella di lavoro: {name}"));
    }
    let dir = root.join(name);
    let md = fs::symlink_metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !md.is_dir() || is_reparse(&md) {
        return Err(format!("{} non è una cartella vera: non si tocca", dir.display()));
    }
    let rd = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut freed = 0;
    let mut problems: Vec<String> = Vec::new();
    for e in rd.take(MAX_ENTRIES) {
        let e = match e {
            Ok(e) => e,
            Err(err) => {
                problems.push(err.to_string());
                continue;
            }
        };
        let p = e.path();
        let fill = e.file_name().to_str().is_some_and(is_fill_name);
        match fs::symlink_metadata(&p) {
            Ok(m) if fill && m.is_file() && !is_reparse(&m) => match xiox_rules::delete_checked_with(&p, guard) {
                Ok(b) => freed += b,
                Err(err) => problems.push(err),
            },
            _ => problems.push(format!("lasciato {}: non è un file di riempimento", p.display())),
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    xiox_rules::delete_checked_with(&dir, guard)?;
    Ok(freed)
}

// ---------------------------------------------------------------------------
// Implementazione vera (Win32). Mai usata dai test, salvo la lettura dal vivo
// `live_wipe_analysis_read_only` (ignorata di norma).

mod win {
    use std::ffi::c_void;
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};

    use xiox_rules::Guard;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileBasicInfo, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetFileInformationByHandleEx,
        GetFinalPathNameByHandleW, GetVolumeInformationW, SetFileAttributesW, WriteFile, CREATE_NEW, DELETE,
        FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NOT_CONTENT_INDEXED,
        FILE_ATTRIBUTE_SPARSE_FILE, FILE_BASIC_INFO, FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_NO_BUFFERING,
        FILE_FLAG_WRITE_THROUGH, FILE_NAME_NORMALIZED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        VOLUME_NAME_DOS,
    };
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use super::{is_fill_name, is_reparse, is_wipe_dir_name, list_wipe_dirs_at, remove_wipe_dir_at, DiskKind, Fill, Volume};

    /// Allineamento del buffer di zeri (`FILE_FLAG_NO_BUFFERING` vuole
    /// l'indirizzo allineato al settore fisico: 512 o 4096).
    const PAGE: usize = 4096;
    const ZERO_LEN: usize = 4 << 20;

    #[repr(C, align(4096))]
    struct Zeros([u8; ZERO_LEN]);

    /// Tutto zero: sta nella sezione `.bss`, non nel file eseguibile.
    static ZEROS: Zeros = Zeros([0; ZERO_LEN]);

    /// `DEVICE_SEEK_PENALTY_DESCRIPTOR` con il `BOOLEAN` come `u8` (mai un
    /// `bool` Rust riempito da Windows).
    #[repr(C)]
    #[derive(Default)]
    struct SeekPenalty {
        version: u32,
        size: u32,
        incurs_seek_penalty: u8,
    }

    /// Handle chiuso all'uscita.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: handle valido aperto da `CreateFileW`, chiuso una volta sola.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Percorso con `\\?\` davanti, terminato da zero.
    fn wide_long(p: &Path) -> Vec<u16> {
        let text = p.to_string_lossy();
        let full = if text.starts_with(r"\\?\") { text.into_owned() } else { format!(r"\\?\{text}") };
        std::ffi::OsStr::new(&full).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// Stessa voce: `\\?\` tolto, maiuscole ignorate.
    fn same_path(a: &str, b: &str) -> bool {
        let norm = |s: &str| s.strip_prefix(r"\\?\").unwrap_or(s).replace('/', "\\").to_lowercase();
        norm(a) == norm(b)
    }

    /// Il volume `X:\` di un percorso `X:\...`.
    fn drive_root(p: &Path) -> Option<String> {
        let s = p.to_string_lossy();
        let b = s.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\')
            .then(|| format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
    }

    pub struct RealVolume<'a> {
        root: PathBuf,
        guard: &'a Guard,
    }

    impl<'a> RealVolume<'a> {
        /// Il volume di sistema, dalla cartella di sistema di Windows
        /// (`GetSystemDirectoryW`, mai dalle variabili del processo).
        pub fn system(guard: &'a Guard) -> Option<RealVolume<'a>> {
            let mut buf = [0u16; 520];
            // SAFETY: `buf` ha `buf.len()` u16 scrivibili e la dimensione passata è quella.
            let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 || n >= buf.len() {
                return None;
            }
            let dir = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
            Some(RealVolume { root: PathBuf::from(drive_root(&dir)?), guard })
        }

        fn root_wide(&self) -> Vec<u16> {
            wide(&self.root.to_string_lossy())
        }

        /// Settore logico che divide l'allineamento del buffer.
        fn sector_ok(&self) -> Result<(), String> {
            let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
            let r = self.root_wide();
            // SAFETY: radice terminata da zero; i quattro puntatori sono u32 locali scrivibili.
            let ok = unsafe { GetDiskFreeSpaceW(r.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
            if ok == 0 {
                return Err(format!("dimensione del settore non leggibile: {}", last_error()));
            }
            if bps == 0 || !PAGE.is_multiple_of(bps as usize) {
                return Err(format!("settore di {bps} byte non gestito"));
            }
            Ok(())
        }
    }

    impl Volume for RealVolume<'_> {
        fn root(&self) -> &Path {
            &self.root
        }

        fn is_fixed_ntfs(&self) -> bool {
            let r = self.root_wide();
            // SAFETY: radice `X:\` terminata da zero.
            if unsafe { GetDriveTypeW(r.as_ptr()) } != DRIVE_FIXED {
                return false;
            }
            let mut fs_name = [0u16; 64];
            // SAFETY: radice terminata da zero; nome del volume e numeri non
            // richiesti (nulli); `fs_name` ha la lunghezza passata.
            let ok = unsafe {
                GetVolumeInformationW(
                    r.as_ptr(),
                    null_mut(),
                    0,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    fs_name.as_mut_ptr(),
                    fs_name.len() as u32,
                )
            };
            let len = fs_name.iter().position(|&c| c == 0).unwrap_or(fs_name.len());
            ok != 0 && String::from_utf16_lossy(&fs_name[..len]).eq_ignore_ascii_case("NTFS")
        }

        fn kind(&self) -> Result<DiskKind, String> {
            let letter = self.root.to_string_lossy().chars().next().ok_or("radice vuota")?;
            let dev = wide(&format!(r"\\.\{letter}:"));
            // SAFETY: nome del dispositivo terminato da zero; accesso 0 (solo
            // domande sul dispositivo, nessuna lettura né scrittura).
            let h = unsafe {
                CreateFileW(dev.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(format!("volume non apribile: {}", last_error()));
            }
            let h = Handle(h);
            let query = STORAGE_PROPERTY_QUERY {
                PropertyId: StorageDeviceSeekPenaltyProperty,
                QueryType: PropertyStandardQuery,
                AdditionalParameters: [0],
            };
            let mut out = SeekPenalty::default();
            let mut got = 0u32;
            // SAFETY: handle valido; ingresso e uscita sono strutture locali
            // delle dimensioni passate; nessun OVERLAPPED (handle sincrono).
            let ok = unsafe {
                DeviceIoControl(
                    h.0,
                    IOCTL_STORAGE_QUERY_PROPERTY,
                    (&query as *const STORAGE_PROPERTY_QUERY).cast::<c_void>(),
                    size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                    (&mut out as *mut SeekPenalty).cast::<c_void>(),
                    size_of::<SeekPenalty>() as u32,
                    &mut got,
                    null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("IncursSeekPenalty non disponibile: {}", last_error()));
            }
            let need = offset_of!(SeekPenalty, incurs_seek_penalty) + 1;
            if (got as usize) < need || (out.size as usize) < need || out.version == 0 {
                return Err("risposta incompleta del disco".into());
            }
            Ok(if out.incurs_seek_penalty != 0 { DiskKind::Hdd } else { DiskKind::Ssd })
        }

        fn free_bytes(&self) -> Result<u64, String> {
            let (mut avail, mut total, mut total_free) = (0u64, 0u64, 0u64);
            let r = self.root_wide();
            // SAFETY: radice terminata da zero; i tre puntatori sono u64 locali scrivibili.
            let ok = unsafe { GetDiskFreeSpaceExW(r.as_ptr(), &mut avail, &mut total, &mut total_free) };
            if ok == 0 {
                return Err(last_error());
            }
            Ok(avail.min(total_free))
        }

        fn leftover_dirs(&self) -> Result<Vec<String>, String> {
            list_wipe_dirs_at(&self.root)
        }

        fn create_dir(&self, name: &str) -> Result<(), String> {
            if !is_wipe_dir_name(name) {
                return Err(format!("nome non valido: {name}"));
            }
            let path = self.root.join(name);
            std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            let md = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !md.is_dir() || is_reparse(&md) {
                return Err("la cartella appena creata non è una cartella vera".into());
            }
            let w = wide_long(&path);
            // SAFETY: percorso terminato da zero; solo attributi. Se fallisce
            // la cartella resta visibile, nient'altro.
            unsafe { SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED) };
            Ok(())
        }

        fn create_fill(&self, dir: &str, file: &str) -> Result<Box<dyn Fill + '_>, String> {
            if !is_wipe_dir_name(dir) || !is_fill_name(file) {
                return Err(format!("nome non valido: {dir}\\{file}"));
            }
            self.sector_ok()?;
            let path = self.root.join(dir).join(file);
            let w = wide_long(&path);
            // SAFETY: percorso terminato da zero. `CREATE_NEW`: mai un file
            // esistente; nessuna condivisione; `DELETE_ON_CLOSE`: Windows lo
            // cancella quando l'handle si chiude, anche se il processo muore.
            let h = unsafe {
                CreateFileW(
                    w.as_ptr(),
                    GENERIC_WRITE | DELETE,
                    0,
                    null(),
                    CREATE_NEW,
                    FILE_ATTRIBUTE_HIDDEN
                        | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED
                        | FILE_FLAG_NO_BUFFERING
                        | FILE_FLAG_WRITE_THROUGH
                        | FILE_FLAG_DELETE_ON_CLOSE,
                    null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(last_error());
            }
            // Da qui il drop chiude l'handle e quindi cancella il file.
            let fill = RealFill(Handle(h));
            let mut buf = [0u16; 1024];
            // SAFETY: handle valido; `buf` ha la lunghezza passata.
            let n = unsafe {
                GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS)
            } as usize;
            if n == 0 || n >= buf.len() {
                return Err(format!("percorso vero non leggibile: {}", last_error()));
            }
            let actual = String::from_utf16_lossy(&buf[..n]);
            if !same_path(&path.to_string_lossy(), &actual) {
                return Err(format!("il file è finito altrove ({actual}): collegamento nel percorso"));
            }
            let mut info: FILE_BASIC_INFO = FILE_BASIC_INFO::default();
            // SAFETY: handle valido; `info` è un FILE_BASIC_INFO intero della dimensione passata.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    h,
                    FileBasicInfo,
                    (&mut info as *mut FILE_BASIC_INFO).cast::<c_void>(),
                    size_of::<FILE_BASIC_INFO>() as u32,
                )
            };
            if ok == 0 {
                return Err(format!("attributi non leggibili: {}", last_error()));
            }
            if info.FileAttributes & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0 {
                return Err("file compresso o sparso: gli zeri non coprirebbero lo spazio libero".into());
            }
            Ok(Box::new(fill))
        }

        fn remove_dir(&self, name: &str) -> Result<u64, String> {
            remove_wipe_dir_at(&self.root, name, self.guard)
        }
    }

    /// File di riempimento aperto; il drop chiude l'handle e Windows lo cancella.
    struct RealFill(Handle);

    impl Fill for RealFill {
        fn write_zeros(&mut self, len: u64) -> Result<(), String> {
            let mut left = len;
            while left > 0 {
                let n = left.min(ZERO_LEN as u64) as u32;
                let mut done = 0u32;
                // SAFETY: `ZEROS` ha `ZERO_LEN >= n` byte leggibili, allineati a
                // 4096; `n` è multiplo di 1 MiB (quindi del settore); handle
                // sincrono aperto da `CreateFileW`, nessun OVERLAPPED.
                let ok = unsafe { WriteFile((self.0).0, ZEROS.0.as_ptr(), n, &mut done, null_mut()) };
                if ok == 0 {
                    return Err(last_error());
                }
                if done != n {
                    return Err(format!("scritti {done} byte su {n}"));
                }
                left -= u64::from(n);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
