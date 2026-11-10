//! Percorsi che una regola esterna non può mai toccare.
//!
//! Contratto per chi chiama: ai **file** e alle **cartelle di partenza** (e al
//! percorso grezzo della regola) si applica [`Guard::protected`]; a ogni
//! **cartella che si toglie essa stessa** (cartelle svuotate, `REMOVESELF`,
//! cancellazione di una cartella) si applica [`Guard::protected_dir_removal`],
//! che comprende `protected`.

use std::path::Path;

use crate::model::{Env, PROTECT_PREFIX, PROTECT_TREE_PREFIX};
use crate::wildcard::wildcard_match;

/// [`Guard::protected`] con un `Guard` costruito per la sola chiamata. Chi
/// controlla molti percorsi costruisce un [`Guard`] una volta.
///
/// Si applica al percorso grezzo della regola (variabili espanse, prima della
/// risoluzione dei jolly) **e** a ogni percorso concreto che ne risulta.
pub fn protected(path: &Path, env: &Env) -> Option<&'static str> {
    Guard::new(env).protected(path)
}

/// [`Guard::protected_dir_removal`] con un `Guard` costruito per la sola
/// chiamata.
pub fn protected_dir_removal(path: &Path, env: &Env) -> Option<&'static str> {
    Guard::new(env).protected_dir_removal(path)
}

/// Le cartelle protette di un ambiente, già normalizzate: si costruisce una
/// volta (per esempio per ogni raccolta) e si interroga per ogni percorso.
#[derive(Debug, Clone)]
pub struct Guard {
    /// Cartelle di sistema e del profilo prese intere.
    whole: Vec<Vec<String>>,
    /// Cartelle dei documenti dell'utente prese intere.
    user_content: Vec<Vec<String>>,
    /// Archivi di chiavi sotto le cartelle dell'ambiente (per i jolly).
    key_stores: Vec<Vec<String>>,
    /// `System32`, `SysWOW64`, `WinSxS`: con il contenuto, anche con i jolly.
    system: Vec<Vec<String>>,
    /// Cache di Windows Installer.
    installer: Vec<Vec<String>>,
    /// Con il contenuto (confronto letterale sui componenti del percorso,
    /// jolly ammessi solo nella cartella protetta): altre cartelle di Windows,
    /// `WindowsApps`, `Windows Defender*`, profilo `Default`.
    subtrees: Vec<Vec<String>>,
    /// Cartelle che si svuotano ma non si tolgono.
    keep_dirs: Vec<Vec<String>>,
    /// Cartelle dell'ambiente con chiave `protect-tree:`
    /// ([`PROTECT_TREE_PREFIX`]): con tutto il contenuto.
    own_trees: Vec<Vec<String>>,
}

impl Guard {
    pub fn new(env: &Env) -> Guard {
        let windows = windows_dirs(env);
        let mut system = Vec::new();
        let mut installer = Vec::new();
        let mut subtrees = Vec::new();
        for win in &windows {
            for sub in SYSTEM_SUBDIRS {
                system.push(joined(win, &[sub]));
            }
            for sub in WINDOWS_SUBTREES {
                subtrees.push(joined(win, &[sub]));
            }
            installer.push(joined(win, &["installer"]));
        }
        for var in ["programfiles", "programfiles(x86)", "programw6432"] {
            if let Some(base) = env_dir_or_default(env, var) {
                subtrees.push(joined(&base, &["windowsapps"]));
                subtrees.push(joined(&base, &["windows defender*"]));
            }
        }
        if let Some(data) = env_dir_or_default(env, "programdata") {
            subtrees.push(joined(&data, &["microsoft", "windows defender", "platform"]));
        }
        let mut whole = whole_dirs(env);
        for users in users_dirs(env) {
            // Gli altri profili presi interi; il profilo `Default` con tutto.
            subtrees.push(joined(&users, &["default"]));
            whole.push(joined(&users, &["*"]));
        }
        subtrees.sort();
        subtrees.dedup();
        Guard {
            whole,
            subtrees,
            user_content: user_content_dirs(env),
            key_stores: key_store_dirs(env),
            system,
            installer,
            keep_dirs: keep_dirs(env, &windows),
            own_trees: own_tree_dirs(env),
        }
    }

