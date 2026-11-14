//! Database dei browser (proposte N9-N12 di `docs/rules-coverage.md`):
//! cookie con la lista da conservare, compattazione dei database, cronologia
//! di Firefox e Thunderbird, ultimo percorso di download.
//!
//! Regole comuni a tutte le voci:
//! - i profili vengono da [`profiles`]: cartelle note dei browser sotto
//!   `%LocalAppData%` e `%AppData%` dell'`Env` fidato delle regole, mai dietro
//!   una giunzione o un collegamento;
//! - prima di toccare un profilo (anche solo per guardare quali file ha) si
//!   rileggono i processi: se il suo browser è aperto, o se il suo file di
//!   blocco (`lockfile` di Chromium, `parent.lock` di Firefox) è tenuto aperto,
//!   il profilo si salta con «<profilo>: chiudi <exe> per pulire», in analisi
//!   e in pulizia. Le voci hanno `running` vuoto: un browser aperto non ferma
//!   gli altri;
//! - ogni modifica a un database è in `BEGIN IMMEDIATE`, con `ROLLBACK` su
//!   errore; il database si apre normalmente (mai creato se manca) e i suoi
//!   file `-wal`/`-shm` restano a SQLite;
//! - `clean_*` agisce solo sui file di `found.items` che sono file attesi dei
//!   profili, e controlla l'annullamento fra un database e l'altro.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use xiox_rules::Env;
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::native::{NativeCleaned, NativeCtx, NativeFound, NativeItem};

pub const GROUP: &str = "Browser";
pub const COOKIES_ID: &str = "cookies_keep_list";
pub const COMPACT_ID: &str = "compact_databases";
pub const FIREFOX_HISTORY_ID: &str = "firefox_history";
pub const DOWNLOAD_DIR_ID: &str = "last_download_dir";

/// In `NativeFound::items` dei cookie: un dominio della lista letta
/// dall'analisi (`keep:google.com`), così la pulizia non rilegge il file.
pub const KEEP_PREFIX: &str = "keep:";

/// Contenuto della lista creata al primo uso: solo commenti.
pub const KEEP_LIST_TEMPLATE: &str = "\
# Cookies to keep (Xiox)
#
# One domain per line. A domain also keeps its subdomains:
# google.com keeps accounts.google.com, but not notgoogle.com.
# Lines starting with # are ignored: remove the # from the examples
# you want to use, then save the file.
#
# google.com
# microsoft.com
# live.com
";

/// File della lista più grande di così: illeggibile (è un elenco di domini).
const KEEP_LIST_MAX: u64 = 1 << 20;
/// `Preferences` più grande di così non si legge.
const PREFS_MAX: u64 = 64 << 20;
/// Attesa massima di un database bloccato da un altro processo.
const BUSY_TIMEOUT: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Browser e profili.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chromium,
    Gecko,
}

#[derive(Clone, Copy)]
enum Base {
    Local,
    Roaming,
}

#[derive(Clone, Copy)]
enum Layout {
    /// `User Data` con un profilo per sottocartella (`Default`, `Profile 2`).
    UserData,
    /// Opera: la cartella stessa è un profilo, le sue sottocartelle con
    /// `Preferences` anche (versioni recenti), e `_side_profiles\*`.
    Opera,
    /// `Profiles\*` di Firefox e Thunderbird.
    Gecko,
}

/// Una cartella di un browser: `rel` sotto la base, `variant` nel nome del
/// profilo («Chrome Beta, Default»).
struct Root {
    base: Base,
    rel: &'static str,
    variant: Option<&'static str>,
}

const fn local(rel: &'static str, variant: Option<&'static str>) -> Root {
    Root { base: Base::Local, rel, variant }
}

const fn roaming(rel: &'static str) -> Root {
    Root { base: Base::Roaming, rel, variant: None }
}

struct BrowserDef {
    name: &'static str,
    layout: Layout,
    /// Eseguibili del browser (minuscolo, con `.exe`).
    processes: &'static [&'static str],
    roots: &'static [Root],
}

/// I browser della tabella §3.1 e §2.2. Percorsi dalle voci di Winapp2
/// (`<prefisso> Web Browsing Cookies *`); i nomi degli eseguibili dei browser
/// di Gen Digital e di Arc sono da verificare (il file di blocco del profilo
/// copre un nome sbagliato, vedi [`busy`]).
const BROWSERS: &[BrowserDef] = &[
    BrowserDef {
        name: "Google Chrome",
        layout: Layout::UserData,
        processes: &["chrome.exe"],
        roots: &[
            local(r"Google\Chrome\User Data", None),
            local(r"Google\Chrome Beta\User Data", Some("Chrome Beta")),
            local(r"Google\Chrome Dev\User Data", Some("Chrome Dev")),
            local(r"Google\Chrome SxS\User Data", Some("Chrome Canary")),
        ],
    },
    BrowserDef {
        name: "Microsoft Edge",
        layout: Layout::UserData,
        processes: &["msedge.exe"],
        roots: &[
            local(r"Microsoft\Edge\User Data", None),
            local(r"Microsoft\Edge Beta\User Data", Some("Edge Beta")),
            local(r"Microsoft\Edge Dev\User Data", Some("Edge Dev")),
            local(r"Microsoft\Edge SxS\User Data", Some("Edge Canary")),
        ],
    },
    BrowserDef {
        name: "Brave",
        layout: Layout::UserData,
        processes: &["brave.exe"],
        roots: &[
            local(r"BraveSoftware\Brave-Browser\User Data", None),
            local(r"BraveSoftware\Brave-Browser-Beta\User Data", Some("Beta")),
            local(r"BraveSoftware\Brave-Browser-Nightly\User Data", Some("Nightly")),
        ],
    },
    BrowserDef { name: "Vivaldi", layout: Layout::UserData, processes: &["vivaldi.exe"], roots: &[local(r"Vivaldi\User Data", None)] },
    BrowserDef { name: "Opera", layout: Layout::Opera, processes: &["opera.exe"], roots: &[roaming(r"Opera Software\Opera Stable")] },
    BrowserDef { name: "Opera GX", layout: Layout::Opera, processes: &["opera.exe"], roots: &[roaming(r"Opera Software\Opera GX Stable")] },
    BrowserDef {
        name: "Avast Secure Browser",
        layout: Layout::UserData,
        processes: &["avastbrowser.exe"],
        roots: &[local(r"Avast Software\Browser\User Data", None)],
    },
    BrowserDef {
        name: "AVG Secure Browser",
        layout: Layout::UserData,
        processes: &["avgbrowser.exe"],
        roots: &[local(r"AVG\Browser\User Data", None)],
    },
    BrowserDef {
        name: "Avira Secure Browser",
        layout: Layout::UserData,
        processes: &["avirabrowser.exe"],
        roots: &[local(r"Avira\Browser\User Data", None)],
    },
    BrowserDef {
        name: "Norton Secure Browser",
        layout: Layout::UserData,
        processes: &["nortonbrowser.exe"],
        roots: &[local(r"Norton\Browser\User Data", None)],
    },
    BrowserDef {
        name: "Arc",
        layout: Layout::UserData,
        processes: &["arc.exe"],
        roots: &[local(r"Packages\TheBrowserCompany.Arc_ttt1ap7aakyb4\LocalCache\Local\Arc\User Data", None)],
    },
    BrowserDef {
        name: "Mozilla Firefox",
        layout: Layout::Gecko,
        processes: &["firefox.exe"],
        roots: &[
            roaming(r"Mozilla\Firefox\Profiles"),
            local(r"Packages\Mozilla.Firefox_n80bbvh6b1yt2\LocalCache\Roaming\Mozilla\Firefox\Profiles", None),
        ],
    },
    BrowserDef {
        name: "Thunderbird",
        layout: Layout::Gecko,
        processes: &["thunderbird.exe"],
        roots: &[
            roaming(r"Thunderbird\Profiles"),
            local(r"Packages\MozillaThunderbird.MozillaThunderbirdRelease_h5892qc0xkpca\LocalCache\Roaming\Thunderbird\Profiles", None),
            local(r"Packages\MozillaThunderbird.MZLA_h5892qc0xkpca\LocalCache\Roaming\Thunderbird\Profiles", None),
        ],
    },
];

