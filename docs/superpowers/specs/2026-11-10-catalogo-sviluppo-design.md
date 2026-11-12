# PulisciPC: catalogo generale dei residui di sviluppo

Data: 2026-11-10. Stato: **da approvare**: rivista dopo la decisione 6 (l'app propone, l'utente sceglie).
Estende `2026-11-10-dev-mode-design.md` (approvato). Dove questo documento non dice nulla, vale quello.

## Intento

PulisciPC è un prodotto da vendere, usabile da tutti, rivale di CCleaner. CCleaner non pulisce le cache e i prodotti di compilazione degli strumenti di sviluppo; PulisciPC sì, ed è ciò che lo distingue. Oggi copre circa 20 voci (7 ecosistemi in `crates/dc-scanner/src/markers.rs`, 11 XML in `D:\Software\BleachBit\cleaners`). Questo documento porta la copertura a un catalogo generale per Windows, ricavato dall'inventario che l'utente ha incollato il 2026-11-10 (circa 150 voci, 20 sezioni, classi C/B/D/L/P).

## Decisioni dell'utente (2026-11-10)

1. PulisciPC deve coprire le voci dell'inventario ("sì").
2. Il catalogo è generale, per i clienti: **non** si tara, si misura o si prova sul PC dell'utente ("ma non devi farlo sul mio pc"; "è un progetto software da vendere e usabile da tutti"; "un rivale di ccleaner").
3. Le dipendenze che si riscaricano ma costano tempo o banda (modelli AI da GB, browser di Playwright, repository Maven, SDK Android) sono **pulibili**.
4. "Il programma deve rilevare in automatico quali sono, tra quelli supportati, i programmi installati effettivamente nel PC", "e mostrare tutti e solo quelli". Niente domande al cliente su cartelle o strumenti.
5. "Va sempre avviato come amministratore", "con lettura veloce MFT".
6. "L'utente sceglie cosa eliminare o cosa no: l'app propone, l'utente seleziona, come in CCleaner." In particolare non vanno bene due esclusioni della bozza: i programmi installati che altri usano (toolchain di rustup, Python di uv, versioni di nvm, `.cargo\bin`) e Docker, WSL e macchine virtuali. Entrano anche quelli.
7. "La pulizia deve essere veloce e multithreading, con worker che si dividono il lavoro, e dare un resoconto finale."

8. "La UI deve essere quella di ccleaner64 di base": conferma la decisione 10 della spec principale. Tutte le parti nuove (livelli e avvisi, selezione per cartella, Parte D, avanzamento, resoconto finale) usano barra laterale, card, colori e componenti di `src/main.rs`; nessuna schermata con uno stile proprio.

## Cosa cambia nella spec principale

Le decisioni 5 sostituiscono, in `2026-11-10-dev-mode-design.md`, "la modalità dev no (lavora nel profilo utente)" e "senza admin cammina le cartelle":
- PulisciPC parte sempre come amministratore. *Scelta mia:* manifest incorporato con `requireAdministrator` (UAC all'avvio); non esiste più una modalità senza admin, e il pulsante che rilancia con `runas` (`src/sys.rs:27`) non serve più.
- Tutte le scansioni disco usano la MFT. Eccezione: un'unità fissa non NTFS (per esempio una Dev Drive ReFS, consigliata da Microsoft proprio agli sviluppatori, o exFAT) non ha MFT. *Scelta mia:* lì si camminano le cartelle, con l'avviso già previsto dalla spec principale.
- *Scelta mia:* se l'utente ha alzato i privilegi con un account diverso da quello connesso, profilo e variabili d'ambiente sono di quell'altro account. Al primo avvio la UI lo dice ("Stai pulendo il profilo di <account>") invece di pulire in silenzio il profilo sbagliato.

## Stato del codice al 2026-11-10, ore 11:15 (commit `b232641`)

Un'altra sessione sta implementando la Fase 1 in questo repository. Questo documento si applica **sopra** quel codice; il piano parte dal codice che trova, non da devclean.

- Esistono `crates/dc-app` (colla senza UI: `run_scan`, `build_plan`, `execute_plan`, `delete_archived`, impostazioni in `%APPDATA%\PulisciPC`) e `crates/dc-sessions`. In corso, non ancora in git: `src/dev.rs` e la scheda Sviluppatori in `src/main.rs`. Il piano di questo documento non tocca quei file finché la Fase 1 non è chiusa.
- **Selezione per cartella.** Oggi `dc_app::build_plan(scan, selected_ids, mode)` e `dc_engine::build_plan` ricevono id di **progetto**, e `checked_plan` (`crates/dc-app/src/lib.rs:435`) ricostruisce il piano canonico per quegli id e rifiuta ciò che non coincide. Vanno cambiati tutti e tre: id di artefatto, e `checked_plan` che accetta un sottoinsieme degli artefatti di un progetto ma continua a rifiutare percorsi, tipi e azioni che l'analisi non ha trovato.
- **Ricerca dei progetti.** Oggi le radici predefinite sono le cartelle convenzionali sotto la home che esistono (`default_settings_for`, `HOME_PROJECT_DIRS`, `crates/dc-app/src/lib.rs:69`), camminate solo con `scan_roots` acceso; `dc_disktree::build_for_roots` legge i volumi di quelle radici. Qui si aggiunge la ricerca dei marcatori su tutti i dischi fissi NTFS dalla MFT; `HOME_PROJECT_DIRS` resta per i dischi senza MFT.
- **Amministratore.** Oggi `run_scan(settings, admin, step)` e `build_for_roots(roots, admin, …)` hanno il ramo senza admin. Con il manifest `requireAdministrator` il parametro resta ma è sempre vero; il ramo che cammina le cartelle resta come ripiego (unità non NTFS, panico nella lettura MFT, già gestito da `guarded_index`), non come modalità.
- **Worker.** `execute_plan` passa da `dc_cleaner::DefaultRemover.execute`, in sequenza: i worker entrano lì.

## Rilevamento automatico

**Strumenti (Parti B, C, D).** Decisione dell'utente (2026-11-10, precisa la 4): una categoria compare **se le sue cartelle esistono** sul disco. Il segnale è la lettura MFT, non il registro né il `PATH`: molti strumenti di sviluppo non lasciano una voce di disinstallazione, e le cartelle sono ciò che si pulisce. Conseguenze: gli avanzi di un programma disinstallato compaiono (per esempio l'SDK Android dopo aver tolto Android Studio); uno strumento installato che non ha lasciato cartelle non compare. Registro e `PATH` servono solo dove occorre un comando (`docker`, `wsl`, `VBoxManage`): se il comando manca, la voce che lo richiede non compare.

