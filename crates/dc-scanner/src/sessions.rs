//! Project roots from the working directories of agent sessions.
//!
//! The agents' sessions say where the user works: each session's working directory is a
//! project, a folder inside one, or a folder that is no project at all (the home, the Desktop,
//! a drive root, a cloud drive, a scratch folder an agent made for a chat). This module turns
//! those directories into the roots the scanner walks, read-only:
//!
//! 1. a directory that no longer exists is skipped (`Missing`): nothing is left to clean;
//! 2. a cloud-synced or placeholder folder is skipped (`Cloud`): walking it can make the sync
//!    client download files;
//! 3. a folder that holds everything (drive root, home, Desktop, Documents, Downloads, AppData,
//!    system folders) is skipped (`Broad`), and the search never climbs into one;
//! 4. otherwise the root is the nearest enclosing git repository, else the nearest enclosing
//!    folder with a project marker (`Cargo.toml`, `package.json`, ...), else the folder itself
//!    when its direct children hold between 1 and [`MAX_WORKSPACE_CHILDREN`] projects (a
//!    `backend` + `frontend` workspace); anything else is `NotAProject`.

use crate::fsutil::list_dir;
use crate::markers::{NEVER_ENTER, detect};
use dc_model::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A folder with no repository and no marker of its own counts as a workspace when its direct
/// children hold at most this many projects. `D:\Projects`, with a hundred, is not one.
pub const MAX_WORKSPACE_CHILDREN: usize = 8;

/// Why a session directory gave no project root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Skip {
    /// The folder no longer exists, or its drive is not connected.
    Missing,
    /// A folder that holds files of every kind and is never a project.
    Broad,
    /// Inside a cloud-synced drive or a cloud placeholder.
    Cloud,
    /// No repository or marker in it or above it, and not a small workspace.
    NotAProject,
}

#[derive(Debug, Default)]
pub struct SessionRoots {
    /// Distinct project roots, sorted by `path_key`. One root may lie inside another (a session
    /// in a workspace and one in a project below it): the scanner keeps the outer one.
    pub roots: Vec<PathBuf>,
    /// Session directories that gave no root, each once, with the reason.
    pub skipped: Vec<(PathBuf, Skip)>,
}

/// Project roots for the session working directories `dirs`. `home` is the user's profile
/// folder (`%USERPROFILE%`); it and its standard children are never roots.
pub fn project_roots_from_sessions(dirs: &[PathBuf], home: Option<&Path>) -> SessionRoots {
    let home_key = home.map(path_key);
    let mut out = SessionRoots::default();
    let mut seen_dirs = HashSet::new();
    let mut seen_roots = HashSet::new();
    for d in dirs {
        let d = normalize_path(d);
        if !seen_dirs.insert(path_key(&d)) {
            continue;
        }
        match resolve(&d, home_key.as_deref()) {
            Ok(root) => {
                if seen_roots.insert(path_key(&root)) {
                    out.roots.push(root);
                }
            }
            Err(why) => out.skipped.push((d, why)),
        }
    }
    out.roots.sort_by_key(|p| path_key(p));
    out
}

fn resolve(d: &Path, home: Option<&str>) -> std::result::Result<PathBuf, Skip> {
    // Names first: no filesystem access at all for a cloud path.
    if is_cloud_name(d) {
        return Err(Skip::Cloud);
    }
    if !d.is_dir() {
        return Err(Skip::Missing);
    }
    if is_cloud_placeholder(d) {
        return Err(Skip::Cloud);
    }
    if is_broad(d, home) {
        return Err(Skip::Broad);
    }
    let mut nearest_marker: Option<PathBuf> = None;
    for p in d.ancestors() {
        if is_broad(p, home) {
            break;
        }
        if p.join(".git").exists() {
            return Ok(normalize_path(p));
        }
        if nearest_marker.is_none() && has_marker(p) {
            nearest_marker = Some(normalize_path(p));
        }
    }
    if let Some(m) = nearest_marker {
        return Ok(m);
    }
    if (1..=MAX_WORKSPACE_CHILDREN).contains(&projects_below(d)) {
        return Ok(normalize_path(d));
    }
    Err(Skip::NotAProject)
}

