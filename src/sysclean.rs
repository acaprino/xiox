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
    /// Canali (nome, numero di eventi, dimensione del file in byte).
    fn channels(&self) -> Vec<(String, u64, u64)>;
    fn clear(&self, channel: &str) -> Result<(), String>;
}

/// I file `.lnk` e le loro destinazioni.
pub trait Shortcuts {
    /// I `.lnk` di `dir` (con le sottocartelle se `recursive`), senza seguire
    /// collegamenti né giunzioni.
    fn list(&self, dir: &Path, recursive: bool) -> Vec<PathBuf>;
    /// `LinkFlags` dell'intestazione del collegamento e destinazioni così come
    /// le dà la shell (`GetPath` normale e con `SLGP_RAWPATH`, che può
    /// contenere `%VAR%`), in UTF-16 senza conversioni. `None` = illeggibile;
    /// nessun percorso = non un file (URL, `shell:`, app pacchettizzate).
    fn targets(&self, lnk: &Path) -> Option<LinkTargets>;
    /// `true` solo per un disco fisso presente (`GetDriveTypeW == DRIVE_FIXED`).
    fn volume_is_fixed(&self, p: &Path) -> bool;
    /// Menu Start comune e Desktop pubblico dalle cartelle note di Windows
    /// (`SHGetKnownFolderPath`), mai dalle variabili del processo.
    fn common_dirs(&self) -> CommonDirs;
}

/// Ciò che si legge di un `.lnk`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkTargets {
    /// `LinkFlags` dell'intestazione MS-SHLLINK (offset 0x14).
    pub flags: u32,
    pub paths: Vec<std::ffi::OsString>,
}

/// Cartelle comuni dei collegamenti (`None` = non risolta).
#[derive(Debug, Clone, Default)]
pub struct CommonDirs {
    pub start_menu: Option<PathBuf>,
    pub public_desktop: Option<PathBuf>,
}

/// Una cartella dei collegamenti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutDir {
    pub path: PathBuf,
    /// Con le sottocartelle (menu Start) o solo il primo livello (Desktop).
    pub recursive: bool,
    /// Comune a tutti gli utenti (menu Start comune, Desktop pubblico).
    pub common: bool,
}

/// `LinkFlags`: il collegamento ha una `LinkInfo` (percorso del file system).
pub const SLDF_HAS_LINK_INFO: u32 = 0x2;
/// `LinkFlags`: collegamento «pubblicizzato» di Windows Installer (Darwin).
pub const SLDF_HAS_DARWINID: u32 = 0x1000;
/// Destinazioni da questa lunghezza in su (UTF-16) possono essere troncate da
/// `IShellLinkW` (`MAX_PATH`): non si giudicano.
pub const MAX_TARGET_LEN: usize = 259;

/// `LinkFlags` dall'intestazione di un `.lnk` (MS-SHLLINK: `HeaderSize`
/// 0x4C, CLSID `00021401-0000-0000-C000-000000000046`, flag a 0x14);
/// `None` se l'intestazione non è quella.
pub fn link_flags(header: &[u8]) -> Option<u32> {
    const CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    let size = u32::from_le_bytes(header.get(0..4)?.try_into().ok()?);
    if size != 0x4C || header.len() < 0x4C || header[4..20] != CLSID {
        return None;
    }
    Some(u32::from_le_bytes(header[0x14..0x18].try_into().ok()?))
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
            warning_extra: None,
            default_on:true,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::clipboard_analyze,
            clean: real::clipboard_clean,
        },
        NativeItem {
            id: "dns_cache",
            group: GROUP,
            label: "Cache DNS",
            description: "Svuota la cache dei nomi di dominio di Windows. Si riempie di nuovo da sola alle prossime visite.",
            warning: None,
            warning_extra: None,
            default_on:false,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::dns_analyze,
            clean: real::dns_clean,
        },
        NativeItem {
            id: "event_logs",
            group: GROUP,
            label: "Registri eventi di Windows",
            description: "Svuota tre registri del Visualizzatore eventi: Applicazione, Sistema e Installazione \
                 (Application, System, Setup). Windows scrive poi nel registro Sistema un evento 104 per ogni \
                 registro svuotato, quindi Sistema non resta vuoto.",
            warning: Some(
                "si svuotano i registri Applicazione, Sistema e Installazione e si perde la storia degli eventi \
                 che serve per la diagnosi dei problemi, senza copia. Sicurezza, Windows Defender e gli altri \
                 registri restano come sono.",
            ),
            warning_extra: None,
            default_on:false,
            needs_admin: true,
            running: &[],
            needs_preview: false,
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
            warning_extra: None,
            default_on:false,
            needs_admin: true,
            running: &[],
            needs_preview: false,
            analyze: real::shortcuts_analyze,
            clean: real::shortcuts_clean,
        },
    ]
}

/// La cartella Documenti dell'utente dalla cartella nota di Windows
/// (`SHGetKnownFolderPath`), mai dalle variabili del processo. La usa
/// `browserdb` per la lista dei cookie da conservare.
pub fn documents_dir() -> Option<PathBuf> {
    win::documents()
}

