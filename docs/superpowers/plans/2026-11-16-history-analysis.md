# Analisi della cronologia in `xiox analyze`: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** rendere visibile in `xiox analyze` ciò che la libreria `provenance` già sa leggere. Le copie si trovano anche dai `cwd` delle sessioni (Parte A). La cronologia delle sessioni del progetto si legge tramite Mukade e produce la categoria `scratch`. Provenienza, rischio e copertura entrano nel risultato.

**Architecture:** un modulo `history` in `crates/xiox-project`, fatto di stadi piccoli.

1. Il listing non aggregato dei descrittori di sessione arriva da `xiox-mukade`.
2. `scope` filtra i `cwd` lessicalmente e li porta alle radici di repository (Parte A).
3. `queue` legge una sessione alla volta, entro un budget, tramite un trait `CallSource`. I test lo iniettano finto.
4. `folders` riunisce i `Ledger` per cartella in una `Provenance`.
5. `scratch` applica la policy dei candidati e `risk` calcola rischi e attività.

`owned::analyze_with` chiama gli stadi fra la ricerca delle copie e l'ordinamento. Senza sorgente di cronologia (test, `XIOX_NO_SESSIONS`, `--no-history`) l'analisi resta quella di oggi.

**Tech Stack:** Rust 2024; `serde`, `serde_json`, `anyhow` e `tempfile`, già presenti. Nessuna dipendenza nuova. Mukade è l'eseguibile precompilato in `vendor/mukade/`.

**Spec:** `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`, revisione 3. Le sezioni sono:

- «Parte A: candidati dai percorsi di lavoro»;
- «Parte B: operazioni della cronologia»;
- «Evidenze di sessione»;
- «La categoria `scratch`»;
- «Rischio, attività e conferma», solo il calcolo del rischio;
- «Controlli e copertura», punti 1, 5 e 6;
- «Lettura, cache e limiti»;
- la sonda `session_probe` di «Verifiche prima e durante la costruzione».

**Ambito rispetto all'«Ordine di implementazione».**

| Parte della specifica | Stato |
|---|---|
| Passo 1: `ToolCall`, `CallReader::dump` (una copia di OpenCode per lettore, timeout reale di 120 s, tetto su stdout) | Fatto, in `main` |
| Passo 1: descrittori non aggregati esposti da `xiox-mukade`; sonda `session_probe` | Questo piano, Task 1 e 9 |
| Passo 2, Parte A: `cwd` → radici → copie, deduplicazione col walk e con l'indice MFT, provenienza per sessione | Questo piano, Task 3. Oggi `search.rs` non usa le sessioni; `project_roots_from_sessions` serve solo alla scansione del motore |
| Passo 3: lessico, struttura, riconoscitori, script, patch, resolver, classificatore, `Ledger` | Fatto (`crates/xiox-project/src/provenance`) |
| Passo 3: coda delle sessioni del progetto, budget, provenienza nel risultato, `scratch`, rischio, copertura | Questo piano, Task 2 e 4–8 |
| Passo 4: livello `danger` e il suo testo, `--confirm-danger`, UI, propagazione ai contenitori, riconvalida delle evidenze prima della rimozione, rifiuto di `--permanent`, eccezione dei worktree, conflitti fra progetti (punto 4 dei controlli), policy Git del sottoalbero per i candidati dentro il progetto, schema 2 | Fuori. Di questo passo serve solo una parte minima: `select` rifiuta ogni voce `scratch` (Task 2), così nessuna cartella trovata nella cronologia si rimuove prima delle conferme |
| Passo 5: verifica integrata, osservatore, prove Windows controllate | Fuori. L'osservatore e MCP continuano a girare senza cronologia (Task 7) |

**Decisione sullo schema JSON: resta `"schema": 1`.** In questo passo il JSON solo si allarga:

- nuovi campi: `items[].provenance`, `history`;
- nuovi valori: la categoria `scratch`, `why.type` `session-folder`, le note `history-partial` e `history-not-read`.

Nessun campo esistente cambia nome, tipo o significato. Il precedente è nella specifica dell'osservatore: «stesso `schema`, con i campi in più». La specifica assegna l'incremento al passo 4, che cambia il significato dei livelli (`danger`) e della selezione (conferme). Un solo incremento per l'intera funzione evita ai consumatori due migrazioni. La guida dichiara la regola (Task 8): dentro uno stesso schema possono comparire campi e valori nuovi; un significato cambiato incrementa lo schema.

## Global Constraints

Dalla specifica, alla lettera:

- «Le date del filesystem non provano creazione, autore o appartenenza a un progetto. Non generano candidati e non confermano né annullano un'associazione ricavata dalla cronologia.»
- «Si elencano anche candidati senza Git e con evidenza debole, rendendo esplicita l'incertezza.»
- «Non si costruisce un secondo reader degli store nativi in Xiox.» «Non si trasferiscono a Mukade categorie, rischio o logiche di pulizia.»
- «Uno store assente è diverso da uno store illeggibile; un descrittore può sopravvivere al suo transcript.»
- «Filtrare i percorsi prima di accedervi.»
- «La provenienza conserva le sessioni distinte e l'ultima attività osservata. Il conteggio non si ricostruisce dai soli aggregati. Le radici non pertinenti al progetto non entrano nel suo elenco; fonti saltate o limiti di copertura sono invece riportati.»
- «il `cwd` iniziale non attribuisce automaticamente tutta la sessione. Se il contesto necessario a risolvere un percorso relativo manca, il percorso resta irrisolto.»
- «Il raggiungimento di un limite è visibile.» «non si dichiara copertura universale.»
- «Un'operazione fallita non è una creazione riuscita. Un esito mancante resta sconosciuto.»
- «per proporre `scratch` deve esserci un riferimento alla sua funzione di cartella di lavoro, copia, prova o destinazione di artefatti nella cronologia. Il semplice elenco di percorsi in un prompt o nell'output di `ls` non basta.»
- «Non esegue comandi letti dalla cronologia.» «Nessun percorso irrisolto diventa candidato.»
- «una cartella solo "usata" non viene descritta come creata dalla sessione.»
- «Non si sale automaticamente all'antenato più alto.»
- «Non si propone come `scratch` una radice di disco, una cartella protetta o larga, il progetto stesso, un suo worktree o un loro contenitore.»
- «`--no-drives` conserva il limite della ricerca esterna al genitore del progetto anche per i candidati ricavati dalle sessioni. I percorsi esterni esclusi sono conteggiati senza accedervi.»
- «Si legge una sessione alla volta e la si libera dopo l'estrazione.» «un timeout solo nominale non basta.»
- «I candidati sono ordinati per attività osservata, con ordinamento stabile per quelli senza data. Ogni troncamento è riportato.»
- «Cache ed elenco standard contengono evidenze sintetiche e riferimenti, non copie delle chat, script, comandi o output completi.» «I testi degli errori non devono riversare contenuto delle sessioni.»
- «Sono `danger` le voci senza Git, quelle con evidenza debole, quelle con copertura insufficiente per valutare un rischio rilevante e quelle create/modificate o usate da sessioni nelle ultime 24 ore. Le date del filesystem possono contribuire a questo indicatore di recenza, mai alla provenienza.» In questo piano diventa la lista strutturata `risks`; il livello `danger` è del passo 4.
- «I test usano repository e store finti sotto `tempfile::tempdir()`. `cargo` gira con `-j 1`; non si avviano build concorrenti. Non si eseguono pulizie su progetti reali.»
- «Le misure su store reali vengono eseguite dall'utente o su sua richiesta esplicita.»

Da `AGENTS.md` e dall'ambiente:

