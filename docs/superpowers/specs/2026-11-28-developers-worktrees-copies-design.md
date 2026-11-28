# Developers: tutti i worktree e le copie dei repository

Data: 2026-11-28. Stato: bozza da approvare. Dimensione: progetto.

## Perché

Il 2026-11-28 il proprietario ha pulito a mano, con un agente, cose che Xiox non gli aveva
proposto: sei worktree `supads-studio-wt5…wt16` creati con `git worktree add` da subagenti accanto al
repository, i worktree in `daodan\.claude\worktrees` e in `WikiAsAService`, riferimenti git a worktree
già cancellati, rami già uniti. In `D:\Projects` restano copie come `supads-work-0.3.5` e
`supads-work-0.3.5-build`.

Nel desktop la categoria `Worktrees` vede solo i worktree che gli agenti registrano nei loro archivi
(`xiox_mukade::claude_worktrees`, `codex_worktrees`, `opencode_worktrees`); gli altri finiscono in
"Other worktrees (created by hand)", senza casella. Le copie non esistono affatto nel desktop.

La riga di comando sa già trovarli (`xiox analyze`, crate `xiox-project`): `worktrees` da
`git worktree list`, `copies` per identità del repository, `named` per nome, con livelli, blocchi e
ricontrollo prima della pulizia. Ma lavora un progetto alla volta e cerca sui dischi da sé: su
`D:\Projects\supads`, da utente normale, ha impiegato 491 s.

## Cosa ha deciso il proprietario

1. **Dove:** in Developers. `Worktrees` mostra tutti i worktree che git conosce; nasce la categoria
   `Copies`.
2. **Quando:** durante Analyze, per tutto il PC; e su richiesta, per un progetto solo.
3. **Approccio 1:** riusare `xiox-project` dandogli quello che Analyze ha già trovato, invece di
   farlo cercare sui dischi; una sola logica di sicurezza per riga di comando e desktop.
4. **Le cartelle create dagli agenti** (`scratch`, dalla cronologia delle sessioni) vanno a un secondo
   giro. Resta valida la regola di `AGENTS.md`: il desktop non mostra voci della cronologia.

## Obiettivi

- In `Worktrees`, ogni worktree collegato di ogni repository che Analyze trova, ovunque sia sul disco,
  con il livello e i blocchi di `xiox-project`.
- In `Copies`, i repository con la stessa identità (un primo commit in comune, o lo stesso `origin`
  normalizzato) e le cartelle col nome di un progetto (`supads-work-0.3.5`), raggruppati per
  repository.
- Su richiesta, "Refresh this project": la stessa ricerca rifatta per un solo repository, in pochi
  secondi, senza rifare Analyze su tutto il PC.
- Pulizia di queste voci attraverso la pulizia di `xiox-project`, non quella del motore del desktop.

## Non obiettivi

- Leggere la cronologia delle sessioni: l'entrata nuova lavora con `history: false`. Le sessioni vive
  bloccano come oggi (`live-session`); se il loro elenco non si legge, worktree e copie non si
  puliscono, come fa `clean` della riga di comando.

- Le cartelle `scratch` e qualunque lettura della cronologia delle sessioni (secondo giro).
- Cancellare rami. I rami vecchi o uniti si possono mostrare come informazione, mai cancellare (come
  la riga di comando).
- `git worktree prune` automatico. Un worktree registrato la cui cartella non c'è più si mostra come
  nota, come nella riga di comando.
- Rendere la scansione annullabile (oggi non lo è).

## Anti-obiettivi

- Una seconda logica di sicurezza solo per il desktop.
- Una copia o una cartella col nome spuntata da sola: partono sempre senza spunta.
- Togliere lavoro non salvato: modifiche, commit che il repository di riferimento non raggiunge, un
  worktree `locked` o con una sessione aperta restano bloccati, come nella riga di comando.
- Una scansione che diventa lenta senza dirlo.

## Vincoli

- Le regole di `AGENTS.md`: niente compilazione su questa macchina (Rust e pagina li prova Verify);
  la pagina usa solo Sigrid; una sola ammissione `clean/admit.rs` per selezione e riconvalida; cartelle
  `danger` solo nel Cestino; worktree solo con `git worktree remove`, mai forzato.
- Nuove misure sugli store reali solo su richiesta esplicita del proprietario: il tempo della ricerca
  su tutto il PC va misurato, quindi il piano chiede il permesso prima di farlo.
