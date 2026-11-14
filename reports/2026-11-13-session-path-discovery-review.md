# Validazione della scoperta dei percorsi: decisioni aggiornate e revisione originaria

Data: 2026-11-13. Stato aggiornato: **specifica revisionata secondo i chiarimenti dell'utente**.

La [revisione 2 della specifica](../docs/superpowers/specs/2026-11-13-session-path-discovery-design.md)
è il riferimento corrente. Il codice della funzionalità non è stato implementato da questa
revisione; le soluzioni descritte devono ancora essere provate durante la costruzione.

## Decisioni che sostituiscono le premesse precedenti

- La conferma è esplicita al tool; non importa se arriva da UI, persona o agente. Non serve
  dimostrare che il confermante sia umano.
- `danger` comunica il rischio e richiede una scelta. Attività recente o vitalità sconosciuta
  non richiedono una nuova garanzia di inattività; restano i blocchi certi già applicati dal motore.
- La provenienza da una sessione viene soltanto dalla cronologia letta da Mukade. Il filesystem
  verifica il candidato attuale e può segnalare recenza; i suoi timestamp non attribuiscono cartelle.
- Mukade trova e legge le sessioni. Xiox interpreta tool, messaggi e risultati per individuare
  percorsi e residui. Si parte da `Reader::list_all` e `Reader::read`: un'estensione di Mukade
  non è un prerequisito. Eventuali miglioramenti della lettura rimangono generici.

## Stato dei dodici rilievi

"Recepito" significa che il design è stato aggiornato, non che il codice abbia già superato i test.
Le severità e i numeri di riga nell'allegato storico riguardano la versione 1.

| Rilievo | Stato nella revisione 2 |
|---|---|
| R01, conferma umana | Superato dal chiarimento: è richiesta una conferma esplicita, valida anche da un agente |
| R02, inattività garantita | Requisito corretto: rischio visibile e confermabile per attività recente/ignota; nessuna garanzia universale di inattività |
| R03, timestamp come provenienza | Recepito: eliminati recupero per data e conferma NTFS obbligatoria; le evidenze distinguono operazione richiesta, esito e menzione |
| R04, conflitti e sorgenti mancanti | Recepito con copertura dichiarata: conflitti documentati bloccano; letture parziali segnalano rischio, non dimostrano assenza di conflitti |
| R05, antenati e categorie | Recepito: candidato limitato al percorso dell'evidenza; controlli trasversali alle categorie e restrizioni dei discendenti rispettate |
| R06, Cestino dei worktree | Recepito con eccezione esplicita: Git rimuove definitivamente i worktree; metodo mostrato nella conferma |
| R07, destinazioni indirette Git | Controllo richiesto dalla spec; da dimostrare su fixture durante l'implementazione |
| R08, cache incrementale | Recepito: nessuna ripresa persistente basata sul solo prefisso; riuso iniziale nella singola analisi e rilettura delle evidenze rilevanti |
| R09, contratto Mukade | Corretto: API esistenti e adapter non aggregato; nessun riconoscimento di residui da trasferire a Mukade |
| R10, limiti di risorse | Recepito: budget effettivi e risultato parziale dichiarato; valori e meccanismi da provare nella prima fase |
| R11, SQLite read-only | Contratto precisato: nessuna modifica dei dati delle sessioni; gestione WAL/sidecar dei reader esplicitata e da verificare |
| R12, campione della sonda | Recepito: conteggi predefiniti e modalità locale `--sample`; prove sintetiche avviabili senza attendere un campione reale |

## Conseguenze operative

Si può preparare il piano di implementazione della revisione 2: prima adapter e prova tecnica
sulle API esistenti, poi candidati dai `cwd`, interpretazione della cronologia in Xiox,
`scratch`, rischio e conferme. Il lavoro comprende test e aggiornamento delle guide.

L'eccezione definitiva dei worktree conserva il metodo Git già usato dal progetto. Per le
cartelle ordinarie `danger` resta il Cestino; una richiesta incompatibile di rimozione
permanente viene rifiutata prima di agire. Nessuna modifica a codice, store o repository Mukade
è stata eseguita nell'aggiornamento dei due documenti.

