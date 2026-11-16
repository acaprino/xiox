//! The `scratch` category: folders the project's sessions created or wrote in, outside the
//! project. The first rules are lexical and run before any access to the disk; a folder that
//! fails one is only counted.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use xiox_model::{is_system_dir, path_key};
use xiox_scanner::{is_broad_dir, is_cloud_dir};

use super::folders::{Folders, Standing};
use super::risk::{activity, risks};
use super::scope::{local_dir, Scope};
use super::Exclusion;
use crate::owned::key_within;
use crate::search::{inside_other_repo, is_link, item_for, Own};
use crate::{Category, Env, Item, Options, Project, Why};

/// The review candidates the sessions' own work gives, in key order. `listed` are the items
/// found otherwise: a folder inside one of them is already covered, its provenance comes from
/// `attach`.
pub(crate) fn scratch_items(
    project: &Project,
    folders: &Folders,
    listed: &[Item],
    report_partial: bool,
    env: &Env,
    opts: &Options,
    excluded: &mut BTreeMap<Exclusion, usize>,
) -> Vec<Item> {
    let policy = Policy::new(project, opts, env);
    // Parents come before the folders inside them, so a folder is merged into the outermost
    // candidate that holds it and never climbs above a folder with evidence of its own.
    let mut accepted: Vec<(PathBuf, String, Standing)> = Vec::new();
    for marked in folders.marked() {
        if listed.iter().any(|i| key_within(marked.key, &i.key)) {
            continue;
        }
        if let Some((_, _, standing)) = accepted.iter_mut().find(|(_, key, _)| key_within(marked.key, key)) {
            *standing = standing.merge(marked.standing);
            continue;
        }
        match policy.admit(marked.path) {
            Ok(path) => accepted.push((path, marked.key.to_string(), marked.standing)),
            Err(why) => *excluded.entry(why).or_default() += 1,
        }
    }
    accepted
        .into_iter()
        .filter_map(|(path, key, standing)| {
            let provenance = folders.provenance_within(&key)?;
            let why = Why::SessionFolder {
                strength: standing.strength,
                from_script: standing.from_script,
                risks: risks(&path, &provenance, report_partial, env.now_ms),
                activity: activity(&path, env),
            };
            Some(Item { provenance: Some(provenance), ..item_for(opts, env, &path, Category::Scratch, why) })
        })
        .collect()
}

/// Where a session folder may be proposed: the rules that need no disk, then the ones that do.
struct Policy {
    own: Own,
    scope: Scope,
    home: Option<PathBuf>,
    temp_key: String,
    /// The `%TEMP%` is a drive root, the home or a broad folder: nothing inside it is exempt.
    temp_is_broad: bool,
    /// The keys no folder may hold: the home, and a `%TEMP%` that is not inside `AppData`.
    keep_whole: Vec<String>,
    fixed_drives: OnceCell<Vec<char>>,
}

impl Policy {
    fn new(project: &Project, opts: &Options, env: &Env) -> Policy {
        let user = env.history.user;
        let non_empty = |p: &Path| Some(p.to_path_buf()).filter(|p| !p.as_os_str().is_empty());
        let home = non_empty(&user.home);
        let temp = non_empty(&user.temp);
        let temp_key = temp.as_deref().map(path_key).unwrap_or_default();
        let home_key = home.as_deref().map(path_key).unwrap_or_default();
        // Every folder under the home's `AppData` is broad, and that is where the usual `%TEMP%`
        // lies, so its holders are already protected. Any other temp must be kept whole here.
        let in_home_app_data = temp.as_deref().zip(home.as_deref()).is_some_and(|(_, h)| {
            key_within(&temp_key, &path_key(&h.join("AppData")))
        });
        let temp_is_broad = temp.as_deref().is_some_and(|t| {
            t.parent().is_none() || temp_key == home_key || (!in_home_app_data && is_broad_dir(t, home.as_deref()))
        });
        let keep_whole = [Some(home_key), (!in_home_app_data).then(|| temp_key.clone())]
            .into_iter()
            .flatten()
            .filter(|k| !k.is_empty())
            .collect();
        Policy {
            own: Own::new(project),
            scope: Scope::of(project, opts, env),
            home,
            temp_key,
            temp_is_broad,
            keep_whole,
            fixed_drives: OnceCell::new(),
        }
    }

    /// The folder as a clean local path, or why it is left out.
    fn admit(&self, raw: &Path) -> Result<PathBuf, Exclusion> {
        let path = local_dir(&raw.to_string_lossy())?;
        let key = path_key(&path);
        if !self.scope.contains(&key) {
            return Err(Exclusion::OutsideScope);
        }
        if self.own.holds(&key) {
            return Err(Exclusion::InsideProject);
        }
        if self.own.inside(&key) || self.protected(&path, &key) {
            return Err(Exclusion::Protected);
        }
        if !self.on_fixed_drive(&path) {
            return Err(Exclusion::NotLocal);
        }
        self.on_disk(&path)?;
        Ok(path)
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

    fn on_fixed_drive(&self, path: &Path) -> bool {
        let letter = path.to_string_lossy().chars().next().map(|c| c.to_ascii_uppercase());
        let drives = self.fixed_drives.get_or_init(|| xiox_disktree::fixed_drives().iter().map(|d| d.letter).collect());
        letter.is_some_and(|l| drives.contains(&l))
    }

    /// The rules that look at the disk, never entering a link.
    fn on_disk(&self, path: &Path) -> Result<(), Exclusion> {
        let mut chain: Vec<&Path> = path.ancestors().filter(|a| a.parent().is_some()).collect();
        chain.reverse();
        for dir in chain {
            match dir.symlink_metadata() {
                Ok(meta) if is_link(&meta) => return Err(Exclusion::Link),
                Ok(_) => {}
                Err(_) => return Err(Exclusion::Missing),
            }
        }
        if is_cloud_dir(path) {
            return Err(Exclusion::Protected);
        }
        if !path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            return Err(Exclusion::Missing);
        }
        if inside_other_repo(path) {
            return Err(Exclusion::OtherRepository);
        }
        Ok(())
    }
}
