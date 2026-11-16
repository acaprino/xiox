# Selezione e pulizia delle voci della cronologia: piano di implementazione (passo 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** rendere rimovibili, con conferma esplicita per voce, le cartelle che la cronologia delle sessioni ha fatto emergere. Il livello `danger` si calcola nel motore. `xiox clean` lo chiede con `--confirm-danger <path>` o con una risposta al prompt. Ogni protezione si riapplica al momento della rimozione, sulla cronologia riletta. Lo schema JSON passa a 2.

**Architecture:** quattro pezzi, ciascuno uno stadio puro o un piccolo tipo di valore in `crates/xiox-project`.

1. **Guardie dei percorsi** (`history/scope.rs`, `history/scratch.rs`): chiudono i buchi lasciati dal passo 3. Sono gli alias 8.3, i discendenti delle cartelle di sistema e dei profili altrui, gli errori di permesso letti come `Missing`, i `%TEMP%` larghi e le cartelle che contengono repository.
2. **Rischio e livello** (`history/danger.rs`): da provenienza, Git e conflitti fra progetti si ricavano `Item.risks` e il livello `Level::Danger`. Il livello è un tipo di `xiox-project`, non di `xiox-model`.
3. **Ammissione** (`clean/admit.rs`): un'unica funzione decide se una voce può entrare nella pulizia, con le sue conferme e quelle dei discendenti. La chiamano sia `select`, sulla scansione mostrata, sia `clean_with`, sulla scansione fresca. Un `Chosen` fatto a mano non ottiene più di quanto darebbe `select`.
4. **Riconvalida** (`clean.rs`): `clean_with` rilegge la cronologia quando la selezione contiene voci che ne dipendono. Confronta il piano confermato con quello fresco: bersaglio, identità della cartella, contenuti rilevanti, rischi, livello e metodo. A ogni differenza salta la voce.

La CLI aggiunge `--confirm-danger`, il prompt per voce e il metodo di rimozione nel riepilogo e nel JSON. L'app desktop non usa `xiox-project` e non cambia (decisione sotto).

**Tech Stack:** Rust 2024; `serde`, `serde_json`, `anyhow`, `tempfile` e `windows-sys`, già presenti. Nessuna dipendenza nuova. Mukade è l'eseguibile precompilato in `vendor/mukade/`.

**Spec:** `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`, revisione 3. Il piano copre queste sezioni:

- «Rischio, attività e conferma», per intero: livello, testo, conferme CLI, contenitori, riconvalida del piano, Cestino, worktree;
- «La categoria `scratch`»: la policy Git del sottoalbero per i candidati dentro il progetto e il ProjectFolder guard non allentato;
- «Controlli e copertura», punti 2, 3 e 4. I punti 1, 5 e 6 erano del passo 3 e qui si stringono nel Task 1;
- «Lettura, cache e limiti»: «La pulizia rilegge tramite Mukade le evidenze rilevanti e rivaluta il candidato»;
- l'incremento dello schema e le guide dell'«Ordine di implementazione», passo 4.

Il piano precedente è `docs/superpowers/plans/2026-11-16-history-analysis.md`. Ne segue formato, tabella dell'ambito e ambiente.

**Ambito rispetto all'«Ordine di implementazione».**

| Parte della specifica | Stato |
|---|---|
| Passi 1–3: adapter, Parte A, analisi della cronologia, `scratch` mostrato e rifiutato da `select` (`SelectError::ReviewOnly`) | Fatto, in `main` (4345f21) |
| Passo 4: buchi rinviati dalla revisione finale del passo 3 (alias 8.3, discendenti di sistema, permessi sugli antenati, `%TEMP%` larghi, cartelle che contengono repository puliti; `clean_with` senza cronologia) | Questo piano, Task 2 e Task 7. Sono condizione per togliere `ReviewOnly`, che sparisce solo nel Task 6 |
| Passo 4: `Level::Danger`, `risks`, testo `danger: explicit confirmation required`, schema 2 | Task 1 (forma e schema) e Task 5 (calcolo) |
| Passo 4: policy Git del sottoalbero per i candidati dentro il progetto | Task 3 |
| Passo 4: conflitti fra progetti (punto 4 dei controlli) | Task 4 |
| Passo 4: selezione, conferme, propagazione ai contenitori, rifiuto di `--permanent`, eccezione dei worktree | Task 6 |
| Passo 4: riconvalida alla rimozione, con cronologia riletta e identità della cartella | Task 7 |
| Passo 4: CLI (`--confirm-danger`, prompt, riepilogo e JSON del metodo) | Task 8 |
| Passo 4: osservatore e MCP sullo schema 2; guide | Task 9 |
| Passo 4: UI | Nessun codice, vedi «Decisione sulla UI» |
| Passo 5: verifica integrata dei casi di accettazione su Windows, osservatore con cronologia, misure su un campione nuovo (D10–D13 e le lacune note) | Fuori |

**Decisione sullo schema JSON: `"schema": 2`.** Cambiano significati, non solo campi:

- il livello `danger`, che nessuna categoria seleziona;
- la selezione, che per le voci `danger` vuole una conferma;
- `scratch`, che ora si può rimuovere;
- `risks`, che esce da `why` e si sposta su ogni voce.

La regola della guida, «un significato cambiato incrementa lo schema», lo impone. Le modifiche:

| Campo | Schema 1 | Schema 2 |
|---|---|---|
| `items[].level` | `recommended`, `optional`, `caution` | in più `danger` |
| `items[].risks` | non c'è; `why.risks` solo in `session-folder` | `items[].risks`, presente su ogni voce, `[]` se non ce ne sono; `why.risks` tolto |
| `items[].blocked` | — | nuovi valori `other-project-writes`, `holds-repositories`, `tracked-files` |
| `items[].target` | `dir`, `file`, `worktree`, `session` | in più `subtree { repo }`: una cartella `scratch` dentro il progetto o un worktree |
| `items[].provenance.otherProjects` | non c'è | le sessioni non del progetto che hanno scritto nella cartella |
| `history.conflicts` | non c'è | `{ read, unread }`: sessioni lette e non lette per cercare scritture di altri progetti |
| `history.excluded` | — | nuove chiavi `short-name` e `unreadable` |
| `clean --json`: `chosen.confirmed` | non c'è | le chiavi `danger` confermate |
| `clean --json`: `outcome.items[]` | `[key, path, category, outcome]` | oggetto `{ key, path, category, method, outcome }`; `method` è `recycle-bin`, `permanent`, `git-worktree-remove` o `harness-session` |
| `schema` in testa a `clean --json` | il letterale `1` in `crates/xiox-cli/src/json.rs:46` | `SCHEMA` |
| eventi di `xiox observe` e `structuredContent` di MCP | `SCHEMA` = 1 | `SCHEMA` = 2; nessun campo nuovo oltre quelli delle voci |

Migrazione dei consumatori:

- **CLI JSON e script:** leggono `schema` e rifiutano o adattano. La guida ha la tabella sopra.
- **Osservatore e MCP:** passano `SCHEMA` senza logica propria. Il `cleanCommand` prende solo le voci `recommended`, quindi non contiene mai voci `danger`; un test lo fissa (Task 9).
- **Desktop:** non legge questo JSON, quindi niente da migrare.

**Decisione sulla UI: nessun codice nel desktop in questo passo.** Lo dice la ricognizione di `apps/desktop`:

- l'app non dipende da `xiox-project` (`apps/desktop/src-tauri/Cargo.toml`, `crates/xiox-core/Cargo.toml`);
- la sua sezione Developers pulisce progetti, worktree e sessioni archiviate tramite `xiox_app::{run_scan, build_plan, execute_plan}`;
- nessuna voce `scratch`, nessuna provenienza e nessun `Level::Danger` può arrivarci.

La specifica chiede alla UI la conferma esplicita delle voci `danger`. Quel requisito vale quando la UI mostra voci `danger`, e oggi non ne mostra. Per questo `danger` non entra in `xiox_model::Level`. Se ci entrasse, i match esaustivi di `crates/xiox-core/src/dev/view.rs:535` (`level_view`), `dev/mod.rs:1180` (`level_word`), `apps/desktop/src/types.ts:6` (`LevelView`), `src/mockmodel.ts`, `src-tauri/src/fake.rs` e `fixtures/views.json` dovrebbero imparare un valore che nessun codice del desktop produce.

Il Task 9 scrive il confine nella guida: le voci della cronologia si rimuovono solo da `xiox clean`. Il dialogo Developers ha oggi un solo «I have read the warnings» per tutte le voci (`dev/mod.rs:881-883`). Il giorno in cui il desktop adotterà l'analisi di `xiox-project`, dovrà avere una conferma per voce. Il seguito resta annotato per quel giorno, non per il passo 5.

## Global Constraints

Dalla specifica, alla lettera:

