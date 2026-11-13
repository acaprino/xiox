> Nota (2026-11-13): il prodotto ora si chiama **Xiox**. I nomi di questo documento (PulisciPC, `pulisci-pc`, `pc-rules`, i crate `dc-*`) sono quelli di allora; oggi sono `xiox-desktop` (binario `xiox-ui`), `xiox-rules` e `xiox-*` (`dc-app` -> `xiox-app`, ecc.). Il testo qui sotto non è stato aggiornato.

# Fase 2, regole esterne: motore `pc-rules`, Winapp2, CleanerML inglobati. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pulizia file di PulisciPC carica le regole di Winapp2 (scaricate a runtime e aggiornate a ogni avvio se il remoto ha una versione nuova) e gli 11 XML CleanerML inglobati nell'exe, e le mostra come categorie native accanto a quelle di Windows.

**Architecture:** Un crate nuovo `crates/pc-rules` (nessuna UI) legge i due formati in un modello comune, espande percorsi e variabili d'ambiente, rileva cosa è installato e restituisce file e chiavi di registro da togliere; un modulo `updater` scarica `Winapp2.ini` in `%APPDATA%\PulisciPC\winapp2\`. `src/cleaner.rs` e la scheda Pulizia file usano il crate per aggiungere categorie dinamiche dopo le 9 statiche.

**Tech Stack:** Rust stabile, `roxmltree` (CleanerML), `curl.exe` di Windows 11 per il download (nessuna dipendenza HTTP nuova), `rayon` già nel workspace, `winreg` 0.56 già in uso.

**Spec:** `docs/superpowers/specs/2026-11-10-dev-mode-design.md` (sezioni "Regole di pulizia esterne", "Copertura nativa di CCleaner") e `docs/superpowers/specs/2026-11-10-catalogo-sviluppo-design.md` (regole 2, 3, 5, 7, 9 e Parte B). Leggerle entrambe.

