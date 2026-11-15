# Estrattore dei percorsi dai comandi e registro delle evidenze: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** dare a Xiox, in Rust, l'estrattore deterministico dei percorsi dai comandi shell e il registro delle evidenze per sessione misurati nel prototipo `bench/path-extractor/`. Entrambi leggono le chiamate di tool delle sessioni tramite Mukade.

**Architecture:** un modulo `provenance` in `crates/xiox-project` fatto di stadi puri. Nell'ordine:

1. lessico per dialetto (`Dialect`);
2. struttura del comando (`Node`);
3. registro di riconoscitori (`Recognizer`);
4. analizzatori di script e di patch;
5. `Resolver`;
6. classificatore delle chiamate shell;
7. `Ledger`.

`crates/xiox-mukade` aggiunge la lettura di `mukade dump --stream` come `ToolCall`. Un esempio `shell_probe` confronta il Rust col prototipo Python sulla cronologia reale, fuori dal repository.

**Tech Stack:** Rust 2024, `serde`/`serde_json`, già presenti. In più, solo la dev-dependency `toml = "0.8"`, già nel `Cargo.lock` alla 0.8.2. Nessuna regex: le scansioni sono scritte a mano e lineari.

**Spec:** `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`, revisione 3. Le sezioni sono «Operazioni riconosciute», «Estrazione dai comandi: decisione e misure», «Struttura del codice» e l'elenco di ciò che si conserva per ogni evidenza.

Il prototipo `bench/path-extractor/extract.py` ed `evidence.py` è il riferimento di comportamento: dove questo piano tace, vale il prototipo. Le divergenze volute sono solo quelle della sezione «Divergenze dichiarate dal prototipo».

**Ambito.** Questo piano consegna la libreria (estrattore, patch, resolver, classificatore, registro) e la lettura delle chiamate tramite Mukade, provate su fixture, più la sonda di parità. Non tocca `analyze`, la categoria `scratch`, `danger`, la CLI né la UI: l'integrazione è il piano successivo, al passo 3 dell'«Ordine di implementazione» della specifica.

## Global Constraints

- Inglese in ogni stringa visibile a utente o programma: identificatori, errori, output della sonda. `scripts/italian-check.mjs` deve restare pulito.
- «L'estrazione è deterministica.» Nessun modello, nessuna chiamata di rete.
- «Xiox … Non esegue comandi letti dalla cronologia.»
- «Variabili d'ambiente o non assegnate, sostituzioni di comando, cicli su glob o output, f-string, template e concatenazioni non producono percorsi: restano irrisolti e sono conteggiati.» Questo vale anche dentro gli script.
- «Nessun percorso irrisolto diventa candidato.»
- «Un'operazione fallita resta visibile ma non porta forza.»
- «Patch: creazione, modifica, spostamento ed eliminazione di file mantengono significati distinti. Una modifica non diventa creazione; una cancellazione non genera una nuova cartella.»
- «Per ogni evidenza si conservano: percorso, … agente, ID sessione, ID/posizione dell'evento, timestamp se disponibile, `cwd` effettivo, tipo di operazione, esito e forza dell'evidenza. … I riferimenti distinguono anche store diversi.»
- «I comandi reali delle sessioni non entrano nel repository.» Le fixture sono solo sintetiche.
- Forza ordinata «usata < scritta < creata»; chiavi di cartella senza distinguere maiuscole e minuscole.
- Lavoro lineare nella lunghezza del comando, per costruzione. Si verifica così:
  - ogni scansione è un solo passaggio `char_indices`;
  - lookahead e pile hanno limiti costanti;
  - un `Budget` condiviso limita espansioni, profondità e operazioni.

  Nessun panic su nessun input, compresi testo non ASCII e testo tagliato a 4.000 caratteri.
- Il database di OpenCode si legge da una copia, mai dall'originale (`AGENTS.md`).
- `cargo` sempre con `-j 1` e `CARGO_INCREMENTAL=0`, un solo processo cargo alla volta. I test usano solo dati in memoria o `tempfile::tempdir()`.
- Non cambiare il comportamento né l'output di `xiox analyze`, `xiox clean` e `MukadeSessions::list`: i loro test esistenti fanno da guardia.
- Commit: solo i file del task, messaggio in italiano (`feat(xiox-project): …`), che termina con
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Divergenze dichiarate dal prototipo

Ogni divergenza ha un test nel task indicato. Nella sonda di parità (Task 13) ogni divergenza è una classe contata a parte, non una differenza.

| # | Prototipo | Rust | Task |
|---|---|---|---|
| D1 | Il `cd` del registro si risolve contro la cwd iniziale della chiamata. | Ogni `cd` si risolve dove è stato eseguito, tramite `Resolved.visits`. | 2, 12 |
| D2 | Un delimitatore di heredoc seguito da `\r` non chiude il corpo, che inghiotte i comandi successivi. | `EOF\r` chiude il corpo; i comandi dopo vengono estratti. | 3 |
| D3 | Un testo tagliato viene estratto come se fosse intero, quindi il taglio può accorciare una destinazione. | Con `Completeness::Truncated`, l'ultimo comando di primo livello viene scartato e conteggiato. | 8 |
| D4 | Espansione senza tetto globale. | `Budget` condiviso; all'esaurimento `flags.exhausted` e le operazioni restanti sono conteggiate. | 8 |
| D5 | I chiamanti si cercano anche dentro le stringhe dello script. | Una sola scansione che salta i letterali: `print("open('x','w')")` non dà nulla. | 7 |
| D6 | Le f-string e gli argomenti non letterali negli script non sono conteggiati. | Sono conteggiati in `unresolved`. | 7 |
| D7 | `apply_patch` non dà evidenze. | Dà evidenze con l'azione della patch. | 9, 12 |

La correzione D1 è già nel prototipo: `resolve(..., visits)` in `extract.py`, usato da `evidence.py`. Rimisurata sui due campioni dopo la correzione, è invariata: verifica 60/106 cartelle dal comando e 70/106 dalla sessione, sviluppo 92/98 e 97/98. Le cifre della specifica restano valide.

## Review Focus

1. **Testo non ASCII e troncato.** Un comando con `è`, `→` o emoji, tagliato a metà di una virgoletta, di un heredoc o di una chiamata Python, non manda mai in panic. Un testo tagliato non produce mai una destinazione accorciata (attenzione agli indici di byte in Rust). Test: `every_prefix_of_every_fixture_extracts_without_panic` e `truncated_prefixes_never_invent` (Task 8), `exit_windows_count_chars` (Task 10).
2. **Heredoc con CRLF e operatori dopo il delimitatore.** Un `python - <<'EOF' > /d/runs/out.json` deve dare sia la scrittura sia il corpo. Un comando dopo `EOF\r` viene estratto. Test: `heredoc_delimiter_tolerates_crlf` e `operators_after_heredoc_delimiter_are_kept` (Task 3), `command_after_crlf_heredoc_is_extracted` (Task 8).
3. **Lavoro limitato.** Test: `nested_loops_and_shells_stop_at_the_budget` (Task 8), `adversarial_scripts_scan_once` (Task 7). I test coprono:
   - cicli annidati da 50 elementi;
   - shell annidate a 10 livelli;
   - 50.000 `open(` non chiusi.
4. **Variabili mai inventate.** `$env:TEMP`, `%TEMP%`, `$(mktemp -d)`, `` `pwd` ``, `*.json`, una f-string e una variabile riassegnata con un valore non letterale non producono candidati e alzano `unresolved`. Test: `unknown_values_are_counted_not_guessed` (Task 8), `non_literals_are_counted` (Task 7).
5. **Provenienza ed esito.** Due sessioni con lo stesso id in store diversi restano distinte. Una chiamata senza risultato ha esito `Unknown`, distinto da `Succeeded`. Una creazione fallita lascia la cartella visibile ma senza forza. Test: `same_session_id_in_two_stores` e `verbatim_cwd_and_outcomes` (Task 12).

---

## File Structure

