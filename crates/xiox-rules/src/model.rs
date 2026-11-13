//! Tipi condivisi dai parser e dal raccoglitore.

use std::collections::HashMap;
use std::path::Path;

/// Variabili d'ambiente (chiavi in minuscolo). Nei test si passa una mappa
/// finta: nessuna lettura di `std::env` fuori da `from_process` e
/// `for_rules`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env(pub HashMap<String, String>);

impl Env {
    /// Copia le variabili del processo, con i nomi in minuscolo.
    ///
    /// Aggiunge (se mancano) le variabili che Winapp2 usa ma Windows non
    /// esporta: `LocalAppDataLow`, `Documents` (segue il reindirizzamento a
    /// OneDrive) e `CommonAppData` (= `ProgramData`).
    pub fn from_process() -> Env {
        let mut map: HashMap<String, String> = std::env::vars_os()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_lowercase(),
                    v.to_string_lossy().into_owned(),
                )
            })
            .collect();
        let profile = map.get("userprofile").cloned();
        let under_profile = |rel: &str| profile.as_ref().map(|p| format!("{p}\\{rel}"));
        if !map.contains_key("locallowappdata")
            && let Some(v) = known_folder(KnownFolder::LocalAppDataLow)
                .or_else(|| under_profile(r"AppData\LocalLow"))
        {
            map.insert("locallowappdata".to_string(), v);
        }
        if !map.contains_key("documents")
            && let Some(v) =
                known_folder(KnownFolder::Documents).or_else(|| under_profile("Documents"))
        {
            map.insert("documents".to_string(), v);
        }
        if !map.contains_key("commonappdata")
            && let Some(v) = map.get("programdata").cloned()
        {
            map.insert("commonappdata".to_string(), v);
        }
        Env(map)
    }

    /// L'ambiente con cui si espandono (e si controllano con `protected`) le
    /// regole esterne.
    ///
    /// **Fiducia.** L'app gira sempre da amministratore ma eredita l'ambiente
    /// dell'utente, e un processo non elevato può cambiare `HKCU\Environment`
    /// e `HKCU\...\User Shell Folders`: con `LOCALAPPDATA=C:\Windows` una
    /// regola di `%LocalAppData%` pulirebbe Windows, e le cartelle che
    /// `protected` difende si sposterebbero. Quindi:
    /// - radici di sistema dalle cartelle note che solo gli amministratori
    ///   cambiano (`SHGetKnownFolderPath`; il profilo viene da `ProfileList`
    ///   in HKLM), mai dalle variabili: `userprofile`, `programdata`,
    ///   `programfiles`, `programfiles(x86)`, `programw6432`,
    ///   `commonprogramfiles`, `commonprogramfiles(x86)`, `commonprogramw6432`,
    ///   `windir` e `systemroot`, `system32`, `public`. Si ricavano da queste
    ///   `systemdrive` (l'unità di `windir`), `allusersprofile` e
    ///   `commonappdata` (= `programdata`), `homedrive` e `homepath` (dal
    ///   profilo); `username` viene da `GetUserNameW`;
    /// - cartelle note dell'utente (`appdata`, `localappdata`,
    ///   `locallowappdata`, `onedrive`, `documents`, `desktop`, `downloads`,
    ///   `music`, `pictures`, `video`): si accettano solo se stanno dentro il
    ///   profilo (le cartelle spostate in `%USERPROFILE%\OneDrive` sì) e, per
    ///   AppData, se `protected` le permette; altrimenti la cartella
    ///   predefinita sotto il profilo. Una cartella dei documenti spostata
    ///   fuori (`D:\Docs`) resta protetta presa intera con la chiave
    ///   `protect:<nome>` ([`PROTECT_PREFIX`]), che `expand` non espande: non
    ///   diventa base delle regole;
    /// - `temp` e `tmp`: il valore del processo, in forma lunga, solo se sta
    ///   dentro il profilo e `protected` lo permette (`…\Temp\2` delle sessioni
    ///   remote); altrimenti `<localappdata>\Temp`;
    /// - `OneDriveConsumer`, `OneDriveCommercial`: come le cartelle dei
    ///   documenti;
    /// - variabili degli strumenti ([`TOOL_VARS`]): un solo percorso assoluto
    ///   con lettera di unità, in forma lunga, permesso da `protected`; dentro
    ///   il profilo basta l'ambiente, fuori dal profilo (`CARGO_HOME=D:\cargo`)
    ///   solo se è il valore della variabile di sistema (HKLM `Session
    ///   Manager\Environment`, `%Var%` espanse con le radici fidate). Un valore
    ///   dell'utente fuori dal profilo (`C:\Program Files\Vendor`) si scarta, e
    ///   con lui le regole che lo usano.
    ///
    /// La forma breve 8.3 (`C:\Users\RUNNER~1\…`) si converte con
    /// `GetLongPathNameW`: altrimenti ogni regola di `%Temp%` cadrebbe nel
    /// rifiuto dei nomi 8.3, che resta per i percorsi scritti nelle regole.
    pub fn for_rules() -> Env {
        Env::for_rules_from(&RuleSources {
            known: &known_folder,
            var: &|name| {
                std::env::var_os(name.to_uppercase()).map(|v| v.to_string_lossy().into_owned())
            },
            long: &long_path,
            user: &user_name,
            machine: &machine_var,
        })
    }

    /// `for_rules` con le sorgenti passate da fuori (nei test, finte).
    fn for_rules_from(src: &RuleSources) -> Env {
        // Un solo percorso assoluto con lettera di unità, in forma lunga.
        let clean = |v: String| -> Option<String> {
            let v = v.trim().to_string();
            if !is_drive_path(&v) || v.contains(';') {
                return None;
            }
            if v.contains('~')
                && let Some(l) = (src.long)(&v).filter(|l| is_drive_path(l))
            {
                return Some(l);
            }
            Some(v)
        };
        // Radici di sistema: cartelle note che solo gli amministratori cambiano.
        let mut map: HashMap<String, String> = HashMap::new();
        for (key, folder) in CORE_FOLDERS {
            if let Some(v) = (src.known)(*folder).and_then(clean) {
                map.insert((*key).to_string(), v);
            }
        }
        if let Some(win) = map.get("windir").cloned() {
            map.insert("systemdrive".to_string(), win[..2].to_string());
            map.insert("systemroot".to_string(), win);
        }
        if let Some(data) = map.get("programdata").cloned() {
            map.insert("allusersprofile".to_string(), data.clone());
            map.insert("commonappdata".to_string(), data);
        }
        if let Some(user) = (src.user)().filter(|u| !u.trim().is_empty()) {
            map.insert("username".to_string(), user);
        }
        let profile_value = map.get("userprofile").cloned();
        let profile = profile_value.as_deref().map(path_comps);
        let inside_profile = |v: &str| {
            let c = path_comps(v);
            profile.as_ref().is_some_and(|p| c.len() > p.len() && c.starts_with(p))
        };
        // Cartelle dell'utente (HKCU, le cambia anche un processo non
        // elevato): valgono solo dentro il profilo, altrimenti la cartella
        // predefinita. Le cartelle dei documenti spostate fuori restano
        // protette (`protect:<nome>`), senza diventare basi delle regole.
        let base_guard = crate::guard::Guard::new(&Env(map.clone()));
        for (key, folder, documents) in USER_FOLDERS {
            let Some(v) = (src.known)(*folder).and_then(clean) else {
                continue;
            };
            if inside_profile(&v) && (*documents || base_guard.protected(Path::new(&v)).is_none()) {
                map.insert((*key).to_string(), v);
            } else if *documents {
                map.insert(format!("{PROTECT_PREFIX}{key}"), v);
            }
        }
        if let Some(profile) = &profile_value {
            for (key, rel) in UNDER_PROFILE {
                map.entry((*key).to_string())
                    .or_insert_with(|| format!("{profile}\\{rel}"));
            }
            map.insert("homedrive".to_string(), profile[..2].to_string());
            map.insert("homepath".to_string(), profile[2..].to_string());
        }

        let trusted = Env(map.clone());
        let guard = crate::guard::Guard::new(&trusted);
        let allowed = |v: &str| guard.protected(Path::new(v)).is_none();
        let default_temp = map.get("localappdata").map(|l| format!("{l}\\Temp"));
        for key in ["temp", "tmp"] {
            let own = (src.var)(key)
                .and_then(clean)
                .filter(|v| inside_profile(v) && allowed(v));
            if let Some(v) = own.or_else(|| default_temp.clone()) {
                map.insert(key.to_string(), v);
            }
        }
        // OneDrive personale e di lavoro: come le cartelle dei documenti.
        for key in CLOUD_VARS {
            if let Some(v) = (src.var)(key).and_then(clean) {
                if inside_profile(&v) {
                    map.insert((*key).to_string(), v);
                } else {
                    map.insert(format!("{PROTECT_PREFIX}{key}"), v);
                }
            }
        }
        // Strumenti: dentro il profilo basta l'ambiente; fuori, solo se il
        // valore è quello delle variabili di sistema.
        let machine = |key: &str| -> Option<String> {
            let raw = (src.machine)(key)?;
            let mut values = crate::expand::expand(&raw, &trusted);
            if values.len() != 1 {
                return None;
            }
            clean(values.pop()?.to_string_lossy().into_owned())
        };
        for key in TOOL_VARS {
            let Some(v) = (src.var)(key).and_then(clean) else {
                continue;
            };
            let scoped = inside_profile(&v)
                || machine(key).is_some_and(|m| path_comps(&m) == path_comps(&v));
            if scoped && allowed(&v) {
                map.insert((*key).to_string(), v);
            }
        }
        Env(map)
    }

    /// Valore di una variabile; il nome si confronta senza maiuscole.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_lowercase()).map(String::as_str)
    }
}

