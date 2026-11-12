use super::*;
use std::fs;
use std::time::{Duration, UNIX_EPOCH};

fn write(p: impl AsRef<Path>, bytes: usize) {
    let p = p.as_ref();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

fn opts() -> ScanOptions {
    ScanOptions { min_artifact_bytes: 0, ..Default::default() }
}

fn scan(root: &Path, o: &ScanOptions) -> Loaded<ProjectCandidate> {
    DefaultScanner.scan(&[root.to_path_buf()], o, None)
}

fn rust_project(dir: &Path) {
    write(dir.join("Cargo.toml"), 20);
    write(dir.join("src/main.rs"), 20);
    write(dir.join("target/CACHEDIR.TAG"), 10);
    write(dir.join("target/debug/a.bin"), 10_000);
}

fn node_project(dir: &Path) {
    write(dir.join("package.json"), 20);
    write(dir.join("node_modules/.package-lock.json"), 10);
    write(dir.join("node_modules/pkg/index.js"), 5_000);
}

fn set_mtime(p: &Path, secs: u64) {
    let f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}

fn find<'a>(l: &'a Loaded<ProjectCandidate>, root: &Path) -> &'a ProjectCandidate {
    l.items.iter().find(|c| path_key(&c.root) == path_key(root)).unwrap_or_else(|| panic!("no candidate for {root:?}: {:?}", l.items.iter().map(|c| &c.root).collect::<Vec<_>>()))
}

fn make_link(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn rust_only() {
    let t = tempfile::tempdir().unwrap();
    let proj = t.path().join("proj");
    rust_project(&proj);
    let r = scan(t.path(), &opts());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.items.len(), 1);
    let c = &r.items[0];
    assert_eq!(c.name, "proj");
    assert_eq!(c.ecosystems, vec!["rust"]);
    assert_eq!(c.artifacts.len(), 1);
    let a = &c.artifacts[0];
    assert_eq!(a.kind, ArtifactKind::RustTarget);
    assert!(a.verified);
    assert_eq!(a.size.logical_bytes, 10_010);
    assert_eq!(a.size.file_count, 2);
    assert!(a.size.allocated_bytes.unwrap() >= a.size.logical_bytes);
    assert!(c.source_mtime_ms.is_some());
}

#[test]
fn node_only() {
    let t = tempfile::tempdir().unwrap();
    node_project(&t.path().join("web"));
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 1);
    let a = &r.items[0].artifacts[0];
    assert_eq!(a.kind, ArtifactKind::NodeModules);
    assert!(a.verified);
    assert_eq!(a.size.file_count, 2);
    assert_eq!(r.items[0].ecosystems, vec!["node"]);
}

#[test]
fn tauri_mixed_yields_both_and_scopes_mtime() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    node_project(&repo);
    write(repo.join("src-tauri/Cargo.toml"), 20);
    write(repo.join("src-tauri/target/CACHEDIR.TAG"), 10);
    write(repo.join("src-tauri/target/x.bin"), 3_000);
    // repo sources are old; the nested crate, node_modules and target are "new" and must NOT
    // leak into the repo's source mtime.
    set_mtime(&repo.join("package.json"), 1_577_836_800); // 2020-01-01
    set_mtime(&repo.join("node_modules/pkg/index.js"), 2_000_000_000);
    set_mtime(&repo.join("src-tauri/Cargo.toml"), 2_000_000_000);

    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 2, "{:?}", r.items.iter().map(|c| &c.root).collect::<Vec<_>>());
    let node = find(&r, &repo);
    assert_eq!(node.ecosystems, vec!["node"]);
    assert_eq!(node.artifacts[0].kind, ArtifactKind::NodeModules);
    assert_eq!(node.source_mtime_ms, Some(1_577_836_800_000));
    let rust = find(&r, &repo.join("src-tauri"));
    assert_eq!(rust.artifacts[0].kind, ArtifactKind::RustTarget);
    assert_eq!(rust.artifacts[0].path, repo.join("src-tauri").join("target"));
    assert_eq!(rust.source_mtime_ms, Some(2_000_000_000_000));
}

#[test]
fn several_markers_one_root_merge() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("poly");
    rust_project(&p);
    node_project(&p);
    write(p.join("pyproject.toml"), 10);
    write(p.join(".venv/pyvenv.cfg"), 10);
    write(p.join("__pycache__/m.pyc"), 100);
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 1);
    let c = &r.items[0];
    assert_eq!(c.ecosystems, vec!["rust", "node", "python"]);
    let kinds: Vec<_> = c.artifacts.iter().map(|a| a.kind).collect();
    for k in [ArtifactKind::RustTarget, ArtifactKind::NodeModules, ArtifactKind::PythonVenv, ArtifactKind::PythonCache] {
        assert!(kinds.contains(&k), "missing {k:?} in {kinds:?}");
    }
    assert!(c.artifacts.iter().all(|a| a.verified));
    // Class from the marker table; the engine assesses level/note/preselected later.
    for a in &c.artifacts {
        let want = match a.kind {
            ArtifactKind::RustTarget => Class::Build,
            ArtifactKind::NodeModules | ArtifactKind::PythonVenv => Class::Dependency,
            ArtifactKind::PythonCache => Class::Cache,
            k => panic!("unexpected {k:?}"),
        };
        assert_eq!(a.class, want, "{:?}", a.path);
        assert_eq!(a.id, path_key(&a.path));
        assert_eq!(a.level, a.class.base_level());
        assert!(!a.ambiguous && !a.preselected);
        // the catalog hint of a Dependency goes into `note`
        assert_eq!(a.note.is_empty(), a.class != Class::Dependency, "{:?}: {}", a.path, a.note);
    }
}

