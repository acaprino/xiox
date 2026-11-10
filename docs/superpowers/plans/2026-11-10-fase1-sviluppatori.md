# Fase 1, modalità Sviluppatori: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** PulisciPC (`D:\Projects\KKK\ccleaner64\app`) ottiene una quarta voce "Sviluppatori" con tre sezioni (Progetti, Worktree, Sessioni archiviate), costruita sui crate di devclean, su mukade `dfde57e` e su una scansione MFT nostra al posto di WizTree.

**Architecture:** `ccleaner64/app` diventa un workspace Cargo: il pacchetto `pulisci-pc` (UI egui) più `crates/dc-*` copiati da devclean, due crate nuovi (`dc-disktree` per la MFT, `dc-sessions` per cancellare le sessioni archiviate) e `dc-app`, la colla senza UI che prima stava in `devclean/src-tauri/src/lib.rs`. La UI chiama `dc-app` da un thread con `mpsc`, come fanno già Pulizia file e Registro.

**Tech Stack:** Rust stabile (edition 2021 per `pulisci-pc`, 2024 per i crate dc-*), eframe/egui 0.33, rusqlite `=0.40.2` bundled, rayon, trash 5.3, windows-sys, rfd 0.15.

**Spec:** `D:\Projects\KKK\ccleaner64\app\docs\superpowers\specs\2026-11-10-dev-mode-design.md` (approvato 2026-11-10). Le Fasi 2 e 3 della spec hanno piani propri.

## Global Constraints

