//! Verifiche dal vivo, **in sola lettura**, sul PC vero: le chiamate Win32
//! delle voci native (`src/sysclean.rs`, Task 2) e le opzioni di registro dei
//! due XML nuovi (`cleaners/windows_extra.xml`, `cleaners/safari.xml`, Task 1),
//! che i test normali provano solo con implementazioni finte.
//!
//! Tutti i test sono `#[ignore]` e si lanciano a mano:
//!
//! ```text
//! cargo test -j 1 live_ -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Regole: si chiamano solo gli `analyze` delle voci e `rules::analyze_option`
//! con `rules::reg_exists` (la stessa lettura dell'anteprima dell'app). Mai
//! `clean`, mai `EmptyClipboard`, mai `DnsFlushResolverCache` (se ne risolve
//! solo l'indirizzo), mai `ipconfig`, mai `EvtClearLog`, nessuna scrittura o
//! cancellazione di file, chiavi o valori. I test stampano quello che trovano;
//! falliscono solo se l'analisi propone qualcosa che la verifica indipendente
//! (scritta qui, con le stesse API in sola lettura) smentisce.

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::{null, null_mut};
use std::sync::atomic::AtomicBool;

use xiox_rules::{CollectCtl, Env, Guard, RegItem, RegRoot};
use windows_sys::core::{GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{FreeLibrary, RPC_E_CHANGED_MODE, S_OK};
use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE, STGM_READ,
};
use windows_sys::Win32::System::DataExchange::CountClipboardFormats;
use windows_sys::Win32::System::EventLog::{
    EvtChannelConfigType, EvtClose, EvtGetChannelConfigProperty, EvtGetLogInfo, EvtLogFileSize,
    EvtLogNumberOfLogRecords, EvtOpenChannelConfig, EvtOpenChannelPath, EvtOpenLog, EvtVarTypeUInt32,
    EvtVarTypeUInt64, EVT_HANDLE, EVT_VARIANT,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_CommonStartMenu, FOLDERID_Desktop, FOLDERID_PublicDesktop, FOLDERID_StartMenu, SHGetKnownFolderPath,
    ShellLink, KF_FLAG_DONT_VERIFY, SLGP_RAWPATH,
};

use crate::native::{NativeCtx, NativeFound};
use crate::sysclean::{self, CommonDirs, MAX_TARGET_LEN, SLDF_HAS_DARWINID, SLDF_HAS_LINK_INFO};

// ---------------------------------------------------------------------------
// Aiuti comuni.

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

/// Analisi reale di una voce nativa (solo `analyze`).
fn analyze_native(id: &str) -> NativeFound {
    let env = Env::for_rules();
    let guard = Guard::new(&env);
    let cancel = AtomicBool::new(false);
    let ctx = NativeCtx { env: &env, cancel: &cancel, guard: &guard };
    let item = crate::native::all()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("native item {id} missing"));
    println!(
        "item {id}: “{}”, default_on={}, needs_admin={}",
        item.label, item.default_on, item.needs_admin
    );
    (item.analyze)(&ctx)
}

fn print_found(found: &NativeFound) {
    println!(
        "  count={} files={} bytes={} skipped={} items={}",
        found.count,
        found.files,
        found.bytes,
        found.skipped,
        found.items.len()
    );
    for line in &found.detail {
        println!("  detail: {line}");
    }
}

/// Una cartella nota di Windows (`SHGetKnownFolderPath`, token del processo).
fn known_folder(id: &GUID) -> Result<PathBuf, String> {
    let mut raw: windows_sys::core::PWSTR = null_mut();
    // SAFETY: GUID valido, token nullo, `raw` riceve una stringa di
    // CoTaskMemAlloc liberata sotto su ogni percorso.
    let hr = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY as u32, null_mut(), &mut raw) };
    let out = if hr >= 0 && !raw.is_null() {
        // SAFETY: stringa terminata da zero restituita da Windows.
        let n = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
        // SAFETY: `n` u16 leggibili da `raw`.
        let w = unsafe { std::slice::from_raw_parts(raw, n) };
        Ok(PathBuf::from(OsString::from_wide(w)))
    } else {
        Err(format!("HRESULT 0x{:08X}", hr as u32))
    };
    // SAFETY: memoria di SHGetKnownFolderPath (o nulla), liberata una volta.
    unsafe { CoTaskMemFree(raw as *const c_void) };
    out
}

