//! Registry scanner. Categories and keys from MECHANICS.md section 2.

use crate::sys;
use std::path::Path;
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
    Category { name: "DLL condivise mancanti", default_on: true },
    Category { name: "Estensioni file inutilizzate", default_on: true },
    Category { name: "Problemi ActiveX e classi", default_on: true },
    Category { name: "Librerie dei tipi", default_on: true },
    Category { name: "Caratteri (font)", default_on: true },
    Category { name: "Percorsi delle applicazioni", default_on: true },
    Category { name: "File della guida", default_on: true },
    Category { name: "Cartelle del programma di installazione", default_on: true },
    Category { name: "Software obsoleto", default_on: true },
    Category { name: "Esecuzione all'avvio", default_on: true },
    Category { name: "Cache MUI", default_on: true },
    Category { name: "Servizi Windows", default_on: false },
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
                    let words: Vec<u16> = v.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
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
                        out.push(issue(c, "DLL condivisa mancante", name.clone(), Fix::DeleteValue { root: Root::Hklm, path: path.into(), value: name }));
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
                    out.push(issue(c, "Estensione file inutilizzata", format!("{ext} → {progid}"), Fix::DeleteKey { root: Root::Hkcu, path: kp }));
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
                                out.push(issue(c, "Classe ActiveX/COM con file mancante", p, Fix::DeleteKey { root, path: kp.clone() }));
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
                        out.push(issue(c, "Libreria dei tipi mancante", paths[0].clone(), Fix::DeleteKey { root: Root::Hklm, path: vp }));
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
                        out.push(issue(c, "Carattere mancante", format!("{name}: {data}"), Fix::DeleteValue { root, path: path.into(), value: name }));
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
                        out.push(issue(c, "Percorso applicazione non valido", p, Fix::DeleteKey { root, path: kp }));
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
                            out.push(issue(c, "File della guida mancante", full, Fix::DeleteValue { root, path: path.into(), value: name }));
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
                    out.push(issue(c, "Cartella dell'installer mancante", name.clone(), Fix::DeleteValue { root: Root::Hklm, path: path.into(), value: name }));
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
                        out.push(issue(c, "Software obsoleto", format!("{name} ({p})"), Fix::DeleteKey { root, path: kp }));
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
                        out.push(issue(c, "Programma all'avvio mancante", format!("{name}: {p}"), Fix::DeleteValue { root, path: path.into(), value: name }));
                    }
                }
            }
        }
        10 => {
            let path = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell\\MuiCache";
            for (name, _) in values(Root::Hkcu, path) {
                if let Some(p) = missing_cmd(&name) {
                    out.push(issue(c, "Voce MUI non valida", p, Fix::DeleteValue { root: Root::Hkcu, path: path.into(), value: name }));
                }
            }
        }
        11 => {
            let base = "SYSTEM\\CurrentControlSet\\Services";
            for svc in subkeys(Root::Hklm, base) {
                let kp = format!("{base}\\{svc}");
                let Some(k) = open(Root::Hklm, &kp) else { continue };
                let ty = k.get_value::<u32, _>("Type").unwrap_or(0);
                if ty & 0x30 == 0 {
                    continue;
                }
                let Ok(img) = k.get_value::<String, _>("ImagePath") else { continue };
                if let Some(p) = missing_cmd(&img) {
                    let mut i = issue(c, "Servizio con file mancante", format!("{svc}: {p}"), Fix::DeleteKey { root: Root::Hklm, path: kp });
                    i.selected = false;
                    out.push(i);
                }
            }
        }
        _ => {}
    }
    out
}

/// Writes one .reg file with every key the fixes touch (via `reg export`). Returns its path.
pub fn backup(issues: &[&Issue]) -> Result<std::path::PathBuf, String> {
    let dir = sys::data_dir();
    let out = dir.join(format!("backup_registro_{}.reg", sys::stamp()));
    let tmp = dir.join("export_tmp.reg");
    let mut keys: Vec<(Root, String)> = issues.iter().map(|i| i.fix.export_key()).collect();
    keys.sort_by(|a, b| (a.0.short(), &a.1).cmp(&(b.0.short(), &b.1)));
    keys.dedup();
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for (root, path) in keys {
        let _ = std::fs::remove_file(&tmp);
        let full = format!("{}\\{}", root.short(), path);
        let st = sys::hidden_cmd("reg").args(["export", &full]).arg(&tmp).arg("/y").status();
        if !matches!(st, Ok(s) if s.success()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&tmp) else { continue };
        let words: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let s = String::from_utf16_lossy(&words);
        let s = s.trim_start_matches('\u{feff}');
        let body = s.split_once('\n').map(|x| x.1).unwrap_or("");
        text.push_str(body);
    }
    let _ = std::fs::remove_file(&tmp);
    let mut data = vec![0xFF, 0xFE];
    for w in text.encode_utf16() {
        data.extend_from_slice(&w.to_le_bytes());
    }
    std::fs::write(&out, data).map_err(|e| e.to_string())?;
    Ok(out)
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
