//! Task 9, Parte B, lotto «jvm»: cache globali di Python (uv, Poetry, Pipenv,
//! Conda, virtualenv, pre-commit, pyenv-win, pytest), Java e Kotlin (Maven,
//! Gradle, Kotlin/Native, sbt), Android, Dart (Pub) e .NET (NuGet, Visual
//! Studio, file singolo).
//!
//! Mai sul PC dell'utente: ogni prova costruisce un profilo finto in una cartella
//! temporanea e passa a `collect` un `Env` finto che punta lì. Non si cancella nulla.
//! I file XML si leggono dalla cartella `cleaners/` del progetto.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use xiox_rules::cleanerml::parse_cleaner;
use xiox_rules::collect::{RegItem, collect, running};
use xiox_rules::model::{Action, Cleaner, Env};

/// Identificatore e testo di ogni XML del lotto.
const XML: [(&str, &str); 17] = [
    ("uv", include_str!("../../../cleaners/uv.xml")),
    ("poetry", include_str!("../../../cleaners/poetry.xml")),
    ("pipenv", include_str!("../../../cleaners/pipenv.xml")),
    ("conda", include_str!("../../../cleaners/conda.xml")),
    ("virtualenv", include_str!("../../../cleaners/virtualenv.xml")),
    ("precommit", include_str!("../../../cleaners/precommit.xml")),
    ("pyenv", include_str!("../../../cleaners/pyenv.xml")),
    ("pytest", include_str!("../../../cleaners/pytest.xml")),
    ("maven", include_str!("../../../cleaners/maven.xml")),
    ("gradle", include_str!("../../../cleaners/gradle.xml")),
    ("kotlin_native", include_str!("../../../cleaners/kotlin_native.xml")),
    ("sbt", include_str!("../../../cleaners/sbt.xml")),
    ("android", include_str!("../../../cleaners/android.xml")),
    ("pub", include_str!("../../../cleaners/pub.xml")),
    ("nuget", include_str!("../../../cleaners/nuget.xml")),
    ("visualstudio", include_str!("../../../cleaners/visualstudio.xml")),
    ("dotnet_bundle", include_str!("../../../cleaners/dotnet_bundle.xml")),
];

/// Id delle opzioni di ogni cleaner, nell'ordine del file.
const OPTIONS: [(&str, &[&str]); 17] = [
    ("uv", &["cache"]),
    ("poetry", &["cache", "virtualenvs"]),
    ("pipenv", &["virtualenvs"]),
    ("conda", &["pkgs"]),
    ("virtualenv", &["app_data"]),
    ("precommit", &["cache"]),
    ("pyenv", &["install_cache"]),
    ("pytest", &["tmp"]),
    ("maven", &["repository", "wrapper_dists"]),
    (
        "gradle",
        &["caches", "wrapper_dists", "native", "jdks", "daemon_logs"],
    ),
    ("kotlin_native", &["data"]),
    ("sbt", &["boot", "ivy_cache", "coursier_cache"]),
    (
        "android",
        &["user_cache", "sdk_temp", "studio_caches", "studio_logs"],
    ),
    ("pub", &["hosted", "git"]),
    (
        "nuget",
        &["packages", "http_cache", "plugins_cache", "scratch"],
    ),
    ("visualstudio", &["component_cache"]),
    ("dotnet_bundle", &["extract"]),
];

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Profilo finto in una cartella temporanea, con l'`Env` che ci punta.
struct Sandbox {
    _tmp: tempfile::TempDir,
    profile: PathBuf,
    env: Env,
}

impl Sandbox {
    /// `vars`: variabili degli strumenti (nome, percorso relativo al profilo).
    fn new(vars: &[(&str, &str)]) -> Sandbox {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("profile");
        let local = profile.join("AppData").join("Local");
        let mut map: HashMap<String, String> = HashMap::from([
            ("userprofile".to_string(), profile.to_string_lossy().into_owned()),
            ("localappdata".to_string(), local.to_string_lossy().into_owned()),
            (
                "appdata".to_string(),
                profile.join("AppData").join("Roaming").to_string_lossy().into_owned(),
            ),
            ("temp".to_string(), local.join("Temp").to_string_lossy().into_owned()),
        ]);
        for (name, rel) in vars {
            map.insert(
                name.to_lowercase(),
                profile.join(rel).to_string_lossy().into_owned(),
            );
        }
        Sandbox {
            _tmp: tmp,
            profile,
            env: Env(map),
        }
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.profile)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn xml_of(id: &str) -> &'static str {
    XML.iter().find(|(i, _)| *i == id).expect("cleaner del lotto").1
}

