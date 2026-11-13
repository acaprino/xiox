> Nota (2026-11-13): il prodotto ora si chiama **Xiox**. I nomi di questo documento (PulisciPC, `pulisci-pc`, `pc-rules`, i crate `dc-*`) sono quelli di allora; oggi sono `xiox-desktop` (binario `xiox-ui`), `xiox-rules` e `xiox-*` (`dc-app` -> `xiox-app`, ecc.). Il testo qui sotto non è stato aggiornato.

# Fase 3a, copertura di CCleaner: nuovi XML, azioni di sistema e database dei browser. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** PulisciPC pulisce tutto ciò che CCleaner pulisce nativamente e che la tabella di copertura assegna a «Nuovo XML» o «Codice nativo nuovo», tranne le voci rinviate alla Fase 3b (PATH, Windows.old, password di rete, spazio libero).

**Architecture:** Due XML nuovi (`windows_extra.xml`, `safari.xml`) inglobati come i 59 esistenti. Un meccanismo nuovo di «voci native» (`src/native.rs`: struct con `analyze`/`clean`) per ciò che non è «cancella file o chiavi»: `src/sysclean.rs` (Appunti, DNS, registri eventi, collegamenti non validi) e `src/browserdb.rs` (cookie con lista, compattazione, cronologia Firefox, ultima cartella di download). `src/rules.rs` e `src/main.rs` le mostrano e le eseguono come le opzioni di Fase 2. Ogni accesso al sistema passa da un trait iniettabile: i test non toccano il PC.

**Tech Stack:** Rust stabile, `windows-sys 0.61` (feature da aggiungere), `rusqlite =0.40.2` con `bundled` (la versione di `crates/dc-mukade/Cargo.toml:14`), `pc-rules` (Fase 2), `egui 0.33`.

**Spec:** `docs/superpowers/specs/2026-11-10-dev-mode-design.md` (sezione «Copertura nativa di CCleaner») e **`docs/ccleaner-coverage.md`** (la tabella: ogni task rimanda alle sue righe e alle proposte N1-N12 della §2.6; leggerla per intero prima di partire).

**Decisioni prese al posto dell'utente (2026-11-11, modificabili):** (1) questo piano è la **v1 = task 1-3 della §4.3** (22 h); PATH (N5), Windows.old (N7), password di rete (N8), spazio libero (N6) vanno in un piano 3b solo se l'utente li vuole. (2) I default di PulisciPC già esistenti non cambiano (punto aperto 5). (3) L'exe di CCleaner non c'è: le celle «da verificare» della tabella restano tali e le voci con default incerto partono **spente**. (4) I nomi italiani sono quelli della tabella.

## Global Constraints

- Windows 11, app sempre da amministratore (build di debug `asInvoker`); testo UI in italiano corretto, mai «BleachBit» né «AGG -»; **layout di ccleaner64 intatto** (barra, `card()`, `primary()`, `secondary()`, `src/theme.rs`).
- La app propone, l'utente sceglie: nessuna voce preselezionata oltre al default di CCleaner/della tabella; tutto ciò che è distruttivo (registri eventi, cookie, cronologia, collegamenti) **spento** e con `warning` nella finestra di conferma.
- File e cartelle si cancellano solo con `pc_rules::delete_checked_with`; chiavi di registro solo con `reg_allowed` + `REG_OPTION_OPEN_LINK` e backup `.reg` di `src/regbackup.rs`.
- Test **mai sul PC dell'utente**: alberi finti in `tempfile::tempdir()`, trait finti per Appunti, DNS, registri eventi, COM, processi, registro. Database SQLite solo su file creati nel test.
- Browser aperto = nessuna scrittura sui suoi file: ogni voce dei database ha `running` (processi del browser) e `rules::blocked` la disattiva.
- Un solo `Guard` per pulizia; `cancel: &AtomicBool` controllato prima di ogni passo; nessun lavoro lungo nel thread della UI.
- Dopo ogni file nuovo in `cleaners/`: aggiungerlo a `crates/pc-rules/src/embedded.rs` e aggiornare i conteggi dei test.
- Il working tree è condiviso con un'altra sessione (piano 1 del catalogo, Task 11 riscrive `src/cleaner.rs` e parte di `src/main.rs`): commit solo con i percorsi del task, mai `crates/dc-*`; `cargo ... -j 2` (con più compilazioni in parallelo il paging file si riempie, errore 1455).
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; `git -c user.name="Alfio Caprino" -c user.email="alfio.caprino@gmail.com"`.

