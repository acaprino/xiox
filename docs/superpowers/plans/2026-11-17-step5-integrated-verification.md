# Verifica integrata: piano di implementazione (passo 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** provare i casi di accettazione della specifica con i programmi veri (`xiox.exe` e `mukade.exe`) su store e repository finti, chiudere il buco dei riferimenti Git indiretti e verificare osservatore e Cestino reale in modo controllato.

**Architecture:** la maggior parte del passo è verifica: test nuovi che attraversano i pezzi già scritti. Quelli che passano subito sono il risultato atteso; uno che fallisce è un difetto da correggere nel task che lo trova. Il codice di produzione nuovo è uno solo: `crates/xiox-project/src/gitref.rs`, un controllo puro che dice se git può leggere un repository candidato senza seguire `gitdir`, `commondir`, `alternates` o file di configurazione inclusi verso destinazioni vietate. Più una riga di testo nel rendering della CLI.

**Tech Stack:** Rust 2024; `tempfile`, `serde_json`, `windows-sys` già presenti. In `xiox-cli` entrano come dev-dependency `xiox-mukade` (feature `test-support`, già del workspace) e `trash = "5.3.0"` (già usato da `xiox-cleaner`). Nessuna dipendenza nuova nel workspace.

**Spec:** `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`, revisione 3: «Ordine di implementazione», punto 5 («Verifica integrata: casi della tabella, osservatore e prove Windows controllate»), la tabella dei casi di accettazione, «Controlli e copertura» punto 1 (destinazioni indirette di Git) e «Lettura, cache e limiti» (prefisso del transcript).

Il piano precedente è `docs/superpowers/plans/2026-11-16-step4-selection-cleanup.md`.

**Ambito.**

| Parte | Stato |
|---|---|
| Casi 1, 8, 10, 13: già coperti da test che asseriscono il risultato richiesto (mappa sotto) | Nessun task |
| Caso 11 (worktree mai forzato): `xiox-cleaner/src/tests.rs:332` prova che un worktree sporco non viene rimosso | Nessun task |
| Casi 2, 7, 15 attraverso i programmi veri | Task 1 |
| Cestino reale per una voce `danger` confermata; percorsi oltre 260 caratteri | Task 2 |
| Caso 12: transcript riscritto con lo stesso prefisso, dimensione e data | Task 3 |
| Casi 3 e 4: date del filesystem uguali o diverse dalla cronologia | Task 4 |
| Caso 14: riferimenti Git indiretti verso destinazioni vietate | Task 5 (codice nuovo) |
| Casi 5 e 9: i testi non promettono ciò che non è registrato | Task 6 |
| Osservatore | Task 7 |
| Misure sugli store reali, campione nuovo D10–D13 | Fuori: solo su richiesta esplicita dell'utente |
| Caso 6, pagine di una stessa sessione | Fuori: Mukade non espone pagine per i transcript di Claude Code; resta un limite dichiarato nel Task 8 |

**Decisione sull'osservatore.** `xiox observe` continua a girare senza cronologia (`Options.history: false`). La sua specifica dice «l'osservatore non legge le sessioni per decidere chi ha creato cosa», e un'analisi della cronologia può durare fino a 240 s, troppo per un controllo a intervalli. Il Task 7 fissa la scelta con un test e verifica dal vero ciò che l'osservatore mostra delle voci `danger`.

**Decisione sui conflitti fra progetti.** Restano lette solo le sessioni vicine alla cartella (`conflicts::writers_near`). Leggerle tutte, su un PC con 2.271 sessioni collegate, costerebbe circa 10 minuti. Il limite è già nella guida. Il Task 1 prova il blocco con una sessione vicina.

## Global Constraints

