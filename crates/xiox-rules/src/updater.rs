//! Scaricamento e aggiornamento di `Winapp2.ini` (mai dentro l'exe).
//!
//! Il file vive in una cartella che solo gli amministratori possono
//! modificare, [`data_dir`] = `%ProgramData%\PulisciPC\winapp2\`, insieme a
//! `etag.txt`: le regole decidono cosa cancella un'app che gira sempre da
//! amministratore, quindi un utente senza privilegi non deve poterle
//! scrivere. Prima di ogni uso [`ensure_admin_dir`] rende fidate, dalla più
//! esterna, `PulisciPC`, `winapp2` e `tmp` (vedi [`trust_chain`]): una
//! cartella creata prima da un utente qualunque, un collegamento al suo posto
//! o ACE estranee la fanno spostare da parte ([`quarantine_dir`], senza
//! seguire collegamenti) e ricreare nuova; poi [`secure_dir`] rimette l'ACL
//! (SYSTEM e Administrators controllo completo, Users sola lettura, niente
//! ereditarietà) e la verifica. [`read_valid`] rifiuta una copia in una
//! catena non fidata, e controlla proprietario e DACL sullo stesso handle da
//! cui legge; il file temporaneo si crea con `create_new`; i file di curl si
//! leggono solo da una cartella di lavoro fidata, ognuno da un solo handle
//! controllato.
//!
//! A ogni avvio si chiede il file con `If-None-Match`: 304 = niente da fare;
//! 200 valido sostituisce la copia in modo atomico (scrittura in `.tmp` +
//! rename, ritentato contro gli antivirus); qualunque altra cosa (rete
//! assente, HTML di errore, file troncato, cartella non protetta) lascia la
//! copia vecchia dov'è. Il download usa il `curl.exe` di Windows, senza
//! finestra e senza passare da una shell, con i file di lavoro nella
//! sottocartella `tmp` della cartella protetta (mai in `%TEMP%`).
//!
//! La vecchia copia in `%APPDATA%\PulisciPC\winapp2\` (prima versione) non si
//! legge mai; si toglie, al meglio, quando quella nuova è a posto.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{KnownFolder, known_folder};

pub const WINAPP2_URL: &str =
    "https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini";

/// Tempo massimo di una richiesta (curl `--max-time` e guardiano esterno).
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Margine concesso a curl oltre `--max-time` prima di ucciderlo.
const WATCHDOG_GRACE: Duration = Duration::from_secs(3);
/// Tempo massimo di `icacls.exe`.
const ICACLS_TIMEOUT: Duration = Duration::from_secs(30);

const MIN_BYTES: usize = 200 * 1024;
const MAX_BYTES: usize = 20 * 1024 * 1024;
const MIN_SECTIONS: usize = 1000;

const INI_NAME: &str = "Winapp2.ini";
const TMP_NAME: &str = "Winapp2.ini.tmp";
const ETAG_NAME: &str = "etag.txt";
/// Sottocartella di lavoro di curl dentro la cartella protetta.
pub const WORK_NAME: &str = "tmp";

/// Tentativi di `rename` (un antivirus può tenere aperto il file appena scritto).
const RENAME_TRIES: u32 = 3;
const RENAME_BACKOFF: Duration = Duration::from_millis(200);

/// Motivo con cui [`read_valid`] rifiuta una copia non protetta.
pub const UNPROTECTED_COPY: &str =
    "cartella delle regole modificabile da utenti non amministratori: copia ignorata";

/// Risposta di una richiesta condizionale.
pub enum Fetched {
    NotModified,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// Sorgente di rete sostituibile nei test.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str, etag: Option<&str>, timeout: Duration) -> Result<Fetched, String>;
}

/// Protezione della cartella delle regole, sostituibile nei test (che non
/// toccano ACL vere).
pub trait Protect: Send + Sync {
    /// Crea `dir` se manca e la rende modificabile solo dagli amministratori
    /// (poi lo verifica).
    fn secure(&self, dir: &Path) -> Result<(), String>;
    /// `Ok` se `path` (cartella o file) è modificabile solo dagli amministratori.
    fn check(&self, path: &Path) -> Result<(), String>;
    /// Come `check`, sul file aperto (lo stesso handle da cui si legge).
    fn check_file(&self, file: &fs::File) -> Result<(), String>;
    /// Toglie di mezzo una cartella non fidata senza seguirne i collegamenti
    /// (vedi [`quarantine_dir`]).
    fn quarantine(&self, dir: &Path) -> Result<(), String>;
}

/// La protezione vera: [`secure_dir`], [`check_admin_only`],
/// [`check_admin_only_file`], [`quarantine_dir`].
pub struct AdminOnly;

impl Protect for AdminOnly {
    fn secure(&self, dir: &Path) -> Result<(), String> {
        secure_dir(dir)
    }
    fn check(&self, path: &Path) -> Result<(), String> {
        check_admin_only(path)
    }
    fn check_file(&self, file: &fs::File) -> Result<(), String> {
        check_admin_only_file(file)
    }
    fn quarantine(&self, dir: &Path) -> Result<(), String> {
        quarantine_dir(dir)
    }
}

/// Cartelle da fidare, dalla più esterna: `PulisciPC` e `winapp2` per
/// [`data_dir`] (e per ogni `…\PulisciPC\<dir>`), altrimenti solo `dir`.
pub fn trust_chain(dir: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    if let Some(parent) = dir.parent()
        && parent.file_name().is_some_and(|n| n.eq_ignore_ascii_case("PulisciPC"))
    {
        chain.push(parent.to_path_buf());
    }
    chain.push(dir.to_path_buf());
    chain
}

/// Rende `dir` una cartella fidata: se esiste ma è un collegamento, non è una
/// cartella o non passa `check` (creata prima da un utente qualunque, ACE
/// estranee che `/grant:r` non toglie), la si toglie di mezzo con
/// `quarantine` e la si ricrea nuova; poi `secure`. Il genitore deve essere
/// già fidato (vedi [`trust_chain`]).
pub fn ensure_admin_dir(dir: &Path, protect: &dyn Protect) -> Result<(), String> {
    for _ in 0..3 {
        match fs::symlink_metadata(dir) {
            Ok(md) => {
                if is_link_md(&md) || !md.is_dir() || protect.check(dir).is_err() {
                    protect.quarantine(dir)?;
                    continue;
                }
                return protect.secure(dir);
            }
            Err(_) => match fs::create_dir(dir) {
                Ok(()) => return protect.secure(dir),
                // Ricreata da altri nel frattempo: si ricontrolla.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cartella non creabile: {e}")),
            },
        }
    }
    Err(format!("{}: ricreata da altri più volte", dir.display()))
}

/// [`ensure_admin_dir`] su ogni cartella di [`trust_chain`], dalla più esterna.
fn ensure_chain(dir: &Path, protect: &dyn Protect) -> Result<(), String> {
    trust_chain(dir).iter().try_for_each(|d| ensure_admin_dir(d, protect))
}

/// Segno nel nome di una cartella non fidata messa da parte.
const QUARANTINE_MARK: &str = ".non-fidata-";

/// Sposta `dir` (un collegamento si sposta come collegamento) accanto a sé con
/// un nome nuovo, poi prova a cancellarla senza mai seguire collegamenti; le
/// cartelle messe da parte prima si riprovano. Spostarla basta: il nome
/// originale è libero e quella messa da parte non si legge mai. Il genitore
/// deve essere fidato (gli amministratori possono spostarne i figli qualunque
/// sia il loro ACL).
pub fn quarantine_dir(dir: &Path) -> Result<(), String> {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return Err("cartella senza genitore".into());
    };
    let name = name.to_string_lossy().into_owned();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let aside = parent.join(format!(
        "{name}{QUARANTINE_MARK}{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::rename(dir, &aside).map_err(|e| format!("cartella non fidata non spostabile: {e}"))?;
    if let Ok(rd) = fs::read_dir(parent) {
        let prefix = format!("{name}{QUARANTINE_MARK}").to_lowercase();
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().to_lowercase().starts_with(&prefix) {
                remove_tree_no_follow(&e.path());
            }
        }
    }
    Ok(())
}

/// Collegamento o giunzione (punto di analisi).
fn is_link_md(md: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        md.file_type().is_symlink() || md.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        md.file_type().is_symlink()
    }
}

/// Cancella `p` e ciò che contiene senza seguire collegamenti (un collegamento
/// si toglie come collegamento). Al meglio: `true` se non resta nulla.
fn remove_tree_no_follow(p: &Path) -> bool {
    let Ok(md) = fs::symlink_metadata(p) else {
        return true;
    };
    if is_link_md(&md) {
        #[cfg(windows)]
        let as_dir = {
            use std::os::windows::fs::MetadataExt;
            md.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let as_dir = false;
        let gone = if as_dir { fs::remove_dir(p) } else { fs::remove_file(p) };
        return gone.is_ok();
    }
    if md.is_dir() {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                remove_tree_no_follow(&e.path());
            }
        }
        return fs::remove_dir(p).is_ok();
    }
    let mut perm = md.permissions();
    if perm.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = fs::set_permissions(p, perm);
    }
    fs::remove_file(p).is_ok()
}

/// Legge al più `max` byte da `path` attraverso un solo handle: aperto senza
/// seguire un collegamento finale e senza condividere la scrittura (nessuno lo
/// cambia mentre si legge), deve essere un file normale e passare
/// `check_file` su quello stesso handle.
fn read_trusted(path: &Path, protect: &dyn Protect, max: u64) -> Result<Vec<u8>, ReadError> {
    use std::io::Read;
    let mut opts = fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0x1) // FILE_SHARE_READ
            .custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = opts.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReadError::Missing
        } else {
            ReadError::Other(format!("{} non si apre: {e}", path.display()))
        }
    })?;
    let md = file
        .metadata()
        .map_err(|e| ReadError::Other(format!("{}: {e}", path.display())))?;
    if is_link_md(&md) || !md.is_file() {
        return Err(ReadError::Other(format!("{} non è un file normale", path.display())));
    }
    if md.len() > max {
        return Err(ReadError::Other(format!("{} troppo grande ({} byte)", path.display(), md.len())));
    }
    protect.check_file(&file).map_err(|_| ReadError::Untrusted)?;
    let mut bytes = Vec::new();
    (&file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ReadError::Other(format!("{} illeggibile: {e}", path.display())))?;
    if bytes.len() as u64 > max {
        return Err(ReadError::Other(format!("{} troppo grande", path.display())));
    }
    Ok(bytes)
}

