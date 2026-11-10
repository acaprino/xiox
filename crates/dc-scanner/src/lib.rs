//! Project/artifact discovery and sizing (`ProjectScanner`).
//!
//! * Filesystem mode: one parallel walk finds every directory holding a marker file (see
//!   [`markers`]), records the artifact dirs it implies, and accumulates the newest source
//!   mtime of the nearest enclosing project. Unlike kondo it never stops at the first project:
//!   nested and mixed projects each get their own `ProjectCandidate`.
//! * Index mode: artifact dirs come from `DirIndex::find_dirs_named`, the marker/identity is
//!   validated on disk and sizes are taken from the index (no size walk).
//!
//! Links, junctions and other reparse points are never followed. Hardlinks are not
//! deduplicated in `allocated_bytes` (needs a file handle per file); allocated size is the
//! logical size rounded up to 4 KiB clusters, or the compressed size for NTFS-compressed or
//! sparse files.

mod fsutil;
mod markers;
pub mod sessions;

pub use sessions::{MAX_WORKSPACE_CHILDREN, SessionRoots, Skip, project_roots_from_sessions};

use dc_model::*;
use fsutil::*;
use markers::{Detected, NEVER_ENTER, artifact_names, detect};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct DefaultScanner;

const NO_MTIME: i64 = i64::MIN;

/// One project root (a directory holding at least one marker file).
struct Owner {
    root: PathBuf,
    ecosystems: Vec<&'static str>,
    /// Lowercase artifact dir names of this project; never entered while walking its sources.
    skip_names: Vec<String>,
    mtime: AtomicI64,
}

impl Owner {
    fn new(root: PathBuf, det: &Detected) -> Arc<Owner> {
        Arc::new(Owner {
            root,
            ecosystems: det.ecosystems.clone(),
            skip_names: det.skip_names(),
            mtime: AtomicI64::new(NO_MTIME),
        })
    }
    fn bump(&self, ms: Option<i64>) {
        if let Some(m) = ms {
            self.mtime.fetch_max(m, Ordering::Relaxed);
        }
    }
}

struct Found {
    owner: Arc<Owner>,
    path: PathBuf,
    kind: ArtifactKind,
    label: &'static str,
    verified: bool,
    size: Option<SizeInfo>,
}

struct Ctx<'a> {
    opts: &'a ScanOptions,
    ignore: HashSet<String>,
    known_names: Vec<String>,
    found: Mutex<Vec<Found>>,
    owners: Mutex<Vec<Arc<Owner>>>,
    warnings: Mutex<Vec<String>>,
}

impl<'a> Ctx<'a> {
    fn new(opts: &'a ScanOptions) -> Self {
        Ctx {
            opts,
            ignore: ignore_set(&opts.ignore),
            known_names: artifact_names().iter().map(|n| n.to_ascii_lowercase()).collect(),
            found: Mutex::new(Vec::new()),
            owners: Mutex::new(Vec::new()),
            warnings: Mutex::new(Vec::new()),
        }
    }
    fn warn(&self, msg: String) {
        self.warnings.lock().unwrap().push(msg);
    }
    fn is_ignored(&self, p: &Path) -> bool {
        !self.ignore.is_empty() && self.ignore.contains(&path_key(p))
    }

    /// May the walker enter directory `name` at `path`?
    fn enter_ok(&self, name: &str, path: &Path, parent_is_root: bool, skip_names: &[String]) -> bool {
        let lname = name.to_ascii_lowercase();
        if NEVER_ENTER.contains(&lname.as_str()) || skip_names.contains(&lname) {
            return false;
        }
        if !self.opts.include_dot_dirs && name.starts_with('.') {
            return false;
        }
        if is_system_dir(name, parent_is_root, || path_key(path)) || self.is_ignored(path) {
            return false;
        }
        // Generated output we can positively identify (a venv, a cargo target, ...) holds
        // thousands of directories and never a project.
        if self.known_names.contains(&lname) && has_any_identity(path) {
            return false;
        }
        true
    }
}

fn project_name(root: &Path) -> String {
    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
}

// ───────────────────────────── filesystem walk ─────────────────────────────

