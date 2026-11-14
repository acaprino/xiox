//! Parte B, lotto «native»: cache globali di Cargo, rustup, Go, ccache e
//! sccache, vcpkg, Conan, Composer, Bundler, Hex e rebar3, Cabal, Julia, Zig,
//! PlatformIO, Minikube, Vagrant, Helm, Terraform e Pulumi, Scoop, Chocolatey,
//! WinGet e Bazel.
//!
//! Mai sul PC dell'utente: ogni prova costruisce un profilo finto in una
//! cartella temporanea e passa a `parse_cleaner` e a `collect` un `Env` finto.
//! I file si leggono da `cleaners/` con `include_str!`, senza passare da
//! `embedded.rs`.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use xiox_rules::cleanerml::parse_cleaner;
use xiox_rules::collect::{collect, running};
use xiox_rules::expand::expand;
use xiox_rules::model::{Action, Cleaner, Env, Opt, TOOL_VARS};

const XMLS: [(&str, &str); 19] = [
    ("bbag_cargo", include_str!("../../../cleaners/bbag_cargo.xml")),
    ("rustup", include_str!("../../../cleaners/rustup.xml")),
    ("bbag_go", include_str!("../../../cleaners/bbag_go.xml")),
    ("ccache", include_str!("../../../cleaners/ccache.xml")),
    ("vcpkg", include_str!("../../../cleaners/vcpkg.xml")),
    ("conan", include_str!("../../../cleaners/conan.xml")),
    ("composer", include_str!("../../../cleaners/composer.xml")),
    ("bundler", include_str!("../../../cleaners/bundler.xml")),
    ("hex", include_str!("../../../cleaners/hex.xml")),
    ("cabal", include_str!("../../../cleaners/cabal.xml")),
    ("julia", include_str!("../../../cleaners/julia.xml")),
    ("zig", include_str!("../../../cleaners/zig.xml")),
    ("platformio", include_str!("../../../cleaners/platformio.xml")),
    ("minikube", include_str!("../../../cleaners/minikube.xml")),
    ("vagrant", include_str!("../../../cleaners/vagrant.xml")),
    ("kubectl", include_str!("../../../cleaners/kubectl.xml")),
    ("terraform", include_str!("../../../cleaners/terraform.xml")),
    ("scoop", include_str!("../../../cleaners/scoop.xml")),
    ("bazel", include_str!("../../../cleaners/bazel.xml")),
];

/// Opzioni attese di ogni cleaner, nell'ordine del file, con tutte le variabili
/// d'ambiente definite.
const EXPECTED: [(&str, &[&str]); 19] = [
    (
        "bbag_cargo",
        &["downloaded_crates", "registry_src", "registry_index", "git_checkouts", "git_db"],
    ),
    ("rustup", &["downloads", "tmp"]),
    ("bbag_go", &["build", "modules"]),
    ("ccache", &["ccache", "sccache"]),
    ("vcpkg", &["binary_cache", "buildtrees", "packages", "downloads"]),
    ("conan", &["packages"]),
    ("composer", &["cache"]),
    ("bundler", &["cache"]),
    ("hex", &["hex_packages", "rebar3_hex"]),
    ("cabal", &["packages"]),
    ("julia", &["compiled"]),
    ("zig", &["global_cache"]),
    ("platformio", &["cache", "packages", "platforms"]),
    ("minikube", &["cache"]),
    ("vagrant", &["boxes"]),
    ("kubectl", &["helm_cache"]),
    ("terraform", &["terraform_plugins", "pulumi_plugins"]),
    ("scoop", &["scoop_cache", "chocolatey_cache", "winget_cache", "winget_logs"]),
    ("bazel", &["output_root"]),
];

/// File di credenziali e configurazione della tabella (🔒): non entrano mai in
/// nessuna opzione.
const LOCKED: [&str; 12] = [
    "token",
    "stored_tokens",
    "credentials.toml",
    "config.toml",
    "settings.json",
    "config",
    "gradle.properties",
    "debug.keystore",
    "adbkey",
    "keras.json",
    "auth.json",
    "config.json",
];

/// I nomi che la protezione blocca ovunque stiano, anche dentro una cartella
/// di cache.
const GUARDED_NAMES: [&str; 3] = ["token", "credentials.toml", "auth.json"];

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Profilo, AppData, LocalAppData, Temp e nome utente: tutto sotto `profile`.
fn base_map(profile: &Path) -> HashMap<String, String> {
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    HashMap::from([
        ("userprofile".to_string(), s(profile.to_path_buf())),
        ("appdata".to_string(), s(profile.join("AppData").join("Roaming"))),
        ("localappdata".to_string(), s(profile.join("AppData").join("Local"))),
        (
            "temp".to_string(),
            s(profile.join("AppData").join("Local").join("Temp")),
        ),
        ("username".to_string(), "tester".to_string()),
    ])
}