#[derive(Debug)]
enum ReadError {
    Missing,
    Untrusted,
    Other(String),
}

/// Scrive un file nuovo: fallisce se esiste già (nessun file piazzato prima,
/// né un collegamento, viene riusato).
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Esito di un giro di aggiornamento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// File nuovo scritto; `version` è il numero di `; Version:`.
    Updated { version: String },
    /// 304: la copia locale è già l'ultima.
    UpToDate,
    /// Scaricamento fallito o file non valido; resta la copia precedente.
    KeptOld(String),
    /// Scaricamento fallito e nessuna copia locale.
    NoCopyYet(String),
}

/// Cartella delle regole: `%ProgramData%\PulisciPC\winapp2` (ProgramData da
/// `SHGetKnownFolderPath`, non dalla variabile d'ambiente).
pub fn data_dir() -> PathBuf {
    program_data().join("PulisciPC").join("winapp2")
}

/// Cartella della prima versione, `%APPDATA%\PulisciPC\winapp2`: non si legge
/// mai; vedi [`remove_legacy_copy`].
pub fn legacy_dir() -> Option<PathBuf> {
    known_folder(KnownFolder::RoamingAppData).map(|p| PathBuf::from(p).join("PulisciPC").join("winapp2"))
}

fn program_data() -> PathBuf {
    known_folder(KnownFolder::ProgramData).map(PathBuf::from).unwrap_or_else(|| {
        let drive = system_directory()
            .and_then(|d| d.to_string_lossy().get(..2).map(str::to_string))
            .unwrap_or_else(|| "C:".to_string());
        PathBuf::from(format!(r"{drive}\ProgramData"))
    })
}

/// Toglie, al meglio, la vecchia copia (`Winapp2.ini`, `.tmp`, `etag.txt`) e
/// la cartella se resta vuota. Si cancella con
/// [`crate::safedelete::delete_checked`]: la cartella è dell'utente e
/// potrebbe essere diventata una giunzione verso altro.
pub fn remove_legacy_copy(old: &Path) {
    #[cfg(windows)]
    {
        let env = crate::model::Env::default();
        for name in [INI_NAME, TMP_NAME, ETAG_NAME] {
            let p = old.join(name);
            if fs::symlink_metadata(&p).is_ok() {
                let _ = crate::safedelete::delete_checked(&p, &env);
            }
        }
        if fs::symlink_metadata(old).is_ok() {
            let _ = crate::safedelete::delete_checked(old, &env);
        }
    }
    #[cfg(not(windows))]
    let _ = old;
}

/// Controlla che `bytes` sia davvero un Winapp2.ini intero. `Ok(versione)`.
pub fn is_valid(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("file vuoto".into());
    }
    if bytes.len() < MIN_BYTES {
        return Err(format!("file troppo piccolo ({} byte)", bytes.len()));
    }
    if bytes.len() > MAX_BYTES {
        return Err(format!("file troppo grande ({} byte)", bytes.len()));
    }
    if bytes.last() != Some(&b'\n') {
        return Err("file troncato (non finisce con un ritorno a capo)".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "il file non è UTF-8".to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut version: Option<String> = None;
    let mut declared: Option<usize> = None;
    let mut sections = 0usize;
    let mut has_filekey = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            sections += 1;
        } else if line.starts_with(';') {
            let rest = line.trim_start_matches(';').trim();
            if version.is_none()
                && let Some(v) = rest.strip_prefix("Version:")
            {
                let v = v.trim();
                if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
                    version = Some(v.to_string());
                }
            } else if declared.is_none()
                && let Some(n) = rest.strip_prefix("# of entries:")
            {
                let digits: String = n.chars().filter(|c| *c != ',' && !c.is_whitespace()).collect();
                declared = digits.parse().ok();
            }
        } else if !has_filekey
            && line.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("FileKey"))
        {
            has_filekey = true;
        }
    }
    let version = version.ok_or_else(|| "riga '; Version:' assente".to_string())?;
    if sections < MIN_SECTIONS {
        return Err(format!("solo {sections} sezioni"));
    }
    if let Some(d) = declared
        && sections < d
    {
        return Err(format!("{sections} sezioni su {d} dichiarate"));
    }
    if !has_filekey {
        return Err("nessuna FileKey".into());
    }
    Ok(version)
}

/// Legge la copia di `dir` per l'app: `Ok((testo, versione))` solo se la
/// cartella e il file sono modificabili solo dagli amministratori e il file
/// passa [`is_valid`]. Una copia in una cartella non protetta si rifiuta con
/// [`UNPROTECTED_COPY`].
pub fn read_valid(dir: &Path) -> Result<(String, String), String> {
    read_valid_with(dir, &AdminOnly)
}

/// Come [`read_valid`], con la protezione scelta da chi chiama.
pub fn read_valid_with(dir: &Path, protect: &dyn Protect) -> Result<(String, String), String> {
    let ini = dir.join(INI_NAME);
    if fs::symlink_metadata(&ini).is_err() {
        return Err("nessuna copia locale".into());
    }
    for d in trust_chain(dir) {
        let link = fs::symlink_metadata(&d).is_ok_and(|m| is_link_md(&m));
        if link || protect.check(&d).is_err() {
            return Err(UNPROTECTED_COPY.to_string());
        }
    }
    // Controllo e lettura sullo stesso handle.
    let bytes = read_trusted(&ini, protect, MAX_BYTES as u64).map_err(|e| match e {
        ReadError::Missing => "nessuna copia locale".to_string(),
        ReadError::Untrusted => UNPROTECTED_COPY.to_string(),
        ReadError::Other(e) => format!("copia locale illeggibile: {e}"),
    })?;
    let version = is_valid(&bytes).map_err(|e| format!("copia locale non valida: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "il file non è UTF-8".to_string())?;
    Ok((text, version))
}

/// Un giro di aggiornamento dentro `dir`, con la protezione vera.
pub fn update(dir: &Path, fetch: &dyn Fetch, url: &str) -> UpdateOutcome {
    update_with(dir, fetch, url, &AdminOnly)
}

/// Un giro di aggiornamento dentro `dir` (creata e protetta con `protect`
/// prima di ogni scrittura: se non si riesce a proteggerla non si scrive).
pub fn update_with(dir: &Path, fetch: &dyn Fetch, url: &str, protect: &dyn Protect) -> UpdateOutcome {
    let ini = dir.join(INI_NAME);
    // Una copia vale solo se è protetta e passa `is_valid`: una copia rovinata
    // o piazzata da altri equivale a nessuna copia (niente If-None-Match).
    let local_version = read_valid_with(dir, protect).ok().map(|(_, v)| v);
    let have_copy = local_version.is_some();
    let fail = |reason: String| {
        if have_copy {
            UpdateOutcome::KeptOld(reason)
        } else {
            UpdateOutcome::NoCopyYet(reason)
        }
    };
    // `PulisciPC` e `winapp2` fidate (una cartella creata prima da altri si
    // toglie di mezzo e si ricrea), prima di ogni rete e scrittura.
    if let Err(e) = ensure_chain(dir, protect) {
        return fail(format!("cartella delle regole non protetta: {e}"));
    }

    // L'etag vale solo se la copia esiste: un 304 senza file non serve a nulla.
    let etag = if have_copy {
        fs::read_to_string(dir.join(ETAG_NAME))
            .ok()
            .and_then(|s| sanitize_etag(&s))
    } else {
        None
    };

    match fetch.get(url, etag.as_deref(), FETCH_TIMEOUT) {
        Err(e) => fail(e),
        Ok(Fetched::NotModified) => {
            if have_copy {
                UpdateOutcome::UpToDate
            } else {
                fail("risposta 304 senza copia locale".into())
            }
        }
        Ok(Fetched::Body { bytes, etag }) => {
            let version = match is_valid(&bytes) {
                Ok(v) => v,
                Err(e) => return fail(format!("file scaricato non valido: {e}")),
            };
            if let Some(old) = &local_version
                && is_older(&version, old)
            {
                return fail("versione più vecchia ignorata".into());
            }
            // La cartella si riprotegge subito prima di scrivere.
            if let Err(e) = ensure_chain(dir, protect) {
                return fail(format!("cartella delle regole non protetta: {e}"));
            }
            // La cartella è fidata: un `.tmp` rimasto è nostro e si toglie; il
            // nuovo si crea con `create_new` (mai un file già esistente).
            let tmp = dir.join(TMP_NAME);
            if fs::symlink_metadata(&tmp).is_ok() && !remove_tree_no_follow(&tmp) {
                return fail("file temporaneo rimasto non cancellabile".into());
            }
            if let Err(e) = write_new(&tmp, &bytes) {
                let _ = fs::remove_file(&tmp);
                return fail(format!("scrittura non riuscita: {e}"));
            }
            if let Err(e) = rename_retry(&tmp, &ini) {
                let _ = fs::remove_file(&tmp);
                return fail(format!("sostituzione non riuscita: {e}"));
            }
            match etag.as_deref().and_then(sanitize_etag) {
                Some(t) => {
                    let _ = fs::write(dir.join(ETAG_NAME), t);
                }
                None => {
                    let _ = fs::remove_file(dir.join(ETAG_NAME));
                }
            }
            UpdateOutcome::Updated { version }
        }
    }
}

/// `fs::rename` ritentato `RENAME_TRIES` volte, `RENAME_BACKOFF` fra un
/// tentativo e l'altro (un antivirus può tenere il file aperto per un attimo).
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..RENAME_TRIES {
        if attempt > 0 {
            thread::sleep(RENAME_BACKOFF);
        }
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("rename non tentato")))
}