fn common_dirs() -> CommonDirs {
    CommonDirs {
        start_menu: known_folder(&FOLDERID_CommonStartMenu).ok(),
        public_desktop: known_folder(&FOLDERID_PublicDesktop).ok(),
    }
}

// ---------------------------------------------------------------------------
// Lettura indipendente dei collegamenti (COM `IShellLinkW`, `STGM_READ`).

const IID_ISHELLLINKW: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSISTFILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

#[repr(C)]
struct IUnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IShellLinkWVtbl {
    unknown: IUnknownVtbl,
    get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> HRESULT,
}

#[repr(C)]
struct IPersistFileVtbl {
    unknown: IUnknownVtbl,
    get_class_id: usize,
    is_dirty: usize,
    load: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
}

struct Com(*mut c_void);

impl Com {
    /// # Safety
    /// `T` deve essere un prefisso `#[repr(C)]` della vtable di `self.0`.
    unsafe fn vtbl<T>(&self) -> &T {
        // SAFETY: un oggetto COM inizia con il puntatore alla sua vtable.
        unsafe { &**(self.0 as *const *const T) }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: interfaccia valida con un riferimento nostro.
            unsafe { (self.vtbl::<IUnknownVtbl>().release)(self.0) };
        }
    }
}

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
            // SAFETY: chiude il CoInitializeEx riuscito di `enter`.
            unsafe { CoUninitialize() };
        }
    }
}

/// Destinazioni di un `.lnk` (`GetPath` normale e `SLGP_RAWPATH`).
fn shell_targets(lnk: &Path) -> Result<Vec<OsString>, String> {
    let _apt = Apartment::enter().ok_or("CoInitializeEx failed")?;
    let mut raw: *mut c_void = null_mut();
    // SAFETY: CLSID e IID validi; `raw` riceve un IShellLinkW (rilasciato da `Com`).
    let hr = unsafe { CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELLLINKW, &mut raw) };
    if hr < 0 || raw.is_null() {
        return Err(format!("CoCreateInstance 0x{:08X}", hr as u32));
    }
    let link = Com(raw);
    let mut raw: *mut c_void = null_mut();
    // SAFETY: `link` vivo; `raw` riceve un IPersistFile (rilasciato da `Com`).
    let hr = unsafe { (link.vtbl::<IUnknownVtbl>().query_interface)(link.0, &IID_IPERSISTFILE, &mut raw) };
    if hr < 0 || raw.is_null() {
        return Err(format!("QueryInterface 0x{:08X}", hr as u32));
    }
    let file = Com(raw);
    let path = wide_path(lnk);
    // SAFETY: IPersistFile vivo, percorso terminato da zero, sola lettura.
    let hr = unsafe { (file.vtbl::<IPersistFileVtbl>().load)(file.0, path.as_ptr(), STGM_READ) };
    if hr < 0 {
        return Err(format!("Load 0x{:08X}", hr as u32));
    }
    let mut out = Vec::new();
    for flag in [0u32, SLGP_RAWPATH as u32] {
        let mut buf = vec![0u16; 32_768];
        // SAFETY: `buf` scrivibile per la lunghezza passata; `pfd` nullo ammesso.
        let hr = unsafe {
            (link.vtbl::<IShellLinkWVtbl>().get_path)(link.0, buf.as_mut_ptr(), buf.len() as i32, null_mut(), flag)
        };
        if hr != S_OK {
            continue;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = OsString::from_wide(&buf[..n]);
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    Ok(out)
}

fn header_flags(lnk: &Path) -> Option<u32> {
    use std::io::Read;
    let mut header = [0u8; 0x4C];
    std::fs::File::open(lnk).ok()?.read_exact(&mut header).ok()?;
    sysclean::link_flags(&header)
}

fn drive_root(p: &Path) -> Option<String> {
    match p.components().next()? {
        Component::Prefix(pre) => match pre.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(format!("{}:\\", (d as char).to_ascii_uppercase())),
            _ => None,
        },
        _ => None,
    }
}

