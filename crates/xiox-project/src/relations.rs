//! The worktrees, the copies and the folders named after one repository, and nothing else: no builds, logs, sessions,
//! history or branches. What the desktop asks of every repository of the PC, with the copies the repository index
//! found handed in as `known`: the search itself only looks beside the repository (`ParentOnly`), so a scan made this
//! way is checked again by [`crate::clean`] without walking the disks.

use crate::history::{HistoryInput, RiskContext, assess};
use crate::owned::{sort_items, worktrees};
use crate::resolve::resolve_full;
use crate::{Env, Error, Note, Options, ProjectScan, SCHEMA, SessionHealth};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// What [`analyze_relations`] needs from the caller: the sessions Analyze read (for the live-session blocks), whether
/// their listing failed, and the clock.
pub struct RelationsInput<'a> {
    pub sessions: &'a [xiox_app::Preloaded],
    pub sessions_failed: bool,
    pub now_ms: i64,
}

/// The `Worktrees`, `Copies` and `Named` items of the repository that holds `path`, each with the level and blocks the
/// command line gives it. `known` are folders found some other way (the other members of the repository's group in the
/// index): they are verified like the folders found beside it. The scan says `ParentOnly` and has no history, so
/// `select` and `clean` take it as they take any other.
pub fn analyze_relations(path: &Path, known: &[PathBuf], input: &RelationsInput) -> Result<ProjectScan, Error> {
    let started = Instant::now();
    let (project, entries) = resolve_full(path)?;
    let mut notes = Vec::new();
    if !project.is_repo {
        notes.push(if xiox_git::git_available() { Note::NotARepository } else { Note::GitMissing });
    }
    if input.sessions_failed {
        notes.push(Note::SessionsNotRead);
    }
    let env = Env {
        sessions: input.sessions,
        archived: &[],
        health: SessionHealth { live_failed: input.sessions_failed, archived_failed: false },
        search_roots: Some(Vec::new()),
        now_ms: input.now_ms,
        history: HistoryInput::none(),
    };
    let opts = Options { drives: false, measure: true, admin: false, history: false };
    // No engine reports: a live session in a worktree is seen from the session records alone.
    let mut items = worktrees(&project, &entries, &[], &env, &opts, &mut notes);
    let found = crate::search::search(&project, &opts, &env, known);
    items.extend(found.items);
    notes.extend(found.notes);
    let none = HashSet::new();
    assess(&mut items, &RiskContext { partial: false, unchecked: &none, now_ms: input.now_ms });
    sort_items(&mut items);
    Ok(ProjectScan {
        schema: SCHEMA,
        project,
        items,
        branches: Vec::new(),
        notes,
        history: None,
        search: found.mode,
        scanned_at_ms: input.now_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
