# Rebrand a Xiox: relazione (2026-11-13)

Piano seguito: `docs/superpowers/plans/2026-11-13-rebrand-xiox.md`. Stato: **completato**, test e build di rilascio verdi.

## Cosa è cambiato, per passo

### 1. Rinomina meccanica (commit `da9777d`)
- Pacchetto `pulisci-pc` -> `xiox-desktop`, binario `PulisciPC` -> `xiox-ui` (la CLI resta `xiox`).
- `crates/dc-{app,cleaner,disktree,engine,git,model,mukade,project,scanner,sessions}` -> `crates/xiox-*` con `git mv`; `crates/pc-rules` -> `crates/xiox-rules`.
- `PulisciPC.{rc,manifest,dev.rc,dev.manifest}` -> `xiox-ui.*`; `build.rs` e i riferimenti nei `.rc` aggiornati.
- Identificatori: `dc_*::`/`pc_rules::` -> `xiox_*::`, dipendenze `path` nei `Cargo.toml`, `-p pulisci-pc` nei commenti dei test; `Cargo.lock` rigenerato.
- Test dopo il passo: 991 passati, 0 falliti, 20 ignorati (come prima).

### 2. Nomi di marchio e dei dati, con i vecchi nomi ancora riconosciuti (commit `d1f13a1`)
- Interfaccia: titolo della finestra, marca nella barra laterale, testi dell'interfaccia, nome proposto per il resoconto (`Resoconto Xiox.txt`), user agent di curl (`-A Xiox`).
- Nuovo modulo `crates/xiox-model/src/legacy.rs` con `migrate_legacy_dir(old, new) -> PathBuf`, `app_dir_in(base)`, `APP_DIR = "Xiox"`, `LEGACY_APP_DIR = "PulisciPC"`. Regola: `new` esiste come cartella -> si usa `new`; esiste solo `old` e `new` è libero -> `rename` (stesso padre) e si usa `new`; **rinomina fallita -> si continua a usare `old`** (mai copia-e-cancella); niente di niente -> `new` (non si crea nulla qui).
- Ogni accessore di cartella dati passa dal modulo, con una variante `*_in(base)` testabile con cartelle temporanee: `xiox_app::settings_path`/`cache_dir` (`%APPDATA%`, `%LOCALAPPDATA%`), `xiox_rules::updater::data_dir` (`%ProgramData%\Xiox\winapp2`, serve l'amministratore per rinominare; senza, resta la vecchia), `sys::data_dir` (`Documents\Xiox`: backup del registro e dei servizi), `browserdb::keep_list_in` (lista cookie), `regbackup::user_work_dir`/`private_backup_dir` (`Local\Xiox\tmp` e `\backup`, backup con le password FTP).
- Protezione dalla pulizia: `rules::own_folders_with` protegge con tutto il contenuto **sia `Xiox` sia `PulisciPC`** in `%ProgramData%` e in `Documents` (profilo e `Env`), perché finché la rinomina non c'è (o fallisce) i backup dell'utente stanno ancora nella vecchia. La catena fidata di Winapp2 (`updater::trust_chain`) riconosce entrambi i nomi. La copia della prima versione (`%APPDATA%\...\winapp2`) si toglie sia sotto `Xiox` sia sotto `PulisciPC` (`legacy_dirs`).
- Nomi temporanei e di registro: si scrivono `Xiox_wipe_*`, `xiox-curl-*`, `xiox-worker-*`, `Xiox_StateFlags0099`, `Xiox_Verify_*`, `%TEMP%\Xiox`; si **riconoscono e si tolgono ancora** `PulisciPC_wipe_*` (`wipe::LEGACY_DIR_PREFIX`), `pulisci-curl-*`, `PulisciPC_StateFlags0099` (`winold::LEGACY_MARKER_NAME`, con `any_marker`/`remove_markers` che trattano entrambi i nomi). Variabile di prova `XIOX_WINAPP2` letta per prima, poi `PULISCI_WINAPP2`.
- Test nuovi (22): migrazione con cartelle temporanee in `xiox-model` (nuova esiste; solo vecchia -> rinomina con tutto il contenuto e seconda chiamata idempotente; rinomina che fallisce, sia con rename finto sia con un fallimento vero del sistema; né l'una né l'altra; nome nuovo occupato da un file), impostazioni e cache (`xiox-app`), cartella dati Winapp2 e catena fidata (`xiox-rules`), backup, lista cookie, cartelle di lavoro (`sys`, `regbackup`, `browserdb`), protezione di entrambe le cartelle (`rules`), residui `PulisciPC_wipe_*` e `pulisci-curl-*`, compagno del registro con entrambi i nomi, variabile d'ambiente.

### 3. Documentazione (commit di questo passo)
- `README.md` (nomi degli eseguibili e dei crate), `docs/xiox-cli.md`, `THIRD_PARTY_NOTICES.md` (solo il nome del prodotto), `MECHANICS.md` (solo il titolo), `docs/ccleaner-coverage.md` e `docs/cleaners-verifica-*.md` (percorsi dei crate, colonna «Copertura Xiox»; le colonne delle fonti restano), `vendor/mukade/VENDORED.md` e `crates/xiox-disktree/VENDORED.md` (solo il nome del prodotto, il codice vendorizzato non è toccato).
- `docs/superpowers/**`: le due spec e il piano del 2026-11-13 (CLI e osservatore) aggiornati ai nomi attuali; le spec e i piani datati 2026-11-10/11 non sono toccati a parte una riga di nota in testa sul cambio di nome; `docs/reference/devclean-*.md` hanno solo la nota in testa aggiornata.

## Verifica finale
- `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade`: **1013 passati, 0 falliti, 20 ignorati** (991 + 22 nuovi).
- `CARGO_INCREMENTAL=0 cargo build -j 1 --release --locked -p xiox-desktop -p xiox-cli`: riuscita (2 min 39 s dopo i test).
  - `target\release\xiox-ui.exe`: 12.262.400 byte
  - `target\release\xiox.exe`: 2.973.696 byte
  - Resta un `target\release\PulisciPC.exe` vecchio (12.263.424 byte, di un build precedente): è dentro `target/` (ignorato da git), non è prodotto dal nuovo build.
- Cartelle reali dell'utente (`%APPDATA%\PulisciPC`, `%LOCALAPPDATA%\PulisciPC`, `Documents\PulisciPC`): controllate prima e dopo le esecuzioni dei test, ancora presenti con il vecchio nome. Nessuna app è stata avviata e nessun test chiama gli accessori veri.

## Controllo `git grep` finale
`git grep -i -n -E "pulisci|pc-rules|pc_rules|\bdc[-_](app|cleaner|disktree|engine|git|model|mukade|project|scanner|sessions)\b"`. Ogni gruppo rimasto, con il motivo:

| Dove | Perché resta |
|---|---|
| `crates/xiox-model/src/legacy.rs` (costante `LEGACY_APP_DIR`, commenti, test) | È il riconoscimento del vecchio nome della cartella dati. |
| `crates/xiox-rules/src/updater.rs` (`LEGACY_SCRATCH_PREFIX`, commenti, test) | Cartelle `pulisci-curl-*` rimaste, trust chain e copia della prima versione sotto `PulisciPC`. |
| `crates/xiox-app/src/lib.rs`, `tests.rs` | Commento e test di migrazione delle impostazioni dalla cartella vecchia. |
| `src/wipe.rs`, `src/wipe/tests.rs` | `LEGACY_DIR_PREFIX = "PulisciPC_wipe_"` e i suoi test. |
| `src/winold.rs`, `src/winold/tests.rs` | `LEGACY_MARKER_NAME = "PulisciPC_StateFlags0099"` e i suoi test. |
| `src/rules.rs` | Protezione delle cartelle `PulisciPC` (`own_folders_with`), test della protezione, lettura di `PULISCI_WINAPP2` come ripiego. |
| `src/sys.rs`, `src/regbackup.rs`, `src/browserdb.rs`, `src/browserdb/tests.rs` | Documentazione e test della migrazione di Documenti, `Local\...\backup` e lista cookie. |
| `src/dev.rs`, `src/main.rs`, `src/native.rs`, `src/wipe.rs` (etichetta), `crates/xiox-cli/src/render.rs`, `docs/ccleaner-coverage.md` (5 righe) | «Pulisci» è il verbo italiano dell'interfaccia («Pulisci», «Pulisci spazio libero», il nome della voce di CCleaner), non il marchio; in `render.rs` è nella lista di parole italiane che la CLI non deve stampare. |
| `docs/superpowers/plans/2026-11-10-*`, `2026-11-11-*`, `specs/2026-11-10-*` | Documenti storici datati: contenuto intatto, in testa solo la nota sul cambio di nome. |
| `docs/superpowers/plans/2026-11-13-rebrand-xiox.md` | Il piano stesso (descrive i vecchi nomi). |
| `docs/superpowers/plans/2026-11-13-project-cleanup-cli.md:410`, `specs/2026-11-13-project-cleanup-cli-design.md` | La parola italiana «Pulisci» nella lista di controllo della CLI; la nota sul vecchio `PulisciPC.exe`. |
| `docs/reference/devclean-*.md` | Copie storiche: solo la nota in testa nomina i vecchi nomi. |

## Decisioni (dove il piano lasciava margine)
- **Posizione di `migrate_legacy_dir`:** il piano suggeriva `xiox-app` (`legacy.rs`), ma `xiox-rules` (cartella dati di Winapp2) non dipende da `xiox-app` e dipendere da lui avrebbe trascinato mukade e tutto il resto. Il modulo sta in `xiox-model` (che `xiox-rules` ora usa), così c'è **una sola** implementazione della regola di sicurezza.
- **Nome nuovo occupato da un file:** `std::fs::rename` su Windows può sostituire un file esistente. Per non distruggere niente, se qualcosa che non è una cartella ha già il nome nuovo, la rinomina non si tenta e resta in uso la cartella vecchia (c'è un test, che verifica anche che il file non cambi).
- **Entrambe le cartelle esistono:** si usa quella nuova (come da piano); la vecchia non si tocca e resta protetta dalla pulizia. Non c'è fusione.
- **`%TEMP%\PulisciPC` (copie temporanee di sessioni verso il Cestino):** diventa `%TEMP%\Xiox` senza riconoscere il vecchio, perché sono copie transitorie che il codice stesso toglie.
- **Variabili di prova `PC_RULES_CLEANERS`/`PC_RULES_WINAPP2` e marcatore `.pulisci-demo`:** rinominati in `XIOX_RULES_*` e `.xiox-demo` senza ripiego. Sono usati solo da test `#[ignore]` e dal demo di debug (`--dev-demo`); il vecchio marcatore non autorizza più la cancellazione definitiva (direzione sicura).
- **`vendor/mukade/VENDORED.md`:** il piano lascia `vendor/mukade` com'è perché è il prodotto Mukade; ho cambiato solo il nome del *nostro* prodotto in quel file di note scritto da noi, non il codice di mukade.
- **`MECHANICS.md`:** cambiato solo il titolo (nome del nostro prodotto); il resto è l'analisi del programma originale e non è toccato.
- **README:** la riga di roadmap «A **Xiox** desktop app: Tauri 2…» (app futura) non è stata toccata.

## Preoccupazioni
- Nessun controllo su un'installazione vera con i dati vecchi (il piano vieta di avviare le app sui dati reali): la rinomina di `%ProgramData%\PulisciPC` richiede l'amministratore ed è provata solo con cartelle temporanee. Da un'installazione non elevata resta in uso la vecchia cartella e la protezione/trust chain la riconoscono.
- Se `Xiox` e `PulisciPC` esistono entrambe in una stessa posizione (per esempio a causa di un intervento manuale), i backup della vecchia non si vedono più dall'app finché l'utente non li sposta a mano (non vengono cancellati né toccati).
- Gli esempi (`examples/`) e i test ignorati che usano i dati veri non sono stati eseguiti.