- Codice organico, pulito ed elegante, con un'architettura studiata. Si usano stadi puri, piccoli tipi di valore, strategy e registry dove migliorano la lettura. Nessuna astrazione senza un secondo uso o un beneficio di lettura evidente. Tutto ciò che entra in un crate ha moduli progettati e test guidati dai dati.
- I comandi reali delle sessioni non entrano mai nel repository. Fixture e store dei test sono solo sintetici. Nessun test legge gli store reali di questo PC.
- Il database di OpenCode si legge da una copia, mai dall'originale. Va sempre attraverso `MukadeSessions::call_reader()`.
- Le divergenze D1–D15 del port non si toccano. Questo piano non cambia `crates/xiox-project/src/provenance/` se non dove un task lo dice.
- Inglese in ogni stringa visibile a utente o programma; `scripts/italian-check.mjs` resta pulito.
- Ambiente di ogni comando cargo: `CARGO_INCREMENTAL=0`, `TEMP` e `TMP` uguali a `D:\Projects\xiox\target\tmp`, perché il `TEMP` ereditato ha il prefisso `\\?\` e rompe il link. Sempre `-j 1`, un solo processo cargo alla volta, mai `cargo fmt`. Forma usata nei passi:
  `CARGO_INCREMENTAL=0 TEMP='D:\Projects\xiox\target\tmp' TMP='D:\Projects\xiox\target\tmp' cargo test -j 1 -p <crate> <filtro>`.
  Nei passi si scrive solo `cargo test -j 1 -p <crate> <filtro>`, ma l'ambiente vale sempre.
- Comportamento invariato dove non è detto altro: i test esistenti di `analyze`, `clean`, `observe`, MCP, `MukadeSessions::list`/`aggregate` e della sonda `shell_probe` fanno da guardia. Ogni task finisce con `cargo test -j 1 -p xiox-project`, e con `-p xiox-cli` o `-p xiox-mukade` quando li tocca, tutti verdi.
- Commit: solo i file del task, messaggio in italiano, che termina con
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Lo stesso percorso scritto in forme diverse.** Una copia trovata dal walk e citata da un `cwd` come `\\?\C:\…`, con le barre `/` o con altre maiuscole è una voce sola, con la provenienza. Test: `cwd_copy_found_by_walk_is_listed_once` (Task 3).
2. **Una lettura che si blocca o esplode.** Un `dump` in timeout, oltre il tetto o fallito è contato per tipo. Le altre sessioni si leggono. La nota `history-partial` compare e nessun testo di Mukade entra nel risultato. Test: `a_failed_read_is_counted_and_the_rest_is_read` (Task 4).
3. **La stessa id in due store.** Restano due sessioni distinte nella coda, nel conteggio e nella provenienza. Test: `same_id_in_two_stores_is_two_sessions` (Task 4).
4. **Un bersaglio della cronologia che oggi è una junction o è sparito.** Non viene mai attraversato né elencato; resta contato fra gli esclusi. Test: `junction_or_missing_folder_is_never_entered` (Task 5).
5. **Cartelle larghe come destinazione.** `mkdir ~\x`, un file scritto in `D:\`, un `cd` nel genitore del progetto: home, radice, `%TEMP%` stesso e il genitore del progetto non diventano mai `scratch`. Un figlio di `%TEMP%` sì. Test: `broad_and_protected_folders_are_never_scratch` (Task 5).

---

## File Structure

| File | Responsabilità |
|---|---|
| `crates/xiox-mukade/src/descriptors.rs` | `Descriptor`, `MukadeSessions::descriptors`, `parse_time_ms` |
| `crates/xiox-mukade/src/calls.rs` | `DumpError`: l'esito tipato di `CallReader::dump` |
| `crates/xiox-app/src/lib.rs` | `load_descriptors`, accanto a `load_sessions` |
| `crates/xiox-scanner/src/sessions.rs` | Esporta `is_broad_dir` e `is_cloud_dir`, le guardie già scritte |
| `crates/xiox-project/src/history/mod.rs` | `HistoryInput`, `UserDirs`, trait `CallSource`, `ReadFailure`, `History`: lo stadio chiamato da `owned` |
| `crates/xiox-project/src/history/report.rs` | Tipi del JSON: `Provenance`, `SessionEvidence`, `Operation`, `Risk`, `Activity`, `HistoryReport`, `SourceReport`, `Limit`, `Exclusion` |
| `crates/xiox-project/src/history/scope.rs` | Parte A: filtro lessicale e di ambito dei `cwd`, poi radici con `project_roots_from_sessions` |
| `crates/xiox-project/src/history/folders.rs` | `Folders`: evidenze per cartella da listing e `Ledger`, conversione in `Provenance` |
| `crates/xiox-project/src/history/queue.rs` | `Queue` e `Budget`: quali sessioni leggere, in che ordine, fino a quando |
| `crates/xiox-project/src/history/scratch.rs` | Policy dei candidati `scratch` |
| `crates/xiox-project/src/history/risk.rs` | `risks`, `activity`, chiave d'ordine per attività |
| `crates/xiox-project/src/history/mukade.rs` | `MukadeCalls`: la `CallSource` reale sopra `CallReader` |
| `crates/xiox-project/src/{lib.rs,owned.rs,search.rs,clean.rs}` | `Category::Scratch`, `Why::SessionFolder`, `Item.provenance`, `ProjectScan.history`, `Options.history`, `Note`, `SelectError::ReviewOnly`; cablaggio |
| `crates/xiox-project/src/tests/history/{mod,fake,scope,folders,queue,scratch,risk,analyze}.rs` | Test guidati da tabelle, con `CallSource` finta e alberi in `tempdir` |
| `crates/xiox-project/examples/session_probe.rs` | Sonda locale: conteggi, copertura e tempi; `--sample` mostra percorsi e riferimenti |
| `crates/xiox-cli/src/{args.rs,run.rs,render.rs,json.rs,observe.rs,mcp.rs,testdata.rs}` | `--no-history`, blocco SCRATCH, riga della cronologia, opzioni dell'osservatore |
| `docs/xiox-cli.md`, `AGENTS.md` | Guida e stato |

---

### Task 1: Descrittori di sessione non aggregati ed errori tipati del dump

La specifica dice: «L'adapter deve esporre i descrittori prima dell'aggregazione per cartella». Oggi `MukadeSessions::list` è privato e Claude Code elenca solo le radici. La coda (Task 4) ha bisogno anche dei subagent, con il loro `parent`. Ha bisogno inoltre di sapere perché un `dump` è fallito, senza leggere il testo dell'errore.

**Files:**
- Create: `crates/xiox-mukade/src/descriptors.rs`
- Modify:
  - `crates/xiox-mukade/src/lib.rs`: `mod descriptors;`, `pub use`; `parse_ms` diventa `descriptors::parse_time_ms`, e `aggregate` la usa senza cambiare comportamento;
  - `crates/xiox-mukade/src/calls.rs`: `DumpError`, firma di `CallReader::dump`;
  - `crates/xiox-app/src/lib.rs`: `load_descriptors`.
- Test: `crates/xiox-mukade/src/descriptors.rs` (`#[cfg(test)] mod tests`), `crates/xiox-mukade/src/calls.rs` (tests esistenti), `crates/xiox-app/src/tests.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug)] pub struct Descriptor { pub agent: Agent, pub harness: String, pub store: String, pub info: SessionInfo }`
    - `harness`: il nome di Mukade (`claude_code`, `codex`, `opencode`, `pi`), lo stesso di `ToolCall.harness`;
    - `store`: `store_root().to_string_lossy()`, la stessa stringa che `CallReader::dump` mette in `ToolCall.store`;
    - `pub fn updated_ms(&self) -> Option<i64>`: `parse_time_ms(info.updated)`, altrimenti mtime di `info.path`, come `aggregate`. Serve alla recenza e all'ordine, mai alla provenienza.
  - `impl MukadeSessions { pub fn descriptors(&self) -> Loaded<Descriptor> }`:
    - ogni sessione dello store, figli compresi: `run_list(true)` per ogni harness, Claude Code incluso;
    - store non configurato o assente: vuoto, senza warning;
    - errore di `mukade list`: un warning `"{agent:?}: {e:#}"` e nessuna voce.
  - `pub fn parse_time_ms(s: &str) -> Option<i64>`: il corpo dell'attuale `parse_ms`.
  - `#[derive(Debug)] pub enum DumpError`, con cinque varianti:
    - `NotConfigured(String)`
    - `Spawn(String)`
    - `TimedOut(Duration)`
    - `TooLarge(usize)`
    - `Exited(String)`, dove la stringa è lo stderr ridotto da `trim()`.

    `Display` riproduce i testi di oggi (`mukade dump {harness} {id}: …`) e c'è `impl std::error::Error`.
  - `pub fn dump(&self, id: &str, session_cwd: Option<&str>) -> Result<Vec<ToolCall>, DumpError>`.
    - `BoundedError::Io` diventa `Spawn`, `TimedOut` resta `TimedOut`, `StdoutCap` diventa `TooLarge`.
    - `dump_calls` resta `anyhow::Result` e converte con `?`.
  - `pub fn load_descriptors(cache_dir: Option<PathBuf>) -> Vec<(Agent, Loaded<Descriptor>)>` in `xiox-app`:
    - le quattro voci di `AGENTS`, in quell'ordine;
    - una sorgente che va in panic diventa `Loaded` con il warning `"{agent:?}: internal error while listing the sessions, ignored"`.