fn load(id: &str, env: &Env) -> Cleaner {
    parse_cleaner(xml_of(id), "partb_jvm", env).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn never(_: &RegItem) -> bool {
    false
}

/// Crea i file (tutti col contenuto "x"), raccoglie con ogni opzione del
/// cleaner e controlla che i file raccolti siano esattamente quelli segnati
/// `true`: i `false` sono ciò che non si deve toccare mai.
fn run_tool(id: &str, files: &[(&str, bool)], vars: &[(&str, &str)]) {
    let sb = Sandbox::new(vars);
    for (rel, _) in files {
        write(&sb.profile.join(rel), "x");
    }
    let cleaner = load(id, &sb.env);
    let mut got: Vec<String> = Vec::new();
    for opt in &cleaner.options {
        let c = collect(opt, &sb.env, &never);
        assert_eq!(c.skipped, 0, "{id}/{}: scartati {:?}", opt.id, c.skipped_reasons);
        assert!(!c.cancelled);
        got.extend(c.files.iter().map(|(p, _)| sb.rel(p)));
    }
    got.sort();
    got.dedup();
    let mut want: Vec<String> = files
        .iter()
        .filter(|(_, yes)| *yes)
        .map(|(rel, _)| rel.to_string())
        .collect();
    want.sort();
    assert_eq!(got, want, "{id}: file raccolti diversi dagli attesi");
}

// --- (1) lettura senza avvisi, (4) tutto spento, (5) avvisi --------------------

/// Tutte le variabili degli strumenti del lotto, definite dentro un profilo finto.
fn all_tool_vars() -> Vec<(&'static str, &'static str)> {
    vec![
        ("UV_CACHE_DIR", "tools/uv"),
        ("WORKON_HOME", "tools/workon"),
        ("CONDA_PKGS_DIRS", "tools/conda"),
        ("PRE_COMMIT_HOME", "tools/precommit"),
        ("GRADLE_USER_HOME", "tools/gradle"),
        ("KONAN_DATA_DIR", "tools/konan"),
        ("COURSIER_CACHE", "tools/coursier"),
        ("ANDROID_USER_HOME", "tools/android"),
        ("ANDROID_HOME", "tools/sdk"),
        ("ANDROID_SDK_ROOT", "tools/sdkroot"),
        ("PUB_CACHE", "tools/pub"),
        ("NUGET_PACKAGES", "tools/nuget"),
        ("NUGET_HTTP_CACHE_PATH", "tools/nugethttp"),
        ("DOTNET_BUNDLE_EXTRACT_BASE_DIR", "tools/bundle"),
    ]
}

#[test]
fn every_xml_reads_with_zero_warnings() {
    let minimal = Sandbox::new(&[]);
    let full = Sandbox::new(&all_tool_vars());
    for (id, xml) in XML {
        for env in [&minimal.env, &full.env] {
            let c = parse_cleaner(xml, "partb_jvm", env)
                .unwrap_or_else(|e| panic!("{id}: XML non letto: {e}"));
            assert_eq!(c.id, id);
            assert_eq!(c.group, "Sviluppo e AI");
            assert!(!c.label.is_empty() && !c.description.is_empty(), "{id}");
            let want: Vec<&str> = OPTIONS.iter().find(|(i, _)| *i == id).unwrap().1.to_vec();
            let got: Vec<&str> = c.options.iter().map(|o| o.id.as_str()).collect();
            assert_eq!(got, want, "{id}: opzioni");
            for o in &c.options {
                assert!(o.unsupported.is_none(), "{id}/{}: {:?}", o.id, o.unsupported);
                assert!(!o.actions.is_empty(), "{id}/{}: nessuna azione", o.id);
                assert!(!o.label.is_empty() && !o.description.is_empty(), "{id}/{}", o.id);
            }
        }
    }
}

#[test]
fn every_option_is_off_by_default() {
    let sb = Sandbox::new(&all_tool_vars());
    for (id, _) in XML {
        for o in &load(id, &sb.env).options {
            assert!(!o.default_on, "{id}/{} è acceso di default", o.id);
        }
    }
}

#[test]
fn texts_are_clean_italian_without_donor_names() {
    let sb = Sandbox::new(&[]);
    for (id, xml) in XML {
        let low = xml.to_lowercase();
        assert!(!low.contains("bleachbit"), "{id}");
        assert!(!low.contains("agg -"), "{id}");
        let c = load(id, &sb.env);
        let mut texts = vec![c.label.clone(), c.description.clone()];
        for o in &c.options {
            texts.push(o.label.clone());
            texts.push(o.description.clone());
            texts.extend(o.warning.clone());
        }
        for t in texts {
            assert!(!t.starts_with("AGG"), "{id}: «{t}»");
            let low = t.to_lowercase();
            for bad in ["puo ", "e stato", "possibilita", "sara piu", "download cache"] {
                assert!(!low.contains(bad), "{id}: «{bad}» in «{t}»");
            }
        }
    }
}

#[test]
fn warnings_are_present_where_the_table_asks_and_on_every_redownloadable_option() {
    let sb = Sandbox::new(&[]);
    let warning = |id: &str, opt: &str| -> String {
        load(id, &sb.env)
            .options
            .into_iter()
            .find(|o| o.id == opt)
            .unwrap_or_else(|| panic!("{id}/{opt}"))
            .warning
            .unwrap_or_else(|| panic!("{id}/{opt}: manca l'avviso"))
    };
    // Tabella: «gli artefatti fatti con `mvn install` esistono solo qui».
    assert!(warning("maven", "repository").contains("mvn install"));
    // Il comando per riaverle, dove la tabella lo dà.
    assert!(warning("poetry", "virtualenvs").contains("poetry install"));
    assert!(warning("pipenv", "virtualenvs").contains("pipenv install"));
    assert!(warning("conda", "pkgs").contains("conda"));
    assert!(warning("pyenv", "install_cache").contains("pyenv install"));
    assert!(warning("pub", "hosted").contains("pub get"));
    assert!(warning("pub", "git").contains("pub get"));
    assert!(warning("nuget", "packages").contains("restore"));
    // L'avviso su Gradle dice come fermare i daemon.
    assert!(warning("gradle", "caches").contains("gradle --stop"));
    // Dati che si riscaricano: sempre un avviso.
    for (id, opt) in [
        ("uv", "cache"),
        ("poetry", "cache"),
        ("virtualenv", "app_data"),
        ("precommit", "cache"),
        ("maven", "wrapper_dists"),
        ("gradle", "wrapper_dists"),
        ("gradle", "jdks"),
        ("kotlin_native", "data"),
        ("sbt", "boot"),
        ("sbt", "ivy_cache"),
        ("sbt", "coursier_cache"),
        ("android", "studio_caches"),
        ("visualstudio", "component_cache"),
        ("dotnet_bundle", "extract"),
    ] {
        assert!(!warning(id, opt).trim().is_empty(), "{id}/{opt}");
    }
}

// --- `<running>` --------------------------------------------------------------

#[test]
fn running_blocks_only_the_processes_of_the_table() {
    let sb = Sandbox::new(&[]);
    let blocked = |id: &str| load(id, &sb.env).running;
    assert_eq!(blocked("android"), vec!["studio64.exe"]);
    assert_eq!(blocked("visualstudio"), vec!["devenv.exe"]);
    // java.exe non basta a dire che c'è un daemon Gradle: solo gradle.exe.
    assert_eq!(blocked("gradle"), vec!["gradle.exe"]);
    let gradle = load("gradle", &sb.env);
    assert!(running(&gradle, &["java.exe".to_string()]).is_empty());
    assert_eq!(
        running(&gradle, &["Gradle.EXE".to_string()]),
        vec!["gradle.exe"]
    );
    for (id, _) in XML {
        if !["android", "visualstudio", "gradle"].contains(&id) {
            assert!(blocked(id).is_empty(), "{id}: processo non previsto dalla tabella");
        }
    }
}

// --- (2) niente verso i file 🔒 e verso ciò che è «regola 2» ---------------------

fn action_paths(c: &Cleaner) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for o in &c.options {
        for a in &o.actions {
            if let Action::Files { path, .. } = a {
                out.push((o.id.clone(), path.to_lowercase()));
            }
        }
    }
    out
}

