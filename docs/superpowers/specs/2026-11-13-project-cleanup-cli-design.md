# Analisi mirata di un progetto e riga di comando (`xiox`)

Data: 2026-11-13. Stato: da approvare.

Primo di due progetti decisi lo stesso giorno. Il secondo, separato, porta l'interfaccia da egui a
Tauri con il nome Xiox e lo stile «Oni» (`docs/superpowers/specs/` avrà il suo documento). Questo
documento non tocca l'interfaccia.

## Perché

L'utente lavora con gli agenti (Claude Code, Codex, OpenCode, Pi) su progetti come mukade,
shinatama, daodan. Dopo qualche settimana ogni progetto lascia in giro worktree, cartelle di
prova, backup, benchmark e build, anche su dischi diversi, e il disco si riempie «andando fuori
controllo» (parole sue). Il 2026-11-13 C: aveva 3,8 GB liberi e D: 9 GB; per shinatama e mukade
c'erano 30 GB di `target`, un worktree da 6,7 GB, quattro cartelle sparse in `D:\` riconoscibili
solo dal nome, e un worktree di daodan dentro la cartella di lavoro di un agente
(`...\sidecar\runs\...\scratchpad\review`).

Oggi l'app analizza tutti i progetti insieme e solo dalla finestra. Mancano: puntare un progetto,
trovare ciò che gli appartiene fuori dalla sua cartella, e farlo da terminale (anche da uno script
o da un agente a fine lavoro).

## Cosa si costruisce

1. **Analisi mirata**: dato un progetto, l'elenco di tutto ciò che gli appartiene sul PC, con
   peso, livello e il motivo per cui è attribuito al progetto.
2. **Riga di comando** `xiox.exe`: `analyze` (sola lettura) e `clean` (cancella solo ciò che i
   parametri nominano).

Regola del prodotto, invariata: l'app propone, l'utente sceglie. Nessuna voce viene nascosta;
cambia solo il livello e la preselezione.

## Decisioni dell'utente

- Analisi e pulizia entrambe, «in base ai parametri».
- **Comandi, parametri e ogni testo stampato in inglese**: l'app è internazionale. Niente italiano.
- Il caso d'uso è il progetto singolo (mukade, shinatama, daodan, ethel).

## Decisioni mie (l'utente non le ha discusse: da controllare)

- **Due eseguibili, come mukade** (`mukade.exe` riga di comando, `mukade-ui.exe` finestra):
  `xiox.exe` è un programma da console, manifest `asInvoker`, nessuna richiesta UAC. La finestra
  resta `PulisciPC.exe` finché il secondo progetto non la rinomina (`xiox-ui.exe`). Motivo: la
  finestra ha `windows_subsystem = "windows"` e in release il manifest `requireAdministrator`;
  nello stesso exe un comando da terminale aprirebbe la richiesta UAC e non scriverebbe nella
  console.
- Senza percorso, `xiox analyze` usa la cartella corrente.
- `clean` senza parametri di categoria non cancella niente: stampa l'elenco come `analyze` ed esce.
- Cestino per impostazione; `--permanent` cancella davvero.
- `clean` chiede conferma prima di cancellare; `--yes` la salta. Se l'ingresso non è un terminale
  e manca `--yes`, non cancella ed esce con errore.
- Le voci `caution` non entrano mai con i parametri di categoria: solo nominandole con
  `--include <path>`.
- I branch vecchi vengono elencati ma non cancellati (non occupano spazio).
- **Ogni worktree collegato del progetto si può togliere, non solo quelli creati dagli agenti.**
  Oggi il motore toglie interi solo i worktree degli agenti (`is_agent_worktree`,
  `dc_engine::whole_worktree_repo`, `checked_plan`) e mostra gli altri «solo per informazione».
  Nell'analisi mirata l'utente nomina il progetto, e i worktree fatti a mano
  (`D:\Projects\mukade-chunked`) sono proprio ciò che vuole togliere. I controlli che oggi
  valgono per i worktree degli agenti (modifiche non salvate, commit non pubblicati) vanno
  calcolati per tutti. La finestra di oggi non cambia comportamento.
- **I worktree si tolgono sempre in modo definitivo** (`git worktree remove`, come nella finestra
  di oggi), anche senza `--permanent`: il riepilogo e la domanda di conferma lo dicono.
- **L'età del progetto non conta nell'analisi mirata.** Nella finestra un progetto usato di
  recente non viene preselezionato; qui l'utente ha nominato il progetto, quindi `--builds`
  prende le cartelle `recommended` anche se il progetto è stato usato oggi. Resta il blocco per
  una sessione viva (`InUse`).
- `xiox` vive in un crate nuovo del workspace (`crates/xiox-cli`, binario `xiox`) con il suo
  manifest `asInvoker`; non chiede mai l'elevazione. Se il terminale è già elevato usa l'MFT.