/// Ambiente con tutte le variabili degli strumenti definite (`<profilo>\tv\<nome>`).
fn full_env(profile: &Path) -> Env {
    let mut map = base_map(profile);
    for key in TOOL_VARS {
        map.insert(
            (*key).to_string(),
            profile.join("tv").join(key).to_string_lossy().into_owned(),
        );
    }
    Env(map)
}

fn load(id: &str, env: &Env) -> Cleaner {
    let &(name, xml) = XMLS
        .iter()
        .find(|(n, _)| *n == id)
        .unwrap_or_else(|| panic!("cleaner {id} non nella lista"));
    parse_cleaner(xml, name, env).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn option_of<'a>(c: &'a Cleaner, id: &str) -> &'a Opt {
    c.options
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("{}: manca l'opzione {id}", c.id))
}

fn option_ids(c: &Cleaner) -> Vec<&str> {
    c.options.iter().map(|o| o.id.as_str()).collect()
}

fn rel(profile: &Path, p: &Path) -> String {
    p.strip_prefix(profile)
        .unwrap_or_else(|_| panic!("{} fuori dal profilo", p.display()))
        .to_string_lossy()
        .replace('\\', "/")
}

/// Crea i file di `create` (contenuto `xx`, 2 byte) in un profilo finto, legge
/// `cleaner` con le variabili `extra` (nome, percorso relativo al profilo) e
/// controlla che `collect` dell'opzione `option` trovi esattamente `expect`.
fn scenario(
    cleaner: &str,
    option: &str,
    extra: &[(&str, &str)],
    create: &[&str],
    expect: &[&str],
) {
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("u");
    let mut map = base_map(&profile);
    for (key, rel_path) in extra {
        map.insert(
            (*key).to_string(),
            profile.join(rel_path).to_string_lossy().into_owned(),
        );
    }
    let env = Env(map);
    for f in create {
        write(&profile.join(f), "xx");
    }
    let c = load(cleaner, &env);
    let opt = option_of(&c, option);
    let got = collect(opt, &env, &|_| false);
    let mut files: Vec<String> = got
        .files
        .iter()
        .map(|(p, n)| {
            assert_eq!(*n, 2, "{cleaner}/{option}: dimensione di {}", p.display());
            rel(&profile, p)
        })
        .collect();
    files.sort();
    let mut want: Vec<String> = expect.iter().map(|s| (*s).to_string()).collect();
    want.sort();
    assert_eq!(files, want, "{cleaner}/{option}");
    assert_eq!(got.skipped, 0, "{cleaner}/{option}: {:?}", got.skipped_reasons);
    assert!(!got.cancelled);
}

// ---------------------------------------------------------------------------
// (1) ogni XML si legge senza avvisi
// ---------------------------------------------------------------------------

#[test]
fn every_xml_parses_with_every_option_supported() {
    assert_eq!(XMLS.len(), EXPECTED.len());
    let full = full_env(Path::new(r"C:\Users\t"));
    for ((xml_id, _), (id, ids)) in XMLS.iter().zip(EXPECTED) {
        assert_eq!(*xml_id, id);
        let c = load(id, &full);
        assert_eq!(c.id, id);
        assert_eq!(c.group, "Development & AI");
        assert_eq!(option_ids(&c), ids.to_vec(), "{id}: opzioni");
        assert!(!c.label.trim().is_empty() && !c.description.trim().is_empty(), "{id}");
        for o in &c.options {
            assert!(o.unsupported.is_none(), "{id}/{}: {:?}", o.id, o.unsupported);
            assert!(!o.actions.is_empty(), "{id}/{}: nessuna azione", o.id);
            assert!(
                !o.label.trim().is_empty() && !o.description.trim().is_empty(),
                "{id}/{}: testo mancante",
                o.id
            );
        }
    }
}

#[test]
fn minimal_environment_keeps_the_default_paths_and_drops_the_rest() {
    // Solo profilo, AppData, LocalAppData, Temp e nome utente: restano le
    // opzioni con un percorso predefinito; spariscono quelle che esistono solo
    // con una variabile (VCPKG_ROOT, TF_PLUGIN_CACHE_DIR).
    let minimal = Env(base_map(Path::new(r"C:\Users\t")));
    for (id, ids) in EXPECTED {
        let want: Vec<&str> = match id {
            "vcpkg" => vec!["binary_cache"],
            "terraform" => vec!["pulumi_plugins"],
            _ => ids.to_vec(),
        };
        let c = load(id, &minimal);
        assert_eq!(option_ids(&c), want, "{id}");
        assert!(c.options.iter().all(|o| o.unsupported.is_none()), "{id}");
    }
}