- «`danger` è un avviso di rischio con conferma esplicita. La conferma può arrivare da una persona nella UI, dalla CLI o da un agente, anche per conto dell'utente.» «Non si cerca di certificare che il confermante sia umano.»
- «Sono `danger` le voci senza Git, quelle con evidenza debole, quelle con copertura insufficiente per valutare un rischio rilevante e quelle create/modificate o usate da sessioni nelle ultime 24 ore. Le date del filesystem possono contribuire a questo indicatore di recenza, mai alla provenienza.»
- «L'attività recente osservata dentro un sottoalbero si riflette sul suo contenitore; il solo `mtime` della directory non descrive tutte le modifiche dei figli.»
- «Il testo è `danger: explicit confirmation required`, con motivi leggibili e strutturati.»
- «Recenza e vitalità sconosciuta non sono nuovi blocchi assoluti: il rischio è mostrato e la scelta resta a chi conferma. I blocchi già certi del motore, compresa una sessione verificata viva, restano validi.»
- «Una categoria non seleziona implicitamente voci `danger`.»
- «La CLI richiede `--include <path>` e una conferma per quella voce: risposta al prompt oppure `--confirm-danger <path>`. Il parametro è valido anche per un agente; `--yes` da solo non basta.»
- «Il percorso confermato deve essere un candidato dell'analisi. Non abilita la rimozione di percorsi arbitrari e non supera i blocchi del motore.»
- «La rimozione di un contenitore deve rispettare i blocchi e le conferme richieste anche dai candidati discendenti. Non è possibile eluderli includendo solo il genitore.»
- «Se la riconvalida cambia bersaglio, contenuti rilevanti, rischio o metodo rispetto al piano confermato, la voce viene saltata e richiede una nuova scelta.»
- «Cartelle ordinarie `danger`: Cestino. Una richiesta `--permanent` che le includa viene rifiutata prima della rimozione; non si cambia metodo in silenzio.»
- «Si mantiene `git worktree remove`, mai `--force`, anche quando il worktree è `danger`. La UI, il riepilogo CLI e il JSON dichiarano che la rimozione è definitiva e non passa dal Cestino. La conferma include questa informazione.»
- «Non si propone come `scratch` una radice di disco, una cartella protetta o larga, il progetto stesso, un suo worktree o un loro contenitore. Per un candidato dentro il progetto serve una verifica Git del sottoalbero: contenuti tracciati, controllo impossibile o repository annidati non rimovibili ne impediscono la rimozione. Serve una policy dedicata; non si allenta il guard esistente di `ProjectFolder` per tutte le categorie.»
- «esistenza, identità, contenuti Git e blocchi vengono ricontrollati prima della rimozione. Il riferimento storico non autorizza automaticamente un altro oggetto comparso in seguito allo stesso percorso.»
- «lavoro Git non salvato, repository illeggibili, cartelle protette e scansioni del sottoalbero incomplete impediscono la rimozione secondo la policy applicabile.»
- «scritture di altri progetti documentate dalla cronologia sono riportate e bloccano la pulizia della cartella condivisa come residuo esclusivo di un progetto.» «Non leggere tutte le sessioni non dimostra assenza di conflitti: la copertura parziale è indicata come rischio.»
- «Non si riutilizza una prova obsoleta come se fosse stata appena riconfermata.»
- «Diventare `named` invece di `scratch` non elimina una restrizione applicabile.»
- «La pulizia rilegge tramite Mukade le evidenze rilevanti e rivaluta il candidato. La sola coincidenza di percorso, dimensione o prefisso del transcript non certifica l'attualità.»
- «controllo degli antenati dall'alto prima di attraversarli, senza seguire link/junction o accedere a cartelle cloud.»
- «I testi degli errori non devono riversare contenuto delle sessioni.»
- «I test usano repository e store finti sotto `tempfile::tempdir()`. `cargo` gira con `-j 1`; non si avviano build concorrenti. Non si eseguono pulizie su progetti reali.»

Da `AGENTS.md` e dall'ambiente:

- Codice organico, pulito ed elegante, con un'architettura studiata. Si usano stadi puri, piccoli tipi di valore, strategy e registry dove migliorano la lettura. Nessuna astrazione senza un secondo uso o un beneficio di lettura evidente. Tutto ciò che entra in un crate ha moduli progettati e test guidati dai dati.
- I comandi reali delle sessioni non entrano mai nel repository. Fixture e store dei test sono solo sintetici. Nessun test legge gli store reali di questo PC. Il database di OpenCode si legge solo tramite `MukadeSessions::call_reader()`.
- Le divergenze D1–D14 del port e `crates/xiox-project/src/provenance/` non si toccano. Una regola nuova dell'estrattore non entra senza un campione nuovo etichettato alla cieca, e questo piano non ne aggiunge.
- Inglese in ogni stringa visibile a utente o programma; `scripts/italian-check.mjs` resta pulito.
- Ambiente di ogni comando cargo: `CARGO_INCREMENTAL=0`, `TEMP` e `TMP` uguali a `D:\Projects\xiox\target\tmp`. Sempre `-j 1`, un solo processo cargo alla volta, mai `cargo fmt`. Forma completa:
  `CARGO_INCREMENTAL=0 TEMP='D:\Projects\xiox\target\tmp' TMP='D:\Projects\xiox\target\tmp' cargo test -j 1 -p <crate> <filtro>`.
  Nei passi si scrive solo `cargo test -j 1 -p <crate> <filtro>`; l'ambiente vale sempre.
- Junction nei test: si creano con `mklink /J`, come in `search.rs::junction_with_a_matching_name_is_ignored`. Se la creazione fallisce, il test stampa `skipped` ed esce.
- Comportamento invariato dove non è detto altro: fanno da guardia i test esistenti di `analyze`, `clean`, `observe`, MCP e `history`. Ogni task finisce con `cargo test -j 1 -p xiox-project` verde, più `-p xiox-cli` e `-p xiox-observer` quando li tocca.
- Commit: solo i file del task, messaggio in italiano, che termina con
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Un contenitore incluso al posto della voce `danger`.** `--include D:\agent-runs`, dove `agent-runs` è una voce `named` che contiene lo `scratch` `danger` `bench-1`, con `--confirm-danger` solo sul genitore o senza conferme, non rimuove nulla. Lo stesso vale con un `Chosen` costruito a mano che passa direttamente a `clean_with`. Test: `container_needs_the_confirmations_of_its_descendants` (Task 6) e `hand_made_chosen_gets_no_more_than_select` (Task 7).
2. **La cartella confermata è stata sostituita.** Fra analisi e rimozione la cartella viene cancellata e ricreata con lo stesso nome, oppure sostituita da una junction, oppure le compare dentro un `.git`. La voce viene saltata con un motivo, e nulla di nuovo viene rimosso. Test: `replaced_folder_is_skipped_by_identity` e `a_changed_risk_needs_a_new_choice` (Task 7).
3. **Lo stesso percorso in forma breve.** Un'evidenza su `C:\Users\ALFIOC~1\x` o `C:\PROGRA~1\App\data` non aggira le guardie per chiave della home, di `%TEMP%` o di sistema. Test: `short_names_never_slip_past_the_guards` (Task 1).
4. **Una lettura negata a metà strada.** Un antenato che dà `PermissionDenied` non diventa `Missing`. Una cartella che non si può elencare non è mai «verificata vuota» e non si rimuove. Test: `permission_denied_is_not_missing` (Task 1).
5. **Solo `--yes`, oppure la conferma di un altro percorso.** Né `--yes` da solo né `--confirm-danger` su una voce non inclusa rimuovono la voce `danger`. Lo stesso vale per `--json` senza conferme e per uno stdin che non è un terminale. Test: `yes_alone_or_another_path_removes_nothing` (Task 8).

---

## File Structure

| File | Responsabilità |
|---|---|
| `crates/xiox-project/src/level.rs` | `Level` del progetto (`Recommended`, `Optional`, `Caution`, `Danger`), `From<xiox_model::Level>`, `DANGER_TEXT` |
| `crates/xiox-project/src/history/scope.rs` | `long_path`/`has_short_component`, `unlinked_chain` con `Exclusion::Unreadable` |
| `crates/xiox-project/src/history/scratch.rs` | `Policy`: profili altrui, discendenti di sistema, `%TEMP%` larghi, candidati dentro il progetto |
| `crates/xiox-project/src/history/subtree.rs` | Policy Git del sottoalbero per i candidati dentro il progetto |
| `crates/xiox-project/src/history/conflicts.rs` | Ricerca delle scritture di altri progetti sui candidati, entro il budget |
| `crates/xiox-project/src/history/danger.rs` | `item_risks` per categoria, `level_for`, propagazione al contenitore |
| `crates/xiox-project/src/history/report.rs` | `Risk::reason`, `Exclusion::Unreadable`, `ConflictReport` |
| `crates/xiox-project/src/identity.rs` | `FolderId` e `folder_id(path)`: volume e file id della cartella, senza seguire link |
| `crates/xiox-project/src/clean.rs` → `crates/xiox-project/src/clean/{mod.rs,admit.rs,method.rs}` | `clean.rs` passa da file a cartella. `admit.rs` contiene l'ammissione condivisa da `select` e `clean_with`; `method.rs` contiene `Method` e `method_of`; `mod.rs` tiene `Confirmation`, `Chosen`, `OutcomeItem`, la riconvalida e la rimozione |
| `crates/xiox-project/src/{lib.rs,owned.rs,search.rs}` | `Item.risks`, `Item.identity`, `Target::Subtree`, `Block` nuovi, `SCHEMA = 2`, cablaggio di `conflicts` e `danger` |
| `crates/xiox-git/src/lib.rs` | `tracked_under`: file tracciati sotto una cartella |
| `crates/xiox-cleaner/src/guard.rs` | `in_system_area`, la regola del primo componente di sistema, condivisa con `scratch` |
| `crates/xiox-project/src/tests/{clean.rs,history/*.rs}` | Test guidati da tabelle, con `CallSource` finta e alberi in `tempdir` |
| `crates/xiox-cli/src/{args.rs,run.rs,render.rs,json.rs}` | `--confirm-danger`, prompt per voce, metodo nel riepilogo e nel JSON, schema |
| `crates/xiox-cli/src/{observe.rs,mcp.rs}`, `crates/xiox-observer/src/state.rs` | Schema 2 nei test; `cleanCommand` senza voci `danger` |
| `docs/xiox-cli.md`, `docs/xiox-observe.md`, `README.md`, `AGENTS.md` | Guida, schema 2 e stato |

---

## Tasks

### Task 1: La forma del risultato, il livello `danger` e lo schema 2

Tutti i tipi nuovi entrano qui, insieme all'aggiornamento meccanico dei letterali e dei test che fissano `schema: 1`. I task successivi toccano solo la logica. Nessun comportamento cambia ancora: nessuna voce diventa `Danger` e `select` rifiuta ancora `scratch`.

**Files:**
- Create: `crates/xiox-project/src/level.rs`, `crates/xiox-project/src/identity.rs`, `crates/xiox-project/src/tests/identity.rs`
- Modify:
  - `crates/xiox-project/src/lib.rs`: `mod level; mod identity;`, `pub use level::{Level, DANGER_TEXT}`, `pub use identity::FolderId`, `SCHEMA = 2`, `Item.level: Level`, `Item.risks`, `Item.identity`, `Target::Subtree`, i tre `Block` nuovi, `Why::SessionFolder` senza `risks`.
  - `crates/xiox-project/src/history/report.rs`: `Risk::reason`, `Exclusion::{ShortName, Unreadable}`, `ConflictReport`, `HistoryReport.conflicts`, `Provenance.other_projects`.
  - `crates/xiox-project/src/history/scratch.rs`: `risks` va in `Item.risks` invece che in `why`.
  - `crates/xiox-project/src/{owned.rs,search.rs,logs.rs}`: `Level::from(a.level)` dove oggi passa un `xiox_model::Level`.
  - `crates/xiox-cli/src/render.rs:44-46`: il match su `Level` con il ramo `Danger`, che stampa `danger`.
  - `crates/xiox-cli/src/json.rs:46`: `to_value(SCHEMA)` al posto del letterale `1`; doc a `json.rs:1,5,13` e help di `--json` in `args.rs:80` con «schema 2».
  - Letterali di `Item`, `Provenance` e `HistoryReport` nei test, con `risks: vec![]`, `identity: None`, `other_projects: vec![]` e `conflicts: ConflictReport::default()`. I file sono `crates/xiox-project/src/tests/**`, `crates/xiox-cli/src/{observe.rs,testdata.rs}` e `crates/xiox-observer/src/tests/fixtures.rs`.
  - I test che scrivono `1` per lo schema passano a `SCHEMA`: `crates/xiox-cli/src/json.rs:62,77,78,113,121`, `mcp.rs:734`, `observe.rs:966,1201`, `run.rs:501`, `render.rs:826`, `crates/xiox-cli/tests/cli.rs:87,321,339,563,579`, `crates/xiox-project/src/tests/clean.rs:278,907`, `tests/history/json.rs:100`, `tests/owned.rs:74`. Dove il test confronta testo JSON, il valore atteso diventa `2`.

