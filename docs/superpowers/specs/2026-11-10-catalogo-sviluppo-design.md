# PulisciPC: catalogo generale dei residui di sviluppo

Data: 2026-11-10. Stato: **da approvare**: domande del revisore risolte, risposta dell'utente sul rilevamento inclusa.
Estende `2026-11-10-dev-mode-design.md` (approvato). Dove questo documento non dice nulla, vale quello.

## Intento

PulisciPC è un prodotto da vendere, usabile da tutti, rivale di CCleaner. CCleaner non pulisce le cache e i prodotti di compilazione degli strumenti di sviluppo; PulisciPC sì, ed è ciò che lo distingue. Oggi copre circa 20 voci (7 ecosistemi in `crates/dc-scanner/src/markers.rs`, 11 XML in `D:\Software\BleachBit\cleaners`). Questo documento porta la copertura a un catalogo generale per Windows, ricavato dall'inventario che l'utente ha incollato il 2026-11-10 (circa 150 voci, 20 sezioni, classi C/B/D/L/P).

## Decisioni dell'utente (2026-11-10)

1. PulisciPC deve coprire le voci dell'inventario ("sì").
2. Il catalogo è generale, per i clienti: **non** si tara, si misura o si prova sul PC dell'utente ("ma non devi farlo sul mio pc"; "è un progetto software da vendere e usabile da tutti"; "un rivale di ccleaner").
3. Le dipendenze che si riscaricano ma costano tempo o banda (modelli AI da GB, browser di Playwright, repository Maven, SDK Android) sono **pulibili**.
4. "Il programma deve rilevare in automatico quali sono, tra quelli supportati, i programmi installati effettivamente nel PC", "e mostrare tutti e solo quelli". Niente domande al cliente su cartelle o strumenti.
5. "Va sempre avviato come amministratore", "con lettura veloce MFT".

## Cosa cambia nella spec principale

