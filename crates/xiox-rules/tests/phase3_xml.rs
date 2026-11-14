//! Fase 3a, Task 1: le voci «Nuovo XML» della tabella di copertura
//! (`docs/rules-coverage.md`, §2 e §3.1) in `cleaners/windows_extra.xml` e
//! `cleaners/safari.xml`.
//!
//! Mai sul PC dell'utente: i percorsi si controllano con un `Env` finto e le
//! raccolte girano su un profilo finto in una cartella temporanea; il registro
//! non si legge (`reg_exists` finto).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use xiox_rules::cleanerml::parse_cleaner;
use xiox_rules::collect::{collect, running};
use xiox_rules::expand::expand;
use xiox_rules::guard::Guard;
use xiox_rules::model::{Action, Cleaner, Env, Opt, RegRoot};
use xiox_rules::reg_allowed;

const XMLS: [(&str, &str); 2] = [
    ("windows_extra", include_str!("../../../cleaners/windows_extra.xml")),
    ("safari", include_str!("../../../cleaners/safari.xml")),
];

const TABLE: &str = include_str!("../../../docs/rules-coverage.md");

/// Opzioni attese, nell'ordine del file. Rispetto alla tabella mancano
/// `ie_autocomplete` (chiave `IntelliForms\Storage1` «da verificare») e
/// `cookies` di Safari (percorso «da verificare»): non si forzano.
const EXPECTED: [(&str, &[&str]); 2] = [
    (
        "windows_extra",
        &[
            "ie_cache",
            "ie_history",
            "ie_cookies",
            "ie_typed_urls",
            "ie_last_download",
            "recent_docs",
            "run_mru",
            "search_assistant",
            "explorer_mrus",
            "chkdsk",
            "menu_order",
            "tray_notify",
            "window_cache",
            "user_assist",
            "iis_logs",
        ],
    ),
    ("safari", &["cache", "history", "forms"]),
];

/// Ambiente realistico (percorsi standard di Windows 11) per la guardia.
fn real_like_env() -> Env {
    Env(HashMap::from([
        ("userprofile".to_string(), r"C:\Users\t".to_string()),
        ("appdata".to_string(), r"C:\Users\t\AppData\Roaming".to_string()),
        ("localappdata".to_string(), r"C:\Users\t\AppData\Local".to_string()),
        ("temp".to_string(), r"C:\Users\t\AppData\Local\Temp".to_string()),
        ("username".to_string(), "t".to_string()),
        ("programdata".to_string(), r"C:\ProgramData".to_string()),
        ("windir".to_string(), r"C:\Windows".to_string()),
        ("systemroot".to_string(), r"C:\Windows".to_string()),
        ("systemdrive".to_string(), "C:".to_string()),
        ("programfiles".to_string(), r"C:\Program Files".to_string()),
    ]))
}

/// Profilo finto sotto `root`: profilo, AppData, LocalAppData, Temp e un
/// disco di sistema finto (`root\disk`).
fn fake_env(root: &Path) -> Env {
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let profile = root.join("Users").join("t");
    Env(HashMap::from([
        ("userprofile".to_string(), s(profile.clone())),
        ("appdata".to_string(), s(profile.join(r"AppData\Roaming"))),
        ("localappdata".to_string(), s(profile.join(r"AppData\Local"))),
        ("temp".to_string(), s(profile.join(r"AppData\Local\Temp"))),
        ("username".to_string(), "t".to_string()),
        ("systemdrive".to_string(), s(root.join("disk"))),
    ]))
}

fn load(id: &str, env: &Env) -> Cleaner {
    let &(name, xml) = XMLS.iter().find(|(n, _)| *n == id).expect("cleaner nella lista");
    parse_cleaner(xml, name, env).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn option_of<'a>(c: &'a Cleaner, id: &str) -> &'a Opt {
    c.options
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("{}: manca l'opzione {id}", c.id))
}

fn write(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "xx").unwrap();
}

