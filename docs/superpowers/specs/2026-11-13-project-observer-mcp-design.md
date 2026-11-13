# Osservatore di un progetto (`xiox observe`)

Data: 2026-11-13. Stato: da approvare.

Dipende da `2026-11-13-project-cleanup-cli-design.md` (analisi mirata di un progetto e binario
`xiox`): va costruito dopo. Non tocca l'interfaccia.

## Perché

Gli agenti (Claude Code, Codex, OpenCode, Pi), lavorando su un progetto, creano worktree, copie,
backup, cartelle di prova e build, e nessuno se ne accorge finché il disco è pieno. L'utente
vuole che qualcosa «stia a osservare» un progetto mentre ci si lavora e segnali «il proliferare
di file, worktree, copie, backup». L'analisi mirata dice cosa c'è adesso; l'osservatore dice cosa
è comparso da quando è partito, e lo dice a chi l'ha avviato.

## Cosa si costruisce

`xiox observe [<project>] [parametri]`: un processo che qualcuno avvia per un progetto. Il
processo:

1. alla partenza fa la fotografia del progetto (analisi mirata);
2. resta acceso e ricontrolla a intervalli;
3. dice a chi l'ha avviato cosa è comparso dalla fotografia in poi.

## Decisioni dell'utente

- **Non parte da solo.** Lo avvia chi ne ha bisogno, per il proprio progetto: un agente, un
  harness, shinatama, oppure la finestra di Xiox. Non scopre i progetti da solo e non osserva le
  sessioni degli altri.
- **Ogni osservatore è un'istanza a sé del processo, con i suoi parametri.** Nessun servizio
  unico per tutto il PC.
- **L'avviso va a chi lo chiama.** Come chi lo chiama lo mostra o lo passa a un agente è un
  problema suo (per esempio di shinatama), non di Xiox.
- **Avvisa soltanto**: non cancella mai niente.
- **Due processi separati: osservatore e pulizia.** Per pulire si lancia a parte
  `xiox clean <project> …`; il resoconto dell'osservatore porta già pronto il comando.
- Deve poter funzionare come **servizio MCP**, così un agente lo interroga direttamente.
- Tutto in inglese (parametri, nomi dei campi, testi), come il resto di Xiox.

## Decisioni mie (da controllare)

- **Due modi di parlare con chi lo chiama, scelti da un parametro:**
  - predefinito, per un programma che lo avvia come processo figlio (shinatama, la finestra di
    Xiox, uno script, un agente che lancia un processo in sottofondo): **una riga JSON per
    evento su stdout**;
  - `--mcp`, per un harness che lo avvia come suo servizio MCP: **MCP su stdio**, con uno
    strumento in sola lettura che l'agente interroga.
- **Si ferma quando chi lo chiama chiude stdin o lo termina.** Nessun demone, nessuna porta di
  rete, nessun file di stato condiviso fra istanze.
- **Il progetto** è l'argomento, o la cartella corrente se manca; vale la regola dell'analisi
  mirata (una sottocartella o un worktree contano per il repository principale).
- **Più osservatori sullo stesso progetto** sono indipendenti, ciascuno con la sua fotografia.
  L'osservatore dice «comparso da quando sono partito», non «creato da questa sessione».
- **Parametri** (con i valori predefiniti): `--interval 600` secondi fra un controllo e l'altro
  (minimo 60); `--grow-threshold 2GB` e `--worktree-threshold 3` per l'evento di attenzione;
  `--no-drives` come in `analyze`; `--mcp`.
- **Non elevato**: se il terminale non è elevato cerca copie e cartelle per nome con la camminata
  limitata dell'analisi mirata (non con l'MFT). Non chiede mai l'elevazione.
- L'integrazione dentro shinatama, mukade o la futura finestra di Xiox non fa parte di questo
  lavoro: qui si costruisce solo il processo e il suo contratto.

## Contratto con chi lo chiama

### Modo predefinito: righe JSON su stdout