    /// `Some(motivo in italiano)` se il percorso è protetto: per i file, per
    /// le cartelle di partenza delle regole e per il percorso grezzo della
    /// regola.
    ///
    /// Il percorso si normalizza prima del confronto, come fa Windows: `/`
    /// come `\`, prefisso `\\?\` tolto, punti e spazi finali tolti da ogni
    /// componente, suffisso di flusso (`:nome`, `::$DATA`) tolto, maiuscole
    /// ignorate. Si accettano solo percorsi assoluti con lettera di unità;
    /// UNC, `\??\`, volumi, percorsi relativi e nomi brevi 8.3 (`PROGRA~1`)
    /// si rifiutano, perché non si possono confrontare con certezza. I valori
    /// dell'ambiente invece si accettano anche in forma breve
    /// (`Env::for_rules` li converte già nella forma lunga).
    ///
    /// I caratteri jolly (`*`, `?`) valgono "qualunque cosa": si rifiuta il
    /// percorso se può coincidere con una cartella protetta o con un suo
    /// antenato, o se è una cartella protetta seguita solo da componenti
    /// jolly (`C:\Program Files\*`, `D:\*`, `E:\*\*`).
    ///
    /// Cosa si protegge:
    /// - prese intere (il contenuto sì): radici dei dischi, cartelle di
    ///   sistema e del profilo (`%WinDir%`, `%ProgramFiles%`,
    ///   `%CommonProgramFiles%`, `%ProgramData%`, `%UserProfile%`,
    ///   `%AppData%`, `%LocalAppData%`, le loro cartelle `Microsoft`,
    ///   `Windows Defender`), le cartelle dei documenti dell'utente (nomi
    ///   inglesi, valori delle cartelle note, cartelle spostate in OneDrive
    ///   anche con i nomi italiani) e le cartelle pubbliche;
    /// - con tutto il contenuto: `System32`, `SysWOW64`, `WinSxS`, la cache
    ///   di Windows Installer (`$PatchCache$` escluso), gli archivi di chiavi
    ///   e credenziali (`Microsoft\Protect`, `Credentials`,
    ///   `SystemCertificates`, `Crypto`, `Vault`, ovunque si trovino), e
    ///   nella radice di ogni disco avvio, ripristino, Cestino, punti di
    ///   ripristino e file di paging; `Fonts`, `servicing`, `Boot`, `INF`,
    ///   `SystemApps` di Windows, `WindowsApps` e `Windows Defender*` nei
    ///   Program Files, `Windows Defender\Platform`, il profilo `Default`;
    /// - con tutto il contenuto: le cartelle delle chiavi `protect-tree:`
    ///   dell'ambiente ([`PROTECT_TREE_PREFIX`], i dati dell'app);
    /// - gli altri profili presi interi (`C:\Users\altro\*`);
    /// - cartelle di soli jolly sotto la radice di un disco, qualunque sia il
    ///   file (`D:\*\*.docx`);
    /// - in ogni componente: `.git`, i file di credenziali, i registri
    ///   dell'utente (`NTUSER.*`, `UsrClass.dat*`) e le cartelle di
    ///   credenziali `.ssh`, `.aws`, `.azure`, `.kube`, `.gnupg`, `.docker`
    ///   con tutto il contenuto; `.codex` e `.claude` non si toccano prese
    ///   intere (le sottocartelle sì, se non sono credenziali).
    ///
    /// La cartella temporanea (`%Temp%`, `%WinDir%\Temp`) qui è permessa,
    /// perché è la cartella di partenza delle regole che la svuotano
    /// (`%Temp%|*.tmp`): la rifiuta [`Guard::protected_dir_removal`].
    ///
    /// **Chi chiama deve applicarla due volte**: al percorso grezzo della
    /// regola (dopo l'espansione delle variabili, prima della risoluzione dei
    /// jolly) e a ogni percorso concreto che ne risulta (ogni cartella di
    /// partenza, ogni cartella visitata, ogni file trovato). Per il contenuto
    /// delle cartelle protette con tutto il contenuto il confronto è
    /// letterale: un jolly prima della cartella (`%AppData%\*\Protect\x`) lo
    /// ferma il controllo sui percorsi concreti.
    pub fn protected(&self, path: &Path) -> Option<&'static str> {
        let comps = match normalize(&path.to_string_lossy(), true) {
            Ok(c) => c,
            Err(reason) => return Some(reason),
        };
        self.check(&comps)
    }

    /// `Some(motivo)` se la cartella `path` non si può togliere essa stessa
    /// (cartelle svuotate, `REMOVESELF`, cancellazione di una cartella):
    /// tutto ciò che rifiuta [`Guard::protected`], più le cartelle che si
    /// svuotano ma restano: le cartelle temporanee (`%Temp%`, `%Tmp%`,
    /// `%WinDir%\Temp`, `%LocalAppData%\Temp`, molti programmi non le
    /// ricreano) e ogni cartella che l'ambiente nomina (`%Cargo_Home%`,
    /// `%NuGet_Packages%`…), con i loro antenati.
    pub fn protected_dir_removal(&self, path: &Path) -> Option<&'static str> {
        let comps = match normalize(&path.to_string_lossy(), true) {
            Ok(c) => c,
            Err(reason) => return Some(reason),
        };
        if let Some(reason) = self.check(&comps) {
            return Some(reason);
        }
        self.keep_dirs
            .iter()
            .any(|dir| comps.len() <= dir.len() && prefix_matches(&comps, dir))
            .then_some("cartella che si svuota ma non si cancella")
    }

    fn check(&self, comps: &[String]) -> Option<&'static str> {
        // Radice di un disco, anche presa con i soli jolly (`D:\*`, `E:\*\*`).
        if comps.len() <= 1 {
            return Some("radice di un disco");
        }
        if comps[1..].iter().all(|c| wildcard_only(c)) {
            return Some("radice di un disco presa per intero con i caratteri jolly");
        }
        // Cartelle prese per intero, i loro antenati e "cartella + solo jolly".
        if self.whole.iter().any(|d| covers_whole(comps, d)) {
            return Some("cartella di sistema o del profilo presa per intero");
        }
        // Cartelle dei documenti dell'utente prese per intero (le sottocartelle sì).
        if self.user_content.iter().any(|d| covers_whole(comps, d)) {
            return Some("cartella dei documenti dell'utente presa per intero");
        }
        // Archivi di chiavi e credenziali di Windows, con il contenuto.
        if in_key_store(comps) || self.key_stores.iter().any(|d| covers_whole(comps, d)) {
            return Some("archivio di chiavi o credenziali di Windows");
        }
        // Avvio, ripristino, Cestino e file di paging nella radice di ogni
        // disco, con il contenuto.
        // Cartelle di soli jolly sotto la radice, qualunque sia il file
        // (`D:\*\*.docx`).
        if comps.len() >= 3 && comps[1..comps.len() - 1].iter().all(|c| wildcard_only(c)) {
            return Some("cartelle di un disco prese con i soli caratteri jolly");
        }
        let first = &comps[1];
        let rest_wild = comps[2..].iter().all(|c| wildcard_only(c));
        if ROOT_ENTRIES
            .iter()
            .any(|name| first == name || (rest_wild && comp_matches(first, name)))
        {
            return Some("file o cartella di sistema nella radice del disco");
        }
        // Cartelle di sistema e il loro contenuto.
        if self
            .system
            .iter()
            .any(|d| comps.len() >= d.len() && prefix_matches(comps, d))
        {
            return Some("cartella di sistema di Windows");
        }
        for dir in &self.installer {
            let patch_cache = comps.get(dir.len()).is_some_and(|c| c == "$patchcache$");
            if covers_whole(comps, dir) || (!patch_cache && covers_subtree(comps, dir)) {
                return Some("cache di Windows Installer");
            }
        }
        if self.subtrees.iter().any(|d| covers_subtree_pattern(comps, d)) {
            return Some("cartella di sistema di Windows o profilo predefinito");
        }
        if self.own_trees.iter().any(|d| covers_subtree(comps, d)) {
            return Some("cartella dei dati di PulisciPC");
        }
        if comps.iter().any(|c| c.starts_with("ntuser.") || c.starts_with("usrclass.dat")) {
            return Some("registro dell'utente (NTUSER.DAT, UsrClass.dat)");
        }
        if comps.iter().any(|c| CREDENTIAL_CONTENT_DIRS.contains(&c.as_str())) {
            return Some("cartella di credenziali");
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
}

