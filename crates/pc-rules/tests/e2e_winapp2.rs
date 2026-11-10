//! Winapp2 (testo) → `parse` → `to_cleaners_with` → `collect`, senza ambiente
//! né registro veri: tutto in una cartella temporanea e con chiusure iniettate.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use pc_rules::collect::{RegItem, collect};
use pc_rules::model::{Env, RegRoot};
use pc_rules::winapp2::{parse, to_cleaners_with};

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

const INI: &str = "\
; Version: 260915
; # of entries: 5

[Alpha Cache *]
LangSecRef=3025
Detect=HKCU\\Software\\Alpha
FileKey1=%AppData%\\Alpha\\Cache|*.*|RECURSE
RegKey1=HKCU\\Software\\Alpha\\Mru
RegKey2=HKCU\\Software\\Alpha\\Settings|LastFile
RegKey3=HKCU\\Software\\Alpha\\Gone

[Beta Temp *]
DetectFile=%AppData%\\Beta
FileKey1=%AppData%\\Beta\\Temp|*.tmp|REMOVESELF

[Gamma Keep *]
DetectFile=%AppData%\\Gamma
FileKey1=%AppData%\\Gamma\\Cache|*.*|RECURSE
ExcludeKey1=FILE|%AppData%\\Gamma\\Cache|keep.txt

[Profile Wipe *]
DetectFile=%AppData%\\Alpha
FileKey1=%UserProfile%|*.*|RECURSE

[Path Exclude *]
DetectFile=%AppData%\\Gamma
FileKey1=%AppData%\\Gamma\\Cache|*.*|RECURSE
ExcludeKey1=PATH|%AppData%\\Gamma\\Cache\\sub
";

#[test]
fn winapp2_text_to_collected_files_and_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let profile = root.join("profile");
    let appdata = profile.join("AppData").join("Roaming");
    let env = Env(HashMap::from([
        ("userprofile".to_string(), profile.to_string_lossy().into_owned()),
        ("appdata".to_string(), appdata.to_string_lossy().into_owned()),
    ]));

    // Alpha: file ricorsivi
    write(&appdata.join("Alpha/Cache/a.bin"), "12345");
    write(&appdata.join("Alpha/Cache/sub/b.bin"), "123");
    // Beta: cartella da togliere
    write(&appdata.join("Beta/Temp/x.tmp"), "12");
    write(&appdata.join("Beta/Temp/y.tmp"), "1");
    // Gamma: un file da tenere
    write(&appdata.join("Gamma/Cache/keep.txt"), "keepme");
    write(&appdata.join("Gamma/Cache/drop.txt"), "1234");
    write(&appdata.join("Gamma/Cache/sub/z.txt"), "1");
    // un file del profilo che non si deve mai toccare
    write(&profile.join("Documents/precious.doc"), "mine");

    let reg_present = |root: RegRoot, path: &str| {
        root == RegRoot::Hkcu && path == r"Software\Alpha"
    };
    let (w, _) = parse(INI);
    let (cleaners, report) = to_cleaners_with(&w, &env, &reg_present);

    // La voce `Path Exclude` (ExcludeKey di specie PATH) non si offre e si conta;
    // la voce del profilo è scartata e contata.
    assert_eq!(report.skipped_excludes, 1);
    assert_eq!(report.discarded_protected, 1);
    let mut ids: Vec<&str> = cleaners.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["winapp2:Alpha Cache", "winapp2:Beta Temp", "winapp2:Gamma Keep"]
    );
    let by = |id: &str| cleaners.iter().find(|c| c.id == id).unwrap();

    // Esistenza del registro iniettata: niente registro vero.
    let exists = |item: &RegItem| match item {
        RegItem::Key { path, .. } => path != r"Software\Alpha\Gone",
        RegItem::Value { .. } => true,
    };

    // Alpha: file e chiavi; i percorsi di registro non hanno la radice dentro.
    let c = collect(&by("winapp2:Alpha Cache").options[0], &env, &exists);
    let mut files: Vec<_> = c
        .files
        .iter()
        .map(|(p, n)| (p.strip_prefix(&appdata).unwrap().to_path_buf(), *n))
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec![
            (Path::new(r"Alpha\Cache\a.bin").to_path_buf(), 5),
            (Path::new(r"Alpha\Cache\sub\b.bin").to_path_buf(), 3),
        ]
    );
    assert_eq!(c.bytes, 8);
    assert_eq!(c.skipped, 0);
    assert!(c.dirs_to_remove.is_empty());
    assert_eq!(
        c.reg,
        vec![
            RegItem::Key { root: RegRoot::Hkcu, path: r"Software\Alpha\Mru".into() },
            RegItem::Value {
                root: RegRoot::Hkcu,
                path: r"Software\Alpha\Settings".into(),
                value: "LastFile".into()
            },
        ]
    );

    // Beta: REMOVESELF, la cartella si elenca dopo i file
    let c = collect(&by("winapp2:Beta Temp").options[0], &env, &exists);
    assert_eq!(c.files.len(), 2);
    assert_eq!(c.bytes, 3);
    assert_eq!(c.dirs_to_remove, vec![appdata.join("Beta").join("Temp")]);

    // Gamma: keep.txt resta, il resto va; la cartella di Gamma\Cache non si toglie
    let c = collect(&by("winapp2:Gamma Keep").options[0], &env, &exists);
    let mut names: Vec<String> = c
        .files
        .iter()
        .map(|(p, _)| p.strip_prefix(&appdata).unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec![r"Gamma\Cache\drop.txt", r"Gamma\Cache\sub\z.txt"]);
    assert_eq!(c.bytes, 5);
    assert!(!c.files.iter().any(|(p, _)| p.ends_with("keep.txt")));

    // Il profilo non si è toccato e la regola del profilo non è nemmeno offerta.
    assert!(profile.join("Documents/precious.doc").exists());
}

#[test]
fn profile_rule_reaches_collect_and_is_discarded_there_too() {
    // Anche se una regola del profilo arrivasse a `collect` (non passando dalla
    // conversione), la raccolta la scarta e la conta.
    use pc_rules::model::{Action, Opt, Search};
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("p");
    write(&profile.join("Documents/precious.doc"), "mine");
    let env = Env(HashMap::from([(
        "userprofile".to_string(),
        profile.to_string_lossy().into_owned(),
    )]));
    let opt = Opt {
        id: "o".into(),
        label: "o".into(),
        description: String::new(),
        warning: None,
        default_on: false,
        actions: vec![Action::Files {
            path: "%UserProfile%".into(),
            patterns: vec!["*.*".into()],
            recurse: true,
            remove_self: false,
            search: Search::Winapp2,
        }],
        excludes: vec![],
        unsupported: None,
    };
    let c = collect(&opt, &env, &|_| false);
    assert!(c.files.is_empty() && c.dirs_to_remove.is_empty());
    assert_eq!(c.skipped, 1);
}