/// Avvia l'aggiornamento della cartella `dir` in un thread staccato (nessun
/// `join`): non blocca la UI né l'uscita dal programma. `on_done` gira sul
/// thread di lavoro ed è chiamata sempre una volta sola (anche se il lavoro va
/// in panico o il thread non riesce a partire). Con `dir` = [`data_dir`],
/// quando la copia nuova è a posto si toglie quella vecchia di `%APPDATA%`.
pub fn spawn_update(dir: PathBuf, on_done: impl FnOnce(UpdateOutcome) + Send + 'static) {
    let fetch = Arc::new(CurlFetch::new(dir.join(WORK_NAME), Arc::new(AdminOnly)));
    let legacy = if dir == data_dir() { legacy_dir() } else { None };
    spawn_update_with(
        dir,
        fetch,
        Arc::new(AdminOnly),
        WINAPP2_URL.to_string(),
        move |outcome| {
            if matches!(outcome, UpdateOutcome::Updated { .. } | UpdateOutcome::UpToDate)
                && let Some(old) = legacy
            {
                remove_legacy_copy(&old);
            }
            on_done(outcome);
        },
    );
}

type DoneFn = Box<dyn FnOnce(UpdateOutcome) + Send>;
type DoneSlot = Arc<Mutex<Option<DoneFn>>>;

/// Come [`spawn_update`] ma con sorgente, protezione e URL scelti da chi chiama.
pub fn spawn_update_with(
    dir: PathBuf,
    fetch: Arc<dyn Fetch>,
    protect: Arc<dyn Protect>,
    url: String,
    on_done: impl FnOnce(UpdateOutcome) + Send + 'static,
) {
    // La callback sta in uno slot condiviso: se il thread non parte la
    // riprende chi chiama, altrimenti la prende il thread (una volta sola).
    let slot: DoneSlot = Arc::new(Mutex::new(Some(Box::new(on_done))));
    let worker_slot = Arc::clone(&slot);
    let worker_dir = dir.clone();
    let spawned = thread::Builder::new()
        .name("winapp2-update".into())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                update_with(&worker_dir, fetch.as_ref(), &url, protect.as_ref())
            }))
            .unwrap_or_else(|_| {
                fallback_outcome(&worker_dir, "aggiornamento interrotto da un errore interno")
            });
            finish(&worker_slot, outcome);
        });
    if spawned.is_err() {
        finish(&slot, fallback_outcome(&dir, "impossibile avviare l'aggiornamento"));
    }
}

fn finish(slot: &Mutex<Option<DoneFn>>, outcome: UpdateOutcome) {
    let cb = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(cb) = cb {
        cb(outcome);
    }
}

fn fallback_outcome(dir: &Path, reason: &str) -> UpdateOutcome {
    if dir.join(INI_NAME).is_file() {
        UpdateOutcome::KeptOld(reason.to_string())
    } else {
        UpdateOutcome::NoCopyYet(reason.to_string())
    }
}

/// ETag accettabile: niente caratteri di controllo (iniezione di header) e al
/// massimo 256 caratteri. Altrimenti `None` (come se non ci fosse).
fn sanitize_etag(raw: &str) -> Option<String> {
    let t = raw.trim_matches(' ');
    if t.is_empty() || t.chars().count() > 256 || t.chars().any(|c| c.is_control()) {
        None
    } else {
        Some(t.to_string())
    }
}

/// `true` se `new` è un numero di versione strettamente minore di `old`.
fn is_older(new: &str, old: &str) -> bool {
    match (new.parse::<u64>(), old.parse::<u64>()) {
        (Ok(n), Ok(o)) => n < o,
        _ => false,
    }
}


// ---------------------------------------------------------------------------
// ACL della cartella delle regole.

/// SID di cui ci si fida per scrivere: SYSTEM, Administrators, TrustedInstaller.
const TRUSTED_SIDS: &[&str] = &[
    "S-1-5-18",
    "S-1-5-32-544",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];

/// SID che non compaiono mai in un token (modelli per i figli): un loro
/// permesso non vale per nessuno.
const TEMPLATE_SIDS: &[&str] = &["S-1-3-0", "S-1-3-1"];

/// Diritti che permettono di cambiare la cartella o il file: scrivere o
/// aggiungere (file, sottocartelle), attributi, cancellare, cambiare ACL o
/// proprietario, e i diritti generici di scrittura.
const WRITE_LIKE: u32 = 0x0000_0002 // FILE_WRITE_DATA / FILE_ADD_FILE
    | 0x0000_0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
    | 0x0000_0010 // FILE_WRITE_EA
    | 0x0000_0040 // FILE_DELETE_CHILD
    | 0x0000_0100 // FILE_WRITE_ATTRIBUTES
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000 // GENERIC_ALL
    | 0x4000_0000; // GENERIC_WRITE

/// Voce di una DACL, già letta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AceInfo {
    /// `ACCESS_ALLOWED_ACE_TYPE` (0), `ACCESS_DENIED_ACE_TYPE` (1) o altro.
    pub ace_type: u8,
    /// La voce vale solo per i figli (`INHERIT_ONLY_ACE`).
    pub inherit_only: bool,
    pub mask: u32,
    pub sid: String,
}

/// Decide se proprietario e DACL lasciano modificare l'oggetto solo agli
/// amministratori. `dacl: None` = DACL nulla (tutti possono tutto).
pub fn acl_verdict(owner: &str, dacl: Option<&[AceInfo]>) -> Result<(), String> {
    if !TRUSTED_SIDS.contains(&owner) {
        // Il proprietario può sempre cambiare l'ACL.
        return Err(format!("proprietario non amministratore ({owner})"));
    }
    let Some(aces) = dacl else {
        return Err("nessuna ACL: chiunque può modificare".into());
    };
    for ace in aces {
        if ace.inherit_only
            || TRUSTED_SIDS.contains(&ace.sid.as_str())
            || TEMPLATE_SIDS.contains(&ace.sid.as_str())
        {
            continue;
        }
        match ace.ace_type {
            1 => continue, // un divieto toglie, non aggiunge
            0 => {
                if ace.mask & WRITE_LIKE != 0 {
                    return Err(format!(
                        "{} ha diritti di modifica (0x{:08x})",
                        ace.sid, ace.mask
                    ));
                }
            }
            other => {
                return Err(format!("voce dell'ACL di tipo {other} non riconosciuta"));
            }
        }
    }
    Ok(())
}

/// Argomenti di `icacls` che rendono `dir` modificabile solo dagli
/// amministratori: niente ereditarietà, SYSTEM e Administrators controllo
/// completo, Users lettura ed esecuzione (SID noti: nessuna lingua in mezzo).
fn icacls_grant_args(dir: &Path) -> Vec<OsString> {
    let mut a = vec![dir.as_os_str().to_os_string()];
    for s in [
        "/inheritance:r",
        "/grant:r",
        "*S-1-5-18:(OI)(CI)F",
        "*S-1-5-32-544:(OI)(CI)F",
        "*S-1-5-32-545:(OI)(CI)RX",
        "/Q",
    ] {
        a.push(OsString::from(s));
    }
    a
}

fn icacls_owner_args(dir: &Path) -> Vec<OsString> {
    vec![
        dir.as_os_str().to_os_string(),
        OsString::from("/setowner"),
        OsString::from("*S-1-5-32-544"),
        OsString::from("/Q"),
    ]
}

/// `true` se `path` o una cartella che lo contiene (fra quelle che esistono)
/// è un collegamento o una giunzione.
fn has_link_in_chain(path: &Path) -> bool {
    path.ancestors().any(|p| {
        fs::symlink_metadata(p).is_ok_and(|m| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0
            }
            #[cfg(not(windows))]
            {
                m.file_type().is_symlink()
            }
        })
    })
}

/// Crea `dir` se manca e la rende modificabile solo dagli amministratori con
/// l'`icacls.exe` di sistema (percorso assoluto, nessuna shell, nessuna
/// finestra): `/inheritance:r /grant:r *S-1-5-18:(OI)(CI)F
/// *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX`, poi (se riesce)
/// `/setowner *S-1-5-32-544`. Alla fine verifica con [`check_admin_only`].
/// Rifiuta una cartella (o un antenato) che sia un collegamento.
pub fn secure_dir(dir: &Path) -> Result<(), String> {
    if has_link_in_chain(dir) {
        return Err("la cartella o una che la contiene è un collegamento".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("cartella non creabile: {e}"))?;
    if has_link_in_chain(dir) {
        return Err("la cartella o una che la contiene è un collegamento".into());
    }
    let icacls = system_directory()
        .map(|d| d.join("icacls.exe"))
        .filter(|p| p.is_file())
        .ok_or_else(|| "icacls.exe non trovato".to_string())?;
    run_hidden(&icacls, &icacls_grant_args(dir))
        .map_err(|e| format!("permessi non impostati: {e}"))?;
    let _ = run_hidden(&icacls, &icacls_owner_args(dir));
    check_admin_only(dir)
}

/// Esegue `exe` senza finestra né shell e aspetta al più `ICACLS_TIMEOUT`.
fn run_hidden(exe: &Path, args: &[OsString]) -> Result<(), String> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("non avviabile: {e}"))?;
    let deadline = Instant::now() + ICACLS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(s)) => {
                return Err(format!("terminato con codice {}", s.code().unwrap_or(-1)));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("tempo scaduto".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                return Err(format!("attesa non riuscita: {e}"));
            }
        }
    }
}

/// `Ok` se `path` (cartella o file) non è un collegamento, ha per proprietario
/// SYSTEM, Administrators o TrustedInstaller, e nessun altro ha diritti di
/// modifica nella DACL (vedi [`acl_verdict`]). Legge i SID con
/// `GetNamedSecurityInfoW`: non dipende dalla lingua di Windows.
pub fn check_admin_only(path: &Path) -> Result<(), String> {
    if has_link_in_chain(path) {
        return Err("collegamento nel percorso".into());
    }
    let (owner, dacl) = read_acl(AclOf::Path(path))?;
    acl_verdict(&owner, dacl.as_deref())
}

/// Come [`check_admin_only`], sull'oggetto aperto da `file` (`GetSecurityInfo`
/// sullo stesso handle da cui poi si legge: nessun intervallo fra controllo e
/// lettura).
pub fn check_admin_only_file(file: &fs::File) -> Result<(), String> {
    let (owner, dacl) = read_acl(AclOf::File(file))?;
    acl_verdict(&owner, dacl.as_deref())
}

