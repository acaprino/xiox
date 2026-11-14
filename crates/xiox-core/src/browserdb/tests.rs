//! Test di `browserdb`: profili finti in `tempfile::tempdir()`, database SQLite
//! creati qui, processi finti. Mai il profilo vero del browser.

use super::*;
use std::collections::HashMap;
use std::fs;

use rusqlite::Connection;

// ----- ambiente e profili finti -----

fn env_in(root: &Path) -> Env {
    let p = root.join("profile");
    let s = |x: &Path| x.to_string_lossy().into_owned();
    Env(HashMap::from([
        ("userprofile".to_string(), s(&p)),
        ("appdata".to_string(), s(&p.join("AppData").join("Roaming"))),
        ("localappdata".to_string(), s(&p.join("AppData").join("Local"))),
    ]))
}

fn local(env: &Env) -> PathBuf {
    PathBuf::from(env.get("localappdata").unwrap())
}

fn roaming(env: &Env) -> PathBuf {
    PathBuf::from(env.get("appdata").unwrap())
}

const PREFS_JSON: &str = r#"{"download":{"default_directory":"C:\\Users\\x\\Scaricati","prompt_for_download":true},"profile":{"name":"Persona 1"},"savefile":{"default_directory":"C:\\Users\\x\\Desktop"},"zoom":{"level":1.2000000000000002}}"#;

/// Un profilo Chromium con `Preferences`; restituisce la cartella.
fn chromium_profile(user_data: &Path, name: &str) -> PathBuf {
    let dir = user_data.join(name);
    fs::create_dir_all(dir.join("Network")).unwrap();
    fs::write(dir.join("Preferences"), PREFS_JSON).unwrap();
    dir
}

fn chrome_user_data(env: &Env) -> PathBuf {
    local(env).join(r"Google\Chrome\User Data")
}

fn firefox_profiles(env: &Env) -> PathBuf {
    roaming(env).join(r"Mozilla\Firefox\Profiles")
}

const PREFS_JS: &str = "// Mozilla User Preferences\r\n\
user_pref(\"browser.download.lastDir\", \"C:\\\\Users\\\\x\\\\Desktop\");\r\n\
user_pref(\"browser.download.lastDir.savePerSite\", true);\r\n\
user_pref(\"browser.startup.page\", 3);\r\n";

fn gecko_profile(profiles: &Path, name: &str) -> PathBuf {
    let dir = profiles.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("prefs.js"), PREFS_JS).unwrap();
    dir
}

const COOKIE_HOSTS: [&str; 7] =
    ["google.com", ".google.com", "accounts.google.com", "evilgoogle.com", ".example.org", "Mail.Live.com", "live.com.evil.net"];

fn chromium_cookies(path: &Path, hosts: &[&str]) {
    let c = Connection::open(path).unwrap();
    c.execute_batch("CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR); CREATE TABLE cookies(creation_utc INTEGER NOT NULL, host_key TEXT NOT NULL, name TEXT NOT NULL, value TEXT NOT NULL);").unwrap();
    for (i, h) in hosts.iter().enumerate() {
        c.execute("INSERT INTO cookies VALUES (?1, ?2, 'n', 'v')", (i as i64, h)).unwrap();
    }
}

fn gecko_cookies(path: &Path, hosts: &[&str]) {
    let c = Connection::open(path).unwrap();
    c.execute_batch("CREATE TABLE moz_cookies(id INTEGER PRIMARY KEY, host TEXT, name TEXT, value TEXT);").unwrap();
    for h in hosts {
        c.execute("INSERT INTO moz_cookies(host, name, value) VALUES (?1, 'n', 'v')", [h]).unwrap();
    }
}

fn hosts_in(path: &Path, table: &str, col: &str) -> Vec<String> {
    let c = Connection::open(path).unwrap();
    let mut st = c.prepare(&format!("SELECT {col} FROM {table} ORDER BY {col}")).unwrap();
    st.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}

fn none() -> Vec<String> {
    Vec::new()
}

fn chrome_running() -> Vec<String> {
    vec!["explorer.exe".to_string(), "CHROME.EXE".to_string()]
}

fn go() -> AtomicBool {
    AtomicBool::new(false)
}

fn keep(list: &[&str]) -> Result<KeepList, String> {
    Ok(KeepList { domains: list.iter().map(|s| s.to_string()).collect(), rejected: vec![] })
}

// ----- profili -----