- «I test usano repository e store finti sotto `tempfile::tempdir()`. `cargo` gira con `-j 1`; non si avviano build concorrenti. Non si eseguono pulizie su progetti reali.»
- «Le misure su store reali vengono eseguite dall'utente o su sua richiesta esplicita.»
- «Xiox non modifica i dati delle sessioni, non esegue migrazioni o pulizie degli store e non scrive SQL applicativo.»
- «Le date del filesystem non provano creazione, autore o appartenenza a un progetto. Non generano candidati e non confermano né annullano un'associazione ricavata dalla cronologia.»
- «Accesso solo a dischi locali fissi; controllo degli antenati dall'alto prima di attraversarli, senza seguire link/junction o accedere a cartelle cloud. Le destinazioni indirette di Git (`gitdir`, directory comuni, object store alternativi e configurazioni esterne) richiedono la stessa policy prima di interrogare il repository; un riferimento non validabile impedisce quella verifica.»
- «La sola coincidenza di percorso, dimensione o prefisso del transcript non certifica l'attualità.»
- «Un'operazione fallita non è una creazione riuscita. Un esito mancante resta sconosciuto.»
- `AGENTS.md`: nessun test raggiunge il Cestino reale, salvo quelli che svuotano dal Cestino la voce che hanno creato prima di ogni controllo; una cartella `danger` si prova a vuoto, salvo il test controllato del Task 2.
- Ambiente di ogni comando cargo, d'ora in poi `$ENV`: `HOME='D:\Software\Shinatama\data\agents\claude_code\home' USERPROFILE=<lo stesso, 'C:\Users\alfio' per xiox-cleaner> CARGO_INCREMENTAL=0 TEMP='D:\Projects\xiox\target\tmp' TMP='D:\Projects\xiox\target\tmp'`. Mai `cargo fmt`.
- Codice e testi in inglese, commit in italiano con le righe `Co-Authored-By` e `Claude-Session`.

## Review Focus

1. Un transcript riscritto con la stessa dimensione e la stessa data: la pulizia non deve riusare le vecchie chiamate. Test: Task 3, riga «same size and date».
2. Un `.git` di una cartella candidata che punta a una condivisione di rete: l'analisi non deve toccare la rete. Test: Task 5, riga UNC di `git_may_read_table` e `a_forbidden_gitdir_is_never_followed`.
3. Il processo figlio dei test integrati legge gli store veri perché una variabile di un harness è ereditata. Test: Task 1, `the_session_world_reads_only_its_own_store`.
4. Un test sul Cestino reale che fallisce prima di svuotarlo lascia la voce nel Cestino. Test: Task 2, la voce si svuota prima di ogni `assert`.
5. Un `include.path` con `~` o relativo risolto male lascia passare una destinazione vietata. Test: Task 5, righe `~/` e relativa.

---

### Task 1: Il mondo delle sessioni dal vero (CLI + Mukade)

**Files:**
- Modify: `crates/xiox-cli/Cargo.toml` (dev-dependency `xiox-mukade = { path = "../xiox-mukade", features = ["test-support"] }`)
- Test: `crates/xiox-cli/tests/cli.rs` (sezione nuova `// ── the session history, end to end ──`)

