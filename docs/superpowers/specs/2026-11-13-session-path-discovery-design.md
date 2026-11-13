# Copie e cartelle di prova scoperte dalle sessioni degli agenti

Data: 2026-11-13. Stato: da approvare.

Dipende da `2026-11-13-project-cleanup-cli-design.md` (analisi mirata, `xiox analyze` / `xiox clean`).
Ricerca di base: `reports/Estrazione path da transcript agenti.md`.

## Perché

Chi lavora con molti agenti insieme, senza controllarli, si ritrova con due tipi di residui che
oggi Xiox non vede:

- **cloni e copie del repository** in posti che non ricorda (a sei livelli di profondità, su un
  altro disco, con un nome qualsiasi): il walk di oggi arriva a 3 livelli;
- **cartelle create senza criterio dalle sessioni**: benchmark, prove, esecuzioni, backup, copie
  fatte con `cp -r`. Spesso senza `.git` e con un nome che non c'entra con il progetto.

Le sessioni degli agenti sanno dove gli agenti hanno lavorato e cosa hanno creato. Xiox le
elenca già, ma usa solo la cartella di lavoro (`cwd`), e solo per attribuire le sessioni al
progetto.

Lo stesso profilo d'uso rende la pulizia più pericolosa: agenti accesi mentre Xiox gira, lavoro
mai committato, e spesso è un agente a lanciare `xiox clean --yes`. Più potenza nella scoperta
va insieme a più guardie.

## I due principi

**Per le copie: una menzione propone, l'identità git decide.** Un percorso trovato in una
sessione è un candidato in più per `classify()`. È una copia solo se ha lo stesso commit radice
o lo stesso origin.

**Per le cartelle senza identità: Xiox le mostra, la persona le controlla.** Dove non c'è git a
decidere, la voce entra in elenco con il livello nuovo `danger` («da controllare a mano») e con
la prova scritta: quale sessione, quale comando, quando. Non si toglie senza un gesto in più
(sezione «Il livello `danger`»).

## Decisioni dell'utente

- Vanno trovate anche le cartelle create dalle sessioni **senza git**: benchmark, test, prove,
  esecuzioni, copie, backup.
- Quelle identificate con prova debole **si elencano**, segnate come pericolose e da controllare
  a mano. Stesso segno per le cartelle **senza git** e per quelle **più recenti di 24 ore**.

## Cosa si costruisce

| Parte | Cosa legge | Cosa trova | Attiva |
|---|---|---|---|
| A. `cwd` | La cartella di lavoro di ogni sessione, di ogni agente (già in memoria) | Copie del repository ovunque un agente abbia lavorato | Sempre |
| B. Chiamate ai tool | L'input di ogni chiamata ai tool, nelle sessioni del progetto e delle sue copie: percorsi dei tool di scrittura e lettura, comandi shell, patch | Altre copie; cartelle create dalle sessioni | Sempre, con cache; la prima lettura può essere lenta |
| C. Data di creazione | La data di creazione NTFS delle cartelle, confrontata con inizio e fine delle sessioni | Conferma di B; cartelle nate durante una sessione e non nominate da nessun comando | Sempre |

La prosa della chat (messaggi, output dei tool, prompt di sistema) non si legge: quello che un
agente crea passa sempre da una chiamata a un tool.

## Fase 0: verifiche su Windows, prima di scrivere il resto

Il contenitore di sviluppo è Linux e la ricerca è fatta su documenti: quello che conta va
misurato sul PC vero. Si costruisce per prima una **sonda in sola lettura**
(`cargo run -p xiox-project --example session_probe -- <progetto>`), che stampa solo conteggi e
tempi: nessun percorso, nessun testo di sessione, così l'uscita si può incollare in una chat. La
lancia l'utente, a mano. Non scrive niente, nemmeno la cache.