- Da utente normale l'indice veloce dei dischi (MFT) non si legge: la ricerca deve funzionare anche
  senza, dicendo che è limitata.

## Approcci

1. **Riusare `xiox-project` con i candidati di Analyze (scelto).** `xiox-project` ottiene un'entrata
   che fa solo worktree, copie e cartelle col nome, prende i candidati da un indice dei repository
   costruito una volta, e tiene le sue regole. Costo: la ricerca va resa sostituibile e serve un
   indice delle identità, perché confrontare ogni progetto con ogni repository vorrebbe dire
   135 × 135 chiamate a git.
2. **`xiox analyze` così com'è su ogni progetto, cercando solo accanto.** Quasi niente codice, ma le
   copie lontane (per esempio in `D:\claude-tmp`) sfuggono, ogni coppia di copie compare due volte e
   l'analisi completa rifà anche build, log e sessioni per ogni progetto.
3. **Logica nuova solo nel desktop.** Scartato: due regole di sicurezza diverse.

## Il disegno

### 1. Entrata nuova di `xiox-project`: le relazioni di un repository

`analyze_with` fa tutto (build, log, worktree, sessioni, cronologia, copie, rami). Per il desktop
serve solo una parte: una funzione pubblica che, per un progetto, restituisce gli `Item` delle
categorie `Worktrees`, `Copies` e `Named` (con i loro `Level`, `Block`, `Why`, `Target`,
`identity`) e le `Note` (`PrunableWorktree`, `WalkMayMissDeepFolders`, `GitMissing`), senza il motore
delle build, senza log, sessioni archiviate e cronologia. I blocchi per sessione aperta restano: le
sessioni vive sono già caricate da Analyze.

Il risultato deve poter passare da `clean::select` e `clean::clean` come oggi un `ProjectScan`: o è un
`ProjectScan` con solo quelle categorie, o `select`/`clean` accettano lo stesso insieme di `Item`.
*(Scelta lasciata a chi costruisce, purché l'ammissione resti una sola.)*

### 2. L'indice dei repository, una volta per Analyze

*(Corretto il 2026-11-28 dopo la lettura del codice: Analyze oggi butta via l'indice del disco e le
sessioni a fine scansione. Analyze conserva **l'elenco dei repository** trovati, non l'indice del disco;
"Refresh this project" rilegge le sessioni.)*

Un tipo nuovo in `xiox-project` (nome indicativo `RepoIndex`) costruito una volta per scansione:

- **I repository del PC.** Con l'indice MFT (amministratore): le cartelle chiamate `.git` sui dischi
  NTFS, e il loro genitore. Senza: la stessa camminata limitata della riga di comando (dischi fissi e
  home, 3 livelli), fatta **una volta per tutti i progetti** e non una per progetto. In più, sempre:
  le radici dei progetti che Analyze ha trovato. La ricerca rispetta le impostazioni di Analyze: con
  `all_drives` spento cerca solo nelle radici di Analyze e nelle cartelle padre dei suoi progetti; con
  `--only-root` solo dentro quella cartella. *(Scelta di Claude.)*
- **L'identità di ciascuno**, calcolata una volta: i commit radice (`xiox_git::root_commits`) e
  l'`origin` normalizzato (`xiox_git::normalize_remote`). Un repository che git non deve leggere
  (`git_may_read`) non entra.
- **I gruppi:** repository legati da un commit radice in comune o dallo stesso `origin` (chiusura
  transitiva: se A è legato a B e B a C, stanno insieme).
- **Le cartelle col nome:** per ogni progetto che è la radice principale di un repository, le
  cartelle **sorelle** nella sua cartella padre che `name_matches_project` accetta, come la modalità
  `ParentOnly` di oggi: è lì che stanno `supads-work-0.3.5` e `-build`. **Nella lista del desktop** non
  sono cartelle col nome: un repository git (è una copia per identità o un progetto a sé), un worktree
  collegato, un progetto di Analyze che è un repository. È un filtro della lista del desktop: `classify`
  e la riga di comando restano come sono, con i loro test. Una cartella che corrisponde a più progetti va a quello col nome più
  lungo. *(Scelte di Claude: sulla ricerca a tutto il PC i nomi su tutti i dischi darebbero centinaia
  di progetti veri come "copie"; un worktree avrebbe due modi di essere tolto.)*

`classify` usa l'identità dall'indice invece di chiamare git su ogni candidato. `has_own_work`, che è
costoso, si chiama solo sulle copie trovate.

### 3. Chi è la copia di chi

In un gruppo di repository con la stessa identità, tutti sono copie gli uni degli altri. Per non
mostrare la stessa coppia due volte e per sapere rispetto a cosa una copia "ha lavoro suo":

- **il riferimento** del gruppo è il repository con l'attività più recente (l'ultima attività che
  Analyze conosce: sessione, commit o file; a pari merito, il primo per percorso);
