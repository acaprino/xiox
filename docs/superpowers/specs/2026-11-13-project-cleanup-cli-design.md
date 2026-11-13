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
  è `xiox-ui.exe` (con il cambio di nome in Xiox ha sostituito `PulisciPC.exe`). Motivo: la
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
  `xiox_engine::whole_worktree_repo`, `checked_plan`) e mostra gli altri «solo per informazione».
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
- Impostazioni e cache restano dove sono oggi (`xiox_app::settings_path()`, `cache_dir()`); il
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
- mai dentro le cartelle di sistema (`xiox_model::is_system_dir`), dentro `AppData`, dentro un altro
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
  spazio: `xiox_app::recycle_shortfall`) e conferma `y/N`.
- Una sola conferma e un solo resoconto per cartelle, file, worktree e sessioni, anche se dentro
  passano da strade diverse (`CleanPlan` per cartelle e worktree, `xiox_app::revalidate_archived` e
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
- Il motore resta nei crate `dc-*`; l'analisi mirata è una funzione di libreria (in `xiox-app` o in
  un crate nuovo) che la finestra Tauri potrà chiamare tale e quale. Il binario `xiox` contiene
  solo argomenti, stampa e conferma. La funzione ha un modo leggero che non misura i pesi
  (serve all'osservatore, `2026-11-13-project-observer-mcp-design.md`) e ogni voce ha una chiave
  stabile (`xiox_model::path_key` del percorso).
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

- Dove vive il codice (funzione in `xiox-app` o crate `xiox-project`), nomi dei tipi, forma esatta
  del JSON, testi inglesi.
- Come riusare `run_scan` limitandolo a un progetto e ai suoi worktree.
- Come ottenere l'elenco dei worktree e l'identità del repository (`xiox-git` ha già
  `main_worktree` e `worktree_info`, privati).
- Soglie e dettagli della camminata senza MFT.

## Esito

Costruito il 2026-11-13. Da `e95e931` fino a `2b40f09` (guida e questa sezione comprese; `git diff --shortstat e95e931..2b40f09`): 34 file modificati, 7.724 righe aggiunte, 13 tolte.

- `xiox-git` (`project.rs`): `repo_main_root`, `list_worktrees`, `root_commits`, `origin_url`,
  `normalize_remote`, `head_kept_elsewhere`, `untracked_files`, `stale_branches`.
- `xiox-project` (crate nuovo): `resolve_project`, builds/worktrees/sessioni (`owned.rs`), ricerca
  di copie e cartelle per nome (`search.rs`: indice MFT da terminale elevato, altrimenti
  camminata), log (`logs.rs`), selezione, riconvalida e rimozione (`clean.rs`), controllo del
  Cestino. Piccole aggiunte in `xiox-cleaner` (`guard` pubblico), `xiox-app`, `xiox-model`
  (`ArtifactKind::ProjectFolder`).
- `xiox-cli` (crate nuovo, `xiox.exe`): parser degli argomenti scritto a mano, uscita leggibile,
  JSON schema 1, manifest `asInvoker` (non chiede mai l'elevazione).
- Guida d'uso in inglese: `docs/xiox-cli.md`, con le uscite copiate da esecuzioni vere su un
  progetto finto.

Test: `cargo test -j 1 --workspace --exclude mukade` = 941 passati, 0 falliti, 20 ignorati
(`xiox-git` 34, `xiox-project` 74, `xiox-cli` 34 unitari + 18 sul binario). Eseguibile release
`target/release/xiox.exe`: 2.928.640 byte (2,8 MB).

Scostamenti dalla spec, decisi in revisione:

- Una copia con commit o modifiche che il progetto non ha è **bloccata** (`unsaved-work`) e non
  si cancella nemmeno con `--include`: è il caso di fallimento della spec («una copia con lavoro
  non salvato venisse cancellata»), che la tabella non copriva.
- Le cartelle direttamente sotto la radice di un disco sono rifiutate dal guard di `xiox_cleaner`
  per ogni tipo di voce: `analyze` le segna («cannot be removed: top-level folder»), il riepilogo
  di `clean` le avvisa e `clean` le salta (uscita 3).
- Flussi: quando `clean` sta per chiedere la conferma, riepilogo, domanda e `Cancelled.` vanno su
  stderr, così un stdout rediretto non nasconde ciò che sta per sparire; altrimenti riepilogo e
  resoconto vanno su stdout (con `--json` tutto il testo va su stderr).
- Interruttori di prova `XIOX_SEARCH_ROOTS` e `XIOX_NO_SESSIONS=1`: letti solo dalle build debug
  (`cfg!(debug_assertions)`), servono ai test sul binario per usare alberi finti e non leggere
  mai le sessioni vere. Non sono una funzione per l'utente e la guida non li nomina.
- Dopo la scelta, `clean` riesegue l'analisi leggera e rimuove la voce *fresca*; una voce
  diventata più sospetta viene saltata. Risposta «no» alla domanda: uscita 0. Cestino troppo
  piccolo: uscita 1, anche con `--dry-run` (la spec non lo diceva).

Scostamento dal piano (non dalla spec): un nome di progetto più corto di 4 caratteri svuota solo
`named` e le copie si trovano comunque per identità git (spec, righe 99-100); il piano diceva
«nessuna ricerca».

Limiti noti e rinvii (dal registro `.superpowers/sdd/2026-11-13-project-cleanup-cli/progress.md`):

- `has_own_work` sconosciuto (git non risponde) lascia la copia non bloccata: `--include` la
  cancella. Se la riconvalida non riesce a interrogare git su una copia prima «senza lavoro
  proprio», procede.
- Con l'indice MFT (terminale elevato) le copie il cui nome non ricorda il progetto si trovano
  solo accanto al progetto o su dischi non indicizzati; il percorso elevato è provato solo con un
  indice finto, mai con il vero MFT.
- `stale_branches` elenca anche i branch estratti in un worktree vivo (nell'esempio `feature` e
  `wip`); sono solo informazione, nessun branch viene mai cancellato.
- I log: `git ls-files --others --ignored` percorre anche `target` e `node_modules` (lento nei
  progetti grandi, da misurare su un progetto vero); se git fallisce o scade, i log ignorati
  spariscono senza una nota.
- Il marcatore «top-level» replica solo la regola di profondità del guard (non le cartelle di
  sistema, `\Users\<x>`, i nomi vietati) ed è solo nel testo, non nel JSON.
- Non provati da test automatici: il Cestino vero e la domanda `y/N` su un terminale vero (solo
  stdin non interattivo e finti). Se la scrittura su stdout fallisce dopo una rimozione, il
  resoconto va su stderr; una pipe chiusa da sola non è un errore.
- Minori di stile: due punti da `cargo fmt` (`xiox-project/src/tests/clean.rs`, un test di
  `render.rs`).

### Dopo la review finale

La review dell'intero ramo (`e95e931..88784c7`) ha trovato un difetto critico e otto importanti;
una sola ondata di correzioni li ha chiusi, con i minori che toccavano la sicurezza. Cosa è cambiato:

- **Una cartella con dentro una voce bloccata non si cancella.** `--include` di una cartella
  `named` che conteneva una copia con commit propri (listata e bloccata) cancellava anche la
  copia: `select` guardava solo i blocchi della voce nominata. Ora `select` rifiuta (uscita 2)
  una voce che ne contiene una bloccata, e la riconvalida (`check_contents`) salta la cartella
  se ne contiene una bloccata o un repository git che l'analisi fresca non elenca come
  rimovibile (la ricerca è limitata: tre livelli, uno con `--no-drives`). Una cartella `named`
  con dentro una copia pulita e listata si toglie ancora con tutto il suo contenuto. Ogni cartella
  di catalogo non può contenere il progetto, un worktree, una copia o una cartella `named`.
- **Copie e cartelle `named` hanno il blocco `live-session`** (una sessione viva con cartella di
  lavoro dentro la voce), come i worktree.
- **Il controllo del lavoro proprio** considera HEAD, branch locali, tag e ogni voce di stash, e
  usa la raggiungibilità (`git rev-list -n1 --missing=print <punte> --not --all` nel progetto),
  non l'esistenza dell'oggetto: un commit che il progetto ha solo come oggetto penzolante (branch
  cancellato) è lavoro della copia. Tutte le chiamate git passano da `xiox_git` (timeout, ambiente
  pulito); la copia privata del comando in `search.rs` è sparita.
- **Repository illeggibili.** Una cartella `named` che è o contiene un repository che git non sa
  leggere è bloccata (`unsaved-work-unknown`, nuovo); lo stesso una copia il cui lavoro proprio
  non si può verificare (prima restava rimovibile). Un repository indipendente con il nome del
  progetto è bloccato se ha modifiche o commit che nessun remoto ha (`unsaved-work`). In
  riconvalida, per una copia serve la risposta positiva «nessun lavoro proprio»: nessuna risposta
  è un rifiuto.
- **Cartelle che contengono molti progetti.** Le bandiere di categoria (`--builds`, `--worktrees`,
  `--logs`, `--sessions`) sono rifiutate con uscita 2 («refusing to clean everything under
  <root>: run it on one project») quando la radice è la radice di un disco, la cartella del
  profilo utente o una sopra di essa (anche se è un repository: un repo di dotfile in `~`), oppure
  non è un repository e contiene più di un repository git o più di una radice di progetto del
  motore. Il conteggio è limitato: tre livelli, 2.000 cartelle, nessun link seguito, nessun
  repository attraversato, si ferma a due. `analyze` funziona e stampa una nota; `--include` di un
  percorso esplicito funziona.
- **Sessioni illeggibili.** Un elenco di sessioni vivo che non restituisce nulla e si lamenta è una
  nota (`sessions-not-read`) e `clean` salta worktree, copie e cartelle `named` («the sessions of
  this PC could not be read, so a live agent session cannot be ruled out»); i log che git non sa
  elencare sono una nota (`logs-not-checked`).
- **Testo inglese.** L'errore del Cestino (`Os { code, description }` del crate `trash`, con la
  descrizione nella lingua di Windows) e gli errori di scrittura su stdout si riducono al codice
  OS.
- Minori: le credenziali di `remote.origin.url` non arrivano più al JSON; la domanda `y/N` dice che
  i worktree vanno via per sempre; una sola dicitura per i blocchi (`Block::reason`); la guida non
  dice più che il `recommended` «non può perdere nulla» (git cancella i file ignorati di un
  worktree rimosso, e un `*.bak` ignorato è `recommended`); i test non ereditano più
  `GIT_DIR`/`GIT_WORK_TREE` (fixture di `xiox-project`, `xiox-git`, `xiox`).

Scostamenti registrati: (1) `checked_plan` (righe 163-164) non c'è: il codice la sostituisce con
una lista di ammissione costruita dall'analisi fresca (`Run::remove` rilegge il progetto e
rimuove solo la voce fresca); con essa si perde il rifiuto per lo stato `Blocked` del motore (un
build in un worktree di agente sporco o non pushato non è bloccato). È noto e vale solo per i
`builds`. (2) Il rifiuto delle bandiere sui contenitori e lo stato «non so» come blocco vanno
oltre la spec e sono scelte di sicurezza.

Rinviati (elenco «può aspettare» della review): `stale_branches` sui worktree vivi e nomi con
caratteri strani, marcatore `top-level` nel JSON e solo la regola di profondità, BrokenPipe dopo
una rimozione, costo di `ls-files` e `inside_other_repo`, righe di test e di testo minori, un
clone vendorizzato non elencato. I due punti di `cargo fmt` sono corretti; il resto del
workspace non è formattato con `cargo fmt` e non è stato toccato.

Sostituiti da questa sezione: il limite «`has_own_work` sconosciuto lascia la copia non
bloccata» e «i log ignorati spariscono senza una nota».

Test dopo le correzioni: `cargo test -j 1 --workspace --exclude mukade` = 991 passati, 0 falliti, 20 ignorati (prima 941; `xiox-git` 42, `xiox-project` 107, `xiox-cli` 39 unitari + 22 sul binario).
