> Nota (2026-11-13): il prodotto ora si chiama **Xiox**. I nomi di questo documento (PulisciPC, `pulisci-pc`, `pc-rules`, i crate `dc-*`) sono quelli di allora; oggi sono `xiox-desktop` (binario `xiox-ui`), `xiox-rules` e `xiox-*` (`dc-app` -> `xiox-app`, ecc.). Il testo qui sotto non è stato aggiornato.

# Catalogo 1: amministratore, livelli, Progetti, worker e resoconto. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** PulisciPC parte sempre come amministratore, propone ogni cartella dei progetti con un livello (Consigliato, Facoltativo, Attenzione) e la sua casella, riconosce i circa 45 ecosistemi del catalogo cercandoli su tutti i dischi fissi dalla MFT, cancella con worker paralleli e chiude ogni pulizia con un resoconto.

**Architecture:** Si lavora sopra il codice della Fase 1 (`crates/dc-*`, `crates/dc-app`, `src/dev.rs`). Il modello (`dc-model`) riceve classi, livelli e selezione per cartella; `dc-scanner` riceve il catalogo come dati in `markers.rs`; `dc-engine` assegna livelli e preselezione; `dc-app` cerca su tutti i dischi e accetta piani per cartella; `dc-cleaner` cancella con un pool rayon e misura il Cestino; la UI resta quella di ccleaner64.

**Tech Stack:** Rust stabile, eframe/egui 0.33, rayon 1.12, trash 5.3, windows-sys 0.61, winreg 0.56, embed-resource 3 (nuovo, solo build).

