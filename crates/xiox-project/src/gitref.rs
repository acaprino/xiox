//! Whether git may be asked about a repository that is not the project. Git reads beyond the
//! repository's own folder when its `.git` says so: the `gitdir:` of a `.git` file, the
//! `commondir` of a linked worktree, the object `alternates` (which can borrow in turn), the
//! configuration files it includes (which can include in turn) and the places its config names
//! (`core.worktree`, `core.excludesFile`, `core.attributesFile`). `git status` also enters every
//! repository inside the working tree (a populated submodule). Each place must pass the policy
//! the session folders pass, local fixed drives and no link from the top down to it, before
//! anything in it is opened. Only files are read here, never git.

use std::fs;
use std::path::{Component, Path, PathBuf};

use xiox_scanner::is_cloud_dir;

use crate::history::scope::{local_dir, unlinked_chain, FixedDrives};
use crate::history::Exclusion;
use crate::nested::nested_repos;
use crate::search::is_link;

/// How deep git follows includes, and alternates that borrow from alternates (git's own caps).
const MAX_INCLUDE_DEPTH: usize = 10;
const MAX_ALTERNATE_DEPTH: usize = 5;
/// What git opens inside a git folder and inside an object store: none of them may be a link.
const GIT_FOLDER_ENTRIES: [&str; 7] = ["objects", "refs", "index", "HEAD", "packed-refs", "info", "modules"];
const OBJECT_STORE_ENTRIES: [&str; 2] = ["info", "pack"];

/// Every place git would read for the repository at `dir` itself is allowed (what `rev-list`,
/// `for-each-ref` and `config` read). What cannot be validated is not asked: a `.git` that is
/// a link, a `.git` file without `gitdir:`, a missing git folder, an unreadable file, a config
/// line continued on the next one. `home` is what `~/` stands for, as [`git_home`] gives it.
pub(crate) fn git_may_read(dir: &Path, home: Option<&Path>) -> bool {
    Gate { drives: FixedDrives::default(), home }.repository(dir).is_some()
}

/// [`git_may_read`] for `dir` and every repository inside it, at any depth: what `git status`
/// enters. `false` too when the folder cannot be looked through to the end.
pub(crate) fn git_may_read_tree(dir: &Path, home: Option<&Path>) -> bool {
    let gate = Gate { drives: FixedDrives::default(), home };
    let mut todo = vec![dir.to_path_buf()];
    while let Some(repo) = todo.pop() {
        if gate.repository(&repo).is_none() {
            return false;
        }
        match nested_repos(&repo) {
            Ok(inner) => todo.extend(inner),
            Err(_) => return false,
        }
    }
    true
}

/// The home git expands `~/` to: `HOME`, else `HOMEDRIVE` and `HOMEPATH`, else `USERPROFILE`.
pub(crate) fn git_home() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    var("HOME")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(var("HOMEDRIVE")?).join(var("HOMEPATH")?)))
        .or_else(|| var("USERPROFILE").map(PathBuf::from))
}

/// Checks each place before it opens anything in it; `None` at the first one refused.
struct Gate<'a> {
    drives: FixedDrives,
    home: Option<&'a Path>,
}

