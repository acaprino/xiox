# Istruzioni per gli agenti

## Qualità del codice

- Il codice di produzione deve essere organico, pulito ed elegante, con un'architettura studiata.
  I pattern di design si usano quando migliorano comprensione, leggibilità e manutenibilità:
  strategy, registry, tipi di valore piccoli, stadi puri di una pipeline.
- Un pattern serve la chiarezza, mai la cerimonia: nessuna astrazione senza un secondo uso o un
  beneficio di lettura evidente.
- Il codice di misura usa e getta può restare semplice, ma va dichiarato come tale. Tutto ciò che
  entra in un crate ha una struttura di moduli progettata, tipi piccoli e focalizzati, e test
  guidati dai dati.

## Estrattore dei percorsi dalle sessioni

- La decisione e le misure sono nella revisione 3 della specifica
  `docs/superpowers/specs/2026-11-13-session-path-discovery-design.md`. Sono parser deterministici
  con un registro delle evidenze per sessione; nessun modello locale, perché SmolLM2-135M e
  LFM2.5-350M inventano molti più percorsi di quanti ne trovino. Non riaprire la questione del
  modello senza un buco misurato su un campione nuovo.
- `bench/path-extractor/` è il prototipo di misura in Python (`extract.py`, `evidence.py`,
  `bench.py`, `real.py`, `harvest.py`, fixture in `crates/xiox-project/tests/data/shell-events.toml`). Va riscritto in Rust in
  `crates/xiox-project` con la struttura del codice della specifica.
- I due campioni reali da 180 comandi (sviluppo, seed 7; verifica, seed 2026) sono già stati
  usati. Una nuova regola (directory di output da variabili d'ambiente, PowerShell eseguito dal
  tool Bash, altri flag di output, `ForEach-Object`) si misura solo su un campione nuovo,
  etichettato alla cieca.
- I comandi reali delle sessioni non entrano mai nel repository: restano in cartelle temporanee.
- Stato del port in Rust (`crates/xiox-project/src/provenance`, in `main` dal 2026-11-16): le
  divergenze volute dal prototipo sono D1–D15, documentate nei commenti del codice e nelle classi di
  `bench/path-extractor/real.py parity` (D14 è solo dell'oracolo: il flag `loop` che il prototipo
  legge nel testo). D3: il taglio a 4.000 caratteri è solo della copia salvata, il comando è stato
  eseguito intero; si scarta solo il comando semplice con una parola al taglio. D15: il tool
  `shell` di OpenCode su Windows esegue PowerShell.
- Parità sulla cronologia reale (Step 3 del piano
  `docs/superpowers/plans/2026-11-15-shell-path-extractor.md`), 2026-11-16, sul `calls.jsonl` del
  2026-11-15: 94.862 chiamate shell e 20.818 cartelle, 0 differenze di chiamata fuori dalle classi;
  restano 8 cartelle OpenCode in cui il prototipo, leggendo PowerShell come Bash, toglie le barre
  rovesciate (Rust è giusto, D15 non le attribuisce ancora). Nessun candidato Rust contiene
  caratteri impossibili in un percorso Windows (erano 48 prima di D12). Le classi sono state
  allargate guardando questi stessi dati: su una cronologia nuova vanno rilette con sospetto.
- `MukadeSessions::call_reader()` è il lettore da usare in `analyze`: una copia del database di
  OpenCode per analisi, timeout reale (120 s) e tetto sull'output per ogni `dump`.
- Cronologia in `xiox analyze` (piano `docs/superpowers/plans/2026-11-16-history-analysis.md`,
  `crates/xiox-project/src/history`): `analyze` legge le sessioni del progetto, dei worktree e
  delle copie, ordinate per attività, dentro un budget (`BUDGET` in `history/queue.rs`: 600
  sessioni, 250.000 chiamate, 240 s). Tarato il 2026-11-16 con
  `cargo run -j 1 -p xiox-project --example session_probe -- <progetto>` sugli store reali di
  `C:\Users\alfio`: su un progetto con 2.271 sessioni collegate il tetto di 250 scattava a 241
  sessioni lette in circa 60 s (0,25 s a sessione, 28.704 chiamate); con 600 si resta sotto i
  240 s, che restano la vera protezione. Il primo avvio paga circa 35 s per la cache vuota
  dell'elenco di Mukade. Nuove misure solo su richiesta esplicita dell'utente. `--no-history` la
  disattiva. La categoria `scratch` è solo da rivedere: non si rimuove fino al passo 4 (conferme,
  `--yes`, contenitori, worktree `danger`, sorgente riscritta, cartella condivisa).
- Misura, 2026-11-16, su un campione nuovo etichettato alla cieca: le 113 chiamate reali in cui
  Rust e prototipo danno candidati diversi per D11, D12 o D13 (D10 non cambia mai i candidati),
  esclusi i due campioni precedenti. Rust con D3 fine e D15, contro il prototipo:
  - comandi interi (38): 131/145 percorsi contro 30/145; Rust non inventa nulla (i suoi 38
    candidati fuori dalle etichette sono tutti `sed -i`, che le etichette non contavano), il
    prototipo ne inventa 50;
  - comandi tagliati (58): 103/113 contro 88/113; Rust ha 9 candidati fuori dalle etichette, tutti
    il bersaglio di `cat > f <<'E'` col corpo tagliato, che il comando reale ha scritto e le
    etichette hanno lasciato vuoto; il prototipo ne inventa 49;
  - tool `shell` di OpenCode (17): 22/22 contro 12/22; Rust 0 inventati, il prototipo 29.
