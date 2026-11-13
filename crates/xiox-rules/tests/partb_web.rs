//! Task 9 (Parte B), lotto «web»: cache globali di npm, pnpm, Yarn, Bun, Deno,
//! Electron, Playwright, Puppeteer, Cypress, VS Code e derivati, Firebase,
//! Hugging Face, PyTorch, Keras, Ollama e CUDA.
//!
//! Mai sul PC dell'utente: ogni prova costruisce un profilo finto in una
//! cartella temporanea (`Env` finto con `userprofile`, `localappdata`,
//! `appdata`, `temp` e le variabili degli strumenti puntate lì) e legge i
//! cleaner dalla cartella `cleaners/` senza passare da `embedded.rs`.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use xiox_rules::cleanerml::parse_cleaner;
use xiox_rules::collect::{Collected, RegItem, collect, present, running};
use xiox_rules::model::{Action, Cleaner, Env, Opt};

const XML: [(&str, &str); 16] = [
    ("bbag_npm", include_str!("../../../cleaners/bbag_npm.xml")),
    ("bbag_shader", include_str!("../../../cleaners/bbag_shader.xml")),
    ("pnpm", include_str!("../../../cleaners/pnpm.xml")),
    ("yarn", include_str!("../../../cleaners/yarn.xml")),
    ("bun", include_str!("../../../cleaners/bun.xml")),
    ("deno", include_str!("../../../cleaners/deno.xml")),
    ("electron", include_str!("../../../cleaners/electron.xml")),
    ("playwright", include_str!("../../../cleaners/playwright.xml")),
    ("puppeteer", include_str!("../../../cleaners/puppeteer.xml")),
    ("cypress", include_str!("../../../cleaners/cypress.xml")),
    ("vscode", include_str!("../../../cleaners/vscode.xml")),
    ("firebase", include_str!("../../../cleaners/firebase.xml")),
    ("huggingface", include_str!("../../../cleaners/huggingface.xml")),
    ("pytorch", include_str!("../../../cleaners/pytorch.xml")),
    ("keras", include_str!("../../../cleaners/keras.xml")),
    ("ollama", include_str!("../../../cleaners/ollama.xml")),
];

/// Variabili d'ambiente degli strumenti del lotto (tutte nella lista di
/// `Env::for_rules`).
const TOOL_VARS: [&str; 11] = [
    "npm_config_cache",
    "bun_install_cache_dir",
    "deno_dir",
    "electron_cache",
    "playwright_browsers_path",
    "puppeteer_cache_dir",
    "cypress_cache_folder",
    "hf_home",
    "torch_home",
    "ollama_models",
    "cuda_cache_path",
];

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn write(path: &Path, content: &str) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
    path.to_path_buf()
}

/// Profilo finto in una cartella temporanea.
struct Fake {
    _tmp: tempfile::TempDir,
    profile: PathBuf,
    local: PathBuf,
    roaming: PathBuf,
    env: Env,
}

impl Fake {
    /// `vars`: (variabile, sottocartella di `<profilo>\custom`): ogni
    /// variabile degli strumenti punta a una cartella dentro il profilo.
    fn new(vars: &[(&str, &str)]) -> Fake {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("profile");
        let local = profile.join("AppData").join("Local");
        let roaming = profile.join("AppData").join("Roaming");
        fs::create_dir_all(local.join("Temp")).unwrap();
        fs::create_dir_all(&roaming).unwrap();
        let mut map = HashMap::from([
            ("userprofile".to_string(), s(&profile)),
            ("localappdata".to_string(), s(&local)),
            ("appdata".to_string(), s(&roaming)),
            ("temp".to_string(), s(&local.join("Temp"))),
        ]);
        for (key, sub) in vars {
            map.insert(key.to_lowercase(), s(&profile.join("custom").join(sub)));
        }
        Fake {
            _tmp: tmp,
            profile,
            local,
            roaming,
            env: Env(map),
        }
    }

    /// Profilo con tutte le variabili degli strumenti definite.
    fn with_all_vars() -> Fake {
        let vars: Vec<(&str, &str)> = TOOL_VARS.iter().map(|v| (*v, *v)).collect();
        Fake::new(&vars)
    }

