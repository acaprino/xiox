//! Plan execution: Recycle Bin / permanent / git worktree remove (`Remover`).
//!
//! Safety model (see docs/DESIGN.md): every item is validated first (`guard`), measured
//! right before removal (`fsops::measure`), removed, then verified gone. `bytes_freed` is
//! the measured size and is only reported when the path is verified gone afterwards.
//! A failed or partial removal is `Failed` and contributes 0 bytes. Processes are never
//! killed; `locked_by` is informational (Windows Restart Manager).

mod fsops;
mod guard;
mod locks;

#[cfg(test)]
mod tests;

pub use fsops::measure as measure_dir;

use dc_model::{
    CleanPlan, CleanResult, ItemOutcome, ItemResult, PlanAction, PlanItem, Remover, RemoveMode, SizeInfo,
    normalize_path, path_key,
};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::{Command, Output};

/// The production [`Remover`].
///
/// * `RemoveMode::Permanent`: unrecoverable delete (read-only bits cleared, retries with
///   backoff, `\\?\` long paths, junctions/symlinks removed as links without following).
/// * `RemoveMode::Recycle`: moves the directory to the Windows Recycle Bin through the
///   `trash` crate. **This does not free disk space until the bin is emptied**; the
///   reported `bytes_freed` is the size that left its original location.
/// * `PlanAction::GitWorktreeRemove`: `git worktree remove` (never `--force`) + `prune`.
/// * An item whose path is itself a link only has the link removed (0 bytes freed),
///   in both modes.
/// * `dry_run = true` runs all validation and measurement and changes nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRemover;

impl Remover for DefaultRemover {
    fn execute(&self, plan: &CleanPlan, dry_run: bool) -> CleanResult {
        let mut result = CleanResult { dry_run, ..Default::default() };
        let mut seen: HashSet<String> = HashSet::new();
        for item in &plan.items {
            let outcome = if !seen.insert(path_key(&item.path)) {
                ItemOutcome::Skipped { reason: "duplicate of an earlier plan item".into() }
            } else {
                catch_unwind(AssertUnwindSafe(|| process(plan.mode, item, dry_run))).unwrap_or_else(|_| {
                    ItemOutcome::Failed { error: "internal error while removing this item".into(), locked_by: vec![] }
                })
            };
            if let ItemOutcome::Removed { bytes_freed } = &outcome {
                result.bytes_freed += bytes_freed;
            }
            result.items.push(ItemResult { path: item.path.clone(), outcome });
        }
        result
    }
}

fn skipped(reason: impl Into<String>) -> ItemOutcome {
    ItemOutcome::Skipped { reason: reason.into() }
}

fn failed(error: impl Into<String>, locked_by: Vec<String>) -> ItemOutcome {
    ItemOutcome::Failed { error: error.into(), locked_by }
}

fn process(mode: RemoveMode, item: &PlanItem, dry_run: bool) -> ItemOutcome {
    let checked = match guard::validate(item) {
        Ok(c) => c,
        Err(reason) => return skipped(reason),
    };
    let path = checked.path.as_path();

    if let PlanAction::GitWorktreeRemove { repo } = &item.action {
        if checked.is_link {
            return skipped("worktree path is a link: refusing git worktree remove");
        }
        if let Err(reason) = validate_repo(repo, path) {
            return skipped(reason);
        }
    }

    // Links: nothing to measure (the target is never touched, so nothing is freed).
    let size = if checked.is_link { SizeInfo::default() } else { fsops::measure(path) };
    let bytes = size.effective_bytes();

    if dry_run {
        return skipped(if checked.is_link {
            "dry-run: would free 0 bytes (path is a link: only the link would be removed)".to_string()
        } else {
            format!("dry-run: would free {bytes} bytes")
        });
    }

    let outcome = if checked.is_link {
        remove_permanent(path, true)
    } else {
        match &item.action {
            PlanAction::RemoveDir => match mode {
                RemoveMode::Permanent => remove_permanent(path, false),
                RemoveMode::Recycle => remove_recycle(path),
            },
            PlanAction::GitWorktreeRemove { repo } => remove_git_worktree(repo, path),
        }
    };
    match outcome {
        // Success only counts once the path is verified gone.
        Ok(()) if fsops::is_gone(path) => ItemOutcome::Removed { bytes_freed: bytes },
        Ok(()) => failed("removal reported success but the path still exists", vec![]),
        Err(f) => f,
    }
}

// ───────────────────────────────── permanent ─────────────────────────────────

fn remove_permanent(path: &Path, link_only: bool) -> Result<(), ItemOutcome> {
    let rep = if link_only { fsops::remove_link_only(path) } else { fsops::remove_path(path) };
    let Some((blocked, err)) = rep.first else { return Ok(()) };
    let locked_by = if matches!(err.raw_os_error(), Some(5 | 32 | 33)) {
        locks::locking_processes(&blocked)
    } else {
        vec![]
    };
    let more = if rep.failures > 1 { format!(" (and {} more entries)", rep.failures - 1) } else { String::new() };
    Err(failed(
        format!("could not remove {}: {err}{more}; the folder may be partially deleted", blocked.display()),
        locked_by,
    ))
}

// ───────────────────────────────── recycle ─────────────────────────────────

fn remove_recycle(path: &Path) -> Result<(), ItemOutcome> {
    match trash::delete(path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Find out which file is holding the directory hostage, for a useful message.
            let probe = fsops::probe_blocking_file(&fsops::verbatim(path));
            let (detail, locked_by) = match probe {
                Some((file, io)) => {
                    let file = normalize_path(&file);
                    (format!("; blocked by {}: {io}", file.display()), locks::locking_processes(&file))
                }
                None => (String::new(), vec![]),
            };
            Err(failed(format!("could not move {} to the Recycle Bin: {e}{detail}", path.display()), locked_by))
        }
    }
}

// ───────────────────────────────── git worktree ─────────────────────────────────

fn validate_repo(repo: &Path, worktree: &Path) -> Result<(), String> {
    if !repo.is_absolute() {
        return Err("worktree repo path is not absolute".into());
    }
    let repo = normalize_path(repo);
    if !repo.is_dir() {
        return Err("worktree repo path does not exist".into());
    }
    let (rk, wk) = (path_key(&repo), path_key(worktree));
    if rk == wk || rk.starts_with(&format!("{wk}\\")) {
        return Err("worktree repo is the worktree itself (or lives inside it)".into());
    }
    Ok(())
}

fn git(args: &[&std::ffi::OsStr]) -> std::io::Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args).env("GIT_TERMINAL_PROMPT", "0").stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.output()
}

fn remove_git_worktree(repo: &Path, worktree: &Path) -> Result<(), ItemOutcome> {
    use std::ffi::OsStr;
    let repo = normalize_path(repo);
    let o = |s: &'static str| OsStr::new(s);

    let out = git(&[o("-C"), repo.as_os_str(), o("worktree"), o("remove"), worktree.as_os_str()])
        .map_err(|e| failed(format!("could not run git: {e}"), vec![]))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // git names the file it could not delete: `failed to delete 'C:/x/y': Permission denied`
        let locked_by = stderr
            .split("failed to delete '")
            .nth(1)
            .and_then(|rest| rest.split('\'').next())
            .map(|p| locks::locking_processes(Path::new(p)))
            .unwrap_or_default();
        return Err(failed(
            format!("git worktree remove failed (not forced; local changes are never discarded): {stderr}"),
            locked_by,
        ));
    }
    // Drop stale registrations. A prune failure does not undo the (verified) removal.
    let _ = git(&[o("-C"), repo.as_os_str(), o("worktree"), o("prune")]);
    Ok(())
}