- Windows 11. Testo della UI in italiano.
- **Layout di ccleaner64 intatto:** barra laterale 212 px colore `SIDEBAR`, voce selezionata `SIDEBAR_SEL` con barretta `ACCENT` 4 px, card bianche (`card()`), pulsanti `primary()`/`secondary()`, finestra 1100×700 (min 900×560), stesso `style()`. Nessun layout di devclean o wiztree64.
- mukade si prende **solo** dal commit `dfde57e` di `D:\Projects\mukade` (`git archive`, mai il working tree, che ha modifiche di un'altra sessione). Solo libreria, regole di `devclean/vendor/mukade/VENDORED.md`.
- `rusqlite = { version = "=0.40.2", features = ["bundled"] }` uguale in `crates/dc-mukade` e `vendor/mukade`.
- Nessun eseguibile esterno per la scansione disco. Alla fine `grep -ri wiztree` fuori da `docs/` e da `VENDORED.md` è vuoto.
- PulisciPC non scrive mai SQL nei database degli harness. Sessioni cancellate solo con `codex delete --force <id>`, `opencode --standalone session export|delete <id>`, o rimuovendo il transcript proprio della sessione (Claude Code, Pi).
- Mai cancellare: una sessione non archiviata, una `live`, `history.jsonl`, `sessions/<pid>.json`, un file `.db`, file di credenziali.
- Rimozione di un worktree sempre definitiva: `git worktree remove` (mai `--force`) + `prune`, come `dc-cleaner` già fa.
- `RemoveMode` default `Recycle`; giorni di inattività default `DEFAULT_INACTIVE_DAYS` (14).
- Impostazioni in `%APPDATA%\PulisciPC\settings.json`; cache di mukade in `%LOCALAPPDATA%\PulisciPC\mukade`.
- L'analisi gira in un thread: la UI non si blocca mai. I dati dell'analisi si liberano quando si cambia voce nella barra laterale (se nessun job è in corso) o si rianalizza.
- Picco RAM della lettura MFT sotto 300 MB su C:.

## Review Focus

1. **CLI dell'harness assente** (Codex o OpenCode non installati o fuori dal PATH): quelle sessioni finiscono `Failed` con "CLI di Codex non trovata" / "CLI di OpenCode non trovata", le altre procedono. Test in Task 4.
2. **Harness aperto durante la cancellazione** (database bloccato, la CLI esce con errore): la sessione resta com'era, l'errore della CLI si vede nel risultato, il file del transcript non viene toccato. Test in Task 4.
3. **Store assente** (nessuna `~/.pi`, nessun OpenCode): l'elenco delle archiviate di quell'harness è vuoto, senza avvisi. Test in Task 3.
4. **Unità non disponibile o non NTFS, PulisciPC non admin**: niente indice MFT per quelle radici, avviso in italiano, l'analisi prosegue con il walker. Test in Task 2.
5. **`cwd` con prefisso `\\?\`** (Codex li scrive così: `\\?\C:\Users\alfio\.codex\worktrees\testing-intake\daodan`): normalizzati come gli altri percorsi, quindi abbinati al progetto e al worktree giusto. Test in Task 3.

---

### Task 0: Workspace unito, git, re-vendor di mukade

Stima: 30-40 min.

**Files:**
- Modify: `Cargo.toml` (aggiunge `[workspace]`)
- Create: `.gitignore`, `crates/dc-{model,scanner,git,cleaner,engine,mukade}/` (copie da `D:\Projects\KKK\devclean\crates`), `vendor/mukade/` (da mukade `dfde57e`), `THIRD_PARTY_NOTICES.md`
- Non si copia: `devclean/crates/dc-wiztree`, `devclean/src-tauri`, `devclean/src`, `devclean/scripts`.

**Interfaces:**
- Produces: workspace con membri `.`, `crates/*`; `vendor/mukade` entra come path dependency di `dc-mukade` (stesso schema di devclean).

- [ ] **Step 1:** `git init` in `D:\Projects\KKK\ccleaner64\app`; `.gitignore` con `target/`, `shots/*.png` esclusi solo se generati dai test (tenere gli `shots/` attuali); commit iniziale del codice di oggi: `chore: stato di PulisciPC prima della modalità Sviluppatori`.
- [ ] **Step 2:** Copiare i sei crate dc-* (con i loro `tests.rs`) senza `target/`. In `crates/dc-engine`, `crates/dc-scanner` e altri: nessuna dipendenza da `dc-wiztree` (verificare con `grep -r dc_wiztree crates`: vuoto).
- [ ] **Step 3:** Vendor di mukade:
  ```bash
  cd /d/Projects/mukade && git archive dfde57e src catalog assets/intent-prompts README.md LICENSE | tar -x -C /d/Projects/KKK/ccleaner64/app/vendor/mukade
  rm -rf /d/Projects/KKK/ccleaner64/app/vendor/mukade/src/bin
  ```
  Copiare `Cargo.toml` e `VENDORED.md` da `devclean/vendor/mukade` (il manifest di mukade non cambia tra `97d827f` e `dfde57e`); in `VENDORED.md` sostituire il commit con `dfde57e` (2026-11-10), "Previously `97d827f`", e "devclean" con "PulisciPC".
- [ ] **Step 4:** `Cargo.toml` radice: aggiungere `[workspace] resolver = "3"`, `members = [".", "crates/*"]`; tenere `[package]`, `[[bin]]` e `[profile.release]` di oggi (`opt-level = "s"`, `lto = true`, `strip = true`).
- [ ] **Step 5:** `THIRD_PARTY_NOTICES.md` da devclean senza la sezione WizTree, con mukade a `dfde57e`.
- [ ] **Step 6:** Verifica.
  Run: `cargo test --workspace --exclude pulisci-pc --exclude mukade`
  Expected: `dc_cleaner` 11, `dc_engine` 37, `dc_git` 20, `dc_model` 5, `dc_mukade` 7 (+1 ignored), `dc_scanner` 27 passed, 0 failed (numeri di devclean il 2026-11-10).
  Run: `cargo build -p pulisci-pc` → compila, la UI è quella di prima.
- [ ] **Step 7:** Commit `chore: workspace con i crate di devclean e mukade dfde57e`.

### Task 1: Impostazioni senza WizTree

Stima: 15 min.

**Files:**
- Modify: `crates/dc-model/src/lib.rs:400-420` (`Settings`)
- Test: test in fondo a `crates/dc-model/src/lib.rs`

**Interfaces:**
- Produces: `Settings { session_projects: bool, roots: Vec<PathBuf>, scan_roots: bool, inactive_days: u32, remove_mode: RemoveMode }` (tolti `use_wiztree`, `wiztree_path`).

- [ ] **Step 1: Test che fallisce**
  ```rust
  #[test]
  fn old_devclean_settings_with_wiztree_fields_still_load() {
      let json = r#"{"sessionProjects":true,"roots":["D:\\Projects"],"scanRoots":false,
                     "inactiveDays":30,"useWiztree":true,"wiztreePath":"C:\\x\\WizTree64.exe",
                     "removeMode":"permanent"}"#;
      let s: Settings = serde_json::from_str(json).unwrap();
      assert_eq!(s.inactive_days, 30);
      assert_eq!(s.remove_mode, RemoveMode::Permanent);
      assert_eq!(s.roots, vec![PathBuf::from(r"D:\Projects")]);
  }
  ```
  Il test passa già con i campi presenti; per renderlo significativo, togliere prima i campi (Step 2) e controllare che `Settings` non abbia `#[serde(deny_unknown_fields)]`.
- [ ] **Step 2:** Togliere `use_wiztree` e `wiztree_path` da `Settings`; aggiornare i test esistenti che li costruiscono.
- [ ] **Step 3:** `cargo test -p dc-model` → 6 passed.
- [ ] **Step 4:** Commit `refactor(model): impostazioni senza WizTree`.

### Task 2: `dc-disktree`, indice delle cartelle dalla MFT

Stima: 60-75 min.

**Files:**
- Create: `crates/dc-disktree/Cargo.toml`, `crates/dc-disktree/src/lib.rs` (indice, `DirIndex`), `crates/dc-disktree/src/mft.rs` (lettura MFT), `crates/dc-disktree/VENDORED.md`, `crates/dc-disktree/examples/peak.rs`
- Origine: `D:\Projects\KKK\wiztree64\app\src\mft.rs` (lettura volume, `fixup`, `data_runs`, ciclo di lettura a blocchi da 131072 byte) e `sys.rs` (solo `wide`, `is_admin`, `drives` se servono). Non si copiano `app.rs`, `treemap.rs`, `fmt.rs`, `model.rs`, `scan.rs`.

**Interfaces:**
- Consumes: `dc_model::{DirIndex, IndexedDir, SizeInfo, path_key, normalize_path}`.
- Produces:
  ```rust
  pub struct Progress { pub records: AtomicU64, pub cancel: AtomicBool }
  pub struct DiskIndex { /* path_key -> (path, SizeInfo, reparse); nome minuscolo -> chiavi */ }
  impl DirIndex for DiskIndex { /* backend_name() == "mft" */ }
  impl DiskIndex {
      pub fn from_nodes(volume_root: &str, nodes: Vec<Node>) -> DiskIndex;
      pub fn merge(&mut self, other: DiskIndex);
  }
  pub struct Node { pub parent: u32, pub is_dir: bool, pub reparse: bool,
                    pub name: Option<Box<str>>, pub size: u64, pub alloc: u64 }
  pub fn volume_index(letter: char, progress: &Progress) -> Result<DiskIndex, String>;
  pub fn build_for_roots(roots: &[PathBuf], admin: bool, progress: &Progress)
      -> (Option<DiskIndex>, Vec<String>);
  ```
  `nodes[0]` è la radice del volume; `name` è `Some` solo per le cartelle (i nomi dei file non si tengono: è il punto del limite di RAM). `parent` dei nodi è un indice in `nodes`.

- [ ] **Step 1: Test che falliscono** (in `src/lib.rs`, `mod tests`), albero fatto a mano:
  ```rust
  // 0 C:\  1 Projects  2 app  3 target  4 f(100/4096) in target  5 f(100/4096) in target
  // 6 f(10/0) in app  7 link (dir, reparse) in Projects named "target"
  #[test] fn sizes_add_up_through_folders() {
      let ix = sample();
      let t = ix.size_of(Path::new(r"C:\Projects\app\target")).unwrap();
      assert_eq!((t.logical_bytes, t.allocated_bytes, t.file_count), (200, Some(8192), 2));
      let a = ix.size_of(Path::new(r"C:\Projects\app")).unwrap();
      assert_eq!((a.logical_bytes, a.allocated_bytes, a.file_count), (210, Some(8192), 3));
  }
  #[test] fn finds_dirs_by_name_under_roots_case_insensitive_without_links() {
      let hits = sample().find_dirs_named(&["TARGET"], &[PathBuf::from(r"C:\Projects")]);
      assert_eq!(hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
                 vec![PathBuf::from(r"C:\Projects\app\target")]);
      assert!(sample().find_dirs_named(&["target"], &[PathBuf::from(r"C:\Other")]).is_empty());
  }
  #[test] fn merge_keeps_both_volumes() { /* C: + D: -> size_of su entrambi Some */ }
  #[test] fn missing_drive_gives_warning_and_no_index() {
      let (ix, w) = build_for_roots(&[PathBuf::from(r"Q:\nowhere")], true, &Progress::default());
      assert!(ix.is_none());
      assert!(w.iter().any(|m| m.contains("Q:")));
  }
  #[test] fn not_admin_gives_no_index() {
      let (ix, w) = build_for_roots(&[PathBuf::from(r"C:\Windows")], false, &Progress::default());
      assert!(ix.is_none() && w.is_empty());
  }
  ```
- [ ] **Step 2:** `cargo test -p dc-disktree` → FAIL (tipi non definiti).
- [ ] **Step 3:** `from_nodes`: somme dirette per cartella, poi un passaggio dai figli ai genitori in ordine di profondità decrescente (niente risalita ricorsiva per ogni file); percorsi delle cartelle costruiti una volta, `path_key` come chiave. `allocated_bytes` sempre `Some`. `find_dirs_named` come `WizTreeIndex` in `devclean/crates/dc-wiztree/src/lib.rs:233-260`, escludendo le cartelle `reparse` (attributo `0x400`).
- [ ] **Step 4:** `mft.rs`: il ciclo di `wiztree64` con un `Rec` compatto (`in_use`, `is_dir`, `reparse`, `parent: u32`, `name` solo se cartella, `size`, `alloc`, `name_rank`), poi conversione in `Vec<Node>` con la radice (record 5) in posizione 0. `build_for_roots`: raggruppa le radici per lettera di unità; `admin == false` → `(None, vec![])`; unità inesistente o non NTFS → avviso `"Unità Q: non disponibile per la lettura veloce: uso la scansione normale."` e la si salta; un volume letto una volta sola.
- [ ] **Step 5:** `cargo test -p dc-disktree` → 5 passed.
- [ ] **Step 6:** `examples/peak.rs`: da admin legge `C:` e `D:`, stampa record, cartelle, tempo e picco della memoria del processo (`GetProcessMemoryInfo`, `PeakWorkingSetSize`), e confronta `size_of` con `dc_cleaner::measure_dir` su 3 cartelle `target` reali sotto `D:\Projects`.
  Run (terminale admin): `cargo run -p dc-disktree --release --example peak`
  Expected: picco < 300 MB; differenza `allocated` MFT vs misura < 1% per ogni `target`. Se il picco supera 300 MB, fermarsi e segnalarlo (la spec dice: MFT solo per le cartelle aggiuntive).
- [ ] **Step 7:** `VENDORED.md`: da dove viene il codice, data/ora della copia, SHA-256 di `wiztree64/app/src/mft.rs` e `sys.rs` al momento della copia (`sha256sum`).
- [ ] **Step 8:** Commit `feat(disktree): indice delle cartelle dalla MFT al posto di WizTree`.

### Task 3: sessioni archiviate e worktree di Claude Code in `dc-mukade`

Stima: 45 min.

**Files:**
- Modify: `crates/dc-mukade/src/lib.rs` (nuove funzioni, `managed_worktrees` per Claude)
- Test: `crates/dc-mukade/src/lib.rs` `mod tests`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug)]
  pub struct ArchivedSession {
      pub agent: Agent, pub id: String, pub title: Option<String>,
      pub cwd: Option<PathBuf>,          // normalize_path
      pub updated_ms: Option<i64>,
      pub file: Option<PathBuf>,         // file proprio della sessione che si può togliere
      pub bytes: u64,                    // dimensione di `file`, 0 se None
      pub blocked: Option<String>,       // motivo in italiano; None = cancellabile
  }
  impl MukadeSessions { pub fn archived(&self) -> Loaded<ArchivedSession>; }
  pub fn select_archived(agent: Agent, store_root: &Path,
                         roots: Vec<SessionInfo>, all: &[SessionInfo]) -> Vec<ArchivedSession>;
  pub fn claude_worktrees(cwds: &[PathBuf]) -> Vec<PathBuf>;
  ```
  `archived()`: `roots` = `Reader::list` (per Claude: `list_cached` filtrato su `parent == None`), `all` = `list_all` / `list_cached`. Store assente → `Loaded::default()` senza avvisi.