- Le stringhe in italiano del motore (`ProjectReport::reason`, `Artifact::note`, `label`) non
  vengono tradotte qui: la riga di comando costruisce i suoi testi in inglese dai campi
  strutturati (`status`, `blocked_by`, `level`, `class`, `kind`). La traduzione del motore è del
  secondo progetto.
- Impostazioni e cache restano dove sono oggi (`dc_app::settings_path()`, `cache_dir()`); il
  cambio di nome delle cartelle è del secondo progetto.

## Cosa appartiene a un progetto

Il progetto è la radice del repository git che contiene il percorso dato (o il percorso stesso se
non è un repository). Se il percorso è un worktree collegato, il progetto è il repository
principale.

| Categoria (parametro) | Come si trova | Livello |
|---|---|---|
| `builds` | cartelle del catalogo dentro il progetto e dentro ogni suo worktree (motore di oggi: `target`, `node_modules`, `dist`, `.venv`...) | quello che il motore dà oggi |
| `worktrees` | `git worktree list --porcelain` dal repository principale: ogni worktree collegato, ovunque sia, anche dentro cartelle di lavoro degli agenti | `recommended` se è pulito e il suo HEAD è raggiungibile da un branch locale di un altro worktree o da un branch remoto (nessun commit andrebbe perso); bloccato se ha modifiche non salvate (`DirtyGit`), commit che nessun altro riferimento contiene (`UnpushedCommits`) o una sessione viva. Una voce registrata la cui cartella non esiste più è solo informazione (`prunable`): sparisce con il `prune` che segue ogni rimozione |
| `copies` | altri repository git sul PC con la stessa identità: almeno un commit radice in comune (`git rev-list --max-parents=0 HEAD`) oppure stesso URL di `origin` dopo la normalizzazione (minuscole, senza `.git` finale, `git@host:a/b` e `https://host/a/b` uguali). Un fork ha la stessa identità: per questo il livello è sempre `caution` | sempre `caution`; indica se contiene commit o modifiche che il progetto non ha |
| `named` | cartelle fuori dal progetto il cui nome è il nome del progetto seguito da un separatore (`-`, `_`, `.`, spazio) e altro testo: `mukade-test-scratch`, `shinatama-tmp`, `daodan-pytest-tmp`. Non sono repository con la stessa identità (altrimenti sono `copies`) né worktree | sempre `caution`: «matched by name only» |
| `logs` | file (non cartelle) `*.log`, `*.tmp`, `*.bak`, `*.dmp` di almeno 1 MB dentro il progetto e i suoi worktree, che git ignora o non traccia (una sola chiamata `git ls-files --others` con e senza `--ignored --exclude-standard`), fuori dalle cartelle già contate in `builds` | `recommended` se ignorati da git, `caution` se solo non tracciati |
| `sessions` | sessioni archiviate degli agenti la cui cartella di lavoro è il progetto o un suo worktree (motore di oggi) | come oggi; mai preselezionate |

Se il percorso non è un repository git, o `git` non è installato, valgono solo `builds`, `named`
e `sessions`; le altre categorie sono vuote e l'uscita dice perché.

Fuori dalle categorie, solo informazione: i branch locali già fusi nel branch principale e quelli
il cui worktree non esiste più.

Limiti della ricerca per nome, per non attribuire a un progetto cartelle altrui:
- il nome del progetto deve avere almeno 4 caratteri, altrimenti la categoria `named` è vuota e
  l'uscita lo dice;
- mai dentro le cartelle di sistema (`dc_model::is_system_dir`), dentro `AppData`, dentro un altro
  repository git o dentro il progetto stesso;
- una cartella trovata per nome non viene mai cancellata da un parametro di categoria.

Dove si cerca (`copies` e `named`): tutti i dischi fissi. Con il terminale elevato si usa
l'indice MFT (`DirIndex::find_dirs_named`, veloce). Senza, si cammina: genitore del progetto,
radice di ogni disco fisso fino a 3 livelli, cartella dell'utente fino a 3 livelli. L'uscita dice
quale dei due modi è stato usato e che il secondo può non vedere cartelle più profonde.
`--no-drives` limita la ricerca al genitore del progetto.

## Riga di comando

```
xiox analyze [<project>] [--json] [--no-drives]
xiox clean   [<project>] [--builds] [--worktrees] [--logs] [--sessions]
             [--include <path>]... [--permanent] [--yes] [--dry-run] [--json] [--no-drives]
xiox --help | --version
```

- `--builds --worktrees --logs --sessions`: prendono le voci `recommended` di quella categoria
  (per `sessions`: tutte quelle non bloccate). `--copies` e `--named` non esistono: quelle voci
  si prendono una per una con `--include`.
- `--include <path>`: aggiunge una voce di qualunque categoria e livello (tranne le sessioni,
  che non hanno un percorso: si prendono solo tutte insieme con `--sessions`), purché sia
  nell'elenco dell'analisi. Un percorso che l'analisi non ha prodotto è un errore, non una
  cancellazione.