impl Gate<'_> {
    fn repository(&self, dir: &Path) -> Option<()> {
        self.place(dir)?;
        let dot_git = dir.join(".git");
        let meta = dot_git.symlink_metadata().ok()?;
        let gitdir = if is_link(&meta) {
            return None;
        } else if meta.is_dir() {
            dot_git
        } else {
            let text = self.read(&dot_git)??;
            resolve(dir, strip_bom(&text).lines().next()?.strip_prefix("gitdir:")?.trim())
        };
        self.git_folder(&gitdir)?;
        let common = match self.read(&gitdir.join("commondir"))? {
            Some(text) => resolve(&gitdir, text.lines().next()?.trim()),
            None => gitdir.clone(),
        };
        if common != gitdir {
            self.git_folder(&common)?;
        }
        self.object_store(&common.join("objects"), 0)?;
        for config in [common.join("config"), gitdir.join("config.worktree")] {
            self.config(&config, dir, &gitdir, 0)?;
        }
        Some(())
    }

    fn git_folder(&self, gitdir: &Path) -> Option<()> {
        self.place(gitdir)?;
        GIT_FOLDER_ENTRIES.iter().all(|e| not_a_link(&gitdir.join(e))).then_some(())
    }

    fn object_store(&self, objects: &Path, depth: usize) -> Option<()> {
        (depth <= MAX_ALTERNATE_DEPTH).then_some(())?;
        self.place(objects)?;
        OBJECT_STORE_ENTRIES.iter().all(|e| not_a_link(&objects.join(e))).then_some(())?;
        if let Some(text) = self.read(&objects.join("info").join("alternates"))? {
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                self.object_store(&resolve(objects, line), depth + 1)?;
            }
        }
        Some(())
    }

    /// A config file and what it makes git read. A missing one is fine: git skips it too.
    fn config(&self, file: &Path, repo: &Path, gitdir: &Path, depth: usize) -> Option<()> {
        (depth <= MAX_INCLUDE_DEPTH).then_some(())?;
        let Some(text) = self.read(file)? else { return Some(()) };
        for (section, key, value) in entries(&text)? {
            match (section.as_str(), key.as_str()) {
                ("include" | "includeif", "path") => self.config(&self.path(file.parent()?, &value)?, repo, gitdir, depth + 1)?,
                // git takes a relative `core.worktree` from the git folder, the other two from
                // where it runs: the repository.
                ("core", "worktree") => self.place(&self.path(gitdir, &value)?)?,
                ("core", "excludesfile" | "attributesfile") => self.place(&self.path(repo, &value)?)?,
                _ => {}
            }
        }
        Some(())
    }

    /// `path` is a local absolute place on a fixed drive, reached from the top without a link,
    /// in no cloud folder. A missing place is refused.
    fn place(&self, path: &Path) -> Option<()> {
        let local = self.local(path)?;
        unlinked_chain(&local).ok()?;
        (!local.ancestors().any(is_cloud_dir)).then_some(())
    }

    /// The text of the file at `path`: `Some(None)` when there is none, `None` when it is refused
    /// or cannot be read. The file itself is never followed when it is a link.
    fn read(&self, path: &Path) -> Option<Option<String>> {
        let local = self.local(path)?;
        match unlinked_chain(&local) {
            Ok(()) => {}
            Err(Exclusion::Missing) => return Some(None),
            Err(_) => return None,
        }
        (!local.ancestors().any(is_cloud_dir)).then_some(())?;
        fs::read_to_string(&local).ok().map(Some)
    }

    /// The text alone says it is a local absolute path on a fixed drive.
    fn local(&self, path: &Path) -> Option<PathBuf> {
        let local = local_dir(&path.to_string_lossy()).ok()?;
        self.drives.holds(&local).then_some(local)
    }

    /// A path git reads from a config value: `~/` from the home, else as [`resolve`] says.
    fn path(&self, base: &Path, value: &str) -> Option<PathBuf> {
        match value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) {
            Some(rest) => Some(self.home?.join(rest)),
            None => Some(resolve(base, value)),
        }
    }
}

/// Not a link, or not there.
fn not_a_link(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(meta) => !is_link(&meta),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// `raw` as git reads it: absolute as written, else relative to `base`; `.` and `..` folded
/// without touching the disk, as Windows folds them.
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

/// The `(section, key, value)` entries of a git config file, section and key in lower case and
/// the subsection left out, a key on the section's own line included. `None` for a value
/// continued on the next line, which this reader does not follow.
fn entries(text: &str) -> Option<Vec<(String, String, String)>> {
    let mut section = String::new();
    let mut out = Vec::new();
    for raw in strip_bom(text).lines() {
        let mut line = raw.trim();
        if line.ends_with('\\') {
            return None;
        }
        if let Some(header) = line.strip_prefix('[') {
            let close = header.find(']')?;
            section = header[..close].split(|c: char| c == '"' || c.is_whitespace()).next()?.to_ascii_lowercase();
            line = header[close + 1..].trim();
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let (key, value) = line.split_once('=').unwrap_or((line, ""));
        out.push((section.clone(), key.trim().to_ascii_lowercase(), value.trim().trim_matches('"').to_string()));
    }
    Some(out)
}
