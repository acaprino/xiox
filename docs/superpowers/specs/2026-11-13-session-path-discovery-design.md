# Copie scoperte dalle sessioni degli agenti

Data: 2026-11-13. Stato: da approvare.

Dipende da `2026-11-13-project-cleanup-cli-design.md` (analisi mirata, `xiox analyze` / `xiox clean`).
Ricerca di base: `reports/Estrazione path da transcript agenti.md`.

## Perché

Chi lavora con molti agenti insieme, senza controllarli, si ritrova con cloni e copie dello
stesso repository in posti che non ricorda: a sei livelli di profondità, su un altro disco, con
un nome qualsiasi. Oggi l'analisi mirata li trova solo se stanno entro 3 livelli dalle radici
dei dischi o dalla home, oppure (terminale elevato) se hanno il nome del progetto.

Le sessioni degli agenti sanno dove gli agenti hanno lavorato. Xiox le elenca già, ma usa la
cartella di lavoro (`cwd`) solo per attribuire al progetto le sessioni che ci stanno dentro.

Lo stesso profilo d'uso rende la pulizia più pericolosa: agenti accesi mentre Xiox gira, lavoro
mai committato, e spesso è un agente a lanciare `xiox clean --yes`. Più potenza nella scoperta
va quindi insieme a più guardie.

## Il principio

**Una menzione propone, l'identità git decide.** Un percorso trovato in una sessione diventa
solo un candidato in più per `classify()` (`crates/xiox-project/src/search.rs`). Resta in elenco
soltanto se ha lo stesso commit radice o lo stesso origin del progetto, e si può togliere solo
se supera i controlli di oggi (`has_own_work`, sessione viva, riconvalida prima di cancellare).
Nessuna cartella diventa rimovibile perché una sessione l'ha nominata.

## Cosa si costruisce

Tre livelli di profondità, dal più economico al più costoso. Ognuno si costruisce solo se la
verifica del precedente lo giustifica.

