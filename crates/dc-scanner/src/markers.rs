//! Data-driven marker table: marker file in a directory -> artifact directories it implies.
//!
//! The table layout (marker file -> artifact dir names per ecosystem) is adapted from
//! `kondo-lib/src/lib.rs` of kondo (MIT) by Trent Billington, https://github.com/tbillington/kondo.
//! Adding an ecosystem = adding one entry here; nothing else in the crate is ecosystem-specific.
//!
//! An artifact `dir` is one of:
//! * a name (`target`) or a sub-path of at most 3 segments (`.nx/cache`), `/`-separated;
//! * a prefix ending in `*` (`checkpoint-*`) or a suffix starting with `*` (`*.egg-info`),
//!   always a direct child of the project;
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
    /// See the module doc: name, sub-path (max 3 segments), `prefix*`, `*suffix` or `*`.
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
        if !self.is_plain() {
            return !rel.contains('/') && dc_model::name_matches(self.dir, rel);
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

/// A † catalog entry: the name can also be a source folder.
const fn amb(dir: &'static str, label: &'static str, class: Class) -> ArtifactSpec {
    ArtifactSpec { ambiguous: true, ..cat(dir, label, class) }
}

/// A catalog `Class::Dependency` entry, with how to get it back.
const fn dep(dir: &'static str, label: &'static str, hint: &'static str) -> ArtifactSpec {
    ArtifactSpec { hint, ..cat(dir, label, Class::Dependency) }
}

/// A † `Class::Dependency` catalog entry.
const fn amb_dep(dir: &'static str, label: &'static str, hint: &'static str) -> ArtifactSpec {
    ArtifactSpec { ambiguous: true, ..dep(dir, label, hint) }
}

use Class::{Build, Cache, Log, Personal};

const HINT_VENV: &str = "Si ricrea con python -m venv e l'installazione dei requisiti";
const HINT_TOX: &str = "Si ricrea al prossimo tox/nox";

static RUST: &[ArtifactSpec] = &[spec("target", ArtifactKind::RustTarget, "Build Rust (target)", Identity::RustTarget, Build)];
static NODE: &[ArtifactSpec] = &[
    ArtifactSpec {
        hint: "Si riscarica con npm install (o pnpm, yarn, bun install)",
        ..spec("node_modules", ArtifactKind::NodeModules, "Dipendenze Node (node_modules)", Identity::NodeModules, Class::Dependency)
    },
    cat(".parcel-cache", "Cache Parcel (.parcel-cache)", Cache),
    cat(".turbo", "Cache Turborepo (.turbo)", Cache),
    cat("coverage", "Copertura dei test (coverage)", Log),
    cat(".nyc_output", "Dati di copertura nyc (.nyc_output)", Log),
    amb("dist", "Build Node (dist)", Build),
    amb("build", "Build Node (build)", Build),
    amb("out", "Build Node (out)", Build),
];
static NEXT: &[ArtifactSpec] = &[cat(".next", "Cache Next.js (.next)", Cache), amb("out", "Build Next.js (out)", Build)];
static NUXT: &[ArtifactSpec] = &[cat(".nuxt", "Cache Nuxt (.nuxt)", Cache), cat(".output", "Build Nuxt (.output)", Build)];
static SVELTEKIT: &[ArtifactSpec] = &[cat(".svelte-kit", "Cache SvelteKit (.svelte-kit)", Cache)];
static ANGULAR: &[ArtifactSpec] =
    &[cat(".angular", "Cache Angular (.angular)", Cache), amb("dist", "Build Angular (dist)", Build)];
static NX: &[ArtifactSpec] = &[
    cat(".nx/cache", "Cache Nx (.nx/cache)", Cache),
    cat(".nx/workspace-data", "Dati di lavoro Nx (.nx/workspace-data)", Cache),
];
static ASTRO: &[ArtifactSpec] = &[cat(".astro", "Cache Astro (.astro)", Cache), amb("dist", "Build Astro (dist)", Build)];
static DOCUSAURUS: &[ArtifactSpec] =
    &[cat(".docusaurus", "Cache Docusaurus (.docusaurus)", Cache), amb("build", "Build Docusaurus (build)", Build)];
static STORYBOOK: &[ArtifactSpec] = &[cat("storybook-static", "Build Storybook (storybook-static)", Build)];
static PLAYWRIGHT: &[ArtifactSpec] = &[
    cat("playwright-report", "Report di Playwright (playwright-report)", Log),
    cat("test-results", "Risultati dei test Playwright (test-results)", Log),
    cat("blob-report", "Report blob di Playwright (blob-report)", Log),
];
static CYPRESS: &[ArtifactSpec] = &[
    cat("cypress/videos", "Video dei test Cypress (cypress/videos)", Log),
    cat("cypress/screenshots", "Schermate dei test Cypress (cypress/screenshots)", Log),
];
static PYTHON: &[ArtifactSpec] = &[
    ArtifactSpec {
        hint: HINT_VENV,
        ..spec(".venv", ArtifactKind::PythonVenv, "Ambiente virtuale Python (.venv)", Identity::PythonVenv, Class::Dependency)
    },
    ArtifactSpec {
        hint: HINT_VENV,
        ..spec("venv", ArtifactKind::PythonVenv, "Ambiente virtuale Python (venv)", Identity::PythonVenv, Class::Dependency)
    },
    spec("__pycache__", ArtifactKind::PythonCache, "Cache Python (__pycache__)", Identity::ParentMarker, Cache),
    spec(".pytest_cache", ArtifactKind::PythonCache, "Cache pytest (.pytest_cache)", Identity::ParentMarker, Cache),
    spec(".mypy_cache", ArtifactKind::PythonCache, "Cache mypy (.mypy_cache)", Identity::ParentMarker, Cache),
    spec(".ruff_cache", ArtifactKind::PythonCache, "Cache Ruff (.ruff_cache)", Identity::ParentMarker, Cache),
    dep(".tox", "Ambienti tox (.tox)", HINT_TOX),
    dep(".nox", "Ambienti nox (.nox)", HINT_TOX),
    cat("*.egg-info", "Metadati del pacchetto Python (*.egg-info)", Build),
    cat(".eggs", "Pacchetti di build Python (.eggs)", Cache),
    cat("htmlcov", "Report di copertura (htmlcov)", Log),
    cat(".pytype", "Cache pytype (.pytype)", Cache),
    cat(".pyre", "Cache Pyre (.pyre)", Cache),
    dep("__pypackages__", "Pacchetti PDM (__pypackages__)", "Si riscarica con pdm install"),
    amb("build", "Build Python (build)", Build),
    amb("dist", "Pacchetti Python (dist)", Build),
    cat(".hypothesis", "Esempi che hanno fatto fallire i test (.hypothesis)", Personal),
    cat("wandb", "Esecuzioni di Weights & Biases (wandb)", Personal),
    cat("mlruns", "Esecuzioni di MLflow (mlruns)", Personal),
    cat("runs", "Esecuzioni di addestramento (runs)", Personal),
    cat("checkpoint-*", "Checkpoint di addestramento (checkpoint-*)", Personal),
];
static GRADLE: &[ArtifactSpec] = &[
    spec("build", ArtifactKind::Gradle, "Build Gradle (build)", Identity::ParentMarker, Build),
    spec(".gradle", ArtifactKind::Gradle, "Cache Gradle (.gradle)", Identity::ParentMarker, Cache),
    cat(".kotlin", "Cache Kotlin (.kotlin)", Cache),
    cat(".cxx", "Build nativa Android (.cxx)", Build),
    cat(".externalNativeBuild", "Build nativa Android (.externalNativeBuild)", Build),
];
static MAVEN: &[ArtifactSpec] = &[spec("target", ArtifactKind::Maven, "Build Maven (target)", Identity::ParentMarker, Build)];
static SBT: &[ArtifactSpec] =
    &[cat("target", "Build sbt (target)", Build), cat("project/target", "Build sbt del progetto (project/target)", Build)];
static LEININGEN: &[ArtifactSpec] = &[cat("target", "Build Leiningen (target)", Build)];
static CLOJURE: &[ArtifactSpec] = &[cat(".cpcache", "Cache Clojure CLI (.cpcache)", Cache)];
static DOTNET: &[ArtifactSpec] = &[
    spec("bin", ArtifactKind::DotNet, "Build .NET (bin)", Identity::ParentMarker, Build),
    spec("obj", ArtifactKind::DotNet, "Build intermedia .NET (obj)", Identity::ParentMarker, Build),
    cat("TestResults", "Risultati dei test .NET (TestResults)", Log),
    cat("BenchmarkDotNet.Artifacts", "Risultati di BenchmarkDotNet (BenchmarkDotNet.Artifacts)", Log),
];
static DOTNET_PROPS: &[ArtifactSpec] = &[amb("artifacts", "Build .NET (artifacts)", Build)];
static CMAKE: &[ArtifactSpec] = &[spec("build", ArtifactKind::CMake, "Build CMake (build)", Identity::ParentMarker, Build)];
static CLANGD: &[ArtifactSpec] =
    &[cat(".cache/clangd", "Indice di clangd (.cache/clangd)", Cache), cat(".ccls-cache", "Cache ccls (.ccls-cache)", Cache)];
static AUTOTOOLS: &[ArtifactSpec] = &[cat("autom4te.cache", "Cache Autotools (autom4te.cache)", Cache)];
static MESON: &[ArtifactSpec] = &[ArtifactSpec { identity: Identity::MesonBuild, ..cat("*", "Build Meson", Build) }];
static VCPKG: &[ArtifactSpec] =
    &[dep("vcpkg_installed", "Pacchetti vcpkg (vcpkg_installed)", "Si ricrea con vcpkg install")];
static PLATFORMIO: &[ArtifactSpec] =
    &[dep(".pio", "Build e librerie PlatformIO (.pio)", "Si riscarica alla prossima build PlatformIO")];
static FLUTTER: &[ArtifactSpec] =
    &[cat(".dart_tool", "Cache Dart (.dart_tool)", Cache), amb("build", "Build Flutter (build)", Build)];
static SWIFT: &[ArtifactSpec] = &[dep(".build", "Build e dipendenze Swift (.build)", "Si ricrea con swift build")];
static GO: &[ArtifactSpec] = &[amb_dep("vendor", "Dipendenze Go (vendor)", "Si ricrea con go mod vendor")];
static PHP: &[ArtifactSpec] = &[amb_dep("vendor", "Dipendenze Composer (vendor)", "Si riscarica con composer install")];
static RUBY: &[ArtifactSpec] =
    &[amb_dep("vendor/bundle", "Gem di Bundler (vendor/bundle)", "Si riscarica con bundle install")];
static ELIXIR: &[ArtifactSpec] =
    &[cat("_build", "Build Elixir (_build)", Build), dep("deps", "Dipendenze Elixir (deps)", "Si riscarica con mix deps.get")];
static ERLANG: &[ArtifactSpec] = &[cat("_build", "Build Erlang (_build)", Build)];
static STACK: &[ArtifactSpec] = &[cat(".stack-work", "Build Stack (.stack-work)", Build)];
static CABAL: &[ArtifactSpec] = &[cat("dist-newstyle", "Build Cabal (dist-newstyle)", Build)];
static OCAML: &[ArtifactSpec] = &[
    cat("_build", "Build Dune (_build)", Build),
    dep("_opam", "Switch opam locale (_opam)", "Si ricrea con opam install . --deps-only"),
];
static ZIG: &[ArtifactSpec] = &[
    cat(".zig-cache", "Cache Zig (.zig-cache)", Cache),
    cat("zig-cache", "Cache Zig (zig-cache)", Cache),
    cat("zig-out", "Build Zig (zig-out)", Build),
];
static UNITY: &[ArtifactSpec] = &[
    cat("Library", "Cache Unity (Library)", Cache),
    cat("Temp", "File temporanei di Unity (Temp)", Cache),
    cat("Obj", "Build Unity (Obj)", Build),
    cat("Logs", "Log di Unity (Logs)", Log),
    cat("UserSettings", "Impostazioni utente di Unity (UserSettings)", Personal),
];
static UNREAL: &[ArtifactSpec] = &[
    cat("Intermediate", "Build intermedia di Unreal (Intermediate)", Build),
    cat("DerivedDataCache", "Cache dei dati derivati di Unreal (DerivedDataCache)", Cache),
    cat("Saved", "Dati salvati di Unreal (Saved)", Personal),
    cat("Binaries", "Binari di Unreal (Binaries)", Personal),
];
static GODOT: &[ArtifactSpec] =
    &[cat(".godot", "Cache Godot (.godot)", Cache), cat(".import", "Risorse importate di Godot (.import)", Build)];
static SPHINX: &[ArtifactSpec] = &[cat("_build", "Build Sphinx (_build)", Build)];
static MKDOCS: &[ArtifactSpec] = &[amb("site", "Sito MkDocs (site)", Build)];
static JEKYLL: &[ArtifactSpec] =
    &[cat("_site", "Sito Jekyll (_site)", Build), cat(".jekyll-cache", "Cache Jekyll (.jekyll-cache)", Cache)];
static HUGO: &[ArtifactSpec] = &[
    cat("resources/_gen", "Risorse generate da Hugo (resources/_gen)", Cache),
    amb("public", "Sito Hugo (public)", Build),
];
static TERRAFORM: &[ArtifactSpec] =
    &[dep(".terraform", "Provider e moduli Terraform (.terraform)", "Si riscarica con terraform init")];
static TERRAGRUNT: &[ArtifactSpec] = &[cat(".terragrunt-cache", "Cache Terragrunt (.terragrunt-cache)", Cache)];
static CDK: &[ArtifactSpec] = &[cat("cdk.out", "Build AWS CDK (cdk.out)", Build)];
static SAM: &[ArtifactSpec] = &[cat(".aws-sam", "Build AWS SAM (.aws-sam)", Build)];
static SERVERLESS: &[ArtifactSpec] = &[cat(".serverless", "Pacchetti Serverless (.serverless)", Build)];
static DBT: &[ArtifactSpec] = &[
    cat("target", "Build dbt (target)", Build),
    dep("dbt_packages", "Pacchetti dbt (dbt_packages)", "Si riscarica con dbt deps"),
    cat("logs", "Log di dbt (logs)", Log),
];
static RSTUDIO: &[ArtifactSpec] = &[cat(".Rproj.user", "Stato di RStudio (.Rproj.user)", Personal)];
static YARN: &[ArtifactSpec] = &[cat(".yarn/cache", "Cache Yarn del progetto (.yarn/cache)", Personal)];
static DVC: &[ArtifactSpec] = &[cat(".dvc/cache", "Cache DVC (.dvc/cache)", Personal)];
static LARAVEL: &[ArtifactSpec] = &[
    cat("bootstrap/cache", "Cache di Laravel (bootstrap/cache)", Personal),
    cat("storage/framework/cache", "Cache di Laravel (storage/framework/cache)", Personal),
    cat("storage/framework/views", "Viste compilate di Laravel (storage/framework/views)", Personal),
];
static SYMFONY: &[ArtifactSpec] = &[cat("var/cache", "Cache di Symfony (var/cache)", Personal)];

const fn m(pattern: Pattern, ecosystem: &'static str, artifacts: &'static [ArtifactSpec]) -> Marker {
    Marker { pattern, ecosystem, artifacts }
}

use Pattern::{All, Dir, Exact, Suffix, Sub};

/// Order matters: it is the order of `ProjectCandidate::ecosystems`.
pub static MARKERS: &[Marker] = &[
    m(Exact("Cargo.toml"), "rust", RUST),
    m(Exact("package.json"), "node", NODE),
    m(Exact("pyproject.toml"), "python", PYTHON),
    m(Exact("requirements.txt"), "python", PYTHON),
    m(Exact("setup.py"), "python", PYTHON),
    m(Exact("setup.cfg"), "python", PYTHON),
    m(Exact("tox.ini"), "python", PYTHON),
    m(Exact("noxfile.py"), "python", PYTHON),
    m(Exact("Pipfile"), "python", PYTHON),
    m(Exact("build.gradle"), "gradle", GRADLE),
    m(Exact("build.gradle.kts"), "gradle", GRADLE),
    m(Exact("settings.gradle"), "gradle", GRADLE),
    m(Exact("settings.gradle.kts"), "gradle", GRADLE),
    m(Exact("pom.xml"), "maven", MAVEN),
    m(Suffix(".csproj"), "dotnet", DOTNET),
    m(Suffix(".fsproj"), "dotnet", DOTNET),
    m(Suffix(".vbproj"), "dotnet", DOTNET),
    m(Exact("Directory.Build.props"), "dotnet", DOTNET_PROPS),
    m(Exact("CMakeLists.txt"), "cmake", CMAKE),
    m(Exact("next.config.js"), "nextjs", NEXT),
    m(Exact("next.config.mjs"), "nextjs", NEXT),
    m(Exact("next.config.cjs"), "nextjs", NEXT),
    m(Exact("next.config.ts"), "nextjs", NEXT),
    m(Exact("nuxt.config.js"), "nuxt", NUXT),
    m(Exact("nuxt.config.ts"), "nuxt", NUXT),
    m(Exact("nuxt.config.mjs"), "nuxt", NUXT),
    m(Exact("svelte.config.js"), "sveltekit", SVELTEKIT),
    m(Exact("svelte.config.ts"), "sveltekit", SVELTEKIT),
    m(Exact("angular.json"), "angular", ANGULAR),
    m(Exact("nx.json"), "nx", NX),
    m(Exact("astro.config.mjs"), "astro", ASTRO),
    m(Exact("astro.config.ts"), "astro", ASTRO),
    m(Exact("astro.config.js"), "astro", ASTRO),
    m(Exact("docusaurus.config.js"), "docusaurus", DOCUSAURUS),
    m(Exact("docusaurus.config.ts"), "docusaurus", DOCUSAURUS),
    m(Dir(".storybook"), "storybook", STORYBOOK),
    m(Exact("playwright.config.ts"), "playwright", PLAYWRIGHT),
    m(Exact("playwright.config.js"), "playwright", PLAYWRIGHT),
    m(Exact("cypress.config.ts"), "cypress", CYPRESS),
    m(Exact("cypress.config.js"), "cypress", CYPRESS),
    m(Exact("cypress.config.mjs"), "cypress", CYPRESS),
    m(Exact("cypress.config.cjs"), "cypress", CYPRESS),
    m(Exact("build.sbt"), "sbt", SBT),
    m(Exact("project.clj"), "leiningen", LEININGEN),
    m(Exact("deps.edn"), "clojure", CLOJURE),
    m(Exact("compile_commands.json"), "cpp", CLANGD),
    m(Exact("configure.ac"), "autotools", AUTOTOOLS),
    m(Exact("meson.build"), "meson", MESON),
    m(Exact("vcpkg.json"), "vcpkg", VCPKG),
    m(Exact("platformio.ini"), "platformio", PLATFORMIO),
    m(Exact("pubspec.yaml"), "flutter", FLUTTER),
    m(Exact("Package.swift"), "swift", SWIFT),
    m(Exact("go.mod"), "go", GO),
    m(Exact("composer.json"), "php", PHP),
    m(Exact("Gemfile"), "ruby", RUBY),
    m(Exact("mix.exs"), "elixir", ELIXIR),
    m(Exact("rebar.config"), "erlang", ERLANG),
    m(Exact("stack.yaml"), "haskell", STACK),
    m(Exact("cabal.project"), "haskell", CABAL),
    m(Suffix(".cabal"), "haskell", CABAL),
    m(Exact("dune-project"), "ocaml", OCAML),
    m(Exact("build.zig"), "zig", ZIG),
    m(Sub("ProjectSettings/ProjectVersion.txt"), "unity", UNITY),
    m(Suffix(".uproject"), "unreal", UNREAL),
    m(Exact("project.godot"), "godot", GODOT),
    m(All(&[Exact("conf.py"), Exact("index.rst")]), "sphinx", SPHINX),
    m(Exact("mkdocs.yml"), "mkdocs", MKDOCS),
    m(All(&[Exact("_config.yml"), Exact("Gemfile")]), "jekyll", JEKYLL),
    m(Exact("hugo.toml"), "hugo", HUGO),
    m(Exact("hugo.yaml"), "hugo", HUGO),
    m(Exact("hugo.json"), "hugo", HUGO),
    m(Suffix(".tf"), "terraform", TERRAFORM),
    m(Exact("terragrunt.hcl"), "terragrunt", TERRAGRUNT),
    m(Exact("cdk.json"), "aws-cdk", CDK),
    m(Exact("samconfig.toml"), "aws-sam", SAM),
    m(Exact("serverless.yml"), "serverless", SERVERLESS),
    m(Exact("serverless.ts"), "serverless", SERVERLESS),
    m(Exact("dbt_project.yml"), "dbt", DBT),
    m(Suffix(".Rproj"), "r", RSTUDIO),
    m(Exact(".yarnrc.yml"), "yarn", YARN),
    m(Exact("dvc.yaml"), "dvc", DVC),
    m(Exact("artisan"), "laravel", LARAVEL),
    m(Exact("symfony.lock"), "symfony", SYMFONY),
];

/// Entries added to every recognised project (personal or mixed data: `Caution`).
pub static COMMON: &[ArtifactSpec] = &[
    cat(".idea", "Impostazioni di JetBrains (.idea)", Personal),
    cat(".vs", "Impostazioni di Visual Studio (.vs)", Personal),
    cat(".ipynb_checkpoints", "Checkpoint di Jupyter (.ipynb_checkpoints)", Personal),
];

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

    /// Names searched in a directory index (`DirIndex::find_dirs_named`): the last segment of
    /// every name or sub-path entry, the prefix / suffix patterns as they are, and
    /// `meson-private` when a Meson entry exists.
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
            } else {
                add(s.dir.rsplit('/').next().unwrap_or(s.dir));
            }
        }
        v
    }

    /// Lexical candidates for an index hit: `(project, rel)` for every name / sub-path entry
    /// whose segments are the last segments of `hit`, for every prefix / suffix entry the name
    /// of `hit` matches, and the Meson build dir for a `meson-private` hit. Nothing here reads
    /// the disk.
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
            } else if !s.is_plain() {
                if s.matches_rel(name)
                    && let Some(c) = tail_rel(hit, 1)
                {
                    add(c);
                }
            } else if s.dir.rsplit('/').next().is_some_and(|l| l.eq_ignore_ascii_case(name)) {
                let k = s.dir.split('/').count();
                if let Some(c) = tail_rel(hit, k).filter(|(rel, _)| rel.eq_ignore_ascii_case(s.dir)) {
                    add(c);
                }
            }
        }
        v
    }

    /// The `dir` of the catalog entry (`ArtifactKind::Catalog`) that `path` ends with, if any;
    /// the one with most segments when several do (`project/target` over `target`).
    /// Lexical, except for `*` (Meson), which needs `path/meson-private` and `path/build.ninja`.
    pub fn dir_match(&self, path: &Path) -> Option<&'static str> {
        let mut best: Option<(usize, &'static str)> = None;
        for s in self.specs().filter(|s| s.kind == ArtifactKind::Catalog) {
            let k = if s.is_plain() { s.dir.split('/').count() } else { 1 };
            if best.is_some_and(|(bk, _)| bk >= k) {
                continue;
            }
            let Some((rel, _)) = tail_rel(path, k) else { continue };
            if s.matches_rel(&rel) && (!s.is_meson() || identity_ok(s.identity, path)) {
                best = Some((k, s.dir));
            }
        }
        best.map(|(_, d)| d)
    }
}

/// The `dir` of every catalog entry (`ArtifactKind::Catalog`) of the production table, once.
pub fn catalog_dirs() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in TABLE.specs().filter(|s| s.kind == ArtifactKind::Catalog) {
        if !v.contains(&s.dir) {
            v.push(s.dir);
        }
    }
    v
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