#[test]
fn no_action_path_names_a_locked_or_rule2_location() {
    let sb = Sandbox::new(&all_tool_vars());
    // 🔒 (credenziali e configurazione), dati unici e toolchain installate.
    let forbidden = [
        "gradle.properties",
        "init.d",
        "debug.keystore",
        "adbkey",
        "localhistory",
        r".ivy2\local",
        r"\envs",
        "global_packages",
        "settings.xml",
        "nuget.config",
        "config.toml",
        r"\versions",
        r"\toolchains",
        r"\avd",
        r"\emulator",
        r"\platforms",
        r"\platform-tools",
        r"\system-images",
    ];
    // `bin` come componente del percorso (i programmi installati da Pub).
    for (id, _) in XML {
        for (opt, path) in action_paths(&load(id, &sb.env)) {
            for bad in forbidden {
                assert!(!path.contains(bad), "{id}/{opt}: «{path}» contiene «{bad}»");
            }
            assert!(
                !path.split('\\').any(|c| c == "bin"),
                "{id}/{opt}: «{path}» passa da bin"
            );
        }
    }
}

#[test]
fn locked_and_rule2_files_are_never_collected() {
    let sb = Sandbox::new(&all_tool_vars());
    let locked = [
        // Gradle
        ".gradle/gradle.properties",
        ".gradle/init.d/cache-settings.init.gradle.kts",
        "tools/gradle/gradle.properties",
        "tools/gradle/init.d/x.gradle",
        // Android
        ".android/debug.keystore",
        ".android/adbkey",
        ".android/adbkey.pub",
        "tools/android/debug.keystore",
        "tools/android/adbkey",
        // dati unici e programmi installati
        ".android/avd/Pixel.avd/userdata-qemu.img",
        "AppData/Local/Google/AndroidStudio2024.1/LocalHistory/changes.storageData",
        ".ivy2/local/org.me/pkg/ivy.xml",
        "miniconda3/envs/web/python.exe",
        ".pyenv/pyenv-win/versions/3.12.0/python.exe",
        "AppData/Local/Pub/Cache/bin/tool.bat",
        "AppData/Local/Pub/Cache/global_packages/p/pubspec.yaml",
        "tools/pub/bin/tool.bat",
        "tools/pub/global_packages/p/pubspec.yaml",
        "AppData/Roaming/pypoetry/config.toml",
        "AppData/Roaming/NuGet/NuGet.Config",
        ".m2/settings.xml",
        "AppData/Roaming/uv/python/cpython-3.12/python.exe",
    ];
    for rel in locked {
        write(&sb.profile.join(rel), "segreto");
    }
    // Accanto, i file di cache che si possono raccogliere (le cartelle esistono).
    for rel in [
        ".gradle/caches/modules-2/a.jar",
        ".android/cache/a.xml",
        "tools/gradle/caches/a.jar",
        "AppData/Local/Pub/Cache/hosted/pub.dev/a-1.0.0/a.dart",
        ".m2/repository/org/a/a.jar",
    ] {
        write(&sb.profile.join(rel), "cache");
    }
    let mut collected: Vec<String> = Vec::new();
    for (id, _) in XML {
        for opt in &load(id, &sb.env).options {
            let c = collect(opt, &sb.env, &never);
            collected.extend(c.files.iter().map(|(p, _)| sb.rel(p)));
        }
    }
    assert!(!collected.is_empty());
    for rel in locked {
        assert!(
            !collected.iter().any(|c| c == rel),
            "{rel} è stato raccolto"
        );
    }
    for c in &collected {
        let low = c.to_lowercase();
        for name in ["gradle.properties", "debug.keystore", "adbkey", "init.d/", "localhistory"] {
            assert!(!low.contains(name), "raccolto «{c}» che contiene «{name}»");
        }
    }
    // Le cache accanto sì.
    for rel in [
        ".gradle/caches/modules-2/a.jar",
        ".android/cache/a.xml",
        "tools/gradle/caches/a.jar",
        "AppData/Local/Pub/Cache/hosted/pub.dev/a-1.0.0/a.dart",
        ".m2/repository/org/a/a.jar",
    ] {
        assert!(collected.iter().any(|c| c == rel), "{rel} non raccolto");
    }
}

