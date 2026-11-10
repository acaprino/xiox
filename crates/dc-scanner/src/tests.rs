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
        assert!(!a.ambiguous && !a.preselected && a.note.is_empty());
    }
}

#[test]
fn marker_table_classes() {
    let mut seen = 0;
    for m in markers::MARKERS {
        for s in m.artifacts {
            let want = match s.dir {
                "target" | "build" | "bin" | "obj" => Class::Build,
                "node_modules" | ".venv" | "venv" => Class::Dependency,
                "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" | ".gradle" => Class::Cache,
                d => panic!("no class expected for {d}"),
            };
            assert_eq!(s.class, want, "{} ({})", s.dir, m.ecosystem);
            seen += 1;
        }
    }
    assert!(seen >= 15, "{seen}");
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
                names.iter().any(|x| x.eq_ignore_ascii_case(&n))
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