- [ ] **Step 1: Write the failing tests** in `descriptors.rs`, con lo store Claude finto di `calls.rs::tests::write_claude_store`, spostato in un `pub(crate)` di test condiviso:
  - `descriptors_list_children_with_their_parent`. Lo store ha una sessione radice e un subagent. Il subagent si aggiunge al writer nella disposizione di Claude Code: `projects/D--proj/<id radice>/subagents/agent-<aid>.jsonl`, con record che portano `sessionId: <id radice>`, `isSidechain: true` e `agentId: <aid>`.

    `vendor/mukade/` non ha documentazione. Prima di scrivere l'asserzione, la disposizione si conferma lanciando `vendor/mukade/mukade.exe list claude_code --children` sullo store in `tempdir`, con `CLAUDE_CONFIG_DIR` e `MUKADE_DATA_DIR` in `tempdir`. Il risultato atteso:
    - due `Descriptor`;
    - il figlio con `info.parent == Some(<id radice>)`;
    - `harness == "claude_code"`;
    - `store == <store>.to_string_lossy()`.

    Se Mukade non elenca il figlio, il rapporto del task lo dice e il test tiene solo le asserzioni sulla radice. La parentela resta allora provata dal Task 4 con descrittori finti.
  - `descriptor_store_matches_the_tool_call_store`: per la sessione radice, `descriptors()[0].store == call_reader().dump(id, None).unwrap()[0].store`.
  - `missing_store_lists_nothing_quietly`: `StoreRoots` verso una cartella inesistente dà `items` e `warnings` vuoti.
  - `updated_ms_reads_rfc3339_and_falls_back_to_the_file`: `"2026-11-01T10:00:00.000Z"` dà `1_793_527_200_000`. Senza `updated`, il valore è l'mtime del file `info.path`.
- [ ] **Step 2: Write the failing test** `a_dump_past_its_timeout_is_timed_out` in `calls.rs`: `call_reader().with_timeout(Duration::from_millis(1)).dump(id, None)` dà `Err(DumpError::TimedOut(_))`. L'avvio di un processo dura più di 1 ms, quindi `run_bounded` deve uccidere il figlio.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-mukade descriptors` e `cargo test -j 1 -p xiox-mukade a_dump_past`. Expected: FAIL, perché `descriptors`, `Descriptor` e `DumpError` non esistono.
- [ ] **Step 4: Implement** `descriptors.rs`, `DumpError` e `load_descriptors` con le firme sopra. `run_list`, `store_root` e `harness` diventano `pub(crate)`. `list()` e `aggregate` non cambiano.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-mukade` e `cargo test -j 1 -p xiox-app`. Expected: PASS, compresi i test esistenti di `list`/`aggregate`/`dump_calls`.
- [ ] **Step 6: Commit** — `feat(xiox-mukade): descrittori non aggregati con i figli ed errori tipati del dump`.

---

### Task 2: La forma del risultato e il rifiuto di `scratch` in `select`

Tutti i tipi nuovi del risultato entrano qui, insieme a un aggiornamento meccanico dei letterali. Così i task successivi toccano solo la logica. La parte minima del passo 4 entra qui: una voce `scratch` non si seleziona, né da sola né dentro un contenitore.

**Files:**
- Create: `crates/xiox-project/src/history/mod.rs`, `crates/xiox-project/src/history/report.rs`
- Create: `crates/xiox-project/src/tests/history/mod.rs`, `crates/xiox-project/src/tests/history/json.rs`
- Modify:
  - `crates/xiox-project/src/lib.rs`: `mod history;`, `pub use history::{…}`; `Category::Scratch` fra `Named` e `Logs`, con `Display` `"scratch"`; `Why::SessionFolder`; `Item.provenance`; `ProjectScan.history`; `Options.history`; `Note::HistoryPartial`, `Note::HistoryNotRead`.
  - `crates/xiox-project/src/provenance/ledger.rs`: `#[serde(rename_all = "camelCase")]` su `Coverage`. La sonda `shell_probe` non serializza `Coverage`: lo si verifica con `grep`.
  - `crates/xiox-project/src/clean.rs`:
    - `SelectError::ReviewOnly`;
    - in `clean_with`, `Options { history: false, .. }`.
  - Letterali di `Item`, `ProjectScan`, `Options` ed `Env`, con `provenance: None`, `history: None`, `history: true`/`false` e `history: HistoryInput::none()`. I file sono:
    - `crates/xiox-project/src/{owned.rs,search.rs,logs.rs,lib.rs,clean.rs}`;
    - `crates/xiox-project/src/tests/{fixtures.rs,clean.rs,logs.rs,search.rs,owned.rs}`;
    - `crates/xiox-cli/src/{observe.rs,testdata.rs,run.rs}`;
    - `crates/xiox-observer/src/tests/fixtures.rs`.

    Si trovano con `grep -rn "Item {\|ProjectScan {\|Options {\|Env {" crates`.
  - `crates/xiox-cli/src/render.rs`: `title_of(Category::Scratch) => "SCRATCH"`, `ORDER` con 7 voci. Per ora `reason_text` dà `"found in session history"`; il testo vero è del Task 8.
- Test: `crates/xiox-project/src/tests/history/json.rs`, `crates/xiox-project/src/tests/clean.rs`

**Interfaces:**
- Produces (in `history/report.rs`, tutti `Clone, Debug, PartialEq, Eq, Serialize`, campi camelCase, valori kebab-case):
  - `pub enum Risk { NoGit, WeakEvidence, PartialCoverage, RecentActivity }`, anche `Copy, Ord`.
  - `pub enum Activity { Active, Unknown }`, anche `Copy`.
    - «non attivo verificato» non si produce: nessuna sorgente lo verifica, perché il `live: None` di Mukade vuol dire «non noto».
    - È un limite dichiarato nella guida (Task 8). La variante si aggiunge quando esiste una sorgente.
  - `pub enum CallOutcome { Succeeded, Failed, Unknown }`, con `From<xiox_mukade::Outcome>`.
  - `#[serde(tag = "type")] pub enum OperationSource`, con cinque varianti:
    - `Tool { tool: String }`
    - `Shell { op: OpKind, script: bool }`
    - `Patch { action: PatchAction }`
    - `Cd`
    - `Cwd`
  - `pub struct Operation { pub source: OperationSource, pub outcome: CallOutcome, pub strength: Option<Strength>, pub entry: Option<String>, pub call: Option<String>, pub time: Option<String>, pub cwd: Option<String> }`, con `From<&EvidenceRef>`. Niente testo di comandi: solo riferimenti.
  - `pub struct SessionEvidence`, con i campi:
    - `pub agent: Agent`, `pub harness: String`, `pub store: String`, `pub id: String`
    - `pub parent: Option<String>`
    - `pub last_activity_ms: Option<i64>`
    - `pub strength: Option<Strength>`
    - `pub listed_cwd: bool`
    - `pub operations: Vec<Operation>`, `pub omitted: usize`
    - `pub failed: usize`
    - `pub gaps: bool`: la `Coverage` della sessione ha `unresolved + looped + script + truncated + exhausted > 0`.
  - `pub struct Provenance { pub strength: Option<Strength>, pub last_activity_ms: Option<i64>, pub sessions: Vec<SessionEvidence> }`
    - `sessions` sono distinte per `(store, harness, id)` e ordinate per `last_activity_ms` decrescente, poi senza data, poi per id.
  - `pub enum ReadFailure { NoReader, TimedOut, TooLarge, Failed }`, anche `Copy, Ord, Hash`.
  - `pub enum Exclusion { NotLocal, OutsideScope, Protected, InsideProject, OtherRepository, NotAProject, Missing, Link }`, anche `Copy, Ord, Hash`.
  - `#[serde(tag = "type")] pub enum Limit { Sessions { max: usize, left: usize }, Calls { max: usize }, Duration { secs: u64, left: usize } }`
  - `pub struct SourceReport { pub agent: Agent, pub listing_failed: bool, pub listed: usize, pub related: usize, pub read: usize, pub failed: BTreeMap<ReadFailure, usize> }`
  - `pub struct HistoryReport { pub sources: Vec<SourceReport>, pub sessions_read: usize, pub calls: usize, pub shell: Coverage, pub excluded: BTreeMap<Exclusion, usize>, pub limits: Vec<Limit> }`
    - `fn partial(&self) -> bool`: un `limits` non vuoto, un `listing_failed`, oppure un `failed` non vuoto.
- Produces (in `history/mod.rs`):
  - `#[derive(Clone, Debug, Default)] pub(crate) struct UserDirs { pub home: PathBuf, pub temp: PathBuf }`
    - `fn from_env() -> UserDirs`: `USERPROFILE` e `std::env::temp_dir()`, senza `\\?\`;
    - `fn path_env(&self) -> PathEnv`.
  - `pub(crate) trait CallSource { fn calls(&self, session: &Descriptor) -> Result<Vec<ToolCall>, ReadFailure>; }`
  - `#[derive(Clone, Copy)] pub(crate) struct HistoryInput<'a> { pub listings: &'a [(Agent, Loaded<Descriptor>)], pub calls: Option<&'a dyn CallSource>, pub user: &'a UserDirs }`
    - `fn none() -> HistoryInput<'static>`: niente listing, niente sorgente, `UserDirs` vuote da uno `static LazyLock<UserDirs>`.
  - nuovo campo `pub history: HistoryInput<'a>` in `Env`.
