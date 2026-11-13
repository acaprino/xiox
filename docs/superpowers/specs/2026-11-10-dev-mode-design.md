> Nota (2026-11-13): il prodotto ora si chiama **Xiox**. I nomi di questo documento (PulisciPC, `pulisci-pc`, `pc-rules`, i crate `dc-*`) sono quelli di allora; oggi sono `xiox-desktop` (binario `xiox-ui`), `xiox-rules` e `xiox-*` (`dc-app` -> `xiox-app`, ecc.). Il testo qui sotto non è stato aggiornato.

# PulisciPC: modalità Sviluppatori, regole Winapp2/CleanerML, copertura CCleaner

Data: 2026-11-10. Stato: **approvato** dall'utente il 2026-11-10, con il vincolo di mantenere tutto il layout di ccleaner64.

## Intento

Un solo programma di pulizia. PulisciPC (`D:\Projects\KKK\ccleaner64\app`, Rust + egui) oggi fa tre cose: file temporanei, registro, servizi e RAM. Assorbe devclean (`D:\Projects\KKK\devclean`, Tauri 2 + React + crate Rust) e aggiunge una **modalità Sviluppatori**: pulizia avanzata per chi sviluppa con agenti AI (Claude Code, Codex, OpenCode, Pi) e compila molto.

Perché egui e non Tauri: PulisciPC nasce per consumare poca RAM; un exe egui è leggero, WebView2 pesa 100-200 MB. I crate di devclean non dipendono da Tauri, quindi si riusano così come sono.

## Decisioni dell'utente (2026-11-10)

