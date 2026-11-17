//! Whether git may be asked about a repository that is not the project. Git reads beyond the
//! repository's own folder when its `.git` says so: the `gitdir:` of a `.git` file, the
//! `commondir` of a linked worktree, the object `alternates` and the configuration files the
//! repository includes. Each of those places must pass the policy the session folders pass
//! (local fixed drives, no link on the way, no cloud folder) before git runs there. Only files
//! are read here, never git, and a file is read only inside a place already checked.

use std::fs;
use std::path::{Component, Path, PathBuf};

use xiox_scanner::is_cloud_dir;

use crate::history::scope::{local_dir, unlinked_chain, FixedDrives};
use crate::search::is_link;

/// Every place git would read for the repository at `dir` is allowed. A `.git` that is a link,
/// a `.git` file without `gitdir:`, a reference that cannot be read and a place that does not
/// exist give `false`: what cannot be validated is not asked. `home` is what `~/` stands for
/// in an include.
pub(crate) fn git_may_read(dir: &Path, home: Option<&Path>) -> bool {
    let drives = FixedDrives::default();
    let allowed = |p: &Path| allowed(p, &drives);
    checked(dir, home, &allowed).is_some()
}

/// `Some` when every reference is allowed; `None` at the first one that is not.
fn checked(dir: &Path, home: Option<&Path>, allowed: &dyn Fn(&Path) -> bool) -> Option<()> {
    let dot_git = dir.join(".git");
    let meta = dot_git.symlink_metadata().ok()?;
    let gitdir = if is_link(&meta) {
        return None;
    } else if meta.is_dir() {
        dot_git
    } else {
        let text = fs::read_to_string(&dot_git).ok()?;
        resolve(dir, text.lines().next()?.strip_prefix("gitdir:")?.trim())
    };
    allowed(&gitdir).then_some(())?;
    let common = match read_optional(&gitdir.join("commondir"))? {
        Some(text) => resolve(&gitdir, text.lines().next()?.trim()),
        None => gitdir.clone(),
    };
    allowed(&common).then_some(())?;
    let objects = common.join("objects");
    if let Some(text) = read_optional(&objects.join("info").join("alternates"))? {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            allowed(&resolve(&objects, line)).then_some(())?;
        }
    }
    for config in [common.join("config"), gitdir.join("config.worktree")] {
        let Some(text) = read_optional(&config)? else { continue };
        for path in include_paths(&text) {
            let target = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
                Some(rest) => home?.join(rest),
                None => resolve(config.parent()?, &path),
            };
            allowed(&target).then_some(())?;
        }
    }
    Some(())
}

/// A local absolute folder or file on a fixed drive, reached from the top without a link, in no
/// cloud folder.
fn allowed(path: &Path, drives: &FixedDrives) -> bool {
    let Ok(local) = local_dir(&path.to_string_lossy()) else { return false };
    drives.holds(&local) && unlinked_chain(&local).is_ok() && !local.ancestors().any(is_cloud_dir)
}

/// `raw` as git reads it: absolute as written, else relative to `base`; `.` and `..` folded
/// without touching the disk.
fn resolve(base: &Path, raw: &str) -> PathBuf {
    let raw = Path::new(raw);
    let joined = if raw.has_root() || raw.is_absolute() { raw.to_path_buf() } else { base.join(raw) };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `Some(None)`: there is no such file; `None`: it is there and cannot be read.
fn read_optional(path: &Path) -> Option<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Some(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

/// The `path` values of the `[include]` and `[includeIf …]` sections of a git config file.
fn include_paths(config: &str) -> Vec<String> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in config.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let name = header.split(|c: char| c == ']' || c == '"' || c.is_whitespace()).next().unwrap_or_default();
            inside = name.eq_ignore_ascii_case("include") || name.eq_ignore_ascii_case("includeif");
        } else if inside
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("path")
        {
            out.push(value.trim().trim_matches('"').to_string());
        }
    }
    out
}
