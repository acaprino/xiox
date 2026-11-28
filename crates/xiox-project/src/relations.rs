//! The worktrees, the copies and the folders named after one repository, and nothing else: no builds, logs, sessions,
//! history or branches. What the desktop asks of every repository of the PC, with the copies the repository index
//! found handed in as `known`: the search itself only looks beside the repository (`ParentOnly`), so a scan made this
//! way is checked again by [`crate::clean`] without walking the disks.

use crate::history::{HistoryInput, RiskContext, assess};
use crate::owned::{sort_items, worktrees};
use crate::resolve::resolve_full;
use crate::{Category, Env, Error, Note, Options, ProjectScan, SCHEMA, SessionHealth};
use xiox_model::path_key;
use std::collections::{HashMap, HashSet};
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

/// Each folder listed once across the scans of the PC. `references` are the `path_key`s of the roots the other members of
/// their group are copies of. In this order:
/// 1. a `Copies` or `Named` item that is the root of a reference goes: the reference is the project, never a copy;
/// 2. a folder listed by several scans stays in one: a worktree (it goes with `git worktree remove`, never as a folder);
///    else a copy, the one of the scan whose project is a reference; else a named folder, the one of the project with
///    the longest name; on a tie, the first scan.
pub fn keep_once(scans: &mut [ProjectScan], references: &HashSet<String>) {
    let copy_or_named = |c: Category| matches!(c, Category::Copies | Category::Named);
    for scan in scans.iter_mut() {
        scan.items.retain(|i| !(copy_or_named(i.category) && references.contains(&i.key)));
    }
    // The scan that keeps each folder, and how strong its claim is: (kind, from a reference, length of the name).
    let mut holder: HashMap<String, (usize, (u8, bool, usize))> = HashMap::new();
    for (at, scan) in scans.iter().enumerate() {
        let from_reference = references.contains(&path_key(&scan.project.root));
        let name = scan.project.name.chars().count();
        for item in &scan.items {
            let claim = match item.category {
                Category::Worktrees => (3, false, 0),
                Category::Copies => (2, from_reference, 0),
                Category::Named => (1, false, name),
                _ => continue,
            };
            match holder.get(&item.key) {
                Some((_, held)) if *held >= claim => {}
                _ => {
                    holder.insert(item.key.clone(), (at, claim));
                }
            }
        }
    }
    for (at, scan) in scans.iter_mut().enumerate() {
        scan.items.retain(|i| {
            !matches!(i.category, Category::Worktrees | Category::Copies | Category::Named)
                || holder.get(&i.key).is_some_and(|(kept, _)| *kept == at)
        });
    }
}