- [ ] **Step 1: Test che falliscono** (fixture `SessionInfo` costruite a mano, file in una cartella temporanea):
  - `only_archived_roots_are_listed`: 3 radici, 1 archiviata → 1 risultato.
  - `live_archived_session_is_blocked`: `live: Some(true)` → `blocked == Some("sessione in uso")`.
  - `archived_parent_with_active_child_is_blocked`: figlia (in `all`, `parent = Some(id)`) non archiviata → `blocked` contiene `"figlia"`; con figlia archiviata → `blocked == None`.
  - `codex_file_is_the_rollout_when_it_exists`: `path` esistente `.jsonl` → `file == Some(path)`, `bytes == len`; `path` inesistente → `file == None`, `blocked == None`.
  - `claude_never_targets_history_or_pid_files`: `path` = `<store>\history.jsonl` e `<store>\sessions\123.json` → `file == None` e `blocked == Some("nessun file della sessione")`; `<store>\projects\x\abc.jsonl` → `file == Some`.
  - `opencode_in_another_database_is_blocked`: `path` diverso da `store_root` → `blocked` contiene `"altro database"`; `file` sempre `None`.
  - `verbatim_prefixed_cwd_is_normalized`: `cwd = Some(r"\\?\C:\Users\x\.codex\worktrees\a\daodan")` → `cwd == Some(PathBuf::from(r"C:\Users\x\.codex\worktrees\a\daodan"))` (via `normalize_path`).
  - `missing_store_lists_nothing_quietly`: `MukadeSessions::with_roots(Agent::Pi, roots su cartella inesistente).archived()` → `items` e `warnings` vuoti.
  - `claude_worktrees_from_cwds`: cwd `…\repo\.claude\worktrees\w1\sub` → `…\repo\.claude\worktrees\w1`; cwd `…\repo2` con `…\repo2\.claude\worktrees\w2\.git` (file) → `…\repo2\.claude\worktrees\w2`; doppioni eliminati.