/// Sottocartelle di `User Data` che non sono profili dell'utente.
const NOT_PROFILES: &[&str] = &["System Profile", "Guest Profile"];

/// Un profilo di un browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub browser: &'static str,
    pub kind: Kind,
    pub dir: PathBuf,
    /// Nome fra parentesi nel resoconto: cartella, con la variante del browser.
    pub name: String,
    pub processes: &'static [&'static str],
    /// File che il browser aperto tiene bloccato: `User Data\lockfile`
    /// (Chromium) o `parent.lock` del profilo (Gecko).
    pub lock: PathBuf,
}

impl Profile {
    /// «Google Chrome (Default)».
    pub fn label(&self) -> String {
        format!("{} ({})", self.browser, self.name)
    }
}

/// I profili dei browser della tabella, nell'ordine di [`BROWSERS`], poi per
/// nome della cartella. Una cartella dietro una giunzione o un collegamento
/// (in qualunque punto sotto `%LocalAppData%`/`%AppData%`) non si usa mai.
pub fn profiles(env: &Env) -> Vec<Profile> {
    let mut out = Vec::new();
    for b in BROWSERS {
        for r in b.roots {
            let key = match r.base {
                Base::Local => "localappdata",
                Base::Roaming => "appdata",
            };
            let Some(base) = env.get(key).filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let Some(root) = plain_path(Path::new(base), r.rel, false) else {
                continue;
            };
            let kind = match b.layout {
                Layout::Gecko => Kind::Gecko,
                _ => Kind::Chromium,
            };
            let named = |name: String| match r.variant {
                Some(v) => format!("{v}, {name}"),
                None => name,
            };
            let mut push = |dir: PathBuf, name: String, lock: PathBuf| {
                out.push(Profile { browser: b.name, kind, dir, name: named(name), processes: b.processes, lock });
            };
            let chromium_lock = root.join("lockfile");
            match b.layout {
                Layout::UserData => {
                    for (name, dir) in subdirs(&root) {
                        if is_chromium_profile(&name, &dir) {
                            push(dir, name, chromium_lock.clone());
                        }
                    }
                }
                Layout::Opera => {
                    if plain_file(&root.join("Preferences")) {
                        push(root.clone(), "main profile".to_string(), chromium_lock.clone());
                    }
                    for (name, dir) in subdirs(&root) {
                        if name != "_side_profiles" && is_chromium_profile(&name, &dir) {
                            push(dir, name, chromium_lock.clone());
                        }
                    }
                    let side = root.join("_side_profiles");
                    if plain_dir(&side) {
                        for (name, dir) in subdirs(&side) {
                            if is_chromium_profile(&name, &dir) {
                                push(dir, format!("side profile {name}"), chromium_lock.clone());
                            }
                        }
                    }
                }
                Layout::Gecko => {
                    for (name, dir) in subdirs(&root) {
                        if ["prefs.js", "places.sqlite", "cookies.sqlite"].iter().any(|f| plain_file(&dir.join(f))) {
                            let lock = dir.join("parent.lock");
                            push(dir, name, lock);
                        }
                    }
                }
            }
        }
    }
    out
}

fn is_chromium_profile(name: &str, dir: &Path) -> bool {
    !NOT_PROFILES.iter().any(|n| n.eq_ignore_ascii_case(name)) && plain_file(&dir.join("Preferences"))
}

/// Le sottocartelle vere (non giunzioni né collegamenti) di `dir`, per nome.
fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let p = e.path();
            plain_dir(&p).then_some((name, p))
        })
        .collect();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

fn plain(p: &Path) -> Option<fs::Metadata> {
    use std::os::windows::fs::MetadataExt;
    fs::symlink_metadata(p).ok().filter(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0)
}

fn plain_dir(p: &Path) -> bool {
    plain(p).is_some_and(|m| m.is_dir())
}

fn plain_file(p: &Path) -> bool {
    plain(p).is_some_and(|m| m.is_file())
}

/// `base\rel` se ogni componente di `rel` è una cartella vera (e l'ultimo un
/// file vero se `file`), senza giunzioni né collegamenti lungo la strada.
fn plain_path(base: &Path, rel: &str, file: bool) -> Option<PathBuf> {
    let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
    let mut p = base.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        if *part == "." || *part == ".." {
            return None;
        }
        p.push(part);
        let last = i + 1 == parts.len();
        let ok = if last && file { plain_file(&p) } else { plain_dir(&p) };
        if !ok {
            return None;
        }
    }
    Some(p)
}

type Procs<'a> = &'a dyn Fn() -> Vec<String>;

/// `Some(nota)` se il profilo non si deve aprire adesso: il suo browser è fra
/// i processi (riletti qui) o il suo file di blocco è tenuto aperto. Il file
/// di blocco si guarda solo se il browser non risulta aperto.
fn busy(p: &Profile, procs: Procs) -> Option<String> {
    let hits = crate::rules::running_among(p.processes, &procs());
    if !hits.is_empty() {
        return Some(format!("{}: close {} to clean", p.label(), hits.join(", ")));
    }
    lock_held(&p.lock).then(|| format!("{}: profile in use, close {} to clean", p.label(), p.processes.join(", ")))
}

/// `true` se un altro processo tiene aperto `lock`: Chromium lo apre con la
/// sola condivisione in lettura, Firefox senza condivisione, quindi
/// un'apertura in scrittura (che non scrive niente) fallisce. Un file che non
/// c'è non blocca; un collegamento o un errore qualunque sì (si salta).
fn lock_held(lock: &Path) -> bool {
    match fs::symlink_metadata(lock) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
        Ok(_) if plain(lock).is_none() => return true,
        Ok(_) => {}
    }
    fs::OpenOptions::new().read(true).write(true).open(lock).is_err()
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn in_found(found: &NativeFound, p: &Path) -> bool {
    found.items.contains(&path_text(p))
}

/// Nome di un file del profilo nel resoconto (`History`, `Network\Cookies`).
fn short(p: &Profile, file: &Path) -> String {
    file.strip_prefix(&p.dir).map(path_text).unwrap_or_else(|_| path_text(file))
}

fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    // Mai `SQLITE_OPEN_CREATE`: un database che manca non si crea.
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.busy_timeout(BUSY_TIMEOUT)?;
    Ok(c)
}

/// Apertura dell'analisi: sola lettura (mai `immutable`), così un `-wal` o un
/// `-journal` rimasti non si riportano nel database. Se non si apre così, il
/// database si salta con una nota: mai riaperto in scrittura.
fn open_db_read(path: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.busy_timeout(BUSY_TIMEOUT)?;
    Ok(c)
}

