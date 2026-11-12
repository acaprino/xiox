//! Temporary-file cleaner. Locations and ages come from MECHANICS.md section 1.

use crate::sys;
use dc_cleaner::{remove_files_parallel, worker_count, Cancel, Tick};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Option "only delete files in Windows Temp folders older than 24 hours" (string 0x141ab7df8).
pub const TEMP_MIN_AGE_HOURS: u64 = 24;
/// Age for "old" Prefetch data: estimated, not found in the code.
pub const PREFETCH_MIN_AGE_DAYS: u64 = 14;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Clone)]
pub struct Target {
    pub dir: PathBuf,
    pub patterns: Vec<&'static str>,
    pub recursive: bool,
    /// Files newer than this are kept (seconds). `temp_age` = follows the 24-hour option.
    pub min_age_secs: u64,
    pub temp_age: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Folders,
    RecycleBin,
}

pub struct Category {
    pub name: &'static str,
    pub group: &'static str,
    pub help: &'static str,
    pub default_on: bool,
    pub needs_admin: bool,
    pub kind: Kind,
}

pub const CATEGORIES: &[Category] = &[
    Category { name: "Cestino", group: "Sistema", help: "Tutti i file nel Cestino", default_on: true, needs_admin: false, kind: Kind::RecycleBin },
    Category { name: "File temporanei dell'utente", group: "Sistema", help: "La cartella Temp del tuo account", default_on: true, needs_admin: false, kind: Kind::Folders },
    Category { name: "File temporanei di Windows", group: "Sistema", help: "La cartella Temp di Windows", default_on: true, needs_admin: true, kind: Kind::Folders },
    Category { name: "Dump di memoria", group: "Sistema", help: "File creati quando un programma o Windows si blocca", default_on: true, needs_admin: false, kind: Kind::Folders },
    Category { name: "File di log di Windows", group: "Sistema", help: "Registri di installazione e manutenzione", default_on: true, needs_admin: true, kind: Kind::Folders },
    Category { name: "Segnalazione errori di Windows", group: "Sistema", help: "Rapporti di errore già inviati o in coda", default_on: true, needs_admin: false, kind: Kind::Folders },
    Category { name: "Cache delle miniature", group: "Esplora file", help: "Anteprime delle immagini: Windows le ricrea", default_on: false, needs_admin: false, kind: Kind::Folders },
    Category { name: "Vecchi dati Prefetch", group: "Avanzate", help: "Dati di avvio dei programmi più vecchi di 14 giorni", default_on: false, needs_admin: true, kind: Kind::Folders },
    Category { name: "File di Ottimizzazione recapito", group: "Avanzate", help: "Copie degli aggiornamenti già installati", default_on: false, needs_admin: true, kind: Kind::Folders },
];