Impronta SHA-256 della specifica revisionata: `06ae470652b331136ee6ef0fc5e5f595dd198ab1df9f09f8435caaa2b44f0586`.

## Allegato storico: revisione della versione 1

Il testo seguente conserva controesempi, prove e fonti raccolte sullo snapshot originario.
Le sue proposte non prevalgono sulle decisioni e sugli stati riportati sopra. In particolare,
non sono più requisiti la conferma necessariamente umana, una prova universale di inattività,
l'attribuzione tramite timestamp o una nuova API Mukade obbligatoria.

## Oggetto e metodo

- Oggetto principale: `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`, 276 righe, SHA-256 `51d79a3a88318f11c798d111c39a38556595b7a3035c77603911423d53734924`.
- Confronto con le specifiche `project-cleanup-cli-design` e `project-observer-mcp-design`, la ricerca `reports/Estrazione path da transcript agenti.md` e il codice delle integrazioni interessate.
- HEAD osservato: `e8b0bbb6dfba3f728e1e71286731d316a91258eb`. La working tree aveva modifiche concorrenti: questa revisione non le modifica. Il riferimento determinante per la spec è l'impronta sopra.
- Verifica delle premesse mediante lettura del codice, controesempi logici, documentazione primaria e quattro prove circoscritte su dati sintetici.
- Nessuna lettura degli store reali degli agenti; nessuna esecuzione di `xiox clean`; nessuna prova distruttiva sui progetti.

Le segnalazioni descrivono difetti o lacune del **design**. Non sostengono che la nuova funzionalità sia già implementata o che i controesempi abbiano già causato perdite reali.

## Valutazione rapida

| Parte | Valutazione |
|---|---|
| Candidati dai `cwd`, verificati tramite Git | Valida; serve conservare dati per sessione e cambi di `cwd` |
| Estrazione selettiva dalle chiamate ai tool | Valida; una richiesta al tool non certifica il successo dell'operazione |
| Data NTFS come informazione aggiuntiva | Utile |
| Data NTFS come prova della creazione da parte di una sessione | Non dimostrata |
| Mostrare candidati deboli con `danger` | Coerente con la richiesta dell'utente |
| `danger` come garanzia di conferma umana e inattività | Contraddetto dalle regole proposte |
| Riutilizzo di Mukade | Corretto; manca l'API adatta a questa lettura selettiva |
| Sonda preliminare | Utile; V12 non è eseguibile con il solo output aggregato previsto |

## R01 — Alta: `--confirm-danger` non dimostra una conferma umana

**Riferimenti:** spec 169–174, 211–213, 247.

La spec richiede una conferma personale per ogni voce ma accetta, senza terminale, `--confirm-danger <path>`. Un agente che conosce il percorso può passare anche questo parametro. Una domanda su terminale non distingue a sua volta una persona da un agente capace di pilotare una PTY.

**Controesempio:** un agente esegue `xiox clean <project> --yes --include <candidate> --confirm-danger <candidate>`. Soddisfa il contratto CLI descritto senza coinvolgere una persona.

**Controverifica:** il parametro aggiuntivo riduce gli errori accidentali rispetto al solo `--yes`; non è inutile. Non soddisfa però la garanzia più forte scritta nella spec.

**Correzione:** scegliere esplicitamente il contratto. Se basta una selezione esplicita per percorso, dichiararlo. Se serve davvero autorizzazione umana, occorre un canale di approvazione con un confine di fiducia separato dall'agente, legato al piano e alla sua versione. Un ulteriore flag o token generabile dallo stesso agente non risolve il problema.

## R02 — Alta: attività recente e inattività verificata sono condizioni diverse

**Riferimenti:** spec 164–165, 185–188, 249; `crates/xiox-project/src/owned.rs:208`.

Per Codex, OpenCode e Pi la spec ammette che la vitalità non sia verificabile; assegna `danger` alle attività delle ultime 24 ore. Una voce `danger`, però, resta rimovibile con la conferma prevista.

