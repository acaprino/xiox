//! Scaricamento e aggiornamento di `Winapp2.ini` (mai dentro l'exe).
//!
//! Il file vive in `%APPDATA%\PulisciPC\winapp2\` insieme a `etag.txt`. A ogni
//! avvio si chiede il file con `If-None-Match`: 304 = niente da fare; 200
//! valido sostituisce la copia in modo atomico (scrittura in `.tmp` + rename);
//! qualunque altra cosa (rete assente, HTML di errore, file troncato) lascia
//! la copia vecchia dov'è. Il download usa il `curl.exe` di Windows, senza
//! finestra e senza passare da una shell.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
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
    let text = std::str::from_utf8(bytes).map_err(|_| "il file non è UTF-8".to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut version: Option<String> = None;
    let mut sections = 0usize;
    let mut has_filekey = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            sections += 1;
        } else if version.is_none() && line.starts_with(';') {
            let rest = line.trim_start_matches(';').trim();
            if let Some(v) = rest.strip_prefix("Version:") {
                let v = v.trim();
                if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
                    version = Some(v.to_string());
                }
            }
        } else if !has_filekey && line.len() >= 7 && line.is_char_boundary(7)
            && line[..7].eq_ignore_ascii_case("FileKey")
        {
            has_filekey = true;
        }
    }
    let version = version.ok_or_else(|| "riga '; Version:' assente".to_string())?;
    if sections < MIN_SECTIONS {
        return Err(format!("solo {sections} sezioni"));
    }
    if !has_filekey {
        return Err("nessuna FileKey".into());
    }
    Ok(version)
}

/// Un giro di aggiornamento dentro `dir` (creata se manca).
pub fn update(dir: &Path, fetch: &dyn Fetch, url: &str) -> UpdateOutcome {
    let ini = dir.join(INI_NAME);
    let have_copy = ini.is_file();
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
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
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
            let tmp = dir.join(TMP_NAME);
            if let Err(e) = fs::write(&tmp, &bytes) {
                let _ = fs::remove_file(&tmp);
                return fail(format!("scrittura non riuscita: {e}"));
            }
            if let Err(e) = fs::rename(&tmp, &ini) {
                let _ = fs::remove_file(&tmp);
                return fail(format!("sostituzione non riuscita: {e}"));
            }
            match etag.filter(|s| !s.trim().is_empty()) {
                Some(t) => {
                    let _ = fs::write(dir.join(ETAG_NAME), t.trim());
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
/// UI né l'uscita dal programma. `on_done` gira sul thread di lavoro.
pub fn spawn_update(dir: PathBuf, on_done: impl FnOnce(UpdateOutcome) + Send + 'static) {
    spawn_update_with(dir, Arc::new(CurlFetch), WINAPP2_URL.to_string(), on_done);
}

/// Come [`spawn_update`] ma con sorgente e URL scelti da chi chiama.
pub fn spawn_update_with(
    dir: PathBuf,
    fetch: Arc<dyn Fetch>,
    url: String,
    on_done: impl FnOnce(UpdateOutcome) + Send + 'static,
) {
    let spawned = thread::Builder::new()
        .name("winapp2-update".into())
        .spawn(move || {
            let outcome = update(&dir, fetch.as_ref(), &url);
            on_done(outcome);
        });
    // Se il thread non parte, l'app va avanti senza aggiornamento.
    drop(spawned);
}

/// Scaricamento con il `curl.exe` di Windows: nessuna finestra, nessuna shell,
/// nessun dato dell'utente nei parametri (i file di lavoro stanno in una
/// cartella temporanea usata come directory corrente di curl).
pub struct CurlFetch;

static SEQ: AtomicU32 = AtomicU32::new(0);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> std::io::Result<ScratchDir> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "pulisci-curl-{}-{}-{}",
            std::process::id(),
            nanos,
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p)?;
        Ok(ScratchDir(p))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Fetch for CurlFetch {
    fn get(&self, url: &str, etag: Option<&str>, timeout: Duration) -> Result<Fetched, String> {
        let scratch = ScratchDir::new().map_err(|e| format!("cartella temporanea: {e}"))?;
        let err_file = fs::File::create(scratch.0.join("err.txt"))
            .map_err(|e| format!("file temporaneo: {e}"))?;

        let mut cmd = Command::new("curl.exe");
        cmd.current_dir(&scratch.0)
            .arg("-sS")
            .arg("-L")
            .arg("--max-time")
            .arg(timeout.as_secs().max(1).to_string())
            .arg("-D")
            .arg("headers.txt")
            .arg("-o")
            .arg("body.bin")
            .arg("-A")
            .arg("PulisciPC");
        if let Some(t) = etag {
            cmd.arg("-H").arg(format!("If-None-Match: {t}"));
        }
        cmd.arg(url)
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
            if let Ok(mut f) = fs::File::open(scratch.0.join("err.txt")) {
                let _ = f.by_ref().take(2000).read_to_string(&mut msg);
            }
            let msg = msg.trim();
            return Err(if msg.is_empty() {
                format!("curl terminato con {status}")
            } else {
                format!("curl: {msg}")
            });
        }

        let headers = fs::read_to_string(scratch.0.join("headers.txt"))
            .or_else(|_| fs::read(scratch.0.join("headers.txt")).map(|b| String::from_utf8_lossy(&b).into_owned()))
            .map_err(|e| format!("header non leggibili: {e}"))?;
        let (code, new_etag) = parse_headers(&headers)?;
        match code {
            304 => Ok(Fetched::NotModified),
            200 => {
                let bytes = fs::read(scratch.0.join("body.bin"))
                    .map_err(|e| format!("corpo non leggibile: {e}"))?;
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
        if line.len() >= 5 && line[..5].eq_ignore_ascii_case("HTTP/") {
            // Nuovo blocco: si riparte (l'ultimo vince).
            code = line.split_whitespace().nth(1).and_then(|c| c.parse().ok());
            etag = None;
        } else if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("etag")
        {
            etag = Some(value.trim().to_string());
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
