//! Whether an item may enter the cleanup, and which `danger` items its removal takes along. One
//! rule for the selection on the scan shown and for the check on the fresh one.

use std::path::PathBuf;

use xiox_model::RemoveMode;

use crate::owned::key_within;
use crate::{Block, Item, Level, Target};

/// Why an item cannot enter the cleanup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Blocked(Vec<Block>),
    /// Removing the item would remove `inner`, which is blocked.
    ContainsBlocked { inner: PathBuf, why: Vec<Block> },
    /// `--permanent` on a `danger` folder, or on a folder that holds one (`inner`).
    DangerPermanent { inner: Option<PathBuf> },
}

/// The `danger` items removing `item` asks to confirm: `item` itself when it is `danger`, then
/// every `danger` item strictly inside it, in key order. A `danger` worktree is no reason to
/// refuse `--permanent`: git removes worktrees for good in every mode.
pub(crate) fn admit<'a>(items: &'a [Item], item: &'a Item, mode: RemoveMode) -> Result<Vec<&'a Item>, Refusal> {
    if !item.blocked.is_empty() {
        return Err(Refusal::Blocked(item.blocked.clone()));
    }
    if let Some(inner) = blocked_inside(items, item) {
        return Err(Refusal::ContainsBlocked { inner: inner.path.clone().unwrap_or_default(), why: inner.blocked.clone() });
    }
    let mut inside: Vec<&Item> = items.iter().filter(|j| j.level == Level::Danger && within(j, item)).collect();
    inside.sort_by(|a, b| a.key.cmp(&b.key));
    let danger: Vec<&Item> = std::iter::once(item).filter(|i| i.level == Level::Danger).chain(inside).collect();
    if mode == RemoveMode::Permanent
        && let Some(folder) = danger.iter().find(|d| matches!(d.target, Target::Dir { .. } | Target::Subtree { .. }))
    {
        let inner = if folder.key == item.key { None } else { folder.path.clone() };
        return Err(Refusal::DangerPermanent { inner });
    }
    Ok(danger)
}

/// The first item of `items` inside `outer` (not `outer` itself) that is blocked.
pub(crate) fn blocked_inside<'a>(items: &'a [Item], outer: &Item) -> Option<&'a Item> {
    items.iter().find(|j| !j.blocked.is_empty() && within(j, outer))
}

/// `item` is a folder strictly inside `outer`.
fn within(item: &Item, outer: &Item) -> bool {
    item.path.is_some() && item.key != outer.key && key_within(&item.key, &outer.key)
}