**Interfaces:**
- Consumes: `xiox_mukade::test_support::write_claude_session(root: &Path, id: &str, cwd: &str, command: &str)`; `xiox()` e `git_command()` di `cli.rs`.
- Produces: `struct SessionWorld { _t: TempDir, base: PathBuf, home: PathBuf, project: PathBuf }` con `fn new() -> SessionWorld` (progetto git `<base>\work\phazly`, home `<base>\home`), `fn session(&self, id: &str, cwd: &Path, command: &str)` (scrive in `<home>\.claude`), `fn xiox(&self) -> Command`. Il `Command` ha `USERPROFILE` e `HOME` = home, `LOCALAPPDATA` = `<home>\AppData\Local`, `TEMP` e `TMP` = `<home>\AppData\Local\Temp` (create), `XIOX_SEARCH_ROOTS` = base, `GIT_CEILING_DIRECTORIES` = genitore di base. Toglie `XIOX_NO_SESSIONS`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_DB`, `PI_CODING_AGENT_DIR`, `XIOX_OPENCODE_EXE` e `XIOX_MUKADE_EXE`. Usato dai Task 2 e 6.

- [ ] **Step 1: Write the failing tests**
  - `the_session_world_reads_only_its_own_store`: una sessione `s1` nel progetto che fa `mkdir "<base>\runs\bench-1"`; `analyze --json`. `history.sources` ha `claude_code` con 1 sessione elencata e 0 per gli altri harness; `history.sessionsRead == 1`.
  - `analyze_lists_what_a_session_made_end_to_end`: con la stessa sessione, la voce `runs\bench-1` ha `category == "scratch"`, `level == "danger"`, `risks` contiene `"no-git"`, `provenance.sessions[0].harness == "claude_code"`, `provenance.sessions[0].operations[0].entry` e `.call` non nulli. Il testo di `analyze` contiene `danger: explicit confirmation required: no git repository of its own`.
  - `clean_takes_a_scratch_item_only_with_its_confirmation_end_to_end`: `clean <p> --include <bench> --yes` dà exit 2, stderr contiene `add --confirm-danger "<bench>"`, la cartella resta. `clean <p> --include <bench> --confirm-danger <bench> --dry-run --json` dà exit 0, `outcome.items[0].outcome.type == "removed"`, `.method == "recycle-bin"`, `chosen.confirmed[0].risks` uguale ai `risks` della voce, e la cartella resta.
  - `a_folder_two_projects_wrote_in_is_shown_blocked_end_to_end`: `s1` nel progetto fa `mkdir "<base>\shared"` e `echo x > "<base>\shared\a.txt"`; `s2` con `cwd` `<base>\shared` fa `echo y > "<base>\shared\b.txt"`. In `analyze --json` la voce `shared` ha `blocked == ["other-project-writes"]` e `provenance.otherProjects[0].id == "s2"`. Il testo mostra la riga `blocked` con `sessions of another project wrote here`.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-cli --test cli session_world end_to_end`. Expected: FAIL nella compilazione (`SessionWorld` non esiste). Dopo la Step 3, ogni test che fallisce ancora è un difetto vero: se ne trova la causa e la si corregge in questo task.
- [ ] **Step 3: Implement `SessionWorld`** in `cli.rs` come da Interfaces; `mukade.exe` si trova da solo (`vendor/mukade`).
- [ ] **Step 4: Run** lo stesso comando, poi `$ENV cargo test -j 1 -p xiox-cli`. Expected: PASS.
- [ ] **Step 5: Commit** `test(xiox-cli): cronologia dal vero con xiox e mukade su uno store finto`.

### Task 2: Prove Windows controllate: Cestino reale e percorsi lunghi

**Files:**
- Modify: `crates/xiox-cli/Cargo.toml` (dev-dependency `trash = "5.3.0"`)
- Test: `crates/xiox-cli/tests/cli.rs`, `crates/xiox-cleaner/src/tests.rs`
- Modify: `AGENTS.md` (nota sui test della pulizia)

**Interfaces:**
- Consumes: `SessionWorld` (Task 1); `trash::os_limited::{list, purge_all}`.
- Produces: nessuna API.

- [ ] **Step 1: Write the tests**
  - `a_confirmed_danger_folder_goes_to_the_real_recycle_bin` (`cli.rs`): la sessione del Task 1; `clean <p> --include <bench> --confirm-danger <bench> --yes --json` in modalità Cestino. Subito dopo l'uscita il test prende da `trash::os_limited::list()` le voci con `original_path() == bench` e le svuota con `purge_all`, prima di ogni `assert`. Poi: exit 0, una sola voce trovata, `bench` non esiste più, `outcome.recycledBytes > 0`.
  - `permanent_removes_a_tree_deeper_than_max_path` (`xiox-cleaner/src/tests.rs`): sotto `tmp`, una catena di cartelle `d0\d1\…` creata con il prefisso `\\?\` fino a oltre 300 caratteri, con un file da 1.000 byte in fondo; `run(RemoveMode::Permanent, item(&t, ArtifactKind::RustTarget), false)` dà `Removed` con `bytes_freed >= 1000` e la cartella non esiste più.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-cli --test cli real_recycle_bin` e `$ENV cargo test -j 1 -p xiox-cleaner deeper_than_max_path` (per `xiox-cleaner` `USERPROFILE='C:\Users\alfio'`). Expected: PASS subito, perché verificano comportamento esistente; un FAIL è il difetto da correggere qui. Il test del Cestino trova e svuota la sua voce: dopo l'esecuzione nel Cestino non resta nulla da `target\tmp`.