fn t(dir: PathBuf, patterns: &[&'static str], recursive: bool) -> Target {
    Target { dir, patterns: patterns.to_vec(), recursive, min_age_secs: 0, temp_age: false }
}

pub fn targets(index: usize) -> Vec<Target> {
    let win = PathBuf::from(sys::windir());
    let local = PathBuf::from(sys::env_or("LOCALAPPDATA", ""));
    let progdata = PathBuf::from(sys::env_or("ProgramData", "C:\\ProgramData"));
    match index {
        1 => {
            let mut x = t(PathBuf::from(sys::env_or("TEMP", "")), &["*"], true);
            x.temp_age = true;
            vec![x]
        }
        2 => {
            let mut x = t(win.join("Temp"), &["*"], true);
            x.temp_age = true;
            vec![x]
        }
        3 => vec![
            t(win.clone(), &["MEMORY.DMP"], false),
            t(win.join("Minidump"), &["*.dmp"], false),
            t(win.join("LiveKernelReports"), &["*.dmp"], true),
            t(local.join("CrashDumps"), &["*.dmp"], false),
        ],
        4 => vec![
            t(win.join("Logs").join("CBS"), &["*.log", "*.cab"], false),
            t(win.join("Logs").join("DISM"), &["*.log"], false),
            t(win.join("Logs").join("MoSetup"), &["*.log"], false),
            t(win.join("Panther"), &["*.log"], false),
            t(win.join("inf"), &["setupapi*.log"], false),
            t(win.clone(), &["*.log", "*.bak", "*log.txt"], false),
        ],
        5 => vec![
            t(progdata.join("Microsoft\\Windows\\WER\\ReportArchive"), &["*"], true),
            t(progdata.join("Microsoft\\Windows\\WER\\ReportQueue"), &["*"], true),
            t(progdata.join("Microsoft\\Windows\\WER\\Temp"), &["*"], true),
            t(local.join("Microsoft\\Windows\\WER\\ReportArchive"), &["*"], true),
            t(local.join("Microsoft\\Windows\\WER\\ReportQueue"), &["*"], true),
            t(local.join("ElevatedDiagnostics"), &["*"], true),
        ],
        6 => vec![
            t(local.join("Microsoft\\Windows\\Explorer"), &["thumbcache_*.db"], false),
            t(local.join("Microsoft\\Windows\\Explorer\\ThumbCacheToDelete"), &["thm*.tmp"], false),
        ],
        7 => {
            let mut x = t(win.join("Prefetch"), &["*.pf"], false);
            x.min_age_secs = PREFETCH_MIN_AGE_DAYS * 86400;
            vec![x]
        }
        8 => vec![t(
            win.join("ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache"),
            &["*"],
            true,
        )],
        _ => vec![],
    }
}

#[derive(Default, Clone)]
pub struct Found {
    pub files: Vec<(PathBuf, u64)>,
    pub bytes: u64,
    pub count: u64,
    pub skipped_dirs: u64,
}

#[derive(Default, Clone)]
pub struct Cleaned {
    pub freed: u64,
    pub removed: u64,
    pub failed: u64,
    /// Emptied folders removed after the files.
    pub dirs: u64,
    /// The files that stayed, with the reason (in use, access denied…).
    pub failures: Vec<(PathBuf, String)>,
    /// «Annulla» stopped this category (or it never started).
    pub cancelled: bool,
}

/// Case-insensitive match with `*` and `?`.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn walk(dir: &Path, target: &Target, min_age: u64, now: SystemTime, out: &mut Found) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        out.skipped_dirs += 1;
        return;
    };
    for e in entries.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || md.file_type().is_symlink() {
            continue; // never follow links or junctions
        }
        let path = e.path();
        if md.is_dir() {
            if target.recursive {
                walk(&path, target, min_age, now, out);
            }
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if !target.patterns.iter().any(|p| wildcard(p, &name)) {
            continue;
        }
        if min_age > 0 {
            let newest = md.modified().ok().into_iter().chain(md.created().ok()).max();
            if let Some(m) = newest {
                if now.duration_since(m).unwrap_or(Duration::ZERO).as_secs() < min_age {
                    continue;
                }
            }
        }
        out.bytes += md.len();
        out.count += 1;
        out.files.push((path, md.len()));
    }
}

pub fn analyze(index: usize, only_old_temp: bool) -> Found {
    let cat = &CATEGORIES[index];
    if cat.kind == Kind::RecycleBin {
        let (bytes, count) = sys::recycle_bin_info();
        return Found { files: vec![], bytes, count, skipped_dirs: 0 };
    }
    let now = SystemTime::now();
    let mut found = Found::default();
    for target in targets(index) {
        if target.dir.as_os_str().is_empty() || !target.dir.is_dir() {
            continue;
        }
        let age = if target.temp_age && only_old_temp { TEMP_MIN_AGE_HOURS * 3600 } else { target.min_age_secs };
        walk(&target.dir, &target, age, now, &mut found);
    }
    found
}

/// Removes the empty folders below `dir` (never `dir` itself, never through a link).
/// Returns how many went.
fn remove_empty_dirs(dir: &Path, is_root: bool) -> u64 {
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_dir() && md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    n += remove_empty_dirs(&e.path(), false);
                }
            }
        }
    }
    // Fails when not empty, that's fine.
    if !is_root && std::fs::remove_dir(dir).is_ok() {
        n += 1;
    }
    n
}