/// `parent.lock` di Firefox tenuto aperto senza condivisione durante la
/// pulizia del profilo: un Firefox che parte adesso dice «già in esecuzione»
/// invece di aprire i database. Creato se manca (vuoto, mai scritto) e tolto
/// alla fine; uno lasciato da Firefox resta.
pub struct ProfileLock {
    file: Option<fs::File>,
    created: Option<PathBuf>,
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        drop(self.file.take());
        if let Some(p) = self.created.take() {
            let _ = fs::remove_file(p);
        }
    }
}

/// `Ok(None)` per Chromium (niente da tenere); `Err` se il blocco di Firefox
/// non si prende (il profilo si salta).
pub fn hold_profile(p: &Profile) -> Result<Option<ProfileLock>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    if p.kind != Kind::Gecko {
        return Ok(None);
    }
    let busy = || format!("{}: profile in use, close {} to clean", p.label(), p.processes.join(", "));
    let existed = match fs::symlink_metadata(&p.lock) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Ok(_) if plain(&p.lock).is_some_and(|m| m.is_file()) => true,
        _ => return Err(busy()),
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(!existed)
        .share_mode(0)
        .open(&p.lock)
        .map_err(|_| busy())?;
    Ok(Some(ProfileLock { file: Some(file), created: (!existed).then(|| p.lock.clone()) }))
}

fn has_column(c: &Connection, table: &str, col: &str) -> bool {
    c.query_row("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2", [table, col], |_| Ok(())).is_ok()
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// Lista dei cookie da conservare.

/// La lista letta: i domini (ASCII, in punycode) e le righe non commentate
/// che non sono un dominio valido, così come scritte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeepList {
    pub domains: Vec<String>,
    pub rejected: Vec<String>,
}

/// Righe scartate mostrate nella conferma, le altre si contano.
const MAX_REJECTED_SHOWN: usize = 5;

/// Domini della lista: una riga per dominio, `#` per i commenti (anche in
/// fondo alla riga), senza maiuscole, senza punto o `*.` iniziale; da un
/// indirizzo (`https://accounts.example.net:443/login`) si tiene l'host. I
/// domini internazionali (`bücher.de`) diventano punycode, come li salvano i
/// browser. Le righe che non danno un dominio valido vanno in `rejected`;
/// niente doppioni.
pub fn parse_keep_list(text: &str) -> KeepList {
    let mut out = KeepList::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        match keep_domain(line) {
            Some(d) => {
                if !out.domains.contains(&d) {
                    out.domains.push(d);
                }
            }
            None => out.rejected.push(line.to_string()),
        }
    }
    out
}

fn keep_domain(line: &str) -> Option<String> {
    let mut d = line.to_lowercase();
    if let Some(i) = d.find("://") {
        d = d[i + 3..].to_string();
    }
    if let Some(i) = d.find(['/', '?']) {
        d.truncate(i);
    }
    if let Some(i) = d.find('@') {
        d = d[i + 1..].to_string();
    }
    if let Some(i) = d.find(':') {
        d.truncate(i);
    }
    let d = d.trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.');
    let d = if d.is_ascii() { d.to_string() } else { idna::domain_to_ascii(d).ok()? };
    let valid = !d.is_empty()
        && d.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    valid.then_some(d)
}

/// `true` se il cookie di `host` (con o senza punto iniziale) appartiene a un
/// dominio della lista o a un suo sottodominio. `notgoogle.com` non è
/// `google.com`.
pub fn is_kept(host: &str, keep: &[String]) -> bool {
    let h = host.trim().trim_start_matches('.').to_ascii_lowercase();
    keep.iter().any(|d| h == *d || (h.len() > d.len() && h.ends_with(d.as_str()) && h.as_bytes()[h.len() - d.len() - 1] == b'.'))
}

/// `<documenti>\Xiox\cookie_da_conservare.txt`. Una cartella `<documenti>\PulisciPC` di una
/// versione precedente si rinomina in `Xiox` (se non si riesce resta in uso quella vecchia:
/// la lista dell'utente non va mai persa, vedi [`xiox_model::legacy::migrate_legacy_dir`]).
pub fn keep_list_in(documents: &Path) -> PathBuf {
    xiox_model::legacy::app_dir_in(documents).join("cookie_da_conservare.txt")
}

const NOT_SAFE: &str = "cookie list not safe (the Xiox folder or the file is a link or a junction)";

/// `Ok` se né `Xiox` né il file della lista sono reparse point
/// (giunzioni, collegamenti, segnaposto di OneDrive): ciò che manca va bene,
/// ciò che c'è deve essere una cartella vera e un file vero.
fn check_keep_list(path: &Path) -> Result<(), String> {
    let ok = |p: &Path, dir: bool| match fs::symlink_metadata(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
        Ok(_) => plain(p).is_some_and(|m| if dir { m.is_dir() } else { m.is_file() }),
    };
    let dir_ok = path.parent().is_none_or(|d| ok(d, true));
    if dir_ok && ok(path, false) { Ok(()) } else { Err(NOT_SAFE.to_string()) }
}

/// Legge la lista. Assente, vuota o con soli commenti: nessun dominio
/// (`Ok`, «lista: 0 domini»). UTF-8 con o senza BOM, UTF-16 con BOM (Blocco
/// note «Unicode»). `Err` (nessun cookie si elimina) se il file non si legge,
/// è un collegamento, non è testo UTF-8 né UTF-16 con BOM (anche UTF-16 senza
/// BOM), o ha righe scritte ma nessun dominio valido.
pub fn read_keep_list(path: &Path) -> Result<KeepList, String> {
    check_keep_list(path)?;
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(KeepList::default()),
        Ok(m) if m.len() > KEEP_LIST_MAX => return Err("file too large".to_string()),
        _ => {}
    }
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(KeepList::default()),
        Err(e) => return Err(e.to_string()),
    };
    let list = parse_keep_list(&decode_text(&bytes)?);
    if list.domains.is_empty() && !list.rejected.is_empty() {
        return Err(format!("no valid domain; unrecognized lines: {}", rejected_text(&list.rejected)));
    }
    Ok(list)
}

/// Le prime [`MAX_REJECTED_SHOWN`] righe scartate, poi «…e altre N».
fn rejected_text(rejected: &[String]) -> String {
    let mut s = rejected.iter().take(MAX_REJECTED_SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if rejected.len() > MAX_REJECTED_SHOWN {
        s.push_str(&format!(" …and {} more", rejected.len() - MAX_REJECTED_SHOWN));
    }
    s
}

/// Testo della lista: UTF-8 (con o senza BOM) o UTF-16 con BOM. Un carattere
/// nullo (UTF-16 senza BOM) o byte non UTF-8 sono un errore.
fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], le: bool| {
        let (pairs, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err("incomplete UTF-16 text".to_string());
        }
        let units: Vec<u16> = pairs.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) }).collect();
        String::from_utf16(&units).map_err(|_| "invalid UTF-16 text".to_string())
    };
    let text = match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true)?,
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false)?,
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).map_err(|_| "text is not UTF-8".to_string())?,
        _ => String::from_utf8(bytes.to_vec()).map_err(|_| "text is not UTF-8: save it as UTF-8 in Notepad".to_string())?,
    };
    if text.contains('\0') {
        return Err("text with null characters (UTF-16 without BOM?): save it as UTF-8 in Notepad".to_string());
    }
    Ok(text)
}

