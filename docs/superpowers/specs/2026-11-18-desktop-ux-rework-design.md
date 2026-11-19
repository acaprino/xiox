# Revisione della UX desktop di Xiox: look Smooth, ONI MODE, ordinamento e caselle di gruppo

Data: 2026-11-18. Segue `2026-11-14-xiox-desktop-ui-design.md`, che resta la descrizione di ONI MODE.

## Problem Statement

L'utente trova la finestra di Xiox inutilizzabile: "non si capisce niente". Sugli schermi di oggi:

- ogni testo è ambra in grassetto (nomi, descrizioni, livelli, date, dimensioni, dettagli): l'occhio non distingue il
  nome di una voce dai suoi dati e non trova la dimensione, che è il numero su cui si decide;
- l'intestazione di una categoria ha la stessa fascia blu della riga selezionata: non si capisce dove si è;
- la lista è sempre in ordine di dimensione decrescente e le intestazioni di colonna non fanno nulla: non si può
  ordinare per nome, data o altro;
- i gruppi di voci non si spuntano in un colpo: i sottogruppi di Winapp2 (Internet, Utilities…) non hanno casella, il
  Registro è una lista piatta di problemi di tipi diversi, le sessioni archiviate (centinaia) non si scelgono per
  progetto, i worktree non si scelgono per repository;
- la colonna delle quattro sezioni prende 160 px per quattro parole, e il pannello dei dettagli prende fino a metà
  della lista ripetendo quello che la riga dice già;
- l'etichetta del livello in maiuscolo su ogni riga è rumore.

## Solution

Due look per la stessa finestra:

- **Smooth**, il nuovo look di default: pulito e quasi nativo, come il tema scuro di Qamille. Due profondità (il
  fondo per la barra in alto e la colonna di sinistra, un foglio più chiaro che porta il lavoro), divisori di 1 px,
  niente card né ombre, il font di sistema, maiuscole solo nelle piccole etichette di sezione. L'ambra ONI è
  l'accento, forte e raro: scheda aperta, voce attiva, riga selezionata, caselle spuntate, totale di una categoria,
  pulsante principale. Le quattro sezioni sono schede nella barra in alto; a sinistra restano categorie e opzioni; il
  pannello dei dettagli è chiuso e si apre a richiesta. (Una prima versione con pannelli arrotondati e menu in
  maiuscolo è stata scartata dall'utente il 2026-11-18: "molto più clean e moderno, quasi nativo, un po' come
  Qamille".)
- **ONI MODE**, il look di oggi, da scegliere in Settings (la finestra che era About): stessa palette, stessa
  disposizione a tre colonne, stesso pannello dei dettagli sempre visibile. La scelta resta tra un avvio e l'altro.

In entrambi i look:

- un clic sull'intestazione di una colonna ordina la lista per quella colonna; un secondo clic inverte il verso; una
  freccia dice colonna e verso. L'ordine di partenza è quello di oggi (dimensione, dalla più grande);
- ogni gruppo di voci ha una casella che spunta o toglie tutto ciò che il gruppo può spuntare: sottogruppi di Winapp2,
  problemi del Registro per tipo, sessioni archiviate per progetto, worktree per repository;
- in Developers → Progetti i progetti stanno in un albero delle cartelle in cui si trovano (richiesta dell'utente del
  2026-11-19: "vedere tutto come un ramo folders"). Una cartella senza progetti propri e con una sola cartella dentro è
  una riga sola (`D:\Projects`); ogni cartella somma i suoi progetti, si piega (parte aperta) e ha la casella dei
  progetti liberi che contiene. Worktree e sessioni restano raggruppati come prima.

## User Stories

1. Come utente, voglio distinguere a colpo d'occhio il nome di una voce dalla sua descrizione e dai suoi dati, così
   capisco cosa sto per pulire.
2. Come utente, voglio che la dimensione sia il numero più evidente di una riga, così decido dove si libera spazio.
3. Come utente, voglio che l'intestazione di una categoria sia diversa dalla riga selezionata, così so dove sono.
4. Come utente, voglio ordinare la lista per dimensione, dalla più grande o dalla più piccola, così trovo subito ciò
   che pesa di più o di meno.
5. Come utente, voglio ordinare la lista per nome, dalla A alla Z o al contrario, così ritrovo una voce che conosco.
6. Come utente, voglio ordinare per le altre colonne (data, progetto, harness, file, stato, livello), così trovo le
   sessioni più vecchie o i servizi fermi.
7. Come utente, voglio vedere su quale colonna e in che verso è ordinata la lista, così non mi chiedo perché è in
   quell'ordine.
8. Come utente, voglio che l'ordinamento non mescoli le categorie di Cleanup, così il menu delle categorie resta
   allineato alla lista.
9. Come utente, voglio che un figlio resti sotto il suo genitore qualunque sia l'ordine, così una cartella di un
   progetto non finisce lontano dal suo progetto.
10. Come utente, voglio che l'ordine scelto resti quando cambio sezione e torno, così non lo devo rifare.
11. Come utente, voglio una casella su ogni sottogruppo di Winapp2, così spunto tutti i browser in un clic.
12. Come utente, voglio i problemi del Registro raggruppati per tipo con una casella per tipo, così correggo un tipo
    intero senza spuntare quaranta righe.
13. Come utente, voglio le sessioni archiviate raggruppate per progetto con una casella per progetto, così pulisco
    tutte le sessioni di un progetto chiuso in un clic.
14. Come utente, voglio i worktree raggruppati per repository con una casella per repository, così libero i worktree
    di un repository in un clic.
15. Come utente, voglio che la casella di un gruppo mostri se il gruppo è tutto, in parte o per niente spuntato, così
    so cosa contiene senza aprirlo.
16. Come utente, voglio che la casella di un gruppo non spunti le voci bloccate o a livello Caution, così non elimino
    per sbaglio qualcosa di rischioso.
17. Come utente, voglio vedere per ogni gruppo quante voci contiene e quanto pesa, così decido senza aprirlo.
18. Come utente da tastiera, voglio spostarmi sulle intestazioni dei gruppi con le frecce e spuntarle con Spazio, così
    non mi serve il mouse.
19. Come utente da tastiera, voglio ordinare la lista dalla tastiera, così l'ordinamento non è solo per il mouse.
20. Come utente di lettore di schermo, voglio sentire su quale colonna è ordinata la lista e in che verso, così so come
    leggerla.
21. Come utente, voglio le quattro sezioni come schede in alto, così la lista ha più spazio in larghezza.
22. Come utente, voglio che il pannello dei dettagli sia chiuso finché non lo chiedo, così la lista usa tutta
    l'altezza.
23. Come utente, voglio aprire i dettagli della riga selezionata con un tasto o un clic e richiuderli allo stesso
    modo, così vedo percorsi e file in uso quando mi servono.
24. Come utente affezionato al look di oggi, voglio attivare ONI MODE e ritrovare la finestra com'era, così non perdo
    la UI che conosco.
25. Come utente, voglio che il look scelto resti al prossimo avvio, così non lo devo riattivare.
26. Come utente, voglio che il cambio di look non cambi spunte, analisi, ricerca od ordinamento, così posso cambiare
    look in qualunque momento.
27. Come utente, voglio che il livello di una voce sia un'etichetta piccola, e che Caution resti evidente, così il
    rischio si vede e il resto non fa rumore.
28. Come utente che riduce le animazioni nel sistema, voglio che il look Smooth non animi nulla, così la finestra non
    si muove.
29. Come utente con i colori a contrasto elevato, voglio che spunte, selezione e fuoco restino visibili in entrambi i
    look, così posso usare Xiox.
30. Come utente con finestra stretta (1000 px) o zoom al 200 %, voglio che nulla si sovrapponga o venga tagliato
    senza tooltip, così la finestra resta leggibile.

## Implementation Decisions

- **Il motore decide, la pagina disegna.** Il contratto delle viste (Rust → TypeScript) resta quello di oggi: una
  sezione invia righe, colonne e opzioni; la pagina ordina, piega e disegna. Le caselle dei gruppi vivono nel motore,
  l'ordinamento, il look e l'apertura dei dettagli nella pagina.
- **Gruppi di voci = intestazioni con casella.** Il tipo di riga `header` (sottotitolo) porta ora uno stato di spunta
  come `group`: `none` resta il sottotitolo semplice di oggi, `on` / `off` / `mixed` disegna la casella. Nessun nuovo
  tipo di riga. Lo stato di un'intestazione è calcolato come quello di una categoria di Cleanup: conta solo le voci
  che può spuntare (né bloccate, né Caution); nessuna voce spuntabile dà `none`.
- **Toggle di un gruppo.** Il toggle di una riga riceve anche l'id di un'intestazione con casella: se tutte le sue
  voci spuntabili sono spuntate le toglie, altrimenti le spunta tutte, come la casella di una categoria di Cleanup.
  Agisce su ciò che la lista mostra (la ricerca può nasconderne alcune). Un'intestazione senza voci spuntabili
  rifiuta il toggle con una frase.
- **Quali gruppi:**
  - Cleanup: i sottogruppi di Winapp2 dentro Applications (id `section:<nome>`), nome e conteggio come oggi;
  - Registro: un'intestazione per tipo di problema, con il nome del tipo e il conteggio; le voci del tipo sotto;
  - Sessioni archiviate: un'intestazione per progetto (la cartella di lavoro della sessione; "nessun progetto" per
    quelle senza cartella), con il conteggio delle sessioni;
  - Worktree: un'intestazione per repository principale, con il conteggio; i worktree creati a mano restano sotto la
    loro intestazione di oggi, senza casella.
  La dimensione di un gruppo è la somma delle sue voci: la calcola la pagina, come oggi per l'ordinamento.