#[test]
fn profiles_are_found_for_chromium_and_gecko_layouts() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let ud = chrome_user_data(&env);
    chromium_profile(&ud, "Default");
    chromium_profile(&ud, "Profile 2");
    chromium_profile(&ud, "System Profile");
    chromium_profile(&ud, "Guest Profile");
    fs::create_dir_all(ud.join("Profile 3")).unwrap(); // senza Preferences: non è un profilo
    fs::write(ud.join("Local State"), "{}").unwrap();
    // Chrome Beta: variante con il nome nel resoconto.
    chromium_profile(&local(&env).join(r"Google\Chrome Beta\User Data"), "Default");
    // Opera: la cartella stessa è il profilo; Opera GX anche con i profili laterali.
    let opera = roaming(&env).join(r"Opera Software\Opera Stable");
    fs::create_dir_all(&opera).unwrap();
    fs::write(opera.join("Preferences"), "{}").unwrap();
    let gx = roaming(&env).join(r"Opera Software\Opera GX Stable");
    fs::create_dir_all(&gx).unwrap();
    fs::write(gx.join("Preferences"), "{}").unwrap();
    chromium_profile(&gx.join("_side_profiles"), "abc");
    // Opera recente: profili nelle sottocartelle (come `Opera Software\Opera*\*` di Winapp2).
    chromium_profile(&opera, "Default");
    // Edge, Brave e Vivaldi: stessa forma di Chrome.
    chromium_profile(&local(&env).join(r"Microsoft\Edge\User Data"), "Default");
    chromium_profile(&local(&env).join(r"BraveSoftware\Brave-Browser\User Data"), "Default");
    chromium_profile(&local(&env).join(r"Vivaldi\User Data"), "Default");
    // Cartelle con un nome simile che non sono browser: mai guardate.
    chromium_profile(&local(&env).join(r"Microsoft\EdgeWebView\User Data"), "Default");
    gecko_profile(&firefox_profiles(&env), "x1y2.default-release");
    fs::create_dir_all(firefox_profiles(&env).join("vuoto")).unwrap(); // né prefs.js né database
    gecko_profile(&roaming(&env).join(r"Thunderbird\Profiles"), "q9.default");

    let got: Vec<(String, Kind)> = profiles(&env).iter().map(|p| (p.label(), p.kind)).collect();
    let want = vec![
        ("Google Chrome (Default)".to_string(), Kind::Chromium),
        ("Google Chrome (Profile 2)".to_string(), Kind::Chromium),
        ("Google Chrome (Chrome Beta, Default)".to_string(), Kind::Chromium),
        ("Microsoft Edge (Default)".to_string(), Kind::Chromium),
        ("Brave (Default)".to_string(), Kind::Chromium),
        ("Vivaldi (Default)".to_string(), Kind::Chromium),
        ("Opera (main profile)".to_string(), Kind::Chromium),
        ("Opera (Default)".to_string(), Kind::Chromium),
        ("Opera GX (main profile)".to_string(), Kind::Chromium),
        ("Opera GX (side profile abc)".to_string(), Kind::Chromium),
        ("Mozilla Firefox (x1y2.default-release)".to_string(), Kind::Gecko),
        ("Thunderbird (q9.default)".to_string(), Kind::Gecko),
    ];
    assert_eq!(got, want);
    let all = profiles(&env);
    let p = &all[0];
    assert_eq!(p.browser, "Google Chrome");
    assert_eq!(p.dir, ud.join("Default"));
    assert_eq!(p.processes, &["chrome.exe"]);
    assert_eq!(p.lock, ud.join("lockfile"), "il lockfile di Chromium sta in `User Data`");
    let edge = all.iter().find(|p| p.browser == "Microsoft Edge").unwrap();
    assert_eq!(edge.processes, &["msedge.exe"]);
    let side = all.iter().find(|p| p.label() == "Opera GX (side profile abc)").unwrap();
    assert_eq!(side.lock, gx.join("lockfile"));
    assert_eq!(side.processes, &["opera.exe"]);
    let ff = all.iter().find(|p| p.kind == Kind::Gecko).unwrap();
    assert_eq!(ff.lock, ff.dir.join("parent.lock"));
    assert_eq!(ff.processes, &["firefox.exe"]);
}

#[test]
fn profile_folders_behind_a_junction_are_never_used() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let elsewhere = t.path().join("altrove");
    chromium_profile(&elsewhere, "Default");
    let ud = chrome_user_data(&env);
    fs::create_dir_all(&ud).unwrap();
    let st = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(ud.join("Default"))
        .arg(elsewhere.join("Default"))
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    assert!(profiles(&env).is_empty());
    // Anche `User Data` intera dietro una giunzione.
    let ff_real = t.path().join("altrove_ff");
    gecko_profile(&ff_real, "a.default-release");
    fs::create_dir_all(roaming(&env).join("Mozilla").join("Firefox")).unwrap();
    junction(&firefox_profiles(&env), &ff_real);
    assert!(profiles(&env).is_empty());
}

fn junction(link: &Path, target: &Path) {
    let st = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
}

// ----- lista dei cookie -----

#[test]
fn keep_list_ignores_comments_blank_lines_case_and_leading_dot() {
    let text = " Google.COM \n\n.live.com\n# x\n   # commento\nexample.org # in fondo\nhttps://Accounts.Example.NET:443/login\n*.wild.it\ngoogle.com\nnon valido\u{FFFD}.com\n";
    let list = parse_keep_list(text);
    assert_eq!(list.domains, vec!["google.com", "live.com", "example.org", "accounts.example.net", "wild.it"]);
    assert_eq!(list.rejected, vec!["non valido\u{FFFD}.com"], "la riga scartata si dice");
    assert_eq!(parse_keep_list(KEEP_LIST_TEMPLATE), KeepList::default(), "gli esempi sono commentati");
    // Domini internazionali: i browser salvano l'host in punycode.
    let idn = parse_keep_list("Bücher.de\nhttps://www.münchen.de/x\n");
    assert_eq!(idn.domains, vec!["xn--bcher-kva.de", "www.xn--mnchen-3ya.de"]);
    assert!(idn.rejected.is_empty());
    assert!(is_kept(".xn--bcher-kva.de", &idn.domains));
    assert!(is_kept("shop.xn--bcher-kva.de", &idn.domains));
    assert!(is_kept(".google.com", &["google.com".to_string()]));
    assert!(is_kept("ACCOUNTS.google.com", &["google.com".to_string()]));
    assert!(!is_kept("evilgoogle.com", &["google.com".to_string()]));
    assert!(!is_kept("google.com.evil.net", &["google.com".to_string()]));
}

/// Chi aveva già la lista in `Documenti\PulisciPC` la ritrova in `Documenti\Xiox` (cartella rinominata), oppure
/// dov'era se la rinomina non riesce: i cookie scelti dall'utente non si perdono di vista.
#[test]
fn keep_list_of_the_old_folder_is_found_after_the_rename() {
    let t = tempfile::tempdir().unwrap();
    let old = t.path().join("PulisciPC");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("cookie_da_conservare.txt"), "google.com\n").unwrap();
    let path = keep_list_in(t.path());
    assert_eq!(path, t.path().join(r"Xiox\cookie_da_conservare.txt"));
    assert!(!old.exists());
    assert_eq!(read_keep_list(&path).unwrap().domains, vec!["google.com".to_string()]);

    // Nome nuovo occupato da un file: resta in uso la cartella vecchia, con la lista dentro.
    let t = tempfile::tempdir().unwrap();
    let old = t.path().join("PulisciPC");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("cookie_da_conservare.txt"), "google.com\n").unwrap();
    fs::write(t.path().join("Xiox"), "x").unwrap();
    let path = keep_list_in(t.path());
    assert_eq!(path, old.join("cookie_da_conservare.txt"));
    assert_eq!(read_keep_list(&path).unwrap().domains, vec!["google.com".to_string()]);
}

