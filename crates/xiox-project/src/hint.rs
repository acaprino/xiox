//! What `xiox clean` would accept of an [`Item`], shared by the listing of `xiox analyze` and by
//! the observer that builds the same `xiox clean` command.

use crate::Item;
use std::path::{Component, Path};
use crate::Level;

/// True when the guard of the cleaner (`xiox_cleaner::guard`) refuses the folder for being
/// directly under a drive root or share root: fewer than two folders below it.
pub fn is_top_level(path: &Path) -> bool {
    path.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2
}

/// The guard of `clean` (`xiox_cleaner::guard`) refuses any path directly under a drive or share
/// root, whatever the kind of item: it is shown, never offered.
pub fn too_shallow(item: &Item) -> bool {
    item.path.as_deref().is_some_and(is_top_level)
}

/// An item `clean` may offer as recommended: recommended level, not blocked, not too shallow.
pub fn is_recommended(i: &Item) -> bool {
    i.level == Level::Recommended && i.blocked.is_empty() && !too_shallow(i)
}
