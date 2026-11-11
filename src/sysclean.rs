//! Azioni di sistema che non sono «cancella file o chiavi» (tabella di
//! copertura, `docs/ccleaner-coverage.md` §2.6 N1-N4): Appunti, cache DNS,
//! registri eventi, collegamenti non validi del menu Start e del Desktop.
//!
//! Ogni accesso al sistema passa da un trait ([`Clipboard`], [`Dns`],
//! [`EventLog`], [`Shortcuts`]): le funzioni `analyze_*`/`clean_*` contengono
//! tutta la logica e i test le provano con implementazioni finte; le
//! implementazioni vere (`win::Real*`) le usa solo [`items`].

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pc_rules::{Env, Guard};

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};

/// Gruppo del pannello delle voci di questo modulo.
pub const GROUP: &str = "Windows";
/// Tentativi di apertura degli Appunti (Review Focus 4).
pub const CLIPBOARD_TRIES: u32 = 3;
/// Attesa fra un tentativo e l'altro.
pub const CLIPBOARD_RETRY: Duration = Duration::from_millis(100);
/// Elemento unico delle voci senza elenco.
pub const CLIPBOARD_ITEM: &str = "clipboard";
pub const DNS_ITEM: &str = "dns_cache";
/// Righe di dettaglio tenute nella tabella (al passaggio del mouse).
const MAX_DETAIL: usize = 50;
/// Voci di cartella lette al più per ogni cartella dei collegamenti.
const MAX_ENTRIES: usize = 20_000;
/// Profondità massima nel menu Start.
const MAX_DEPTH: usize = 8;

/// Gli Appunti della sessione.
pub trait Clipboard {
    /// Numero di formati presenti (0 = vuoti).
    fn count(&self) -> u32;
    /// Un tentativo di svuotarli; `Err` se un'altra app li tiene aperti.
    fn empty(&self) -> Result<(), String>;
}

/// Un modo di svuotare la cache DNS.
pub trait Dns {
    fn flush(&self) -> Result<(), String>;
}

/// I registri del Visualizzatore eventi.
pub trait EventLog {
    /// Canali (nome, numero di eventi).
    fn channels(&self) -> Vec<(String, u64)>;
    fn clear(&self, channel: &str) -> Result<(), String>;
}

/// I file `.lnk` e le loro destinazioni.
pub trait Shortcuts {
    /// I `.lnk` di `dir` (con le sottocartelle se `recursive`), senza seguire
    /// collegamenti né giunzioni.
    fn list(&self, dir: &Path, recursive: bool) -> Vec<PathBuf>;
    /// Destinazioni scritte nel collegamento così come le dà la shell
    /// (`GetPath` normale e con `SLGP_RAWPATH`, che può contenere `%VAR%`).
    /// Vuoto = non leggibile o non un file (URL, `shell:`, app pacchettizzate).
    fn targets(&self, lnk: &Path) -> Vec<String>;
    /// `true` solo per un disco fisso presente (`GetDriveTypeW == DRIVE_FIXED`).
    fn volume_is_fixed(&self, p: &Path) -> bool;
}

// ---------------------------------------------------------------------------
// Voci.

/// Le quattro voci, con default e avvisi della §2.6.
pub fn items() -> Vec<NativeItem> {
    vec![
        NativeItem {
            id: "clipboard",
            group: GROUP,
            label: "Appunti",
            description: "Svuota gli Appunti di Windows, cioè quello che hai copiato. La cronologia di Win+V non si tocca.",
            warning: None,
            default_on: true,
            needs_admin: false,
            running: &[],
            analyze: real::clipboard_analyze,
            clean: real::clipboard_clean,
        },
        NativeItem {
            id: "dns_cache",
            group: GROUP,
            label: "Cache DNS",
            description: "Svuota la cache dei nomi di dominio di Windows. Si riempie di nuovo da sola alle prossime visite.",
            warning: None,
            default_on: false,
            needs_admin: false,
            running: &[],
            analyze: real::dns_analyze,
            clean: real::dns_clean,
        },
        NativeItem {
            id: "event_logs",
            group: GROUP,
            label: "Registri eventi di Windows",
            description: "Svuota i registri del Visualizzatore eventi che contengono eventi (Applicazione, Sistema, Sicurezza e gli altri canali amministrativi e operativi).",
            warning: Some(
                "si perde la storia degli eventi che serve per la diagnosi dei problemi, e non se ne salva una copia; \
                 il registro Sicurezza, svuotato, scrive l'evento 1102.",
            ),
            default_on: false,
            needs_admin: true,
            running: &[],
            analyze: real::event_logs_analyze,
            clean: real::event_logs_clean,
        },
        NativeItem {
            id: "broken_shortcuts",
            group: GROUP,
            label: "Collegamenti non validi",
            description: "Elimina i collegamenti del menu Start e del Desktop (dell'utente e pubblici) la cui destinazione non esiste più su un disco fisso. Le destinazioni su dischi di rete o rimovibili non si considerano mai.",
            warning: Some(
                "un programma spostato o un disco che ha cambiato lettera fanno sembrare non valido un collegamento \
                 che serve ancora: controlla l'elenco dopo «Analizza».",
            ),
            default_on: false,
            needs_admin: true,
            running: &[],
            analyze: real::shortcuts_analyze,
            clean: real::shortcuts_clean,
        },
    ]
}

/// Collegamento fra le voci e le implementazioni vere.
mod real {
    use super::*;

    pub fn clipboard_analyze(_: &NativeCtx) -> NativeFound {
        analyze_clipboard(&win::RealClipboard)
    }
    pub fn clipboard_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_clipboard(&win::RealClipboard, found, ctx.cancel, &std::thread::sleep)
    }
    pub fn dns_analyze(_: &NativeCtx) -> NativeFound {
        analyze_dns()
    }
    pub fn dns_clean(_: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        let ipconfig = win::Ipconfig { system32: win::system_dir() };
        clean_dns(found, &[&win::DnsApi, &ipconfig])
    }
    pub fn event_logs_analyze(_: &NativeCtx) -> NativeFound {
        analyze_event_logs(&win::RealEventLog)
    }
    pub fn event_logs_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_event_logs(&win::RealEventLog, found, ctx.cancel)
    }
    pub fn shortcuts_analyze(ctx: &NativeCtx) -> NativeFound {
        analyze_shortcuts(&win::RealShortcuts, ctx.env, ctx.guard, ctx.cancel)
    }
    pub fn shortcuts_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_shortcuts(&win::RealShortcuts, ctx.env, ctx.guard, found, ctx.cancel)
    }
}