#[test]
fn bazel_root_needs_the_user_name() {
    // Senza `username` il percorso `_bazel_<utente>` non si può scrivere: si
    // scarta, non si usa alla lettera.
    let mut map = base_map(Path::new(r"C:\Users\t"));
    map.remove("username");
    let c = load("bazel", &Env(map));
    assert!(c.options.is_empty());
}

// ---------------------------------------------------------------------------
// (4) tutte le opzioni spente di default; (5) avvisi dove la tabella li chiede
// ---------------------------------------------------------------------------

#[test]
fn every_option_is_off_by_default() {
    let full = full_env(Path::new(r"C:\Users\t"));
    for (id, _) in EXPECTED {
        for o in &load(id, &full).options {
            assert!(!o.default_on, "{id}/{} è acceso di default", o.id);
        }
    }
}

#[test]
fn warnings_are_present_where_the_table_asks_for_them() {
    let full = full_env(Path::new(r"C:\Users\t"));
    let warning = |cleaner: &str, option: &str| -> String {
        option_of(&load(cleaner, &full), option)
            .warning
            .clone()
            .unwrap_or_else(|| panic!("{cleaner}/{option}: manca l'avviso"))
    };
    // Avvisi espliciti della colonna Note.
    assert!(warning("bbag_go", "modules").contains("read-only"));
    assert!(warning("conan", "packages").contains("conan create"));
    assert!(warning("vagrant", "boxes").contains("vagrant box add"));
    // Voci che si riscaricano (D): l'avviso dice come riaverle.
    for (cleaner, option, command) in [
        ("bbag_cargo", "downloaded_crates", "cargo build"),
        ("bbag_cargo", "registry_index", "cargo fetch"),
        ("bbag_cargo", "git_db", "cargo fetch"),
        ("rustup", "downloads", "rustup update"),
        ("vcpkg", "binary_cache", "vcpkg install"),
        ("vcpkg", "downloads", "vcpkg install"),
        ("composer", "cache", "composer install"),
        ("julia", "compiled", "Julia"),
        ("zig", "global_cache", "zig build"),
        ("platformio", "packages", "pio run"),
        ("platformio", "platforms", "pio run"),
        ("minikube", "cache", "minikube start"),
        ("kubectl", "helm_cache", "helm repo update"),
        ("terraform", "terraform_plugins", "terraform init"),
        ("terraform", "pulumi_plugins", "pulumi plugin install"),
        ("scoop", "scoop_cache", "scoop install"),
        ("scoop", "chocolatey_cache", "choco install"),
        ("scoop", "winget_cache", "winget install"),
        ("bazel", "output_root", "bazel shutdown"),
    ] {
        let text = warning(cleaner, option);
        assert!(text.contains(command), "{cleaner}/{option}: «{text}» senza «{command}»");
    }
}

#[test]
fn running_processes_block_the_cleaners_that_the_table_names() {
    let full = full_env(Path::new(r"C:\Users\t"));
    let active = |names: &[&str]| -> Vec<String> { names.iter().map(|s| (*s).to_string()).collect() };
    assert_eq!(
        running(&load("ccache", &full), &active(&["sccache.exe", "notepad.exe"])),
        vec!["sccache.exe"]
    );
    assert_eq!(running(&load("bazel", &full), &active(&["bazel.exe"])), vec!["bazel.exe"]);
    assert_eq!(running(&load("bbag_go", &full), &active(&["go.exe"])), vec!["go.exe"]);
    assert_eq!(
        running(&load("bbag_cargo", &full), &active(&["cargo.exe", "rustc.exe"])),
        vec!["cargo.exe", "rustc.exe"]
    );
    assert!(running(&load("bazel", &full), &active(&["notepad.exe"])).is_empty());
}