/// Crea la lista con gli esempi commentati se non c'è; mai sovrascritta. Mai
/// attraverso un collegamento o una giunzione.
pub fn ensure_keep_list(path: &Path) -> io::Result<()> {
    check_keep_list(path).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    check_keep_list(path).map_err(io::Error::other)?;
    match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => f.write_all(KEEP_LIST_TEMPLATE.replace('\n', "\r\n").as_bytes()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Testo per la conferma e per il resoconto: «lista: N domini» (con le righe
/// non riconosciute, se ce ne sono), oppure che la lista non vale e nessun
/// cookie verrà eliminato.
pub fn keep_list_summary(list: &Result<KeepList, String>) -> String {
    match list {
        Ok(v) if v.rejected.is_empty() => format!("list: {}", plural(v.domains.len() as u64, "domain", "domains")),
        Ok(v) => format!(
            "list: {}; unrecognized lines, not protected: {}",
            plural(v.domains.len() as u64, "domain", "domains"),
            rejected_text(&v.rejected)
        ),
        Err(e) => format!("Cookie list not valid: no cookie will be deleted ({e})"),
    }
}

/// Crea la lista se manca e la apre con `open` (nell'app: il Blocco note di
/// sistema). Niente se `Xiox` o il file sono collegamenti o giunzioni.
pub fn open_keep_list_in(documents: &Path, open: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
    let path = keep_list_in(documents);
    check_keep_list(&path)?;
    ensure_keep_list(&path).map_err(|e| e.to_string())?;
    check_keep_list(&path)?;
    open(&path)
}

/// `<system32>\notepad.exe` e l'argomento fra virgolette (un percorso di
/// Windows non può contenere `"`).
pub fn editor_command(system32: &Path, path: &Path) -> (PathBuf, String) {
    (system32.join("notepad.exe"), format!("\"{}\"", path.display()))
}

// ---------------------------------------------------------------------------
// Cookie (N9).

/// Database dei cookie del profilo che ci sono davvero, con tabella e colonna
/// dell'host. Chromium: `Network\Cookies` e, nelle versioni vecchie,
/// `Cookies`; Gecko: `cookies.sqlite`.
fn cookie_dbs(p: &Profile) -> Vec<(PathBuf, &'static str, &'static str)> {
    let (names, table, col): (&[&str], _, _) = match p.kind {
        Kind::Chromium => (&[r"Network\Cookies", "Cookies"], "cookies", "host_key"),
        Kind::Gecko => (&["cookies.sqlite"], "moz_cookies", "host"),
    };
    names.iter().filter_map(|n| plain_path(&p.dir, n, true)).map(|f| (f, table, col)).collect()
}

/// Host distinti dei cookie con il numero di righe di ciascuno.
fn cookie_hosts(c: &Connection, table: &str, col: &str) -> Result<Vec<(Option<String>, u64)>, String> {
    if !has_column(c, table, col) {
        return Err(format!("table {table} is missing"));
    }
    let read = || -> rusqlite::Result<Vec<(Option<String>, u64)>> {
        let mut st = c.prepare(&format!("SELECT \"{col}\", COUNT(*) FROM \"{table}\" GROUP BY \"{col}\""))?;
        let rows = st.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?;
        rows.collect()
    };
    read().map_err(|e| e.to_string())
}

/// Analisi dei cookie: in `items` i database con cookie da eliminare, poi i
/// domini della lista (`keep:<dominio>`). Lista illeggibile = niente.
pub fn analyze_cookies(profiles: &[Profile], list: Result<KeepList, String>, procs: Procs, cancel: &AtomicBool) -> NativeFound {
    let summary = keep_list_summary(&list);
    let mut f = NativeFound::default();
    let keep = match list {
        Ok(k) => k.domains,
        Err(_) => {
            f.detail.push(summary);
            return f;
        }
    };
    let mut lines = Vec::new();
    for p in profiles {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            f.skipped += 1;
            lines.push(note);
            continue;
        }
        for (db, table, col) in cookie_dbs(p) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let hosts = open_db_read(&db).map_err(|e| e.to_string()).and_then(|c| cookie_hosts(&c, table, col));
            match hosts {
                Ok(hosts) => {
                    let (mut del, mut kept) = (0, 0);
                    for (h, n) in hosts {
                        if is_kept(h.as_deref().unwrap_or(""), &keep) {
                            kept += n;
                        } else {
                            del += n;
                        }
                    }
                    if del > 0 {
                        f.count += del;
                        f.items.push(path_text(&db));
                    }
                    if del + kept > 0 {
                        lines.push(format!("{} – {}: {} to delete, {kept} to keep", p.label(), short(p, &db), plural(del, "cookie", "cookies")));
                    }
                }
                Err(e) => {
                    f.skipped += 1;
                    lines.push(format!("{} – {}: not readable ({e})", p.label(), short(p, &db)));
                }
            }
        }
    }
    if !f.items.is_empty() {
        f.items.extend(keep.iter().map(|d| format!("{KEEP_PREFIX}{d}")));
    }
    f.detail.push(if f.count > 0 {
        format!("{} to delete ({summary})", plural(f.count, "cookie", "cookies"))
    } else {
        format!("No cookies to delete ({summary})")
    });
    f.detail.extend(lines);
    f
}

/// Elimina i cookie dei database di `found.items` che non sono dei domini
/// `keep:` scritti dall'analisi (il file della lista non si rilegge).
pub fn clean_cookies(profiles: &[Profile], found: &NativeFound, procs: Procs, cancel: &AtomicBool) -> NativeCleaned {
    let keep: Vec<String> = found.items.iter().filter_map(|i| i.strip_prefix(KEEP_PREFIX)).map(str::to_string).collect();
    let mut done = NativeCleaned::default();
    for p in profiles {
        let wanted = |db: &Path| in_found(found, db);
        if !expected_cookie_paths(p).iter().any(|db| wanted(db)) {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            done.note(note);
            continue;
        }
        // Firefox: `parent.lock` preso per tutta la pulizia del profilo.
        let _lock = match hold_profile(p) {
            Ok(l) => l,
            Err(note) => {
                done.note(note);
                continue;
            }
        };
        for (db, table, col) in cookie_dbs(p).into_iter().filter(|(db, ..)| wanted(db)) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let before = file_size(&db);
            match delete_cookies(&db, table, col, &keep) {
                Ok((n, inside)) => {
                    done.removed += n;
                    done.freed += before.saturating_sub(file_size(&db));
                    done.done.push(format!(
                        "{} – {}: {} deleted, {} freed in the database",
                        p.label(),
                        short(p, &db),
                        plural(n, "cookie", "cookies"),
                        crate::sys::fmt_size(inside)
                    ));
                }
                Err(e) => {
                    done.failed += 1;
                    done.note(format!("{} – {}: cookies not deleted, nothing changed ({e})", p.label(), short(p, &db)));
                }
            }
        }
    }
    done
}

/// I percorsi dei database dei cookie del profilo, senza guardare il disco.
fn expected_cookie_paths(p: &Profile) -> Vec<PathBuf> {
    match p.kind {
        Kind::Chromium => vec![p.dir.join(r"Network\Cookies"), p.dir.join("Cookies")],
        Kind::Gecko => vec![p.dir.join("cookies.sqlite")],
    }
}

