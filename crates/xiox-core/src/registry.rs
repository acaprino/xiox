//! Registry scanner: lists broken or leftover registry entries by category, backs up the keys and repairs them.

use crate::{regbackup, sys};
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::RegKey;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Root {
    Hkcu,
    Hklm,
}

impl Root {
    pub fn short(self) -> &'static str {
        match self {
            Root::Hkcu => "HKCU",
            Root::Hklm => "HKLM",
        }
    }
    fn rule_root(self) -> xiox_rules::RegRoot {
        match self {
            Root::Hkcu => xiox_rules::RegRoot::Hkcu,
            Root::Hklm => xiox_rules::RegRoot::Hklm,
        }
    }
    fn key(self) -> RegKey {
        RegKey::predef(match self {
            Root::Hkcu => HKEY_CURRENT_USER,
            Root::Hklm => HKEY_LOCAL_MACHINE,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Fix {
    DeleteValue { root: Root, path: String, value: String },
    DeleteKey { root: Root, path: String },
}

impl Fix {
    pub fn key_text(&self) -> String {
        match self {
            Fix::DeleteValue { root, path, .. } | Fix::DeleteKey { root, path } => format!("{}\\{}", root.short(), path),
        }
    }
    fn export_key(&self) -> (Root, String) {
        match self {
            Fix::DeleteValue { root, path, .. } | Fix::DeleteKey { root, path } => (*root, path.clone()),
        }
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct Issue {
    pub category: usize,
    pub problem: String,
    pub data: String,
    pub fix: Fix,
    pub selected: bool,
}

pub struct Category {
    pub name: &'static str,
    pub default_on: bool,
}

pub const CATEGORIES: &[Category] = &[
    Category { name: "Missing shared DLLs", default_on: true },
    Category { name: "Unused file extensions", default_on: true },
    Category { name: "ActiveX and class problems", default_on: true },
    Category { name: "Type libraries", default_on: true },
    Category { name: "Fonts", default_on: true },
    Category { name: "Application paths", default_on: true },
    Category { name: "Help files", default_on: true },
    Category { name: "Installer folders", default_on: true },
    Category { name: "Obsolete software", default_on: true },
    Category { name: "Run at startup", default_on: true },
    Category { name: "MUI cache", default_on: true },
];

const KNOWN_EXT: &[&str] = &[".exe", ".dll", ".ocx", ".sys", ".cpl", ".scr", ".com", ".chm", ".hlp", ".tlb", ".olb", ".msc", ".bat", ".cmd", ".ax"];

/// Pulls an absolute file path out of a registry value (command line, icon spec...).
pub fn extract_path(raw: &str) -> Option<String> {
    let mut s = sys::expand_env(raw.trim());
    if s.starts_with('@') {
        return None;
    }
    if let Some(rest) = s.strip_prefix("\\??\\") {
        s = rest.to_string();
    }
    if s.len() > 12 && s[..12].eq_ignore_ascii_case("\\SystemRoot\\") {
        s = format!("{}\\{}", sys::windir(), &s[12..]);
    }
    let path = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else {
        let lower = s.to_lowercase();
        let mut cut = None;
        for ext in KNOWN_EXT {
            if let Some(i) = lower.find(ext) {
                let end = i + ext.len();
                let next = lower[end..].chars().next();
                if matches!(next, None | Some(' ') | Some(',') | Some('"') | Some('/') | Some('.') | Some('\\')) {
                    cut = Some(cut.map_or(end, |c: usize| c.min(end)));
                }
            }
        }
        match cut {
            Some(end) => s[..end].to_string(),
            None => s.split(',').next().unwrap_or("").to_string(),
        }
    };
    let p = path.trim().to_string();
    let b = p.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        Some(p)
    } else {
        None
    }
}

/// True when the path is on a drive that is present but the file/folder itself is gone.
pub fn missing(path: &str) -> bool {
    if path.to_lowercase().contains("\\windowsapps\\") {
        return false; // packaged apps: not readable, never assume missing
    }
    let drive = format!("{}\\", &path[..2]);
    // Only "not found" counts: a locked (access denied) file or folder is not missing.
    Path::new(&drive).exists()
        && matches!(std::fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

fn is_abs(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

fn missing_cmd(raw: &str) -> Option<String> {
    extract_path(raw).filter(|p| missing(p))
}

fn open(root: Root, path: &str) -> Option<RegKey> {
    root.key().open_subkey_with_flags(path, KEY_READ).ok()
}

fn values(root: Root, path: &str) -> Vec<(String, String)> {
    let Some(k) = open(root, path) else { return vec![] };
    k.enum_values()
        .flatten()
        .map(|(n, v)| {
            let text = match v.vtype {
                REG_SZ | REG_EXPAND_SZ => {
                    let words: Vec<u16> = v.bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    String::from_utf16_lossy(&words).trim_end_matches('\0').to_string()
                }
                _ => String::new(),
            };
            (n, text)
        })
        .collect()
}

fn subkeys(root: Root, path: &str) -> Vec<String> {
    open(root, path).map(|k| k.enum_keys().flatten().collect()).unwrap_or_default()
}

fn default_value(root: Root, path: &str) -> Option<String> {
    open(root, path).and_then(|k| k.get_value::<String, _>("").ok())
}

fn issue(category: usize, problem: &str, data: String, fix: Fix) -> Issue {
    Issue { category, problem: problem.to_string(), data, fix, selected: true }
}

pub fn scan(category: usize, progress: &dyn Fn(&str)) -> Vec<Issue> {
    let mut out = Vec::new();
    let c = category;
    match c {
        0 => {
            for path in [
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\SharedDlls",
                "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\SharedDlls",
            ] {
                for (name, _) in values(Root::Hklm, path) {
                    if is_abs(&name) && missing(&name) {
                        out.push(issue(c, "Missing shared DLL", name.clone(), Fix::DeleteValue { root: Root::Hklm, path: path.into(), value: name }));
                    }
                }
            }
        }
        1 => {
            let base = "Software\\Classes";
            for ext in subkeys(Root::Hkcu, base).into_iter().filter(|k| k.starts_with('.')) {
                let kp = format!("{base}\\{ext}");
                let Some(k) = open(Root::Hkcu, &kp) else { continue };
                if k.enum_keys().flatten().next().is_some() {
                    continue;
                }
                let Ok(progid) = k.get_value::<String, _>("") else { continue };
                if progid.is_empty() {
                    continue;
                }
                let exists = RegKey::predef(HKEY_CLASSES_ROOT).open_subkey_with_flags(&progid, KEY_READ).is_ok();
                if !exists {
                    out.push(issue(c, "Unused file extension", format!("{ext} → {progid}"), Fix::DeleteKey { root: Root::Hkcu, path: kp }));
                }
            }
        }
        2 => {
            for (root, base) in [(Root::Hkcu, "Software\\Classes\\CLSID"), (Root::Hklm, "SOFTWARE\\Classes\\CLSID")] {
                progress(base);
                for clsid in subkeys(root, base) {
                    let kp = format!("{base}\\{clsid}");
                    for server in ["InprocServer32", "LocalServer32"] {
                        if let Some(v) = default_value(root, &format!("{kp}\\{server}")) {
                            if let Some(p) = missing_cmd(&v) {
                                out.push(issue(c, "ActiveX/COM class with a missing file", p, Fix::DeleteKey { root, path: kp.clone() }));
                                break;
                            }
                        }
                    }
                }
            }
        }
        3 => {
            let base = "SOFTWARE\\Classes\\TypeLib";
            for guid in subkeys(Root::Hklm, base) {
                for ver in subkeys(Root::Hklm, &format!("{base}\\{guid}")) {
                    let vp = format!("{base}\\{guid}\\{ver}");
                    let mut paths = Vec::new();
                    for lcid in subkeys(Root::Hklm, &vp) {
                        for arch in ["win32", "win64"] {
                            if let Some(v) = default_value(Root::Hklm, &format!("{vp}\\{lcid}\\{arch}")) {
                                if let Some(p) = extract_path(&v) {
                                    paths.push(p);
                                }
                            }
                        }
                    }
                    if !paths.is_empty() && paths.iter().all(|p| missing(p)) {
                        out.push(issue(c, "Missing type library", paths[0].clone(), Fix::DeleteKey { root: Root::Hklm, path: vp }));
                    }
                }
            }
        }
        4 => {
            let path = "Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
            let win_fonts = format!("{}\\Fonts", sys::windir());
            let user_fonts = format!("{}\\Microsoft\\Windows\\Fonts", sys::env_or("LOCALAPPDATA", ""));
            for root in [Root::Hklm, Root::Hkcu] {
                for (name, data) in values(root, path) {
                    if data.is_empty() {
                        continue;
                    }
                    let candidates: Vec<String> = match extract_path(&data) {
                        Some(p) => vec![p],
                        None if !data.contains('\\') => vec![format!("{win_fonts}\\{data}"), format!("{user_fonts}\\{data}")],
                        None => continue,
                    };
                    if candidates.iter().all(|p| missing(p)) {
                        out.push(issue(c, "Missing font", format!("{name}: {data}"), Fix::DeleteValue { root, path: path.into(), value: name }));
                    }
                }
            }
        }
        5 => {
            let base = "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths";
            for root in [Root::Hklm, Root::Hkcu] {
                for app in subkeys(root, base) {
                    let kp = format!("{base}\\{app}");
                    if let Some(p) = default_value(root, &kp).and_then(|v| missing_cmd(&v)) {
                        out.push(issue(c, "Invalid application path", p, Fix::DeleteKey { root, path: kp }));
                    }
                }
            }
        }
        6 => {
            for root in [Root::Hklm, Root::Hkcu] {
                for path in ["Software\\Microsoft\\Windows\\Help", "Software\\Microsoft\\Windows\\HTML Help"] {
                    for (name, folder) in values(root, path) {
                        if folder.is_empty() || name.is_empty() {
                            continue;
                        }
                        let full = format!("{}\\{}", sys::expand_env(&folder).trim_end_matches('\\'), name);
                        if extract_path(&full).is_some() && missing(&full) {
                            out.push(issue(c, "Missing help file", full, Fix::DeleteValue { root, path: path.into(), value: name }));
                        }
                    }
                }
            }
        }
        7 => {
            let path = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\Folders";
            for (name, _) in values(Root::Hklm, path) {
                let p = name.trim_end_matches('\\');
                if extract_path(p).is_some() && missing(p) {
                    out.push(issue(c, "Missing installer folder", name.clone(), Fix::DeleteValue { root: Root::Hklm, path: path.into(), value: name }));
                }
            }
        }
        8 => {
            for (root, base) in [
                (Root::Hklm, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
                (Root::Hklm, "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
                (Root::Hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
            ] {
                for app in subkeys(root, base) {
                    let kp = format!("{base}\\{app}");
                    let Some(k) = open(root, &kp) else { continue };
                    if k.get_value::<u32, _>("WindowsInstaller").unwrap_or(0) == 1 {
                        continue;
                    }
                    let Ok(cmd) = k.get_value::<String, _>("UninstallString") else { continue };
                    if cmd.to_lowercase().contains("msiexec") {
                        continue;
                    }
                    if let Some(p) = missing_cmd(&cmd) {
                        let name = k.get_value::<String, _>("DisplayName").unwrap_or_else(|_| app.clone());
                        out.push(issue(c, "Obsolete software", format!("{name} ({p})"), Fix::DeleteKey { root, path: kp }));
                    }
                }
            }
        }
        9 => {
            for (root, path) in [
                (Root::Hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                (Root::Hklm, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run"),
                (Root::Hklm, "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run"),
            ] {
                for (name, cmd) in values(root, path) {
                    if let Some(p) = missing_cmd(&cmd) {
                        out.push(issue(c, "Missing startup program", format!("{name}: {p}"), Fix::DeleteValue { root, path: path.into(), value: name }));
                    }
                }
            }
        }
        10 => {
            let path = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell\\MuiCache";
            for (name, _) in values(Root::Hkcu, path) {
                if let Some(p) = missing_cmd(&name) {
                    out.push(issue(c, "Invalid MUI entry", p, Fix::DeleteValue { root: Root::Hkcu, path: path.into(), value: name }));
                }
            }
        }
        _ => {}
    }
    out
}

/// `.reg` backup of the keys the repairs touch, in `Documents\Xiox\registry-backup-<stamp>.reg`
/// (`_2`, `_3`… if the name is taken), with the safe export of [`regbackup`]. The app never reads
/// it back: the user double-clicks it. `Ok(None)` if no key exists any more. `Err` if even one
/// existing key cannot be saved: then nothing is repaired (see [`fix_all`]).
pub fn backup(issues: &[&Issue]) -> Result<Option<PathBuf>, String> {
    let stem = format!("registry-backup-{}", sys::stamp());
    backup_to(&sys::data_dir(), &stem, &regbackup::admin_work_dir, &key_exists, &regbackup::reg_exe_export, issues)
}

/// Come [`backup`], con cartelle, esistenza delle chiavi ed esportazione
/// iniettate. Una chiave che non si sa se esista fa fallire tutto; una che non
/// c'è si salta.
pub fn backup_to(
    dest: &Path,
    stem: &str,
    work: &regbackup::WorkDir,
    exists: &dyn Fn(Root, &str) -> Result<bool, String>,
    export: &regbackup::Export,
    issues: &[&Issue],
) -> Result<Option<PathBuf>, String> {
    let mut keys = Vec::new();
    for issue in issues {
        let (root, path) = issue.fix.export_key();
        let full = format!("{}\\{path}", root.short());
        if exists(root, &path).map_err(|e| format!("{full}: {e}"))? {
            keys.push((root.rule_root(), path));
        }
    }
    regbackup::write_backup(dest, stem, work, export, &keys)
}

/// Esistenza vera di una chiave (sola lettura, vista a 64 bit): `Err` se non
/// si apre per un motivo diverso da «non trovata».
fn key_exists(root: Root, path: &str) -> Result<bool, String> {
    match root.key().open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// Backup delle chiavi di una riparazione (in produzione [`backup`]).
pub type BackupFn<'a> = dyn Fn(&[&Issue]) -> Result<Option<PathBuf>, String> + 'a;

/// Esito di una riparazione.
#[derive(Debug, Default)]
pub struct FixOutcome {
    pub ok: usize,
    pub failed: usize,
    pub backup: Option<PathBuf>,
    /// Backup chiesto e non riuscito: il motivo. Allora non si è riparato nulla.
    pub refused: Option<String>,
}

/// Ripara `chosen`. Con `with_backup` prima chiama `backup`: se fallisce non si
/// ripara nulla e `refused` dice perché. `progress(k)` prima della voce `k`.
pub fn fix_all(
    chosen: &[Issue],
    with_backup: bool,
    backup: &BackupFn,
    apply: &mut dyn FnMut(&Fix) -> bool,
    progress: &mut dyn FnMut(usize),
) -> FixOutcome {
    let mut out = FixOutcome::default();
    if with_backup {
        let refs: Vec<&Issue> = chosen.iter().collect();
        match backup(&refs) {
            Ok(path) => out.backup = path,
            Err(e) => {
                out.refused = Some(e);
                return out;
            }
        }
    }
    for (k, issue) in chosen.iter().enumerate() {
        progress(k);
        if apply(&issue.fix) {
            out.ok += 1;
        } else {
            out.failed += 1;
        }
    }
    out
}

pub fn apply(fix: &Fix) -> bool {
    match fix {
        Fix::DeleteValue { root, path, value } => root
            .key()
            .open_subkey_with_flags(path, KEY_SET_VALUE)
            .and_then(|k| k.delete_value(value))
            .is_ok(),
        Fix::DeleteKey { root, path } => {
            let (parent, name) = path.rsplit_once('\\').unwrap_or(("", path));
            root.key()
                .open_subkey_with_flags(parent, KEY_ALL_ACCESS)
                .and_then(|k| k.delete_subkey_all(name))
                .is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_path;
    #[test]
    fn paths() {
        assert_eq!(extract_path("\"C:\\Program Files\\A b\\x.exe\" /bg").as_deref(), Some("C:\\Program Files\\A b\\x.exe"));
        assert_eq!(extract_path("C:\\Program Files\\A b\\x.exe -m").as_deref(), Some("C:\\Program Files\\A b\\x.exe"));
        assert_eq!(extract_path("C:\\x\\y.dll,-100").as_deref(), Some("C:\\x\\y.dll"));
        assert_eq!(extract_path("mscoree.dll"), None);
        assert_eq!(extract_path("C:\\a\\b.exe.FriendlyAppName").as_deref(), Some("C:\\a\\b.exe"));
    }
}

/// Backup e riparazione con registro, esportazione e cartelle finti: nessun
/// test tocca il registro vero o `%ProgramData%`.
#[cfg(test)]
mod backup_tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::path::PathBuf;

    fn issue(root: Root, path: &str, value: Option<&str>) -> Issue {
        let fix = match value {
            Some(v) => Fix::DeleteValue { root, path: path.into(), value: v.into() },
            None => Fix::DeleteKey { root, path: path.into() },
        };
        Issue { category: 0, problem: "p".into(), data: "d".into(), fix, selected: true }
    }

    /// Esportazione finta: scrive un `.reg` UTF-16 come `reg.exe`.
    fn fake_export(root: xiox_rules::RegRoot, path: &str, out: &Path) -> Result<(), String> {
        let text = format!(
            "Windows Registry Editor Version 5.00\r\n\r\n[{}\\{path}]\r\n\"x\"=\"1\"\r\n",
            crate::regbackup::root_name(root)
        );
        let mut data = vec![0xFF, 0xFE];
        for w in text.encode_utf16() {
            data.extend_from_slice(&w.to_le_bytes());
        }
        fs::write(out, data).map_err(|e| e.to_string())
    }

    fn all_exist(_: Root, _: &str) -> Result<bool, String> {
        Ok(true)
    }

    /// Cartella di lavoro finta (in produzione: solo amministratori, nome casuale).
    fn work_in(root: &Path) -> impl Fn() -> Result<PathBuf, String> + '_ {
        move || {
            for n in 0.. {
                let d = root.join(format!("work{n}"));
                if fs::create_dir(&d).is_ok() {
                    return Ok(d);
                }
            }
            unreachable!()
        }
    }

    fn reg_text(p: &Path) -> String {
        let b = fs::read(p).unwrap();
        let w: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16(&w).unwrap()
    }

    fn dirs(t: &Path) -> (PathBuf, PathBuf) {
        let dest = t.join("Documenti");
        let work = t.join("admin");
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&work).unwrap();
        (dest, work)
    }

    #[test]
    fn two_registry_backups_in_the_same_second_get_two_files() {
        let t = tempfile::tempdir().unwrap();
        let (dest, work_root) = dirs(t.path());
        let work = work_in(&work_root);
        let a = issue(Root::Hkcu, r"Software\FakeApp\A", None);
        let b = issue(Root::Hklm, r"SOFTWARE\FakeApp", Some("Mru"));
        let stem = "registry-backup-20261111_120000";
        let p1 = backup_to(&dest, stem, &work, &all_exist, &fake_export, &[&a]).unwrap().unwrap();
        let p2 = backup_to(&dest, stem, &work, &all_exist, &fake_export, &[&b]).unwrap().unwrap();
        assert_ne!(p1, p2);
        assert_eq!(p1.file_name().unwrap(), "registry-backup-20261111_120000.reg");
        assert_eq!(p2.file_name().unwrap(), "registry-backup-20261111_120000_2.reg");
        assert!(reg_text(&p1).contains(r"[HKCU\Software\FakeApp\A]"));
        assert!(reg_text(&p2).contains(r"[HKLM\SOFTWARE\FakeApp]"));
        // Né cartelle di lavoro né temporanei restano.
        assert_eq!(fs::read_dir(&work_root).unwrap().count(), 0);
    }

    #[test]
    fn registry_backup_fails_closed_when_a_key_does_not_export() {
        let t = tempfile::tempdir().unwrap();
        let (dest, work_root) = dirs(t.path());
        let work = work_in(&work_root);
        let a = issue(Root::Hkcu, r"Software\FakeApp\A", None);
        let b = issue(Root::Hkcu, r"Software\FakeApp\B", None);
        let export = |root: xiox_rules::RegRoot, path: &str, out: &Path| {
            if path.ends_with('B') { Err("accesso negato".to_string()) } else { fake_export(root, path, out) }
        };
        let r = backup_to(&dest, "b", &work, &all_exist, &export, &[&a, &b]);
        assert!(r.as_ref().is_err_and(|e| e.contains(r"HKCU\Software\FakeApp\B")), "{r:?}");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&work_root).unwrap().count(), 0);
    }

    #[test]
    fn registry_backup_fails_closed_without_an_admin_work_folder() {
        let t = tempfile::tempdir().unwrap();
        let (dest, _) = dirs(t.path());
        let no_work = || Err::<PathBuf, String>("cartella di lavoro non protetta".into());
        let exported = Cell::new(0);
        let export = |root: xiox_rules::RegRoot, path: &str, out: &Path| {
            exported.set(exported.get() + 1);
            fake_export(root, path, out)
        };
        let a = issue(Root::Hkcu, r"Software\FakeApp\A", None);
        assert!(backup_to(&dest, "b", &no_work, &all_exist, &export, &[&a]).is_err());
        assert_eq!(exported.get(), 0);
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    /// Una chiave che non si sa se esista (errore di apertura) fa fallire il
    /// backup; una che non c'è si salta (non c'è niente da riparare).
    #[test]
    fn registry_backup_checks_existence_strictly() {
        let t = tempfile::tempdir().unwrap();
        let (dest, work_root) = dirs(t.path());
        let work = work_in(&work_root);
        let gone = issue(Root::Hkcu, r"Software\FakeApp\Gone", None);
        let broken = issue(Root::Hkcu, r"Software\FakeApp\Broken", None);
        let exists = |_: Root, p: &str| match p {
            r"Software\FakeApp\Gone" => Ok(false),
            r"Software\FakeApp\Broken" => Err("accesso negato".to_string()),
            _ => Ok(true),
        };
        assert_eq!(backup_to(&dest, "b", &work, &exists, &fake_export, &[&gone]), Ok(None));
        assert!(backup_to(&dest, "b", &work, &exists, &fake_export, &[&gone, &broken]).is_err());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn registry_backup_refuses_a_destination_that_is_a_link() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("altrove");
        fs::create_dir_all(&target).unwrap();
        let dest = t.path().join("Documenti");
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&dest)
            .arg(&target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("giunzione non creata: test saltato");
            return;
        }
        let work = work_in(t.path());
        let a = issue(Root::Hkcu, r"Software\FakeApp\A", None);
        let r = backup_to(&dest, "b", &work, &all_exist, &fake_export, &[&a]);
        assert!(r.is_err(), "{r:?}");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    /// Backup chiesto e non riuscito: nessuna riparazione, e il motivo.
    #[test]
    fn failed_backup_stops_the_repair() {
        let chosen = vec![issue(Root::Hkcu, r"Software\FakeApp\A", None)];
        let applied = RefCell::new(Vec::new());
        let out = fix_all(
            &chosen,
            true,
            &|_| Err("reg.exe ended with code 1".to_string()),
            &mut |f| {
                applied.borrow_mut().push(f.key_text());
                true
            },
            &mut |_| {},
        );
        assert!(applied.borrow().is_empty());
        assert_eq!((out.ok, out.failed, out.backup.clone()), (0, 0, None));
        assert!(out.refused.as_deref().is_some_and(|r| r.contains("reg.exe ended with code 1")), "{:?}", out.refused);
    }

    #[test]
    fn repair_runs_after_a_good_backup_or_without_one() {
        let chosen = vec![
            issue(Root::Hkcu, r"Software\FakeApp\A", None),
            issue(Root::Hkcu, r"Software\FakeApp\B", Some("v")),
        ];
        let backups = Cell::new(0);
        let backup = |_: &[&Issue]| {
            backups.set(backups.get() + 1);
            Ok(Some(PathBuf::from(r"C:\finto\registry-backup-x.reg")))
        };
        let mut n = 0;
        let out = fix_all(&chosen, true, &backup, &mut |_| { n += 1; n == 1 }, &mut |_| {});
        assert_eq!((out.ok, out.failed, out.refused.clone()), (1, 1, None));
        assert_eq!(out.backup, Some(PathBuf::from(r"C:\finto\registry-backup-x.reg")));
        assert_eq!(backups.get(), 1);
        // Senza backup chiesto: non si chiama e si ripara come prima.
        let out = fix_all(&chosen, false, &backup, &mut |_| true, &mut |_| {});
        assert_eq!((out.ok, out.failed, out.backup, out.refused), (2, 0, None, None));
        assert_eq!(backups.get(), 1);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn scan_all() {
        for c in 0..super::CATEGORIES.len() {
            let t = std::time::Instant::now();
            let v = super::scan(c, &|_| {});
            println!("{} -> {} issues in {:?}", super::CATEGORIES[c].name, v.len(), t.elapsed());
            for i in v.iter().take(4) {
                println!("    {} | {} | {}", i.problem, i.data, i.fix.key_text());
            }
        }
    }
}