**Controesempio:** Codex lavora in una copia pulita, al momento tra due scritture. La copia è `danger`, viene confermata e rimossa mentre la sessione è ancora aperta. Anche un processo lungo senza eventi da oltre 24 ore sfugge alla semplice soglia temporale.

**Controverifica:** una sessione già verificata viva resta bloccata. Il difetto riguarda gli stati sconosciuti e le attività esterne alla cartella `cwd`, non quel blocco esistente.

**Correzione:** modellare `active / inactive / unknown`; un timestamp recente è un segnale, non una prova di chiusura. Per mantenere l'invariante assoluto occorrono cooperazione del runtime, registrazioni/lease affidabili o un blocco quando l'inattività non è dimostrabile. Il cambio di attività tra controllo e rimozione richiede inoltre un protocollo di coordinamento; ripetere la scansione non rende atomiche le due operazioni.

## R03 — Alta: l'intervallo della sessione non prova creazione, proprietà o autore delle modifiche

**Riferimenti:** spec 102–103, 129–138, 193–194, 246–248.

La presenza di un comando e di una data compatibile dimostra una correlazione. Non dimostra chi abbia creato la cartella, se il comando sia riuscito, né se il contenuto sia ancora quello lasciato dall'agente. Il primo e l'ultimo record non delimitano necessariamente un periodo continuo di lavoro: una sessione può essere ripresa dopo giorni.

**Controesempi:**

- Una cartella vicina viene creata da una persona mentre una sessione è aperta: soddisfa il recupero basato sulla sola data.
- Un `mkdir` fallisce o è soltanto tentato; un altro processo crea la cartella nell'intervallo considerato.
- L'agente termina lunedì; una persona modifica un risultato martedì; la stessa sessione viene ripresa mercoledì. La modifica umana precede l'ultimo record e G3 non la blocca.
- Una copia o un ripristino conserva o reimposta i timestamp: il filtro obbligatorio può scartare un residuo vero o attribuire una cartella preesistente.

**Evidenza:** Windows permette di impostare la data di creazione senza modificare il contenuto [S1]. La prova T1 lo ha confermato su una cartella sintetica. T2 mostra anche che scrivere un file figlio non aggiorna necessariamente il `mtime` della directory: “cartella modificata” deve specificare se riguarda l'intero sottoalbero.

**Controverifica:** la spec riconosce il problema dei timestamp in V11 e nei limiti noti. Consentire voci deboli è una decisione esplicita dell'utente, quindi non va eliminato il recupero per data. Va corretta la forza attribuita alla prova e la promessa di protezione delle modifiche umane.

**Correzione:** distinguere `path-mentioned`, `write-attempted`, `write-succeeded`, `creation-observed` e `time-correlated`. Usare esiti strutturati dei tool quando disponibili, senza conservare stdout o chat. Conservare gli eventi temporali, non un solo intervallo globale. Per dimostrare che il contenuto non è cambiato servono osservazioni precedenti, identità del file e un confronto dello stato; senza queste informazioni, dichiarare l'incertezza.

## R04 — Alta: G3 richiede informazioni che la parte B esclude; gli errori possono nascondere blocchi

**Riferimenti:** spec 97–98, 190–204, 216–219; `crates/xiox-project/src/clean.rs:634`; `crates/xiox-project/src/nested.rs:22`.

B legge le sessioni del progetto e delle sue copie. G3 vuole bloccare cartelle in cui hanno scritto sessioni di un altro progetto. Non è specificato come conoscere quelle scritture senza leggere anche le altre sessioni.

**Controesempio:** Phazly e Hostay scrivono nella stessa `D:\agent-runs\shared`. Analizzando Phazly, Xiox non legge i tool della sessione Hostay: non può applicare il blocco di condivisione. Lo stesso accade se il transcript Hostay esiste ma è illeggibile.

“Se una sessione non si legge, non aggiungere candidati” è conservativo per la scoperta, ma non per la cancellazione: la sessione mancante potrebbe contenere proprio il motivo per vietarla. Anche una scansione incompleta del contenuto deve essere distinta dall'assenza di repository annidati; l'helper attuale `nested_repos` salta alcuni errori di lettura/metadata.