/// Variabili le cui cartelle non si toccano prese intere (le sottocartelle sì).
const WHOLE_DIRS: &[&str] = &[
    "windir",
    "systemroot",
    "programfiles",
    "programfiles(x86)",
    "programw6432",
    "commonprogramfiles",
    "commonprogramfiles(x86)",
    "commonprogramw6432",
    "programdata",
    "allusersprofile",
    "userprofile",
    "appdata",
    "localappdata",
    "systemdrive",
];

/// Sottocartelle prese intere: (variabile, percorso relativo).
const WHOLE_SUBDIRS: &[(&str, &[&str])] = &[
    ("appdata", &["microsoft"]),
    ("localappdata", &["microsoft"]),
    ("programdata", &["microsoft"]),
    ("programdata", &["microsoft", "windows defender"]),
    // `%CommonProgramFiles%` anche quando la variabile manca.
    ("programfiles", &["common files"]),
    ("programfiles(x86)", &["common files"]),
    ("programw6432", &["common files"]),
];

/// Cartelle che si usano anche senza la variabile.
const DEFAULT_DIRS: &[(&str, &[&str])] = &[
    ("windir", &["c:", "windows"]),
    ("programfiles", &["c:", "program files"]),
    ("programfiles(x86)", &["c:", "program files (x86)"]),
    ("programdata", &["c:", "programdata"]),
];

/// Cartelle dei documenti nel profilo (nomi su disco, uguali in ogni lingua).
const USER_CONTENT: &[&str] = &["documents", "desktop", "pictures", "videos", "music", "downloads"];

/// Variabili delle cartelle note dei documenti (le aggiunge `Env::for_rules`).
const USER_CONTENT_VARS: &[&str] = &["documents", "desktop", "pictures", "video", "music", "downloads"];

const CLOUD_ROOTS: &[&str] = &["onedrive*", "dropbox", "google drive"];

const CLOUD_VARS: &[&str] = &["onedrive", "onedriveconsumer", "onedrivecommercial"];

/// Cartelle spostate in OneDrive (Known Folder Move): OneDrive le crea con il
/// nome nella lingua di Windows.
const ONEDRIVE_FOLDERS: &[&str] = &[
    "documents",
    "desktop",
    "pictures",
    "videos",
    "music",
    "downloads",
    "documenti",
    "immagini",
    "musica",
    "video",
    "scrivania",
    "download",
];

/// Archivi di chiavi e credenziali: `Microsoft\<nome>`.
const KEY_STORES: &[&str] = &["protect", "credentials", "systemcertificates", "crypto", "vault"];

/// Dove stanno gli archivi di chiavi: (variabile, archivi).
const KEY_STORE_DIRS: &[(&str, &[&str])] = &[
    ("appdata", KEY_STORES),
    ("localappdata", &["vault", "credentials"]),
    ("programdata", &["crypto", "vault"]),
];

/// Nella radice di ogni disco, con il contenuto.
const ROOT_ENTRIES: &[&str] = &[
    "bootmgr",
    "bootnxt",
    "boot",
    "recovery",
    "$recycle.bin",
    "system volume information",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
];

const SYSTEM_SUBDIRS: &[&str] = &["system32", "syswow64", "winsxs"];

/// Altre cartelle di Windows protette con il contenuto.
const WINDOWS_SUBTREES: &[&str] = &["fonts", "servicing", "boot", "inf", "systemapps"];

/// Cartelle di credenziali: né prese intere né il loro contenuto.
const CREDENTIAL_CONTENT_DIRS: &[&str] = &[".ssh", ".aws", ".azure", ".kube", ".gnupg", ".docker"];

const CREDENTIAL_FILES: &[&str] = &[".credentials.json", "auth.json", "credentials.toml", "token"];

const CREDENTIAL_DIRS: &[&str] = &[".codex", ".claude"];

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

/// Come `env_dir`, ma con la cartella predefinita se la variabile manca.
fn env_dir_or_default(env: &Env, var: &str) -> Option<Vec<String>> {
    env_dir(env, var).or_else(|| {
        DEFAULT_DIRS
            .iter()
            .find(|(v, _)| *v == var)
            .map(|(_, d)| d.iter().map(|c| (*c).to_string()).collect())
    })
}

/// `base` seguita dai componenti `rel`.
fn joined(base: &[String], rel: &[&str]) -> Vec<String> {
    let mut d = base.to_vec();
    d.extend(rel.iter().map(|c| (*c).to_string()));
    d
}

