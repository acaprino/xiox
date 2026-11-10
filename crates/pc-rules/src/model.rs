//! Tipi condivisi dai parser e dal raccoglitore.

use std::collections::HashMap;

/// Variabili d'ambiente (chiavi in minuscolo). Nei test si passa una mappa
/// finta: nessuna lettura di `std::env` fuori da `from_process`.
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

    /// Valore di una variabile; il nome si confronta senza maiuscole.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_lowercase()).map(String::as_str)
    }
}

#[derive(Clone, Copy)]
enum KnownFolder {
    LocalAppDataLow,
    Documents,
}

/// Percorso di una cartella nota di Windows (`SHGetKnownFolderPath`).
#[cfg(windows)]
fn known_folder(which: KnownFolder) -> Option<String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Documents, FOLDERID_LocalAppDataLow, SHGetKnownFolderPath,
    };
    let id = match which {
        KnownFolder::LocalAppDataLow => &FOLDERID_LocalAppDataLow,
        KnownFolder::Documents => &FOLDERID_Documents,
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
fn known_folder(_which: KnownFolder) -> Option<String> {
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