- Produces (in `lib.rs`):
  - `Why::SessionFolder { strength: Strength, from_script: bool, risks: Vec<Risk>, activity: Activity }`, con `type` `session-folder`.
  - `Item.provenance: Option<Provenance>`, serializzato sempre (`null`), come `bytes`.
  - `ProjectScan.history: Option<HistoryReport>`: `None` quando la cronologia non è stata chiesta o non c'era una sorgente.
  - `Options.history: bool`, con default `true`.
  - `SelectError::ReviewOnly { path: PathBuf, inner: Option<PathBuf> }`, con il testo `"listed for review only: folders found in session history cannot be removed yet: {path}"`. Se `inner` è `Some`, il testo è invece `"cannot be removed: it holds {inner}, found in session history: {path}"`.

- [ ] **Step 1: Write the failing tests** in `tests/history/json.rs`:
  - `scratch_item_serializes_with_its_provenance`: un `Item` `Scratch` con `Why::SessionFolder { strength: Created, from_script: false, risks: vec![NoGit, RecentActivity], activity: Unknown }` e una `Provenance` di una sessione con una `Operation` `Shell { op: Create, script: false }` `Succeeded`. `serde_json::to_value`, poi `assert_eq!` sui campi:
    - `category` vale `"scratch"`;
    - `why.type` vale `"session-folder"`;
    - `why.risks` vale `["no-git","recent-activity"]`;
    - `provenance.sessions[0].operations[0].source` vale `{"type":"shell","op":"create","script":false}`;
    - `outcome` vale `"succeeded"`;
    - `strength` vale `"created"`;
    - `lastActivityMs` è presente.
  - `old_items_carry_null_provenance_and_scan_stays_schema_1`: un `Builds` ha `"provenance": null`; una scansione con `history: None` ha `"history": null` e `"schema": 1`.
  - `history_report_serializes_counts_by_kind`: `failed: {TimedOut: 1}` dà `{"timed-out":1}`; `excluded: {OutsideScope: 2}` dà `{"outside-scope":2}`; `Limit::Sessions { max: 250, left: 3 }` dà `{"type":"sessions","max":250,"left":3}`; `shell.shellCalls` è presente.
- [ ] **Step 2: Write the failing tests** in `tests/clean.rs`:
  - `select_refuses_a_scratch_item`: `--include` di una voce `Scratch` sbloccata dà `Err(SelectError::ReviewOnly { inner: None, .. })`.
  - `select_refuses_a_folder_that_holds_a_scratch_item`: `--include` di una voce `Named` che contiene una voce `Scratch` dà `ReviewOnly { inner: Some(<scratch>) }`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project history::json` e `cargo test -j 1 -p xiox-project select_refuses`. Expected: FAIL in compilazione, perché i tipi non esistono.
- [ ] **Step 4: Implement** i tipi e il controllo in `select`, accanto a `blocked_inside`. Il controllo va dopo `Blocked` e prima di `ContainsBlocked`. Poi i letterali, con `cargo check -j 1 --workspace --tests`.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project`, `cargo test -j 1 -p xiox-cli` e `cargo test -j 1 -p xiox-observer`. Expected: PASS. I test esistenti di JSON e render vedono un `provenance: null` e un `history: null` in più: è l'unica differenza ammessa. Un test che confronta il JSON intero si aggiorna solo con quei due campi.
- [ ] **Step 6: Commit** — `feat(xiox-project): forma del risultato per scratch, provenienza e cronologia`.

---

### Task 3: Parte A, copie dai `cwd` delle sessioni

**Files:**
- Create:
  - `crates/xiox-project/src/history/scope.rs`, `crates/xiox-project/src/history/folders.rs`, per la parte del listing;
  - `crates/xiox-project/src/tests/history/{fake.rs,scope.rs}`.
- Modify:
  - `crates/xiox-project/src/search.rs`: `search` riceve le radici in più; la costruzione dell'`Item` diventa `item_for`; nuova `verify_dirs`; `named_blocks` diventa `pub(crate)`.
  - `crates/xiox-project/src/owned.rs`: `History::start`, `listed_roots`, `attach`.
  - `crates/xiox-scanner/src/sessions.rs` e `crates/xiox-scanner/src/lib.rs`: `pub fn is_broad_dir(p: &Path, home: Option<&Path>) -> bool`, `pub fn is_cloud_dir(p: &Path) -> bool`, cioè nome cloud oppure placeholder. Sono le funzioni private di oggi, senza cambiare comportamento.

**Interfaces:**
- Consumes: `Descriptor` (Task 1); `HistoryInput`, `Provenance`, `SessionEvidence`, `Exclusion` (Task 2); `xiox_scanner::{project_roots_from_sessions, Skip}`.
- Produces:
  - `pub(crate) enum Scope { Everywhere, Within(Vec<String>) }`, con le `path_key`.
    - `fn of(project: &Project, opts: &Options, env: &Env) -> Scope`:
      - `opts.drives` e `env.search_roots == None` danno `Everywhere`;
      - `opts.drives` con `Some(roots)` dà `Within(genitore + roots)`;
      - `!opts.drives` dà `Within(genitore)`.
  - `pub(crate) fn local_dir(raw: &str) -> Result<PathBuf, Exclusion>`, solo lessicale, senza disco:
    - toglie `\\?\` con `strip_verbatim`, mette `\` al posto di `/`, normalizza con `normalize_path`;
    - accetta solo `X:\…` assoluto;
    - UNC, `\\.\`, percorsi relativi e componenti `..` danno `NotLocal`.
  - `pub(crate) fn cwd_roots(project: &Project, dirs: &[PathBuf], scope: &Scope, home: Option<&Path>, excluded: &mut BTreeMap<Exclusion, usize>) -> Vec<PathBuf>`:
    - fuori da `scope` dà `OutsideScope` e non tocca il disco;
    - dentro il progetto o un worktree dà `InsideProject`;
    - poi `project_roots_from_sessions`, con le `Skip` mappate così: `Missing` → `Missing`, `Broad` e `Cloud` → `Protected`, `NotAProject` → `NotAProject`;
    - si scartano le radici che sono il progetto o un suo worktree.
  - `pub(crate) struct Folders`, accumulatore per `path_key` di cartella:
    - `fn add_listed(&mut self, dir: &Path, d: &Descriptor)`: una `SessionEvidence` con `listed_cwd: true` e `strength: Some(Used)`;
    - `fn provenance_within(&self, root_key: &str) -> Option<Provenance>`: unisce le evidenze della cartella e di tutto ciò che sta sotto, una `SessionEvidence` per sessione;
      - `strength` è il massimo dei non falliti;
      - `last_activity_ms` è il massimo di `Descriptor::updated_ms` e dei `time` delle operazioni, letti con `parse_time_ms`.
  - in `search.rs`:
    - `pub(crate) fn search(project: &Project, opts: &Options, env: &Env, extra: &[PathBuf]) -> Found`: `extra` entra nei `candidates` prima della deduplicazione per `path_key`;
    - `pub(crate) fn verify_dirs(project: &Project, opts: &Options, env: &Env, dirs: &[PathBuf]) -> Vec<Item>`: `classify` più `item_for`, per il Task 4.
  - in `history/mod.rs`:
    - `pub(crate) struct History<'a>`;
    - `fn start(project: &Project, opts: &Options, env: &'a Env) -> Option<History<'a>>`: `None` quando `!opts.history` o quando `env.history.listings` è vuoto;
    - `fn listed_roots(&mut self, project: &Project, opts: &Options, env: &Env) -> Vec<PathBuf>`;
    - `fn attach(&self, items: &mut [Item])`: `provenance` per le voci `Copies`, `Named` e `Scratch`, da `provenance_within(item.key)`.
- Nei test (`fake.rs`):
  - `fn descriptor(agent: Agent, store: &str, id: &str, cwd: &str, updated: &str, parent: Option<&str>) -> Descriptor`;
  - `fn input<'a>(listings: &'a [(Agent, Loaded<Descriptor>)], calls: Option<&'a dyn CallSource>, user: &'a UserDirs) -> HistoryInput<'a>`.

