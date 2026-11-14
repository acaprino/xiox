> Internal working notes, written in Italian.

> Copia di `D:\Projects\KKK\devclean\docs\DESIGN.md` (progetto unito in Xiox (prima PulisciPC) come voce Sviluppatori), presa il 2026-11-12 prima di eliminare la cartella. I percorsi qui sotto (`src-tauri`, `vendor/mukade`, WizTree) sono quelli di devclean. Dal 2026-11-13 i crate `dc-*` si chiamano `xiox-*` (`dc-mukade` -> `xiox-mukade`); il resto del testo è quello di allora.

# devclean: design (v0, 2026-11-08)

Cleaner in stile BleachBit per chi sviluppa con agenti AI (Claude Code, Codex, OpenCode, Pi) e compila Rust. Libera disco capendo **cosa è ancora in uso** leggendo le sessioni degli agenti, non solo guardando i nomi delle cartelle.

Stato: la richiesta dell'utente ("crea app tauri rust con react") ha chiuso le domande. Questo documento fissa quanto detto in conversazione; le voci marcate **[call mia]** sono scelte fatte da me, non concordate.

## Perché

I dischi si riempiono di `target/` Rust, `node_modules`, cache di compilazione e worktree creati dagli agenti AI. Cancellare a mano fa paura: non si sa quale progetto è ancora vivo. I cleaner esistenti sono statici (kondo) oppure puliscono solo i dati di un agente (`~/.claude`), vedi `PRIOR_ART.md`.

## Obiettivi