fn drive_is_fixed(p: &Path) -> bool {
    let Some(root) = drive_root(p) else { return false };
    let w = wide(&root);
    // SAFETY: radice `X:\` terminata da zero.
    unsafe { GetDriveTypeW(w.as_ptr()) == DRIVE_FIXED }
}

fn is_reparse(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_type().is_symlink() || md.file_attributes() & 0x400 != 0
}

/// Esito della discesa dalla radice, come la fa `truly_missing` ma con il motivo.
#[derive(Debug)]
enum Walk {
    Exists,
    Missing,
    Reparse(PathBuf),
    Error(Option<i32>),
}

fn walk(p: &Path) -> Walk {
    let chain: Vec<&Path> = p.ancestors().collect();
    for a in chain.iter().rev() {
        if a.parent().is_none() {
            continue;
        }
        match std::fs::symlink_metadata(a) {
            Ok(md) if is_reparse(&md) => return Walk::Reparse(a.to_path_buf()),
            Ok(_) => {}
            Err(e) => {
                return match e.raw_os_error() {
                    Some(2 | 3) => Walk::Missing,
                    other => Walk::Error(other),
                }
            }
        }
    }
    Walk::Exists
}

/// `%Var%` espanse con l'ambiente delle regole; `None` se una manca.
fn expand(s: &str, env: &Env) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let j = after.find('%')?;
        out.push_str(env.get(&after[..j])?);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Gemelle WOW64 grossolane (sostituzione di testo senza maiuscole), solo