fn walk(ctx: &Ctx, dir: &Path, depth: usize, inherited: Option<Arc<Owner>>) {
    let listing = match list_dir(dir) {
        Ok(l) => l,
        Err(e) => {
            ctx.warn(format!("cannot read {}: {e}", dir.display()));
            return;
        }
    };
    let mut owner = inherited;
    let mut consumed = vec![false; listing.dirs.len()];

    if let Some(det) = detect(listing.files.iter().map(|f| f.name.as_str())) {
        let o = Owner::new(dir.to_path_buf(), &det);
        for (i, sd) in listing.dirs.iter().enumerate() {
            let Some((spec, verified)) = det.resolve(&sd.name, |id| identity_ok(id, &sd.path)) else { continue };
            consumed[i] = true; // artifact dirs are never descended into
            if ctx.is_ignored(&sd.path) {
                continue;
            }
            ctx.found.lock().unwrap().push(Found {
                owner: o.clone(),
                path: sd.path.clone(),
                kind: spec.kind,
                label: spec.label,
                verified,
                size: None,
            });
        }
        ctx.owners.lock().unwrap().push(o.clone());
        owner = Some(o);
    }

    if let Some(o) = &owner {
        for f in &listing.files {
            o.bump(f.mtime_ms);
        }
    }

    let skip: &[String] = owner.as_ref().map(|o| o.skip_names.as_slice()).unwrap_or(&[]);
    let parent_is_root = dir.parent().is_none();
    let children: Vec<&SubDir> = listing
        .dirs
        .iter()
        .zip(&consumed)
        .filter(|(sd, c)| !**c && ctx.enter_ok(&sd.name, &sd.path, parent_is_root, skip))
        .map(|(sd, _)| sd)
        .collect();

    if ctx.opts.max_depth.is_some_and(|m| depth >= m) {
        // Depth budget for discovery is spent; still finish the owner's source mtime.
        if let Some(o) = &owner {
            mtime_only(ctx, o, children.iter().map(|sd| sd.path.clone()).collect(), false);
        }
        return;
    }
    children.par_iter().for_each(|sd| walk(ctx, &sd.path, depth + 1, owner.clone()));
}

/// Newest source mtime below `starts` for `owner`, stopping at nested project roots.
/// `starts_are_root`: the single start is the project root itself (do not treat it as nested).
fn mtime_only(ctx: &Ctx, owner: &Owner, starts: Vec<PathBuf>, starts_are_root: bool) {
    let mut stack: Vec<(PathBuf, bool)> = starts.into_iter().map(|p| (p, starts_are_root)).collect();
    while let Some((dir, is_root)) = stack.pop() {
        let Ok(listing) = list_dir(&dir) else { continue };
        if !is_root && detect(listing.files.iter().map(|f| f.name.as_str())).is_some() {
            continue; // nested project root: it has its own candidate
        }
        for f in &listing.files {
            owner.bump(f.mtime_ms);
        }
        let parent_is_root = dir.parent().is_none();
        for sd in &listing.dirs {
            if ctx.enter_ok(&sd.name, &sd.path, parent_is_root, &owner.skip_names) {
                stack.push((sd.path.clone(), false));
            }
        }
    }
}

// ───────────────────────────── index mode ─────────────────────────────

/// Is `p_key` equal to or below `root_key` (both `path_key` strings)?
fn inside(root_key: &str, p_key: &str) -> bool {
    if p_key == root_key {
        return true;
    }
    let sep = std::path::MAIN_SEPARATOR;
    if root_key.ends_with(sep) { p_key.starts_with(root_key) } else { p_key.strip_prefix(root_key).is_some_and(|r| r.starts_with(sep)) }
}

struct IdxEntry {
    path: PathBuf,
    size: SizeInfo,
}

fn discover_indexed(ctx: &Ctx, ix: &dyn DirIndex, roots: &[PathBuf]) {
    let names = artifact_names();
    let root_keys: Vec<String> = roots.iter().map(|r| path_key(r)).collect();
    let mut groups: HashMap<String, (PathBuf, Vec<IdxEntry>)> = HashMap::new();

    'dirs: for d in ix.find_dirs_named(&names, roots) {
        let path = normalize_path(&d.path);
        let key = path_key(&path);
        let Some(ri) = root_keys.iter().position(|rk| key != *rk && inside(rk, &key)) else { continue };
        let Some(parent) = path.parent() else { continue };
        if ctx.is_ignored(&path) {
            continue;
        }
        // Every directory between the scan root (exclusive) and the artifact (exclusive,
        // parent included) must be one the walker would have entered.
        let mut depth = 0usize;
        for a in path.ancestors().skip(1) {
            if path_key(a).len() <= root_keys[ri].len() {
                break;
            }
            let Some(an) = a.file_name().map(|n| n.to_string_lossy().into_owned()) else { break };
            let a_parent_is_root = a.parent().is_some_and(|p| p.parent().is_none());
            if !ctx.enter_ok(&an, a, a_parent_is_root, &[]) {
                continue 'dirs;
            }
            depth += 1;
        }
        if ctx.opts.max_depth.is_some_and(|m| depth > m) {
            continue;
        }
        let size = if d.size == SizeInfo::default() { ix.size_of(&path).unwrap_or(d.size) } else { d.size };
        groups
            .entry(path_key(parent))
            .or_insert_with(|| (parent.to_path_buf(), Vec::new()))
            .1
            .push(IdxEntry { path, size });
    }

    let groups: Vec<(PathBuf, Vec<IdxEntry>)> = groups.into_values().collect();
    groups.par_iter().for_each(|(parent, entries)| {
        let Ok(listing) = list_dir(parent) else {
            ctx.warn(format!("index entry parent not readable: {}", parent.display()));
            return;
        };
        let Some(det) = detect(listing.files.iter().map(|f| f.name.as_str())) else { return };
        let owner = Owner::new(parent.clone(), &det);
        let mut any = false;
        for e in entries {
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // Must exist on disk as a real (non-link) directory.
            let Some(sd) = listing.dirs.iter().find(|sd| sd.name.eq_ignore_ascii_case(&name)) else {
                ctx.warn(format!("index entry not found on disk (stale index?): {}", e.path.display()));
                continue;
            };
            let Some((spec, verified)) = det.resolve(&sd.name, |id| identity_ok(id, &sd.path)) else { continue };
            any = true;
            ctx.found.lock().unwrap().push(Found {
                owner: owner.clone(),
                path: sd.path.clone(),
                kind: spec.kind,
                label: spec.label,
                verified,
                size: Some(e.size.clone()),
            });
        }
        if any {
            mtime_only(ctx, &owner, vec![parent.clone()], true);
            ctx.owners.lock().unwrap().push(owner);
        }
    });
}

