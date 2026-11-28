# Developers: vista flat e sotto-progetti sotto il padre

Data: 2026-11-28. Stato: bozza da approvare. Dimensione: modifica piccola.

## Perché

Nella lista Projects di Developers l'ordine per dimensione non dice quali progetti pesano di più:
vale solo dentro ogni cartella dell'albero. In più un progetto che contiene altri progetti compare
due volte, una come progetto e una come intestazione di cartella (`tri-llm-gateway` con `admin-ui` e
`vscode-extension`; `react-app` con `src-tauri\gen\android`), e le due righe finiscono lontane.

## Cosa cambia

1. **Sotto-progetti sotto il padre** (deciso dall'utente). Un progetto la cui cartella sta dentro
   quella di un altro progetto è una riga sotto la riga del progetto più vicino che lo contiene. Il
   suo nome è il percorso relativo al padre (`admin-ui`, `src-tauri\gen\android`). Non nasce più
   un'intestazione di cartella per una cartella che è essa stessa un progetto.
2. **Vista flat** (deciso dall'utente). Una nuova scelta `Layout` tra le opzioni di Developers:
   `Tree` (predefinita, come oggi) e `Flat`. In flat non ci sono intestazioni di cartella: ogni
   progetto, sotto-progetti compresi, è una riga di primo livello, con il **percorso completo** come
   seconda riga, così si capisce chi sta dentro chi. Le cartelle da pulire restano apribili sotto il
   loro progetto. L'ordine per dimensione vale su tutta la lista.
3. La scelta si salva con le altre opzioni di Developers e torna al prossimo avvio. Con
   `--only-root` resta in memoria, come le altre.

Vale solo per Projects. Worktrees e Archived sessions non cambiano.

## Non obiettivi

Campo di ricerca in Developers, i 13 avvisi grezzi in cima alla lista, velocità dell'analisi,
worktree e copie non riconosciuti: lavori successivi, ognuno col suo disegno.

Anti-obiettivo: nessun cambiamento a cosa si può pulire. Selezione, blocchi, conferme e piano di
pulizia restano identici; cambia solo come le righe sono disposte.

## Approcci

- **A. Lo fa il motore (scelto).** `crates/xiox-core/src/dev/view.rs` manda le righe già in albero
  o in flat; un nuovo intent `setLayout` e una opzione di tipo `choice`, lo stesso controllo di
  `Deletion`. Rispetta la regola "la pagina non decide nulla", si prova con test Rust guidati dai
  dati, la scelta si salva. Costo: è Rust, si verifica solo su GitHub Actions.
- **B. Lo fa la pagina.** `rows.ts` toglie le intestazioni in flat. Scartato: i sotto-progetti
  richiedono comunque Rust, la pagina dovrebbe ricavare il percorso dai dettagli della riga, e la
  scelta non si salverebbe.
- **Solo flat, via l'albero.** Scartato: l'albero serve a spuntare una cartella intera.

## Scelte di Claude (da controllare)

1. **Casella del padre:** spunta solo le sue cartelle da pulire, non quelle dei sotto-progetti. Un
   sotto-progetto ha la sua casella. Così non si pulisce più del voluto.
2. **Dimensione del padre in albero:** la sua più quella dei sotto-progetti, come fanno le
   intestazioni di cartella, così l'ordine per dimensione lo mette al posto giusto anche da chiuso.
   I dettagli mostrano le due cifre separate. *(Cambio rispetto a quanto detto in chat: avevo
   proposto la sola dimensione propria, ma da chiuso un sotto-progetto pesante sarebbe invisibile.)*
3. **Dimensione in flat:** ogni riga mostra solo la propria, quindi nessun doppio conteggio.
4. **Seconda riga in flat:** il percorso completo al posto del motivo ("Recent activity…"); il
   motivo resta nei dettagli. *(Aggiornato dall'utente il 2026-11-28: anche una riga bloccata
   mostra il percorso, così la struttura resta visibile; resta grigia e senza casella, e perché è
   bloccata è nei dettagli.)*
5b. **Il controllo** *(aggiornato dall'utente il 2026-11-28)*: non nel pannello Options ma un
   pulsante piccolo `Flat` accanto a Options (premuto = lista piatta); sotto 860 px scende nella
   riga degli strumenti sotto il titolo, che va a capo.
5. **Il controllo sta nel pannello Options**, non sopra la lista: usa un componente che c'è già.
   Un interruttore in vista sopra la lista richiederebbe di verificare un componente di Sigrid.
6. **Un sotto-progetto è sempre in vista sotto il padre**, anche col padre chiuso, e viene dopo le
   cartelle da pulire del padre. La freccia del padre apre solo quelle cartelle. Motivo: l'analisi
   spunta da sola le cartelle dei progetti inattivi, e una spunta non deve mai restare nascosta. Per
   questo `rows.ts` cambia (ordine e visibilità), diversamente da quanto detto sotto in "Cosa tocca".
7. **La radice di un disco non è mai un padre**: un progetto in `D:\` non raccoglie sotto di sé
   tutti i progetti del disco.
8. **La finestra finta** (`XIOX_UI_FAKE=1`) ignora la scelta Layout, come ignora le altre opzioni.
9. **`All` e la casella in cima alla lista** contano i sotto-progetti come progetti, in entrambe le
   viste.

## Cosa tocca

- Rust: `crates/xiox-core/src/dev/view.rs` (`folder_tree`, `push_folder`, `push_project`, opzioni),
  `dev/mod.rs` (stato e intent), `crates/xiox-core/src/view.rs` (`DevIntent::SetLayout`), il posto
  dove si salvano le opzioni di Developers in `pref.json`, i test in `dev/tests.rs` e
  `dev/intent_tests.rs`.
- Pagina: `apps/desktop/src/types.ts` e `intents.ts` (il nuovo intent), `mockmodel.ts` (un progetto
  con un sotto-progetto). `rows.ts` e `list.tsx` non dovrebbero cambiare: una riga sotto una riga
  progetto è già gestita; va confermato con un test di `rows.ts`.
- Guida: `docs/xiox-desktop.md`, capitolo Developers.

## Lasciato a chi costruisce

- Come riconoscere il padre più vicino (confronto con `path_key`, come il resto dell'albero).
- Il nome del campo salvato e il suo valore predefinito quando manca (`Tree`).
- La forma esatta dei test guidati dai dati.

## Verifica

Niente si compila su questa macchina: test Rust e pagina girano su GitHub Actions dopo il push su
`develop`. Prima di dire finito: `npx imago-check` e `npx --no sigrid-check apps/desktop/src`. La
prova a occhio sulla finestra vera si fa con l'eseguibile dell'artefatto del workflow.