Una riga = un oggetto JSON con `"schema": 1`, `"event"` e `"time"`. Niente altro su stdout; i
messaggi per le persone vanno su stderr.

| Evento | Quando | Contenuto |
|---|---|---|
| `started` | finita la fotografia iniziale | progetto, parametri, totali per categoria |
| `changed` | un controllo trova voci nuove o sparite | le voci (categoria, percorso, peso, livello, motivo, `first_seen`), crescita in byte dalla fotografia |
| `attention` | una soglia viene superata (una volta per soglia, e di nuovo solo dopo essere rientrati sotto) | quale soglia, valori, il comando `xiox clean …` che toglierebbe le voci `recommended` nuove |
| `report` | chi lo chiama scrive la riga `report` su stdin | l'elenco completo, come sotto |
| `error` | un controllo fallisce | motivo; il processo continua |

Su stdin accetta le righe `report` e `stop`.

### Modo `--mcp`

Uno strumento in sola lettura:

| Strumento | Ingresso | Uscita |
|---|---|---|
| `project_leftovers` | niente (il progetto è quello con cui il processo è partito) | l'elenco dell'analisi mirata (categorie `builds`, `worktrees`, `copies`, `named`, `logs`, `sessions`, con percorso, peso, livello, motivo), più per ogni voce `new: true/false`, `first_seen`, `measured_at`; totali; crescita in byte dalla fotografia; `attention`; `stale` se la risposta viene da un controllo precedente; il comando `xiox clean …` per le voci `recommended` nuove |

MCP risponde solo a chi chiede: le istruzioni del servizio (campo `instructions`) e la
descrizione dello strumento dicono all'agente di interrogarlo prima di dichiarare finito un
lavoro e dopo aver creato worktree o copie.

L'elenco di `report` e di `project_leftovers` ha la stessa forma del `--json` di `xiox analyze`
(stesso `schema`), con i campi in più.

## Come osserva

- Alla partenza: analisi mirata completa (con i pesi) = fotografia iniziale. Ciò che esiste già
  in quel momento non è mai «nuovo».
- A ogni intervallo: analisi leggera (`git worktree list`, cartelle per nome, copie, senza
  pesi). I pesi si misurano per le voci nuove e, per la crescita, sulle cartelle `builds` già
  note.
- A ogni richiesta (`report` o `project_leftovers`): analisi leggera e nuova misura dei pesi,
  poi la risposta. Se un controllo è in corso aspetta fino a 20 secondi; oltre, risponde con
  l'ultimo risultato e `stale: true`.
- L'analisi leggera (senza pesi) e la chiave stabile di ogni voce (`xiox_model::path_key` del
  percorso) fanno parte della funzione di libreria dell'analisi mirata.

## Vincoli

- Deve pesare poco: la macchina ha poca memoria libera e dischi quasi pieni, e possono esserci
  più osservatori insieme. Nessuna camminata completa dei dischi a intervalli; mai più di
  un'analisi alla volta per processo.
- Sola lettura: l'osservatore non contiene una strada verso la pulizia. Non scrive niente su
  disco, tranne la cache dell'elenco delle sessioni che il motore scrive già oggi.
- Su stdout passano solo le righe JSON o il protocollo MCP.
- `cargo` sempre con `-j 1`, mai due build insieme. Le dipendenze per MCP entrano solo nel
  binario `xiox`, non nei crate del motore. Mukade usa `rmcp` 2.2.0: va bene la stessa versione,
  oppure il protocollo scritto a mano se resta piccolo (uno strumento solo).
- Test su alberi finti in `%TEMP%`; mai sui progetti veri dell'utente.

## Non si fa

- Nessuna pulizia, nessuno strumento MCP che cancella.
- Nessuna scoperta automatica dei progetti, nessun servizio unico, nessuna porta di rete,
  nessun avvio automatico con Windows.
- Nessuna notifica di Windows, nessuna icona nell'area di notifica.
- Nessuna attribuzione a una sessione o a un harness: l'osservatore non legge le sessioni per
  decidere chi ha creato cosa.