#[test]
fn marker_table_shape() {
    let legacy = |k: ArtifactKind| {
        matches!(
            k,
            ArtifactKind::RustTarget
                | ArtifactKind::NodeModules
                | ArtifactKind::PythonVenv
                | ArtifactKind::PythonCache
                | ArtifactKind::Gradle
                | ArtifactKind::Maven
                | ArtifactKind::DotNet
                | ArtifactKind::CMake
        )
    };
    let mut seen = 0;
    for m in markers::MARKERS {
        for (i, s) in m.artifacts.iter().chain(markers::COMMON).enumerate() {
            assert!(s.kind == ArtifactKind::Catalog || legacy(s.kind), "{}", s.dir);
            assert!(s.dir.split('/').count() <= 3, "{}", s.dir);
            assert!(!s.label.is_empty() && !s.label.starts_with("AGG"), "{}", s.dir);
            // «<Cosa> <Ecosistema> (<cartella>)»; Meson ha una cartella di build dal nome libero.
            if s.dir != "*" {
                assert!(s.label.ends_with(&format!("({})", s.dir)), "label of {}: {}", s.dir, s.label);
                assert!(s.label.chars().next().is_some_and(char::is_uppercase), "label of {}: {}", s.dir, s.label);
            }
            assert_eq!(s.hint.is_empty(), s.class != Class::Dependency, "hint of {} ({})", s.dir, m.ecosystem);
            assert_eq!(s.identity == Identity::MesonBuild, s.dir == "*", "{}", s.dir);
            let dup = m.artifacts.iter().chain(markers::COMMON).skip(i + 1).any(|o| o.dir.eq_ignore_ascii_case(s.dir));
            assert!(!dup, "{} twice for {}", s.dir, m.ecosystem);
            seen += 1;
        }
    }
    assert!(seen >= 150, "{seen}");
}

/// The † names of the spec table, and nothing else.
#[test]
fn ambiguous_names_are_the_dagger_ones() {
    let mut amb: Vec<&str> = markers::MARKERS
        .iter()
        .flat_map(|m| m.artifacts.iter())
        .chain(markers::COMMON)
        .filter(|s| s.ambiguous)
        .map(|s| s.dir)
        .collect();
    amb.sort();
    amb.dedup();
    assert_eq!(amb, vec!["artifacts", "build", "dist", "out", "public", "site", "vendor", "vendor/bundle"]);
}

/// The Dependency hints of the plan, word for word.
#[test]
fn dependency_hints() {
    let want: &[(&str, &str, &str)] = &[
        ("node", "node_modules", "Si riscarica con npm install (o pnpm, yarn, bun install)"),
        ("python", ".venv", "Si ricrea con python -m venv e l'installazione dei requisiti"),
        ("python", "venv", "Si ricrea con python -m venv e l'installazione dei requisiti"),
        ("python", ".tox", "Si ricrea al prossimo tox/nox"),
        ("python", ".nox", "Si ricrea al prossimo tox/nox"),
        ("python", "__pypackages__", "Si riscarica con pdm install"),
        ("terraform", ".terraform", "Si riscarica con terraform init"),
        ("go", "vendor", "Si ricrea con go mod vendor"),
        ("php", "vendor", "Si riscarica con composer install"),
        ("ruby", "vendor/bundle", "Si riscarica con bundle install"),
        ("elixir", "deps", "Si riscarica con mix deps.get"),
        ("ocaml", "_opam", "Si ricrea con opam install . --deps-only"),
        ("vcpkg", "vcpkg_installed", "Si ricrea con vcpkg install"),
        ("platformio", ".pio", "Si riscarica alla prossima build PlatformIO"),
        ("swift", ".build", "Si ricrea con swift build"),
        ("dbt", "dbt_packages", "Si riscarica con dbt deps"),
    ];
    let mut deps = 0;
    for m in markers::MARKERS {
        for s in m.artifacts.iter().filter(|s| s.class == Class::Dependency) {
            let w = want.iter().find(|(e, d, _)| *e == m.ecosystem && *d == s.dir);
            assert_eq!(w.map(|w| w.2), Some(s.hint), "{} ({})", s.dir, m.ecosystem);
            deps += 1;
        }
    }
    assert!(deps >= want.len());
    for (e, d, _) in want {
        assert!(markers::MARKERS.iter().any(|m| m.ecosystem == *e && m.artifacts.iter().any(|s| s.dir == *d)), "{d} ({e})");
    }
}

/// The Attenzione entries of the spec are Personal, with the markers of the plan.
#[test]
fn personal_entries() {
    let common: Vec<&str> = markers::COMMON.iter().map(|s| s.dir).collect();
    assert_eq!(common, vec![".idea", ".vs", ".ipynb_checkpoints"]);
    assert!(markers::COMMON.iter().all(|s| s.class == Class::Personal));
    let want: &[(&str, &[&str])] = &[
        ("python", &[".hypothesis", "wandb", "mlruns", "runs", "checkpoint-*"]),
        ("r", &[".Rproj.user"]),
        ("yarn", &[".yarn/cache"]),
        ("unreal", &["Saved", "Binaries"]),
        ("unity", &["UserSettings"]),
        ("dvc", &[".dvc/cache"]),
        ("laravel", &["bootstrap/cache", "storage/framework/cache", "storage/framework/views"]),
        ("symfony", &["var/cache"]),
    ];
    for m in markers::MARKERS {
        let mut got: Vec<&str> = m.artifacts.iter().filter(|s| s.class == Class::Personal).map(|s| s.dir).collect();
        let mut exp: Vec<&str> = want.iter().find(|(e, _)| *e == m.ecosystem).map(|w| w.1.to_vec()).unwrap_or_default();
        got.sort();
        exp.sort();
        assert_eq!(got, exp, "{}", m.ecosystem);
    }
}

#[test]
fn nested_worktree_like_dirs_each_get_a_candidate() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    rust_project(&repo);
    let wt1 = repo.join(".claude/worktrees/wt1");
    let wt2 = repo.join(".claude/worktrees/wt2");
    rust_project(&wt1);
    rust_project(&wt2);
    // newer file in a worktree must not count towards the parent repo.
    set_mtime(&repo.join("Cargo.toml"), 1_600_000_000);
    set_mtime(&repo.join("src/main.rs"), 1_600_000_000);

    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 3);
    for root in [&repo, &wt1, &wt2] {
        let c = find(&r, root);
        assert_eq!(c.artifacts.len(), 1);
        assert_eq!(c.artifacts[0].path, root.join("target"));
    }
    assert_eq!(find(&r, &repo).source_mtime_ms, Some(1_600_000_000_000));

    // dot dirs excluded -> worktrees unreachable.
    let r = scan(t.path(), &ScanOptions { include_dot_dirs: false, ..opts() });
    assert_eq!(r.items.len(), 1);
    assert_eq!(path_key(&r.items[0].root), path_key(&repo));
}

