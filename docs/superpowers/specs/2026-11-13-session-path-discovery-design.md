# Copie e cartelle di prova scoperte dalle sessioni degli agenti

Data: 2026-11-13. Revisione 3 (2026-11-15): estrattore dei comandi misurato sulla cronologia reale;
deciso l'approccio deterministico con evidenze di sessione, senza modello. Implementazione da pianificare.

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

L'estrazione è deterministica. Un modello locale è stato misurato e scartato (vedi
[Estrazione dai comandi: decisione e misure](#estrazione-dai-comandi-decisione-e-misure)).
Messaggi e risultati sono disponibili come contesto da consultare; una frase della chat non
è prova automatica del successo di un comando.

### Operazioni riconosciute

- Tool di scrittura/modifica, con i loro parametri percorso e gli esiti disponibili. I campi
  strutturati (`file_path`, `path`, `notebook_path`) si leggono direttamente.
- Patch: creazione, modifica, spostamento ed eliminazione di file mantengono significati
  distinti. Una modifica non diventa creazione; una cancellazione non genera una nuova cartella.
- Comandi espliciti: `mkdir`/`md`, `New-Item`, `cp`, `Copy-Item`, `robocopy`, `xcopy`, `mv`,
  `Move-Item`, `git clone`, `git worktree add`, `git init <dir>`, `cargo new`/`init`,
  `npm`/`pnpm`/`yarn`/`bun create`, `npx create-*`, `uv init`/`venv`, `python -m venv`,
  `dotnet new -o`, estrazioni (`tar -x -C`, `unzip -d`, `Expand-Archive`), scritture (`>`,
  `>>`, `tee`, `Out-File`, `Set-Content`, `Add-Content`, `touch`, `sed -i`) e directory di
  output indicate da flag (`--out`, `--output`, `--out-dir`, `--target-dir`, `--basetemp`).
- Contesto dentro il comando: `cd`, `pushd`/`popd`, `Set-Location`, `cd /d`, `git -C`;
  variabili assegnate con un valore letterale nello stesso comando; cicli `for` su liste
  letterali, espansi; shell annidate (`bash -c`, `powershell -Command`, `cmd /c`); heredoc.
- Script inline Python e JavaScript (`-c`, `-e`, heredoc, script scritto da un heredoc ed
  eseguito nello stesso comando): solo chiamate note con un percorso letterale, come
  `open(…, "w")`, `os.makedirs`, `Path(…).mkdir`, `shutil.copytree`, `writeFileSync`, `mkdirSync`.

Xiox distingue sorgente e destinazione delle copie e applica la grammatica della shell
indicata dalla chiamata. Non esegue comandi letti dalla cronologia. Variabili d'ambiente o
non assegnate, sostituzioni di comando, cicli su glob o output, f-string, template e
concatenazioni non producono percorsi: restano irrisolti e sono conteggiati. Il comportamento
di ciascun parser è provato su fixture.

### Estrazione dai comandi: decisione e misure

**Decisione.** Parser deterministici per comando e dialetto, più un registro delle evidenze
per sessione. Nessun modello generativo: sui comandi reali inventa percorsi più di quanti ne
trovi. Un estrattore di span non generativo (tipo GLiNER) non è stato misurato; si valuta solo
se un nuovo campione mostra un buco ricorrente nei corpi degli script.

**Formato.** Per ogni chiamata shell l'estrattore restituisce, in ordine:

- operazioni `{op, src, dst}` con `op` tra `create`, `copy`, `move`, `clone`, `worktree`,
  `write`, e `cd` solo come contesto; `base` per `git -C`;
- i segnali di incertezza `loop` e `script`.

Un resolver comune trasforma i percorsi così come sono scritti in percorsi assoluti:

- tiene conto di `cd` e pushd, `base`, forme Git Bash (`/d/x`, `/tmp`, `~`) e dispositivi
  nulli (`/dev/null`, `nul`, `$null`), scartati;
- un `clone` senza destinazione va nella cwd col nome del repository;
- conta i percorsi irrisolti, che non diventano mai candidati.

Sessione, progetto, esito e riferimento all'evento li collega Xiox, non l'estrattore.

**Evidenze di sessione.** Il registro della sessione riunisce per cartella, senza distinguere
maiuscole e minuscole, le evidenze di tutte le chiamate:

| Fonte | Forza |
|---|---|
| Destinazione di `create`, `copy`, `move`, `clone`, `worktree` | creata |
| File scritto (`write`, Write/Edit strutturati) | scritta, per la cartella del file |
| Read, Glob, Grep, `cd`, cwd della chiamata | usata |

Per ogni cartella si conservano la forza più alta e i riferimenti agli eventi. Un'operazione
fallita resta visibile ma non porta forza. Il motivo mostrato all'utente usa la forza: una
cartella solo "usata" non viene descritta come creata dalla sessione. Le evidenze ricavate da
uno script sono indicate come tali. Un ultimo segmento con estensione è trattato come file:
è un'approssimazione dichiarata, da rivedere quando il risultato del tool indica il tipo.

**Misure (2026-11-15, su richiesta esplicita dell'utente).** Lettura tramite Mukade (`list`,
`dump --stream`) di tutti gli store presenti:

- 8 store, 6 harness; 1.848 sessioni lette su 1.905 leggibili; 157.214 chiamate di tool;
- 94.862 comandi shell con cwd: Claude Code 83.270, Pi 6.077, OpenCode 4.726, Codex 786,
  Hermes 3;
- sessioni non lette: 50 Pi, che Mukade elenca tutte con lo stesso id `session`, e 7
  Claude Code con catene di subagent ripresi o parent mancante.

L'OpenCode dell'utente è stato letto da una copia del database.

Le etichette sono state scritte alla cieca da tre agenti indipendenti, che vedevano solo
comando, shell e cwd. Il campione di sviluppo (180 comandi) è servito a scrivere le regole. Il
campione di verifica (180 comandi, seed diverso, nessuna sovrapposizione) dà i numeri seguenti.

| Campione di verifica | Solo comando | Con evidenze di sessione |
|---|---|---|
| Percorsi inventati | 0 | 0 |
| Percorsi esatti per comando | 82/157 | — |
| Comandi con tutte le cartelle esterne trovate | 34/48 (71%) | 41/48 (85%) |
| Cartelle esterne distinte trovate | 46/85 (54%) | 54/85 (64%) |

Sulle cartelle distinte pesa un solo script che ne genera 22 con un ciclo su un dizionario:
senza di esso la sessione ne trova circa l'86%. Sul campione di sviluppo gli stessi numeri
erano 98–99%: la differenza indica quanto le regole siano tarate sui casi visti. Il prototipo
in Python analizza i 94.862 comandi in circa 75 s, con un massimo di 104 ms per comando. Sulle
106 fixture sintetiche: 105 esatte, 0 percorsi inventati.

Modelli misurati su CPU (Intel Core Ultra 7 155H, 4 thread, llama.cpp b12024, uscita vincolata
da JSON schema, guardia che accetta solo percorsi presenti nel comando), sul campione di
sviluppo reale:

| Modello | Trovati / 149 | Inventati | In aggiunta al deterministico | RAM di picco | Latenza mediana |
|---|---|---|---|---|---|
| SmolLM2-135M-Instruct Q8_0 | 3 | 75 | +0 trovati, +29 inventati | ~300 MB | ~1,0 s |
| LFM2.5-350M Q4_K_M | 4 | 266 | +1 trovato, +117 inventati | ~520 MB | ~3,5 s |

Le latenze sono indicative: parte delle prove ha condiviso la CPU con altre misure.

**Comportamento quando l'estrazione è incerta.** Nessun percorso irrisolto diventa candidato.
I segnali `loop` e `script` e i percorsi irrisolti sono conteggiati nella copertura. Una
cartella può comunque emergere da altre evidenze della stessa sessione, con la forza che
quelle evidenze consentono.

**Lacune note, da misurare su un nuovo campione prima di colmarle:**

1. Directory di output passate per variabile d'ambiente (`CARGO_TARGET_DIR=…`).
2. Comandi PowerShell eseguiti dal tool Bash.
3. Altri flag di output (`--user-data-dir`, `--local-dir`, `--screenshot`).
4. `ForEach-Object` di PowerShell su liste letterali.

I percorsi calcolati negli script (cicli su dizionari, argomenti di funzione) restano fuori
per scelta.

**Fixture.** I casi sintetici etichettati sono la base dei test guidati dai dati
dell'implementazione. I comandi reali delle sessioni non entrano nel repository.

### Struttura del codice

L'implementazione in `crates/xiox-project` segue stadi puri e piccoli tipi di valore:

1. **Lessico per dialetto:** un trait `Dialect`, con implementazioni Bash, PowerShell e Cmd per
   virgolette, escape, operatori e heredoc.
2. **Struttura del comando:** comandi semplici, assegnazioni, cicli e heredoc al posto di liste
   di token.
3. **Registro di riconoscitori:** un trait `Recognizer` per famiglia di comandi (directory,
   copie, Git, archivi, package manager, scritture, shell annidate, interpreti). Un comando
   nuovo si aggiunge con un riconoscitore nuovo.
4. **Analizzatori di script:** una tabella dichiarativa di chiamate per linguaggio; argomenti
   letti in un solo passaggio lineare, senza espressioni regolari con backtracking.
5. **`Resolver` ed `EvidenceLedger`:** risoluzione dei percorsi con il contesto; registro per
   sessione con forza ordinata (usata < scritta < creata).

I test usano le fixture sintetiche e una tabella per caso.

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
3. **Analisi Xiox della cronologia:** operazioni riconosciute, contesto, registro delle evidenze
   di sessione e `scratch`, con la struttura del codice descritta sopra; includere rischio e
   copertura nel risultato di `analyze`.
4. **Selezione e pulizia:** `danger`, conferme CLI/UI, propagazione ai contenitori, riconvalida
   ed eccezione esplicita dei worktree; incrementare lo schema JSON e aggiornare le guide.
5. **Verifica integrata:** casi della tabella, osservatore e prove Windows controllate. Solo
   dopo le misure valutare ottimizzazioni o estensioni generiche della lettura Mukade.

Il piano di implementazione deve assegnare i test insieme alle rispettive funzionalità.
Questa specifica non avvia la costruzione né modifica il codice applicativo.

## Limiti dichiarati

- Cronologie eliminate, store non supportati e sessioni non collegate al progetto riducono
  la copertura; una prova mai registrata o non leggibile non viene ricostruita dalle date.
- Uno script può creare percorsi che non compaiono nei dati leggibili della sessione. Sul
  campione di verifica il registro di sessione trova tutte le cartelle esterne nell'85% dei
  comandi che ne hanno; il resto è soprattutto percorsi calcolati dentro gli script.
- Mukade non legge tutte le sessioni: al 2026-11-15 restano fuori 50 sessioni Pi con id
  ambiguo e 7 di Claude Code con catene di subagent che Mukade rifiuta.
- Una menzione o una scrittura non dimostrano che il contenuto sia inutile.
- La conferma può essere automatizzata da un agente. `danger` rende esplicita la decisione,
  non certifica un intervento umano o l'assenza di attività concorrente.
- L'osservatore segnala ciò che compare mentre è in funzione; non sostituisce automaticamente
  la provenienza delle sessioni né costituisce, da solo, un archivio storico persistente.
