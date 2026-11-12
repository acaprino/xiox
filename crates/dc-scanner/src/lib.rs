//! Project/artifact discovery and sizing (`ProjectScanner`).
//!
//! * Filesystem mode: one parallel walk finds every directory holding a marker file (see
//!   [`markers`]), records the artifact dirs it implies, and accumulates the newest source
//!   mtime of the nearest enclosing project. Unlike kondo it never stops at the first project:
//!   nested and mixed projects each get their own `ProjectCandidate`.
//! * Index mode: artifact dirs come from `DirIndex::find_dirs_named` (names, prefix / suffix
//!   patterns and `meson-private`), the marker/identity is validated on disk and sizes are
//!   taken from the index (no size walk).
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
use markers::{ArtifactSpec, Detected, NEVER_ENTER, TABLE, Table, join_rel, real_subdir};
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
    /// `path_key`s of the artifacts found for this project (sub-paths, prefixes and Meson build
    /// dirs have no fixed name): never entered either.
    skip_paths: Vec<String>,
    mtime: AtomicI64,
}

impl Owner {
    fn new(root: PathBuf, det: &Detected, skip_paths: Vec<String>) -> Arc<Owner> {
        Arc::new(Owner {
            root,
            ecosystems: det.ecosystems.clone(),
            skip_names: det.skip_names(),
            skip_paths,
            mtime: AtomicI64::new(NO_MTIME),
        })
    }
    fn bump(&self, ms: Option<i64>) {
        if let Some(m) = ms {
            self.mtime.fetch_max(m, Ordering::Relaxed);
        }
    }
    /// Is `path` (named `lname`, lowercase) one of this project's artifacts?
    fn skips(&self, lname: &str, path: &Path) -> bool {
        self.skip_names.iter().any(|n| n == lname)
            || (!self.skip_paths.is_empty() && self.skip_paths.contains(&path_key(path)))
    }
}

struct Found {
    owner: Arc<Owner>,
    path: PathBuf,
    kind: ArtifactKind,
    label: &'static str,
    class: Class,
    verified: bool,
    ambiguous: bool,
    hint: &'static str,
    size: Option<SizeInfo>,
}

/// An artifact folder of one project, before its `Owner` exists.
struct Art {
    path: PathBuf,
    spec: &'static ArtifactSpec,
    verified: bool,
    ambiguous: bool,
    size: Option<SizeInfo>,
}

impl Art {
    fn new(path: PathBuf, r: (&'static ArtifactSpec, bool, bool), size: Option<SizeInfo>) -> Art {
        Art { path, spec: r.0, verified: r.1, ambiguous: r.2, size }
    }
}

/// File and directory names of a listing.
fn names(l: &Listing) -> (Vec<&str>, Vec<&str>) {
    (l.files.iter().map(|f| f.name.as_str()).collect(), l.dirs.iter().map(|d| d.name.as_str()).collect())
}

fn has_name(names: &[&str], n: &str) -> bool {
    names.iter().any(|x| x.eq_ignore_ascii_case(n))
}

struct Ctx<'a> {
    table: &'static Table,
    opts: &'a ScanOptions,
    ignore: HashSet<String>,
    known_names: Vec<String>,
    found: Mutex<Vec<Found>>,
    owners: Mutex<Vec<Arc<Owner>>>,
    warnings: Mutex<Vec<String>>,
}

