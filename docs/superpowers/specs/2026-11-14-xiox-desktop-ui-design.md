# Interfaccia desktop di Xiox (Tauri, stile Oni)

Data: 2026-11-14. Stato: da approvare.

## Perché

L'app con finestra di oggi (`xiox-ui.exe`) è la vecchia interfaccia egui di PulisciPC con il nome
cambiato: sfondo chiaro, schede a sinistra, testi italiani. Non è quello che l'utente ha chiesto
(«porta tutto in Rust Tauri con una UI figa come shinatama/mukade»). Il rebranding ha
rinominato i file e i dati, non l'interfaccia. Questo progetto costruisce l'interfaccia vera e
**elimina quella egui**.

## Decisioni dell'utente

- **Tauri**, come mukade e shinatama; l'app egui sparisce del tutto.
- **Stile Oni**: sfondo viola-nero con trama a circuito, cornici blu acciaio con gli angoli
  tagliati e la tacca in alto, testo ambra in grassetto, titoli azzurri. Il mock-up approvato
  («PERFETTO») è `docs/assets/branding/xiox-ui-preview.png`; la sorgente HTML è
  `.superpowers/brainstorm/17805-1794570359/content/oni-xiox-2barre-en.html` (copiarla in
  `docs/design/` come riferimento vivo).
- **Due barre laterali**: sezioni, poi categorie della sezione aperta; il dettaglio della voce
  scelta sotto l'elenco. Niente schede in alto.
- **Niente figura**: solo la scritta XIOX (`docs/assets/branding/xiox-logo.png`).
- **Solo scuro, solo inglese** (nessun cambio di lingua).
- Il nome è **Xiox** ovunque; stesse funzioni di oggi, nessuna aggiunta di funzioni.
- Le quattro sezioni di oggi: Cleanup (file), Registry, Services & RAM, Developers.

## Cosa si costruisce

Un'app Tauri 2 con interfaccia web (TypeScript senza framework, come mukade) che chiama il motore
Rust esistente. Le funzioni, le regole e le protezioni **non cambiano**: cambia solo il livello
di presentazione.

### Struttura (decisioni mie, da controllare)

- **Cartella `apps/desktop/`** nel workspace, con la stessa forma di mukade:
  `index.html`, `src/` (TypeScript + CSS), `src-tauri/` (crate Rust `xiox-ui`, binario
  `xiox-ui.exe`), `package.json`, `vite.config.ts`, `tests/`. Versioni agganciate a quelle di
  mukade (`D:\Projects\mukade\apps\desktop\package.json` e `src-tauri/Cargo.toml`: Tauri 2.12.4,
  Vite 8.3.5, TypeScript 7.0.2); il builder le legge da lì.
- **Il motore esce dal binario egui.** I moduli di `src/` che non dipendono da egui
  (`cleaner`, `rules`, `registry`, `regbackup`, `services`, `sysclean`, `wipe`, `winold`,
  `netcreds`, `envpath`, `browserdb`, `live_checks`, `native`, `account`, `sys`) e la logica di
  `dev.rs` che non è disegno si spostano in un crate di libreria **`xiox-core`**
  (`crates/xiox-core`), senza cambiare il comportamento né i test. Il disegno (`main.rs`,
  `theme.rs`, il codice egui di `dev.rs`) viene cancellato quando l'interfaccia nuova li
  sostituisce, insieme alle dipendenze `eframe`, `rfd`, `image` che servivano solo a lui.
  `xiox-desktop` (il pacchetto egui) sparisce dal workspace.
- **Comandi Tauri** (`src-tauri/src/commands/`): un modulo per sezione, che chiama `xiox-core`;
  le operazioni lunghe girano su un thread e mandano eventi (`progress`, `done`, `error`) alla
  finestra; si possono annullare. Nessun comando espone scrittura libera su disco o shell: ogni
  rimozione passa per le stesse protezioni di oggi (`xiox-cleaner::guard`, riconvalida).