- [ ] **Step 2:** `cargo test -p dc-mukade` → i nuovi FAIL.
- [ ] **Step 3:** Implementare. `managed_worktrees()` per `Agent::Claude` = `claude_worktrees(cwd delle sessioni)`.
- [ ] **Step 4:** `cargo test -p dc-mukade` → 16 passed (7 + 9), 1 ignored.
- [ ] **Step 5:** Commit `feat(mukade): sessioni archiviate cancellabili e worktree di Claude Code`.

### Task 4: `dc-sessions`, cancellazione delle sessioni archiviate

Stima: 45 min.

**Files:**
- Create: `crates/dc-sessions/Cargo.toml` (dipende da `dc-model`, `dc-mukade`, `trash = "5.3.0"`), `crates/dc-sessions/src/lib.rs`, `crates/dc-sessions/src/cli.rs`

**Interfaces:**
- Consumes: `dc_mukade::ArchivedSession`, `dc_model::{Agent, RemoveMode, ItemOutcome}`.
- Produces:
  ```rust
  pub trait HarnessCli: Send + Sync {
      fn codex_delete(&self, id: &str) -> Result<(), String>;
      fn opencode_export(&self, id: &str, out: &Path) -> Result<(), String>;
      fn opencode_delete(&self, id: &str) -> Result<(), String>;
  }
  pub trait Trash: Send + Sync { fn to_bin(&self, path: &Path) -> Result<(), String>; }
  pub struct SystemTrash;                    // trash::delete
  pub struct SystemCli { pub codex: Option<PathBuf>, pub opencode: Option<PathBuf>, pub timeout: Duration }
  impl SystemCli { pub fn locate() -> SystemCli; }   // timeout 60 s
  pub struct SessionOutcome { pub agent: Agent, pub id: String, pub outcome: ItemOutcome }
  pub fn delete_sessions(items: &[ArchivedSession], mode: RemoveMode,
                         cli: &dyn HarnessCli, trash: &dyn Trash) -> Vec<SessionOutcome>;
  ```
  Regole per harness (spec, tabella "Cancellazione delle sessioni archiviate"):
  - bloccata → `Skipped { reason: blocked }`.
  - Claude Code, Pi: `Recycle` → `trash.to_bin(file)`; `Permanent` → `remove_file`. `Removed { bytes_freed: bytes }` solo se il file non esiste più.
  - Codex: `Recycle` e `file` presente → copia in `%TEMP%\PulisciPC\<id>.jsonl`, `trash.to_bin(copia)`, poi `codex_delete`; `Permanent` → solo `codex_delete`. Errore della CLI → `Failed { error, locked_by: vec![] }` e il rollout originale resta.
  - OpenCode: `Recycle` → `opencode_export` in `%TEMP%\PulisciPC\opencode-<id>.json`, `trash.to_bin(export)`, poi `opencode_delete`; `Permanent` → solo `opencode_delete`.