- gli altri sono **copie del riferimento**, con `has_own_work` rispetto a lui.

*(Scelta di Claude. Alternativa: il riferimento è quello con più sessioni di agenti, o quello che il
proprietario indica. Si cambia in un punto solo.)*

### 4. Worktree

Per ogni repository principale dell'indice (non un worktree collegato): `git worktree list`, e per
ogni worktree collegato che esiste un `Item` della categoria `Worktrees`. Togliere un worktree esegue
solo `git worktree remove` (mai `--force`) e `prune`: **nessun ramo viene cancellato**. Su questo il
proprietario ha deciso tre stati, al posto dei due di oggi, per riga di comando e desktop insieme:

| Stato del worktree | Livello |
|---|---|
| pulito, HEAD contenuta anche in un ramo remoto o in un altro ramo locale (oggi `head_kept_elsewhere`) | `recommended` |
| pulito, i commit stanno **solo sul ramo del worktree**, che resta nel repository | `caution`, mai spuntato da solo; la conferma dice quale ramo resta e con quanti commit propri |
| HEAD staccata, con commit su nessun ramo | bloccato: togliendolo i commit si perderebbero |

Restano come oggi: bloccato se ha modifiche non salvate, se è `locked`, se una sessione è aperta
dentro; `danger` se usato o cambiato nelle ultime 24 ore. *(Decisione del proprietario, 2026-11-28,
su consiglio di Claude: il livello `caution` del secondo caso perché stamattina, su questi worktree,
il proprietario ha voluto decidere lui e ha tenuto i rami.)*

I worktree che gli agenti registrano (oggi la sola fonte del desktop) sono un caso di questi: la riga
dice da quale agente vengono quando lo si sa.

- **Spunta iniziale:** partono spuntati solo i worktree `recommended`, come oggi quelli inattivi e
  puliti. *(Scelta di Claude.)*
- **Cartelle di build dentro un worktree:** restano pulibili come oggi, dal motore del desktop, anche
  quando il worktree intero non si può togliere; stanno sotto la riga del worktree. *(Scelta di
  Claude: è il comportamento di oggi, e non tocca la logica di `xiox-project`.)*

La lista di oggi "Other worktrees (created by hand, not removed from here)" scompare: ogni worktree ha
la sua casella, se le regole lo permettono.

### 5. La pagina

- **`Worktrees`:** come oggi, raggruppati per repository; righe e stato vengono dagli `Item`.
- **`Copies` (nuova, terza voce, prima di `Archived sessions`):** un gruppo per identità. Intestazione:
  il nome e l'`origin` del riferimento. Sotto, una riga per copia (`State`: `no own work`,
  `has own work`, `not checked`) e una per cartella col nome (`matched by name only`). Colonne:
  cartella, stato, ultima attività, dimensione. Nessuna riga parte spuntata; la casella del gruppo
  non spunta le righe `caution` o `danger`, come oggi `All`.
- **Progetto singolo, "Refresh this project":** in `Projects`, sulla riga selezionata (nei dettagli
  del progetto e nella palette dei comandi). Rifà la ricerca di questo disegno per quel solo
  repository, contro l'indice dell'ultimo Analyze (nessuna camminata sui dischi), e sostituisce le sue
  righe in `Worktrees` e `Copies`. Serve dopo aver lavorato su un progetto, senza rifare Analyze su
  tutto il PC. Le righe rinfrescate portano la loro ora: la regola dei 30 minuti per pulirle conta da
  lì; le altre restano legate all'ora dell'ultimo Analyze. Senza un Analyze fatto prima il comando non
  c'è. *(Decisione del proprietario, 2026-11-28, su consiglio di Claude.)*
- **Avanzamento di Analyze:** un passo in più, "Looking for worktrees and copies… n of m repositories".
- **Note:** i worktree registrati senza cartella e la ricerca limitata (senza MFT) sono avvisi in
  testa alla lista, nella forma delle note di oggi.