| File | Responsabilità |
|---|---|
| `crates/xiox-project/src/provenance/mod.rs` | Ingresso del modulo, riesportazioni pubbliche |
| `crates/xiox-project/src/provenance/op.rs` | Tipi di valore: `OpKind`, `RawOp`, `Flags`, `Completeness`, `Extraction`, `Candidate` |
| `crates/xiox-project/src/provenance/paths.rs` | `Env`, `resolve_path`, `resolve`: da percorsi scritti a candidati e visite |
| `crates/xiox-project/src/provenance/dialect.rs` | `Shell`, trait `Dialect` e le tre strategie Bash, PowerShell, Cmd |
| `crates/xiox-project/src/provenance/lex.rs` | `Token`, `lex`: parole, operatori, heredoc |
| `crates/xiox-project/src/provenance/command.rs` | `Arg`, `Simple`, `Node`, `parse`: comandi semplici, assegnazioni, cicli `for` |
| `crates/xiox-project/src/provenance/args.rs` | Parametri POSIX e PowerShell |
| `crates/xiox-project/src/provenance/recognize/mod.rs` | `Invocation`, `Context`, trait `Recognizer`, `Registry` |
| `crates/xiox-project/src/provenance/recognize/{location,directories,transfer,archives,writers,git,packages,shells,interpreters,output_flags}.rs` | Un riconoscitore per famiglia di comandi |
| `crates/xiox-project/src/provenance/script.rs` | `Lang`, tabella `Call`, `ScriptScan`, `scan_script` |
| `crates/xiox-project/src/provenance/budget.rs` | `Budget`: tetti di espansione, profondità e operazioni |
| `crates/xiox-project/src/provenance/extract.rs` | `extract`: la pipeline con variabili, cicli, script e budget |
| `crates/xiox-project/src/provenance/patch.rs` | `PatchAction`, `patch_ops`: le intestazioni di `apply_patch` |
| `crates/xiox-project/src/provenance/call.rs` | `ShellCommand`, `shell_command`: quali chiamate sono shell e in che dialetto |
| `crates/xiox-project/src/provenance/ledger.rs` | `Strength`, `Outcome`, `SessionKey`, `Source`, `EvidenceRef`, `FolderEvidence`, `Coverage`, `Ledger`, `folder_of` |
| `crates/xiox-project/src/tests/provenance/{mod,fixtures,paths,lex,command,args,recognize,script,extract,patch,call,ledger}.rs` | Test guidati dai dati, uno per stadio |
| `crates/xiox-project/tests/data/shell-events.toml` | Le 106 fixture, spostate da `bench/path-extractor/events.toml` |
| `crates/xiox-mukade/src/calls.rs` | `ToolCall`, `Outcome`, `cut_input`, `parse_stream`; `MukadeSessions::dump_calls` e la preparazione di OpenCode |
| `crates/xiox-project/examples/shell_probe.rs` | Sonda locale: risultati Rust da un `calls.jsonl` |
| `bench/path-extractor/real.py` | Sottocomando `parity` che confronta Python e Rust |

---

### Task 1: Tipi delle operazioni e fixture guidate dai dati

**Files:**
- Create: `crates/xiox-project/src/provenance/mod.rs`, `crates/xiox-project/src/provenance/op.rs`
- Create: `crates/xiox-project/src/tests/provenance/mod.rs`, `crates/xiox-project/src/tests/provenance/fixtures.rs`
- Move: `bench/path-extractor/events.toml` → `crates/xiox-project/tests/data/shell-events.toml` (`git mv`)
- Modify:
  - `crates/xiox-project/src/lib.rs`: `mod provenance;`, `pub use provenance::…`;
  - `crates/xiox-project/src/tests/mod.rs`: `mod provenance;`;
  - `crates/xiox-project/Cargo.toml`: `[dev-dependencies] toml = "0.8"`;
  - `bench/path-extractor/bench.py`: default di `--events`;
  - `AGENTS.md`: percorso delle fixture.

**Interfaces:**
- Produces:
  - `pub enum OpKind { Create, Copy, Move, Clone, Worktree, Write }`
    - `Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize`, kebab-case
    - `fn as_str(self) -> &'static str`
  - `pub enum RawOp`, con tre varianti:
    - `Path { kind: OpKind, src: Option<String>, dst: Option<String>, base: Option<String>, script: bool }`
    - `Cd { dst: Option<String>, push: bool }`
    - `PopCd`
  - `#[derive(Default)] pub struct Flags { pub looped: bool, pub script: bool, pub truncated: bool, pub exhausted: bool }`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Completeness { Whole, Truncated }`
  - `pub struct Extraction { pub ops: Vec<RawOp>, pub flags: Flags, pub unresolved: usize }`
    - `unresolved`: quanto l'estrattore non ha potuto leggere, cioè argomenti non letterali negli script, operazioni oltre il budget e il comando finale di un testo tagliato;
    - i percorsi irrisolti dal resolver si sommano dopo, in `Resolved`.
  - `pub struct Candidate { pub kind: OpKind, pub src: Option<String>, pub dst: String, pub script: bool }`
  - nei test, `fn load_fixtures() -> Fixtures`: carica il file da `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/shell-events.toml")`
    - `Fixtures { env: (home, temp), events: Vec<Fixture> }`
    - `Fixture { id, cat, shell, cwd, cmd (trim), exit, gold: Vec<(OpKind, Option<String>, String)> }`