/// La cartella di sistema (`GetSystemDirectoryW`), mai da `%SystemRoot%`. La
/// usa `browserdb` per il Blocco note che apre la lista dei cookie.
pub fn system32_dir() -> Option<PathBuf> {
    win::system_dir()
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

/// Tempo massimo di `ipconfig /flushdns`.
pub const IPCONFIG_TIMEOUT: Duration = Duration::from_secs(5);
/// Intervallo fra due controlli del processo.
const POLL_STEP: Duration = Duration::from_millis(50);

/// Aspetta la fine di un processo (`poll` dà `Some(codice)` quando è finito)
/// al più `timeout`. Oltre: `Err("Cache DNS: tempo scaduto")`, e chi chiama
/// termina il processo. Codice diverso da 0: errore con il codice.
pub fn wait_bounded(
    poll: &mut dyn FnMut() -> Result<Option<i32>, String>,
    timeout: Duration,
    sleep: &dyn Fn(Duration),
) -> Result<(), String> {
    let mut waited = Duration::ZERO;
    loop {
        match poll()? {
            Some(0) => return Ok(()),
            Some(code) => return Err(format!("ipconfig /flushdns: codice {code}")),
            None if waited >= timeout => return Err("Cache DNS: tempo scaduto".to_string()),
            None => {
                sleep(POLL_STEP);
                waited += POLL_STEP;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Registri eventi (N3).

/// Gli unici registri che si svuotano (v1). Sicurezza, Defender, PowerShell,
/// CodeIntegrity, Sysmon, AppLocker, Firewall e gli altri canali restano:
/// svuotarli in blocco è la tecnica che gli antivirus segnalano (T1070.001)
/// e cancella le prove di un'infezione.
pub const CLEARABLE_CHANNELS: [&str; 3] = ["Application", "System", "Setup"];

/// `true` solo per i nomi esatti di [`CLEARABLE_CHANNELS`] (senza maiuscole).
pub fn is_clearable_channel(name: &str) -> bool {
    CLEARABLE_CHANNELS.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// Canali ammessi ([`is_clearable_channel`]) con almeno un evento. Guarda e
/// basta: non svuota niente.
pub fn analyze_event_logs(ev: &dyn EventLog) -> NativeFound {
    let channels: Vec<(String, u64, u64)> =
        ev.channels().into_iter().filter(|(c, n, _)| *n > 0 && is_clearable_channel(c)).collect();
    if channels.is_empty() {
        return NativeFound::default();
    }
    let events: u64 = channels.iter().map(|(_, n, _)| n).sum();
    let bytes: u64 = channels.iter().map(|(_, _, b)| b).sum();
    let mut detail = vec![format!(
        "{}, {}, {}",
        plural(channels.len() as u64, "registro", "registri"),
        plural(events, "evento", "eventi"),
        crate::sys::fmt_size(bytes)
    )];
    detail.extend(
        channels
            .iter()
            .take(MAX_DETAIL)
            .map(|(c, n, b)| format!("{c}: {}, {}", plural(*n, "evento", "eventi"), crate::sys::fmt_size(*b))),
    );
    NativeFound {
        detail,
        count: channels.len() as u64,
        bytes,
        items: channels.into_iter().map(|(c, _, _)| c).collect(),
        ..Default::default()
    }
}

/// Svuota i canali trovati dall'analisi, solo se ammessi
/// ([`is_clearable_channel`]); un canale che non si svuota non ferma gli
/// altri. `done` elenca i canali svuotati.
pub fn clean_event_logs(ev: &dyn EventLog, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    for channel in &found.items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !is_clearable_channel(channel) {
            out.failed += 1;
            out.note(format!("{channel}: registro non ammesso, lasciato"));
            continue;
        }
        match ev.clear(channel) {
            Ok(()) => {
                out.removed += 1;
                out.done.push(format!("Svuotato: {channel}"));
            }
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

/// Cartelle dei collegamenti: menu Start dell'utente (`%AppData%`
/// dell'ambiente fidato) e comune, Desktop dell'utente e pubblico (solo il
/// primo livello). Le cartelle comuni vengono da `common` (cartelle note di
/// Windows), mai dalle variabili. Una cartella che non si risolve in un
/// percorso assoluto con lettera di unità si salta, con una nota.
pub fn shortcut_dirs(env: &Env, common: &CommonDirs) -> (Vec<ShortcutDir>, Vec<String>) {
    let start_menu = env.get("appdata").map(|b| Path::new(b).join("Microsoft").join("Windows").join("Start Menu"));
    let desktop = env
        .get("desktop")
        .map(PathBuf::from)
        .or_else(|| env.get("userprofile").map(|p| Path::new(p).join("Desktop")));
    let mut dirs = Vec::new();
    let mut notes = Vec::new();
    for (name, path, recursive, is_common) in [
        ("Menu Start dell'utente", start_menu, true, false),
        ("Menu Start comune", common.start_menu.clone(), true, true),
        ("Desktop dell'utente", desktop, false, false),
        ("Desktop pubblico", common.public_desktop.clone(), false, true),
    ] {
        match path.filter(|d| is_drive_path(d)) {
            Some(path) => dirs.push(ShortcutDir { path, recursive, common: is_common }),
            None => notes.push(format!("{name}: cartella non trovata, saltata")),
        }
    }
    (dirs, notes)
}

/// Variabili per utente: in una cartella comune una destinazione che le usa
/// vale per un altro utente in un altro modo, quindi non si giudica.
const PER_USER_VARS: &[&str] = &[
    "userprofile", "appdata", "localappdata", "locallowappdata", "homepath", "homedrive", "homeshare",
    "temp", "tmp", "onedrive", "onedriveconsumer", "onedrivecommercial", "username", "userdomain",
];

fn uses_per_user_var(raw: &str) -> bool {
    let low = raw.to_lowercase();
    PER_USER_VARS.iter().any(|v| low.contains(&format!("%{v}%")))
}

/// La radice dei profili (`C:\Users`), dal profilo dell'ambiente.
fn profiles_root(env: &Env) -> Option<PathBuf> {
    env.get("userprofile").and_then(|p| Path::new(p).parent().map(Path::to_path_buf))
}

/// Destinazione mancante di un collegamento da proporre, o `None`.
///
/// Non si giudicano mai: collegamenti illeggibili, «pubblicizzati» di
/// Windows Installer (`SLDF_HAS_DARWINID`), senza `LinkInfo` (non solo
/// ID-list di app pacchettizzate e cartelle virtuali: anche molti collegamenti
/// di sistema, come Prompt dei comandi o Strumenti di amministrazione, che
/// hanno la destinazione solo in `EnvironmentVariableDataBlock`, flag
/// `SLDF_HAS_EXP_SZ` più `SLDF_FORCE_NO_LINKINFO`: limite noto, nel verso
/// prudente, perché non si propongono mai), senza percorsi, con un percorso
/// non Unicode valido o lungo [`MAX_TARGET_LEN`] o più (forse troncato).
/// Nelle cartelle comuni (`common`) neppure quelli con variabili per utente o
/// con la destinazione dentro un profilo diverso da `Public`.
///
/// Si propone solo se **ogni** destinazione letta (con le variabili espanse
/// dall'ambiente fidato, più le gemelle WOW64: `Program Files` ↔
/// `Program Files (x86)`, `System32` ↔ `SysWOW64`) è un percorso assoluto
/// con lettera di unità, sta su un disco fisso presente e manca davvero
/// secondo [`truly_missing`]. UNC, dischi rimovibili o di rete, URL,
/// `shell:` e variabili sconosciute non si propongono mai.
pub fn broken_target(sc: &dyn Shortcuts, env: &Env, lnk: &Path, common: bool) -> Option<PathBuf> {
    let link = sc.targets(lnk)?;
    if link.flags & SLDF_HAS_DARWINID != 0 || link.flags & SLDF_HAS_LINK_INFO == 0 || link.paths.is_empty() {
        return None;
    }
    let users = profiles_root(env);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for raw in &link.paths {
        let raw = raw.to_str()?.trim();
        if raw.is_empty() || raw.encode_utf16().count() >= MAX_TARGET_LEN {
            return None;
        }
        if common && uses_per_user_var(raw) {
            return None;
        }
        let expanded = expand_vars(raw, env)?;
        // `\\?\C:\…` come `C:\…`, così le gemelle WOW64 si riconoscono.
        let p = PathBuf::from(match expanded.strip_prefix(r"\\?\") {
            Some(rest) if drive_letter(Path::new(rest)).is_some() => rest.to_string(),
            _ => expanded,
        });
        if !is_drive_path(&p) {
            return None;
        }
        let in_profile = |users: &PathBuf| {
            strictly_under(&p, users) && !comps(&p).get(comps(users).len()).is_some_and(|c| c == "public")
        };
        if common && users.as_ref().is_some_and(in_profile) {
            return None;
        }
        for q in std::iter::once(p.clone()).chain(wow64_twins(&p, env)) {
            if !candidates.iter().any(|c| same_path(c, &q)) {
                candidates.push(q);
            }
        }
    }
    if candidates.iter().all(|c| sc.volume_is_fixed(c) && truly_missing(c)) {
        candidates.into_iter().next()
    } else {
        None
    }
}

/// `true` solo se `p` manca davvero sul disco della sua lettera.
///
/// Si scende dalla radice componente per componente con `symlink_metadata`
/// (che non segue il componente esaminato): un componente esistente che è un
/// punto di reparse (collegamento simbolico, giunzione, cartella di
/// montaggio) fa rispondere `false`, perché la destinazione vera può stare su
/// un disco di rete o staccato; il primo componente che manca deve dare
/// `ERROR_FILE_NOT_FOUND` (2) o `ERROR_PATH_NOT_FOUND` (3), ogni altro errore
/// (rete, accesso negato, nome non valido) fa rispondere `false`. Un percorso
/// con `..` non si giudica. Si apre solo ciò che sta prima del primo reparse:
/// nessuna attesa di rete.
pub fn truly_missing(p: &Path) -> bool {
    if p.components().any(|c| matches!(c, Component::ParentDir)) || !is_drive_path(p) {
        return false;
    }
    let chain: Vec<&Path> = p.ancestors().collect();
    for a in chain.iter().rev() {
        if a.parent().is_none() {
            continue; // radice del disco
        }
        match std::fs::symlink_metadata(a) {
            Ok(md) if is_reparse(&md) => return false,
            Ok(_) => {}
            Err(e) => return matches!(e.raw_os_error(), Some(2 | 3)),
        }
    }
    false
}

/// I collegamenti non validi delle [`shortcut_dirs`]; quelli che il `Guard`
/// rifiuta, e quelli con un nome non Unicode valido, si contano fra i saltati.
pub fn analyze_shortcuts(sc: &dyn Shortcuts, env: &Env, guard: &Guard, cancel: &AtomicBool) -> NativeFound {
    let mut found = NativeFound::default();
    let mut lines: Vec<String> = Vec::new();
    let (dirs, notes) = shortcut_dirs(env, &sc.common_dirs());
    'dirs: for dir in dirs {
        for lnk in sc.list(&dir.path, dir.recursive) {
            if cancel.load(Ordering::Relaxed) {
                break 'dirs;
            }
            let Some(text) = lnk.to_str().map(str::to_string) else {
                found.skipped += 1;
                continue;
            };
            if guard.protected(&lnk).is_some() {
                found.skipped += 1;
                continue;
            }
            let Some(target) = broken_target(sc, env, &lnk, dir.common) else { continue };
            found.count += 1;
            found.bytes += std::fs::symlink_metadata(&lnk).map(|m| m.len()).unwrap_or(0);
            if lines.len() < MAX_DETAIL {
                lines.push(format!("{text} → {}", target.display()));
            }
            found.items.push(text);
        }
    }
    found.files = found.count;
    if found.skipped > 0 {
        notes_push_skipped(&mut found);
    }
    if found.count > 0 {
        found.detail.push(plural(found.count, "collegamento non valido", "collegamenti non validi"));
        found.detail.extend(lines);
        if found.count as usize > MAX_DETAIL {
            found.detail.push(format!("… e altri {}", found.count as usize - MAX_DETAIL));
        }
    }
    found.detail.extend(notes);
    found
}

fn notes_push_skipped(found: &mut NativeFound) {
    let what = plural(found.skipped, "collegamento lasciato", "collegamenti lasciati");
    let line = format!("{what}: protetti o con un nome non leggibile");
    found.detail.push(line);
}

/// Cancella i `.lnk` trovati dall'analisi, ciascuno solo se: sta in una delle
/// [`shortcut_dirs`] (il Desktop solo al primo livello), finisce in `.lnk`, è
/// ancora non valido. La cancellazione passa sempre da
/// `pc_rules::delete_checked_with` con il `Guard` della pulizia.
pub fn clean_shortcuts(sc: &dyn Shortcuts, env: &Env, guard: &Guard, found: &NativeFound, cancel: &AtomicBool) -> NativeCleaned {
    let mut out = NativeCleaned::default();
    let (dirs, _) = shortcut_dirs(env, &sc.common_dirs());
    for item in &found.items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let lnk = PathBuf::from(item);
        let home = dirs.iter().find(|d| {
            if d.recursive {
                strictly_under(&lnk, &d.path)
            } else {
                lnk.parent().is_some_and(|p| same_path(p, &d.path))
            }
        });
        let Some(home) = home.filter(|_| is_lnk(&lnk)) else {
            out.failed += 1;
            out.note(format!("{item}: fuori dalle cartelle dei collegamenti, lasciato"));
            continue;
        };
        if matches!(std::fs::symlink_metadata(&lnk), Err(e) if e.kind() == ErrorKind::NotFound) {
            out.note(format!("{item}: già assente"));
            continue;
        }
        if broken_target(sc, env, &lnk, home.common).is_none() {
            out.note(format!("{item}: non più da togliere (destinazione tornata o non giudicabile), lasciato"));
            continue;
        }
        match pc_rules::delete_checked_with(&lnk, guard) {
            Ok(bytes) => {
                out.removed += 1;
                out.freed += bytes;
                out.done.push(item.clone());
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

/// Componenti confrontabili (minuscole, senza `\\?\` e senza `.`); `..`
/// resta come componente e fa fallire i confronti.
fn comps(p: &Path) -> Vec<String> {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    Path::new(s)
        .components()
        .filter(|c| !matches!(c, Component::CurDir | Component::RootDir))
        .map(|c| comp_text(&c))
        .collect()
}

fn has_parent_dir(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::ParentDir))
}

/// `p` sta dentro `dir` (non è `dir`); mai con `..`.
fn strictly_under(p: &Path, dir: &Path) -> bool {
    if has_parent_dir(p) || has_parent_dir(dir) {
        return false;
    }
    let (p, d) = (comps(p), comps(dir));
    d.len() >= 2 && p.len() > d.len() && p[..d.len()] == d[..]
}

/// Stessa voce (stessi componenti); mai con `..`.
fn same_path(a: &Path, b: &Path) -> bool {
    !has_parent_dir(a) && !has_parent_dir(b) && comps(a) == comps(b)
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
    use windows_sys::Win32::Foundation::{FreeLibrary, RPC_E_CHANGED_MODE, S_OK};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        COINIT_DISABLE_OLE1DDE, STGM_READ,
    };
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, CountClipboardFormats, EmptyClipboard, OpenClipboard};
    use windows_sys::Win32::System::EventLog::{
        EvtChannelConfigType, EvtClearLog, EvtClose, EvtGetChannelConfigProperty, EvtGetLogInfo, EvtLogFileSize, EvtLogNumberOfLogRecords,
        EvtOpenChannelConfig, EvtOpenChannelPath, EvtOpenLog, EvtVarTypeUInt32, EvtVarTypeUInt64, EVT_HANDLE,
        EVT_VARIANT,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_CommonStartMenu, FOLDERID_Documents, FOLDERID_PublicDesktop, SHGetKnownFolderPath, ShellLink,
        KF_FLAG_DONT_VERIFY, SLGP_RAWPATH,
    };

    use std::ffi::OsString;
    use std::io::Read;
    use std::os::windows::ffi::OsStringExt;

    use super::{Clipboard, CommonDirs, Dns, EventLog, LinkTargets, Shortcuts};

    /// Una cartella nota di Windows (`SHGetKnownFolderPath`, token del processo).
    fn known_folder(id: &GUID) -> Option<PathBuf> {
        let mut raw: windows_sys::core::PWSTR = null_mut();
        // SAFETY: GUID valido, token nullo (utente del processo), `raw` riceve
        // una stringa allocata con CoTaskMemAlloc, liberata sotto su ogni percorso
        // (anche in errore la funzione può scriverla: si libera comunque).
        let hr = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY as u32, null_mut(), &mut raw) };
        let out = if hr >= 0 && !raw.is_null() {
            // SAFETY: stringa terminata da zero restituita da Windows.
            let n = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
            // SAFETY: `n` u16 leggibili a partire da `raw` (contati sopra).
            let wide = unsafe { std::slice::from_raw_parts(raw, n) };
            Some(PathBuf::from(OsString::from_wide(wide)))
        } else {
            None
        };
        // SAFETY: memoria di SHGetKnownFolderPath (o nulla), liberata una volta.
        unsafe { CoTaskMemFree(raw as *const c_void) };
        out
    }

    /// Documenti dell'utente del processo (segue il reindirizzamento a OneDrive).
    pub fn documents() -> Option<PathBuf> {
        known_folder(&FOLDERID_Documents)
    }

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
            let mut child = std::process::Command::new(exe)
                .arg("/flushdns")
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let mut poll = || match child.try_wait() {
                Ok(Some(st)) => Ok(Some(st.code().unwrap_or(-1))),
                Ok(None) => Ok(None),
                Err(e) => Err(e.to_string()),
            };
            let out = super::wait_bounded(&mut poll, super::IPCONFIG_TIMEOUT, &std::thread::sleep);
            if out.is_err() {
                let _ = child.kill();
                let _ = child.wait();
            }
            out
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
        fn channels(&self) -> Vec<(String, u64, u64)> {
            // Solo i canali ammessi: nessuna enumerazione di tutti i registri.
            super::CLEARABLE_CHANNELS
                .iter()
                .filter_map(|name| channel_records(name).map(|(n, bytes)| (name.to_string(), n, bytes)))
                .collect()
        }

        fn clear(&self, channel: &str) -> Result<(), String> {
            let w = wide(channel);
            // SAFETY: sessione locale (0), nome terminato da zero, nessun file
            // di esportazione (puntatore nullo), flag 0.
            if unsafe { EvtClearLog(0, w.as_ptr(), null(), 0) } != 0 { Ok(()) } else { Err(last_error()) }
        }
    }

    /// Eventi e dimensione del file (`EvtLogFileSize`, 0 se non si legge) di
    /// un canale amministrativo o operativo (gli analitici e di debug non si
    /// toccano); `None` se gli eventi non si leggono.
    fn channel_records(name: &str) -> Option<(u64, u64)> {
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
        // Una proprietà UInt64 del registro aperto; `None` se non si legge.
        let info = |prop| {
            let mut v = EVT_VARIANT::default();
            let mut used = 0u32;
            // SAFETY: `v` è un EVT_VARIANT intero e la dimensione passata è la sua;
            // `EvtLogNumberOfLogRecords` e `EvtLogFileSize` sono UInt64.
            let ok = unsafe { EvtGetLogInfo(log.0, prop, size_of::<EVT_VARIANT>() as u32, &mut v, &mut used) };
            // SAFETY: si legge il campo dell'unione solo se `Type` dice UInt64.
            if ok == 0 {
                None
            } else if v.Type == EvtVarTypeUInt64 as u32 {
                Some(unsafe { v.Anonymous.UInt64Val })
            } else {
                Some(0)
            }
        };
        let records = info(EvtLogNumberOfLogRecords)?;
        Some((records, info(EvtLogFileSize).unwrap_or(0)))
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

        fn targets(&self, lnk: &Path) -> Option<LinkTargets> {
            // Prima l'intestazione, letta come byte (nessun codice della shell):
            // un collegamento pubblicizzato o senza `LinkInfo` non si apre con COM.
            let mut header = [0u8; 0x4C];
            std::fs::File::open(lnk).ok()?.read_exact(&mut header).ok()?;
            let flags = super::link_flags(&header)?;
            if flags & super::SLDF_HAS_DARWINID != 0 || flags & super::SLDF_HAS_LINK_INFO == 0 {
                return Some(LinkTargets { flags, paths: Vec::new() });
            }
            // `_apt` dichiarato prima: le interfacce (`Com`) si rilasciano
            // prima di `CoUninitialize`, su ogni percorso di uscita.
            let _apt = Apartment::enter()?;
            let mut raw: *mut c_void = null_mut();
            // SAFETY: CLSID e IID validi, nessun aggregato, `raw` riceve
            // un'interfaccia IShellLinkW con un riferimento (rilasciato da `Com`).
            let hr = unsafe { CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut raw) };
            if hr < 0 || raw.is_null() {
                return None;
            }
            let link = Com(raw);
            let mut raw: *mut c_void = null_mut();
            // SAFETY: `link` è un IShellLinkW vivo; QueryInterface scrive in
            // `raw` un IPersistFile con un riferimento (rilasciato da `Com`).
            let hr = unsafe { (link.vtbl::<IUnknownVtbl>().query_interface)(link.0, &IID_IPERSISTFILE, &mut raw) };
            if hr < 0 || raw.is_null() {
                return None;
            }
            let file = Com(raw);
            let path = wide_path(lnk);
            // SAFETY: `file` è un IPersistFile vivo, percorso terminato da zero,
            // `STGM_READ`: il collegamento si legge e basta.
            if unsafe { (file.vtbl::<IPersistFileVtbl>().load)(file.0, path.as_ptr(), STGM_READ) } < 0 {
                return None;
            }
            let mut paths: Vec<OsString> = Vec::new();
            for flag in [0u32, SLGP_RAWPATH as u32] {
                let mut buf = vec![0u16; 32_768];
                // SAFETY: `buf` ha `buf.len()` u16 scrivibili e `cch` è quella
                // lunghezza; `pfd` nullo è ammesso; il risultato è terminato da zero.
                let hr = unsafe {
                    (link.vtbl::<IShellLinkWVtbl>().get_path)(link.0, buf.as_mut_ptr(), buf.len() as i32, null_mut(), flag)
                };
                if hr != S_OK {
                    continue; // S_FALSE: nessun percorso (URL, shell:, app pacchettizzata).
                }
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                // Senza conversioni: un surrogato spaiato resta tale e il
                // collegamento poi non si giudica (`to_str` dà `None`).
                let s = OsString::from_wide(&buf[..n]);
                if !s.is_empty() && !paths.contains(&s) {
                    paths.push(s);
                }
            }
            Some(LinkTargets { flags, paths })
        }

        fn common_dirs(&self) -> CommonDirs {
            CommonDirs {
                start_menu: known_folder(&FOLDERID_CommonStartMenu),
                public_desktop: known_folder(&FOLDERID_PublicDesktop),
            }
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
        channels: Vec<(String, u64, u64)>,
        fail: Vec<String>,
        cleared: RefCell<Vec<String>>,
    }

    impl EventLog for FakeEvents {
        fn channels(&self) -> Vec<(String, u64, u64)> {
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
        /// Destinazioni (testo); i flag sono quelli di un collegamento normale
        /// (ID-list e `LinkInfo`) se `flags` non dice altro.
        targets: HashMap<PathBuf, Vec<String>>,
        flags: HashMap<PathBuf, u32>,
        /// Destinazioni in UTF-16 grezzo (per i nomi non Unicode validi).
        wide: HashMap<PathBuf, Vec<std::ffi::OsString>>,
        /// Lettere di unità non fisse (rimovibili, di rete, assenti).
        not_fixed: Vec<char>,
        common: CommonDirs,
    }

    /// `LinkFlags` di un collegamento normale a un file: ID-list e `LinkInfo`.
    const NORMAL: u32 = 0x1 | SLDF_HAS_LINK_INFO;

    impl Shortcuts for FakeLinks {
        fn list(&self, dir: &Path, recursive: bool) -> Vec<PathBuf> {
            list_lnk(dir, recursive)
        }
        fn targets(&self, lnk: &Path) -> Option<LinkTargets> {
            let flags = self.flags.get(lnk).copied().unwrap_or(NORMAL);
            if let Some(w) = self.wide.get(lnk) {
                return Some(LinkTargets { flags, paths: w.clone() });
            }
            let t = self.targets.get(lnk)?;
            Some(LinkTargets { flags, paths: t.iter().map(std::ffi::OsString::from).collect() })
        }
        fn volume_is_fixed(&self, p: &Path) -> bool {
            drive_letter(p).is_some_and(|d| !self.not_fixed.contains(&(d as char).to_ascii_uppercase()))
        }
        fn common_dirs(&self) -> CommonDirs {
            self.common.clone()
        }
    }

    /// Giunzione `link` → `target` (`mklink /J`, nessun privilegio).
    fn junction(link: &Path, target: &Path) {
        let st = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "mklink /J");
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
        assert_eq!(f.files, 0, "gli Appunti non sono un file");
        assert_eq!(analyze_dns().files, 0, "il DNS non è un file");
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
            channels: vec![
                ("Application".into(), 10, 20_975_616),
                ("Security".into(), 5, 4096),
                ("Setup".into(), 0, 69_632),
                ("Microsoft-Windows-Windows Defender/Operational".into(), 50, 1_052_672),
                ("Setup".into(), 3, 1_052_672),
                ("System".into(), 7, 2 * 1024 * 1024),
            ],
            fail: fail.iter().map(|s| s.to_string()).collect(),
            cleared: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn event_logs_continue_after_a_channel_fails() {
        let ev = events(&["Setup"]);
        let found = analyze_event_logs(&ev);
        assert_eq!(found.count, 3);
        assert_eq!(found.items, vec!["Application", "Setup", "System"]);
        assert_eq!(found.files, 0, "i registri non sono file");
        let out = clean_event_logs(&ev, &found, &AtomicBool::new(false));
        assert_eq!((out.removed, out.failed), (2, 1));
        assert_eq!(*ev.cleared.borrow(), vec!["Application", "Setup", "System"]);
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].contains("Setup"), "{:?}", out.notes);
        assert_eq!(out.done, vec!["Svuotato: Application".to_string(), "Svuotato: System".to_string()]);
    }

    #[test]
    fn event_logs_are_never_cleared_by_analysis() {
        let ev = events(&[]);
        let found = analyze_event_logs(&ev);
        assert!(ev.cleared.borrow().is_empty());
        assert!(found.detail[0].contains("3 registri"), "{:?}", found.detail);
        assert!(found.detail[0].contains("20 eventi"), "{:?}", found.detail);
        // Dimensione dei file dei registri (`EvtLogFileSize`), solo dei canali proposti.
        assert_eq!(found.bytes, 20_975_616 + 1_052_672 + 2 * 1024 * 1024);
        assert!(found.detail[0].contains("23,0 MB"), "{:?}", found.detail);
        assert!(found.detail.iter().any(|d| d == "Application: 10 eventi, 20,0 MB"), "{:?}", found.detail);
        // Si svuota solo ciò che l'analisi ha trovato.
        let partial = NativeFound { items: vec!["System".into()], ..found };
        clean_event_logs(&ev, &partial, &AtomicBool::new(false));
        assert_eq!(*ev.cleared.borrow(), vec!["System"]);
    }

    #[test]
    fn security_and_defender_channels_are_never_listed() {
        for ok in ["Application", "System", "Setup", "application", "SYSTEM"] {
            assert!(is_clearable_channel(ok), "{ok}");
        }
        for no in [
            "Security",
            "Microsoft-Windows-Windows Defender/Operational",
            "Microsoft-Windows-Windows Defender/WHC",
            "Windows PowerShell",
            "Microsoft-Windows-PowerShell/Operational",
            "Microsoft-Windows-CodeIntegrity/Operational",
            "Microsoft-Windows-Sysmon/Operational",
            "Microsoft-Windows-AppLocker/EXE and DLL",
            "Microsoft-Windows-Windows Firewall With Advanced Security/Firewall",
            "Microsoft-Windows-Security-Mitigations/KernelMode",
            "ForwardedEvents",
            "System ",
            "System/Operational",
            "Microsoft-Windows-Setup/Analytic",
            "",
        ] {
            assert!(!is_clearable_channel(no), "{no}");
        }
        let ev = events(&[]);
        let found = analyze_event_logs(&ev);
        assert!(!found.items.iter().any(|c| c == "Security" || c.contains("Defender")), "{:?}", found.items);
        // Un `NativeFound` forzato con Security non lo svuota.
        let forged = NativeFound { items: vec!["Security".into(), "System".into()], ..Default::default() };
        let out = clean_event_logs(&ev, &forged, &AtomicBool::new(false));
        assert_eq!(*ev.cleared.borrow(), vec!["System"]);
        assert_eq!((out.removed, out.failed), (1, 1));
    }

    // ----- DNS: tempo massimo di ipconfig -----

    #[test]
    fn ipconfig_wait_is_bounded() {
        let slept = Cell::new(Duration::ZERO);
        let sleep = |d: Duration| slept.set(slept.get() + d);
        let mut never = || Ok(None);
        assert_eq!(wait_bounded(&mut never, IPCONFIG_TIMEOUT, &sleep), Err("Cache DNS: tempo scaduto".to_string()));
        assert!(slept.get() >= IPCONFIG_TIMEOUT && slept.get() <= IPCONFIG_TIMEOUT + Duration::from_millis(100));
        let calls = Cell::new(0);
        let mut later = || {
            calls.set(calls.get() + 1);
            Ok(if calls.get() < 3 { None } else { Some(0) })
        };
        assert_eq!(wait_bounded(&mut later, IPCONFIG_TIMEOUT, &no), Ok(()));
        let mut bad = || Ok(Some(1));
        assert!(wait_bounded(&mut bad, IPCONFIG_TIMEOUT, &no).unwrap_err().contains("codice 1"));
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

        assert_eq!(broken_target(&sc, &env, &broken, false), Some(gone.clone()));
        assert_eq!(broken_target(&sc, &env, &fine, false), None);
        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        let mut items = found.items.clone();
        items.sort();
        let mut want = vec![broken.to_string_lossy().into_owned(), on_desktop.to_string_lossy().into_owned()];
        want.sort();
        assert_eq!(items, want);
        assert_eq!(found.count, 2);
        assert_eq!(found.bytes, 6);
        assert_eq!(found.files, 2, "i collegamenti sono file");
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
            assert_eq!(broken_target(&sc, &env, &lnk, false), None, "{name}");
        }
        // Gemella WOW64: `%ProgramFiles%` letto a 64 bit, programma a 32 bit.
        let wow = dir.join("wow.lnk");
        touch(&wow);
        touch(&t.path().join("PF86").join("Vecchio").join("app.exe"));
        sc.targets.insert(wow.clone(), vec![t.path().join("PF").join("Vecchio").join("app.exe").to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &wow, false), None, "gemella in Program Files (x86)");
        // Anche scritta con il prefisso `\\?\`.
        let verbatim = format!(r"\\?\{}", t.path().join("PF").join("Vecchio").join("app.exe").display());
        sc.targets.insert(wow.clone(), vec![verbatim]);
        assert_eq!(broken_target(&sc, &env, &wow, false), None, "gemella con \\\\?\\");
        // Una destinazione che esiste fra due lette basta a tenerlo.
        let two = dir.join("due.lnk");
        touch(&two);
        let here = t.path().join("qui.exe");
        touch(&here);
        sc.targets.insert(two.clone(), vec![t.path().join("manca.exe").to_string_lossy().into_owned(), here.to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &two, false), None);

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

        // `..` che esce dal menu Start, e un `.lnk` raggiunto da una giunzione nel menu Start.
        let dotdot = start_menu(&env).join(r"..\..\..\..\..\..\..").join("Altrove").join("Rotto.lnk");
        let jdir = start_menu(&env).join("Giunzione");
        junction(&jdir, outside.parent().unwrap());
        let via_junction = jdir.join("Rotto.lnk");
        sc.targets.insert(via_junction.clone(), vec![gone.clone()]);
        let found = found_items(&[&broken, &outside, &not_lnk, &deep_desktop, &healed, &dotdot, &via_junction]);
        let out = clean_shortcuts(&sc, &env, &guard, &found, &AtomicBool::new(false));
        assert!(!broken.exists(), "il collegamento trovato e ancora rotto si cancella");
        assert_eq!(out.removed, 1);
        assert_eq!(out.freed, 3);
        for p in [&outside, &not_lnk, &deep_desktop, &healed, &not_found] {
            assert!(p.exists(), "{} non si tocca", p.display());
        }
        assert_eq!(out.failed, 5, "{:?}", out.notes);
        assert_eq!(out.done, vec![broken.to_string_lossy().into_owned()]);
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
        let common = CommonDirs {
            start_menu: Some(r"C:\ProgramData\Microsoft\Windows\Start Menu".into()),
            public_desktop: Some(r"C:\Users\Public\Desktop".into()),
        };
        let (dirs, notes) = shortcut_dirs(&env, &common);
        let d = |p: &str, recursive: bool, common: bool| ShortcutDir { path: p.into(), recursive, common };
        let want = vec![
            d(r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu", true, false),
            d(r"C:\ProgramData\Microsoft\Windows\Start Menu", true, true),
            d(r"C:\Users\u\Desktop", false, false),
            d(r"C:\Users\Public\Desktop", false, true),
        ];
        assert_eq!(dirs, want);
        assert!(notes.is_empty());
        let guard = Guard::new(&env);
        for dir in &dirs {
            let lnk = if dir.recursive { dir.path.join("Programs").join("App").join("App.lnk") } else { dir.path.join("App.lnk") };
            assert_eq!(guard.protected(&lnk), None, "{}", lnk.display());
        }
    }

    #[test]
    fn common_folders_come_from_known_folders_or_are_skipped() {
        // Le variabili `programdata` e `public` non contano: solo `CommonDirs`.
        let env = Env(HashMap::from([
            ("appdata".to_string(), r"C:\Users\u\AppData\Roaming".to_string()),
            ("desktop".to_string(), r"C:\Users\u\Desktop".to_string()),
            ("programdata".to_string(), r"C:\Finta".to_string()),
            ("public".to_string(), r"C:\FintoPubblico".to_string()),
        ]));
        let common = CommonDirs { start_menu: None, public_desktop: Some(r"\\server\Desktop".into()) };
        let (dirs, notes) = shortcut_dirs(&env, &common);
        assert_eq!(dirs.iter().map(|d| d.path.clone()).collect::<Vec<_>>(), vec![
            PathBuf::from(r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu"),
            PathBuf::from(r"C:\Users\u\Desktop"),
        ]);
        assert_eq!(notes, vec![
            "Menu Start comune: cartella non trovata, saltata".to_string(),
            "Desktop pubblico: cartella non trovata, saltata".to_string(),
        ]);
        // La nota arriva nel resoconto dell'analisi.
        let found = analyze_shortcuts(&FakeLinks { common, ..Default::default() }, &env, &Guard::new(&env), &AtomicBool::new(false));
        assert!(found.detail.iter().any(|l| l.contains("Menu Start comune")), "{:?}", found.detail);
    }

    #[test]
    fn per_user_targets_in_common_folders_are_never_listed() {
        let t = tempfile::tempdir().unwrap();
        // Profili in `<tmp>\Users`, `%LocalAppData%` fuori dalla radice dei
        // profili: così la sola variabile per utente basta a escludere.
        let mut env = env_in(t.path());
        let users = t.path().join("Users");
        env.0.insert("userprofile".into(), users.join("u").to_string_lossy().into_owned());
        env.0.insert("localappdata".into(), t.path().join("Fuori").join("Local").to_string_lossy().into_owned());
        let common_menu = t.path().join("ProgramData").join(r"Microsoft\Windows\Start Menu");
        let sc_common = CommonDirs { start_menu: Some(common_menu.clone()), public_desktop: None };
        let mut sc = FakeLinks { common: sc_common, ..Default::default() };
        let per_var = common_menu.join("Programs").join("PerUtente.lnk");
        let in_profile = common_menu.join("Programs").join("NelProfilo.lnk");
        let shared = common_menu.join("Programs").join("Comune.lnk");
        for l in [&per_var, &in_profile, &shared] {
            touch(l);
        }
        sc.targets.insert(per_var.clone(), vec![r"%LocalAppData%\Programs\X\x.exe".into()]);
        // La radice dei profili è la cartella di `userprofile` (qui la tempdir).
        sc.targets.insert(in_profile.clone(), vec![users.join("altro").join("x.exe").to_string_lossy().into_owned()]);
        let gone = users.join("Public").join("manca.exe");
        sc.targets.insert(shared.clone(), vec![gone.to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &per_var, true), None);
        assert_eq!(broken_target(&sc, &env, &in_profile, true), None);
        assert_eq!(broken_target(&sc, &env, &shared, true), Some(gone));
        // Nella cartella dell'utente la stessa variabile si giudica.
        assert!(broken_target(&sc, &env, &per_var, false).is_some());
        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        assert_eq!(found.items, vec![shared.to_string_lossy().into_owned()]);
    }

    #[test]
    fn target_behind_a_junction_or_symlink_is_never_missing() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let real = t.path().join("Giochi_E");
        fs::create_dir_all(&real).unwrap();
        let j = t.path().join("Giochi");
        junction(&j, &real);
        fs::remove_dir(&real).unwrap(); // disco «staccato»: la giunzione resta
        let through = j.join("gioco.exe");
        assert!(!truly_missing(&through), "giunzione verso una cartella che non c'è");
        let plain = t.path().join("Vecchio").join("app.exe");
        assert!(truly_missing(&plain));
        assert!(truly_missing(&t.path().join("manca.exe")));
        // Una giunzione viva: il file esiste dall'altra parte, quindi non manca.
        let alive = t.path().join("vivo");
        fs::create_dir_all(&alive).unwrap();
        let j2 = t.path().join("Vivo_j");
        junction(&j2, &alive);
        assert!(!truly_missing(&j2.join("assente.exe")), "anche una giunzione viva ferma il giudizio");
        // `..` e nomi non validi non si giudicano.
        assert!(!truly_missing(&t.path().join("a").join("..").join("manca.exe")));
        assert!(!truly_missing(&t.path().join("bad<name>.exe")));

        let lnk = start_menu(&env).join("Gioco.lnk");
        touch(&lnk);
        let mut sc = FakeLinks::default();
        sc.targets.insert(lnk.clone(), vec![through.to_string_lossy().into_owned()]);
        assert_eq!(broken_target(&sc, &env, &lnk, false), None);
        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        assert_eq!(found.count, 0);
        let out = clean_shortcuts(&sc, &env, &Guard::new(&env), &found_items(&[&lnk]), &AtomicBool::new(false));
        assert!(lnk.exists());
        assert_eq!(out.removed, 0);

        // Collegamento simbolico verso un UNC che non risponde: solo se si può
        // creare (modalità sviluppatore o amministratore), altrimenti saltato.
        let s_unc = t.path().join("s_unc");
        if std::os::windows::fs::symlink_dir(r"\\host-inesistente.invalid\share", &s_unc).is_ok() {
            assert!(!truly_missing(&s_unc.join("app.exe")), "symlink verso UNC");
        }
    }

    #[test]
    fn skipped_shortcuts_show_up_in_the_report() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        // Una cartella di credenziali nel menu Start: il `Guard` la rifiuta.
        let lnk = start_menu(&env).join(".ssh").join("x.lnk");
        touch(&lnk);
        let mut sc = FakeLinks::default();
        sc.targets.insert(lnk.clone(), vec![t.path().join("manca.exe").to_string_lossy().into_owned()]);
        let found = analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false));
        assert_eq!((found.count, found.skipped), (0, 1));
        assert!(found.detail.iter().any(|l| l == "1 collegamento lasciato: protetti o con un nome non leggibile"), "{:?}", found.detail);
    }

    #[test]
    fn advertised_msi_and_id_list_only_shortcuts_are_never_listed() {
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let gone = t.path().join("manca").join("Icon.exe").to_string_lossy().into_owned();
        let mut sc = FakeLinks::default();
        let darwin = start_menu(&env).join("Msi.lnk");
        let id_only = start_menu(&env).join("App.lnk");
        let empty = start_menu(&env).join("Vuoto.lnk");
        let unreadable = start_menu(&env).join("Illeggibile.lnk");
        for l in [&darwin, &id_only, &empty, &unreadable] {
            touch(l);
        }
        sc.targets.insert(darwin.clone(), vec![gone.clone()]);
        sc.flags.insert(darwin.clone(), NORMAL | SLDF_HAS_DARWINID);
        sc.targets.insert(id_only.clone(), vec![gone.clone()]);
        sc.flags.insert(id_only.clone(), 0x1);
        sc.targets.insert(empty.clone(), vec!["   ".into()]);
        assert_eq!(broken_target(&sc, &env, &darwin, false), None, "Darwin");
        assert_eq!(broken_target(&sc, &env, &id_only, false), None, "solo ID-list");
        assert_eq!(broken_target(&sc, &env, &empty, false), None, "destinazione vuota");
        assert_eq!(broken_target(&sc, &env, &unreadable, false), None, "illeggibile");
        assert_eq!(analyze_shortcuts(&sc, &env, &Guard::new(&env), &AtomicBool::new(false)).count, 0);
    }

    #[test]
    fn link_flags_are_read_from_a_real_header() {
        let mut h = vec![0u8; 0x4C];
        h[0] = 0x4C;
        h[4..20].copy_from_slice(&[0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
        h[0x14..0x18].copy_from_slice(&0x50ddu32.to_le_bytes()); // flag letti su un collegamento MSI vero
        assert_eq!(link_flags(&h), Some(0x50dd));
        assert_ne!(link_flags(&h).unwrap() & SLDF_HAS_DARWINID, 0);
        let mut bad = h.clone();
        bad[4] = 0x02;
        assert_eq!(link_flags(&bad), None, "CLSID diverso");
        assert_eq!(link_flags(&h[..0x30]), None, "troppo corta");
        let mut size = h.clone();
        size[0] = 0x50;
        assert_eq!(link_flags(&size), None, "HeaderSize diverso");
    }

    #[test]
    fn long_or_non_unicode_targets_are_never_listed() {
        use std::os::windows::ffi::OsStringExt;
        let t = tempfile::tempdir().unwrap();
        let env = env_in(t.path());
        let mut sc = FakeLinks::default();
        let long = start_menu(&env).join("Lungo.lnk");
        let odd = start_menu(&env).join("Strano.lnk");
        touch(&long);
        touch(&odd);
        let base = t.path().to_string_lossy().into_owned();
        // Tante cartelle corte (non un nome troppo lungo): senza il limite
        // risulterebbe «mancante» (errore 3).
        let long_target = format!("{base}\\{}app.exe", "cartella\\".repeat(30));
        sc.targets.insert(long.clone(), vec![long_target]);
        let mut w: Vec<u16> = format!("{base}\\manca").encode_utf16().collect();
        w.push(0xD800); // surrogato spaiato
        w.extend(".exe".encode_utf16());
        sc.wide.insert(odd.clone(), vec![std::ffi::OsString::from_wide(&w)]);
        assert_eq!(broken_target(&sc, &env, &long, false), None, "≥ MAX_PATH: forse troncato");
        assert_eq!(broken_target(&sc, &env, &odd, false), None, "non Unicode valido");
        // Lunghezza appena sotto il limite: si giudica.
        let ok = start_menu(&env).join("Giusto.lnk");
        touch(&ok);
        let pad = MAX_TARGET_LEN - 1 - base.encode_utf16().count() - "\\.exe".len();
        let short = format!("{base}\\{}.exe", "y".repeat(pad));
        assert_eq!(short.encode_utf16().count(), MAX_TARGET_LEN - 1);
        sc.targets.insert(ok.clone(), vec![short]);
        assert!(broken_target(&sc, &env, &ok, false).is_some());
    }

    #[test]
    fn parent_dir_components_never_match_a_folder() {
        let dir = Path::new(r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu");
        let evil = Path::new(r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\..\..\..\..\..\..\Altrove\Vittima.lnk");
        assert!(!strictly_under(evil, dir));
        assert!(strictly_under(&dir.join("Programs").join("A.lnk"), dir));
        assert!(!same_path(Path::new(r"C:\Users\u\Desktop\x\.."), Path::new(r"C:\Users\u\Desktop\x\..")));
        assert!(same_path(Path::new(r"C:\Users\u\Desktop"), Path::new(r"c:\users\U\desktop")));
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
        assert!(w.contains("diagnosi") && w.contains("Applicazione, Sistema e Installazione"), "{w}");
        assert!(w.contains("Sicurezza, Windows Defender") && w.contains("restano"), "{w}");
        assert!(ev.description.contains("104"), "{}", ev.description);
        assert!(!by("broken_shortcuts").default_on && by("broken_shortcuts").warning.is_some());
        assert_eq!(by("clipboard").label, "Appunti");
        assert_eq!(by("dns_cache").label, "Cache DNS");
        for i in &items {
            let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap_or(""));
            assert!(!text.contains("BleachBit") && !text.contains("AGG -"), "{text}");
        }
    }
}