    /// La cartella a cui punta una variabile (`<profilo>\custom\<sub>`).
    fn custom(&self, sub: &str) -> PathBuf {
        self.profile.join("custom").join(sub)
    }

    fn cleaner(&self, name: &str) -> Cleaner {
        let (n, xml) = XML.iter().find(|(n, _)| *n == name).expect("cleaner del lotto");
        parse_cleaner(xml, n, &self.env).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn collected(&self, cleaner: &str, option: &str) -> Collected {
        let c = self.cleaner(cleaner);
        let opt = c
            .options
            .iter()
            .find(|o| o.id == option)
            .unwrap_or_else(|| panic!("opzione {cleaner}/{option} assente"));
        collect(opt, &self.env, &|_: &RegItem| false)
    }

    /// I file raccolti, in minuscolo e ordinati (Windows non distingue le
    /// maiuscole).
    fn files(&self, cleaner: &str, option: &str) -> BTreeSet<String> {
        let c = self.collected(cleaner, option);
        assert_eq!(c.skipped, 0, "{cleaner}/{option}: {:?}", c.skipped_reasons);
        assert!(!c.cancelled);
        norm(c.files.iter().map(|(p, _)| p.clone()))
    }
}

fn norm<I: IntoIterator<Item = PathBuf>>(paths: I) -> BTreeSet<String> {
    paths.into_iter().map(|p| s(&p).to_lowercase()).collect()
}

fn expected(paths: &[&PathBuf]) -> BTreeSet<String> {
    norm(paths.iter().map(|p| (*p).clone()))
}

fn action_paths(opt: &Opt) -> Vec<String> {
    opt.actions
        .iter()
        .filter_map(|a| match a {
            Action::Files { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

// (1) ogni XML si legge con 0 avvisi, con gli id attesi.

#[test]
fn every_xml_reads_with_no_warnings_and_expected_options() {
    let want: [(&str, &[&str]); 16] = [
        ("bbag_npm", &["downloads", "npx", "logs", "custom_cache"]),
        ("bbag_shader", &["directx", "nvidia", "amd", "cuda"]),
        ("pnpm", &["store", "cache"]),
        ("yarn", &["classic", "berry"]),
        ("bun", &["cache"]),
        ("deno", &["modules", "compiled"]),
        ("electron", &["download", "builder"]),
        ("playwright", &["browsers"]),
        ("puppeteer", &["browsers"]),
        ("cypress", &["binary"]),
        ("vscode", &["cache", "logs"]),
        ("firebase", &["emulators"]),
        ("huggingface", &["hub", "datasets", "xet", "assets"]),
        ("pytorch", &["hub"]),
        ("keras", &["datasets", "models"]),
        ("ollama", &["models"]),
    ];
    // Con tutte le variabili definite, nessuna opzione sparisce.
    let fake = Fake::with_all_vars();
    for (name, ids) in want {
        let c = fake.cleaner(name);
        assert_eq!(c.id, name);
        assert_eq!(c.group, "Sviluppo e AI");
        let got: Vec<&str> = c.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(got, ids, "{name}: opzioni");
        for o in &c.options {
            assert!(o.unsupported.is_none(), "{name}/{}: {:?}", o.id, o.unsupported);
            assert!(!o.actions.is_empty(), "{name}/{} senza azioni", o.id);
            assert!(!o.label.trim().is_empty() && !o.description.trim().is_empty());
        }
    }
}

// (4) niente è acceso di default.

#[test]
fn every_option_is_off_by_default() {
    let fake = Fake::with_all_vars();
    for (name, _) in XML {
        for o in &fake.cleaner(name).options {
            assert!(!o.default_on, "{name}/{} è acceso di default", o.id);
        }
    }
}

// (5) l'avviso c'è dove la tabella dà il comando per riavere i file.

#[test]
fn warnings_name_the_command_to_get_the_files_back() {
    let fake = Fake::with_all_vars();
    let cases: [(&str, &str, &str); 17] = [
        ("pnpm", "store", "pnpm install"),
        ("yarn", "berry", "yarn install"),
        ("bun", "cache", "bun install"),
        ("deno", "modules", "deno cache"),
        ("electron", "download", "npm install"),
        ("electron", "builder", "electron-builder"),
        ("playwright", "browsers", "npx playwright install"),
        ("puppeteer", "browsers", "npm install"),
        ("cypress", "binary", "npx cypress install"),
        ("firebase", "emulators", "firebase emulators:start"),
        ("huggingface", "hub", "Si riscaricano"),
        ("huggingface", "datasets", "Si riscaricano"),
        ("pytorch", "hub", "torch.hub.load"),
        ("keras", "datasets", "Si riscaricano"),
        ("keras", "models", "Si riscaricano"),
        ("ollama", "models", "ollama pull"),
        ("bbag_npm", "custom_cache", "riscaricati"),
    ];
    for (name, option, needle) in cases {
        let c = fake.cleaner(name);
        let o = c.options.iter().find(|o| o.id == option).unwrap();
        let w = o.warning.as_deref().unwrap_or_else(|| panic!("{name}/{option} senza avviso"));
        assert!(w.contains(needle), "{name}/{option}: «{w}» non contiene «{needle}»");
    }
    // Ollama: anche che i modelli spariscono da `ollama list`.
    let c = fake.cleaner("ollama");
    assert!(c.options[0].warning.as_deref().unwrap().contains("ollama list"));
}

// (2) nessuna azione si espande verso un file 🔒 né verso dati dell'utente.

#[test]
fn locked_and_user_files_are_never_collected() {
    // Due profili: senza variabili (percorsi predefiniti) e con tutte le
    // variabili puntate in cartelle personalizzate che contengono gli stessi
    // file di valore.
    for fake in [Fake::new(&[]), Fake::with_all_vars()] {
        let p = &fake.profile;
        let pick = |var: &str, default: PathBuf| {
            fake.env.get(var).map(PathBuf::from).unwrap_or(default)
        };
        let hf_home = pick("hf_home", p.join(".cache").join("huggingface"));
        let torch_home = pick("torch_home", p.join(".cache").join("torch"));
        let deno_home = pick("deno_dir", fake.local.join("deno"));
        let ollama_models = pick("ollama_models", p.join(".ollama").join("models"));

        // Cache vere dentro ogni cartella toccata: devono essere raccolte.
        let cache = vec![
            write(&hf_home.join("hub").join("models--a").join("blobs").join("b1"), "x"),
            write(&hf_home.join("datasets").join("d").join("f.arrow"), "x"),
            write(&hf_home.join("xet").join("shard"), "x"),
            write(&hf_home.join("assets").join("a.bin"), "x"),
            write(&torch_home.join("hub").join("checkpoints").join("w.pth"), "x"),
            write(&p.join(".keras").join("datasets").join("mnist.npz"), "x"),
            write(&p.join(".keras").join("models").join("resnet.h5"), "x"),
            write(&fake.local.join("pnpm").join("store").join("v3").join("f"), "x"),
            write(&fake.local.join("pnpm-cache").join("metadata").join("m.json"), "x"),
            write(&deno_home.join("remote").join("https").join("h"), "x"),
            write(&deno_home.join("npm").join("registry").join("n"), "x"),
            write(&deno_home.join("v8_code_cache_v2"), "x"),
            write(&fake.roaming.join("Code").join("Cache").join("Cache_Data").join("f_1"), "x"),
            write(&fake.roaming.join("Code").join("CachedData").join("c").join("js"), "x"),
            write(&fake.roaming.join("Code").join("logs").join("20260101").join("main.log"), "x"),
            write(&ollama_models.join("blobs").join("sha256-1"), "x"),
            write(&p.join(".cache").join("firebase").join("emulators").join("e.jar"), "x"),
        ];

        // File 🔒 e dati dell'utente accanto alle cache, nei loro posti veri.
        let code = fake.roaming.join("Code");
        let locked = vec![
            write(&hf_home.join("token"), "t"),
            write(&hf_home.join("stored_tokens"), "t"),
            write(&p.join(".keras").join("keras.json"), "{}"),
            write(&code.join("User").join("settings.json"), "{}"),
            write(&code.join("User").join("keybindings.json"), "[]"),
            write(&fake.roaming.join("Cursor").join("User").join("settings.json"), "{}"),
            write(&fake.roaming.join("VSCodium").join("User").join("settings.json"), "{}"),
            write(&code.join("User").join("History").join("h").join("entries.json"), "[]"),
            write(&code.join("Local Storage").join("leveldb").join("1.log"), "x"),
            write(&code.join("Workspaces").join("w").join("workspace.json"), "{}"),
            write(&code.join("Backups").join("b").join("unsaved"), "x"),
            write(&deno_home.join("location_data").join("o").join("local_storage"), "x"),
            write(&deno_home.join("deno_history.txt"), "x"),
            write(&fake.local.join("pnpm").join("config").join("auth.ini"), "x"),
            write(&p.join(".kube").join("config"), "x"),
            write(&p.join(".gradle").join("gradle.properties"), "x"),
            write(&p.join(".android").join("debug.keystore"), "x"),
            write(&p.join(".android").join("adbkey"), "x"),
            write(&p.join(".cargo").join("credentials.toml"), "x"),
            write(&p.join(".cargo").join("config.toml"), "x"),
            write(&fake.roaming.join("Composer").join("auth.json"), "x"),
            write(&fake.roaming.join("Composer").join("config.json"), "x"),
            write(&torch_home.join("README-owner"), "x"),
            write(&p.join(".ollama").join("id_ed25519"), "x"),
        ];

        let mut got_files: BTreeSet<String> = BTreeSet::new();
        let mut got_dirs: Vec<PathBuf> = Vec::new();
        for (name, _) in XML {
            for o in &fake.cleaner(name).options {
                let c = fake.collected(name, &o.id);
                got_files.extend(norm(c.files.iter().map(|(f, _)| f.clone())));
                got_dirs.extend(c.dirs_to_remove);
            }
        }
        for f in &locked {
            assert!(!got_files.contains(&s(f).to_lowercase()), "raccolto: {}", f.display());
            for d in &got_dirs {
                assert!(
                    !f.starts_with(d),
                    "{} si toglierebbe con la cartella {}",
                    f.display(),
                    d.display()
                );
            }
        }
        // E le cache vere sono raccolte: la prova non passa per caso.
        for f in &cache {
            assert!(got_files.contains(&s(f).to_lowercase()), "non raccolto: {}", f.display());
        }
    }
}

// (3) con un profilo finto, `collect` trova esattamente i file attesi.

#[test]
fn pnpm_yarn_and_bun_find_exactly_their_files() {
    let fake = Fake::new(&[("BUN_INSTALL_CACHE_DIR", "bun")]);
    let (l, p) = (&fake.local, &fake.profile);

    // pnpm: store e cache; la configurazione accanto non si tocca.
    let store = write(&l.join("pnpm").join("store").join("v3").join("files").join("ab"), "123");
    let cache = write(&l.join("pnpm-cache").join("metadata").join("npm.json"), "12");
    write(&l.join("pnpm").join("config").join("auth.ini"), "secret");
    assert_eq!(fake.files("pnpm", "store"), expected(&[&store]));
    assert_eq!(fake.files("pnpm", "cache"), expected(&[&cache]));
    assert_eq!(fake.collected("pnpm", "store").bytes, 3);

    // Yarn: Classic, e Berry in entrambe le cartelle.
    let classic = write(&l.join("Yarn").join("Cache").join("v6").join("npm-a.tgz"), "1");
    write(&l.join("Yarn").join("Data").join("global").join("package.json"), "{}");
    let berry_a = write(&l.join("Yarn").join("Berry").join("cache").join("a-npm-1.zip"), "1");
    let berry_b = write(&p.join(".yarn").join("berry").join("cache").join("b-npm-1.zip"), "1");
    write(&p.join(".yarn").join("berry").join("metadata").join("m"), "x");
    assert_eq!(fake.files("yarn", "classic"), expected(&[&classic]));
    assert_eq!(fake.files("yarn", "berry"), expected(&[&berry_a, &berry_b]));

    // Bun: prima la variabile, poi il predefinito (qui esistono entrambi).
    let def = write(&p.join(".bun").join("install").join("cache").join("x@1").join("i.js"), "1");
    let own = write(&fake.custom("bun").join("y@1").join("i.js"), "1");
    write(&p.join(".bun").join("bin").join("bun.exe"), "exe");
    assert_eq!(fake.files("bun", "cache"), expected(&[&def, &own]));
}

#[test]
fn deno_and_vscode_find_exactly_their_files() {
    let fake = Fake::new(&[]);
    let (l, r) = (&fake.local, &fake.roaming);

    // Deno (predefinito): moduli e cache di compilazione, non i dati di origine.
    let deno = l.join("deno");
    let remote = write(&deno.join("remote").join("https").join("deno.land").join("m"), "1");
    let npm = write(&deno.join("npm").join("registry.npmjs.org").join("p").join("i.js"), "1");
    let gen_ = write(&deno.join("gen").join("a.js"), "1");
    let reg = write(&deno.join("registries").join("r.json"), "1");
    let dl = write(&deno.join("dl").join("deno.zip"), "1");
    let db1 = write(&deno.join("v8_code_cache_v2"), "1");
    let db2 = write(&deno.join("check_cache_v2-wal"), "1");
    write(&deno.join("location_data").join("o").join("local_storage"), "x");
    write(&deno.join("deno_history.txt"), "x");
    write(&deno.join("latest.txt"), "x");
    assert_eq!(fake.files("deno", "modules"), expected(&[&remote, &npm]));
    assert_eq!(fake.files("deno", "compiled"), expected(&[&gen_, &reg, &dl, &db1, &db2]));

    // VS Code: tre editor installati, due no; solo cache e log.
    let mut cache = Vec::new();
    let mut logs = Vec::new();
    for editor in ["Code", "VSCodium", "Cursor"] {
        let base = r.join(editor);
        for dir in ["Cache", "CachedData", "Code Cache", "GPUCache", "CachedExtensionVSIXs"] {
            cache.push(write(&base.join(dir).join("sub").join("f"), "1"));
        }
        logs.push(write(&base.join("logs").join("20260101T000000").join("main.log"), "1"));
        write(&base.join("User").join("settings.json"), "{}");
        write(&base.join("User").join("History").join("h").join("e.json"), "[]");
        write(&base.join("User").join("workspaceStorage").join("w").join("state.vscdb"), "x");
        write(&base.join("Local Storage").join("leveldb").join("1.log"), "x");
        write(&base.join("Workspaces").join("w").join("workspace.json"), "x");
    }
    let cache_refs: Vec<&PathBuf> = cache.iter().collect();
    let logs_refs: Vec<&PathBuf> = logs.iter().collect();
    assert_eq!(fake.files("vscode", "cache"), expected(&cache_refs));
    assert_eq!(fake.files("vscode", "logs"), expected(&logs_refs));
}

#[test]
fn ai_caches_find_exactly_their_files() {
    // Hugging Face, PyTorch e Ollama con la variabile e con il predefinito
    // insieme.
    let fake = Fake::new(&[("HF_HOME", "hf"), ("TORCH_HOME", "torch"), ("OLLAMA_MODELS", "models")]);
    let p = &fake.profile;
    let std_hf = p.join(".cache").join("huggingface");
    let hf = fake.custom("hf");

    let hub_std = write(&std_hf.join("hub").join("models--x").join("snapshots").join("f"), "1");
    let hub_own = write(&hf.join("hub").join("models--y").join("f"), "1");
    let ds_std = write(&std_hf.join("datasets").join("a").join("f.arrow"), "1");
    let ds_own = write(&hf.join("datasets").join("b.arrow"), "1");
    let xet_std = write(&std_hf.join("xet").join("shard-cache").join("s"), "1");
    let assets_std = write(&std_hf.join("assets").join("a.json"), "1");
    write(&std_hf.join("token"), "t");
    write(&std_hf.join("stored_tokens"), "t");
    write(&hf.join("token"), "t");
    assert_eq!(fake.files("huggingface", "hub"), expected(&[&hub_std, &hub_own]));
    assert_eq!(fake.files("huggingface", "datasets"), expected(&[&ds_std, &ds_own]));
    assert_eq!(fake.files("huggingface", "xet"), expected(&[&xet_std]));
    assert_eq!(fake.files("huggingface", "assets"), expected(&[&assets_std]));

    // PyTorch: solo la sottocartella hub.
    let t_std = write(&p.join(".cache").join("torch").join("hub").join("checkpoints").join("a.pth"), "1");
    let t_own = write(&fake.custom("torch").join("hub").join("owner_repo_main").join("h.py"), "1");
    write(&fake.custom("torch").join("kernels").join("k"), "x");
    assert_eq!(fake.files("pytorch", "hub"), expected(&[&t_std, &t_own]));

    // Keras: datasets e models, non keras.json.
    let k_ds = write(&p.join(".keras").join("datasets").join("mnist.npz"), "1");
    let k_md = write(&p.join(".keras").join("models").join("vgg16.h5"), "1");
    write(&p.join(".keras").join("keras.json"), "{}");
    assert_eq!(fake.files("keras", "datasets"), expected(&[&k_ds]));
    assert_eq!(fake.files("keras", "models"), expected(&[&k_md]));

    // Ollama: i modelli, non la chiave.
    let o_std = write(&p.join(".ollama").join("models").join("blobs").join("sha256-a"), "1");
    let o_own = write(&fake.custom("models").join("manifests").join("m"), "1");
    write(&p.join(".ollama").join("id_ed25519"), "k");
    assert_eq!(fake.files("ollama", "models"), expected(&[&o_std, &o_own]));
}

#[test]
fn browsers_binaries_and_emulators_find_exactly_their_files() {
    let fake = Fake::new(&[]);
    let (l, p) = (&fake.local, &fake.profile);

    let pw = write(&l.join("ms-playwright").join("chromium-1100").join("chrome-win").join("chrome.exe"), "1");
    let pw2 = write(&l.join("ms-playwright").join(".links"), "1");
    assert_eq!(fake.files("playwright", "browsers"), expected(&[&pw, &pw2]));

    let pp = write(&p.join(".cache").join("puppeteer").join("chrome").join("win64-1").join("chrome.exe"), "1");
    assert_eq!(fake.files("puppeteer", "browsers"), expected(&[&pp]));

    let cy = write(&l.join("Cypress").join("Cache").join("13.0.0").join("Cypress").join("Cypress.exe"), "1");
    assert_eq!(fake.files("cypress", "binary"), expected(&[&cy]));

    let el = write(&l.join("electron").join("Cache").join("abc").join("electron-v30-win32-x64.zip"), "1");
    let eb = write(&l.join("electron-builder").join("Cache").join("nsis").join("n.7z"), "1");
    assert_eq!(fake.files("electron", "download"), expected(&[&el]));
    assert_eq!(fake.files("electron", "builder"), expected(&[&eb]));

    let fb = write(&p.join(".cache").join("firebase").join("emulators").join("cloud-firestore-emulator-1.jar"), "1");
    write(&p.join(".cache").join("firebase").join("other").join("keep"), "x");
    assert_eq!(fake.files("firebase", "emulators"), expected(&[&fb]));
}

#[test]
fn cuda_and_custom_npm_cache_find_exactly_their_files() {
    let fake = Fake::new(&[("CUDA_CACHE_PATH", "cuda"), ("npm_config_cache", "npm")]);
    let (l, r) = (&fake.local, &fake.roaming);

    // CUDA: variabile e predefinito, solo file.
    let cuda_std = write(&r.join("NVIDIA").join("ComputeCache").join("0").join("a"), "1");
    let cuda_own = write(&fake.custom("cuda").join("1").join("b"), "1");
    write(&l.join("NVIDIA").join("DXCache").join("d"), "x");
    assert_eq!(fake.files("bbag_shader", "cuda"), expected(&[&cuda_std, &cuda_own]));
    // Le altre opzioni delle shader restano com'erano: CUDA non entra in `nvidia`.
    assert!(!fake.files("bbag_shader", "nvidia").contains(&s(&cuda_std).to_lowercase()));

    // npm: la cartella indicata da npm_config_cache; non il predefinito.
    let n = fake.custom("npm");
    let c1 = write(&n.join("_cacache").join("content-v2").join("sha512").join("ab"), "1");
    let c2 = write(&n.join("_npx").join("1234").join("node_modules").join("x.js"), "1");
    let c3 = write(&n.join("_logs").join("2026-01-01-debug-0.log"), "1");
    write(&n.join("_logs").join("keep.txt"), "x");
    write(&n.join("other").join("o"), "x");
    let default_npm = write(&l.join("npm-cache").join("_cacache").join("z"), "1");
    let custom = fake.files("bbag_npm", "custom_cache");
    assert_eq!(custom, expected(&[&c1, &c2, &c3]));
    assert!(!custom.contains(&s(&default_npm).to_lowercase()));
    assert_eq!(fake.files("bbag_npm", "downloads"), expected(&[&default_npm]));
}

// Regola 3: una variabile non definita scarta il suo valore.

#[test]
fn undefined_variable_discards_only_its_value() {
    let without = Fake::new(&[]);
    let with = Fake::new(&[("BUN_INSTALL_CACHE_DIR", "bun")]);
    let bun_paths = |f: &Fake| action_paths(&f.cleaner("bun").options[0]);
    assert_eq!(
        bun_paths(&without),
        vec![r"%UserProfile%\.bun\install\cache".to_string()]
    );
    assert_eq!(
        bun_paths(&with),
        vec![
            "%BUN_INSTALL_CACHE_DIR%".to_string(),
            r"%UserProfile%\.bun\install\cache".to_string()
        ]
    );
    // Il cleaner delle shader senza CUDA_CACHE_PATH tiene il predefinito.
    let shader = without.cleaner("bbag_shader");
    let cuda = shader.options.iter().find(|o| o.id == "cuda").unwrap();
    assert_eq!(action_paths(cuda), vec![r"%AppData%\NVIDIA\ComputeCache".to_string()]);
    // L'opzione di npm con la cartella personalizzata sparisce senza variabile.
    assert!(
        !without.cleaner("bbag_npm").options.iter().any(|o| o.id == "custom_cache"),
        "custom_cache dovrebbe sparire senza npm_config_cache"
    );
}

// «Bloccata con X»: i processi dei programmi in uso.

#[test]
fn running_programs_block_vscode_and_ollama() {
    let fake = Fake::new(&[]);
    let vs = fake.cleaner("vscode");
    for exe in ["Code.exe", "Code - Insiders.exe", "VSCodium.exe", "Cursor.exe", "Windsurf.exe"] {
        assert_eq!(running(&vs, &[exe.to_string()]), vec![exe], "{exe}");
    }
    assert!(running(&vs, &["notepad.exe".to_string()]).is_empty());
    let ol = fake.cleaner("ollama");
    assert_eq!(running(&ol, &["ollama.exe".to_string()]), vec!["ollama.exe"]);
}

// Rilevamento: un cleaner compare solo se le sue cartelle esistono.

#[test]
fn cleaners_are_present_only_when_their_folders_exist() {
    let fake = Fake::new(&[]);
    let none = |_: &RegItem| false;
    for name in ["playwright", "huggingface", "ollama", "vscode"] {
        assert!(!present(&fake.cleaner(name), &fake.env, &none), "{name} senza cartelle");
    }
    fs::create_dir_all(fake.local.join("ms-playwright")).unwrap();
    fs::create_dir_all(fake.profile.join(".cache").join("huggingface").join("hub")).unwrap();
    assert!(present(&fake.cleaner("playwright"), &fake.env, &none));
    assert!(present(&fake.cleaner("huggingface"), &fake.env, &none));
    assert!(!present(&fake.cleaner("ollama"), &fake.env, &none));
    // Una sola cartella dell'elenco basta: Cursor senza Code.
    fs::create_dir_all(fake.roaming.join("Cursor").join("logs")).unwrap();
    assert!(present(&fake.cleaner("vscode"), &fake.env, &none));
    // Con la variabile definita ma la cartella assente, non compare.
    let var = Fake::new(&[("OLLAMA_MODELS", "models")]);
    assert!(!present(&var.cleaner("ollama"), &var.env, &none));
}