#[test]
fn visible_texts_are_clean() {
    let full = full_env(Path::new(r"C:\Users\t"));
    for (id, xml) in XMLS {
        let low = xml.to_lowercase();
        assert!(!low.contains("bleachbit"), "{id}");
        let c = load(id, &full);
        let mut texts = vec![c.label.clone(), c.description.clone()];
        for o in &c.options {
            assert!(!o.label.starts_with("AGG"), "{id}/{}", o.id);
            texts.push(o.label.clone());
            texts.push(o.description.clone());
            texts.extend(o.warning.clone());
        }
        for t in texts {
            let low = t.to_lowercase();
            for bad in [
                "sara piu",
                "possibilita",
                "puo ",
                "e stato",
                "le build",
                "cache di build",
            ] {
                assert!(!low.contains(bad), "{id}: «{bad}» in «{t}»");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// (3) con un profilo finto, `collect` trova esattamente i file attesi
// ---------------------------------------------------------------------------

const CARGO_FILES: [&str; 11] = [
    "cargohome/registry/cache/i1/foo-1.0.0.crate",
    "cargohome/registry/src/i1/foo-1.0.0/lib.rs",
    "cargohome/registry/index/i1/.cache/fo/o/foo",
    "cargohome/git/checkouts/repo-abc/1234567/src/main.rs",
    "cargohome/git/db/repo-abc/HEAD",
    "cargohome/git/db/repo-abc/objects/pack/p.pack",
    "cargohome/config.toml",
    "cargohome/credentials.toml",
    "cargohome/bin/cargo.exe",
    "cargohome/.crates.toml",
    ".cargo/registry/src/i2/bar-2.0.0/lib.rs",
];

#[test]
fn cargo_options_take_only_the_cache_folders() {
    let env = [("cargo_home", "cargohome")];
    scenario(
        "bbag_cargo",
        "downloaded_crates",
        &env,
        &CARGO_FILES,
        &["cargohome/registry/cache/i1/foo-1.0.0.crate"],
    );
    scenario(
        "bbag_cargo",
        "registry_src",
        &env,
        &CARGO_FILES,
        &[
            "cargohome/registry/src/i1/foo-1.0.0/lib.rs",
            ".cargo/registry/src/i2/bar-2.0.0/lib.rs",
        ],
    );
    scenario(
        "bbag_cargo",
        "registry_index",
        &env,
        &CARGO_FILES,
        &["cargohome/registry/index/i1/.cache/fo/o/foo"],
    );
    scenario(
        "bbag_cargo",
        "git_checkouts",
        &env,
        &CARGO_FILES,
        &["cargohome/git/checkouts/repo-abc/1234567/src/main.rs"],
    );
    scenario(
        "bbag_cargo",
        "git_db",
        &env,
        &CARGO_FILES,
        &["cargohome/git/db/repo-abc/HEAD", "cargohome/git/db/repo-abc/objects/pack/p.pack"],
    );
}

#[test]
fn cargo_without_cargo_home_uses_the_default_folder() {
    scenario(
        "bbag_cargo",
        "registry_src",
        &[],
        &[".cargo/registry/src/i2/bar-2.0.0/lib.rs", "cargohome/registry/src/x/y.rs", ".cargo/config.toml"],
        &[".cargo/registry/src/i2/bar-2.0.0/lib.rs"],
    );
}

#[test]
fn go_build_and_module_caches() {
    let files = [
        "gocache/ab/abcdef-d",
        "gocache/trim.txt",
        "AppData/Local/go-build/cd/cdef-d",
        "gomod/github.com/a/b@v1.0.0/x.go",
        "gomod/cache/download/github.com/a/b/@v/v1.0.0.zip",
        "go/pkg/mod/github.com/c/d@v1.0.0/d.go",
        "gp/pkg/mod/m@v1.0.0/m.go",
        "go/bin/tool.exe",
    ];
    scenario(
        "bbag_go",
        "build",
        &[("gocache", "gocache")],
        &files,
        &["gocache/ab/abcdef-d", "gocache/trim.txt", "AppData/Local/go-build/cd/cdef-d"],
    );
    scenario(
        "bbag_go",
        "modules",
        &[("gomodcache", "gomod"), ("gopath", "gp")],
        &files,
        &[
            "gomod/github.com/a/b@v1.0.0/x.go",
            "gomod/cache/download/github.com/a/b/@v/v1.0.0.zip",
            "go/pkg/mod/github.com/c/d@v1.0.0/d.go",
            "gp/pkg/mod/m@v1.0.0/m.go",
        ],
    );
}

#[test]
fn go_module_cache_read_only_files_are_listed_and_not_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("u");
    let env = Env(base_map(&profile));
    let file = profile.join("go/pkg/mod/github.com/a/b@v1.0.0/x.go");
    write(&file, "xx");
    let mut perm = fs::metadata(&file).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&file, perm).unwrap();

    let c = load("bbag_go", &env);
    let got = collect(option_of(&c, "modules"), &env, &|_| false);
    assert_eq!(got.files.len(), 1);
    assert_eq!(got.skipped, 0, "{:?}", got.skipped_reasons);
    assert!(option_of(&c, "modules").warning.as_deref().unwrap().contains("read-only"));

    // Lascia il file cancellabile alla pulizia della cartella temporanea.
    let mut perm = fs::metadata(&file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    fs::set_permissions(&file, perm).unwrap();
}

#[test]
fn ccache_takes_the_hex_folders_and_keeps_its_settings() {
    let files = [
        "cc/0/1/abc.R",
        "cc/f/stats",
        "cc/ccache.conf",
        "cc/tmp/t.tmp",
        "cc/lock/auto_cleanup",
        "AppData/Local/ccache/a/b/c.R",
        "AppData/Local/ccache/ccache.conf",
    ];
    scenario(
        "ccache",
        "ccache",
        &[("ccache_dir", "cc")],
        &files,
        &["cc/0/1/abc.R", "cc/f/stats", "AppData/Local/ccache/a/b/c.R"],
    );
    // Senza CCACHE_DIR: solo la cartella predefinita, e le impostazioni restano.
    scenario(
        "ccache",
        "ccache",
        &[],
        &files,
        &["AppData/Local/ccache/a/b/c.R"],
    );
}

#[test]
fn sccache_takes_its_disk_cache_only() {
    let files = [
        "sc/ab/cd/obj",
        "AppData/Local/Mozilla/sccache/cache/12/obj2",
        "AppData/Roaming/Mozilla/sccache/config/config",
        "AppData/Local/Mozilla/sccache/other.txt",
    ];
    scenario(
        "ccache",
        "sccache",
        &[("sccache_dir", "sc")],
        &files,
        &["sc/ab/cd/obj", "AppData/Local/Mozilla/sccache/cache/12/obj2"],
    );
}

#[test]
fn composer_cache_inside_the_home_never_takes_config_or_auth() {
    // COMPOSER_CACHE_DIR messa nella stessa cartella di COMPOSER_HOME: config.json
    // e auth.json stanno accanto a files, repo e vcs.
    let files = [
        "AppData/Roaming/Composer/config.json",
        "AppData/Roaming/Composer/auth.json",
        "AppData/Roaming/Composer/composer.json",
        "AppData/Roaming/Composer/files/vendor/pkg/1.zip",
        "AppData/Roaming/Composer/repo/https---repo.packagist.org/p2/x.json",
        "AppData/Roaming/Composer/vcs/y/HEAD",
        "AppData/Roaming/Composer/vendor/bin/tool.bat",
        "AppData/Local/Composer/files/z.zip",
        "AppData/Local/Composer/config.json",
        "AppData/Local/Composer/.htaccess",
    ];
    scenario(
        "composer",
        "cache",
        &[("composer_cache_dir", "AppData/Roaming/Composer")],
        &files,
        &[
            "AppData/Roaming/Composer/files/vendor/pkg/1.zip",
            "AppData/Roaming/Composer/repo/https---repo.packagist.org/p2/x.json",
            "AppData/Roaming/Composer/vcs/y/HEAD",
            "AppData/Local/Composer/files/z.zip",
        ],
    );
    scenario(
        "composer",
        "cache",
        &[],
        &files,
        &["AppData/Local/Composer/files/z.zip"],
    );
}

#[test]
fn conan_cache_folder_takes_the_database_with_the_packages() {
    let files = [
        "ch/p/cache.sqlite3",
        "ch/p/zlib1a2b3c/e/conanfile.py",
        "ch/p/t/x",
        "ch/global.conf",
        "ch/profiles/default",
        "ch/remotes.json",
        ".conan2/p/other/f",
        ".conan2/global.conf",
    ];
    scenario(
        "conan",
        "packages",
        &[("conan_home", "ch")],
        &files,
        &["ch/p/cache.sqlite3", "ch/p/zlib1a2b3c/e/conanfile.py", "ch/p/t/x", ".conan2/p/other/f"],
    );
}

#[test]
fn hex_and_rebar3_keep_configuration_and_installed_rebar3() {
    let files = [
        ".hex/packages/hexpm/a-1.0.0.tar",
        ".hex/hex.config",
        ".hex/cache.ets",
        ".cache/rebar3/hex/hexpm/packages/b-2.0.0.tar",
        ".cache/rebar3/hex/default/packages.idx",
        ".cache/rebar3/bin/rebar3",
        ".cache/rebar3/lib/rebar/ebin/rebar.beam",
        ".config/rebar3/rebar.config",
    ];
    scenario("hex", "hex_packages", &[], &files, &[".hex/packages/hexpm/a-1.0.0.tar"]);
    scenario(
        "hex",
        "rebar3_hex",
        &[],
        &files,
        &[
            ".cache/rebar3/hex/hexpm/packages/b-2.0.0.tar",
            ".cache/rebar3/hex/default/packages.idx",
        ],
    );
}

#[test]
fn cabal_packages_never_touch_the_store() {
    scenario(
        "cabal",
        "packages",
        &[],
        &[
            "AppData/Roaming/cabal/packages/hackage.haskell.org/a.tar.gz",
            "AppData/Roaming/cabal/config",
            "AppData/Local/cabal/packages/hackage.haskell.org/b.tar.gz",
            "AppData/Local/cabal/store/ghc-9.6/lib/x.a",
        ],
        &[
            "AppData/Roaming/cabal/packages/hackage.haskell.org/a.tar.gz",
            "AppData/Local/cabal/packages/hackage.haskell.org/b.tar.gz",
        ],
    );
}

#[test]
fn julia_takes_only_compiled() {
    scenario(
        "julia",
        "compiled",
        &[],
        &[
            ".julia/compiled/v1.11/Foo/a.ji",
            ".julia/packages/Foo/x/src/Foo.jl",
            ".julia/registries/General/Registry.toml",
            ".julia/config/startup.jl",
        ],
        &[".julia/compiled/v1.11/Foo/a.ji"],
    );
}

#[test]
fn zig_global_cache_by_variable_and_by_default() {
    scenario(
        "zig",
        "global_cache",
        &[("zig_global_cache_dir", "zc")],
        &["zc/p/a/build.zig", "zc/o/b/obj", "AppData/Local/zig/z/f"],
        &["zc/p/a/build.zig", "zc/o/b/obj", "AppData/Local/zig/z/f"],
    );
}

#[test]
fn platformio_cache_packages_and_platforms_are_separate_options() {
    let files = [
        ".platformio/.cache/c1",
        ".platformio/packages/toolchain-x/bin/gcc.exe",
        ".platformio/platforms/atmelavr/platform.json",
        ".platformio/lib/Lib/l.h",
        ".platformio/appstate.json",
    ];
    scenario("platformio", "cache", &[], &files, &[".platformio/.cache/c1"]);
    scenario(
        "platformio",
        "packages",
        &[],
        &files,
        &[".platformio/packages/toolchain-x/bin/gcc.exe"],
    );
    scenario(
        "platformio",
        "platforms",
        &[],
        &files,
        &[".platformio/platforms/atmelavr/platform.json"],
    );
}

#[test]
fn minikube_vagrant_and_bundler_take_only_their_cache_folder() {
    scenario(
        "minikube",
        "cache",
        &[("minikube_home", "mh")],
        &[
            "mh/.minikube/cache/iso/a.iso",
            "mh/.minikube/profiles/minikube/config.json",
            "mh/.minikube/machines/m/disk",
            ".minikube/cache/kic/k",
            ".minikube/config/config.json",
        ],
        &["mh/.minikube/cache/iso/a.iso", ".minikube/cache/kic/k"],
    );
    scenario(
        "vagrant",
        "boxes",
        &[("vagrant_home", "vh")],
        &[
            "vh/boxes/hashicorp-VAGRANTSLASH-bionic64/1.0/virtualbox/box.vmdk",
            "vh/data/machine-index/index",
            "vh/insecure_private_keys/vagrant.key.rsa",
            ".vagrant.d/boxes/x/y",
        ],
        &[
            "vh/boxes/hashicorp-VAGRANTSLASH-bionic64/1.0/virtualbox/box.vmdk",
            ".vagrant.d/boxes/x/y",
        ],
    );
    scenario(
        "bundler",
        "cache",
        &[],
        &[".bundle/cache/compact_index/x", ".bundle/cache/git/y", ".bundle/config"],
        &[".bundle/cache/compact_index/x", ".bundle/cache/git/y"],
    );
}

#[test]
fn helm_cache_by_variable_and_in_temp_and_nothing_from_kube() {
    scenario(
        "kubectl",
        "helm_cache",
        &[("helm_cache_home", "hc")],
        &[
            "hc/repository/idx.yaml",
            "AppData/Local/Temp/helm/repository/stable-index.yaml",
            "AppData/Local/Temp/other/x",
            ".kube/config",
            ".kube/cache/discovery/x.json",
        ],
        &["hc/repository/idx.yaml", "AppData/Local/Temp/helm/repository/stable-index.yaml"],
    );
}

#[test]
fn terraform_plugin_cache_and_pulumi_plugins() {
    let files = [
        "tf/registry.terraform.io/hashicorp/aws/5.0.0/windows_amd64/p.exe",
        ".terraform.d/plugin-cache/x",
        "AppData/Roaming/terraform.d/plugin-cache/y",
        ".pulumi/plugins/resource-aws-v6.0.0/pulumi-resource-aws.exe",
        ".pulumi/credentials.json",
        ".pulumi/workspaces/w.json",
        ".pulumi/bin/pulumi.exe",
    ];
    scenario(
        "terraform",
        "terraform_plugins",
        &[("tf_plugin_cache_dir", "tf")],
        &files,
        &["tf/registry.terraform.io/hashicorp/aws/5.0.0/windows_amd64/p.exe"],
    );
    scenario(
        "terraform",
        "pulumi_plugins",
        &[],
        &files,
        &[".pulumi/plugins/resource-aws-v6.0.0/pulumi-resource-aws.exe"],
    );
}

#[test]
fn package_managers_for_windows_take_cache_and_logs_only() {
    scenario(
        "scoop",
        "scoop_cache",
        &[("scoop", "sd")],
        &[
            "sd/cache/git#2.0#url.7z",
            "sd/apps/git/current/git.exe",
            "sd/apps/git/1.0/git.exe",
            "sd/persist/p/data",
            "sd/buckets/main/b.json",
            "scoop/cache/old.7z",
        ],
        &["sd/cache/git#2.0#url.7z", "scoop/cache/old.7z"],
    );
    scenario(
        "scoop",
        "chocolatey_cache",
        &[],
        &[
            "AppData/Local/Temp/chocolatey/git/2.0/git.nupkg",
            "AppData/Local/Temp/other/x",
            "AppData/Local/Temp/WinGet/p/i.exe",
        ],
        &["AppData/Local/Temp/chocolatey/git/2.0/git.nupkg"],
    );
    scenario(
        "scoop",
        "winget_cache",
        &[],
        &[
            "AppData/Local/Temp/WinGet/Pkg.1.0/inst.exe",
            "AppData/Local/Temp/keep.txt",
            "AppData/Local/Temp/chocolatey/c",
        ],
        &["AppData/Local/Temp/WinGet/Pkg.1.0/inst.exe"],
    );
    let state = "AppData/Local/Packages/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe/LocalState";
    let log = format!("{state}/DiagOutputDir/WinGet-2026.log");
    let settings = format!("{state}/settings.json");
    scenario(
        "scoop",
        "winget_logs",
        &[],
        &[log.as_str(), settings.as_str()],
        &[log.as_str()],
    );
}

#[test]
fn bazel_takes_only_the_output_root_of_the_current_user() {
    scenario(
        "bazel",
        "output_root",
        &[],
        &[
            "_bazel_tester/install/abc/bin/bazel",
            "_bazel_tester/cache/repos/v1/x",
            "_bazel_tester/f00d/execroot/w/out.o",
            "_bazel_other/f/g",
            "bazel-out/h",
        ],
        &[
            "_bazel_tester/install/abc/bin/bazel",
            "_bazel_tester/cache/repos/v1/x",
            "_bazel_tester/f00d/execroot/w/out.o",
        ],
    );
}

#[test]
fn rustup_takes_downloads_and_tmp_but_not_the_toolchains() {
    let files = [
        "rh/downloads/a.partial",
        "rh/tmp/t_dir/b",
        "rh/toolchains/stable-x86_64-pc-windows-msvc/bin/rustc.exe",
        "rh/settings.toml",
        "rh/update-hashes/stable",
        ".rustup/downloads/c",
    ];
    scenario(
        "rustup",
        "downloads",
        &[("rustup_home", "rh")],
        &files,
        &["rh/downloads/a.partial", ".rustup/downloads/c"],
    );
    scenario("rustup", "tmp", &[("rustup_home", "rh")], &files, &["rh/tmp/t_dir/b"]);
}

#[test]
fn vcpkg_options_follow_the_variables() {
    let files = [
        "vr/buildtrees/foo/log.txt",
        "vr/packages/foo_x64-windows/lib.lib",
        "vr/downloads/tools/cmake.zip",
        "vr/installed/x64-windows/lib/x.lib",
        "vr/vcpkg.exe",
        "AppData/Local/vcpkg/archives/aa/aabb.zip",
        "bc/cc/ccdd.zip",
    ];
    let env = [("vcpkg_root", "vr"), ("vcpkg_default_binary_cache", "bc")];
    scenario(
        "vcpkg",
        "binary_cache",
        &env,
        &files,
        &["AppData/Local/vcpkg/archives/aa/aabb.zip", "bc/cc/ccdd.zip"],
    );
    scenario("vcpkg", "buildtrees", &env, &files, &["vr/buildtrees/foo/log.txt"]);
    scenario("vcpkg", "packages", &env, &files, &["vr/packages/foo_x64-windows/lib.lib"]);
    scenario("vcpkg", "downloads", &env, &files, &["vr/downloads/tools/cmake.zip"]);
}

// ---------------------------------------------------------------------------
// (2) nessuna azione si espande verso un file protetto (🔒)
// ---------------------------------------------------------------------------

/// Cartelle che una azione espande (la cartella base, senza l'ultimo componente
/// se questo ha caratteri jolly).
fn action_dirs(opt: &Opt, env: &Env) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for a in &opt.actions {
        let Action::Files { path, .. } = a else {
            panic!("azione non su file in {}", opt.id);
        };
        for p in expand(path, env) {
            let wild = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains(['*', '?']));
            let dir = if wild { p.parent().unwrap().to_path_buf() } else { p };
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
}

#[test]
fn no_option_reaches_a_credentials_or_configuration_file() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("u");
    let env = full_env(&profile);

    // `.kube` (la protezione la prende per intero): non entra in nessuna opzione.
    write(&profile.join(".kube/config"), "xx");
    write(&profile.join(".kube/cache/discovery/x.json"), "xx");

    let mut cleaners: Vec<Cleaner> = Vec::new();
    // I file protetti stanno nelle cartelle in cui vivono davvero: accanto alle
    // cartelle di cache (la home dello strumento) e nella home superiore.
    let mut homes: BTreeSet<PathBuf> = BTreeSet::new();
    let mut cache_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for (id, _) in EXPECTED {
        let c = load(id, &env);
        for o in &c.options {
            for dir in action_dirs(o, &env) {
                if let Some(parent) = dir.parent() {
                    homes.insert(parent.to_path_buf());
                    if let Some(grand) = parent.parent() {
                        homes.insert(grand.to_path_buf());
                    }
                }
                cache_dirs.insert(dir);
            }
        }
        cleaners.push(c);
    }
    // Nessuna home sta dentro una cartella di cache: sarebbe una cache vera.
    for h in &homes {
        assert!(
            !cache_dirs.iter().any(|d| h.starts_with(d)),
            "{} sta dentro una cartella di cache",
            h.display()
        );
    }
    let mut decoys: BTreeSet<PathBuf> = BTreeSet::new();
    for home in &homes {
        for name in LOCKED {
            let p = home.join(name);
            write(&p, "xx");
            decoys.insert(p);
        }
    }
    // Dentro le cartelle di cache: un file normale (si raccoglie) e i nomi che la
    // protezione blocca ovunque (non si raccolgono).
    let mut normal: Vec<PathBuf> = Vec::new();
    let mut guarded: Vec<PathBuf> = Vec::new();
    for dir in &cache_dirs {
        let is_glob_parent = dir.file_name().is_some_and(|n| n == "ccache_dir" || n == "ccache");
        if is_glob_parent {
            // ccache: solo le cartelle di una cifra esadecimale.
            write(&dir.join("0/normal.bin"), "xx");
            normal.push(dir.join("0/normal.bin"));
        } else {
            write(&dir.join("normal.bin"), "xx");
            normal.push(dir.join("normal.bin"));
        }
        for name in GUARDED_NAMES {
            for sub in ["", "sub"] {
                let p = dir.join(sub).join(name);
                write(&p, "xx");
                guarded.push(p);
            }
        }
    }

    let mut collected: BTreeSet<PathBuf> = BTreeSet::new();
    for c in &cleaners {
        for o in &c.options {
            let got = collect(o, &env, &|_| false);
            assert!(!got.cancelled);
            for (p, _) in got.files {
                collected.insert(p);
            }
        }
    }
    for p in &decoys {
        assert!(!collected.contains(p), "raccolto un file protetto: {}", p.display());
    }
    for p in &guarded {
        assert!(!collected.contains(p), "raccolto un file bloccato dalla protezione: {}", p.display());
    }
    for p in &collected {
        let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(!GUARDED_NAMES.contains(&name.as_str()), "{}", p.display());
        assert!(
            !p.components().any(|c| c.as_os_str() == ".kube"),
            "raccolto qualcosa sotto .kube: {}",
            p.display()
        );
    }
    // Il motore ha lavorato: i file normali delle cartelle di cache sono stati raccolti.
    for p in &normal {
        assert!(collected.contains(p), "non raccolto: {}", p.display());
    }
}