/// Variabili che `Env::for_rules` prende dall'ambiente del processo (con i
/// controlli descritti lì): le cartelle degli strumenti di sviluppo che i
/// cleaner usano (catalogo, Parte B).
pub const TOOL_VARS: &[&str] = &[
    "android_avd_home",
    "android_home",
    "android_sdk_root",
    "android_user_home",
    "bun_install_cache_dir",
    "cargo_home",
    "ccache_dir",
    "composer_cache_dir",
    "conan_home",
    "conda_pkgs_dirs",
    "coursier_cache",
    "cuda_cache_path",
    "cypress_cache_folder",
    "deno_dir",
    "dotnet_bundle_extract_base_dir",
    "electron_cache",
    "gocache",
    "gomodcache",
    "gopath",
    "gradle_user_home",
    "helm_cache_home",
    "hf_home",
    "hf_hub_cache",
    "konan_data_dir",
    "minikube_home",
    "npm_config_cache",
    "nuget_http_cache_path",
    "nuget_packages",
    "ollama_models",
    "pip_cache_dir",
    "playwright_browsers_path",
    "pre_commit_home",
    "pub_cache",
    "puppeteer_cache_dir",
    "rustup_home",
    "sccache_dir",
    "scoop",
    "tf_plugin_cache_dir",
    "torch_home",
    "uv_cache_dir",
    "vagrant_home",
    "vcpkg_default_binary_cache",
    "vcpkg_root",
    "workon_home",
    "zig_global_cache_dir",
];