**Progetti (Parte A).** Si trovano da soli, senza chiedere cartelle al primo avvio:
- dalle sessioni degli agenti, come oggi;
- dalla MFT di tutti i dischi fissi NTFS. `dc-disktree` tiene il nome di un file solo se corrisponde a un marcatore della tabella, così la RAM resta sotto il limite di 300 MB della spec principale.

Esclusi sempre: marcatori sotto cartelle artefatto o `NEVER_ENTER`, `%WinDir%`, `Program Files*`, `ProgramData`, `AppData`, `$Recycle.Bin`. Le cartelle aggiuntive in Impostazioni restano, facoltative.

**Incrocio con mukade.** La chiave è il percorso: dalla cartella di una sessione si risale alla radice del progetto (radice git o primo marcatore, come oggi in `crates/dc-scanner/src/sessions.rs`) e la si confronta con i progetti trovati dalla MFT. Progetto con sessioni: data di ultimo uso dalla sessione; sessione viva: non toccabile. Approvato dall'utente (2026-11-10): progetto senza sessioni (cambia `crates/dc-engine/src/lib.rs:171`, dove oggi non è mai preselezionato): vale l'ultimo commit git o l'ultima modifica dei file, con la stessa soglia di inattività (14 giorni), perché la maggior parte dei clienti non usa agenti. Cartella di sessione su un disco senza MFT: si cammina. I progetti non dipendono dagli strumenti installati: un `node_modules` si pulisce anche se Node non c'è più.

## Obiettivi

- **Parte A, Progetti** (modalità Sviluppatori): nuovi ecosistemi in `markers.rs`, cartelle dentro ogni progetto.
- **Parte B, cache globali** (Pulizia file → gruppo "Sviluppo e AI"): nuovi XML CleanerML inglobati, uno per strumento.
- **Parte C, logica nativa**: emulatori Android e immagini di sistema non usate.
- **Parte D, Docker, Podman, WSL e macchine virtuali**: pulizia con i loro strumenti, poi lo spazio torna a Windows.
- Ogni voce: etichetta e avviso in italiano corretto; se la voce si riscarica, l'avviso dice **come riaverla** (comando o azione).

## Non obiettivi

- macOS: PulisciPC usa registro, servizi e MFT, è solo Windows. I percorsi macOS dell'inventario restano fuori, così come Xcode, CocoaPods, Carthage, simulatori Apple, Homebrew. I percorsi Linux servono solo dentro le distro WSL (Parte D).
- File sparsi nei progetti (`*.tsbuildinfo`, `.eslintcache`, `.coverage`, `*.hprof`, ausiliari LaTeX): pesano poco. Il modello di oggi gestisce solo cartelle.
- Cache dentro `node_modules` (`.cache`, `.vite`): si tolgono già con `node_modules`.
- Leggere i percorsi lanciando gli strumenti (`npm config get cache`, `pnpm store path`…): lento, e alcuni comandi scrivono metadati. Si usano variabili d'ambiente e percorsi predefiniti.

## Anti-obiettivi (fallimento anche se "funziona")

- Cancellare qualcosa che l'utente non ha spuntato, o dati unici (emulatori, volumi Docker, dischi di VM, stato Terraform, checkpoint di training, cronologie locali degli IDE) senza che abbia letto e confermato l'avviso.
- Rompere qualcosa senza dirlo prima: se una voce fa sparire un comando dal PATH o rompe un `.venv`, l'avviso lo scrive con i nomi di cosa si rompe.
- Preselezionare una cartella dal nome ambiguo (`dist`, `build`, `out`, `vendor`, `public`, `site`, `artifacts`) che contiene sorgenti.
- Cancellare file mentre lo strumento li sta usando.

## Regole valide per tutte le parti