- [ ] **Step 3: Update `AGENTS.md`**: la nota sui test della pulizia nomina il secondo test che usa il Cestino reale e la regola «svuotare prima di ogni controllo».
- [ ] **Step 4: Commit** `test: Cestino reale per una voce danger confermata e percorsi oltre MAX_PATH`.

### Task 3: Transcript riscritto con lo stesso prefisso

**Files:**
- Test: `crates/xiox-project/src/tests/history/analyze.rs`
- Modify: `crates/xiox-project/src/tests/clean.rs:50,70` (`FakeBin`, `FakeBin::new` e `run_with` diventano `pub(super)`)

**Interfaces:**
- Consumes: `World` di `analyze.rs`; `crate::tests::clean::run_with(scan, chosen, mode, dry, env, bin, fresh) -> CleanOutcome`; `crate::tests::clean::FakeBin`.
- Produces: `World::env(&self) -> Env` (estratto da `World::scan`, così analisi e pulizia vedono gli stessi store).

- [ ] **Step 1: Write the failing test** `a_rewritten_transcript_is_read_again_not_reused`. Tabella `(nome, comando nuovo, mtime originale rimesso, esito)`; per ogni riga `World::new()`, analisi, `select` di `bench-1` con `--confirm-danger`, poi la sessione riscritta con lo stesso id e lo stesso primo record:
  - «another folder»: `mkdir "<base>\runs\bench-2"`, mtime nuovo → `Skipped { reason: "no longer there" }`;
  - «same size and date»: `mkdir "<base>\runs\bench-X"`, con la stessa lunghezza del comando originale, e mtime originale rimesso con `File::set_modified` → `Skipped { reason: "no longer there" }`;
  - «only a write is left»: `echo x > "<bench-1>\a.txt"` → `Skipped { reason: "its risks changed since it was confirmed: choose it again" }`.
  La pulizia gira a vuoto (`dry: true`) con `run_with(…, &World::env(), &FakeBin::new(base.join("bin")), &|_| Vec::new())`.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-project --lib rewritten_transcript`. Expected: FAIL nella compilazione (`World::env`). Dopo la Step 3 un FAIL della riga «same size and date» è un difetto vero: le chiamate non vengono rilette. La correzione va alla radice, nel lettore delle chiamate di `xiox-mukade`, e resta in questo task.
- [ ] **Step 3: Implement** `World::env`, e la visibilità `pub(super)` di `FakeBin`, `FakeBin::new` e `run_with`.
- [ ] **Step 4: Run** lo stesso comando. Expected: PASS.
- [ ] **Step 5: Commit** `test(xiox-project): un transcript riscritto si rilegge, mai riusato per prefisso, dimensione o data`.

### Task 4: Le date del filesystem non attribuiscono e non annullano

**Files:**
- Modify: `crates/xiox-project/src/tests/fixtures.rs` (`set_times`), `crates/xiox-project/src/tests/history/risk.rs:42` (`set_mtime` usa `set_times`)
- Test: `crates/xiox-project/src/tests/history/analyze.rs`

**Interfaces:**
- Produces: `pub fn set_times(dir: &Path, created_ms: Option<i64>, modified_ms: i64)` in `fixtures.rs` (`FILE_FLAG_BACKUP_SEMANTICS`, `FileTimesExt::set_created`).

- [ ] **Step 1: Write the test** `file_dates_neither_attribute_nor_cancel`, con `World` (sessione del 2026-11-01T10:00:01Z che crea `runs\bench-1`). Tabella `(nome, cartella, creata, modificata, attesa)`:
  - «a neighbour made at the session's second»: `runs\neighbour`, entrambe le date al ms della chiamata → nessuna voce per `neighbour`;
  - «created a year before»: `bench-1`, entrambe un anno prima → voce `Scratch`, `Strength::Created`, una sessione;
  - «changed after the session»: `bench-1`, creata alla chiamata, modificata due ore dopo → voce `Scratch`, `Strength::Created`, una sessione.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-project --lib file_dates_neither`. Expected: FAIL nella compilazione (`set_times`), poi PASS. Un FAIL dopo l'helper è un difetto da correggere qui.
