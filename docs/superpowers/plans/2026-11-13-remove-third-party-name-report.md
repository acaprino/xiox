# Rimozione dei riferimenti al prodotto di terzi: relazione

Nota: questa relazione ha il nome `remove-third-party-name` e non quello richiesto, perché il nome richiesto conteneva il nome del prodotto e avrebbe fatto fallire `git ls-files | grep -i`.

Richiesta dell'utente: eliminare ogni riferimento al prodotto di pulizia di terzi (qualunque grafia, compresi nomi di file e di cartella) dai file tracciati da git. Winapp2 resta, citato come fonte delle regole. Questa relazione non scrive il nome del prodotto, per non reintrodurlo nel grep di verifica.

## Per gruppo

- **Tabella di copertura:** `docs/<vecchio nome>.md` diventa `docs/rules-coverage.md` (`git mv`). Colonne «Voce (italiano)» e «Default» (prima nominavano il prodotto). Tolte le fonti F1-F7 (stringhe lette dall'eseguibile con hash e indirizzi, esportazioni di file del prodotto, documentazione, articolo forense, due discussioni di forum) e i paragrafi «Il buco principale», «Come si chiude il buco», «Cosa non si è potuto verificare» riscritto senza confronti; restano le fonti `WA` (tabella dei `LangSecRef` di Winapp2, senza le righe del prodotto), `W` (file Winapp2 letto in sola lettura) e `S` (nomi italiani citati dalla spec). Nella colonna `Fonte` le sigle F1-F7 sono tolte (cella «—» se resta vuota). Tolti i nomi delle routine interne (`N_*`), i conteggi del prodotto e le frasi di puro confronto («come in …», «Default diversi da …», punto aperto 1 riscritto come «Default e nomi italiani»). Tutte le righe, gli id, i titoli `## 2. Scheda Windows`, `## 3. Scheda Applicazioni`, `**Safari per Windows**`, `### 3.2` e le 7 colonne su cui si appoggia il test sono intatti.
- **Test `crates/xiox-rules/tests/phase3_xml.rs`:** percorso di `include_str!` aggiornato; test rinominato `phase3_options_are_off_unless_table_default_on`, variabile `table_default`; tolta la riga che controllava l'assenza del nome del prodotto nei testi visibili (le altre asserzioni del test restano). Nessun test rimosso.
- **Commenti:** `cleaners/windows_extra.xml`, `cleaners/safari.xml`, `crates/xiox-rules/src/winapp2.rs` (2), `src/browserdb.rs`, `src/envpath.rs`, `src/netcreds.rs`, `src/sysclean.rs`, `src/winold.rs`, `src/wipe.rs` (due frasi), `src/cleaner.rs` (tolto l'indirizzo di decompilazione), `src/registry.rs`, `src/services.rs`: percorso nuovo della tabella o frase neutra.
- **Catalogo browser:** tolta da `src/browserdb.rs` la voce del browser del prodotto (nome, processo, radice del profilo: una sola `BrowserDef`). Nessun test dipendeva da quella voce (nessun conteggio fisso), quindi nessun test tolto; le righe della tabella e le frasi sui «11 browser» ora dicono 10. Si perde la pulizia di quel solo browser; può tornare con un'etichetta neutra.
- **I due file di meccaniche (un .md e un .json nella radice):** `git grep` mostrava solo tre commenti in `src/` (`cleaner.rs`, `registry.rs`, `services.rs`) e la tabella; nessun codice, test o build li leggeva. Eliminati con `git rm` entrambi; i tre commenti non rimandano più a loro. Nessun dato è stato conservato o rinominato.
- **Documenti storici** (`docs/superpowers/specs/*`, `plans/2026-11-10-*`): riscritte o tolte solo le frasi che nominavano il prodotto (confronti, «rivale di …», «come in …», percorsi assoluti del checkout, indirizzo del file Winapp2 della sottocartella di terzi, che si rimanda alla costante `WINAPP2_URL`). Il piano della Fase 3a ha ora il nome `2026-11-11-fase3-copertura-regole.md` e i rimandi sono aggiornati.
- **Altro:** `docs/reference/devclean-DESIGN.md`, `docs/xiox-cli.md` («the workspace root, the folder that holds the top-level `Cargo.toml`»), piano e relazione del rebranding.

## Decisione sull'URL di Winapp2

Controllo in sola lettura: il file della radice del repository `Winapp2.ini` risponde 200 (1.829.579 byte), ma non è equivalente a quello usato: stesso numero di sezioni (4068) ma 139 sezioni in posizione diversa e 2523 righe `LangSecRef` contro 2231. È una variante diversa, quindi **l'URL non è stato cambiato**. Resta l'unica occorrenza, in `crates/xiox-rules/src/updater.rs:54`, con il commento «Upstream path of the rules file in the Winapp2 repository (third party, not ours).» che non nomina il prodotto.

## Verifica finale

- `git grep -i -n` sul nome del prodotto: una sola riga, `crates/xiox-rules/src/updater.rs:54` (l'URL di cui sopra).
- `git ls-files | grep -i` sul nome del prodotto: nessun output.
- `git grep -i -n -E "mechanics\.(md|json)"` (e `MECHANICS`): nessun output.
- `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade`: 1013 passati, 0 falliti, 20 ignorati (come prima).