- Nessuna osservazione in tempo reale dei file (`ReadDirectoryChangesW`, giornale USN): solo
  controlli a intervalli e a richiesta.
- Nessuna modifica a shinatama, a mukade o ai file di configurazione degli agenti.

## Sarebbe un fallimento se

- l'osservatore cancellasse o spostasse qualcosa;
- rallentasse la macchina mentre gli agenti compilano;
- segnalasse come «nuovo» ciò che c'era già alla partenza;
- sporcasse stdout e rompesse il collegamento con chi lo chiama;
- restasse acceso dopo che chi l'ha avviato è morto.

## Lasciato al builder

- Nomi dei tipi, forma esatta dei campi, testi inglesi di `instructions` e della descrizione
  dello strumento.
- Libreria MCP o protocollo scritto a mano.
- Come accorgersi che chi l'ha avviato è morto quando stdin non viene chiuso.
- Un esempio di configurazione MCP nella guida di `xiox observe --help`.

## Esito

Data: 2026-11-14. Costruito secondo il piano `docs/superpowers/plans/2026-11-14-xiox-observe.md`.
Guida per chi lo usa: `docs/xiox-observe.md` (in inglese).

### Cosa è stato costruito

- Il crate `xiox-observer`: fotografia iniziale, differenze fra un controllo e l'altro, soglie con
  riarmo, resoconto, misura dei pesi con limiti (200.000 voci e 10 secondi per cartella, i collegamenti
  non si seguono, `bytes: null` per una voce mai misurata oltre il limite). Funzioni pure: non toccano disco, orologio né git.
  `too_shallow` e `is_recommended` sono passate in `xiox-project`, in modo che l'elenco di `analyze`
  e il comando dell'osservatore usino la stessa regola.
- Il comando `xiox observe [<project>]` in `xiox-cli`: parametri `--interval` (predefinito 600,
  da 60 a 2.592.000 secondi), `--grow-threshold` (predefinito 2GB, unità binarie), `--worktree-threshold`
  (predefinito 3), `--no-drives`, `--mcp`. Modo predefinito: righe JSON `started`, `changed`,
  `attention`, `report`, `error` su stdout; su stdin `report` e `stop`; fine con `stop`, con la chiusura
  di stdin o con la pipe di stdout chiusa. Su stdout nient'altro; U+2028 e U+2029 sono scritti come
  escape; le righe di stdin sono limitate a 64 KiB.
- Il modo `--mcp`: server MCP su stdio scritto a mano (`initialize`, `ping`, `tools/list`,
  `tools/call`; versioni di protocollo 2025-06-18, 2025-03-26, 2024-11-05), con uno strumento solo in
  sola lettura, `project_leftovers`, e il campo `instructions`. Nessun timer: un controllo a ogni
  chiamata. Nessuna dipendenza nuova oltre a `chrono` (già nel workspace) e `serde_json`: né `rmcp` né
  un runtime asincrono.
- La documentazione: `docs/xiox-observe.md`, il testo di `xiox --help` (con un esempio di
  configurazione MCP), il README (sezione «Observe a project» e la voce della Roadmap spuntata), un
  rimando in `docs/xiox-cli.md`. Gli esempi della guida vengono da esecuzioni vere di una build di debug
  su un albero finto in `%TEMP%`.

### Le due deviazioni del piano rispetto a questo documento

1. **Ciclo a un solo thread, niente attesa di 20 secondi.** Il documento prevede che una richiesta
   (`report` o `project_leftovers`) durante un controllo aspetti fino a 20 secondi e poi risponda con
   l'ultimo risultato e `stale: true`. Il piano non ha una regola a tempo: la richiesta viene servita
   quando il controllo in corso finisce (un controllo alla volta, un solo thread che li esegue). Se il
   controllo nuovo fallisce, si risponde con l'ultimo risultato buono, `stale: true`, e in modo
   predefinito esce un evento `error`.
