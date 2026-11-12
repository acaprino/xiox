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

fn keep(list: &[&str]) -> Result<Vec<String>, String> {
    Ok(list.iter().map(|s| s.to_string()).collect())
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
        ("Opera (profilo principale)".to_string(), Kind::Chromium),
        ("Opera (Default)".to_string(), Kind::Chromium),
        ("Opera GX (profilo principale)".to_string(), Kind::Chromium),
        ("Opera GX (profilo laterale abc)".to_string(), Kind::Chromium),
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
    let side = all.iter().find(|p| p.label() == "Opera GX (profilo laterale abc)").unwrap();
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
    assert_eq!(parse_keep_list(text), vec!["google.com", "live.com", "example.org", "accounts.example.net", "wild.it"]);
    assert!(parse_keep_list(KEEP_LIST_TEMPLATE).is_empty(), "gli esempi sono commentati");
    assert!(is_kept(".google.com", &["google.com".to_string()]));
    assert!(is_kept("ACCOUNTS.google.com", &["google.com".to_string()]));
    assert!(!is_kept("evilgoogle.com", &["google.com".to_string()]));
    assert!(!is_kept("google.com.evil.net", &["google.com".to_string()]));
}

#[test]
fn empty_or_missing_keep_list_protects_nothing_and_says_so() {
    let t = tempfile::tempdir().unwrap();
    let path = keep_list_in(t.path());
    assert_eq!(path, t.path().join(r"PulisciPC\cookie_da_conservare.txt"));
    // Assente.
    assert_eq!(read_keep_list(&path), Ok(vec![]));
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "lista: 0 domini");
    // Creata al primo uso con gli esempi commentati, mai sovrascritta.
    ensure_keep_list(&path).unwrap();
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "lista: 0 domini");
    fs::write(&path, "google.com\n").unwrap();
    ensure_keep_list(&path).unwrap();
    assert_eq!(keep_list_summary(&read_keep_list(&path)), "lista: 1 dominio");
    // Vuota.
    fs::write(&path, "").unwrap();
    assert_eq!(read_keep_list(&path), Ok(vec![]));
    // Non UTF-8: nessun panico, i domini ASCII restano.
    fs::write(&path, b"\xE9\xE8\xFF\nlive.com\n").unwrap();
    assert_eq!(read_keep_list(&path), Ok(vec!["live.com".to_string()]));
    // UTF-16 con BOM (Blocco note «Unicode»).
    let mut wide = vec![0xFF, 0xFE];
    wide.extend("google.com\r\nlive.com\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&path, wide).unwrap();
    assert_eq!(read_keep_list(&path), Ok(vec!["google.com".to_string(), "live.com".to_string()]));
    // Illeggibile (una cartella): errore, e la finestra dice che non si tocca niente.
    let dir = t.path().join("cartella");
    fs::create_dir_all(&dir).unwrap();
    let err = read_keep_list(&dir);
    assert!(err.is_err());
    assert!(keep_list_summary(&err).contains("nessun cookie verrà eliminato"), "{}", keep_list_summary(&err));

    // L'analisi con la lista vuota propone tutti i cookie e lo dice.
    let env = env_in(t.path());
    let prof = chromium_profile(&chrome_user_data(&env), "Default");
    chromium_cookies(&prof.join(r"Network\Cookies"), &COOKIE_HOSTS);
    let found = analyze_cookies(&profiles(&env), keep(&[]), &none, &go());
    assert!(found.detail[0].contains("lista: 0 domini"), "{:?}", found.detail);
    assert_eq!(found.count, COOKIE_HOSTS.len() as u64);
    // Lista illeggibile: nessun cookie proposto.
    let found = analyze_cookies(&profiles(&env), Err("lista illeggibile".into()), &none, &go());
    assert_eq!(found.count, 0);
    assert!(found.items.is_empty());
    assert!(found.detail[0].contains("nessun cookie verrà eliminato"), "{:?}", found.detail);
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
    assert!(found.detail[0].contains("lista: 2 domini"), "{:?}", found.detail);
    let done = clean_cookies(&profiles(&env), &found, &none, &go());
    assert_eq!(done.removed, 6, "{:?}", done.notes);
    assert_eq!(done.failed, 0);
    // Quante righe e quanti byte, per ogni database.
    assert_eq!(done.done.len(), 2, "{:?}", done.done);
    assert!(done.done.iter().all(|d| d.contains("3 cookie eliminati") && d.contains("liberati nel database")), "{:?}", done.done);
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
    assert!(done.notes.iter().any(|n| n.contains("controllo di integrità")), "{:?}", done.notes);
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
    assert!(done.done[0].contains("4 visite e 3 pagine tolte") && done.done[0].contains("liberati nel database"), "{:?}", done.done);

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
    assert!(done.notes.iter().any(|n| n.contains("segnalibri")), "{:?}", done.notes);
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
    assert!(done.notes.iter().any(|n| n.contains("segnalibri")), "{:?}", done.notes);
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
    assert!(found.detail.iter().any(|d| d.contains("non riconosciuto")), "{:?}", found.detail);
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
    assert_eq!(strip_chromium_prefs(b"{\"download\": {\"default_directory\": \"x\"}"), Err("non è JSON valido".to_string()));
    assert_eq!(strip_chromium_prefs(b"[1, 2]"), Err("non è un oggetto JSON".to_string()));
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
        assert!(found.detail.iter().any(|d| d.contains("Google Chrome (Default): chiudi chrome.exe per pulire")), "{:?}", found.detail);
        assert!(!found.detail.iter().any(|d| d.contains("leggibil")), "{:?}", found.detail);
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
    assert!(done.notes.iter().any(|n| n.contains("chiudi chrome.exe per pulire")), "{:?}", done.notes);
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
    assert!(found.detail.iter().any(|d| d == "Google Chrome (Default): chiudi chrome.exe per pulire"), "{:?}", found.detail);
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
    assert!(found.detail.iter().any(|d| d.contains("Mozilla Firefox (a.default-release): chiudi firefox.exe per pulire")), "{:?}", found.detail);
    // Un `found` fatto a browser chiuso non serve se intanto Firefox si è aperto.
    let found = analyze_history(&profiles(&env), &none, &go());
    let before = fs::read(&db).unwrap();
    let done = clean_history(&profiles(&env), &found, &ff_running, &go());
    assert_eq!(done.removed, 0);
    assert!(done.notes.iter().any(|n| n.contains("chiudi firefox.exe per pulire")), "{:?}", done.notes);
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
    assert!(found.detail.iter().any(|d| d.contains("Google Chrome (Default): profilo in uso, chiudi chrome.exe per pulire")), "{:?}", found.detail);
    assert!(found.detail.iter().any(|d| d.contains("Mozilla Firefox (a.default-release): profilo in uso")), "{:?}", found.detail);
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
    assert!(items[0].warning.unwrap().contains("tutti i cookie tranne i domini della lista"));
}
