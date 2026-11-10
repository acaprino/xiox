//! Pre-flight validation of a `PlanItem`. Every check fails closed: when in doubt, refuse.

use crate::fsops::verbatim;
use dc_model::{ArtifactKind, PlanItem, normalize_path, path_key};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

pub struct Checked {
    /// Normalized (no `\\?\`) absolute path.
    pub path: PathBuf,
    /// The item path itself is a symlink / junction: only the link may be removed.
    pub is_link: bool,
}

/// First path component below the drive that marks a system area, on any drive.
fn system_first_component(first: &str) -> bool {
    matches!(first, "windows" | "programdata" | "system volume information" | "$recycle.bin" | "recovery")
        || first.starts_with("program files")
}

/// Names that are never artifacts, whatever the plan says.
const DENY_NAMES: &[&str] = &[
    ".git", ".ssh", ".gnupg", ".claude", ".codex", ".config", "appdata", "desktop", "documents", "downloads",
    "music", "pictures", "videos", "onedrive", "users", "windows", "src", "system32", "programdata",
];

fn name_plausible(kind: ArtifactKind, name: &str) -> bool {
    match kind {
        ArtifactKind::RustTarget => name == "target",
        ArtifactKind::NodeModules => name == "node_modules",
        ArtifactKind::PythonVenv => matches!(name, ".venv" | "venv" | "env" | ".env") || name.contains("venv"),
        ArtifactKind::PythonCache => {
            matches!(name, "__pycache__" | ".tox" | ".nox" | ".hypothesis" | ".ipynb_checkpoints")
                || name.ends_with("_cache")
        }
        ArtifactKind::Gradle => matches!(name, ".gradle" | "build"),
        ArtifactKind::Maven => name == "target",
        ArtifactKind::DotNet => matches!(name, "bin" | "obj"),
        ArtifactKind::CMake => {
            name.contains("build") || matches!(name, "out" | "cmakefiles" | ".cmake" | "_deps")
        }
        // Whole worktree directories can have any name (the deny list above still applies).
        ArtifactKind::WorktreeDir => true,
        // `Other` has no fixed name: only the deny list protects it.
        ArtifactKind::Other => true,
    }
}

fn is_under(key: &str, root_key: &str) -> bool {
    key == root_key || key.strip_prefix(root_key).is_some_and(|rest| rest.starts_with('\\'))
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn protected_reason(path: &Path) -> Option<String> {
    let key = path_key(path);

    for var in ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"] {
        if let Some(root) = env_path(var)
            && is_under(&key, &path_key(&root))
        {
            return Some(format!("protected system location ({var})"));
        }
    }

    let mut profiles: Vec<PathBuf> = ["USERPROFILE", "HOME"].iter().filter_map(|v| env_path(v)).collect();
    if let (Some(d), Some(p)) = (env_path("HOMEDRIVE"), env_path("HOMEPATH")) {
        profiles.push(PathBuf::from(format!("{}{}", d.display(), p.display())));
    }
    for profile in profiles {
        let pk = path_key(&profile);
        if key == pk {
            return Some("path is the user profile root".into());
        }
        if is_under(&pk, &key) {
            return Some("path is an ancestor of the user profile".into());
        }
    }
    None
}

/// `Some(reason)` if the path breaks a purely lexical rule (no filesystem access).
fn lexical_refusal(path: &Path, kind: ArtifactKind) -> Option<String> {
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::UNC(..)) => {}
        _ => return Some("path has no drive letter or UNC share".into()),
    }
    if !matches!(comps.next(), Some(Component::RootDir)) {
        return Some("path is not rooted".into());
    }
    let mut normals: Vec<String> = Vec::new();
    for c in comps {
        match c {
            Component::Normal(s) => normals.push(s.to_string_lossy().to_lowercase()),
            _ => return Some("path contains '.' or '..' components".into()),
        }
    }
    if normals.len() < 2 {
        return Some(format!(
            "path too shallow ({} component(s) below the drive root): refusing drive roots and top-level folders",
            normals.len()
        ));
    }
    if system_first_component(&normals[0]) {
        return Some(format!("protected system location (\\{})", normals[0]));
    }
    if normals[0] == "users" && normals.len() <= 2 {
        return Some("path is a user profile root or the Users folder".into());
    }
    let name = normals.last().expect("len >= 2");
    if DENY_NAMES.contains(&name.as_str()) {
        return Some(format!("'{name}' is never a removable artifact"));
    }
    if !name_plausible(kind, name) {
        return Some(format!("folder name '{name}' does not look like a {kind:?} artifact"));
    }
    protected_reason(path)
}

/// Validate `item`; `Err(reason)` means the item must be reported as `Skipped`.
pub fn validate(item: &PlanItem) -> Result<Checked, String> {
    if !item.path.is_absolute() {
        return Err("path is not absolute".into());
    }
    let path = normalize_path(&item.path);
    if let Some(r) = lexical_refusal(&path, item.kind) {
        return Err(r);
    }

    let md = match fs::symlink_metadata(verbatim(&path)) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err("path does not exist".into()),
        Err(e) => return Err(format!("cannot access path: {e}")),
    };
    let is_link = md.file_type().is_symlink();
    if !is_link && !md.is_dir() {
        return Err("path is not a directory".into());
    }

    // Resolve links in the *parent* chain (e.g. D:\proj being a junction into C:\Windows)
    // and re-run the protected-location rules on the real location.
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(real_parent) = fs::canonicalize(verbatim(parent))
    {
        let real = normalize_path(&real_parent).join(name);
        if path_key(&real) != path_key(&path)
            && let Some(r) = lexical_refusal(&real, item.kind)
        {
            return Err(format!("resolves to {}: {r}", real.display()));
        }
    }
    Ok(Checked { path, is_link })
}