fn has_marker(dir: &Path) -> bool {
    list_dir(dir).is_ok_and(|l| detect(l.files.iter().map(|f| f.name.as_str())).is_some())
}

/// Direct children that are a repository or hold a marker; stops counting past the limit.
fn projects_below(dir: &Path) -> usize {
    let Ok(listing) = list_dir(dir) else { return 0 };
    let mut n = 0;
    for child in &listing.dirs {
        if NEVER_ENTER.contains(&child.name.to_ascii_lowercase().as_str()) {
            continue;
        }
        if child.path.join(".git").exists() || has_marker(&child.path) {
            n += 1;
            if n > MAX_WORKSPACE_CHILDREN {
                break;
            }
        }
    }
    n
}

/// Children of the home folder that collect files of every kind.
const BROAD_HOME_CHILDREN: &[&str] =
    &["desktop", "documents", "downloads", "pictures", "videos", "music", "appdata", "favorites"];

/// Drive root, home, standard home folders, anything under AppData, system folders.
fn is_broad(p: &Path, home: Option<&str>) -> bool {
    let Some(parent) = p.parent() else { return true };
    let key = path_key(p);
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some(home) = home {
        if key == home {
            return true;
        }
        if path_key(parent) == home && (BROAD_HOME_CHILDREN.contains(&name.as_str()) || name.starts_with("onedrive")) {
            return true;
        }
        // Everything under the home's AppData: application data, never a project.
        let appdata = format!("{home}{}appdata", std::path::MAIN_SEPARATOR);
        if key.strip_prefix(&appdata).is_some_and(|rest| rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR)) {
            return true;
        }
    }
    if parent.parent().is_none()
        && (name == "windows" || name == "programdata" || name.starts_with("program files") || name == "users")
    {
        return true;
    }
    name == "$recycle.bin" || name == "system volume information"
}

/// Folder names of Google Drive (also in Italian), OneDrive, Dropbox and iCloud Drive.
fn is_cloud_name(p: &Path) -> bool {
    p.components().any(|c| {
        let n = c.as_os_str().to_string_lossy().to_lowercase();
        matches!(
            n.as_str(),
            "google drive" | "il mio drive" | "my drive" | "drive condivisi" | "shared drives" | "dropbox" | "icloud drive" | "iclouddrive"
        ) || n.starts_with("onedrive")
    })
}

/// A folder the cloud client keeps only as a placeholder (Windows cloud-files attributes).
#[cfg(windows)]
fn is_cloud_placeholder(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    std::fs::metadata(p).is_ok_and(|m| m.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0)
}