// --- (3) file attesi, uno strumento alla volta --------------------------------

#[test]
fn uv_collects_its_cache_and_not_python_or_tools() {
    run_tool(
        "uv",
        &[
            ("AppData/Local/uv/cache/wheels-v5/pypi/a/a-1.0-py3-none-any.whl", true),
            ("AppData/Local/uv/cache/archive-v0/abc/b.py", true),
            ("AppData/Roaming/uv/python/cpython-3.12/python.exe", false),
            ("AppData/Roaming/uv/tools/ruff/Scripts/ruff.exe", false),
        ],
        &[],
    );
    // UV_CACHE_DIR aggiunge la sua cartella; il percorso predefinito resta.
    run_tool(
        "uv",
        &[
            ("tools/uvcache/wheels-v5/x.whl", true),
            ("AppData/Local/uv/cache/y.whl", true),
        ],
        &[("UV_CACHE_DIR", "tools/uvcache")],
    );
}

#[test]
fn poetry_separates_cache_from_virtualenvs() {
    let files = [
        ("AppData/Local/pypoetry/Cache/cache/repositories/PyPI/a.json", true),
        ("AppData/Local/pypoetry/Cache/artifacts/aa/bb/c.whl", true),
        ("AppData/Local/pypoetry/Cache/virtualenvs/web-py3.12/Scripts/python.exe", true),
        ("AppData/Roaming/pypoetry/config.toml", false),
        ("AppData/Roaming/pypoetry/auth.toml", false),
    ];
    run_tool("poetry", &files, &[]);
    // Le opzioni sono separate: la cache non tocca gli ambienti.
    let sb = Sandbox::new(&[]);
    for (rel, _) in files {
        write(&sb.profile.join(rel), "x");
    }
    let c = load("poetry", &sb.env);
    let cache = collect(c.options.iter().find(|o| o.id == "cache").unwrap(), &sb.env, &never);
    assert_eq!(cache.files.len(), 2);
    assert!(cache.files.iter().all(|(p, _)| !sb.rel(p).contains("virtualenvs")));
}