**Controverifica:** il codice attuale blocca alcune rimozioni quando fallisce il listing delle sessioni. Questo è positivo, ma non copre automaticamente un listing riuscito con transcript parziali o una sola sorgente incompleta.

**Correzione:** separare ambito di scoperta e ambito dei controlli di conflitto. Definire un indice delle scritture per percorso, sessione e progetto, con copertura esplicita; può essere costruito attraverso Mukade senza introdurre una UI globale. Quando le informazioni necessarie a escludere un conflitto non sono disponibili, conservare il candidato ma bloccare la rimozione interessata. Errori, retention e formati non supportati devono restare distinguibili da “nessun evento”.

## R05 — Alta: categoria e antenati possono aggirare le nuove protezioni

**Riferimenti:** spec 142–155, 169–174, 190–198; `crates/xiox-project/src/clean.rs:112`, `:163`, `:938`.

La stessa cartella può diventare `named` invece di `scratch` per il solo nome, ma G3 è formulata soltanto per `scratch`. Inoltre la scelta della cartella più alta nata durante la sessione può includere rami non descritti dalla prova.

**Controesempi:**

- `benchmark-42` riceve G3; rinominata `phazly-benchmark-42`, diventa `named` e quelle verifiche non sono più esplicitamente richieste.
- Il tool scrive in `D:\agent-runs\batch\phazly`; la regola promuove la voce a `batch`, che contiene anche risultati di Hostay.
- Un contenitore selezionato contiene una voce `danger`. La regola attuale propaga i blocchi dei discendenti, non il loro livello o l'obbligo di conferma individuale. Confermare solo il contenitore può quindi eludere la nuova regola “voce per voce”.

**Controverifica:** i discendenti già `blocked` sono protetti dal codice esistente. Una voce `danger` non è necessariamente bloccata: la nuova policy va aggiunta esplicitamente.

**Correzione:** applicare i vincoli alla prova e all'intero sottoalbero, indipendentemente dalla categoria visuale. Non salire oltre la directory con evidenza diretta senza una verifica completa del contenuto. Un antenato deve ereditare i blocchi e gli obblighi dei discendenti; la rimozione non può rendere meno restrittiva la loro policy.

## R06 — Alta: “sempre nel Cestino” confligge con la rimozione dei worktree

**Riferimenti:** spec 164, 174, 242; `crates/xiox-project/src/clean.rs:104`; `crates/xiox-cleaner/src/parallel.rs:167`.

I worktree recenti diventano `danger`, ma il percorso di rimozione attuale usa `GitWorktreeRemove` indipendentemente da `RemoveMode`. La specifica precedente impone questa strada; il codice la descrive espressamente come definitiva anche in modalità Cestino.

**Controesempio:** worktree pulito creato un'ora fa, selezionato e confermato come `danger`. Ereditando il comportamento esistente viene rimosso da Git, non spostato nel Cestino.

**Controverifica:** la rimozione Git senza `--force` protegge diversi stati sporchi [S2]. Non offre però il recupero dal Cestino promesso dal nuovo contratto.

**Correzione:** specificare una scelta: bloccare i worktree `danger`, progettare una procedura recuperabile con gestione dei metadati Git, oppure introdurre un'eccezione esplicita alla promessa del Cestino. Non basta ignorare `--permanent`.

## R07 — Alta: il filtro del percorso non controlla le destinazioni indirette di Git

**Riferimenti:** spec 86–90, 178–183, 250; `crates/xiox-project/src/search.rs:118`; `crates/xiox-git/src/lib.rs:202`.

G1 controlla il percorso candidato e i suoi antenati. Un repository locale può però contenere un normale file `.git` che indica un'altra directory; esistono anche object store alternativi e configurazioni con riferimenti esterni [S3]. Questi non sono necessariamente symlink o junction.

**Controesempio statico:** `D:\agent-runs\candidate` supera i controlli del percorso, ma il suo `.git` contiene un `gitdir:` verso una destinazione vietata. `classify()` invoca Git sulla cartella, lasciando a Git la risoluzione di quel riferimento.