/// File che `collect` trova per l'opzione, relativi a `root`, con `/`.
fn found(cleaner: &str, option: &str, root: &Path, create: &[&str]) -> Vec<String> {
    for f in create {
        write(&root.join(f));
    }
    let env = fake_env(root);
    let c = load(cleaner, &env);
    let got = collect(option_of(&c, option), &env, &|_| false);
    assert_eq!(got.skipped, 0, "{cleaner}/{option}: {:?}", got.skipped_reasons);
    let mut files: Vec<String> = got
        .files
        .iter()
        .map(|(p, _)| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    files.sort();
    files
}

fn sorted(v: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|s| (*s).to_string()).collect();
    v.sort();
    v
}

/// Righe della tabella fra `start` e `end` (esclusa).
fn table_rows<'a>(start: &str, end: &str) -> Vec<Vec<&'a str>> {
    let from = TABLE.find(start).unwrap_or_else(|| panic!("tabella: manca «{start}»"));
    let rest = &TABLE[from..];
    let to = rest.find(end).unwrap_or(rest.len());
    rest[..to]
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| l.trim_matches('|').split('|').map(str::trim).collect())
        .collect()
}

#[test]
fn every_phase3_xml_parses_with_zero_warnings() {
    let env = real_like_env();
    let mut total = 0;
    for ((name, xml), (want, ids)) in XMLS.iter().zip(EXPECTED) {
        assert_eq!(*name, want);
        let c = parse_cleaner(xml, name, &env).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(c.id, *name);
        let got: Vec<&str> = c.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(got, ids, "{name}: opzioni");
        for o in &c.options {
            assert!(o.unsupported.is_none(), "{name}/{}: {:?}", o.id, o.unsupported);
            assert!(!o.actions.is_empty(), "{name}/{}: senza azioni", o.id);
            assert!(!o.label.is_empty() && !o.description.is_empty(), "{name}/{}", o.id);
        }
        total += c.options.len();
    }
    assert_eq!(total, 18);
    // Numero di azioni per file: nessuna sparisce in silenzio.
    let actions = |id: &str| load(id, &env).options.iter().map(|o| o.actions.len()).sum::<usize>();
    assert_eq!(actions("windows_extra"), 35);
    assert_eq!(actions("safari"), 8);
}

/// Ogni opzione viene da una riga «Nuovo XML» della tabella (nella sua
/// sezione) e parte spenta, qualunque sia il default indicato in tabella: come tutti
/// gli inglobati, l'XML non ha `default_on`.
#[test]
fn phase3_options_are_off_unless_table_default_on() {
    let env = real_like_env();
    let sections = [
        ("windows_extra", table_rows("## 2. Scheda Windows", "## 3. Scheda Applicazioni")),
        ("safari", table_rows("**Safari per Windows**", "### 3.2")),
    ];
    for (id, rows) in sections {
        let c = load(id, &env);
        for o in &c.options {
            let needle = format!("opzione `{}`", o.id);
            let row = rows
                .iter()
                .find(|r| r.len() == 7 && r[4].contains(&needle))
                .unwrap_or_else(|| panic!("{id}/{}: nessuna riga della tabella", o.id));
            // La cella può avere in coda lo stato («Nuovo XML · **Stato:** fatto …»).
            let coverage = row[3].split(" · ").next().unwrap_or("");
            assert_eq!(coverage, "Nuovo XML", "{id}/{}", o.id);
            let table_default = row[2];
            assert!(
                table_default.starts_with("Sì") || table_default.starts_with("No"),
                "{id}/{}: default «{table_default}»",
                o.id
            );
            assert!(!o.default_on, "{id}/{} è acceso di default", o.id);
        }
    }
}

