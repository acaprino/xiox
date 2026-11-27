//! The choices of Cleanup kept from one start to the next: the `cleanup` section of `pref.json`
//! ([`xiox_app::prefs`]). Only what differs from the defaults is written: a category or a native
//! item that a later version adds starts at its own default, and the options of rules that have not
//! loaded yet (Winapp2 arrives after the window) keep their keys as they are.

use crate::{cleaner, native, rules};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// What the section holds. A missing or unreadable section, and any field it lacks, are the defaults.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// Built-in categories by name (their place in `CATEGORIES` may change), where the choice differs
    /// from `default_on`.
    pub categories: BTreeMap<String, bool>,
    /// Native items (`native:<id>`) and rule options (`<cleaner>/<option>`) where the choice differs
    /// from the default: a native's `default_on`, off for an option.
    pub items: BTreeMap<String, bool>,
    pub only_old: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { categories: BTreeMap::new(), items: BTreeMap::new(), only_old: true }
    }
}

impl Prefs {
    /// The choices on screen as differences from the defaults (`sel` follows `CATEGORIES`).
    pub fn capture(sel: &[bool], chosen: &HashSet<String>, natives: &[native::NativeItem], only_old: bool) -> Prefs {
        let categories = cleaner::CATEGORIES
            .iter()
            .zip(sel.iter().copied())
            .filter(|(c, on)| c.default_on != *on)
            .map(|(c, on)| (c.name.to_string(), on))
            .collect();
        let defaults = rules::native_defaults(natives);
        let on = chosen.difference(&defaults).map(|k| (k.clone(), true));
        let off = defaults.difference(chosen).map(|k| (k.clone(), false));
        Prefs { categories, items: on.chain(off).collect(), only_old }
    }

    /// Applies the choices over the defaults. A category name `CATEGORIES` no longer has is ignored; an
    /// unknown item key is harmless in `chosen`, as only the rows the list shows are read from it.
    pub fn apply(&self, sel: &mut [bool], chosen: &mut HashSet<String>, only_old: &mut bool) {
        for (c, on) in cleaner::CATEGORIES.iter().zip(sel.iter_mut()) {
            if let Some(&v) = self.categories.get(c.name) {
                *on = v;
            }
        }
        for (key, &v) in &self.items {
            if v {
                chosen.insert(key.clone());
            } else {
                chosen.remove(key);
            }
        }
        *only_old = self.only_old;
    }
}