#[test]
fn empty_or_missing_keep_list_protects_nothing_and_says_so() {
    let t = tempfile::tempdir().unwrap();
    let path = keep_list_in(t.path());
    assert_eq!(path, t.path().join(r"Xiox\cookie_da_conservare.txt"));
    // Assente.
    assert_eq!(read_keep_list(&path), Ok(KeepList::default()));
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "list: 0 domains");
    // Creata al primo uso con gli esempi commentati, mai sovrascritta.
    ensure_keep_list(&path).unwrap();
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "list: 0 domains");
    fs::write(&path, "google.com\n").unwrap();
    ensure_keep_list(&path).unwrap();
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "list: 1 domain");
    // Vuota, o con soli commenti e righe vuote.
    fs::write(&path, "").unwrap();
    assert_eq!(read_keep_list(&path), Ok(KeepList::default()));
    fs::write(&path, "# google.com\r\n\r\n   \n").unwrap();
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "list: 0 domains");
    // UTF-8 con BOM.
    fs::write(&path, b"\xEF\xBB\xBFgoogle.com\n").unwrap();
    assert_eq!(read_keep_list(&path).unwrap().domains, vec!["google.com"]);
    // UTF-16 con BOM (Blocco note «Unicode»), little e big endian.
    let mut wide = vec![0xFF, 0xFE];
    wide.extend("google.com\r\nlive.com\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&path, wide).unwrap();
    assert_eq!(read_keep_list(&path).unwrap().domains, vec!["google.com", "live.com"]);
    let mut wide = vec![0xFE, 0xFF];
    wide.extend("live.com\n".encode_utf16().flat_map(u16::to_be_bytes));
    fs::write(&path, wide).unwrap();
    assert_eq!(read_keep_list(&path).unwrap().domains, vec!["live.com"]);
    // Illeggibile (una cartella): errore, e la finestra dice che non si tocca niente.
    let dir = t.path().join("cartella");
    fs::create_dir_all(&dir).unwrap();
    let err = read_keep_list(&dir);
    assert!(err.is_err());
    assert!(keep_list_summary(&err).contains("no cookie will be deleted"), "{}", keep_list_summary(&err));

    // L'analisi con la lista vuota propone tutti i cookie e lo dice.
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    chromium_cookies(&prof.join(r"Network\Cookies"), &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert!(found.detail[0].contains("list: 0 domains"), "{:?}", found.detail);
    assert_eq!(found.count, COOKIE_HOSTS.len() as u64);
    // Lista illeggibile: nessun cookie proposto.
    let found = analyze_cookies(&profiles(&env), Err("lista illeggibile".into()), &none, &go());
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
    assert!(found.detail[0].contains("no cookie will be deleted"), "{:?}", found.detail);
}

#[test]
fn keep_list_that_cannot_be_trusted_blocks_the_cookies() {
    let t = tempfile::tempdir().unwrap();
    let path = keep_list_in(t.path());
    ensure_keep_list(&path).unwrap();
    let blocked = |bytes: &[u8]| {
        fs::write(&path, bytes).unwrap();
        let list = read_keep_list(&path);
        assert!(list.is_err(), "{list:?}");
        let s = keep_list_summary(&list);
        assert!(s.starts_with("Cookie list not valid: no cookie will be deleted"), "{s}");
        s
    };
    // UTF-16 senza BOM: ogni carattere ha uno zero accanto.
    let wide: Vec<u8> = "google.com\r\nlive.com\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    blocked(&wide);
    // Non UTF-8 (per esempio ANSI con lettere accentate).
    blocked(b"# conserva i siti \xE0 \xE8\nlive.com\n");
    // Righe scritte ma nessun dominio valido: le righe compaiono nella conferma.
    let s = blocked(b"google,com\nfoo bar\n");
    assert!(s.contains("google,com") && s.contains("foo bar"), "{s}");
    // Qualche dominio valido e righe scartate: si pulisce, ma la conferma le elenca (al più 5).
    fs::write(&path, "google.com\na b\nc d\ne f\ng h\ni j\nk l\nm n\n").unwrap();
    let list = read_keep_list(&path);
    assert_eq!(list.as_ref().unwrap().domains, vec!["google.com"]);
    let s = keep_list_summary(&list);
    assert!(s.starts_with("list: 1 domain"), "{s}");
    assert!(s.contains("a b, c d, e f, g h, i j") && s.contains("…and 2 more") && !s.contains("k l"), "{s}");

    // L'analisi con una lista non valida non propone cookie.
    fs::write(&path, wide).unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    chromium_cookies(&prof.join(r"Network\Cookies"), &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), read_keep_list(&path), &none, &go());
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
    assert!(found.detail[0].contains("Cookie list not valid"), "{:?}", found.detail);
}

#[test]
fn keep_list_behind_a_junction_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let docs = t.path().join("Documenti");
    let elsewhere = t.path().join("altrove");
    fs::create_dir_all(&docs).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    junction(&docs.join("Xiox"), &elsewhere);
    let path = keep_list_in(&docs);
    let launched = std::cell::Cell::new(0);
    let launch = |_: &Path| {
        launched.set(launched.get() + 1);
        Ok(())
    };
    let err = open_keep_list_in(&docs, &launch).unwrap_err();
    assert!(err.contains("cookie list not safe"), "{err}");
    assert!(ensure_keep_list(&path).is_err());
    let read = read_keep_list(&path);
    assert!(read.as_ref().is_err_and(|e| e.contains("not safe")), "{read:?}");
    assert_eq!(launched.get(), 0);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "nessun file creato nella destinazione");

    // Cartella normale: la lista si crea e l'editor riceve il suo percorso.
    let docs = t.path().join("Documenti2");
    fs::create_dir_all(&docs).unwrap();
    let seen = std::cell::RefCell::new(Vec::new());
    open_keep_list_in(&docs, &|p: &Path| {
        seen.borrow_mut().push(p.to_path_buf());
        Ok(())
    })
    .unwrap();
    assert_eq!(*seen.borrow(), vec![keep_list_in(&docs)]);
    assert_eq!(keep_list_summary(&read_keep_list(&keep_list_in(&docs))), "list: 0 domains");
    // Il comando dell'editor: notepad.exe della cartella di sistema, percorso fra virgolette.
    let (exe, arg) = editor_command(Path::new(r"C:\Windows\System32"), &keep_list_in(&docs));
    assert_eq!(exe, Path::new(r"C:\Windows\System32\notepad.exe"));
    assert_eq!(arg, format!("\"{}\"", keep_list_in(&docs).display()));
}