// ---------------------------------------------------------------------------
// Appunti (N1).

/// «Appunti con contenuto» se c'è almeno un formato; niente se sono vuoti.
pub fn analyze_clipboard(cb: &dyn Clipboard) -> NativeFound {
    if cb.count() == 0 {
        return NativeFound::default();
    }
    NativeFound {
        detail: vec!["Appunti con contenuto".to_string()],
        count: 1,
        items: vec![CLIPBOARD_ITEM.to_string()],
        ..Default::default()
    }
}

/// Svuota gli Appunti se l'analisi li ha trovati pieni: al più
/// [`CLIPBOARD_TRIES`] tentativi a [`CLIPBOARD_RETRY`] l'uno dall'altro, poi
/// «Appunti occupati» come errore della voce. `sleep` è iniettato (il job lo
/// chiama nel suo thread, mai in quello della UI).
pub fn clean_clipboard(cb: &dyn Clipboard, found: &NativeFound, cancel: &AtomicBool, sleep: &dyn Fn(Duration)) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    if !found.items.iter().any(|i| i == CLIPBOARD_ITEM) || cancel.load(Ordering::Relaxed) {
        return out;
    }
    let mut last = String::new();
    for attempt in 1..=CLIPBOARD_TRIES {
        match cb.empty() {
            Ok(()) => {
                out.removed = 1;
                return out;
            }
            Err(e) => last = e,
        }
        if attempt < CLIPBOARD_TRIES {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            sleep(CLIPBOARD_RETRY);
        }
    }
    out.failed = 1;
    out.note(format!("Appunti occupati: {last}"));
    out
}

// ---------------------------------------------------------------------------
// Cache DNS (N2).

/// Nessuna anteprima di dimensione: una sola operazione.
pub fn analyze_dns() -> NativeFound {
    NativeFound {
        detail: vec!["1 operazione".to_string()],
        count: 1,
        items: vec![DNS_ITEM.to_string()],
        ..Default::default()
    }
}

/// Prova i metodi in ordine (`DnsFlushResolverCache`, poi `ipconfig /flushdns`)
/// e si ferma al primo che riesce.
pub fn clean_dns(found: &NativeFound, methods: &[&dyn Dns]) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    if !found.items.iter().any(|i| i == DNS_ITEM) {
        return out;
    }
    let mut errors: Vec<String> = Vec::new();
    for m in methods {
        match m.flush() {
            Ok(()) => {
                out.removed = 1;
                return out;
            }
            Err(e) => errors.push(e),
        }
    }
    out.failed = 1;
    out.note(format!("Cache DNS non svuotata: {}", errors.join("; ")));
    out
}

// ---------------------------------------------------------------------------
// Registri eventi (N3).

/// Canali con almeno un evento. Guarda e basta: non svuota niente.
pub fn analyze_event_logs(ev: &dyn EventLog) -> NativeFound {
    let channels: Vec<(String, u64)> = ev.channels().into_iter().filter(|(_, n)| *n > 0).collect();
    if channels.is_empty() {
        return NativeFound::default();
    }
    let events: u64 = channels.iter().map(|(_, n)| n).sum();
    let mut detail = vec![format!(
        "{}, {}",
        plural(channels.len() as u64, "registro", "registri"),
        plural(events, "evento", "eventi")
    )];
    detail.extend(channels.iter().take(MAX_DETAIL).map(|(c, n)| format!("{c}: {}", plural(*n, "evento", "eventi"))));
    NativeFound {
        detail,
        count: channels.len() as u64,
        items: channels.into_iter().map(|(c, _)| c).collect(),
        ..Default::default()
    }
}