#[test]
fn pipenv_and_virtualenv_collect_their_folders() {
    run_tool(
        "pipenv",
        &[
            (".virtualenvs/web-AbC123/Scripts/python.exe", true),
            (".virtualenvs/web-AbC123/Lib/site-packages/x.py", true),
            ("Pipfile", false),
            ("work/Pipfile.lock", false),
        ],
        &[],
    );
    run_tool(
        "pipenv",
        &[("tools/workon/api-zz/pyvenv.cfg", true), (".virtualenvs/old/pyvenv.cfg", true)],
        &[("WORKON_HOME", "tools/workon")],
    );
    run_tool(
        "virtualenv",
        &[
            ("AppData/Local/pypa/virtualenv/Cache/wheel/3.12/x.whl", true),
            ("AppData/Local/pypa/virtualenv/py_info/1/abc.json", true),
            ("AppData/Local/pypa/other-tool/data.db", false),
        ],
        &[],
    );
}

#[test]
fn conda_takes_pkgs_of_three_installations_and_never_envs() {
    run_tool(
        "conda",
        &[
            ("miniconda3/pkgs/numpy-1.26.4-py312/lib/x.py", true),
            ("miniconda3/pkgs/cache/abc.json", true),
            ("miniconda3/envs/web/python.exe", false),
            ("miniconda3/condabin/conda.bat", false),
            ("anaconda3/pkgs/scipy-1.0/y.py", true),
            ("anaconda3/envs/ds/python.exe", false),
            ("miniforge3/pkgs/z.conda", true),
            // Mambaforge non ha un percorso documentato: non si cerca.
            ("mambaforge/pkgs/w.conda", false),
            (".condarc", false),
        ],
        &[],
    );
    run_tool(
        "conda",
        &[
            ("tools/condapkgs/p/q.conda", true),
            ("miniconda3/pkgs/r.conda", true),
        ],
        &[("CONDA_PKGS_DIRS", "tools/condapkgs")],
    );
}