**Controverifica:** l'ambiente dei subprocess Git viene ripulito da alcune variabili e sono previsti timeout. Queste misure non validano i riferimenti presenti nel repository. Non è stato contattato alcun server per provare il caso UNC.

**Correzione:** estendere il confine di validazione alle destinazioni che Git può seguire, oppure isolare le interrogazioni con restrizioni che rendano effettiva la promessa di nessun accesso alla rete/cloud. L'apertura dei percorsi va fatta controllando gli antenati prima di attraversarli. La semplice verifica lessicale del candidato non dimostra l'invariante completo.

## R08 — Alta: la regola della cache può riutilizzare prove obsolete

**Riferimenti:** spec 118–125, 206–207.

“File cresciuto e primo KiB invariato” non implica che il contenuto precedente all'offset sia rimasto identico.

**Controesempio riprodotto, T3:** header di oltre 1 KiB invariato; una prova successiva viene sostituita da `alpha` a `bravo`, mantenendo la lunghezza della riga; si aggiunge una riga finale. La regola descritta riparte dall'offset precedente, conserva `alpha` e non vede `bravo`. Può quindi perdere una nuova attribuzione o una scrittura che dovrebbe impedire la cancellazione.

**Controverifica:** per un formato garantito append-only la strategia può funzionare. Tale garanzia non è stabilita per tutte le sorgenti. Un hash vicino all'offset aiuta contro alcune rotazioni ma non prova, da solo, l'immutabilità dell'intero prefisso.

**Correzione:** definire identità/generazione della sorgente e il contratto append-only per ciascun formato; in caso di dubbio invalidare e rileggere. La sicurezza della rimozione non deve dipendere da prove riutilizzate senza validità sufficiente. La cache deve preservare anche lo stato necessario alla ripresa, come il `cwd` effettivo del turno, oltre al cursore. Per SQLite servono revisioni/transazioni proprie: gli aggiornamenti possono stare nel WAL mentre il file principale resta invariato [S4].

## R09 — Media: il contratto con Mukade non espone ancora i dati necessari

**Riferimenti:** spec 48, 84–93, 109–111, 221–222.

Il riuso di Mukade è corretto, ma non basta collegarsi ai dati già caricati:

- `crates/xiox-mukade/src/lib.rs:132` aggrega per cartella e conserva l'ID più recente: il conteggio e la provenienza completa delle sessioni sono persi.
- `vendor/mukade/src/reader/mod.rs` espone un solo `cwd` per `SessionInfo`.
- `vendor/mukade/src/reader/codex.rs:1008` legge il `cwd` dall'header: non è la sequenza dei cambi di directory dei turni.
- `vendor/mukade/src/reader/json.rs:116` legge l'intero JSONL in memoria. Usare direttamente `Reader::read` non realizza il nuovo contratto di lettura limitata e incrementale.
- Il reader OpenCode documenta `session_v2` e `session_message`; la lettura completa usa l'export nativo. La tabella generica della spec non definisce ancora l'adapter effettivo.

**Controesempio:** una sessione parte nel progetto A e poi cambia `cwd` al progetto B. Il solo listing iniziale perde B e può attribuire male eventi successivi. Molte sessioni nella stessa cartella diventano invece un solo record Xiox.

**Controverifica:** i reader esistenti possiedono parte delle informazioni e possono essere estesi. È un contratto da completare, non una ragione per abbandonare Mukade.

**Correzione:** definire un'API Mukade per enumerare descrittori non aggregati e scorrere eventi selezionati con `session_id`, sorgente, posizione, timestamp, `cwd` effettivo, tipo di tool, parametri utili, eventuale esito e stato di copertura. `xiox-mukade` adatta questi dati; Xiox decide candidati, rischio e pulizia. Preferire l'estensione a monte con aggiornamento della versione vendorizzata, evitando due parser permanenti. Prevedere fixture per chiamate Codex custom/freeform, pagine, compressione e schemi OpenCode supportati.

## R10 — Media: leggere a blocchi non impone limiti di risorse

**Riferimenti:** spec 118–120, 198–201, 251, 275–276.