#[test]
fn cookies_outside_the_keep_list_are_deleted_chromium() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let ud = chrome_user_data(&env);
    let new = chromium_profile(&ud, "Default");
    chromium_cookies(&new.join(r"Network\Cookies"), &COOKIE_HOSTS);
    // Versioni vecchie: `Cookies` nella cartella del profilo.
    let old = chromium_profile(&ud, "Profile 2");
    chromium_cookies(&old.join("Cookies"), &COOKIE_HOSTS);

    let list = keep(&["google.com", "live.com"]);
    let found = analyze_cookies(&profiles(&env), list, &none, &go());
    assert_eq!(found.count, 6, "{:?}", found.detail);
    assert!(found.detail[0].contains("list: 2 domains"), "{:?}", found.detail);
    let done = clean_cookies(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 6, "{:?}", done.notes);
    assert_eq!(done.failed, 0);
    // Quante righe e quanti byte, per ogni database.
    assert_eq!(done.done.len(), 2, "{:?}", done.done);
    assert!(done.done.iter().all(|d| d.contains("3 cookies deleted") && d.contains("freed in the database")), "{:?}", done.done);
    // Rifatto con la stessa lista: niente più da eliminare.
    let again = analyze_cookies(&profiles(&env), keep(&["google.com", "live.com"]), &none, &go());
    assert_eq!(again.count, 0);
    for db in [new.join(r"Network\Cookies"), old.join("Cookies")] {
        assert_eq!(hosts_in(&db, "cookies", "host_key"), vec![".google.com", "Mail.Live.com", "accounts.google.com", "google.com"]);
    }
}

#[test]
fn cookies_outside_the_keep_list_are_deleted_gecko() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = prof.join("cookies.sqlite");
    gecko_cookies(&db, &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&["google.com"]), &none, &go());
    assert_eq!(found.count, 4);
    let done = clean_cookies(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 4, "{:?}", done.notes);
    assert_eq!(hosts_in(&db, "moz_cookies", "host"), vec![".google.com", "accounts.google.com", "google.com"]);
}

#[test]
fn cookie_clean_uses_the_list_read_by_the_analysis() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = prof.join("cookies.sqlite");
    gecko_cookies(&db, &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&["google.com", "example.org"]), &none, &go());
    assert!(found.items.contains(&format!("{KEEP_PREFIX}example.org")));
    // La pulizia non rilegge il file della lista (che intanto potrebbe essere
    // cambiato): usa i domini che l'analisi ha scritto in `items`.
    let done = clean_cookies(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 3, "{:?}", done.notes);
    let left = hosts_in(&db, "moz_cookies", "host");
    assert!(left.contains(&".example.org".to_string()) && left.contains(&"accounts.google.com".to_string()), "{left:?}");
}

// ----- compattazione -----

/// Database con pagine libere: righe grandi inserite e poi cancellate.
fn fragmented(path: &Path) -> i64 {
    let c = Connection::open(path).unwrap();
    c.execute_batch("PRAGMA page_size = 4096; CREATE TABLE a(b BLOB); CREATE TABLE keep(x TEXT); INSERT INTO keep VALUES ('resta');").unwrap();
    for _ in 0..60 {
        c.execute("INSERT INTO a VALUES (zeroblob(4000))", []).unwrap();
    }
    c.execute("DELETE FROM a", []).unwrap();
    let free: i64 = c.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
    assert!(free > 50);
    free * 4096
}

#[test]
fn compact_reports_recoverable_bytes_from_freelist() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    let history = prof.join("History");
    let free = fragmented(&history);
    // Un database senza pagine libere non si propone.
    let c = Connection::open(prof.join("Web Data")).unwrap();
    c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);").unwrap();
    drop(c);
    let found = analyze_compact(&profiles(&env), &none, &go());
    assert_eq!(found.bytes, free as u64);
    assert_eq!(found.count, 1);
    assert_eq!(found.items, vec![history.to_string_lossy().into_owned()]);
    let before = fs::metadata(&history).unwrap().len();
    let done = clean_compact(&profiles(&env), &found, &none, &go());
    let after = fs::metadata(&history).unwrap().len();
    assert_eq!(done.removed, 1, "{:?}", done.notes);
    assert!(after < before);
    assert_eq!(done.freed, before - after);
    assert!(done.done[0].contains("History"), "{:?}", done.done);
    let c = Connection::open(&history).unwrap();
    let kept: String = c.query_row("SELECT x FROM keep", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, "resta");
}

#[test]
fn compact_runs_only_after_integrity_check_ok() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    let history = prof.join("History");
    fragmented(&history);
    // Rovinato di proposito: l'intestazione della pagina radice di `keep`.
    let root: i64 = {
        let c = Connection::open(&history).unwrap();
        c.query_row("SELECT rootpage FROM sqlite_master WHERE name = 'keep'", [], |r| r.get(0)).unwrap()
    };
    let mut bytes = fs::read(&history).unwrap();
    let at = (root as usize - 1) * 4096;
    bytes[at..at + 12].fill(0xFF);
    fs::write(&history, &bytes).unwrap();

    let found = analyze_compact(&profiles(&env), &none, &go());
    assert_eq!(found.count, 1, "{:?}", found.detail);
    let done = clean_compact(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 0);
    assert_eq!(done.freed, 0);
    assert!(done.notes.iter().any(|n| n.contains("integrity check")), "{:?}", done.notes);
    assert_eq!(fs::read(&history).unwrap(), bytes, "il database rovinato non si tocca");
}

// ----- cronologia di Firefox -----