#[test]
fn target_identity_verified_or_not() {
    let t = tempfile::tempdir().unwrap();
    let bare = t.path().join("bare");
    write(bare.join("Cargo.toml"), 10);
    write(bare.join("target/stuff.txt"), 10);
    let info = t.path().join("info");
    write(info.join("Cargo.toml"), 10);
    write(info.join("target/.rustc_info.json"), 10);
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 2);
    assert!(!find(&r, &bare).artifacts[0].verified);
    assert!(find(&r, &info).artifacts[0].verified);
}

#[test]
fn node_modules_and_venv_identity() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("p");
    write(p.join("package.json"), 10);
    write(p.join("node_modules/random/file.js"), 10); // no identity file
    write(p.join("requirements.txt"), 10);
    write(p.join("venv/lib/x.py"), 10); // no pyvenv.cfg
    let r = scan(t.path(), &opts());
    let c = find(&r, &p);
    assert_eq!(c.artifacts.len(), 2);
    assert!(c.artifacts.iter().all(|a| !a.verified));
    // pnpm layout
    let q = t.path().join("q");
    write(q.join("package.json"), 10);
    write(q.join("node_modules/.pnpm/x/y.js"), 10);
    let r = scan(&q, &opts());
    assert!(r.items[0].artifacts[0].verified);
}

#[test]
fn maven_target_and_gradle_dotnet_cmake() {
    let t = tempfile::tempdir().unwrap();
    let m = t.path().join("m");
    write(m.join("pom.xml"), 10);
    write(m.join("target/classes/A.class"), 10);
    let g = t.path().join("g");
    write(g.join("build.gradle.kts"), 10);
    write(g.join("build/out.jar"), 10);
    write(g.join(".gradle/cache.bin"), 10);
    let d = t.path().join("d");
    write(d.join("App.csproj"), 10);
    write(d.join("bin/Debug/App.dll"), 10);
    write(d.join("obj/x.json"), 10);
    let c = t.path().join("c");
    write(c.join("CMakeLists.txt"), 10);
    write(c.join("build/CMakeCache.txt"), 10);
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 4);
    let ma = &find(&r, &m).artifacts;
    assert_eq!((ma[0].kind, ma[0].verified), (ArtifactKind::Maven, true));
    assert_eq!(find(&r, &g).artifacts.len(), 2);
    assert!(find(&r, &g).artifacts.iter().all(|a| a.kind == ArtifactKind::Gradle && a.verified));
    assert_eq!(find(&r, &d).artifacts.len(), 2);
    assert!(find(&r, &d).artifacts.iter().all(|a| a.kind == ArtifactKind::DotNet));
    assert_eq!(find(&r, &c).artifacts[0].kind, ArtifactKind::CMake);
}

#[test]
fn does_not_descend_into_artifact_dirs() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("p");
    node_project(&p);
    // a package inside node_modules with its own package.json and node_modules
    write(p.join("node_modules/dep/package.json"), 10);
    write(p.join("node_modules/dep/node_modules/.package-lock.json"), 10);
    write(p.join("node_modules/dep/node_modules/sub/a.js"), 10);
    // cargo target full of crates
    rust_project(&p);
    write(p.join("target/debug/build/foo/Cargo.toml"), 10);
    write(p.join("target/debug/build/foo/target/CACHEDIR.TAG"), 10);
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 1, "{:?}", r.items.iter().map(|c| &c.root).collect::<Vec<_>>());
    assert_eq!(r.items[0].artifacts.len(), 2);
}

#[test]
fn npm_workspaces_and_cargo_members() {
    let t = tempfile::tempdir().unwrap();
    let w = t.path().join("mono");
    node_project(&w);
    node_project(&w.join("packages/a"));
    node_project(&w.join("packages/b"));
    write(w.join("Cargo.toml"), 10);
    write(w.join("crates/x/Cargo.toml"), 10);
    write(w.join("crates/x/target/CACHEDIR.TAG"), 10);
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 4);
    assert_eq!(find(&r, &w).ecosystems, vec!["rust", "node"]);
    assert_eq!(find(&r, &w.join("crates/x")).artifacts[0].kind, ArtifactKind::RustTarget);
}

#[test]
fn links_are_not_followed() {
    let t = tempfile::tempdir().unwrap();
    let real = t.path().join("elsewhere");
    rust_project(&real.join("proj"));
    let scan_root = t.path().join("ws");
    fs::create_dir_all(&scan_root).unwrap();
    // directory link to a tree holding a project: not entered
    if !make_link(&scan_root.join("lnk"), &real) {
        eprintln!("skipping: cannot create link");
        return;
    }
    // project whose node_modules is a link to a real directory: artifact not reported
    let p = scan_root.join("web");
    write(p.join("package.json"), 10);
    let store = t.path().join("store");
    write(store.join(".modules.yaml"), 10);
    write(store.join("big.js"), 100_000);
    assert!(make_link(&p.join("node_modules"), &store));
    let r = scan(&scan_root, &opts());
    assert!(r.items.is_empty(), "{:?}", r.items.iter().map(|c| (&c.root, c.artifacts.len())).collect::<Vec<_>>());

    // a link inside an artifact is not followed when sizing
    let q = scan_root.join("rs");
    rust_project(&q);
    assert!(make_link(&q.join("target").join("debug").join("lnk"), &store));
    let r = scan(&q, &opts());
    assert_eq!(r.items[0].artifacts[0].size.logical_bytes, 10_010);
}

#[test]
fn ignore_list_uses_path_key() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    rust_project(&a);
    rust_project(&b);
    let mut ignore = a.to_string_lossy().to_uppercase();
    ignore.push('\\');
    let r = scan(t.path(), &ScanOptions { ignore: vec![PathBuf::from(ignore)], ..opts() });
    if cfg!(windows) {
        assert_eq!(r.items.len(), 1);
        assert_eq!(path_key(&r.items[0].root), path_key(&b));
    }
    // exact ignore of an artifact dir hides that artifact only
    let r = scan(t.path(), &ScanOptions { ignore: vec![b.join("target")], ..opts() });
    assert_eq!(r.items.len(), 1);
    assert_eq!(path_key(&r.items[0].root), path_key(&a));
}