#[test]
fn precommit_pyenv_and_pytest_collect_their_folders() {
    run_tool(
        "precommit",
        &[
            (".cache/pre-commit/repoabc/hooks.yaml", true),
            (".cache/pre-commit/pre-commit.log", true),
            (".cache/pip/http/a", false),
            (".pre-commit-config.yaml", false),
        ],
        &[],
    );
    run_tool(
        "precommit",
        &[("tools/pchome/repo1/x", true), (".cache/pre-commit/repo2/y", true)],
        &[("PRE_COMMIT_HOME", "tools/pchome")],
    );
    run_tool(
        "pyenv",
        &[
            (".pyenv/pyenv-win/install_cache/python-3.12.0-amd64.exe", true),
            (".pyenv/pyenv-win/versions/3.12.0/python.exe", false),
            (".pyenv/pyenv-win/shims/python.bat", false),
        ],
        &[],
    );
    // Il modello pytest-of-* prende ogni cartella pytest-of-<utente>, e niente altro.
    run_tool(
        "pytest",
        &[
            ("AppData/Local/Temp/pytest-of-alfio/pytest-0/test_a0/out.txt", true),
            ("AppData/Local/Temp/pytest-of-alfio/pytest-1/test_b0/out.txt", true),
            ("AppData/Local/Temp/pytest-of-ci/pytest-0/x.log", true),
            ("AppData/Local/Temp/pytest-notmine/z.txt", false),
            ("AppData/Local/Temp/other.txt", false),
            ("work/.pytest_cache/v/cache/lastfailed", false),
        ],
        &[],
    );
}

#[test]
fn maven_collects_repository_and_wrapper_but_not_settings() {
    run_tool(
        "maven",
        &[
            (".m2/repository/org/apache/maven/maven-core/3.9.6/maven-core-3.9.6.jar", true),
            (".m2/repository/com/me/app/1.0/app-1.0.jar", true),
            (".m2/wrapper/dists/apache-maven-3.9.6-bin/abc/apache-maven-3.9.6/bin/mvn.cmd", true),
            (".m2/settings.xml", false),
            (".m2/settings-security.xml", false),
        ],
        &[],
    );
}

#[test]
fn gradle_collects_caches_dists_native_jdks_and_daemon_logs_only() {
    let files = [
        (".gradle/caches/modules-2/files-2.1/org.x/a/1.0/a.jar", true),
        (".gradle/caches/8.5/scripts/b.bin", true),
        (".gradle/wrapper/dists/gradle-8.5-bin/abc/gradle-8.5/lib/g.jar", true),
        (".gradle/native/jna/windows-amd64/jnidispatch.dll", true),
        (".gradle/jdks/eclipse_adoptium-21/bin/java.exe", true),
        (".gradle/daemon/8.5/daemon-1234.out.log", true),
        (".gradle/daemon/8.5/daemon-5678.out.log", true),
        (".gradle/daemon/8.5/registry.bin", false),
        (".gradle/daemon/8.5/notes.txt", false),
        (".gradle/gradle.properties", false),
        (".gradle/init.d/cache-settings.init.gradle.kts", false),
        ("proj/.gradle/8.5/fileChanges/x.bin", false),
    ];
    run_tool("gradle", &files, &[]);
    // GRADLE_USER_HOME sposta la cartella: le sue cache entrano, i file 🔒 accanto no.
    run_tool(
        "gradle",
        &[
            ("tools/gh/caches/modules-2/a.jar", true),
            ("tools/gh/daemon/8.6/daemon-1.out.log", true),
            ("tools/gh/gradle.properties", false),
            ("tools/gh/init.d/x.gradle", false),
        ],
        &[("GRADLE_USER_HOME", "tools/gh")],
    );
}