const PLACES_SCHEMA: &str = "
    CREATE TABLE moz_origins (id INTEGER PRIMARY KEY, prefix TEXT NOT NULL, host TEXT NOT NULL, frecency INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, visit_count INTEGER DEFAULT 0,
        typed INTEGER DEFAULT 0 NOT NULL, last_visit_date INTEGER, foreign_count INTEGER DEFAULT 0 NOT NULL, origin_id INTEGER);
    CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, from_visit INTEGER, place_id INTEGER, visit_date INTEGER, visit_type INTEGER);
    CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL, parent INTEGER, title LONGVARCHAR);
    CREATE TABLE moz_anno_attributes (id INTEGER PRIMARY KEY, name VARCHAR(32) UNIQUE NOT NULL);
    CREATE TABLE moz_annos (id INTEGER PRIMARY KEY, place_id INTEGER NOT NULL, anno_attribute_id INTEGER, content LONGVARCHAR);
    CREATE TABLE moz_inputhistory (place_id INTEGER NOT NULL, input LONGVARCHAR NOT NULL, use_count INTEGER, PRIMARY KEY (place_id, input));
    CREATE TABLE moz_keywords (id INTEGER PRIMARY KEY AUTOINCREMENT, keyword TEXT UNIQUE, place_id INTEGER, post_data TEXT);
    CREATE TABLE moz_places_metadata (id INTEGER PRIMARY KEY, place_id INTEGER NOT NULL, total_view_time INTEGER NOT NULL DEFAULT 0);
    INSERT INTO moz_origins VALUES (1, 'https://', 'www.mozilla.org', 1), (2, 'https://', 'example.com', 1), (3, 'https://', 'segreto.net', 1);
    INSERT INTO moz_places VALUES
        (1, 'https://www.mozilla.org/', 'Mozilla', 5, 1, 1700000000000000, 1, 1),
        (2, 'https://example.com/a', 'A', 2, 0, 1700000000000001, 0, 2),
        (3, 'https://segreto.net/x', 'X', 1, 1, 1700000000000002, 0, 3),
        (4, 'https://example.com/file.zip', 'file.zip', 1, 0, 1700000000000003, 0, 2),
        (5, 'https://example.com/segnalibro', 'S', 0, 0, NULL, 0, 2),
        (6, 'https://example.com/cerca?q=%s', 'parola chiave', 0, 0, NULL, 0, 2);
    INSERT INTO moz_keywords (keyword, place_id) VALUES ('ex', 6);
    INSERT INTO moz_places_metadata VALUES (1, 2, 100), (2, 1, 50);
    INSERT INTO moz_historyvisits VALUES (1, 0, 1, 1, 1), (2, 0, 2, 2, 1), (3, 0, 3, 3, 2), (4, 0, 4, 4, 7);
    INSERT INTO moz_bookmarks VALUES (1, 2, NULL, 0, 'radice'), (2, 1, 1, 1, 'Mozilla'), (3, 1, 5, 1, 'Segnalibro con foreign_count sbagliato');
    INSERT INTO moz_anno_attributes VALUES (1, 'downloads/destinationFileURI'), (2, 'downloads/metaData'), (3, 'bookmarkProperties/description');
    INSERT INTO moz_annos VALUES (1, 4, 1, 'file:///C:/x/file.zip'), (2, 1, 2, '{}'), (3, 1, 3, 'descrizione');
    INSERT INTO moz_inputhistory VALUES (2, 'exa', 1);
";

fn places(prof: &Path) -> PathBuf {
    let db = prof.join("places.sqlite");
    let c = Connection::open(&db).unwrap();
    // Firefox tiene places.sqlite in WAL: alla chiusura `-wal` e `-shm` spariscono.
    c.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    c.execute_batch(PLACES_SCHEMA).unwrap();
    drop(c);
    db
}

fn ids(c: &Connection, sql: &str) -> Vec<i64> {
    let mut st = c.prepare(sql).unwrap();
    st.query_map([], |r| r.get::<_, i64>(0)).unwrap().map(Result::unwrap).collect()
}