- [ ] **Step 1: Write the failing tests** in `tests/history/scope.rs`:
  - `local_dir_table`, una tabella `(input, atteso)`:
    - `r"\\?\C:\w\a"` → `Ok(C:\w\a)`;
    - `"/d/x"` → `Err(NotLocal)`: le forme Git Bash si risolvono solo nel resolver, con il contesto, e un `cwd` di listing ne è privo;
    - `"D:/x/y"` → `Ok(D:\x\y)`;
    - `r"\\srv\share\x"` → `NotLocal`;
    - `r"\\.\pipe\x"` → `NotLocal`;
    - `"rel\x"` → `NotLocal`;
    - `r"D:\a\..\b"` → `NotLocal`.
  - `deep_copy_in_a_session_cwd_is_a_copy_with_provenance`:
    - una copia clonata a profondità 5 sotto la base, oltre `WALK_DEPTH`, con `search_roots = [base]`;
    - due descrittori `claude` con id diversi e `cwd` in `<copia>\src`, più uno `codex` con lo stesso `cwd`.

    `analyze_with` dà una voce `Copies` per la copia con:
    - `why` `SameRepository { shared_root_commit: true, .. }`;
    - `provenance.sessions.len() == 3`, ogni `listed_cwd == true` e `strength == Some(Used)`;
    - `last_activity_ms` pari al più recente dei tre `updated`.
  - `cwd_copy_found_by_walk_is_listed_once` (Review Focus 1): la copia è figlia del genitore, quindi la trova il walk, e un descrittore ha come `cwd` `\\?\` + la stessa copia in minuscolo con `/`. Il risultato è una sola voce con quel `key` e la `provenance` non nulla.
  - `no_drives_keeps_session_roots_next_to_the_project`: con `opts.drives = false` ci sono due descrittori fuori dal genitore del progetto. Uno punta a una copia vera in un'altra base, l'altro a una cartella inesistente. Il risultato:
    - nessuna voce;
    - `history.excluded["outside-scope"] == 2`;
    - nessuna chiave `"missing"`, che prova lo scarto prima di ogni accesso al disco.
  - `unrelated_repository_in_a_cwd_is_not_listed`: un repository senza commit radice comune e senza origin uguale non dà nessuna voce.
  - `session_inside_the_project_counts_as_inside`: un `cwd` in `<progetto>\src` dà `excluded["inside-project"] == 1` e nessuna voce nuova.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project history::scope`. Expected: FAIL.
- [ ] **Step 3: Implement** `scope.rs`, `Folders::add_listed`/`provenance_within`, `History::{start, listed_roots, attach}` e i cambi di `search`. In `owned::analyze_with` l'ordine è:
  1. `History::start`;
  2. `listed_roots`;
  3. `search(…, &extra)`;
  4. `attach`;
  5. `sort_items`.

  `ProjectScan.history = Some(report)` quando `start` ha dato `Some`.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-scanner`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): copie dai cwd delle sessioni con la loro provenienza`.

---

### Task 4: La coda delle sessioni del progetto e la lettura con budget

**Files:**
- Create: `crates/xiox-project/src/history/queue.rs`, `crates/xiox-project/src/tests/history/{queue.rs,folders.rs}`
- Modify: `crates/xiox-project/src/history/{mod.rs,folders.rs}`, `crates/xiox-project/src/owned.rs`

**Interfaces:**
- Consumes:
  - `CallSource`, `ReadFailure`, `HistoryReport`, `Limit`, `SourceReport` (Task 2);
  - `Folders`, `verify_dirs`, `cwd_roots` (Task 3);
  - `session_ledgers`, `Ledger`, `Coverage`, `SessionKey` (in `main`).
- Produces:
  - `pub(crate) struct Budget { pub max_sessions: usize, pub max_calls: usize, pub max_duration: Duration }`
    - `pub(crate) const BUDGET: Budget = Budget { max_sessions: 250, max_calls: 250_000, max_duration: Duration::from_secs(240) }`. Sono valori iniziali, da tarare con `session_probe` (Task 9). La specifica li lascia alla prova tecnica.
    - La durata si controlla prima di avviare un `dump`. Un `dump` già avviato è limitato dal suo timeout reale di 120 s (`Limits::default`).
  - `pub(crate) struct Queue<'a>`:
    - `fn new(listings: &'a [(Agent, Loaded<Descriptor>)], roots: &[PathBuf]) -> Queue<'a>`;
    - `fn add_root(&mut self, root: &Path)`;
    - `fn next(&mut self) -> Option<&'a Descriptor>`.
  - Le regole di `next`:
    - correlata è una sessione il cui `cwd`, passato da `local_dir`, sta in una radice: progetto, worktree o copia verificata. È correlato anche un figlio (`info.parent`, stesso `store` e `harness`) di una sessione correlata;
    - la prossima è la correlata non letta con `updated_ms` più recente; a parità, o senza data, vale l'ordine `(store, harness, id)`;
    - la chiave di deduplicazione è `(store, harness, id)`, mai la sola id;
    - un descrittore con `info.unreadable` è contato come `Failed` e non viene passato alla sorgente.
  - `impl History`:
    - `fn read(&mut self, project: &Project, items: &mut Vec<Item>, verify: &mut dyn FnMut(&[PathBuf]) -> Vec<Item>)`.
    - Per ogni sessione:
      1. `calls`;
      2. `session_ledgers(&calls, &user.path_env())`;
      3. `Folders::add_ledger`;
      4. `drop(calls)`.
    - Poi le cartelle nuove con evidenza, fuori dal progetto, passano a `cwd_roots`, quindi a `verify`. La specifica dice: «gli ulteriori cambi di contesto osservati nelle sessioni lette».
    - Ogni nuova voce `Copies` entra in `items` e in `Queue::add_root`.
    - Ci si ferma a coda vuota o a budget esaurito. Il `Limit` registra le sessioni correlate rimaste in `left`.
  - `Folders::add_ledger(&mut self, d: &Descriptor, ledger: &Ledger)`: per ogni `FolderEvidence`, una `SessionEvidence` con:
    - `operations`: i riferimenti, fino a `MAX_OPERATIONS_SHOWN = 20`, con il resto in `omitted`;
    - `failed`: `failed.len()`;
    - `strength`;
    - `gaps` dalla `Coverage` della sessione.
  - `HistoryReport.shell` è la somma delle `Coverage`; `calls` conta le chiamate lette; `sources` ha una riga per agente, con `listed`, `related`, `read` e `failed`.

- [ ] **Step 1: Write the failing tests** in `tests/history/queue.rs`, con `FakeCalls` in `fake.rs`. `FakeCalls` è `HashMap<(String, String), Result<Vec<ToolCall>, ReadFailure>>`, con chiave `(store, id)`, più un `RefCell<Vec<String>>` delle id lette. Le `ToolCall` si costruiscono con i builder di `tests/provenance/ledger.rs` (`call`, `bash`), spostati in `fake.rs`.
  - `only_sessions_of_the_project_are_read_newest_first`: tre descrittori, due nel progetto con `updated` 10:00 e 12:00 e uno altrove. Le id lette sono, nell'ordine, quella delle 12:00 e quella delle 10:00.
  - `child_sessions_follow_their_parent`: un figlio con `cwd` fuori dal progetto ma `parent` correlato viene letto.
  - `same_id_in_two_stores_is_two_sessions` (Review Focus 3): la stessa id in due store, entrambi nel progetto, dà due letture e due `SessionEvidence` sulla stessa cartella.
  - `a_failed_read_is_counted_and_the_rest_is_read` (Review Focus 2): tre sessioni, la seconda `Err(TimedOut)`. Il risultato è:
    - `read == 2`;
    - `sources[claude].failed == {TimedOut: 1}`;
    - `notes` contiene `HistoryPartial`;
    - il JSON della scansione non contiene `"mukade"`.
  - `budget_stops_and_says_how_many_were_left`: con `Budget { max_sessions: 2, .. }` e 5 sessioni correlate, il risultato è `read == 2` e `limits == [Sessions { max: 2, left: 3 }]`.
  - `a_clone_made_by_a_session_extends_the_queue`:
    - la sessione A nel progetto ha `git clone <progetto> <base>\c2`; il test crea davvero il clone, perché la sorgente non esegue nulla;
    - la sessione B ha `cwd` in `<base>\c2`.

    Le letture sono A, poi B. La voce `Copies` per `c2` ha `provenance` con A, di forza `Created`, e B, di forza `Used`.
  - `listing_failure_is_reported_apart_from_an_absent_store`: un `Loaded` con un warning e nessuna voce dà `listing_failed: true`. Uno vuoto senza warning dà `listing_failed: false` e `listed: 0`.
- [ ] **Step 2: Write the failing tests** in `tests/history/folders.rs`:
  - `failed_create_keeps_only_failed_refs`: `mkdir <d>` con `error: Some(true)` dà `strength == None` e `failed == 1`.
  - `operations_are_capped_and_counted`: 25 Write nella stessa cartella danno 20 `operations` e `omitted == 5`.
  - `relative_path_without_context_stays_unresolved`: una chiamata senza `cwd` con `mkdir out` non dà cartelle; `shell.unresolved >= 1`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project history::queue` e `cargo test -j 1 -p xiox-project history::folders`. Expected: FAIL.
- [ ] **Step 4: Implement** `queue.rs`, `Folders::add_ledger`, `History::read`. `owned` chiama `read` dopo `search`, passando come `verify` una chiusura su `search::verify_dirs`. Il budget si inietta in `History` (`with_budget`, solo per i test).
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project`. Expected: PASS.
- [ ] **Step 6: Commit** — `feat(xiox-project): lettura delle sessioni del progetto con coda, budget e copertura`.

---

### Task 5: La categoria `scratch`

