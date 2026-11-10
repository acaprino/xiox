//! Temporary-file cleaner. Locations and ages come from MECHANICS.md section 1.

use crate::sys;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Option "only delete files in Windows Temp folders older than 24 hours" (string 0x141ab7df8).
pub const TEMP_MIN_AGE_HOURS: u64 = 24;
/// Age for "old" Prefetch data: estimated, not found in the code.
pub const PREFETCH_MIN_AGE_DAYS: u64 = 14;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
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
            t(win.clone(), &["*.log"], false),
        ],
        5 => vec![
            t(progdata.join("Microsoft\\Windows\\WER\\ReportArchive"), &["*"], true),
            t(progdata.join("Microsoft\\Windows\\WER\\ReportQueue"), &["*"], true),
            t(progdata.join("Microsoft\\Windows\\WER\\Temp"), &["*"], true),
            t(local.join("Microsoft\\Windows\\WER\\ReportArchive"), &["*"], true),
            t(local.join("Microsoft\\Windows\\WER\\ReportQueue"), &["*"], true),
        ],
        6 => vec![t(local.join("Microsoft\\Windows\\Explorer"), &["thumbcache_*.db"], false)],
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

fn remove_empty_dirs(dir: &Path, is_root: bool) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_dir() && md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    remove_empty_dirs(&e.path(), false);
                }
            }
        }
    }
    if !is_root {
        let _ = std::fs::remove_dir(dir); // fails when not empty, that's fine
    }
}

pub fn clean(index: usize, found: &Found) -> Cleaned {
    let cat = &CATEGORIES[index];
    let mut r = Cleaned::default();
    if cat.kind == Kind::RecycleBin {
        if found.count > 0 && sys::empty_recycle_bin() {
            r.freed = found.bytes;
            r.removed = found.count;
        } else if found.count > 0 {
            r.failed = found.count;
        }
        return r;
    }
    for (path, size) in &found.files {
        let mut ok = std::fs::remove_file(path).is_ok();
        if !ok {
            if let Ok(md) = std::fs::metadata(path) {
                if md.file_attributes() & FILE_ATTRIBUTE_READONLY != 0 {
                    let mut p = md.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    let _ = std::fs::set_permissions(path, p);
                    ok = std::fs::remove_file(path).is_ok();
                }
            }
        }
        if ok {
            r.freed += size;
            r.removed += 1;
        } else {
            r.failed += 1;
        }
    }
    for target in targets(index) {
        if target.recursive && target.dir.is_dir() {
            remove_empty_dirs(&target.dir, true);
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
