//! Scaricamento e aggiornamento di `Winapp2.ini` (mai dentro l'exe).
//!
//! Il file vive in `%APPDATA%\PulisciPC\winapp2\` insieme a `etag.txt`. A ogni
//! avvio si chiede il file con `If-None-Match`: 304 = niente da fare; 200
//! valido sostituisce la copia in modo atomico (scrittura in `.tmp` + rename);
//! qualunque altra cosa (rete assente, HTML di errore, file troncato) lascia
//! la copia vecchia dov'è. Il download usa il `curl.exe` di Windows, senza
//! finestra e senza passare da una shell.

use std::fs;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const WINAPP2_URL: &str =
    "https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini";

/// Tempo massimo di una richiesta (curl `--max-time` e guardiano esterno).
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Margine concesso a curl oltre `--max-time` prima di ucciderlo.
const WATCHDOG_GRACE: Duration = Duration::from_secs(3);

const MIN_BYTES: usize = 200 * 1024;
const MAX_BYTES: usize = 20 * 1024 * 1024;
const MIN_SECTIONS: usize = 1000;

const INI_NAME: &str = "Winapp2.ini";
const TMP_NAME: &str = "Winapp2.ini.tmp";
const ETAG_NAME: &str = "etag.txt";

/// Risposta di una richiesta condizionale.
pub enum Fetched {
    NotModified,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// Sorgente di rete sostituibile nei test.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str, etag: Option<&str>, timeout: Duration) -> Result<Fetched, String>;
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

/// Un giro di aggiornamento dentro `dir` (creata se manca).
pub fn update(dir: &Path, fetch: &dyn Fetch, url: &str) -> UpdateOutcome {
    let ini = dir.join(INI_NAME);
    // Una copia vale solo se si legge e passa `is_valid`: una copia rovinata
    // equivale a nessuna copia (niente If-None-Match, niente 304 a vuoto).
    let local_version = read_valid_local(&ini);
    let have_copy = local_version.is_some();
    let fail = |reason: String| {
        if have_copy {
            UpdateOutcome::KeptOld(reason)
        } else {
            UpdateOutcome::NoCopyYet(reason)
        }
    };
    if let Err(e) = fs::create_dir_all(dir) {
        return fail(format!("cartella dati non creabile: {e}"));
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
            let tmp = dir.join(TMP_NAME);
            if let Err(e) = write_synced(&tmp, &bytes) {
                let _ = fs::remove_file(&tmp);
                return fail(format!("scrittura non riuscita: {e}"));
            }
            if let Err(e) = fs::rename(&tmp, &ini) {
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

/// Avvia l'aggiornamento in un thread staccato (nessun `join`): non blocca la
/// UI né l'uscita dal programma. `on_done` gira sul thread di lavoro ed è
/// chiamata sempre una volta sola (anche se il lavoro va in panico o il thread
/// non riesce a partire).
pub fn spawn_update(dir: PathBuf, on_done: impl FnOnce(UpdateOutcome) + Send + 'static) {
    spawn_update_with(dir, Arc::new(CurlFetch), WINAPP2_URL.to_string(), on_done);
}

type DoneFn = Box<dyn FnOnce(UpdateOutcome) + Send>;
type DoneSlot = Arc<Mutex<Option<DoneFn>>>;

/// Come [`spawn_update`] ma con sorgente e URL scelti da chi chiama.
pub fn spawn_update_with(
    dir: PathBuf,
    fetch: Arc<dyn Fetch>,
    url: String,
    on_done: impl FnOnce(UpdateOutcome) + Send + 'static,
) {
    // La callback sta in uno slot condiviso: se il thread non parte la
    // riprende chi chiama, altrimenti la prende il thread (una volta sola).
    let slot: DoneSlot =
        Arc::new(Mutex::new(Some(Box::new(on_done))));
    let worker_slot = Arc::clone(&slot);
    let worker_dir = dir.clone();
    let spawned = thread::Builder::new()
        .name("winapp2-update".into())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| update(&worker_dir, fetch.as_ref(), &url)))
                .unwrap_or_else(|_| fallback_outcome(&worker_dir, "aggiornamento interrotto da un errore interno"));
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

/// Versione della copia locale se il file si legge e passa `is_valid`.
fn read_valid_local(ini: &Path) -> Option<String> {
    let len = fs::metadata(ini).ok()?.len();
    if len > MAX_BYTES as u64 {
        return None;
    }
    is_valid(&fs::read(ini).ok()?).ok()
}

/// `true` se `new` è un numero di versione strettamente minore di `old`.
fn is_older(new: &str, old: &str) -> bool {
    match (new.parse::<u64>(), old.parse::<u64>()) {
        (Ok(n), Ok(o)) => n < o,
        _ => false,
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Scaricamento con il `curl.exe` di Windows: percorso assoluto della cartella
/// di sistema, nessuna finestra, nessuna shell, nessun dato dell'utente nei
/// parametri (i file di lavoro stanno in una cartella temporanea usata come
/// directory corrente di curl).
pub struct CurlFetch;

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Cartelle di lavoro di curl rimaste da un'esecuzione interrotta.
const SCRATCH_PREFIX: &str = "pulisci-curl-";
const STALE_AFTER: Duration = Duration::from_secs(600);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> std::io::Result<ScratchDir> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
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

#[cfg(windows)]
fn system_directory() -> Option<PathBuf> {
    use std::ffi::OsString;
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
        let curl = curl_exe_path()?;
        remove_stale_scratch(&std::env::temp_dir(), STALE_AFTER);
        let scratch = ScratchDir::new().map_err(|e| format!("cartella temporanea: {e}"))?;
        let err_file = fs::File::create(scratch.0.join("err.txt"))
            .map_err(|e| format!("file temporaneo: {e}"))?;

        let mut cmd = Command::new(&curl);
        cmd.current_dir(&scratch.0)
            .args(curl_args(url, etag, timeout))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(err_file));
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
                        return Err("timeout: curl interrotto".into());
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
            let mut msg = String::new();
            if let Ok(f) = fs::File::open(scratch.0.join("err.txt")) {
                let _ = f.take(2000).read_to_string(&mut msg);
            }
            let msg = msg.trim();
            return Err(if msg.is_empty() {
                format!("curl terminato con {status}")
            } else {
                format!("curl: {msg}")
            });
        }

        let headers = fs::read(scratch.0.join("headers.txt"))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| format!("header non leggibili: {e}"))?;
        let (code, new_etag) = parse_headers(&headers)?;
        match code {
            304 => Ok(Fetched::NotModified),
            200 => {
                let body = scratch.0.join("body.bin");
                let len = fs::metadata(&body)
                    .map_err(|e| format!("corpo non leggibile: {e}"))?
                    .len();
                if len > MAX_BYTES as u64 {
                    return Err(format!("corpo troppo grande ({len} byte)"));
                }
                let bytes = fs::read(&body).map_err(|e| format!("corpo non leggibile: {e}"))?;
                Ok(Fetched::Body { bytes, etag: new_etag })
            }
            other => Err(format!("risposta HTTP {other}")),
        }
    }
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
        let out = update(dir.path(), &fake, "http://x/y");
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
        update(dir.path(), &fake, "u");
        let ini = dir.path().join("Winapp2.ini");
        let before = mtime(&ini);
        std::thread::sleep(Duration::from_millis(30));
        let out = update(dir.path(), &fake, "u");
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
        update(dir.path(), &fake, "u");
        let out = update(dir.path(), &fake, "u");
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
        update(dir.path(), &fake, "u");
        let out = update(dir.path(), &fake, "u");
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
            let out = update(dir.path(), &fake, "u");
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
        let out = update(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::NoCopyYet("curl: Could not resolve host".into()));
        assert!(!dir.path().join("Winapp2.ini").exists());
    }

