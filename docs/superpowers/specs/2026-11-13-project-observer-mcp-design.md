# Osservatore dei progetti e servizio MCP (`xiox serve`)

Data: 2026-11-13. Stato: da approvare.

Dipende da `2026-11-13-project-cleanup-cli-design.md` (analisi mirata di un progetto e binario
`xiox`): va costruito dopo. Non tocca l'interfaccia.

## Perché

Gli agenti (Claude Code, Codex, OpenCode, Pi), lavorando su un progetto, creano worktree, copie,
backup, cartelle di prova e build, e nessuno se ne accorge finché il disco è pieno. L'utente
vuole che qualcosa «stia a osservare le sessioni di un harness per un progetto» e segnali «il
proliferare di file, worktree, copie, backup». L'analisi mirata dice cosa c'è adesso;
l'osservatore dice cosa è comparso, quando e durante quale sessione, e lo dice all'agente che
sta lavorando, così può rimediare prima di chiudere.

## Cosa si costruisce

Un servizio locale, `xiox serve`, che:

1. tiene una fotografia di ogni progetto su cui c'è una sessione di un agente aperta;
2. rifà l'analisi mirata a intervalli e alla chiusura di ogni sessione, e calcola la differenza;
3. espone la differenza come servizio MCP in sola lettura, a cui gli agenti chiedono «cosa è
   stato lasciato in giro in questo progetto».

## Decisioni dell'utente

- L'osservatore **avvisa soltanto**: non cancella mai niente.
- L'avviso arriva **solo come risposta agli agenti** (feedback attraverso MCP). Niente notifiche
  di Windows, niente finestra.
- La forma è un **servizio MCP che resta in ascolto**.
- **Due processi separati: monitor e pulizia.** Il monitor (`xiox serve`) non pulisce mai. Per
  pulire si lancia a parte `xiox clean <project> …`, dall'utente o dall'agente che ha ricevuto
  l'avviso; la risposta del monitor porta già pronto il comando da lanciare.
- Tutto in inglese (nomi degli strumenti, descrizioni, testi), come il resto di Xiox.

## Decisioni mie (da controllare)

- **Stesso impianto di mukade**: servizio HTTP locale su `127.0.0.1` (MCP «streamable HTTP»), con
  le stesse librerie e versioni di mukade (`rmcp` 2.2.0, `axum` 0.8.9), in sola lettura. Porta
  fissa diversa da quella di mukade (47631); se è occupata, errore chiaro.
- **Nessuna configurazione dei progetti**: il servizio scopre da solo i progetti dall'attività
  degli agenti. Un agente può anche nominare un progetto qualunque nella richiesta (una cartella
  esistente su un disco fisso): se non c'è una fotografia, la prima richiesta la crea.
- **Cosa vuol dire «sessione aperta».** Il codice di oggi lo sa con certezza solo per Claude Code
  (`SessionInfo.live`, dal file `sessions/<pid>.json`); per Codex, OpenCode e Pi il dato non
  esiste. Regola unica per tutti: un harness è **attivo** su un progetto se la sua ultima
  attività su quel progetto è di meno di 15 minuti fa, oppure (solo Claude Code) se la sessione
  risulta viva. Inizio e fine di un periodo di attività sono gli istanti in cui il monitor lo
  vede cominciare e finire, non orari letti dalle sessioni. Niente modifiche a `vendor/mukade`.
- **Il progetto di una sessione** è quello dell'analisi mirata: una sessione partita in una
  sottocartella o in un worktree (anche dentro le cartelle di lavoro degli agenti) conta per il
  repository principale; una cartella senza repository è un progetto a sé.
- **L'elenco delle sessioni è per harness, non per singola sessione**: `dc-mukade` tiene, per
  progetto e harness, solo l'attività più recente e un id. L'attribuzione quindi nomina
  l'harness, più l'id della sessione quando è l'unico noto in quel periodo.
- **A chi è attribuita una cosa nuova**: a un harness se è comparsa fra due analisi durante le
  quali era l'unico attivo sul progetto; altrimenti a «uno fra» quelli attivi; a nessuno se non
  ce n'erano o se c'è un dubbio (periodi finiti nello stesso giro di controllo, monitor partito
  dopo). Ciò che esisteva già alla prima fotografia di un progetto non è mai «nuovo», nemmeno se
  la sessione era cominciata prima che il monitor partisse.
- **Come l'agente viene a saperlo**: MCP risponde solo a chi chiede. Le istruzioni del servizio
  (campo `instructions`) e la descrizione dello strumento dicono all'agente di interrogarlo prima
  di dichiarare finito un lavoro e dopo aver creato worktree o copie. Niente hook dei singoli
  harness.
- **Soglie**: la risposta porta un campo `attention` vero quando, dall'inizio della sessione, il
  progetto è cresciuto di più di 2 GB o ha più di 3 worktree collegati. Valori fissi nel codice
  per ora, facili da spostare in impostazioni.
