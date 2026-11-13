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
