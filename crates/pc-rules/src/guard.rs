//! Percorsi che una regola esterna non può mai toccare.

use std::path::Path;

use crate::model::Env;
use crate::wildcard::wildcard_match;

/// `Some(motivo in italiano)` se il percorso è protetto.
///
/// Il percorso si normalizza prima del confronto, come fa Windows: `/` come
/// `\`, prefisso `\\?\` tolto, punti e spazi finali tolti da ogni componente,
/// suffisso di flusso (`:nome`, `::$DATA`) tolto, maiuscole ignorate. Si
/// accettano solo percorsi assoluti con lettera di unità; UNC, `\??\`,
/// volumi, percorsi relativi e nomi brevi 8.3 (`PROGRA~1`) si rifiutano,
/// perché non si possono confrontare con certezza.
///
/// I caratteri jolly (`*`, `?`) valgono "qualunque cosa": si rifiuta il
/// percorso se può coincidere con una cartella protetta o con un suo
/// antenato, o se è una cartella protetta seguita solo da componenti jolly
/// (`C:\Program Files\*`).
///
/// **Chi chiama deve applicarla a ogni percorso concreto** (dopo la
/// risoluzione dei jolly: cartella più nome del file trovato), non solo alla
/// cartella di base della regola. `.git` e i nomi di file di credenziali si
/// cercano in ogni componente; le cartelle `.codex`, `.claude`, `.ssh`,
/// `.gnupg`, `.aws`, `.azure`, `.kube` non si toccano prese intere (le
/// sottocartelle sì, se non sono credenziali).
pub fn protected(path: &Path, env: &Env) -> Option<&'static str> {
    let comps = match normalize(&path.to_string_lossy(), true) {
        Ok(c) => c,
        Err(reason) => return Some(reason),
    };
    // Radice di un disco.
    if comps.len() <= 1 {
        return Some("radice di un disco");
    }
    // Cartelle prese per intero, i loro antenati e "cartella + solo jolly".
    for dir in whole_dirs(env) {
        if covers_whole(&comps, &dir) {
            return Some("cartella di sistema o del profilo presa per intero");
        }
    }
    // Cartelle dei documenti dell'utente prese per intero (le sottocartelle sì).
    for dir in user_content_dirs(env) {
        if covers_whole(&comps, &dir) {
            return Some("cartella dei documenti dell'utente presa per intero");
        }
    }
    // Cartelle di sistema e il loro contenuto.
    for win in windows_dirs(env) {
        for sub in SYSTEM_SUBDIRS {
            let mut dir = win.clone();
            dir.push((*sub).to_string());
            if comps.len() >= dir.len() && prefix_matches(&comps, &dir) {
                return Some("cartella di sistema di Windows");
            }
        }
    }
    if comps.iter().any(|c| c == ".git") {
        return Some("cartella .git di un repository");
    }
    if comps.iter().any(|c| CREDENTIAL_FILES.contains(&c.as_str())) {
        return Some("file di credenziali");
    }
    if let Some(last) = comps.last()
        && CREDENTIAL_DIRS.contains(&last.as_str())
    {
        return Some("cartella di credenziali o configurazione di un agente");
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

const USER_CONTENT: &[&str] = &["documents", "desktop", "pictures", "videos", "music", "downloads"];

const CLOUD_ROOTS: &[&str] = &["onedrive*", "dropbox", "google drive"];

const SYSTEM_SUBDIRS: &[&str] = &["system32", "syswow64", "winsxs"];

const CREDENTIAL_FILES: &[&str] = &[".credentials.json", "auth.json", "credentials.toml", "token"];

const CREDENTIAL_DIRS: &[&str] = &[".codex", ".claude", ".ssh", ".gnupg", ".aws", ".azure", ".kube"];

const NOT_DRIVE: &str = "percorso non assoluto o con prefisso non riconosciuto (UNC, volume, relativo)";

/// Componenti normalizzati (minuscolo, senza punti/spazi finali né flussi). Il
/// primo è sempre la lettera di unità (`c:`). `strict` rifiuta i nomi 8.3.
fn normalize(raw: &str, strict: bool) -> Result<Vec<String>, &'static str> {
    let s = raw.replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if s.starts_with('\\') {
        return Err(NOT_DRIVE);
    }
    let mut out: Vec<String> = Vec::new();
    for raw_comp in s.split('\\') {
        if raw_comp.is_empty() || raw_comp == "." {
            continue;
        }
        if out.is_empty() {
            let b = raw_comp.as_bytes();
            if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                out.push(raw_comp.to_lowercase());
                continue;
            }
            return Err(NOT_DRIVE);
        }
        if raw_comp == ".." {
            return Err("il percorso contiene «..» e può uscire dalla sua cartella");
        }
        // `nome:flusso` e `nome::$DATA`: conta solo il nome.
        let name = raw_comp.split(':').next().unwrap_or("");
        let name = name.trim_end_matches(['.', ' ']);
        if name.is_empty() {
            return Err("nome di componente non valido (solo punti, spazi o flusso)");
        }
        if strict && has_short_name_marker(name) {
            return Err("nome breve 8.3: non si può confrontare con certezza");
        }
        out.push(name.to_lowercase());
    }
    if out.is_empty() {
        return Err(NOT_DRIVE);
    }
    Ok(out)
}