#[test]
fn min_size_filter() {
    let t = tempfile::tempdir().unwrap();
    rust_project(&t.path().join("small"));
    let big = t.path().join("big");
    write(big.join("Cargo.toml"), 10);
    write(big.join("target/CACHEDIR.TAG"), 10);
    write(big.join("target/huge.bin"), 2 << 20);
    let r = scan(t.path(), &ScanOptions { min_artifact_bytes: 1 << 20, ..Default::default() });
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].name, "big");
    assert!(r.items[0].artifacts[0].size.effective_bytes() >= 2 << 20);
}

#[test]
fn max_depth_limits_discovery_not_mtime() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("a/b/proj");
    rust_project(&p);
    write(p.join("src/deep/er/est/file.rs"), 5);
    set_mtime(&p.join("src/deep/er/est/file.rs"), 2_100_000_000);
    assert!(scan(t.path(), &ScanOptions { max_depth: Some(2), ..opts() }).items.is_empty());
    let r = scan(t.path(), &ScanOptions { max_depth: Some(3), ..opts() });
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].source_mtime_ms, Some(2_100_000_000_000));
}

#[test]
fn system_dirs_skipped_but_not_lookalikes() {
    let t = tempfile::tempdir().unwrap();
    rust_project(&t.path().join("$Recycle.Bin/p"));
    rust_project(&t.path().join("System Volume Information/p"));
    rust_project(&t.path().join("flutter_app/windows/p")); // not directly under a drive root
    let r = scan(t.path(), &opts());
    assert_eq!(r.items.len(), 1);
    assert!(r.items[0].root.ends_with("flutter_app\\windows\\p") || r.items[0].root.ends_with("flutter_app/windows/p"));
}

#[test]
fn missing_root_is_a_warning() {
    let t = tempfile::tempdir().unwrap();
    let r = scan(&t.path().join("nope"), &opts());
    assert!(r.items.is_empty());
    assert_eq!(r.warnings.len(), 1);
}

#[test]
fn overlapping_roots_do_not_duplicate() {
    let t = tempfile::tempdir().unwrap();
    rust_project(&t.path().join("p"));
    let r = DefaultScanner.scan(&[t.path().to_path_buf(), t.path().join("p"), t.path().to_path_buf()], &opts(), None);
    assert_eq!(r.items.len(), 1);
}

// ───────────────────────────── index mode ─────────────────────────────

struct FakeIndex {
    dirs: Vec<IndexedDir>,
}

impl DirIndex for FakeIndex {
    fn backend_name(&self) -> &'static str {
        "fake"
    }
    fn size_of(&self, dir: &Path) -> Option<SizeInfo> {
        self.dirs.iter().find(|d| path_key(&d.path) == path_key(dir)).map(|d| d.size.clone())
    }
    fn find_dirs_named(&self, names: &[&str], roots: &[PathBuf]) -> Vec<IndexedDir> {
        self.dirs
            .iter()
            .filter(|d| {
                let n = d.path.file_name().unwrap().to_string_lossy();
                names.iter().any(|x| dc_model::name_matches(x, &n))
                    && roots.iter().any(|r| path_key(&d.path).starts_with(&path_key(r)))
            })
            .cloned()
            .collect()
    }
}

fn gib(n: u64) -> SizeInfo {
    SizeInfo { logical_bytes: n << 30, allocated_bytes: Some(n << 30), file_count: 1000 * n }
}

#[test]
fn index_mode_validates_on_disk_and_uses_index_sizes() {
    let t = tempfile::tempdir().unwrap();
    let ws = t.path();
    let a = ws.join("a");
    rust_project(&a);
    let b = ws.join("b");
    node_project(&b);
    write(b.join("node_modules/pkg/package.json"), 10);
    write(b.join("node_modules/pkg/node_modules/z.js"), 10);
    let c = ws.join("c"); // no marker file: `build` is not ours
    write(c.join("build/x.o"), 10);
    let d = ws.join("d"); // marker present, artifact gone (stale index)
    write(d.join("Cargo.toml"), 10);
    let e = ws.join("e"); // unverified target
    write(e.join("Cargo.toml"), 10);
    write(e.join("target/y"), 10);
    let skipped = ws.join("$Recycle.Bin/f");
    rust_project(&skipped);

    let ix = FakeIndex {
        dirs: vec![
            IndexedDir { path: a.join("target"), size: gib(5) },
            IndexedDir { path: b.join("node_modules"), size: gib(2) },
            IndexedDir { path: b.join("node_modules/pkg/node_modules"), size: gib(1) },
            IndexedDir { path: c.join("build"), size: gib(3) },
            IndexedDir { path: d.join("target"), size: gib(4) },
            IndexedDir { path: e.join("target"), size: gib(1) },
            IndexedDir { path: skipped.join("target"), size: gib(9) },
        ],
    };
    let r = DefaultScanner.scan(&[ws.to_path_buf()], &ScanOptions::default(), Some(&ix));
    let mut roots: Vec<_> = r.items.iter().map(|c| path_key(&c.root)).collect();
    roots.sort();
    let mut want = vec![path_key(&a), path_key(&b), path_key(&e)];
    want.sort();
    assert_eq!(roots, want, "{:?}", r.items);

    let ca = find(&r, &a);
    assert_eq!(ca.artifacts[0].size, gib(5)); // from the index, not from disk
    assert!(ca.artifacts[0].verified);
    assert!(ca.source_mtime_ms.is_some());
    assert_eq!(find(&r, &b).artifacts.len(), 1);
    assert_eq!(find(&r, &b).artifacts[0].size, gib(2));
    assert!(!find(&r, &e).artifacts[0].verified);
    assert!(r.warnings.iter().any(|w| w.contains("stale")), "{:?}", r.warnings);

    // min size applies to index sizes too
    let r = DefaultScanner.scan(&[ws.to_path_buf()], &ScanOptions { min_artifact_bytes: 3 << 30, ..Default::default() }, Some(&ix));
    assert_eq!(r.items.len(), 1);
    assert_eq!(path_key(&r.items[0].root), path_key(&a));
}