/// Dimensione del file su disco (0 se non si legge).
fn file_size(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Byte diventati liberi dentro il database dopo `free_before` (le righe
/// tolte liberano pagine, il file si accorcia solo compattandolo).
fn freed_inside(c: &Connection, free_before: u64) -> u64 {
    free_bytes(c).map(|b| b.saturating_sub(free_before)).unwrap_or(0)
}

/// Una transazione `BEGIN IMMEDIATE`: un errore a metà la annulla (`ROLLBACK`
/// alla chiusura senza `commit`). Restituisce le righe tolte e i byte
/// liberati dentro il database.
fn delete_cookies(db: &Path, table: &str, col: &str, keep: &[String]) -> Result<(u64, u64), String> {
    let mut c = open_db(db).map_err(|e| e.to_string())?;
    let free_before = free_bytes(&c).map_err(|e| e.to_string())?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let hosts = cookie_hosts(&tx, table, col)?;
    let run = || -> rusqlite::Result<u64> {
        let mut n = 0u64;
        let mut del = tx.prepare(&format!("DELETE FROM \"{table}\" WHERE \"{col}\" IS ?1"))?;
        for (h, _) in &hosts {
            if !is_kept(h.as_deref().unwrap_or(""), keep) {
                n += del.execute([h])? as u64;
            }
        }
        Ok(n)
    };
    let n = run().map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok((n, freed_inside(&c, free_before)))
}

// ---------------------------------------------------------------------------
// Compattazione (N10).

const CHROMIUM_DBS: &[&str] =
    &["History", "Favicons", "Top Sites", "Shortcuts", "Web Data", "Network Action Predictor", r"Network\Cookies", "Cookies"];
const GECKO_DBS: &[&str] =
    &["places.sqlite", "cookies.sqlite", "formhistory.sqlite", "favicons.sqlite", "permissions.sqlite", "content-prefs.sqlite"];

fn compact_names(p: &Profile) -> &'static [&'static str] {
    match p.kind {
        Kind::Chromium => CHROMIUM_DBS,
        Kind::Gecko => GECKO_DBS,
    }
}

/// Byte recuperabili: `freelist_count × page_size`.
fn free_bytes(c: &Connection) -> rusqlite::Result<u64> {
    let free: i64 = c.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let page: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    Ok((free.max(0) as u64).saturating_mul(page.max(0) as u64))
}

/// Database dei profili con pagine libere (`freelist_count > 0`).
pub fn analyze_compact(profiles: &[Profile], procs: Procs, cancel: &AtomicBool) -> NativeFound {
    let mut f = NativeFound::default();
    let mut lines = Vec::new();
    for p in profiles {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            f.skipped += 1;
            lines.push(note);
            continue;
        }
        for db in compact_names(p).iter().filter_map(|n| plain_path(&p.dir, n, true)) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            match open_db_read(&db).and_then(|c| free_bytes(&c)) {
                Ok(0) => {}
                Ok(b) => {
                    f.count += 1;
                    f.bytes += b;
                    f.items.push(path_text(&db));
                    lines.push(format!("{} – {}: {} recoverable", p.label(), short(p, &db), crate::sys::fmt_size(b)));
                }
                Err(e) => {
                    f.skipped += 1;
                    lines.push(format!("{} – {}: not readable ({e})", p.label(), short(p, &db)));
                }
            }
        }
    }
    f.detail.push(if f.count > 0 {
        format!("{}, {} recoverable", plural(f.count, "database", "databases"), crate::sys::fmt_size(f.bytes))
    } else {
        "No database to compact".to_string()
    });
    f.detail.extend(lines);
    f
}

/// `VACUUM` dei database di `found.items`, ciascuno solo se
/// `PRAGMA integrity_check` dà `ok`; uno rovinato si lascia com'è.
pub fn clean_compact(profiles: &[Profile], found: &NativeFound, procs: Procs, cancel: &AtomicBool) -> NativeCleaned {
    clean_compact_with(profiles, found, procs, cancel, &win::free_space, &std::env::temp_dir())
}

/// [`clean_compact`] con lo spazio libero (`free`, byte disponibili sul
/// volume di un percorso, `None` se non si sa) e la cartella temporanea di
/// SQLite (`temp`) passati da fuori. `VACUUM` scrive una copia del database
/// in `temp` e, in WAL, fa crescere il `-wal`: serve almeno il doppio della
/// dimensione del file su tutti e due i volumi, altrimenti si salta.
pub fn clean_compact_with(
    profiles: &[Profile],
    found: &NativeFound,
    procs: Procs,
    cancel: &AtomicBool,
    free: &dyn Fn(&Path) -> Option<u64>,
    temp: &Path,
) -> NativeCleaned {
    let mut done = NativeCleaned::default();
    for p in profiles {
        let mine: Vec<PathBuf> = compact_names(p).iter().map(|n| p.dir.join(n)).filter(|db| in_found(found, db)).collect();
        if mine.is_empty() {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            done.note(note);
            continue;
        }
        // Firefox: `parent.lock` preso per tutta la pulizia del profilo.
        let _lock = match hold_profile(p) {
            Ok(l) => l,
            Err(note) => {
                done.note(note);
                continue;
            }
        };
        for db in mine {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let name = short(p, &db);
            if !plain_file(&db) {
                done.note(format!("{} – {name}: gone", p.label()));
                continue;
            }
            let need = file_size(&db).saturating_mul(2);
            let room = |dir: &Path| free(dir).is_some_and(|b| b >= need);
            if !db.parent().is_some_and(room) || !room(temp) {
                done.note(format!(
                    "{} – {name}: not enough space to compact ({} free needed here and in the temporary folder)",
                    p.label(),
                    crate::sys::fmt_size(need)
                ));
                continue;
            }
            match compact(&db) {
                Ok((before, after)) => {
                    done.removed += 1;
                    done.freed += before.saturating_sub(after);
                    done.done.push(format!(
                        "{} – {name}: {} → {}",
                        p.label(),
                        crate::sys::fmt_size(before),
                        crate::sys::fmt_size(after)
                    ));
                }
                Err(CompactError::NotOk(why)) => {
                    done.note(format!("{} – {name}: the integrity check does not return “ok” ({why}): left alone", p.label()));
                }
                Err(CompactError::Db(e)) => {
                    done.failed += 1;
                    done.note(format!("{} – {name}: not compacted ({e})", p.label()));
                }
            }
        }
    }
    done
}

enum CompactError {
    NotOk(String),
    Db(String),
}

/// Dimensione del file prima e dopo il `VACUUM` (dopo la chiusura, quando
/// SQLite ha già riportato il `-wal` nel database).
fn compact(db: &Path) -> Result<(u64, u64), CompactError> {
    let size = || fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    let before = size();
    let c = open_db(db).map_err(|e| CompactError::Db(e.to_string()))?;
    let check: rusqlite::Result<String> = c.query_row("PRAGMA integrity_check", [], |r| r.get(0));
    match check {
        Ok(s) if s == "ok" => {}
        Ok(s) => return Err(CompactError::NotOk(s)),
        Err(e) => return Err(CompactError::NotOk(e.to_string())),
    }
    c.execute_batch("VACUUM").map_err(|e| CompactError::Db(e.to_string()))?;
    c.close().map_err(|(_, e)| CompactError::Db(e.to_string()))?;
    Ok((before, size()))
}

// ---------------------------------------------------------------------------
// Cronologia di Firefox e Thunderbird (N11).

/// Tabelle e colonne senza le quali `places.sqlite` non si tocca.
const PLACES_REQUIRED: &[(&str, &str)] = &[
    ("moz_places", "id"),
    ("moz_places", "foreign_count"),
    ("moz_places", "visit_count"),
    ("moz_places", "last_visit_date"),
    ("moz_historyvisits", "place_id"),
    ("moz_bookmarks", "fk"),
];