1. **L'app propone, l'utente sceglie** (decisione 6). Ogni voce dell'inventario compare come opzione con la sua casella. Il livello decide solo la preselezione e l'avviso (*scelta mia*, come CCleaner):
   - **Consigliato**, preselezionato: C, B e L, si rigenerano senza scaricare nulla.
   - **Facoltativo**, non selezionato, avviso "si riscarica" con il comando per riaverla: D non in uso.
   - **Attenzione**, non selezionato, avviso rosso da confermare: P (dati unici), D in uso, nomi ambigui non ignorati da git, cartelle tracciate da git.

   Gli 11 XML di oggi restano spenti di default, come deciso nella spec principale. *Scelta mia:* unica esclusione, i file di credenziali e configurazione (token, chiavi, `credentials.toml`, `config.toml`, `settings.json`, `.kube\config`, `gradle.properties`, `debug.keystore`, `adbkey`): non sono residui e non liberano spazio. Nelle tabelle sono segnati 🔒.
2. **In uso.** Toolchain e programmi installati entrano (decisione 6): toolchain di `rustup`, `.cargo\bin`, `go\bin`, `.dotnet\tools`, pipx, Python e tool di uv, versioni di nvm-windows e pyenv, ambienti Conda (`envs`), versioni vecchie e `persist` di Scoop, cache renv, `bin` della cache Pub. *Scelta mia:* l'app capisce cosa è in uso: la toolchain predefinita (`settings.toml` di rustup), la versione attiva di nvm, il Python a cui punta un venv trovato dall'analisi (`home` in `pyvenv.cfg`), i progetti renv collegati alla cache. Le voci in uso vanno al livello Attenzione e l'avviso dice cosa si rompe ("2 venv usano questo Python: smetteranno di funzionare"); le altre sono Facoltative.
3. **Percorsi.** Prima la variabile d'ambiente dello strumento (`CARGO_HOME`, `GRADLE_USER_HOME`, `NUGET_PACKAGES`, `GOCACHE`, `GOMODCACHE`, `HF_HOME`, `OLLAMA_MODELS`, `PLAYWRIGHT_BROWSERS_PATH`…); se non è definita, il percorso predefinito di Windows. Una variabile non definita **scarta** quel percorso, non lo usa alla lettera.
4. **Verifica dei percorsi.** I percorsi delle tabelle vengono dall'inventario incollato e da quanto ne so. Prima di scrivere una voce, il builder controlla il percorso Windows sulla documentazione ufficiale dello strumento; se non lo trova documentato, la voce si toglie e si annota in fondo a questo file.
5. **Visibilità.** Una categoria compare se e solo se le sue cartelle esistono (vedi Rilevamento automatico).
6. **Selezione.** Come in CCleaner: ogni voce ha la sua casella, quella del gruppo seleziona o toglie tutte le sue voci, il totale selezionato si aggiorna subito.
7. **Processi attivi.** Se lo strumento è in esecuzione (`<running>` di CleanerML), alla pulizia l'app chiede di chiuderlo o di saltare quella voce, come fa CCleaner con i browser. "Bloccata con X" nelle tabelle vuol dire questo. I file bloccati da altri processi si saltano e si contano nel riepilogo.
8. **Cestino.** *Scelta mia:* il flag Cestino/definitivo vale anche qui. Prima di pulire, se il totale supera lo spazio libero del Cestino su quell'unità (quota da registro, occupato da `SHQueryRecycleBin`), la UI lo dice e propone il definitivo per quella pulizia; non si lascia decidere a Windows.
9. **Una regola sola per percorso.** *Scelta mia:* i livelli valgono anche per le regole Winapp2: una regola che tocca dati P o in uso (per esempio `LocalHistory` di JetBrains, `User\History`/`Local Storage` di VS Code) prende il livello Attenzione; una che tocca file 🔒 si scarta. Se una sezione Winapp2 copre lo stesso strumento di una categoria inglobata (stessi percorsi o sottopercorsi), la sezione Winapp2 si nasconde: vince la categoria inglobata.
10. **Prove.** Mai sul PC dell'utente: i test usano un profilo finto in una cartella temporanea (variabili `USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `TEMP` e quelle degli strumenti puntate lì) e alberi di progetto costruiti dal test.

## Parte A: Progetti (`markers.rs`)

Stesso modello di oggi: un file marcatore nella cartella del progetto implica le cartelle artefatto accanto.

### Cosa serve al motore

- **Marcatore come sottopercorso o cartella**: `ProjectSettings/ProjectVersion.txt` (Unity), `.storybook/` (Storybook).
- **Artefatto come sottopercorso** (massimo 3 segmenti): `.nx/cache`, `resources/_gen`, `cypress/videos`, `vendor/bundle`, `.cache/clangd`, `project/target`.
- **Artefatto riconosciuto dal contenuto**: una build Meson è qualsiasi sottocartella diretta del progetto che contiene `meson-private/` e `build.ninja`.
- **Nomi ambigui** (segnati † nella tabella): l'artefatto entra solo se il progetto è un repository git e `git check-ignore` dice che la cartella è ignorata e non tracciata. Altrimenti compare al livello Attenzione ("potrebbe contenere sorgenti"), non preselezionato. Serve una funzione `is_ignored` in `dc-git`. *Scelta mia:* se un marcatore del progetto segna un nome come †, la regola † vale per quel nome anche quando un altro marcatore lo accetterebbe senza git (progetto con `package.json` e `CMakeLists.txt`: `build` richiede git). Senza marcatori † (solo Gradle o solo CMake) `build` resta com'è oggi: nessuna regressione.
- **Selezione per cartella** (decisione 6, cambia rispetto a oggi): ogni artefatto ha la sua casella, la riga del progetto le seleziona tutte. Preselezionate solo le voci Consigliate dei progetti inattivi; nei progetti attivi niente. Gli artefatti "non verificati" e quelli tracciati da git non bloccano più il progetto: vanno al livello Attenzione. Cambiano `Artifact` (serve un id), `build_plan` e `checked_plan` (vedi Stato del codice) e la griglia.
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