- Le voci bloccate (sessione viva, modifiche non salvate, commit non pubblicati) non si possono
  cancellare nemmeno con `--include`: l'uscita dice perché.
- `--dry-run`: fa tutto tranne cancellare, e stampa ciò che avrebbe cancellato.
- Prima di cancellare: riepilogo (voci, peso, Cestino o definitiva, avviso se il Cestino non ha
  spazio: `dc_app::recycle_shortfall`) e conferma `y/N`.
- Una sola conferma e un solo resoconto per cartelle, file, worktree e sessioni, anche se dentro
  passano da strade diverse (`CleanPlan` per cartelle e worktree, `dc_app::revalidate_archived` e
  `delete_archived` per le sessioni, la rimozione di file già usata da «Pulizia file» per i
  `logs`). L'analisi è fatta nello stesso processo subito prima: la regola dei 30 minuti
  (`MAX_SCAN_AGE_MS`) non serve.
- Dopo: il resoconto di oggi (`CleanReport`) in inglese; voci fallite con il motivo.
- `--json`: un solo oggetto JSON su stdout, niente altro testo; i messaggi vanno su stderr.
  Struttura stabile e con `"schema": 1`. Con `--json` non si chiede conferma: senza `--yes` non
  si cancella.
- Codici di uscita: 0 riuscito (anche `--dry-run` e `analyze`, qualunque cosa trovino); 1
  errore; 2 uso sbagliato; 3 pulizia parziale (almeno una voce fallita o saltata).

Uscita leggibile di `analyze` (forma indicativa, i testi li decide il builder):

```
Project  mukade   D:\Projects\mukade   (git, origin github.com/…/mukade)
Search   all fixed drives, MFT index

BUILDS                                              6.0 GB
  recommended   6.0 GB  D:\Projects\mukade\target            Rust build
WORKTREES                                           265 MB
  recommended   265 MB  D:\Projects\mukade-chunked           clean, detached at b574bc7
NAMED                                                21 MB
  caution        21 MB  D:\mukade-test-scratch               matched by name only
…
Total 6.3 GB · recommended 6.2 GB
Clean with: xiox clean D:\Projects\mukade --builds --worktrees
```

## Vincoli

- Sicurezza della cancellazione uguale a oggi: si passa da `CleanPlan` e dai controlli esistenti
  (`checked_plan`, nessun link seguito, riconvalida subito prima di cancellare). I worktree si
  tolgono con `PlanAction::GitWorktreeRemove` (mai `--force`). Le cartelle `copies` e `named` e i
  file `logs` sono tipi di voce nuovi (oggi `RemoveDir` e `guard.rs` accettano solo cartelle del
  catalogo): per i file valgono Cestino e definitiva come per le cartelle. Per `copies` e `named` il piano le accetta solo se il percorso viene dall'analisi appena fatta,
  esiste, non è un link e non contiene né è contenuto nel progetto o in una cartella di sistema.
- `analyze` non scrive niente, tranne la cache dell'elenco delle sessioni che il motore scrive
  già oggi.
- Mai provare o tarare sui progetti veri dell'utente: test su alberi finti in `%TEMP%`, con
  repository git creati dai test.
- Macchina: `cargo` sempre con `-j 1`, mai due build insieme, dischi quasi pieni.
- Il motore resta nei crate `dc-*`; l'analisi mirata è una funzione di libreria (in `dc-app` o in
  un crate nuovo) che la finestra Tauri potrà chiamare tale e quale. Il binario `xiox` contiene
  solo argomenti, stampa e conferma. La funzione ha un modo leggero che non misura i pesi
  (serve all'osservatore, `2026-11-13-project-observer-mcp-design.md`) e ogni voce ha una chiave
  stabile (`dc_model::path_key` del percorso).
- Nessuna dipendenza nuova pesante: per gli argomenti va bene un parser piccolo o scritto a mano.

## Non si fa

- Niente modifica all'interfaccia egui.
- Niente analisi di tutti i progetti da riga di comando (`xiox analyze --all`): solo un progetto.
- Niente cancellazione di branch, niente `git gc`, niente `git worktree remove --force`.
- Niente pulizia di file, registro, servizi dalla riga di comando.
- Niente ricerca di copie per contenuto dei file (solo identità git e nome).

## Sarebbe un fallimento se

- una cartella che non è del progetto venisse cancellata perché ha un nome simile;
- un worktree o una copia con lavoro non salvato venisse cancellato;
- `clean` cancellasse qualcosa senza che un parametro lo nomini;
- comparisse testo in italiano nell'uscita.

## Lasciato al builder

- Dove vive il codice (funzione in `dc-app` o crate `dc-project`), nomi dei tipi, forma esatta
  del JSON, testi inglesi.
- Come riusare `run_scan` limitandolo a un progetto e ai suoi worktree.
- Come ottenere l'elenco dei worktree e l'identità del repository (`dc-git` ha già
  `main_worktree` e `worktree_info`, privati).
- Soglie e dettagli della camminata senza MFT.