fn places_ok(c: &Connection) -> bool {
    PLACES_REQUIRED.iter().all(|(t, col)| has_column(c, t, col))
}

/// Pagine che si possono togliere: nessun riferimento (`foreign_count = 0`),
/// e comunque mai una pagina di un segnalibro o di una parola chiave anche
/// se `foreign_count` fosse sbagliato.
fn removable_places(c: &Connection) -> String {
    let mut w = "foreign_count = 0 AND id NOT IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL)".to_string();
    if has_column(c, "moz_keywords", "place_id") {
        w.push_str(" AND id NOT IN (SELECT place_id FROM moz_keywords WHERE place_id IS NOT NULL)");
    }
    w
}

fn count(c: &Connection, sql: &str) -> rusqlite::Result<u64> {
    c.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n.max(0) as u64)
}

/// (visite, pagine) che la pulizia toglierebbe.
fn history_counts(c: &Connection) -> rusqlite::Result<(u64, u64)> {
    let visits = count(c, "SELECT COUNT(*) FROM moz_historyvisits")?;
    let pages = count(c, &format!("SELECT COUNT(*) FROM moz_places WHERE {}", removable_places(c)))?;
    Ok((visits, pages))
}

/// I profili Gecko: visite e pagine senza segnalibri da togliere.
pub fn analyze_history(profiles: &[Profile], procs: Procs, cancel: &AtomicBool) -> NativeFound {
    let mut f = NativeFound::default();
    let mut lines = Vec::new();
    for p in profiles.iter().filter(|p| p.kind == Kind::Gecko) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            f.skipped += 1;
            lines.push(note);
            continue;
        }
        let Some(db) = plain_path(&p.dir, "places.sqlite", true) else {
            continue;
        };
        let counts = open_db_read(&db).and_then(|c| if places_ok(&c) { history_counts(&c).map(Some) } else { Ok(None) });
        match counts {
            Ok(Some((visits, pages))) if visits + pages > 0 => {
                f.count += visits + pages;
                f.items.push(path_text(&db));
                lines.push(format!(
                    "{}: {} and {} without bookmarks",
                    p.label(),
                    plural(visits, "visit", "visits"),
                    plural(pages, "page", "pages")
                ));
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                f.skipped += 1;
                lines.push(format!("{}: places.sqlite not recognized, left alone", p.label()));
            }
            Err(e) => {
                f.skipped += 1;
                lines.push(format!("{}: places.sqlite not readable ({e})", p.label()));
            }
        }
    }
    f.detail.push(if f.count > 0 {
        format!("{} to remove, bookmarks excluded", plural(f.count, "item", "items"))
    } else {
        "No history to remove".to_string()
    });
    f.detail.extend(lines);
    f
}

/// Toglie la cronologia dei `places.sqlite` di `found.items`; `ROLLBACK` se i
/// segnalibri (righe di `moz_bookmarks` e pagine a cui puntano) cambiano.
pub fn clean_history(profiles: &[Profile], found: &NativeFound, procs: Procs, cancel: &AtomicBool) -> NativeCleaned {
    let mut done = NativeCleaned::default();
    for p in profiles.iter().filter(|p| p.kind == Kind::Gecko) {
        let db = p.dir.join("places.sqlite");
        if !in_found(found, &db) {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            done.note(note);
            continue;
        }
        // Firefox: `parent.lock` preso per tutta la pulizia del profilo.
        let _lock = match hold_profile(p) {
            Ok(l) => l,
            Err(note) => {
                done.note(note);
                continue;
            }
        };
        if plain_path(&p.dir, "places.sqlite", true).is_none() {
            done.note(format!("{}: places.sqlite is gone", p.label()));
            continue;
        }
        let before = file_size(&db);
        match delete_history(&db) {
            Ok(HistoryOutcome::Done(visits, pages, inside)) => {
                done.removed += visits + pages;
                done.freed += before.saturating_sub(file_size(&db));
                done.done.push(format!(
                    "{}: {} and {} removed, bookmarks intact, {} freed in the database",
                    p.label(),
                    plural(visits, "visit", "visits"),
                    plural(pages, "page", "pages"),
                    crate::sys::fmt_size(inside)
                ));
            }
            Ok(HistoryOutcome::BookmarksChanged) => {
                done.failed += 1;
                done.note(format!(
                    "{}: history not removed, because the bookmarks would have changed (nothing changed)",
                    p.label()
                ));
            }
            Ok(HistoryOutcome::Unknown) => {
                done.note(format!("{}: places.sqlite not recognized, left alone", p.label()));
            }
            Err(e) => {
                done.failed += 1;
                done.note(format!("{}: history not removed, nothing changed ({e})", p.label()));
            }
        }
    }
    done
}

enum HistoryOutcome {
    /// Visite, pagine, byte liberati dentro il database.
    Done(u64, u64, u64),
    BookmarksChanged,
    Unknown,
}

/// Le righe di `moz_bookmarks` (tutte le colonne) e quanti segnalibri e
/// parole chiave puntano a una pagina che non c'è.
fn bookmark_state(c: &Connection) -> rusqlite::Result<(Vec<Vec<Value>>, u64, u64)> {
    let mut st = c.prepare("SELECT * FROM moz_bookmarks ORDER BY id")?;
    let n = st.column_count();
    let rows = st
        .query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect::<rusqlite::Result<Vec<Value>>>())?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let lost = count(c, "SELECT COUNT(*) FROM moz_bookmarks WHERE fk IS NOT NULL AND fk NOT IN (SELECT id FROM moz_places)")?;
    let lost_kw = if has_column(c, "moz_keywords", "place_id") {
        count(c, "SELECT COUNT(*) FROM moz_keywords WHERE place_id IS NOT NULL AND place_id NOT IN (SELECT id FROM moz_places)")?
    } else {
        0
    };
    Ok((rows, lost, lost_kw))
}

/// Righe di `table` il cui `col` punta a una riga che non c'è più in
/// `parent` (solo se tabella e colonna esistono).
fn delete_orphans(c: &Connection, table: &str, col: &str, parent: &str, parent_col: &str) -> rusqlite::Result<()> {
    if has_column(c, table, col) {
        c.execute(
            &format!("DELETE FROM \"{table}\" WHERE \"{col}\" NOT IN (SELECT \"{parent_col}\" FROM \"{parent}\" WHERE \"{parent_col}\" IS NOT NULL)"),
            [],
        )?;
    }
    Ok(())
}