2. **Il comando `clean` usa `--include`, non i flag di categoria.** Il documento dice «il comando
   `xiox clean …` che toglierebbe le voci `recommended` nuove». Un flag come `--worktrees` prenderebbe
   anche le voci raccomandate che c'erano già alla partenza e che l'osservatore non garantisce. Il
   comando è quindi fatto di `--include <percorso>` per le sole voci nuove, raccomandate, non bloccate,
   con una cartella, non sotto una radice di disco e che non contengono una voce bloccata; al massimo
   25, con `cleanCommandTruncated: true` se ce ne sono di più. Copie e cartelle trovate per nome sono
   `caution`: non vi entrano mai.

### Altre decisioni prese durante la costruzione

- In modo `--mcp` la fotografia iniziale si fa subito alla partenza, su un thread a parte
  (`xiox-mcp-baseline`): `initialize`, `ping` e `tools/list` rispondono subito anche se l'analisi è
  lenta; `tools/call` aspetta la fotografia fino a 30 secondi e poi risponde con un risultato
  `isError` («still running, call again») senza perderla. Se la prima analisi fallisce il server resta
  acceso e ogni chiamata la ritenta; in quel caso la fotografia è presa dalla prima chiamata riuscita, e
  le voci create prima sono `new: false` (`startedAtMs` del resoconto dice quando è stata presa). In
  modo predefinito, invece, una prima analisi fallita fa uscire con codice 1 e stdout vuoto.
- L'evento `attention` scatta quando il valore è **maggiore** della soglia (con 3 al quarto
  worktree nuovo), una volta per superamento, e si riarma quando il valore torna a soglia o sotto.
- Le voci non misurate valgono 0 nella crescita e vengono misurate di nuovo a ogni controllo; dopo la
  partenza si rimisurano solo le voci nuove e le cartelle `builds` della fotografia.
- Un cambio di categoria della stessa cartella fra due controlli (osservato: un worktree creato mentre
  un controllo girava, elencato come copia da quel controllo e come worktree dal successivo): la
  chiave è il percorso, quindi la voce non è mai contata due volte; `changed` la elenca di nuovo in
  `added` nella categoria nuova (`new` e `firstSeenMs` invariati). Corretto dopo la revisione: la
  prima versione taceva e chi leggeva solo gli eventi restava con la categoria vecchia.
- Il panic hook scrive solo su stderr, senza poter restare bloccato dal blocco di stderr, e il processo
  esce con un codice diverso da zero. `--interval` ha un massimo (30 giorni).

### Prove