- [ ] **Step 1: Test che falliscono** con `FakeCli` (registra le chiamate, errore configurabile) e `FakeTrash` (registra i percorsi, sposta il file in una cartella temporanea):
  - `blocked_session_is_skipped_and_nothing_called`
  - `claude_permanent_removes_the_transcript` → file sparito, `Removed { bytes_freed: len }`
  - `claude_recycle_sends_the_transcript_to_the_bin` → `FakeTrash` ha il percorso del file
  - `codex_recycle_bins_a_copy_then_deletes` → trash riceve un percorso ≠ rollout con lo stesso contenuto; `codex_delete(id)` chiamato dopo
  - `codex_without_rollout_is_one_cli_call` → `Removed { bytes_freed: 0 }`
  - `cli_failure_keeps_the_session` → `Failed` con il testo dell'errore; rollout ancora presente
  - `opencode_recycle_exports_then_deletes` → ordine `export`, `to_bin(export)`, `delete`
  - `missing_codex_cli_fails_only_codex` → `SystemCli { codex: None, .. }`: `codex_delete` → `Err("CLI di Codex non trovata")`; con `opencode: None` → `Err("CLI di OpenCode non trovata")`
- [ ] **Step 2:** `cargo test -p dc-sessions` → FAIL.
- [ ] **Step 3:** Prima di scrivere `cli.rs`, controllare la CLI reale e annotare l'esito nel commento in testa al file:
  Run: `opencode --standalone session delete --help` e `opencode --standalone session export --help`
  Expected: nessuna conferma interattiva richiesta; se c'è un flag di conferma (`--yes`, `--force`), usarlo. Se `export` scrive su file invece che su stdout, usare il suo flag.