**Files:**
- Create: `crates/xiox-project/src/history/scratch.rs`, `crates/xiox-project/src/tests/history/scratch.rs`
- Modify: `crates/xiox-project/src/history/mod.rs`, `crates/xiox-project/src/owned.rs`

**Interfaces:**
- Consumes: `Folders` (Task 3, 4); `named_blocks`, `is_link`, `is_git_repo`, `inside_other_repo`, `live_session_in` (`search.rs`/`owned.rs`, resi `pub(crate)` dove serve); `is_broad_dir`, `is_cloud_dir` (Task 3); `xiox_disktree::fixed_drives`.
- Produces:
  - `pub(crate) fn scratch_items(project: &Project, folders: &Folders, listed: &[Item], env: &Env, opts: &Options, excluded: &mut BTreeMap<Exclusion, usize>) -> Vec<Item>`. Le voci hanno:
    - `category: Scratch`, `level: Caution`;
    - `target: Dir { kind: ProjectFolder }`;
    - `bytes` misurato se `opts.measure`;
    - `blocked` da `named_blocks`, più `LiveSession` se `live_session_in`;
    - `why: SessionFolder { strength, from_script, risks: vec![], activity: Unknown }`, completato dal Task 6.
  - `impl History { fn scratch(&mut self, project: &Project, items: &[Item], env: &Env, opts: &Options) -> Vec<Item> }`.
  - Regole, in quest'ordine. Le prime quattro sono lessicali e si applicano prima di ogni accesso al disco:
    1. `strength >= Some(Written)`. Una cartella solo `Used`, o con soli `failed`, non è `scratch`; non è un'esclusione contata.
    2. `local_dir` e lo `Scope` del Task 3 danno `NotLocal` o `OutsideScope`.
    3. Dentro il progetto o un worktree dà `InsideProject`; la policy Git del sottoalbero è del passo 4. Contenitore del progetto o di un worktree (`Own::inside`) dà `Protected`.
    4. Radice di disco, `user.home`, `user.temp` stesso, `is_broad_dir(p, home)` dà `Protected`, salvo un discendente stretto di `user.temp`, che resta ammesso. Lo stesso vale per `is_system_dir` sull'ultimo componente.
    5. Un drive che non è fra `fixed_drives()` dà `NotLocal`.
    6. Un antenato, dall'alto verso il basso, o la cartella stessa che è un link o una junction (`symlink_metadata` più `is_link`) dà `Link`, senza scendere oltre.
    7. `is_cloud_dir` dà `Protected`.
    8. Una cartella che non esiste o non è una directory dà `Missing`.
    9. `inside_other_repo` dà `OtherRepository`.
    10. Già una voce in `listed` con la stessa `key`: niente `scratch`; la provenienza arriva da `attach`.
    11. Fra i candidati rimasti, uno dentro un altro candidato confluisce nel più esterno, con l'evidenza unita da `provenance_within`. Non si sale mai oltre una cartella che abbia evidenza propria.
  - `from_script`: tutte le operazioni che danno la forza vengono da `Shell { script: true }`.

- [ ] **Step 1: Write the failing tests** in `tests/history/scratch.rs`. Una tabella di casi `(nome, chiamate della sessione, cartelle create sul disco, atteso)` su una base in `tempdir`, con `UserDirs { home: <base>\home, temp: <base>\home\AppData\Local\Temp }`:
  - `mkdir <base>\agent-runs\planner-benchmark-42` e Write di `benchmark.py` lì dentro danno `Scratch` di forza `Created`, con il caso d'esempio della specifica.
  - Write di `<base>\existing\out.txt`, con `existing` preesistente, dà forza `Written`: il motivo non dice «creata».
  - Un `mkdir` fallito, con la cartella che comunque esiste, non dà nessuna voce.
  - Read, Glob e `cd` su una cartella non danno nessuna voce.
  - `<base>\agent-runs\batch\phazly` creata dà solo `phazly`, mai `batch`.
  - Una cartella accanto, senza evidenze ma creata nello stesso momento, non dà nessuna voce.
  - Un `git init` creato dalla sessione con un commit non spinto dà `Scratch` con `blocked == [UnsavedWork]`.
  - `<base>\runs\a` creata e `<base>\runs\a\src\x.rs` scritto danno una sola voce, `a`, con entrambe le operazioni nella `provenance`.
- [ ] **Step 2: Write the failing tests**:
  - `broad_and_protected_folders_are_never_scratch` (Review Focus 5), una tabella `(chiamata, atteso)`. Questi casi non danno nessuna voce:
    - Write di `home\notes.txt`, che punta alla home: `protected`;
    - Write di `home\Desktop\a.txt`: `protected`;
    - Write di `<drive>:\out.txt`, che punta alla radice del disco della base: `protected`;
    - Write in `temp\a.txt`, che punta a `temp` stesso: `protected`;
    - Write nel genitore del progetto: `protected`;
    - Write nella cartella che contiene un worktree: `protected`;
    - Write in `<progetto>\tmp\a.txt`: `inside-project`.

    Questi casi restano ammessi e danno `Scratch`, a controllo che la guardia non sia troppo larga:
    - `mkdir ~\x`, risolto in `home\x`;
    - Write in `temp\run1\a.txt`.
  - `junction_or_missing_folder_is_never_entered` (Review Focus 4). La creazione della junction segue lo stesso schema di `search.rs::junction_with_a_matching_name_is_ignored`, con `skipped` se non si può creare. I casi:
    - una cartella creata dalla sessione e poi sostituita da una junction verso `<base>\target` dà `excluded["link"] == 1`, nessuna voce, e nessun file di `target` misurato;
    - una cartella creata e poi cancellata dà `excluded["missing"] == 1`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project history::scratch`. Expected: FAIL.
- [ ] **Step 4: Implement** `scratch.rs` e `History::scratch`. `owned` aggiunge le voci dopo `read` e prima di `attach`.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project`. Expected: PASS.
- [ ] **Step 6: Commit** — `feat(xiox-project): categoria scratch dalle evidenze delle sessioni`.

---

### Task 6: Rischio, attività e ordine per attività osservata

**Files:**
- Create: `crates/xiox-project/src/history/risk.rs`, `crates/xiox-project/src/tests/history/risk.rs`
- Modify: `crates/xiox-project/src/history/scratch.rs`, `crates/xiox-project/src/owned.rs` (`sort_items`)

**Interfaces:**
- Consumes: `Provenance`, `Risk`, `Activity`, `HistoryReport::partial` (Task 2); `live_session_in`.
- Produces:
  - `pub(crate) const RECENT_MS: i64 = 24 * 60 * 60 * 1000;`
  - `pub(crate) fn risks(dir: &Path, provenance: &Provenance, report_partial: bool, now_ms: i64) -> Vec<Risk>`, ordinati e senza doppi:
    - `NoGit`: nessun `.git` in `dir`;
    - `WeakEvidence`: `provenance.strength < Some(Created)`;
    - `PartialCoverage`: `report_partial`, oppure una sessione della provenienza con `gaps`;
    - `RecentActivity`: `last_activity_ms >= now_ms - RECENT_MS`, oppure mtime di `dir` nella stessa finestra. È solo recenza: l'mtime non entra mai in `Provenance`.
  - `pub(crate) fn activity(dir: &Path, env: &Env) -> Activity`: `Active` se `live_session_in(dir, env)`, altrimenti `Unknown`.
  - `sort_items`: ordine di categoria come oggi; dentro `Scratch` vale `Reverse(last_activity_ms)` con le voci senza data in fondo, poi `key`. Le altre categorie restano ordinate per byte decrescenti, poi `key`.
  - `scratch_items` riempie `risks` e `activity`. Il `report_partial` è quello di fine lettura, quindi `History::scratch` si chiama dopo `read`.

- [ ] **Step 1: Write the failing tests** in `tests/history/risk.rs`:
  - `risks_table`, con casi `(forza, .git, ultimo ms, mtime, partial, gaps, atteso)`. `now = 1_793_527_200_000`; la cartella è in `tempdir`, con `File::set_modified` per l'mtime.
    - `Created`, senza `.git`, `now - 1h`, `partial` falso → `[NoGit, RecentActivity]`;
    - `Written`, con `.git`, `now - 48h`, mtime vecchio → `[WeakEvidence]`;
    - `Created`, con `.git`, `now - 48h`, mtime `now - 1h` → `[RecentActivity]`;
    - `Created`, con `.git`, vecchio, `partial` vero → `[PartialCoverage]`;
    - `Created`, con `.git`, vecchio, `gaps` in una sessione → `[PartialCoverage]`.
  - `old_mtime_does_not_drop_a_candidate`: una cartella creata nella cronologia con mtime di un anno prima resta `Scratch`, con gli stessi `strength` e `provenance`. È il caso «Timestamp NTFS diverso dalla cronologia» della specifica.
  - `scratch_is_sorted_by_observed_activity`: tre voci con ultimo ms 10:00, nessuno e 12:00 danno l'ordine 12:00, 10:00, senza data. Due voci senza data restano in ordine di `key`.
  - `live_session_inside_is_active_and_blocked`: un `SessionRecord` `live` in `dir` dà `activity == Active` e `blocked` con `LiveSession`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project history::risk`. Expected: FAIL.
- [ ] **Step 3: Implement** `risk.rs` e il nuovo `sort_items`.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project`. Expected: PASS. I test di ordinamento esistenti restano verdi.
- [ ] **Step 5: Commit** — `feat(xiox-project): rischi, attività e ordine per attività delle voci scratch`.