**Interfaces:**
- Consumes: `xiox_model::Level`, `Risk`, `Provenance`, `HistoryReport` di oggi.
- Produces:
  - `pub enum Level { Recommended, Optional, Caution, Danger }` in `level.rs`, con `#[serde(rename_all = "kebab-case")]`, `Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize` e `impl From<xiox_model::Level> for Level`. È lo stesso ordine di `xiox_model::Level`, con `Danger` in coda.
  - `pub const DANGER_TEXT: &str = "danger: explicit confirmation required";`
  - `impl Risk { pub fn reason(self) -> &'static str }`, con questi testi esatti:
    - `NoGit` → `"not a git repository"`;
    - `WeakEvidence` → `"the sessions wrote or worked here; no creation is recorded"`;
    - `PartialCoverage` → `"the session history was read only in part"`;
    - `RecentActivity` → `"used or changed in the last 24 hours"`.
  - `Item`:
    - `pub level: Level`;
    - `pub risks: Vec<Risk>`, in ordine di `Risk`, senza ripetizioni;
    - `#[serde(skip)] pub identity: Option<FolderId>`.
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FolderId { volume: u64, file: [u8; 16] }`.
  - `pub(crate) fn folder_id(path: &Path) -> Option<FolderId>`. Su Windows apre la cartella con `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`, così non segue i link, e legge `FileIdInfo` con `GetFileInformationByHandleEx`, come `crates/xiox-cleaner/src/fsops.rs:77-96`. Altrove dà `None`.
  - `Block::OtherProjectWrites` → `"sessions of another project wrote here"`; `Block::HoldsRepositories` → `"holds git repositories"`; `Block::TrackedFiles` → `"holds files tracked by git"`.
  - `Target::Subtree { repo: PathBuf }`: una cartella dentro il progetto o un worktree; `repo` è quel repository. In JSON vale `{"type": "subtree", "repo": …}`.
  - `Exclusion::ShortName` e `Exclusion::Unreadable`, in coda all'enum.
  - `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)] pub struct ConflictReport { pub read: usize, pub unread: usize }` e `HistoryReport.conflicts: ConflictReport`.
  - `Provenance.other_projects: Vec<SessionEvidence>`, in JSON `otherProjects`.

- [ ] **Step 1: Write the failing tests** in `crates/xiox-project/src/tests/history/json.rs`:
  - `schema_2_moves_risks_to_the_item`: una voce `scratch` serializzata ha `"risks": ["no-git"]` sulla voce e nessun `risks` dentro `why`. `"schema"` vale `2`.
  - `danger_level_and_new_blocks_are_kebab_case`: `Level::Danger` dà `"danger"`; `Block::OtherProjectWrites`, `HoldsRepositories` e `TrackedFiles` danno `"other-project-writes"`, `"holds-repositories"` e `"tracked-files"`; `Target::Subtree` dà `{"type":"subtree","repo":…}`.
  - `identity_never_reaches_the_json`: una voce con `identity: Some(_)` non ha la chiave `identity`.
  - `risk_reasons_are_the_guide_texts`: tabella `(Risk, testo)` con i quattro testi sopra.
- [ ] **Step 2: Write the failing test** `folder_id_is_stable_and_changes_when_recreated` in `crates/xiox-project/src/tests/identity.rs`, con `#[cfg(windows)]`:
  - due letture della stessa cartella danno lo stesso `FolderId`;
  - cancellata e ricreata con lo stesso nome, dà un `FolderId` diverso;
  - una junction verso la cartella dà l'id della junction, non quello del bersaglio. Se la junction non si crea, il caso stampa `skipped`.