- [ ] **Step 1: Write the failing test** `fixtures_load_and_cover_every_category` in `tests/provenance/fixtures.rs`:
  - `load_fixtures().events.len() == 106`;
  - le categorie includono `create, copy, move, clone, worktree, write, readonly, delete, chain, spaces, failed, variable, loop, script, nested, tool-flag`;
  - ogni `gold.dst` è assoluto (`X:\`);
  - `cwd` di default `D:\Projects\Phazly`.
- [ ] **Step 2: Run test to verify it fails.** Run: `cargo test -j 1 -p xiox-project provenance::fixtures`. Expected: FAIL (modulo o file mancanti).
- [ ] **Step 3: Implement.** Il lavoro comprende:
  - `git mv` delle fixture;
  - i tipi di `op.rs`;
  - il loader con `toml::from_str` su strutture `Deserialize` private ai test;
  - il default `--events` di `bench.py` sul nuovo percorso.
- [ ] **Step 4: Run test to verify it passes.** Run il comando dello Step 2: PASS. Poi `python bench/path-extractor/bench.py --out <scratch>/det.json` stampa ancora `exact 105/106 … invented 0`.
- [ ] **Step 5: Commit** — `feat(xiox-project): tipi delle operazioni e fixture dell'estrattore`.

### Task 2: Resolver dei percorsi

**Files:**
- Create: `crates/xiox-project/src/provenance/paths.rs`, `crates/xiox-project/src/tests/provenance/paths.rs`

**Interfaces:**
- Consumes: `RawOp`, `Candidate`, `OpKind` (Task 1).
- Produces:
  - `pub struct Env { pub home: String, pub temp: String }`
  - `pub enum PathError { Unknown, NoCwd, Unsupported }`
  - `pub fn resolve_path(p: &str, cwd: Option<&str>, env: &Env) -> Result<String, PathError>`
  - `pub fn strip_verbatim(p: &str) -> &str`: toglie `\\?\`; la usano anche la cwd delle chiamate e l'ambiente di OpenCode
  - `pub fn is_null_device(p: &str) -> bool`: vero per `/dev/null`, `nul`, `nul:`, `$null`, `-`, senza distinguere maiuscole
  - `pub struct Resolved { pub candidates: Vec<Candidate>, pub visits: Vec<String>, pub unresolved: usize }`
    - `visits`: ogni cartella in cui un `cd` o un `pushd` è entrato, risolta contro la cwd di quel momento, nell'ordine; `cd .` e `PopCd` non sono visite (D1)
  - `pub fn resolve(ops: &[RawOp], cwd: &str, env: &Env) -> Resolved`

- [ ] **Step 1: Write the failing tests**, una tabella `(input, cwd, atteso)` in `resolve_path_table`:
  - `/d/runs/x` → `D:\runs\x`
  - `/tmp/a` → `<temp>\a`
  - `~/s` → `<home>\s`
  - `../b` da `D:\P\q` → `D:\P\b`
  - `D:/x/` → `D:\x`
  - `$OUT/a`, `%TEMP%\a`, `` `pwd` ``, `*.json` → `Err(Unknown)`
  - `/usr/bin` e `D:foo` → `Err(Unsupported)`
  - relativo con cwd `None` → `Err(NoCwd)`
  - `\\server\s` resta invariato

  Gli altri test:
  - `resolve_tracks_cd_push_pop_and_base`: `Cd /d/runs` + create `a`, `Cd{push} x` + `PopCd`, `base: D:\P` dà i candidati attesi.
  - `visits_follow_each_cd_where_it_ran`: da `D:\P`, `cd /d/a; cd b; pushd c; popd; cd .` dà `visits == [D:\a, D:\a\b, D:\a\b\c]`, lo stesso risultato del prototipo corretto.
  - `clone_without_destination_takes_repo_name`: `https://h/acme/planner.git` e `git@h:acme/docs.git` → `<cwd>\planner`, `<cwd>\docs`.
  - `copy_without_destination_is_cwd`.
  - `null_devices_are_dropped_not_counted`: `unresolved == 0`.
  - `unknown_cd_poisons_relative_paths_after_it`: dopo `cd $X`, il percorso `a` relativo conta come irrisolto e non dà visite.
  - `unresolved_src_keeps_the_op`: `src: None`, `dst` risolto.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::paths`. Expected: FAIL.
- [ ] **Step 3: Implement** `paths.rs` su stringhe, come `resolve_path` e `resolve(..., visits)` del prototipo.
  - Normalizzazione `.` e `..` scritta a mano su componenti separati da `\`; nessun accesso al disco.
  - Un componente `..` oltre la radice si ferma alla radice: `D:\..\x` → `D:\x`, come `ntpath.normpath`.
  - URL di `src` riconosciuti da `://` oppure dalla forma `user@host:`.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): resolver dei percorsi scritti nei comandi`.

### Task 3: Dialetti e lessico

**Files:**
- Create: `crates/xiox-project/src/provenance/dialect.rs`, `crates/xiox-project/src/provenance/lex.rs`, `crates/xiox-project/src/tests/provenance/lex.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Shell { Bash, PowerShell, Cmd }`
  - `impl Shell { pub fn dialect(self) -> &'static dyn Dialect }`
  - `pub trait Dialect: Sync`, con i metodi:
    - `fn escape(&self) -> Option<char>`: `\`, `` ` ``, `^`
    - `fn single_quotes(&self) -> bool`: Cmd falso; in PowerShell `''` dentro le virgolette singole vale `'`
    - `fn escapes_in_double(&self, next: char) -> bool`: Bash solo per `" \ $ ` `` ` ``; PowerShell `` ` ``; Cmd mai
    - `fn operators(&self) -> &'static [&'static str]`: le liste di `OPERATORS` del prototipo, dalla più lunga; comprendono `>&` per PowerShell e Cmd e `,` `{` `}` per PowerShell
    - `fn shell(&self) -> Shell`
  - `pub enum Token { Word(String), Op(&'static str), Heredoc(Option<String>) }`
  - `pub fn lex(cmd: &str, dialect: &dyn Dialect) -> Vec<Token>`

- [ ] **Step 1: Write the failing tests:**
  - `bash_backslashes_follow_bash_quoting`:
    - `mkdir "D:\runs\x"` → `Word("D:\\runs\\x")`
    - `echo \"a\"` → `"a"`
    - `a\ b` → `a b`
  - `powershell_quotes`: `'it''s'` → `it's`; `` "a`"b" `` → `a"b`.
  - `cmd_has_no_single_quotes`.
  - `fd_redirects_lex_as_redirect`: `2>`, `*>`, `2>&1` diventano `Op(">")` oppure `Op(">&")` seguiti da parola.
  - `dollar_paren_stays_in_one_word`: `cp src "$(mktemp -d)"` e `$(a (b))`.
  - `newline_is_a_separator`.
  - `operators_after_heredoc_delimiter_are_kept`: `python - <<'EOF' > /d/o.json\nprint(1)\nEOF` dà `Heredoc(Some("print(1)\n"))`, poi `Op(">")` e `Word("/d/o.json")`.
  - `heredoc_delimiter_tolerates_crlf` (D2): `cat <<EOF\r\nx\r\nEOF\r\nmkdir /d/after` dà il corpo `x\r\n` e poi `Word("mkdir")` e `Word("/d/after")`.
  - `unterminated_quote_and_heredoc_end_the_input`: nessun panic; l'heredoc prende il resto.
  - `non_ascii_words`: `mkdir "D:\Città\è"`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::lex`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Strategie: tre struct unitarie che implementano `Dialect`.
  - `lex` scorre per `char_indices`, mai indici di byte calcolati a mano.
  - Gli heredoc in attesa sono `(delim, indice del token)`. Al primo a capo, il corpo si chiude alla riga `^\t*<delim>[ \t\r]*$`.
  - `<<-` è accettato.
  - Un commento in testa a `lex` dichiara D2.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): dialetti shell e lessico dei comandi`.

### Task 4: Struttura del comando

**Files:**
- Create: `crates/xiox-project/src/provenance/command.rs`, `crates/xiox-project/src/tests/provenance/command.rs`

**Interfaces:**
- Consumes: `Token`, `Dialect` (Task 3).
- Produces:
  - `pub enum Arg { Text(String), Comma }`
  - `pub struct Simple { pub words: Vec<Arg>, pub redirects: Vec<String>, pub heredoc: Option<String> }`
  - `pub enum Node`, con tre varianti:
    - `Simple(Simple)`
    - `Assign { name: String, value: String }`
    - `For { var: String, items: Option<Vec<String>>, body: Vec<Node>, done_redirects: Vec<String> }`
  - `pub fn parse(tokens: Vec<Token>, dialect: &dyn Dialect) -> Vec<Node>`: i nodi di primo livello, nell'ordine; l'ultimo è quello che raggiunge la fine del testo

- [ ] **Step 1: Write the failing tests:**
  - `separators_split_simple_commands`: `&&`, `||`, `;`, `|`, `&`, parentesi e graffe PowerShell.
  - `redirect_targets_leave_the_words`:
    - `cargo bench > out.txt 2>&1` dà `words [cargo, bench]` e `redirects ["out.txt"]`;
    - `<` e `>&` consumano la loro parola.
  - `assignments_per_dialect`:
    - `W=/d/r` → `Assign`
    - `$o = 'D:\x'` → `Assign { name: "o" }` in minuscolo
    - `$o='D:\x'` → `Assign { name: "o" }`
    - `set X=v` → `Assign { name: "x" }`
  - `leading_assignments_are_dropped_from_a_command`: `X=1 Y=2 mkdir /d/r` dà un solo `Simple` con `words [mkdir, /d/r]` e nessun `Assign`. Come in Bash, il valore vale solo per quel comando.
  - `bash_for_loop_collects_body_until_matching_done`: `for i in 1 2; do mkdir $i; for j in a; do :; done; done > log` dà:
    - `items Some([1,2])`;
    - il corpo col `for` annidato;
    - `done_redirects ["log"]`.
  - `for_over_arithmetic_or_missing_in_has_no_items`: `for ((i=0;i<3;i++))` → `items None`.
  - `keywords_do_then_else_are_stripped`.
  - `powershell_commas_are_kept_as_args`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::command`. Expected: FAIL.