#[test]
fn index_mode_honours_ignore_and_depth() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("x/y/a");
    rust_project(&a);
    let ix = FakeIndex { dirs: vec![IndexedDir { path: a.join("target"), size: gib(1) }] };
    let go = |o: ScanOptions| DefaultScanner.scan(&[t.path().to_path_buf()], &o, Some(&ix)).items.len();
    assert_eq!(go(opts()), 1);
    assert_eq!(go(ScanOptions { max_depth: Some(2), ..opts() }), 0);
    assert_eq!(go(ScanOptions { max_depth: Some(3), ..opts() }), 1);
    assert_eq!(go(ScanOptions { ignore: vec![t.path().join("x/y")], ..opts() }), 0);
}

// ───────────────────────────── catalog as data (test table) ─────────────────────────────

use markers::{ArtifactSpec, Identity, Marker, Pattern, Table};

const fn cat(dir: &'static str, label: &'static str, class: Class) -> ArtifactSpec {
    ArtifactSpec { dir, kind: ArtifactKind::Catalog, label, identity: Identity::ParentMarker, class, ambiguous: false, hint: "" }
}

static T_NX: &[ArtifactSpec] = &[cat(".nx/cache", "Cache Nx (.nx/cache)", Class::Cache)];
static T_UNITY: &[ArtifactSpec] = &[cat("Library", "Unity Library", Class::Cache)];
static T_STORYBOOK: &[ArtifactSpec] = &[cat("storybook-static", "Storybook", Class::Build)];
static T_MESON: &[ArtifactSpec] =
    &[ArtifactSpec { identity: Identity::MesonBuild, ..cat("*", "Build Meson", Class::Build) }];
static T_PYTHON: &[ArtifactSpec] = &[
    cat("checkpoint-*", "Checkpoint", Class::Personal),
    ArtifactSpec { kind: ArtifactKind::PythonCache, ..cat("__pycache__", "Cache Python (__pycache__)", Class::Cache) },
];
static T_NODE: &[ArtifactSpec] = &[ArtifactSpec { ambiguous: true, ..cat("build", "Build Node", Class::Build) }];
static T_CMAKE: &[ArtifactSpec] =
    &[ArtifactSpec { kind: ArtifactKind::CMake, ..cat("build", "Build CMake (build)", Class::Build) }];
static T_SPHINX: &[ArtifactSpec] = &[cat("_build", "Sphinx _build", Class::Build)];
static T_RUBY: &[ArtifactSpec] =
    &[ArtifactSpec { hint: "bundle install", ..cat("vendor/bundle", "Gem (vendor/bundle)", Class::Dependency) }];

static T_MARKERS: &[Marker] = &[
    Marker { pattern: Pattern::Exact("nx.json"), ecosystem: "nx", artifacts: T_NX },
    Marker { pattern: Pattern::Sub("ProjectSettings/ProjectVersion.txt"), ecosystem: "unity", artifacts: T_UNITY },
    Marker { pattern: Pattern::Dir(".storybook"), ecosystem: "storybook", artifacts: T_STORYBOOK },
    Marker { pattern: Pattern::Exact("meson.build"), ecosystem: "meson", artifacts: T_MESON },
    Marker { pattern: Pattern::Exact("pyproject.toml"), ecosystem: "python", artifacts: T_PYTHON },
    Marker { pattern: Pattern::Exact("package.json"), ecosystem: "node", artifacts: T_NODE },
    Marker { pattern: Pattern::Exact("CMakeLists.txt"), ecosystem: "cmake", artifacts: T_CMAKE },
    Marker {
        pattern: Pattern::All(&[Pattern::Exact("conf.py"), Pattern::Exact("index.rst")]),
        ecosystem: "sphinx",
        artifacts: T_SPHINX,
    },
    Marker { pattern: Pattern::Exact("Gemfile"), ecosystem: "ruby", artifacts: T_RUBY },
];

static T_TABLE: Table = Table { markers: T_MARKERS, common: &[] };

fn tscan(root: &Path, index: Option<&dyn DirIndex>) -> Loaded<ProjectCandidate> {
    DefaultScanner::scan_table(&T_TABLE, &[root.to_path_buf()], &opts(), index)
}

/// Builds every catalog case of the brief under `ws`; returns what an MFT index would hold
/// (the searched names are filtered by `FakeIndex`; the other entries only give sizes).
fn catalog_tree(ws: &Path) -> Vec<IndexedDir> {
    let mut ix = Vec::new();
    let mut add = |p: PathBuf| ix.push(IndexedDir { path: p, size: gib(1) });
    // Nx: `.nx/cache` is the artifact; `src/cache` and a `.nx/cache` without nx.json are not.
    let nx = ws.join("nx");
    write(nx.join("nx.json"), 10);
    write(nx.join(".nx/cache/a.bin"), 100);
    write(nx.join("src/cache/keep.ts"), 10);
    write(nx.join("src/.nx/cache/keep.ts"), 10);
    add(nx.join(".nx/cache"));
    add(nx.join("src/cache"));
    add(nx.join("src/.nx/cache"));
    // Unity: the marker is a file in a subfolder.
    let u = ws.join("unity");
    write(u.join("ProjectSettings/ProjectVersion.txt"), 10);
    write(u.join("Library/x.asset"), 100);
    add(u.join("Library"));
    let nu = ws.join("not-unity"); // `Library` without the marker file
    write(nu.join("ProjectSettings/other.txt"), 10);
    write(nu.join("Library/x.asset"), 100);
    add(nu.join("Library"));
    // Storybook: the marker is a folder.
    let sb = ws.join("sb");
    write(sb.join(".storybook/main.js"), 10);
    write(sb.join("storybook-static/index.html"), 100);
    add(sb.join("storybook-static"));
    // Meson: any direct child holding meson-private/ and build.ninja.
    let me = ws.join("meson");
    write(me.join("meson.build"), 10);
    write(me.join("builddir/meson-private/coredata.dat"), 10);
    write(me.join("builddir/build.ninja"), 100);
    write(me.join("other/file.c"), 10);
    add(me.join("builddir/meson-private"));
    add(me.join("builddir"));
    // Python: prefix entry `checkpoint-*`, found next to an indexed `__pycache__`.
    let py = ws.join("py");
    write(py.join("pyproject.toml"), 10);
    write(py.join("checkpoint-500/model.bin"), 100);
    write(py.join("checkpointless/notes.txt"), 10);
    write(py.join("__pycache__/m.pyc"), 10);
    add(py.join("checkpoint-500"));
    add(py.join("__pycache__"));
    // `build`: † for Node, plain for CMake.
    let mix = ws.join("mix");
    write(mix.join("package.json"), 10);
    write(mix.join("CMakeLists.txt"), 10);
    write(mix.join("build/out.o"), 100);
    add(mix.join("build"));
    let cm = ws.join("cmake");
    write(cm.join("CMakeLists.txt"), 10);
    write(cm.join("build/out.o"), 100);
    add(cm.join("build"));
    // Sphinx: `All` needs both conf.py and index.rst.
    let sp = ws.join("sphinx");
    write(sp.join("conf.py"), 10);
    write(sp.join("index.rst"), 10);
    write(sp.join("_build/html/index.html"), 100);
    add(sp.join("_build"));
    let half = ws.join("half-sphinx");
    write(half.join("conf.py"), 10);
    write(half.join("_build/x.html"), 100);
    add(half.join("_build"));
    // Multi-segment Dependency entry with a hint.
    let rb = ws.join("rb");
    write(rb.join("Gemfile"), 10);
    write(rb.join("vendor/bundle/gem.rb"), 100);
    add(rb.join("vendor/bundle"));
    ix
}