/// `~` seguito da una cifra (`PROGRA~1`).
fn has_short_name_marker(name: &str) -> bool {
    let b = name.as_bytes();
    b.windows(2).any(|w| w[0] == b'~' && w[1].is_ascii_digit())
}

fn env_dir(env: &Env, var: &str) -> Option<Vec<String>> {
    let value = env.get(var)?;
    if value.trim().is_empty() {
        return None;
    }
    // Il valore dell'ambiente può essere in forma breve: si accetta com'è.
    normalize(value, false).ok().filter(|d| !d.is_empty())
}

fn whole_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = WHOLE_DIRS.iter().filter_map(|v| env_dir(env, v)).collect();
    if env_dir(env, "windir").is_none() && env_dir(env, "systemroot").is_none() {
        dirs.push(default_windows());
    }
    dirs
}

/// Cartelle con i file personali: documenti, desktop, immagini, download e
/// cloud nel profilo, e le cartelle pubbliche. `onedrive*` vale per ogni
/// cartella OneDrive (anche "OneDrive - Azienda").
fn user_content_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs = Vec::new();
    if let Some(profile) = env_dir(env, "userprofile") {
        for name in USER_CONTENT.iter().chain(CLOUD_ROOTS) {
            let mut d = profile.clone();
            d.push((*name).to_string());
            dirs.push(d);
        }
    }
    let public = env_dir(env, "public").unwrap_or_else(|| vec!["c:".to_string(), "users".to_string(), "public".to_string()]);
    for p in [public, vec!["c:".to_string(), "users".to_string(), "public".to_string()]] {
        for name in USER_CONTENT {
            let mut d = p.clone();
            d.push((*name).to_string());
            dirs.push(d);
        }
        dirs.push(p);
    }
    dirs
}

/// Cartella di Windows: `windir`/`systemroot`, o `C:\Windows` se mancano.
fn windows_dirs(env: &Env) -> Vec<Vec<String>> {
    let dirs: Vec<Vec<String>> = ["windir", "systemroot"]
        .iter()
        .filter_map(|v| env_dir(env, v))
        .collect();
    if dirs.is_empty() {
        vec![default_windows()]
    } else {
        dirs
    }
}

fn default_windows() -> Vec<String> {
    vec!["c:".to_string(), "windows".to_string()]
}

/// `path` (che può avere jolly) coincide con `dir` o con un suo antenato, o è
/// `dir` seguita solo da componenti fatti di jolly.
fn covers_whole(path: &[String], dir: &[String]) -> bool {
    if path.len() <= dir.len() {
        prefix_matches(path, dir)
    } else {
        prefix_matches(&path[..dir.len()], dir) && path[dir.len()..].iter().all(|c| wildcard_only(c))
    }
}

/// I componenti di `path` corrispondono, uno a uno (jolly compresi), ai primi
/// componenti di `dir`.
fn prefix_matches(path: &[String], dir: &[String]) -> bool {
    path.iter().zip(dir).all(|(p, d)| wildcard_match(p, d) || wildcard_match(d, p))
}