/// Sorgenti di `Env::for_rules`: cartelle note, variabili del processo,
/// conversione in forma lunga, nome dell'utente.
struct RuleSources<'a> {
    known: &'a dyn Fn(KnownFolder) -> Option<String>,
    var: &'a dyn Fn(&str) -> Option<String>,
    long: &'a dyn Fn(&str) -> Option<String>,
    user: &'a dyn Fn() -> Option<String>,
    /// Variabili di sistema (HKLM, scrivibili solo dagli amministratori),
    /// col valore com'è nel registro (`%Var%` non espanse).
    machine: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KnownFolder {
    Profile,
    RoamingAppData,
    LocalAppData,
    LocalAppDataLow,
    ProgramData,
    ProgramFiles,
    ProgramFilesX86,
    ProgramFilesX64,
    ProgramFilesCommon,
    ProgramFilesCommonX86,
    ProgramFilesCommonX64,
    Windows,
    System,
    Public,
    OneDrive,
    Documents,
    Desktop,
    Downloads,
    Music,
    Pictures,
    Videos,
}

/// Variabili che `for_rules` prende dalle cartelle note di sistema (registro
/// HKLM o cartelle fisse: solo gli amministratori le cambiano).
const CORE_FOLDERS: &[(&str, KnownFolder)] = &[
    ("userprofile", KnownFolder::Profile),
    ("programdata", KnownFolder::ProgramData),
    ("programfiles", KnownFolder::ProgramFiles),
    ("programfiles(x86)", KnownFolder::ProgramFilesX86),
    ("programw6432", KnownFolder::ProgramFilesX64),
    ("commonprogramfiles", KnownFolder::ProgramFilesCommon),
    ("commonprogramfiles(x86)", KnownFolder::ProgramFilesCommonX86),
    ("commonprogramw6432", KnownFolder::ProgramFilesCommonX64),
    ("windir", KnownFolder::Windows),
    ("system32", KnownFolder::System),
    ("public", KnownFolder::Public),
];

/// Cartelle note dell'utente (`HKCU\...\User Shell Folders`, le cambia anche
/// un processo non elevato): (variabile, cartella, è una cartella dei
/// documenti).
const USER_FOLDERS: &[(&str, KnownFolder, bool)] = &[
    ("appdata", KnownFolder::RoamingAppData, false),
    ("localappdata", KnownFolder::LocalAppData, false),
    ("locallowappdata", KnownFolder::LocalAppDataLow, false),
    ("onedrive", KnownFolder::OneDrive, true),
    ("documents", KnownFolder::Documents, true),
    ("desktop", KnownFolder::Desktop, true),
    ("downloads", KnownFolder::Downloads, true),
    ("music", KnownFolder::Music, true),
    ("pictures", KnownFolder::Pictures, true),
    ("video", KnownFolder::Videos, true),
];

/// Cartelle OneDrive dall'ambiente (`OneDriveConsumer`, `OneDriveCommercial`).
const CLOUD_VARS: &[&str] = &["onedriveconsumer", "onedrivecommercial"];

/// Prefisso delle chiavi che solo la protezione legge: cartelle dei documenti
/// spostate fuori dal profilo. `expand` non le espande (il nome contiene
/// `:`), quindi non diventano basi delle regole.
pub const PROTECT_PREFIX: &str = "protect:";

