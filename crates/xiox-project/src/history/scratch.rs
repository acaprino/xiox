//! The `scratch` category: folders the project's sessions created or wrote in, outside the
//! project. The first rules are lexical and run before any access to the disk; a folder that
//! fails one is only counted.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use xiox_model::{is_system_dir, path_key};
use xiox_scanner::{is_broad_dir, is_cloud_dir};

use super::folders::{Folders, Standing};
use super::scope::{local_dir, Scope};
use super::Exclusion;
use crate::owned::key_within;
use crate::search::{inside_other_repo, is_link, item_for, Own};
use crate::{Activity, Category, Env, Item, Options, Project, Why};

/// The review candidates the sessions' own work gives, in key order. `listed` are the items
/// found otherwise: a folder inside one of them is already covered, its provenance comes from
/// `attach`.
pub(crate) fn scratch_items(
    project: &Project,
    folders: &Folders,
    listed: &[Item],
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
        .map(|(path, _, standing)| {
            let why = Why::SessionFolder {
                strength: standing.strength,
                from_script: standing.from_script,
                risks: Vec::new(),
                activity: Activity::Unknown,
            };
            item_for(opts, env, &path, Category::Scratch, why)
        })
        .collect()
}

/// Where a session folder may be proposed: the rules that need no disk, then the ones that do.
struct Policy {
    own: Own,
    scope: Scope,
    home: Option<PathBuf>,
    temp_key: String,
    fixed_drives: OnceCell<Vec<char>>,
}

impl Policy {
    fn new(project: &Project, opts: &Options, env: &Env) -> Policy {
        let user = env.history.user;
        let non_empty = |p: &Path| Some(p.to_path_buf()).filter(|p| !p.as_os_str().is_empty());
        Policy {
            own: Own::new(project),
            scope: Scope::of(project, opts, env),
            home: non_empty(&user.home),
            temp_key: non_empty(&user.temp).map(|t| path_key(&t)).unwrap_or_default(),
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

    /// A drive root, the home, `%TEMP%` itself, or a broad or system folder. What lies strictly
    /// inside `%TEMP%` is where agents leave their work, so it stays allowed.
    fn protected(&self, path: &Path, key: &str) -> bool {
        let under_temp = !self.temp_key.is_empty() && key != self.temp_key && key_within(key, &self.temp_key);
        let parent_is_root = path.parent().and_then(Path::parent).is_none();
        let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let wide = path.parent().is_none()
            || key == self.temp_key
            || self.home.as_deref().is_some_and(|h| path_key(h) == key)
            || is_broad_dir(path, self.home.as_deref())
            || is_system_dir(&name, parent_is_root);
        wide && !under_temp
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