/// Componente fatto solo di `*`, `?` e `.` con almeno un jolly (`*`, `*.*`).
fn wildcard_only(c: &str) -> bool {
    c.contains(['*', '?']) && c.chars().all(|ch| matches!(ch, '*' | '?' | '.'))
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

    fn assert_refused(list: &[&str]) {
        for p in list {
            assert!(refused(p), "doveva essere protetto: {p}");
        }
    }

    fn assert_allowed(list: &[&str]) {
        for p in list {
            assert!(!refused(p), "non doveva essere protetto: {p}");
        }
    }

    #[test]
    fn trailing_dots_spaces_and_streams_are_normalised() {
        assert_refused(&[
            r"C:\Users\u.",
            r"C:\Users\u\...",
            "C:\\Users\\u\\ ",
            "C:\\Users\\u\\. ",
            r"C:\Windows.\System32\x",
            r"C:\Windows\System32.\x",
            r"C:\Program Files.",
            "C:\\Program Files \\",
            r"D:\proj\.git.",
            r"D:\proj\.git \objects",
            r"C:\x\auth.json.",
            r"C:\x\auth.json::$DATA",
            r"C:\x\auth.json:stream",
            r"C:\Users\u::$INDEX_ALLOCATION",
            r"C:\x\...\y",
        ]);
        assert_allowed(&[r"C:\Users\u\AppData\Local\Foo.", r"D:\proj\target."]);
    }

    #[test]
    fn short_8_3_names_are_refused() {
        assert_refused(&[
            r"C:\PROGRA~1",
            r"C:\PROGRA~2\Foo",
            r"C:\PROGRA~3",
            r"C:\USERNA~1\x",
            r"C:\Users\USERNA~1",
            r"C:\Users\u\AppData\Local\Foo\LONGFI~1\x",
        ]);
        // la tilde senza cifra è un nome normale
        assert_allowed(&[r"C:\Users\u\AppData\Local\Foo~bar\x", r"D:\a~\b"]);
    }

    #[test]
    fn only_drive_letter_paths_are_accepted() {
        assert_refused(&[
            r"\\server\share\x",
            r"\\?\UNC\server\share\x",
            "//localhost/C$/Windows/System32",
            r"\\localhost\C$\Users\u",
            r"\??\C:\Windows",
            r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\Windows",
            r"\\.\C:\Windows",
            r"\Windows\System32",
            r"\Users\u",
            r"Windows\System32",
            r"foo\bar",
            r"C:foo",
            r"C:Windows\System32",
            "",
        ]);
        assert_allowed(&[
            r"\\?\C:\Users\u\AppData\Local\Foo",
            "C:/Users/u/AppData/Local/Foo",
        ]);
    }

    #[test]
    fn forward_slashes_are_normalised() {
        assert_refused(&["C:/Windows/System32/x", "C:/Users/u", "c:/Program Files"]);
    }

    #[test]
    fn wildcards_that_reach_protected_dirs_are_refused() {
        assert_refused(&[
            r"C:\Users\u\*",
            r"C:\Users\u\*.*",
            r"C:\Users\u\*\*",
            r"C:\Users\*",
            r"C:\*",
            r"C:\*\System32",
            r"C:\Windows\Sys*",
            r"C:\Program Files\*",
            r"C:\Program Files (x86)\*.*",
            r"C:\Users\u\AppData\Roaming\*",
            r"C:\Users\u\AppData\Local\*",
            r"C:\Users\u\AppData\L*",
            r"C:\Users\?",
            r"C:\Users\u\App*",
        ]);
        assert_allowed(&[
            r"C:\Users\u\AppData\Local\Foo\*\Cache",
            r"C:\Users\u\AppData\Local\Google\AndroidStudio*\caches",
            r"C:\Users\u\AppData\Local\JetBrains\*\caches",
            r"C:\Users\u\AppData\Roaming\Foo\*.log",
            r"C:\Users\u\AppData\Local\Foo\*",
            r"D:\proj\*\target",
        ]);
    }

    #[test]
    fn many_wildcards_are_checked_quickly() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        for stars in [12usize, 40] {
            let comp = format!("{}z", "*".repeat(stars));
            assert!(!refused(&format!(r"C:\{comp}\x")));
            assert!(!refused(&format!(r"C:\Program Files (x86)\{comp}")));
        }
        assert!(start.elapsed() < Duration::from_millis(100), "{:?}", start.elapsed());
    }

    #[test]
    fn agent_dot_folders_are_refused_whole_but_not_their_subfolders() {
        assert_refused(&[
            r"C:\Users\u\.codex",
            r"C:\Users\u\.claude",
            r"C:\Users\u\.ssh",
            r"C:\Users\u\.gnupg",
            r"C:\Users\u\.aws",
            r"C:\Users\u\.azure",
            r"C:\Users\u\.kube",
            r"D:\x\.SSH",
            r"C:\Users\u\.codex\auth.json",
            r"C:\Users\u\.ssh\token\x",
        ]);
        assert_allowed(&[
            r"C:\Users\u\.claude\projects",
            r"C:\Users\u\.codex\sessions",
            r"C:\Users\u\.cargo\registry\cache",
        ]);
    }

    #[test]
    fn git_and_credentials_are_checked_on_every_component() {
        assert_refused(&[r"D:\a\.git\b\c", r"D:\a\auth.json\x", r"D:\a\.GIT"]);
    }

    #[test]
    fn empty_environment_assumes_default_windows_dir() {
        let e = Env::default();
        for p in [
            r"C:\Windows\System32\x",
            r"C:\Windows\SysWOW64",
            r"C:\Windows\WinSxS\a",
            r"C:\Windows",
        ] {
            assert!(protected(Path::new(p), &e).is_some(), "{p}");
        }
        assert!(protected(Path::new(r"C:\Windows\Temp"), &e).is_none());
    }

    #[test]
    fn blank_environment_values_are_ignored() {
        let mut e = env();
        e.0.insert("userprofile".into(), "  ".into());
        e.0.insert("windir".into(), String::new());
        e.0.insert("systemroot".into(), String::new());
        assert!(protected(Path::new(r"C:\Windows\System32\x"), &e).is_some());
        assert!(protected(Path::new(r"C:\Users\u\Foo"), &e).is_none());
    }

    #[test]
    fn short_names_in_environment_values_do_not_break_other_paths() {
        let mut e = env();
        e.0.insert("userprofile".into(), r"C:\Users\USERNA~1".into());
        assert!(protected(Path::new(r"D:\proj\target"), &e).is_none());
    }

    #[test]
    fn user_content_folders_are_refused_whole() {
        assert_refused(&[
            r"C:\Users\u\Documents",
            r"C:\Users\u\Desktop",
            r"C:\Users\u\Pictures",
            r"C:\Users\u\Videos",
            r"C:\Users\u\Music",
            r"C:\Users\u\Downloads",
            r"C:\Users\u\OneDrive",
            r"C:\Users\u\OneDrive - Contoso",
            r"C:\Users\u\Dropbox",
            r"C:\Users\u\Google Drive",
            r"c:/users/U/DOCUMENTS",
            r"C:\Users\u\Documents.",
            r"C:\Users\u\Documents\",
            r"\\?\C:\Users\u\Downloads",
            r"C:\Users\u\Doc*",
            r"C:\Users\u\Down?oads",
            r"C:\Users\Public",
            r"C:\Users\Public\Documents",
            r"C:\Users\Public\Downloads",
            r"C:\Users\Public\Desktop",
        ]);
    }

    #[test]
    fn wildcard_files_directly_in_user_content_folders_are_refused() {
        assert_refused(&[
            r"C:\Users\u\Downloads\*.*",
            r"C:\Users\u\Downloads\*",
            r"C:\Users\u\Documents\*",
            r"C:\Users\u\Desktop\*.*",
            r"C:\Users\u\OneDrive\*",
            r"C:\Users\u\OneDrive - Contoso\*.*",
            r"C:\Users\u\Dropbox\*",
            r"C:\Users\Public\*",
            r"C:\Users\Public\Documents\*",
            r"C:\Users\u\Documents\*\*",
        ]);
    }

    #[test]
    fn subfolders_of_user_content_folders_are_allowed() {
        assert_allowed(&[
            r"C:\Users\u\Documents\My Games\X\logs",
            r"C:\Users\u\Downloads\Foo\cache\*",
            r"C:\Users\u\Downloads\Foo",
            r"C:\Users\u\Documents\My Games\X\logs\*.log",
            r"C:\Users\u\OneDrive\Cache",
            r"C:\Users\u\Dropbox\.dropbox.cache",
            r"C:\Users\u\Videos\Captures\tmp\*.tmp",
            r"C:\Users\u\AppData\Local\Foo",
        ]);
    }

    #[test]
    fn public_environment_variable_moves_the_public_folders() {
        let mut e = env();
        e.0.insert("public".into(), r"D:\Pub".into());
        for p in [r"D:\Pub", r"D:\Pub\Documents", r"D:\Pub\Pictures\*", r"C:\Users\Public"] {
            assert!(protected(Path::new(p), &e).is_some(), "{p}");
        }
        assert!(protected(Path::new(r"D:\Pub\Documents\App\cache"), &e).is_none());
    }
}
