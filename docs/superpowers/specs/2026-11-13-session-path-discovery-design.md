# Copie e cartelle di prova scoperte dalle sessioni degli agenti

Data: 2026-11-13. Revisione 2: aggiornata alle decisioni dell'utente; implementazione da pianificare.

Dipende da [analisi mirata e pulizia CLI](2026-11-13-project-cleanup-cli-design.md).
Si integra con [l'osservatore di progetto](2026-11-13-project-observer-mcp-design.md).
La [revisione tecnica](../../../reports/2026-11-13-session-path-discovery-review.md) conserva
le verifiche precedenti e indica come sono state recepite o superate.

## Obiettivo

Trovare copie del repository, worktree e cartelle di lavoro lasciate dagli agenti anche fuori
dai percorsi raggiunti dalla ricerca attuale: benchmark, test, prove, esecuzioni e backup,
compresi quelli senza `.git` e con nomi diversi dal progetto.

La nuova fonte di candidati è la cronologia delle sessioni letta tramite Mukade. Xiox collega
ogni candidato alle operazioni che lo hanno fatto emergere, verifica cosa esiste sul disco e
lo presenta con un motivo e un livello di rischio. La ricerca già esistente per Git e nome
continua a funzionare: questa specifica aggiunge una fonte, non sostituisce le altre.

## Decisioni dell'utente

- **Mukade trova e legge le sessioni. Xiox interpreta le operazioni e trova i residui.**
- La provenienza da una sessione si ricava esclusivamente dalla sua cronologia: messaggi,
  tool call, argomenti, esiti e contesto disponibile attraverso Mukade.
- Le date del filesystem non provano creazione, autore o appartenenza a un progetto. Non
  generano candidati e non confermano né annullano un'associazione ricavata dalla cronologia.
- Si elencano anche candidati senza Git e con evidenza debole, rendendo esplicita l'incertezza.
- `danger` è un avviso di rischio con conferma esplicita. La conferma può arrivare da una
  persona nella UI, dalla CLI o da un agente, anche per conto dell'utente.
- Non si cerca di certificare che il confermante sia umano. Non si promette che `danger`
  impedisca ogni rimozione di una cartella ancora in uso: chi conferma decide sul rischio mostrato.

La parte C della prima versione, basata sulle coincidenze temporali NTFS, è eliminata dalla
scoperta e dall'attribuzione.

## Responsabilità e codice

| Componente | Responsabilità |
|---|---|
| Mukade | Enumerare e leggere le sessioni dei diversi harness; restituire contenuto, metadati e record disponibili |
| `crates/xiox-mukade` | Adattare le API Mukade a Xiox, conservando identità e provenienza delle singole sessioni |
| `crates/xiox-project` | Interpretare comandi, patch e operazioni; estrarre percorsi; associare candidati; verificare Git, contenuti e rischio |
| CLI e UI Xiox | Mostrare motivi e limiti; raccogliere le scelte; eseguire la pulizia con i controlli del motore |

Si parte dalle API Mukade già esistenti, `Reader::list_all` e `Reader::read`, e dal modello
`Session` / `Entry`, compresi i record `raw` quando servono per ricostruire il contesto.
L'adapter deve esporre i descrittori prima dell'aggregazione per cartella: il record aggregato
oggi usato da Xiox non basta per leggere tutte le sessioni o contarne la provenienza.

Non si costruisce un secondo reader degli store nativi in Xiox. L'interpretazione delle
operazioni di filesystem resta invece in Xiox, anche quando usa campi dei record restituiti
da Mukade. Non si trasferiscono a Mukade categorie, rischio o logiche di pulizia.

Una modifica di Mukade non è un prerequisito funzionale. Un'eventuale API generica per lettura
progressiva o selettiva sarà valutata se le misure ne dimostrano la necessità; non deve
conoscere `scratch`, `danger`, cartelle residue o regole di appartenenza Xiox.

## Parte A: candidati dai percorsi di lavoro

1. Enumerare tramite Mukade le sessioni disponibili di tutti gli harness supportati,
   conservando agente, ID, sorgente, parentela e metadati disponibili. Uno store assente è
   diverso da uno store illeggibile; un descrittore può sopravvivere al suo transcript.
2. Raccogliere i `cwd` dei descrittori e gli ulteriori cambi di contesto osservati nelle
   sessioni lette nella parte B. Un listing con un solo `cwd` non dimostra tutti i percorsi
   usati durante la sessione.
3. Filtrare i percorsi prima di accedervi e risalire alle radici di repository esistenti con
   la logica di `xiox_scanner::project_roots_from_sessions`, applicando le guardie sotto.
4. Aggiungere le radici ai candidati della ricerca. `classify()` mantiene la verifica Git:
   commit radice condiviso oppure origin normalizzato comune. Questo identifica una relazione
   fra repository, non prova che una copia sia inutile; i controlli sul lavoro proprio restano.
5. Deduplicare con i risultati del walk, dell'indice MFT e dei worktree già noti.

La provenienza conserva le sessioni distinte e l'ultima attività osservata. Il conteggio non
si ricostruisce dai soli aggregati. Le radici non pertinenti al progetto non entrano nel suo
elenco; fonti saltate o limiti di copertura sono invece riportati.

## Parte B: operazioni della cronologia

Si leggono tramite Mukade le sessioni associate al progetto, ai suoi worktree e alle copie
verificate. Durante la lettura, l'associazione degli eventi segue il contesto effettivo del
turno o della chiamata: il `cwd` iniziale non attribuisce automaticamente tutta la sessione.
Se il contesto necessario a risolvere un percorso relativo manca, il percorso resta irrisolto.

La coda delle sessioni può ampliarsi quando viene verificata una nuova copia. Si deduplicano
sessioni e radici e si prosegue fino a esaurimento della coda o al budget della scansione.
Il raggiungimento di un limite è visibile. Le sessioni iniziate altrove e mai collegate al
progetto dai dati letti possono sfuggire: non si dichiara copertura universale.

### Evidenze e loro significato

| Evidenza nella cronologia | Uso in Xiox |
|---|---|
| `cwd`, percorso di lettura o ricerca | Candidato per la ricerca di repository; non dimostra creazione |
| Destinazione esplicita di creazione o copia | Candidato con operazione e riferimento alla sessione |
| Scrittura o modifica di un file | Associazione di lavoro; da sola non dimostra che la directory sia stata creata dalla sessione |
| Esito del tool associabile alla chiamata | Distingue successo, fallimento e risultato sconosciuto; il successo di una scrittura non implica directory nuova |
| Percorso citato in messaggi o risultati | Contesto o menzione debole; non viene promosso automaticamente a operazione eseguita |

Un'operazione fallita non è una creazione riuscita. Un esito mancante resta sconosciuto.
Una menzione può far cercare una copia Git; per proporre `scratch` deve esserci un riferimento
alla sua funzione di cartella di lavoro, copia, prova o destinazione di artefatti nella
cronologia. Il semplice elenco di percorsi in un prompt o nell'output di `ls` non basta.

La prima implementazione usa regole deterministiche sulle operazioni riconosciute. Messaggi
e risultati sono disponibili come contesto da consultare; non si introduce un LLM né si
considera una frase della chat prova automatica del successo di un comando.

### Operazioni riconosciute

- Tool di scrittura/modifica, con i loro parametri percorso e gli esiti disponibili.
- Patch: creazione, modifica, spostamento ed eliminazione di file mantengono significati
  distinti. Una modifica non diventa creazione; una cancellazione non genera una nuova cartella.
- Comandi espliciti: `mkdir`, `New-Item`, `cp -r`, `Copy-Item`, `robocopy`, `git clone`,
  `git worktree add`, `cargo new`/`init`, `npm create` e redirezioni supportate.

Xiox distingue sorgente e destinazione delle copie e applica la grammatica della shell
indicata dalla chiamata. Non esegue comandi letti dalla cronologia. Variabili, cicli,
sostituzioni e script non interpretati non producono percorsi inventati. Le forme non
supportate sono conteggiate; il comportamento di ciascun parser è provato su fixture.

Per ogni evidenza si conservano: percorso, progetto associato e motivo dell'associazione,
agente, ID sessione, ID/posizione dell'evento, timestamp se disponibile, `cwd` effettivo,
tipo di operazione, esito e forza dell'evidenza. Un timestamp assente non elimina da solo
l'evidenza. I riferimenti distinguono anche store diversi e pagine della stessa sessione.

## La categoria `scratch`

`scratch` è una cartella candidata alla revisione emersa dalle operazioni della sessione,
che non è già un worktree, una copia Git del progetto o una voce `named`. Può essere senza
Git. Il motivo distingue una creazione documentata da una destinazione di lavoro osservata
con evidenza più debole; non tutte le voci sono descritte come "create dalla sessione".

Il candidato è la directory indicata dall'evidenza, o il contenitore diretto di un artefatto
quando è quello individuabile. Non si sale automaticamente all'antenato più alto. Se una
sessione scrive in `D:\agent-runs\batch\phazly`, questo non attribuisce anche `batch` al progetto.

Non si propone come `scratch` una radice di disco, una cartella protetta o larga, il progetto
stesso, un suo worktree o un loro contenitore. Per un candidato dentro il progetto serve una
verifica Git del sottoalbero: contenuti tracciati, controllo impossibile o repository annidati
non rimovibili ne impediscono la rimozione. Serve una policy dedicata; non si allenta il guard
esistente di `ProjectFolder` per tutte le categorie.

Se la cronologia documenta che una cartella esisteva già, il motivo può indicare che vi si è
lavorato, ma non che sia stata creata. Le date NTFS non decidono questa distinzione.

### Esempio

Una sessione di Phazly registra la creazione di `D:\agent-runs\planner-benchmark-42` e la
scrittura di `benchmark.py`. Xiox verifica che la cartella esista, la propone come `scratch`
con livello `danger` e collega il motivo alla sessione e alle operazioni.

Una cartella accanto, creata alla stessa ora ma senza evidenze nella cronologia letta, non
viene attribuita a quella sessione. Una data NTFS diversa non scarta invece il primo candidato.

## Rischio, attività e conferma

Sono `danger` le voci senza Git, quelle con evidenza debole, quelle con copertura insufficiente
per valutare un rischio rilevante e quelle create/modificate o usate da sessioni nelle ultime
24 ore. Le date del filesystem possono contribuire a questo indicatore di recenza, mai alla
provenienza. L'attività recente osservata dentro un sottoalbero si riflette sul suo contenitore;
il solo `mtime` della directory non descrive tutte le modifiche dei figli.

Il testo è `danger: explicit confirmation required`, con motivi leggibili e strutturati.
Lo stato di attività osservato distingue attivo, non attivo verificato e sconosciuto.
Recenza e vitalità sconosciuta non sono nuovi blocchi assoluti: il rischio è mostrato e la
scelta resta a chi conferma. I blocchi già certi del motore, compresa una sessione verificata
viva, restano validi; questa feature non promette di riconoscere ogni attività in corso.

La conferma riguarda l'operazione, non l'identità di chi la fornisce:

- Una categoria non seleziona implicitamente voci `danger`.
- La CLI richiede `--include <path>` e una conferma per quella voce: risposta al prompt oppure
  `--confirm-danger <path>`. Il parametro è valido anche per un agente; `--yes` da solo non basta.
- La UI permette la selezione e conferma esplicita delle voci, con rischi e metodo di rimozione.
- Il percorso confermato deve essere un candidato dell'analisi. Non abilita la rimozione di
  percorsi arbitrari e non supera i blocchi del motore.
- La rimozione di un contenitore deve rispettare i blocchi e le conferme richieste anche dai
  candidati discendenti. Non è possibile eluderli includendo solo il genitore.
- Se la riconvalida cambia bersaglio, contenuti rilevanti, rischio o metodo rispetto al piano
  confermato, la voce viene saltata e richiede una nuova scelta.

**Cartelle ordinarie `danger`: Cestino.** Una richiesta `--permanent` che le includa viene
rifiutata prima della rimozione; non si cambia metodo in silenzio.

**Worktree: eccezione esplicita.** Si mantiene `git worktree remove`, mai `--force`, anche
quando il worktree è `danger`. La UI, il riepilogo CLI e il JSON dichiarano che la rimozione
è definitiva e non passa dal Cestino. La conferma include questa informazione. Questa è
un'eccezione documentata, non una promessa di ripristino dei worktree dal Cestino.

## Controlli e copertura

1. **Percorsi:** scarto lessicale di UNC, dispositivi, percorsi non assoluti, componenti
   vietate e forme non supportate prima dell'accesso. Accesso solo a dischi locali fissi;
   controllo degli antenati dall'alto prima di attraversarli, senza seguire link/junction o
   accedere a cartelle cloud. Le destinazioni indirette di Git (`gitdir`, directory comuni,
   object store alternativi e configurazioni esterne) richiedono la stessa policy prima di
   interrogare il repository; un riferimento non validabile impedisce quella verifica.
2. **Stato attuale:** esistenza, identità, contenuti Git e blocchi vengono ricontrollati prima
   della rimozione. Il riferimento storico non autorizza automaticamente un altro oggetto
   comparso in seguito allo stesso percorso.
3. **Contenuti:** lavoro Git non salvato, repository illeggibili, cartelle protette e scansioni
   del sottoalbero incomplete impediscono la rimozione secondo la policy applicabile. Non si
   conclude che una persona abbia modificato un file dal confronto con la fine di una sessione.
4. **Condivisione:** scritture di altri progetti documentate dalla cronologia sono riportate
   e bloccano la pulizia della cartella condivisa come residuo esclusivo di un progetto.
   Xiox può leggere tramite Mukade altre sessioni per cercare conflitti sui candidati, entro
   il budget dichiarato. Non leggere tutte le sessioni non dimostra assenza di conflitti:
   la copertura parziale è indicata come rischio, senza pretendere un inventario storico completo.
5. **Errori:** sorgente assente, transcript eliminato, lettura fallita, schema non supportato
   e limite raggiunto restano distinti. Non generano nuove prove; i candidati già verificabili
   possono essere mostrati con copertura e rischio. Non si riutilizza una prova obsoleta come
   se fosse stata appena riconfermata.
6. **Categorie:** i controlli dipendono da contenuti ed evidenze, non solo dall'etichetta.
   Diventare `named` invece di `scratch` non elimina una restrizione applicabile.

`--no-drives` conserva il limite della ricerca esterna al genitore del progetto anche per i
candidati ricavati dalle sessioni. I percorsi esterni esclusi sono conteggiati senza accedervi.
La lettura degli store Mukade configurati rimane separata dall'ambito della ricerca di cartelle.

## Lettura, cache e limiti

La prima versione usa i reader Mukade esistenti entro budget finiti di sorgenti, dimensioni,
memoria e durata. Si legge una sessione alla volta e la si libera dopo l'estrazione. Le
operazioni non interrompibili devono essere contenute da un limite effettivo o escluse con
una nota: un timeout solo nominale non basta. La prova tecnica determina limiti iniziali e
modalità di isolamento necessarie, senza richiedere in anticipo una riscrittura di Mukade.

La cache dei listing esistente resta distinta dalle prove estratte. Nella prima versione le
prove sono riutilizzate nella singola analisi, con deduplicazione; non si introduce la ripresa
persistente basata su primo KiB e offset. Una successiva cache persistente deve legarsi a una
versione verificata dell'intera sorgente e dell'estrattore, compreso il contesto di ripresa.
SQLite, file compressi, pagine e rotazioni richiedono contratti specifici: non ereditano
automaticamente la semantica append-only di un JSONL.

La pulizia rilegge tramite Mukade le evidenze rilevanti e rivaluta il candidato. La sola
coincidenza di percorso, dimensione o prefisso del transcript non certifica l'attualità.

Si definiscono limiti anche per numero di eventi, percorsi estratti, profondità di parsing,
scansione dei sottoalberi e interrogazioni Git. I candidati sono ordinati per attività
osservata, con ordinamento stabile per quelli senza data. Ogni troncamento è riportato.

Xiox non modifica i dati delle sessioni, non esegue migrazioni o pulizie degli store e non
scrive SQL applicativo. I reader SQLite sono usati in sola lettura con transazioni brevi;
si verifica la loro gestione di WAL e sidecar. Non si confonde read-only con l'assenza di
qualsiasi effetto fisico dei file ausiliari e non si dichiara immutabile un database attivo.

Cache ed elenco standard contengono evidenze sintetiche e riferimenti, non copie delle chat,
script, comandi o output completi. Un dettaglio locale richiesto esplicitamente può recuperare
il contesto tramite Mukade. I testi degli errori non devono riversare contenuto delle sessioni.

## Verifiche prima e durante la costruzione

I test usano repository e store finti sotto `tempfile::tempdir()`. `cargo` gira con `-j 1`;
non si avviano build concorrenti. Non si eseguono pulizie su progetti reali.

Si aggiunge una sonda di sola analisi, `session_probe`, che usa le stesse API Mukade. La modalità
predefinita restituisce conteggi, copertura e tempi; una modalità locale `--sample` mostra
percorsi, tipo di evidenza e riferimenti alle sessioni, così il campione è verificabile.
Non si esporta automaticamente il testo delle conversazioni. La sonda non scrive la cache
Xiox; il contratto dei reader SQLite è quello descritto sopra.

Le misure su store reali vengono eseguite dall'utente o su sua richiesta esplicita. Non sono
un prerequisito per implementare e verificare le parti su fixture. Si misurano per harness e
tipo di evidenza: copertura rispetto alla ricerca attuale, costo della lettura, falsi
collegamenti, percorsi non risolti e fonti saltate. Il campione include casi negativi e
sessioni concorrenti. I risultati servono a tarare supporto e limiti, non a trasformare una
"maggioranza di residui" in una garanzia di cancellazione sicura.

| Caso di accettazione | Risultato richiesto |
|---|---|
| Copia profonda presente nel `cwd` di una sessione | Candidata aggiunta e relazione Git verificata |
| Tool crea una cartella fuori dal progetto | Evidenza con sessione/operazione; `scratch` se pertinente |
| Cartella coeva senza evidenze nella cronologia | Nessuna attribuzione alla sessione tramite la data |
| Timestamp NTFS diverso dalla cronologia | Non invalida da solo il candidato |
| Scrittura in directory preesistente; creazione fallita | Il motivo non afferma una creazione riuscita |
| Cambio `cwd`, percorsi relativi, pagine, sessioni figlie | Contesto e provenienza conservati, oppure limite esplicito |
| Agente conferma una voce `danger` alla CLI | Conferma accettata se piano e controlli lo consentono |
| Solo `--yes`, oppure conferma di un altro percorso | Nessuna rimozione della voce `danger` |
| Attività recente con vitalità sconosciuta | `danger` e motivazione; nessuna promessa di inattività |
| Contenitore con discendente bloccato o non confermato | Restrizioni non aggirabili dal contenitore |
| Worktree `danger` confermato | Metodo Git definitivo esplicito; mai `--force` |
| Sorgente riscritta con stesso prefisso | Nessun riuso delle vecchie prove senza validazione |
| Lettura enorme, parziale o fallita | Limite rispettato, risultato parziale dichiarato |
| UNC, junction, riferimento Git esterno vietato | Nessun attraversamento del bersaglio vietato |
| Cartella condivisa documentata da due progetti | Conflitto visibile e blocco applicato |

## Ordine di implementazione

1. **Adapter e prova tecnica:** esporre sessioni non aggregate e leggere fixture attraverso
   Mukade; misurare costi e limiti. Preparare la sonda locale.
2. **Parte A:** aggiungere candidati dai `cwd`, deduplicazione e provenienza.
3. **Analisi Xiox della cronologia:** operazioni riconosciute, contesto, evidenze e `scratch`;
   includere rischio e copertura nel risultato di `analyze`.
4. **Selezione e pulizia:** `danger`, conferme CLI/UI, propagazione ai contenitori, riconvalida
   ed eccezione esplicita dei worktree; incrementare lo schema JSON e aggiornare le guide.
5. **Verifica integrata:** casi della tabella, osservatore e prove Windows controllate. Solo
   dopo le misure valutare ottimizzazioni o estensioni generiche della lettura Mukade.

Il piano di implementazione deve assegnare i test insieme alle rispettive funzionalità.
Questa specifica non avvia la costruzione né modifica il codice applicativo.

## Limiti dichiarati

- Cronologie eliminate, store non supportati e sessioni non collegate al progetto riducono
  la copertura; una prova mai registrata o non leggibile non viene ricostruita dalle date.
- Uno script può creare percorsi che non compaiono nei dati leggibili della sessione.
- Una menzione o una scrittura non dimostrano che il contenuto sia inutile.
- La conferma può essere automatizzata da un agente. `danger` rende esplicita la decisione,
  non certifica un intervento umano o l'assenza di attività concorrente.
- L'osservatore segnala ciò che compare mentre è in funzione; non sostituisce automaticamente
  la provenienza delle sessioni né costituisce, da solo, un archivio storico persistente.
