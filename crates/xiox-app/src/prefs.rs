//! `pref.json`: the settings of the app in one file beside the executable, so the portable copy keeps its settings in
//! its own folder (the installer puts the program in a folder of the user's, which the user can write). The window's
//! place, theme and glass are imago-shell's own files, in the same folder.
//!
//! The file is one JSON object with a section per part of the app ([`SECTIONS`]). A part reads and writes only its own
//! section, and a write keeps every other one as it is, also those this version does not know. A new setting is a new
//! section added to [`SECTIONS`], or a new field of a section: its type reads it with a default (`#[serde(default)]`), or
//! its owner reads the section over its defaults ([`Store::read_over`]), so the file of an older version keeps the
//! other fields. A section that cannot be read at all is no section: its owner falls back to its defaults (or to the
//! file an older version kept it in).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = "pref.json";

/// The Developers options (`xiox_model::Settings`).
pub const DEVELOPERS: &str = "developers";
/// The choices of Cleanup (ticked items, only old Temp files).
pub const CLEANUP: &str = "cleanup";
/// Every section this version writes.
pub const SECTIONS: [&str; 2] = [DEVELOPERS, CLEANUP];

/// The writers of one process take turns: each one reads the file again and puts back the others' sections.
static WRITING: Mutex<()> = Mutex::new(());

/// The settings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// `pref.json` beside the executable; beside the old `settings.json` (`%APPDATA%\Xiox`) when the executable's path
    /// cannot be read.
    pub fn beside_exe() -> Store {
        let path = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(FILE)));
        Store { path: path.unwrap_or_else(|| crate::settings_path().with_file_name(FILE)) }
    }

    /// The file at `path` (the tests).
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The section, if the file has it and it reads as `T`.
    pub fn read<T: DeserializeOwned>(&self, section: &str) -> Option<T> {
        self.sections().ok()?.remove(section).and_then(|value| serde_json::from_value(value).ok())
    }

    /// The section laid over `defaults`, for a type with no serde default of its own: the fields the section has replace
    /// those of `defaults`, and a field it lacks (added by a later version) keeps its default. `None` without a section;
    /// a section that still does not read is `defaults`.
    pub fn read_over<T: Serialize + DeserializeOwned>(&self, section: &str, defaults: T) -> Option<T> {
        let Value::Object(saved) = self.sections().ok()?.remove(section)? else { return Some(defaults) };
        let Ok(Value::Object(mut merged)) = serde_json::to_value(&defaults) else { return Some(defaults) };
        merged.extend(saved);
        Some(serde_json::from_value(Value::Object(merged)).unwrap_or(defaults))
    }

    /// Replaces the section, keeping the others. A file that is not a JSON object (edited by hand and broken) is kept
    /// as `pref.json.bad` before a new one is written: its sections are not lost without a trace.
    pub fn write<T: Serialize>(&self, section: &str, value: &T) -> Result<(), String> {
        let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
        let _turn = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = match self.sections() {
            Ok(all) => all,
            Err(()) => {
                std::fs::copy(&self.path, self.path.with_extension("json.bad")).map_err(|e| e.to_string())?;
                Map::new()
            }
        };
        all.insert(section.to_string(), value);
        let text = serde_json::to_string_pretty(&Value::Object(all)).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // Written beside and then renamed over the file: a crash halfway leaves the old file whole.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &self.path).map_err(|e| {
            // The file is held (an editor, an antivirus): no temporary file is left beside it.
            let _ = std::fs::remove_file(&temp);
            e.to_string()
        })
    }

    /// Every section; no file is no section. `Err`: a file that is there but cannot be read or is not a JSON object.
    fn sections(&self) -> Result<Map<String, Value>, ()> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(all)) => Ok(all),
                _ => Err(()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
            Err(_) => Err(()),
        }
    }
}
