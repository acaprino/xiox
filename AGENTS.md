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
  divergenze volute dal prototipo sono D1–D11, documentate nei commenti del codice e nelle classi di
  `bench/path-extractor/real.py parity`. Aperti:
  - la parità sulla cronologia reale (Step 3 del piano
    `docs/superpowers/plans/2026-11-15-shell-path-extractor.md`) non è ancora stata eseguita;
  - prima di fidarsi dello Step 3, le classi D10 (`cd -` e opzioni di `cd`) e D11 (graffe Bash)
    di `parity` vanno ristrette alla direzione della differenza che spiegano, come le altre;
  - il piano di integrazione in `analyze` deve dare a `MukadeSessions::dump_calls` un timeout reale
    e una sola copia del database di OpenCode per analisi, non una per sessione;
  - D10 e D11 cambiano quante cartelle si trovano: si misurano su un campione nuovo.
- Limiti di Mukade trovati il 2026-11-15:
  - `list pi` dà lo stesso id `session` a 50 sessioni;
  - 7 catene di subagent di Claude Code vengono rifiutate;
  - `dump opencode` richiede l'`opencode.exe` nativo (`--opencode-exe`) e `USERPROFILE`/`TEMP`
    senza prefisso `\\?\`. Il database di OpenCode si legge da una copia, mai dall'originale.