- [ ] **Step 3: Run** `cargo test -j 1 -p xiox-project history::json` e `cargo test -j 1 -p xiox-project identity`. Expected: FAIL, perché i tipi non esistono.
- [ ] **Step 4: Implement** i tipi e gli aggiornamenti meccanici sopra. `scratch_items` mette i suoi `risks` in `Item.risks`. Le altre voci hanno `risks: vec![]`. `item_for` e `worktrees` riempiono `identity` con `folder_id`.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project`, `cargo test -j 1 -p xiox-cli` e `cargo test -j 1 -p xiox-observer`. Expected: PASS.
- [ ] **Step 6: Commit** — `feat(xiox-project): schema 2, livello danger, rischi sulla voce e identità delle cartelle`.

---

### Task 2: Le guardie dei percorsi rinviate dal passo 3

Questi buchi vanno chiusi prima che `scratch` diventi rimovibile. Tutte le regole valgono due volte: nell'analisi e, attraverso l'analisi fresca del Task 7, al momento della rimozione.

**Files:**
- Modify:
  - `crates/xiox-project/src/history/scope.rs`: `has_short_component`, `long_path`, `unlinked_chain`;
  - `crates/xiox-project/src/history/mod.rs`: `UserDirs::from_env` e il campo `resolved`;
  - `crates/xiox-project/src/history/scratch.rs`: `Policy::{new, admit, protected, on_disk}`;
  - `crates/xiox-project/src/search.rs`: `named_blocks`;
  - `crates/xiox-cleaner/src/guard.rs`: `pub fn in_system_area(path: &Path) -> bool`, sopra `system_first_component`;
  - `crates/xiox-project/src/clean.rs`: `check_dir`.
- Test: `crates/xiox-project/src/tests/history/scratch.rs`, `crates/xiox-project/src/tests/history/scope.rs`, `crates/xiox-cleaner/src/guard.rs` (modulo di test esistente).

**Interfaces:**
- Consumes: `Exclusion::{ShortName, Unreadable}`, `Block::HoldsRepositories` (Task 1); `nested_repos` (`crates/xiox-project/src/nested.rs`).
- Produces:
  - `pub(crate) fn has_short_component(path: &Path) -> bool`. È vero quando un componente ha la forma di un alias 8.3: almeno un carattere, poi un `~`, poi una o più cifre, poi la fine del nome o un `.`. Esempi: `ALFIOC~1`, `PROGRA~1`, `LONGNA~12.TXT`. Il controllo è lessicale, in un solo passaggio sui byte.
  - `pub(crate) fn long_path(path: &Path) -> Option<PathBuf>`. Su Windows chiama `GetLongPathNameW`; dà `None` se fallisce o se il risultato ha ancora un componente breve. Si usa solo sulle cartelle dell'utente lette dall'ambiente, mai sui percorsi della cronologia.
  - `unlinked_chain(path) -> Result<(), Exclusion>`: `NotFound` dà `Missing`, ogni altro errore dà `Unreadable`.
  - `UserDirs { home, temp, resolved: bool }`. `from_env` passa `home` e `temp` per `long_path`. Quando uno dei due resta breve o non risolto, `resolved` vale `false`. Le `UserDirs` finte dei test hanno `resolved: true`.
  - `pub fn in_system_area(path: &Path) -> bool` in `guard.rs`: il primo componente sotto il disco è di sistema secondo `system_first_component`. `lexical_refusal` lo usa al posto del confronto inline.
  - `named_blocks(dir)` aggiunge `Block::HoldsRepositories` quando `nested_repos(dir)` trova almeno un repository. Il `.git` della cartella stessa non conta. Vale per `named` e per `scratch` («Diventare `named` invece di `scratch` non elimina una restrizione applicabile»).
- Regole nuove di `Policy::admit`, in quest'ordine, prima delle regole esistenti:
  1. `has_short_component(raw)` dà `ShortName`. È lessicale e non accede al disco.
  2. `!user.resolved` dà `Protected` per ogni candidato. Con una home o un `%TEMP%` non confrontabili per chiave, nessuna guardia per chiave è affidabile, quindi si chiude tutto.
  3. `in_system_area(path)` dà `Protected` anche per i discendenti, per esempio `C:\Windows\Temp\x` e `C:\Program Files\App\data`.
  4. `<disco>:\Users\<nome>\…`, con `<nome>` diverso dall'ultimo componente della home, dà `Protected`. Vale anche per `Public` e `Default`.
- Regola nuova di `%TEMP%` in `Policy::new`. Il `%TEMP%` esenta i propri figli dalla regola delle cartelle larghe solo in due casi:
  - è esattamente `<home>\AppData\Local\Temp`;
  - sta fuori da `<home>\AppData`, non è largo (`is_broad_dir`) e non è una radice di disco.

  Un `%TEMP%` uguale a `<home>\AppData\Local`, una radice di disco o la home non esenta nulla. `in_home_app_data` sparisce.
- `on_disk` distingue i casi: una `symlink_metadata` con `NotFound` dà `Missing`, ogni altro errore dà `Unreadable`.
- `check_dir` aggiunge due motivi:
  - `has_short_component(path)` → `"its path uses a short (8.3) name: not followed"`;
  - `unlinked_chain` con `Unreadable` → `"cannot access one of the folders above it"`, invece di `"no longer there"`.

- [ ] **Step 1: Write the failing tests** in `tests/history/scope.rs`:
  - `short_components_are_recognised`: tabella `(nome, atteso)`. Sono veri `ALFIOC~1`, `PROGRA~1`, `LONGNA~12.TXT` e `A~1`. Sono falsi `a~b`, `~1`, `report~final`, `x~`, `name~1x` e `~`.
  - `permission_denied_is_not_missing` (Review Focus 4). Una cartella `<base>\locked\child` riceve sul genitore `locked` un ACL che nega la lettura a Everyone, con `icacls <base>\locked /deny *S-1-1-0:(RX)`. Il risultato:
    - `unlinked_chain(child) == Err(Exclusion::Unreadable)`;
    - una voce `scratch` creata dalla sessione lì dentro è esclusa come `unreadable`, non come `missing`;
    - a fine test l'ACL si ripristina con `icacls … /remove:d *S-1-1-0`.

    Se `icacls` non nega l'accesso, il test stampa `skipped`. Succede quando gira da amministratore.
- [ ] **Step 2: Write the failing tests** in `tests/history/scratch.rs`:
  - `short_names_never_slip_past_the_guards` (Review Focus 3). Tabella di chiamate che non danno nessuna voce, con il motivo atteso in `excluded`:
    - Write in `<drive>:\Users\ALFIOC~1\x\a.txt`: `short-name`;
    - `mkdir <base>\PROGRA~1\App\data`: `short-name`;
    - `UserDirs` con `resolved: false` e una scrittura in `<base>\runs\a`: `protected`.

    Il caso `resolved: false` nasce da `UserDirs::from_env` con `USERPROFILE` breve e inesistente. Lo prova un secondo test, `unresolvable_short_home_is_not_resolved`.
  - `system_descendants_and_other_profiles_are_protected`. Tabella di percorsi, ognuno `protected` secondo `Policy::admit` con `home = C:\Users\me`. Le regole 3 e 4 sono lessicali, quindi i percorsi non serve crearli:
    - `C:\Windows\Temp\x`;
    - `C:\Program Files\App\data`;
    - `C:\ProgramData\x`;
    - `C:\Users\other\work`;
    - `C:\Users\Public\x`.

    Il caso ammesso è `<base>\Users\me\work\run1`, creato in `tempdir` con `home = <base>\Users\me`: passa le regole lessicali e quelle sul disco.
  - `broad_temps_exempt_nothing`. Tabella `(TEMP, cartella, atteso)`:
    - `TEMP=<home>\AppData\Local` e `<home>\AppData\Local\Microsoft\x`: `protected`;
    - `TEMP=<home>\AppData\Local\Temp` e `<home>\AppData\Local\Temp\run1`: ammesso;
    - `TEMP=<drive>:\` e `<drive>:\`: `protected`;
    - `TEMP=<drive>:\` e `<home>\AppData\x`: `protected`;
    - `TEMP=<base>\tmp`, fuori dalla home, e `<base>\tmp\run1`: ammesso.
  - `a_written_folder_holding_clean_repos_is_blocked`. Una sessione scrive `<base>\work\notes.txt`; sotto `work` ci sono due repository git puliti, `work\a` e `work\b`. Il risultato è una voce `scratch` `work` con `blocked == [HoldsRepositories]`.
- [ ] **Step 3: Write the failing test** `system_area_is_the_first_component_rule` nel modulo di test di `guard.rs`. Sono veri `C:\Windows\Temp\x`, `D:\Program Files (x86)\a\b`, `C:\ProgramData\x` e `C:\$Recycle.Bin\x`. Sono falsi `C:\Projects\windows`, `D:\agent-runs\x` e `C:\Users\me\Program Files`.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project history::scope`, `cargo test -j 1 -p xiox-project history::scratch` e `cargo test -j 1 -p xiox-cleaner system_area`. Expected: FAIL.
- [ ] **Step 5: Implement** le funzioni e le regole sopra.
- [ ] **Step 6: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-cleaner`. Expected: PASS. In `broad_and_protected_folders_are_never_scratch` i casi ammessi `mkdir ~\x` e `temp\run1` restano ammessi. Un test esistente di `named` che contiene un repository pulito ora vede `HoldsRepositories`: si aggiorna, e il rapporto del task lo nomina.
- [ ] **Step 7: Commit** — `fix(xiox-project): alias 8.3, discendenti di sistema, profili altrui, permessi e TEMP larghi nelle guardie di scratch`.

---

### Task 3: I candidati dentro il progetto e la policy Git del sottoalbero

Oggi un candidato dentro il progetto o un worktree è contato come `inside-project` e scartato. La specifica chiede invece una verifica Git del sottoalbero, con una policy dedicata e senza allentare il guard di `ProjectFolder`. Per questo nasce `Target::Subtree`, un bersaglio distinto: il controllo `overlaps_project` di `check_dir` resta com'è per tutte le altre voci.

**Files:**
- Create: `crates/xiox-project/src/history/subtree.rs`, `crates/xiox-project/src/tests/history/subtree.rs`
- Modify:
  - `crates/xiox-git/src/lib.rs`: `tracked_under`, accanto a `working_tree_dirty`;
  - `crates/xiox-project/src/history/scratch.rs`: `Policy::admit` restituisce il bersaglio, e `scratch_items` lo usa;
  - `crates/xiox-project/src/clean.rs`: `check_dir` e `check_contents` con il ramo `Target::Subtree`.

**Interfaces:**
- Consumes: `Target::Subtree`, `Block::{TrackedFiles, HoldsRepositories}` (Task 1); `named_blocks` (Task 2); `Own` (`search.rs`).
- Produces:
  - `pub fn tracked_under(repo: &Path, dir: &Path) -> Option<bool>` in `xiox-git`. Esegue `git -C <repo> ls-files -z --cached -- <dir relativo a repo>`. Dà `Some(true)` se l'output non è vuoto, `Some(false)` se lo è, `None` se git fallisce o non risponde.
  - `pub(crate) fn subtree_blocks(repo: &Path, dir: &Path) -> Vec<Block>`: `TrackedFiles` per `Some(true)`, `UnsavedWorkUnknown` per `None`, più `named_blocks(dir)`.
  - `Policy::admit(&self, raw: &Path) -> Result<(PathBuf, Target), Exclusion>`:
    - un candidato fuori dal progetto dà `Target::Dir { kind: ProjectFolder }`, come oggi;
    - un candidato strettamente dentro una radice propria dà `Target::Subtree { repo }`, dove `repo` è la radice propria più interna che lo contiene.
  - Regole dei candidati interni, al posto di `InsideProject`:
    1. il progetto o un worktree stesso dà `InsideProject`;
    2. un componente `.git` nel percorso dà `Protected`;
    3. un contenitore di un worktree (`Own::inside`) dà `Protected`, come oggi;
    4. le regole sul disco restano: link, `Missing`, `Unreadable`. `inside_other_repo` non si applica, perché l'antenato repository è il progetto stesso.
  - `scratch_items` calcola `blocked` con `subtree_blocks(repo, dir)` per i `Subtree` e con `named_blocks(dir)` per gli altri. `LiveSession` resta in entrambi i casi.
  - `check_dir` per `Target::Subtree { repo }` rifiuta in questi casi:
    - `repo` non è la radice del progetto fresco né uno dei suoi worktree → `"its repository is no longer the project or one of its worktrees"`;
    - il percorso non è strettamente dentro `repo`, oppure contiene una radice propria → `"it overlaps the project or one of its worktrees"`;
    - un componente `.git` → `"it is inside the git folder"`.

    Altrimenti dà un `PlanItem` con `kind: ArtifactKind::Other` e `PlanAction::RemoveDir`. `guard::validate` resta l'ultima parola.
  - `check_contents` applica ai `Subtree` lo stesso controllo dei repository annidati che oggi fa per `ProjectFolder`.

- [ ] **Step 1: Write the failing tests** in `tests/history/subtree.rs`, tabella `(nome, preparazione, atteso)` su un progetto git in `tempdir` con un commit:
  - `<proj>\tmp\run1`, creata dalla sessione e non tracciata: `Scratch`, `Target::Subtree { repo: <proj> }`, `blocked == []`;
  - `<proj>\bench`, creata dalla sessione, con un file aggiunto e committato: `blocked == [TrackedFiles]`;
  - `<proj>\vendor\lib`, scritta dalla sessione, che contiene un repository git: `blocked` contiene `HoldsRepositories`;
  - un file scritto in `<proj>\notes.txt`, la cui cartella è il progetto: nessuna voce, `excluded["inside-project"] == 1`;
  - `<proj>\.git\x`: nessuna voce, `protected`;
  - `<proj>\wt-parent`, che contiene un worktree registrato: nessuna voce, `protected`;
  - `<proj>\tmp\run2` con `<proj>\.git\HEAD` sovrascritto da testo non valido, così che git non risponde: `blocked` contiene `UnsavedWorkUnknown`.
- [ ] **Step 2: Write the failing test** `check_dir_keeps_subtrees_inside_their_repo` in `crates/xiox-project/src/tests/clean.rs`. È una tabella di `Item` costruiti a mano:
  - un `Subtree` con un `repo` che non è il progetto: rifiutato;
  - un `Subtree` fuori da `repo`: rifiutato;
  - un `Subtree` sotto `<proj>\.git`: rifiutato;
  - un `Dir { ProjectFolder }` dentro il progetto: rifiutato come oggi, con `"it overlaps the project or one of its worktrees"`;
  - un `Subtree` in `<proj>\tmp\run1`: accettato.
- [ ] **Step 3: Write the failing test** `tracked_under_reads_the_index` nel modulo di test di `xiox-git`. Una cartella con un file committato dà `Some(true)`; una non tracciata dà `Some(false)`; un `repo` che non è un repository dà `None`.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project history::subtree`, `cargo test -j 1 -p xiox-project check_dir_keeps` e `cargo test -j 1 -p xiox-git tracked_under`. Expected: FAIL.
- [ ] **Step 5: Implement** come sopra.
- [ ] **Step 6: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-git`. Expected: PASS. Il caso `<progetto>\tmp\a.txt: inside-project` di `broad_and_protected_folders_are_never_scratch` diventa una voce `Subtree`: il caso si sposta nella tabella di questo task.
- [ ] **Step 7: Commit** — `feat(xiox-project): candidati scratch dentro il progetto con la policy Git del sottoalbero`.

---

### Task 4: Conflitti fra progetti

Una cartella dove hanno scritto sessioni che non sono del progetto non è un residuo esclusivo del progetto: si blocca. Si leggono, entro lo stesso budget dell'analisi, le sessioni non del progetto che hanno lavorato vicino ai candidati. Quelle che il budget lascia fuori rendono la copertura del candidato parziale.

**Files:**
- Create: `crates/xiox-project/src/history/conflicts.rs`, `crates/xiox-project/src/tests/history/conflicts.rs`
- Modify:
  - `crates/xiox-project/src/history/mod.rs`: `History` conserva `related`, il contatore della lettura e `unchecked`; nuovo metodo `conflicts`;
  - `crates/xiox-project/src/history/queue.rs`: `Queue::related()` restituisce anche le chiavi;
  - `crates/xiox-project/src/owned.rs`: chiama `conflicts` dopo `attach`;
  - `crates/xiox-project/src/tests/history/fake.rs`: `FakeCalls::read_ids()`, se la finta non registra già le letture.

**Interfaces:**
- Consumes: `Folders`, `Queue`, `Budget`, `CallSource`, `session_ledgers` (passo 3); `Block::OtherProjectWrites`, `ConflictReport`, `Provenance.other_projects` (Task 1).
- Produces:
  - `pub(crate) fn writers_near<'a>(listings: &'a [(Agent, Loaded<Descriptor>)], related: &HashSet<(String, String, String)>, candidates: &[(String, PathBuf)], home: Option<&Path>) -> Vec<(&'a Descriptor, Vec<usize>)>`. Dà le sessioni non correlate, con gli indici dei candidati a cui sono vicine, dalla più recente; a pari data, per `(store, harness, id)`. Una sessione è vicina a un candidato quando il suo `cwd` del listing:
    - è il candidato stesso;
    - sta dentro il candidato;
    - oppure è un suo antenato stretto che non sia una radice di disco, la home o una cartella larga (`is_broad_dir`).

    Un `cwd` non locale (`local_dir` in errore) non è vicino a niente.
  - `impl History { pub fn conflicts(&mut self, items: &mut [Item]) }`:
    - i candidati sono le voci `Scratch` e le voci `Named` con `provenance`;
    - le sessioni vicine si leggono una alla volta, con gli stessi contatori di `read` (sessioni, chiamate, durata) e senza azzerarli;
    - per ogni cartella del `Ledger` con forza `>= Written` dentro un candidato, il candidato riceve `Block::OtherProjectWrites`, una sola volta, e la `SessionEvidence` della sessione in `provenance.other_projects`;
    - un fallimento di lettura si conta in `sources[..].failed` come in `read`;
    - `conflicts.read` conta le sessioni lette, `conflicts.unread` quelle vicine non lette, perché il budget è finito o la lettura è fallita;
    - le chiavi dei candidati con almeno una sessione vicina non letta entrano in `History::unchecked: HashSet<String>`, che il Task 5 legge.
  - `pub fn unchecked(&self) -> &HashSet<String>`.

- [ ] **Step 1: Write the failing tests** in `tests/history/conflicts.rs`, con la `CallSource` finta e descrittori finti:
  - `writes_of_another_project_block_the_shared_folder`:
    - la sessione A, correlata (`cwd` = progetto), crea `<base>\shared\run`;
    - la sessione B, non correlata, con `cwd` = `<base>\shared`, scrive `<base>\shared\run\b.txt`.

    Il risultato: la voce `run` ha `blocked` con `OtherProjectWrites`, `provenance.other_projects[0].id == "B"` e `conflicts == ConflictReport { read: 1, unread: 0 }`.
  - `a_session_that_only_read_there_is_no_conflict`: B fa solo Read e Glob in `run`. Nessun blocco, `conflicts.read == 1`.
  - `far_sessions_are_never_read`: una sessione non correlata con `cwd` = `<base>\elsewhere` non compare in `FakeCalls::read_ids()`. `conflicts` vale `{0, 0}`.
  - `a_home_or_drive_cwd_is_not_near`: sessioni non correlate con `cwd` uguale alla home o alla radice del disco della base non vengono lette.
  - `unread_writers_leave_the_folder_unchecked`: con `Budget { max_sessions: 1, .. }` la sola sessione correlata esaurisce il budget. `conflicts.unread == 1` e `unchecked()` contiene la chiave di `run`; nessun blocco.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project history::conflicts`. Expected: FAIL.
- [ ] **Step 3: Implement** `conflicts.rs` e il cablaggio. In `owned::analyze_with` l'ordine diventa `read`, `scratch`, `attach`, `conflicts`.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): scritture di altri progetti bloccano la cartella condivisa`.

---

### Task 5: Rischi e livello `danger` di ogni voce

I rischi si calcolano per ogni voce secondo una tabella per categoria; il livello `Danger` ne discende. Questo stadio gira anche senza cronologia, perché «Sono `danger` le voci senza Git» non dipende dalla cronologia.

**Files:**
- Create: `crates/xiox-project/src/history/danger.rs`, `crates/xiox-project/src/tests/history/danger.rs`
- Modify:
  - `crates/xiox-project/src/history/risk.rs`: `risks` prende `Option<&Provenance>` e i flag dal contesto;
  - `crates/xiox-project/src/history/scratch.rs`: non calcola più i rischi;
  - `crates/xiox-project/src/history/mod.rs`: `attach` copre anche `Category::Worktrees`; il doc di `Item.provenance` in `lib.rs` lo dice;
  - `crates/xiox-project/src/owned.rs`: `danger::assess` dopo `conflicts` e prima di `sort_items`, sempre, con o senza cronologia.

**Interfaces:**
- Consumes: `Item.risks`, `Level::Danger` (Task 1); `History::unchecked` (Task 4); `RECENT_MS` e `risks` (`risk.rs`).
- Produces:
  - `pub(crate) struct RiskContext<'a> { pub partial: bool, pub unchecked: &'a HashSet<String>, pub now_ms: i64 }`. Senza cronologia vale `partial: false` con un `unchecked` vuoto.
  - `pub(crate) fn considered(category: Category, has_provenance: bool) -> &'static [Risk]`. È la tabella, scritta come `match`:

    | Categoria | Rischi considerati | Perché |
    |---|---|---|
    | `Scratch` | tutti e quattro | la voce esiste solo per la cronologia |
    | `Named` con provenienza | tutti e quattro | lo stesso: «diventare `named` invece di `scratch` non elimina una restrizione applicabile» |
    | `Named` senza provenienza | `NoGit`, `RecentActivity` | «voci senza Git»; la recenza vale anche dalle date del filesystem |
    | `Copies`, `Worktrees` | `RecentActivity` | la relazione è provata da Git; evidenza e copertura non la qualificano, e senza Git non possono essere |
    | `Builds`, `Logs`, `Sessions` | nessuno | il catalogo del motore, che ha le sue regole |

  - `PartialCoverage` vale quando `ctx.partial`, quando una sessione della provenienza ha `gaps` o quando `ctx.unchecked` contiene la chiave della voce.
  - `RecentActivity` si propaga al contenitore: una voce la riceve se una voce strettamente dentro di lei (`key_within`) ce l'ha e la sua categoria la considera. Il solo `mtime` della directory non basta a escluderla.
  - `pub(crate) fn assess(items: &mut [Item], ctx: &RiskContext)`. Prima calcola `risks` per ogni voce, poi propaga `RecentActivity`, infine assegna `level = Level::Danger` a ogni voce con `risks` non vuoto. Le altre voci tengono il livello che avevano. I `blocked` non cambiano: un rischio non è un blocco.

- [ ] **Step 1: Write the failing tests** in `tests/history/danger.rs`:
  - `danger_follows_the_risks_of_each_category`, tabella `(nome, preparazione, rischi attesi, livello atteso)`. Le date vecchie si ottengono impostando l'`mtime` della cartella a tre giorni prima con `std::fs::File::set_times`; su Windows la cartella si apre con `FILE_FLAG_BACKUP_SEMANTICS`. Le date delle sessioni stanno nei descrittori finti.
    - `scratch` creata senza `.git`, copertura piena, vecchia: `[NoGit]`, `Danger`;
    - `scratch` creata con `git init` e un commit, copertura piena, vecchia: `[]`, `Caution`;
    - `scratch` scritta e non creata, vecchia: `[NoGit, WeakEvidence]`, `Danger`;
    - `named` senza Git e senza provenienza, vecchia: `[NoGit]`, `Danger`;
    - `named` con Git, vecchia: `[]`, `Caution`;
    - `copies` usata da una sessione un'ora prima: `[RecentActivity]`, `Danger`;
    - `copies` vecchia, con l'ultima sessione tre giorni prima: `[]`, `Caution`;
    - `worktrees` con una sessione di un'ora prima nel suo `cwd`: `[RecentActivity]`, `Danger`;
    - `worktrees` vecchio: `[]`, `Recommended`;
    - `builds` appena modificato: `[]`, `Recommended`.
  - `phazly_example_is_danger`: l'esempio della specifica. Una sessione crea `<base>\agent-runs\planner-benchmark-42` e scrive `benchmark.py`. Il risultato è `Scratch`, `Danger`, con `NoGit` fra i rischi e la sessione nella provenienza.
  - `recent_activity_inside_reaches_the_container`: `named` `<base>\agent-runs`, vecchia, contiene lo `scratch` `bench-1` usato un'ora prima. Anche `agent-runs` ha `RecentActivity`.
  - `unchecked_conflicts_are_partial_coverage`: lo scenario di `unread_writers_leave_the_folder_unchecked` (Task 4) dà a `run` il rischio `PartialCoverage`.
  - `without_history_named_without_git_is_still_danger`: `Options { history: false, .. }`; una `named` senza Git è `Danger` con `[NoGit]`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project history::danger`. Expected: FAIL.
- [ ] **Step 3: Implement** `danger.rs`, il nuovo `risks` e il cablaggio.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-cli`. Expected: PASS. I test esistenti che si aspettano `caution` su una `named` senza Git passano a `danger`; il rapporto del task li elenca.
- [ ] **Step 5: Commit** — `feat(xiox-project): rischi per categoria e livello danger`.

---

### Task 6: Ammissione, conferme e selezione

`ReviewOnly` sparisce. Una voce entra nella pulizia secondo un'unica funzione, `admit`. La funzione guarda la voce e tutti i candidati che contiene: blocchi, voci `danger` da confermare, `--permanent`. `select` la usa sulla scansione mostrata; il Task 7 la riusa sulla scansione fresca. `clean.rs` diventa una cartella perché acquista due responsabilità distinte.

**Files:**
- Move: `crates/xiox-project/src/clean.rs` → `crates/xiox-project/src/clean/mod.rs` (`git mv`, nessun cambiamento di contenuto nel commit dello spostamento)
- Create: `crates/xiox-project/src/clean/admit.rs`, `crates/xiox-project/src/clean/method.rs`
- Modify: `crates/xiox-project/src/clean/mod.rs` (`select`, `SelectError`, `Chosen`, `CleanRequest`), `crates/xiox-project/src/lib.rs` (`pub use clean::{Confirmation, Method, method_of}`)
- Test: `crates/xiox-project/src/tests/clean.rs`. I test di `ReviewOnly` vengono riscritti come quelli sotto.

**Interfaces:**
- Consumes: `Level::Danger`, `Item.risks`, `Target::Subtree` (Task 1, 3, 5); `blocked_inside`, `key_within`.
- Produces:
  - In `method.rs`:
    - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "kebab-case")] pub enum Method { RecycleBin, Permanent, GitWorktreeRemove, HarnessSession }`.
    - `pub fn method_of(target: &Target, mode: RemoveMode) -> Method`. `Worktree` dà `GitWorktreeRemove` in ogni modalità; `Session` dà `HarnessSession`; ogni altro bersaglio dà `RecycleBin` o `Permanent` secondo `mode`.
    - `impl Method { pub fn describe(self) -> &'static str }`, con questi testi:
      - `RecycleBin` → `"to the Recycle Bin"`;
      - `Permanent` → `"deleted for good"`;
      - `GitWorktreeRemove` → `"removed by git for good, not to the Recycle Bin"`;
      - `HarnessSession` → `"removed with the session tools of its harness"`.
  - In `mod.rs`:
    - `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "camelCase")] pub struct Confirmation { pub key: String, pub risks: Vec<Risk>, pub method: Method }`. È il piano confermato di una voce `danger`.
    - `Chosen` aggiunge `pub confirmed: Vec<Confirmation>` e `pub pending: Vec<Confirmation>`.
    - `impl Chosen { pub fn confirm(&mut self, key: &str) -> bool }`: sposta la voce da `pending` a `confirmed`, e dà `false` se `key` non è in `pending`.
    - `CleanRequest` aggiunge `pub confirm_danger: Vec<PathBuf>`.
    - `SelectError` perde `ReviewOnly` e aggiunge tre varianti, con questi testi di `Display`:
      - `ConfirmNotIncluded(PathBuf)` → `"--confirm-danger needs that folder, or a folder that holds it, with --include: {path}"`;
      - `ConfirmNotDanger(PathBuf)` → `"--confirm-danger is only for danger items, and this one is not: {path}"`;
      - `DangerPermanent { path, inner: Option<PathBuf> }` → `"refusing --permanent: danger folders go to the Recycle Bin only: {path}"`. Con `inner` il testo è `"refusing --permanent: it holds {inner}, a danger folder that goes to the Recycle Bin only: {path}"`.
  - In `admit.rs`:
    - `pub(crate) enum Refusal { Blocked(Vec<Block>), ContainsBlocked { inner: PathBuf, why: Vec<Block> }, DangerPermanent { inner: Option<PathBuf> } }`.
    - `pub(crate) fn admit<'a>(items: &'a [Item], item: &'a Item, mode: RemoveMode) -> Result<Vec<&'a Item>, Refusal>`. Dà le voci `danger` che la rimozione di `item` richiede di confermare: la voce stessa, se è `danger`, e ogni voce `danger` strettamente dentro di lei, in ordine di `key`. Le regole, in ordine:
      1. `item.blocked` non vuoto dà `Blocked`;
      2. una voce bloccata dentro (`blocked_inside`) dà `ContainsBlocked`;
      3. con `mode == Permanent`, una voce `danger` con bersaglio `Dir` o `Subtree`, la stessa o una interna, dà `DangerPermanent`. Un worktree `danger` non lo dà: il suo metodo è `git worktree remove` in ogni modalità.
- `select`, nell'ordine:
  1. Le categorie prendono le voci `Recommended`, sbloccate, per le quali `admit` dà `Ok` con una lista vuota. Una voce che è `danger` o contiene una voce `danger` resta fuori senza errore, come oggi le bloccate.
  2. Ogni `--include` passa per `admit`, che mappa su `Blocked`, `ContainsBlocked` o `DangerPermanent`. Le voci da confermare diventano `Confirmation { key, risks, method: method_of(&target, mode) }`.
  3. Ogni `--confirm-danger` si risolve con `path_key(absolute(..))` come `--include`:
     - assente dall'analisi dà `NotInAnalysis`;
     - una voce non `danger` dà `ConfirmNotDanger`;
     - una voce che nessun `--include` richiede di confermare dà `ConfirmNotIncluded`.
  4. Le conferme richieste e date vanno in `confirmed`, le altre in `pending`, senza ripetizioni, nell'ordine dell'elenco.

- [ ] **Step 1: Write the failing tests** in `tests/clean.rs`, su scansioni costruite in memoria come i test di `select` esistenti:
  - `scratch_is_selectable_with_confirmation`: `--include <bench-1>`, uno `scratch` `danger` con `risks == [NoGit]`, dà `keys == [bench-1]`, `confirmed == []` e `pending == [Confirmation { key, risks: [NoGit], method: RecycleBin }]`. Con anche `--confirm-danger <bench-1>` la conferma va in `confirmed` e `pending` è vuoto.
  - `container_needs_the_confirmations_of_its_descendants` (Review Focus 1). Una `named` `agent-runs`, non `danger`, contiene gli `scratch` `danger` `a` e `b`:
    - `--include agent-runs` dà `pending` con `a` e `b`;
    - `--include agent-runs --confirm-danger agent-runs` dà `ConfirmNotDanger(agent-runs)`;
    - `--include agent-runs --confirm-danger a` dà `confirmed == [a]` e `pending == [b]`;
    - un `b` bloccato da `OtherProjectWrites` dà `ContainsBlocked { inner: b, why: [OtherProjectWrites] }`.
  - `confirm_must_name_an_included_danger_item`. È una tabella:
    - `--confirm-danger` su un percorso non analizzato dà `NotInAnalysis`;
    - su uno `scratch` `danger` non incluso, né contenuto in una voce inclusa, dà `ConfirmNotIncluded`;
    - su una build `Recommended` inclusa dà `ConfirmNotDanger`.
  - `category_never_takes_danger`. Un worktree `Recommended`, diventato `danger` per attività recente, non viene preso da `--worktrees`. Una build `Recommended` che contiene uno `scratch` `danger` non viene presa da `--builds`. Nessuno dei due casi è un errore.
  - `permanent_refuses_danger_folders_but_not_worktrees`. Con `RemoveMode::Permanent`:
    - uno `scratch` `danger` incluso dà `DangerPermanent { inner: None }`;
    - una `named` che contiene uno `scratch` `danger` dà `DangerPermanent { inner: Some(..) }`;
    - un worktree `danger` incluso è accettato, con `method == GitWorktreeRemove` e la chiave in `permanent_anyway`.
  - `chosen_confirm_moves_only_pending_keys`: `confirm` di una chiave in `pending` dà `true` e la sposta; di qualsiasi altra chiave dà `false` e non cambia nulla.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project clean`. Expected: FAIL.
- [ ] **Step 3: Commit lo spostamento**: `git mv crates/xiox-project/src/clean.rs crates/xiox-project/src/clean/mod.rs`, con `cargo test -j 1 -p xiox-project` verde e il messaggio `refactor(xiox-project): clean diventa un modulo con sottomoduli`. I test nuovi non si committano qui.
- [ ] **Step 4: Implement** `method.rs`, `admit.rs` e il nuovo `select`. `scratch_inside` sparisce.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-cli`. Expected: PASS. La CLI compila con `confirm_danger: Vec::new()`; il flag arriva nel Task 8.
- [ ] **Step 6: Commit** — `feat(xiox-project): ammissione con conferme per voce, contenitori e rifiuto di --permanent`.

---

### Task 7: Riconvalida alla rimozione

`clean_with` non si fida di `Chosen`, che ha campi pubblici, né della scansione mostrata. Quando la selezione contiene voci che dipendono dalla cronologia, rilegge la cronologia attraverso l'analisi fresca. Poi riapplica `admit` alla voce fresca e confronta il piano confermato con quello di adesso.

**Files:**
- Modify:
  - `crates/xiox-project/src/clean/mod.rs`: `clean_impl`, `clean_with`, `recheck`, `Run::remove`, `CleanOutcome`;
  - `crates/xiox-project/src/lib.rs`: `with_real_env` diventa `pub(crate)`; `pub use clean::OutcomeItem`.
- Test: `crates/xiox-project/src/tests/clean.rs`, con l'`Env` e l'`Io` finti dei test di `clean_with` e la `CallSource` finta di `tests/history/fake.rs`.

**Interfaces:**
- Consumes: `admit`, `Confirmation`, `Method`, `method_of` (Task 6); `folder_id`, `Item.identity` (Task 1); `with_real_env` (`lib.rs`).
- Produces:
  - `pub(crate) fn wants_history(old: &[&Item]) -> bool`: vero se una voce scelta ha `provenance` o è `danger`.
  - `clean_impl` con `Sessions::Real` costruisce l'`Env` con `with_real_env(xiox_app::cache_dir(), wants_history(..), ..)`. Così la cronologia si rilegge tramite Mukade con lo stesso `BUDGET` dell'analisi; senza voci di quel tipo non si legge nulla, come oggi. `Sessions::None` resta senza cronologia.
  - In `clean_with`, `opts.history = wants_history(&old)`. Se la cronologia era voluta e la scansione fresca ha `history` assente o `not_read()`, ogni voce scelta con `provenance` viene saltata con `"the session history could not be read again"`.
  - `pub(crate) fn recheck<'a>(old: &Item, fresh: Option<&'a Item>, confirmation: Option<&Confirmation>, mode: RemoveMode) -> Result<&'a Item, String>`. Ai controlli di oggi si aggiungono questi, in quest'ordine:
    1. identità: con `old.identity` presente, una `fresh.identity` diversa o assente dà `"it is not the folder that was analysed: it was replaced"`. Con `old.identity` assente, una voce fresca `danger` dà `"its identity could not be checked"`;
    2. una voce fresca `danger` senza `confirmation` dà `DANGER_TEXT`;
    3. con `confirmation`, `fresh.risks != confirmation.risks` dà `"its risks changed since it was confirmed: choose it again"`. Vale anche quando i rischi diminuiscono;
    4. con `confirmation`, `method_of(&fresh.target, mode) != confirmation.method` dà `"its removal method changed since it was confirmed: choose it again"`;
    5. la regola esistente su `Caution` vale solo per una `old` non `danger`.
  - La `confirmation` di una chiave si cerca solo in `chosen.confirmed`; `pending` non conta mai.
  - Dopo `recheck`, `Run::remove` chiama `admit(fresh.items, f, mode)` per ogni voce fresca. Gli esiti:
    - `Refusal` → `Skipped` con il testo della `SelectError` corrispondente, senza il percorso finale;
    - ogni voce interna `danger` senza una `Confirmation` in `chosen.confirmed` con gli stessi `risks` e lo stesso `method` → `Skipped` con `"it holds {inner}, which needs an explicit confirmation"`;
    - una voce interna con `risks` cambiati → `Skipped` con `"it holds {inner}, whose risks changed since it was confirmed"`.
  - `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "camelCase")] pub struct OutcomeItem { pub key: String, pub path: Option<PathBuf>, pub category: Category, pub method: Method, pub outcome: Outcome }`. `CleanOutcome.items: Vec<OutcomeItem>` sostituisce la tupla; `method` è `method_of(&old.target, mode)`.

- [ ] **Step 1: Write the failing tests** in `tests/clean.rs`:
  - `hand_made_chosen_gets_no_more_than_select` (Review Focus 1). Su un albero vero in `tempdir`, con la cronologia finta, la `named` `agent-runs` contiene lo `scratch` `danger` `bench-1`. Casi, tutti con `dry_run: false` e `RemoveMode::Recycle`, e una `Trash` finta che registra:
    - `Chosen { keys: [agent-runs], confirmed: [], pending: [] }` dà `Skipped` `"it holds …bench-1, which needs an explicit confirmation"`;
    - `Chosen { keys: [bench-1] }` senza conferme dà `Skipped` `DANGER_TEXT`;
    - `Chosen { keys: [bench-1], confirmed: [Confirmation { risks: vec![], .. }] }`, con rischi falsi, dà `Skipped` «risks changed».

    Alla fine `agent-runs` e `bench-1` esistono ancora, e né la `Trash` né il `ParallelRemover` hanno ricevuto nulla.
  - `replaced_folder_is_skipped_by_identity` (Review Focus 2). Dopo l'analisi e la conferma di `bench-1`:
    - cancellata e ricreata con lo stesso nome, dà `Skipped` `"it is not the folder that was analysed: it was replaced"`;
    - sostituita da una junction verso `<base>\target`, dà `Skipped` `"now a link: not followed"`, e `target` resta intatto. Se la junction non si crea, il caso stampa `skipped`.
  - `a_changed_risk_needs_a_new_choice` (Review Focus 2):
    - dopo la conferma, un `git init` dentro `bench-1` toglie `NoGit` e dà `Skipped` «risks changed»;
    - una sessione finta nuova, di un minuto prima, che usa `bench-1` aggiunge `RecentActivity`, con lo stesso esito;
    - una sessione finta nuova, non del progetto, che scrive in `bench-1` dà `Skipped` `"now blocked: sessions of another project wrote here"`.
  - `history_is_read_again_at_removal`. Il `clean_with` di uno `scratch` confermato fa almeno una `dump` nella fase di pulizia, contata da `FakeCalls::read_ids()`. Con una `CallSource` che alla pulizia fallisce ogni lettura e un listing che fallisce, il risultato è `Skipped` `"the session history could not be read again"`.
  - `confirmed_scratch_passes_every_check_in_a_dry_run`: con `dry_run: true` e la conferma giusta, il risultato è `Removed { bytes_freed }`, con `method == RecycleBin` nell'`OutcomeItem`.
  - `permanent_never_reaches_a_danger_folder`: un `Chosen` a mano con `RemoveMode::Permanent` e la conferma di `bench-1` dà `Skipped` `"refusing --permanent: danger folders go to the Recycle Bin only"`. La cartella esiste ancora.
  - `danger_worktree_is_removed_by_git_never_forced`:
    - un worktree con una sessione di un'ora prima nel suo `cwd` è `danger`; confermato con `method: GitWorktreeRemove` e `RemoveMode::Recycle`, viene rimosso, con `OutcomeItem.method == GitWorktreeRemove`;
    - un secondo worktree `danger` con un file non tracciato viene rifiutato da git senza `--force`: `Failed` o `Skipped`, e la cartella esiste ancora.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project clean`. Expected: FAIL.
- [ ] **Step 3: Implement** come sopra. `recycled_bytes` resta calcolato com'è, sul bersaglio della voce.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-project` e `cargo test -j 1 -p xiox-cli`. Expected: PASS. I test esistenti che leggono `CleanOutcome.items` come tupla passano ai campi di `OutcomeItem`. Fanno lo stesso, in modo meccanico, `crates/xiox-cli/src/{json.rs,render.rs}`. La forma JSON nuova la fissa il Task 8.
- [ ] **Step 5: Commit** — `feat(xiox-project): la pulizia rilegge la cronologia e riconvalida identità, rischi, metodo e contenitori`.

---

### Task 8: La CLI: `--confirm-danger`, il prompt per voce, metodo e rischi

**Files:**
- Modify:
  - `crates/xiox-cli/src/args.rs`: `--confirm-danger <path>`, ripetibile, in `Command::Clean` e in `USAGE`;
  - `crates/xiox-cli/src/run.rs`: `CleanFlags.confirm_danger`, `clean_run`, `danger_question`;
  - `crates/xiox-cli/src/render.rs`: la riga `danger` nell'elenco, il metodo nel riepilogo;
  - `crates/xiox-cli/src/json.rs`: `chosen` e `outcome` dello schema 2;
  - `crates/xiox-cli/src/testdata.rs`: una scansione con uno `scratch` `danger` e un worktree `danger`.
- Test: moduli di test di `args.rs`, `run.rs`, `render.rs` e `json.rs`; `crates/xiox-cli/tests/cli.rs`.

**Interfaces:**
- Consumes: `CleanRequest.confirm_danger`, `Chosen::{confirm, confirmed, pending}`, `Confirmation`, `Method::describe` (Task 6); `OutcomeItem` (Task 7); `DANGER_TEXT`, `Risk::reason` (Task 1).
- Produces:
  - `Command::Clean { …, confirm_danger: Vec<PathBuf> }`. La riga di `USAGE` è `--confirm-danger <path>  confirm one danger item: the folder, or one inside a folder given with --include`.
  - `fn danger_question(err: &mut dyn Write, input: &mut dyn BufRead, path: &Path, c: &Confirmation) -> bool`. Scrive su `err` questo blocco e legge una riga; la fine dell'input è un no:

    ```text
    {path}
      danger: explicit confirmation required
      - {Risk::reason}            (una riga per rischio)
      removal: {Method::describe}
    Remove this danger item? [y/N] 
    ```
  - `clean_run`, dopo `select` e prima del riepilogo, quando `chosen.pending` non è vuoto:
    1. con `--dry-run` non si chiede nulla. Le voci `pending` escono come `Skipped` `DANGER_TEXT` dal `clean_with` del Task 7;
    2. con `--yes`, `--json` o uno stdin che non è un terminale non si rimuove nulla. Per ogni voce in sospeso si scrive su stderr `error: {path}: danger: explicit confirmation required ({reasons}): add --confirm-danger "{path}"`, con `reasons` uguali ai `Risk::reason` separati da `, `. L'uscita è `EXIT_USAGE`;
    3. altrimenti si fa `danger_question` per ogni voce in sospeso, in ordine. Un sì chiama `chosen.confirm(key)`. Al primo no si scrive `Cancelled.` su stderr, si esce con `EXIT_OK` e non si rimuove nulla.

    Poi il riepilogo e la domanda finale restano quelli di oggi; `--yes` salta solo la domanda finale.
  - `render_scan`: sotto ogni voce `danger` compare la riga `  danger: explicit confirmation required: {reasons}`. La colonna del livello stampa `danger`.
  - `render_summary`: ogni voce `danger` confermata e ogni worktree hanno la riga `  {path}: {Method::describe}`. La frase di oggi sui worktree nella domanda finale resta.
  - JSON di `clean`:
    - `chosen` serializza `Chosen` con `confirmed` e `pending`;
    - `outcome.items` è un array di `OutcomeItem`;
    - `schema` viene da `SCHEMA`.

- [ ] **Step 1: Write the failing tests** in `args.rs`:
  - `clean_confirm_danger_repeats`: `["clean", "--include", "a", "--confirm-danger", "a", "--confirm-danger", "b"]` dà `confirm_danger == ["a", "b"]`;
  - `confirm_danger_needs_a_value`: un `--confirm-danger` finale dà un errore d'uso;
  - `clean_defaults`, aggiornato con `confirm_danger: vec![]`.
- [ ] **Step 2: Write the failing tests** in `run.rs`, con i `Deps` finti di `run_clean` e la scansione `danger` di `testdata.rs`:
  - `yes_alone_or_another_path_removes_nothing` (Review Focus 5). Tabella `(flag, stdin, terminale, codice atteso)`. In ogni caso `remove` non viene mai chiamato con `dry_run: false`:
    - `--include bench --yes`: `EXIT_USAGE`, e stderr contiene `danger: explicit confirmation required (not a git repository)` e `--confirm-danger "…bench"`;
    - `--include bench --include build --confirm-danger build --yes`: `EXIT_USAGE` con il testo di `ConfirmNotDanger`;
    - `--include build --confirm-danger bench --yes`: `EXIT_USAGE` con il testo di `ConfirmNotIncluded`;
    - `--include bench --json --yes`: `EXIT_USAGE`, e stdout resta vuoto;
    - `--include bench`, senza terminale: `EXIT_USAGE`.
  - `agent_confirms_a_danger_item_with_confirm_danger`. È il caso di accettazione «Agente conferma una voce `danger` alla CLI»: `--include bench --confirm-danger bench --yes` chiama `remove` una volta, con `chosen.confirmed[0].key == <bench>` e `pending` vuoto. L'uscita è `EXIT_OK`.
  - `the_danger_question_names_the_risks_and_the_method`. Al terminale con `"y\ny\n"`, stderr contiene nell'ordine:
    - il percorso;
    - `danger: explicit confirmation required`;
    - `- not a git repository`;
    - `removal: to the Recycle Bin`;
    - `Remove this danger item? [y/N]`;
    - la domanda finale.

    Per il worktree `danger` la riga del metodo è `removal: removed by git for good, not to the Recycle Bin`.
  - `a_no_to_a_danger_question_cancels_everything`: con due voci `danger` e le risposte `"y\nn\n"`, l'uscita è `EXIT_OK`, stderr contiene `Cancelled.` e `remove` non viene chiamato.
  - `dry_run_asks_nothing_and_reports_the_confirmation`: `--include bench --dry-run` non legge stdin e chiama `remove` con `dry_run: true`.
- [ ] **Step 3: Write the failing tests** in `render.rs` e `json.rs`:
  - `danger_items_say_why_in_the_listing`: la scansione `danger` ha la riga `danger: explicit confirmation required: not a git repository, used or changed in the last 24 hours`;
  - `summary_names_the_method_of_danger_items_and_worktrees`;
  - `clean_json_is_schema_2_with_method_and_confirmations`: l'oggetto ha `"schema": 2`, `chosen.confirmed[0].method == "recycle-bin"` e `outcome.items[0]` con le chiavi `key`, `path`, `category`, `method` e `outcome`.
- [ ] **Step 4: Write the failing test** `clean_danger_without_confirmation_exits_2` in `crates/xiox-cli/tests/cli.rs`, sul progetto finto dei test esistenti, con `XIOX_NO_SESSIONS`, `--no-history` e una cartella `named` senza Git, quindi `danger` per il Task 5:
  - `xiox clean <proj> --include <named> --yes` esce con 2 e la cartella esiste ancora;
  - `xiox clean <proj> --include <named> --confirm-danger <named> --yes --dry-run` esce con 0.

  Il test non rimuove davvero nulla: una rimozione in Cestino da un test toccherebbe il Cestino reale.
- [ ] **Step 5: Run** `cargo test -j 1 -p xiox-cli`. Expected: FAIL.
- [ ] **Step 6: Implement** come sopra.
- [ ] **Step 7: Run** `cargo test -j 1 -p xiox-cli`. Expected: PASS.
- [ ] **Step 8: Commit** — `feat(xiox-cli): --confirm-danger, domanda per voce danger, metodo e rischi nel riepilogo e nel JSON`.

---

### Task 9: Osservatore, MCP, guide e stato

**Files:**
- Modify:
  - `crates/xiox-observer/src/tests/state.rs`: un test sul `cleanCommand`;
  - `crates/xiox-cli/src/mcp.rs`: la descrizione del tool, righe 30–40;
  - `docs/xiox-cli.md`: livelli, `scratch`, conferme, metodo, schema;
  - `docs/xiox-observe.md`: righe 104, 139, 171–185, 237, 297 e 410;
  - `README.md:90`;
  - `AGENTS.md`: sezione «Estrattore dei percorsi dalle sessioni», stato del passo 4.

**Interfaces:**
- Consumes: tutto quanto sopra; nessun tipo nuovo.
- Produces: testo. La guida `docs/xiox-cli.md` deve contenere:
  - la tabella dei livelli, con `danger` e il testo `danger: explicit confirmation required`;
  - la tabella dei rischi per categoria del Task 5;
  - la tabella dei blocchi nuovi con i loro testi;
  - `--confirm-danger`, con le regole del Task 6 e del Task 8 ed esempi per una persona e per un agente;
  - il Cestino obbligatorio per le cartelle `danger` e l'eccezione dei worktree, con `git worktree remove` definitivo e mai `--force`;
  - la riconvalida: cronologia riletta, identità, rischi e metodo confrontati con la conferma;
  - i candidati dentro il progetto (`subtree`) e la loro policy;
  - i conflitti fra progetti e la loro copertura parziale. Si cercano solo fra le sessioni che hanno lavorato dentro o intorno alla cartella;
  - la tabella dello schema 2 di questo piano e la regola delle versioni;
  - il confine della UI: «Folders found in the session history are removed only by `xiox clean`; the desktop app does not list them.»

  La descrizione MCP aggiunge: `"Items at level danger are never in cleanCommand: they need xiox clean --include <path> --confirm-danger <path>."`

- [ ] **Step 1: Write the failing tests**:
  - `clean_command_never_carries_danger` in `crates/xiox-observer/src/tests/state.rs`. Una scansione ha una build `Recommended` e un worktree `danger` che prima era `Recommended`. Il `cleanCommand` contiene `--include` della build e non quello del worktree.
  - `the_tool_description_names_confirm_danger` nel modulo di test di `mcp.rs`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-observer clean_command_never` e `cargo test -j 1 -p xiox-cli the_tool_description`. Expected: il primo PASS già, perché `danger` non è `Recommended`, e resta come guardia; il secondo FAIL.
- [ ] **Step 3: Implement** la descrizione MCP e le guide.
- [ ] **Step 4: Run** questi controlli:
  - `cargo test -j 1 -p xiox-project`, `cargo test -j 1 -p xiox-cli`, `cargo test -j 1 -p xiox-observer`, `cargo test -j 1 -p xiox-cleaner` e `cargo test -j 1 -p xiox-git`. Expected: PASS;
  - `node scripts/italian-check.mjs`. Expected: nessuna stringa italiana;
  - `git diff --stat main -- apps/desktop crates/xiox-core crates/xiox-model`. Expected: vuoto, come deciso per la UI. `npm test` e `npm run smoke` non servono, perché il desktop non cambia.
  - `grep -rn "ReviewOnly\|schema 1\|\"schema\": 1" crates docs README.md`. Expected: nessuna riga, salvo la storia nella guida («schema 1 → 2»).
- [ ] **Step 5: Commit** — `docs: schema 2, livello danger, conferme e pulizia delle voci della cronologia`.

---

## Self-review

1. **Copertura della specifica.**

   | Requisito | Dove |
   |---|---|
   | `danger`: senza Git, evidenza debole, copertura insufficiente, recenti 24 h; date del filesystem solo per la recenza | Task 5 |
   | Attività dentro un sottoalbero riflessa sul contenitore | Task 5, `recent_activity_inside_reaches_the_container` |
   | Testo `danger: explicit confirmation required` con motivi strutturati | Task 1 (`DANGER_TEXT`, `Risk::reason`), Task 8 |
   | Recenza non è un blocco; i blocchi certi restano | Task 5 (`assess` non tocca `blocked`); `LiveSession` resta (Task 3) |
   | Una categoria non seleziona `danger` | Task 6, `category_never_takes_danger` |
   | `--include` più prompt o `--confirm-danger`; `--yes` da solo non basta; valido per un agente | Task 6 e 8 |
   | Il confermato deve essere un candidato; nessun percorso arbitrario | Task 6, `confirm_must_name_an_included_danger_item` |
   | Contenitori: blocchi e conferme dei discendenti | Task 6 (`admit`), Task 7 (di nuovo alla rimozione) |
   | Riconvalida: bersaglio, contenuti, rischio, metodo | Task 7 |
   | `--permanent` rifiutato per le cartelle `danger` | Task 6 e 7 |
   | Worktree: `git worktree remove`, mai `--force`, dichiarato in riepilogo e JSON | Task 6 (`Method`), 7 e 8 |
   | Policy Git del sottoalbero, guard di `ProjectFolder` non allentato | Task 3 |
   | Controlli 1: forme non supportate, antenati, link | Task 2 (8.3, permessi, sistema, profili) |
   | Controlli 2: esistenza, identità, contenuti, blocchi ricontrollati | Task 7 (`FolderId`, analisi fresca) |
   | Controlli 3: lavoro non salvato, illeggibili, protette, scansioni incomplete | `named_blocks`, `subtree_blocks`, `check_contents` (Task 2, 3); `nested_repos` in errore dà `UnsavedWorkUnknown` |
   | Controlli 4: conflitti fra progetti, copertura parziale come rischio | Task 4 e 5 |
   | Controlli 5: prove obsolete non riusate | Task 7: la cronologia si rilegge; senza lettura la voce si salta |
   | Controlli 6: `named` non elimina restrizioni | Task 2 (`HoldsRepositories` anche per `named`), Task 5 (tabella) |
   | «La pulizia rilegge tramite Mukade le evidenze» | Task 7, `history_is_read_again_at_removal` |
   | Schema e guide | Task 1 e 9 |
   | UI | Decisione in testa: nessuna voce `danger` arriva al desktop |

   Rinvii della revisione finale del passo 3:

   | Rinvio | Task |
   |---|---|
   | Alias 8.3 di home e `%TEMP%` | 2 |
   | Discendenti di sistema e profili altrui | 2 |
   | Permessi su un antenato letti come `Missing` | 2 |
   | `%TEMP%` radice di disco o `AppData\Local` | 2 |
   | `clean_with` senza cronologia e senza controlli dei contenitori | 7 |
   | Cartella solo `written` con molti repository puliti | 2 |

   Casi di accettazione di questo passo:

   | Caso | Task |
   |---|---|
   | Agente conferma una voce `danger` alla CLI | 8 |
   | Solo `--yes`, oppure conferma di un altro percorso | 8 |
   | Attività recente con vitalità sconosciuta | 5 |
   | Contenitore con discendente bloccato o non confermato | 6 e 7 |
   | Worktree `danger` confermato | 7 e 8 |
   | Sorgente riscritta con stesso prefisso | 7: nessuna prova è riusata, la cronologia si rilegge |
   | Cartella condivisa documentata da due progetti | 4 |

   La verifica integrata di tutta la tabella su Windows resta del passo 5.
2. **Passi.** Ogni passo di codice nomina file, firma, ordine delle regole e testi esatti. Due correzioni fatte qui:
   - il Task 7 dice che anche `json.rs` e `render.rs` si adattano a `OutcomeItem`, altrimenti la CLI non compila fra il Task 7 e l'8;
   - il test CLI del Task 8 non rimuove davvero nulla, perché una rimozione in Cestino da un test toccherebbe il Cestino reale.

   Una terza correzione riguarda l'ordine dei task. I conflitti (Task 4) vengono prima del livello (Task 5), perché `PartialCoverage` legge `unchecked`. La tabella dell'ambito in testa segue questi numeri.
3. **Coerenza dei tipi.**
   - `Level` e `DANGER_TEXT` (Task 1) sono letti in 5, 6, 7 e 8.
   - `Item.risks` (1) è scritto in 5 e confrontato con `Confirmation.risks` in 7.
   - `FolderId` e `folder_id` (1) sono confrontati in 7.
   - `Target::Subtree` (1) è prodotto in 3 e gestito da `check_dir`/`check_contents` (3) e da `method_of` (6).
   - `Block::{OtherProjectWrites, HoldsRepositories, TrackedFiles}` (1) sono prodotti in 4, 2 e 3.
   - `History::unchecked` (4) è letto da `RiskContext` (5).
   - `admit`, `Refusal`, `Confirmation`, `Method` e `method_of` (6) sono usati in 7 e 8.
   - `OutcomeItem` (7) è serializzato in 8.
   - `Exclusion::{ShortName, Unreadable}` (1) sono contati in 2.
4. **Review Focus.** I cinque punti hanno test nominati:
   - `container_needs_the_confirmations_of_its_descendants` (6) e `hand_made_chosen_gets_no_more_than_select` (7);
   - `replaced_folder_is_skipped_by_identity` e `a_changed_risk_needs_a_new_choice` (7);
   - `short_names_never_slip_past_the_guards` (2);
   - `permission_denied_is_not_missing` (2);
   - `yes_alone_or_another_path_removes_nothing` (8).
5. **Proporzione.** Il piano descrive circa 1.200 righe di codice e di test in 9 task. Non contiene corpi di funzione: solo firme, ordine delle regole, tabelle di casi e testi esatti. È più lungo della sezione della specifica che implementa perché porta i sei rinvii del passo 3 e le decisioni su schema e UI, che la specifica non prende.

## Execution Handoff

Piano salvato in `docs/superpowers/plans/2026-11-16-step4-selection-cleanup.md`. Rivedi il piano, per favore. Quale metodo di esecuzione preferisci?

- **Subagent-driven:** un subagente nuovo implementa ogni task e un revisore nuovo lo controlla prima del successivo, poi c'è una revisione dell'intero ramo. È il metodo più accurato; costa un contesto nuovo per task e per revisione.
- **Native:** implemento ogni task io, in questa sessione, poi un revisore nuovo, sul modello più capace, controlla tutto il ramo. È il metodo più economico e veloce, ma non c'è revisione indipendente fino alla fine.

**Per questo piano consiglio subagent-driven.** I task 6 e 7 dipendono dalle interfacce dei task 1–5. Un errore che arriva in produzione rimuoverebbe cartelle trovate nella cronologia senza le conferme dovute. Un revisore per task è la difesa giusta. Il piano descrive ciò che vuoi, e quale metodo usiamo?

**If Subagent-driven chosen:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development

**If Native chosen:** REQUIRED SUB-SKILL: Use superpowers:executing-plans