fn delete_history(db: &Path) -> rusqlite::Result<HistoryOutcome> {
    let mut c = open_db(db)?;
    let free_before = free_bytes(&c)?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !places_ok(&tx) {
        return Ok(HistoryOutcome::Unknown);
    }
    let before = bookmark_state(&tx)?;
    let removable = removable_places(&tx);
    let visits = tx.execute("DELETE FROM moz_historyvisits", [])? as u64;
    let pages = tx.execute(&format!("DELETE FROM moz_places WHERE {removable}"), [])? as u64;
    // Ciò che dipende dalle visite e dalle pagine tolte.
    delete_orphans(&tx, "moz_historyvisits_extra", "visit_id", "moz_historyvisits", "id")?;
    delete_orphans(&tx, "moz_annos", "place_id", "moz_places", "id")?;
    if has_column(&tx, "moz_annos", "anno_attribute_id") && has_column(&tx, "moz_anno_attributes", "name") {
        // Annotazioni dei download (cronologia download).
        tx.execute(
            "DELETE FROM moz_annos WHERE anno_attribute_id IN (SELECT id FROM moz_anno_attributes WHERE name LIKE 'downloads/%')",
            [],
        )?;
    }
    delete_orphans(&tx, "moz_inputhistory", "place_id", "moz_places", "id")?;
    delete_orphans(&tx, "moz_places_metadata", "place_id", "moz_places", "id")?;
    delete_orphans(&tx, "moz_places_extra", "place_id", "moz_places", "id")?;
    if has_column(&tx, "moz_places_metadata", "search_query_id") {
        delete_orphans(&tx, "moz_places_metadata_search_queries", "id", "moz_places_metadata", "search_query_id")?;
    }
    if has_column(&tx, "moz_places", "origin_id") {
        delete_orphans(&tx, "moz_origins", "id", "moz_places", "origin_id")?;
    }
    // Le pagine dei segnalibri restano, senza il conto delle visite.
    let typed = if has_column(&tx, "moz_places", "typed") { ", typed = 0" } else { "" };
    tx.execute(&format!("UPDATE moz_places SET visit_count = 0, last_visit_date = NULL{typed}"), [])?;
    if bookmark_state(&tx)? != before {
        tx.rollback()?;
        return Ok(HistoryOutcome::BookmarksChanged);
    }
    tx.commit()?;
    Ok(HistoryOutcome::Done(visits, pages, freed_inside(&c, free_before)))
}

// ---------------------------------------------------------------------------
// Ultimo percorso di download (N12).

/// Chiave di `Preferences` con l'ultima cartella di «Salva con nome»
/// (oggetto, chiave). `download.default_directory` è la cartella scelta
/// nelle impostazioni, non l'ultimo percorso: non si tocca.
const CHROMIUM_LAST_DIR: (&str, &str) = ("savefile", "default_directory");
const GECKO_LAST_DIR: &[u8] = b"\"browser.download.lastDir\"";

/// `Preferences` senza l'ultimo percorso: `Ok(None)` se la chiave non c'è
/// («niente da fare»), `Err` se il file non è un oggetto JSON (non si
/// tocca). Il risultato si rilegge come JSON uguale all'originale meno la
/// chiave, con l'ordine delle chiavi e i numeri di prima.
pub fn strip_chromium_prefs(bytes: &[u8]) -> Result<Option<(Vec<u8>, usize)>, String> {
    let mut v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| "not valid JSON".to_string())?;
    let root = v.as_object_mut().ok_or_else(|| "not a JSON object".to_string())?;
    let (obj, key) = CHROMIUM_LAST_DIR;
    let removed = root.get_mut(obj).and_then(|o| o.as_object_mut()).and_then(|o| o.shift_remove(key)).is_some();
    if !removed {
        return Ok(None);
    }
    let out = serde_json::to_vec(&v).map_err(|e| e.to_string())?;
    match serde_json::from_slice::<serde_json::Value>(&out) {
        Ok(back) if back == v => Ok(Some((out, 1))),
        _ => Err("the rewritten file does not read back the same".to_string()),
    }
}

/// `prefs.js` senza le righe `user_pref("browser.download.lastDir", …);`
/// (le altre righe, anche `browser.download.lastDir.savePerSite`, restano
/// byte per byte). `None` se non ce ne sono.
pub fn strip_prefs_js(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut n = 0;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if is_last_dir_line(line) {
            n += 1;
        } else {
            out.extend_from_slice(line);
        }
    }
    (n > 0).then_some((out, n))
}

fn is_last_dir_line(line: &[u8]) -> bool {
    let rest = line.trim_ascii_start();
    let Some(rest) = rest.strip_prefix(b"user_pref(") else {
        return false;
    };
    let Some(rest) = rest.trim_ascii_start().strip_prefix(GECKO_LAST_DIR) else {
        return false;
    };
    rest.trim_ascii_start().starts_with(b",")
}

/// File delle preferenze del profilo (senza guardare il disco).
fn prefs_file(p: &Profile) -> PathBuf {
    p.dir.join(match p.kind {
        Kind::Chromium => "Preferences",
        Kind::Gecko => "prefs.js",
    })
}

/// Il file delle preferenze letto e ripulito: `Ok(None)` = niente da fare.
fn stripped_prefs(p: &Profile, file: &Path) -> Result<Option<(Vec<u8>, usize)>, String> {
    let m = fs::metadata(file).map_err(|e| e.to_string())?;
    if m.len() > PREFS_MAX {
        return Err("file too large".to_string());
    }
    let bytes = fs::read(file).map_err(|e| e.to_string())?;
    match p.kind {
        Kind::Chromium => strip_chromium_prefs(&bytes),
        Kind::Gecko => Ok(strip_prefs_js(&bytes)),
    }
}

pub fn analyze_download_dir(profiles: &[Profile], procs: Procs, cancel: &AtomicBool) -> NativeFound {
    let mut f = NativeFound::default();
    let mut lines = Vec::new();
    for p in profiles {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            f.skipped += 1;
            lines.push(note);
            continue;
        }
        let name = match p.kind {
            Kind::Chromium => "Preferences",
            Kind::Gecko => "prefs.js",
        };
        let Some(file) = plain_path(&p.dir, name, true) else {
            continue;
        };
        match stripped_prefs(p, &file) {
            Ok(Some((_, n))) => {
                f.count += n as u64;
                f.items.push(path_text(&file));
                lines.push(format!("{}: last folder in {name}", p.label()));
            }
            Ok(None) => {}
            Err(e) => {
                f.skipped += 1;
                lines.push(format!("{}: {name} {e}, left alone", p.label()));
            }
        }
    }
    f.detail.push(if f.count > 0 {
        format!("Last download path in {}", plural(f.items.len() as u64, "profile", "profiles"))
    } else {
        "Nothing to do: no last path saved".to_string()
    });
    f.detail.extend(lines);
    f
}

pub fn clean_download_dir(profiles: &[Profile], found: &NativeFound, procs: Procs, cancel: &AtomicBool) -> NativeCleaned {
    let mut done = NativeCleaned::default();
    for p in profiles {
        let file = prefs_file(p);
        if !in_found(found, &file) {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(note) = busy(p, procs) {
            done.note(note);
            continue;
        }
        // Firefox: `parent.lock` preso per tutta la pulizia del profilo.
        let _lock = match hold_profile(p) {
            Ok(l) => l,
            Err(note) => {
                done.note(note);
                continue;
            }
        };
        let name = short(p, &file);
        if !plain_file(&file) {
            done.note(format!("{}: {name} is gone", p.label()));
            continue;
        }
        let res = stripped_prefs(p, &file).and_then(|s| match s {
            None => Ok(0),
            Some((out, n)) => {
                let kind = p.kind;
                let expected = out.clone();
                write_replacing(&file, &out, &move |back: &[u8]| match kind {
                    Kind::Chromium => {
                        let a = serde_json::from_slice::<serde_json::Value>(back);
                        let b = serde_json::from_slice::<serde_json::Value>(&expected);
                        matches!((a, b), (Ok(a), Ok(b)) if a == b)
                    }
                    Kind::Gecko => back == expected.as_slice(),
                })
                .map(|()| n)
                .map_err(|e| e.to_string())
            }
        });
        match res {
            Ok(0) => {}
            Ok(n) => {
                done.removed += n as u64;
                done.done.push(format!("{}: last path removed from {name}", p.label()));
            }
            Err(e) => {
                done.failed += 1;
                done.note(format!("{}: {name} not modified ({e})", p.label()));
            }
        }
    }
    done
}

/// Scrive `bytes` in un file temporaneo con un nome nuovo (pid e
/// nanosecondi) nella stessa cartella, lo rilegge, e solo se `check` lo
/// approva lo mette al posto di `path` con `ReplaceFileW`, che tiene ACL e
/// attributi dell'originale. In errore il temporaneo (creato qui) si toglie e
/// `path` resta com'era. Un temporaneo vecchio non dà fastidio.
fn write_replacing(path: &Path, bytes: &[u8], check: &dyn Fn(&[u8]) -> bool) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| io::Error::other("missing name"))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".xiox.{}-{nanos}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let res = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if !check(&fs::read(&tmp)?) {
            return Err(io::Error::other("the rewritten file does not read back the same"));
        }
        win::replace(path, &tmp)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