## Review Focus

1. **Browser aperto mentre si toccano i suoi database** (cookie, `places.sqlite`, `Preferences`): non si apre il file, la voce è disattivata con «Chiudi <browser> per pulire». Test in Task 3 con un processo finto attivo.
2. **Segnalibri persi dalla cancellazione della cronologia di Firefox**: il numero di `moz_bookmarks` dopo la pulizia è uguale a prima, altrimenti `ROLLBACK`. Test in Task 3 su un `places.sqlite` finto con segnalibri.
3. **Cookie: lista assente, vuota, con righe sporche** (spazi, maiuscole, `.dominio`, commenti `#`): lista assente o vuota = nessun dominio protetto e lo dice la finestra di conferma («lista: 0 domini»); mai un panico. Test in Task 3.
4. **Appunti tenuti aperti da un'altra app**: al massimo 3 tentativi da 100 ms, poi «Appunti occupati» come errore della voce, mai un blocco della UI. Test in Task 2.
5. **Collegamenti non validi su dischi staccati o di rete**: un `.lnk` la cui destinazione sta su un volume non fisso, su UNC o non risolvibile **non si propone mai**. Test in Task 2.

---

### Task 1: nuovi XML e ritocchi alle categorie esistenti

Stima: 4 h.

**Files:**
- Create: `cleaners/windows_extra.xml`, `cleaners/safari.xml`, `crates/pc-rules/tests/phase3_xml.rs`
- Modify: `crates/pc-rules/src/embedded.rs` (elenco e conteggio 59 → 61; gruppo per cleaner), `src/cleaner.rs` (solo `targets()`: i ritocchi di §4.1 della tabella), `src/rules.rs` (raggruppamento per `cleaner.group`)

**Interfaces:**
- Consumes: le righe con copertura «Nuovo XML» di `docs/ccleaner-coverage.md` (§2 e §3.1; 16 opzioni per `windows_extra.xml`, 4 per `safari.xml`) e i ritocchi della §4.1; formato CleanerML già supportato da `pc_rules::cleanerml` (stessi attributi dei 59 esistenti: `delete` con `search`, `winreg`, `<running>`, `<warning>`, `<var>`).
- Produces: `embedded()` restituisce 61 cleaner; `Cleaner.group` è `"Windows"` per `windows_extra` e `"Browser"` per `safari`, `"Sviluppo e AI"` per gli altri; in `src/rules.rs` i gruppi si mostrano nell'ordine `Windows`, `Browser`, `Sviluppo e AI`, `Applicazioni`.