Un transcript può contenere una singola riga enorme, dati compressi che si espandono molto o un flusso che cresce mentre viene letto. Il tetto ai candidati verificati con Git non limita questi costi. “Leggere a blocchi” non limita la memoria se i blocchi vengono accumulati fino al newline.

**Controverifica:** V6 serve a misurare prestazioni reali. Una misura sui dati normali non definisce il comportamento per input patologici.

**Correzione:** contrattualizzare limiti di riga, byte decodificati, tempo, profondità JSON, numero di prove e dimensione della cache; leggere fino a un confine di snapshot; definire cancellazione e uscita parziale. Un limite raggiunto deve produrre copertura incompleta e non un falso “nessuna scrittura”. I valori possono essere tarati nella fase 0, ma i limiti devono esistere già nel design.

## R11 — Media: SQLite read-only non significa zero scritture nella directory

**Riferimenti:** spec 61, 71, 233; `vendor/mukade/src/reader/opencode.rs:481`; `crates/xiox-mukade/src/lib.rs:319`.

Aprire il database con `SQLITE_OPEN_READ_ONLY` impedisce gli aggiornamenti SQL al database principale. Non basta a garantire l'assenza di file ausiliari nella directory: SQLite documenta che una connessione WAL in sola lettura può creare `-wal` e `-shm` [S4].

**Prova T4:** su un database sintetico WAL, una connessione Python `mode=ro` seguita da `SELECT` ha creato entrambi i file. Runtime della prova: Python 3.13, SQLite 3.50.4; non è una prova del binario Rust vincolato dal progetto.

**Controverifica:** nessuna riga utente è stata modificata dalla query. Se l'intenzione era vietare modifiche ai dati, il contratto può essere ristretto a quello. La formulazione attuale vieta ogni scrittura nello store.

**Correzione:** decidere e testare il livello effettivo di non interferenza, includendo sidecar, lock e durata delle transazioni. Per zero scritture reali serve una strategia compatibile con WAL attivo, da verificare sul runtime Rust. Non impostare semplicemente `immutable=1` su un database vivo: SQLite avverte che può restituire risultati errati se il file cambia [S5].

## R12 — Media: la sonda non rende eseguibile il criterio di approvazione V12

**Riferimenti:** spec 59–61, 69, 75–80, 264–265.

La sonda può stampare solo conteggi e tempi, ma V12 chiede alla persona di controllare un campione delle cartelle trovate. Non è previsto come identificare quel campione. Anche V5 chiede i nomi dei campi, e V11 richiede esperimenti di copia/spostamento/ripristino che non appartengono a una sonda che non scrive nulla.

**Controverifica:** una verifica manuale separata è possibile. Deve però essere descritta perché V12 è un requisito per costruire B, non un controllo opzionale.

**Correzione:** separare riepilogo condivisibile e vista locale del campione, che può mostrare percorsi e tipi di prova senza esportare chat. Eseguire V11 su fixture sintetiche separate. Definire la selezione e la dimensione del campione per agente/tipo di prova, includendo negativi e progetti concorrenti. Sostituire “maggioranza netta” con una soglia misurabile e distinguere precisione della scoperta da sicurezza della cancellazione.

## Altri punti da chiarire

| Punto | Perché serve una decisione |
|---|---|
| Mostrare “quale comando” ma vietare testo della sessione nell'output | Conservare un tipo di operazione e un riferimento all'evento; scegliere esplicitamente se la UI locale può recuperare il comando su richiesta |
| `scratch` dentro il progetto | `check_dir` rifiuta attualmente `ProjectFolder` sovrapposti al progetto (`clean.rs:888`). Serve una policy dedicata che verifichi anche i discendenti tracciati; non si può rimuovere quel controllo per tutte le cartelle |
| Transizione `caution` → `danger` dopo la selezione | `recheck` confronta oggi esplicitamente `Level::Caution` (`clean.rs:620`). Va definito un ordinamento dei rischi e invalidata una conferma divenuta insufficiente |
| `--no-drives` | Chiarire se i percorsi scoperti nelle sessioni possono comunque portare ad altri dischi, o se il limite di ricerca vale anche per loro |
| Sessioni di copie scoperte tramite altre sessioni | Definire se la scoperta è a un passaggio o iterativa, con deduplicazione e limiti; altrimenti la copertura dipende dall'ordine |
| `xiox observe` come copertura futura | La sua spec dice “comparso da quando sono partito”, esclude l'attribuzione alla sessione e non garantisce un archivio storico persistente. Non sostituisce automaticamente le prove dei transcript scaduti |

