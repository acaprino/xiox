//! Data-driven marker table: marker file in a directory -> artifact directories it implies.
//!
//! The table layout (marker file -> artifact dir names per ecosystem) is adapted from
//! `kondo-lib/src/lib.rs` of kondo (MIT) by Trent Billington, https://github.com/tbillington/kondo.
//! Adding an ecosystem = adding one entry here; nothing else in the crate is ecosystem-specific.
//!
//! An artifact `dir` is one of:
//! * a name (`target`) or a sub-path of at most 3 segments (`.nx/cache`), `/`-separated;
//! * a prefix ending in `*` (`checkpoint-*`), always a direct child of the project;
//! * `*` with `Identity::MesonBuild`: any direct child that holds `meson-private/` and
//!   `build.ninja`.
//!
//! Names are always compared ASCII case-insensitively, like NTFS.

use crate::fsutil::identity_ok;
use dc_model::{ArtifactKind, Class};
use std::fs;
use std::path::{Path, PathBuf};

/// How a project marker is matched against one directory listing.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // until the catalog table uses every variant
pub enum Pattern {
    Exact(&'static str),
    Suffix(&'static str),
    /// A marker folder, e.g. `.storybook`.
    Dir(&'static str),
    /// A file in a subfolder, e.g. `ProjectSettings/ProjectVersion.txt`.
    Sub(&'static str),
    /// Every pattern present, e.g. `conf.py` + `index.rst`.
    All(&'static [Pattern]),
}

fn has_suffix(name: &str, s: &str) -> bool {
    name.len() > s.len() && name.is_char_boundary(name.len() - s.len()) && name[name.len() - s.len()..].eq_ignore_ascii_case(s)
}

/// `base` joined with a `/`-separated relative path.
pub fn join_rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
}

impl Pattern {
    /// `files` / `dirs`: names listed in `dir`. Only `Sub` reads the disk, and only once its
    /// first segment is among `dirs`.
    pub fn matches(&self, dir: &Path, files: &[&str], dirs: &[&str]) -> bool {
        match self {
            Pattern::Exact(n) => files.iter().any(|f| f.eq_ignore_ascii_case(n)),
            Pattern::Suffix(s) => files.iter().any(|f| has_suffix(f, s)),
            Pattern::Dir(n) => dirs.iter().any(|d| d.eq_ignore_ascii_case(n)),
            Pattern::Sub(rel) => {
                let first = rel.split('/').next().unwrap_or_default();
                dirs.iter().any(|d| d.eq_ignore_ascii_case(first))
                    && fs::symlink_metadata(join_rel(dir, rel)).is_ok_and(|m| m.is_file())
            }
            Pattern::All(ps) => ps.iter().all(|p| p.matches(dir, files, dirs)),
        }
    }
}

/// Identity check applied to the artifact directory itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // until the catalog table has a Meson entry
pub enum Identity {
    /// No identity file exists for this kind: verified when the marker file sits in the parent.
    ParentMarker,
    /// `CACHEDIR.TAG` or `.rustc_info.json`.
    RustTarget,
    /// `.modules.yaml`, `.package-lock.json`, `.yarn-integrity` or a `.pnpm` dir.
    NodeModules,
    /// `pyvenv.cfg`.
    PythonVenv,
    /// A `meson-private/` folder and a `build.ninja` file (Meson build dir, any name).
    MesonBuild,
}

#[derive(Debug)]
pub struct ArtifactSpec {
    /// See the module doc: name, sub-path (max 3 segments), `prefix*` or `*`.
    pub dir: &'static str,
    pub kind: ArtifactKind,
    pub label: &'static str,
    pub identity: Identity,
    pub class: Class,
    /// † name of the catalog: it can also be a source folder.
    pub ambiguous: bool,
    /// For `Class::Dependency`: how to get it back (becomes `Artifact::note`); "" otherwise.
    pub hint: &'static str,
}

impl ArtifactSpec {
    fn is_meson(&self) -> bool {
        self.dir == "*"
    }
    fn prefix(&self) -> Option<&'static str> {
        self.dir.strip_suffix('*').filter(|p| !p.is_empty())
    }
    /// A name or a sub-path (no `*`).
    pub fn is_plain(&self) -> bool {
        !self.dir.contains('*')
    }
    /// `/`-separated sub-path of more than one segment.
    pub fn is_multi(&self) -> bool {
        self.dir.contains('/')
    }
    /// Does `rel` (relative to the project root, `/`-separated) name this entry? For `*` any
    /// single name matches: the identity check decides.
    fn matches_rel(&self, rel: &str) -> bool {
        if rel.is_empty() {
            return false;
        }
        if self.is_meson() {
            return !rel.contains('/');
        }
        if let Some(p) = self.prefix() {
            return !rel.contains('/') && rel.len() > p.len() && rel.is_char_boundary(p.len()) && rel[..p.len()].eq_ignore_ascii_case(p);
        }
        rel.eq_ignore_ascii_case(self.dir)
    }
}

#[derive(Debug)]
pub struct Marker {
    pub pattern: Pattern,
    pub ecosystem: &'static str,
    pub artifacts: &'static [ArtifactSpec],
}

const fn spec(
    dir: &'static str,
    kind: ArtifactKind,
    label: &'static str,
    identity: Identity,
    class: Class,
) -> ArtifactSpec {
    ArtifactSpec { dir, kind, label, identity, class, ambiguous: false, hint: "" }
}

/// A catalog entry (`ArtifactKind::Catalog`) recognised by the marker in its project.
const fn cat(dir: &'static str, label: &'static str, class: Class) -> ArtifactSpec {
    spec(dir, ArtifactKind::Catalog, label, Identity::ParentMarker, class)
}

static RUST: &[ArtifactSpec] = &[spec("target", ArtifactKind::RustTarget, "Rust target/", Identity::RustTarget, Class::Build)];
static NODE: &[ArtifactSpec] =
    &[spec("node_modules", ArtifactKind::NodeModules, "node_modules", Identity::NodeModules, Class::Dependency)];
static PYTHON: &[ArtifactSpec] = &[
    spec(".venv", ArtifactKind::PythonVenv, "Python venv (.venv)", Identity::PythonVenv, Class::Dependency),
    spec("venv", ArtifactKind::PythonVenv, "Python venv (venv)", Identity::PythonVenv, Class::Dependency),
    spec("__pycache__", ArtifactKind::PythonCache, "Python __pycache__", Identity::ParentMarker, Class::Cache),
    spec(".pytest_cache", ArtifactKind::PythonCache, "pytest cache", Identity::ParentMarker, Class::Cache),
    spec(".mypy_cache", ArtifactKind::PythonCache, "mypy cache", Identity::ParentMarker, Class::Cache),
    spec(".ruff_cache", ArtifactKind::PythonCache, "ruff cache", Identity::ParentMarker, Class::Cache),
];
static GRADLE: &[ArtifactSpec] = &[
    spec("build", ArtifactKind::Gradle, "Gradle build/", Identity::ParentMarker, Class::Build),
    spec(".gradle", ArtifactKind::Gradle, "Gradle .gradle/", Identity::ParentMarker, Class::Cache),
];
static MAVEN: &[ArtifactSpec] = &[spec("target", ArtifactKind::Maven, "Maven target/", Identity::ParentMarker, Class::Build)];
static DOTNET: &[ArtifactSpec] = &[
    spec("bin", ArtifactKind::DotNet, ".NET bin/", Identity::ParentMarker, Class::Build),
    spec("obj", ArtifactKind::DotNet, ".NET obj/", Identity::ParentMarker, Class::Build),
];
static CMAKE: &[ArtifactSpec] = &[spec("build", ArtifactKind::CMake, "CMake build/", Identity::ParentMarker, Class::Build)];
static NX: &[ArtifactSpec] = &[
    cat(".nx/cache", "Cache Nx (.nx/cache)", Class::Cache),
    cat(".nx/workspace-data", "Dati di lavoro Nx (.nx/workspace-data)", Class::Cache),
];
static CYPRESS: &[ArtifactSpec] = &[
    cat("cypress/videos", "Video dei test Cypress (cypress/videos)", Class::Log),
    cat("cypress/screenshots", "Schermate dei test Cypress (cypress/screenshots)", Class::Log),
];

/// Order matters: it is the order of `ProjectCandidate::ecosystems`.
pub static MARKERS: &[Marker] = &[
    Marker { pattern: Pattern::Exact("Cargo.toml"), ecosystem: "rust", artifacts: RUST },
    Marker { pattern: Pattern::Exact("package.json"), ecosystem: "node", artifacts: NODE },
    Marker { pattern: Pattern::Exact("pyproject.toml"), ecosystem: "python", artifacts: PYTHON },
    Marker { pattern: Pattern::Exact("requirements.txt"), ecosystem: "python", artifacts: PYTHON },
    Marker { pattern: Pattern::Exact("setup.py"), ecosystem: "python", artifacts: PYTHON },
    Marker { pattern: Pattern::Exact("build.gradle"), ecosystem: "gradle", artifacts: GRADLE },
    Marker { pattern: Pattern::Exact("build.gradle.kts"), ecosystem: "gradle", artifacts: GRADLE },
    Marker { pattern: Pattern::Exact("pom.xml"), ecosystem: "maven", artifacts: MAVEN },
    Marker { pattern: Pattern::Suffix(".csproj"), ecosystem: "dotnet", artifacts: DOTNET },
    Marker { pattern: Pattern::Exact("CMakeLists.txt"), ecosystem: "cmake", artifacts: CMAKE },
    Marker { pattern: Pattern::Exact("nx.json"), ecosystem: "nx", artifacts: NX },
    Marker { pattern: Pattern::Exact("cypress.config.ts"), ecosystem: "cypress", artifacts: CYPRESS },
    Marker { pattern: Pattern::Exact("cypress.config.js"), ecosystem: "cypress", artifacts: CYPRESS },
];

/// Entries added to every recognised project (filled by the catalog, Task 6).
pub static COMMON: &[ArtifactSpec] = &[];

/// Directory names that are never entered, anywhere (they never contain projects we care about
/// and can hold hundreds of thousands of entries).
pub const NEVER_ENTER: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".angular",
    ".turbo",
    ".dart_tool",
    ".terraform",
    ".terragrunt-cache",
    ".stack-work",
    "dist-newstyle",
    ".zig-cache",
    ".pio",
];

/// The marker table and the entries common to every project. The scanner works over one
/// (`TABLE` in production; tests pass their own).
pub struct Table {
    pub markers: &'static [Marker],
    pub common: &'static [ArtifactSpec],
}

pub static TABLE: Table = Table { markers: MARKERS, common: COMMON };

/// Last `k` path names of `path`, `/`-joined, and the project folder `k` levels up (which must
/// itself be a named folder, never a drive root). Purely lexical.
pub fn tail_rel(path: &Path, k: usize) -> Option<(String, &Path)> {
    let mut names = Vec::with_capacity(k);
    let mut p = path;
    for _ in 0..k {
        names.push(p.file_name()?.to_str()?);
        p = p.parent()?;
    }
    p.file_name()?;
    names.reverse();
    Some((names.join("/"), p))
}

impl Table {
    fn specs(&self) -> impl Iterator<Item = &'static ArtifactSpec> + '_ {
        self.markers.iter().flat_map(|m| m.artifacts.iter()).chain(self.common.iter())
    }

    /// Markers matching in `dir` (whose listing is `files` + `dirs`); `None` if none does.
    pub fn detect(&self, dir: &Path, files: &[&str], dirs: &[&str]) -> Option<Detected> {
        let mut d = Detected::default();
        let push = |d: &mut Detected, a: &'static ArtifactSpec| {
            if !d.specs.iter().any(|s| std::ptr::eq(*s, a)) {
                d.specs.push(a);
            }
        };
        for m in self.markers {
            if m.pattern.matches(dir, files, dirs) {
                if !d.ecosystems.contains(&m.ecosystem) {
                    d.ecosystems.push(m.ecosystem);
                }
                m.artifacts.iter().for_each(|a| push(&mut d, a));
            }
        }
        if d.ecosystems.is_empty() {
            return None;
        }
        self.common.iter().for_each(|a| push(&mut d, a));
        Some(d)
    }

    /// Distinct single names (no sub-path, no `*`) in the table: generated output the walker
    /// does not enter once its identity file is seen.
    pub fn plain_names(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for s in self.specs().filter(|s| s.is_plain() && !s.is_multi()) {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(s.dir)) {
                v.push(s.dir);
            }
        }
        v
    }