/// Cleans what `analyze(index, …)` found: the files with the worker pool
/// (`dc_cleaner::remove_files_parallel`, read-only files included), then the
/// emptied folders of the recursive targets. The Recycle Bin is emptied with
/// `sys::empty_recycle_bin`. «Annulla» stops before the next file.
pub fn clean(index: usize, found: &Found, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> Cleaned {
    if CATEGORIES[index].kind == Kind::RecycleBin {
        let mut r = Cleaned::default();
        if found.count == 0 {
            return r;
        }
        if cancel.is_cancelled() {
            r.cancelled = true;
        } else if sys::empty_recycle_bin() {
            r.freed = found.bytes;
            r.removed = found.count;
        } else {
            r.failed = found.count;
            r.failures.push((PathBuf::from(CATEGORIES[index].name), "Svuotamento del Cestino non riuscito".into()));
        }
        return r;
    }
    let roots: Vec<PathBuf> = targets(index).into_iter().filter(|t| t.recursive).map(|t| t.dir).collect();
    clean_files(found, &roots, tick, cancel)
}

/// The files of `found` (and nothing else: they are what the analysis listed
/// under the category's folders), then the empty folders below each of `roots`.
/// `cancelled` only when «Annulla» really left some file untried; then the
/// folders are not touched.
fn clean_files(found: &Found, roots: &[PathBuf], tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> Cleaned {
    let (freed, removed, failures, untried) = remove_files_parallel(&found.files, worker_count(), tick, cancel);
    let mut r = Cleaned { freed, removed, failed: failures.len() as u64, failures, ..Default::default() };
    r.cancelled = untried > 0;
    if r.cancelled {
        return r;
    }
    for dir in roots {
        if !dir.as_os_str().is_empty() && dir.is_dir() {
            r.dirs += remove_empty_dirs(dir, true);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::wildcard;
    #[test]
    fn wild() {
        assert!(wildcard("*.log", "CBS.LOG"));
        assert!(wildcard("thumbcache_*.db", "thumbcache_256.db"));
        assert!(!wildcard("*.dmp", "a.dmp.txt"));
        assert!(wildcard("*", "x"));
        assert!(wildcard("setupapi*.log", "setupapi.dev.log"));
    }

    /// Ritocchi della Fase 3 (tabella di copertura, §4.1): si aggiungono solo
    /// i percorsi che la guardia delle regole esterne accetterebbe; il resto
    /// (`System32\wbem\Logs`, il valore di registro di WER) resta fuori.
    #[test]
    fn phase3_additions_pass_the_rules_guard() {
        use pc_rules::{Env, Guard};
        use std::collections::HashMap;
        use std::path::Path;
        let env = Env(HashMap::from([
            ("windir".to_string(), r"C:\Windows".to_string()),
            ("systemroot".to_string(), r"C:\Windows".to_string()),
            ("systemdrive".to_string(), "C:".to_string()),
            ("userprofile".to_string(), r"C:\Users\t".to_string()),
            ("localappdata".to_string(), r"C:\Users\t\AppData\Local".to_string()),
            ("appdata".to_string(), r"C:\Users\t\AppData\Roaming".to_string()),
            ("programdata".to_string(), r"C:\ProgramData".to_string()),
        ]));
        let g = Guard::new(&env);
        for p in [
            r"C:\Users\t\AppData\Local\Microsoft\Windows\Explorer\ThumbCacheToDelete\thm*.tmp",
            r"C:\Users\t\AppData\Local\Microsoft\Windows\Explorer\ThumbCacheToDelete\thm1A2B.tmp",
            r"C:\Windows\*.bak",
            r"C:\Windows\win.ini.bak",
            r"C:\Windows\*log.txt",
            r"C:\Windows\setuplog.txt",
            r"C:\Users\t\AppData\Local\ElevatedDiagnostics",
            r"C:\Users\t\AppData\Local\ElevatedDiagnostics\123\report.cab",
        ] {
            assert_eq!(g.protected(Path::new(p)), None, "{p}");
        }
        assert!(g.protected(Path::new(r"C:\Windows\System32\wbem\Logs\*.log")).is_some());
    }

    /// 500 file in una cartella finta, uno tenuto aperto senza condivisione:
    /// 499 rimossi, uno fallito; le sottocartelle svuotate se ne vanno, la radice resta.
    #[test]
    fn clean_removes_all_but_the_locked_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let mut found = super::Found::default();
        for i in 0..500 {
            let dir = t.path().join(format!("d{}", i % 10));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(format!("f{i}.tmp"));
            std::fs::write(&p, b"0123456789").unwrap();
            found.files.push((p, 10));
        }
        // Uno in sola lettura: si toglie lo stesso.
        let ro = &found.files[7].0;
        let mut perm = std::fs::metadata(ro).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(ro, perm).unwrap();
        found.count = 500;
        found.bytes = 5000;
        let locked_path = found.files[42].0.clone();
        let locked = std::fs::OpenOptions::new().read(true).share_mode(0).open(&locked_path).unwrap();
        let ticks = std::sync::atomic::AtomicUsize::new(0);
        let tick = |_: dc_cleaner::Tick| {
            ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        };
        let c = super::clean_files(&found, &[t.path().to_path_buf()], &tick, &dc_cleaner::Cancel::default());
        drop(locked);
        assert_eq!(c.removed, 499);
        assert_eq!(c.failed, 1);
        assert_eq!(c.freed, 4990);
        assert_eq!(c.failures.len(), 1);
        assert_eq!(c.failures[0].0, locked_path);
        assert!(!c.cancelled);
        assert!(ticks.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        // 9 cartelle vuote tolte, quella con il file bloccato resta, la radice resta.
        assert_eq!(c.dirs, 9);
        assert!(locked_path.exists());
        assert!(t.path().is_dir());
    }

    /// «Annulla» prima di iniziare: nessun file tolto.
    #[test]
    fn clean_cancelled_before_start_removes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.tmp");
        std::fs::write(&p, b"x").unwrap();
        let found = super::Found { files: vec![(p.clone(), 1)], bytes: 1, count: 1, skipped_dirs: 0 };
        let cancel = dc_cleaner::Cancel::default();
        cancel.cancel();
        let empty = t.path().join("vuota");
        std::fs::create_dir(&empty).unwrap();
        let c = super::clean_files(&found, &[t.path().to_path_buf()], &|_| {}, &cancel);
        assert_eq!((c.removed, c.failed, c.freed, c.dirs), (0, 0, 0, 0));
        assert!(c.cancelled);
        assert!(p.exists());
        assert!(empty.is_dir(), "annullata: le cartelle vuote non si toccano");
    }

    /// «Annulla» arrivato quando non restava nessun file da provare: la categoria non è
    /// annullata e le cartelle vuote si tolgono come sempre.
    #[test]
    fn cancel_with_nothing_left_is_not_a_cancelled_category() {
        let t = tempfile::tempdir().unwrap();
        let empty = t.path().join("vuota");
        std::fs::create_dir(&empty).unwrap();
        let cancel = dc_cleaner::Cancel::default();
        cancel.cancel();
        let c = super::clean_files(&super::Found::default(), &[t.path().to_path_buf()], &|_| {}, &cancel);
        assert!(!c.cancelled);
        assert_eq!(c.dirs, 1);
        assert!(!empty.exists());
    }

    #[test]
    fn phase3_additions_are_in_the_targets() {
        let has = |index: usize, dir_end: &str, pattern: &str| {
            super::targets(index)
                .iter()
                .any(|t| t.dir.ends_with(dir_end) && t.patterns.contains(&pattern))
        };
        assert!(has(6, r"Microsoft\Windows\Explorer\ThumbCacheToDelete", "thm*.tmp"));
        let win = std::path::PathBuf::from(crate::sys::windir());
        let root = super::targets(4).into_iter().find(|t| t.dir == win).expect("cartella di Windows");
        assert_eq!(root.patterns, vec!["*.log", "*.bak", "*log.txt"]);
        assert!(!root.recursive);
        assert!(has(5, "ElevatedDiagnostics", "*"));
        // `System32\wbem\Logs` no: la guardia delle regole esterne lo rifiuta.
        assert!(!super::targets(4).iter().any(|t| t.dir.ends_with(r"wbem\Logs")));
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn analyze_all() {
        for i in 0..super::CATEGORIES.len() {
            let t = std::time::Instant::now();
            let f = super::analyze(i, true);
            println!("{} -> {} files, {} in {:?}", super::CATEGORIES[i].name, f.count, crate::sys::fmt_size(f.bytes), t.elapsed());
        }
    }
}