- [ ] **Step 3: Implement** `parse`.
  - Nodi piatti: un `Simple` per segmento, `Assign` riconosciuti dalle regole di `assignment` del prototipo.
  - Solo per Bash: raccolta di `For` con il conteggio della profondità su `for`, `while`, `until`, `select` / `done`.
  - Dalla testa di `Simple.words` si tolgono:
    - i prefissi `NAME=value` che precedono un comando;
    - i prefissi `sudo env time command exec nohup do then else elif if while until & ! { call start`.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): struttura dei comandi shell`.

### Task 5: Parametri POSIX e PowerShell

**Files:**
- Create: `crates/xiox-project/src/provenance/args.rs`, `crates/xiox-project/src/tests/provenance/args.rs`

**Interfaces:**
- Consumes: `Arg` (Task 4).
- Produces:
  - `pub struct Posix { pub positional: Vec<String>, pub options: HashMap<String, Option<String>> }`, dove `None` è un interruttore
  - `pub fn posix(args: &[Arg], with_value: &[&str]) -> Posix`: `--` chiude le opzioni; gestisce `--k=v`
  - `pub struct PsParams { pub positional: Vec<Vec<String>>, pub named: HashMap<&'static str, Vec<String>>, pub switches: HashSet<String> }`
  - `pub fn powershell(args: &[Arg], params: &[(&str, &'static str)]) -> PsParams`
    - `params` sono coppie (nome o alias minuscolo, chiave canonica);
    - prefisso univoco, `-Name:value`, array con virgole;
    - un parametro a valore senza valore va negli interruttori.

- [ ] **Step 1: Write the failing tests:**
  - `posix_values_switches_and_double_dash`: `-m 755 -p a -- -b` con `with_value ["-m"]`.
  - `powershell_prefix_alias_inline_and_arrays`:
    - `-Dest D:\x` → `dst`
    - `-it Directory` → `itemtype`
    - `-Path:D:\a`
    - `-Path a, b` → `["a","b"]`
    - `-Recurse` → interruttore
  - `powershell_value_param_without_value_is_a_switch`: `New-Item -Path` non va in panic e non dà `named["path"]`.
  - `powershell_ambiguous_prefix_is_a_switch`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::args`. Expected: FAIL.
- [ ] **Step 3: Implement** `args.rs`, come `posix` e `ps` del prototipo.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): parametri POSIX e PowerShell`.

### Task 6: Registro e riconoscitori di file system

**Files:**
- Create: `crates/xiox-project/src/provenance/recognize/mod.rs`
- Create: `crates/xiox-project/src/provenance/recognize/{location,directories,transfer,archives,writers,output_flags}.rs`
- Test: `crates/xiox-project/src/tests/provenance/recognize.rs`

**Interfaces:**
- Consumes: `Arg`, `Simple` (Task 4); `posix`, `powershell` (Task 5); `RawOp`, `Flags` (Task 1); `Shell` (Task 3).
- Produces:
  - `pub struct Invocation<'a> { pub name: String, pub args: &'a [Arg], pub shell: Shell, pub heredoc: Option<&'a str> }`
    - `name`: basename in minuscolo, senza `.exe`
  - `pub struct Context { pub flags: Flags, pub unresolved: usize, pub scripts: HashMap<String, String> }`
    - il `Budget` si aggiunge nel Task 8
  - `pub trait Recognizer: Sync`, con due metodi:
    - `fn names(&self) -> &'static [&'static str]`
    - `fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp>`
  - `pub struct Registry`, con:
    - `pub fn standard() -> &'static Registry` (`OnceLock`)
    - `pub fn recognize(&self, inv: &Invocation, ctx: &mut Context) -> Vec<RawOp>`

  `Registry::recognize` cerca per nome:
  - i nomi `for foreach foreach-object %` mettono `flags.looped` e restituiscono vuoto;
  - se nessuna operazione esce, applica il fallback di `output_flags`: `--target-dir --basetemp --out --output --out-dir --outdir --output-dir --output-directory` → `Write`.

  Ogni nome appartiene a un solo riconoscitore. Un comando con più forme, come `bun`, smista internamente (Task 8).
- Riconoscitori di questo task, con i nomi del prototipo:
  - `location`: `cd chdir set-location sl pushd push-location popd pop-location`
  - `directories`: `mkdir md new-item ni`
  - `transfer`: `cp copy copy-item cpi mv move move-item mi robocopy xcopy`
  - `archives`: `tar bsdtar unzip expand-archive`
  - `writers`: `tee tee-object out-file set-content add-content sc ac touch sed`

  In PowerShell, `cp`, `mv`, `mkdir` e `tee` usano la grammatica dei cmdlet. `robocopy` e `xcopy` usano la grammatica Cmd da qualsiasi shell.

- [ ] **Step 1: Write the failing tests**, una tabella `(shell, comando, RawOp attese)` in `file_system_recognizers`, con almeno:
  - `cd /d D:\runs`
  - `mkdir -p a b` → due create
  - `New-Item -Path D:\runs -Name r -ItemType Directory` → `D:\runs\r`
  - `New-Item f.txt` → write
  - `cp -t /d/x a b`
  - `Copy-Item .\data D:\s -Recurse` → src `.\data`, dst `D:\s`
  - `robocopy src dst /E /XD node_modules`
  - `xcopy /E /I a D:\b`
  - `tar -xzf a.tgz -C /d/x` → copy
  - `tar -czf out.tgz dir` → write
  - `unzip -q d.zip -d /d/u`
  - `Expand-Archive -Path r.zip -DestinationPath D:\e`
  - `sed -i 's/a/b/' x.rs y.md` → due write
  - `sed -n 1p f` → nulla
  - `pytest --basetemp=/d/p` → write
  - `Copy-Item a` senza destinazione → nulla

  Gli altri test:
  - `loop_names_set_the_flag`.
  - `registry_names_are_unique`: nessun nome in due riconoscitori. La costruzione di `Registry::standard` fallisce con `debug_assert!` su un duplicato.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::recognize`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Ogni famiglia è una struct unitaria che implementa `Recognizer`.
  - `Registry::standard` costruisce la mappa da `names()`.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): registro dei riconoscitori e comandi di file system`.

### Task 7: Analizzatori di script

**Files:**
- Create: `crates/xiox-project/src/provenance/script.rs`, `crates/xiox-project/src/tests/provenance/script.rs`

**Interfaces:**
- Consumes: `RawOp`, `OpKind` (Task 1).
- Produces:
  - `pub enum Lang { Python, JavaScript }`
  - `pub struct ScriptScan { pub ops: Vec<RawOp>, pub unresolved: usize }`
    - `ops`: operazioni con `script: true`, nell'ordine in cui compaiono;
    - `unresolved`: chiamate della tabella il cui argomento percorso non è un letterale intero (D6).
  - `pub fn scan_script(code: &str, lang: Lang) -> ScriptScan`
  - privata, `struct Call { callee: &'static [&'static str], arg: usize, kind: OpKind, then: Option<&'static [&'static str]>, mode: Option<usize> }`

  La tabella di chiamate, uguale a `CALLS` del prototipo:
  - Python: `open` (mode 1, deve contenere `w`, `a` o `x`), `os.makedirs` `os.mkdir`, `Path(…).mkdir`, `Path(…).write_text` `.write_bytes` `.touch`, `shutil.copytree` `copy` `copy2` `copyfile` (arg 1), `shutil.move` (arg 1);
  - JS, con qualsiasi ricevitore: `writeFileSync writeFile appendFileSync createWriteStream`, `mkdirSync mkdir`, `copyFileSync copyFile cpSync` (arg 1), `renameSync` (arg 1).

  Un chiamante è valido solo se il carattere che lo precede non è un carattere di identificatore Unicode, cioè `!(c.is_alphanumeric() || c == '_')`, come `(?<!\w)` del prototipo.

  Un `open` con mode non letterale non è una scrittura certa: non dà operazioni e non viene conteggiato.

- [ ] **Step 1: Write the failing tests:**
  - `python_literal_calls`:
    - `os.makedirs('D:/r', exist_ok=True)` → create
    - `open("a.txt", "w")` → write
    - `open("a.txt")` → nulla
    - `open('x', mode='wb')` → write
    - `Path("D:/p").mkdir(parents=True)` → create
    - `shutil.copytree("src", "D:/t")` → copy `D:/t`
    - `r"D:\x"` resta `D:\x`
    - `"D:\\x"` → `D:\x`
  - `js_literal_calls`:
    - `require('fs').mkdirSync('D:/n', {recursive: true})`
    - `fs.writeFileSync("D:/p.json", JSON.stringify({ ok: true }))`
  - `unicode_identifier_boundary`: `éopen('D:/ghost','w')` e `_open('D:/g','w')` → nulla; `(open('D:/k','w'))` → write.
  - `calls_inside_string_literals_are_text` (D5): `print("open('D:/x','w')")` → nulla.
  - `non_literals_are_counted` (D6): nessuna operazione e `unresolved == 1` per ognuno di questi casi:
    - `os.makedirs(f"{d}/x")`
    - `open("a" + b, "w")`
    - `` fs.mkdirSync(`${r}/x`) ``
    - `open("{}/x".format(d), "w")`
    - `os.makedirs(os.path.join(a, "b"))`
    - `os.makedirs(root)`
  - `nested_calls_are_read_once`: `shutil.copy(src, os.path.join("D:/a", n))` → `unresolved == 1`, nessuna operazione.
  - `truncated_call_gives_nothing`: `open("a.txt", "w` a fine testo dà nulla e nessun panic.
  - `adversarial_scripts_scan_once`: 50.000 ripetizioni di `open(` non chiuse e 20.000 `\\` dentro un `open("…` senza chiusura danno nessuna operazione e nessun panic. Il limite è strutturale (Step 3); un tetto di tempo largo, 2 s anche in debug, fa solo da guardia contro regressioni grossolane.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::script`. Expected: FAIL.
- [ ] **Step 3: Implement** `scan_script` come una sola scansione `char_indices` con una pila di chiamate aperte.
  - Fuori dai letterali, a ogni confine di identificatore si prova la tabella dei chiamanti. Una corrispondenza apre una cornice sulla pila quando segue `(`.
  - Letterali (`'…'`, `"…"`, triple, prefissi `r`/`f`/`b`; in JS anche `` `…` ``) e parentesi annidate fanno avanzare la cornice in cima senza rileggere testo.
  - Gli argomenti si delimitano sulle virgole di profondità zero della cornice.
  - Alla `)` che chiude, la cornice si valuta:
    - `then` si controlla con un lookahead di al massimo 64 caratteri;
    - `literal` accetta solo un letterale intero senza `{`, `}`, a capo o prefisso `f`/`b`.
  - Limiti costanti: la pila ha al massimo 64 cornici; oltre, le nuove aperture si ignorano.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): chiamate con percorso letterale negli script`.

### Task 8: Riconoscitori di strumenti e pipeline di estrazione

**Files:**
- Create:
  - `crates/xiox-project/src/provenance/recognize/{git,packages,shells,interpreters}.rs`
  - `crates/xiox-project/src/provenance/{budget,extract}.rs`
  - `crates/xiox-project/src/tests/provenance/extract.rs`
- Modify: `crates/xiox-project/src/provenance/recognize/mod.rs` (registrazione, `Context.budget`), `crates/xiox-project/src/provenance/mod.rs`

**Interfaces:**
- Consumes: Task 3–7.
- Produces:
  - `pub struct Budget`, con tre costanti pubbliche e un metodo:
    - `MAX_LOOP_ITEMS = 50`: elementi di un ciclo, come il prototipo;
    - `MAX_DEPTH = 4`: annidamento di cicli e shell annidate insieme;
    - `MAX_OPS = 500`: operazioni per chiamata;
    - `fn take(&mut self) -> bool`: falso quando le operazioni sono finite.
  - `pub fn extract(cmd: &str, shell: Shell, completeness: Completeness) -> Extraction`
  - riconoscitori, coi nomi del prototipo:
    - `git`: `-C`, `clone`, `worktree add`, `init <dir>`, `--help`
    - `packages`: `cargo new/init`, `npm pnpm yarn create/init`, `npx pnpx bunx create-*`, `uv init/venv`, `dotnet new -o/-n`, `virtualenv`
    - `shells`:
      - `bash sh zsh dash` con `-c`, `powershell pwsh` con `-Command`, `cmd` con `/c` e `/k`;
      - ricorsione su `extract`, che consuma un livello di `MAX_DEPTH`, con `Cd{push} "."` prima e `PopCd` dopo;
      - `-File` e `-EncodedCommand` mettono `script`.
    - `interpreters`: `python python3 py node deno bun`.
      - `python -m venv` → create.
      - La sorgente è `-c`/`-e`/`--eval`/`-p`/`--print`, l'heredoc, oppure un file in `ctx.scripts`. Mette `script`, chiama `scan_script` e somma il suo `unresolved` a `ctx.unresolved`.
      - Un interprete con heredoc mette sempre `script`.
      - **Proprietario unico di `bun`:** `bun create …` e `bun init …` vanno alla stessa funzione di `packages`; ogni altra forma è un interprete.

  La pipeline di `extract`:
  - percorre i `Node` con le variabili note.
    - `Assign` con valore letterale (dopo la sostituzione dei nomi già noti) registra il nome.
    - `Assign` con valore non letterale **toglie** il nome dalle variabili note, così un uso successivo resta irrisolto.
    - La sostituzione vale solo per i nomi noti: `$x` e `${x}` in Bash; senza maiuscole in PowerShell; `%x%` in Cmd.
  - `For` con elementi letterali (al massimo `MAX_LOOP_ITEMS`, nessun `$ ` * ? [ ] { } ( )`) viene espanso una volta per elemento, consumando un livello di `MAX_DEPTH`. Altrimenti mette `looped` e percorre il corpo una volta.
  - `cat`/`tee` con heredoc e un bersaglio registrano `ctx.scripts[target] = body`.
  - Budget (D4): quando `take` fallisce o la profondità supera `MAX_DEPTH`, il nodo non viene percorso. Si mettono `flags.exhausted` e `unresolved += 1` una volta per nodo saltato.
  - Testo tagliato (D3): con `Completeness::Truncated` l'ultimo nodo di primo livello di `parse` non viene percorso. Si mettono `flags.truncated` e `unresolved += 1`. Il taglio può cadere dentro una parola, una virgoletta o un heredoc: la parte che resta non dice nulla di certo.
  - `Extraction.unresolved = ctx.unresolved`.

- [ ] **Step 1: Write the failing tests:**
  - `every_fixture_is_exact_except_known_limits`: per ogni fixture, `resolve(extract(cmd, shell, Whole).ops, cwd, env)` dà lo stesso insieme `(op, dst minuscolo)` di `gold`. Unica eccezione ammessa, scritta nel test: `["u06"]`, cioè `ForEach-Object`, limite della specifica.
  - `no_fixture_invents_a_path`: nessun candidato fuori da `gold` in nessuna fixture, compresa u06.
  - `no_fixture_exhausts_the_budget`: `flags.exhausted == false` su tutte le fixture.
  - `every_prefix_of_every_fixture_extracts_without_panic`: ogni prefisso, sui confini di `char`, di ogni `cmd`, sia `Whole` sia `Truncated`.
  - `truncated_prefixes_never_invent` (D3): per ogni prefisso proprio di ogni fixture estratto come `Truncated`, ogni candidato è in `gold` della fixture intera. Il caso nominato: `mkdir D:\keep; mkdir "D:\runs\destination` tagliato dopo `destin` dà solo `D:\keep`; lo stesso testo come `Whole` dà anche `D:\runs\destin`, come il prototipo.
  - `command_after_crlf_heredoc_is_extracted` (D2): `cat <<EOF > /d/a.txt\r\nx\r\nEOF\r\nmkdir /d/after` → write `D:\a.txt` e create `D:\after`.
  - `nested_loops_and_shells_stop_at_the_budget` (D4):
    - tre `for` annidati da 50 elementi letterali con `mkdir $a$b$c` danno al massimo `MAX_OPS` operazioni, `exhausted` e `unresolved > 0`;
    - 10 livelli di `bash -c` annidati danno `exhausted` e nessun panic;
    - `for i in 1 2; do mkdir /d/$i; done` resta esatto.
  - `bun_has_one_owner_and_three_forms`: `bun create vite D:\a` → create; `bun init D:\b` → create; `bun -e "require('fs').mkdirSync('D:/c')"` → create con `script`.
  - `unknown_values_are_counted_not_guessed`: questi comandi danno 0 candidati e `extraction.unresolved + resolved.unresolved > 0`:
    - `New-Item "$env:TEMP\x"`
    - `mkdir %TEMP%\y`
    - `cp a "$(mktemp -d)"`
    - `` mkdir `pwd`/z ``
    - `T=$(mktemp -d); mkdir $T/x`
    - `T=/d/old; T=$(mktemp -d); mkdir $T/x`: la riassegnazione toglie il valore letterale
    - `python -c "import os; os.makedirs(f'{r}/x')"`
  - `prefix_assignment_reaches_the_command`: `X=1 mkdir /d/r` → create `D:\r`.
  - `flags_mark_scripts_and_loops`: x01 → `script`; x13 → `looped`; c01 → nessun flag.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::extract`. Expected: FAIL.
- [ ] **Step 3: Implement** `Budget`, i quattro riconoscitori ed `extract`. Le divergenze D3 e D4 hanno ognuna un commento nel punto del codice che le applica.
- [ ] **Step 4: Run** il comando dello Step 2, poi `cargo test -j 1 -p xiox-project`. Expected: PASS, compresi i test esistenti di `analyze` e `clean`.
- [ ] **Step 5: Commit** — `feat(xiox-project): estrazione deterministica dei percorsi dai comandi`.

### Task 9: Patch

**Files:**
- Create: `crates/xiox-project/src/provenance/patch.rs`, `crates/xiox-project/src/tests/provenance/patch.rs`

**Interfaces:**
- Consumes: `Completeness` (Task 1).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] pub enum PatchAction { Add, Update, Delete, MoveFrom, MoveTo }`, kebab-case
  - `pub fn patch_ops(text: &str, completeness: Completeness) -> Vec<(PatchAction, String)>`
    - legge le righe `*** Add File: <p>`, `*** Update File: <p>`, `*** Delete File: <p>` e `*** Move to: <p>`, il formato `apply_patch` di Codex;
    - un `Move to` trasforma l'`Update File` che lo precede in `MoveFrom`;
    - con `Truncated` l'ultima riga si scarta se il testo non finisce con un a capo;
    - i percorsi restano come scritti: li risolve il registro contro la cwd della chiamata.

  Nella cronologia, 238 chiamate `apply_patch` di Codex hanno canonico `edit` e input stringa: 133 Add, 279 Update, 6 Delete. 90 di queste chiamate sono tagliate.

- [ ] **Step 1: Write the failing tests**, una tabella in `patch_headers`:
  - un Add, un Update, un Delete;
  - un Update seguito da `Move to` → `MoveFrom` e `MoveTo`;
  - righe di contenuto che iniziano con `+*** Add File:` non contano;
  - un testo tagliato a metà della riga `*** Add File: D:/a/b` dà niente per quella riga con `Truncated` e la riga intera con `Whole`;
  - righe con `\r\n`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::patch`. Expected: FAIL.
- [ ] **Step 3: Implement** `patch.rs` con una scansione per righe.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): azioni distinte delle patch`.

### Task 10: Chiamate di tool da Mukade

**Files:**
- Create: `crates/xiox-mukade/src/calls.rs`
- Modify: `crates/xiox-mukade/src/lib.rs`
  - `mod calls; pub use calls::{ToolCall, Outcome, cut_input, parse_stream, CUT_MARK, MAX_INPUT_CHARS};`
  - un `fn mukade_command(&self, store: &Path) -> anyhow::Result<Command>` estratto da `run_list`: exe, `MUKADE_DATA_DIR`, `MUKADE_INTENT_DIR`, variabile dello store, `stdin` nullo, `CREATE_NO_WINDOW`. È usato da `run_list`, che passa lo store originale e resta invariato, e da `dump_command`.
  - `pub fn with_opencode_exe(self, exe: Option<PathBuf>) -> Self`; `from_env` lo legge da `XIOX_OPENCODE_EXE`.
- Test: i test di `calls.rs` stanno nel blocco `#[cfg(test)] mod tests` dello stesso file, come nel resto del crate

**Interfaces:**
- Produces:
  - `pub const MAX_INPUT_CHARS: usize = 4000; pub const CUT_MARK: char = '…';`
  - `pub fn cut_input(v: Value) -> (Value, bool)`: taglia ogni stringa oltre `MAX_INPUT_CHARS` caratteri e aggiunge `CUT_MARK`, come `cut` di `harvest.py`; `true` se ha tagliato
  - `pub fn was_cut(s: &str) -> bool`: `s.chars().count() == MAX_INPUT_CHARS + 1 && s.ends_with(CUT_MARK)`. Riconosce il taglio anche nelle righe di `calls.jsonl` scritte da `harvest.py`.
  - `pub enum Outcome { Succeeded, Failed, Unknown }`, `Clone, Copy, Debug, PartialEq, Eq, Serialize`
  - `pub struct ToolCall`, `Clone, Debug, Serialize, Deserialize`, con gli stessi nomi di campo delle righe scritte da `bench/path-extractor/harvest.py`:
    - `harness: String`, `store: String`, `session: String`, `parent: Option<String>`
    - `entry`, `call`, `tool`, `cwd`, `time`: `Option<String>`
    - `canonical: Option<String>`, solo quando è una stringa
    - `input: serde_json::Value`
    - `error: Option<bool>`, `exit: Option<i64>`
    - `result_head: Option<String>`: `None` se nessun risultato è stato abbinato
  - `impl ToolCall { pub fn outcome(&self) -> Outcome }`, con tre casi:
    - `Failed` se `error == Some(true)` oppure `exit` vale qualcosa diverso da `0`;
    - altrimenti `Unknown` se `result_head` è `None`;
    - altrimenti `Succeeded`.
  - `pub fn parse_stream(stream: &str, harness: &str, store: &str, session: &str, session_cwd: Option<&str>) -> Vec<ToolCall>`
  - `impl MukadeSessions`, con due metodi:
    - `pub fn dump_calls(&self, id: &str, session_cwd: Option<&str>) -> anyhow::Result<Vec<ToolCall>>`: esegue `mukade dump <harness> <id> --stream`;
    - `fn dump_command(&self, id: &str, store: &Path) -> anyhow::Result<Command>`: privata, testabile senza eseguire.
  - per OpenCode, privata, `fn opencode_snapshot(db: &Path, into: &Path) -> std::io::Result<PathBuf>`: copia `opencode.db` e, se presenti, `opencode.db-wal` e `opencode.db-shm` in una `tempdir`

  Regole di `parse_stream`, come `calls_of` di `harvest.py`:
  - divide solo su `\n`;
  - ignora le righe non JSON, `meta` e i `kind` che non sono oggetti;
  - abbina `results[].call_id` alle chiamate;
  - legge `exit` da `exit code: N`, `exited with code N` o `exit status N`, senza maiuscole. La ricerca guarda i primi 2.000 **caratteri** del testo del risultato, poi gli ultimi 500 caratteri. I confini si prendono con `char_indices`, mai per byte.
  - `result_head`: i primi 300 caratteri;
  - cwd: prima `input.workdir`, `cwd`, `working_directory` o `directory`, poi `raw[0].cwd`, infine `session_cwd`;
  - `time`: `raw[0].timestamp` o `raw[0].time`;
  - `input` passa da `cut_input`.

  Preparazione di OpenCode in `dump_calls`, come in `AGENTS.md`:
  - lo store passato a `dump_command` è la copia di `opencode_snapshot`, mai l'originale. La `tempdir` vive fino alla fine di `dump_calls`.
  - si aggiunge `--opencode-exe <exe>`. Senza exe configurato, `dump_calls` restituisce un errore che lo nomina e non avvia Mukade.
  - `USERPROFILE`, `HOME`, `APPDATA`, `LOCALAPPDATA`, `TEMP` e `TMP` passano da `strip_verbatim`, la funzione del Task 2 copiata come funzione privata di una riga: `xiox-mukade` non dipende da `xiox-project`.

- [ ] **Step 1: Write the failing tests:**
  - `stream_pairs_calls_with_results`: due chiamate (Bash, Write), un risultato con `is_error: true` e testo `Exit code: 128`, una riga `{"meta":…}`, una riga con `\u2028` dentro una stringa.
  - `outcomes_are_three`: chiamata senza risultato → `Unknown`; risultato senza errore → `Succeeded`; `exit 2` → `Failed`.
  - `exit_windows_count_chars`, tre testi di 5.000 caratteri fatti di `è`:
    - `exit code: 3` che inizia al carattere 1.980 → `exit == Some(3)`. Il codice è dentro i primi 2.000 caratteri ma oltre i primi 2.000 byte, quindi una finestra in byte lo perderebbe.
    - `exit code: 4` che finisce all'ultimo carattere → `Some(4)`, dalla finestra di coda.
    - `exit code: 5` al carattere 3.000 → `None`: è fuori da entrambe le finestre, come nel prototipo.

    In nessuno dei tre casi c'è un panic sui confini multibyte.
  - `cut_input_marks_and_was_cut_recognizes`: 4.001 caratteri non ASCII → 4.000 + `…`, `true`; `was_cut` è vero sull'uscita e falso sull'originale corto.
  - `cwd_falls_back_in_order`.
  - `dump_calls_reads_a_fake_claude_store`:
    - store Claude Code finto in `tempdir`: `projects/<p>/<id>.jsonl` con un prompt, un `tool_use` Bash `mkdir /d/x` e il suo `tool_result`;
    - `MukadeSessions::with_roots` dà una `ToolCall` con `canonical == Some("bash")` e `store` uguale alla radice.
    - Se Mukade rifiuta i record minimi, si copia la forma del campo `raw` di un `dump --stream` di una sessione sintetica.
  - `opencode_dump_uses_a_copy_and_the_native_exe`:
    - un `opencode.db` finto con `-wal` in `tempdir`;
    - `dump_command` ha `--opencode-exe <exe>` negli argomenti;
    - `OPENCODE_DB` punta a un file diverso dall'originale, con lo stesso contenuto;
    - `USERPROFILE` e `TEMP` sono senza `\\?\` quando l'ambiente li ha con il prefisso.
  - `opencode_without_exe_is_an_error`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-mukade calls`. Expected: FAIL.
- [ ] **Step 3: Implement** `calls.rs` e il refactoring di `run_list`, che non cambia comportamento.
- [ ] **Step 4: Run** `cargo test -j 1 -p xiox-mukade`. Expected: PASS, compresi i test esistenti.
- [ ] **Step 5: Commit** — `feat(xiox-mukade): chiamate di tool da mukade dump --stream`.

### Task 11: Classificatore delle chiamate shell

**Files:**
- Create: `crates/xiox-project/src/provenance/call.rs`, `crates/xiox-project/src/tests/provenance/call.rs`

**Interfaces:**
- Consumes: `ToolCall`, `was_cut` (Task 10); `Shell` (Task 3); `Completeness` (Task 1).
- Produces:
  - `pub struct ShellCommand { pub shell: Shell, pub text: String, pub completeness: Completeness }`
  - `pub fn shell_command(call: &ToolCall) -> Option<ShellCommand>`: porta intera di `shell_of` di `real.py`, unica per registro e sonda. I suoi passi:
    - tabella `SHELLS` su `canonical`, poi sul nome del tool in minuscolo. I nomi sono `bash`, `terminal` → Bash; `power_shell`, `powershell` → PowerShell; `cmd` → Cmd; `shell`, `exec_command` → da indizio.
    - indizio da `input.shell` o `input.shell_type`: contiene `power` o `pwsh` → PowerShell; `cmd` → Cmd; altrimenti Bash.
    - testo da `input.command` o `input.cmd`. Una lista argv si unisce con spazi, mettendo tra virgolette doppie gli elementi con spazi.
    - testo vuoto o non stringa → `None`.
    - il testo si passa senza spazi in testa e in coda; `completeness` è `Truncated` se `was_cut` sul testo originale.

- [ ] **Step 1: Write the failing tests**, una tabella `(canonical, tool, input, atteso)` in `shell_selection_table`, con:
  - Claude `Bash`;
  - Codex `shell` con argv `["powershell.exe","-Command","mkdir D:\\x"]`;
  - Codex `exec_command` con `cmd` e `shell: "pwsh"`;
  - OpenCode `bash`;
  - un tool `terminal`;
  - `cmd`;
  - canonico non stringa con tool `PowerShell`;
  - `Read` → `None`;
  - comando vuoto → `None`;
  - testo tagliato → `Truncated`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::call`. Expected: FAIL.
- [ ] **Step 3: Implement** `call.rs` con la tabella come `const` e una funzione per passo.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): classificatore delle chiamate shell`.

### Task 12: Registro delle evidenze di sessione

**Files:**
- Create: `crates/xiox-project/src/provenance/ledger.rs`, `crates/xiox-project/src/tests/provenance/ledger.rs`

**Interfaces:**
- Consumes:
  - `ToolCall`, `Outcome` (Task 10);
  - `shell_command` (Task 11);
  - `extract` (Task 8);
  - `patch_ops`, `PatchAction` (Task 9);
  - `resolve`, `resolve_path`, `strip_verbatim`, `Env` (Task 2).
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)] pub enum Strength { Used = 1, Written = 2, Created = 3 }`
  - `#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)] pub struct SessionKey { pub store: String, pub harness: String, pub session: String }`
  - `pub enum Source`, con cinque varianti:
    - `Structured(String)`
    - `Shell { kind: OpKind, script: bool }`
    - `Patch(PatchAction)`
    - `Cd`
    - `Cwd`
  - `pub struct EvidenceRef`, con i campi:
    - `pub session: SessionKey`
    - `pub entry: Option<String>`, `pub call: Option<String>`
    - `pub time: Option<String>`
    - `pub cwd: Option<String>`: la cwd effettiva della chiamata, senza `\\?\`
    - `pub source: Source`
    - `pub outcome: Outcome`
  - `pub struct FolderEvidence { pub path: String, pub strength: Option<Strength>, pub refs: Vec<EvidenceRef>, pub failed: Vec<EvidenceRef> }`
  - `#[derive(Default)] pub struct Coverage { pub shell_calls: usize, pub unresolved: usize, pub looped: usize, pub script: usize, pub truncated: usize, pub exhausted: usize }`
    - `unresolved` è la somma di `Extraction.unresolved` e `Resolved.unresolved`;
    - gli altri campi contano le chiamate con quel flag.
  - `pub fn folder_of(kind: OpKind, path: &str) -> String`, con il commento `shortcut: un ultimo segmento con estensione è trattato come file, così --out D:\runs\v1.2 conta come file; da aggiornare quando il risultato del tool indica il tipo.`
  - `#[derive(Default)] pub struct Ledger`, con:
    - `pub fn record(&mut self, call: &ToolCall, env: &Env)`
    - `pub fn get(&self, folder: &str) -> Option<&FolderEvidence>`
    - `pub fn folders(&self) -> impl Iterator<Item = &FolderEvidence>`
    - `pub fn coverage(&self) -> &Coverage`
  - `pub fn session_ledgers<'a>(calls: impl IntoIterator<Item = &'a ToolCall>, env: &Env) -> BTreeMap<SessionKey, Ledger>`: raggruppa per chiave completa, non per il solo id

  Mappa delle forze, come `STRENGTH_OF` e `STRUCTURED` del prototipo:
  - destinazioni di `create`, `copy`, `move`, `clone`, `worktree` → creata;
  - `write` e Write/Edit/NotebookEdit strutturati → scritta, per la cartella del file;
  - Read come file, Glob/Grep come cartella, ogni `Resolved.visits` (D1) e la cwd → usata.

  Patch (D7): una chiamata il cui input è una stringa con `*** Begin Patch` passa da `patch_ops`, con `Truncated` se `was_cut`. I percorsi si risolvono contro la cwd della chiamata. Le forze sono:
  - `Add`, `Update`, `MoveTo` → scritta, per la cartella del file;
  - `Delete` e `MoveFrom` → usata. Una cancellazione non genera una nuova cartella.

  Esito: le evidenze di una chiamata `Failed` vanno in `failed`, senza forza. `Unknown` porta forza come `Succeeded`, come il prototipo, ma resta scritto nel riferimento.

