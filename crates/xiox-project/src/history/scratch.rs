//! The `scratch` category: folders the project's sessions created or wrote in, outside the
//! project. The first rules are lexical and run before any access to the disk; a folder that
//! fails one is only counted.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use xiox_model::{is_system_dir, path_key, ArtifactKind};
use xiox_cleaner::guard::in_system_area;
use xiox_scanner::{is_broad_dir, is_cloud_dir};

use super::folders::{Folders, Standing};
use super::risk::{activity, risks};
use super::scope::{has_short_component, local_dir, unlinked_chain, FixedDrives, Scope};
use super::{Exclusion, HistoryReport, Limit};
use crate::owned::key_within;
use crate::search::{inside_other_repo, item_for, Own};
use super::subtree::tracked_blocks;
use crate::{Category, Env, Item, Options, Project, Target, Why};

/// The session folders examined as `scratch` candidates, most recently active first.
pub(crate) const MAX_SCRATCH: usize = 200;

/// The review candidates the sessions' own work gives, in key order. `listed` are the items
/// found otherwise: a folder inside one of them is already covered, its provenance comes from
/// `attach`.
pub(crate) fn scratch_items(
    project: &Project,
    folders: &Folders,
    listed: &[Item],
    env: &Env,
    opts: &Options,
    report: &mut HistoryReport,
) -> Vec<Item> {
    let policy = Policy::new(project, opts, env);
    // Parents come before the folders inside them, so a folder is merged into the outermost
    // candidate that holds it and never climbs above a folder with evidence of its own.
    let mut accepted: Vec<(PathBuf, String, Standing, Target)> = Vec::new();
    for marked in folders.marked() {
        if listed.iter().any(|i| key_within(marked.key, &i.key)) {
            continue;
        }
        if let Some((_, _, standing, _)) = accepted.iter_mut().find(|(_, key, ..)| key_within(marked.key, key)) {
            *standing = standing.merge(marked.standing);
            continue;
        }
        match policy.admit(marked.path) {
            Ok((path, target)) => accepted.push((path, marked.key.to_string(), marked.standing, target)),
            Err(why) => *report.excluded.entry(why).or_default() += 1,
        }
    }
    // Sizing and git status are the costly part: only the most recently active folders get them.
    let mut candidates: Vec<_> = accepted
        .into_iter()
        .filter_map(|(path, key, standing, target)| Some((path, standing, folders.provenance_within(&key)?, target)))
        .collect();
    if candidates.len() > MAX_SCRATCH {
        candidates.sort_by_key(|(_, _, p, _)| (p.last_activity_ms.is_none(), Reverse(p.last_activity_ms)));
        report.limits.push(Limit::Scratch { max: MAX_SCRATCH, left: candidates.len() - MAX_SCRATCH });
        candidates.truncate(MAX_SCRATCH);
        candidates.sort_by_key(|(path, ..)| path_key(path));
    }
    let partial = report.partial();
    candidates
        .into_iter()
        .map(|(path, standing, provenance, target)| {
            let why = Why::SessionFolder {
                strength: standing.strength,
                from_script: standing.from_script,
                activity: activity(&path, env),
            };
            let risks = risks(&path, &provenance, partial, env.now_ms);
            let mut item = item_for(opts, env, &path, Category::Scratch, why);
            if let Target::Subtree { repo } = &target {
                let mut blocked = tracked_blocks(repo, &path);
                blocked.extend(item.blocked.drain(..).filter(|b| !blocked.contains(b)).collect::<Vec<_>>());
                item.blocked = blocked;
            }
            Item { risks, target, provenance: Some(provenance), ..item }
        })
        .collect()
}

/// Where a session folder may be proposed: the rules that need no disk, then the ones that do.
struct Policy {
    own: Own,
    /// The project and its worktrees, for the repository a folder inside one of them belongs to.
    roots: Vec<PathBuf>,
    scope: Scope,
    home: Option<PathBuf>,
    temp_key: String,
    /// The `%TEMP%` is a drive root, the home or a broad folder: nothing inside it is exempt.
    temp_is_broad: bool,
    /// The keys no folder may hold: the home, and a `%TEMP%` that is not inside `AppData`.
    keep_whole: Vec<String>,
    fixed_drives: FixedDrives,
    /// The home and the temp are in their long form; otherwise nothing is admitted.
    resolved: bool,
}