Livello Attenzione dentro i progetti (P o misti, mai preselezionati): `.ipynb_checkpoints`, `.hypothesis` (database degli esempi che hanno fatto fallire i test), `.vs`, `.idea`, `.Rproj.user`, `.yarn/cache` (Zero Installs: sta nel repository), Unreal `Saved` e `Binaries` (DLL di plugin senza sorgenti), Unity `UserSettings`, `wandb`, `mlruns`, `runs`, `checkpoint-*`, `.dvc/cache`, cache di Laravel e Symfony (contengono `.gitignore` tracciati). I link `bazel-*` non liberano spazio: lo spazio vero è la radice di output di Bazel, voce della Parte B.

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
| Yarn | `%LOCALAPPDATA%\Yarn\Cache` (Classic), `%LOCALAPPDATA%\Yarn\Berry\cache`, `%USERPROFILE%\.yarn\berry\cache` | C/D | La `.yarn/cache` dentro i progetti è in Parte A, livello Attenzione |
| Bun | `%BUN_INSTALL_CACHE_DIR%`, `%USERPROFILE%\.bun\install\cache` | C/D | Progetti con linker isolato: rifare `bun install` |
| Deno | `%DENO_DIR%`, `%LOCALAPPDATA%\deno` | C/D | — |
| Electron | `%ELECTRON_CACHE%`, `%LOCALAPPDATA%\electron\Cache`, `%LOCALAPPDATA%\electron-builder\Cache` | C/D | — |
| pip (esiste) | — | C | — |
| uv | `%UV_CACHE_DIR%`, `%LOCALAPPDATA%\uv\cache` | C | Python e tool installati: opzioni separate, regola 2 |
| Poetry | `%LOCALAPPDATA%\pypoetry\Cache\cache` e `\artifacts`; opzione separata `\virtualenvs` | C / D | Ambienti: rifare `poetry install` |
| Pipenv | `%WORKON_HOME%`, `%USERPROFILE%\.virtualenvs` | D | Rifare `pipenv install` |
| Conda | `<radice>\pkgs` per `%USERPROFILE%\{miniconda3,anaconda3,miniforge3,mambaforge}` e `%CONDA_PKGS_DIRS%` | C/D | `envs`: opzione separata, regola 2 |
| virtualenv | `%LOCALAPPDATA%\pypa\virtualenv` | C | — |
| pre-commit | `%PRE_COMMIT_HOME%`, `%USERPROFILE%\.cache\pre-commit` | C/D | — |
| pyenv-win | `%USERPROFILE%\.pyenv\pyenv-win\install_cache` | C | Versioni installate: opzione separata, regola 2 |
| Hugging Face | `%HF_HUB_CACHE%`, `%HF_HOME%\hub`, `%USERPROFILE%\.cache\huggingface\{hub,datasets,xet,assets}` | D | 🔒 `token` e `stored_tokens`. Si riscaricano al prossimo uso |
| PyTorch | `%TORCH_HOME%\hub`, `%USERPROFILE%\.cache\torch\hub` | D | — |
| Keras | `%USERPROFILE%\.keras\{datasets,models}` | D | 🔒 `keras.json` |
| Ollama | `%OLLAMA_MODELS%`, `%USERPROFILE%\.ollama\models` | D | Bloccata con `ollama.exe` attivo. Avviso: i modelli spariscono da `ollama list`, rifare `ollama pull` |
| CUDA | `%CUDA_CACHE_PATH%`, `%APPDATA%\NVIDIA\ComputeCache` | C | Nel cleaner delle shader GPU |
| Maven | `%USERPROFILE%\.m2\repository`; `%USERPROFILE%\.m2\wrapper\dists` | D | Avviso: gli artefatti fatti con `mvn install` esistono solo qui |
| Gradle | `%GRADLE_USER_HOME%` o `%USERPROFILE%\.gradle`: `caches`, `wrapper\dists`, `native`, `jdks`; log `daemon\*\*.out.log` | C/D/L | Bloccata con un daemon Gradle attivo. 🔒 `gradle.properties`, `init.d` |
| Kotlin/Native | `%KONAN_DATA_DIR%`, `%USERPROFILE%\.konan` | D | — |
| sbt / Ivy / Coursier | `%USERPROFILE%\.sbt\boot`, `%USERPROFILE%\.ivy2\cache`, `%COURSIER_CACHE%`, `%LOCALAPPDATA%\Coursier\Cache` | C/D | `.ivy2\local`: Attenzione, i pacchetti pubblicati in locale esistono solo qui |
| Android | `%ANDROID_USER_HOME%\cache` o `%USERPROFILE%\.android\cache`; SDK (`%ANDROID_HOME%`, `%ANDROID_SDK_ROOT%`, `%LOCALAPPDATA%\Android\Sdk`) `\.temp`, `\.downloadIntermediates`; `%LOCALAPPDATA%\Google\AndroidStudio*\caches`, `\log` | C/L | 🔒 `debug.keystore`, `adbkey`. `LocalHistory`: Attenzione. Emulatori: Parte C. Bloccata con `studio64.exe` |
| Pub (Dart/Flutter) | `%PUB_CACHE%` o `%LOCALAPPDATA%\Pub\Cache`: `hosted`, `git` | D | `bin` e `global_packages`: regola 2 |
| NuGet | `%NUGET_PACKAGES%` o `%USERPROFILE%\.nuget\packages`; `%NUGET_HTTP_CACHE_PATH%` o `%LOCALAPPDATA%\NuGet\v3-cache`; `plugins-cache`; `%TEMP%\NuGetScratch` | D/C | Si riscarica al prossimo restore |
| Visual Studio | `%LOCALAPPDATA%\Microsoft\VisualStudio\*\ComponentModelCache` | C | Bloccata con `devenv.exe` |
| .NET single-file | `%DOTNET_BUNDLE_EXTRACT_BASE_DIR%`, `%TEMP%\.net` | C | — |
| Cargo (esiste) | in più: `registry\src`, `registry\index`, `git\checkouts`, `git\db`, con `%CARGO_HOME%` | C/D | `bin`: regola 2. 🔒 `credentials.toml`, `config.toml` |
| rustup | `%RUSTUP_HOME%` o `%USERPROFILE%\.rustup`: `downloads`, `tmp` | C | `toolchains`: regola 2 (la predefinita è in uso) |
| Go (esiste) | in più: moduli `%GOMODCACHE%` o `%USERPROFILE%\go\pkg\mod`, con `%GOCACHE%` | D | File in sola lettura. `go\bin`: regola 2 |
| ccache / sccache | `%CCACHE_DIR%`, `%LOCALAPPDATA%\ccache`; `%SCCACHE_DIR%`, `%LOCALAPPDATA%\Mozilla\sccache\cache` | C | Bloccata con `sccache.exe` |
| vcpkg | `%VCPKG_DEFAULT_BINARY_CACHE%`, `%LOCALAPPDATA%\vcpkg\archives`; con `%VCPKG_ROOT%`: `buildtrees`, `packages`, `downloads` | C/B/D | — |
| Conan | `%CONAN_HOME%\p` o `%USERPROFILE%\.conan2\p` | D | Avviso: i pacchetti fatti con `conan create` esistono solo qui |
| Composer | `%COMPOSER_CACHE_DIR%`, `%LOCALAPPDATA%\Composer` | C | `%APPDATA%\Composer\vendor` (pacchetti globali): regola 2. 🔒 `config.json`, `auth.json` |
| Bundler | `%USERPROFILE%\.bundle\cache` | C | — |
| Hex / rebar3 | `%USERPROFILE%\.hex\packages`, `%USERPROFILE%\.cache\rebar3` | C | — |
| Cabal | `%APPDATA%\cabal\packages` | C | — |
| Julia | `%USERPROFILE%\.julia\compiled` | C | Solo `compiled` |
| Zig | `%ZIG_GLOBAL_CACHE_DIR%`, `%LOCALAPPDATA%\zig` | C | — |
| PlatformIO | `%USERPROFILE%\.platformio\.cache`; opzione separata `packages`, `platforms` | C / D | Toolchain riscaricate alla prossima build |
| Playwright | `%PLAYWRIGHT_BROWSERS_PATH%`, `%LOCALAPPDATA%\ms-playwright` | D | Avviso: rifare `npx playwright install` |
| Puppeteer | `%PUPPETEER_CACHE_DIR%`, `%USERPROFILE%\.cache\puppeteer` | D | Rifare `npm install` |
| Cypress | `%CYPRESS_CACHE_FOLDER%`, `%LOCALAPPDATA%\Cypress\Cache` | D | Rifare `npx cypress install` |
| VS Code e derivati | `%APPDATA%\{Code,Code - Insiders,VSCodium,Cursor,Windsurf}\{Cache,CachedData,Code Cache,GPUCache,CachedExtensionVSIXs,logs}` | C/L | `User\History`, `User\workspaceStorage`, `Backups` (modifiche non salvate): Attenzione. 🔒 `settings.json`, `keybindings.json`. Bloccata con l'editor aperto |
| JetBrains (esiste) | — | C | In più `LocalHistory`: Attenzione |
| Minikube | `%MINIKUBE_HOME%\.minikube\cache`, `%USERPROFILE%\.minikube\cache` | C/D | — |
| Vagrant | `%VAGRANT_HOME%\boxes`, `%USERPROFILE%\.vagrant.d\boxes` | D | Avviso: rifare `vagrant box add` |
| kubectl / Helm | `%USERPROFILE%\.kube\cache`; `%HELM_CACHE_HOME%`, `%TEMP%\helm` | C | 🔒 `.kube\config` |
| Terraform / Pulumi | `%TF_PLUGIN_CACHE_DIR%`, `%APPDATA%\terraform.d\plugin-cache`; `%USERPROFILE%\.pulumi\plugins` | D | — |
| Firebase | `%USERPROFILE%\.cache\firebase\emulators` | D | Gli export dei dati stanno dove li salva l'utente: non si cercano |
| Scoop / Chocolatey / WinGet | `%SCOOP%\cache` o `%USERPROFILE%\scoop\cache`; `%TEMP%\chocolatey`; `%TEMP%\WinGet` e i log `DiagOutputDir` di `Microsoft.DesktopAppInstaller_8wekyb3d8bbwe` | C/L | Versioni vecchie in `apps\<app>\<versione>` (non `current`): Facoltativo. `persist`: Attenzione |
| pytest | `%TEMP%\pytest-of-*` | C | — |
| Bazel | radice di output `%USERPROFILE%\_bazel_<utente>` | C/B | Come `bazel clean --expunge` su tutti i workspace. Bloccata con un server Bazel attivo |