Le decisioni 5 sostituiscono, in `2026-11-10-dev-mode-design.md`, "la modalità dev no (lavora nel profilo utente)" e "senza admin cammina le cartelle":
- PulisciPC parte sempre come amministratore. *Scelta mia:* manifest incorporato con `requireAdministrator` (UAC all'avvio); non esiste più una modalità senza admin, e il pulsante che rilancia con `runas` (`src/sys.rs:27`) non serve più.
- Tutte le scansioni disco usano la MFT. Eccezione: un'unità fissa non NTFS (per esempio una Dev Drive ReFS, consigliata da Microsoft proprio agli sviluppatori, o exFAT) non ha MFT. *Scelta mia:* lì si camminano le cartelle, con l'avviso già previsto dalla spec principale.
- *Scelta mia:* se l'utente ha alzato i privilegi con un account diverso da quello connesso, profilo e variabili d'ambiente sono di quell'altro account. Al primo avvio la UI lo dice ("Stai pulendo il profilo di <account>") invece di pulire in silenzio il profilo sbagliato.

## Rilevamento automatico

**Strumenti (Parte B e C).** Per ogni strumento del catalogo, una ricetta di rilevamento nel suo XML (un elemento nuovo, per esempio `<detect>`), valutata a ogni analisi, senza lanciare programmi:
- voci di disinstallazione nel registro (`HKLM` e `HKCU` `…\Uninstall`, anche `WOW6432Node`) per nome visualizzato;
- eseguibile nel `PATH` (`cargo.exe`, `pnpm.cmd`, `ollama.exe`…) o in `App Paths`;
- cartelle di installazione note o variabili d'ambiente (`JAVA_HOME`, `ANDROID_HOME`, `CARGO_HOME`…).

Si mostrano **tutti e solo** gli strumenti installati (decisione 4): uno strumento installato compare anche se oggi non ha nulla da pulire (0 B); uno non installato non compare, anche se sul disco restano sue cartelle.

**Progetti (Parte A).** Si trovano da soli, senza chiedere cartelle al primo avvio:
- dalle sessioni degli agenti, come oggi;
- dalla MFT di tutti i dischi fissi NTFS. `dc-disktree` tiene il nome di un file solo se corrisponde a un marcatore della tabella, così la RAM resta sotto il limite di 300 MB della spec principale.

Esclusi sempre: marcatori sotto cartelle artefatto o `NEVER_ENTER`, `%WinDir%`, `Program Files*`, `ProgramData`, `AppData`, `$Recycle.Bin`. Le cartelle aggiuntive in Impostazioni restano, facoltative. I progetti non dipendono dagli strumenti installati: un `node_modules` si pulisce anche se Node non c'è più.

## Obiettivi

- **Parte A, Progetti** (modalità Sviluppatori): nuovi ecosistemi in `markers.rs`, cartelle dentro ogni progetto.
- **Parte B, cache globali** (Pulizia file → gruppo "Sviluppo e AI"): nuovi XML CleanerML inglobati, uno per strumento.
- **Parte C, logica nativa**: immagini di sistema Android non usate da nessun emulatore.
- Ogni voce: etichetta e avviso in italiano corretto; se la voce si riscarica, l'avviso dice **come riaverla** (comando o azione).

## Non obiettivi

- macOS e Linux: PulisciPC usa registro, servizi e MFT, è solo Windows. *Scelta mia:* i percorsi macOS/Linux dell'inventario restano fuori, così come Xcode, CocoaPods, Carthage, simulatori Apple, Homebrew, APT, Nix.
- Docker, Podman, WSL, VM: lo spazio liberato resta dentro il disco virtuale (`ext4.vhdx`, `Docker.raw`) e Windows non lo vede finché non si compatta. *Scelta mia:* fuori da questo documento. Si può fare dopo con la CLI di Docker (`docker builder prune`, mai i volumi) e la compattazione del VHDX.
- File sparsi nei progetti (`*.tsbuildinfo`, `.eslintcache`, `.coverage`, `*.hprof`, ausiliari LaTeX): pesano poco. Il modello di oggi gestisce solo cartelle.
- Cache dentro `node_modules` (`.cache`, `.vite`): si tolgono già con `node_modules`.
- Leggere i percorsi lanciando gli strumenti (`npm config get cache`, `pnpm store path`…): lento, e alcuni comandi scrivono metadati. Si usano variabili d'ambiente e percorsi predefiniti.

## Anti-obiettivi (fallimento anche se "funziona")

- Cancellare dati unici: emulatori Android (AVD), volumi e dischi di container, stato Terraform, checkpoint di training, cronologie locali degli IDE, database, credenziali.
- Rompere in silenzio qualcosa che il cliente usa: un comando nel PATH che sparisce, un `.venv` che punta a un Python cancellato, progetti renv collegati alla cache.
- Togliere una cartella dal nome ambiguo (`dist`, `build`, `out`, `vendor`, `public`, `site`, `artifacts`) che contiene sorgenti.
- Cancellare file mentre lo strumento li sta usando.

## Regole valide per tutte le parti

1. **Classi.** C (cache) e B (build) entrano. D (dipendenze, SDK, modelli, browser) entra, con avviso e comando per riaverla (decisione 3). L (log, report) entra. P (dati unici) **non entra mai**.
2. *Scelta mia:* **D che rompe in silenzio non entra**: toolchain e programmi installati che altri ambienti o il PATH usano. Esempi: `rustup` toolchains, `.cargo\bin`, `go\bin`, `.dotnet\tools`, pipx, Python e tool di uv, versioni di nvm-windows e pyenv, ambienti Conda (`envs`), app e `persist` di Scoop, cache renv (i progetti ci puntano con junction), `bin` della cache Pub. Si riscaricano solo se il cliente sa cosa reinstallare.
3. **Percorsi.** Prima la variabile d'ambiente dello strumento (`CARGO_HOME`, `GRADLE_USER_HOME`, `NUGET_PACKAGES`, `GOCACHE`, `GOMODCACHE`, `HF_HOME`, `OLLAMA_MODELS`, `PLAYWRIGHT_BROWSERS_PATH`…); se non è definita, il percorso predefinito di Windows. Una variabile non definita **scarta** quel percorso, non lo usa alla lettera.
4. **Verifica dei percorsi.** I percorsi delle tabelle vengono dall'inventario incollato e da quanto ne so. Prima di scrivere una voce, il builder controlla il percorso Windows sulla documentazione ufficiale dello strumento; se non lo trova documentato, la voce si toglie e si annota in fondo a questo file.
5. **Visibilità.** Una categoria compare se e solo se lo strumento è installato (vedi Rilevamento automatico).
6. **Default.** Come da spec principale: tutte le opzioni inglobate sono spente di default. La Parte A segue la preselezione di oggi (progetti inattivi).
7. **Processi attivi.** Se lo strumento è in esecuzione, l'opzione è bloccata (`<running>` di CleanerML, come `bbag_jetbrains.xml`). I file bloccati da altri processi si saltano e si contano nel riepilogo.
8. **Cestino.** *Scelta mia:* il flag Cestino/definitivo vale anche qui. Prima di pulire, se il totale supera lo spazio libero del Cestino su quell'unità (quota da registro, occupato da `SHQueryRecycleBin`), la UI lo dice e propone il definitivo per quella pulizia; non si lascia decidere a Windows.
9. **Una regola sola per percorso.** *Scelta mia:* le "mai" di questo documento (P, credenziali, cronologie locali come `LocalHistory` di JetBrains e `User\History`/`Local Storage` di VS Code) filtrano anche le regole Winapp2. Se una sezione Winapp2 copre lo stesso strumento di una categoria inglobata (stessi percorsi o sottopercorsi), la sezione Winapp2 si nasconde: vince la categoria inglobata.
10. **Prove.** Mai sul PC dell'utente: i test usano un profilo finto in una cartella temporanea (variabili `USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `TEMP` e quelle degli strumenti puntate lì) e alberi di progetto costruiti dal test.

## Parte A: Progetti (`markers.rs`)

Stesso modello di oggi: un file marcatore nella cartella del progetto implica le cartelle artefatto accanto.

### Cosa serve al motore

- **Marcatore come sottopercorso o cartella**: `ProjectSettings/ProjectVersion.txt` (Unity), `.storybook/` (Storybook).
- **Artefatto come sottopercorso** (massimo 3 segmenti): `.nx/cache`, `resources/_gen`, `cypress/videos`, `vendor/bundle`, `.cache/clangd`, `project/target`.
- **Artefatto riconosciuto dal contenuto**: una build Meson è qualsiasi sottocartella diretta del progetto che contiene `meson-private/` e `build.ninja`.
- **Nomi ambigui** (segnati † nella tabella): l'artefatto entra solo se il progetto è un repository git e `git check-ignore` dice che la cartella è ignorata e non tracciata. Altrimenti **non compare** (non blocca il progetto come farebbe un artefatto "non verificato"). Serve una funzione `is_ignored` in `dc-git`. *Scelta mia:* se un marcatore del progetto segna un nome come †, la regola † vale per quel nome anche quando un altro marcatore lo accetterebbe senza git (progetto con `package.json` e `CMakeLists.txt`: `build` richiede git). Senza marcatori † (solo Gradle o solo CMake) `build` resta com'è oggi: nessuna regressione.
- *Scelta mia:* la selezione resta per progetto, come oggi (`node_modules` e venv sono già dipendenze e partono con il progetto). Le cartelle D nuove fanno lo stesso; l'anteprima le segna "si riscarica" con il comando per riaverle.
- `crates/dc-cleaner/src/guard.rs::name_plausible` resta una lista chiusa per tipo: ogni nome nuovo va aggiunto lì, altrimenti la pulizia lo rifiuta.
- `NEVER_ENTER` si estende con i nomi non ambigui e pesanti (`.next`, `.nuxt`, `.svelte-kit`, `.angular`, `.turbo`, `.dart_tool`, `.terraform`, `.terragrunt-cache`, `.stack-work`, `dist-newstyle`, `.zig-cache`, `.pio`).
- `ArtifactKind`: varianti nuove o un tipo generico con id del catalogo, a scelta del builder. L'etichetta è in italiano ("Cache Next.js (.next)").

### Tabella

| Ecosistema | Marcatore | Cartelle | Classe |
|---|---|---|---|
| Node | `package.json` | `.parcel-cache`, `.turbo`; `coverage`, `.nyc_output`; `dist`†, `build`†, `out`† | C/L/B |
| Next.js | `next.config.{js,mjs,cjs,ts}` | `.next`; `out`† | C/B |
| Nuxt | `nuxt.config.{js,ts,mjs}` | `.nuxt`, `.output` | C/B |
| SvelteKit | `svelte.config.{js,ts}` | `.svelte-kit` | C/B |
| Angular | `angular.json` | `.angular`; `dist`† | C/B |
| Nx | `nx.json` | `.nx/cache`, `.nx/workspace-data` | C |
| Astro | `astro.config.{mjs,ts,js}` | `.astro`; `dist`† | C/B |
| Docusaurus | `docusaurus.config.{js,ts}` | `.docusaurus`; `build`† | C/B |
| Storybook | cartella `.storybook/` | `storybook-static` | B |
| Playwright | `playwright.config.{ts,js}` | `playwright-report`, `test-results`, `blob-report` | L |
| Cypress | `cypress.config.{ts,js}` | `cypress/videos`, `cypress/screenshots` | L |
| Python | in più: `setup.cfg`, `tox.ini`, `noxfile.py`, `Pipfile` | `.tox`, `.nox`, `*.egg-info`, `.eggs`, `htmlcov`, `.pytype`, `.pyre`, `__pypackages__`; `build`†, `dist`† | D/B/C/L |
| Gradle / Kotlin / Android | in più: `settings.gradle`, `settings.gradle.kts` | `.kotlin`, `.cxx`, `.externalNativeBuild` | C/B |
| sbt | `build.sbt` | `target`, `project/target` | B |
| Leiningen | `project.clj` | `target` | B |
| Clojure CLI | `deps.edn` | `.cpcache` | C |
| .NET | in più: `*.fsproj`, `*.vbproj` | `TestResults`, `BenchmarkDotNet.Artifacts`; `artifacts`† (marcatore `Directory.Build.props`) | L/B |
| C/C++ | `compile_commands.json` | `.cache/clangd`, `.ccls-cache` | C |
| Autotools | `configure.ac` | `autom4te.cache` | C |
| Meson | `meson.build` | cartelle riconosciute dal contenuto (sopra) | B |
| vcpkg | `vcpkg.json` | `vcpkg_installed` | D |
| PlatformIO | `platformio.ini` | `.pio` | B/D |
| Flutter / Dart | `pubspec.yaml` | `.dart_tool`; `build`† | C/B |
| Swift | `Package.swift` | `.build` | B/D |
| Go | `go.mod` | `vendor`† | D |
| PHP | `composer.json` | `vendor`† | D |
| Ruby | `Gemfile` | `vendor/bundle`† | D |
| Elixir | `mix.exs` | `_build`, `deps` | B/D |
| Erlang | `rebar.config` | `_build` | B |
| Haskell | `stack.yaml` / `cabal.project` o `*.cabal` | `.stack-work` / `dist-newstyle` | B |
| OCaml | `dune-project` | `_build`, `_opam` | B/D |
| Zig | `build.zig` | `.zig-cache`, `zig-cache`, `zig-out` | C/B |
| Unity | `ProjectSettings/ProjectVersion.txt` | `Library`, `Temp`, `Obj`, `Logs` | C/B/L |
| Unreal | `*.uproject` | `Intermediate`, `DerivedDataCache` | C/B |
| Godot | `project.godot` | `.godot`, `.import` | C/B |
| Sphinx | `conf.py` con `index.rst` | `_build` | B |
| MkDocs | `mkdocs.yml` | `site`† | B |
| Jekyll | `_config.yml` con `Gemfile` | `_site`, `.jekyll-cache` | B/C |
| Hugo | `hugo.{toml,yaml,json}` | `resources/_gen`; `public`† | C/B |
| Terraform | `*.tf` | `.terraform` (avviso: rifare `terraform init`) | D |
| Terragrunt | `terragrunt.hcl` | `.terragrunt-cache` | C |
| AWS CDK | `cdk.json` | `cdk.out` | B |
| AWS SAM | `samconfig.toml` | `.aws-sam` | B/C |
| Serverless | `serverless.{yml,ts}` | `.serverless` | B |
| dbt | `dbt_project.yml` | `target`, `dbt_packages`, `logs` | B/D/L |

Esclusi dentro i progetti (P o misti): `.ipynb_checkpoints`, `.hypothesis` (database degli esempi che hanno fatto fallire i test), `.vs`, `.idea`, `.Rproj.user`, Unreal `Saved` e `Binaries` (DLL di plugin senza sorgenti), Unity `UserSettings`, `wandb`, `mlruns`, `runs`, `checkpoint-*`, `.dvc/cache`, link `bazel-*` (puntano fuori dal progetto: togliere il link non libera spazio), cache di Laravel e Symfony (contengono `.gitignore` tracciati).

## Parte B: cache globali (XML CleanerML, gruppo "Sviluppo e AI")

Un XML per strumento in `cleaners/`, scritto da zero (nessuna derivazione da BleachBit, nessun prefisso "AGG -"). Gli 11 XML di oggi restano, con le aggiunte indicate.

### Cosa serve al motore CleanerML di `pc-rules` (Fase 2)

- `<var>` con più valori, ognuno con variabile d'ambiente; valore scartato se la variabile non è definita (regola 3).
- `glob` con `*` anche nei segmenti intermedi (`%LocalAppData%\Google\AndroidStudio*\caches`).
- `<running>`, `<warning>` per opzione, rilevamento della presenza (regola 5).
- Cancellazione di file in sola lettura (la cache dei moduli Go li rende tali).

### Tabella (Pulizia file → Sviluppo e AI)

| Strumento | Opzioni e percorsi Windows (variabile d'ambiente prima) | Classe | Note e avviso |
|---|---|---|---|
| npm (esiste) | in più: `%npm_config_cache%` | C/L | — |
| pnpm | store `%LOCALAPPDATA%\pnpm\store`; `%LOCALAPPDATA%\pnpm-cache` | D/C | Si riscarica al prossimo `pnpm install`; i `node_modules` esistenti restano validi (hard link) |
| Yarn | `%LOCALAPPDATA%\Yarn\Cache` (Classic), `%LOCALAPPDATA%\Yarn\Berry\cache`, `%USERPROFILE%\.yarn\berry\cache` | C/D | Mai la `.yarn/cache` dentro i progetti (Zero Installs) |
| Bun | `%BUN_INSTALL_CACHE_DIR%`, `%USERPROFILE%\.bun\install\cache` | C/D | Progetti con linker isolato: rifare `bun install` |
| Deno | `%DENO_DIR%`, `%LOCALAPPDATA%\deno` | C/D | — |
| Electron | `%ELECTRON_CACHE%`, `%LOCALAPPDATA%\electron\Cache`, `%LOCALAPPDATA%\electron-builder\Cache` | C/D | — |
| pip (esiste) | — | C | — |
| uv | `%UV_CACHE_DIR%`, `%LOCALAPPDATA%\uv\cache` | C | Solo la cache: mai Python e tool installati (regola 2) |
| Poetry | `%LOCALAPPDATA%\pypoetry\Cache\cache` e `\artifacts`; opzione separata `\virtualenvs` | C / D | Ambienti: rifare `poetry install` |
| Pipenv | `%WORKON_HOME%`, `%USERPROFILE%\.virtualenvs` | D | Rifare `pipenv install` |
| Conda | `<radice>\pkgs` per `%USERPROFILE%\{miniconda3,anaconda3,miniforge3,mambaforge}` e `%CONDA_PKGS_DIRS%` | C/D | Mai `envs` |
| virtualenv | `%LOCALAPPDATA%\pypa\virtualenv` | C | — |
| pre-commit | `%PRE_COMMIT_HOME%`, `%USERPROFILE%\.cache\pre-commit` | C/D | — |
| pyenv-win | `%USERPROFILE%\.pyenv\pyenv-win\install_cache` | C | Mai le versioni |
| Hugging Face | `%HF_HUB_CACHE%`, `%HF_HOME%\hub`, `%USERPROFILE%\.cache\huggingface\{hub,datasets,xet,assets}` | D | Mai `token` e `stored_tokens`. Si riscaricano al prossimo uso |
| PyTorch | `%TORCH_HOME%\hub`, `%USERPROFILE%\.cache\torch\hub` | D | — |
| Keras | `%USERPROFILE%\.keras\{datasets,models}` | D | Mai `keras.json` |
| Ollama | `%OLLAMA_MODELS%`, `%USERPROFILE%\.ollama\models` | D | Bloccata con `ollama.exe` attivo. Avviso: i modelli spariscono da `ollama list`, rifare `ollama pull` |
| CUDA | `%CUDA_CACHE_PATH%`, `%APPDATA%\NVIDIA\ComputeCache` | C | Nel cleaner delle shader GPU |
| Maven | `%USERPROFILE%\.m2\repository`; `%USERPROFILE%\.m2\wrapper\dists` | D | Avviso: gli artefatti fatti con `mvn install` esistono solo qui |
| Gradle | `%GRADLE_USER_HOME%` o `%USERPROFILE%\.gradle`: `caches`, `wrapper\dists`, `native`, `jdks`; log `daemon\*\*.out.log` | C/D/L | Bloccata con un daemon Gradle attivo. Mai `gradle.properties`, `init.d` |
| Kotlin/Native | `%KONAN_DATA_DIR%`, `%USERPROFILE%\.konan` | D | — |
| sbt / Ivy / Coursier | `%USERPROFILE%\.sbt\boot`, `%USERPROFILE%\.ivy2\cache`, `%COURSIER_CACHE%`, `%LOCALAPPDATA%\Coursier\Cache` | C/D | Mai `.ivy2\local` |
| Android | `%ANDROID_USER_HOME%\cache` o `%USERPROFILE%\.android\cache`; SDK (`%ANDROID_HOME%`, `%ANDROID_SDK_ROOT%`, `%LOCALAPPDATA%\Android\Sdk`) `\.temp`, `\.downloadIntermediates`; `%LOCALAPPDATA%\Google\AndroidStudio*\caches`, `\log` | C/L | Mai `avd`, `debug.keystore`, chiavi `adbkey`, `LocalHistory`. Bloccata con `studio64.exe` |
| Pub (Dart/Flutter) | `%PUB_CACHE%` o `%LOCALAPPDATA%\Pub\Cache`: `hosted`, `git` | D | Mai `bin` e `global_packages` |
| NuGet | `%NUGET_PACKAGES%` o `%USERPROFILE%\.nuget\packages`; `%NUGET_HTTP_CACHE_PATH%` o `%LOCALAPPDATA%\NuGet\v3-cache`; `plugins-cache`; `%TEMP%\NuGetScratch` | D/C | Si riscarica al prossimo restore |
| Visual Studio | `%LOCALAPPDATA%\Microsoft\VisualStudio\*\ComponentModelCache` | C | Bloccata con `devenv.exe` |
| .NET single-file | `%DOTNET_BUNDLE_EXTRACT_BASE_DIR%`, `%TEMP%\.net` | C | — |
| Cargo (esiste) | in più: `registry\src`, `registry\index`, `git\checkouts`, `git\db`, con `%CARGO_HOME%` | C/D | Mai `bin`, `credentials.toml`, `config.toml` |
| rustup | `%RUSTUP_HOME%` o `%USERPROFILE%\.rustup`: `downloads`, `tmp` | C | Mai `toolchains` |
| Go (esiste) | in più: moduli `%GOMODCACHE%` o `%USERPROFILE%\go\pkg\mod`, con `%GOCACHE%` | D | File in sola lettura. Mai `go\bin` |
| ccache / sccache | `%CCACHE_DIR%`, `%LOCALAPPDATA%\ccache`; `%SCCACHE_DIR%`, `%LOCALAPPDATA%\Mozilla\sccache\cache` | C | Bloccata con `sccache.exe` |
| vcpkg | `%VCPKG_DEFAULT_BINARY_CACHE%`, `%LOCALAPPDATA%\vcpkg\archives`; con `%VCPKG_ROOT%`: `buildtrees`, `packages`, `downloads` | C/B/D | — |
| Conan | `%CONAN_HOME%\p` o `%USERPROFILE%\.conan2\p` | D | Avviso: i pacchetti fatti con `conan create` esistono solo qui |
| Composer | `%COMPOSER_CACHE_DIR%`, `%LOCALAPPDATA%\Composer` | C | Mai `%APPDATA%\Composer` |
| Bundler | `%USERPROFILE%\.bundle\cache` | C | — |
| Hex / rebar3 | `%USERPROFILE%\.hex\packages`, `%USERPROFILE%\.cache\rebar3` | C | — |
| Cabal | `%APPDATA%\cabal\packages` | C | — |
| Julia | `%USERPROFILE%\.julia\compiled` | C | Solo `compiled` |
| Zig | `%ZIG_GLOBAL_CACHE_DIR%`, `%LOCALAPPDATA%\zig` | C | — |
| PlatformIO | `%USERPROFILE%\.platformio\.cache`; opzione separata `packages`, `platforms` | C / D | Toolchain riscaricate alla prossima build |
| Playwright | `%PLAYWRIGHT_BROWSERS_PATH%`, `%LOCALAPPDATA%\ms-playwright` | D | Avviso: rifare `npx playwright install` |
| Puppeteer | `%PUPPETEER_CACHE_DIR%`, `%USERPROFILE%\.cache\puppeteer` | D | Rifare `npm install` |
| Cypress | `%CYPRESS_CACHE_FOLDER%`, `%LOCALAPPDATA%\Cypress\Cache` | D | Rifare `npx cypress install` |
| VS Code e derivati | `%APPDATA%\{Code,Code - Insiders,VSCodium,Cursor,Windsurf}\{Cache,CachedData,Code Cache,GPUCache,CachedExtensionVSIXs,logs}` | C/L | Mai `User` (cronologia locale, workspaceStorage) e `Backups`. Bloccata con l'editor aperto |
| JetBrains (esiste) | — | C | Già esclude local history |
| Minikube | `%MINIKUBE_HOME%\.minikube\cache`, `%USERPROFILE%\.minikube\cache` | C/D | — |
| Vagrant | `%VAGRANT_HOME%\boxes`, `%USERPROFILE%\.vagrant.d\boxes` | D | Avviso: rifare `vagrant box add` |
| kubectl / Helm | `%USERPROFILE%\.kube\cache`; `%HELM_CACHE_HOME%`, `%TEMP%\helm` | C | Mai `.kube\config` |
| Terraform / Pulumi | `%TF_PLUGIN_CACHE_DIR%`, `%APPDATA%\terraform.d\plugin-cache`; `%USERPROFILE%\.pulumi\plugins` | D | — |
| Firebase | `%USERPROFILE%\.cache\firebase\emulators` | D | Mai gli export dei dati |
| Scoop / Chocolatey / WinGet | `%SCOOP%\cache` o `%USERPROFILE%\scoop\cache`; `%TEMP%\chocolatey`; `%TEMP%\WinGet` e i log `DiagOutputDir` di `Microsoft.DesktopAppInstaller_8wekyb3d8bbwe` | C/L | Mai `apps`, `persist` |
| pytest | `%TEMP%\pytest-of-*` | C | — |

## Parte C: logica nativa

**Immagini di sistema Android non usate.** Le immagini in `<SDK>\system-images\android-*\*\*` pesano 1-3 GB l'una. Un'immagine si offre solo se nessun AVD la usa: si leggono `image.sysdir.1` dai `config.ini` in `%ANDROID_AVD_HOME%`, `%ANDROID_USER_HOME%\avd` o `%USERPROFILE%\.android\avd`. Se un `config.ini` non si legge, non si offre nessuna immagine (fail closed). Bloccata con `emulator.exe` o `qemu-system-*.exe` attivi. Avviso: riscaricabile da Android Studio, SDK Manager.

## Ordine e stime

1. Parte A: dopo la Fase 1, in un task proprio (tocca `dc-scanner`, `dc-git`, `dc-cleaner`). Circa 3 ore.
2. Parte B: dentro la Fase 2, dopo il motore `pc-rules`. Circa 4 ore: la maggior parte è verificare i percorsi sulla documentazione (regola 4).
3. Parte C: con la Fase 3 (codice nativo in `cleaner.rs`). Circa 1 ora.

## Verifica

- Parte A: un test per ogni riga della tabella, con un albero finto (marcatore + cartella → artefatto trovato con l'etichetta giusta); per i nomi † tre casi: repo con cartella ignorata (trovata), non ignorata (assente), senza git (assente). `guard.rs` rifiuta un nome non in lista.
- Parte B: ogni XML si carica dall'exe; con il profilo finto (regola 8) l'anteprima elenca i file attesi e **non** quelli esclusi (`token` di Hugging Face, `credentials.toml` di Cargo, `User\History` di VS Code, `avd` di Android); con variabile d'ambiente impostata vince il suo percorso; con variabile non definita nessun percorso letterale `%…%`.
- Parte C: un AVD finto che usa un'immagine e un'altra immagine libera: si offre solo la seconda; un `config.ini` illeggibile: nessuna offerta.
- Rilevamento: con registro e `PATH` finti (chiavi di prova sotto `HKCU\Software\PulisciPC-test`, `PATH` del processo puntato a una cartella temporanea) uno strumento installato senza cartelle compare con 0 B, uno con sole cartelle e nessuna installazione non compare; la ricerca dei progetti trova un `Cargo.toml` in un albero finto e ignora quello sotto `node_modules` e sotto `AppData`.

## Lasciato al builder

- Come rappresentare i tipi nuovi in `ArtifactKind`.
- Come riconoscere un daemon Gradle attivo.
- Le etichette esatte, purché in italiano corretto e senza nomi di prodotti terzi nella UI oltre a quello dello strumento.
- Le voci da togliere perché il percorso non è documentato (regola 4): si annotano qui sotto.

## Percorsi non verificati e tolti

(vuoto finché il builder non verifica)