- [ ] **Step 3: Implement** `set_times`; `set_mtime` diventa `set_times(dir, None, ms)`.
- [ ] **Step 4: Run** lo stesso comando e `$ENV cargo test -j 1 -p xiox-project --lib risks_table`. Expected: PASS.
- [ ] **Step 5: Commit** `test(xiox-project): le date del filesystem non attribuiscono né annullano una cartella`.

### Task 5: Riferimenti Git indiretti

**Files:**
- Create: `crates/xiox-project/src/gitref.rs`
- Modify: `crates/xiox-project/src/lib.rs` (`mod gitref;`), `crates/xiox-project/src/search.rs:104-208` (`classify`, `named_blocks`), `crates/xiox-project/src/clean/mod.rs` (`check_contents`, i repository annidati)
- Test: `crates/xiox-project/src/tests/gitref.rs` (con `mod gitref;` in `tests/mod.rs`), `crates/xiox-project/src/tests/history/scratch.rs`
- Modify: `docs/xiox-cli.md` (sezione Safety)

**Interfaces:**
- Consumes: `history::scope::{local_dir, unlinked_chain, FixedDrives}`, `xiox_scanner::is_cloud_dir`.
- Produces: `pub(crate) fn git_may_read(dir: &Path) -> bool` in `gitref.rs`. Legge solo file, mai git. Il `gitdir` è la cartella `dir\.git`, oppure la riga `gitdir: <p>` del file `dir\.git`, relativa a `dir`. Poi raccoglie `<gitdir>\commondir` (una riga, relativa a gitdir), `<common>\objects\info\alternates` (una destinazione per riga, salvo vuote e `#`, relative a `objects`) e le righe `path = <p>` nelle sezioni `[include]` e `[includeIf …]` di `<common>\config` e `<gitdir>\config.worktree`. Queste ultime sono relative alla cartella del file, e `~/` vale la home di `UserDirs::from_env`. `true` solo se ogni destinazione passa `local_dir`, sta su un disco fisso, passa `unlinked_chain` e non è una cartella cloud. Un `.git` che è un link, un file senza `gitdir:` e ogni errore di lettura danno `false`.