#[test]
fn gradle_drops_the_value_of_an_undefined_variable() {
    let without = Sandbox::new(&[]);
    let with = Sandbox::new(&[("GRADLE_USER_HOME", "tools/gh")]);
    let count = |sb: &Sandbox| {
        load("gradle", &sb.env)
            .options
            .iter()
            .find(|o| o.id == "caches")
            .unwrap()
            .actions
            .len()
    };
    assert_eq!(count(&without), 1);
    assert_eq!(count(&with), 2);
}

#[test]
fn kotlin_native_and_sbt_collect_their_folders() {
    run_tool(
        "kotlin_native",
        &[
            (".konan/dependencies/llvm-11.1.0-windows-x64/bin/clang.exe", true),
            (".konan/kotlin-native-prebuilt-windows-x86_64-2.0.0/bin/konanc.bat", true),
            (".konan/cache/x.klib", true),
            (".gradle/caches/a.jar", false),
        ],
        &[],
    );
    run_tool(
        "kotlin_native",
        &[("tools/konan/dependencies/x", true), (".konan/dependencies/y", true)],
        &[("KONAN_DATA_DIR", "tools/konan")],
    );
    run_tool(
        "sbt",
        &[
            (".sbt/boot/scala-2.12.18/lib/scala-library.jar", true),
            (".sbt/1.0/plugins/plugins.sbt", false),
            (".sbt/1.0/global.sbt", false),
            (".ivy2/cache/org.scala-lang/scala-library/jars/scala-library-2.13.jar", true),
            (".ivy2/local/org.me/pkg/1.0/jars/pkg.jar", false),
            (".ivy2/.credentials", false),
            ("AppData/Local/Coursier/Cache/v1/https/repo1.maven.org/a.jar", true),
        ],
        &[],
    );
    run_tool(
        "sbt",
        &[("tools/csr/v1/https/b.jar", true), ("AppData/Local/Coursier/Cache/v1/c.jar", true)],
        &[("COURSIER_CACHE", "tools/csr/v1")],
    );
}

#[test]
fn android_collects_caches_logs_and_sdk_leftovers_but_not_keys_or_emulators() {
    let files = [
        (".android/cache/addons_list-3.xml", true),
        (".android/debug.keystore", false),
        (".android/adbkey", false),
        (".android/adbkey.pub", false),
        (".android/avd/Pixel_7.avd/userdata-qemu.img", false),
        (".android/avd/Pixel_7.avd/snapshots/default_boot/ram.bin", false),
        ("AppData/Local/Android/Sdk/.temp/PackageOperation01/pkg.zip", true),
        ("AppData/Local/Android/Sdk/.downloadIntermediates/abc.bin", true),
        ("AppData/Local/Android/Sdk/platforms/android-34/android.jar", false),
        ("AppData/Local/Android/Sdk/platform-tools/adb.exe", false),
        ("AppData/Local/Android/Sdk/system-images/android-34/google_apis/x86_64/system.img", false),
        ("AppData/Local/Google/AndroidStudio2024.1/caches/content-hashes.dat", true),
        ("AppData/Local/Google/AndroidStudio2023.3/caches/names.dat", true),
        ("AppData/Local/Google/AndroidStudio2024.1/log/idea.log", true),
        ("AppData/Local/Google/AndroidStudio2024.1/LocalHistory/changes.storageData", false),
        ("AppData/Local/Google/AndroidStudio2024.1/tmp/x", false),
        ("AppData/Roaming/Google/AndroidStudio2024.1/options/other.xml", false),
        ("AppData/Local/Google/Chrome/User Data/Default/Cache/data_0", false),
    ];
    run_tool("android", &files, &[]);
    // Variabili: ANDROID_USER_HOME e ANDROID_HOME spostano .android e l'SDK.
    run_tool(
        "android",
        &[
            ("tools/au/cache/x.xml", true),
            ("tools/au/debug.keystore", false),
            ("tools/au/adbkey", false),
            ("tools/sdk/.temp/a/b.zip", true),
            ("tools/sdk/.downloadIntermediates/c.bin", true),
            ("tools/sdk/platforms/android-34/android.jar", false),
        ],
        &[("ANDROID_USER_HOME", "tools/au"), ("ANDROID_HOME", "tools/sdk")],
    );
}