/// Prefisso delle chiavi che solo la protezione legge: cartelle protette
/// **con tutto il contenuto** (a differenza di [`PROTECT_PREFIX`], che le
/// protegge solo prese intere). Le aggiunge chi usa le regole per le proprie
/// cartelle (l'app: `%ProgramData%\PulisciPC`, i backup del registro).
/// Come [`PROTECT_PREFIX`], `expand` non le espande.
pub const PROTECT_TREE_PREFIX: &str = "protect-tree:";

/// Cartelle del profilo usate quando Windows non le dà: (variabile, posto
/// sotto `%USERPROFILE%`).
const UNDER_PROFILE: &[(&str, &str)] = &[
    ("appdata", r"AppData\Roaming"),
    ("localappdata", r"AppData\Local"),
    ("locallowappdata", r"AppData\LocalLow"),
    ("documents", "Documents"),
    ("desktop", "Desktop"),
    ("downloads", "Downloads"),
    ("music", "Music"),
    ("pictures", "Pictures"),
    ("video", "Videos"),
];

/// `X:\…`: percorso assoluto con lettera di unità.
fn is_drive_path(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/')
}

/// Componenti in minuscolo, per sapere se un percorso sta dentro un altro.
fn path_comps(p: &str) -> Vec<String> {
    p.replace('/', "\\")
        .split('\\')
        .map(|c| c.trim_end_matches(['.', ' ']).to_lowercase())
        .filter(|c| !c.is_empty())
        .collect()
}

/// Percorso di una cartella nota di Windows (`SHGetKnownFolderPath`).
#[cfg(windows)]
pub(crate) fn known_folder(which: KnownFolder) -> Option<String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData,
        FOLDERID_LocalAppDataLow, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile,
        FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesCommon,
        FOLDERID_ProgramFilesCommonX64, FOLDERID_ProgramFilesCommonX86, FOLDERID_ProgramFilesX64,
        FOLDERID_ProgramFilesX86, FOLDERID_Public, FOLDERID_RoamingAppData, FOLDERID_SkyDrive,
        FOLDERID_System, FOLDERID_Videos, FOLDERID_Windows, SHGetKnownFolderPath,
    };
    let id = match which {
        KnownFolder::Profile => &FOLDERID_Profile,
        KnownFolder::RoamingAppData => &FOLDERID_RoamingAppData,
        KnownFolder::LocalAppData => &FOLDERID_LocalAppData,
        KnownFolder::LocalAppDataLow => &FOLDERID_LocalAppDataLow,
        KnownFolder::ProgramData => &FOLDERID_ProgramData,
        KnownFolder::ProgramFiles => &FOLDERID_ProgramFiles,
        KnownFolder::ProgramFilesX86 => &FOLDERID_ProgramFilesX86,
        KnownFolder::ProgramFilesX64 => &FOLDERID_ProgramFilesX64,
        KnownFolder::ProgramFilesCommon => &FOLDERID_ProgramFilesCommon,
        KnownFolder::ProgramFilesCommonX86 => &FOLDERID_ProgramFilesCommonX86,
        KnownFolder::ProgramFilesCommonX64 => &FOLDERID_ProgramFilesCommonX64,
        KnownFolder::Windows => &FOLDERID_Windows,
        KnownFolder::System => &FOLDERID_System,
        KnownFolder::Public => &FOLDERID_Public,
        KnownFolder::OneDrive => &FOLDERID_SkyDrive,
        KnownFolder::Documents => &FOLDERID_Documents,
        KnownFolder::Desktop => &FOLDERID_Desktop,
        KnownFolder::Downloads => &FOLDERID_Downloads,
        KnownFolder::Music => &FOLDERID_Music,
        KnownFolder::Pictures => &FOLDERID_Pictures,
        KnownFolder::Videos => &FOLDERID_Videos,
    };
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: `raw` è un puntatore di uscita valido; su successo contiene una
    // stringa UTF-16 terminata da zero, allocata da COM, che si libera con
    // `CoTaskMemFree` dopo la copia.
    unsafe {
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw);
        if hr < 0 || raw.is_null() {
            if !raw.is_null() {
                CoTaskMemFree(raw as *const _);
            }
            return None;
        }
        let mut len = 0;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
        CoTaskMemFree(raw as *const _);
        Some(text).filter(|t| !t.is_empty())
    }
}

#[cfg(not(windows))]
pub(crate) fn known_folder(_which: KnownFolder) -> Option<String> {
    None
}

/// Forma lunga di un percorso esistente (`GetLongPathNameW`); `None` se il
/// percorso non esiste o la chiamata fallisce.
#[cfg(windows)]
fn long_path(path: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut buf: Vec<u16> = vec![0; 512];
    for _ in 0..3 {
        let cap = u32::try_from(buf.len()).ok()?;
        // SAFETY: `wide` è terminata da zero; `buf` ha `cap` elementi
        // scrivibili. Il risultato è la lunghezza scritta (senza lo zero) o,
        // se `buf` è troppo corto, quella necessaria (con lo zero).
        let n = unsafe { GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), cap) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            buf.truncate(n);
            return Some(String::from_utf16_lossy(&buf));
        }
        buf.resize(n, 0);
    }
    None
}