- [ ] **Step 1: Write the failing tests**
  - `git_may_read_table` (`tests/gitref.rs`), una riga per caso, su cartelle sotto `tmp`:
    - una `.git` normale → `true`;
    - un file `.git` con `gitdir: ..\real\.git` relativo e locale → `true`;
    - `gitdir: \\127.0.0.1\share\x.git` → `false`;
    - `gitdir:` attraverso una junction → `false` (riga saltata se la junction non si crea);
    - un file `.git` senza `gitdir:` → `false`;
    - `commondir` con `\\srv\s\c` → `false`;
    - `alternates` con una riga locale → `true`; con una riga `\\srv\s\objects` → `false`;
    - `[include] path = \\srv\s\x.cfg` → `false`;
    - `[includeIf "gitdir:x"] path = ~/x.cfg`, con il file nella home finta → `true`;
    - `path = ..\..\x.cfg` relativo e locale → `true`.
  - `a_forbidden_gitdir_is_never_followed` (`tests/history/scratch.rs`): una cartella fatta dalla sessione contiene un file `.git` con `gitdir:` attraverso una junction al `.git` di un repository vero senza remote. Seguendola, git direbbe `UnsavedWork`. La voce ha `UnsavedWorkUnknown` fra i blocchi e non `UnsavedWork`. Riga saltata se la junction non si crea.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-project --lib git_may_read forbidden_gitdir`. Expected: FAIL (`gitref` non esiste).
- [ ] **Step 3: Implement `git_may_read`** e usarlo prima di ogni chiamata git su una cartella che non è il progetto né un suo worktree:
  - `classify`: un candidato con `git_may_read == false` non è una copia; resta `Named` se il nome corrisponde;
  - `named_blocks`: il repository stesso e ognuno di quelli annidati, se `false`, danno `UnsavedWorkUnknown` senza chiamare git;
  - `check_contents`: lo stesso alla rimozione.
- [ ] **Step 4: Run** lo stesso comando, poi `$ENV cargo test -j 1 -p xiox-project`. Expected: PASS.
- [ ] **Step 5: Docs**: in `docs/xiox-cli.md`, «Safety», una riga: un repository candidato il cui `.git`, `commondir`, `alternates` o file inclusi puntano fuori dai dischi locali fissi o attraverso un link non viene interrogato ed è bloccato (`could not be checked for unsaved work`).
- [ ] **Step 6: Commit** `feat(xiox-project): git non segue riferimenti indiretti verso destinazioni vietate`.

### Task 6: I testi non promettono ciò che non è registrato

**Files:**
- Modify: `crates/xiox-cli/src/render.rs:121-126` (motivo di `SessionFolder`)
- Test: `crates/xiox-cli/src/render.rs` (modulo dei test)

**Interfaces:**
- Consumes: `Provenance.sessions[].operations[]` (`strength: Option<Strength>`, `outcome: CallOutcome`).
- Produces: per `Strength::Created`, il testo `made by a session (its result is not recorded)` quando nessuna operazione con `strength == Some(Created)` ha `outcome == Succeeded`; altrimenti `made by a session`, invariato.

- [ ] **Step 1: Write the failing test** `scratch_reasons_never_claim_what_was_not_recorded`. Tabella `(forza, esiti delle operazioni, testo atteso)`:
  - `Created`, `[Succeeded]` → `made by a session`;
  - `Created`, `[Unknown]` → `made by a session (its result is not recorded)`;
  - `Written`, `[Succeeded]` → `files written here by a session`.
  Per ogni riga, e per `danger_scan()`, il testo della scansione non contiene `inactive`, `not active`, `no longer used` né `not in use`.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-cli scratch_reasons_never`. Expected: FAIL sulla riga `[Unknown]`.
- [ ] **Step 3: Implement** il testo nel motivo di `SessionFolder`.
- [ ] **Step 4: Run** `$ENV cargo test -j 1 -p xiox-cli`. Expected: PASS.
- [ ] **Step 5: Docs**: la tabella dei motivi di `scratch` in `docs/xiox-cli.md` aggiunge il testo nuovo.
- [ ] **Step 6: Commit** `fix(xiox-cli): una creazione senza esito registrato non è detta riuscita`.

### Task 7: Osservatore

**Files:**
- Test: `crates/xiox-cli/src/observe.rs:1492` (`the_analysis_of_the_observer_never_uses_the_admin_search`), `crates/xiox-cli/tests/cli.rs` (sezione observe)
- Modify: `docs/xiox-observe.md` (una riga nei limiti)

**Interfaces:**
- Consumes: `Observer::spawn`, `Observer::until` di `cli.rs`; `git_command`.

- [ ] **Step 1: Write the tests**
  - In `the_analysis_of_the_observer_never_uses_the_admin_search`, `assert!(!o.history, "{o:?}")`, con il test rinominato `the_analysis_of_the_observer_never_uses_the_admin_search_or_the_history`.
  - `observe_shows_a_worktree_made_during_the_run_as_danger`: un progetto git sotto tmp e `Observer::spawn(&project, &["--worktree-threshold", "1"])`. Dopo `started`, `git worktree add <base>\phazly-spike -b spike`. L'evento `changed` ha in `added` la voce del worktree con `level == "danger"`, `risks == ["recent-activity"]` e `provenance == null`. L'evento `attention` con `kind == "worktrees"` ha `cleanCommand == null`.