- **Sicurezza di Tauri**: capability minime (solo i comandi dell'app, niente plugin `fs` o
  `shell` per il JavaScript), CSP `default-src 'self'`, nessun contenuto remoto, nessuna
  richiesta di rete dall'interfaccia (l'aggiornamento delle regole Winapp2 resta nel motore, come
  oggi).
- **Privilegi: come oggi.** La versione di produzione chiede l'amministratore all'avvio
  (`requireAdministrator`, una richiesta UAC); le build di sviluppo e i test restano `asInvoker`
  (stessa coppia di manifesti di oggi: `xiox-ui.manifest` e `xiox-ui.dev.manifest`, scelta in
  `build.rs`). Decisione dell'utente. Nessun rilancio da utente normale in questo lavoro. La
  barra di stato mostra `administrator` o `standard user` come oggi.
- **Finestra**: minimo 1000 x 640, ricorda dimensione e posizione, una sola istanza.
  Barra del titolo del sistema (non disegnata a mano) con il colore scuro del tema se Windows lo
  permette.
- **Icona**: ricavata dal logo (la X iniziale su fondo scuro), in tutte le dimensioni dell'`.ico`.

### Come si presenta (dal mock-up)

- Barra in alto: `XIOX` (scritta del logo in piccolo), versione; a destra i pulsanti `ANALYZE a` e
  `CLEAN <n> ITEMS ↵` (il secondo diventa `Clean 3 folders`, `Remove 1 worktree`… nella sezione
  Developers, come nella versione egui di oggi), `AUTO` (aggiornamento automatico dell'analisi
  non c'è: il pulsante non esiste se non fa niente).
- Riga di stato sotto: etichetta dello stato (`ANALYSIS DONE`), totali, tempo; a destra
  «administrator / standard user · RAM <n> GB».
- Barra 1 (`SECTIONS`): Cleanup 1, Registry 2, Services & RAM 3, Developers 4 (tasti rapidi).
- Barra 2 (`CATEGORIES`): le categorie della sezione aperta con conteggio e peso; per Developers
  Projects, Worktrees, Archived sessions; vuota o assente dove la sezione non ha categorie
  (Registry, Services & RAM: la barra 2 mostra le opzioni della sezione).
- Area centrale: l'elenco della categoria con casella, nome, percorso o motivo, livello
  (`RECOMMENDED` azzurro, `OPTIONAL` grigio-blu, `CAUTION` arancio), dimensione a destra in
  carattere a spaziatura fissa; sotto, il pannello di dettaglio della voce selezionata
  (voce, file, dimensione, livello, modo, perché).
- Barra in basso: stato e tasti (`↑↓ move · space toggle · a analyze · ↵ clean`).
- Finestre di dialogo: stesse del programma di oggi (conferma di pulizia con elenco e avvisi e,
  dove oggi c'è, «I have read the warnings»; backup del registro prima di correggere; resoconto
  finale con voci eliminate, lasciate perché in uso, errori; «Save as text…»).
- Cornici ad angoli tagliati con `clip-path`, trama a circuito in CSS, font **Bahnschrift** per i
  titoli e **Cascadia Mono** per i numeri e i percorsi (già presenti in Windows), testo 13 px.
  Colori e bordi presi dal mock-up (token CSS nel file `src/styles/tokens.css`).
- **Tastiera completa**: `1`–`4` cambiano sezione, frecce muovono la selezione, spazio spunta,
  `a` analizza, Invio pulisce (sempre con la conferma), `Esc` annulla/chiude, Tab raggiunge ogni
  controllo con un anello di fuoco visibile.
- Rispetta `prefers-reduced-motion`; nessuna animazione continua.

### Le quattro sezioni (stesse funzioni di oggi)

Il comportamento di ogni sezione è quello dell'app egui, descritto dal suo codice (`src/main.rs`,
`src/dev.rs`) e dalle specifiche già approvate (`2026-11-10-dev-mode-design.md`,
`2026-11-10-catalogo-sviluppo-design.md`, `2026-11-12`/`-13` per le regole). Il builder le rilegge;
qui solo l'elenco, con il nome inglese dei testi. Restano anche: la finestra About con
l'attribuzione CC-BY-SA di Winapp2 e la versione caricata (obbligatoria), l'avviso di profilo
diverso, la riga di stato di Winapp2, la ricerca fra le regole e la scelta delle cartelle
aggiuntive di Developers (plugin di dialogo di Tauri, come mukade).