fn whole_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = WHOLE_DIRS
        .iter()
        .filter_map(|v| env_dir_or_default(env, v))
        .collect();
    for (var, rel) in WHOLE_SUBDIRS {
        if let Some(base) = env_dir_or_default(env, var) {
            dirs.push(joined(&base, rel));
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Cartelle con i file personali: documenti, desktop, immagini, download e
/// cloud nel profilo, i valori delle cartelle note, le cartelle spostate in
/// OneDrive e le cartelle pubbliche. `onedrive*` vale per ogni cartella
/// OneDrive (anche "OneDrive - Azienda").
fn user_content_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs = Vec::new();
    let mut onedrive_roots: Vec<Vec<String>> = CLOUD_VARS.iter().filter_map(|v| env_dir(env, v)).collect();
    if let Some(profile) = env_dir(env, "userprofile") {
        for name in USER_CONTENT.iter().chain(CLOUD_ROOTS) {
            dirs.push(joined(&profile, &[name]));
        }
        onedrive_roots.push(joined(&profile, &["onedrive*"]));
    }
    for root in onedrive_roots {
        for name in ONEDRIVE_FOLDERS {
            dirs.push(joined(&root, &[name]));
        }
        dirs.push(root);
    }
    dirs.extend(USER_CONTENT_VARS.iter().filter_map(|v| env_dir(env, v)));
    // Cartelle dei documenti spostate fuori dal profilo (`Env::for_rules`).
    dirs.extend(
        env.0
            .iter()
            .filter(|(k, v)| k.starts_with(PROTECT_PREFIX) && !v.trim().is_empty())
            .filter_map(|(_, v)| normalize(v, false).ok())
            .filter(|d| d.len() > 1),
    );
    let default_public = vec!["c:".to_string(), "users".to_string(), "public".to_string()];
    let public = env_dir(env, "public").unwrap_or_else(|| default_public.clone());
    for p in [public, default_public] {
        for name in USER_CONTENT {
            dirs.push(joined(&p, &[name]));
        }
        dirs.push(p);
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Cartelle protette con tutto il contenuto dalle chiavi `protect-tree:`
/// dell'ambiente (percorsi assoluti con lettera di unità; gli altri valori si
/// ignorano).
fn own_tree_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = env
        .0
        .iter()
        .filter(|(k, v)| k.starts_with(PROTECT_TREE_PREFIX) && !v.trim().is_empty())
        .filter_map(|(_, v)| normalize(v, false).ok())
        .filter(|d| d.len() > 1)
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Archivi di chiavi sotto le cartelle dell'ambiente (per i jolly:
/// `%AppData%\Microsoft\Prot*`).
fn key_store_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs = Vec::new();
    for (var, stores) in KEY_STORE_DIRS {
        if let Some(base) = env_dir_or_default(env, var) {
            for store in *stores {
                dirs.push(joined(&base, &["microsoft", store]));
            }
        }
    }
    dirs
}

/// `Microsoft\<archivio>` in un punto qualsiasi del percorso (profilo,
/// ProgramData, altri utenti, profili di servizio): confronto letterale.
fn in_key_store(comps: &[String]) -> bool {
    comps
        .windows(2)
        .any(|w| w[0] == "microsoft" && KEY_STORES.contains(&w[1].as_str()))
}

/// Cartelle che si svuotano ma non si tolgono: le temporanee e ogni cartella
/// che l'ambiente nomina.
fn keep_dirs(env: &Env, windows: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = env
        .0
        .values()
        .filter_map(|v| normalize(v, false).ok())
        .filter(|d| d.len() > 1)
        .collect();
    for win in windows {
        dirs.push(joined(win, &["temp"]));
    }
    if let Some(local) = env_dir(env, "localappdata") {
        dirs.push(joined(&local, &["temp"]));
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Cartella di Windows: `windir`/`systemroot`, o `C:\Windows` se mancano.
fn windows_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = ["windir", "systemroot"]
        .iter()
        .filter_map(|v| env_dir(env, v))
        .collect();
    if dirs.is_empty() {
        dirs.push(vec!["c:".to_string(), "windows".to_string()]);
    }
    dirs.dedup();
    dirs
}

/// `path` (che può avere jolly) coincide con `dir` o con un suo antenato, o è
/// `dir` seguita solo da componenti fatti di jolly.
fn covers_whole(path: &[String], dir: &[String]) -> bool {
    if path.len() <= dir.len() {
        prefix_matches(path, dir)
    } else {
        // Prima la coda (controllo economico: un percorso concreto non ha jolly).
        path[dir.len()..].iter().all(|c| wildcard_only(c)) && prefix_matches(&path[..dir.len()], dir)
    }
}

/// Come `covers_whole`, e in più ogni percorso dentro `dir` (confronto
/// letterale; `dir` può avere jolly, il percorso no).
fn covers_subtree_pattern(path: &[String], dir: &[String]) -> bool {
    covers_whole(path, dir)
        || (path.len() > dir.len()
            && path[..dir.len()]
                .iter()
                .zip(dir)
                .all(|(p, d)| p == d || (has_wildcard(d) && wildcard_match(d, p))))
}

/// Cartelle dei profili (`C:\Users`): quella del profilo e di `Public`, e
/// `C:\Users`.
fn users_dirs(env: &Env) -> Vec<Vec<String>> {
    let mut dirs: Vec<Vec<String>> = ["userprofile", "public"]
        .iter()
        .filter_map(|v| env_dir(env, v))
        .filter(|d| d.len() > 2)
        .map(|d| d[..d.len() - 1].to_vec())
        .collect();
    dirs.push(vec!["c:".to_string(), "users".to_string()]);
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Come `covers_whole`, e in più ogni percorso dentro `dir` (confronto
/// letterale sui componenti di `dir`).
fn covers_subtree(path: &[String], dir: &[String]) -> bool {
    covers_whole(path, dir) || (path.len() > dir.len() && path[..dir.len()] == *dir)
}

/// I componenti di `path` corrispondono, uno a uno (jolly compresi), ai primi
/// componenti di `dir`.
fn prefix_matches(path: &[String], dir: &[String]) -> bool {
    path.iter().zip(dir).all(|(p, d)| comp_matches(p, d))
}

/// Due componenti (già in minuscolo) possono indicare lo stesso nome: uguali,
/// o uno dei due è un modello con jolly che corrisponde all'altro.
fn comp_matches(p: &str, d: &str) -> bool {
    p == d || (has_wildcard(p) && wildcard_match(p, d)) || (has_wildcard(d) && wildcard_match(d, p))
}

fn has_wildcard(c: &str) -> bool {
    c.bytes().any(|b| b == b'*' || b == b'?')
}

/// Componente fatto solo di `*`, `?` e `.` con almeno un jolly (`*`, `*.*`).
fn wildcard_only(c: &str) -> bool {
    has_wildcard(c) && c.bytes().all(|b| matches!(b, b'*' | b'?' | b'.'))
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
            ("commonprogramfiles", r"C:\Program Files\Common Files"),
            ("commonprogramfiles(x86)", r"C:\Program Files (x86)\Common Files"),
            ("temp", r"C:\Users\u\AppData\Local\Temp"),
            ("tmp", r"C:\Users\u\AppData\Local\Temp"),
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
            r"C:\Windows\Temp\*",
            r"C:\Windows\Temp\x.log",
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
        assert!(protected(Path::new(r"C:\Windows\Temp\x.log"), &e).is_none());
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

    #[test]
    fn protect_tree_keys_protect_the_folder_with_its_content() {
        use crate::model::PROTECT_TREE_PREFIX;
        let inside = [
            r"C:\ProgramData\PulisciPC",
            r"C:\ProgramData\PulisciPC\winapp2",
            r"C:\ProgramData\PulisciPC\winapp2\Winapp2.ini",
            r"C:\ProgramData\PulisciPC\winapp2\tmp\*.*",
            r"C:\ProgramData\PulisciPC\*",
            r"c:/programdata/PULISCIPC/Winapp2/etag.txt",
            r"\\?\C:\ProgramData\PulisciPC\winapp2\etag.txt",
            r"C:\Users\u\Documents\PulisciPC\backup_regole_1.reg",
            r"C:\Users\u\Documents\PulisciPC\sub\x.reg",
        ];
        // Senza le chiavi `protect-tree:` il contenuto è permesso (le cartelle
        // madri sono protette solo prese intere).
        for p in [inside[1], inside[2], inside[7]] {
            assert!(protected(Path::new(p), &env()).is_none(), "{p}");
        }
        let mut e = env();
        e.0.insert(format!("{PROTECT_TREE_PREFIX}dati"), r"C:\ProgramData\PulisciPC".into());
        e.0.insert(format!("{PROTECT_TREE_PREFIX}backup"), r"C:\Users\u\Documents\PulisciPC\".into());
        for p in inside {
            assert!(protected(Path::new(p), &e).is_some(), "doveva essere protetto: {p}");
            assert!(protected_dir_removal(Path::new(p), &e).is_some(), "doveva essere protetto: {p}");
        }
        for p in [
            r"C:\ProgramData\PulisciPCx\cache\a.tmp",
            r"C:\ProgramData\Other\cache\a.tmp",
            r"C:\Users\u\Documents\My Games\x.log",
        ] {
            assert!(protected(Path::new(p), &e).is_none(), "non doveva essere protetto: {p}");
        }
        // Un valore vuoto o relativo non protegge niente e non rompe il resto.
        let mut e = env();
        e.0.insert(format!("{PROTECT_TREE_PREFIX}vuota"), "  ".into());
        e.0.insert(format!("{PROTECT_TREE_PREFIX}relativa"), r"PulisciPC".into());
        assert!(protected(Path::new(r"C:\ProgramData\Other\a.tmp"), &e).is_none());
    }

    #[test]
    fn drive_roots_taken_through_wildcards_are_refused() {
        assert_refused(&[
            r"D:\*",
            r"E:\*\*",
            r"D:\*\*\*",
            r"C:\*.*",
            r"Z:\?",
            r"D:\*.*\*",
            "D:/*/",
            r"\\?\F:\*",
        ]);
        let e = Env::default();
        for p in [r"D:\*", r"E:\*\*", r"C:\*.*"] {
            assert!(protected(Path::new(p), &e).is_some(), "{p}");
        }
        assert_allowed(&[r"D:\Games\*\cache", r"D:\Games\*", r"D:\*\cache\*.tmp"]);
    }

    #[test]
    fn key_and_credential_stores_are_refused_with_their_contents() {
        assert_refused(&[
            r"C:\Users\u\AppData\Roaming\Microsoft\Protect",
            r"C:\Users\u\AppData\Roaming\Microsoft\Protect\*",
            r"C:\Users\u\AppData\Roaming\Microsoft\Protect\S-1-5-21-1\abcd",
            r"C:\Users\u\AppData\Roaming\Microsoft\Credentials",
            r"C:\Users\u\AppData\Roaming\Microsoft\Credentials\*.*",
            r"C:\Users\u\AppData\Roaming\Microsoft\SystemCertificates",
            r"C:\Users\u\AppData\Roaming\Microsoft\SystemCertificates\My\Certificates\x",
            r"C:\Users\u\AppData\Roaming\Microsoft\Crypto",
            r"C:\Users\u\AppData\Roaming\Microsoft\Crypto\RSA\S-1-5-21-1\k",
            r"C:\Users\u\AppData\Roaming\Microsoft\Vault",
            r"C:\Users\u\AppData\Local\Microsoft\Vault",
            r"C:\Users\u\AppData\Local\Microsoft\Vault\*",
            r"C:\Users\u\AppData\Local\Microsoft\Credentials",
            r"C:\Users\u\AppData\Local\Microsoft\Credentials\ABCDEF",
            r"C:\ProgramData\Microsoft\Crypto",
            r"C:\ProgramData\Microsoft\Crypto\RSA\MachineKeys",
            r"C:\ProgramData\Microsoft\Crypto\RSA\MachineKeys\*",
            r"C:\ProgramData\Microsoft\Vault",
            r"C:\Users\other\AppData\Roaming\Microsoft\Protect\x",
            r"C:\Users\*\AppData\Roaming\Microsoft\Credentials\*",
            r"C:\Users\u\AppData\Roaming\Microsoft\Prot*",
            r"C:\Users\u\AppData\Roaming\Microsoft\*",
            r"C:\Users\u\AppData\Local\Microsoft\*",
            r"C:\ProgramData\Microsoft\*",
            r"C:\ProgramData\Microsoft\*.*",
            r"C:\ProgramData\Microsoft",
        ]);
        assert_allowed(&[
            r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Recent\*",
            r"C:\Users\u\AppData\Local\Microsoft\Windows\INetCache\*",
            r"C:\Users\u\AppData\Local\Microsoft\*\Cache",
            r"C:\ProgramData\Microsoft\Windows\WER\ReportArchive\*",
        ]);
    }

    #[test]
    fn defender_and_common_files_are_refused_whole() {
        assert_refused(&[
            r"C:\ProgramData\Microsoft\Windows Defender",
            r"C:\ProgramData\Microsoft\Windows Defender\*",
            r"C:\ProgramData\Microsoft\Windows Defender\*.*",
            r"C:\Program Files\Common Files",
            r"C:\Program Files\Common Files\*",
            r"C:\Program Files (x86)\Common Files",
            r"C:\Program Files (x86)\Common Files\*.*",
        ]);
        assert_allowed(&[
            r"C:\ProgramData\Microsoft\Windows Defender\Scans\History\Results\Quick\*",
            r"C:\ProgramData\Microsoft\Windows Defender\Support\*.log",
            r"C:\Program Files\Common Files\Foo\log",
        ]);
        // Senza le variabili si deducono da `ProgramFiles`.
        let mut e = env();
        e.0.remove("commonprogramfiles");
        e.0.remove("commonprogramfiles(x86)");
        for p in [r"C:\Program Files\Common Files", r"C:\Program Files (x86)\Common Files\*"] {
            assert!(protected(Path::new(p), &e).is_some(), "{p}");
        }
    }

    fn removal_refused(p: &str) -> bool {
        protected_dir_removal(&PathBuf::from(p), &env()).is_some()
    }

    /// La cartella temporanea si svuota (`%Temp%|*.tmp` passa a `protected`
    /// proprio `%Temp%`) ma non si cancella essa stessa.
    #[test]
    fn temp_folders_can_be_emptied_but_not_removed() {
        let itself = [
            r"C:\Users\u\AppData\Local\Temp",
            "C:\\Users\\u\\AppData\\Local\\Temp\\",
            r"C:\Users\u\AppData\Local\Te*",
            r"C:\Windows\Temp",
            "C:\\Windows\\Temp\\",
            r"C:\Windows\T?mp",
        ];
        for p in itself {
            assert!(removal_refused(p), "non si doveva poter cancellare: {p}");
        }
        assert_allowed(&itself[..2]);
        assert_allowed(&[r"C:\Windows\Temp"]);
        let contents = [
            r"C:\Users\u\AppData\Local\Temp\*",
            r"C:\Users\u\AppData\Local\Temp\*.*",
            r"C:\Users\u\AppData\Local\Temp\claude",
            r"C:\Users\u\AppData\Local\Temp\*.tmp",
            r"C:\Windows\Temp\*",
            r"C:\Windows\Temp\x.log",
        ];
        assert_allowed(&contents);
        for p in contents {
            assert!(!removal_refused(p), "si doveva poter cancellare: {p}");
        }
        let mut e = env();
        e.0.insert("tmp".into(), r"D:\Tmp".into());
        assert!(protected_dir_removal(Path::new(r"D:\Tmp"), &e).is_some());
        assert!(protected(Path::new(r"D:\Tmp"), &e).is_none());
        assert!(protected_dir_removal(Path::new(r"D:\Tmp\a"), &e).is_none());
        // Senza ambiente vale ancora `C:\Windows\Temp`.
        let e = Env::default();
        assert!(protected_dir_removal(Path::new(r"C:\Windows\Temp"), &e).is_some());
        // Quello che `protected` rifiuta non si cancella nemmeno.
        assert!(removal_refused(r"C:\Users\u\Documents"));
        assert!(removal_refused(r"D:\*"));
    }

    #[test]
    fn boot_recovery_and_system_files_at_a_root_are_refused() {
        assert_refused(&[
            r"C:\bootmgr",
            r"C:\BOOTMGR",
            r"C:\Recovery",
            r"C:\Recovery\WindowsRE\Winre.wim",
            r"C:\Recov*",
            r"C:\Boot",
            r"C:\Boot\BCD",
            r"C:\$Recycle.Bin",
            r"C:\$Recycle.Bin\*",
            r"C:\$Recycle.Bin\S-1-5-21-1\$R1.txt",
            r"D:\$Recycle.Bin",
            r"C:\System Volume Information",
            r"D:\System Volume Information\x",
            r"C:\pagefile.sys",
            r"D:\pagefile.sys",
            r"C:\hiberfil.sys",
            r"C:\swapfile.sys",
            r"C:\*.sys",
        ]);
        assert_allowed(&[
            r"C:\Users\u\AppData\Local\Foo\pagefile.sys",
            r"D:\Games\Boot\cache",
            r"C:\*.log",
        ]);
    }

    #[test]
    fn windows_installer_cache_is_refused_with_its_contents() {
        assert_refused(&[
            r"C:\Windows\Installer",
            r"C:\Windows\Installer\*",
            r"C:\Windows\Installer\1a2b3c.msi",
            r"C:\Windows\Installer\*.msp",
            r"C:\Windows\Inst*",
        ]);
        let e = Env::default();
        assert!(protected(Path::new(r"C:\Windows\Installer\x.msi"), &e).is_some());
    }

    #[test]
    fn known_folder_values_are_refused_whole() {
        let mut e = env();
        for (k, v) in [
            ("documents", r"D:\Docs"),
            ("music", r"D:\Media\Musik"),
            ("pictures", r"C:\Users\u\OneDrive\Immagini"),
            ("video", r"E:\Filme"),
            ("desktop", r"C:\Users\u\OneDrive\Scrivania"),
            ("downloads", r"F:\Scaricati"),
            ("onedrive", r"G:\Cloud"),
        ] {
            e.0.insert(k.into(), v.into());
        }
        for p in [
            r"D:\Docs",
            r"D:\Docs\*",
            r"D:\Media\Musik\*.*",
            r"C:\Users\u\OneDrive\Immagini",
            r"E:\Filme",
            r"C:\Users\u\OneDrive\Scrivania\*",
            r"F:\Scaricati",
            r"G:\Cloud",
            r"G:\Cloud\*",
            r"G:\Cloud\Documenti",
            r"G:\Cloud\Documents\*",
        ] {
            assert!(protected(Path::new(p), &e).is_some(), "doveva essere protetto: {p}");
        }
        for p in [r"D:\Docs\My Games\X\logs", r"F:\Scaricati\Foo\cache", r"G:\Cloud\App\cache"] {
            assert!(protected(Path::new(p), &e).is_none(), "non doveva essere protetto: {p}");
        }
    }

    #[test]
    fn onedrive_known_folder_move_names_are_refused_whole() {
        assert_refused(&[
            r"C:\Users\u\OneDrive\Documenti",
            r"C:\Users\u\OneDrive\Immagini\*",
            r"C:\Users\u\OneDrive\Musica",
            r"C:\Users\u\OneDrive\Video",
            r"C:\Users\u\OneDrive\Scrivania",
            r"C:\Users\u\OneDrive\Desktop\*.*",
            r"C:\Users\u\OneDrive\Download",
            r"C:\Users\u\OneDrive\Documents",
            r"C:\Users\u\OneDrive\Pictures\*",
            r"C:\Users\u\OneDrive - Contoso\Documenti",
            r"C:\Users\u\OneDrive - Contoso\Desktop\*",
        ]);
        assert_allowed(&[
            r"C:\Users\u\OneDrive\Documenti\App\cache",
            r"C:\Users\u\OneDrive - Contoso\Desktop\Foo\tmp",
        ]);
    }

    /// I controlli della revisione, scritti come nelle regole.
    #[test]
    fn review_probes_with_variables() {
        let e = env();
        let check = |rule: &str| -> Vec<bool> {
            let paths = crate::expand::expand(rule, &e);
            assert!(!paths.is_empty(), "variabile non definita in {rule}");
            paths.iter().map(|p| protected(p, &e).is_some()).collect()
        };
        for rule in [
            r"D:\*",
            r"E:\*\*",
            r"%UserProfile%\*",
            r"%ProgramData%\Microsoft\*",
            r"%AppData%\Microsoft\Protect",
            r"%CommonProgramFiles%",
            r"%CommonProgramFiles(x86)%",
            r"C:\bootmgr",
            r"C:\Recovery",
            r"%WinDir%\Installer",
        ] {
            assert!(check(rule).iter().all(|&r| r), "doveva essere protetto: {rule}");
        }
        // La cartella temporanea stessa: non si cancella (il contenuto sì).
        for rule in [r"%Temp%", "%Temp%\\", r"%Tmp%", r"%WinDir%\Temp"] {
            let paths = crate::expand::expand(rule, &e);
            assert!(!paths.is_empty(), "variabile non definita in {rule}");
            for p in paths {
                assert!(protected_dir_removal(&p, &e).is_some(), "doveva essere protetto: {rule}");
            }
        }
        for rule in [
            r"D:\Games\*\cache",
            r"%LocalAppData%\Foo\*\Cache",
            r"%AppData%\Foo\Bar",
            r"%Temp%\claude",
            r"%Temp%\*",
            r"%WinDir%\Temp\*",
        ] {
            assert!(check(rule).iter().all(|&r| !r), "non doveva essere protetto: {rule}");
        }
    }

    const SAMPLE: &[&str] = &[
        r"C:\Windows\System32\x",
        r"C:\Users\u\AppData\Local\Foo\cache\x.tmp",
        r"C:\Users\u\AppData\Local\Temp",
        r"C:\Users\u\AppData\Local\Temp\a\b.txt",
        r"C:\Users\u\Documents",
        r"C:\Users\u\Documents\My Games\x",
        r"D:\*",
        r"D:\Games\*\cache",
        r"C:\Users\u\AppData\Roaming\Microsoft\Protect\x",
        r"C:\Windows\Installer\x.msi",
        r"C:\$Recycle.Bin",
        r"C:\PROGRA~1",
        r"\\server\share",
        r"D:\proj\.git\x",
        r"C:\Users\u\.ssh",
        r"C:\Windows\Temp",
    ];

    #[test]
    fn guard_built_once_answers_like_the_free_functions() {
        let e = env();
        let g = Guard::new(&e);
        for p in SAMPLE {
            let p = Path::new(p);
            assert_eq!(g.protected(p), protected(p, &e), "{}", p.display());
            assert_eq!(
                g.protected_dir_removal(p),
                protected_dir_removal(p, &e),
                "{}",
                p.display()
            );
        }
    }

    #[test]
    fn folders_named_by_the_environment_are_emptied_not_removed() {
        let mut e = env();
        e.0.insert("cargo_home".into(), r"C:\Users\u\.cargo".into());
        e.0.insert("nuget_packages".into(), r"D:\nuget".into());
        let g = Guard::new(&e);
        for p in [r"C:\Users\u\.cargo", r"D:\nuget", r"D:\nuget\", r"D:\nu*"] {
            assert!(g.protected_dir_removal(Path::new(p)).is_some(), "{p}");
            assert!(g.protected(Path::new(p)).is_none(), "{p}");
        }
        for p in [r"D:\nuget\packages\foo", r"C:\Users\u\.cargo\registry\cache", r"D:\other"] {
            assert!(g.protected_dir_removal(Path::new(p)).is_none(), "{p}");
        }
        // ciò che `protected` rifiuta non si toglie
        for p in [
            r"C:\Users\u\Documents",
            r"C:\Windows\System32\x",
            r"\\server\x",
            r"C:\PROGRA~1",
            r"C:\Users\u\AppData\Roaming\Microsoft\Vault\x",
        ] {
            assert!(g.protected_dir_removal(Path::new(p)).is_some(), "{p}");
        }
    }

    /// Tempo per chiamata (`cargo test -p pc-rules --release -- --ignored
    /// guard_speed --nocapture`): l'obiettivo è meno di 5 µs.
    #[test]
    #[ignore]
    fn guard_speed() {
        use std::time::Instant;
        let mut e = env();
        e.0.insert("onedrive".into(), r"C:\Users\u\OneDrive".into());
        e.0.insert("documents".into(), r"C:\Users\u\OneDrive\Documenti".into());
        let g = Guard::new(&e);
        let paths: Vec<PathBuf> = (0..20_000)
            .map(|i| PathBuf::from(format!(r"C:\Users\u\AppData\Local\Foo\cache\sub{}\file{i}.tmp", i % 50)))
            .collect();
        let start = Instant::now();
        let mut refused = 0;
        for p in &paths {
            refused += usize::from(g.protected(p).is_some());
            refused += usize::from(g.protected_dir_removal(p).is_some());
        }
        let per_call = start.elapsed() / (2 * paths.len() as u32);
        println!("Guard: {per_call:?} per chiamata ({refused} rifiuti)");
        let start = Instant::now();
        for p in paths.iter().take(2_000) {
            refused += usize::from(protected(p, &e).is_some());
        }
        println!("protected(path, env): {:?} per chiamata", start.elapsed() / 2_000);
        assert_eq!(refused, 0);
    }

    #[test]
    fn other_profiles_default_profile_and_registry_hives_are_refused() {
        assert_refused(&[
            r"C:\Users\other",
            r"C:\Users\other\*",
            r"C:\Users\other\*.*",
            r"C:\Users\other\*\*",
            r"C:\Users\Default",
            r"C:\Users\Default\*",
            r"C:\Users\Default\AppData\Local\Temp\x.tmp",
            r"C:\Users\u\NTUSER.DAT",
            r"C:\Users\u\ntuser.dat.LOG1",
            r"C:\Users\u\NTUSER.DAT{016888bd-6c6f-11de-8d1d-001e0bcde3ec}.TM.blf",
            r"C:\Users\u\ntuser.ini",
            r"C:\Users\other\NTUSER.DAT",
            r"C:\Users\u\AppData\Local\Microsoft\Windows\UsrClass.dat",
            r"C:\Users\u\AppData\Local\Microsoft\Windows\UsrClass.dat.LOG2",
        ]);
        assert_allowed(&[
            r"C:\Users\other\AppData\Local\Foo\cache",
            r"C:\Users\u\AppData\Local\Microsoft\Windows\INetCache\*",
        ]);
    }

    #[test]
    fn more_windows_folders_are_refused_with_their_contents() {
        assert_refused(&[
            r"C:\Windows\Fonts",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\*",
            r"C:\Windows\servicing",
            r"C:\Windows\servicing\Packages\x.mum",
            r"C:\Windows\INF",
            r"C:\Windows\INF\oem1.inf",
            r"C:\Windows\Boot\EFI\bootmgfw.efi",
            r"C:\Windows\SystemApps",
            r"C:\Windows\SystemApps\Microsoft.Windows.Search_cw5n1h2txyewy\x",
            r"C:\Program Files\WindowsApps",
            r"C:\Program Files\WindowsApps\*",
            r"C:\Program Files\WindowsApps\Pkg_1.0_x64\app.exe",
            r"C:\Program Files\Windows Defender",
            r"C:\Program Files\Windows Defender\MsMpEng.exe",
            r"C:\Program Files\Windows Defender Advanced Threat Protection\x",
            r"C:\Program Files (x86)\Windows Defender\x",
            r"C:\ProgramData\Microsoft\Windows Defender\Platform",
            r"C:\ProgramData\Microsoft\Windows Defender\Platform\*",
            r"C:\ProgramData\Microsoft\Windows Defender\Platform\4.18.1\MsMpEng.exe",
        ]);
        assert_allowed(&[
            r"C:\Windows\Logs\CBS\*.log",
            r"C:\Windows\Prefetch\*.pf",
            r"C:\Windows\Temp\*",
            r"C:\Program Files\Foo\log",
            r"C:\Program Files\*\Logs",
            r"C:\ProgramData\Microsoft\Windows Defender\Scans\History\Results\Quick\*",
        ]);
    }

    #[test]
    fn contents_of_credential_folders_are_refused() {
        assert_refused(&[
            r"C:\Users\u\.ssh\id_rsa",
            r"C:\Users\u\.ssh\*",
            r"C:\Users\u\.aws\config",
            r"C:\Users\u\.azure\msal_token_cache.json",
            r"C:\Users\u\.kube\config",
            r"C:\Users\u\.kube\cache\x",
            r"C:\Users\u\.gnupg\pubring.kbx",
            r"C:\Users\u\.docker",
            r"C:\Users\u\.docker\config.json",
            r"D:\x\.docker\*",
        ]);
        assert_allowed(&[
            r"C:\Users\u\.claude\projects",
            r"C:\Users\u\.codex\sessions",
            r"C:\Users\u\.dockerignore",
            r"D:\proj\.docker-cache\x",
        ]);
    }

    #[test]
    fn wildcard_folders_under_a_drive_root_are_refused_whatever_the_file() {
        assert_refused(&[
            r"D:\*\*.docx",
            r"E:\*\*\*.pdf",
            r"C:\*\*.log",
            r"D:\?\x.txt",
            r"D:\*.*\report.docx",
        ]);
        assert_allowed(&[r"C:\*.log", r"D:\*\cache\*.tmp", r"D:\Games\*\x.log"]);
    }
}