    /// Names searched in a directory index: the last segment of every name or sub-path entry
    /// (prefixes and `*` excluded), plus `meson-private` when a Meson entry exists.
    pub fn index_names(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        let mut add = |n: &'static str| {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                v.push(n);
            }
        };
        for s in self.specs() {
            if s.is_meson() {
                add("meson-private");
            } else if s.is_plain() {
                add(s.dir.rsplit('/').next().unwrap_or(s.dir));
            }
        }
        v
    }

    /// Lexical candidates for an index hit: `(project, rel)` for every name / sub-path entry
    /// whose segments are the last segments of `hit`, and the Meson build dir for a
    /// `meson-private` hit. Nothing here reads the disk.
    pub fn index_candidates<'p>(&self, hit: &'p Path) -> Vec<(&'p Path, String)> {
        let mut v: Vec<(&'p Path, String)> = Vec::new();
        let mut add = |c: (String, &'p Path)| {
            if !v.iter().any(|(p, r)| *p == c.1 && r.eq_ignore_ascii_case(&c.0)) {
                v.push((c.1, c.0));
            }
        };
        let Some(name) = hit.file_name().and_then(|n| n.to_str()) else { return v };
        for s in self.specs() {
            if s.is_meson() {
                if name.eq_ignore_ascii_case("meson-private")
                    && let Some(c) = hit.parent().and_then(|b| tail_rel(b, 1))
                {
                    add(c);
                }
            } else if s.is_plain() && s.dir.rsplit('/').next().is_some_and(|l| l.eq_ignore_ascii_case(name)) {
                let k = s.dir.split('/').count();
                if let Some(c) = tail_rel(hit, k).filter(|(rel, _)| rel.eq_ignore_ascii_case(s.dir)) {
                    add(c);
                }
            }
        }
        v
    }

    /// The `dir` of the catalog entry (`ArtifactKind::Catalog`) that `path` ends with, if any.
    /// Lexical, except for `*` (Meson), which needs `path/meson-private` and `path/build.ninja`.
    pub fn dir_match(&self, path: &Path) -> Option<&'static str> {
        for s in self.specs().filter(|s| s.kind == ArtifactKind::Catalog) {
            let k = if s.is_plain() { s.dir.split('/').count() } else { 1 };
            let Some((rel, _)) = tail_rel(path, k) else { continue };
            if s.matches_rel(&rel) && (!s.is_meson() || identity_ok(s.identity, path)) {
                return Some(s.dir);
            }
        }
        None
    }
}

