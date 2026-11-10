//! Tipi condivisi dai parser e dal raccoglitore.

use std::collections::HashMap;

/// Variabili d'ambiente (chiavi in minuscolo). Nei test si passa una mappa
/// finta: nessuna lettura di `std::env` fuori da `from_process`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env(pub HashMap<String, String>);

impl Env {
    /// Copia le variabili del processo, con i nomi in minuscolo.
    pub fn from_process() -> Env {
        Env(std::env::vars_os()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_lowercase(),
                    v.to_string_lossy().into_owned(),
                )
            })
            .collect())
    }

    /// Valore di una variabile; il nome si confronta senza maiuscole.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_lowercase()).map(String::as_str)
    }
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