- [ ] **Step 4:** Implementare. `SystemCli::locate`: `codex.exe` nel PATH, poi `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin\codex.exe`; `opencode.cmd` / `opencode.exe` nel PATH, poi `%APPDATA%\npm\opencode.cmd`. Comandi con stdin nullo, senza finestra (`CREATE_NO_WINDOW`), uccisi allo scadere del timeout; `Err` = codice di uscita ≠ 0 con le ultime righe di stderr.
- [ ] **Step 5:** `cargo test -p dc-sessions` → 8 passed.
- [ ] **Step 6:** Commit `feat(sessions): cancellazione delle sessioni archiviate tramite le CLI degli harness`.

### Task 5: `dc-app`, la colla senza UI

Stima: 45 min.

**Files:**
- Create: `crates/dc-app/Cargo.toml`, `crates/dc-app/src/lib.rs`, `crates/dc-app/examples/smoke.rs`
- Origine: `devclean/src-tauri/src/lib.rs` (`Preloaded`, `load_sessions`, `ScanRoots`, `key_within`, `plan_roots`, `SplitScanner`, `run_scan`, l'allow-list di `execute_plan`) e `devclean/src-tauri/examples/smoke.rs`, senza Tauri e senza WizTree.

**Interfaces:**
- Consumes: Task 1-4.
- Produces:
  ```rust
  pub fn default_settings() -> Settings;            // remove_mode: Recycle; roots come devclean
  pub fn settings_path() -> PathBuf;                // %APPDATA%\PulisciPC\settings.json
  pub fn cache_dir() -> PathBuf;                    // %LOCALAPPDATA%\PulisciPC\mukade
  pub fn load_settings() -> Settings;  pub fn save_settings(s: &Settings) -> Result<(), String>;
  pub struct DevScan {
      pub projects: Vec<ProjectReport>,     // !is_agent_worktree && !git.is_linked_worktree
      pub worktrees: Vec<ProjectReport>,    // is_agent_worktree (selezionabili)
      pub other_worktrees: Vec<ProjectReport>, // git.is_linked_worktree && !is_agent_worktree (solo info)
      pub archived: Vec<ArchivedSession>,
      pub warnings: Vec<String>, pub backend: String, pub elapsed_ms: u64,
  }
  pub fn run_scan(settings: &Settings, admin: bool, step: &dyn Fn(&str, f32)) -> DevScan;
  pub fn build_plan(scan: &DevScan, selected_ids: &[String], mode: RemoveMode) -> Result<CleanPlan, String>;
  pub fn execute_plan(scan: &DevScan, plan: &CleanPlan) -> Result<CleanResult, String>;
  pub fn delete_archived(scan: &DevScan, picked: &[(Agent, String)], mode: RemoveMode,
                         cli: &dyn HarnessCli, trash: &dyn Trash) -> Result<Vec<SessionOutcome>, String>;
  ```
  `run_scan`: passi `step("Lettura delle sessioni…", 0.05)`, `("Lettura veloce del disco…", 0.25)`, `("Analisi dei progetti…", 0.45)`, `("Sessioni archiviate…", 0.9)`. Con `admin` l'indice `dc-disktree` si costruisce per **tutte** le radici e si passa al scanner per tutte (proposta della spec: dimensioni degli artefatti dalla MFT); senza admin `index = None` e tutto va col walker di `dc-scanner`. `backend` = `"mft"` o `"walker"`.

- [ ] **Step 1: Test che falliscono** (`DevScan` costruito a mano):
  - `execute_plan_refuses_paths_not_in_the_last_scan` → `Err` che contiene `"rifiutato"`
  - `delete_archived_refuses_unknown_or_blocked_ids` → `Err` per un id assente e per uno con `blocked: Some(_)`; nessuna chiamata a `FakeCli`
  - `reports_are_split_into_projects_worktrees_and_other_worktrees` (funzione `split_reports(Vec<ProjectReport>) -> (Vec, Vec, Vec)`)
  - `plan_roots_*`: portare i casi di `devclean` se esistono; altrimenti un test: radice dentro un'altra radice → resta solo l'esterna
- [ ] **Step 2:** `cargo test -p dc-app` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** `cargo test -p dc-app` → tutti passati.
- [ ] **Step 5:** `examples/smoke.rs`: stampa progetti, worktree, altri worktree, archiviate per harness (totali e bloccate), backend, tempo.
  Run: `cargo run -p dc-app --release --example smoke`
  Expected (dati del 2026-11-10, possono cambiare): progetti circa 111; archiviate Codex circa 664; Claude Code 0, Pi 0; tempo ≤ 64 s senza admin.
- [ ] **Step 6:** Commit `feat(app): analisi e pulizia per sviluppatori senza UI`.

### Task 6: voce "Sviluppatori" nella UI

Stima: 75-90 min.

**Files:**
- Create: `src/theme.rs` (costanti colore, `card`, `primary`, `secondary` spostate da `src/main.rs` **senza modifiche**), `src/dev.rs`
- Modify: `src/main.rs` (`Tab::Dev`, voce nella barra laterale, `update`, `--tab`, `--analyze`), `Cargo.toml` (dipendenze `dc-app`, `dc-model`, `dc-mukade`, `dc-sessions`, `rfd = "0.15"`)

**Interfaces:**
- Consumes: `dc_app::*` (Task 5), `sys::is_admin()`, `sys::fmt_size()`.
- Produces: `pub struct DevState` + `pub fn dev_tab(state: &mut DevState, ui: &mut Ui, admin: bool)` + `DevState::poll(&mut self)` + `DevState::busy(&self) -> bool` + `DevState::release(&mut self)` (libera `scan` se non `busy`).

Layout (come `cleaner_tab`, `src/main.rs:394-475`):
- Barra laterale: quarta voce `(Tab::Dev, "💻", "Sviluppatori")` dopo "Servizi e RAM". Se l'icona esce come quadratino nello screenshot, usare `"⌨"`. Cambiando voce si chiama `release()`.
- Pannello sinistro 270 px su `CARD` con bordo `LINE`: gruppo **Sezione** (tre `selectable_label`: "Progetti (N)", "Worktree (N)", "Sessioni archiviate (N)"), gruppo **Opzioni**: "Giorni di inattività" (`DragValue` 1..=365), **Eliminazione**: radio "Nel Cestino (recuperabile)" / "Definitiva" con nota piccola `MUTED`: "Il Cestino non libera spazio finché non lo svuoti. «Cestino» in Pulizia file lo svuota.", **Cartelle aggiuntive**: checkbox `scan_roots`, elenco, "Aggiungi cartella…" (`rfd::FileDialog::pick_folder`), "Togli". Ogni modifica salva le impostazioni.
- Barra in basso: `primary("Avvia pulizia")` (attivo con qualcosa selezionato), `secondary("Analizza")`.
- Card centrale: durante l'analisi testo + `ProgressBar` `ACCENT`; vuota: titolo "Pulizia per sviluppatori", sottotitolo "Premi «Analizza»: i progetti vengono dalle sessioni di Claude Code, Codex, OpenCode e Pi."; dopo l'analisi una griglia `striped` per sezione con casella di selezione:
  - Progetti: Progetto, Stato (`reason`), Ultimo uso, Da liberare. Preselezione = `preselected`.
  - Worktree: Worktree, Repository (`git.main_worktree`), Stato, Dimensione; sotto, "Altri worktree (creati a mano, non si rimuovono da qui)" in `MUTED` senza casella.
  - Sessioni archiviate: Harness, Titolo, Progetto, Data, Dimensione ("solo riga nel database" se `file == None`), motivo in `MUTED` se bloccata (casella disattivata). *Scelta di piano:* preselezionate tutte quelle non bloccate.
  - Gli `warnings` in una striscia `WARN_BG` come `admin_strip`.
- Conferma: `egui::Modal` come `confirm_clean`; testo per `Recycle`: "Gli elementi selezionati vanno nel Cestino. I worktree vengono rimossi definitivamente con git."; per `Permanent`: "Gli elementi selezionati verranno eliminati definitivamente."
- Esito: "Pulizia completata", "Liberati X" in `GREEN`, griglia con ogni elemento e Rimosso / Saltato (motivo) / Non riuscito (errore).
- Job: thread + `mpsc::Receiver<DevMsg>`, `enum DevMsg { Step(String, f32), Scanned(DevScan), Cleaned(DevCleaned) }`; `update` richiede repaint ogni 150 ms anche quando `dev.busy()`; `handle_screenshot` considera `dev.busy()` come gli altri job.
- `--tab dev|sviluppatori` apre la voce; `--analyze` avvia l'analisi all'apertura; `--dev-section projects|worktree|sessions` sceglie la sezione (tutti e tre per gli screenshot).

- [ ] **Step 1:** Spostare costanti e helper in `src/theme.rs`. `cargo build -p pulisci-pc`; screenshot `--tab cleaner --screenshot shots/check-cleaner.png` identico a `shots/cleaner.png` a occhio (stesso layout).
- [ ] **Step 2:** `Tab::Dev`, `DevState`, pannelli e griglie come sopra.
- [ ] **Step 3:** Verifica visiva.
  Run: `cargo run -p pulisci-pc --release -- --tab dev --screenshot shots/dev-empty.png`, poi con `--analyze` per `shots/dev-projects.png`; per Worktree e Sessioni aggiungere `--dev-section worktree|sessions`.
  Expected: stessa barra laterale con la quarta voce, stesse card e colori; griglie leggibili a 1100×700; icona visibile (non un quadratino).
- [ ] **Step 4:** Prova a mano: avviare l'analisi, passare a "Registro" e tornare: il job continua e il risultato arriva; dopo la fine, cambiare voce libera i dati (la sezione torna vuota).
- [ ] **Step 5:** Commit `feat(ui): voce Sviluppatori con Progetti, Worktree e Sessioni archiviate`.

### Task 7: verifica finale della Fase 1

Stima: 45 min.

- [ ] **Step 1:** `cargo test --workspace --exclude mukade` → tutto verde; annotare i numeri.
- [ ] **Step 2:** `grep -ri wiztree --exclude-dir=target --exclude-dir=docs . | grep -v VENDORED.md` → vuoto.
- [ ] **Step 3: Cancellazione reale su una copia, mai sullo store vero.** Copia di `~/.codex/state_5.sqlite` in `<scratchpad>\codexhome` (backup SQLite, come il 2026-11-10); `CODEX_HOME=<scratchpad>\codexhome` e PulisciPC avviato da quel terminale; analisi; nella sezione Sessioni archiviate selezionare 2 sessioni Codex; una pulizia in modalità Definitiva (1 sessione) e una in Cestino (1 sessione).
  Expected: esito "Rimosso" per entrambe; `python -I` sulla copia: le due righe non ci sono più, archiviate 3802 → 3800 (o il numero di partenza − 2); `~/.codex/state_5.sqlite` vero invariato (stesso numero di archiviate di prima).
- [ ] **Step 4:** RAM: Gestione attività / `Get-Process PulisciPC` → working set a riposo prima dell'analisi, dopo l'analisi, dopo aver cambiato voce. Expected: dopo aver cambiato voce torna vicino al valore iniziale (± 30 MB).
- [ ] **Step 5:** Tempo dell'analisi da admin (MFT) e senza admin; confronto con i 60-64 s di devclean.
- [ ] **Step 6:** `cargo build -p pulisci-pc --release`; dimensione dell'exe annotata.
- [ ] **Step 7:** Aggiornare `D:\Projects\KKK\ccleaner64\app\docs\superpowers\specs\2026-11-10-dev-mode-design.md` sezione Verifica con i numeri misurati; commit `docs: verifica della Fase 1`.