/// `TABLE.detect`: the production table.
pub fn detect(dir: &Path, files: &[&str], dirs: &[&str]) -> Option<Detected> {
    TABLE.detect(dir, files, dirs)
}

/// Markers found in one directory: ecosystems (table order, unique) and candidate artifact specs.
#[derive(Debug, Default)]
pub struct Detected {
    pub ecosystems: Vec<&'static str>,
    pub specs: Vec<&'static ArtifactSpec>,
}

impl Detected {
    /// Pick the spec for the directory `rel` (relative to the project, `/`-separated) and
    /// decide `verified` and `ambiguous`.
    /// Several specs can claim one name (`target` = Cargo or Maven; `build` = Gradle or CMake):
    /// prefer one whose identity file is actually present, then one with no identity file
    /// (marker in the parent is all we can ask), else report the first as unverified. A `*`
    /// entry only claims a folder whose identity holds.
    /// `ambiguous`: ANY claiming spec is a † name, even when another one would accept the folder
    /// without git (`package.json` + `CMakeLists.txt`: `build` needs git).
    pub fn resolve(
        &self,
        rel: &str,
        identity_ok: impl Fn(Identity) -> bool,
    ) -> Option<(&'static ArtifactSpec, bool, bool)> {
        let cands: Vec<&'static ArtifactSpec> = self
            .specs
            .iter()
            .copied()
            .filter(|s| s.matches_rel(rel) && (!s.is_meson() || identity_ok(s.identity)))
            .collect();
        let first = *cands.first()?;
        let ambiguous = cands.iter().any(|s| s.ambiguous);
        if let Some(s) = cands.iter().find(|s| s.identity != Identity::ParentMarker && (s.is_meson() || identity_ok(s.identity))) {
            return Some((s, true, ambiguous));
        }
        if let Some(s) = cands.iter().find(|s| s.identity == Identity::ParentMarker) {
            return Some((s, true, ambiguous));
        }
        Some((first, false, ambiguous))
    }

    /// Specs with a sub-path (`.nx/cache`): probed below the project root, not among its
    /// direct children.
    pub fn multi_specs(&self) -> impl Iterator<Item = &'static ArtifactSpec> + '_ {
        self.specs.iter().copied().filter(|s| s.is_multi())
    }

    /// Does any spec need a look at direct children beyond their name (prefix or `*`)?
    pub fn has_patterns(&self) -> bool {
        self.specs.iter().any(|s| !s.is_plain())
    }

    /// Lowercase single-name artifact dirs of this project (skipped when walking its sources).
    pub fn skip_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.specs.iter().filter(|s| s.is_plain() && !s.is_multi()) {
            let l = s.dir.to_ascii_lowercase();
            if !v.contains(&l) {
                v.push(l);
            }
        }
        v
    }
}

/// Is `p` a real directory (not a link or junction) reached without crossing a link below
/// `base`? Used for sub-path artifacts.
pub fn real_subdir(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = base.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
        if !crate::fsutil::is_real_dir(&p) {
            return None;
        }
    }
    Some(p)
}