| # | Verifica | Cosa decide |
|---|---|---|
| V1 | Per agente: sessioni, `cwd` distinti, quanti esistono ancora, radici di repository distinte | Quanto rende la parte A |
| V2 | Quante di quelle radici il walk di oggi **non** raggiunge | Il guadagno della parte A sulle copie |
| V3 | Forme dei percorsi visti: `C:\`, `c:\`, `C:/`, `\\?\`, UNC, `\\wsl.localhost\`, `/c/`, `/mnt/c/`, POSIX; con spazi, non ASCII, oltre 260 caratteri | Quali forme normalizzare e quali scartare |
| V4 | Per agente: file, GB, file più grande, data della sessione più vecchia, presenza di `.jsonl.zst` | Costo della parte B; quanto indietro vedono le sessioni |
| V5 | Nomi dei campi realmente presenti: chiavi dei tool di Claude Code, forma di `arguments` di Codex, tabelle e colonne di `opencode.db`, header di Pi | La parte B si scrive sui nomi veri |
| V6 | Tempo per leggere tutti i transcript del progetto, a freddo e a caldo, con Defender attivo | Se serve un avviso di attesa o un parametro per saltare la parte B |
| V7 | Leggere un transcript mentre l'agente ci scrive, e `opencode.db` mentre OpenCode è acceso: l'agente continua a scrivere, rinominare, troncare? | Modo di apertura dei file |
| V8 | Tempo di una chiamata `git.exe`; quante volte git risponde `dubious ownership`, da terminale normale ed elevato | Il tetto ai candidati per esecuzione |
| V9 | Quanti percorsi cadono in cartelle cloud o su unità non fisse | Conferma dei filtri |
| V10 | **Per tipo di prova** (tool di scrittura, `mkdir`, `cp`, `git clone`, redirezione, solo data): quante cartelle escono, quante esistono ancora, quante hanno la data di creazione dentro la sessione | Quali comandi vale la pena interpretare; quanto filtra la parte C |
| V11 | La data di creazione NTFS è affidabile? Dopo una copia, uno spostamento tra dischi, un ripristino, un `git clone` | Se la parte C può fare da conferma o solo da indizio |
| V12 | L'utente guarda a mano un campione delle cartelle uscite da V10: quante sono davvero residui, quante no | **Il tasso di falsi positivi.** Decide cosa entra in elenco |

Criteri: la parte A si costruisce se V2 > 0 su almeno un progetto reale. La parte B e la
categoria `scratch` si costruiscono se in V12 la maggioranza netta del campione è fatta di
residui veri; i tipi di prova che in V12 danno soprattutto falsi positivi non si costruiscono.

## Parte A: i `cwd` di tutte le sessioni

1. Si raccolgono i `cwd` di tutte le sessioni (vive e archiviate, tutti gli agenti), con agente
   e data dell'ultima attività.
2. Ogni `cwd` passa il filtro dei percorsi (G1) prima di qualunque accesso al disco.
3. Si risale alla radice del repository con la logica di
   `xiox_scanner::project_roots_from_sessions` (cartella mancante, cloud, cartelle «larghe»).
4. Le radici si aggiungono ai candidati di `search()`; `classify()` decide. Le radici che non
   sono copie del progetto si scartano in silenzio.

Ogni copia porta la **provenienza** (agenti, numero di sessioni, data dell'ultima), in
`Why::SameRepository` e nella riga stampata. `SCHEMA` sale di uno.

## Parte B: le chiamate ai tool

Si leggono le sessioni **del progetto** (il cui `cwd` è nel progetto, in un suo worktree o in
una sua copia). Da ogni chiamata si ricava una **prova**: percorso, tipo, sessione, istante.

| Tipo di prova | Da dove | Forza |
|---|---|---|
| Scrittura | `file_path`/`path` dei tool che scrivono; `modifiedFiles` di Pi; righe `*** Add File:` di `apply_patch` | Forte |
| Creazione da comando | `mkdir`, `New-Item`, `cp -r`, `Copy-Item`, `robocopy`, `git clone`, `git worktree add`, `cargo new`/`init`, `npm create`, redirezioni `>` | Forte se il percorso è scritto per intero |
| Lettura | tool che leggono o cercano | Solo per trovare copie del repository; non crea voci `scratch` |

| Agente | Dove stanno | Note |
|---|---|---|
| Pi | header, `toolCall.arguments`, `details.readFiles`/`modifiedFiles` | Il più pulito |
| Codex | `turn_context.payload.cwd`, `function_call.arguments` | `arguments` è JSON in una stringa: due decodifiche. `workdir` relativo al `cwd`. Rollout vecchi in `.jsonl.zst` |
| Claude Code | `cwd` di ogni record, input dei `tool_use` | Formato non documentato: nomi da V5 |
| OpenCode | `session.directory`, `project.worktree`, parti dei tool | Colonne controllate a runtime; se mancano, si salta con una nota |

Comandi shell: si spezzano in parole con le regole della shell (bash e PowerShell), si
riconosce solo l'elenco di comandi qui sopra, i percorsi relativi si risolvono sul `cwd` della
chiamata. Variabili, sostituzioni di comando e cicli non si espandono: un comando che non si
capisce per intero non dà nessuna prova.

Lettura dei file: aperti con condivisione completa, letti a blocchi, **mai mmap**; righe separate
con `memchr`, interpretate solo se contengono una delle chiavi cercate; la riga incompleta in
fondo si lascia alla prossima lettura; una riga che non si interpreta si salta.

Cache in `xiox_app::cache_dir()`, per file: dimensione, data, impronta del primo KiB, offset
dopo l'ultima riga completa, prove estratte. File invariato: si salta. Cresciuto con la stessa
impronta: si riprende dall'offset. **La cache contiene percorsi, tipo di prova, sessione e
istante: mai il testo dei comandi o delle chat.** Ha un numero di versione.

## Parte C: la data di creazione

Di ogni sessione si conoscono inizio e fine (primo e ultimo record). Di ogni cartella NTFS dà
la data di creazione.

- **Conferma.** Una cartella con una prova della parte B entra in elenco solo se è stata creata
  durante quella sessione (con un margine). Se esisteva già, la sessione ci ha solo scritto
  dentro: non è sua e resta fuori. È il filtro principale contro i falsi positivi.
- **Recupero.** Una cartella nata durante una sessione del progetto, senza nessuna prova della
  parte B, entra in elenco solo se è **accanto** al progetto (figlia della stessa cartella
  madre) oppure è una cartella non tracciata e non ignorata **dentro** il progetto. Copre lo
  script che crea da solo le sue cartelle. È la prova più debole.

## La categoria `scratch`

Voce nuova dell'elenco: una cartella che una sessione del progetto ha creato e che non è una
copia del repository (quelle restano `copies`) né ha il nome del progetto (quelle restano
`named`). La voce è la cartella creata più in alto: se la sessione ha creato `D:\runs\a\b`, la
voce è la prima di quelle tre che è nata durante la sessione.

Ogni voce porta il motivo (`Why::SessionMade`): tipo di prova, agente, sessione, data, e se la
data di creazione conferma.

Non è mai una voce `scratch`:

- una cartella «larga» o protetta (radice di disco, home, Desktop, Documenti, cartelle di
  sistema, `AppData`), o una cartella che contiene il progetto o un suo worktree;
- una cartella che esisteva prima della sessione;
- una cartella dentro il progetto che git traccia.

## Il livello `danger`

Livello nuovo sopra `caution`. Nell'elenco è marcato in modo evidente
(`danger  check by hand`) con il motivo accanto. Sono `danger`:

- ogni voce `scratch` o `named` **senza git** (nessun repository che ne provi l'identità);
- ogni voce `scratch` trovata con la sola data di creazione;
- ogni voce `worktrees`, `copies`, `named` o `scratch` **creata o modificata nelle ultime 24
  ore**, o in cui una sessione ha lavorato nelle ultime 24 ore.

Come si toglie una voce `danger`:

- mai con un parametro di categoria;
- solo nominandola con `--include <path>`;
- **mai con il solo `--yes`**: serve la conferma a mano, voce per voce, da terminale. Senza un
  terminale serve `--confirm-danger <path>` ripetuto per ogni voce. Un agente che lancia
  `xiox clean --yes --include …` su una voce `danger` non cancella niente ed esce con errore;
- sempre nel Cestino: `--permanent` non vale per le voci `danger`.

## Guardie

**G1. Filtro dei percorsi, prima di toccare il disco.** Il contenuto di un transcript non è
fidato. Un percorso si accetta solo se: è assoluto con lettera di unità su un disco fisso;
**non è UNC né un percorso di dispositivo** (solo guardare se esiste un percorso di rete fa
partire un'autenticazione verso quel server); non ha caratteri di controllo o vietati; non
supera una lunghezza massima; non è in una cartella cloud; **nessun antenato è un link o una
junction**. Il resto si scarta e si conta in una nota.

**G2. Sessione viva.** Resta il blocco di oggi (sessione verificata viva: non si toglie). Per
gli agenti la cui vitalità non è verificabile (Codex, OpenCode, Pi) oggi non c'è niente: una
copia in cui Codex sta lavorando adesso non è bloccata. Con questa spec l'attività nelle ultime
24 ore porta la voce a `danger`.

**G3. Blocchi delle voci senza git.** Una voce `scratch` è bloccata (non si toglie nemmeno con
la conferma) se:

- contiene un file modificato **dopo** la fine dell'ultima sessione che l'ha toccata e fuori da
  ogni altra sessione: ci ha messo mano una persona;
- contiene un repository git con lavoro non salvato, o che git non riesce a leggere (regola di
  oggi per `named`);
- sessioni di **un altro progetto** ci hanno scritto: non è solo di questo progetto;
- è troppo grande per essere controllata dentro.

**G4. Tetto e ordine.** I candidati si controllano dal più recente fino a un tetto (da V8).
Oltre: nota, mai un silenzio.

**G5. Le sessioni non lette non allargano niente.** Se la lettura fallisce, l'analisi prosegue
senza quelle voci e lo dice.

Non cambia: riconvalida di ogni voce subito prima di cancellare (comprese data di creazione e
G3), nessun link seguito, cartelle protette rifiutate.

## Decisioni mie (l'utente non le ha discusse: da controllare)

- **`danger` è un livello nuovo e chiede la conferma a mano**, non un semplice testo di avviso:
  altrimenti un agente con `--yes` lo salta. È un cambio di contratto per chi usa la riga di
  comando (`--confirm-danger`, `SCHEMA`).
- **Le voci recenti (24 ore) passano a `danger` anche se trovate dal walk di oggi**: una copia
  oggi `caution` e toglibile con `--include --yes` non lo sarà più senza conferma.
- **Solo le sessioni del progetto e delle sue copie.** Le cartelle create da sessioni aperte
  nella home, sul Desktop o in cartelle di prova non appartengono a nessun progetto e qui non
  si vedono. Servirebbe una vista non legata a un progetto, che la spec precedente esclude: la
  propongo come spec a parte, da costruire sopra questa.
- **La prosa della chat non si legge**, nemmeno per le voci `danger`.
- **Dove sta il codice della parte B.** `vendor/mukade` non si tocca. Propongo
  `crates/xiox-mukade`, e di portarlo a monte in mukade dopo.
- **Dipendenze nuove**: `memchr`, `zstd`, un crate per spezzare i comandi shell (da scegliere);
  `typed-path` solo nei test.
- **Elenco chiuso di comandi riconosciuti.** Niente tentativo di capire script o comandi
  composti: meno voci trovate, ma nessuna inventata.

## Vincoli

- Quelli della spec dell'analisi mirata: testi in inglese, `analyze` non scrive niente tranne la
  cache, mai provare sui progetti veri (la sonda è l'unica eccezione: la lancia l'utente e
  stampa solo conteggi), `cargo -j 1`.
- Niente viene scritto negli store degli agenti. `opencode.db` si apre in sola lettura.
- I test costruiscono store finti per i quattro agenti sotto `tempfile::tempdir()`.

## Non si fa

- Niente lettura di messaggi, output dei tool, prompt di sistema.
- Niente ricerca per contenuto dei file.
- Niente vista di tutti i progetti, niente sessioni senza progetto (spec a parte).
- Niente percorsi WSL o di rete.
- Niente cancellazione definitiva delle voci `danger`.

## Sarebbe un fallimento se

- una cartella che esisteva prima della sessione venisse elencata come creata dalla sessione;
- una voce `danger` venisse tolta senza che una persona l'abbia confermata, una per una;
- una cartella in cui una persona ha lavorato dopo la sessione venisse tolta;
- una copia o una cartella in cui un agente sta lavorando venisse tolta;
- un percorso preso da una sessione facesse toccare a Xiox la rete, un link o una cartella cloud;
- un transcript illeggibile, enorme o scritto apposta facesse fallire o bloccare `analyze`;
- la lettura di Xiox facesse fallire una scrittura dell'agente;
- testo di una sessione finisse nella cache, nell'uscita o nel JSON.

## Limiti noti

- Claude Code cancella i transcript dopo 30 giorni (impostazione predefinita): per le cartelle
  più vecchie la prova non c'è più. Per quelle future c'è `xiox observe`.
- Uno script che crea cartelle lontano dal progetto non lascia prova: non si trova.
- La data di creazione può non essere quella vera dopo uno spostamento o un ripristino (V11).

## Ordine di costruzione

1. **Sonda** (fase 0). L'utente la lancia su Windows e riporta i numeri, compreso il campione
   guardato a mano (V12).
2. **Livello `danger`** e guardie G1 e G2, con i loro test: utili anche senza il resto.
3. **Parte A** con la provenienza.
4. **Parte B** per i soli tipi di prova che V12 ha promosso, con la parte C e la categoria
   `scratch`.

## Lasciato al builder

- I nomi dei campi JSON nuovi e il testo esatto delle righe `danger`.
- Il formato della cache.
- I valori del tetto, del margine sulla data di creazione e della lunghezza massima, a partire
  dai numeri della fase 0.