- **Ordinamento nella pagina.** Una chiave (colonna) e un verso per sezione, nello stato della pagina, non nel motore.
  Default: dimensione, decrescente. Il nome si confronta con la collazione naturale (maiuscole e numeri come li legge
  una persona); una cella con numeri raggruppati (18,204) si confronta come numero; il livello per rischio (Caution,
  Optional, Recommended); le righe senza valore vanno in fondo in entrambi i versi. L'ordinamento è stabile e
  rispetta la struttura di oggi: le categorie (`group`) restano nell'ordine del motore; dentro una categoria le
  intestazioni si ordinano tra loro con la chiave (la dimensione di un'intestazione è la somma delle sue voci) e le
  voci dentro ciascuna; i figli restano sotto il genitore e si ordinano tra loro.
- **Intestazioni di colonna.** Ogni intestazione di colonna è un pulsante con `aria-sort`; il clic sulla colonna
  attiva ne inverte il verso, il clic su un'altra la attiva nel suo verso naturale (dimensione e numeri decrescenti,
  testo crescente). Da tastiera: una scorciatoia cicla la colonna di ordinamento e un'altra ne inverte il verso,
  scritte nel suggerimento dei tasti.
- **Due look, un DOM.** Il markup è uno; il look è un attributo sulla radice del documento. Ogni look ha il proprio
  foglio di stile completo; il foglio di ONI MODE è quello di oggi, toccato solo per i nuovi controlli (pulsanti di
  ordinamento, casella delle intestazioni, scelta del look). Il primo fotogramma e lo sfondo della finestra
  restano `#0b0416` in entrambi i look, così all'avvio non c'è lampo.
- **Scelta del look.** In Settings (la finestra About, rinominata), voce `Look`: Smooth o ONI mode. L'interruttore
  nella barra in alto è stato tolto su richiesta dell'utente il 2026-11-19 ("elimina tasto oni mode, il tema vecchio
  resta attivabile da settings"). La scelta si salva nella memoria del webview come lo zoom; senza memoria vale Smooth.
