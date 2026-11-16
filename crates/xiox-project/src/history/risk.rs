//! What makes a session folder risky to remove, and whether a session may still be in it. The
//! risks are facts for the review; the `danger` level that reads them is decided elsewhere.

use std::path::Path;
use std::time::UNIX_EPOCH;

use super::{Activity, Provenance, Risk};
use crate::owned::live_session_in;
use crate::{Env, Strength};

/// How long ago still counts as recent: a day.
pub(crate) const RECENT_MS: i64 = 24 * 60 * 60 * 1000;

/// The risks of removing `dir`, in `Risk` order and without repeats.
pub(crate) fn risks(dir: &Path, provenance: &Provenance, report_partial: bool, now_ms: i64) -> Vec<Risk> {
    let recent_since = now_ms - RECENT_MS;
    let observed_recently = provenance.last_activity_ms.is_some_and(|t| t >= recent_since);
    let conditions = [
        (Risk::NoGit, !dir.join(".git").exists()),
        (Risk::WeakEvidence, provenance.strength < Some(Strength::Created)),
        (Risk::PartialCoverage, report_partial || provenance.sessions.iter().any(|s| s.gaps)),
        // The folder date only feeds this recency; it never enters the provenance.
        (Risk::RecentActivity, observed_recently || modified_ms(dir).is_some_and(|t| t >= recent_since)),
    ];
    conditions.into_iter().filter_map(|(risk, holds)| holds.then_some(risk)).collect()
}

/// `Active` when a live session works in `dir`. Nothing tells that a session is not active.
pub(crate) fn activity(dir: &Path, env: &Env) -> Activity {
    if live_session_in(dir, env) {
        Activity::Active
    } else {
        Activity::Unknown
    }
}

fn modified_ms(dir: &Path) -> Option<i64> {
    let since_epoch = dir.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}