/// per spiegare un «mancante» non proposto.
fn twins(p: &str) -> Vec<String> {
    let low = p.to_lowercase();
    let mut out = Vec::new();
    for (a, b) in [
        (r"\program files (x86)\", r"\program files\"),
        (r"\program files\", r"\program files (x86)\"),
        (r"\system32\", r"\syswow64\"),
        (r"\syswow64\", r"\system32\"),
    ] {
        if let Some(i) = low.find(a) {
            out.push(format!("{}{}{}", &p[..i], b, &p[i + a.len()..]));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Protetto,
    Illeggibile,
    MsiPubblicizzato,
    SoloIdList,
    SenzaPercorso,
    NonGiudicabile,
    ReteONonFisso,
    ComunePerUtente,
    Valida,
    Reparse,
    Errore,
    Mancante,
}

struct Classified {
    kind: Kind,
    flags: Option<u32>,
    targets: Vec<String>,
    note: String,
}

fn classify(lnk: &Path, common: bool, env: &Env, guard: &Guard) -> Classified {
    let mut c = Classified { kind: Kind::Illeggibile, flags: None, targets: Vec::new(), note: String::new() };
    if guard.protected(lnk).is_some() || lnk.to_str().is_none() {
        c.kind = Kind::Protetto;
        return c;
    }
    let Some(flags) = header_flags(lnk) else {
        c.note = "header not readable".into();
        return c;
    };
    c.flags = Some(flags);
    if flags & SLDF_HAS_DARWINID != 0 {
        c.kind = Kind::MsiPubblicizzato;
        return c;
    }
    if flags & SLDF_HAS_LINK_INFO == 0 {
        c.kind = Kind::SoloIdList;
        return c;
    }
    let raw = match shell_targets(lnk) {
        Ok(t) => t,
        Err(e) => {
            c.note = e;
            return c;
        }
    };
    if raw.is_empty() {
        c.kind = Kind::SenzaPercorso;
        return c;
    }
    let users = env.get("userprofile").and_then(|p| Path::new(p).parent().map(Path::to_path_buf));
    let mut expanded: Vec<PathBuf> = Vec::new();
    for t in &raw {
        let Some(s) = t.to_str() else {
            c.kind = Kind::NonGiudicabile;
            c.note = "target is not Unicode".into();
            return c;
        };
        c.targets.push(s.to_string());
        let s = s.trim();
        if s.encode_utf16().count() >= MAX_TARGET_LEN {
            c.kind = Kind::NonGiudicabile;
            c.note = "long target".into();
            return c;
        }
        let low = s.to_lowercase();
        if common
            && ["%userprofile%", "%appdata%", "%localappdata%", "%temp%", "%tmp%", "%onedrive%", "%homepath%", "%username%"]
                .iter()
                .any(|v| low.contains(v))
        {
            c.kind = Kind::ComunePerUtente;
            c.note = "per-user variable in a common folder".into();
            return c;
        }
        let Some(e) = expand(s, env) else {
            c.kind = Kind::NonGiudicabile;
            c.note = "unknown variable".into();
            return c;
        };
        let e = e.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(e);
        let p = PathBuf::from(&e);
        if drive_root(&p).is_none() || !p.is_absolute() || !drive_is_fixed(&p) {
            c.kind = Kind::ReteONonFisso;
            return c;
        }
        let under = |dir: &Path| {
            let (pl, dl) = (p.to_string_lossy().to_lowercase(), dir.to_string_lossy().to_lowercase());
            pl.starts_with(&format!("{}\\", dl.trim_end_matches('\\')))
        };
        let in_other_profile = users.as_ref().is_some_and(|u| under(u) && !under(&u.join("Public")));
        if common && in_other_profile {
            c.kind = Kind::ComunePerUtente;
            c.note = "target in a user profile, in a common folder".into();
            return c;
        }
        expanded.push(p);
    }
    let mut all_missing = true;
    for p in &expanded {
        match walk(p) {
            Walk::Exists => {
                c.kind = Kind::Valida;
                return c;
            }
            Walk::Missing => {}
            Walk::Reparse(at) => {
                all_missing = false;
                c.kind = Kind::Reparse;
                c.note = format!("reparse point in {}", at.display());
            }
            Walk::Error(code) => {
                all_missing = false;
                if c.kind != Kind::Reparse {
                    c.kind = Kind::Errore;
                    c.note = format!("error {code:?}");
                }
            }
        }
    }
    if all_missing {
        c.kind = Kind::Mancante;
        let found_twin: Vec<String> = expanded
            .iter()
            .flat_map(|p| twins(&p.to_string_lossy()))
            .filter(|t| std::fs::symlink_metadata(t).is_ok())
            .collect();
        if !found_twin.is_empty() {
            c.note = format!("WOW64 twin present: {}", found_twin.join(" | "));
        }
    }
    c
}

// ---------------------------------------------------------------------------
// N4: collegamenti non validi.

#[test]
#[ignore = "live, read only: cargo test live_ -- --ignored --nocapture"]
fn live_broken_shortcuts_analysis() {
    let found = analyze_native("broken_shortcuts");
    print_found(&found);
    for lnk in &found.items {
        println!("PROPOSTO\t{lnk}");
    }

    // Verifica indipendente: stesse cartelle, classificazione di ogni `.lnk`.
    let env = Env::for_rules();
    let guard = Guard::new(&env);
    let (dirs, notes) = sysclean::shortcut_dirs(&env, &common_dirs());
    for n in notes {
        println!("folders note: {n}");
    }
    let mut counts: std::collections::BTreeMap<Kind, u64> = Default::default();
    let mut missing: Vec<(String, Classified)> = Vec::new();
    let mut total = 0u64;
    for d in &dirs {
        let lnks = sysclean::list_lnk(&d.path, d.recursive);
        println!(
            "folder {} (recursive={}, common={}): {} .lnk",
            d.path.display(),
            d.recursive,
            d.common,
            lnks.len()
        );
        for lnk in lnks {
            total += 1;
            let c = classify(&lnk, d.common, &env, &guard);
            *counts.entry(c.kind).or_default() += 1;
            let text = lnk.to_string_lossy().into_owned();
            match c.kind {
                Kind::Valida => {}
                Kind::Mancante => {
                    println!("MANCANTE\t{text}\t{}\t{}", c.targets.join(" | "), c.note);
                    missing.push((text, c));
                }
                k => println!(
                    "SALTATO {k:?}\t{text}\t{}\tflag={:?}\t{}",
                    c.targets.join(" | "),
                    c.flags.map(|f| format!("0x{f:x}")),
                    c.note
                ),
            }
        }
    }
    println!("total .lnk: {total}");
    for (k, n) in &counts {
        println!("  {k:?}: {n}");
    }

    let proposed: Vec<String> = found.items.iter().map(|s| s.to_lowercase()).collect();
    let mine: Vec<String> = missing
        .iter()
        .filter(|(_, c)| c.note.is_empty())
        .map(|(t, _)| t.to_lowercase())
        .collect();
    for (t, c) in &missing {
        if !proposed.contains(&t.to_lowercase()) {
            println!("missing but not proposed (expected with a WOW64 twin): {t} [{}]", c.note);
        }
    }
    let unexpected: Vec<&String> = found.items.iter().filter(|p| !mine.contains(&p.to_lowercase())).collect();
    for p in &unexpected {
        println!("DEFECT: proposed but not missing for the independent check: {p}");
    }
    assert!(unexpected.is_empty(), "shortcuts proposed without confirmation: {unexpected:?}");
}

// ---------------------------------------------------------------------------
// N3: registri eventi.

struct Evt(EVT_HANDLE);

impl Drop for Evt {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: handle aperto da una funzione Evt* e non ancora chiuso.
            unsafe { EvtClose(self.0) };
        }
    }
}

/// Tipo del canale, eventi e dimensione del file (sola lettura).
fn channel_info(name: &str) -> String {
    let w = wide(name);
    let mut out = Vec::new();
    // SAFETY: sessione locale, nome terminato da zero; chiuso da `Evt`.
    let cfg = Evt(unsafe { EvtOpenChannelConfig(0, w.as_ptr(), 0) });
    if cfg.0 == 0 {
        out.push(format!("config not opened: {}", last_error()));
    } else {
        let mut v = EVT_VARIANT::default();
        let mut used = 0u32;
        // SAFETY: `v` intero, dimensione sua; la proprietà è un UInt32.
        let ok = unsafe {
            EvtGetChannelConfigProperty(cfg.0, EvtChannelConfigType, 0, size_of::<EVT_VARIANT>() as u32, &mut v, &mut used)
        };
        if ok != 0 && v.Type == EvtVarTypeUInt32 as u32 {
            // SAFETY: il tipo dice UInt32.
            out.push(format!("type={}", unsafe { v.Anonymous.UInt32Val }));
        } else {
            out.push(format!("type not read: {}", last_error()));
        }
    }
    // SAFETY: come sopra.
    let log = Evt(unsafe { EvtOpenLog(0, w.as_ptr(), EvtOpenChannelPath) });
    if log.0 == 0 {
        out.push(format!("registry not opened: {}", last_error()));
        return out.join(", ");
    }
    for (label, prop) in [("eventi", EvtLogNumberOfLogRecords), ("byte", EvtLogFileSize)] {
        let mut v = EVT_VARIANT::default();
        let mut used = 0u32;
        // SAFETY: `v` intero, dimensione sua; le due proprietà sono UInt64.
        let ok = unsafe { EvtGetLogInfo(log.0, prop, size_of::<EVT_VARIANT>() as u32, &mut v, &mut used) };
        if ok != 0 && v.Type == EvtVarTypeUInt64 as u32 {
            // SAFETY: il tipo dice UInt64.
            out.push(format!("{label}={}", unsafe { v.Anonymous.UInt64Val }));
        } else {
            out.push(format!("{label} not read: {}", last_error()));
        }
    }
    out.join(", ")
}

#[test]
#[ignore = "live, read only: cargo test live_ -- --ignored --nocapture"]
fn live_event_channels_analysis() {
    let found = analyze_native("event_logs");
    print_found(&found);
    for c in &found.items {
        println!("CANALE\t{c}");
    }
    for name in ["Application", "System", "Setup", "Security"] {
        println!("direct read {name}: {}", channel_info(name));
    }
    let bad: Vec<&String> = found.items.iter().filter(|c| !sysclean::is_clearable_channel(c)).collect();
    assert!(bad.is_empty(), "channels not allowed were proposed: {bad:?}");
    assert_eq!(found.count as usize, found.items.len());
}

// ---------------------------------------------------------------------------
// N1 e N2: Appunti e DNS (nessuna azione).

#[test]
#[ignore = "live, read only: cargo test live_ -- --ignored --nocapture"]
fn live_clipboard_and_dns_probe() {
    // SAFETY: nessun argomento; non apre gli Appunti.
    let formats = unsafe { CountClipboardFormats() };
    println!("CountClipboardFormats = {formats}");
    let found = analyze_native("clipboard");
    print_found(&found);
    assert_eq!(found.count, u64::from(formats > 0), "the clipboard analysis does not follow CountClipboardFormats");

    let found = analyze_native("dns_cache");
    print_found(&found);
    let name = wide("dnsapi.dll");
    // SAFETY: nome terminato da zero, solo System32, nessun file; liberato sotto.
    let module = unsafe { LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    assert!(!module.is_null(), "dnsapi.dll not loaded: {}", last_error());
    // SAFETY: modulo valido, nome ANSI terminato da zero. Il puntatore NON si chiama.
    let proc = unsafe { GetProcAddress(module, c"DnsFlushResolverCache".as_ptr().cast()) };
    println!("DnsFlushResolverCache resolved (not called): {}", proc.map(|f| format!("{:p}", f as *const ())).unwrap_or("missing".into()));
    // SAFETY: libera il modulo caricato sopra.
    unsafe { FreeLibrary(module) };
    let mut buf = [0u16; 520];
    // SAFETY: `buf` scrivibile per la lunghezza passata.
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let sys = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    let ipconfig = sys.join("ipconfig.exe");
    println!("fallback {} present: {}", ipconfig.display(), ipconfig.is_file());
    assert!(proc.is_some() || ipconfig.is_file(), "neither DnsFlushResolverCache nor ipconfig.exe");
}

// ---------------------------------------------------------------------------
// Cartelle note dei collegamenti.

#[test]
#[ignore = "live, read only: cargo test live_ -- --ignored --nocapture"]
fn live_known_folders() {
    for (name, id) in [
        ("StartMenu (utente)", &FOLDERID_StartMenu),
        ("Desktop (utente)", &FOLDERID_Desktop),
        ("CommonStartMenu", &FOLDERID_CommonStartMenu),
        ("PublicDesktop", &FOLDERID_PublicDesktop),
    ] {
        match known_folder(id) {
            Ok(p) => println!("{name}: {} (exists: {})", p.display(), p.is_dir()),
            Err(e) => println!("{name}: not resolved ({e})"),
        }
    }
    let env = Env::for_rules();
    println!("Env::for_rules appdata = {:?}", env.get("appdata"));
    println!("Env::for_rules desktop = {:?}", env.get("desktop"));
    println!("Env::for_rules userprofile = {:?}", env.get("userprofile"));
    let common = common_dirs();
    let (dirs, notes) = sysclean::shortcut_dirs(&env, &common);
    for d in &dirs {
        println!("folder used by N4: {} (recursive={}, common={})", d.path.display(), d.recursive, d.common);
    }
    for n in &notes {
        println!("nota: {n}");
    }
    let user_start = known_folder(&FOLDERID_StartMenu).ok();
    let user_desktop = known_folder(&FOLDERID_Desktop).ok();
    let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    for (label, kf, recursive, is_common) in [
        ("User Start menu", user_start, true, false),
        ("User Desktop", user_desktop, false, false),
        ("Common Start menu", common.start_menu.clone(), true, true),
        ("Public Desktop", common.public_desktop.clone(), false, true),
    ] {
        let used = dirs.iter().find(|d| d.recursive == recursive && d.common == is_common);
        match (kf, used) {
            (Some(k), Some(u)) if same(&k, &u.path) => println!("OK {label}: known folder = folder used"),
            (k, u) => println!("DIFFERENT {label}: known {k:?}, used {:?}", u.map(|d| &d.path)),
        }
    }
    assert_eq!(dirs.len(), 4, "N4 does not use all four folders: {notes:?}");
}

// ---------------------------------------------------------------------------
// Task 1: opzioni di windows_extra e safari, anteprima vera.

fn reg_text(item: &RegItem) -> String {
    let root = |r: &RegRoot| if matches!(r, RegRoot::Hkcu) { "HKCU" } else { "HKLM" };
    match item {
        RegItem::Key { root: r, path } => format!("{}\\{path}", root(r)),
        RegItem::Value { root: r, path, value } => format!("{}\\{path} [{value}]", root(r)),
    }
}

/// Sottochiavi e valori di una chiave di HKCU (sola lettura), `None` se manca.
fn hkcu_key_info(path: &str) -> Option<(usize, usize)> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let k = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(path, KEY_READ).ok()?;
    Some((k.enum_keys().count(), k.enum_values().count()))
}

#[test]
#[ignore = "live, read only: cargo test live_ -- --ignored --nocapture"]
fn live_windows_extra_registry_analysis() {
    let env = Env::for_rules();
    let (all, warnings) = xiox_rules::embedded(&env);
    for w in &warnings {
        println!("embedded warning: {w}");
    }
    let cancel = AtomicBool::new(false);
    let tick = |_: u64| {};
    let ctl = CollectCtl { cancel: &cancel, tick: &tick };
    let mut found_reg: Vec<RegItem> = Vec::new();
    for id in ["windows_extra", "safari"] {
        let cleaner = all.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("cleaner {id} missing"));
        let present = xiox_rules::present(cleaner, &env, &crate::rules::reg_exists);
        println!("== {id} “{}” (group {}, present on this PC: {present})", cleaner.label, cleaner.group);
        for opt in &cleaner.options {
            let f = crate::rules::analyze_option(opt, &env, &crate::rules::reg_exists, &ctl);
            println!(
                "OPTION {id}/{}: files={} bytes={} folders={} registry={} skipped={} unsupported={:?}",
                opt.id,
                f.files.len(),
                f.bytes,
                f.dirs_to_remove.len(),
                f.reg.len(),
                f.skipped,
                opt.unsupported
            );
            for (p, size) in f.files.iter().take(5) {
                println!("    file: {} ({size} byte)", p.display());
            }
            if f.files.len() > 5 {
                println!("    … and {} more file", f.files.len() - 5);
            }
            for r in &f.reg {
                println!("    registry: {}", reg_text(r));
            }
            for why in &f.skipped_reasons {
                println!("    skipped: {why}");
            }
            found_reg.extend(f.reg);
        }
    }

    // Controllo incrociato: le chiavi che esistono devono essere nell'anteprima.
    let mut missed = Vec::new();
    for path in [
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{5E6AB780-7743-11CF-A12B-00AA004AE837}\Count",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{75048700-EF1F-11D0-9888-006097DEACF9}\Count",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\Count",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{F4E57C4B-2036-45F0-A9AB-443BCFE33D9F}\Count",
        r"Software\Microsoft\Internet Explorer\TypedURLs",
        r"Software\Microsoft\Internet Explorer\TypedURLsTime",
    ] {
        let info = hkcu_key_info(path);
        let in_preview = found_reg.iter().any(
            |r| matches!(r, RegItem::Key { root: RegRoot::Hkcu, path: p } if p.eq_ignore_ascii_case(path)),
        );
        println!(
            "CONTROLLO HKCU\\{path}: {} — nell'anteprima: {in_preview}",
            match info {
                Some((k, v)) => format!("exists ({k} subkeys, {v} values)"),
                None => "does not exist".to_string(),
            }
        );
        if info.is_some() && !in_preview {
            missed.push(path);
        }
    }
    assert!(missed.is_empty(), "existing keys not found by the preview: {missed:?}");
}