- [ ] **Step 1: Write the failing tests:**
  - `strongest_evidence_wins_per_folder`: Read di `D:\r\a.txt`, poi `mkdir D:\r` → `Created`, con due riferimenti.
  - `folder_keys_ignore_case`.
  - `structured_and_shell_sources_are_distinguished`, compreso `Shell { script: true }` da `python -c "os.makedirs('D:/s')"`.
  - `cd_evidence_is_resolved_where_it_ran` (D1): cwd `D:\P`, `cd /d/a; cd b` → `D:\a\b` usata, e nessuna evidenza per `D:\P\b`.
  - `patch_actions_keep_their_meaning`:
    - Add `D:/n/new.rs` → `D:\n` scritta;
    - Update `src/a.rs` da cwd `D:\P` → `D:\P\src` scritta;
    - Delete `D:/old/x.rs` → `D:\old` solo usata;
    - Move da `D:/m/a.rs` a `D:/k/a.rs` → `D:\m` usata, `D:\k` scritta.
  - `verbatim_cwd_and_outcomes`:
    - cwd `\\?\C:\w` → cartella `C:\w` e `EvidenceRef.cwd == Some("C:\\w")`;
    - un `git clone … D:\c` con `error: Some(true)` dà `get("D:\\c").strength == None`, con un `failed` di esito `Failed`;
    - lo stesso clone senza risultato dà `Created` con esito `Unknown`;
    - con un risultato `Succeeded` dà `Created` con esito `Succeeded`.
  - `same_session_id_in_two_stores`: due chiamate con `session` uguale e `store` diverso danno due chiavi in `session_ledgers`; ogni riferimento conserva `harness`, `store` e `time`.
  - `coverage_counts_what_was_not_read`: una sessione con `mkdir $X/a`, un `python -c` con f-string, un ciclo su glob e un comando tagliato dà `shell_calls 4`, `unresolved >= 3`, `looped 1`, `script 1`, `truncated 1`.
  - `folder_of_table`: `(Write, D:\a\b.txt) → D:\a`, `(Write, D:\a\out) → D:\a\out`, `(Create, D:\a.b) → D:\a.b`.