#[test]
fn registry_options_stay_inside_hkcu_software() {
    let env = real_like_env();
    let mut keys = 0;
    for (name, _) in XMLS {
        for o in &load(name, &env).options {
            for a in &o.actions {
                let (root, path, value) = match a {
                    Action::RegKey { root, path } => (*root, path, None),
                    Action::RegValue { root, path, value } => (*root, path, Some(value.as_str())),
                    Action::Files { .. } => continue,
                };
                keys += 1;
                assert!(
                    path.to_lowercase().starts_with(r"software\"),
                    "{name}/{}: {path}",
                    o.id
                );
                assert_eq!(reg_allowed(root, path, value), Ok(()), "{name}/{}: {path}", o.id);
                // Solo il valore Direct3D di «Altri MRU di Esplora» sta in HKLM\Software.
                if root == RegRoot::Hklm {
                    assert_eq!(
                        (o.id.as_str(), path.as_str(), value),
                        ("explorer_mrus", r"Software\Microsoft\Direct3D\MostRecentApplication", Some("Name"))
                    );
                }
            }
        }
    }
    assert_eq!(keys, 27);
}

/// Nessun percorso è rifiutato dalla guardia delle regole esterne: né il
/// percorso grezzo né la cartella da cui parte la raccolta.
#[test]
fn file_actions_pass_the_guard() {
    let env = real_like_env();
    let guard = Guard::new(&env);
    for (name, _) in XMLS {
        for o in &load(name, &env).options {
            for a in &o.actions {
                let Action::Files { path, .. } = a else { continue };
                let expanded = expand(path, &env);
                assert_eq!(expanded.len(), 1, "{name}/{}: {path}", o.id);
                let p = &expanded[0];
                assert_eq!(guard.protected(p), None, "{name}/{}: {}", o.id, p.display());
            }
        }
    }
}

#[test]
fn ie_files_take_only_their_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let local = "Users/t/AppData/Local/Microsoft/Windows";
    let create = [
        format!("{local}/INetCache/IE/A1B2/x.gif"),
        format!("{local}/INetCache/Low/IE/C3D4/y.js"),
        format!("{local}/INetCache/Other/keep.dat"),
        format!("{local}/History/History.IE5/z.dat"),
        format!("{local}/INetCookies/c.cookie"),
        format!("{local}/WebCache/WebCacheV01.dat"),
    ];
    let create: Vec<&str> = create.iter().map(String::as_str).collect();
    assert_eq!(
        found("windows_extra", "ie_cache", root, &create),
        sorted(&[
            "Users/t/AppData/Local/Microsoft/Windows/INetCache/IE/A1B2/x.gif",
            "Users/t/AppData/Local/Microsoft/Windows/INetCache/Low/IE/C3D4/y.js",
        ])
    );
    assert_eq!(
        found("windows_extra", "ie_history", root, &[]),
        sorted(&["Users/t/AppData/Local/Microsoft/Windows/History/History.IE5/z.dat"])
    );
    assert_eq!(
        found("windows_extra", "ie_cookies", root, &[]),
        sorted(&["Users/t/AppData/Local/Microsoft/Windows/INetCookies/c.cookie"])
    );
}

#[test]
fn recent_documents_take_only_top_level_shortcuts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let recent = "Users/t/AppData/Roaming/Microsoft/Windows/Recent";
    let create = [
        format!("{recent}/report.docx.lnk"),
        format!("{recent}/photos.lnk"),
        format!("{recent}/desktop.ini"),
        format!("{recent}/AutomaticDestinations/abc.automaticDestinations-ms"),
        format!("{recent}/CustomDestinations/def.customDestinations-ms"),
        format!("{recent}/Sub/inner.lnk"),
    ];
    let create: Vec<&str> = create.iter().map(String::as_str).collect();
    assert_eq!(
        found("windows_extra", "recent_docs", root, &create),
        sorted(&[
            "Users/t/AppData/Roaming/Microsoft/Windows/Recent/photos.lnk",
            "Users/t/AppData/Roaming/Microsoft/Windows/Recent/report.docx.lnk",
        ])
    );
}