// ───────────────────────────── scanner ─────────────────────────────

/// Normalize, drop duplicates and roots nested inside another root.
fn dedup_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<(String, PathBuf)> = roots.iter().map(|r| normalize_path(r)).map(|p| (path_key(&p), p)).collect();
    v.sort_by_key(|(k, _)| k.len());
    let mut kept: Vec<(String, PathBuf)> = Vec::new();
    for (k, p) in v {
        if !kept.iter().any(|(kk, _)| inside(kk, &k)) {
            kept.push((k, p));
        }
    }
    kept.into_iter().map(|(_, p)| p).collect()
}

impl ProjectScanner for DefaultScanner {
    fn scan(&self, roots: &[PathBuf], opts: &ScanOptions, index: Option<&dyn DirIndex>) -> Loaded<ProjectCandidate> {
        let ctx = Ctx::new(opts);
        let roots = dedup_roots(roots);

        let mut live_roots = Vec::new();
        for r in &roots {
            if ctx.is_ignored(r) {
                continue;
            }
            if r.is_dir() {
                live_roots.push(r.clone());
            } else {
                ctx.warn(format!("scan root not found or not a directory: {}", r.display()));
            }
        }

        match index {
            Some(ix) => discover_indexed(&ctx, ix, &live_roots),
            None => live_roots.par_iter().for_each(|r| walk(&ctx, r, 0, None)),
        }

        // Sizes: one walk per artifact dir, in parallel (skipped when the index gave one).
        let mut found = std::mem::take(&mut *ctx.found.lock().unwrap());
        found.par_iter_mut().for_each(|f| {
            if f.size.is_none() {
                let (s, w) = dir_size(&f.path);
                f.size = Some(s);
                if !w.is_empty() {
                    ctx.warnings.lock().unwrap().extend(w);
                }
            }
        });

        let mut by_owner: HashMap<*const Owner, Vec<Artifact>> = HashMap::new();
        for f in found {
            let size = f.size.unwrap_or_default();
            if size.effective_bytes() < opts.min_artifact_bytes {
                continue;
            }
            by_owner.entry(Arc::as_ptr(&f.owner)).or_default().push(Artifact {
                path: f.path,
                kind: f.kind,
                label: f.label.to_string(),
                size,
                verified: f.verified,
            });
        }

        let owners = std::mem::take(&mut *ctx.owners.lock().unwrap());
        let mut seen_roots: HashSet<String> = HashSet::new();
        let mut items = Vec::new();
        for o in owners {
            let Some(mut artifacts) = by_owner.remove(&Arc::as_ptr(&o)) else { continue };
            if !seen_roots.insert(path_key(&o.root)) {
                continue;
            }
            artifacts.sort_by(|a, b| {
                b.size.effective_bytes().cmp(&a.size.effective_bytes()).then_with(|| a.path.cmp(&b.path))
            });
            let m = o.mtime.load(Ordering::Relaxed);
            items.push(ProjectCandidate {
                name: project_name(&o.root),
                ecosystems: o.ecosystems.iter().map(|s| s.to_string()).collect(),
                root: o.root.clone(),
                artifacts,
                source_mtime_ms: (m != NO_MTIME).then_some(m),
            });
        }
        let total = |c: &ProjectCandidate| c.artifacts.iter().map(|a| a.size.effective_bytes()).sum::<u64>();
        items.sort_by(|a, b| total(b).cmp(&total(a)).then_with(|| path_key(&a.root).cmp(&path_key(&b.root))));

        let mut warnings = ctx.warnings.into_inner().unwrap();
        warnings.sort();
        warnings.dedup();
        Loaded { items, warnings }
    }
}

#[cfg(test)]
mod tests;