fn check_catalog(r: &Loaded<ProjectCandidate>, ws: &Path) {
    let roots: Vec<String> = r.items.iter().map(|c| path_key(&c.root)).collect();
    let arts = |root: &str| -> Vec<&Artifact> {
        let mut v: Vec<&Artifact> = find(r, &ws.join(root)).artifacts.iter().collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    };
    let rel = |a: &Artifact, root: &str| -> String {
        a.path.strip_prefix(ws.join(root)).unwrap().to_string_lossy().replace('\\', "/").to_lowercase()
    };

    let nx = arts("nx");
    assert_eq!(nx.len(), 1, "{nx:?}");
    assert_eq!(rel(nx[0], "nx"), ".nx/cache");
    assert_eq!((nx[0].kind, nx[0].class, nx[0].label.as_str()), (ArtifactKind::Catalog, Class::Cache, "Cache Nx (.nx/cache)"));
    assert!(nx[0].verified && !nx[0].ambiguous);
    assert!(!roots.contains(&path_key(&ws.join("nx/src"))), "{roots:?}");

    let u = arts("unity");
    assert_eq!((u.len(), rel(u[0], "unity").as_str()), (1, "library"));
    assert!(!roots.contains(&path_key(&ws.join("not-unity"))), "{roots:?}");

    let sb = arts("sb");
    assert_eq!((sb.len(), rel(sb[0], "sb").as_str()), (1, "storybook-static"));

    let me = arts("meson");
    assert_eq!(me.len(), 1, "{me:?}");
    assert_eq!(rel(me[0], "meson"), "builddir");
    assert_eq!(me[0].label, "Build Meson");
    assert!(me[0].verified);

    let py = arts("py");
    let py_rels: Vec<String> = py.iter().map(|a| rel(a, "py")).collect();
    assert_eq!(py_rels, vec!["__pycache__", "checkpoint-500"]);
    assert_eq!(py[1].class, Class::Personal);

    let mix = arts("mix");
    assert_eq!(mix.len(), 1);
    assert!(mix[0].ambiguous, "a † entry of the project claims `build`");
    let cm = arts("cmake");
    assert_eq!(cm.len(), 1);
    assert!(!cm[0].ambiguous, "no † entry: `build` stays as today");
    assert_eq!(cm[0].kind, ArtifactKind::CMake);

    let sp = arts("sphinx");
    assert_eq!((sp.len(), rel(sp[0], "sphinx").as_str()), (1, "_build"));
    assert!(!roots.contains(&path_key(&ws.join("half-sphinx"))), "{roots:?}");

    let rb = arts("rb");
    assert_eq!((rb.len(), rel(rb[0], "rb").as_str()), (1, "vendor/bundle"));
    assert_eq!(rb[0].note, "bundle install", "the hint goes into note");
    assert_eq!(rb[0].class, Class::Dependency);
}

#[test]
fn catalog_walker() {
    let t = tempfile::tempdir().unwrap();
    catalog_tree(t.path());
    let r = tscan(t.path(), None);
    check_catalog(&r, t.path());
    assert_eq!(r.items.len(), 9, "{:?}", r.items.iter().map(|c| &c.root).collect::<Vec<_>>());
}

#[test]
fn catalog_index() {
    let t = tempfile::tempdir().unwrap();
    let ix = FakeIndex { dirs: catalog_tree(t.path()) };
    let r = tscan(t.path(), Some(&ix));
    check_catalog(&r, t.path());
    assert_eq!(r.items.len(), 9, "{:?}", r.items.iter().map(|c| &c.root).collect::<Vec<_>>());
    // Sizes come from the index, also for the prefix and Meson entries.
    for c in &r.items {
        for a in &c.artifacts {
            assert_eq!(a.size, gib(1), "{:?}", a.path);
        }
    }
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn catalog_index_search_names() {
    let names = T_TABLE.index_names();
    for n in ["cache", "Library", "storybook-static", "meson-private", "checkpoint-*", "__pycache__", "build", "_build", "bundle"] {
        assert!(names.iter().any(|x| x.eq_ignore_ascii_case(n)), "{n} missing from {names:?}");
    }
    // sub-paths become their last segment; Meson's lone `*` is searched as `meson-private`
    assert!(!names.iter().any(|x| x.contains('/') || *x == "*"), "{names:?}");
}

#[test]
fn catalog_multi_segment_contents_do_not_count_as_sources() {
    let t = tempfile::tempdir().unwrap();
    let nx = t.path().join("nx");
    write(nx.join("nx.json"), 10);
    write(nx.join(".nx/cache/a.bin"), 100);
    set_mtime(&nx.join("nx.json"), 1_600_000_000);
    set_mtime(&nx.join(".nx/cache/a.bin"), 2_000_000_000);
    let ix = FakeIndex { dirs: vec![IndexedDir { path: nx.join(".nx/cache"), size: gib(1) }] };
    for r in [tscan(t.path(), None), tscan(t.path(), Some(&ix))] {
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].artifacts.len(), 1);
        assert_eq!(r.items[0].source_mtime_ms, Some(1_600_000_000_000));
    }
}