| Sezione | Funzioni da portare |
|---|---|
| **Cleanup** | categorie `System`, `Explorer`, `Advanced`, browser e applicazioni (dal catalogo di regole); le voci native (pulizia dello spazio libero, `PATH`, credenziali di rete, dati vecchi di Windows, elenco dei cookie da tenere) con i loro avvisi e le loro conferme; opzione «Only delete Temp files older than 24 hours»; analisi con progresso e annulla; elenco per voce con dimensione, livello, motivo; pulizia con conferma e avvisi delle opzioni scelte; resoconto; pulizia dello spazio libero (wipe) e le sue protezioni com'è oggi; Cestino |
| **Registry** | scansione dei problemi, elenco, backup del registro prima di correggere (cartella `Xiox` dei backup), correzione. Nessun ripristino dentro l'app (oggi non c'è: si fa con un doppio clic sul file `.reg` del backup) |
| **Services & RAM** | elenco dei servizi con stato e avvio, interruttore Recommended/All, ricerca, aggiorna; azioni `Disable`, `Set to Manual` e `Restore originals`, ognuna con la sua conferma; memoria in uso e stima di RAM liberabile |
| **Developers** | `Projects`, `Worktrees`, `Archived sessions`; opzioni giorni di inattività, Cestino/definitiva, cartelle aggiuntive; livelli, «I have read the warnings», pulsante e conferma per sezione («Clean 5 folders»…), resoconto, «Save as text…», annulla a metà |

I testi italiani dell'interfaccia di oggi si traducono in inglese (una sola lingua). I nomi dei
file di dati e le regole non cambiano.

## Decisioni dopo il controllo del builder (mie, da controllare)

Un revisore ha letto il codice contro questo documento. Le risposte che ho dato:

- **Lo stato vive in Rust.** `DevState` (selezioni, preselezione, controllo di vecchiaia
  dell'analisi, limite di 30 minuti sulla conferma, righe `caution`, Cestino/definitiva, mancanza
  di spazio nel Cestino) e l'orchestrazione che oggi sta in `main.rs` (thread di pulizia,
  `poll`, `finish_clean`, `clean_report`, lavori di registro e servizi) escono dall'egui e vanno in
  `xiox-core` senza `egui::Context`: al suo posto un'interfaccia di notifica (un trait) che il
  livello Tauri implementa mandando eventi. L'interfaccia web **disegna** soltanto: non decide
  cosa è selezionabile né cosa si può rimuovere. I 37 test di `dev.rs` si spostano intatti; la
  logica non si riscrive in TypeScript. In TypeScript restano solo formattazione, ordinamenti e
  i testi dei pulsanti.
- **Cleanup: cosa sono le righe.** Come oggi si spuntano le voci prima dell'analisi e l'analisi
  restituisce il totale (byte e file) per voce. Il mock-up si realizza così: barra 2 = i gruppi
  del catalogo (`System`, `Explorer`, `Advanced`, browser, applicazioni); area centrale = le voci
  (regole) del gruppo con casella, dimensione dopo l'analisi, e un livello **derivato**:
  `RECOMMENDED` se la regola è accesa di default, `OPTIONAL` se è spenta di default, `CAUTION` se
  ha un avviso; modo `to the Recycle Bin` solo per la voce Recycle Bin, `permanently` per le
  altre. «Why» è la descrizione della regola dove il catalogo ce l'ha, altrimenti il campo si
  nasconde. Nessun dato inventato.
- **Italiano nel motore**: si traduce **alla fonte** (nomi delle categorie, avvisi delle regole,
  nomi del registro, suggerimenti dei servizi, `start_text`, `ABOUT_RULES`, `NO_PREVIEW`, i testi
  di errore); i test che asseriscono quei testi si aggiornano. «I test del motore non cambiano»
  vale per la logica, non per le stringhe tradotte.