impl<'a> Ctx<'a> {
    fn new(table: &'static Table, opts: &'a ScanOptions) -> Self {
        Ctx {
            table,
            opts,
            ignore: ignore_set(&opts.ignore),
            known_names: table.plain_names().iter().map(|n| n.to_ascii_lowercase()).collect(),
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

    /// May the walker enter directory `name` at `path`? `owner`: the enclosing project, whose
    /// artifacts are never entered.
    fn enter_ok(&self, name: &str, path: &Path, parent_is_root: bool, owner: Option<&Owner>) -> bool {
        let lname = name.to_ascii_lowercase();
        if NEVER_ENTER.contains(&lname.as_str()) || owner.is_some_and(|o| o.skips(&lname, path)) {
            return false;
        }
        if !self.opts.include_dot_dirs && name.starts_with('.') {
            return false;
        }
        if is_system_dir(name, parent_is_root) || self.is_ignored(path) {
            return false;
        }
        // Generated output we can positively identify (a venv, a cargo target, ...) holds
        // thousands of directories and never a project.
        if self.known_names.contains(&lname) && has_any_identity(path) {
            return false;
        }
        true
    }

    /// Record the artifacts of the project at `root` (ignored ones are dropped, but still never
    /// entered) and return its owner. An artifact inside another one of the same project
    /// (`vendor/bundle` inside `vendor`) is dropped: its bytes are already counted, and it would
    /// be gone once the outer one is removed.
    fn add_project(&self, root: PathBuf, det: &Detected, arts: Vec<Art>) -> Arc<Owner> {
        let keys: Vec<String> = arts.iter().map(|a| path_key(&a.path)).collect();
        let o = Owner::new(root, det, keys.clone());
        let mut found = self.found.lock().unwrap();
        for (i, a) in arts.into_iter().enumerate() {
            let nested = keys.iter().enumerate().any(|(j, k)| j != i && *k != keys[i] && inside(k, &keys[i]));
            if nested || self.is_ignored(&a.path) {
                continue;
            }
            found.push(Found {
                owner: o.clone(),
                path: a.path,
                kind: a.spec.kind,
                label: a.spec.label,
                class: a.spec.class,
                verified: a.verified,
                ambiguous: a.ambiguous,
                hint: a.spec.hint,
                size: a.size,
            });
        }
        o
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

    let (files, dirs) = names(&listing);
    if let Some(det) = ctx.table.detect(dir, &files, &dirs) {
        let mut arts = Vec::new();
        // Direct children: names, prefixes and Meson build dirs.
        for (i, sd) in listing.dirs.iter().enumerate() {
            let Some(r) = det.resolve(&sd.name, |id| identity_ok(id, &sd.path)) else { continue };
            consumed[i] = true; // artifact dirs are never descended into
            arts.push(Art::new(sd.path.clone(), r, None));
        }
        // Sub-paths (`.nx/cache`): probed only when their first segment is listed here.
        for s in det.multi_specs() {
            if !has_name(&dirs, s.dir.split('/').next().unwrap_or_default()) {
                continue;
            }
            let Some(p) = real_subdir(dir, s.dir) else { continue };
            let k = path_key(&p);
            if arts.iter().any(|a| path_key(&a.path) == k) {
                continue;
            }
            let Some(r) = det.resolve(s.dir, |id| identity_ok(id, &p)) else { continue };
            arts.push(Art::new(p, r, None));
        }
        let o = ctx.add_project(dir.to_path_buf(), &det, arts);
        ctx.owners.lock().unwrap().push(o.clone());
        owner = Some(o);
    }

    if let Some(o) = &owner {
        for f in &listing.files {
            o.bump(f.mtime_ms);
        }
    }

    let parent_is_root = dir.parent().is_none();
    let children: Vec<&SubDir> = listing
        .dirs
        .iter()
        .zip(&consumed)
        .filter(|(sd, c)| !**c && ctx.enter_ok(&sd.name, &sd.path, parent_is_root, owner.as_deref()))
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
        let (files, dirs) = names(&listing);
        if !is_root && ctx.table.detect(&dir, &files, &dirs).is_some() {
            continue; // nested project root: it has its own candidate
        }
        for f in &listing.files {
            owner.bump(f.mtime_ms);
        }
        let parent_is_root = dir.parent().is_none();
        for sd in &listing.dirs {
            if ctx.enter_ok(&sd.name, &sd.path, parent_is_root, Some(owner)) {
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
    /// Relative to the project, `/`-separated (`target`, `.nx/cache`, a Meson build dir).
    rel: String,
    size: Option<SizeInfo>,
}

fn discover_indexed(ctx: &Ctx, ix: &dyn DirIndex, roots: &[PathBuf]) {
    let search = ctx.table.index_names();
    let root_keys: Vec<String> = roots.iter().map(|r| path_key(r)).collect();
    // Hits grouped by project root. A hit's project is found lexically (for a k-segment entry
    // the folder k levels up, intermediate segments matching; for `meson-private` the
    // grandparent) before anything is read from disk.
    let mut groups: HashMap<String, (PathBuf, Vec<IdxEntry>)> = HashMap::new();

    for d in ix.find_dirs_named(&search, roots) {
        let hit = normalize_path(&d.path);
        let hit_key = path_key(&hit);
        let Some(ri) = root_keys.iter().position(|rk| hit_key != *rk && inside(rk, &hit_key)) else { continue };
        'cands: for (project, rel) in ctx.table.index_candidates(&hit) {
            let pkey = path_key(project);
            if !inside(&root_keys[ri], &pkey) {
                continue;
            }
            let art = join_rel(project, &rel);
            if ctx.is_ignored(&art) {
                continue;
            }
            // Every directory between the scan root (exclusive) and the project (inclusive)
            // must be one the walker would have entered.
            let mut depth = 0usize;
            for a in project.ancestors() {
                if path_key(a).len() <= root_keys[ri].len() {
                    break;
                }
                let Some(an) = a.file_name().map(|n| n.to_string_lossy().into_owned()) else { break };
                let a_parent_is_root = a.parent().is_some_and(|p| p.parent().is_none());
                if !ctx.enter_ok(&an, a, a_parent_is_root, None) {
                    continue 'cands;
                }
                depth += 1;
            }
            if ctx.opts.max_depth.is_some_and(|m| depth > m) {
                continue;
            }
            let size = if path_key(&art) != hit_key {
                ix.size_of(&art) // Meson: the hit is `meson-private`, the artifact its parent
            } else if d.size == SizeInfo::default() {
                Some(ix.size_of(&hit).unwrap_or(d.size.clone()))
            } else {
                Some(d.size.clone())
            };
            groups.entry(pkey).or_insert_with(|| (project.to_path_buf(), Vec::new())).1.push(IdxEntry { rel, size });
        }
    }

    let groups: Vec<(PathBuf, Vec<IdxEntry>)> = groups.into_values().collect();
    groups.par_iter().for_each(|(project, entries)| {
        let Ok(listing) = list_dir(project) else {
            ctx.warn(format!("index entry parent not readable: {}", project.display()));
            return;
        };
        let (files, dirs) = names(&listing);
        let Some(det) = ctx.table.detect(project, &files, &dirs) else { return };
        let mut arts: Vec<Art> = Vec::new();
        for e in entries {
            // Must exist on disk as a real (non-link) directory, every segment of it.
            let (first, rest) = e.rel.split_once('/').map_or((e.rel.as_str(), None), |(f, r)| (f, Some(r)));
            let path = listing
                .dirs
                .iter()
                .find(|sd| sd.name.eq_ignore_ascii_case(first))
                .and_then(|sd| match rest {
                    None => Some(sd.path.clone()),
                    Some(r) => real_subdir(&sd.path, r),
                });
            let Some(path) = path else {
                ctx.warn(format!("index entry not found on disk (stale index?): {}", join_rel(project, &e.rel).display()));
                continue;
            };
            let Some(r) = det.resolve(&e.rel, |id| identity_ok(id, &path)) else { continue };
            let k = path_key(&path);
            if !arts.iter().any(|a| path_key(&a.path) == k) {
                arts.push(Art::new(path, r, e.size.clone()));
            }
        }
        if arts.is_empty() {
            return;
        }
        let owner = ctx.add_project(project.clone(), &det, arts);
        mtime_only(ctx, &owner, vec![project.clone()], true);
        ctx.owners.lock().unwrap().push(owner);
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
        Self::scan_table(&TABLE, roots, opts, index)
    }
}

impl DefaultScanner {
    /// `scan` over a given marker table (tests pass their own).
    fn scan_table(
        table: &'static Table,
        roots: &[PathBuf],
        opts: &ScanOptions,
        index: Option<&dyn DirIndex>,
    ) -> Loaded<ProjectCandidate> {
        let ctx = Ctx::new(table, opts);
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
            // `level` and `preselected` are placeholders and `note` holds the catalog hint: the
            // engine assesses each folder.
            by_owner.entry(Arc::as_ptr(&f.owner)).or_default().push(Artifact {
                id: path_key(&f.path),
                path: f.path,
                kind: f.kind,
                label: f.label.to_string(),
                size,
                verified: f.verified,
                class: f.class,
                level: f.class.base_level(),
                note: f.hint.to_string(),
                ambiguous: f.ambiguous,
                preselected: false,
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

pub use markers::catalog_dirs;

/// Do the last segments of `path` name a catalog entry (`ArtifactKind::Catalog`)? A prefix
/// entry (`checkpoint-*`) matches by prefix, a suffix entry (`*.egg-info`) by suffix; `*` (Meson)
/// needs `path/meson-private` on disk.
pub fn catalog_dir_plausible(path: &Path) -> bool {
    catalog_dir_match(path).is_some()
}

/// The catalog entry's `dir` that `path` ends with (`".nx/cache"`, `"checkpoint-*"`, `"*"`).
pub fn catalog_dir_match(path: &Path) -> Option<&'static str> {
    TABLE.dir_match(path)
}

#[cfg(test)]
mod tests;