// ---------------------------------------------------------------------------
// Voci.

pub fn items() -> Vec<NativeItem> {
    vec![
        NativeItem {
            id: COOKIES_ID,
            group: GROUP,
            label: "Cookies (with a keep list)",
            description: "Deletes the cookies of Chrome, Edge, Firefox and the other browsers, except those of the domains \
                 written in Documents\\Xiox\\cookie_da_conservare.txt (one domain per line, subdomains included). \
                 The profile of an open browser is skipped.",
            warning: Some(
                "you are signed out of sites, and in Thunderbird of web mail too: all cookies will be deleted \
                 except the domains on the list",
            ),
            warning_extra: Some(real::keep_list_warning),
            default_on: false,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::cookies_analyze,
            clean: real::cookies_clean,
        },
        NativeItem {
            id: COMPACT_ID,
            group: GROUP,
            label: "Compact databases",
            description: "Compacts the browser profile databases (history, cookies, icons, forms) to \
                 recover the free space inside them. No data is deleted; a database that does not \
                 pass the integrity check is left alone. The profile of an open browser is skipped.",
            warning: Some(
                "databases are rewritten in full: you need as much free space as the largest database, and \
                 a browser left open mid-work would find it locked. Close the browsers first.",
            ),
            warning_extra: None,
            default_on: false,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::compact_analyze,
            clean: real::compact_clean,
        },
        NativeItem {
            id: FIREFOX_HISTORY_ID,
            group: GROUP,
            label: "Firefox and Thunderbird history",
            description: "Removes the browsing and download history of Firefox and Thunderbird from \
                 places.sqlite. Bookmarks and keywords stay: if they would change, the cleanup is cancelled. \
                 The profile of an open browser is skipped.",
            warning: Some("the browsing and download history of Firefox and Thunderbird is lost; bookmarks stay."),
            warning_extra: None,
            default_on: false,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::history_analyze,
            clean: real::history_clean,
        },
        NativeItem {
            id: DOWNLOAD_DIR_ID,
            group: GROUP,
            label: "Last download path",
            description: "Makes the browser forget the last folder used: the one of “Save as” for Chrome, Edge and \
                 the other browsers (Preferences file), the download one for Firefox and Thunderbird (prefs.js). \
                 The download folder chosen in the settings does not change.",
            warning: Some(
                "the browser preferences file (Preferences or prefs.js) is rewritten, only with the browser closed \
                 and only if the result reads back without errors.",
            ),
            warning_extra: None,
            default_on: false,
            needs_admin: false,
            running: &[],
            needs_preview: false,
            analyze: real::download_dir_analyze,
            clean: real::download_dir_clean,
        },
    ]
}

/// La lista vera: `Documenti\Xiox\cookie_da_conservare.txt`, dalla
/// cartella nota Documenti (mai dalle variabili del processo).
fn keep_list_path() -> Option<PathBuf> {
    crate::sysclean::documents_dir().map(|d| keep_list_in(&d))
}

/// Crea la lista se manca e la apre con il Blocco note di sistema
/// (`<GetSystemDirectoryW>\notepad.exe "<lista>"`): nessuna associazione di
/// file di HKCU, nessun `ShellExecuteW` da amministratore. Da chiamare fuori
/// dal thread della UI.
pub fn open_keep_list() -> Result<(), String> {
    let docs = crate::sysclean::documents_dir().ok_or_else(|| "Documents folder not found".to_string())?;
    let system32 = crate::sysclean::system32_dir().ok_or_else(|| "system folder not found".to_string())?;
    open_keep_list_in(&docs, &|path: &Path| {
        use std::os::windows::process::CommandExt;
        let (exe, arg) = editor_command(&system32, path);
        std::process::Command::new(exe).raw_arg(arg).spawn().map(drop).map_err(|e| format!("Notepad not started ({e})"))
    })
}

mod real {
    use super::*;

    fn keep_list() -> Result<KeepList, String> {
        keep_list_path().ok_or_else(|| "Documents folder not found".to_string()).and_then(|p| read_keep_list(&p))
    }

    pub fn keep_list_warning() -> String {
        keep_list_summary(&keep_list())
    }
    pub fn cookies_analyze(ctx: &NativeCtx) -> NativeFound {
        analyze_cookies(&profiles(ctx.env), keep_list(), &crate::rules::processes, ctx.cancel)
    }
    pub fn cookies_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_cookies(&profiles(ctx.env), found, &crate::rules::processes, ctx.cancel)
    }
    pub fn compact_analyze(ctx: &NativeCtx) -> NativeFound {
        analyze_compact(&profiles(ctx.env), &crate::rules::processes, ctx.cancel)
    }
    pub fn compact_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_compact(&profiles(ctx.env), found, &crate::rules::processes, ctx.cancel)
    }
    pub fn history_analyze(ctx: &NativeCtx) -> NativeFound {
        analyze_history(&profiles(ctx.env), &crate::rules::processes, ctx.cancel)
    }
    pub fn history_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_history(&profiles(ctx.env), found, &crate::rules::processes, ctx.cancel)
    }
    pub fn download_dir_analyze(ctx: &NativeCtx) -> NativeFound {
        analyze_download_dir(&profiles(ctx.env), &crate::rules::processes, ctx.cancel)
    }
    pub fn download_dir_clean(ctx: &NativeCtx, found: &NativeFound) -> NativeCleaned {
        clean_download_dir(&profiles(ctx.env), found, &crate::rules::processes, ctx.cancel)
    }
}

mod win {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Byte disponibili per questo utente sul volume di `dir`.
    pub fn free_space(dir: &Path) -> Option<u64> {
        let w = wide(dir);
        let mut avail = 0u64;
        // SAFETY: percorso terminato da zero; solo il primo puntatore di uscita, gli altri nulli.
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, null_mut(), null_mut()) };
        (ok != 0).then_some(avail)
    }

    /// Mette `replacement` al posto di `target` (`ReplaceFileW`, che tiene
    /// ACL, attributi e flussi dell'originale), senza file di backup.
    pub fn replace(target: &Path, replacement: &Path) -> io::Result<()> {
        let (t, r) = (wide(target), wide(replacement));
        // SAFETY: due percorsi terminati da zero che vivono fino al ritorno;
        // nessun file di backup, `lpexclude` e `lpreserved` nulli come richiesto.
        let ok = unsafe { ReplaceFileW(t.as_ptr(), r.as_ptr(), null(), REPLACEFILE_WRITE_THROUGH, null(), null()) };
        if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

#[cfg(test)]
mod tests;