---

### Task 7: La sorgente reale e il cablaggio in `analyze` e `clean`

**Files:**
- Create: `crates/xiox-project/src/history/mukade.rs`, `crates/xiox-project/src/tests/history/analyze.rs`
- Modify:
  - `crates/xiox-project/src/lib.rs`: `analyze_project` carica i descrittori e costruisce `MukadeCalls`; `analyze_project_in` e `analyze_project_without_sessions` restano senza cronologia, e il loro doc lo dice;
  - `crates/xiox-project/src/clean.rs`: le `Env` di `clean_impl` hanno `HistoryInput::none()`;
  - `crates/xiox-cli/src/args.rs`: `--no-history` per `analyze` e `clean`, in `USAGE`;
  - `crates/xiox-cli/src/run.rs`: `analyze_path(project, no_drives, no_history, err)`;
  - `crates/xiox-cli/src/observe.rs`: `observe_options` con `history: false`;
  - `crates/xiox-cli/src/mcp.rs`, se costruisce `Options` per proprio conto.

**Interfaces:**
- Consumes: `Descriptor`, `DumpError`, `load_descriptors`, `CallReader` (Task 1); `CallSource`, `ReadFailure`, `HistoryInput`, `UserDirs` (Task 2).
- Produces:
  - `pub(crate) struct MukadeCalls<'a>`:
    - `fn new(sessions: &'a [MukadeSessions]) -> MukadeCalls<'a>`, con un `MukadeSessions` per agente nell'ordine di `AGENTS`;
    - `impl CallSource`.
  - Il lettore di un agente nasce pigramente alla prima sessione di quell'agente, in un `OnceCell<Option<CallReader<'a>>>`. Così la copia del database di OpenCode si fa una volta per analisi e solo se serve.
  - Un `call_reader()` in errore (OpenCode senza `XIOX_OPENCODE_EXE`) dà `NoReader` per ogni sessione di quell'agente.
  - La mappa degli errori è `TimedOut` → `TimedOut`, `TooLarge` → `TooLarge`, ogni altro errore → `Failed`. Il testo dell'errore non si conserva.
  - `dump(&d.info.id, d.info.cwd.as_deref())`.
  - `Command::Analyze { project, json, no_drives, no_history }` e `Command::Clean { …, no_history }`; `Options.history = !no_history`.

- [ ] **Step 1: Write the failing tests** in `tests/history/analyze.rs`:
  - `analyze_reads_a_fake_claude_store_through_mukade`:
    - un progetto git in `tempdir`;
    - uno store Claude finto, con il writer del Task 1 reso parametrico su `cwd` e comando, che ha una sessione con `cwd` uguale al progetto e `Bash` `mkdir "<base>\runs\bench-1"`;
    - `<base>\runs\bench-1` creata sul disco;
    - `MukadeSessions::with_roots` più `with_cache_dir(<base>\data)`, `MukadeCalls`, `Env` con `search_roots: Some(vec![])` e `UserDirs` finte.

    Il risultato è una voce `Scratch` `bench-1` di forza `Created`, con `provenance.sessions[0].harness == "claude_code"`. `history.sessionsRead` vale 1.
  - `without_history_the_scan_is_the_old_one`: lo stesso albero con `Options { history: false, .. }` dà `history: None`, nessuna voce `Scratch` e `provenance: None` ovunque. Le voci sono uguali, campo per campo, a quelle di `analyze_project_without_sessions`.
  - `opencode_without_exe_counts_no_reader`: un descrittore `opencode` correlato e `MukadeSessions` senza `opencode_exe` danno `failed == {NoReader: 1}` e `HistoryPartial`.
- [ ] **Step 2: Write the failing tests** in `crates/xiox-cli/src/args.rs`:
  - `analyze_no_history`: `["analyze", "--no-history"]` dà `no_history: true`.
  - `clean_no_history`.
  - `analyze_defaults` aggiornato con `no_history: false`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project history::analyze` e `cargo test -j 1 -p xiox-cli no_history`. Expected: FAIL.
- [ ] **Step 4: Implement** `mukade.rs` e il cablaggio. `analyze_project` fa, nell'ordine:
  1. `load_sessions`;
  2. `load_archived_sessions`;
  3. `load_descriptors(cache)`;
  4. `let sessions = AGENTS.map(|a| MukadeSessions::from_env(a).with_cache_dir(cache))`;
  5. `MukadeCalls::new(&sessions)`;
  6. `UserDirs::from_env()`.

  `clean_with` continua a riconvalidare con `history: false`: nessuna voce `scratch` è selezionabile, quindi la riconvalida non ne ha bisogno. Il passo 4 la cambierà.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project`, `cargo test -j 1 -p xiox-cli` e `cargo test -j 1 -p xiox-observer`. Expected: PASS. I test di `observe` e MCP non leggono mai la cronologia.
- [ ] **Step 6: Commit** — `feat: la cronologia delle sessioni in xiox analyze, --no-history`.

---

### Task 8: Testo della CLI e guida

**Files:**
- Modify: `crates/xiox-cli/src/render.rs`, `docs/xiox-cli.md`
- Test: `crates/xiox-cli/src/render.rs` (tests)

**Interfaces:**
- Consumes: `Why::SessionFolder`, `Provenance`, `HistoryReport`, `Note::{HistoryPartial, HistoryNotRead}` (Task 2–7).
- Produces: il testo esatto, in inglese.
  - Motivo `scratch`:
    - `Created` → `"made by a session"`;
    - `Written` → `"files written here by a session"`.

    Seguono sempre:
    - `" ({n} session[s], last {YYYY-MM-DD})"`, oppure `" ({n} session[s], no date)"`;
    - `", from a script"` se `from_script`;
    - `"; risk: "` più i rischi uniti da `", "`, con i testi `no git`, `weak evidence`, `partial history`, `recent activity`.
  - Motivo di `copies`/`named` con provenienza: il testo di oggi più `"; seen in {n} session[s]"`.
  - Una riga dopo `Search`, solo quando `history` è `Some`. La forma è `History  {read} sessions read of {related} related`, poi `; {f} could not be read` se `f > 0`, poi `; stopped at the {limit} limit` per ogni `Limit`. I testi dei limiti sono `session`, `tool call` e `time`.
  - Note:
    - `HistoryPartial` → `"the session history was read only in part: folders made by agents may be missing (see \"history\" in --json)"`;
    - `HistoryNotRead` → `"the session history could not be listed: no folder was taken from it"`. Si produce quando ogni listing ha `listing_failed`; va aggiunta in `History::finish`.
  - Una voce `scratch` nel riepilogo di `clean` non compare mai, perché `select` la rifiuta.

- [ ] **Step 1: Write the failing tests** in `render.rs`:
  - `scratch_block_and_history_line`: una scansione con due voci `Scratch` (`Created` con 2 sessioni e data; `Written` senza data, `from_script`, rischi `[NoGit, WeakEvidence]`), `history` con `read 3`, `related 4`, `failed {TimedOut:1}`, `limits [Sessions]`. Il testo contiene, alla lettera:
    - `SCRATCH`;
    - `made by a session (2 sessions, last 2026-11-15)`;
    - `files written here by a session (1 session, no date), from a script; risk: no git, weak evidence`;
    - `History  3 sessions read of 4 related; 1 could not be read; stopped at the session limit`.
  - `history_notes_have_their_text`: per le due note nuove, i testi sopra.
  - `no_history_line_without_history`: con `history: None` non c'è la riga `History`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-cli render`. Expected: FAIL.