- Etichette: `sed -i` è una scrittura, e un heredoc col corpo tagliato scrive comunque il suo
  bersaglio. Le prossime istruzioni agli etichettatori lo devono dire.
- Passo 4, selezione e pulizia (piano `docs/superpowers/plans/2026-11-16-step4-selection-cleanup.md`,
  ramo `feat/step4`): schema 2 e livello `danger` di `xiox-project` (non di `xiox_model`), rischi per
  categoria in `history/danger.rs`, una sola ammissione `clean/admit.rs` per `select` e per la
  riconvalida, `--confirm-danger` e domanda per voce nella CLI. Alla pulizia la cronologia si
  rilegge solo se la scansione mostrata l'aveva letta (`--no-history` vale anche lì); identità
  della cartella, rischi e metodo devono restare quelli confermati. Le cartelle `danger` vanno
  solo nel Cestino, i worktree solo con `git worktree remove`. Il desktop non mostra voci della
  cronologia.
- Passo 5, verifica integrata (piano `docs/superpowers/plans/2026-11-17-step5-integrated-verification.md`,
  in fondo la mappa dei 15 casi di accettazione e dei test che li provano). Due cose da sapere:
  - `crates/xiox-cli/tests/sessions.rs` fa girare il vero `xiox` con il vero `mukade` su una
    home finta: `SessionWorld` imposta `USERPROFILE`, `LOCALAPPDATA` e `TEMP` propri e toglie le
    variabili degli store. Sta in un binario di test a parte perché, in parallelo ai test di
    `observe` e MCP, ne spinge la prima analisi oltre i 30 s di attesa.
  - `analyze_project_in` (build di debug con `XIOX_SEARCH_ROOTS`) legge la cronologia come
    `analyze_project`.
  `gitref.rs`: git non viene interrogato su un repository candidato il cui `.git`, `commondir`,
  `alternates` o file inclusi escono dai dischi locali fissi, passano per un link o finiscono in
  una cartella cloud. L'osservatore resta senza cronologia. Le misure sugli store reali del
  passo 5 non sono state fatte: solo su richiesta esplicita.
- Test della pulizia: nessun test raggiunge il Cestino reale (le cartelle in modalità Cestino
  passano dal `ParallelRemover`, non dalla `Trash` finta), salvo `recycle_mode_moves_tree_to_bin`
  di `xiox-cleaner` e `a_confirmed_danger_folder_goes_to_the_real_recycle_bin` della CLI, che
  svuotano dal Cestino la voce che hanno creato prima di ogni controllo. Una cartella `danger` si
  prova con `dry_run: true`, salvo quel test controllato; una rimozione vera si prova solo su una
  cartella che non è `danger` (per una `named`, un clone pulito di un repository estraneo). La fixture `env()` ha l'orologio due giorni
  avanti: ciò che il test crea non è attività recente, salvo un orologio proprio.
- Limiti di Mukade trovati il 2026-11-15:
  - `list pi` dà lo stesso id `session` a 50 sessioni;
  - 7 catene di subagent di Claude Code vengono rifiutate;
  - `dump opencode` richiede l'`opencode.exe` nativo (`--opencode-exe`) e `USERPROFILE`/`TEMP`
    senza prefisso `\\?\`. Il database di OpenCode si legge da una copia, mai dall'originale.

## Git

- I rami sono due, e solo due: `main` (ciò che si rilascia) e `develop` (dove si integra).
- Si lavora sempre su `develop`, o su un ramo breve che parte da `develop`, torna in `develop` e poi si
  cancella. Mai merge o push in `main`, e nessun altro ramo, né locale né su GitHub.
- Una release è la pull request `develop` → `main`, aperta e unita dal proprietario: ogni merge in `main` è
  una release (`.github/workflows/release.yml`, lo stesso in Shinatama, Mukade, Xiox, Muro, Ethel e
  artcraft-web).
  La versione sale di una patch, o di una minor o major se la pull request ha l'etichetta `release:minor` o
  `release:major`; `build-release.bat` costruisce su Windows; si pubblicano il tag `v<versione>` e la GitHub
  Release (l'installer di `release\installer\` e uno zip portabile con tutto `release\` in una cartella; nessun
  programma sciolto), e `main` torna in `develop`. Se la build
  fallisce non si pubblica nulla.
- Mai cambiare la versione o creare tag di release a mano: lo fa il workflow (`.github/release.json` elenca i
  file della versione, `.github/scripts/release.mjs` li scrive).

## Rust is built and tested on GitHub only (the owner's absolute rule)

- Never run `cargo` on this machine: no `cargo build`, `cargo test`, `cargo clippy`, `cargo check`, `cargo run`,
  `cargo doc`, no `tauri dev`, no `tauri build`, and no npm or batch script that calls them. `cargo fmt` is the
  only exception.
- Rust is compiled, linted and tested only by GitHub Actions. To know whether Rust code is right, push to
  `develop` and read the run (`gh run watch`, `gh run view --log-failed`).
- A program needed on this machine is taken from a release or a workflow artifact (`gh run download`), never
  built here.
- This holds for every agent and every subagent. A plan, a skill or a brief that says to run cargo locally is
  wrong on that point: write the code and the tests, do not run them, and say they wait for CI.

## UI: Imago

Before UI work, read node_modules/@acaprino/imago/docs/design-file.md and the component or style files it references.
Apply its tokens and confirmed design choices within the requested scope.
Use the package's im- classes and --im- tokens. Build a control of your own from the tokens, never from literal colours or sizes.
Report conflicts and keep unresolved choices explicit.
Run npx imago-check before you say UI work is done.