#[test]
fn pub_collects_hosted_and_git_but_not_global_packages_or_bin() {
    run_tool(
        "pub",
        &[
            ("AppData/Local/Pub/Cache/hosted/pub.dev/http-1.2.0/lib/http.dart", true),
            ("AppData/Local/Pub/Cache/git/cache/pkg-abc/HEAD", true),
            ("AppData/Local/Pub/Cache/git/pkg-abc/lib/p.dart", true),
            ("AppData/Local/Pub/Cache/bin/flutterfire.bat", false),
            ("AppData/Local/Pub/Cache/global_packages/flutterfire_cli/pubspec.lock", false),
            ("AppData/Local/Pub/Cache/hosted-hashes/pub.dev/http-1.2.0.sha256", false),
        ],
        &[],
    );
    run_tool(
        "pub",
        &[("tools/pubc/hosted/pub.dev/a/a.dart", true), ("tools/pubc/bin/t.bat", false)],
        &[("PUB_CACHE", "tools/pubc")],
    );
}

#[test]
fn nuget_collects_its_four_folders_but_not_the_config() {
    run_tool(
        "nuget",
        &[
            (".nuget/packages/newtonsoft.json/13.0.3/lib/net6.0/Newtonsoft.Json.dll", true),
            ("AppData/Local/NuGet/v3-cache/abc123/list_serilog.dat", true),
            ("AppData/Local/NuGet/plugins-cache/plugin.json", true),
            ("AppData/Local/Temp/NuGetScratch/lock/abc", true),
            ("AppData/Roaming/NuGet/NuGet.Config", false),
            ("AppData/Local/Temp/other/x.tmp", false),
            ("src/App/packages/old/old.nupkg", false),
        ],
        &[],
    );
    run_tool(
        "nuget",
        &[
            ("tools/np/a/1.0/a.dll", true),
            ("tools/nh/x.dat", true),
            (".nuget/packages/b/1.0/b.dll", true),
        ],
        &[("NUGET_PACKAGES", "tools/np"), ("NUGET_HTTP_CACHE_PATH", "tools/nh")],
    );
}

#[test]
fn visualstudio_and_dotnet_bundle_collect_only_their_folders() {
    run_tool(
        "visualstudio",
        &[
            ("AppData/Local/Microsoft/VisualStudio/17.0_abc123/ComponentModelCache/Microsoft.VisualStudio.Default.cache", true),
            ("AppData/Local/Microsoft/VisualStudio/16.0_def456/ComponentModelCache/x.cache", true),
            ("AppData/Local/Microsoft/VisualStudio/17.0_abc123/Settings/CurrentSettings.vssettings", false),
            ("AppData/Local/Microsoft/VisualStudio/17.0_abc123/Extensions/ext/x.dll", false),
            ("AppData/Local/Microsoft/VisualStudio/ComponentModelCache/y.cache", false),
        ],
        &[],
    );
    run_tool(
        "dotnet_bundle",
        &[
            ("AppData/Local/Temp/.net/myapp/abc123/lib.dll", true),
            ("AppData/Local/Temp/.net/other/def456/lib2.dll", true),
            ("AppData/Local/Temp/other/x.dll", false),
        ],
        &[],
    );
    run_tool(
        "dotnet_bundle",
        &[("tools/bundle/app/h/a.dll", true), ("AppData/Local/Temp/.net/b/h/b.dll", true)],
        &[("DOTNET_BUNDLE_EXTRACT_BASE_DIR", "tools/bundle")],
    );
}