#[test]
fn chkdsk_fragments_and_iis_logs_on_the_system_drive() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let create = [
        "disk/FILE0000.CHK",
        "disk/file0001.chk",
        "disk/FOUND.000/FILE0002.CHK",
        "disk/FOUND.000/other.txt",
        "disk/notes.chk",
        "disk/inetpub/logs/LogFiles/W3SVC1/u_ex261111.log",
        "disk/inetpub/wwwroot/index.html",
    ];
    assert_eq!(
        found("windows_extra", "chkdsk", root, &create),
        sorted(&["disk/FILE0000.CHK", "disk/FOUND.000/FILE0002.CHK", "disk/file0001.chk"])
    );
    assert_eq!(
        found("windows_extra", "iis_logs", root, &[]),
        sorted(&["disk/inetpub/logs/LogFiles/W3SVC1/u_ex261111.log"])
    );
}

#[test]
fn safari_options_take_the_paths_of_the_table() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let roaming = "Users/t/AppData/Roaming/Apple Computer/Safari";
    let local = "Users/t/AppData/Local/Apple Computer/Safari";
    let create = [
        format!("{local}/Cache.db"),
        format!("{local}/History/a.dat"),
        format!("{local}/Webpage Previews/p.jpg"),
        format!("{roaming}/History.plist"),
        format!("{roaming}/LastSession.plist"),
        format!("{roaming}/TopSites.plist"),
        format!("{roaming}/Downloads.plist"),
        format!("{roaming}/Form Values.plist"),
        format!("{roaming}/Bookmarks.plist"),
        format!("{roaming}/Cookies/Cookies.binarycookies"),
    ];
    let create: Vec<&str> = create.iter().map(String::as_str).collect();
    assert_eq!(
        found("safari", "cache", root, &create),
        sorted(&["Users/t/AppData/Local/Apple Computer/Safari/Cache.db"])
    );
    assert_eq!(
        found("safari", "history", root, &[]),
        sorted(&[
            "Users/t/AppData/Local/Apple Computer/Safari/History/a.dat",
            "Users/t/AppData/Local/Apple Computer/Safari/Webpage Previews/p.jpg",
            "Users/t/AppData/Roaming/Apple Computer/Safari/Downloads.plist",
            "Users/t/AppData/Roaming/Apple Computer/Safari/History.plist",
            "Users/t/AppData/Roaming/Apple Computer/Safari/LastSession.plist",
            "Users/t/AppData/Roaming/Apple Computer/Safari/TopSites.plist",
        ])
    );
    assert_eq!(
        found("safari", "forms", root, &[]),
        sorted(&["Users/t/AppData/Roaming/Apple Computer/Safari/Form Values.plist"])
    );
}

#[test]
fn warnings_and_running_processes() {
    let env = real_like_env();
    let w = load("windows_extra", &env);
    // Rischio «Medio» nella tabella: i cookie disconnettono dai siti.
    assert!(option_of(&w, "ie_cookies").warning.is_some());
    // Un `<running>` vale per tutto il cleaner: Esplora risorse è sempre
    // aperto, quindi nessuno qui (i file in uso si saltano).
    assert!(w.running.is_empty());
    let s = load("safari", &env);
    assert_eq!(running(&s, &["Safari.exe".to_string()]), vec!["Safari.exe"]);
    assert!(running(&s, &["notepad.exe".to_string()]).is_empty());
}

#[test]
fn visible_texts_are_clean() {
    let env = real_like_env();
    for (name, xml) in XMLS {
        assert!(!xml.to_lowercase().contains("bleachbit"), "{name}");
        let c = load(name, &env);
        let mut texts = vec![c.label.clone(), c.description.clone()];
        for o in &c.options {
            texts.push(o.label.clone());
            texts.push(o.description.clone());
            texts.extend(o.warning.clone());
        }
        for t in texts {
            assert!(!t.starts_with("AGG"), "{name}: «{t}»");
            for bad in ["sara piu", "possibilita", "puo ", "e stato", "perche "] {
                assert!(!t.contains(bad), "{name}: «{bad}» in «{t}»");
            }
        }
    }
}