- **Chiaro e scuro.** Smooth segue il tema di Windows (chiesto dall'utente il 2026-11-18: "anche light voglio"): la
  finestra non forza più il tema scuro, la pagina dice a Rust se il look è solo scuro e Rust chiede a Windows il tema
  scuro per ONI MODE e per Smooth quello del suo selettore (Auto, Chiaro, Scuro; Auto = nessuno, segue Windows; la scelta resta
  tra un avvio e l'altro); il runtime passa il tema della finestra a WebView2, così barra del titolo e
  `prefers-color-scheme` concordano. In chiaro: fondo grigio, foglio bianco, l'ambra resta per riempimenti e segni e
  scurisce quando è testo, il blu ONI diventa l'anello di fuoco.
- **Fedeltà a Qamille e Shinatama (2026-11-18, "moderno e clean, solo con accent color, più fedele possibile").** I
  token di Qamille chiari e scuri, IBM Plex Sans (portato con la sua licenza OFL), la colonna scura anche in chiaro, il
  marchio quadrato; dal look Smooth di Shinatama il vetro (finestra trasparente sull'acrilico di Windows, tinto per il
  tema) e la barra del titolo propria: finestra senza cornice in Smooth, pulsanti riduci/ingrandisci/chiudi della pagina,
  Snap Layouts dalla finestrella `HTMAXBUTTON` portata da Shinatama. ONI MODE riprende la cornice di Windows e niente vetro.
  Limiti noti: il menu di sistema col clic destro sulla barra e Alt+Spazio non sono ancora portati.
- **Look Smooth.** Neutri ardesia scuri (fondo, foglio, fascia della categoria), testo primario chiaro per i nomi,
  secondario e terziario smorzati per descrizioni e colonne, ambra ONI come unico accento di stato e d'azione, blu
  ghiaccio per il fuoco, arancio ONI per Caution. Font di sistema (Segoe UI Variable, con Segoe UI di riserva), in
  minuscolo; maiuscole spaziate solo per le etichette di sezione e i nomi di categoria. Controlli da 32 px, angoli di
  4–10 px, linee di 1 px, transizioni di 150–300 ms, nessuna animazione con la riduzione del movimento attiva.
- **Disposizione Smooth.** Barra in alto sul fondo: marchio sopra la colonna, schede delle sezioni allineate al
  foglio con la barra ambra sotto quella aperta, tema (Auto, Light, Dark) a destra. Colonna di categorie e opzioni sul
  fondo; sul foglio: riga di stato, lista, dettagli, pulsanti della sezione in basso a destra, piede. La colonna delle
  sezioni non si vede (ONI MODE la mostra come oggi). Le scorciatoie 1–4 delle sezioni restano.
- **Dettagli a richiesta.** Nello stato della pagina un flag "dettagli aperti", chiuso all'inizio. In Smooth il
  pannello si apre e si chiude con un pulsante sulla riga selezionata e con un tasto; in ONI MODE il pannello si vede
  sempre, come oggi, e il pulsante non c'è.
- **Livello come badge.** In Smooth il livello è un'etichetta piccola; Recommended e Optional sono smorzati, Caution è
  pieno e arancio. In ONI MODE resta com'è.
- **Mock e fixture.** Il modello di prova della pagina e le viste di prova seguono il motore: intestazioni con
  casella nelle stesse liste, toggle dei gruppi con le stesse regole e le stesse frasi; il test di deriva tra viste di
  prova e motore resta verde.

## Testing Decisions

- Un buon test prova il comportamento esterno: l'ordine delle righe che la pagina disegna, la spunta che una vista
  mostra dopo un toggle, ciò che la finestra mostra in ciascun look; mai la struttura interna.
- Punti di prova (i più alti possibile, già esistenti):
  - i test di nodo delle funzioni pure della pagina (ordinamento e visibilità delle righe, stato della pagina): ordine
    per ogni chiave e verso, struttura conservata (categorie, intestazioni, figli), righe senza valore in fondo;
  - i test delle intenzioni del motore per Cleanup, Developers e Registro: toggle di un gruppo da spento, da parziale e
    da pieno, voci bloccate e Caution escluse, ricerca rispettata, rifiuto senza voci spuntabili, rifiuto durante un
    lavoro;
  - il test di deriva tra viste di prova e motore, e i test del modello di prova;
  - lo smoke test con Playwright: ogni stato delle viste di prova disegnato e controllato in entrambi i look e nelle
    due misure di finestra (nulla tagliato, nulla sovrapposto), token di ciascun look, clic su un'intestazione di
    colonna e su una casella di gruppo con il loro giro nel modello di prova;
  - il test di contrasto dei token, esteso ai token di Smooth (AA per testo e controlli).
- Esempi già nel codice: i test di ordinamento e visibilità delle righe, i test delle categorie di Cleanup nelle
  intenzioni del motore, i controlli di misura e token dello smoke test.
- Le suite si fanno girare solo su richiesta dell'utente; il controllo dei tipi e la compilazione non sono test.

## Out of Scope

- Riscrivere la pagina in React o adottare la libreria SmoothUI: la pagina è TypeScript senza framework; dello stile
  SmoothUI si riprende l'aspetto, non il codice.
- Cambiare le frasi del motore, salvo nomi e conteggi delle nuove intestazioni.
- Ordinamento salvato tra un avvio e l'altro (resta per la sessione della finestra).
- Grafici, barre di proporzione o anelli delle dimensioni: si valutano nella revisione di rifinitura, non qui.
- Cambiare cosa fa la pulizia, la conferma o il rapporto.

## Further Notes

- Il nome "SmoothUI" indica uno stile, non una dipendenza: nessun pacchetto nuovo.
- Dopo le fette di questa spec, il passo di revisione di rifinitura (accessibilità, gerarchia e ritmo, stati,
  aspetto generico) si fa sul look Smooth finito e corregge in ordine di gravità.
- I file di screenshot con nomi reali di progetti e sessioni restano fuori dal repository (già ignorati).