- **Prove di schermata**: due livelli. Rust: `Backend` finto per i test dei comandi. Interfaccia:
  `src/mock.ts` con gli stessi dati e uno script Playwright su Vite in Chromium, come lo
  `smoke.mjs` di mukade (Playwright non guida direttamente una finestra Tauri/WebView2).
- **Versioni** (dal controllo): `package.json` di mukade sta nella radice del repository, non in
  `apps/desktop`; Tauri 2.12.4 è la versione npm, il crate Rust è bloccato a 2.12.5;
  `tauri-plugin-dialog` è agganciato a `=2.8.4`. Il builder copia i blocchi esatti da
  `D:\Projects\mukade\Cargo.lock` e `package-lock.json`.

## Come si prova

- **Modalità finta per le prove**: l'interfaccia parla con un `Backend` (un trait Rust) che ha
  l'implementazione vera e una finta con dati fissi; la finta si attiva solo in build di debug con
  `XIOX_UI_FAKE=1` (come gli altri interruttori solo-debug: `cfg!(debug_assertions)`). Nessuna
  prova automatica o di schermata tocca il sistema vero, i progetti veri o il registro vero.
- **Rust**: test dei comandi con il backend finto (eventi di avanzamento, annullamento, errori in
  inglese); i test di `xiox-core` sono quelli dei moduli spostati, invariati.
- **TypeScript**: logica pura (formattazione, ordinamenti, stato della selezione, testi dei
  pulsanti) con `node --experimental-strip-types --test`, come mukade.
- **Schermate**: script Playwright che apre la build di debug con `XIOX_UI_FAKE=1` e confronta
  le quattro sezioni a 1000 x 640 e 1440 x 900; il controllo guarda che non ci sia testo
  tagliato o sovrapposto e che i colori dei token siano quelli giusti.

## Vincoli

- `cargo` sempre `-j 1` con `CARGO_INCREMENTAL=0`; una build alla volta; i dischi sono quasi
  pieni: `node_modules` e `target` di Tauri pesano diversi GB, niente copie.
- Le funzioni del motore non si riscrivono: si spostano. Ogni test esistente continua a passare
  (1013 + quelli di `xiox observe`).
- Tutto il testo visibile e gli identificatori in inglese.
- L'eseguibile di produzione (`xiox-ui.exe`) non deve richiedere runtime esterni oltre a WebView2
  (già presente in Windows 11).
- Nessuna dipendenza che apra la rete dall'interfaccia.

## Non si fa

- Niente funzioni nuove (la vista per progetto, l'osservatore, la tab di mukade sono lavori a
  parte); niente tema chiaro; niente cambio di lingua; niente installatore (MSI) in questo
  lavoro; niente aggiornamento automatico dell'app; niente icona nell'area di notifica.
- Niente figura o mascotte.
- Non si copia né si riproduce la grafica del videogioco Oni: lo stile si ispira a forme e colori.

## Sarebbe un fallimento se

- l'interfaccia sembra ancora un'app egui o un'app qualunque;
- una prova automatica o una schermata modifica il sistema vero;
- una funzione di oggi manca o ha protezioni più deboli;
- la versione di produzione non chiede l'amministratore all'avvio come oggi, o le build di sviluppo e i test lo chiedono;
- c'è testo italiano visibile o testo tagliato a 1000 x 640;
- `cargo test` o la build di produzione non passano.

## Lasciato al builder

- Dettaglio dei comandi Tauri e dei messaggi, forma del `Backend`, come dividere i file
  TypeScript, nomi dei componenti; come ottenere la barra del titolo scura; come generare
  l'`.ico`.
- Il disegno preciso dei dialoghi, della barra di avanzamento e degli stati vuoti, nel rispetto
  del mock-up e dei token.
- L'ordine di porting delle sezioni (si consiglia Developers, poi Cleanup, poi Registry, poi
  Services & RAM, perché Developers è quella già ripulita e con più test).