/// Svuota i canali trovati dall'analisi; un canale che non si svuota non
/// ferma gli altri.
pub fn clean_event_logs(ev: &dyn EventLog, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    for channel in &found.items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match ev.clear(channel) {
            Ok(()) => out.removed += 1,
            Err(e) => {
                out.failed += 1;
                out.note(format!("{channel}: {e}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Collegamenti non validi (N4).

/// Cartelle dei collegamenti (`true` = con le sottocartelle): menu Start
/// dell'utente e comune, Desktop dell'utente e pubblico (solo il primo
/// livello). Solo percorsi assoluti con lettera di unità dell'ambiente fidato.
pub fn shortcut_dirs(env: &Env) -> Vec<(PathBuf, bool)> {
    let start_menu = |var: &str| env.get(var).map(|b| Path::new(b).join("Microsoft").join("Windows").join("Start Menu"));
    let desktop = env
        .get("desktop")
        .map(PathBuf::from)
        .or_else(|| env.get("userprofile").map(|p| Path::new(p).join("Desktop")));
    let public = env.get("public").map(|p| Path::new(p).join("Desktop"));
    [(start_menu("appdata"), true), (start_menu("programdata"), true), (desktop, false), (public, false)]
        .into_iter()
        .filter_map(|(d, rec)| d.map(|d| (d, rec)))
        .filter(|(d, _)| is_drive_path(d))
        .collect()
}

/// Destinazione mancante di un collegamento da proporre, o `None`.
///
/// Si propone solo se **ogni** destinazione letta (con le variabili espanse
/// dall'ambiente fidato, più le gemelle WOW64: `Program Files` ↔
/// `Program Files (x86)`, `System32` ↔ `SysWOW64`) è un percorso assoluto
/// con lettera di unità, sta su un disco fisso presente e manca con
/// `ErrorKind::NotFound` (mai `Path::exists`, che dice «no» anche per un
/// accesso negato). UNC, dischi rimovibili o di rete, URL, `shell:`,
/// variabili sconosciute e collegamenti illeggibili non si propongono mai.
pub fn broken_target(sc: &dyn Shortcuts, env: &Env, lnk: &Path) -> Option<PathBuf> {
    let raw = sc.targets(lnk);
    if raw.is_empty() {
        return None;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for r in &raw {
        let p = PathBuf::from(expand_vars(r.trim(), env)?);
        if !is_drive_path(&p) {
            return None;
        }
        for q in std::iter::once(p.clone()).chain(wow64_twins(&p, env)) {
            if !candidates.iter().any(|c| same_path(c, &q)) {
                candidates.push(q);
            }
        }
    }
    if candidates.iter().all(|c| sc.volume_is_fixed(c) && missing(c)) {
        candidates.into_iter().next()
    } else {
        None
    }
}

/// I collegamenti non validi delle [`shortcut_dirs`]; quelli che il `Guard`
/// rifiuta si contano fra i saltati.
pub fn analyze_shortcuts(sc: &dyn Shortcuts, env: &Env, guard: &Guard, cancel: &AtomicBool) -> NativeFound {
    let mut found = NativeFound::default();
    let mut lines: Vec<String> = Vec::new();
    'dirs: for (dir, recursive) in shortcut_dirs(env) {
        for lnk in sc.list(&dir, recursive) {
            if cancel.load(Ordering::Relaxed) {
                break 'dirs;
            }
            if guard.protected(&lnk).is_some() {
                found.skipped += 1;
                continue;
            }
            let Some(target) = broken_target(sc, env, &lnk) else { continue };
            found.count += 1;
            found.bytes += std::fs::symlink_metadata(&lnk).map(|m| m.len()).unwrap_or(0);
            if lines.len() < MAX_DETAIL {
                lines.push(format!("{} → {}", lnk.display(), target.display()));
            }
            found.items.push(lnk.to_string_lossy().into_owned());
        }
    }
    if found.count > 0 {
        found.detail.push(plural(found.count, "collegamento non valido", "collegamenti non validi"));
        found.detail.extend(lines);
    }
    found
}

/// Cancella i `.lnk` trovati dall'analisi, ciascuno solo se: sta in una delle
/// [`shortcut_dirs`] (il Desktop solo al primo livello), finisce in `.lnk`, è
/// ancora non valido. La cancellazione passa sempre da
/// `pc_rules::delete_checked_with` con il `Guard` della pulizia.
pub fn clean_shortcuts(sc: &dyn Shortcuts, env: &Env, guard: &Guard, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let dirs = shortcut_dirs(env);
    for item in &found.items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let lnk = PathBuf::from(item);
        let allowed = is_lnk(&lnk)
            && dirs.iter().any(|(d, recursive)| {
                if *recursive {
                    strictly_under(&lnk, d)
                } else {
                    lnk.parent().is_some_and(|p| same_path(p, d))
                }
            });
        if !allowed {
            out.failed += 1;
            out.note(format!("{item}: fuori dalle cartelle dei collegamenti, lasciato"));
            continue;
        }
        if missing(&lnk) {
            out.note(format!("{item}: già assente"));
            continue;
        }
        if broken_target(sc, env, &lnk).is_none() {
            out.note(format!("{item}: la destinazione c'è di nuovo, lasciato"));
            continue;
        }
        match pc_rules::delete_checked_with(&lnk, guard) {
            Ok(bytes) => {
                out.removed += 1;
                out.freed += bytes;
            }
            Err(e) => {
                out.failed += 1;
                out.note(format!("{item}: {e}"));
            }
        }
    }
    out
}

/// I `.lnk` di `dir` senza seguire collegamenti o giunzioni (né la cartella
/// stessa se lo è), al più [`MAX_ENTRIES`] voci e [`MAX_DEPTH`] livelli.
pub fn list_lnk(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::fs::symlink_metadata(dir) {
        Ok(md) if md.is_dir() && !is_reparse(&md) => {}
        _ => return out,
    }
    let mut seen = 0usize;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                out.sort();
                return out;
            }
            let path = e.path();
            let Ok(md) = std::fs::symlink_metadata(&path) else { continue };
            if is_reparse(&md) {
                continue;
            }
            if md.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if is_lnk(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Percorsi.

fn drive_letter(p: &Path) -> Option<u8> {
    match p.components().next()? {
        Component::Prefix(pre) => match pre.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

/// Assoluto e con lettera di unità (`C:\…`, `\\?\C:\…`).
fn is_drive_path(p: &Path) -> bool {
    drive_letter(p).is_some() && p.is_absolute()
}

/// Radice del disco (`C:\`) per `GetDriveTypeW`.
fn drive_root(p: &Path) -> Option<String> {
    drive_letter(p).map(|d| format!("{}:\\", (d as char).to_ascii_uppercase()))
}

fn comp_text(c: &Component) -> String {
    c.as_os_str().to_string_lossy().to_lowercase()
}

/// Componenti confrontabili (minuscole, senza `\\?\` e senza `.`).
fn comps(p: &Path) -> Vec<String> {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    Path::new(s)
        .components()
        .filter(|c| !matches!(c, Component::CurDir | Component::RootDir))
        .map(|c| comp_text(&c))
        .collect()
}

/// `p` sta dentro `dir` (non è `dir`).
fn strictly_under(p: &Path, dir: &Path) -> bool {
    let (p, d) = (comps(p), comps(dir));
    d.len() >= 2 && p.len() > d.len() && p[..d.len()] == d[..]
}

fn same_path(a: &Path, b: &Path) -> bool {
    comps(a) == comps(b)
}

/// `p` con la cartella `from` sostituita da `to` (confronto senza maiuscole).
fn rebase(p: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let pc: Vec<Component> = p.components().collect();
    let fc: Vec<Component> = from.components().collect();
    if fc.len() < 3 || pc.len() <= fc.len() || !pc.iter().zip(&fc).all(|(a, b)| comp_text(a) == comp_text(b)) {
        return None;
    }
    let mut out = to.to_path_buf();
    for c in &pc[fc.len()..] {
        out.push(c.as_os_str());
    }
    Some(out)
}

/// Le gemelle WOW64 di una destinazione: un collegamento scritto da un
/// installatore a 32 bit con `%ProgramFiles%` o `System32` può indicare la
/// cartella `(x86)` o `SysWOW64`.
fn wow64_twins(p: &Path, env: &Env) -> Vec<PathBuf> {
    let dir = |var: &str| env.get(var).map(PathBuf::from);
    let windir = dir("windir").or_else(|| dir("systemroot"));
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (a, b) in [
        ("programfiles", "programfiles(x86)"),
        ("programw6432", "programfiles(x86)"),
        ("commonprogramfiles", "commonprogramfiles(x86)"),
        ("commonprogramw6432", "commonprogramfiles(x86)"),
    ] {
        if let (Some(a), Some(b)) = (dir(a), dir(b)) {
            pairs.push((a.clone(), b.clone()));
            pairs.push((b, a));
        }
    }
    if let Some(w) = windir {
        pairs.push((w.join("System32"), w.join("SysWOW64")));
        pairs.push((w.join("SysWOW64"), w.join("System32")));
        pairs.push((w.join("Sysnative"), w.join("System32")));
    }
    pairs.iter().filter_map(|(from, to)| rebase(p, from, to)).collect()
}

/// `%Var%` sostituite con l'ambiente; `None` se una variabile non c'è o un
/// `%` resta aperto.
fn expand_vars(s: &str, env: &Env) -> Option<String> {
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
        out.push_str(env.get(name)?);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn is_lnk(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
}

/// Manca davvero: `NotFound` (file o percorso), non un altro errore.
fn missing(p: &Path) -> bool {
    matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == ErrorKind::NotFound)
}

/// Punto di reparse (collegamento simbolico, giunzione, segnaposto cloud).
fn is_reparse(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    md.file_type().is_symlink() || md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

// ---------------------------------------------------------------------------
// Implementazioni vere (Win32). Mai chiamate dai test.

mod win {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};

    use windows_sys::core::{BOOL, GUID, HRESULT, PCWSTR};
    use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError, ERROR_INSUFFICIENT_BUFFER, RPC_E_CHANGED_MODE, S_OK};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        COINIT_DISABLE_OLE1DDE, STGM_READ,
    };
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, CountClipboardFormats, EmptyClipboard, OpenClipboard};
    use windows_sys::Win32::System::EventLog::{
        EvtChannelConfigType, EvtClearLog, EvtClose, EvtGetChannelConfigProperty, EvtGetLogInfo, EvtLogNumberOfLogRecords,
        EvtNextChannelPath, EvtOpenChannelConfig, EvtOpenChannelEnum, EvtOpenChannelPath, EvtOpenLog, EvtVarTypeUInt32,
        EvtVarTypeUInt64, EVT_HANDLE, EVT_VARIANT,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::UI::Shell::{ShellLink, SLGP_RAWPATH};

    use super::{Clipboard, Dns, EventLog, Shortcuts};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_path(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// `%SystemRoot%\System32` da `GetSystemDirectoryW` (mai dal `PATH`).
    pub fn system_dir() -> Option<PathBuf> {
        let mut buf = [0u16; 520];
        // SAFETY: `buf` ha `buf.len()` u16 scrivibili e la dimensione passata è quella.
        let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }

    // ----- Appunti -----

    pub struct RealClipboard;

    impl Clipboard for RealClipboard {
        fn count(&self) -> u32 {
            // SAFETY: nessun argomento; non serve aprire gli Appunti.
            let n = unsafe { CountClipboardFormats() };
            n.max(0) as u32
        }

        fn empty(&self) -> Result<(), String> {
            // SAFETY: `OpenClipboard(NULL)` apre gli Appunti per questo thread
            // senza finestra proprietaria; se riesce, `CloseClipboard` si
            // chiama sotto su ogni percorso, anche se `EmptyClipboard` fallisce.
            if unsafe { OpenClipboard(null_mut()) } == 0 {
                return Err(last_error());
            }
            // SAFETY: Appunti aperti da questo thread (riga sopra).
            let emptied = unsafe { EmptyClipboard() } != 0;
            let err = (!emptied).then(last_error);
            // SAFETY: chiude l'apertura riuscita sopra.
            unsafe { CloseClipboard() };
            err.map_or(Ok(()), Err)
        }
    }

    // ----- DNS -----

    /// `DnsFlushResolverCache` di `dnsapi.dll`: export non documentato,
    /// caricato con `GetProcAddress` (se manca si passa a `ipconfig`).
    pub struct DnsApi;

    impl Dns for DnsApi {
        fn flush(&self) -> Result<(), String> {
            let name = wide("dnsapi.dll");
            // SAFETY: nome terminato da zero; `LOAD_LIBRARY_SEARCH_SYSTEM32`
            // cerca solo in System32; nessun file (`hfile` nullo). Il modulo
            // si libera con `FreeLibrary` su ogni percorso sotto.
            let module = unsafe { LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
            if module.is_null() {
                return Err(format!("dnsapi.dll non caricata: {}", last_error()));
            }
            // SAFETY: modulo valido (sopra), nome ANSI terminato da zero.
            let proc = unsafe { GetProcAddress(module, c"DnsFlushResolverCache".as_ptr().cast()) };
            let out = match proc {
                None => Err("DnsFlushResolverCache non presente in dnsapi.dll".to_string()),
                Some(f) => {
                    // SAFETY: l'export ha firma `BOOL WINAPI DnsFlushResolverCache(void)`
                    // (la stessa che usa ipconfig): nessun argomento, convenzione
                    // "system", ritorno BOOL (i32); il puntatore resta valido
                    // finché il modulo è caricato, cioè fino a `FreeLibrary` sotto.
                    let flush: unsafe extern "system" fn() -> BOOL = unsafe { std::mem::transmute(f) };
                    // SAFETY: vedi sopra.
                    if unsafe { flush() } != 0 { Ok(()) } else { Err(last_error()) }
                }
            };
            // SAFETY: libera il modulo caricato sopra; nessun puntatore suo resta in uso.
            unsafe { FreeLibrary(module) };
            out
        }
    }

    /// Ripiego: `ipconfig.exe /flushdns` per percorso assoluto, senza finestra.
    pub struct Ipconfig {
        pub system32: Option<PathBuf>,
    }

    impl Dns for Ipconfig {
        fn flush(&self) -> Result<(), String> {
            use std::os::windows::process::CommandExt;
            let exe = self
                .system32
                .as_ref()
                .map(|d| d.join("ipconfig.exe"))
                .filter(|p| p.is_file())
                .ok_or("ipconfig.exe non trovato")?;
            let st = std::process::Command::new(exe)
                .arg("/flushdns")
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            if st.success() { Ok(()) } else { Err(format!("ipconfig /flushdns: codice {}", st.code().unwrap_or(-1))) }
        }
    }

    // ----- Registri eventi -----

    /// Handle di wevtapi chiuso con `EvtClose` quando esce di scena.
    struct Evt(EVT_HANDLE);

    impl Drop for Evt {
        fn drop(&mut self) {
            if self.0 != 0 {
                // SAFETY: handle aperto da una funzione Evt* e non ancora chiuso.
                unsafe { EvtClose(self.0) };
            }
        }
    }

    pub struct RealEventLog;

    impl EventLog for RealEventLog {
        fn channels(&self) -> Vec<(String, u64)> {
            let mut out = Vec::new();
            // SAFETY: sessione locale (0), flag 0; l'handle lo chiude `Evt`.
            let en = Evt(unsafe { EvtOpenChannelEnum(0, 0) });
            if en.0 == 0 {
                return out;
            }
            let mut buf = vec![0u16; 512];
            loop {
                let mut used = 0u32;
                // SAFETY: `buf` ha `buf.len()` u16 scrivibili, la dimensione
                // passata è quella; `used` riceve i caratteri scritti o quelli
                // necessari (zero finale compreso).
                let ok = unsafe { EvtNextChannelPath(en.0, buf.len() as u32, buf.as_mut_ptr(), &mut used) };
                if ok == 0 {
                    // SAFETY: nessun argomento.
                    let err = unsafe { GetLastError() };
                    if err == ERROR_INSUFFICIENT_BUFFER && used as usize > buf.len() && used <= 32_768 {
                        buf.resize(used as usize, 0);
                        continue;
                    }
                    break; // ERROR_NO_MORE_ITEMS o un errore: fine.
                }
                let n = (used as usize).saturating_sub(1).min(buf.len());
                let name = String::from_utf16_lossy(&buf[..n]);
                if let Some(records) = channel_records(&name).filter(|&r| r > 0) {
                    out.push((name, records));
                }
            }
            out
        }

        fn clear(&self, channel: &str) -> Result<(), String> {
            let w = wide(channel);
            // SAFETY: sessione locale (0), nome terminato da zero, nessun file
            // di esportazione (puntatore nullo), flag 0.
            if unsafe { EvtClearLog(0, w.as_ptr(), null(), 0) } != 0 { Ok(()) } else { Err(last_error()) }
        }
    }

    /// Eventi di un canale amministrativo o operativo (gli analitici e di
    /// debug non si toccano); `None` se non si legge.
    fn channel_records(name: &str) -> Option<u64> {
        let w = wide(name);
        // SAFETY: sessione locale, nome terminato da zero, flag 0; chiuso da `Evt`.
        let cfg = Evt(unsafe { EvtOpenChannelConfig(0, w.as_ptr(), 0) });
        if cfg.0 == 0 {
            return None;
        }
        let mut v = EVT_VARIANT::default();
        let mut used = 0u32;
        // SAFETY: `v` è un EVT_VARIANT intero e la dimensione passata è la sua;
        // la proprietà `EvtChannelConfigType` è un UInt32 (nessun dato fuori da `v`).
        let ok = unsafe {
            EvtGetChannelConfigProperty(cfg.0, EvtChannelConfigType, 0, size_of::<EVT_VARIANT>() as u32, &mut v, &mut used)
        };
        // SAFETY: si legge il campo dell'unione solo se `Type` dice UInt32.
        if ok == 0 || v.Type != EvtVarTypeUInt32 as u32 || unsafe { v.Anonymous.UInt32Val } > 1 {
            return None; // 0 = Admin, 1 = Operational
        }
        // SAFETY: come sopra; chiuso da `Evt`.
        let log = Evt(unsafe { EvtOpenLog(0, w.as_ptr(), EvtOpenChannelPath) });
        if log.0 == 0 {
            return None;
        }
        let mut v = EVT_VARIANT::default();
        // SAFETY: `v` è un EVT_VARIANT intero e la dimensione passata è la sua;
        // `EvtLogNumberOfLogRecords` è un UInt64.
        let ok = unsafe { EvtGetLogInfo(log.0, EvtLogNumberOfLogRecords, size_of::<EVT_VARIANT>() as u32, &mut v, &mut used) };
        if ok == 0 {
            return None;
        }
        // SAFETY: si legge il campo dell'unione solo se `Type` dice UInt64.
        Some(if v.Type == EvtVarTypeUInt64 as u32 { unsafe { v.Anonymous.UInt64Val } } else { 0 })
    }

    // ----- Collegamenti (COM, IShellLinkW) -----

    const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

    /// Inizio delle vtable usate (windows-sys non ha le interfacce COM): solo
    /// i metodi fino all'ultimo chiamato, nell'ordine di ShObjIdl.h/ObjIdl.h.
    #[repr(C)]
    struct IUnknownVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct IShellLinkWVtbl {
        unknown: IUnknownVtbl,
        /// `GetPath(pszFile, cch, pfd, fFlags)`; `pfd` può essere nullo.
        get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> HRESULT,
    }

    #[repr(C)]
    struct IPersistFileVtbl {
        unknown: IUnknownVtbl,
        get_class_id: usize,
        is_dirty: usize,
        load: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
    }

    /// Puntatore a un'interfaccia COM, rilasciato con `Release` quando esce di scena.
    struct Com(*mut c_void);

    impl Com {
        /// La vtable dell'oggetto vista come `T` (prefisso della vera).
        ///
        /// # Safety
        /// `T` deve essere un prefisso `#[repr(C)]` della vtable dell'interfaccia di `self.0`.
        unsafe fn vtbl<T>(&self) -> &T {
            // SAFETY: un oggetto COM inizia con il puntatore alla sua vtable,
            // valida finché l'oggetto vive (lo tiene vivo `self`).
            unsafe { &**(self.0 as *const *const T) }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: interfaccia valida con un riferimento nostro, rilasciato una volta sola.
                unsafe { (self.vtbl::<IUnknownVtbl>().release)(self.0) };
            }
        }
    }

    /// `CoInitializeEx` accoppiato a `CoUninitialize` (in `Drop`) quando
    /// l'inizializzazione è nostra (`S_OK`/`S_FALSE`). Con `RPC_E_CHANGED_MODE`
    /// il thread ha già COM in un altro modello: si usa così e non si chiude.
    struct Apartment(bool);

    impl Apartment {
        fn enter() -> Option<Apartment> {
            // SAFETY: argomento riservato nullo, flag validi.
            let hr = unsafe { CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
            if hr >= 0 {
                Some(Apartment(true))
            } else if hr == RPC_E_CHANGED_MODE {
                Some(Apartment(false))
            } else {
                None
            }
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: chiude il `CoInitializeEx` riuscito di `enter`, sullo stesso thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub struct RealShortcuts;

    impl Shortcuts for RealShortcuts {
        fn list(&self, dir: &Path, recursive: bool) -> Vec<PathBuf> {
            super::list_lnk(dir, recursive)
        }

        fn targets(&self, lnk: &Path) -> Vec<String> {
            // `_apt` dichiarato prima: le interfacce (`Com`) si rilasciano
            // prima di `CoUninitialize`, su ogni percorso di uscita.
            let Some(_apt) = Apartment::enter() else { return Vec::new() };
            let mut raw: *mut c_void = null_mut();
            // SAFETY: CLSID e IID validi, nessun aggregato, `raw` riceve
            // un'interfaccia IShellLinkW con un riferimento (rilasciato da `Com`).
            let hr = unsafe { CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut raw) };
            if hr < 0 || raw.is_null() {
                return Vec::new();
            }
            let link = Com(raw);
            let mut raw: *mut c_void = null_mut();
            // SAFETY: `link` è un IShellLinkW vivo; QueryInterface scrive in
            // `raw` un IPersistFile con un riferimento (rilasciato da `Com`).
            let hr = unsafe { (link.vtbl::<IUnknownVtbl>().query_interface)(link.0, &IID_IPERSISTFILE, &mut raw) };
            if hr < 0 || raw.is_null() {
                return Vec::new();
            }
            let file = Com(raw);
            let path = wide_path(lnk);
            // SAFETY: `file` è un IPersistFile vivo, percorso terminato da zero,
            // `STGM_READ`: il collegamento si legge e basta.
            if unsafe { (file.vtbl::<IPersistFileVtbl>().load)(file.0, path.as_ptr(), STGM_READ) } < 0 {
                return Vec::new();
            }
            let mut out: Vec<String> = Vec::new();
            for flags in [0u32, SLGP_RAWPATH as u32] {
                let mut buf = vec![0u16; 32_768];
                // SAFETY: `buf` ha `buf.len()` u16 scrivibili e `cch` è quella
                // lunghezza; `pfd` nullo è ammesso; il risultato è terminato da zero.
                let hr = unsafe {
                    (link.vtbl::<IShellLinkWVtbl>().get_path)(link.0, buf.as_mut_ptr(), buf.len() as i32, null_mut(), flags)
                };
                if hr != S_OK {
                    continue; // S_FALSE: nessun percorso (URL, shell:, app pacchettizzata).
                }
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let s = String::from_utf16_lossy(&buf[..n]);
                if !s.trim().is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
            out
        }

        fn volume_is_fixed(&self, p: &Path) -> bool {
            let Some(root) = super::drive_root(p) else { return false };
            let w = wide(&root);
            // SAFETY: radice `X:\` terminata da zero; nessun accesso al disco oltre la domanda del tipo.
            unsafe { GetDriveTypeW(w.as_ptr()) == DRIVE_FIXED }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::fs;

    // ----- finti -----

    /// Appunti finti: falliscono `busy` volte, poi riescono (`u32::MAX` = sempre occupati).
    struct FakeClip {
        formats: u32,
        busy: u32,
        calls: Cell<u32>,
    }

    impl Clipboard for FakeClip {
        fn count(&self) -> u32 {
            self.formats
        }
        fn empty(&self) -> Result<(), String> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            if n < self.busy { Err("aperti da un'altra app".into()) } else { Ok(()) }
        }
    }

    struct FakeDns {
        ok: bool,
        calls: Cell<u32>,
    }

    impl Dns for FakeDns {
        fn flush(&self) -> Result<(), String> {
            self.calls.set(self.calls.get() + 1);
            if self.ok { Ok(()) } else { Err("export mancante".into()) }
        }
    }

    struct FakeEvents {
        channels: Vec<(String, u64)>,
        fail: Vec<String>,
        cleared: RefCell<Vec<String>>,
    }

    impl EventLog for FakeEvents {
        fn channels(&self) -> Vec<(String, u64)> {
            self.channels.clone()
        }
        fn clear(&self, channel: &str) -> Result<(), String> {
            self.cleared.borrow_mut().push(channel.to_string());
            if self.fail.iter().any(|f| f == channel) { Err("accesso negato".into()) } else { Ok(()) }
        }
    }

    /// Collegamenti finti: i `.lnk` sono file veri in una tempdir (così la
    /// cancellazione passa davvero da `delete_checked_with`), le destinazioni
    /// e il tipo di disco sono finti.
    #[derive(Default)]
    struct FakeLinks {
        targets: HashMap<PathBuf, Vec<String>>,
        /// Lettere di unità non fisse (rimovibili, di rete, assenti).
        not_fixed: Vec<char>,
    }

    impl Shortcuts for FakeLinks {
        fn list(&self, dir: &Path, recursive: bool) -> Vec<PathBuf> {
            list_lnk(dir, recursive)
        }
        fn targets(&self, lnk: &Path) -> Vec<String> {
            self.targets.get(lnk).cloned().unwrap_or_default()
        }
        fn volume_is_fixed(&self, p: &Path) -> bool {
            drive_letter(p).is_some_and(|d| !self.not_fixed.contains(&(d as char).to_ascii_uppercase()))
        }
    }

    fn env_in(root: &Path) -> Env {
        let p = root.join("profile");
        let s = |x: &Path| x.to_string_lossy().into_owned();
        Env(HashMap::from([
            ("userprofile".to_string(), s(&p)),
            ("appdata".to_string(), s(&p.join("AppData").join("Roaming"))),
            ("localappdata".to_string(), s(&p.join("AppData").join("Local"))),
            ("desktop".to_string(), s(&p.join("Desktop"))),
            ("programdata".to_string(), s(&root.join("ProgramData"))),
            ("public".to_string(), s(&root.join("Public"))),
            ("programfiles".to_string(), s(&root.join("PF"))),
            ("programfiles(x86)".to_string(), s(&root.join("PF86"))),
            ("username".to_string(), "u".to_string()),
        ]))
    }

    fn start_menu(env: &Env) -> PathBuf {
        Path::new(env.get("appdata").unwrap()).join(r"Microsoft\Windows\Start Menu\Programs")
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"lnk").unwrap();
    }

    fn found_items(items: &[&Path]) -> NativeFound {
        NativeFound {
            count: items.len() as u64,
            items: items.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
            ..Default::default()
        }
    }

    fn no(_: Duration) {}

    // ----- Appunti -----

    #[test]
    fn clipboard_retries_three_times_then_reports_busy() {
        let cancel = AtomicBool::new(false);
        let full = NativeFound { count: 1, items: vec![CLIPBOARD_ITEM.into()], ..Default::default() };

        let cb = FakeClip { formats: 2, busy: 2, calls: Cell::new(0) };
        let slept = Cell::new(0u32);
        let out = clean_clipboard(&cb, &full, &cancel, &|d| {
            assert_eq!(d, CLIPBOARD_RETRY);
            slept.set(slept.get() + 1);
        });
        assert_eq!((out.removed, out.failed), (1, 0));
        assert_eq!(cb.calls.get(), 3);
        assert_eq!(slept.get(), 2);

        let cb = FakeClip { formats: 2, busy: u32::MAX, calls: Cell::new(0) };
        let out = clean_clipboard(&cb, &full, &cancel, &no);
        assert_eq!((out.removed, out.failed), (0, 1));
        assert_eq!(cb.calls.get(), CLIPBOARD_TRIES);
        assert!(out.notes.iter().any(|n| n.contains("Appunti occupati")), "{:?}", out.notes);
    }

    #[test]
    fn clipboard_is_not_listed_when_empty() {
        let cb = FakeClip { formats: 0, busy: 0, calls: Cell::new(0) };
        let f = analyze_clipboard(&cb);
        assert_eq!(f.count, 0);
        assert!(f.items.is_empty());
        let out = clean_clipboard(&cb, &f, &AtomicBool::new(false), &no);
        assert_eq!(cb.calls.get(), 0, "nessun tentativo senza l'analisi");
        assert_eq!((out.removed, out.failed), (0, 0));

        let cb = FakeClip { formats: 3, busy: 0, calls: Cell::new(0) };
        let f = analyze_clipboard(&cb);
        assert_eq!(f.count, 1);
        assert_eq!(f.detail, vec!["Appunti con contenuto".to_string()]);
        assert_eq!(cb.calls.get(), 0, "l'analisi non svuota");
    }

    // ----- DNS -----

    #[test]
    fn dns_flush_falls_back_when_export_is_missing() {
        let first = FakeDns { ok: false, calls: Cell::new(0) };
        let second = FakeDns { ok: true, calls: Cell::new(0) };
        let out = clean_dns(&analyze_dns(), &[&first, &second]);
        assert_eq!((out.removed, out.failed), (1, 0));
        assert_eq!((first.calls.get(), second.calls.get()), (1, 1));

        let ok = FakeDns { ok: true, calls: Cell::new(0) };
        let unused = FakeDns { ok: true, calls: Cell::new(0) };
        clean_dns(&analyze_dns(), &[&ok, &unused]);
        assert_eq!(unused.calls.get(), 0, "il ripiego si usa solo se serve");

        let bad = FakeDns { ok: false, calls: Cell::new(0) };
        let out = clean_dns(&analyze_dns(), &[&bad, &bad]);
        assert_eq!((out.removed, out.failed), (0, 1));
        assert!(!out.notes.is_empty());

        let never = FakeDns { ok: true, calls: Cell::new(0) };
        let out = clean_dns(&NativeFound::default(), &[&never]);
        assert_eq!((never.calls.get(), out.removed), (0, 0), "niente senza l'analisi");
    }

    // ----- Registri eventi -----

    fn events(fail: &[&str]) -> FakeEvents {
        FakeEvents {
            channels: vec![("Application".into(), 10), ("Security".into(), 5), ("Empty".into(), 0), ("System".into(), 7)],
            fail: fail.iter().map(|s| s.to_string()).collect(),
            cleared: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn event_logs_continue_after_a_channel_fails() {
        let ev = events(&["Security"]);
        let found = analyze_event_logs(&ev);
        assert_eq!(found.count, 3);
        assert_eq!(found.items, vec!["Application", "Security", "System"]);
        let out = clean_event_logs(&ev, &found, &AtomicBool::new(false));
        assert_eq!((out.removed, out.failed), (2, 1));
        assert_eq!(*ev.cleared.borrow(), vec!["Application", "Security", "System"]);
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].contains("Security"), "{:?}", out.notes);
    }

    #[test]
    fn event_logs_are_never_cleared_by_analysis() {
        let ev = events(&[]);
        let found = analyze_event_logs(&ev);
        assert!(ev.cleared.borrow().is_empty());
        assert!(found.detail[0].contains("3 registri"), "{:?}", found.detail);
        assert!(found.detail[0].contains("22 eventi"), "{:?}", found.detail);
        // Si svuota solo ciò che l'analisi ha trovato (mai «Empty»).
        let partial = NativeFound { items: vec!["System".into()], ..found };
        clean_event_logs(&ev, &partial, &AtomicBool::new(false));
        assert_eq!(*ev.cleared.borrow(), vec!["System"]);
    }

    // ----- Collegamenti -----

    #[test]
    fn shortcut_to_missing_target_on_fixed_volume_is_listed() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let broken = start_menu(&env).join("Vecchio").join("App.lnk");
        let fine = start_menu(&env).join("Buono.lnk");
        let on_desktop = Path::new(env.get("desktop").unwrap()).join("Gioco.lnk");
        let exists = t.path().join("PF").join("Buono").join("buono.exe");
        touch(&broken);
        touch(&fine);
        touch(&on_desktop);
        touch(&exists);
        let gone = t.path().join("PF").join("Vecchio").join("app.exe");
        let mut sc = FakeLinks::default();
        sc.targets.insert(broken.clone(), vec![gone.to_string_lossy().into_owned()]);
        sc.targets.insert(fine.clone(), vec![exists.to_string_lossy().into_owned()]);
        // Variabile espansa con l'ambiente fidato.
        sc.targets.insert(on_desktop.clone(), vec![r"%ProgramFiles%\Gioco\gioco.exe".into()]);

        assert_eq!(broken_target(&sc, &env, &broken), Some(gone.clone()));
        assert_eq!(broken_target(&sc, &env, &fine), None);
        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        let mut items = found.items.clone();
        items.sort();
        let mut want = vec![broken.to_string_lossy().into_owned(), on_desktop.to_string_lossy().into_owned()];
        want.sort();
        assert_eq!(items, want);
        assert_eq!(found.count, 2);
        assert_eq!(found.bytes, 6);
        assert!(found.detail[0].contains("2 collegamenti"), "{:?}", found.detail);
        assert!(broken.exists() && on_desktop.exists(), "l'analisi non cancella");
    }

    #[test]
    fn shortcut_to_unc_network_or_removable_target_is_never_listed() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let dir = start_menu(&env);
        let mut sc = FakeLinks { not_fixed: vec!['Q'], ..Default::default() };
        let cases: Vec<(&str, Vec<String>)> = vec![
            ("unc", vec![r"\\server\share\app.exe".into()]),
            ("unc_lungo", vec![r"\\?\UNC\server\share\app.exe".into()]),
            ("rimovibile", vec![r"Q:\Programmi\app.exe".into()]),
            ("nessuna", vec![]),
            ("variabile_ignota", vec![r"%NonEsiste%\app.exe".into()]),
            ("percento_aperto", vec![r"%ProgramFiles\app.exe".into()]),
            ("shell", vec!["shell:AppsFolder\\Microsoft.App".into()]),
            ("url", vec!["https://example.com/".into()]),
            ("relativo", vec![r"app.exe".into()]),
            ("senza_radice", vec![r"C:app.exe".into()]),
            // Una delle destinazioni su un disco non fisso basta a escluderlo.
            ("mista", vec![t.path().join("manca.exe").to_string_lossy().into_owned(), r"Q:\manca.exe".into()]),
        ];
        for (name, targets) in &cases {
            let lnk = dir.join(format!("{name}.lnk"));
            touch(&lnk);
            sc.targets.insert(lnk.clone(), targets.clone());
            assert_eq!(broken_target(&sc, &env, &lnk), None, "{name}");
        }
        // Gemella WOW64: `%ProgramFiles%` letto a 64 bit, programma a 32 bit.
        let wow = dir.join("wow.lnk");
        touch(&wow);
        touch(&t.path().join("PF86").join("Vecchio").join("app.exe"));
        sc.targets.insert(wow.clone(), vec![t.path().join("PF").join("Vecchio").join("app.exe").to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &wow), None, "gemella in Program Files (x86)");
        // Una destinazione che esiste fra due lette basta a tenerlo.
        let two = dir.join("due.lnk");
        touch(&two);
        let here = t.path().join("qui.exe");
        touch(&here);
        sc.targets.insert(two.clone(), vec![t.path().join("manca.exe").to_string_lossy().into_owned(), here.to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &two), None);

        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        assert_eq!(found.count, 0, "{:?}", found.items);
    }

    #[test]
    fn clean_acts_only_on_found_items() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let guard = Guard::new(&env);
        let gone = t.path().join("manca.exe").to_string_lossy().into_owned();
        let desktop = PathBuf::from(env.get("desktop").unwrap());

        let broken = start_menu(&env).join("Rotto.lnk");
        let outside = t.path().join("Altrove").join("Rotto.lnk");
        let not_lnk = start_menu(&env).join("Rotto.txt");
        let deep_desktop = desktop.join("Cartella").join("Rotto.lnk");
        let healed = start_menu(&env).join("Tornato.lnk");
        let not_found = start_menu(&env).join("Mai analizzato.lnk");
        for p in [&broken, &outside, &not_lnk, &deep_desktop, &healed, &not_found] {
            touch(p);
        }
        let back = t.path().join("tornato.exe");
        touch(&back);
        let mut sc = FakeLinks::default();
        for p in [&broken, &outside, &not_lnk, &deep_desktop, &not_found] {
            sc.targets.insert(p.clone(), vec![gone.clone()]);
        }
        sc.targets.insert(healed.clone(), vec![back.to_string_lossy().into_owned()]);

        let found = found_items(&[&broken, &outside, &not_lnk, &deep_desktop, &healed]);
        let out = clean_shortcuts(&sc, &env, &guard, &found, &AtomicBool::new(false));
        assert!(!broken.exists(), "il collegamento trovato e ancora rotto si cancella");
        assert_eq!(out.removed, 1);
        assert_eq!(out.freed, 3);
        for p in [&outside, &not_lnk, &deep_desktop, &healed, &not_found] {
            assert!(p.exists(), "{} non si tocca", p.display());
        }
        assert_eq!(out.failed, 3, "{:?}", out.notes);
        assert!(out.notes.iter().any(|n| n.contains("Tornato.lnk")), "{:?}", out.notes);

        // Annullata prima di cominciare: niente.
        let again = start_menu(&env).join("Ancora.lnk");
        touch(&again);
        sc.targets.insert(again.clone(), vec![gone.clone()]);
        let out = clean_shortcuts(&sc, &env, &guard, &found_items(&[&again]), &AtomicBool::new(true));
        assert!(again.exists());
        assert_eq!(out.removed, 0);
    }

    #[test]
    fn listing_skips_junctions_and_desktop_subfolders() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("menu");
        touch(&root.join("a.lnk"));
        touch(&root.join("sub").join("b.LNK"));
        touch(&root.join("c.txt"));
        let elsewhere = t.path().join("altrove");
        touch(&elsewhere.join("x.lnk"));
        let st = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(root.join("giunzione"))
            .arg(&elsewhere)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "mklink /J");
        let names = |v: Vec<PathBuf>| -> Vec<String> {
            v.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(names(list_lnk(&root, true)), vec!["a.lnk".to_string(), r"sub\b.LNK".to_string()]);
        assert_eq!(names(list_lnk(&root, false)), vec!["a.lnk".to_string()]);
        assert!(list_lnk(&root.join("giunzione"), true).is_empty(), "una cartella giunzione non si apre");
        assert!(list_lnk(&t.path().join("non esiste"), true).is_empty());
    }

    #[test]
    fn shortcut_folders_pass_the_rules_guard() {
        let env = Env(HashMap::from([
            ("userprofile".to_string(), r"C:\Users\u".to_string()),
            ("appdata".to_string(), r"C:\Users\u\AppData\Roaming".to_string()),
            ("localappdata".to_string(), r"C:\Users\u\AppData\Local".to_string()),
            ("desktop".to_string(), r"C:\Users\u\Desktop".to_string()),
            ("programdata".to_string(), r"C:\ProgramData".to_string()),
            ("public".to_string(), r"C:\Users\Public".to_string()),
            ("windir".to_string(), r"C:\Windows".to_string()),
            ("programfiles".to_string(), r"C:\Program Files".to_string()),
            ("username".to_string(), "u".to_string()),
        ]));
        let dirs = shortcut_dirs(&env);
        let want: Vec<(PathBuf, bool)> = vec![
            (r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu".into(), true),
            (r"C:\ProgramData\Microsoft\Windows\Start Menu".into(), true),
            (r"C:\Users\u\Desktop".into(), false),
            (r"C:\Users\Public\Desktop".into(), false),
        ];
        assert_eq!(dirs, want);
        let guard = Guard::new(&env);
        for (d, rec) in &dirs {
            let lnk = if *rec { d.join("Programs").join("App").join("App.lnk") } else { d.join("App.lnk") };
            assert_eq!(guard.protected(&lnk), None, "{}", lnk.display());
        }
    }

    #[test]
    fn items_follow_the_coverage_table() {
        let items = items();
        let ids: Vec<&str> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec!["clipboard", "dns_cache", "event_logs", "broken_shortcuts"]);
        assert!(items.iter().all(|i| i.group == "Windows"));
        let by = |id: &str| items.iter().find(|i| i.id == id).unwrap();
        assert!(by("clipboard").default_on && by("clipboard").warning.is_none());
        assert!(!by("dns_cache").default_on && by("dns_cache").warning.is_none());
        let ev = by("event_logs");
        assert!(!ev.default_on && ev.needs_admin);
        let w = ev.warning.unwrap();
        assert!(w.contains("diagnosi") && w.contains("Sicurezza") && w.contains("1102"), "{w}");
        assert!(!by("broken_shortcuts").default_on && by("broken_shortcuts").warning.is_some());
        assert_eq!(by("clipboard").label, "Appunti");
        assert_eq!(by("dns_cache").label, "Cache DNS");
        for i in &items {
            let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap_or(""));
            assert!(!text.contains("BleachBit") && !text.contains("AGG -"), "{text}");
        }
    }
}