#[test]
fn resolve_ambiguity_and_meson_identity() {
    let t = tempfile::tempdir().unwrap();
    let both = T_TABLE.detect(t.path(), &["package.json", "CMakeLists.txt"], &["build"]).unwrap();
    let (s, verified, amb) = both.resolve("build", |_| true).unwrap();
    assert!(verified && amb);
    assert_eq!(s.kind, ArtifactKind::Catalog);
    let cmake = T_TABLE.detect(t.path(), &["CMakeLists.txt"], &["build"]).unwrap();
    let (s, verified, amb) = cmake.resolve("BUILD", |_| true).unwrap();
    assert_eq!((s.kind, verified, amb), (ArtifactKind::CMake, true, false));
    // `*` (Meson) only matches when its identity holds, and only a direct child.
    let meson = T_TABLE.detect(t.path(), &["meson.build"], &[]).unwrap();
    assert!(meson.resolve("src", |id| id != Identity::MesonBuild).is_none());
    assert!(meson.resolve("builddir", |id| id == Identity::MesonBuild).is_some());
    assert!(meson.resolve("a/b", |_| true).is_none());
    // prefix entries need something after the prefix
    let py = T_TABLE.detect(t.path(), &["pyproject.toml"], &[]).unwrap();
    assert!(py.resolve("checkpoint-", |_| true).is_none());
    assert!(py.resolve("Checkpoint-12", |_| true).is_some());
    // multi-segment entries match the whole relative path only
    let nx = T_TABLE.detect(t.path(), &["nx.json"], &[]).unwrap();
    assert!(nx.resolve(".NX/Cache", |_| true).is_some());
    assert!(nx.resolve("cache", |_| true).is_none());
    // `All` needs every pattern; `Sub` needs the file, not only its folder.
    assert!(T_TABLE.detect(t.path(), &["conf.py"], &[]).is_none());
    assert!(T_TABLE.detect(t.path(), &[], &["ProjectSettings"]).is_none());
}

#[test]
fn catalog_dir_match_in_test_table() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("p");
    assert_eq!(T_TABLE.dir_match(&p.join(".nx").join("cache")), Some(".nx/cache"));
    assert_eq!(T_TABLE.dir_match(&p.join(".NX").join("Cache")), Some(".nx/cache"));
    assert_eq!(T_TABLE.dir_match(&p.join("src").join("cache")), None);
    assert_eq!(T_TABLE.dir_match(&p.join("cache")), None);
    assert_eq!(T_TABLE.dir_match(&p.join("checkpoint-9")), Some("checkpoint-*"));
    assert_eq!(T_TABLE.dir_match(&p.join("vendor").join("bundle")), Some("vendor/bundle"));
    // only entries of kind Catalog count (`__pycache__` here is PythonCache)
    assert_eq!(T_TABLE.dir_match(&p.join("__pycache__")), None);
    // Meson needs meson-private on disk
    assert_eq!(T_TABLE.dir_match(&p.join("bd")), None);
    write(p.join("bd/meson-private/x"), 1);
    write(p.join("bd/build.ninja"), 1);
    assert_eq!(T_TABLE.dir_match(&p.join("bd")), Some("*"));
    // the project above a multi-segment entry must be a named folder, not a drive root
    let root = p.ancestors().last().unwrap().to_path_buf();
    assert_eq!(T_TABLE.dir_match(&root.join(".nx").join("cache")), None);
}

#[test]
fn never_enter_heavy_unambiguous_dirs() {
    for n in [
        ".next", ".nuxt", ".svelte-kit", ".angular", ".turbo", ".dart_tool", ".terraform", ".terragrunt-cache",
        ".stack-work", "dist-newstyle", ".zig-cache", ".pio",
    ] {
        assert!(markers::NEVER_ENTER.contains(&n), "{n}");
    }
}

#[test]
fn real_table_catalog_dirs() {
    let p = Path::new(r"D:\ws\proj");
    assert!(catalog_dir_plausible(&p.join(".nx").join("cache")));
    assert!(catalog_dir_plausible(&p.join("cypress").join("videos")));
    assert!(!catalog_dir_plausible(&p.join("videos")));
    assert!(!catalog_dir_plausible(&p.join("src")));
    assert!(!catalog_dir_plausible(&p.join("node_modules")), "node_modules is not a catalog entry");
    assert!(catalog_dir_plausible(&p.join("pkg.egg-info")));
    assert!(catalog_dir_plausible(&p.join("checkpoint-12")));
    assert!(!catalog_dir_plausible(&p.join("checkpoint-")));
    // every catalog dir of the table is accepted, as the cleaner's guard asks
    for d in catalog_dirs() {
        let rel = d.replace("*.", "x.").replace('*', "1");
        let path = markers::join_rel(p, &rel);
        if d == "*" {
            continue; // Meson: needs meson-private on disk, see catalog_dir_match_in_test_table
        }
        let got = catalog_dir_match(&path).unwrap_or_else(|| panic!("{rel} refused"));
        assert!(got.eq_ignore_ascii_case(d), "{rel}: {got}");
    }
}

// ───────────────────────────── the production catalog ─────────────────────────────

fn make_marker(dir: &Path, p: &Pattern) {
    match p {
        Pattern::Exact(n) => write(dir.join(n), 10),
        Pattern::Suffix(s) => write(dir.join(format!("proj{s}")), 10),
        Pattern::Dir(n) => write(dir.join(n).join("main.js"), 10),
        Pattern::Sub(rel) => write(markers::join_rel(dir, rel), 10),
        Pattern::All(ps) => ps.iter().for_each(|p| make_marker(dir, p)),
    }
}

/// A folder name the entry `dir` claims (`checkpoint-*` -> `checkpoint-1`, `*.egg-info` ->
/// `x.egg-info`, `*` -> a Meson build dir).
fn sample_rel(dir: &str) -> String {
    if dir == "*" { "builddir".into() } else { dir.replace("*.", "x.").replace('*', "1") }
}