- [ ] **Step 1: Test che falliscono** (`crates/pc-rules/tests/phase3_xml.rs` e `embedded.rs`):
  - `every_phase3_xml_parses_with_zero_warnings` (due file, 20 opzioni in tutto)
  - `phase3_options_are_off_unless_ccleaner_default_on` (legge il default dalla tabella: nell'XML `default_on` è sempre `false`, come tutti gli embedded: il test lo afferma)
  - `registry_options_stay_inside_hkcu_software` (nessuna `RegKey` esce da `Software`; `reg_allowed` accetta ogni azione del file)
  - `all_embedded_cleaners_parse` aggiornato a 61 e `xml_structure_unchanged` invariato per i primi 11.
- [ ] **Step 2:** FAIL. **Step 3:** scrivere i due XML riga per riga dalla tabella (una `<option>` per riga, etichetta italiana della tabella, `<warning>` dove la colonna «Rischio» lo chiede, `<running>` dove serve), aggiungerli a `CLEANERS` con il gruppo, e applicare i ritocchi di §4.1 a `targets()` (miniature `ThumbCacheToDelete\thm*.tmp`, log `%WinDir%\*.bak`, `*log.txt`, `System32\wbem\Logs`: **solo se** `guard` le accetta: altrimenti si annota «non coperto» nella tabella e non si forza). **Step 4:** `cargo test -p pc-rules -j 2` e `cargo test -p pulisci-pc -j 2` verdi; screenshot con profilo finto (`--tab cleaner --screenshot`, variabili `USERPROFILE`/`LOCALAPPDATA`/`APPDATA`/`TEMP` verso una cartella temporanea): gruppo «Windows» con le voci presenti nel profilo finto.
- [ ] **Step 5:** Commit `feat(cleaners): voci extra di Windows e Safari inglobate`.

### Task 2: voci native e azioni di sistema (Appunti, DNS, registri eventi, collegamenti non validi)

Stima: 6 h.

**Files:**
- Create: `src/native.rs`, `src/sysclean.rs`
- Modify: `src/main.rs` (mod, messaggio, tabelle, conferma), `src/rules.rs` (selezione `native:<id>`, `blocked` per le voci native), `Cargo.toml` (feature `Win32_System_DataExchange`, `Win32_System_EventLog`, `Win32_System_Com`, `Win32_UI_Shell`, `Win32_System_LibraryLoader` di `windows-sys`)

**Interfaces:**
- Produces in `src/native.rs`:
  ```rust
  pub struct NativeCtx<'a> { pub env: &'a pc_rules::Env, pub cancel: &'a AtomicBool }
  #[derive(Default, Clone)]
  pub struct NativeFound { pub detail: Vec<String>, pub count: u64, pub bytes: u64, pub skipped: u64, pub items: Vec<String> }
  #[derive(Default, Clone)]
  pub struct NativeCleaned { pub freed: u64, pub removed: u64, pub failed: u64, pub notes: Vec<String> }
  pub struct NativeItem {
      pub id: &'static str, pub group: &'static str, pub label: &'static str,
      pub description: &'static str, pub warning: Option<&'static str>,
      pub default_on: bool, pub needs_admin: bool, pub running: &'static [&'static str],
      pub analyze: fn(&NativeCtx) -> NativeFound,
      pub clean: fn(&NativeCtx, &NativeFound) -> NativeCleaned,
  }
  pub fn all() -> Vec<NativeItem>;      // sysclean::items() + browserdb::items()
  ```
  `clean` agisce solo su `found.items` (ciò che l'analisi ha trovato nello stesso job), mai su altro.
- Produces in `src/sysclean.rs`: `pub fn items() -> Vec<NativeItem>` con gli id `clipboard`, `dns_cache`, `event_logs`, `broken_shortcuts` (gruppo `"Windows"`), e i trait iniettabili `trait Clipboard { fn count(&self) -> u32; fn empty(&self) -> Result<(), String>; }`, `trait Dns { fn flush(&self) -> Result<(), String>; }`, `trait EventLog { fn channels(&self) -> Vec<(String, u64)>; fn clear(&self, channel: &str) -> Result<(), String>; }`, `trait Shortcuts { fn list(&self, dirs: &[PathBuf]) -> Vec<PathBuf>; fn target(&self, lnk: &Path) -> Option<PathBuf>; fn volume_is_fixed(&self, p: &Path) -> bool; }` con le implementazioni vere (`RealClipboard`…) usate da `items()` e quelle finte nei test.
- Default e avvisi dalla §2.6 della tabella: Appunti acceso, Cache DNS spenta, Registri eventi spenti con `warning`, Collegamenti non validi spenti con `warning`.

- [ ] **Step 1: Test che falliscono** (`src/sysclean.rs`, `src/native.rs`):
  - `clipboard_retries_three_times_then_reports_busy` (finto occupato due volte poi libero → `removed == 1`; sempre occupato → `failed == 1` e nota «Appunti occupati»; mai più di 3 tentativi)
  - `clipboard_is_not_listed_when_empty` (`count() == 0` → `NativeFound.count == 0`)
  - `dns_flush_falls_back_when_export_is_missing` (primo metodo `Err`, secondo `Ok` → `removed == 1`)
  - `event_logs_continue_after_a_channel_fails` (3 canali, il secondo fallisce: `removed == 2`, `failed == 1`, una nota con il nome del canale)
  - `event_logs_are_never_cleared_by_analysis` (`analyze` non chiama `clear`)
  - `shortcut_to_missing_target_on_fixed_volume_is_listed` (tempdir: `.lnk` finto con destinazione inesistente su volume fisso → proposto)
  - `shortcut_to_unc_network_or_removable_target_is_never_listed` (UNC, volume non fisso, destinazione non risolvibile → non proposto)
  - `clean_acts_only_on_found_items` (un `NativeFound` forzato con un percorso fuori dalle cartelle ammesse non cancella nulla: ogni `.lnk` passa da `delete_checked_with`)
  - `native_item_is_blocked_while_its_process_runs` (in `rules.rs`, con `running` e un elenco processi finto)
- [ ] **Step 2:** FAIL. **Step 3:** implementare `native.rs` e `sysclean.rs`: Appunti con `OpenClipboard`/`EmptyClipboard`/`CloseClipboard` e `CountClipboardFormats`; DNS con `DnsFlushResolverCache` caricata con `GetProcAddress` da `dnsapi.dll`, ripiego `%SystemRoot%\System32\ipconfig.exe /flushdns` per percorso assoluto; registri eventi con `EvtOpenChannelEnum`/`EvtNextChannelPath`/`EvtClearLog` (nessun export in v1: l'avviso lo dice); collegamenti con `IShellLinkW::GetPath` (`SLGP_RAWPATH`) e `GetDriveTypeW == DRIVE_FIXED`, cartelle Start Menu e Desktop (utente e pubblico), destinazione «assente» solo per `ErrorKind::NotFound` (mai `Path::exists`). In `rules.rs`/`main.rs`: le voci native compaiono come caselle sotto il titolo del loro `group`, selezione `native:<id>`, righe nelle stesse tabelle di analisi e pulizia (`NativeFound.detail` come testo), `warning` nel `Modal`, `needs_admin` con 🛡 come le categorie statiche. **Step 4:** `cargo test -p pulisci-pc -j 2` verde; clippy senza nuovi avvisi; screenshot con profilo finto: le quattro voci nel gruppo «Windows».
- [ ] **Step 5:** Commit `feat(cleaner): voci native di sistema (Appunti, DNS, registri eventi, collegamenti non validi)`.

### Task 3: database dei browser (cookie, compattazione, Firefox, ultima cartella di download)

Stima: 12 h.

**Files:**
- Create: `src/browserdb.rs`
- Modify: `src/native.rs` (`all()`), `src/main.rs` (pulsante «Modifica lista cookie»), `Cargo.toml` (`rusqlite = { version = "=0.40.2", features = ["bundled"] }`, `serde_json = "1"`)

**Interfaces:**
- Consumes: `NativeItem`, `NativeCtx`, `NativeFound`, `NativeCleaned` (Task 2); l'elenco dei browser e dei percorsi dei profili dalla tabella (§3.1 e N9-N12 di §2.6): famiglia Chromium (11 browser) e Gecko (Firefox, Thunderbird).
- Produces: `pub fn items() -> Vec<NativeItem>` con gli id `cookies_keep_list`, `compact_databases`, `firefox_history`, `last_download_dir` (gruppo `"Browser"`), `running` = processi del browser di quel profilo; `pub fn profiles(env: &Env) -> Vec<Profile>` con `Profile { browser: &'static str, kind: Kind, dir: PathBuf }`, `Kind::{Chromium, Gecko}`; `pub fn parse_keep_list(text: &str) -> Vec<String>`; `pub fn keep_list_path(env: &Env) -> PathBuf` (`Documenti\PulisciPC\cookie_da_conservare.txt`, creato al primo uso con le righe d'esempio **commentate** `# google.com`, `# microsoft.com`, `# live.com`).
- Default: tutte le voci **spente**, con `warning` (cookie: «Verranno eliminati tutti i cookie tranne i domini della lista (lista: N domini)»).

- [ ] **Step 1: Test che falliscono** (`src/browserdb.rs`, database creati in `tempdir`):
  - `profiles_are_found_for_chromium_and_gecko_layouts` (profilo `Default`, `Profile 2`, `*.default-release`)
  - `keep_list_ignores_comments_blank_lines_case_and_leading_dot` (`" Google.COM "`, `.live.com`, `# x` → `["google.com", "live.com"]`)
  - `cookies_outside_the_keep_list_are_deleted_chromium` e `..._gecko` (tabelle `cookies.host_key` e `moz_cookies.host`: restano `google.com`, `.google.com`, `accounts.google.com`; spariscono gli altri)
  - `empty_or_missing_keep_list_protects_nothing_and_says_so` (la finestra riceve «lista: 0 domini»; nessun panico con file assente o non UTF-8)
  - `compact_runs_only_after_integrity_check_ok` (database rovinato di proposito → saltato con nota, non toccato)
  - `compact_reports_recoverable_bytes_from_freelist` (`freelist_count × page_size`)
  - `firefox_history_keeps_bookmarks` (`moz_bookmarks` uguale prima e dopo; `moz_historyvisits` vuota; `moz_places` senza segnalibri cancellata)
  - `firefox_history_rolls_back_if_bookmark_count_changes` (con un trigger finto che cancella un segnalibro: nessuna modifica)
  - `last_download_dir_removed_from_preferences_and_prefs_js` (JSON di `Preferences` valido dopo la modifica; le altre chiavi invariate; `prefs.js` senza `browser.download.lastDir`; **le chiavi di Chromium sono da verificare** (tabella N12): se il browser vero non le ha, la voce risulta «niente da fare»)
  - `database_of_a_running_browser_is_never_opened` (processo finto attivo → `analyze` e `clean` non aprono il file: `skipped` e nota)
- [ ] **Step 2:** FAIL. **Step 3:** implementare con `rusqlite` in transazione (`BEGIN IMMEDIATE`; `PRAGMA integrity_check` prima di compattare; `VACUUM` fuori transazione; `ROLLBACK` se il controllo dei segnalibri fallisce); `Preferences` con `serde_json = "1"` (già nel build, aggiungerla al `Cargo.toml` radice): si legge, si tolgono le due chiavi, si riscrive solo se il risultato si rilegge come JSON valido (altrimenti nessuna scrittura); pulsante «Modifica lista cookie» che apre il file nell'editor predefinito (`ShellExecuteW "open"`), nello stesso stile di `secondary()`. **Step 4:** `cargo test -p pulisci-pc -j 2` verde, clippy pulito, screenshot con profilo finto che contiene due profili di browser veri a forma (database finti): il gruppo «Browser» con le quattro voci.
- [ ] **Step 5:** Commit `feat(cleaner): database dei browser (cookie con lista, compattazione, cronologia Firefox, cartella di download)`.

### Task 4: verifica finale e tabella aggiornata

Stima: 1 h.

**Files:**
- Modify: `docs/ccleaner-coverage.md` (colonna «Stato» nelle tabelle: `fatto <commit>` / `rinviato 3b` / `da verificare`)

- [ ] **Step 1:** `cargo test --workspace --exclude mukade -j 2` verde; annotare i numeri per crate nel ledger.
- [ ] **Step 2:** `grep -rniE "bleachbit|AGG -" src crates cleaners` senza risultati fuori dai test che li escludono.
- [ ] **Step 3:** Ogni riga «Nuovo XML» e «Codice nativo nuovo» della tabella ha lo stato; le quattro rinviate (N5-N8) dicono `rinviato 3b`; nessuna riga vuota.
- [ ] **Step 4:** Screenshot finale con profilo finto (`--tab cleaner`): gruppi `Windows`, `Browser`, `Sviluppo e AI`, `Applicazioni`, stessa barra e stesse card.
- [ ] **Step 5:** Commit `docs(fase3): stato di copertura di CCleaner aggiornato`.