impl Policy {
    fn new(project: &Project, opts: &Options, env: &Env) -> Policy {
        let user = env.history.user;
        let non_empty = |p: &Path| Some(p.to_path_buf()).filter(|p| !p.as_os_str().is_empty());
        let home = non_empty(&user.home);
        let temp = non_empty(&user.temp);
        let temp_key = temp.as_deref().map(path_key).unwrap_or_default();
        let home_key = home.as_deref().map(path_key).unwrap_or_default();
        let app_data = home.as_deref().map(|h| path_key(&h.join("AppData")));
        let under_app_data = app_data.as_deref().is_some_and(|a| key_within(&temp_key, a));
        // A `%TEMP%` exempts its own children from the broad-folder rule only when it is the
        // usual `<home>\AppData\Local\Temp`, or a plain folder outside `AppData`.
        let usual = home.as_deref().is_some_and(|h| temp_key == path_key(&h.join("AppData").join("Local").join("Temp")));
        let plain = temp.as_deref().is_some_and(|t| {
            !under_app_data && t.parent().is_some() && temp_key != home_key && !is_broad_dir(t, home.as_deref())
        });
        let temp_is_broad = !(usual || plain);
        // Every folder under the home's `AppData` is broad, so the holders of a temp there are
        // already protected; any other temp must be kept whole here.
        let keep_whole = [Some(home_key), (!under_app_data).then(|| temp_key.clone())]
            .into_iter()
            .flatten()
            .filter(|k| !k.is_empty())
            .collect();
        Policy {
            own: Own::new(project),
            roots: std::iter::once(&project.root).chain(project.worktrees.iter()).cloned().collect(),
            scope: Scope::of(project, opts, env),
            home,
            temp_key,
            temp_is_broad,
            keep_whole,
            fixed_drives: FixedDrives::default(),
            resolved: user.resolved,
        }
    }

    /// The folder as a clean local path, or why it is left out.
    fn admit(&self, raw: &Path) -> Result<(PathBuf, Target), Exclusion> {
        if has_short_component(raw) {
            return Err(Exclusion::ShortName);
        }
        let path = local_dir(&raw.to_string_lossy())?;
        if !self.resolved || in_system_area(&path) || self.other_profile(&path) {
            return Err(Exclusion::Protected);
        }
        let key = path_key(&path);
        if !self.scope.contains(&key) {
            return Err(Exclusion::OutsideScope);
        }
        if let Some(repo) = self.repo_holding(&key) {
            // Inside the project or a worktree: the subtree policy, never the outside rules.
            let in_git = path.components().any(|c| c.as_os_str().eq_ignore_ascii_case(".git"));
            if in_git || self.own.inside(&key) {
                return Err(Exclusion::Protected);
            }
            if !self.fixed_drives.holds(&path) {
                return Err(Exclusion::NotLocal);
            }
            unlinked_chain(&path)?;
            present_dir(&path)?;
            return Ok((path, Target::Subtree { repo }));
        }
        if self.own.holds(&key) {
            return Err(Exclusion::InsideProject);
        }
        if self.own.inside(&key) || self.protected(&path, &key) {
            return Err(Exclusion::Protected);
        }
        if !self.fixed_drives.holds(&path) {
            return Err(Exclusion::NotLocal);
        }
        self.on_disk(&path)?;
        Ok((path, Target::Dir { kind: ArtifactKind::ProjectFolder }))
    }

    /// The innermost of the project and its worktrees that strictly holds `key`.
    fn repo_holding(&self, key: &str) -> Option<PathBuf> {
        self.roots
            .iter()
            .map(|r| (path_key(r), r))
            .filter(|(rk, _)| rk.as_str() != key && key_within(key, rk))
            .max_by_key(|(rk, _)| rk.len())
            .map(|(_, r)| r.clone())
    }

    /// A drive root, the home or a folder that holds it, `%TEMP%` itself, a folder that holds a
    /// `%TEMP%` outside `AppData`, or a broad or system folder. What lies strictly inside a
    /// `%TEMP%` that is not broad is where agents leave their work, so it stays allowed.
    fn protected(&self, path: &Path, key: &str) -> bool {
        let parent_is_root = path.parent().and_then(Path::parent).is_none();
        let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let holds_a_root = self.keep_whole.iter().any(|kept| key_within(kept, key));
        let broad = is_broad_dir(path, self.home.as_deref()) || is_system_dir(&name, parent_is_root);
        let under_plain_temp = !self.temp_is_broad && key != self.temp_key && key_within(key, &self.temp_key);
        path.parent().is_none() || key == self.temp_key || holds_a_root || (broad && !under_plain_temp)
    }

    /// `<drive>:\Users\<name>\…` of a profile other than the home (also `Public` and `Default`).
    fn other_profile(&self, path: &Path) -> bool {
        let mut names = path.components().filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        });
        let own = self.home.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase());
        names.next().as_deref() == Some("users") && names.next().is_some_and(|name| Some(name) != own)
    }

    /// The rules that look at the disk, never entering a link.
    fn on_disk(&self, path: &Path) -> Result<(), Exclusion> {
        unlinked_chain(path)?;
        if is_cloud_dir(path) {
            return Err(Exclusion::Protected);
        }
        present_dir(path)?;
        if inside_other_repo(path) {
            return Err(Exclusion::OtherRepository);
        }
        Ok(())
    }
}

/// The folder is there and is a folder; an access error is not taken for its absence.
fn present_dir(path: &Path) -> Result<(), Exclusion> {
    match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(Exclusion::Missing),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Exclusion::Missing),
        Err(_) => Err(Exclusion::Unreadable),
    }
}