fn make_artifact(project: &Path, s: &ArtifactSpec) -> PathBuf {
    let a = markers::join_rel(project, &sample_rel(s.dir));
    match s.identity {
        Identity::RustTarget => write(a.join("CACHEDIR.TAG"), 10),
        Identity::NodeModules => write(a.join(".package-lock.json"), 10),
        Identity::PythonVenv => write(a.join("pyvenv.cfg"), 10),
        Identity::MesonBuild => {
            write(a.join("meson-private/coredata.dat"), 10);
            write(a.join("build.ninja"), 10);
        }
        Identity::ParentMarker => {}
    }
    write(a.join("payload.bin"), 100);
    a
}

/// Every folder below `root`, as an MFT index would list it.
fn index_everything(root: &Path) -> FakeIndex {
    let mut dirs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                dirs.push(IndexedDir { path: e.path(), size: gib(1) });
                stack.push(e.path());
            }
        }
    }
    FakeIndex { dirs }
}

/// One project per (marker, entry) of `MARKERS`, plus the entries of `COMMON`: the scan finds
/// the folder with the entry's label, class and † flag, verified, in both modes.
#[test]
fn every_catalog_entry_is_found() {
    let t = tempfile::tempdir().unwrap();
    let mut cases: Vec<(PathBuf, &'static ArtifactSpec, &'static str)> = Vec::new();
    let mut add = |i: usize, j: usize, p: &Pattern, s: &'static ArtifactSpec, eco: &'static str| {
        let project = t.path().join(format!("m{i}_a{j}"));
        make_marker(&project, p);
        cases.push((make_artifact(&project, s), s, eco));
    };
    for (i, m) in markers::MARKERS.iter().enumerate() {
        for (j, s) in m.artifacts.iter().enumerate() {
            add(i, j, &m.pattern, s, m.ecosystem);
        }
    }
    for (j, s) in markers::COMMON.iter().enumerate() {
        add(999, j, &markers::MARKERS[0].pattern, s, "common");
    }
    let ix = index_everything(t.path());
    for (mode, r) in [("walker", scan(t.path(), &opts())), ("index", DefaultScanner.scan(&[t.path().to_path_buf()], &opts(), Some(&ix)))] {
        for (path, s, eco) in &cases {
            let project = path.ancestors().find(|a| a.parent() == Some(t.path())).unwrap();
            let c = find(&r, project);
            let a = c
                .artifacts
                .iter()
                .find(|a| path_key(&a.path) == path_key(path))
                .unwrap_or_else(|| panic!("{mode}: {} ({eco}) not found in {:?}", s.dir, c.artifacts));
            assert_eq!(
                (a.label.as_str(), a.class, a.ambiguous, a.verified, a.kind),
                (s.label, s.class, s.ambiguous, true, s.kind),
                "{mode}: {} ({eco})",
                s.dir
            );
            assert_eq!(a.note, s.hint, "{mode}: {}", s.dir);
        }
    }
}

/// Review Focus 2: a `package.json` with `node_modules` under `AppData` or inside another
/// `node_modules` is not a project.
#[test]
fn no_projects_under_appdata_or_inside_node_modules() {
    let t = tempfile::tempdir().unwrap();
    let hidden = t.path().join("Users/u/AppData/Roaming/tool");
    node_project(&hidden);
    let local = t.path().join("Users/u/appdata/Local/Temp/x");
    rust_project(&local);
    let p = t.path().join("Users/u/source/web");
    node_project(&p);
    let inner = p.join("node_modules/pkg");
    write(inner.join("package.json"), 10);
    write(inner.join("node_modules/.package-lock.json"), 10);
    write(inner.join("node_modules/z/index.js"), 10);
    let ix = index_everything(t.path());
    for r in [scan(t.path(), &opts()), DefaultScanner.scan(&[t.path().to_path_buf()], &opts(), Some(&ix))] {
        let roots: Vec<String> = r.items.iter().map(|c| path_key(&c.root)).collect();
        assert_eq!(roots, vec![path_key(&p)]);
        assert_eq!(r.items[0].artifacts.len(), 1);
    }
}

/// M5.2: `vendor` (Composer) and `vendor/bundle` (Bundler) of one project: only the outer one.
#[test]
fn nested_artifacts_of_one_project_count_once() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("app");
    write(p.join("composer.json"), 10);
    write(p.join("Gemfile"), 10);
    write(p.join("vendor/autoload.php"), 10);
    write(p.join("vendor/bundle/ruby/gem.rb"), 100);
    let ix = index_everything(t.path());
    for r in [scan(t.path(), &opts()), DefaultScanner.scan(&[t.path().to_path_buf()], &opts(), Some(&ix))] {
        let c = find(&r, &p);
        let paths: Vec<String> = c.artifacts.iter().map(|a| path_key(&a.path)).collect();
        assert_eq!(paths, vec![path_key(&p.join("vendor"))]);
        assert_eq!(c.artifacts[0].label, "Dipendenze Composer (vendor)");
    }
}

/// With the outer `vendor` on the ignore list, `vendor/bundle` is still offered (both modes).
#[test]
fn nested_artifact_stays_when_the_outer_one_is_ignored() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("app");
    write(p.join("composer.json"), 10);
    write(p.join("Gemfile"), 10);
    write(p.join("vendor/autoload.php"), 10);
    write(p.join("vendor/bundle/ruby/gem.rb"), 100);
    let ix = index_everything(t.path());
    let o = ScanOptions { ignore: vec![p.join("vendor")], ..opts() };
    for (mode, r) in [("walk", scan(t.path(), &o)), ("index", DefaultScanner.scan(&[t.path().to_path_buf()], &o, Some(&ix)))] {
        let c = find(&r, &p);
        let paths: Vec<String> = c.artifacts.iter().map(|a| path_key(&a.path)).collect();
        assert_eq!(paths, vec![path_key(&p.join("vendor").join("bundle"))], "{mode}");
    }
}

/// M5.1: in index mode a project whose only artifact is a prefix entry is found too.
#[test]
fn index_finds_a_project_with_only_a_prefix_entry() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("train");
    write(p.join("pyproject.toml"), 10);
    write(p.join("checkpoint-7/model.bin"), 100);
    write(p.join("checkpointless/x"), 10);
    let ix = index_everything(t.path());
    let r = tscan(t.path(), Some(&ix));
    let c = find(&r, &p);
    assert_eq!(c.artifacts.len(), 1);
    assert!(c.artifacts[0].path.ends_with("checkpoint-7"));
    assert_eq!(c.artifacts[0].size, gib(1));
}