fn bookmarks(c: &Connection) -> Vec<(i64, Option<i64>, String)> {
    let mut st = c.prepare("SELECT id, fk, title FROM moz_bookmarks ORDER BY id").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn firefox_history_keeps_bookmarks() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = places(&prof);
    let before = bookmarks(&Connection::open(&db).unwrap());

    let found = analyze_history(&profiles(&env), &none, &go());
    assert_eq!(found.count, 4 + 3, "4 visite e 3 pagine: {:?}", found.detail);
    let done = clean_history(&profiles(&env), &found, &none, &go());
    assert_eq!(done.failed, 0, "{:?}", done.notes);
    assert_eq!(done.removed, 7);
    assert!(done.done[0].contains("4 visits and 3 pages removed") && done.done[0].contains("freed in the database"), "{:?}", done.done);

    let c = Connection::open(&db).unwrap();
    assert_eq!(bookmarks(&c), before);
    assert!(ids(&c, "SELECT id FROM moz_historyvisits").is_empty());
    assert_eq!(ids(&c, "SELECT id FROM moz_places ORDER BY id"), vec![1, 5, 6], "segnalibri e parole chiave restano");
    assert_eq!(ids(&c, "SELECT id FROM moz_keywords"), vec![1]);
    assert_eq!(ids(&c, "SELECT id FROM moz_places_metadata ORDER BY id"), vec![2], "metadati delle pagine tolte via");
    assert_eq!(ids(&c, "SELECT id FROM moz_annos ORDER BY id"), vec![3], "download e annotazioni orfane via");
    assert_eq!(ids(&c, "SELECT id FROM moz_origins ORDER BY id"), vec![1, 2]);
    assert!(ids(&c, "SELECT place_id FROM moz_inputhistory").is_empty());
    let (visits, last): (i64, Option<i64>) =
        c.query_row("SELECT visit_count, last_visit_date FROM moz_places WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((visits, last), (0, None));
}

#[test]
fn firefox_history_rolls_back_if_bookmark_count_changes() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = places(&prof);
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TRIGGER finto AFTER DELETE ON moz_historyvisits BEGIN DELETE FROM moz_bookmarks WHERE id = 3; END;")
        .unwrap();
    let found = analyze_history(&profiles(&env), &none, &go());
    let done = clean_history(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 0);
    assert_eq!(done.failed, 1);
    assert!(done.notes.iter().any(|n| n.contains("bookmarks")), "{:?}", done.notes);
    let c = Connection::open(&db).unwrap();
    assert_eq!(bookmarks(&c).len(), 3);
    assert_eq!(ids(&c, "SELECT id FROM moz_historyvisits ORDER BY id"), vec![1, 2, 3, 4]);
    assert_eq!(ids(&c, "SELECT id FROM moz_places ORDER BY id"), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn firefox_history_rolls_back_if_a_bookmark_loses_its_page() {
    // Stesso numero di segnalibri, ma uno resta senza la sua pagina: è un
    // segnalibro perso anche se `moz_bookmarks` non cambia.
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = places(&prof);
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TRIGGER finto AFTER DELETE ON moz_historyvisits BEGIN DELETE FROM moz_places WHERE id = 1; END;")
        .unwrap();
    let found = analyze_history(&profiles(&env), &none, &go());
    let done = clean_history(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 0);
    assert_eq!(done.failed, 1);
    assert!(done.notes.iter().any(|n| n.contains("bookmarks")), "{:?}", done.notes);
    let c = Connection::open(&db).unwrap();
    assert_eq!(ids(&c, "SELECT id FROM moz_places ORDER BY id"), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn places_without_the_firefox_tables_are_never_touched() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = prof.join("places.sqlite");
    Connection::open(&db).unwrap().execute_batch("CREATE TABLE moz_places(id INTEGER PRIMARY KEY); INSERT INTO moz_places VALUES (1);").unwrap();
    let bytes = fs::read(&db).unwrap();
    let found = analyze_history(&profiles(&env), &none, &go());
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
    assert_eq!(found.skipped, 1);
    assert!(found.detail.iter().any(|d| d.contains("not recognized")), "{:?}", found.detail);
    assert_eq!(fs::read(&db).unwrap(), bytes);
}

// ----- ultima cartella di download -----

#[test]
fn last_download_dir_removed_from_preferences_and_prefs_js() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let chrome = chromium_profile(&chrome_user_data(&env), "Default");
    let ff = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let found = analyze_download_dir(&profiles(&env), &none, &go());
    assert_eq!(found.count, 2, "{:?}", found.detail);
    let done = clean_download_dir(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 2, "{:?}", done.notes);
    assert_eq!(done.failed, 0);

    let text = fs::read_to_string(chrome.join("Preferences")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(v["savefile"].get("default_directory").is_none());
    // La cartella dei download scelta nelle impostazioni non è «l'ultimo
    // percorso»: resta com'è.
    assert_eq!(v["download"]["default_directory"], r"C:\Users\x\Scaricati");
    assert_eq!(v["download"]["prompt_for_download"], serde_json::json!(true));
    // Stesso ordine delle chiavi e stessi numeri di prima.
    let mut want: serde_json::Value = serde_json::from_str(PREFS_JSON).unwrap();
    want["savefile"].as_object_mut().unwrap().remove("default_directory");
    assert_eq!(text, serde_json::to_string(&want).unwrap());
    assert_eq!(v["profile"]["name"], "Persona 1");
    assert_eq!(v["zoom"]["level"].as_f64(), Some(1.2000000000000002));
    let js = fs::read_to_string(ff.join("prefs.js")).unwrap();
    assert_eq!(
        js,
        "// Mozilla User Preferences\r\nuser_pref(\"browser.download.lastDir.savePerSite\", true);\r\nuser_pref(\"browser.startup.page\", 3);\r\n"
    );
    // Nessun file temporaneo lasciato.
    for dir in [&chrome, &ff] {
        assert!(fs::read_dir(dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }
    // Rifatto: niente da fare (le chiavi non ci sono più, come in un browser che non le ha).
    let again = analyze_download_dir(&profiles(&env), &none, &go());
    assert_eq!(again.count, 0);
    assert!(again.items.is_empty());
}

#[test]
fn preferences_that_are_not_json_are_never_rewritten() {
    assert_eq!(strip_chromium_prefs(b"{\"download\": {\"default_directory\": \"x\"}"), Err("not valid JSON".to_string()));
    assert_eq!(strip_chromium_prefs(b"[1, 2]"), Err("not a JSON object".to_string()));
    assert_eq!(strip_chromium_prefs(b"{\"download\": \"x\"}"), Ok(None));
    assert_eq!(strip_chromium_prefs(b"{\"savefile\": \"x\"}"), Ok(None));
    assert_eq!(strip_chromium_prefs(b"{\"download\": {\"default_directory\": \"x\"}}"), Ok(None));
    let (out, n) = strip_chromium_prefs(PREFS_JSON.as_bytes()).unwrap().unwrap();
    assert_eq!(n, 1);
    assert!(serde_json::from_slice::<serde_json::Value>(&out).is_ok());
    assert_eq!(strip_prefs_js(b"user_pref(\"browser.download.lastDir.savePerSite\", true);\n"), None);
    let (out, n) = strip_prefs_js(b"user_pref(\"a\", 1);\n  user_pref(\"browser.download.lastDir\" , \"C:\\\\x\");\nuser_pref(\"b\", 2);").unwrap();
    assert_eq!(n, 1);
    assert_eq!(out, b"user_pref(\"a\", 1);\nuser_pref(\"b\", 2);");
}

// ----- browser aperto -----

#[test]
fn database_of_a_running_browser_is_never_opened() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    // Se l'analisi lo aprisse direbbe «non leggibile», non «chiudi».
    fs::write(prof.join(r"Network\Cookies"), b"non sono un database").unwrap();
    fs::write(prof.join("History"), b"non sono un database").unwrap();
    for found in [
        analyze_cookies(&profiles(&env), keep(&[]), &chrome_running, &go()),
        analyze_compact(&profiles(&env), &chrome_running, &go()),
        analyze_download_dir(&profiles(&env), &chrome_running, &go()),
    ] {
        assert_eq!(found.count, 0);
        assert_eq!(found.skipped, 1);
        assert!(found.items.is_empty());
        assert!(found.detail.iter().any(|d| d.contains("Google Chrome (Default): close chrome.exe to clean")), "{:?}", found.detail);
        assert!(!found.detail.iter().any(|d| d.contains("not readable")), "{:?}", found.detail);
    }

    // Pulizia con un `found` dell'analisi fatta a browser chiuso: il browser
    // nel frattempo si è aperto, quindi nessun file si tocca.
    let cookies = prof.join(r"Network\Cookies");
    fs::remove_file(&cookies).unwrap();
    chromium_cookies(&cookies, &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert_eq!(found.count, COOKIE_HOSTS.len() as u64);
    let bytes = fs::read(&cookies).unwrap();
    let prefs = fs::read(prof.join("Preferences")).unwrap();
    let done = clean_cookies(&profiles(&env), &found, &chrome_running, &go());
    assert_eq!(done.removed, 0);
    assert!(done.notes.iter().any(|n| n.contains("close chrome.exe to clean")), "{:?}", done.notes);
    let found = analyze_download_dir(&profiles(&env), &none, &go());
    let done = clean_download_dir(&profiles(&env), &found, &chrome_running, &go());
    assert_eq!(done.removed, 0);
    assert_eq!(fs::read(&cookies).unwrap(), bytes);
    assert_eq!(fs::read(prof.join("Preferences")).unwrap(), prefs);
}

#[test]
fn profile_of_a_running_browser_is_skipped_but_other_profiles_are_cleaned() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let chrome = chromium_profile(&chrome_user_data(&env), "Default");
    let chrome_db = chrome.join(r"Network\Cookies");
    chromium_cookies(&chrome_db, &COOKIE_HOSTS);
    let ff = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let ff_db = ff.join("cookies.sqlite");
    gecko_cookies(&ff_db, &COOKIE_HOSTS);
    let chrome_bytes = fs::read(&chrome_db).unwrap();

    // Chrome aperto, Firefox chiuso.
    let found = analyze_cookies(&profiles(&env), keep(&["google.com"]), &chrome_running, &go());
    assert_eq!(found.count, 4, "solo i cookie di Firefox: {:?}", found.detail);
    assert_eq!(found.skipped, 1);
    assert_eq!(found.items, vec![ff_db.to_string_lossy().into_owned(), format!("{KEEP_PREFIX}google.com")]);
    assert!(found.detail.iter().any(|d| d == "Google Chrome (Default): close chrome.exe to clean"), "{:?}", found.detail);
    let done = clean_cookies(&profiles(&env), &found, &chrome_running, &go());
    assert_eq!(done.removed, 4, "{:?}", done.notes);
    assert_eq!(hosts_in(&ff_db, "moz_cookies", "host"), vec![".google.com", "accounts.google.com", "google.com"]);
    assert_eq!(fs::read(&chrome_db).unwrap(), chrome_bytes, "il database di Chrome non si apre");

    // E al contrario: Firefox aperto, la sua cronologia non si apre.
    let db = places(&ff);
    let ff_running = || vec!["Firefox.EXE".to_string()];
    let found = analyze_history(&profiles(&env), &ff_running, &go());
    assert_eq!(found.count, 0);
    assert_eq!(found.skipped, 1);
    assert!(found.detail.iter().any(|d| d.contains("Mozilla Firefox (a.default-release): close firefox.exe to clean")), "{:?}", found.detail);
    // Un `found` fatto a browser chiuso non serve se intanto Firefox si è aperto.
    let found = analyze_history(&profiles(&env), &none, &go());
    let before = fs::read(&db).unwrap();
    let done = clean_history(&profiles(&env), &found, &ff_running, &go());
    assert_eq!(done.removed, 0);
    assert!(done.notes.iter().any(|n| n.contains("close firefox.exe to clean")), "{:?}", done.notes);
    assert_eq!(fs::read(&db).unwrap(), before);
}

/// Tiene aperto `path` come fa il browser: Chromium il `lockfile` di `User
/// Data` (`FILE_SHARE_READ`), Firefox il `parent.lock` (nessuna condivisione).
fn hold(path: &Path, share: u32) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(share).open(path).unwrap()
}

#[test]
fn profile_whose_lock_is_held_is_skipped_even_with_an_unknown_process_name() {
    // Il nome dell'eseguibile di alcuni browser è da verificare: se il browser
    // gira con un nome diverso, il suo file di blocco lo dice lo stesso.
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let ud = chrome_user_data(&env);
    let chrome = chromium_profile(&ud, "Default");
    chromium_cookies(&chrome.join(r"Network\Cookies"), &COOKIE_HOSTS);
    let ff = gecko_profile(&firefox_profiles(&env), "a.default-release");
    gecko_cookies(&ff.join("cookies.sqlite"), &COOKIE_HOSTS);

    // Firefox lascia `parent.lock` anche da chiuso: un file non tenuto aperto non blocca.
    fs::write(ff.join("parent.lock"), b"").unwrap();
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert_eq!(found.count, 2 * COOKIE_HOSTS.len() as u64, "{:?}", found.detail);
    assert_eq!(found.skipped, 0);

    const FILE_SHARE_READ: u32 = 1;
    let lock_chrome = hold(&ud.join("lockfile"), FILE_SHARE_READ);
    let lock_ff = hold(&ff.join("parent.lock"), 0);
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert_eq!(found.count, 0, "{:?}", found.detail);
    assert_eq!(found.skipped, 2);
    assert!(found.detail.iter().any(|d| d.contains("Google Chrome (Default): profile in use, close chrome.exe to clean")), "{:?}", found.detail);
    assert!(found.detail.iter().any(|d| d.contains("Mozilla Firefox (a.default-release): profile in use")), "{:?}", found.detail);
    for found in [analyze_compact(&profiles(&env), &none, &go()), analyze_download_dir(&profiles(&env), &none, &go())] {
        assert_eq!(found.count, 0, "{:?}", found.detail);
        assert_eq!(found.skipped, 2);
    }
    drop((lock_chrome, lock_ff));
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert_eq!(found.count, 2 * COOKIE_HOSTS.len() as u64);
}

#[test]
fn clean_acts_only_on_found_items() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    chromium_cookies(&prof.join(r"Network\Cookies"), &COOKIE_HOSTS);
    // Un database con la stessa forma fuori dai profili.
    let outside = t.path().join("Cookies");
    chromium_cookies(&outside, &COOKIE_HOSTS);
    let forced = NativeFound {
        count: 7,
        items: vec![outside.to_string_lossy().into_owned(), format!("{KEEP_PREFIX}google.com")],
        ..Default::default()
    };
    let done = clean_cookies(&profiles(&env), &forced, &none, &go());
    assert_eq!(done.removed, 0);
    assert_eq!(hosts_in(&outside, "cookies", "host_key").len(), COOKIE_HOSTS.len());
    assert_eq!(hosts_in(&prof.join(r"Network\Cookies"), "cookies", "host_key").len(), COOKIE_HOSTS.len());
    // Lo stesso per compattazione, cronologia e preferenze.
    let forced = NativeFound { count: 1, items: vec![prof.join("Preferences").to_string_lossy().into_owned()], ..Default::default() };
    assert_eq!(clean_compact(&profiles(&env), &forced, &none, &go()).removed, 0);
    assert_eq!(clean_history(&profiles(&env), &forced, &none, &go()).removed, 0);
    let forced = NativeFound { count: 1, items: vec![outside.to_string_lossy().into_owned()], ..Default::default() };
    assert_eq!(clean_download_dir(&profiles(&env), &forced, &none, &go()).removed, 0);
}

#[test]
fn cancel_stops_between_databases() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    chromium_cookies(&prof.join(r"Network\Cookies"), &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    let stop = AtomicBool::new(true);
    assert_eq!(clean_cookies(&profiles(&env), &found, &none, &stop).removed, 0);
    assert_eq!(analyze_compact(&profiles(&env), &none, &stop).count, 0);
}

// ----- correzioni della revisione -----

#[test]
fn analysis_never_writes_a_database_with_a_leftover_wal() {
    // Firefox chiuso male: `places.sqlite` con un `-wal` rimasto e senza `-shm`.
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let db = places(&prof);
    let wal = prof.join("places.sqlite-wal");
    let c = Connection::open(&db).unwrap();
    c.execute_batch("PRAGMA wal_autocheckpoint = 0; INSERT INTO moz_historyvisits VALUES (5, 0, 2, 5, 1);").unwrap();
    let (main_bytes, wal_bytes) = (fs::read(&db).unwrap(), fs::read(&wal).unwrap());
    drop(c);
    fs::write(&db, &main_bytes).unwrap();
    fs::write(&wal, &wal_bytes).unwrap();
    let _ = fs::remove_file(prof.join("places.sqlite-shm"));

    let found = analyze_history(&profiles(&env), &none, &go());
    assert_eq!(found.count, 5 + 3, "la visita nel -wal si vede: {:?}", found.detail);
    analyze_compact(&profiles(&env), &none, &go());
    analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert_eq!(fs::read(&db).unwrap(), main_bytes, "l'analisi non fa il checkpoint");
    assert_eq!(fs::read(&wal).unwrap(), wal_bytes);
}

#[test]
fn rewrite_failure_leaves_the_original_and_no_temporary() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let chrome = chromium_profile(&chrome_user_data(&env), "Default");
    let prefs = chrome.join("Preferences");
    let tmps = |dir: &Path| fs::read_dir(dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp")).count();
    // Il controllo della rilettura fallisce.
    assert!(write_replacing(&prefs, b"{}", &|_| false).is_err());
    assert_eq!(fs::read_to_string(&prefs).unwrap(), PREFS_JSON);
    assert_eq!(tmps(&chrome), 0);
    // La sostituzione fallisce: un altro processo tiene il file senza condividerlo in cancellazione.
    let found = analyze_download_dir(&profiles(&env), &none, &go());
    assert_eq!(found.count, 1);
    const FILE_SHARE_READ: u32 = 1;
    let held = hold(&prefs, FILE_SHARE_READ);
    let done = clean_download_dir(&profiles(&env), &found, &none, &go());
    drop(held);
    assert_eq!((done.removed, done.failed), (0, 1), "{:?}", done.notes);
    assert_eq!(fs::read_to_string(&prefs).unwrap(), PREFS_JSON);
    assert_eq!(tmps(&chrome), 0);
    // Un temporaneo rimasto da un'interruzione non blocca la voce.
    fs::write(chrome.join("Preferences.xiox.tmp"), b"vecchio").unwrap();
    let done = clean_download_dir(&profiles(&env), &found, &none, &go());
    assert_eq!((done.removed, done.failed), (1, 0), "{:?}", done.notes);
    assert_eq!(tmps(&chrome), 1, "solo quello vecchio");
}

#[test]
fn compact_is_skipped_without_enough_free_space() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    let history = prof.join("History");
    fragmented(&history);
    let bytes = fs::read(&history).unwrap();
    let size = bytes.len() as u64;
    let found = analyze_compact(&profiles(&env), &none, &go());
    let asked = std::cell::RefCell::new(Vec::new());
    // Appena meno del doppio sul volume della cartella temporanea.
    let free = |p: &Path| {
        asked.borrow_mut().push(p.to_path_buf());
        Some(if p.starts_with(&prof) { 10 * size } else { 2 * size - 1 })
    };
    let done = clean_compact_with(&profiles(&env), &found, &none, &go(), &free, Path::new(r"X:\temp"));
    assert_eq!((done.removed, done.freed), (0, 0));
    assert!(done.notes.iter().any(|n| n.contains("not enough space to compact")), "{:?}", done.notes);
    assert_eq!(fs::read(&history).unwrap(), bytes);
    assert!(asked.borrow().iter().any(|p| p == Path::new(r"X:\temp")), "{:?}", asked.borrow());
    // Spazio sconosciuto: come insufficiente.
    let done = clean_compact_with(&profiles(&env), &found, &none, &go(), &|_| None, Path::new(r"X:\temp"));
    assert_eq!(done.removed, 0);
    // Abbastanza spazio: si compatta.
    let done = clean_compact_with(&profiles(&env), &found, &none, &go(), &|_| Some(2 * size), Path::new(r"X:\temp"));
    assert_eq!(done.removed, 1, "{:?}", done.notes);
}

#[test]
fn firefox_lock_is_held_while_a_profile_is_cleaned() {
    let t = tempfile::tempdir().unwrap();
    let env = env_in(t.path());
    let prof = gecko_profile(&firefox_profiles(&env), "a.default-release");
    let p = profiles(&env).into_iter().next().unwrap();
    let lock = prof.join("parent.lock");
    {
        let held = hold_profile(&p).expect("parent.lock preso");
        assert!(lock_held(&lock), "Firefox che parte adesso trova il profilo in uso");
        drop(held);
    }
    assert!(!lock.exists(), "creato da noi, tolto alla fine");
    // Un parent.lock rimasto da Firefox resta dov'è.
    fs::write(&lock, b"").unwrap();
    drop(hold_profile(&p).unwrap());
    assert!(lock.exists());
    // Chromium: niente da tenere.
    let chrome = chromium_profile(&chrome_user_data(&env), "Default");
    let c = profiles(&env).into_iter().find(|q| q.dir == chrome).unwrap();
    assert!(hold_profile(&c).is_ok_and(|h| h.is_none()));
}

// ----- voci -----

#[test]
fn items_follow_the_coverage_table() {
    let items = items();
    let ids: Vec<&str> = items.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![COOKIES_ID, COMPACT_ID, FIREFOX_HISTORY_ID, DOWNLOAD_DIR_ID]);
    for i in &items {
        assert_eq!(i.group, "Browser");
        assert!(!i.default_on, "{} deve partire spenta", i.id);
        assert!(i.warning.is_some(), "{} senza avviso", i.id);
        assert!(!i.needs_admin);
        assert!(i.running.is_empty(), "il controllo del browser aperto è per profilo");
        let text = format!("{} {} {}", i.label, i.description, i.warning.unwrap());
        assert!(!text.to_lowercase().contains("bleachbit") && !text.contains("AGG -"));
    }
    assert!(items[0].warning_extra.is_some());
    assert!(items[1..].iter().all(|i| i.warning_extra.is_none()));
    assert!(items[0].warning.unwrap().contains("all cookies will be deleted except the domains on the list"));
}
