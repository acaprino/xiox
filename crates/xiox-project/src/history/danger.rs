//! The `danger` level: an item whose removal carries a risk its category weighs asks for an
//! explicit confirmation. The other levels stay as the analysis set them.

use std::collections::HashSet;

use super::risk::risks;
use super::Risk;
use crate::owned::key_within;
use crate::{Category, Item, Level};

/// What the risks read beyond each folder.
pub(crate) struct RiskContext<'a> {
    /// The history was read only in part.
    pub partial: bool,
    /// The candidates a session of another project may have written in unseen.
    pub unchecked: &'a HashSet<String>,
    pub now_ms: i64,
}

const ALL: &[Risk] = &[Risk::NoGit, Risk::WeakEvidence, Risk::PartialCoverage, Risk::RecentActivity];

/// The risks a category weighs. Builds, logs and archived sessions are rebuilt or kept
/// elsewhere: none. A folder known only by its name has no evidence to be weak or partial.
pub(crate) fn considered(category: Category, has_provenance: bool) -> &'static [Risk] {
    match category {
        Category::Scratch => ALL,
        Category::Named if has_provenance => ALL,
        Category::Named => &[Risk::NoGit, Risk::RecentActivity],
        Category::Copies | Category::Worktrees => &[Risk::RecentActivity],
        Category::Builds | Category::Logs | Category::Sessions => &[],
    }
}

/// Sets the risks of every item, lets recent activity reach the folders that hold it, then
/// raises to `Danger` every item with a risk.
pub(crate) fn assess(items: &mut [Item], ctx: &RiskContext) {
    for item in items.iter_mut() {
        let weighed = considered(item.category, item.provenance.is_some());
        let Some(dir) = item.path.as_deref().filter(|_| !weighed.is_empty()) else { continue };
        let partial = ctx.partial || ctx.unchecked.contains(&item.key);
        item.risks = risks(dir, item.provenance.as_ref(), partial, ctx.now_ms);
        item.risks.retain(|r| weighed.contains(r));
    }
    reach_containers(items);
    for item in items.iter_mut().filter(|i| !i.risks.is_empty()) {
        item.level = Level::Danger;
    }
}

/// Recent activity inside a folder is recent activity of the folder.
fn reach_containers(items: &mut [Item]) {
    let recent: Vec<String> =
        items.iter().filter(|i| i.risks.contains(&Risk::RecentActivity)).map(|i| i.key.clone()).collect();
    for item in items.iter_mut() {
        let weighs = considered(item.category, item.provenance.is_some()).contains(&Risk::RecentActivity);
        let inside = recent.iter().any(|k| k != &item.key && key_within(k, &item.key));
        if weighs && inside && !item.risks.contains(&Risk::RecentActivity) {
            // The last risk in `Risk` order: pushing keeps the order.
            item.risks.push(Risk::RecentActivity);
        }
    }
}