    #[test]
    fn network_error_with_copy_keeps_old() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = valid_ini("260915");
        let fake = FakeFetch::new(vec![body(v1.clone(), Some("\"e\"")), Err("timeout".into())]);
        update(dir.path(), &fake, "u");
        let out = update(dir.path(), &fake, "u");
        assert_eq!(out, UpdateOutcome::KeptOld("timeout".into()));
        assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), v1);
    }

    #[test]
    fn etag_is_not_sent_without_a_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("etag.txt"), "\"stale\"").unwrap();
        let fake = FakeFetch::new(vec![Ok(Fetched::NotModified)]);
        let out = update(dir.path(), &fake, "u");
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
        spawn_update_with(dir.path().to_path_buf(), Arc::new(SlowFetch), "u".into(), move |o| {
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
        update(dir.path(), &fake, "u");
        fs::write(dir.path().join("etag.txt"), "\"e1\"\nX-Evil: 1").unwrap();
        let out = update(dir.path(), &fake, "u");
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
            let out = update(dir.path(), &fake, "u");
            assert_eq!(out, UpdateOutcome::Updated { version: "260915".into() });
            assert_eq!(fake.seen_etags.lock().unwrap()[0], None, "etag inviato con copia rovinata");
            assert_eq!(fs::read(dir.path().join("Winapp2.ini")).unwrap(), fresh);
        }
        // un 304 non lascia mai "UpToDate" su una copia rovinata
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Winapp2.ini"), b"").unwrap();
        fs::write(dir.path().join("etag.txt"), "\"old\"").unwrap();
        let fake = FakeFetch::new(vec![Ok(Fetched::NotModified)]);
        let out = update(dir.path(), &fake, "u");
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
        update(dir.path(), &fake, "u");
        fs::create_dir(dir.path().join("Winapp2.ini.tmp")).unwrap();
        let out = update(dir.path(), &fake, "u");
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
        update(dir.path(), &fake, "u");
        let out = update(dir.path(), &fake, "u");
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
        spawn_update_with(dir.path().to_path_buf(), Arc::new(PanicFetch), "u".into(), move |o| {
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

    /// Prova di rete vera (sola lettura): `cargo test -p pc-rules real_network -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_network_check() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/winapp2-updater-check");
        let _ = fs::remove_dir_all(&dir);
        let first = update(&dir, &CurlFetch, WINAPP2_URL);
        println!("primo giro: {first:?}");
        let second = update(&dir, &CurlFetch, WINAPP2_URL);
        println!("secondo giro: {second:?}");
        assert!(matches!(first, UpdateOutcome::Updated { .. }), "{first:?}");
        assert_eq!(second, UpdateOutcome::UpToDate);
    }
}