/// Di chi leggere proprietario e DACL.
enum AclOf<'a> {
    Path(&'a Path),
    File(&'a fs::File),
}

#[cfg(windows)]
fn read_acl(of: AclOf) -> Result<(String, Option<Vec<AceInfo>>), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetNamedSecurityInfoW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
        GetAce, GetAclInformation, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PSID,
    };

    /// SID in forma testuale (`S-1-5-…`).
    unsafe fn sid_text(sid: PSID) -> Option<String> {
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` è un SID valido letto dal descrittore; `raw` si libera
        // con `LocalFree` dopo la copia.
        unsafe {
            if ConvertSidToStringSidW(sid, &mut raw) == 0 || raw.is_null() {
                return None;
            }
            let mut len = 0;
            while *raw.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
            LocalFree(raw.cast());
            Some(s)
        }
    }

    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = std::ptr::null_mut();
    let what = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    // SAFETY: il nome è terminato da zero, l'handle è aperto per tutta la
    // chiamata; i puntatori di uscita sono validi; il descrittore si libera con
    // `LocalFree` alla fine.
    let err = unsafe {
        match of {
            AclOf::Path(path) => {
                let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                GetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    what,
                    &mut owner,
                    std::ptr::null_mut(),
                    &mut dacl,
                    std::ptr::null_mut(),
                    &mut sd,
                )
            }
            AclOf::File(file) => GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                what,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            ),
        }
    };
    if err != 0 {
        return Err(format!(
            "permessi illeggibili: {}",
            std::io::Error::from_raw_os_error(err as i32)
        ));
    }
    // SAFETY: `owner` e `dacl` puntano dentro `sd`, valido fino a `LocalFree`.
    let result = unsafe {
        let owner_text = if owner.is_null() { None } else { sid_text(owner) };
        let aces = if dacl.is_null() {
            None
        } else {
            let mut info = ACL_SIZE_INFORMATION {
                AceCount: 0,
                AclBytesInUse: 0,
                AclBytesFree: 0,
            };
            if GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ) == 0
            {
                LocalFree(sd.cast());
                return Err("ACL illeggibile".into());
            }
            let mut list = Vec::new();
            for i in 0..info.AceCount {
                let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
                if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                    LocalFree(sd.cast());
                    return Err("voce dell'ACL illeggibile".into());
                }
                let header = *(ace as *const ACE_HEADER);
                // ACCESS_ALLOWED_ACE e ACCESS_DENIED_ACE: intestazione (4 byte),
                // maschera (4 byte), poi il SID.
                let (mask, sid) = if header.AceType <= 1 {
                    let mask = *(ace as *const u8).add(4).cast::<u32>();
                    (mask, sid_text((ace as *mut u8).add(8).cast()).unwrap_or_default())
                } else {
                    (0, String::new())
                };
                list.push(AceInfo {
                    ace_type: header.AceType,
                    inherit_only: u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0,
                    mask,
                    sid,
                });
            }
            Some(list)
        };
        LocalFree(sd.cast());
        (owner_text, aces)
    };
    let (owner, aces) = result;
    Ok((owner.ok_or("proprietario illeggibile")?, aces))
}

#[cfg(not(windows))]
fn read_acl(_of: AclOf) -> Result<(String, Option<Vec<AceInfo>>), String> {
    Err("permessi non verificabili fuori da Windows".into())
}

// ---------------------------------------------------------------------------
// Cartella di sistema.

#[cfg(windows)]
fn system_directory() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    let mut buf = [0u16; 260];
    // SAFETY: buffer valido per `buf.len()` elementi.
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..n])))
}

#[cfg(not(windows))]
fn system_directory() -> Option<PathBuf> {
    None
}

// ---------------------------------------------------------------------------
// Scaricamento con curl.

/// Scaricamento con il `curl.exe` di Windows: percorso assoluto della cartella
/// di sistema, nessuna finestra, nessuna shell, nessun dato dell'utente nei
/// parametri. I file di lavoro (`headers.txt`, `body.bin`) stanno in una
/// cartella nuova dentro `work` (la sottocartella `tmp` della cartella
/// protetta), usata come directory corrente di curl e tolta alla fine.
pub struct CurlFetch {
    work: PathBuf,
    protect: Arc<dyn Protect>,
}

impl CurlFetch {
    /// `work` è la sottocartella di lavoro dentro la cartella fidata; si rende
    /// fidata con `protect` (vedi [`ensure_admin_dir`]) prima di ogni uso.
    pub fn new(work: PathBuf, protect: Arc<dyn Protect>) -> CurlFetch {
        CurlFetch { work, protect }
    }
}

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Cartelle di lavoro di curl rimaste da un'esecuzione interrotta.
const SCRATCH_PREFIX: &str = "pulisci-curl-";
const STALE_AFTER: Duration = Duration::from_secs(600);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new_in(base: &Path) -> std::io::Result<ScratchDir> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = base.join(format!(
            "{SCRATCH_PREFIX}{}-{}-{}",
            std::process::id(),
            nanos,
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p)?; // deve essere nuova: niente cartelle altrui
        Ok(ScratchDir(p))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Toglie da `base` le cartelle `pulisci-curl-*` più vecchie di `older_than`.
fn remove_stale_scratch(base: &Path, older_than: Duration) {
    let Ok(rd) = fs::read_dir(base) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(SCRATCH_PREFIX) {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if old_enough {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// `<dir>\curl.exe` se esiste.
fn locate_curl(system_dir: &Path) -> Result<PathBuf, String> {
    let p = system_dir.join("curl.exe");
    if p.is_file() {
        Ok(p)
    } else {
        Err("curl.exe non trovato".to_string())
    }
}

/// Percorso assoluto del `curl.exe` di Windows (`GetSystemDirectoryW`, con
/// ripiego su `%SystemRoot%\System32`); mai una ricerca nel PATH.
fn curl_exe_path() -> Result<PathBuf, String> {
    let dir = system_directory()
        .or_else(|| std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32")))
        .ok_or_else(|| "curl.exe non trovato".to_string())?;
    locate_curl(&dir)
}

/// Codice d'uscita di curl in parole italiane (il testo di curl, in inglese,
/// non arriva all'utente).
fn curl_error_text(code: Option<i32>) -> String {
    let what = match code {
        Some(5) => "server proxy non trovato",
        Some(6) => "server non trovato (nessuna connessione o DNS non raggiungibile)",
        Some(7) => "connessione al server non riuscita",
        Some(16 | 92) => "errore del protocollo HTTP/2",
        Some(18) => "scaricamento interrotto a metà",
        Some(22) => "il server ha risposto con un errore",
        Some(23) => "scrittura del file scaricato non riuscita",
        Some(28) => "tempo scaduto",
        Some(35) => "connessione sicura (TLS) non riuscita",
        Some(47) => "troppi reindirizzamenti",
        Some(52) => "il server non ha risposto",
        Some(55) => "invio dei dati non riuscito",
        Some(56) => "ricezione dei dati non riuscita",
        Some(58 | 59 | 77 | 83) => "certificati della connessione sicura non utilizzabili",
        Some(60) => "certificato del server non valido",
        Some(63) => "file più grande del massimo consentito",
        Some(1 | 3) => "indirizzo di scaricamento non valido",
        Some(c) => return format!("scaricamento non riuscito (curl, codice {c})"),
        None => "scaricamento interrotto",
    };
    let code = code.map(|c| format!(" (curl, codice {c})")).unwrap_or_default();
    format!("{what}{code}")
}

/// Argomenti di curl. `-q` per primo (nessun .curlrc); solo https anche nei
/// redirect; dimensione massima; file di lavoro con nomi relativi.
fn curl_args(url: &str, etag: Option<&str>, timeout: Duration) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |s: &str| a.push(OsString::from(s));
    push("-q");
    push("-sS");
    push("-L");
    push("--proto");
    push("=https");
    push("--proto-redir");
    push("=https");
    push("--max-redirs");
    push("3");
    push("--max-filesize");
    push(&MAX_BYTES.to_string());
    push("--max-time");
    push(&timeout.as_secs().max(1).to_string());
    push("-D");
    push("headers.txt");
    push("-o");
    push("body.bin");
    push("-A");
    push("PulisciPC");
    if let Some(t) = etag.and_then(sanitize_etag) {
        push("-H");
        push(&format!("If-None-Match: {t}"));
    }
    push(url);
    a
}

impl Fetch for CurlFetch {
    fn get(&self, url: &str, etag: Option<&str>, timeout: Duration) -> Result<Fetched, String> {
        let result = self.get_in_work_dir(url, etag, timeout);
        // La cartella di lavoro si toglie se è rimasta vuota.
        let _ = fs::remove_dir(&self.work);
        result
    }
}

impl CurlFetch {
    fn get_in_work_dir(
        &self,
        url: &str,
        etag: Option<&str>,
        timeout: Duration,
    ) -> Result<Fetched, String> {
        let curl = curl_exe_path()?;
        ensure_admin_dir(&self.work, self.protect.as_ref())
            .map_err(|e| format!("cartella di lavoro non protetta: {e}"))?;
        remove_stale_scratch(&self.work, STALE_AFTER);
        let scratch = ScratchDir::new_in(&self.work).map_err(|e| format!("cartella di lavoro: {e}"))?;

        let mut cmd = Command::new(&curl);
        cmd.current_dir(&scratch.0)
            .args(curl_args(url, etag, timeout))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("curl.exe non avviabile: {e}"))?;

        // Guardiano: se curl non finisce entro timeout + margine, lo si uccide.
        let deadline = Instant::now() + timeout + WATCHDOG_GRACE;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("tempo scaduto: scaricamento interrotto".into());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(format!("attesa di curl non riuscita: {e}"));
                }
            }
        };

        if !status.success() {
            return Err(curl_error_text(status.code()));
        }

        // Si legge solo da una cartella di lavoro fidata, ogni file da un solo
        // handle controllato.
        let read = |name: &str, max: u64| read_scratch(&scratch.0, name, self.protect.as_ref(), max);
        let headers = String::from_utf8_lossy(&read("headers.txt", 64 * 1024)?).into_owned();
        let (code, new_etag) = parse_headers(&headers)?;
        match code {
            304 => Ok(Fetched::NotModified),
            200 => {
                let bytes = read("body.bin", MAX_BYTES as u64)?;
                Ok(Fetched::Body { bytes, etag: new_etag })
            }
            other => Err(format!("risposta HTTP {other}")),
        }
    }
}

/// Un file di lavoro di curl, se la cartella `scratch` è fidata subito prima e
/// il file passa il controllo sul suo stesso handle.
fn read_scratch(scratch: &Path, name: &str, protect: &dyn Protect, max: u64) -> Result<Vec<u8>, String> {
    let link = fs::symlink_metadata(scratch).is_ok_and(|m| is_link_md(&m));
    if link || protect.check(scratch).is_err() {
        return Err("cartella di lavoro non protetta: risposta ignorata".into());
    }
    read_trusted(&scratch.join(name), protect, max).map_err(|e| match e {
        ReadError::Missing => format!("{name} mancante"),
        ReadError::Untrusted => format!("{name} non protetto: risposta ignorata"),
        ReadError::Other(e) => e,
    })
}

/// Dall'ultimo blocco di header (dopo eventuali redirect): codice e ETag.
fn parse_headers(raw: &str) -> Result<(u16, Option<String>), String> {
    let mut code: Option<u16> = None;
    let mut etag: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim_end();
        if line.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("HTTP/")) {
            // Nuovo blocco: si riparte (l'ultimo vince).
            code = line.split_whitespace().nth(1).and_then(|c| c.parse().ok());
            etag = None;
        } else if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("etag")
        {
            etag = sanitize_etag(value);
        }
    }
    code.map(|c| (c, etag))
        .ok_or_else(|| "header senza riga di stato".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::SystemTime;

    /// Winapp2 finto ma valido: `n` sezioni con una FileKey ciascuna.
    fn make_ini(version: &str, n: usize) -> Vec<u8> {
        let mut s = format!("; Version: {version}\n; Winapp2.ini finto per i test\n\n");
        for i in 0..n {
            s.push_str(&format!(
                "[Voce di prova numero {i} *]\nLangSecRef=3025\nDetectFile=%AppData%\\ProvaVoce{i}\nDefault=False\nFileKey1=%AppData%\\ProvaVoce{i}\\cache|*.*|RECURSE\n\n"
            ));
        }
        s.into_bytes()
    }

    /// Protezione finta: crea la cartella e si fida di tutto (nessuna ACL vera).
    struct Open;

    impl Protect for Open {
        fn check_file(&self, _file: &fs::File) -> Result<(), String> {
            Ok(())
        }
        fn quarantine(&self, dir: &Path) -> Result<(), String> {
            quarantine_dir(dir)
        }
        fn secure(&self, dir: &Path) -> Result<(), String> {
            fs::create_dir_all(dir).map_err(|e| e.to_string())
        }
        fn check(&self, _path: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    fn upd(dir: &Path, fetch: &dyn Fetch, url: &str) -> UpdateOutcome {
        update_with(dir, fetch, url, &Open)
    }

    fn valid_ini(version: &str) -> Vec<u8> {
        make_ini(version, 1500)
    }

    type Reply = Result<Fetched, String>;

    struct FakeFetch {
        replies: Mutex<Vec<Reply>>,
        seen_etags: Mutex<Vec<Option<String>>>,
    }

    impl FakeFetch {
        fn new(mut replies: Vec<Reply>) -> FakeFetch {
            replies.reverse();
            FakeFetch { replies: Mutex::new(replies), seen_etags: Mutex::new(Vec::new()) }
        }
    }

    impl Fetch for FakeFetch {
        fn get(&self, _url: &str, etag: Option<&str>, _t: Duration) -> Reply {
            self.seen_etags.lock().unwrap().push(etag.map(String::from));
            self.replies.lock().unwrap().pop().expect("risposta finta esaurita")
        }
    }

    fn body(bytes: Vec<u8>, etag: Option<&str>) -> Reply {
        Ok(Fetched::Body { bytes, etag: etag.map(String::from) })
    }

    fn mtime(p: &Path) -> SystemTime {
        fs::metadata(p).unwrap().modified().unwrap()
    }

    #[test]
    fn valid_file_passes_and_bom_is_allowed() {
        assert_eq!(is_valid(&valid_ini("260915")), Ok("260915".to_string()));
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend(valid_ini("260916"));
        assert_eq!(is_valid(&with_bom), Ok("260916".to_string()));
    }

    #[test]
    fn first_run_downloads_validates_and_stores_etag() {
        let dir = tempfile::tempdir().unwrap();
        let data = valid_ini("260915");
        let fake = FakeFetch::new(vec![body(data.clone(), Some("\"abc\""))]);
        let out = upd(dir.path(), &fake, "http://x/y");
        assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), data);
        assert_eq!(fs::read_to_string(dir.path().join("etag.txt")).unwrap(), "\"abc\"");
        assert!(!dir.path().join("Winapp2.ini.tmp").exists());
        assert_eq!(fake.seen_etags.lock().unwrap()[0], None);
    }

    #[test]
    fn second_run_with_304_keeps_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let data = valid_ini("260915");
        let fake = FakeFetch::new(vec![body(data.clone(), Some("\"abc\"")), Ok(Fetched::NotModified)]);
        upd(dir.path(), &fake, "u");
        let ini = dir.path().join("Winapp2.ini");
        let before = mtime(&ini);
        std::thread::sleep(Duration::from_millis(30));
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::UpToDate);
        assert_eq!(fs::read(&ini).unwrap(), data);
        assert_eq!(mtime(&ini), before);
        assert_eq!(fake.seen_etags.lock().unwrap()[1].as_deref(), Some("\"abc\""));
    }

    #[test]
    fn changed_etag_replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let v2 = valid_ini("261001");
        let fake = FakeFetch::new(vec![body(v1, Some("\"e1\"")), body(v2.clone(), Some("\"e2\""))]);
        upd(dir.path(), &fake, "u");
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::Updated { version: "261001".into() });
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), v2);
        assert_eq!(fs::read_to_string(dir.path().join("etag.txt")).unwrap(), "\"e2\"");
        assert!(!dir.path().join("Winapp2.ini.tmp").exists());
    }

    #[test]
    fn html_error_page_is_rejected_and_old_copy_kept() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let mut html = b"<!DOCTYPE html><html><body><h1>429 Too Many Requests</h1>".to_vec();
        html.resize(400 * 1024, b' '); // anche se grande, resta HTML
        let fake = FakeFetch::new(vec![body(v1.clone(), Some("\"e1\"")), body(html, Some("\"e2\""))]);
        upd(dir.path(), &fake, "u");
        let out = upd(dir.path(), &fake, "u");
        assert!(matches!(out, UpdateOutcome::KeptOld(_)), "{out:?}");
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), v1);
        assert_eq!(fs::read_to_string(dir.path().join("etag.txt")).unwrap(), "\"e1\"");
        assert!(!dir.path().join("Winapp2.ini.tmp").exists());
    }

    #[test]
    fn empty_and_truncated_bodies_are_rejected() {
        let full = valid_ini("260915");
        assert!(full.len() > 150 * 1024);
        let header_only = b"; Version: 260915\n; Winapp2.ini\n".to_vec();
        let cut_150k = full[..150 * 1024].to_vec();
        // Troncato a metà voce ma ancora sopra 200 KB: poche sezioni -> rifiutato.
        let big_few_sections = {
            let mut v = make_ini("260915", 20);
            v.resize(300 * 1024, b';');
            v
        };
        for (name, bytes) in [
            ("0 byte", Vec::new()),
            ("solo intestazione", header_only),
            ("150 KB", cut_150k),
            ("poche sezioni", big_few_sections),
        ] {
            assert!(is_valid(&bytes).is_err(), "{name} doveva essere rifiutato");
            let dir = tempfile::tempdir().unwrap();
            let fake = FakeFetch::new(vec![body(bytes, Some("\"x\""))]);
            let out = upd(dir.path(), &fake, "u");
            assert!(matches!(out, UpdateOutcome::NoCopyYet(_)), "{name}: {out:?}");
            assert!(!dir.path().join("Winapp2.ini").exists(), "{name}");
            assert!(!dir.path().join("etag.txt").exists(), "{name}");
        }
        // Senza FileKey, senza versione, non UTF-8, oltre 20 MB.
        let no_filekey = String::from_utf8(valid_ini("1")).unwrap().replace("FileKey", "XKey");
        assert!(is_valid(no_filekey.as_bytes()).is_err());
        let no_version = String::from_utf8(valid_ini("1")).unwrap().replace("; Version:", "; Vers:");
        assert!(is_valid(no_version.as_bytes()).is_err());
        let mut bad_utf8 = valid_ini("1");
        bad_utf8.push(0xFF);
        assert!(is_valid(&bad_utf8).is_err());
        assert!(is_valid(&vec![b'a'; 21 * 1024 * 1024]).is_err());
    }

    #[test]
    fn network_error_without_copy_reports_no_copy_yet() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeFetch::new(vec![Err("curl: Could not resolve host".into())]);
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::NoCopyYet("curl: Could not resolve host".into()));
        assert!(!dir.path().join("Winapp2.ini").exists());
    }

    #[test]
    fn network_error_with_copy_keeps_old() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let fake = FakeFetch::new(vec![body(v1.clone(), Some("\"e\"")), Err("timeout".into())]);
        upd(dir.path(), &fake, "u");
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::KeptOld("timeout".into()));
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), v1);
    }

    #[test]
    fn etag_is_not_sent_without_a_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("etag.txt"), "\"stale\"").unwrap();
        let fake = FakeFetch::new(vec![Ok(Fetched::NotModified)]);
        let out = upd(dir.path(), &fake, "u");
        assert!(matches!(out, UpdateOutcome::NoCopyYet(_)), "{out:?}");
        assert_eq!(fake.seen_etags.lock().unwrap()[0], None);
    }

    struct SlowFetch;
    impl Fetch for SlowFetch {
        fn get(&self, _u: &str, _e: Option<&str>, _t: Duration) -> Reply {
            std::thread::sleep(Duration::from_secs(2));
            Err("lento".into())
        }
    }

    #[test]
    fn spawn_update_returns_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let t0 = Instant::now();
        spawn_update_with(dir.path().to_path_buf(), Arc::new(SlowFetch), Arc::new(Open), "u".into(), move |o| {
            let _ = tx.send(o);
        });
        assert!(t0.elapsed() < Duration::from_millis(100), "{:?}", t0.elapsed());
        let out = rx.recv_timeout(Duration::from_secs(10)).expect("on_done non chiamato");
        assert_eq!(out, UpdateOutcome::NoCopyYet("lento".into()));
    }

    #[test]
    fn parse_headers_takes_the_last_block_and_etag() {
        let raw = "HTTP/2 301\r\nlocation: x\r\n\r\nHTTP/2 200\r\nContent-Type: text/plain\r\nETag: W/\"abc\"\r\n\r\n";
        assert_eq!(parse_headers(raw), Ok((200, Some("W/\"abc\"".to_string()))));
        let raw304 = "HTTP/2 304 \r\netag: \"q\"\r\n\r\n";
        assert_eq!(parse_headers(raw304), Ok((304, Some("\"q\"".to_string()))));
        assert!(parse_headers("").is_err());
    }

    // ---- Fix round 1 ----

    fn ini_with_entries(version: &str, n: usize, declared: &str) -> Vec<u8> {
        let body = String::from_utf8(make_ini(version, n)).unwrap();
        body.replacen(
            "\n; Winapp2.ini finto",
            &format!("\n; # of entries: {declared}\n; Winapp2.ini finto"),
            1,
        )
        .into_bytes()
    }

    #[test]
    fn file_cut_mid_line_is_rejected() {
        let full = valid_ini("260915");
        let cut = &full[..full.len() - 20];
        assert!(cut.len() > 200 * 1024);
        assert_ne!(cut.last(), Some(&b'\n'));
        assert!(is_valid(cut).is_err());
        // Anche un file intero senza il ritorno a capo finale è sospetto.
        let mut no_nl = full.clone();
        while no_nl.last() == Some(&b'\n') {
            no_nl.pop();
        }
        assert!(is_valid(&no_nl).is_err());
    }

    #[test]
    fn declared_entries_must_be_reached() {
        // dichiarate 4,068 ma presenti 1500: rifiutato
        let short = ini_with_entries("260915", 1500, "4,068");
        let err = is_valid(&short).unwrap_err();
        assert!(err.contains("4068"), "{err}");
        // dichiarate 1,500 e presenti 1500: valido; dichiarate meno: valido
        assert!(is_valid(&ini_with_entries("260915", 1500, "1,500")).is_ok());
        assert!(is_valid(&ini_with_entries("260915", 1500, "1200")).is_ok());
    }

    #[test]
    fn etag_sanitising() {
        assert_eq!(sanitize_etag("\"abc\""), Some("\"abc\"".into()));
        assert_eq!(sanitize_etag("  W/\"abc\"  "), Some("W/\"abc\"".into()));
        assert_eq!(sanitize_etag("a\nb"), None);
        assert_eq!(sanitize_etag("a\r\nX-Evil: 1"), None);
        assert_eq!(sanitize_etag("a\rb"), None);
        assert_eq!(sanitize_etag("a\0b"), None);
        assert_eq!(sanitize_etag(""), None);
        assert_eq!(sanitize_etag(&"x".repeat(257)), None);
        assert_eq!(sanitize_etag(&"x".repeat(256)), Some("x".repeat(256)));
    }

    #[test]
    fn dirty_etag_in_file_is_not_sent_and_dirty_header_etag_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let fake = FakeFetch::new(vec![
            body(v1.clone(), Some("\"e1\"")),
            body(valid_ini("261001"), Some("bad\r\nX: 1")),
        ]);
        upd(dir.path(), &fake, "u");
        fs::write(dir.path().join("etag.txt"), "\"e1\"\nX-Evil: 1").unwrap();
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::Updated { version: "261001".into() });
        assert_eq!(fake.seen_etags.lock().unwrap()[1], None);
        assert!(!dir.path().join("etag.txt").exists());
    }

    #[test]
    fn parse_headers_rejects_dirty_etag_and_survives_multibyte() {
        let long = format!("HTTP/2 200\r\netag: {}\r\n\r\n", "x".repeat(300));
        assert_eq!(parse_headers(&long), Ok((200, None)));
        // riga con carattere multibyte a cavallo del byte 5: niente panic
        let raw = "HTTP/2 200\r\nàààà\r\nétag: x\r\n\r\n";
        assert_eq!(parse_headers(raw), Ok((200, None)));
    }

    #[test]
    fn curl_args_are_hardened() {
        let args: Vec<String> =
            curl_args("https://example.org/x.ini", Some("\"e1\""), Duration::from_secs(20))
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
        assert_eq!(args[0], "-q");
        let joined = args.join(" ");
        for needle in [
            "--proto =https",
            "--proto-redir =https",
            "--max-redirs 3",
            "--max-filesize 20971520",
            "--max-time 20",
            "-A PulisciPC",
            "-H If-None-Match: \"e1\"",
        ] {
            assert!(joined.contains(needle), "manca '{needle}' in {joined}");
        }
        assert_eq!(args.last().unwrap(), "https://example.org/x.ini");
        let tmp = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(!joined.contains(&tmp), "percorso temporaneo nei parametri");
        assert!(!joined.to_lowercase().contains("users"), "dato utente nei parametri");
        // etag sporco o assente: nessun If-None-Match
        for e in [Some("a\nb"), Some(""), None] {
            let a = curl_args("u", e, Duration::from_secs(20));
            assert!(!a.iter().any(|x| x.to_string_lossy().contains("If-None-Match")));
        }
    }

    #[test]
    fn locate_curl_uses_the_given_system_dir_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(locate_curl(dir.path()), Err("curl.exe non trovato".to_string()));
        fs::write(dir.path().join("curl.exe"), b"x").unwrap();
        assert_eq!(locate_curl(dir.path()), Ok(dir.path().join("curl.exe")));
        // il percorso vero del sistema esiste su Windows 11 ed è assoluto
        let real = curl_exe_path().expect("curl.exe di sistema");
        assert!(real.is_absolute(), "{real:?}");
    }

    #[test]
    fn damaged_local_copy_is_treated_as_no_copy() {
        for damaged in [Vec::new(), valid_ini("260915")[..150 * 1024].to_vec()] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("Winapp2.ini"), &damaged).unwrap();
            fs::write(dir.path().join("etag.txt"), "\"old\"").unwrap();
            let fresh = valid_ini("260915");
            let fake = FakeFetch::new(vec![body(fresh.clone(), Some("\"new\""))]);
            let out = upd(dir.path(), &fake, "u");
            assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
            assert_eq!(fake.seen_etags.lock().unwrap()[0], None, "etag inviato con copia rovinata");
            assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), fresh);
        }
        // un 304 non lascia mai "UpToDate" su una copia rovinata
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Winapp2.ini"), b"").unwrap();
        fs::write(dir.path().join("etag.txt"), "\"old\"").unwrap();
        let fake = FakeFetch::new(vec![Ok(Fetched::NotModified)]);
        let out = upd(dir.path(), &fake, "u");
        assert!(matches!(out, UpdateOutcome::NoCopyYet(_)), "{out:?}");
    }

    #[test]
    fn failed_tmp_write_keeps_old_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let fake = FakeFetch::new(vec![
            body(v1.clone(), Some("\"e1\"")),
            body(valid_ini("261001"), Some("\"e2\"")),
        ]);
        upd(dir.path(), &fake, "u");
        // Un `.tmp` rimasto che non si riesce a togliere (un file dentro è
        // aperto senza condivisione): non si scrive.
        let stuck = dir.path().join("Winapp2.ini.tmp");
        fs::create_dir(&stuck).unwrap();
        fs::write(stuck.join("x"), "x").unwrap();
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new().read(true).share_mode(0).open(stuck.join("x")).unwrap()
        };
        let out = upd(dir.path(), &fake, "u");
        assert!(matches!(out, UpdateOutcome::KeptOld(_)), "{out:?}");
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), v1);
        assert_eq!(fs::read_to_string(dir.path().join("etag.txt")).unwrap(), "\"e1\"");
    }

    #[test]
    fn older_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let new = valid_ini("261001");
        let fake = FakeFetch::new(vec![
            body(new.clone(), Some("\"e1\"")),
            body(valid_ini("260915"), Some("\"e2\"")),
        ]);
        upd(dir.path(), &fake, "u");
        let out = upd(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::KeptOld("versione più vecchia ignorata".into()));
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), new);
        assert_eq!(fs::read_to_string(dir.path().join("etag.txt")).unwrap(), "\"e1\"");
    }

    struct PanicFetch;
    impl Fetch for PanicFetch {
        fn get(&self, _u: &str, _e: Option<&str>, _t: Duration) -> Reply {
            panic!("rete esplosa (prova)");
        }
    }

    #[test]
    fn on_done_is_called_exactly_once_even_if_worker_panics() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_update_with(dir.path().to_path_buf(), Arc::new(PanicFetch), Arc::new(Open), "u".into(), move |o| {
            let _ = tx.send(o);
        });
        let out = rx.recv_timeout(Duration::from_secs(10)).expect("on_done non chiamato");
        assert!(matches!(out, UpdateOutcome::NoCopyYet(_)), "{out:?}");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "on_done chiamato due volte");
    }

    #[test]
    fn stale_scratch_dirs_are_removed() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir(base.path().join("pulisci-curl-1-2-3")).unwrap();
        fs::write(base.path().join("pulisci-curl-1-2-3").join("body.bin"), b"x").unwrap();
        fs::create_dir(base.path().join("altra-cartella")).unwrap();
        fs::write(base.path().join("pulisci-curl-file"), b"x").unwrap();
        // con soglia lunga non si tocca nulla (cartelle recenti = altra istanza al lavoro)
        remove_stale_scratch(base.path(), Duration::from_secs(3600));
        assert!(base.path().join("pulisci-curl-1-2-3").exists());
        remove_stale_scratch(base.path(), Duration::ZERO);
        assert!(!base.path().join("pulisci-curl-1-2-3").exists());
        assert!(base.path().join("altra-cartella").exists());
        assert!(base.path().join("pulisci-curl-file").exists(), "solo cartelle");
    }

    /// Prova di rete vera (sola lettura): `cargo test -p xiox-rules real_network -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_network_check() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/winapp2-updater-check");
        let _ = fs::remove_dir_all(&dir);
        let curl = CurlFetch::new(dir.join(WORK_NAME), Arc::new(Open));
        let first = update_with(&dir, &curl, WINAPP2_URL, &Open);
        println!("primo giro: {first:?}");
        let second = update_with(&dir, &curl, WINAPP2_URL, &Open);
        println!("secondo giro: {second:?}");
        assert!(matches!(first, UpdateOutcome::Updated { .. }), "{first:?}");
        assert_eq!(second, UpdateOutcome::UpToDate);
    }

    fn ace(ace_type: u8, mask: u32, sid: &str) -> AceInfo {
        AceInfo { ace_type, inherit_only: false, mask, sid: sid.into() }
    }

    const USERS: &str = "S-1-5-32-545";
    const ADMINS: &str = "S-1-5-32-544";
    const SYSTEM: &str = "S-1-5-18";

    #[test]
    fn acl_verdict_accepts_only_admin_writable_objects() {
        // Quello che lascia `secure_dir`: SYSTEM e Administrators F, Users RX.
        let secured = vec![
            ace(0, 0x001f_01ff, SYSTEM),
            ace(0, 0x001f_01ff, ADMINS),
            ace(0, 0x0012_00a9, USERS),
        ];
        assert_eq!(acl_verdict(ADMINS, Some(&secured)), Ok(()));
        assert_eq!(acl_verdict(SYSTEM, Some(&secured)), Ok(()));
        // Proprietario utente (copia piazzata prima): rifiutata.
        assert!(acl_verdict("S-1-5-21-1-2-3-1001", Some(&secured)).is_err());
        // DACL nulla: chiunque può tutto.
        assert!(acl_verdict(ADMINS, None).is_err());
        // Ereditata da ProgramData o allargata: rifiutata.
        for (mask, sid) in [
            (0x0000_0002, USERS),                 // WD / AD
            (0x0000_0004, USERS),                 // AD
            (0x0000_0116, USERS),                 // WD, AD, WEA, WA
            (0x0013_01bf, USERS),                 // M
            (0x001f_01ff, "S-1-1-0"),             // Everyone F
            (0x0001_0000, "S-1-5-11"),            // Authenticated Users DELETE
            (0x0004_0000, USERS),                 // WRITE_DAC
            (0x4000_0000, USERS),                 // GENERIC_WRITE
            (0x1000_0000, "S-1-5-21-1-2-3-1001"), // GENERIC_ALL a un utente
        ] {
            let mut acl = secured.clone();
            acl.push(ace(0, mask, sid));
            assert!(acl_verdict(ADMINS, Some(&acl)).is_err(), "{sid} 0x{mask:08x} accettato");
        }
        // Voci solo per i figli, divieti e CREATOR OWNER non contano.
        let mut acl = secured.clone();
        acl.push(AceInfo { inherit_only: true, ..ace(0, 0x001f_01ff, USERS) });
        acl.push(ace(1, 0x001f_01ff, USERS));
        acl.push(ace(0, 0x1000_0000, "S-1-3-0"));
        assert_eq!(acl_verdict(ADMINS, Some(&acl)), Ok(()));
        // Tipo di voce sconosciuto: rifiutato per prudenza.
        let mut acl = secured;
        acl.push(ace(5, 0, USERS));
        assert!(acl_verdict(ADMINS, Some(&acl)).is_err());
    }

    #[test]
    fn icacls_arguments_use_well_known_sids() {
        let dir = Path::new(r"C:\ProgramData\PulisciPC\winapp2");
        let grant: Vec<String> =
            icacls_grant_args(dir).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            grant,
            vec![
                r"C:\ProgramData\PulisciPC\winapp2",
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
                "*S-1-5-32-545:(OI)(CI)RX",
                "/Q",
            ]
        );
        let owner: Vec<String> =
            icacls_owner_args(dir).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(owner[1..], ["/setowner", "*S-1-5-32-544", "/Q"]);
    }

    #[test]
    fn curl_exit_codes_become_italian_text() {
        for (code, words) in [
            (6, "server non trovato"),
            (7, "connessione al server non riuscita"),
            (28, "tempo scaduto"),
            (35, "connessione sicura"),
            (60, "certificato del server non valido"),
            (63, "più grande"),
        ] {
            let t = curl_error_text(Some(code));
            assert!(t.contains(words) && t.contains(&format!("codice {code}")), "{code}: {t}");
        }
        assert_eq!(curl_error_text(Some(99)), "scaricamento non riuscito (curl, codice 99)");
        assert_eq!(curl_error_text(None), "scaricamento interrotto");
        for code in 0..100 {
            let t = curl_error_text(Some(code)).to_lowercase();
            for english in ["could not", "couldn", "resolve", "failed", "timeout", "timed out"] {
                assert!(!t.contains(english), "{code}: {t}");
            }
        }
    }

    #[test]
    fn rename_is_retried_then_reported() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        fs::write(&a, "x").unwrap();
        assert!(rename_retry(&a, &dir.path().join("b")).is_ok());
        // Destinazione che è una cartella non vuota: fallisce sempre.
        fs::write(&a, "x").unwrap();
        fs::create_dir_all(dir.path().join("c").join("d")).unwrap();
        let t0 = Instant::now();
        assert!(rename_retry(&a, &dir.path().join("c")).is_err());
        assert!(t0.elapsed() >= RENAME_BACKOFF * (RENAME_TRIES - 1), "{:?}", t0.elapsed());
        assert!(a.exists());
    }

    /// Protezione finta che rifiuta ogni controllo (copia piazzata da altri).
    struct Untrusted;

    impl Protect for Untrusted {
        fn check_file(&self, _file: &fs::File) -> Result<(), String> {
            Err("Users ha diritti di modifica".into())
        }
        fn quarantine(&self, dir: &Path) -> Result<(), String> {
            quarantine_dir(dir)
        }
        fn secure(&self, dir: &Path) -> Result<(), String> {
            fs::create_dir_all(dir).map_err(|e| e.to_string())
        }
        fn check(&self, _path: &Path) -> Result<(), String> {
            Err("Users ha diritti di modifica".into())
        }
    }

    /// Protezione finta che non riesce a proteggere la cartella.
    struct CannotSecure;

    impl Protect for CannotSecure {
        fn check_file(&self, _file: &fs::File) -> Result<(), String> {
            Ok(())
        }
        fn quarantine(&self, dir: &Path) -> Result<(), String> {
            quarantine_dir(dir)
        }
        fn secure(&self, _dir: &Path) -> Result<(), String> {
            Err("icacls.exe non trovato".into())
        }
        fn check(&self, _path: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn read_valid_returns_text_and_version_only_for_protected_copies() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_valid_with(dir.path(), &Open).is_err(), "nessuna copia");
        let ini = valid_ini("260915");
        fs::write(dir.path().join(INI_NAME), &ini).unwrap();
        let (text, version) = read_valid_with(dir.path(), &Open).unwrap();
        assert_eq!(version, "260915");
        assert_eq!(text.as_bytes(), &ini[..]);
        assert_eq!(read_valid_with(dir.path(), &Untrusted), Err(UNPROTECTED_COPY.to_string()));
    }

    #[test]
    fn planted_copy_in_an_unprotected_folder_is_ignored_and_replaced() {
        let base = tempfile::tempdir().unwrap();
        let rules = base.path().join("regole");
        fs::create_dir(&rules).unwrap();
        fs::write(rules.join(INI_NAME), valid_ini("999999")).unwrap();
        fs::write(rules.join(ETAG_NAME), "\"piazzato\"").unwrap();
        let fake = FakeFetch::new(vec![body(valid_ini("260915"), Some("\"vero\""))]);
        let out = update_with(&rules, &fake, "u", &Untrusted);
        // Nessun If-None-Match dalla copia piazzata, nessun «più vecchia».
        assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
        assert_eq!(*fake.seen_etags.lock().unwrap(), vec![None]);
        assert_eq!(is_valid(&fs::read(rules.join(INI_NAME)).unwrap()), Ok("260915".into()));
        // La cartella piazzata è stata tolta di mezzo (anche il suo etag).
        assert!(!rules.join(ETAG_NAME).exists() || fs::read_to_string(rules.join(ETAG_NAME)).unwrap() == "\"vero\"");
        assert_eq!(leftovers(base.path()), 0);
    }

    #[test]
    fn folder_that_cannot_be_protected_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("regole");
        let fake = FakeFetch::new(vec![body(valid_ini("260915"), None)]);
        let out = update_with(&target, &fake, "u", &CannotSecure);
        assert_eq!(
            out,
            UpdateOutcome::NoCopyYet(
                "cartella delle regole non protetta: icacls.exe non trovato".into()
            )
        );
        assert!(!target.join(INI_NAME).exists());
        assert!(fake.seen_etags.lock().unwrap().is_empty(), "niente rete senza cartella protetta");
        // Con una copia buona resta quella.
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(INI_NAME), valid_ini("260900")).unwrap();
        let out = update_with(&target, &fake, "u", &CannotSecure);
        assert!(
            matches!(out, UpdateOutcome::KeptOld(ref r) if r.starts_with("cartella delle regole non protetta")),
            "{out:?}"
        );
    }

    #[test]
    fn curl_work_files_live_inside_the_given_folder() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join(WORK_NAME);
        fs::create_dir_all(&work).unwrap();
        let scratch = ScratchDir::new_in(&work).unwrap();
        assert_eq!(scratch.0.parent(), Some(work.as_path()));
        let p = scratch.0.clone();
        drop(scratch);
        assert!(!p.exists());
    }

    #[test]
    fn legacy_copy_is_removed_but_a_junction_is_not_followed() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("PulisciPC").join("winapp2");
        fs::create_dir_all(&old).unwrap();
        for n in [INI_NAME, ETAG_NAME] {
            fs::write(old.join(n), "x").unwrap();
        }
        remove_legacy_copy(&old);
        assert!(!old.exists(), "vecchia cartella rimasta");
        // La vecchia cartella è diventata una giunzione verso altro: i file
        // della destinazione non si toccano.
        let other = base.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join(INI_NAME), "prezioso").unwrap();
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&old)
            .arg(&other)
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            println!("SKIP: mklink /J non riuscito");
            return;
        }
        remove_legacy_copy(&old);
        assert_eq!(fs::read_to_string(other.join(INI_NAME)).unwrap(), "prezioso");
    }

    /// ACL vere su una cartella creata apposta:
    /// `cargo test -p xiox-rules secure_dir_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn secure_dir_real_acl() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/acl-test");
        fs::create_dir_all(&root).unwrap();
        let dir = root.join(format!("d{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // Prima: cartella normale, ereditata e di proprietà dell'utente.
        fs::create_dir_all(&dir).unwrap();
        println!("prima: {:?}", check_admin_only(&dir));
        match secure_dir(&dir) {
            Ok(()) => {
                println!("dopo secure_dir: {:?}", check_admin_only(&dir));
                assert_eq!(check_admin_only(&dir), Ok(()));
                fs::write(dir.join(INI_NAME), valid_ini("260915")).unwrap();
                let got = read_valid(&dir).map(|(_, v)| v);
                println!("read_valid: {got:?}");
                assert_eq!(got, Ok("260915".to_string()));
            }
            Err(e) => println!("secure_dir non riuscita (serve l'amministratore?): {e}"),
        }
        println!("data_dir: {}", data_dir().display());
        println!("legacy_dir: {:?}", legacy_dir());
        assert!(data_dir().ends_with(r"PulisciPC\winapp2"));
    }
    /// Cartelle messe da parte rimaste accanto a quelle vere.
    fn leftovers(parent: &Path) -> usize {
        fs::read_dir(parent)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(QUARANTINE_MARK))
            .count()
    }

    /// Protezione finta con verdetti preparati: le cartelle in `tainted` non
    /// passano `check` e `secure` fallisce su di loro (ACE estranee che
    /// `/grant:r` non toglie) finché non vengono tolte di mezzo e ricreate.
    #[derive(Default)]
    struct Scripted {
        tainted: Mutex<std::collections::HashSet<PathBuf>>,
        files_untrusted: bool,
        log: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn tainted(paths: &[&Path]) -> Scripted {
            Scripted {
                tainted: Mutex::new(paths.iter().map(|p| p.to_path_buf()).collect()),
                ..Default::default()
            }
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Protect for Scripted {
        fn secure(&self, dir: &Path) -> Result<(), String> {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            self.log.lock().unwrap().push(format!("secure {name}"));
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            if self.tainted.lock().unwrap().contains(dir) {
                return Err("ACE estranea rimasta".into());
            }
            Ok(())
        }
        fn check(&self, path: &Path) -> Result<(), String> {
            if self.tainted.lock().unwrap().contains(path) {
                Err("Users ha diritti di modifica".into())
            } else {
                Ok(())
            }
        }
        fn check_file(&self, _file: &fs::File) -> Result<(), String> {
            if self.files_untrusted {
                Err("proprietario utente".into())
            } else {
                Ok(())
            }
        }
        fn quarantine(&self, dir: &Path) -> Result<(), String> {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            self.log.lock().unwrap().push(format!("quarantine {name}"));
            // Ciò che conteneva sparisce con lei.
            self.tainted.lock().unwrap().retain(|p| !p.starts_with(dir));
            quarantine_dir(dir)
        }
    }

    /// N1: `PulisciPC\winapp2` creata prima da un utente qualunque (con una
    /// copia e un `tmp` piazzati) si toglie di mezzo e si ricrea.
    #[test]
    fn planted_folders_are_recreated_before_use() {
        let base = tempfile::tempdir().unwrap();
        let top = base.path().join("PulisciPC");
        let rules = top.join("winapp2");
        fs::create_dir_all(rules.join(WORK_NAME)).unwrap();
        fs::write(rules.join(INI_NAME), valid_ini("999999")).unwrap();
        fs::write(rules.join(WORK_NAME).join("body.bin"), "piazzato").unwrap();
        let p = Scripted::tainted(&[&top, &rules]);
        let fake = FakeFetch::new(vec![body(valid_ini("260915"), None)]);
        let out = update_with(&rules, &fake, "u", &p);
        assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
        assert_eq!(*fake.seen_etags.lock().unwrap(), vec![None], "copia piazzata usata");
        assert_eq!(
            p.log()[..3],
            ["quarantine PulisciPC", "secure PulisciPC", "secure winapp2"]
        );
        assert!(!rules.join(WORK_NAME).exists(), "tmp piazzato sopravvissuto");
        assert_eq!(leftovers(base.path()), 0);
    }

    /// N1: ACE estranee che `secure` non toglie non bloccano per sempre: la
    /// cartella si ricrea.
    #[test]
    fn foreign_ace_folder_is_recreated_not_a_permanent_failure() {
        let base = tempfile::tempdir().unwrap();
        let rules = base.path().join("regole");
        fs::create_dir(&rules).unwrap();
        let p = Scripted::tainted(&[&rules]);
        assert!(p.secure(&rules).is_err(), "la finta deve fallire come icacls");
        let fake = FakeFetch::new(vec![body(valid_ini("260915"), None)]);
        assert_eq!(
            update_with(&rules, &fake, "u", &p),
            UpdateOutcome::Updated { version: "260915".into() }
        );
        assert!(p.log().contains(&"quarantine regole".to_string()));
    }

    /// N1: `tmp` di curl piazzato: tolto di mezzo e ricreato; un collegamento al
    /// posto della cartella si toglie come collegamento.
    #[test]
    fn planted_work_dir_is_recreated_and_links_are_not_followed() {
        let base = tempfile::tempdir().unwrap();
        let work = base.path().join(WORK_NAME);
        fs::create_dir(&work).unwrap();
        fs::write(work.join("body.bin"), "piazzato").unwrap();
        let p = Scripted::tainted(&[&work]);
        ensure_admin_dir(&work, &p).unwrap();
        assert!(work.is_dir() && !work.join("body.bin").exists());
        assert_eq!(p.log(), ["quarantine tmp", "secure tmp"]);
        // Giunzione verso un'altra cartella: si sposta e si toglie il
        // collegamento; la destinazione resta com'è.
        fs::remove_dir(&work).unwrap();
        let other = base.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("keep.txt"), "keep").unwrap();
        let linked = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&work)
            .arg(&other)
            .output()
            .is_ok_and(|o| o.status.success());
        if !linked {
            println!("SKIP: mklink /J non riuscito");
            return;
        }
        let p = Scripted::default();
        ensure_admin_dir(&work, &p).unwrap();
        let md = fs::symlink_metadata(&work).unwrap();
        assert!(md.is_dir() && !is_link_md(&md));
        assert_eq!(fs::read_to_string(other.join("keep.txt")).unwrap(), "keep");
        assert_eq!(leftovers(base.path()), 0);
    }

    /// N1: un `Winapp2.ini.tmp` piazzato prima non si riusa: si toglie (la
    /// cartella è fidata) e il nuovo si crea con `create_new`.
    #[test]
    fn stale_tmp_file_is_never_reused() {
        let base = tempfile::tempdir().unwrap();
        let rules = base.path().join("regole");
        fs::create_dir(&rules).unwrap();
        fs::write(rules.join(TMP_NAME), "piazzato").unwrap();
        assert_eq!(
            write_new(&rules.join(TMP_NAME), b"x").unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let fake = FakeFetch::new(vec![body(valid_ini("260915"), None)]);
        let out = update_with(&rules, &fake, "u", &Scripted::default());
        assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
        assert!(!rules.join(TMP_NAME).exists());
        assert_eq!(is_valid(&fs::read(rules.join(INI_NAME)).unwrap()), Ok("260915".into()));
    }

    /// N1: la copia si legge dallo stesso handle che si controlla; una copia
    /// il cui file non passa il controllo, o in una catena non fidata, non vale.
    #[test]
    fn copy_is_checked_on_the_handle_it_is_read_from() {
        let base = tempfile::tempdir().unwrap();
        let top = base.path().join("PulisciPC");
        let rules = top.join("winapp2");
        fs::create_dir_all(&rules).unwrap();
        fs::write(rules.join(INI_NAME), valid_ini("260915")).unwrap();
        assert!(read_valid_with(&rules, &Scripted::default()).is_ok());
        let p = Scripted { files_untrusted: true, ..Default::default() };
        assert_eq!(read_valid_with(&rules, &p), Err(UNPROTECTED_COPY.to_string()));
        assert_eq!(
            read_valid_with(&rules, &Scripted::tainted(&[&top])),
            Err(UNPROTECTED_COPY.to_string())
        );
        // Lettura limitata e su file normali soltanto.
        assert!(matches!(read_trusted(&rules.join(INI_NAME), &Open, 10), Err(ReadError::Other(_))));
        assert!(read_trusted(&rules, &Open, MAX_BYTES as u64).is_err());
        assert!(matches!(read_trusted(&rules.join("nope"), &Open, 10), Err(ReadError::Missing)));
    }

    /// N1: i file di curl si leggono solo da una cartella di lavoro fidata.
    #[test]
    fn curl_output_is_read_only_from_a_trusted_scratch_folder() {
        let base = tempfile::tempdir().unwrap();
        let scratch = base.path().join("pulisci-curl-1");
        fs::create_dir(&scratch).unwrap();
        fs::write(scratch.join("body.bin"), "corpo").unwrap();
        assert_eq!(read_scratch(&scratch, "body.bin", &Scripted::default(), 100).unwrap(), b"corpo");
        let err = read_scratch(&scratch, "body.bin", &Scripted::tainted(&[&scratch]), 100).unwrap_err();
        assert!(err.starts_with("cartella di lavoro non protetta"), "{err}");
        let p = Scripted { files_untrusted: true, ..Default::default() };
        assert!(read_scratch(&scratch, "body.bin", &p, 100).unwrap_err().contains("non protetto"));
    }

    #[test]
    fn trust_chain_covers_the_pulisci_pc_folder() {
        let d = Path::new(r"C:\ProgramData\PulisciPC\winapp2");
        assert_eq!(
            trust_chain(d),
            vec![PathBuf::from(r"C:\ProgramData\PulisciPC"), d.to_path_buf()]
        );
        assert_eq!(trust_chain(Path::new(r"C:\x\regole")), vec![PathBuf::from(r"C:\x\regole")]);
    }
}