### 5b. Una copia che ha worktree suoi

Una copia con worktree collegati che esistono è **bloccata** (motivo nuovo, per esempio "holds the
git data of linked worktrees"): toglierla distruggerebbe il deposito git di quei worktree, anche
quando stanno fuori dalla copia. La regola vale anche per la riga di comando, che oggi non la ha.
*(Scelta di Claude: è una regola di sicurezza in più, coerente con gli anti-obiettivi.)*

### 6. La pulizia

Le voci di `Worktrees` e `Copies` si puliscono con `clean::select` + `clean::clean` di
`xiox-project`, non con il piano del motore del desktop:

- worktree: `git worktree remove`, mai forzato, sempre definitivo (come oggi);
- copie e cartelle col nome: Cestino o definitivo secondo l'opzione `Deletion`; quelle `danger` solo
  nel Cestino;
- la conferma elenca ogni voce `caution` e `danger` con i suoi rischi, e una voce `danger` va
  confermata da sola (una casella per voce), come `--confirm-danger` nella riga di comando;
- prima di togliere, l'identità della cartella, i rischi e il metodo si ricontrollano (`recheck` e
  `Run::admitted` in `clean/mod.rs`, che rifanno l'analisi del progetto con le cartelle scelte come
  `known`, senza camminare sui dischi quando la scansione è `ParentOnly`): se qualcosa è cambiato, la
  voce si salta con il motivo;
- un'analisi più vecchia di 30 minuti si rifiuta, come oggi.

### 7. Tempo

La ricerca su tutto il PC costa, in chiamate a git: 3 per repository (`resolve_full`), 2 per l'identità,
circa 12 per ogni worktree collegato (`GitCli::state` più `head_kept_elsewhere`), 5 o più per ogni copia
trovata (`has_own_work`). *(Corretto il 2026-11-28: la prima stima diceva 1 chiamata per i worktree.)* Obiettivo: meno di 60 s in più su Analyze sul PC
del proprietario (circa 135 progetti), misurato con l'eseguibile di CI; Analyze riporta quanto ha
aggiunto. Il tempo vero va misurato prima di deciderne il tetto: il piano include una misura, da fare
solo con il permesso del proprietario. *(Obiettivo: scelta di Claude.)*
Se la ricerca supera il tetto, si ferma, mostra quello che ha trovato e lo dice in un avviso.

## I piani

Il lavoro si fa in quattro piani, ognuno pubblicabile da solo *(consiglio del revisore, 2026-11-28)*:

1. **Motore e riga di comando:** i tre stati dei worktree, il blocco delle copie con worktree,
   l'indice dei repository, l'entrata nuova di `xiox-project`, l'eliminazione dei doppioni; Verify
   prova anche `xiox-project`, `xiox-git` e `xiox-cli`.
2. **`Copies` nel desktop** (si aggiunge, non toglie nulla).
3. **`Worktrees` del desktop** passa alle voci di `xiox-project` (la parte che può peggiorare).
4. **"Refresh this project".**

## Lasciato a chi costruisce

- La nuova fase di Analyze vive fuori da `xiox_app::run_scan`: `xiox-project` dipende da `xiox-app`,
  il contrario sarebbe un ciclo.

- I nomi dei tipi e delle funzioni nuove, e se l'entrata nuova restituisce un `ProjectScan` ridotto o
  un altro tipo, purché `select`/`clean`/`admit` restino gli stessi per riga di comando e desktop.
- Come il desktop conserva gli `Item` di più progetti nella sua scansione (`DevScan`) e li ritrova per
  la pulizia.
- Il parallelismo delle chiamate a git e il tetto di tempo, dopo la misura.
- Le stringhe esatte dell'interfaccia (inglese, come il resto della sezione).

## Verifica

- Test guidati dai dati in `xiox-project` (repository finti in cartelle temporanee, come i test che ci
  sono): gruppi per identità, transitività, riferimento del gruppo, copie con e senza lavoro proprio,
  cartelle col nome sorelle, worktree fuori dalla cartella del repository, worktree `locked`.
- Test della vista in `xiox-core` per le categorie `Worktrees` e `Copies` e per le conferme `danger`.
- `npm test`, `imago-check`, `sigrid-check` e il fumo su Verify.
- Prova sulla finestra vera da parte del proprietario: `supads-work-0.3.5` in `Copies`, un worktree
  creato con `git worktree add` fuori dalla cartella del repository in `Worktrees`.