#[cfg(not(windows))]
fn long_path(_path: &str) -> Option<String> {
    None
}

/// Nome dell'utente del processo (`GetUserNameW`).
#[cfg(windows)]
fn user_name() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
    // UNLEN (256) più lo zero finale.
    let mut buf: Vec<u16> = vec![0; 257];
    let mut len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` ha `len` elementi scrivibili; su successo `len` è il
    // numero di caratteri scritti, zero finale compreso.
    let ok = unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return None;
    }
    buf.truncate((len as usize).saturating_sub(1));
    Some(String::from_utf16_lossy(&buf)).filter(|u| !u.is_empty())
}

#[cfg(not(windows))]
fn user_name() -> Option<String> {
    None
}

/// Valore di una variabile di sistema
/// (`HKLM\System\CurrentControlSet\Control\Session Manager\Environment`),
/// com'è nel registro.
#[cfg(windows)]
fn machine_var(name: &str) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"System\CurrentControlSet\Control\Session Manager\Environment",
            KEY_READ,
        )
        .ok()?;
    key.get_value::<String, _>(name).ok()
}

#[cfg(not(windows))]
fn machine_var(_name: &str) -> Option<String> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Winapp2,
    CleanerMl(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaner {
    pub id: String,
    pub label: String,
    pub group: String,
    pub origin: Origin,
    pub description: String,
    pub running: Vec<String>,
    pub options: Vec<Opt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt {
    pub id: String,
    pub label: String,
    pub description: String,
    pub warning: Option<String>,
    pub default_on: bool,
    pub actions: Vec<Action>,
    /// File che la raccolta deve lasciare (Winapp2 `ExcludeKey`).
    pub excludes: Vec<ExcludeRule>,
    /// Motivo per cui l'opzione compare disattivata ("non supportato").
    pub unsupported: Option<String>,
}

/// Esclusione dalla raccolta: `pattern: None` esclude il percorso (file o
/// cartella) e tutto ciò che contiene; con un modello (`;` ne separa più
/// d'uno) esclude, in quella cartella e sotto, i file con quei nomi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludeRule {
    pub path: String,
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Files {
        path: String,
        patterns: Vec<String>,
        recurse: bool,
        remove_self: bool,
        search: Search,
    },
    RegKey {
        root: RegRoot,
        path: String,
    },
    RegValue {
        root: RegRoot,
        path: String,
        value: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Search {
    File,
    Glob,
    WalkAll,
    WalkFiles,
    WalkTop,
    Winapp2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegRoot {
    Hkcu,
    Hklm,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cartelle note finte di un utente `u` (`profile` cambia il profilo).
    fn known_for(profile: &'static str) -> impl Fn(KnownFolder) -> Option<String> {
        move |f| {
            let local = format!(r"{profile}\AppData\Local");
            Some(match f {
                KnownFolder::Profile => profile.to_string(),
                KnownFolder::RoamingAppData => format!(r"{profile}\AppData\Roaming"),
                KnownFolder::LocalAppData => local,
                KnownFolder::LocalAppDataLow => format!(r"{profile}\AppData\LocalLow"),
                KnownFolder::ProgramData => r"C:\ProgramData".to_string(),
                KnownFolder::ProgramFiles | KnownFolder::ProgramFilesX64 => {
                    r"C:\Program Files".to_string()
                }
                KnownFolder::ProgramFilesX86 => r"C:\Program Files (x86)".to_string(),
                KnownFolder::ProgramFilesCommon | KnownFolder::ProgramFilesCommonX64 => {
                    r"C:\Program Files\Common Files".to_string()
                }
                KnownFolder::ProgramFilesCommonX86 => {
                    r"C:\Program Files (x86)\Common Files".to_string()
                }
                KnownFolder::Windows => r"C:\Windows".to_string(),
                KnownFolder::System => r"C:\Windows\System32".to_string(),
                KnownFolder::Public => r"C:\Users\Public".to_string(),
                KnownFolder::OneDrive => format!(r"{profile}\OneDrive"),
                KnownFolder::Documents => format!(r"{profile}\OneDrive\Documenti"),
                KnownFolder::Downloads => r"D:\Scaricati".to_string(),
                KnownFolder::Desktop | KnownFolder::Music | KnownFolder::Pictures => return None,
                KnownFolder::Videos => return None,
            })
        }
    }

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_string())
        }
    }

    fn no_long(_: &str) -> Option<String> {
        None
    }

    /// Finto `GetLongPathNameW`: `RUNNER~1` diventa `runneradmin`.
    fn fake_long(p: &str) -> Option<String> {
        p.contains("RUNNER~1").then(|| p.replace("RUNNER~1", "runneradmin"))
    }

    fn user() -> Option<String> {
        Some("u".to_string())
    }

    fn build(
        known: &dyn Fn(KnownFolder) -> Option<String>,
        var: &dyn Fn(&str) -> Option<String>,
        long: &dyn Fn(&str) -> Option<String>,
    ) -> Env {
        build_m(known, var, long, &|_| None)
    }

    /// Con le variabili di sistema (HKLM) finte.
    fn build_m(
        known: &dyn Fn(KnownFolder) -> Option<String>,
        var: &dyn Fn(&str) -> Option<String>,
        long: &dyn Fn(&str) -> Option<String>,
        machine: &dyn Fn(&str) -> Option<String>,
    ) -> Env {
        Env::for_rules_from(&RuleSources {
            known,
            var,
            long,
            user: &user,
            machine,
        })
    }

    /// Un ambiente ostile: radici spostate su Windows e sul disco.
    const HOSTILE: &[(&str, &str)] = &[
        ("USERPROFILE", r"C:\Windows"),
        ("APPDATA", r"C:\Windows\System32"),
        ("LOCALAPPDATA", r"C:\"),
        ("PROGRAMDATA", r"D:\"),
        ("ProgramFiles", r"C:\Users\u"),
        ("WINDIR", r"D:\Fake"),
        ("SystemRoot", r"D:\Fake"),
        ("SystemDrive", "D:"),
        ("PUBLIC", r"C:\"),
        ("USERNAME", "attacker"),
        ("HOMEDRIVE", "Z:"),
        ("PATH", r"C:\Windows\System32"),
        ("ComSpec", r"C:\Windows\System32\cmd.exe"),
        ("Documents", r"C:\Windows"),
        ("CARGO_HOME", r"D:\cargo"),
    ];

    #[test]
    fn core_roots_come_from_known_folders_not_from_the_environment() {
        let env = build(&known_for(r"C:\Users\u"), &vars(HOSTILE), &no_long);
        for (k, v) in [
            ("userprofile", r"C:\Users\u"),
            ("appdata", r"C:\Users\u\AppData\Roaming"),
            ("localappdata", r"C:\Users\u\AppData\Local"),
            ("locallowappdata", r"C:\Users\u\AppData\LocalLow"),
            ("programdata", r"C:\ProgramData"),
            ("allusersprofile", r"C:\ProgramData"),
            ("commonappdata", r"C:\ProgramData"),
            ("programfiles", r"C:\Program Files"),
            ("programfiles(x86)", r"C:\Program Files (x86)"),
            ("programw6432", r"C:\Program Files"),
            ("commonprogramfiles", r"C:\Program Files\Common Files"),
            ("commonprogramfiles(x86)", r"C:\Program Files (x86)\Common Files"),
            ("commonprogramw6432", r"C:\Program Files\Common Files"),
            ("windir", r"C:\Windows"),
            ("systemroot", r"C:\Windows"),
            ("system32", r"C:\Windows\System32"),
            ("systemdrive", "C:"),
            ("public", r"C:\Users\Public"),
            ("onedrive", r"C:\Users\u\OneDrive"),
            ("homedrive", "C:"),
            ("homepath", r"\Users\u"),
            ("username", "u"),
            ("documents", r"C:\Users\u\OneDrive\Documenti"),
            ("downloads", r"C:\Users\u\Downloads"),
            ("protect:downloads", r"D:\Scaricati"),
            ("temp", r"C:\Users\u\AppData\Local\Temp"),
            ("tmp", r"C:\Users\u\AppData\Local\Temp"),
        ] {
            assert_eq!(env.get(k), Some(v), "{k}");
        }
        for k in ["path", "comspec", "cargo_home"] {
            assert_eq!(env.get(k), None, "{k}");
        }
    }

    #[test]
    fn missing_known_documents_folders_go_under_the_profile() {
        let env = build(&known_for(r"C:\Users\u"), &vars(&[]), &no_long);
        assert_eq!(env.get("desktop"), Some(r"C:\Users\u\Desktop"));
        assert_eq!(env.get("music"), Some(r"C:\Users\u\Music"));
        assert_eq!(env.get("pictures"), Some(r"C:\Users\u\Pictures"));
        assert_eq!(env.get("video"), Some(r"C:\Users\u\Videos"));
        // senza cartelle note non si inventa nulla, nemmeno dall'ambiente
        let env = build(&|_| None, &vars(HOSTILE), &no_long);
        for k in ["userprofile", "windir", "music", "temp", "systemdrive", "programdata", "cargo_home"] {
            assert_eq!(env.get(k), None, "{k}");
        }
    }

    #[test]
    fn temp_from_the_process_only_inside_the_profile() {
        let local_temp = Some(r"C:\Users\u\AppData\Local\Temp");
        for (temp, expected) in [
            // sessione remota: si tiene
            (r"C:\Users\u\AppData\Local\Temp\2", Some(r"C:\Users\u\AppData\Local\Temp\2")),
            (r"C:\Users\u\AppData\Local\Temp", local_temp),
            // fuori dal profilo, o cartella protetta, o non un percorso
            (r"D:\Temp", local_temp),
            (r"C:\Windows\Temp", local_temp),
            (r"C:\Users\u", local_temp),
            (r"C:\Users\u\Documents", local_temp),
            (r"C:\Users\u\AppData\Local", local_temp),
            (r"C:\Users\uu\Temp", local_temp),
            (r"C:\Users\u\..\x\Temp", local_temp),
            ("Temp", local_temp),
            (r"\\server\share\Temp", local_temp),
        ] {
            let pairs: &'static [(&'static str, &'static str)] =
                Box::leak(vec![("TEMP", temp), ("TMP", temp)].into_boxed_slice());
            let env = build(&known_for(r"C:\Users\u"), &vars(pairs), &no_long);
            assert_eq!(env.get("temp"), expected, "TEMP={temp}");
            assert_eq!(env.get("tmp"), expected, "TMP={temp}");
        }
    }

    #[test]
    fn tool_variables_are_checked() {
        let env = build(
            &known_for(r"C:\Users\u"),
            &vars(&[
                ("CARGO_HOME", r"D:\cargo"),
                ("RUSTUP_HOME", r"C:\Users\u\.rustup"),
                ("GRADLE_USER_HOME", r"gradle"),
                ("NPM_CONFIG_CACHE", r"C:\Windows"),
                ("PIP_CACHE_DIR", r"C:\Users\u"),
                ("HF_HOME", r"\\server\share\hf"),
                ("TORCH_HOME", r"C:\Windows\System32\torch"),
                ("GOPATH", r"D:\"),
                ("CONDA_PKGS_DIRS", r"C:\a;D:\b"),
                ("UV_CACHE_DIR", r"C:\Users\u\AppData\Local"),
                ("OLLAMA_MODELS", r"C:\Users\u\Documents"),
                ("ONEDRIVECOMMERCIAL", r"E:\OneDrive - Contoso"),
                ("ONEDRIVECONSUMER", r"C:\Users\u\OneDrive"),
            ]),
            &no_long,
        );
        assert_eq!(env.get("rustup_home"), Some(r"C:\Users\u\.rustup"));
        assert_eq!(env.get("onedriveconsumer"), Some(r"C:\Users\u\OneDrive"));
        // OneDrive di lavoro su un altro disco: protetta, ma non base delle regole
        assert_eq!(env.get("onedrivecommercial"), None);
        assert_eq!(env.get("protect:onedrivecommercial"), Some(r"E:\OneDrive - Contoso"));
        for k in [
            "cargo_home",
            "gradle_user_home",
            "npm_config_cache",
            "pip_cache_dir",
            "hf_home",
            "torch_home",
            "gopath",
            "conda_pkgs_dirs",
            "uv_cache_dir",
            "ollama_models",
        ] {
            assert_eq!(env.get(k), None, "{k}");
        }
    }

    /// Cartelle note dell'utente: valgono solo dentro il profile, altrimenti
    /// la cartella predefinita (e il valore spostato resta protetto).
    #[test]
    fn user_known_folders_outside_the_profile_fall_back_to_defaults() {
        let known = |f: KnownFolder| -> Option<String> {
            match f {
                KnownFolder::RoamingAppData => Some(r"C:\Program Files\Vendor".into()),
                KnownFolder::LocalAppData => Some(r"D:\Local".into()),
                KnownFolder::LocalAppDataLow => Some(r"C:\Users\u\AppData\LocalLow".into()),
                KnownFolder::Documents => Some(r"D:\Docs".into()),
                KnownFolder::Desktop => Some(r"C:\Users\u\OneDrive\Desktop".into()),
                KnownFolder::Pictures => Some(r"C:\Users\u\OneDrive\Immagini".into()),
                KnownFolder::Music => Some(r"C:\Users\other\Music".into()),
                KnownFolder::Videos => Some(r"C:\Users\u".into()),
                KnownFolder::OneDrive => Some(r"E:\OneDrive".into()),
                _ => known_for(r"C:\Users\u")(f),
            }
        };
        let env = build(&known, &vars(&[]), &no_long);
        for (k, v) in [
            ("appdata", Some(r"C:\Users\u\AppData\Roaming")),
            ("localappdata", Some(r"C:\Users\u\AppData\Local")),
            ("locallowappdata", Some(r"C:\Users\u\AppData\LocalLow")),
            ("documents", Some(r"C:\Users\u\Documents")),
            ("desktop", Some(r"C:\Users\u\OneDrive\Desktop")),
            ("pictures", Some(r"C:\Users\u\OneDrive\Immagini")),
            ("music", Some(r"C:\Users\u\Music")),
            ("video", Some(r"C:\Users\u\Videos")),
            ("onedrive", None),
            ("temp", Some(r"C:\Users\u\AppData\Local\Temp")),
            ("protect:documents", Some(r"D:\Docs")),
            ("protect:music", Some(r"C:\Users\other\Music")),
            ("protect:onedrive", Some(r"E:\OneDrive")),
        ] {
            assert_eq!(env.get(k), v, "{k}");
        }
        // AppData e LocalAppData spostate non sono cartelle dei documenti
        assert_eq!(env.get("protect:appdata"), None);
        // il valore spostato resta protetto preso intero, ma non è una base
        for p in [r"D:\Docs", r"D:\Docs\*", r"E:\OneDrive\*", r"C:\Users\other\Music"] {
            assert!(crate::guard::protected(Path::new(p), &env).is_some(), "{p}");
        }
        assert!(crate::expand::expand(r"%Protect:Documents%\x", &env).is_empty());
    }

    #[test]
    fn tool_variables_outside_the_profile_need_machine_scope() {
        let machine = |name: &str| -> Option<String> {
            match name {
                "cargo_home" => Some(r"D:\cargo".into()),
                "gradle_user_home" => Some(r"%SystemDrive%\gradle".into()),
                "nuget_packages" => Some(r"D:\nuget".into()),
                "android_home" => Some(r"C:\Program Files\Android".into()),
                _ => None,
            }
        };
        let env = build_m(
            &known_for(r"C:\Users\u"),
            &vars(&[
                // di sistema, fuori dal profilo: si accettano
                ("CARGO_HOME", r"D:\cargo"),
                ("GRADLE_USER_HOME", r"C:\gradle"),
                ("ANDROID_HOME", r"C:\Program Files\Android"),
                // dell'utente (il valore del processo non è quello di sistema)
                ("NUGET_PACKAGES", r"E:\nuget"),
                ("PIP_CACHE_DIR", r"D:\pip"),
                ("HF_HOME", r"C:\Program Files\Vendor"),
                // dentro il profilo: basta l'ambiente
                ("UV_CACHE_DIR", r"C:\Users\u\.cache\uv"),
            ]),
            &no_long,
            &machine,
        );
        assert_eq!(env.get("cargo_home"), Some(r"D:\cargo"));
        assert_eq!(env.get("gradle_user_home"), Some(r"C:\gradle"));
        assert_eq!(env.get("android_home"), Some(r"C:\Program Files\Android"));
        assert_eq!(env.get("uv_cache_dir"), Some(r"C:\Users\u\.cache\uv"));
        for k in ["nuget_packages", "pip_cache_dir", "hf_home"] {
            assert_eq!(env.get(k), None, "{k}");
        }
        // di sistema ma protetta: si scarta comunque
        let env = build_m(
            &known_for(r"C:\Users\u"),
            &vars(&[("CARGO_HOME", r"C:\Windows")]),
            &no_long,
            &|n| (n == "cargo_home").then(|| r"C:\Windows".to_string()),
        );
        assert_eq!(env.get("cargo_home"), None);
    }

    const SHORT: &[(&str, &str)] = &[
        ("TEMP", r"C:\Users\RUNNER~1\AppData\Local\Temp"),
        ("TMP", r"C:\Users\RUNNER~1\AppData\Local\Temp\2"),
        ("CARGO_HOME", r"C:\Users\RUNNER~1\.cargo"),
        ("RUSTUP_HOME", r"D:\TOOLS~2\rustup"),
    ];

    fn short_env() -> Env {
        build(&known_for(r"C:\Users\runneradmin"), &vars(SHORT), &fake_long)
    }

    #[test]
    fn short_names_are_converted_to_long_ones() {
        let env = short_env();
        assert_eq!(env.get("temp"), Some(r"C:\Users\runneradmin\AppData\Local\Temp"));
        assert_eq!(env.get("tmp"), Some(r"C:\Users\runneradmin\AppData\Local\Temp\2"));
        assert_eq!(env.get("cargo_home"), Some(r"C:\Users\runneradmin\.cargo"));
        // la conversione non riesce: il nome 8.3 resta e `protected` lo scarta
        assert_eq!(env.get("rustup_home"), None);
    }

    #[test]
    fn long_environment_is_accepted_while_short_rule_paths_are_refused() {
        let env = short_env();
        for rule in [r"%Temp%\claude", r"%Temp%\*", r"%Tmp%\x", r"%Cargo_Home%\registry\cache"] {
            let paths = crate::expand::expand(rule, &env);
            assert!(!paths.is_empty(), "{rule}");
            for p in paths {
                assert_eq!(crate::guard::protected(&p, &env), None, "{}", p.display());
            }
        }
        for p in [
            r"C:\Users\RUNNER~1\AppData\Local\Temp\claude",
            r"C:\PROGRA~1\Foo",
            r"C:\Users\runneradmin\AppData\Local\FOOBAR~1\cache",
        ] {
            assert!(crate::guard::protected(Path::new(p), &env).is_some(), "{p}");
        }
        for p in crate::expand::expand(r"%UserProfile%\*", &env) {
            assert!(crate::guard::protected(&p, &env).is_some(), "{}", p.display());
        }
    }

    /// Stampa l'ambiente vero (`cargo test -p xiox-rules -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn print_real_for_rules() {
        let env = Env::for_rules();
        let mut keys: Vec<&String> = env.0.keys().collect();
        keys.sort();
        for k in keys {
            println!("{k} = {}", env.0[k]);
        }
    }
}