**Sequenza con il piano 1 del catalogo** (`2026-11-10-catalogo-1-progetti-e-pulizia.md`, eseguito da un'altra sessione): i Task 1-6 qui sotto creano un crate nuovo e due cartelle (`cleaners/`, `crates/pc-rules/`): nessun file in comune, si possono eseguire subito. I Task 7-10 toccano `src/cleaner.rs`, `src/main.rs`, `Cargo.toml`: **iniziano solo dopo il Task 11 del piano 1** (`feat(cleaner): Pulizia file con worker paralleli e resoconto finale`), perché usano `clean(index, found, tick, cancel)`, `CleanReport` e i worker. Controllare con `git log --oneline | grep "Pulizia file con worker"` e `git status --short` vuoto.

## Global Constraints

- Windows 11, testo della UI in italiano corretto (accenti, niente prefissi "AGG -", nessuna dicitura "BleachBit" nella UI).
- **Layout di ccleaner64 intatto**: barra laterale, `card()`, `primary()`, `secondary()`, colori di `src/theme.rs`. Le categorie nuove usano le stesse caselle e gli stessi gruppi della scheda.
- Winapp2: file `Non-CCleaner/Winapp2.ini` del ramo `master` di `MoscaDotTo/Winapp2`, URL `https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini`. Misura del 2026-11-10: `; Version: 260915`, 4068 voci, 1.833.427 byte, ETag presente; chiavi `FileKey` 14121, `RegKey` 3677, `DetectFile` 3322, `LangSecRef` 2231, `Section` 1837, `Detect` 1444, `ExcludeKey` 86, `Warning` 5; flag `RECURSE` e `REMOVESELF`.
- Il file non entra mai nell'exe. Cartella dati `%ProgramData%\PulisciPC\winapp2\` (`Winapp2.ini`, `etag.txt`, sottocartella `tmp` per `curl`), **scrivibile solo da SYSTEM e Amministratori** (ACL impostata con `icacls` e SID noti; gli Utenti hanno solo lettura): l'app gira sempre da amministratore e non si fida di una copia che un processo senza privilegi può modificare (decisione dell'utente 2026-11-10). Una copia vecchia in `%APPDATA%` si ignora. Le variabili d'ambiente di base (profilo, AppData, ProgramData, Windows, Temp…) per le regole vengono dalle cartelle note di Windows, non dall'ambiente del processo (`Env::for_rules`). Download in sottofondo a ogni avvio con `If-None-Match`; 304 = niente; 200 valido sostituisce in modo atomico; non valido o errore = si tiene il vecchio. User-Agent `PulisciPC`, nessun dato dell'utente nella richiesta. Primo avvio offline: la sezione dice "Winapp2 non ancora scaricato", il resto funziona.
- CleanerML: gli 11 XML di `D:\Software\BleachBit\cleaners` (`ai_claude_code`, `ai_codex`, `ai_gemini_cli`, `ai_opencode`, `bbag_cargo`, `bbag_crash`, `bbag_go`, `bbag_jetbrains`, `bbag_npm`, `bbag_pip`, `bbag_shader`) si copiano in `cleaners/` del progetto e si compilano nell'exe con `include_str!`; nessuna lettura di `D:\Software\BleachBit` a runtime. Solo `os="windows"` o senza `os`. Tutte le opzioni **spente di default**, anche quelle che toccano sessioni (decisione dell'utente 2026-11-10); `<warning>` chiede conferma; `<running>` blocca l'opzione se il processo è attivo.
- Supportati in CleanerML: `<cleaner>`, `<label>`, `<description>`, `<option>`, `<warning>`, `<var>` con più `<value>` e `$$nome$$`, `<running>`, `<action command="delete">` con `search` = `file`, `glob`, `walk.all`, `walk.files`, `walk.top` e `regex`/`type` (non `nregex`: vedi sotto), `command="winreg"`. Non supportati in questa fase (l'opzione compare disattivata con "non supportato", mai allargata): `nregex`, `wholeregex`, `nwholeregex`, ogni attributo sconosciuto, `sqlite.vacuum`, `json`, `ini`, `cookie`, `chrome.*`, `mozilla.*`, `office_registrymodifications`, `deep`, `process`, `win.shell.change.notify`.
- **Percorsi protetti per le regole esterne** (lista propria, non quella di `dc-cleaner/src/guard.rs`): mai la radice di un disco; mai `%WinDir%`, `%ProgramFiles%`, `%ProgramFiles(x86)%`, `%ProgramData%`, `%UserProfile%`, `%AppData%`, `%LocalAppData%`, `%SystemDrive%` presi interi (le sottocartelle sì); mai `System32`, `SysWOW64`, `WinSxS` e il loro contenuto; mai una cartella `.git`; mai file di credenziali degli agenti (`.credentials.json`, `auth.json`, `credentials.toml`, `token`). Una regola che punta lì si scarta e si conta nei avvisi.
- Chiavi di registro: solo sotto `HKCU\Software`, `HKLM\Software`, `HKCU\Software\Classes`, `HKLM\Software\Classes`; prima di cancellare si fa il backup `.reg` come `registry::backup` (`src/registry.rs:366`).
- Variabile d'ambiente non definita: il percorso si **scarta**, non si usa alla lettera (regola 3 del catalogo).
- Una regola Winapp2 che copre lo stesso strumento di una categoria inglobata (stessi percorsi o sottopercorsi) si nasconde: vince l'inglobata (regola 9).
- Test **mai sul PC dell'utente**: alberi finti in `tempfile::tempdir()`, mappa di variabili d'ambiente finta passata alle funzioni (nessuna lettura di `std::env` nei test), nessuna cancellazione fuori dalle cartelle temporanee. Il file Winapp2 vero serve solo per il test di lettura (sola lettura).
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; `git -c user.name="Alfio Caprino" -c user.email="alfio.caprino@gmail.com"`; solo i percorsi del task.

## Review Focus

1. **Il percorso di una regola esce dalla sua cartella** (`..\..\Windows`, un link simbolico o una giunzione dentro la cartella cache che punta a `C:\Users\x\Documents`): non si segue, il link si toglie come link, non il contenuto. Test in Task 1.
2. **Una regola Winapp2 che si espande a `%UserProfile%` o a `%AppData%` interi** (variabile + `*.*` RECURSE): scartata e contata. Test in Task 1 e Task 5.
3. **File scaricato incompleto o sbagliato** (HTML di errore, 0 byte, troncato a metà voce, solo l'intestazione): non sostituisce il vecchio. Test in Task 4.
4. **Rete lenta o assente all'avvio**: l'app parte subito, l'aggiornamento non blocca la UI né l'uscita dal programma (thread, timeout 20 s, nessun `join` in chiusura). Test in Task 4.
5. **`Winapp2.ini` molto grande** (1,8 MB, 4068 voci): il caricamento e il rilevamento non bloccano la UI e non tengono in RAM più del necessario (le voci non rilevate si scartano dopo il rilevamento). Misura in Task 10.

---

### Task 1: crate `pc-rules`: modello, espansione dei percorsi, percorsi protetti

Stima: 60-75 min.

**Files:**
- Create: `crates/pc-rules/Cargo.toml` (edition 2024; dipendenze `roxmltree = "0.20"`; dev: `tempfile`), `crates/pc-rules/src/lib.rs`, `crates/pc-rules/src/model.rs`, `crates/pc-rules/src/expand.rs`, `crates/pc-rules/src/guard.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Env(pub HashMap<String, String>);            // chiavi in minuscolo
  impl Env { pub fn from_process() -> Env; pub fn get(&self, name: &str) -> Option<&str>; }
  pub enum Origin { Winapp2, CleanerMl(&'static str) }
  pub struct Cleaner { pub id: String, pub label: String, pub group: String, pub origin: Origin,
                       pub description: String, pub running: Vec<String>, pub options: Vec<Opt> }
  pub struct Opt { pub id: String, pub label: String, pub description: String, pub warning: Option<String>,
                   pub default_on: bool, pub actions: Vec<Action>, pub unsupported: Option<String> }
  pub enum Action {
      Files { path: String, patterns: Vec<String>, recurse: bool, remove_self: bool, search: Search },
      RegKey { root: RegRoot, path: String },
      RegValue { root: RegRoot, path: String, value: String },
  }
  pub enum Search { File, Glob, WalkAll, WalkFiles, WalkTop, Winapp2 }
  pub enum RegRoot { Hkcu, Hklm }
  pub fn expand(path: &str, env: &Env) -> Vec<PathBuf>;     // %Var%, $$var$$ non qui; vuoto se una variabile manca; (x86) incluso
  pub fn protected(path: &Path, env: &Env) -> Option<&'static str>;   // Some(motivo in italiano) se il percorso è protetto
  ```
  `expand`: sostituisce `%Nome%` (case-insensitive) con il valore; `%ProgramFiles%` produce sia `Program Files` sia `Program Files (x86)` se esistono nella mappa (`programfiles`, `programfiles(x86)`); variabile mancante → vettore vuoto.

- [ ] **Step 1: Test che falliscono** (`expand.rs` e `guard.rs`, mappa `Env` finta):
  - `expands_known_variables_case_insensitively`: `%LOCALAPPDATA%\Foo` con `localappdata=C:\U\L` → `C:\U\L\Foo`.
  - `unknown_variable_discards_the_path`: `%NOPE%\x` → vuoto.
  - `programfiles_expands_to_both_roots`: due risultati.
  - `protected_roots_are_refused`: `C:\`, `C:\Windows`, `C:\Windows\System32\x`, `C:\Program Files`, `C:\Users\u` (profilo intero), `C:\Users\u\AppData\Roaming`, `<proj>\.git`, `C:\Users\u\.codex\auth.json`; mentre `C:\Users\u\AppData\Roaming\Foo\Cache` e `C:\Program Files\Foo\log` no.
  - `parent_dir_components_are_refused`: `C:\Users\u\AppData\Local\Foo\..\..\..\Windows`.
- [ ] **Step 2:** `cargo test -p pc-rules` → FAIL.
- [ ] **Step 3:** Implementare modello, `expand` e `protected` (confronto con `dc_model::path_key` copiato o riscritto: il crate non dipende dagli altri dc-*; un `fn key(&Path) -> String` locale con minuscole, `\` e senza prefisso `\\?\`).
- [ ] **Step 4:** `cargo test -p pc-rules` → tutti verdi.
- [ ] **Step 5:** Commit `feat(rules): modello, espansione dei percorsi e percorsi protetti`.

### Task 2: parser Winapp2

Stima: 60-75 min.

**Files:**
- Create: `crates/pc-rules/src/winapp2.rs`, `crates/pc-rules/tests/fixtures/winapp2_sample.ini`
- Modify: `crates/pc-rules/src/lib.rs` (`pub mod winapp2;`)

**Interfaces:**
- Consumes: `Cleaner`, `Opt`, `Action`, `Env`, `expand` (Task 1).
- Produces:
  ```rust
  pub struct Winapp2 { pub version: Option<String>, pub declared_entries: Option<u32>, pub entries: Vec<Entry> }
  pub struct Entry { pub name: String, pub section: Option<String>, pub lang_sec_ref: Option<u32>,
                     pub detect: Vec<Detect>, pub default_on: bool, pub warning: Option<String>,
                     pub file_keys: Vec<FileKey>, pub reg_keys: Vec<RegKeyRule>, pub excludes: Vec<Exclude> }
  pub enum Detect { RegKey { root: RegRoot, path: String }, File(String) }
  pub struct FileKey { pub path: String, pub patterns: Vec<String>, pub recurse: bool, pub remove_self: bool }
  pub struct RegKeyRule { pub root: RegRoot, pub path: String, pub value: Option<String> }
  pub struct Exclude { pub kind: String, pub path: String, pub pattern: Option<String> }
  pub fn parse(text: &str) -> (Winapp2, Vec<String>);          // avvisi: chiavi sconosciute contate, righe illeggibili
  pub fn to_cleaners(w: &Winapp2, env: &Env) -> (Vec<Cleaner>, Report);   // solo voci rilevate; Report { discarded_protected, discarded_missing_var, skipped_unknown }
  ```
  Prima di scrivere il parser **leggere la sintassi ufficiale**: `gh api repos/MoscaDotTo/Winapp2/contents/README.md --jq .content | base64 -d` (e `CONTRIBUTING.md`): `FileKey<n>=PERCORSO|MODELLI(;)|FLAG`, `RegKey<n>=ROOT\chiave|valore?`, `Detect<n>`, `DetectFile<n>`, `ExcludeKey<n>`, `Default=False`, `Warning`, `LangSecRef`, `Section`, i flag `RECURSE`/`REMOVESELF`. Le chiavi che non conosce le ignora e le conta. Un'unica opzione per voce (tutta la voce è una casella: così fa CCleaner con Winapp2). `to_cleaners`: la voce è rilevata se almeno un `Detect`/`DetectFile` esiste (nessun `Detect` → rilevata se esiste la cartella di almeno un `FileKey`); le azioni con percorso protetto o variabile mancante si scartano e si contano; `Default=False` e ogni voce è spenta di default (decisione dell'utente: nessuna voce preselezionata). `group` = `Section` se presente, altrimenti "Applicazioni"; `label` = nome senza ` *` finale.

- [ ] **Step 1: Test che falliscono** con `winapp2_sample.ini` (circa 12 voci scritte a mano, copiate nella forma di quelle vere: una con `FileKey` + `RECURSE`, una con `REMOVESELF`, una con `RegKey` senza valore e una con valore, una con `ExcludeKey`, una con `Warning`, una senza `Detect`, una con chiave sconosciuta, intestazione con `; Version: 260915` e `; # of entries: 4,068`):
  - `reads_version_and_declared_entries`
  - `file_key_flags_and_patterns` (`*.log;*.tmp|RECURSE` → due modelli, `recurse`)
  - `reg_key_with_and_without_value`
  - `unknown_keys_are_counted_not_fatal`
  - `entry_without_any_installed_target_is_not_detected` (cartelle in `tempdir`, `Env` finta)
  - `rule_expanding_to_a_protected_path_is_discarded_and_counted` (`FileKey1=%UserProfile%|*.*|RECURSE`)
  - `everything_is_off_by_default`
  - `parses_the_real_file_when_given` (`#[ignore]`; legge il percorso in `PC_RULES_WINAPP2`, sola lettura; atteso: 4068 voci ±1%, `version == "260915"` o successiva)
- [ ] **Step 2:** `cargo test -p pc-rules` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** `cargo test -p pc-rules` verde; poi sul file vero scaricato in una cartella temporanea di scratch (`curl -L -o <scratch>\Winapp2.ini <URL>`): `PC_RULES_WINAPP2=<scratch>\Winapp2.ini cargo test -p pc-rules parses_the_real_file -- --ignored` → passa; annotare nel report quante voci, quante chiavi sconosciute e il tempo di lettura (atteso sotto 500 ms in release).
- [ ] **Step 5:** Commit `feat(rules): lettura di Winapp2.ini`.

### Task 3: parser CleanerML

Stima: 60 min.

**Files:**
- Create: `crates/pc-rules/src/cleanerml.rs`, `crates/pc-rules/tests/fixtures/cleanerml_sample.xml`
- Modify: `crates/pc-rules/src/lib.rs`

**Interfaces:**
- Produces: `pub fn parse_cleaner(xml: &str, origin: &'static str, env: &Env) -> Result<Cleaner, String>;` — `Cleaner.origin = Origin::CleanerMl(origin)`, `id` dall'attributo, `label`/`description`/`warning` in italiano come scritti nel file, `running` da `<running type="exe">`, `Opt.unsupported = Some("non supportato: <comando o search>")` per ciò che la sezione "Global Constraints" elenca come non supportato, `default_on = false` sempre. `<var id="x"><value>…</value>…</var>` e `$$x$$` nei percorsi: l'azione si moltiplica per i valori; una variabile d'ambiente non definita in un valore scarta quel valore. Un cleaner con `os` diverso da `windows` → `Err("os non windows")`.
- Consumes: modello e `expand` (Task 1).

- [ ] **Step 1: Test che falliscono** (XML di esempio nel formato dei file dell'utente, come `ai_claude_code.xml`):
  - `reads_label_options_and_warning`
  - `running_processes_are_collected` (`claude.exe`)
  - `search_kinds_map_to_actions` (`walk.all` su cartella, `walk.files` + `regex` `\.jsonl($|\.)`, `file`, `glob`)
  - `var_with_several_values_multiplies_the_actions` e `var_value_with_undefined_env_is_dropped`
  - `unsupported_commands_are_flagged_not_dropped` (`sqlite.vacuum` → opzione con `unsupported`)
  - `non_windows_cleaner_is_rejected`
  - `everything_is_off_by_default`
- [ ] **Step 2:** FAIL. **Step 3:** implementare con `roxmltree`. **Step 4:** verde.
- [ ] **Step 5:** Commit `feat(rules): lettura di CleanerML`.

### Task 4: aggiornamento di Winapp2 all'avvio

Stima: 45-60 min.

**Files:**
- Create: `crates/pc-rules/src/updater.rs`
- Modify: `crates/pc-rules/src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub trait Fetch: Send + Sync { fn get(&self, url: &str, etag: Option<&str>, timeout: Duration) -> Result<Fetched, String>; }
  pub enum Fetched { NotModified, Body { bytes: Vec<u8>, etag: Option<String> } }
  pub struct CurlFetch;                                       // curl.exe di Windows, senza finestra, User-Agent PulisciPC
  pub enum UpdateOutcome { Updated { version: String }, UpToDate, KeptOld(String), NoCopyYet(String) }
  pub fn update(dir: &Path, fetch: &dyn Fetch, url: &str) -> UpdateOutcome;
  pub fn is_valid(bytes: &[u8]) -> Result<String, String>;    // Ok(version)
  pub const WINAPP2_URL: &str = "https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini";
  pub fn spawn_update(dir: PathBuf, on_done: impl FnOnce(UpdateOutcome) + Send + 'static);   // thread, timeout 20 s, non bloccante
  ```
  `is_valid`: UTF-8 (BOM ammesso), riga `; Version: <numero>`, almeno 1000 sezioni `[...]`, almeno una `FileKey`, dimensione tra 200 KB e 20 MB; HTML o 0 byte → `Err`. `update`: legge `etag.txt`; 304 → `UpToDate`; 200 valido → scrive `Winapp2.ini.tmp` poi rinomina su `Winapp2.ini`, salva l'etag; non valido o errore → `KeptOld(motivo)` se esiste una copia, altrimenti `NoCopyYet(motivo)`. `CurlFetch`: `curl.exe -sS -L --max-time 20 -D <headers> -o <corpo> -A PulisciPC [-H "If-None-Match: <etag>"] <url>`, esito 304 dai header; nessun dato dell'utente nei parametri.

- [ ] **Step 1: Test che falliscono** con `FakeFetch` e una `tempdir`:
  - `first_run_downloads_validates_and_stores_etag`
  - `second_run_with_304_keeps_the_file_untouched` (stessi byte e mtime)
  - `changed_etag_replaces_atomically`
  - `html_error_page_is_rejected_and_old_copy_kept`
  - `empty_and_truncated_bodies_are_rejected` (0 byte; solo intestazione; tagliato a 150 KB)
  - `network_error_without_copy_reports_no_copy_yet`
  - `spawn_update_returns_immediately` (un `Fetch` che dorme 2 s: `spawn_update` ritorna in meno di 100 ms)
- [ ] **Step 2:** FAIL. **Step 3:** implementare. **Step 4:** verde.
- [ ] **Step 5 (una sola prova di rete, sola lettura):** in una cartella di scratch, `CurlFetch` contro l'URL vero: primo giro scarica e `is_valid` dà `260915` o successiva; secondo giro `UpToDate` (304). Annotare nel report.
- [ ] **Step 6:** Commit `feat(rules): scaricamento e aggiornamento di Winapp2.ini`.

### Task 5: raccolta di file e chiavi da una regola

Stima: 60-75 min.

**Files:**
- Create: `crates/pc-rules/src/collect.rs`
- Modify: `crates/pc-rules/src/lib.rs`

**Interfaces:**
- Consumes: `Cleaner`, `Opt`, `Action`, `expand`, `protected`.
- Produces:
  ```rust
  pub struct Collected { pub files: Vec<(PathBuf, u64)>, pub dirs_to_remove: Vec<PathBuf>, pub reg: Vec<RegItem>,
                         pub bytes: u64, pub skipped: u64 }
  pub enum RegItem { Key { root: RegRoot, path: String }, Value { root: RegRoot, path: String, value: String } }
  pub fn collect(opt: &Opt, env: &Env, reg_exists: &dyn Fn(&RegItem) -> bool) -> Collected;
  pub fn running(cleaner: &Cleaner, processes: &[String]) -> Vec<String>;   // processi di `running` presenti (nome exe, case-insensitive)
  ```
  `collect`: per ogni `Action::Files`: espande il percorso, scarta i protetti, cammina la cartella (o il file/glob secondo `search`) con i modelli (`*` e `?`, case-insensitive; stessa regola di `cleaner::wildcard`, `src/cleaner.rs:124`), `recurse` per `RECURSE`/`walk.*`; **non segue** link simbolici e giunzioni (un link nella cartella si conta come file di 0 byte e si toglie come link); `remove_self` mette la cartella (se protetta, mai) in `dirs_to_remove`; le `Exclude` di Winapp2 tolgono i file indicati. Le azioni di registro passano da `reg_exists` (iniettato, così i test non toccano il registro).

- [ ] **Step 1: Test che falliscono** (alberi in `tempdir`, `Env` finta):
  - `collects_matching_files_recursively_with_sizes`
  - `patterns_are_case_insensitive_and_semicolon_separated`
  - `exclude_key_keeps_the_named_file`
  - `remove_self_lists_the_folder_after_files`
  - `junction_or_symlink_inside_is_not_followed` (creare un collegamento di cartella con `std::os::windows::fs::symlink_dir`, saltare il test con un messaggio se il privilegio manca)
  - `protected_expansion_collects_nothing_and_counts` (`%UserProfile%` RECURSE)
  - `registry_items_are_filtered_by_existence`
  - `running_matches_exe_names_case_insensitively`
- [ ] **Step 2:** FAIL. **Step 3:** implementare. **Step 4:** verde.
- [ ] **Step 5:** Commit `feat(rules): raccolta di file e chiavi da una regola`.

### Task 6: cleaner inglobati nell'exe

Stima: 40 min.

**Files:**
- Create: `cleaners/*.xml` (11 file copiati da `D:\Software\BleachBit\cleaners`), `crates/pc-rules/src/embedded.rs`
- Modify: `crates/pc-rules/src/lib.rs`, `THIRD_PARTY_NOTICES.md`

**Interfaces:**
- Produces: `pub fn embedded(env: &Env) -> (Vec<Cleaner>, Vec<String>);` — 11 `include_str!("../../../cleaners/<nome>.xml")`, ognuno passato a `parse_cleaner`; un XML che non si legge dà un avviso e non ferma gli altri. Gruppo di tutti: "Sviluppo e AI".
- Nelle copie: tolto ogni prefisso `AGG - `, etichette inglesi tradotte (`Rust Cargo downloads` → "Download di Cargo", `Go build cache` → "Cache di build di Go", `pip package cache` → "Cache dei pacchetti pip", `Cache shader GPU` resta), accenti rimessi nelle frasi (`sara piu` → "sarà più", `possibilita` → "possibilità"); nessuna opzione tolta e nessun percorso cambiato. I commenti XML non citano BleachBit.

- [ ] **Step 1: Test che falliscono:** `all_eleven_embedded_cleaners_parse` (11 cleaner, 0 avvisi); `no_label_has_the_old_prefix_or_missing_accents` (nessuna etichetta inizia con `AGG`; nessuna delle stringhe `sara piu`, `possibilita`, `puo `, `e stato` nei testi); `every_option_is_off_by_default`; `claude_code_cleaner_blocks_while_claude_runs` (con `running(cleaner, ["claude.exe"])` → `["claude.exe"]`).
- [ ] **Step 2:** FAIL. **Step 3:** copiare, correggere, implementare. **Step 4:** verde.
- [ ] **Step 5:** `THIRD_PARTY_NOTICES.md`: sezione Winapp2 (CC-BY-SA-4.0, `https://github.com/MoscaDotTo/Winapp2`, "il file si scarica a runtime, non fa parte dell'eseguibile") e nota sugli XML inglobati.
- [ ] **Step 6:** Commit `feat(rules): gli 11 cleaner CleanerML inglobati nell'exe`.

> **Da qui in poi: iniziare solo dopo il Task 11 del piano 1** (vedi "Sequenza").

### Task 7: categorie dinamiche in Pulizia file

Stima: 90-120 min.

**Files:**
- Create: `src/rules.rs`
- Modify: `src/main.rs`, `src/cleaner.rs`, `Cargo.toml` (`pc-rules = { path = "crates/pc-rules" }`)

**Interfaces:**
- Consumes: `pc_rules::{embedded, winapp2::{parse, to_cleaners}, collect, running, Env, Cleaner, Opt}`, `cleaner::{Found, Cleaned, clean}` e i worker del piano 1, `registry::backup` (`src/registry.rs:366`).
- Produces in `src/rules.rs`:
  ```rust
  pub struct RuleSet { pub cleaners: Vec<Cleaner>, pub winapp2_version: Option<String>, pub status: String, pub warnings: Vec<String> }
  pub fn load(dir: &Path) -> RuleSet;                     // embedded + Winapp2.ini della cartella dati, nasconde le voci Winapp2 che coprono uno strumento inglobato
  pub fn analyze_option(opt: &Opt, env: &Env) -> cleaner::Found;
  pub fn clean_option(found: &cleaner::Found, opt: &Opt, ...) -> cleaner::Cleaned;   // file con i worker di dc-cleaner, directory vuote, chiavi con backup .reg
  ```
  UI: dopo le categorie statiche, nel pannello sinistro, un gruppo "Sviluppo e AI" (gli 11 inglobati: una `CollapsingHeader` per cleaner, una casella per opzione) e un gruppo "Applicazioni" (Winapp2: una casella per voce, raggruppate per `Section` in `CollapsingHeader` chiusi, con campo "Cerca" sopra), entrambi nello stile di `cleaner_tab`. Opzione con `unsupported`: casella disattivata con la spiegazione al passaggio del mouse. Opzione con `running` attivo: disattivata, "Chiudi <processo> per pulire". Opzione con `warning`: alla conferma della pulizia compare l'avviso nella finestra `Modal` esistente. `RuleSet::load` gira in un thread all'avvio e invia `RulesMsg::Loaded(RuleSet)`; finché non arriva, il gruppo non c'è. Gli indici delle categorie statiche restano come prima.

- [ ] **Step 1: Test che falliscono** (`src/rules.rs`, profilo finto in `tempdir`):
  - `embedded_cleaner_shows_only_when_installed_files_exist`
  - `winapp2_entry_covering_an_embedded_tool_is_hidden` (voce Winapp2 con `FileKey` su `%UserProfile%\.cargo\registry` + cleaner `bbag_cargo`: resta solo l'inglobato)
  - `analyze_then_clean_removes_only_collected_files` (tempdir: il file fuori dal modello resta)
  - `registry_rules_make_a_backup_before_deleting` (con registro iniettato)
  - `option_with_active_process_cannot_be_selected`
- [ ] **Step 2:** FAIL. **Step 3:** implementare. **Step 4:** `cargo test -p pulisci-pc` verde; screenshot `--tab cleaner --screenshot shots/check-cleaner.png`: stessa barra, stesse card, i nuovi gruppi dentro il pannello sinistro con lo scorrimento esistente.
- [ ] **Step 5:** Commit `feat(cleaner): categorie dinamiche da CleanerML e Winapp2`.

### Task 8: aggiornamento all'avvio e informazioni

Stima: 45 min.

**Files:**
- Modify: `src/main.rs`, `src/rules.rs`

**Interfaces:**
- Consumes: `pc_rules::updater::{spawn_update, UpdateOutcome}`.
- All'avvio (`App::new`): `rules::load` in thread; subito dopo `spawn_update(dir, ...)`; quando arriva `Updated`, ricarica il `RuleSet`; lo `status` mostra sopra l'elenco una riga piccola `MUTED`: "Winapp2 260915 aggiornato" / "Winapp2 già aggiornato" / "Winapp2 non aggiornato: <motivo>" / "Winapp2 non ancora scaricato". L'uscita dal programma non aspetta il thread.
- "Informazioni": una voce in fondo alla barra laterale (sopra "RAM"), cliccabile, apre una finestra con "PulisciPC", la versione e l'attribuzione: "Le regole di pulizia delle applicazioni vengono da Winapp2 (https://github.com/MoscaDotTo/Winapp2), licenza CC-BY-SA-4.0, scaricate a ogni avvio."

- [ ] **Step 1: Test:** `status_text_for_each_outcome` (quattro esiti → quattro testi esatti); `closing_does_not_wait_for_the_update` (un `Fetch` lento, il thread non è atteso).
- [ ] **Step 2-4:** FAIL → implementare → verde; screenshot della finestra Informazioni.
- [ ] **Step 5:** Commit `feat(app): aggiornamento di Winapp2 all'avvio e finestra Informazioni`.

### Task 9: Parte B, cache globali (XML nuovi)

Stima: 4 ore. **Lotto indipendente: si può affidare a un agente a parte.**

**Files:**
- Create: `cleaners/<strumento>.xml` (uno per strumento), `docs/cleaners-verifica.md`
- Modify: `crates/pc-rules/src/embedded.rs` (elenco), eventualmente `crates/pc-rules/src/cleanerml.rs` per le esigenze elencate nella spec ("Cosa serve al motore CleanerML": più valori per `<var>`, `glob` con `*` nei segmenti intermedi, `<running>`, `<warning>`, cancellazione di file in sola lettura)

**Interfaces:** consumo la tabella "Tabella (Pulizia file → Sviluppo e AI)" della spec del catalogo (da `docs/superpowers/specs/2026-11-10-catalogo-sviluppo-design.md`, "Parte B"): una riga = un XML con le opzioni e i percorsi della riga; variabile d'ambiente prima (`<var>` con valore scartato se non definita), percorso predefinito dopo; avvisi dalla colonna "Note e avviso"; 🔒 = non entra mai in nessuna opzione; "Bloccata con X" = `<running>`; livelli (classe D/P/in uso) restano fuori da questo task (i livelli sono del catalogo 1 e della regola 2).

- [ ] **Step 1:** Per ogni strumento: **verificare i percorsi Windows sulla documentazione ufficiale** (regola 4 della spec); la voce che non si trova documentata si toglie e si annota in `docs/cleaners-verifica.md` con la fonte (URL) o il motivo.
- [ ] **Step 2: Test:** `every_part_b_cleaner_parses_with_zero_warnings`; `credential_files_are_never_targeted` (per ogni XML, nessuna azione espande verso `token`, `stored_tokens`, `credentials.toml`, `config.toml`, `settings.json`, `.kube\config`, `gradle.properties`, `debug.keystore`, `adbkey`, `keras.json`); `undefined_tool_variable_drops_the_default_only_when_set` (con `CARGO_HOME` definita il percorso predefinito non viene usato).
- [ ] **Step 3:** Scrivere gli XML in lotti da 5 strumenti, un commit per lotto (`feat(cleaners): cache di <strumenti>`), dopo ogni lotto `cargo test -p pc-rules`.
- [ ] **Step 4:** Verifica a campione con un profilo finto: costruire in `tempdir` le cartelle di 5 strumenti e controllare che l'analisi le trovi (nessuna prova sul PC dell'utente).

### Task 10: verifica finale della Fase 2

Stima: 45 min.

- [ ] **Step 1:** `cargo test --workspace --exclude mukade` verde (annotare i numeri per crate).
- [ ] **Step 2:** Lettura del Winapp2 vero (Task 2 Step 4) con il tempo e la RAM del processo di prova (picco e dopo `drop`), sola lettura. Atteso: sotto 500 ms, sotto 60 MB di picco.
- [ ] **Step 3:** Con un profilo finto (variabili `USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `TEMP` puntate a una `tempdir` con 5 programmi finti tra Winapp2 e inglobati): avviare `PulisciPC.exe --tab cleaner --screenshot` con quelle variabili e guardare lo screenshot: i gruppi compaiono, le voci non rilevate no. Mai un'analisi o una pulizia sul profilo vero.
- [ ] **Step 4:** Offline: avviare con `curl.exe` non raggiungibile (variabile `PULISCI_OFFLINE=1` letta da `CurlFetch` solo nelle build di prova, oppure blocco del firewall): l'app parte subito e dice "Winapp2 non ancora scaricato" o "non aggiornato".
- [ ] **Step 5:** `grep -ri "bleachbit" src crates cleaners` non trova nulla fuori da `THIRD_PARTY_NOTICES.md`; nessuna etichetta con "AGG -".
- [ ] **Step 6:** Commit `docs: verifica della Fase 2` con i numeri nella sezione "Verifica" della spec.