## Prove eseguite e loro limiti

Ambiente osservato: Windows NT 10.0.26200.0. Tutte le scritture riguardano fixture temporanee generate per questa revisione.

| ID | Prova | Risultato |
|---|---|---|
| T1 | Creare una cartella e impostarne `CreationTimeUtc` al 2020 | Data riletta esattamente uguale al valore impostato: timestamp modificabile |
| T2 | Impostare il `mtime` della directory, poi riscrivere un file già presente | `mtime` del figlio aggiornato; quello della directory invariato |
| T3 | Sostituire una riga dopo il primo KiB e aggiungere una riga finale | Header identico, file più lungo; la ripresa dal vecchio offset non legge la prova sostituita |
| T4 | Creare e chiudere un database WAL, riaprirlo `mode=ro`, fare `SELECT` | Compaiono `fake-opencode.db-wal` e `fake-opencode.db-shm` |

T3 riproduce l'algoritmo descritto dalla spec, non un'implementazione Xiox già esistente. T4 dimostra che la sola modalità read-only non fornisce la garanzia; va ripetuta con SQLite/Rust del progetto quando si implementa l'adapter. Per T1 il controllo definitivo è stato ripetuto usando un valore UTC esplicito e confrontando i tick, eliminando l'ambiguità di conversione `DateTime` del primo tentativo.

Non sono stati eseguiti: sonda su sessioni reali, misure di falsi positivi, prove sul Cestino, test di concorrenza con agenti attivi o accessi UNC. Non è stata lanciata la suite Rust: non è cambiato codice eseguibile. Gli strumenti di mappatura completa, registrazione formale della review e verifica con revisori isolati previsti dal processo completo non erano disponibili; questo rapporto non attesta un PASS di quel processo. Le contraddizioni logiche e le prove sopra restano verificabili separatamente.

## Contratto proposto per la revisione della spec

1. **Mukade fornisce osservazioni.** Descrittori di sessione ed eventi selettivi, con contesto e copertura; nessuna decisione di pulizia.
2. **Xiox propone candidati.** Una menzione amplia la ricerca. La verifica Git conferma una relazione fra repository; la sola data resta correlazione esplicita.
3. **Xiox valuta separatamente la rimozione.** Attribuzione, vitalità, integrità della scansione, contenuti annidati e autorizzazione sono dimensioni diverse. `danger` non sostituisce un blocco.
4. **L'incertezza resta visibile.** Non leggere una sorgente o saltare un comando non equivale a dimostrare che nessun altro abbia scritto.
5. **Prima si rendono verificabili le invarianti di sicurezza**, poi si implementano la scoperta A e la lettura B. I candidati deboli possono essere mostrati anche quando la rimozione resta vietata.

Questa struttura conserva la richiesta di trovare anche cartelle senza Git, senza presentare una deduzione temporale come una certezza sulla loro origine o sulla possibilità di cancellarle.

## Fonti primarie verificate

- [S1 — Microsoft: SetFileTime](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfiletime): modifica dei timestamp di file e directory.
- [S2 — Git: git-worktree](https://git-scm.com/docs/git-worktree): semantica e limiti della rimozione dei worktree.
- [S3 — Git: repository layout](https://git-scm.com/docs/gitrepository-layout): gitfile, directory comuni e object store alternativi.
- [S4 — SQLite: WAL, in particolare sezioni 2 e 5](https://www.sqlite.org/wal.html): aggiornamenti nel WAL e condizioni di apertura read-only.
- [S5 — SQLite: URI, parametro immutable](https://www.sqlite.org/uri.html): implicazioni di dichiarare immutabile un database che può cambiare.