- Suite completa del workspace (`cargo test -j 1 --workspace --exclude mukade`): **1097 superati, 0
  falliti, 20 ignorati** (prima dell'osservatore: 1013 superati, 0 falliti, 20 ignorati).
  Totali dei crate toccati: `xiox-observer` 31 test, `xiox-cli` 87 di unità e 27 sul binario (compresi
  quelli di `observe` e `--mcp` che avviano il binario di debug con gli interruttori di prova su alberi
  finti), `xiox-project` 107.
- Build di rilascio (`cargo build -j 1 --release --locked -p xiox-desktop -p xiox-cli`): `xiox.exe`
  3.191.808 byte (circa 3,2 MB), `xiox-ui.exe` 12.262.400 byte (invariato dall'osservatore: l'app non
  dipende da `xiox-cli`).

### Limiti noti

- Non c'è osservazione in tempo reale: un worktree creato e tolto fra due controlli non si vede mai.
- Le dimensioni hanno un tetto (200.000 voci o 10 secondi per cartella): oltre, una voce mai misurata
  ha `bytes: null` e vale 0 nella crescita; una voce già misurata conserva l'ultimo valore e il suo
  `measuredAtMs`, quindi una cartella `builds` che supera il tetto dopo la partenza smette di crescere
  nel resoconto senza dirlo (la crescita può essere sottostimata); un sottoalbero illeggibile fa
  sottostimare il peso.
- Senza elevazione la ricerca di copie e cartelle per nome è la camminata limitata dell'analisi mirata
  (tre livelli): un progetto più in profondità può sfuggire (nota `walk-may-miss-deep-folders`).
- Nessuna attribuzione a una sessione o a un harness: «comparso da quando sono partito».
- Con `--mcp` la prima chiamata che segue una prima analisi fallita ridefinisce «nuovo» (vedi sopra).
- Un chiamante che sparisce senza chiudere le pipe viene notato alla prossima scrittura su stdout o alla
  fine di stdin (Windows le chiude quando un processo muore).
- Il resoconto è lungo per progetti con molte voci; in MCP viaggia due volte (testo e
  `structuredContent`).
- Non è stato misurato il carico su un progetto reale grande: i tetti sono verificati dai test, non da
  una misura sul campo.

### Terza deviazione: nessun timer in modalità MCP

Il documento chiedeva un osservatore che ricontrolla a intervalli. In modalità `--mcp` non c'è nessun
timer: un controllo gira solo quando arriva `tools/call`, e `--interval` è accettato ma ignorato.
Conseguenza dichiarata (guida e `--help`): ciò che viene creato e tolto fra due chiamate non si vede
mai, e nulla viene notato finché l'agente non chiama. È una conseguenza della scelta di non avere
thread di sfondo in un server che vive dentro l'harness; la modalità predefinita (righe JSON) ha il
timer.

### Revisione finale dell'intero ramo: correzioni

La revisione dell'intero ramo (nessun difetto critico) ha dato quattro punti importanti, un punto
obbligatorio e alcuni minori; tutti sono stati corretti con test scritti prima. Decisioni prese:

- **Mai la ricerca da amministratore** (I1). `observe` passa sempre `admin: false`
  (`observe_options`), anche da un terminale elevato: prima ogni controllo, `report` e chiamata MCP
  leggeva l'intera MFT di ogni disco NTFS (circa 300 MB di picco per controllo per osservatore),
  contro «nessuna camminata completa dei dischi a intervalli» e «più osservatori insieme». Linea di
  base e controlli usano la stessa ricerca; il campo `search` non vale più `mft-index`.
- **Misura parziale come limite inferiore** (I2). `measure_bytes` ora dà `Measured { bytes, complete }`:
  oltre i tetti (200.000 voci o 10 s) restituisce quanto contato con `complete: false`, `None` solo se
  la radice non si legge. Il totale parziale entra nella crescita e nei totali (una `target` enorme e
  nuova fa scattare `attention`); la voce porta `bytesPartial: true` (camelCase, sempre presente,
  `false` quando la misura è intera o manca). Scelte: *back-off*: una voce la cui ultima misura ha
  toccato i tetti si rimisura al più ogni 10 minuti (`PARTIAL_BACK_OFF_MS`, a tempo e non «ogni N
  controlli» perché in MCP i controlli sono le chiamate); *budget*: ogni controllo misura al più 30 s
  in totale (`MEASURE_BUDGET`, controllato prima di ogni cartella: si può sforare di una misura, 10 s),
  nell'ordine «mai misurate prima, poi la misura più vecchia»; il resto passa al controllo seguente;
  il budget si legge dall'orologio dell'osservatore (`now_ms`), così i test lo guidano senza
  attese; la linea di base non ha budget (ogni cartella resta limitata dai tetti). Resta vero che di
  una cartella parziale la crescita oltre ciò che la camminata raggiunge non si vede: è un limite
  inferiore, dichiarato.
- **La prima chiamata MCP non lavora due volte** (I3). La chiamata che ha dovuto aspettare la linea
  di base (o che l'ha rifatta perché la prima era fallita) la riporta così com'è, senza un secondo
  `check`. Scelta più prudente della lettera della richiesta: se il risultato della linea di base era
  già pronto quando la chiamata è arrivata (l'agente chiama molto dopo l'avvio), la chiamata fa il
  controllo, altrimenti ciò che è nato nel frattempo resterebbe invisibile fino alla chiamata dopo.
- **Testo MCP compatto** (I4). `structuredContent` resta il resoconto COMPLETO; il testo `content`
  diventa una vista compatta: progetto, `startedAtMs`/`checkedAtMs`/`stale`, totali, crescita,
  `attention`, `cleanCommand` (e se è troncato), solo le voci NUOVE (al più 50, una riga ciascuna:
  categoria, livello, byte, percorso o titolo della sessione, motivi di blocco) e un'ultima riga con
  quante altre voci esistono e dove sta l'elenco completo. Con 300 sessioni archiviate il testo sta
  sotto 8 KB e nomina comunque il worktree nuovo. Descrizione dello strumento, `instructions` e guida
  dicono con chiarezza che un client che inoltra al modello solo `content` vede la vista compatta.
- **Gara nel test del binario** (M1). Il test attende, con un limite, lo stato finale (il worktree
  elencato come worktree) accettando la sequenza copia, poi worktree; il test MCP che dormiva 400 ms
  ora attende la condizione («il risultato della linea di base è in canale») con un sondaggio limitato.
- **Controllo degradato** (minore). Un controllo con un'altra radice del progetto, che non è più un
  repository, o senza worktree dove ce n'erano (e git, interrogato di nuovo, fallisce o ne elenca)
  è un controllo FALLITO (evento `error`, resoconto `stale: true`), mai passato a `update`: una
  scadenza di git non produce più «tutti rimossi, poi tutti nuovi». La domanda a git è una funzione
  nuova di `xiox-project` (`linked_worktree_count`); il comportamento di `analyze` e `clean` non cambia.
  Costo dichiarato: un progetto che smette davvero di essere un repository dà errore a ogni controllo
  finché non si riavvia l'osservatore.
- **Altri minori**: il messaggio del panic del lavoratore della linea di base MCP dice «the first
  analysis stopped on an internal error; the next call retries it»; la coda di stdin è limitata (16
  righe) e le righe `report` già in coda condividono un solo controllo (ognuna ha comunque il suo
  evento `report`); un test strutturale legge i sorgenti di `observe.rs`, `mcp.rs` e di
  `xiox-observer` e prova che non nominano `clean`, `remove`, `delete`, `trash`, `fs::write`,
  `File::create`, `OpenOptions`, `is_elevated` (con la lista dei pochi usi legittimi: il
  `cleanCommand` solo stampato, `remove` di mappe in memoria e il campo `removed` di `changed`).
- **Documentazione**: le dimensioni dell'osservatore sono la lunghezza apparente e possono differire da
  `xiox analyze --json`; `cleanCommand` è citato per il parser di argomenti di Windows, non per
  PowerShell; un solo percorso non Unicode blocca ogni `report` finché è elencato; in MCP le richieste
  si trattano una alla volta; la guida non cita più `mft-index` come modo possibile.

I «Limiti noti» di sopra sulle dimensioni oltre il tetto (`bytes: null`, voce ferma) e sul resoconto MCP
che viaggia due volte sono superati da questa sezione; la ricerca da amministratore non esiste più
nell'osservatore.

Non fatto (elenco «può aspettare» della revisione): salto dei reparse point sui file, radice `Some(0)`
dei link, `is_link` duplicato, dipendenze solo di prova, voce nuova dentro una cartella `builds`
rimisurata, O(n²) di crescita e `holds_blocked`, sovrapposizione di `by_category`, `Closed` trattato
due volte, copia di `report(false)` in `check`, codice di uscita 101 del panic, ripiego dell'hook se
non parte alcun thread, messaggio dell'hook perso se il thread tiene stderr, interruttore di prova che
sostituisce l'hook globale, claim dell'uscita 1 per radice non codificabile che poggia solo sul codice.

Prove dopo la revisione: suite completa del workspace (`cargo test -j 1 --workspace --exclude
mukade`): **1123 superati, 0 falliti, 20 ignorati** (prima: 1097, 0, 20). Totali dei crate toccati:
`xiox-observer` 39 test (erano 31), `xiox-cli` 104 di unità (erano 87) e 27 sul binario,
`xiox-project` 108 (erano 107). Build di rilascio: `xiox.exe` 3.220.480 byte, `xiox-ui.exe`
12.262.400 byte (invariato).