- [ ] **Step 2: Run** `cargo test -j 1 -p xiox-project provenance::ledger`. Expected: FAIL.
- [ ] **Step 3: Implement** `ledger.rs`. `BTreeMap` con chiave in minuscolo, così l'ordine è stabile. `record` è una funzione per sorgente di evidenze: cwd, strutturata, patch, shell. Le chiama in quest'ordine.
- [ ] **Step 4: Run** il comando dello Step 2. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(xiox-project): registro delle evidenze di sessione`.

### Task 13: Sonda di parità col prototipo

**Files:**
- Create: `crates/xiox-project/examples/shell_probe.rs`
- Modify: `bench/path-extractor/real.py` (sottocomando `parity`), `bench/path-extractor/evidence.py` (funzione `ledger_rows` riusata da `parity`)

**Interfaces:**
- Consumes: `ToolCall`, `shell_command`, `extract`, `resolve`, `session_ledgers`, `Env`.
- Produces:
  - `cargo run -j 1 -p xiox-project --example shell_probe -- --calls <calls.jsonl> --out <dir> --home <dir>` scrive due file JSONL, senza testo dei comandi:
    - `calls.jsonl`: per ogni chiamata shell, `{"store","session","entry","call","candidates":[[op,src,dst]...],"flags":[...],"unresolved":n}`;
    - `ledger.jsonl`: per ogni cartella di ogni sessione, `{"store","session","folder","strength","failed":n,"sources":[...]}`.

    Su stderr stampa conteggi e tempo totale.
  - `python real.py parity --calls <calls.jsonl> --rust <dir>`: confronta campo per campo, con le chiavi `(store, session, entry, call)` e `(store, session, folder)`. I campi confrontati sono:
    - candidati con `src`;
    - flag `looped` e `script`;
    - `unresolved` del resolver;
    - forza e numero di `failed` per cartella.

    Le chiamate o cartelle toccate da una divergenza dichiarata sono classi contate a parte, non differenze:
    - D2: corpo heredoc con `\r` sul delimitatore;
    - D3: `truncated`;
    - D4: `exhausted`;
    - D5 e D6: `unresolved` degli script, confrontato solo dal lato Rust;
    - D7: fonti `patch`.

    Stampa, per campo, il numero di differenze e i primi 10 identificativi, senza il testo dei comandi.

- [ ] **Step 1: Write the failing check.** `python real.py parity --self-test` costruisce in `tempfile` un `calls.jsonl` sintetico di 4 righe (Bash, PowerShell, Write, `apply_patch`) e l'uscita Rust attesa. Controlla:
  - `0 differences` sull'uscita esatta;
  - una differenza per ogni campo su copie alterate apposta: un `src` cambiato, un flag tolto, `unresolved` +1, una forza abbassata, un `failed` in più;
  - la riga `apply_patch` contata nella classe D7.

  Expected adesso: errore, sottocomando assente.
- [ ] **Step 2: Implement** l'esempio e il sottocomando. Il `--self-test` non richiede la sonda Rust: l'uscita attesa è scritta a mano nel test.
- [ ] **Step 3: Run sulla cronologia reale, fuori dal repository.**
  - Su richiesta esplicita dell'utente, come vuole la specifica: `harvest.py` in una cartella temporanea, con `--opencode-exe` e una copia del database di OpenCode; poi la sonda, poi `parity`.
  - Expected: `0 differences` sui 94.862 comandi fuori dalle classi dichiarate. Ogni altra differenza va spiegata e corretta nel task che possiede il codice, con un test nuovo.
- [ ] **Step 4: Commit** — `test(xiox-project): sonda di parità dell'estrattore col prototipo`. Contiene solo l'esempio, `real.py` ed `evidence.py`: nessun dato reale.