- Elenco **per progetto**, ordinato per GB recuperabili (schermata B scelta dall'utente).
- Per ogni progetto: ultimo uso (dalle sessioni), peso per tipo (`target/`, `node_modules`, worktree), stato in chiaro: inattivo da N giorni / in uso / bloccato da git.
- Cancella davvero, di default nel Cestino. Preselezionati solo i progetti sicuri.
- Legge sessioni di Claude Code, Codex, OpenCode e Pi.
- I progetti vengono dalle sessioni degli agenti (decisione dell'utente, 2026-11-08); le cartelle da analizzare per intero sono un'opzione spenta di default, e solo per quelle si usa WizTree se presente.

## Non obiettivi

- Non è un cleaner di sistema generico (registro, browser, cache di Windows).
- Non termina mai processi.
- Non tocca credenziali, impostazioni, skill, plugin, memorie degli agenti.

## Anti-obiettivi (sarebbe un fallimento anche se "funziona")

- Cancellare un progetto in uso o con modifiche git non committate.
- Segnare come orfano un progetto su disco scollegato o non raggiungibile.
- Riportare GB liberati che non sono stati liberati davvero.

## Stack

- Tauri 2 + Rust + React 19 + TypeScript + Vite + pnpm (scelta dell'utente; TypeScript **[call mia]**).
- Nome provvisorio `devclean` **[call mia]**. Identificatore `com.alfio.devclean`.
- Windows 11 per primo **[call mia]**.

## Fonti di sessione (verificate sulla macchina)

| Agente | Fonte | Campi |
|---|---|---|
| Claude Code | `~/.claude/history.jsonl`, `~/.claude/projects/<codificato>/*.jsonl`, `~/.claude/sessions/<pid>.json` | `project` + `timestamp`, `cwd`, pid vivo |
| Codex | `~/.codex/state_5.sqlite` tabella `threads`, `~/.codex/worktrees/` | `cwd`, `updated_at_ms`, `archived`, `git_branch` |
| OpenCode | `~/.local/share/opencode/opencode.db` tabelle `session_v2`, `worktree`, `project` | `directory`, `time_updated`, `time_archived` |
| Pi | `~/.pi/agent/sessions/**/*.jsonl` | `cwd`; nessuna data registrata: fa fede la mtime del file |

Il nome cartella di `~/.claude/projects` è una codifica lossy (`C--Users-alfio-Desktop`): il progetto si ricava SEMPRE dal campo `cwd`.

Tutte e quattro le fonti sono lette dalla libreria mukade inclusa (`vendor/mukade`, solo elenchi, sola lettura): per Claude Code anche `history.jsonl` (sessioni il cui transcript è stato cancellato) e `sessions/<pid>.json` (processo verificato vivo = `live`); per Codex anche le righe di `state_<n>.sqlite` senza rollout; per OpenCode tutti gli `opencode*.db` accanto a quello selezionato. I worktree gestiti dagli agenti (`~/.codex/worktrees`, tabelle `worktree` e `project.sandboxes` di OpenCode) li trova `crates/dc-mukade`.

## Regole di sicurezza

1. Un solo flusso piano -> anteprima -> esecuzione: l'anteprima è identica all'esecuzione.
2. Rimozione **definitiva** di default (scelta dell'utente: il Cestino non libera spazio); il Cestino resta selezionabile. L'anteprima mostra sempre cosa verrà rimosso e la rimozione si può simulare.
3. Blocca: git sporco, branch con commit non pushati, sessione con pid vivo, directory tracciata da git.
4. Distingue "cancellato" da "non raggiungibile".
5. Verifica l'identità dell'artefatto prima di toccarlo (`CACHEDIR.TAG`/`.rustc_info.json` per `target`, `.modules.yaml` o `.package-lock.json` per `node_modules`, `pyvenv.cfg` per `.venv`).
6. Layout degli agenti letto da una tabella dati; voci sconosciute = "non toccare".
7. Report truthful: i byte liberati si misurano dopo, non si assumono.

## Decisioni dell'utente (2026-11-08)

- Metodo di rimozione predefinito: **Definitivo** (il Cestino non libera spazio finché non lo svuoti).
- Privilegi: l'app **release** chiede sempre l'amministratore all'avvio (`requireAdministrator` in `src-tauri/build.rs`), così il WizTree incluso può leggere la MFT. Le build di debug restano `asInvoker` per far partire `pnpm tauri dev` da una shell normale (`DEVCLEAN_ADMIN=1` forza il manifest elevato).
- WizTree incluso e usato di default; mukade incluso (vendored) e usato per le sessioni di tutti e quattro gli agenti. I lettori dedicati `dc-claude` e `dc-codex-opencode` sono stati rimossi il 2026-11-08, quando mukade (commit 97d827f) ha coperto history e sessioni live di Claude Code, `state_<n>.sqlite` di Codex e tutti i database OpenCode (prima elencava 0 progetti Codex contro 60 e Claude Code in 4.8 s contro 0.3 s).

## Quali sono i progetti (2026-11-08)

Decisione dell'utente: mukade serve anche a sapere **quali** sono i progetti, non solo quando sono stati usati. Ogni cartella di lavoro delle sessioni diventa una radice di scansione (`crates/dc-scanner/src/sessions.rs`):

1. cartella inesistente: saltata (non c'è niente da pulire);
2. cartella cloud (Google Drive, OneDrive, Dropbox, iCloud, placeholder di Windows): saltata, percorrerla può far scaricare file;
3. home, Desktop, Documents, Downloads, AppData, radici dei dischi, cartelle di sistema: saltate, e la ricerca non risale mai fin lì;
4. altrimenti: il repository git che la contiene, se no la cartella più vicina con un marcatore (`Cargo.toml`, `package.json`, ...), se no la cartella stessa se le sue figlie dirette contengono da 1 a 8 progetti (workspace `backend` + `frontend`).

Le cartelle in Impostazioni restano come "cartelle aggiuntive", percorse per intero solo se l'utente lo attiva (per i progetti mai aperti con un agente). WizTree serve solo a loro. Per l'abbinamento sessione -> progetto il motore usa come "contenitori" la home e quelle cartelle (`EngineConfig::container_roots`), non le radici trovate dalle sessioni.

Misura sulla macchina dello sviluppatore (2026-11-08): 30 radici da 89 cartelle di sessione (28 inesistenti, 22 non progetti, 6 cloud, 3 troppo generiche), 111 progetti tutti con sessioni, 60-64 s; con `D:\Projects` percorso per intero 121 progetti, 10 senza sessioni (circa 1 GB), 65-71 s. Il tempo è dominato dalla misura degli artefatti, non dalla ricerca dei progetti.

## Regola "inattivo" **[call mia]**

Candidato = nessuna sessione da almeno 14 giorni, git pulito, nessun pid vivo. Soglia configurabile.

## Da decidere con l'utente

- Soglia giorni di default.
- Se mostrare anche cache globali (`~/.cargo/registry`, `~/.npm`, `~/.gradle`) o solo progetti.
- Licenza del tool (le fonti MIT permettono qualsiasi scelta, con `THIRD_PARTY_NOTICES`).

## Prossimo passo

Fetta verticale 1: comando Rust che trova progetti con `Cargo.toml` + `target/` sotto una radice, ne calcola la dimensione, e li mostra in una lista React ordinata per GB.