- [ ] **Step 2: Run** `$ENV cargo test -j 1 -p xiox-cli observer`. Expected: PASS subito, perché verificano il comportamento scelto; un FAIL è il difetto da correggere qui.
- [ ] **Step 3: Docs**: in `docs/xiox-observe.md`, nei limiti, `xiox observe` non legge la cronologia delle sessioni: non mostra voci `scratch` né provenienza. Per vederle serve `xiox analyze`.
- [ ] **Step 4: Commit** `test(xiox-cli): l'osservatore senza cronologia e i worktree danger dal vero`.

### Task 8: Stato e limiti

**Files:**
- Modify: `AGENTS.md`, `docs/xiox-cli.md` (Limits)

- [ ] **Step 1:** in `AGENTS.md`, la riga di stato del passo 5:
  - i casi della tabella e dove sono provati;
  - il mondo `SessionWorld`;
  - `gitref.rs`;
  - l'osservatore senza cronologia;
  - le misure reali non fatte.
- [ ] **Step 2:** in `docs/xiox-cli.md`, «Limits»: le pagine di una stessa sessione non sono distinte nei riferimenti, perché Mukade non le espone per Claude Code.
- [ ] **Step 3: Run** `node scripts/italian-check.mjs crates/xiox-cli/src crates/xiox-project/src crates/xiox-cleaner/src docs/xiox-cli.md docs/xiox-observe.md`, poi tutte le suite: `xiox-project`, `xiox-cli`, `xiox-observer`, `xiox-git` e `xiox-cleaner`, una alla volta. Expected: nessun riscontro e PASS.
- [ ] **Step 4: Commit** `docs: stato del passo 5 e limiti`.

## Mappa dei casi di accettazione

| Caso | Dove è provato |
|---|---|
| 1 Copia profonda nel `cwd` | `tests/history/scope.rs:51`, `:88`, `:131`; `tests/clean.rs:1602` |
| 2 Tool crea una cartella fuori | `tests/history/scratch.rs:119`, `tests/history/analyze.rs:50`; Task 1 |
| 3 Cartella coeva senza evidenze | `tests/history/scratch.rs:153`; Task 4 |
| 4 Timestamp NTFS diverso | `tests/history/risk.rs:84`; Task 4 |
| 5 Scrittura, creazione fallita | `tests/history/scratch.rs:129`, `:135`, `folders.rs:34`, `ledger.rs:107`; Task 6 |
| 6 `cwd`, percorsi relativi, sessioni figlie | `ledger.rs:74`, `paths.rs:87`, `:112`, `:146`, `queue.rs:80`, `:261`; pagine: limite (Task 8) |
| 7 Agente conferma alla CLI | `run.rs:748`, `cli.rs:460`; Task 1 |
| 8 Solo `--yes` o un altro percorso | `run.rs:722`, `cli.rs:466`, `tests/clean.rs:1423`, `:1448` |
| 9 Attività recente, vitalità sconosciuta | `danger.rs:76`, `risk.rs:151`, `:168`; Task 6 |
| 10 Contenitore con discendente | `tests/clean.rs:1399`, `:1471`, `:1862`, `:1896`, `danger.rs:165` |
| 11 Worktree `danger` confermato | `tests/clean.rs:2026`, `:1508`; `xiox-cleaner/src/tests.rs:332` |
| 12 Sorgente riscritta | `tests/clean.rs:1958`, `:1914`; Task 3 |
| 13 Lettura enorme, parziale o fallita | `queue.rs:134`, `:169`, `:182`, `scratch.rs:369`, `folders.rs:46`; `xiox-mukade` `calls.rs:611`, `bounded.rs:169` |
| 14 UNC, junction, Git esterno | `scope.rs:16`, `scratch.rs:252`, `:276`, `scope.rs:224`, `tests/clean.rs:1570`; Task 5 |
| 15 Cartella condivisa da due progetti | `conflicts.rs:67`, `:114`, `tests/clean.rs:1914`; Task 1 |