## Parte C: logica nativa

**Emulatori Android (AVD).** Ogni AVD in `%ANDROID_AVD_HOME%`, `%ANDROID_USER_HOME%\avd` o `%USERPROFILE%\.android\avd` compare con nome, peso e ultimo avvio. Voci: snapshot (`snapshots\`, Facoltativo: il prossimo avvio è a freddo); dati del dispositivo (`userdata-qemu.img*`, `cache.img*`, Attenzione: il dispositivo torna come nuovo); AVD intero, cartella e `.ini` accanto (Attenzione). Bloccata con `emulator.exe` o `qemu-system-*.exe` attivi.

**Immagini di sistema Android non usate.** Le immagini in `<SDK>\system-images\android-*\*\*` pesano 1-3 GB l'una. Un'immagine si offre solo se nessun AVD la usa: si leggono `image.sysdir.1` dai `config.ini` in `%ANDROID_AVD_HOME%`, `%ANDROID_USER_HOME%\avd` o `%USERPROFILE%\.android\avd`. Se un `config.ini` non si legge, non si offre nessuna immagine (fail closed). Bloccata con `emulator.exe` o `qemu-system-*.exe` attivi. Avviso: riscaricabile da Android Studio, SDK Manager.

## Parte D: Docker, Podman, WSL e macchine virtuali

Entrano per decisione dell'utente (decisione 6). Lo spazio liberato dentro un disco virtuale torna a Windows solo dopo la compattazione: ogni sezione finisce con la voce "Restituisci lo spazio a Windows", con il peso del file prima e dopo.

**Docker e Podman.** Rilevati da registro e `PATH` (`docker.exe`, `podman.exe`). Si lavora solo con la loro CLI, mai sui file del disco virtuale.
- Analisi con `docker system df -v`: immagini, container, volumi, cache di build, spazio recuperabile. *Scelta mia:* se Docker Desktop è chiuso, la sezione lo dice e offre "Avvia Docker"; non lo avvia da sola, ci mette decine di secondi.
- Voci: cache di build (`docker builder prune -a`, Consigliato); immagini senza tag (`docker image prune`, Consigliato); immagini non usate da nessun container (Facoltativo, si riscaricano con `docker pull`); container fermi, uno per uno (Attenzione: il loro filesystem si perde); volumi non collegati a container, uno per uno con nome e peso (Attenzione: spesso contengono database).
- Podman: stessi comandi (`podman system df`, `podman image prune`…).
- "Restituisci lo spazio a Windows": chiude Docker Desktop e WSL, compatta il disco di Docker Desktop. Il percorso del disco cambia tra le versioni di Docker Desktop: il builder lo verifica (regola 4).

**WSL.** Distro rilevate dal registro (`HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`: `DistributionName`, `BasePath` → `ext4.vhdx`), senza avviarle.
- Dentro ogni distro, il catalogo Linux dell'inventario: cache dei pacchetti (`/var/cache/apt/archives`, `dnf`, `pacman`, `zypp`), journal (`journalctl --vacuum-size`), `/tmp`, `/var/tmp`, e per ogni utente le cache della Parte B con i percorsi Linux (`~/.cache/pip`, `~/.npm/_cacache`, `~/.cargo/registry`, `~/.m2/repository`, `~/.gradle/caches`, `~/.cache/huggingface`…). Stesse regole: rilevamento dello strumento dentro la distro, livelli, 🔒.
- Progetti dentro le distro: stessa tabella della Parte A, cercati sotto `/home` e `/root`.
- *Scelta mia:* misura e cancellazione girano dentro la distro (`wsl -d <distro> -u root -- …`), perché camminare `\\wsl.localhost\` da Windows è lento. Le distro spente si avviano per l'analisi e si rispengono alla fine (`wsl --terminate`); quelle già accese restano accese.
- "Restituisci lo spazio a Windows" (Facoltativo, avviso: chiude tutte le distro e Docker Desktop): `wsl --shutdown`, poi `diskpart` `compact vdisk` sul `.vhdx`. Funziona anche su Windows Home, e PulisciPC è sempre amministratore.
- Distro intera (Attenzione, conferma scrivendo il nome): `wsl --unregister`.

**Macchine virtuali.** Rilevate da registro e `PATH`: VirtualBox (`VBoxManage.exe`), VMware Workstation e Player (`vmrun.exe`, inventario `%APPDATA%\VMware\inventory.vmls`), Hyper-V (solo Windows Pro, via WMI `root\virtualization\v2`), Vagrant.
- Ogni VM: nome, peso, ultimo avvio. Voci: compatta i dischi a VM spenta (`VBoxManage modifymedium --compact`, `vmware-vdiskmanager -k`, `Optimize-VHD`; Facoltativo); snapshot, uno per uno, eliminati con il programma della VM (`VBoxManage snapshot delete`, `vmrun deleteSnapshot`; Attenzione); VM intera con i suoi dischi (`VBoxManage unregistervm --delete`, `vmrun deleteVM`, `Remove-VM`; Attenzione).
- Dischi orfani: file `.vdi`, `.vmdk`, `.vhd`, `.vhdx`, `.qcow2` che nessuna VM registrata usa, trovati dalla MFT (`dc-disktree` tiene anche queste estensioni). Attenzione: possono essere l'unica copia. Esclusi i dischi di WSL e Docker (sezioni sopra) e i dischi montati.
- ISO dentro le cartelle delle VM: Facoltativo.

## Fase 3: processi in background e programmi all'avvio

Decisione dell'utente (2026-11-10): "va implementato in una Fase 3". Oggi la scheda "Servizi e RAM" gestisce solo i servizi (`src/services.rs`: `set_start` disattiva e ferma, `restore_all` ripristina). Mancano, e si aggiungono nella stessa scheda con gli stessi componenti:

- **Processi in esecuzione**: elenco con nome, RAM e CPU; "Termina" sul processo scelto. *Scelta mia:* i processi di sistema e quelli protetti (stessa idea di `PROTECTED` per i servizi) non si possono terminare; l'avviso dice che il lavoro non salvato si perde.
- **Programmi all'avvio**: chiavi `Run` e `RunOnce` (`HKCU`, `HKLM`, `WOW6432Node`), cartelle Esecuzione automatica, attività pianificate all'accesso. Disattiva e riattiva, senza cancellare: *scelta mia*, si usa lo stesso meccanismo di Gestione attività (`StartupApproved`), così Windows e PulisciPC mostrano lo stesso stato, con backup e "Ripristina tutto" come per i servizi.
- Fuori: la sospensione automatica dei programmi in background (come "Performance Optimizer" di CCleaner). È un prodotto a sé; se serve, documento suo.

## Esecuzione della pulizia: worker e resoconto

Decisione 7. Vale per Pulizia file e per Sviluppatori.

- **Worker.** Un gruppo di thread (*scelta mia:* uno per core logico, massimo 8) prende il lavoro da una coda comune. Le cartelle grandi si spezzano in sottoalberi, così un `node_modules` da 200.000 file non tiene fermo un solo worker: i file si cancellano in parallelo, le cartelle vuote dopo, dal basso verso l'alto. Un worker che finisce prende lavoro dalla coda degli altri.
- **Controlli prima, in un solo punto.** Le verifiche di sicurezza (`guard.rs`, percorsi dell'ultima analisi, processi attivi) si fanno prima di mettere una voce in coda; i worker cancellano soltanto.
- **Cosa resta in sequenza.** I comandi degli strumenti (`docker`, `wsl`, `VBoxManage`, `git worktree remove`) vanno uno alla volta per strumento. *Scelta mia:* con il Cestino i worker lavorano per voce di primo livello (una cartella intera = uno spostamento), perché il Cestino di Windows non guadagna dal parallelismo sui singoli file; il definitivo è il modo veloce, e la UI lo dice.
- **Avanzamento e annulla.** Barra con byte liberati e voci fatte sul totale; "Annulla" ferma i worker alla fine del file in corso e il resoconto dice cosa è stato fatto.
- **Resoconto finale.** Sempre, a fine pulizia: spazio liberato in totale e per categoria, numero di file e cartelle, durata, voci saltate con il motivo (file in uso, accesso negato, strumento aperto), errori. *Scelta mia:* si può salvare come file di testo; lo spazio nel Cestino è indicato a parte ("si libera svuotando il Cestino").

## Ordine e stime

1. Avvio sempre da amministratore (manifest, avviso sull'account): circa 30 minuti, subito, è indipendente.
2. Livelli, selezione per cartella e riconoscimento di cosa è in uso (`dc-model`, `dc-engine`, UI): circa 5 ore, prima delle Parti A e B.
   Worker e resoconto finale: circa 4 ore, insieme a questo punto.
3. Parte A: dopo la Fase 1, in un task proprio (tocca `dc-scanner`, `dc-git`, `dc-cleaner`, `dc-disktree`). Circa 3 ore per la tabella, più 2 ore per la ricerca dei marcatori dalla MFT.
4. Parte B: dentro la Fase 2, dopo il motore `pc-rules`. Circa 4 ore: la maggior parte è verificare i percorsi sulla documentazione (regola 4).
5. Parte C: con la Fase 3 (codice nativo in `cleaner.rs`). Circa 2 ore.
6. Processi e programmi all'avvio: con la Fase 3. Circa 4 ore.
7. Parte D: dopo la Fase 3, **nello stesso rilascio** (decisione dell'utente, 2026-11-10: "ora", non in un secondo rilascio). Docker e Podman circa 4 ore, WSL circa 6 ore, macchine virtuali circa 4 ore.

Totale: circa 38 ore e mezza.

## Verifica

- Parte A: un test per ogni riga della tabella, con un albero finto (marcatore + cartella → artefatto trovato con l'etichetta giusta); per i nomi † tre casi: repo con cartella ignorata (livello della sua classe), non ignorata (Attenzione, non preselezionata), senza git (Attenzione). Selezione: una sola cartella di un progetto finisce nel piano, le altre no. `guard.rs` rifiuta un nome non in lista.
- Parte B: ogni XML si carica dall'exe; con il profilo finto (regola 10) l'anteprima elenca i file attesi e **mai** quelli 🔒 (`token` di Hugging Face, `credentials.toml` di Cargo, `settings.json` di VS Code); le voci Attenzione (`User\History` di VS Code, toolchain predefinita di rustup) compaiono non selezionate; un Python usato da un venv finto va ad Attenzione con il nome del venv nell'avviso; con variabile d'ambiente impostata vince il suo percorso; con variabile non definita nessun percorso letterale `%…%`.
- Parte C: un AVD finto che usa un'immagine e un'altra immagine libera: si propone solo la seconda; un `config.ini` illeggibile: nessuna immagine proposta. L'AVD compare con le sue tre voci, nessuna preselezionata.
- Parte D: con CLI finte (come `FakeCli` di `dc-sessions`) per `docker`, `wsl`, `VBoxManage`, `vmrun`: l'analisi interpreta output registrati, la pulizia manda i comandi attesi e solo per le voci spuntate; il registro `Lxss` finto dà le distro e i loro `.vhdx`; lo script `diskpart` generato punta al file giusto. Nessun comando reale sul PC dell'utente.
- Worker: un albero finto con 10.000 file si cancella tutto con 1 e con 8 worker, stesso risultato; un file tenuto aperto finisce nel resoconto come saltato e non ferma gli altri; "Annulla" a metà lascia un resoconto con i conteggi giusti. Tempi misurati solo su alberi finti.
- Rilevamento: con registro e `PATH` finti (chiavi di prova sotto `HKCU\Software\PulisciPC-test`, `PATH` del processo puntato a una cartella temporanea) una categoria con le cartelle presenti compare anche senza voce di disinstallazione, una senza cartelle non compare; la ricerca dei progetti trova un `Cargo.toml` in un albero finto e ignora quello sotto `node_modules` e sotto `AppData`.

### Esito del piano 1 del catalogo (2026-11-12, commit `7643f2b`)

Verificato in una sessione cloud Linux: compilazione per `x86_64-pc-windows-gnu` (mingw) e test Windows eseguiti con wine. Nessuna prova sul PC dell'utente.

- `cargo check --workspace --exclude mukade --tests` (debug e `--release`, target Windows): nessun avviso.
- Test sotto wine (`cargo test --workspace --exclude mukade`): dc-app 39/39, dc-scanner 44/44, dc-model 11/11, dc-engine e dc-disktree verdi, dc-sessions 21/21, dc-mukade 22/22 (1 ignorato), dc-cleaner 24/27, pc-rules 195/200, pulisci-pc 230/235 (13 ignorati).
- Tutti i fallimenti dipendono da wine e c'erano già prima del piano (verificato sul commit `17e7ac8`, o su crate non toccati): junction e link simbolici che wine non segnala come tali, Restart Manager senza processi, `git` assente (dc-git 18 test), `curl` di sistema. **Vanno rilanciati su Windows vero**: `cargo test -j 1 --workspace --exclude mukade`.
- Il catalogo ha 95 cartelle distinte; un test guidato dalla tabella prova ogni coppia marcatore + cartella con il walker e con un indice finto, e il guard le accetta tutte.
- Exe release (target gnu, indicativo: la build MSVC dell'utente sarà diversa): 10,6 MB.
- Screenshot con wine + Xvfb su un albero finto (`--only-root`): livelli, conferma con "Ho letto gli avvisi", resoconto, annulla, Pulizia file invariata.

**Restano da fare su Windows** (non eseguibili nel cloud):
- tempo dell'analisi su tutti i dischi e picco RAM (`Get-Process PulisciPC`, limite 300 MB su C:);
- prova a mano della voce Sviluppatori (spuntare `dist`, Annulla a metà, Salva come testo, Cestino, avviso del Cestino stretto con un Cestino vero);
- i test che sotto wine si saltano: radice sostituita da una junction dopo la convalida, file bloccato con il processo in `locked_by`;
- Task 12 Step 5-6: rinomina in `.old` delle cartelle unificate, copia dei riferimenti di devclean in `docs/reference/` ed eliminazione solo dopo il sì dell'utente.

## Lasciato al builder

- Come rappresentare i tipi nuovi in `ArtifactKind`.
- Come riconoscere un daemon Gradle attivo.
- Il percorso del disco di Docker Desktop per versione, e come misurare dentro le distro WSL (`du` o altro).
- Le etichette esatte, purché in italiano corretto e senza nomi di prodotti terzi nella UI oltre a quello dello strumento.
- Le voci da togliere perché il percorso non è documentato (regola 4): si annotano qui sotto.

## Percorsi non verificati e tolti

(vuoto finché il builder non verifica)
