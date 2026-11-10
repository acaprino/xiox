//! Data-driven marker table: marker file in a directory -> artifact directories it implies.
//!
//! The table layout (marker file -> artifact dir names per ecosystem) is adapted from
//! `kondo-lib/src/lib.rs` of kondo (MIT) by Trent Billington, https://github.com/tbillington/kondo.
//! Adding an ecosystem = adding one entry here; nothing else in the crate is ecosystem-specific.

use dc_model::ArtifactKind;

/// How a marker file name is matched (always ASCII case-insensitive, like NTFS).
#[derive(Clone, Copy, Debug)]
pub enum Pattern {
    Exact(&'static str),
    Suffix(&'static str),
}

impl Pattern {
    pub fn matches(&self, file_name: &str) -> bool {
        match self {
            Pattern::Exact(n) => file_name.eq_ignore_ascii_case(n),
            Pattern::Suffix(s) => {
                file_name.len() > s.len()
                    && file_name.is_char_boundary(file_name.len() - s.len())
                    && file_name[file_name.len() - s.len()..].eq_ignore_ascii_case(s)
            }
        }
    }
}

/// Identity check applied to the artifact directory itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// No identity file exists for this kind: verified when the marker file sits in the parent.
    ParentMarker,
    /// `CACHEDIR.TAG` or `.rustc_info.json`.
    RustTarget,
    /// `.modules.yaml`, `.package-lock.json`, `.yarn-integrity` or a `.pnpm` dir.
    NodeModules,
    /// `pyvenv.cfg`.
    PythonVenv,
}

#[derive(Debug)]
pub struct ArtifactSpec {
    pub dir: &'static str,
    pub kind: ArtifactKind,
    pub label: &'static str,
    pub identity: Identity,
}

#[derive(Debug)]
pub struct Marker {
    pub pattern: Pattern,
    pub ecosystem: &'static str,
    pub artifacts: &'static [ArtifactSpec],
}

const fn spec(dir: &'static str, kind: ArtifactKind, label: &'static str, identity: Identity) -> ArtifactSpec {
    ArtifactSpec { dir, kind, label, identity }
}

static RUST: &[ArtifactSpec] = &[spec("target", ArtifactKind::RustTarget, "Rust target/", Identity::RustTarget)];
static NODE: &[ArtifactSpec] =
    &[spec("node_modules", ArtifactKind::NodeModules, "node_modules", Identity::NodeModules)];
static PYTHON: &[ArtifactSpec] = &[
    spec(".venv", ArtifactKind::PythonVenv, "Python venv (.venv)", Identity::PythonVenv),
    spec("venv", ArtifactKind::PythonVenv, "Python venv (venv)", Identity::PythonVenv),
    spec("__pycache__", ArtifactKind::PythonCache, "Python __pycache__", Identity::ParentMarker),
    spec(".pytest_cache", ArtifactKind::PythonCache, "pytest cache", Identity::ParentMarker),
    spec(".mypy_cache", ArtifactKind::PythonCache, "mypy cache", Identity::ParentMarker),
    spec(".ruff_cache", ArtifactKind::PythonCache, "ruff cache", Identity::ParentMarker),
];
static GRADLE: &[ArtifactSpec] = &[
    spec("build", ArtifactKind::Gradle, "Gradle build/", Identity::ParentMarker),
    spec(".gradle", ArtifactKind::Gradle, "Gradle .gradle/", Identity::ParentMarker),
];
static MAVEN: &[ArtifactSpec] = &[spec("target", ArtifactKind::Maven, "Maven target/", Identity::ParentMarker)];
static DOTNET: &[ArtifactSpec] = &[
    spec("bin", ArtifactKind::DotNet, ".NET bin/", Identity::ParentMarker),
    spec("obj", ArtifactKind::DotNet, ".NET obj/", Identity::ParentMarker),
];
static CMAKE: &[ArtifactSpec] = &[spec("build", ArtifactKind::CMake, "CMake build/", Identity::ParentMarker)];

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
];

/// Directory names that are never entered, anywhere (they never contain projects we care about
/// and can hold hundreds of thousands of entries).
pub const NEVER_ENTER: &[&str] =
    &[".git", "node_modules", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache"];

/// All distinct artifact directory names in the table (lowercase), used for index discovery.
pub fn artifact_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for m in MARKERS {
        for a in m.artifacts {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(a.dir)) {
                v.push(a.dir);
            }
        }
    }
    v
}

/// Markers found in one directory: ecosystems (table order, unique) and candidate artifact specs.
#[derive(Debug, Default)]
pub struct Detected {
    pub ecosystems: Vec<&'static str>,
    pub specs: Vec<&'static ArtifactSpec>,
}

/// `None` if no marker file is present.
pub fn detect<'a>(file_names: impl Iterator<Item = &'a str>) -> Option<Detected> {
    let names: Vec<&str> = file_names.collect();
    let mut d = Detected::default();
    for m in MARKERS {
        if names.iter().any(|n| m.pattern.matches(n)) {
            if !d.ecosystems.contains(&m.ecosystem) {
                d.ecosystems.push(m.ecosystem);
            }
            for a in m.artifacts {
                if !d.specs.iter().any(|s| std::ptr::eq(*s, a)) {
                    d.specs.push(a);
                }
            }
        }
    }
    if d.ecosystems.is_empty() { None } else { Some(d) }
}

impl Detected {
    /// Pick the spec for a directory called `dir_name` and decide `verified`.
    /// Several specs can claim one name (`target` = Cargo or Maven; `build` = Gradle or CMake):
    /// prefer one whose identity file is actually present, then one with no identity file
    /// (marker in the parent is all we can ask), else report the first as unverified.
    pub fn resolve(
        &self,
        dir_name: &str,
        identity_ok: impl Fn(Identity) -> bool,
    ) -> Option<(&'static ArtifactSpec, bool)> {
        let cands: Vec<&'static ArtifactSpec> =
            self.specs.iter().copied().filter(|s| s.dir.eq_ignore_ascii_case(dir_name)).collect();
        if cands.is_empty() {
            return None;
        }
        if let Some(s) = cands.iter().find(|s| s.identity != Identity::ParentMarker && identity_ok(s.identity)) {
            return Some((s, true));
        }
        if let Some(s) = cands.iter().find(|s| s.identity == Identity::ParentMarker) {
            return Some((s, true));
        }
        Some((cands[0], false))
    }

    /// Lowercase artifact dir names of this project (skipped when walking its sources).
    pub fn skip_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in &self.specs {
            let l = s.dir.to_ascii_lowercase();
            if !v.contains(&l) {
                v.push(l);
            }
        }
        v
    }
}