- [ ] **Step 3: Implement** in `render.rs`. Poi la guida, in inglese, in `docs/xiox-cli.md`:
  - `## What belongs to a project`: la categoria `scratch` e la sua regola, cioè evidenza almeno «written», nessuna promessa che il contenuto sia inutile, rimozione non ancora possibile;
  - `## analyze reference`: `--no-history`, il budget (250 sessioni, 250.000 chiamate, 240 s, 120 s per sessione), la riga `History`;
  - `### JSON`:
    - la regola di versione «within one `schema`, new fields and new values may appear; a changed meaning increments it»;
    - righe di tabella per `items[].provenance` (sessioni, `operations`, `omitted`, `failed`, `gaps`, `listedCwd`), `why` `session-folder` (`strength`, `fromScript`, `risks`, `activity`) e `history` (`sources`, `sessionsRead`, `calls`, `shell`, `excluded`, `limits`);
    - le note nuove;
  - `## Limits`: le sessioni iniziate altrove e mai collegate possono sfuggire; «inactive» non è mai affermato; non sono lette 50 sessioni Pi con id ambigua e 7 catene di subagent Claude Code; `clean --include` di una voce `scratch` viene rifiutato fino alle conferme.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-cli` e `node scripts/italian-check.mjs`. Expected: PASS e nessun avviso.
- [ ] **Step 5: Commit** — `docs: scratch, provenienza e cronologia in xiox analyze`.

---

### Task 9: Sonda `session_probe` e stato in `AGENTS.md`

È codice di misura e va dichiarato come tale in testa al file. Usa le stesse API dell'analisi e non scrive la cache Xiox.

**Files:**
- Create: `crates/xiox-project/examples/session_probe.rs`
- Modify:
  - `crates/xiox-project/src/lib.rs`: `pub fn history_probe(path: &Path, budget: Option<(usize, usize, u64)>) -> Result<ProjectScan, Error>`, documentata come «for the probe». È `analyze_project` con `measure: false` e il budget dato, cioè sessioni, chiamate e secondi. Si appoggia a `pub(crate) fn probe_with(path: &Path, budget: Budget, env: &Env) -> Result<ProjectScan, Error>`, che i test chiamano con una `Env` finta.
  - `AGENTS.md`: stato dell'integrazione.

**Interfaces:**
- Consumes: `analyze_project` (Task 7), `HistoryReport`.
- Produces: `cargo run -j 1 -p xiox-project --example session_probe -- <project> [--sample N] [--max-sessions N] [--max-secs N]`.
  - La modalità predefinita stampa su stdout un oggetto JSON con `history`, il numero di voci per categoria ed `elapsedMs`. Non stampa percorsi.
  - `--sample N` stampa su stderr, per le prime `N` voci `scratch` e copie con provenienza, il percorso, la forza, le id di sessione e le operazioni `{source, outcome}`. Non stampa testo dei comandi.
  - La cache Xiox non si scrive: `with_cache_dir` punta a una cartella sotto `std::env::temp_dir()`.

- [ ] **Step 1: Write the failing check** `probe_runs_on_a_fake_project`, in `tests/history/analyze.rs`. `probe_with` gira sul progetto finto del Task 7, con `FakeCalls`, `Budget { max_sessions: 1, .. }` e due sessioni correlate. Il risultato:
  - `history` è `Some`;
  - `limits == [Sessions { max: 1, left: 1 }]`;
  - tutte le voci hanno `bytes == None`.

  Expected adesso: FAIL, perché la funzione non esiste.
- [ ] **Step 2: Implement** l'esempio e `history_probe`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project probe_runs` e `cargo build -j 1 -p xiox-project --examples`. Expected: PASS e build riuscita.
- [ ] **Step 4: Misura sugli store reali, solo su richiesta esplicita dell'utente.** Non è un prerequisito del merge. Si lancia `session_probe` su un progetto dell'utente, con l'OpenCode nativo in `XIOX_OPENCODE_EXE`. L'uscita resta in una cartella temporanea fuori dal repository. Si riportano tempi, sessioni lette e limiti raggiunti, per tarare `BUDGET`.
- [ ] **Step 5: Aggiorna `AGENTS.md`** con una voce nella sezione dell'estrattore:
  - `analyze` legge la cronologia (piano `docs/superpowers/plans/2026-11-16-history-analysis.md`);
  - i valori di `BUDGET` sono iniziali;
  - `scratch` non si rimuove fino al passo 4;
  - la sonda è `session_probe`.
- [ ] **Step 6: Commit** — `test(xiox-project): sonda session_probe e stato in AGENTS.md`.

---

## Self-review

1. **Copertura della specifica.**

   | Requisito | Dove |
   |---|---|
   | Parte A.1: tutti gli harness, con agente, ID, sorgente, parentela; store assente diverso da store illeggibile | Task 1, poi `listing_failed` nel Task 4 |
   | Parte A.2: `cwd` dei descrittori e cambi di contesto delle sessioni lette | Task 3 e 4 |
   | Parte A.3: filtro prima dell'accesso, `project_roots_from_sessions` | Task 3 |
   | Parte A.4: `classify` | Task 3 |
   | Parte A.5: deduplicazione col walk e con l'indice MFT | Task 3: stessa lista `candidates` |
   | Provenienza: sessioni distinte, ultima attività | Task 3 e 4 |
   | Parte B: sessioni del progetto, dei worktree e delle copie; contesto per chiamata; coda che si allarga; budget visibile | Task 4 |
   | Evidenze e forze, esiti falliti o sconosciuti | `Ledger` in `main`, conservati in `Operation` nel Task 4 |
   | `scratch`: regole, esempio Phazly, cartella coeva | Task 5 |
   | Rischio come indicatori | Task 6 |
   | Controlli 1 (percorsi, link, cloud, dischi fissi) | Task 5 |
   | Controlli 5 (errori distinti) | `ReadFailure`, `Exclusion`, Task 4 e 5 |
   | Controlli 6 (`named` non elimina restrizioni) | `named_blocks` anche su `scratch`, Task 5 |
   | `--no-drives` | Task 3 e 5 |
   | Lettura una alla volta, timeout reale, troncamenti riportati | Task 4 |
   | Ordine per attività | Task 6 |
   | Niente testo delle chat né degli errori | Task 2 e 4 |
   | Sonda | Task 9 |

   Casi di accettazione coperti:

   | Caso | Task |
   |---|---|
   | Copia profonda nel `cwd` | 3 |
   | Tool che crea fuori dal progetto | 5 e 7 |
   | Cartella coeva | 5 |
   | Timestamp NTFS | 6 |
   | Scrittura in preesistente, creazione fallita | 5 e 4 |
   | Cambio di `cwd`, relativi, sessioni figlie | 4 |
   | Lettura parziale o fallita | 4 |
   | UNC e junction | 3 e 5 |

   Gli altri casi della tabella (conferme, `--yes`, contenitori, worktree `danger`, sorgente riscritta, cartella condivisa) sono del passo 4, come dichiarato in testa. Le pagine Codex le legge `mukade dump`; la chiave `(store, harness, id)` separa gli store.
2. **Passi.** Ogni passo di codice nomina file, firma e valori: budget, `RECENT_MS`, `MAX_OPERATIONS_SHOWN` e i testi esatti. Due correzioni fatte qui:
   - il Task 3 ora dice dove si chiama ogni stadio in `owned`;
   - il Task 6 dice che `scratch` si calcola dopo `read`, perché serve `partial`.

   Altre tre correzioni:
   - il test della parentela nel Task 1 ora dice come confermare la disposizione dello store, visto che `vendor/mukade/` non ha documentazione;
   - il test di `--no-drives` nel Task 3 ora prova l'assenza di accessi al disco con un `missing` assente;
   - nella tabella delle cartelle protette del Task 5 i casi rifiutati e quelli ammessi sono separati.
3. **Coerenza dei tipi.**
   - `Descriptor.store` e `Descriptor.harness` (Task 1) sono le chiavi di `Queue` (4) e di `SessionEvidence` (2), uguali a `ToolCall.store`/`harness`: lo prova `descriptor_store_matches_the_tool_call_store`.
   - `ReadFailure` (2) viene prodotto da `FakeCalls` (4) e da `MukadeCalls` (7), che lo ricava da `DumpError` (1).
   - `HistoryInput` (2) è letto in 3, 4 e 7.
   - `Folders::{add_listed, add_ledger, provenance_within}` (3 e 4) sono usati da `attach` (3) e da `scratch_items` (5).
   - `Exclusion` (2) viene contato in 3 e 5.
   - `risks` (6) riempie `Why::SessionFolder.risks` (2).
   - `Options.history` (2) è impostato in 7.
4. **Review Focus.** I cinque punti hanno test nominati nei Task 3, 4 e 5:
   - `cwd_copy_found_by_walk_is_listed_once`;
   - `a_failed_read_is_counted_and_the_rest_is_read`;
   - `same_id_in_two_stores_is_two_sessions`;
   - `junction_or_missing_folder_is_never_entered`;
   - `broad_and_protected_folders_are_never_scratch`.
5. **Proporzione.** Il piano descrive circa 1.000 righe di codice e di test in 9 task, ed è più corto della specifica che implementa. Non contiene corpi di funzione: solo firme, tabelle di casi, valori e testi esatti.

## Execution Handoff

Piano salvato in `docs/superpowers/plans/2026-11-16-history-analysis.md`. Il metodo l'ha già scelto l'utente: **subagent-driven**, con implementatori e revisori Sonnet, un implementatore e un revisore nuovi per task, e una revisione dell'intero ramo alla fine. L'esecuzione parte subito sul ramo `plan/analyze-history`.

**REQUIRED SUB-SKILL:** Use superpowers:subagent-driven-development