#[cfg(not(windows))]
fn is_cloud_placeholder(_p: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk(p: &Path) {
        fs::create_dir_all(p).unwrap();
    }

    fn marker(dir: &Path, name: &str) {
        mk(dir);
        fs::write(dir.join(name), "x").unwrap();
    }

    fn roots(dirs: &[&Path], home: &Path) -> SessionRoots {
        let v: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
        project_roots_from_sessions(&v, Some(home))
    }

    fn keys(v: &[PathBuf]) -> Vec<String> {
        v.iter().map(|p| path_key(p)).collect()
    }

    #[test]
    fn git_root_wins_over_a_marker_below_it() {
        let td = tempfile::tempdir().unwrap();
        let repo = td.path().join("work").join("repo");
        mk(&repo.join(".git"));
        let sub = repo.join("src-tauri").join("src");
        marker(&repo.join("src-tauri"), "Cargo.toml");
        mk(&sub);
        let r = roots(&[&sub, &repo.join("src-tauri")], td.path());
        assert_eq!(keys(&r.roots), vec![path_key(&repo)], "both sessions give the repository");
        assert!(r.skipped.is_empty());
    }

    #[test]
    fn linked_worktree_checkout_is_a_repository() {
        let td = tempfile::tempdir().unwrap();
        let wt = td.path().join("wt").join("fix-a");
        mk(&wt);
        fs::write(wt.join(".git"), "gitdir: elsewhere").unwrap();
        assert_eq!(keys(&roots(&[&wt], td.path()).roots), vec![path_key(&wt)]);
    }

    #[test]
    fn nearest_marker_without_a_repository() {
        let td = tempfile::tempdir().unwrap();
        let proj = td.path().join("work").join("app");
        marker(&proj, "package.json");
        let deep = proj.join("src").join("pages");
        mk(&deep);
        assert_eq!(keys(&roots(&[&deep], td.path()).roots), vec![path_key(&proj)]);
    }

    #[test]
    fn small_workspace_yes_container_of_many_projects_no() {
        let td = tempfile::tempdir().unwrap();
        let ws = td.path().join("work").join("shop");
        marker(&ws.join("backend"), "Cargo.toml");
        marker(&ws.join("frontend"), "package.json");
        mk(&ws.join("docs"));
        let many = td.path().join("work").join("everything");
        for i in 0..=MAX_WORKSPACE_CHILDREN {
            marker(&many.join(format!("p{i}")), "Cargo.toml");
        }
        let empty = td.path().join("work").join("notes");
        mk(&empty);
        let r = roots(&[&ws, &many, &empty], td.path());
        assert_eq!(keys(&r.roots), vec![path_key(&ws)]);
        let mut skipped: Vec<(String, Skip)> = r.skipped.iter().map(|(p, s)| (path_key(p), *s)).collect();
        skipped.sort();
        let mut want = vec![(path_key(&many), Skip::NotAProject), (path_key(&empty), Skip::NotAProject)];
        want.sort();
        assert_eq!(skipped, want);
    }

    #[test]
    fn home_desktop_appdata_and_drive_root_are_never_roots() {
        let td = tempfile::tempdir().unwrap();
        let home = td.path().join("home");
        let desktop = home.join("Desktop");
        mk(&desktop);
        // A repository in the home folder (dotfiles) must not turn the home into a project.
        mk(&home.join(".git"));
        let chat = home.join("Documents").join("Codex").join("2026-08-20").join("chat");
        mk(&chat);
        let plugin = home.join("AppData").join("Roaming").join("tool").join("plugin");
        marker(&plugin, "package.json");
        let drive = td.path().ancestors().last().unwrap().to_path_buf();
        let r = roots(&[&home, &desktop, &chat, &plugin, &drive], &home);
        assert!(r.roots.is_empty(), "{:?}", r.roots);
        let why: Vec<Skip> = r.skipped.iter().map(|(_, s)| *s).collect();
        assert_eq!(why, vec![Skip::Broad, Skip::Broad, Skip::NotAProject, Skip::Broad, Skip::Broad]);
    }

    #[test]
    fn a_project_in_the_home_folder_is_kept() {
        let td = tempfile::tempdir().unwrap();
        let home = td.path().join("home");
        let proj = home.join("dev").join("tool");
        marker(&proj, "Cargo.toml");
        assert_eq!(keys(&roots(&[&proj], &home).roots), vec![path_key(&proj)]);
    }

    #[test]
    fn cloud_folders_are_skipped_without_touching_them() {
        let td = tempfile::tempdir().unwrap();
        let r = roots(
            &[
                Path::new(r"G:\Drive condivisi\Amministrazione\Documenti"),
                Path::new(r"H:\Il mio Drive\Contabilità"),
                &td.path().join("OneDrive - Acme").join("repo"),
            ],
            td.path(),
        );
        assert!(r.roots.is_empty());
        assert!(r.skipped.iter().all(|(_, s)| *s == Skip::Cloud), "{:?}", r.skipped);
    }

    #[test]
    fn missing_folders_and_duplicates() {
        let td = tempfile::tempdir().unwrap();
        let proj = td.path().join("work").join("x");
        marker(&proj, "Cargo.toml");
        let upper = PathBuf::from(proj.to_string_lossy().to_uppercase() + "\\");
        let gone = td.path().join("gone");
        let r = roots(&[&proj, &upper, &gone, &gone], td.path());
        assert_eq!(r.roots.len(), 1);
        assert_eq!(r.skipped, vec![(normalize_path(&gone), Skip::Missing)]);
    }
}
