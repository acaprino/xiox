//! Percorsi che una regola esterna non può mai toccare.

use std::path::Path;

use crate::model::Env;

/// `Some(motivo in italiano)` se il percorso è protetto.
pub fn protected(path: &Path, env: &Env) -> Option<&'static str> {
    let comps = components(&path.to_string_lossy());
    if comps.iter().any(|c| c == "..") {
        return Some("il percorso contiene «..» e può uscire dalla sua cartella");
    }
    // Radice di un disco (o percorso vuoto).
    if comps.len() <= 1 {
        return Some("radice di un disco");
    }
    // Cartelle prese per intero, e tutto ciò che le contiene.
    for var in WHOLE_DIRS {
        if let Some(value) = env.get(var) {
            let dir = components(value);
            if !dir.is_empty() && dir.starts_with(&comps) {
                return Some("cartella di sistema o del profilo presa per intero");
            }
        }
    }
    // Cartelle di sistema e il loro contenuto.
    for var in ["windir", "systemroot"] {
        if let Some(value) = env.get(var) {
            let win = components(value);
            if win.is_empty() {
                continue;
            }
            for sub in SYSTEM_SUBDIRS {
                let mut dir = win.clone();
                dir.push((*sub).to_string());
                if comps.starts_with(&dir) {
                    return Some("cartella di sistema di Windows");
                }
            }
        }
    }
    if comps.iter().any(|c| c == ".git") {
        return Some("cartella .git di un repository");
    }
    if let Some(last) = comps.last()
        && CREDENTIAL_FILES.contains(&last.as_str())
    {
        return Some("file di credenziali");
    }
    None
}

/// Variabili le cui cartelle non si toccano prese intere (le sottocartelle sì).
const WHOLE_DIRS: &[&str] = &[
    "windir",
    "systemroot",
    "programfiles",
    "programfiles(x86)",
    "programdata",
    "userprofile",
    "appdata",
    "localappdata",
    "systemdrive",
];

const SYSTEM_SUBDIRS: &[&str] = &["system32", "syswow64", "winsxs"];

const CREDENTIAL_FILES: &[&str] = &[".credentials.json", "auth.json", "credentials.toml", "token"];

/// Componenti in minuscolo, con `\` o `/` come separatori, senza prefisso
/// `\\?\`, senza componenti vuoti né `.`. `..` resta (lo rifiuta il chiamante).
fn components(raw: &str) -> Vec<String> {
    let s = raw.replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.split('\\')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn env() -> Env {
        let pairs = [
            ("windir", r"C:\Windows"),
            ("systemroot", r"C:\Windows"),
            ("systemdrive", "C:"),
            ("programfiles", r"C:\Program Files"),
            ("programfiles(x86)", r"C:\Program Files (x86)"),
            ("programdata", r"C:\ProgramData"),
            ("userprofile", r"C:\Users\u"),
            ("appdata", r"C:\Users\u\AppData\Roaming"),
            ("localappdata", r"C:\Users\u\AppData\Local"),
        ];
        Env(pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>())
    }

    fn refused(p: &str) -> bool {
        protected(&PathBuf::from(p), &env()).is_some()
    }

    #[test]
    fn protected_roots_are_refused() {
        for p in [
            "C:\\",
            "C:",
            r"c:\windows",
            r"C:\Windows",
            r"C:\Windows\System32\x",
            r"C:\Windows\SysWOW64",
            r"C:\Windows\WinSxS\amd64_x\y.dll",
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users\u",
            "C:\\Users\\u\\",
            r"C:\Users\u\AppData\Roaming",
            r"C:\Users\u\AppData\Local",
            r"C:\Users\u\AppData",
            r"\\?\C:\Users\u",
            r"D:\proj\.git",
            r"D:\proj\.git\objects\ab",
            r"C:\Users\u\.codex\auth.json",
            r"C:\Users\u\.claude\.credentials.json",
            r"C:\Users\u\.cargo\credentials.toml",
            r"C:\Users\u\.config\TOKEN",
        ] {
            assert!(refused(p), "doveva essere protetto: {p}");
        }
    }

    #[test]
    fn ordinary_subfolders_are_allowed() {
        for p in [
            r"C:\Users\u\AppData\Roaming\Foo\Cache",
            r"C:\Users\u\AppData\Local\Foo",
            r"C:\Program Files\Foo\log",
            r"C:\ProgramData\Foo",
            r"C:\Windows\Temp",
            r"C:\Users\u\.cargo\registry\cache",
            r"D:\proj\target",
            r"D:\proj\.github\x",
        ] {
            assert!(!refused(p), "non doveva essere protetto: {p}");
        }
    }

    #[test]
    fn parent_dir_components_are_refused() {
        assert!(refused(r"C:\Users\u\AppData\Local\Foo\..\..\..\Windows"));
        assert!(refused(r"C:\Users\u\AppData\Local\Foo\.."));
    }

    #[test]
    fn empty_environment_still_refuses_roots_and_dot_git() {
        let e = Env::default();
        assert!(protected(Path::new("C:\\"), &e).is_some());
        assert!(protected(Path::new(r"D:\a\.git"), &e).is_some());
        assert!(protected(Path::new(r"C:\x\auth.json"), &e).is_some());
        assert!(protected(Path::new(r"C:\x\y"), &e).is_none());
    }
}