---

## Self-review

1. **Copertura della specifica:**
   - «Operazioni riconosciute» → Task 6 e 8, script nel Task 7, patch nel Task 9, strumenti strutturati nel Task 12.
   - Formato e resolver → Task 1 e 2.
   - Copertura dei non letti → `Extraction.unresolved` (7, 8) e `Coverage` (12).
   - Provenienza di ogni evidenza (store, agente, sessione, evento, timestamp, cwd effettiva, operazione, esito, forza) → `EvidenceRef` (12) da `ToolCall` (10).
   - «Struttura del codice»: `Dialect` (3), struttura del comando (4), `Recognizer` (6 e 8), analizzatori (7 e 9), `Resolver` ed `EvidenceLedger` (2 e 12).
   - Fixture e test guidati dai dati → Task 1 e 8.
   - Lettura tramite Mukade, OpenCode compreso → Task 10.
   - Fuori ambito, dichiarato in testa: integrazione in `analyze`, `scratch`, `danger`.
2. **Revisione di Codex `gpt-6-astra`, 15 osservazioni, tutte assegnate:**

   | # | Osservazione | Dove |
   |---|---|---|
   | 1 | Tetti di espansione | `Budget`, D4, Task 8 |
   | 2 | Testo tagliato | `Completeness`, D3, Task 8, 10, 11 |
   | 3 | Confine Unicode | Task 7 |
   | 4 | OpenCode | Task 10 |
   | 5 | Provenienza ed esito | Task 10, 12 |
   | 6 | `cd` | `visits`, D1, Task 2, 12; prototipo corretto e rimisurato |
   | 7 | `bun` | Task 8 |
   | 8 | Classificatore | Task 11 |
   | 9 | Diagnostica degli script | `ScriptScan.unresolved`, `Coverage` |
   | 10 | Patch | Task 9 |
   | 11 | Scansione lineare | Task 7 |
   | 12 | Finestre in caratteri | Task 10 |
   | 13 | Prefissi e riassegnazione | Task 4, 8 |
   | 14 | Oracolo di parità | Task 13 |
   | 15 | CRLF | D2, Task 3, 8 |
3. **Passi:** ogni passo di codice nomina firma, file e valori del prototipo; i test nominano casi e risultati.
4. **Coerenza dei tipi:**
   - `RawOp::Path { … script }` (1) è usato da `scan_script` (7), diventa `Candidate.script` (2) e poi `Source::Shell { script }` (12).
   - `Flags` e `Completeness` (1) sono gli stessi in 6, 8, 9, 11.
   - `Extraction.unresolved` (1) è alimentato da `ScriptScan.unresolved` (7) e dal `Budget` (8), e finisce in `Coverage` (12).
   - `Resolved.visits` (2) diventa `Source::Cd` (12).
   - `ToolCall` e `Outcome` (10) sono usati da `shell_command` (11), `Ledger` (12) e dalla sonda (13).
5. **Review Focus:** i cinque punti hanno test nominati nei loro task (3, 7, 8, 10, 12).
6. **Proporzione:** il piano porta circa 1.200 righe di prototipo più patch, classificatore e preparazione di OpenCode. Non c'è codice di corpo, solo firme, tabelle di casi e i valori della specifica.