**Spec:** `docs/superpowers/specs/2026-11-10-catalogo-sviluppo-design.md` (approvata dall'utente il 2026-11-10) sopra `docs/superpowers/specs/2026-11-10-dev-mode-design.md`. Leggere entrambe.

**Questo è il piano 1 di 4.** Gli altri, scritti a parte quando arriva il loro turno:
- Piano 2, con la Fase 2 della spec principale: motore `pc-rules`, Winapp2, CleanerML inglobati, Parte B (cache globali), regola 2 "in uso".
- Piano 3, con la Fase 3: copertura CCleaner, Parte C (emulatori e immagini Android), processi e programmi all'avvio.
- Piano 4: Parte D (Docker, Podman, WSL, macchine virtuali).

**Prerequisito:** la Fase 1 è chiusa da un'altra sessione (`docs/superpowers/plans/2026-11-10-fase1-sviluppatori.md`, Task 6 e 7). Non iniziare finché `git log` non mostra il commit `docs: verifica della Fase 1` e `git status --short` è vuoto. Questo piano modifica `src/dev.rs`, `src/main.rs` e `Cargo.toml`, che quella sessione sta scrivendo.

## Global Constraints

- Windows 11, testo della UI in italiano corretto (accenti, niente prefissi "AGG -").
- **UI di ccleaner64 di base** (decisione 8 della spec): barra laterale, `card()`, `primary()`, `secondary()`, colori di `src/theme.rs`. Nessuna schermata con uno stile proprio.
- **L'app propone, l'utente sceglie** (decisione 6): ogni cartella ha la sua casella; il livello decide solo preselezione e avviso. Unica esclusione: file di credenziali e configurazione (non toccati da questo piano).
- Livelli: **Consigliato** = classi C, B, L, preselezionato solo nei progetti inattivi; **Facoltativo** = D, mai preselezionato, avviso con il comando per riaverla; **Attenzione** = P, cartelle tracciate da git, identità non verificata, nomi † non ignorati da git: mai preselezionato, avviso da confermare.
- `node_modules`, `.venv`, `venv` sono Dependency, quindi Facoltativo: non preselezionati nemmeno nei progetti inattivi (approvato dall'utente il 2026-11-10).
- Esecuzione: subagent-driven (scelta dell'utente il 2026-11-10).
- Inattività: `DEFAULT_INACTIVE_DAYS` (14). Vale anche per i progetti senza sessioni (ultimo commit git o ultima modifica dei file), approvato dall'utente.
- Sempre amministratore (`requireAdministrator`), lettura MFT su tutti i dischi fissi NTFS; i dischi fissi non NTFS si camminano.
- Esclusi sempre dalla ricerca: cartelle artefatto e `NEVER_ENTER`, `Windows`, `Program Files*`, `ProgramData` alla radice, `AppData` ovunque, `$Recycle.Bin`, `System Volume Information`.
- Worker: `min(std::thread::available_parallelism(), 8)` thread. I controlli di sicurezza (`guard::validate`, allow-list di `checked_plan`) prima di mettere in coda; i worker cancellano soltanto. `git worktree remove` in sequenza.
- Test **mai sul PC dell'utente**: alberi finti in `tempfile::tempdir()`, git finto o repository creati dal test. Le verifiche visive usano `--only-root <cartella finta>`.
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Cartella `dist`/`build`/`out` con sorgenti** in un progetto senza git, o non ignorata da git: compare al livello Attenzione con "Potrebbe contenere sorgenti", mai preselezionata, nel piano solo se spuntata. Test in Task 3.
2. **Ricerca dalla radice del disco**: un `package.json` con `node_modules` sotto `AppData`, sotto `Program Files` o dentro un altro `node_modules` non diventa un progetto. Test in Task 6.
3. **Una cartella di nome comune** (`cache`, `videos`, `target`) fuori dal suo progetto: `.nx/cache` vale solo con `nx.json` due livelli sopra; un `cache` qualsiasi non viene proposto. Test in Task 5.
4. **File bloccato o "Annulla" a metà**: le altre voci proseguono; il resoconto conta file, byte e voci saltate giusti. Test in Task 8.
5. **Cestino troppo piccolo** per la pulizia scelta: la UI lo dice prima di partire e propone Definitiva, invece di lasciar decidere a Windows. Test in Task 9.

---

### Task 1: avvio sempre da amministratore e avviso sull'account

Stima: 45 min.

**Files:**
- Create: `build.rs`, `PulisciPC.rc`, `PulisciPC.manifest`
- Modify: `Cargo.toml` (`[build-dependencies] embed-resource = "3"`), `src/sys.rs` (togliere `relaunch_as_admin`, aggiungere `profile_mismatch`), `src/main.rs:385` (togliere il pulsante "Riavvia come amministratore" e la striscia non-admin)

**Interfaces:**
- Produces: `pub fn profile_mismatch() -> Option<(String, String)>` in `src/sys.rs` (utente del processo, utente della sessione interattiva); `fn names_differ(process: &str, session: &str) -> bool`.

- [ ] **Step 1: Test che falliscono** in `src/sys.rs` (`#[cfg(test)] mod tests`):
```rust
#[test] fn same_user_any_case() { assert!(!names_differ("Alfio", "alfio")); }
#[test] fn domain_prefix_ignored() { assert!(!names_differ(r"PC\alfio", "alfio")); }
#[test] fn other_account() { assert!(names_differ("Admin", "alfio")); }
```
- [ ] **Step 2:** `cargo test -p pulisci-pc names_differ` → FAIL (funzione assente).
- [ ] **Step 3:** `names_differ`: confronto senza maiuscole della parte dopo l'ultimo `\`. `profile_mismatch`: `GetUserNameW` per il processo, `WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSUserName)` per la sessione; `None` se una delle due fallisce o i nomi coincidono. Feature windows-sys `Win32_System_RemoteDesktop`.
- [ ] **Step 4:** Manifest con `<requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>` e `supportedOS` di Windows 10/11; `PulisciPC.rc` con `1 24 "PulisciPC.manifest"`; `build.rs` con `embed_resource::compile("PulisciPC.rc", embed_resource::NONE)`.
- [ ] **Step 5:** In `src/main.rs` togliere il ramo non-admin con "Riavvia come amministratore"; se `profile_mismatch()` è `Some((p, s))`, striscia `WARN_BG` come `admin_strip`: "Stai pulendo il profilo di «{p}», non quello di «{s}» che ha aperto la sessione."
- [ ] **Step 6:** `cargo test -p pulisci-pc` verde; `cargo build -p pulisci-pc --release`; `grep -c requireAdministrator target/release/PulisciPC.exe` → almeno 1.
- [ ] **Step 7:** Commit `feat(app): avvio sempre da amministratore, avviso se il profilo non è quello della sessione`.

### Task 2: `is_ignored` in `dc-git`

Stima: 30 min.

**Files:**
- Modify: `crates/dc-model/src/lib.rs` (trait `GitProbe`), `crates/dc-git/src/lib.rs`, ogni git finto dei test (`crates/dc-engine/src/tests.rs`, `crates/dc-app/src/tests.rs`)
- Test: `crates/dc-git/src/lib.rs` (`mod tests`, come `tracked_vs_ignored_target`)

**Interfaces:**
- Produces: in `GitProbe`, `fn is_ignored(&self, root: &Path, rel: &Path) -> Option<bool>;` (`Some(true)` ignorata, `Some(false)` non ignorata, `None` = non è un repository o git non risponde).

- [ ] **Step 1: Test che falliscono** con un repository creato dal test (`git init`, `.gitignore` con `dist/`, cartelle `dist` e `out` create):
  - `is_ignored(repo, "dist") == Some(true)`; `is_ignored(repo, "out") == Some(false)`;
  - cartella temporanea senza repository → `None`.
- [ ] **Step 2:** `cargo test -p dc-git` → FAIL.
- [ ] **Step 3:** `git check-ignore -q -- <rel con / e una / finale>` (la `/` finale fa valere anche i modelli di sola cartella come `dist/`): codice 0 → `Some(true)`, 1 → `Some(false)`, altro o nessuna risposta → `None`. Stesso `git()` e timeout di `is_tracked_checked`. I git finti dei test restituiscono `None` salvo diversa configurazione.
- [ ] **Step 4:** `cargo test --workspace --exclude mukade` verde.
- [ ] **Step 5:** Commit `feat(git): is_ignored con git check-ignore`.

### Task 3: classi, livelli e selezione per cartella (`dc-model`, `dc-engine`)

Stima: 3 ore.

**Files:**
- Modify: `crates/dc-model/src/lib.rs`, `crates/dc-engine/src/lib.rs`, `crates/dc-scanner/src/lib.rs` (costruzione di `Artifact` con i campi nuovi; valori dal catalogo arrivano nel Task 5, qui i 7 ecosistemi di oggi con le classi sotto), `crates/dc-engine/src/tests.rs`

**Interfaces:**
- Produces in `dc-model`:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class { Cache, Build, Dependency, Log, Personal }   // C B D L P della spec
impl Class { pub fn base_level(self) -> Level }              // C,B,L → Recommended; D → Optional; P → Caution

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level { Recommended, Optional, Caution }

pub struct Artifact {            // campi nuovi accanto a quelli di oggi
    pub id: String,              // path_key(&path)
    pub class: Class,
    pub level: Level,
    pub note: String,            // avviso in italiano; "" se nessuno
    pub ambiguous: bool,         // nome † della spec
    pub preselected: bool,
    /* path, kind, label, size, verified come oggi */
}
// ArtifactKind: nuova variante `Catalog` (tutte le voci nuove del Task 6).

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Selection { pub artifacts: Vec<String>, pub worktrees: Vec<String> }
```
- Produces in `dc-engine`: `pub fn build_plan(reports: &[ProjectReport], sel: &Selection, mode: RemoveMode) -> Result<CleanPlan>`; `fn assess(a: &mut Artifact, tracked: bool, ignored: Option<bool>, project_inactive: bool, project_blocked: bool)`.
- Classi dei 7 ecosistemi di oggi: `target` Rust e Maven = Build; `node_modules`, `.venv`, `venv` = Dependency; `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.gradle` = Cache; `build` Gradle e CMake, `bin`, `obj` = Build.

Regole di `assess` (in quest'ordine, la prima vince sul livello; le note si sommano a quella del catalogo):
1. `level = class.base_level()`.
2. `tracked` → Caution, nota "Contiene file tracciati da git".
3. `!verified` → Caution, nota "Cartella non riconosciuta con certezza".
4. `ambiguous` e `ignored != Some(true)` → Caution, nota "Potrebbe contenere sorgenti: git non la ignora" (`Some(false)`) o "Potrebbe contenere sorgenti: non è in un repository git" (`None`).
5. `preselected = project_inactive && !project_blocked && level == Recommended`.

`project_inactive` = `inactive_days >= cfg.inactive_days`, con o senza sessioni. `TrackedArtifact` e `UnverifiedArtifact` non bloccano più il progetto (non si emettono più). `LiveSession` blocca come oggi; sporco e commit non pushati bloccano solo i worktree degli agenti, come oggi. `ProjectReport.preselected = artifacts.iter().any(|a| a.preselected)`, salvo i worktree degli agenti, che tengono la regola di oggi.

`build_plan`: ogni id di `sel.artifacts` deve essere l'`id` di un artefatto di un report, altrimenti `Err("Cartella non trovata nell'analisi: {id}")`; report `InUse`, `Unreachable` o con `blocked_by` non vuoto → gli errori di oggi. `sel.worktrees` sono id di report di worktree: logica di oggi (rimozione intera o artefatti). Duplicati per `path_key` una volta sola.

- [ ] **Step 1: Test che falliscono** in `crates/dc-engine/src/tests.rs`, con scanner e git finti come i test di oggi:
  - `node_modules` di un progetto inattivo da 30 giorni → `level == Optional`, `!preselected`; `target` Rust dello stesso → `Recommended`, `preselected`;
  - progetto senza sessioni, ultimo commit 30 giorni fa → `target` preselezionato; 3 giorni fa → no;
  - artefatto tracciato da git → `Caution`, nota contiene "tracciati da git", il progetto **non** è `Blocked`;
  - artefatto `ambiguous` con `is_ignored == Some(false)` → `Caution`, "git non la ignora"; con `None` → `Caution`, "non è in un repository git"; con `Some(true)` → livello della sua classe;
  - progetto con `node_modules` e `target`: `build_plan` con `Selection { artifacts: vec![id_target], .. }` → un solo `PlanItem`, quello di `target`;
  - id inesistente → `Err` con "Cartella non trovata nell'analisi";
  - sessione `live` → `build_plan` su un suo artefatto → `Err` come oggi.
- [ ] **Step 2:** `cargo test -p dc-engine` → i nuovi FAIL.
- [ ] **Step 3:** Implementare tipi, `assess`, preselezione e `build_plan`. Aggiornare i test esistenti che si aspettavano `TrackedArtifact`/`UnverifiedArtifact` come blocchi: ora sono `Caution`.
- [ ] **Step 4:** `cargo test --workspace --exclude mukade` verde (compila anche `dc-app` e la UI: qui `dc_app::build_plan` tiene la firma di oggi e converte gli id di progetto in una `Selection` con tutti i loro artefatti; il cambio vero è nel Task 4, la UI per cartella nel Task 10).
- [ ] **Step 5:** Commit `feat(engine): classi, livelli e selezione per cartella`.

### Task 4: `dc-app` accetta piani per cartella

Stima: 1 ora.

**Files:**
- Modify: `crates/dc-app/src/lib.rs` (`build_plan`, `checked_plan` a riga 435), `crates/dc-app/src/tests.rs`

**Interfaces:**
- Consumes: `dc_engine::build_plan(&[ProjectReport], &Selection, RemoveMode)` (Task 3).
- Produces: `pub fn build_plan(scan: &DevScan, sel: &Selection, mode: RemoveMode) -> Result<CleanPlan, String>`; `checked_plan` ricostruisce la `Selection` dal piano ricevuto (`RemoveDir` → `path_key` dell'elemento in `artifacts`; `GitWorktreeRemove` → `project_id` in `worktrees`), rifà il piano canonico e confronta percorso, tipo e azione come oggi.

- [ ] **Step 1: Test che falliscono** in `crates/dc-app/src/tests.rs`:
  - piano con un solo artefatto di un progetto che ne ha due → accettato, eseguito solo quello (l'altro resta su disco);
  - piano falsificato con un percorso figlio dell'artefatto scelto → `Err` "Percorso non presente nell'ultima analisi, rifiutato";
  - piano con `kind` cambiato su un percorso valido → `Err` "Piano rifiutato: l'elemento non corrisponde all'ultima analisi".
- [ ] **Step 2:** `cargo test -p dc-app` → FAIL.
- [ ] **Step 3:** Implementare; aggiornare i test di oggi che passano id di progetto.
- [ ] **Step 4:** `cargo test -p dc-app` verde.
- [ ] **Step 5:** Commit `feat(app): piani per cartella, controllati contro l'ultima analisi`.

### Task 5: catalogo come dati: marcatori, sottopercorsi, prefissi, Meson, nomi †

Stima: 2 ore 30 min.

**Files:**
- Modify: `crates/dc-scanner/src/markers.rs`, `crates/dc-scanner/src/lib.rs` (`walk`, `discover_indexed`, `Found`), `crates/dc-cleaner/src/guard.rs` (`name_plausible` → arm `Catalog`), `crates/dc-cleaner/Cargo.toml` (dipendenza `dc-scanner`; prima `cargo tree -p dc-scanner | grep dc-cleaner` deve essere vuoto)
- Test: `crates/dc-scanner/src/tests.rs`, `crates/dc-cleaner/src/tests.rs`

**Interfaces:**
- Produces in `markers.rs`:
```rust
pub enum Pattern {
    Exact(&'static str), Suffix(&'static str),
    Dir(&'static str),                 // cartella marcatore, es. ".storybook"
    Sub(&'static str),                 // file in sottocartella, es. "ProjectSettings/ProjectVersion.txt"
    All(&'static [Pattern]),           // tutti presenti, es. conf.py + index.rst
}
pub struct ArtifactSpec {
    pub dir: &'static str,             // "target", ".nx/cache" (max 3 segmenti), "checkpoint-*" (prefisso), "*" (solo con Identity::MesonBuild)
    pub kind: ArtifactKind, pub label: &'static str, pub identity: Identity,
    pub class: Class, pub ambiguous: bool,
    pub hint: &'static str,            // per Dependency: come riaverla; "" altrimenti
}
// Identity: nuova variante MesonBuild (la cartella contiene `meson-private/` e `build.ninja`).
pub fn detect(dir: &Path, files: &[&str], dirs: &[&str]) -> Option<Detected>;
impl Detected { pub fn resolve(&self, rel: &str, identity_ok: impl Fn(Identity) -> bool) -> Option<(&'static ArtifactSpec, bool /*verified*/, bool /*ambiguous*/)> }
pub static COMMON: &[ArtifactSpec];   // aggiunte a ogni progetto riconosciuto (Task 6)
```
- Produces in `dc-scanner`: `pub fn catalog_dir_plausible(path: &Path) -> bool`: vero se gli ultimi segmenti di `path` corrispondono al `dir` di una voce del catalogo (prefisso per `*-*`; per `*` serve `path/meson-private` esistente).

Regole:
- `resolve`: `ambiguous = true` se **una qualsiasi** voce del progetto che corrisponde a quel nome è †, anche se un'altra lo accetta senza git (spec, Parte A). Poi la scelta di oggi (identità presente, poi marcatore nel genitore).
- Walker: per un progetto, oltre ai figli diretti, si provano le voci a più segmenti (`root.join(dir).is_dir()`), le voci a prefisso sui figli diretti, e `MesonBuild` sui figli diretti.
- Indice: si cercano con `find_dirs_named` gli **ultimi segmenti** dei `dir` (prefissi e `*` esclusi); per una voce a k segmenti il progetto è l'antenato k livelli sopra e i segmenti intermedi devono coincidere; si cerca anche `meson-private` (progetto = nonno). Poi, per ogni progetto trovato, si provano prefissi e `MesonBuild` sui figli diretti già elencati da `list_dir`.
- `Found` porta `class`, `ambiguous`, `hint`; lo scanner li copia in `Artifact` (`note = hint`).

- [ ] **Step 1: Test che falliscono** in `crates/dc-scanner/src/tests.rs`, con voci di prova aggiunte da una tabella di test o con le voci reali che il Task 6 metterà (qui basta una tabella minima dentro i test):
  - `nx.json` + `.nx/cache` → artefatto `.nx/cache`; un `cache` qualsiasi sotto `src/` dello stesso progetto → nessun artefatto (Review Focus 3);
  - `ProjectSettings/ProjectVersion.txt` + `Library` → artefatto `Library`;
  - `.storybook/` + `storybook-static` → artefatto;
  - `meson.build` + `builddir/meson-private/` + `builddir/build.ninja` → artefatto `builddir`; `other/` senza `meson-private` → niente;
  - `checkpoint-500/` sotto un progetto Python → artefatto con prefisso;
  - `package.json` + `CMakeLists.txt` + `build/` → `ambiguous == true`; solo `CMakeLists.txt` + `build/` → `ambiguous == false`;
  - stessi casi con un indice finto (`DirIndex` di test) oltre che con il walker.
- [ ] **Step 2:** In `crates/dc-cleaner/src/tests.rs`: `PlanItem` con `kind: Catalog` su `…\.nx\cache` → passa `guard::validate`; su `…\src` → rifiutato.
- [ ] **Step 3:** `cargo test -p dc-scanner -p dc-cleaner` → FAIL.
- [ ] **Step 4:** Implementare come sopra; `NEVER_ENTER` aggiunge `.next`, `.nuxt`, `.svelte-kit`, `.angular`, `.turbo`, `.dart_tool`, `.terraform`, `.terragrunt-cache`, `.stack-work`, `dist-newstyle`, `.zig-cache`, `.pio`.
- [ ] **Step 5:** `cargo test --workspace --exclude mukade` verde.
- [ ] **Step 6:** Commit `feat(scanner): catalogo come dati con sottopercorsi, prefissi, Meson e nomi ambigui`.

### Task 6: le voci del catalogo e la ricerca su tutti i dischi

Stima: 3 ore.

**Files:**
- Modify: `crates/dc-scanner/src/markers.rs` (tabella), `crates/dc-scanner/src/fsutil.rs` (`is_system_dir`: `appdata` a qualunque livello), `crates/dc-disktree/src/lib.rs` (dischi fissi), `crates/dc-model/src/lib.rs` (`Settings.all_drives: bool`, `#[serde(default = "yes")]`, default `true`), `crates/dc-app/src/lib.rs` (`plan_roots`, `run_scan`)
- Test: `crates/dc-scanner/src/tests.rs`, `crates/dc-disktree/src/lib.rs`, `crates/dc-app/src/tests.rs`

**Interfaces:**
- Consumes: `Pattern`, `ArtifactSpec`, `COMMON` (Task 5).
- Produces: `pub struct FixedDrive { pub letter: char, pub ntfs: bool }`, `pub fn fixed_drives() -> Vec<FixedDrive>` e `fn classify(letter: char, drive_type: u32, fs_name: &str) -> Option<FixedDrive>` in `dc-disktree`; `pub fn plan_roots(settings: &Settings, sessions: &[Preloaded], home: Option<&Path>, drives: &[PathBuf]) -> ScanRoots` in `dc-app`.

**Tabella.** Una voce per ogni riga della tabella "Parte A" della spec, con queste regole di lettura:
- nella colonna Cartelle i gruppi separati da `;` prendono le classi della colonna Classe nello stesso ordine (`.parcel-cache`, `.turbo` = C; `coverage`, `.nyc_output` = L; `dist`†, `build`†, `out`† = B); una sola classe vale per tutte; "C / D" sono due opzioni distinte;
- `{a,b}` nei marcatori sono più `Exact`; "`conf.py` con `index.rst`" e "`_config.yml` con `Gemfile`" sono `All`; `*.tf`, `*.uproject`, `*.cabal`, `*.fsproj`, `*.vbproj` sono `Suffix`;
- etichette in italiano: "Cache Next.js (.next)", "Build Next.js (out)", "Dipendenze Elixir (deps)"…;
- `hint` delle Dependency: `node_modules` "Si riscarica con npm install (o pnpm, yarn, bun install)"; `.venv`/`venv` "Si ricrea con python -m venv e l'installazione dei requisiti"; `.tox`/`.nox` "Si ricrea al prossimo tox/nox"; `__pypackages__` "Si riscarica con pdm install"; `.terraform` "Si riscarica con terraform init"; `vendor` Go "Si ricrea con go mod vendor"; `vendor` PHP "Si riscarica con composer install"; `vendor/bundle` "Si riscarica con bundle install"; `deps` Elixir "Si riscarica con mix deps.get"; `_opam` "Si ricrea con opam install . --deps-only"; `vcpkg_installed` "Si ricrea con vcpkg install"; `.pio` "Si riscarica alla prossima build PlatformIO"; `.build` Swift "Si ricrea con swift build"; `dbt_packages` "Si riscarica con dbt deps".

**Voci Attenzione (Personal)** dalla riga "Livello Attenzione dentro i progetti" della spec, con questi marcatori: `COMMON` = `.idea`, `.vs`, `.ipynb_checkpoints` (ogni progetto riconosciuto); Python: `.hypothesis`, `wandb`, `mlruns`, `runs`, `checkpoint-*`; `*.Rproj`: `.Rproj.user`; `.yarnrc.yml`: `.yarn/cache`; `*.uproject`: `Saved`, `Binaries`; Unity: `UserSettings`; `dvc.yaml`: `.dvc/cache`; `artisan` (Laravel): `bootstrap/cache`, `storage/framework/cache`, `storage/framework/views`; `symfony.lock`: `var/cache`.

**Ricerca su tutti i dischi.** `run_scan`, con `settings.all_drives`: `fixed_drives()`; indice MFT per i dischi NTFS (`build_for_roots` sulle radici `X:\`); radici = dischi NTFS indicizzati + dischi non NTFS + le radici di oggi (sessioni, worktree, cartelle aggiuntive, e come ripiego `HOME_PROJECT_DIRS` esistenti). Le radici dentro un'altra si scartano (regola di oggi): quindi le radici di oggi restano solo su un disco NTFS la cui lettura MFT è fallita. Le radici dei dischi vanno anche in `container_roots`. `--only-root` (Task 10) mette `all_drives = false`.

- [ ] **Step 1: Test che falliscono:**
  - in `dc-scanner`, un test guidato da tabella: per ogni voce di `MARKERS` crea marcatore e cartella in una `tempdir` e verifica che venga trovato un artefatto con quella `label`, `class` e `ambiguous`;
  - `C:\…\AppData\Roaming\x\package.json` + `node_modules` sotto una radice finta → nessun progetto; `node_modules\pkg\package.json` → nessun progetto annidato (Review Focus 2);
  - `classify('C', DRIVE_FIXED, "NTFS") == Some(FixedDrive { letter: 'C', ntfs: true })`; `classify('E', DRIVE_REMOVABLE, "FAT32") == None`; `classify('D', DRIVE_FIXED, "ReFS") == Some(.. ntfs: false)`;
  - `plan_roots` con `drives = [X:\]` e una sessione in `X:\p` → radici `[X:\]`; con `drives = []` → `[X:\p]` come oggi.
- [ ] **Step 2:** `cargo test --workspace --exclude mukade` → i nuovi FAIL.
- [ ] **Step 3:** Implementare tabella, `appdata` in `is_system_dir`, `fixed_drives` (`GetLogicalDrives`, `GetDriveTypeW`, `GetVolumeInformationW`), `plan_roots`, `run_scan`.
- [ ] **Step 4:** `cargo test --workspace --exclude mukade` verde.
- [ ] **Step 5:** Commit `feat(catalogo): voci dei progetti della spec e ricerca su tutti i dischi fissi`.

### Task 7: resoconto finale nel modello

Stima: 45 min.

**Files:**
- Modify: `crates/dc-model/src/lib.rs`
- Test: `crates/dc-model/src/lib.rs` (`mod tests`)

**Interfaces:**
- Produces:
```rust
pub struct CleanReport {
    pub freed_bytes: u64,          // liberati davvero (definitivo)
    pub recycled_bytes: u64,       // spostati nel Cestino: si liberano svuotandolo
    pub files: u64, pub dirs: u64, pub elapsed_ms: u64, pub cancelled: bool,
    pub by_category: Vec<(String, u64)>,           // etichetta → byte, ordine decrescente
    pub skipped: Vec<(PathBuf, String)>, pub failed: Vec<(PathBuf, String)>,
}
impl CleanReport { pub fn to_text(&self) -> String }
```

- [ ] **Step 1: Test che fallisce:** un `CleanReport` con 2 categorie, 1 saltata, 1 fallita, `recycled_bytes > 0`: `to_text()` contiene "Spazio liberato", le due etichette con le dimensioni, "Saltati (1)", "Non riusciti (1)" e "si libera svuotando il Cestino".
- [ ] **Step 2:** `cargo test -p dc-model` → FAIL.
- [ ] **Step 3:** Implementare (dimensioni con la stessa formattazione di `sys::fmt_size`: spostare la funzione pura in `dc-model` come `fmt_size` e farla usare a `src/sys.rs`).
- [ ] **Step 4:** `cargo test -p dc-model -p pulisci-pc` verde.
- [ ] **Step 5:** Commit `feat(model): resoconto finale della pulizia`.

### Task 8: cancellazione con worker paralleli

Stima: 3 ore.

**Files:**
- Create: `crates/dc-cleaner/src/parallel.rs`
- Modify: `crates/dc-cleaner/src/lib.rs`, `crates/dc-cleaner/Cargo.toml` (`rayon = "1.12.0"`), `crates/dc-model/src/lib.rs` (`CleanResult`: `files_removed`, `dirs_removed`, `elapsed_ms`, `cancelled`, `recycled_bytes`), `crates/dc-app/src/lib.rs` (`execute_plan` con avanzamento e annulla)
- Test: `crates/dc-cleaner/src/tests.rs`, `crates/dc-app/src/tests.rs`

**Interfaces:**
- Produces in `dc-cleaner`:
```rust
pub fn worker_count() -> usize;                                     // min(available_parallelism, 8)
#[derive(Clone, Default)] pub struct Cancel(Arc<AtomicBool>);       // cancel(), is_cancelled()
pub struct Tick { pub done_bytes: u64, pub total_bytes: u64, pub done_items: usize, pub total_items: usize }
pub struct ParallelRemover { pub workers: usize }
impl ParallelRemover {
    pub fn execute_with(&self, plan: &CleanPlan, dry_run: bool, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> CleanResult;
}
impl Remover for ParallelRemover { /* execute = execute_with senza tick e senza annulla */ }
pub fn remove_files_parallel(files: &[(PathBuf, u64)], workers: usize, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> (u64 /*byte*/, u64 /*file*/, Vec<(PathBuf, String)> /*falliti*/);  // per Pulizia file (Task 11)
```
- Produces in `dc-app`: `pub fn execute_plan(scan: &DevScan, plan: &CleanPlan, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> Result<CleanResult, String>` e `pub fn report(scan: &DevScan, plan: &CleanPlan, res: &CleanResult) -> CleanReport` (categoria = `label` dell'artefatto; i worktree = "Worktree degli agenti").

Algoritmo:
1. In sequenza: `guard::validate` e misura (`fsops::measure`) di ogni elemento; i rifiutati diventano `Skipped` come oggi.
2. Un pool `rayon::ThreadPoolBuilder::new().num_threads(workers)`.
3. Definitivo, `RemoveDir`: dentro `pool.scope`, ogni cartella è un task che cancella i propri file (stessi tentativi e sola lettura di `fsops::remove_dir_rec`) e lancia un task per ogni sottocartella; i link si tolgono senza seguirli. Finiti i task, le cartelle si tolgono dalla più profonda. L'esito di un elemento è `Removed` solo se il percorso non esiste più (`fsops::is_gone`), come oggi.
4. Cestino: un task per elemento (`trash::delete`), stesso pool.
5. `GitWorktreeRemove`: in sequenza, dopo il resto.
6. Annulla: ogni task controlla `cancel` prima di ogni file; un elemento interrotto è `Skipped { reason: "Annullato: liberati {n} byte" }`; gli elementi non iniziati `Skipped { reason: "Annullato" }`; `cancelled = true`.
7. `tick` al massimo ogni 100 ms (contatori atomici, un solo thread chiama `tick`).

- [ ] **Step 1: Test che falliscono** in `crates/dc-cleaner/src/tests.rs`:
  - albero finto di 10.000 file in 200 cartelle annidate: con `workers = 1` e `workers = 8` tutto rimosso, `files_removed == 10_000`, stessi `bytes_freed`;
  - un file tenuto aperto senza condivisione in un elemento (`OpenOptions` + `share_mode(0)` di `std::os::windows::fs::OpenOptionsExt`): quell'elemento `Failed` con il processo in `locked_by`, gli altri `Removed` (Review Focus 4);
  - `Cancel` attivato dopo il primo `tick`: `cancelled == true`, la somma dei byte nei motivi degli `Skipped` più `bytes_freed` è ≤ al totale, nessun panico;
  - `remove_files_parallel` su 1.000 file, 3 dei quali bloccati → 997 rimossi, 3 nei falliti.
- [ ] **Step 2:** `cargo test -p dc-cleaner` → FAIL.
- [ ] **Step 3:** Implementare; `DefaultRemover` resta per compatibilità e delega a `ParallelRemover { workers: worker_count() }`.
- [ ] **Step 4:** In `dc-app`, test: `report` su un `CleanResult` costruito a mano → `by_category` somma per `label`, ordine decrescente.
- [ ] **Step 5:** `src/dev.rs` chiama il nuovo `execute_plan` con un `tick` che non fa nulla e un `Cancel::default()` (la UI vera nel Task 10). `cargo test --workspace --exclude mukade` verde.
- [ ] **Step 6:** Commit `feat(cleaner): cancellazione con worker paralleli, annulla e conteggi`.

### Task 9: il Cestino basta?

Stima: 1 ora.

**Files:**
- Create: `crates/dc-cleaner/src/recycle.rs`
- Modify: `crates/dc-cleaner/Cargo.toml` (`winreg = "0.56"`, windows-sys `Win32_UI_Shell`), `crates/dc-app/src/lib.rs`

**Interfaces:**
- Produces in `dc-cleaner`: `pub fn recycle_room(volume: char) -> Option<u64>` (spazio libero nel Cestino di quel volume: quota meno occupato; `None` se non si sa) e `pub fn shortfall(needed: &[(char, u64)], room: impl Fn(char) -> Option<u64>) -> Vec<(char, u64 /*serve*/, u64 /*c'è*/)>`.
- Produces in `dc-app`: `pub fn recycle_shortfall(plan: &CleanPlan) -> Vec<(char, u64, u64)>` (vuoto se `plan.mode == Permanent`).

`recycle_room`: occupato da `SHQueryRecycleBinW("X:\\")`; quota da `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{GUID}\MaxCapacity` (MB), con il GUID da `GetVolumeNameForVolumeMountPointW`; `NukeOnDelete = 1` → `Some(0)`.

- [ ] **Step 1: Test che falliscono** (puri, nessun Cestino vero): `shortfall(&[('C', 10 GB)], |_| Some(2 GB))` → `[('C', 10 GB, 2 GB)]`; con `Some(20 GB)` → vuoto; con `None` → vuoto (non si sa: nessun avviso); due volumi, uno solo stretto → un elemento (Review Focus 5).
- [ ] **Step 2:** `cargo test -p dc-cleaner recycle` → FAIL.
- [ ] **Step 3:** Implementare; `recycle_shortfall` somma `expected_bytes` per lettera di unità.
- [ ] **Step 4:** `cargo test --workspace --exclude mukade` verde.
- [ ] **Step 5:** Commit `feat(cleaner): controllo dello spazio nel Cestino prima di pulire`.

### Task 10: UI Sviluppatori: livelli, caselle per cartella, avanzamento, resoconto

Stima: 3 ore.

**Files:**
- Modify: `src/dev.rs`, `src/main.rs` (flag `--only-root <dir>`), `src/theme.rs` (solo se serve: `pub const RED: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28)`)

**Interfaces:**
- Consumes: `Selection`, `Level`, `Artifact.{id, level, note, preselected}` (Task 3), `dc_app::{build_plan, execute_plan, report, recycle_shortfall}` (Task 4, 8, 9), `Cancel`, `Tick` (Task 8), `CleanReport::to_text` (Task 7).

Layout (componenti di oggi, nessuno stile nuovo):
- **Progetti**: la riga del progetto ha una casella a tre stati (tutte, nessuna, alcune) e si apre (`CollapsingHeader`) sulle sue cartelle. Riga cartella: casella, etichetta, livello ("Consigliato" in `GREEN`, "Facoltativo" in `ACCENT`, "Attenzione" in `RED`), dimensione, `note` in `MUTED`. Preselezione = `Artifact.preselected`. Il totale selezionato si aggiorna subito.
- **Conferma**: il modale di oggi; se la selezione contiene voci Attenzione, le elenca con la nota e "Pulisci" resta disattivato finché non si spunta "Ho letto gli avvisi". Se `recycle_shortfall` non è vuoto: "Il Cestino di {X}: ha {c'è} liberi, la pulizia ne sposta {serve}. Eliminare definitivamente questa volta?" con "Definitiva" e "Annulla".
- **Durante la pulizia**: `ProgressBar` `ACCENT` con "{done} di {total} · {voci fatte} di {voci}" e `secondary("Annulla")` che chiama `Cancel::cancel`.
- **Resoconto**: al posto dell'esito di oggi: "Spazio liberato" in `GREEN`, file, cartelle, durata; tabella per categoria; "Saltati (n)" e "Non riusciti (n)" con il motivo; se `recycled_bytes > 0`: "{n} nel Cestino: si liberano svuotandolo"; `secondary("Salva come testo…")` → `rfd::FileDialog::save_file`, scrive `to_text()`.
- `--only-root <dir>`: impostazioni in memoria con `session_projects = false`, `roots = [dir]`, `scan_roots = true`, `all_drives = false`; nulla salvato su disco.

- [ ] **Step 1:** Implementare.
- [ ] **Step 2:** Albero finto (script in scratchpad, mai nei progetti dell'utente): un progetto Rust inattivo (`Cargo.toml`, `target/` con `CACHEDIR.TAG`, mtime 30 giorni fa), un progetto Node con `node_modules` e `dist/` non ignorata da git, un progetto Next.js con `.next/`.
- [ ] **Step 3:** Verifica visiva: `cargo run -p pulisci-pc --release -- --tab dev --only-root <albero> --analyze --screenshot shots/dev-levels.png`. Expected: `target` spuntato e "Consigliato"; `node_modules` non spuntato e "Facoltativo" con "Si riscarica con npm install…"; `dist` non spuntato, "Attenzione", "Potrebbe contenere sorgenti…"; layout di ccleaner64.
- [ ] **Step 4:** Prova a mano sull'albero finto: spuntare `dist` → la conferma chiede "Ho letto gli avvisi"; pulizia Definitiva → resoconto con le categorie giuste; "Salva come testo…" scrive il file; una seconda pulizia con "Annulla" subito → resoconto con "Annullato".
- [ ] **Step 5:** Commit `feat(ui): livelli, caselle per cartella, avanzamento e resoconto nella voce Sviluppatori`.

### Task 11: Pulizia file con i worker e lo stesso resoconto

Stima: 1 ora 30 min.

**Files:**
- Modify: `src/cleaner.rs` (`clean`), `src/main.rs` (esito di Pulizia file), `Cargo.toml` (`dc-cleaner`)

**Interfaces:**
- Consumes: `dc_cleaner::{remove_files_parallel, worker_count, Cancel, Tick}` (Task 8), `CleanReport` (Task 7).
- Produces: `pub fn clean(index: usize, found: &Found, tick: &(dyn Fn(Tick) + Sync), cancel: &Cancel) -> Cleaned`; dopo tutte le categorie la UI costruisce un `CleanReport` (categoria = nome della categoria di `CATEGORIES`) e mostra lo stesso resoconto del Task 10. Il Cestino resta `sys::empty_recycle_bin`; `remove_empty_dirs` resta dopo i file.

- [ ] **Step 1:** Test in `src/cleaner.rs`: `Found` su una `tempdir` con 500 file, uno bloccato → `Cleaned.removed == 499`, `failed == 1`.
- [ ] **Step 2:** `cargo test -p pulisci-pc` → FAIL.
- [ ] **Step 3:** Implementare; i file in sola lettura come oggi (lo fa già `remove_files_parallel` nel Task 8).
- [ ] **Step 4:** `cargo test -p pulisci-pc` verde; screenshot `--tab cleaner --screenshot shots/check-cleaner.png` uguale a prima a occhio.
- [ ] **Step 5:** Commit `feat(cleaner): Pulizia file con worker paralleli e resoconto finale`.

### Task 12: verifica finale ed eliminazione delle cartelle unificate

Stima: 1 ora, più l'attesa della conferma dell'utente.

- [ ] **Step 1:** `cargo test --workspace --exclude mukade` verde; annotare i numeri.
- [ ] **Step 2:** Tempo dell'analisi su tutti i dischi (sola lettura, nessuna pulizia) e picco RAM (`Get-Process PulisciPC`): annotare; picco sotto 300 MB su C:, come la spec principale.
- [ ] **Step 3:** `cargo build -p pulisci-pc --release`; dimensione dell'exe annotata.
- [ ] **Step 4:** Sezione "Verifica" di `docs/superpowers/specs/2026-11-10-catalogo-sviluppo-design.md` con i numeri; commit `docs: verifica del piano 1 del catalogo`.
- [ ] **Step 5: Controlli prima di eliminare** `D:\Projects\KKK\devclean`, `D:\Projects\KKK\wiztree64`, `D:\Projects\KKK\WizTree`, `D:\Projects\KKK\_refs`:
  1. rinominarle aggiungendo `.old`; `cargo build --workspace` e `cargo test --workspace --exclude mukade` verdi;
  2. copiare in `docs/reference/` `devclean/docs/DESIGN.md` e `devclean/vendor/mukade/VENDORED.md`, e correggere i rimandi delle spec a quei percorsi; commit `docs: riferimenti di devclean dentro il repository`;
  3. chiedere all'utente se `ccleaner64/app` ha una copia fuori da questo PC (oggi il git è solo locale).
- [ ] **Step 6:** Solo dopo il sì esplicito dell'utente, eliminare le quattro cartelle `.old`. **Non** toccare `D:\Projects\KKK\ccleaner64` fuori da `app` (serve alla Fase 3), `D:\Projects\mukade` e `D:\Software\BleachBit`.