1. Si uniscono i due programmi; la base è ccleaner64, con il supporto dev dentro.
2. La modalità dev pulisce, oltre a quello che devclean già fa, le **sessioni archiviate degli harness**. Ogni harness (Codex via ChatGPT, Claude Code GUI, OpenCode/OpenChamber ecc.) permette di archiviare sessioni.
3. **Identificare le sessioni archiviate è compito di mukade**, non di codice nuovo in PulisciPC. Si usa mukade aggiornato, anche per trovare sessioni archiviate e worktree.
4. mukade i worktree li trova (dal `cwd` delle sessioni che ci hanno lavorato). In pratica: mukade dà i `cwd`; che un `cwd` sia un worktree collegato lo dice `dc-git`; i worktree creati dagli agenti li elenca `dc-mukade::managed_worktrees()` (codice di devclean, non di mukade).
5. Un **flag Cestino / definitivo** per la cancellazione.
6. **Via la dipendenza da WizTree.** Al suo posto, il nostro tool `D:\Projects\KKK\wiztree64` (egui, "albero-disco") usato come libreria di scansione e logica.
7. **Winapp2 nativo** (https://github.com/MoscaDotTo/Winapp2): caricato a runtime, aggiornato a ogni avvio se il remoto ha una versione nuova.
8. **Cleaner BleachBit (CleanerML) inglobati come configurazione nativa**, come le categorie di pulizia di Windows: quelli di `D:\Software\BleachBit\cleaners` diventano parte di PulisciPC, non un componente esterno.
9. **Copertura nativa di CCleaner**: PulisciPC deve pulire tutto quello che CCleaner pulisce nativamente come file (Pulizia personalizzata, schede Windows e Applicazioni).
10. **Si mantiene tutto il layout dell'app ccleaner64** (approvazione, 2026-11-10): barra laterale scura con le voci, card bianche, colori e costanti di `main.rs` (`SIDEBAR`, `ACCENT`, `CARD`, `BG`…), barra RAM, flusso Analizza → Avvia pulizia, finestra 1100×700. Le parti nuove (voce Sviluppatori, schede, categorie Winapp2/CleanerML) usano gli stessi componenti e lo stesso stile; nessun layout di devclean o wiztree64 viene portato.

## Obiettivi

- Quarta voce nella barra laterale: "Sviluppatori", accanto a Pulizia file, Registro, Servizi e RAM.
- Tre sezioni dentro la modalità:
  1. **Progetti**: quello che fa devclean oggi (artefatti rigenerabili: `target/`, `node_modules`, venv, cache Python, Gradle, Maven, .NET, CMake), per progetto, con ultimo uso dalle sessioni, stato git e motivo in chiaro.
  2. **Worktree**: worktree degli agenti inattivi, rimossi con `git worktree remove` + prune come fa già devclean (`PlanAction::GitWorktreeRemove`). *Scelta mia:* selezionabili solo quelli creati da un agente: `~/.codex/worktrees/*`, tabella `worktree`/`project.sandboxes` di OpenCode, `<repo>/.claude/worktrees/*` (da aggiungere a `managed_worktrees()` per Claude Code). Gli altri `cwd` di sessione che sono worktree collegati (anche fatti a mano) compaiono come informazione, non selezionabili. Un worktree compare solo in questa scheda, non anche in Progetti. La rimozione di un worktree è sempre definitiva (`dc-cleaner` ignora `RemoveMode` per `GitWorktreeRemove`, `crates/dc-cleaner/src/lib.rs:101-107`): spostarlo nel Cestino lascerebbe metadati git rotti al ripristino. La UI lo dice; i controlli git (sporco, commit non pushati) bloccano comunque la rimozione.
  3. **Sessioni archiviate**: elenco per harness delle sessioni che mukade marca `archived: true`, con titolo, progetto (`cwd`), data, peso; selezione e cancellazione.
- Stesso schema di sicurezza di devclean: analizza → anteprima → esegui con lo stesso codice; il backend accetta solo percorsi/id trovati dall'ultima analisi.
- Flag Cestino / definitivo in Impostazioni, valido per artefatti e sessioni (i worktree sono sempre definitivi, vedi sopra). *Scelta mia:* default Cestino. Nota per la UI: il Cestino non libera spazio finché non si svuota, e la categoria "Cestino" di Pulizia file (attiva di default, `src/cleaner.rs:42`) lo svuota alla pulizia successiva: la scheda lo scrive accanto al flag. devclean aveva default definitivo; qui si preferisce il recupero possibile.

## Non obiettivi

- Nessun lettore di sessioni nuovo in PulisciPC. Se un harness o una GUI non viene riconosciuto come archiviato, si aggiunge in mukade upstream (`D:\Projects\mukade`) e si rifà il vendor.
- Nessuna scrittura SQL diretta nei database degli harness (`~/.codex/state_5.sqlite`, `opencode*.db`).
- Registro e Servizi e RAM non cambiano. Pulizia file si estende (Fasi 2 e 3: nuove categorie, flag Cestino/definitivo, codice nativo nuovo) ma le 9 categorie di oggi restano com'erano.
- L'app Tauri devclean non viene più sviluppata; la cartella resta com'è come riferimento.

## Anti-obiettivi (fallimento anche se "funziona")

- Cancellare una sessione **non** archiviata, una sessione con processo vivo (`live: true`), o credenziali/impostazioni/memorie degli agenti.
- Corrompere il database di un harness.
- Bloccare la UI per i ~60 s della scansione sessioni.
- Far crescere la RAM a riposo di PulisciPC oltre il necessario (i dati della scansione si liberano quando si esce dalla modalità o si rianalizza).

## Vincoli

- Windows 11, Rust stabile, egui/eframe 0.33 (già in uso).
- mukade si usa **vendored** (compilato dentro l'exe, nessun binario esterno, nessuna rete), come in devclean (`docs/reference/devclean-mukade-VENDORED.md`). Vendor dal commit `dfde57e` di `D:\Projects\mukade` (master del 2026-11-10, include `0dd0498` "--archived" e la lettura di `archived_sessions/` di Codex). Solo codice committato: il working tree upstream può avere modifiche in corso di un'altra sessione.
- `rusqlite = "=0.40.2"` bundled, stessa versione in mukade e nei crate (due versioni di `libsqlite3-sys` non convivono).
- Nessun eseguibile esterno per la scansione disco: niente WizTree64.exe, niente export CSV, niente nota di licenza.
- Funzioni distruttive su servizi/registro richiedono già admin; la modalità dev no (lavora nel profilo utente). Da admin la scansione usa la MFT (più veloce), senza admin cammina le cartelle.

## Come è fatto

### Struttura

`ccleaner64/app` diventa un workspace Cargo:

```
ccleaner64/app/
  Cargo.toml          workspace + pacchetto pulisci-pc (bin PulisciPC)
  src/                app egui: main.rs, cleaner.rs, registry.rs, services.rs, sys.rs, + dev/ (nuovo)
  crates/dc-*         copiati da devclean/crates (model, scanner, git, cleaner, engine, mukade); dc-wiztree NO
  crates/dc-disktree  nuovo: libreria di scansione estratta da wiztree64/app/src (mft.rs, scan.rs, model.rs + le parti di sys.rs che servono)
  vendor/mukade       vendored da D:\Projects\mukade @ dfde57e (stesse regole di VENDORED.md)
  docs/
```

### Scansione disco senza WizTree

`dc-wiztree` (che lanciava `WizTree64.exe` ed esportava un CSV) sparisce. Al suo posto `dc-disktree`, la parte non grafica di wiztree64:

- lettura MFT NTFS (`mft.rs`: `\\.\X:` in sola lettura, `FSCTL_GET_NTFS_VOLUME_DATA`, record da 1 KiB, letture da 128 KiB) quando PulisciPC è admin e il volume è NTFS; altrimenti walk parallelo delle cartelle (`scan.rs::walk_scan`, rayon);
- albero compatto (`model.rs`: `Raw` → `Tree` con dimensione, allocato, mtime, conteggi per cartella);
- avanzamento e annullamento (`scan.rs::Progress`).

`dc-disktree` implementa il trait già esistente `DirIndex` di `dc-model` (`backend_name`, `size_of`, `find_dirs_named`), quindi `dc-scanner` e `dc-engine` non cambiano. Niente treemap né UI di wiztree64 dentro PulisciPC.

*Proposta mia:* oggi WizTree serviva solo per le cartelle aggiuntive, e i ~60 s dell'analisi dev sono quasi tutti misura delle dimensioni degli artefatti (`docs/reference/devclean-DESIGN.md`). Da admin, una sola lettura MFT del volume dà le dimensioni di tutti gli artefatti via `size_of`, anche per i progetti trovati dalle sessioni. Senza admin si misura come oggi.

Cosa non si porta in PulisciPC (devclean resta com'è, non si cancella nulla lì): `dc-wiztree`, `Settings.use_wiztree`, `Settings.wiztree_path` (un file impostazioni vecchio che li contiene deve ancora caricarsi), `scripts/fetch-wiztree.ps1`, `src-tauri/resources/wiztree/`, la nota di licenza WizTree.

wiztree64 è un progetto in corso (stato "building", senza git): *scelta mia*, si copia all'inizio della Fase 1 e in `crates/dc-disktree/VENDORED.md` si scrivono data, ora e SHA-256 di ogni file copiato, così si sa da quale stato viene. La cartella `D:\Projects\KKK\wiztree64` non si tocca.

RAM: `mft.rs` tiene un record per ogni voce della MFT con il nome, poi un `Vec<RawEntry>`; su tutto C: sono centinaia di MB. *Scelta mia:* `dc-disktree` riusa il lettore MFT ma costruisce solo ciò che serve a `DirIndex` (cartelle con dimensione aggregata; i nomi dei file si scartano subito), e il risultato si libera finita l'analisi. Limite: picco sotto 300 MB su C:, misurato; se non ci sta, la MFT si usa solo per le cartelle aggiuntive e le dimensioni degli artefatti si misurano come oggi.

La colla oggi in `devclean/src-tauri/src/lib.rs` (comandi `scan`, `build_plan`, `execute_plan`, impostazioni) diventa un modulo Rust normale chiamato dalla UI egui tramite thread + `mpsc`, lo stesso schema che `main.rs` usa per Pulizia file e Registro (`CleanMsg`, `RegMsg`).

### Fonte dei dati

Tutto ciò che riguarda gli harness passa da mukade, attraverso `crates/dc-mukade`:

- progetti e ultimo uso: come oggi (`SessionRecord`, `dc-scanner/src/sessions.rs`);
- sessioni archiviate: `SessionInfo { id, title, cwd, updated, archived, path, unreadable, live }` filtrate su `archived == true` (equivalente di `mukade list <harness> --archived true` meno quelle con `--archived false`). `dc-mukade` oggi aggrega per progetto e perde i singoli id: va esteso per esporre anche l'elenco delle singole sessioni archiviate;
- worktree: `cwd` delle sessioni (mukade) + `dc-git` per riconoscere un worktree collegato + `managed_worktrees()` in `dc-mukade`.
- elenco sessioni archiviate: `Reader::list` (sole radici), come `mukade list`; il conteggio 664 Codex è di `list`.

Misura sulla macchina dell'utente (2026-11-10, mukade `dfde57e`): Claude Code 352 sessioni / 0 archiviate / 8 cwd in worktree; Codex 1107 / **664** / 5; Pi 113 / 0 / 0; OpenCode non misurato (189 sessioni nei due db, 0 archiviate). Claude Code e Pi risultano 0 perché mukade dice che i loro store non registrano l'archiviazione: la Claude Code GUI va prima capita e aggiunta in mukade (vedi "Aperto").

### Cancellazione delle sessioni archiviate

Il flag Cestino/definitivo è `RemoveMode::{Recycle, Permanent}` già in `dc-model`.

| Harness | Come si cancella | In modalità Cestino |
|---|---|---|
| Codex | `codex delete --force <id>` (senza `--force` rifiuta fuori da un terminale). Verificato il 2026-11-10 su una **copia** di `state_5.sqlite` in un `CODEX_HOME` di prova: una riga archiviata senza rollout viene tolta (`Deleted session …`, exit 0, archiviate 3802 → 3801) | una copia del rollout (se c'è) va nel Cestino (copia temporanea spostata nel Cestino), poi `codex delete --force`; se la delete fallisce la sessione resta com'era e la copia nel Cestino è solo un doppione |
| OpenCode | `opencode --standalone session delete <id>` (come fa mukade: `--standalone`; toglie anche le sessioni figlie) | prima `opencode --standalone session export <id>` in un file JSON che va nel Cestino, poi `delete` |
| Claude Code, Pi | il file indicato da `path`, solo se è il transcript della sessione stessa (`.jsonl` che esiste, sotto `projects/` o `sessions/` dello store). Mai `history.jsonl`, `sessions/<pid>.json`, un file `.db`, né un `path` che non esiste | il file va nel Cestino |

Solo sessioni radice (`Reader::list`, non `list_all`): una radice archiviata con una figlia non archiviata o `live` si salta, con il motivo. Le sessioni `unreadable` che stanno in un altro `opencode*.db` si saltano (la CLI di OpenCode lavora solo sul proprio database e migra quelli che apre).

Si usa la CLI dell'harness per gli store sqlite perché conosce il proprio schema e i propri lock; PulisciPC non scrive mai nei database. Se la CLI non si trova o fallisce, la sessione resta e l'esito lo dice. Non si cancella mai una sessione con `live: true` né una che mukade non marca archiviata. Le 664 archiviate Codex sulla macchina dell'utente non hanno più il rollout: cancellarle libera solo righe del database.

### UI

Stile e colori di `main.rs` (barra laterale scura, card bianche). Nella modalità: tre schede (Progetti, Worktree, Sessioni archiviate), pulsante "Analizza", barra di avanzamento, elenco con caselle, totale selezionato, "Anteprima" → dialogo con l'elenco esatto → "Pulisci". Impostazioni: giorni di inattività (default 14), cartelle aggiuntive (spente), Cestino/definitivo.

## Regole di pulizia esterne: Winapp2 e CleanerML

Un nuovo crate `pc-rules` legge due formati di regole della comunità e li porta in un solo modello, che la scheda **Pulizia file** mostra accanto alle categorie già presenti (`cleaner.rs::CATEGORIES`). Stesso schema: Analizza → anteprima → pulizia, stesso flag Cestino/definitivo.

Modello comune: `Cleaner { id, label, origine (Winapp2 | CleanerML <file>), gruppo, rilevato: bool, opzioni: [Opzione { id, label, descrizione, avviso, azioni: [Azione] }] }`. Azioni supportate: cancella file/cartelle (con pattern e ricorsione), cancella chiave/valore di registro. Un cleaner non rilevato sul PC non compare. Tutte le opzioni sono spente di default; quelle con avviso chiedono conferma.

### Winapp2

- File: `Non-CCleaner/Winapp2.ini` del ramo `master` (la variante per programmi diversi da CCleaner). Misura 2026-11-10: `; Version: 260915`, 4068 voci, 1.8 MB; chiavi `FileKey` 14121, `RegKey` 3677, `DetectFile` 3322, `Detect` 1444, `ExcludeKey` 86, `Warning` 5; flag `RECURSE` / `REMOVESELF`; variabili `%LocalAppData%`, `%AppData%`, `%ProgramFiles%` (sia x64 che x86), `%ProgramData%`, `%UserProfile%`, `%WinDir%`, `%SystemDrive%`, `%CommonProgramFiles%`, `%Public%`, `%Temp%`, `%LocalLowAppData%`, `%UserName%`, `%Documents%`. Sintassi di riferimento: README del repo.
- Caricato a runtime da `%APPDATA%\PulisciPC\winapp2\Winapp2.ini`, mai compilato nell'exe.
- A ogni avvio, in sottofondo (la UI non aspetta): GET di `https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini` con `If-None-Match` sull'ETag salvato. 304 → niente. 200 → il file nuovo si analizza; se è valido (riga `; Version:`, almeno qualche migliaio di voci) sostituisce il vecchio in modo atomico e si ricarica; se no si tiene il vecchio. User-Agent generico (`PulisciPC`), nessun dato dell'utente.
- Offline al primo avvio: la sezione Winapp2 dice "non ancora scaricato" e il resto funziona. *Scelta mia:* nessuna copia inclusa nell'exe.
- Licenza CC-BY-SA-4.0: attribuzione in "Informazioni" e in `THIRD_PARTY_NOTICES.md`.

### CleanerML (BleachBit), inglobati

- Gli 11 XML di `D:\Software\BleachBit\cleaners` (`ai_claude_code`, `ai_codex`, `ai_gemini_cli`, `ai_opencode`, `bbag_cargo`, `bbag_crash`, `bbag_go`, `bbag_jetbrains`, `bbag_npm`, `bbag_pip`, `bbag_shader`) si copiano nel progetto (`ccleaner64/app/cleaners/*.xml`) e si compilano dentro l'exe (`include_str!`). PulisciPC non legge `D:\Software\BleachBit` a runtime e funziona senza BleachBit installato.
- Nella scheda Pulizia file compaiono come categorie native, allo stesso livello di quelle di Windows (Cestino, Temp, dump ecc.), nel gruppo "Sviluppo e AI", con etichette e avvisi in italiano corretto: *scelta mia*, nella copia inglobata si tolgono i prefissi "AGG -", si traducono le etichette inglesi ("Rust Cargo downloads", "Go build cache", "pip package cache", "Cache shader GPU") e si rimettono gli accenti ("sarà più", "possibilità"). Nessuna dicitura "BleachBit" nella UI; l'origine resta in `THIRD_PARTY_NOTICES.md` se i file derivano da BleachBit.
- Decisione dell'utente (2026-11-10): tutte le opzioni dei cleaner AI restano, **spente di default e con il loro avviso**, anche quelle che cancellano sessioni non archiviate (`ai_claude_code` transcripts e prompt_history, `ai_codex` sessions, archived e log_db, `ai_opencode` legacy_sessions). L'anti-obiettivo "mai cancellare una sessione non archiviata" vale per la modalità Sviluppatori, non per un'opzione che l'utente spunta da sé in Pulizia file. Un'opzione con `<warning>` chiede conferma mostrando l'avviso; `<running>` la blocca se l'harness è aperto.
- L'XML resta la fonte: si interpreta con il motore CleanerML di `pc-rules`, non si riscrive a mano in Rust. Aggiungere una categoria nativa = aggiungere un XML in `cleaners/` e ricompilare.
- *Scelta mia:* niente caricamento di cartelle CleanerML esterne in v1. Il `winapp2.ini` che sta in `D:\Software\BleachBit\cleaners` (variante BleachBit) non si usa: Winapp2 arriva dal remoto.
- Solo cleaner e azioni con `os="windows"` o senza `os`.
- Supportati: `<cleaner>`, `<label>`, `<description>`, `<option>`, `<warning>`, `<var>`/`$$nome$$`, `<running>` (se il processo è attivo l'opzione è bloccata con il motivo), `command="delete"` con `search` = `file`, `glob`, `walk.all`, `walk.files`, `walk.top`, e `regex`/`nregex`/`type`; `command="winreg"`.
- Non supportati in v1 (l'opzione compare disattivata con "non supportato"): `sqlite.vacuum`, `json`, `ini`, `cookie`, `chrome.*`, `mozilla.*`, `office_registrymodifications`, `deep`, `process`, `win.shell.change.notify`. Sui 65 ufficiali + 11 utente: `delete` 1185 azioni, `winreg` 119, `sqlite.vacuum` 227, `json` 64, il resto sotto 30.

### Sicurezza delle regole esterne

- *Scelta mia:* le regole di Pulizia file hanno una lista protetta loro, non quella di devclean (`crates/dc-cleaner/src/guard.rs` rifiuta tutto `%WinDir%`, `%ProgramFiles%`, `%ProgramData%` e le cartelle `.claude`/`.codex`/`.config`/`appdata`, e scarterebbe gran parte di Winapp2). Mai: la radice di un disco; `%WinDir%`, `%ProgramFiles%`, `%ProgramData%`, `%UserProfile%`, `%AppData%`, `%LocalAppData%` presi interi (sottocartelle sì); `System32`/`SysWOW64`/`WinSxS` e il loro contenuto; qualsiasi `.git`; file di credenziali degli agenti (`.credentials.json`, `auth.json`, `credential` nei db). Una regola che punta lì viene scartata e segnalata. La lista di devclean resta per la modalità Sviluppatori.
- Le cancellazioni di registro fanno prima un backup `.reg` come il modulo Registro (`registry.rs`).
- L'anteprima elenca i file reali trovati, non le regole.

## Copertura nativa di CCleaner

Obiettivo: ogni voce di pulizia file che CCleaner offre nativamente (Pulizia personalizzata → Windows e Applicazioni) ha un equivalente in PulisciPC. Decisione dell'utente (2026-11-10): **tutto l'elenco**, anche le voci per programmi non installati su questo PC. Oggi PulisciPC ne ha 9 (`cleaner.rs:42-50`: Cestino, Temp utente, Temp Windows, Dump di memoria, Log di Windows, Segnalazione errori, Miniature, Prefetch, Ottimizzazione recapito).

1. **Inventario dall'originale.** Le voci si leggono da CCleaner sulla macchina dell'utente (`C:\Users\alfio\Desktop\CCleaner\CCleaner64.exe` e la tabella stringhe italiana `Lang\lang-1040.dll`, 3356 stringhe), più quello che l'indagine di ccleaner64 ha già trovato (`ccleaner64/investigation.cany/investigation/findings.md`, fnd_2). Esempi già visti nelle stringhe: Appunti, Cache DNS, Frammenti file di chkdsk, Documenti recenti, Esegui (nel Menu Start), Collegamenti nel menu Start, Collegamenti sul Desktop, Cronologia assistenza utente, Cache MUI, Cache Web di Windows, Password salvate, Compatta database, Cookie (con "Cookie da conservare"), Pulisci spazio libero, più le voci per browser e applicazioni.
2. **Tabella di copertura** in `docs/ccleaner-coverage.md`: per ogni voce, come la copre PulisciPC. Le possibilità, in ordine di preferenza:
   - categoria nativa già esistente;
   - una voce Winapp2 della variante base (`Non-CCleaner/Winapp2.ini` contiene apposta anche le regole che CCleaner fa da sé);
   - un CleanerML inglobato (nuovo XML in `cleaners/`, se basta cancellare file o chiavi);
   - codice nativo nuovo in `cleaner.rs`, per ciò che non è "cancella file": Appunti, Cache DNS (`DnsFlushResolverCache`), Registri eventi, liste MRU nel registro, compattazione dei database dei browser, cookie con lista da conservare, frammenti chkdsk, spazio libero;
   - non fattibile, con il motivo.
3. **Implementazione** di quello che la tabella assegna a codice nativo o a nuovi XML. Le voci che CCleaner spegne di default restano spente anche qui (password salvate, spazio libero, cookie).

Verifica: la tabella non ha righe vuote; per ogni riga "coperta" l'anteprima di PulisciPC trova qualcosa sul PC dell'utente quando lo trova anche CCleaner (confronto a campione su 10 voci, CCleaner in sola analisi).

## Fasi

Tutto sta nello stesso documento, ma si costruisce in tre blocchi, ognuno con la sua verifica:

1. **Fase 1, modalità Sviluppatori**: workspace unito, `dc-disktree` al posto di WizTree, mukade `dfde57e`, schede Progetti / Worktree / Sessioni archiviate. Stima: 5 ore circa.
2. **Fase 2, regole**: `pc-rules`, Winapp2 con aggiornamento all'avvio, CleanerML inglobati. Stima: 4 ore circa.
3. **Fase 3, copertura CCleaner**: inventario completo, tabella, voci native mancanti. Stima: 6-8 ore, dipende da quante voci restano a codice nativo dopo Winapp2.

La Fase 3 dipende dalla 2 (Winapp2 copre gran parte delle voci). La Fase 1 è indipendente.

## Lasciato al builder

- Come rendere in egui le componenti React (`ProjectRow`, `PreviewDialog`, `SettingsDialog`), purché ci siano le stesse informazioni.
- Dove salvare impostazioni e cache mukade (accanto all'exe o in `%APPDATA%\PulisciPC`).
- Gestione del timeout delle CLI degli harness.
- Struttura interna del modulo `src/dev/`.

## Aperto

- Dove la **Claude Code GUI** (`%LOCALAPPDATA%\com.claude-code-gui.app`, solo WebView) salva le archiviate: non trovato. Va aggiunto in mukade upstream quando si sa; PulisciPC lo prende al prossimo vendor.
- OpenChamber usa OpenCode (`~/.config/openchamber/managed-opencode`, oggi vuoto): coperto da OpenCode se usa lo stesso db.

## Verifica

- `cargo test --workspace` (escluso mukade vendored) verde, inclusi i test esistenti dei crate dc-*.
- Smoke su dati reali: la scheda Sessioni archiviate mostra 664 Codex (numero del 2026-11-10, può cambiare); Progetti ≈ 111 come devclean.
- `dc-disktree`: test sullo stesso albero di prova del walker; da admin, le dimensioni da MFT di alcuni `target/` reali coincidono con quelle del walker (entro l'arrotondamento al cluster); tempo dell'analisi dev da admin misurato contro i 60-64 s di devclean.
- Nessun riferimento a WizTree rimasto (`grep -ri wiztree` vuoto fuori dalla storia).
- Winapp2: primo avvio online scarica e carica; secondo avvio riceve 304 e non riscarica; con un ETag falso riscarica; offline parte senza errori. Parser: le 4068 voci di `260915` si leggono senza errori; i cleaner rilevati sul PC sono un numero plausibile (decine, non migliaia).
- CleanerML: gli 11 XML inglobati si caricano dall'exe (anche rinominando `D:\Software\BleachBit`); `bbag_cargo` "Download compressi Cargo" in anteprima elenca i file di `%UserProfile%\.cargo\registry\cache`; con `cargo.exe` attivo l'opzione è bloccata.
- Prova di cancellazione su una sessione Codex archiviata di test, in entrambe le modalità, con verifica che `mukade list codex` non la elenchi più e (Cestino) che il file sia nel Cestino.
- Build release, RAM a riposo misurata prima/dopo l'analisi dev, screenshot delle tre schede.