| Livello | Cosa legge | Costo | Attivo |
|---|---|---|---|
| 1. `cwd` | La cartella di lavoro di ogni sessione, di ogni agente (già in memoria dopo l'elenco) | Nessuna lettura in più | Sempre |
| 2. Campi strutturati | Dentro i transcript: `cwd` per turno, `workdir`, `file_path`/`path` dei tool, elenchi `readFiles`/`modifiedFiles` di Pi | Lettura dei transcript, con cache | Su richiesta (`--deep-sessions`) |
| 3. Comandi shell e patch | `command` dei tool shell, righe `*** Update File:` di `apply_patch` | Interpretazione fragile | Non si costruisce finché la fase 0 non dice che serve |

Il testo libero (messaggi, output dei tool) non si legge a nessun livello: vedi «Non si fa».

Insieme alla scoperta entrano quattro guardie nuove (sezione «Guardie»), che valgono anche per
le copie trovate dal walk di oggi.

## Fase 0: verifiche su Windows, prima di scrivere il resto

Il contenitore di sviluppo è Linux e la ricerca è fatta su documenti: quello che conta va
misurato sul PC vero. Si costruisce per prima una **sonda in sola lettura**
(`cargo run -p xiox-project --example session_probe`), che stampa solo conteggi e tempi: nessun
percorso, nessun testo di sessione, così l'uscita si può incollare in una chat senza rischi.
La lancia l'utente, a mano. Non scrive niente, nemmeno la cache.

| # | Verifica | Cosa decide |
|---|---|---|
| V1 | Per agente: quante sessioni, quanti `cwd` distinti, quanti esistono ancora, quante radici di repository distinte ne escono | Se il livello 1 trova abbastanza |
| V2 | Quante di quelle radici il walk di oggi **non** raggiunge (più di 3 livelli, o fuori dalle radici cercate) | **Il guadagno del livello 1.** Se è zero, la funzione non serve |
| V3 | Forme dei `cwd` visti: `C:\`, `c:\`, `C:/`, `\\?\`, UNC, `\\wsl.localhost\`, `/c/`, `/mnt/c/`, POSIX puro; con spazi, non ASCII, oltre 260 caratteri | Quali forme normalizzare e quali scartare |
| V4 | Per agente: file totali, GB totali, file più grande, data della sessione più vecchia, presenza di `.jsonl.zst` (Codex) | Costo del livello 2; quanto indietro vedono le sessioni (Claude Code cancella dopo 30 giorni) |
| V5 | Nomi dei campi realmente presenti: chiavi dei tool di Claude Code, forma di `arguments` di Codex, tabelle e colonne di `opencode.db` (`PRAGMA table_info`), header di Pi | Il livello 2 si scrive sui nomi veri, non su quelli della ricerca |
| V6 | Tempo per leggere tutti i transcript con il solo filtro sulle chiavi, a freddo e a caldo, con Defender attivo | Se il livello 2 può stare in `analyze` o va tenuto dietro al parametro |
| V7 | Aprire in lettura un transcript mentre l'agente ci scrive, e `opencode.db` mentre OpenCode è acceso: l'agente continua a scrivere, rinominare e troncare? | Modo di apertura dei file (condivisione completa; SQLite in sola lettura, mai `immutable`) |
| V8 | Tempo medio di una chiamata `git.exe` e numero di chiamate per candidato; quante volte git risponde `dubious ownership`, da terminale normale e da terminale elevato | Il tetto ai candidati per esecuzione; se serve leggere `.git` senza avviare git |
| V9 | Quanti `cwd` cadono in cartelle cloud (OneDrive, Google Drive) o su unità non fisse | Conferma dei filtri prima di toccare il disco |
| V10 | Solo se V2 lascia scoperto molto: quante radici **in più** dà il livello 2 rispetto al livello 1, e quante il livello 3 rispetto al 2 | Se costruire i livelli 2 e 3 |

Criteri per andare avanti:

- **Livello 1**: si costruisce se V2 > 0 su almeno un progetto reale.
- **Livello 2**: si costruisce se V10 mostra radici nuove che sono davvero copie di un progetto
  (controllate a mano dall'utente su un campione), e V6 resta sotto qualche secondo a caldo.
- **Livello 3**: solo se il livello 2 lascia fuori copie che l'utente sa esistere.

## Livello 1: i `cwd` di tutte le sessioni

1. Si raccolgono i `cwd` di tutte le sessioni (vive e archiviate, tutti gli agenti) da
   `Env::sessions` ed `Env::archived`, con agente e data dell'ultima attività.
2. Ogni `cwd` passa il **filtro dei percorsi** (sotto) prima di qualunque accesso al disco.
3. Si risale alla radice con la logica che c'è già in
   `xiox_scanner::project_roots_from_sessions` (cartella mancante, cloud, cartelle «larghe»),
   tenendo solo le radici che sono repository git: una cartella con un marker e senza `.git`
   non ha identità e non entra.
4. Le radici si aggiungono ai candidati di `search()`, senza doppioni con walk e indice MFT.
5. `classify()` decide. Le radici che non sono copie del progetto si scartano in silenzio.

Vale anche con `--no-drives`: non costa letture e non allarga il walk.

Ogni copia così trovata porta la **provenienza**: da quali agenti, in quante sessioni, data
dell'ultima. Entra in `Why::SameRepository` (campo nuovo, `SCHEMA` sale di uno) e nella riga
stampata, per esempio `seen in 3 Codex sessions, last 2026-11-02`.

## Livello 2: i campi strutturati dei transcript

Si leggono solo le sessioni **del progetto** (il cui `cwd` è nel progetto o in un suo worktree)
e quelle delle copie già trovate: sono loro che possono aver creato o toccato altre copie. Non
si leggono le sessioni degli altri progetti.

| Agente | Campi | Note |
|---|---|---|
| Pi | `cwd` dell'header; `details.readFiles` e `details.modifiedFiles` | Il più pulito |
| Codex | `turn_context.payload.cwd`; `workdir` dentro `arguments` | `arguments` è JSON in una stringa: due decodifiche. `workdir` è relativo al `cwd`. I rollout vecchi sono `.jsonl.zst` |
| Claude Code | `cwd` di ogni record; `file_path`, `path`, `notebook_path` dei `tool_use` | Formato non documentato: i nomi vanno confermati da V5 |
| OpenCode | `session.directory`, `project.worktree`; le chiavi dei tool solo se V5 le conferma | Colonne controllate a runtime; se mancano, si salta con una nota |

Lettura:

- file aperti in lettura con condivisione completa, letti a blocchi; **mai mmap** (su Windows una
  mappa aperta fa fallire la troncatura da parte dell'agente);
- righe separate con `memchr`; una riga si interpreta solo se contiene una delle chiavi cercate;
- una riga incompleta in fondo (l'agente sta scrivendo) si lascia alla prossima lettura;
- una riga che non si interpreta si salta: mai un errore dell'analisi.

Cache, nella cartella che Xiox usa già (`xiox_app::cache_dir()`), per file: dimensione, data,
impronta del primo KiB, offset dopo l'ultima riga completa, e il risultato (cartelle
normalizzate con conteggio e data). File invariato: si salta. File cresciuto con la stessa
impronta: si riprende dall'offset. Altrimenti si rilegge. I `.zst` si rileggono interi quando
cambiano. **La cache contiene solo percorsi di cartelle, conteggi e date: mai testo delle
sessioni.** Ha un numero di versione; se cambia l'estrattore, si butta.

Da ogni percorso di file si risale alla radice del repository; le cartelle di dipendenze e di
build (`node_modules`, `target`, `.venv`, …) tagliano il percorso invece di scartarlo.

## Guardie

Valgono per `worktrees`, `copies` e `named`, comunque siano stati trovati.

**G1. Filtro dei percorsi, prima di toccare il disco.** Il contenuto di un transcript non è
fidato: può contenere qualsiasi stringa (output di comandi, pagine web, testo scritto apposta).
Un percorso preso da una sessione si accetta solo se:

- è assoluto con lettera di unità, e l'unità è un disco fisso;
- **non è UNC né un percorso di dispositivo** (`\\server\…`, `\\?\UNC\…`, `\\.\…`,
  `\\wsl.localhost\…`): solo guardare se esiste un percorso di rete fa partire
  un'autenticazione verso quel server e può bloccare l'analisi;
- non ha caratteri di controllo né caratteri vietati nei nomi Windows, e non supera una
  lunghezza massima;
- non è in una cartella cloud (controllo sul nome, prima dell'accesso, come oggi);
- **nessun antenato è un link o una junction**: il walk non segue mai i link, e un percorso
  preso da una sessione non deve farlo al suo posto.

Quello che non passa si scarta in silenzio e si conta in una nota.

**G2. Attività recente di un agente.** Oggi una cartella è «in uso» solo se la sessione è
verificata viva, e questo si sa solo per Claude Code: per Codex, OpenCode e Pi `live` non è mai
vero. Una copia in cui Codex sta lavorando adesso oggi non è bloccata. Nuovo blocco
`RecentAgentActivity`: la voce è bloccata se una sessione di un agente la cui vitalità non è
verificabile ha `cwd` lì dentro ed è stata aggiornata nelle ultime 24 ore. Non vale per
`builds` (decisione già presa: l'età del progetto non conta per le build).

**G3. Tetto e ordine.** Con migliaia di sessioni i candidati possono essere centinaia, e ogni
candidato costa alcune chiamate a git. Si classificano dal più recente, fino a un tetto (numero
da V8), con i controlli economici per primi (origin e commit radice) e `has_own_work` solo su
chi è risultato una copia. Oltre il tetto: nota `SessionCandidatesTruncated`, mai un silenzio.

**G4. Le sessioni non lette non allargano niente.** Se l'elenco o la lettura delle sessioni
fallisce, l'analisi prosegue senza quei candidati e lo dice (nota). Il blocco che già esiste
quando le sessioni vive non si leggono resta com'è.

Quello che non cambia: le copie sono sempre `caution`, non entrano mai con un parametro di
categoria, si tolgono solo con `--include <path>`; Cestino per impostazione; riconvalida subito
prima di cancellare; nessun link seguito.

## Decisioni mie (l'utente non le ha discusse: da controllare)

- **Il livello 1 è sempre attivo**, senza parametro: non legge niente in più e non può rendere
  rimovibile nulla. Il livello 2 sta dietro `--deep-sessions` finché V6 non dice che è veloce.
- **G2 a 24 ore**, e vale anche per le voci trovate dal walk. È un cambio di comportamento: una
  copia oggi `caution` può diventare bloccata.
- **Le cartelle senza `.git` nominate dalle sessioni non entrano in elenco.** Senza identità git
  resta solo il nome, e quello c'è già (`named`). Un `cp -r` senza `.git` e con un nome diverso
  resta invisibile: lo considero il prezzo giusto.
- **Il livello 2 legge solo le sessioni del progetto e delle sue copie**, non tutte: meno
  letture, meno rumore, e un progetto non «vede» nelle sessioni degli altri.
- **Dove sta il codice del livello 2.** `vendor/mukade` non si tocca. Propongo di scriverlo in
  `crates/xiox-mukade` usando i lettori di riga di mukade, e di portarlo a monte in mukade solo
  dopo, se serve anche lì.
- **Dipendenze nuove**: `memchr` e `zstd` (livello 2). `typed-path` solo nei test, per provare i
  percorsi Windows su Linux. Niente `regex` fino al livello 3.
- **La provenienza si stampa sempre**, anche quando la copia l'avrebbe trovata comunque il walk.

## Vincoli

- Quelli della spec dell'analisi mirata: testi in inglese, `analyze` non scrive niente tranne la
  cache, mai provare sui progetti veri (la sonda della fase 0 è l'unica eccezione, la lancia
  l'utente e stampa solo conteggi), `cargo -j 1`.
- Niente viene scritto negli store degli agenti. `opencode.db` si apre in sola lettura.
- I test costruiscono store finti per i quattro agenti sotto `tempfile::tempdir()`.

## Non si fa

- Niente lettura del testo libero: messaggi, output dei tool, prompt di sistema.
- Niente percorso «rimovibile perché nominato»: nessuna categoria nuova senza identità git.
- Niente ricerca per contenuto dei file (resta la decisione della spec precedente).
- Niente vista di tutti i progetti (`analyze --all`, «famiglie di cloni»): resta fuori, come
  deciso. Se l'utente la vuole, è una spec a parte e il livello 1 ne è già la base.
- Niente percorsi WSL o di rete.
- Niente sostituzione di `git.exe` con una libreria (`gix`): solo se V8 lo impone.

## Sarebbe un fallimento se

- una cartella venisse elencata come copia, o tolta, solo perché una sessione la nomina;
- una copia in cui un agente sta lavorando venisse tolta;
- un percorso preso da una sessione facesse toccare a Xiox la rete, un link o una cartella cloud;
- un transcript illeggibile, enorme o scritto apposta facesse fallire o bloccare `analyze`;
- la lettura di Xiox facesse fallire una scrittura dell'agente sul suo transcript;
- testo di una sessione finisse nella cache, nell'uscita o nel JSON;
- `analyze` diventasse sensibilmente più lento per chi ha migliaia di sessioni.

## Ordine di costruzione

1. **Sonda** (fase 0). L'utente la lancia su Windows e riporta i numeri.
2. **Guardie G1 e G2**, con i loro test: utili anche senza il resto.
3. **Livello 1** con G3, G4 e la provenienza.
4. Si rilancia la sonda (V10) e si decide sul **livello 2**.
5. **Livello 3** solo su prova che serve.

## Lasciato al builder

- Il nome esatto dei campi JSON della provenienza e delle note nuove.
- Il formato della cache del livello 2.
- I valori del tetto e della lunghezza massima, a partire dai numeri della fase 0.