- **Avvio**: `xiox serve` si lancia a mano e resta in primo piano nel terminale (Ctrl+C lo ferma).
  `xiox serve --print-config` stampa il frammento da incollare nella configurazione MCP di ogni
  harness. Niente avvio automatico con Windows e niente scrittura nei file di configurazione
  degli agenti.
- **Non elevato**: il servizio gira senza permessi di amministratore, quindi cerca copie e
  cartelle per nome con la camminata limitata dell'analisi mirata (non con l'MFT).
- **Le fotografie stanno su disco** (in `dc_app::cache_dir()`), così riavviare il servizio non
  azzera la storia. Si tengono per progetto: la fotografia di inizio di ogni sessione ancora
  aperta e l'ultima; il resto si butta.

## Strumenti MCP (sola lettura)

| Strumento | Ingresso | Uscita |
|---|---|---|
| `project_leftovers` | `project` (percorso; anche un worktree o una sottocartella), `since` facoltativo: `session` (predefinito: dall'inizio del periodo di attività in corso sul progetto; se nessun harness è attivo, dall'inizio dell'ultimo) o `first_seen` (dalla prima fotografia) | l'elenco dell'analisi mirata (categorie `builds`, `worktrees`, `copies`, `named`, `logs`, `sessions`, con percorso, peso, livello, motivo), più per ogni voce `new: true/false`, `first_seen`, `attributed_to` (harness e, se noto, id della sessione; o `one_of`; o niente), `measured_at` (quando è stato misurato il peso); totali; crescita in byte da `since`; `attention`; `stale` se la risposta viene da un'analisi precedente; il comando `xiox clean …` che toglierebbe le voci `recommended` |
| `watched_projects` | niente | i progetti osservati, con sessioni aperte, peso attuale, crescita, `attention` |

La forma dell'elenco è la stessa del `--json` di `xiox analyze` (stesso `schema`), con i campi in
più. Il servizio non ha strumenti che scrivono.

## Come osserva

- Ogni 60 secondi: elenco dell'attività degli agenti (costa poco). Harness che diventa attivo su
  un progetto senza fotografia → fotografia. Periodo di attività finito → analisi e confronto.
- Ogni 10 minuti, per i progetti con un harness attivo: analisi leggera (`git worktree list`,
  cartelle per nome, copie, senza pesi). I pesi si misurano per le voci nuove, alla fine di un
  periodo di attività e quando un agente chiede `project_leftovers`; la crescita si calcola sui
  pesi misurati, e ogni voce dice quando è stata misurata.
- Una richiesta `project_leftovers` rifà l'analisi leggera e rimisura i pesi prima di
  rispondere. Se un'altra analisi è in corso aspetta fino a 20 secondi; oltre, risponde con
  l'ultimo risultato e `stale: true`.
- L'analisi leggera (senza pesi) e la chiave stabile di ogni voce (`dc_model::path_key` del
  percorso) fanno parte della funzione di libreria dell'analisi mirata.

## Vincoli

- Deve pesare poco: la macchina ha poca memoria libera e dischi quasi pieni. Nessuna camminata
  completa dei dischi a intervalli; mai più di un'analisi alla volta.
- Ascolta solo su `127.0.0.1`. Nessun dato esce dal PC.
- Sola lettura: il servizio non chiama mai la pulizia e non contiene il codice per farlo
  raggiungibile da MCP.
- `cargo` sempre con `-j 1`, mai due build insieme. `rmcp`, `axum` e `tokio` entrano solo nel
  binario `xiox`, non nei crate del motore né nella finestra.
- Test su alberi finti e sessioni finte in `%TEMP%`; mai sui progetti e sulle sessioni vere
  dell'utente.

## Non si fa

- Nessuna pulizia automatica, nessuno strumento MCP che cancella.
- Nessuna notifica di Windows, nessuna icona nell'area di notifica, nessuna finestra.
- Nessun avvio automatico, nessun servizio di Windows.
- Nessuna osservazione in tempo reale dei file (`ReadDirectoryChangesW`, giornale USN): solo
  analisi a intervalli.
- Nessuna modifica ai file di configurazione degli agenti.

## Sarebbe un fallimento se

- il servizio cancellasse o spostasse qualcosa;
- rallentasse la macchina mentre gli agenti compilano;
- attribuisse con certezza a una sessione una cosa creata da un'altra;
- segnalasse come «nuovo» ciò che c'era già prima della sessione.

## Lasciato al builder

- Porta, nomi dei tipi, forma esatta dei campi aggiunti, testi inglesi di `instructions` e delle
  descrizioni.
- Come riusare il riconoscimento delle sessioni vive e l'elenco delle sessioni per progetto
  (`dc-mukade`, `dc-sessions`, `dc_app::load_sessions`).
- Formato dei file delle fotografie.
- Come tenere fuori `tokio`/`axum` dal resto del workspace (feature o crate separato).
