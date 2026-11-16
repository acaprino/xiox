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
- Limiti di Mukade trovati il 2026-11-15:
  - `list pi` dà lo stesso id `session` a 50 sessioni;
  - 7 catene di subagent di Claude Code vengono rifiutate;
  - `dump opencode` richiede l'`opencode.exe` nativo (`--opencode-exe`) e `USERPROFILE`/`TEMP`
    senza prefisso `\\?\`. Il database di OpenCode si legge da una copia, mai dall'originale.
