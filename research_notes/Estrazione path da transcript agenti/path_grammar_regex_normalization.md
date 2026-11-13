# Riconoscimento, estrazione e normalizzazione di path locali (Windows/POSIX) da testo e JSON/JSONL, con focus su Rust

Scope: come trovare in modo affidabile path assoluti dentro transcript di agenti (Claude Code, Codex CLI, OpenCode, Pi; file JSONL) e normalizzarli per confrontarli, per usarli come cartelle candidate su Windows. Data delle note: 2026-11-13.

## 1. Grammatica dei path Windows (drive, UNC, verbatim, device, separatori, nomi riservati, lunghezza)

### Takeaway
Un path Windows "fully qualified" inizia con `X:\` (o `X:/`), `\\server\share`, `\\?\` o `\\.\`. `C:foo` e `\foo` NON sono assoluti. Windows normalizza `/` in `\`, collassa i separatori, risolve `.`/`..` e taglia punti/spazi finali, tranne che per i path che iniziano esattamente con `\\?\`. Il confronto deve essere case-insensitive.

### Cited Findings
- Un path DOS ha tre componenti: volume/drive letter + `:`, directory, filename opzionale. "If all three components are present, the path is absolute". `C:Projects\apilibrary\apilibrary.sln` è "a relative path from the current directory of the C: drive", e `\Program Files\...` è "relative from the root of the current drive". MS lo definisce "a common source of bugs" — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Ordine di classificazione usato dalla normalizzazione di Windows: (1) device path (`\\?` o `\\.`), (2) UNC (due separatori senza `?` né `.`), (3) DOS fully qualified (`C:\`), (4) device legacy (`CON`, `LPT1`), (5) relativo alla root del drive corrente (`\`), (6) relativo alla cwd di un drive (`C:` senza separatore), (7) relativo alla cwd — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- UNC: `\\` + server (nome NetBIOS, IPv4/IPv6 o FQDN) + `\` + share; "Together, the server and share name make up the volume". "UNC paths must always be fully qualified". Esempi: `\\system07\C$\`, `\\Server2\Share\Test\Foo.txt` — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Device path: `\\.\C:\Test\Foo.txt`, `\\?\C:\Test\Foo.txt`, con GUID di volume `\\?\Volume{b75e2c83-...}\Test\Foo.txt`, e link UNC `\\?\UNC\Server\Share\...` / `\\.\UNC\Server\Share\...`. "DOS device paths are fully qualified by definition and can't begin with a relative directory segment" — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Separatori: "All forward slashes (/) are converted into the standard Windows separator, the back slash (\). If they are present, a series of slashes that follow the first two slashes are collapsed into a single slash." — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- `.` e `..` vengono valutati, ma "Parent directories are only removed if they aren't past the root of the path" (la root è `C:\`, `\\Server\Share` o il prefisso device) — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Trim: "If the path doesn't end in a separator, all trailing periods and spaces (U+0020) are removed"; anche un singolo punto finale di un segmento viene rimosso; i segmenti di 3 o più punti sono nomi validi — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Salto della normalizzazione: "Unless the path starts exactly with `\\?\` (note the use of the canonical backslash), it is normalized." I path `\\?\` vengono comunque normalizzati se passati esplicitamente a GetFullPathName — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Case: "directory and file name comparisons are case-insensitive" — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats). "Volume designators (drive letters) are similarly case-insensitive"; "NTFS supports POSIX semantics for case sensitivity but this is not the default behavior" — [MS Learn, Naming Files, Paths, and Namespaces](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- Caratteri riservati nei nomi: `< > : " / \ | ? *`, NUL e i codici 1–31 (salvo negli alternate data streams) — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- Nomi riservati: CON, PRN, AUX, NUL, COM1–COM9, COM¹ COM² COM³, LPT1–LPT9, LPT¹ LPT² LPT³. Valgono anche con estensione ("NUL.txt and NUL.tar.gz are both equivalent to NUL") — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file). Prima di Windows 11 `Path.GetFullPath` trattava qualsiasi path che inizia con un nome di device legacy come device (`COM1.TXT\file1.txt` diventava `\\.\COM1`); su Windows 11 non è più così — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- "Do not end a file or directory name with a space or a period", anche se il file system può supportarlo — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- Nomi corti 8.3: Windows può creare un alias 8.3, di solito con `~` (es. `PROGRA~1`). La generazione può essere disattivata, quindi "do not make the assumption that the 8.3 alias already exists on-disk". Si convertono con GetShortPathName/GetLongPathName — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- MAX_PATH = 260 caratteri; da Windows 10 1607 il limite si rimuove con registro/Group Policy. Il prefisso `\\?\` "tells the Windows APIs to disable all string parsing and to send the string that follows it straight to the file system" e permette di superare MAX_PATH (con le API Unicode) — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- `\\.\` serve per il namespace dei device (`\\.\PhysicalDrive0`, `\\.\COM56`), "use the `\\.\` prefix to access devices only and not files". `\\?\GLOBALROOT` dà accesso alla root dell'object manager — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
- Esempi equivalenti per lo stesso file: `c:\temp\test-file.txt`, `\\127.0.0.1\c$\temp\...`, `\\LOCALHOST\c$\temp\...`, `\\.\c:\temp\...`, `\\?\c:\temp\...`, `\\.\UNC\LOCALHOST\c$\temp\...` — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)

### Inferences
- Per le cartelle candidate conviene accettare solo: `[A-Za-z]:[\\/]...`, `\\server\share\...`, `\\?\X:\...`, `\\?\UNC\server\share\...` (e i rispettivi `\\.\`). Gli ultimi due vanno ricondotti alla forma `X:\` o `\\server\share`. `\\.\PhysicalDrive*`, `\\?\Volume{GUID}` e `\\?\GLOBALROOT` vanno scartati: non sono cartelle utili come progetto.
- `C:foo`, `\foo` e i path relativi vanno scartati, o risolti solo se è noto un `cwd` (i transcript spesso hanno un campo cwd per evento; questo va verificato nei formati specifici, è fuori dallo scope di queste note).
- Chiave di confronto: lettera di drive maiuscola, separatore `\`, niente `\` finale (salvo la root `C:\`), segmenti `.`/`..` risolti in modo lessicale, punti/spazi finali tagliati, poi confronto case-insensitive (es. lowercase o case folding Unicode). La forma originale va conservata per la visualizzazione.
- Una componente con `~` seguita da cifre (`RUNNER~1`) indica un probabile nome 8.3. Si espande solo se il path esiste, con GetLongPathNameW.

### Gaps
- Non ho verificato se `\\?\` più slash (`\\?/C:/...`) compaia nei transcript reali. Per MS la forma canonica richiede backslash.
- Non ho dati su quanto spesso compaiano path UNC o 8.3 nei transcript degli agenti.

## 2. Come compaiono i path in JSON/JSONL, nei comandi shell, in Markdown e negli URI file://

### Takeaway
Va fatto prima il parsing JSON (serde_json) e poi l'estrazione sulle stringhe decodificate. Il matching sulla riga grezza vede `C:\\Users` (doppio backslash) e fa confondere gli escape JSON con i separatori. Le forme Git Bash (`/c/Users`), WSL (`/mnt/c/Users`) e `file:///C:/...` vanno mappate esplicitamente a `C:\...`.

### Cited Findings
- In JSON il backslash dentro una stringa va escapato come `\\`, e i caratteri di controllo U+0000–U+001F devono essere escapati (`\n`, `\t`, `\uXXXX`). Quindi `C:\Users` nel file è `"C:\\Users"`. Se il contenuto è JSON annidato serializzato in una stringa (es. tool input/output), il raw diventa `"C:\\\\Users"` — [RFC 8259 §7](https://www.rfc-editor.org/rfc/rfc8259#section-7)
- I nomi file Windows non possono contenere caratteri 1–31 (salvo gli ADS) — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file). Ne segue che un path decodificato che contiene `\n` reale (newline) non è un path valido: il path è troncato lì.
- Git Bash/MSYS2: i drive sono montati come `/c/`; `cygpath -u` converte `C:\foo` in `/c/foo`. MSYS2 converte automaticamente in path Windows "all the arguments that look like Unix paths", con euristiche "not perfect" (si disattiva con `MSYS2_ARG_CONV_EXCL`) — [MSYS2, Filesystem Paths](https://www.msys2.org/docs/filesystem-paths/)
- In WSL i drive Windows sono montati sotto `/mnt/<lettera>/` (la root è configurabile tramite `automount.root` in wsl.conf); `wslpath` converte tra le due forme — [MS Learn, WSL wsl.conf](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)
- `url::Url::to_file_path` (url 2.5.8): "This does not actually check the URL's scheme". Restituisce Err se l'host non è vuoto né "localhost" (eccetto su Windows, dove sono ammessi host non locali, cioè UNC), se il path decodificato contiene NUL, o su Windows se non è UTF-8. `from_file_path` richiede un path assoluto e su Windows un prefisso disk o UNC — [docs.rs url::Url](https://docs.rs/url/latest/url/struct.Url.html)
- VS Code riconosce in testa ai path Windows i prefissi `file:///` e `\\?\`: `winDrivePrefix = '(?:\\\\\\\\\\?\\\\|file:\\/\\/\\/)?[a-zA-Z]:'` ("matches the start of an absolute path on Windows, such as: `C:`, `c:`, `file:///c:` (uri) and `\\?\C:`") — [VS Code terminalLinkParsing.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkParsing.ts)
- tmux-thumbs ha un pattern dedicato ai link Markdown `\[[^]]*\]\(([^)]+)\)` e un pattern URL che include `file:///` — [tmux-thumbs src/state.rs](https://github.com/fcsonline/tmux-thumbs/blob/master/src/state.rs)

### Inferences
- Pipeline consigliata per JSONL: per ogni riga `serde_json::from_str::<Value>`, visita ricorsiva di tutte le stringhe (o solo dei campi rilevanti, come cwd, input dei tool `file_path`/`path`/`command`/`workdir`, testo dei messaggi). Se una stringa inizia con `{` o `[` si prova un parse annidato. I campi strutturati (cwd, file_path) sono molto più affidabili del testo libero: conviene dare loro un peso maggiore.
- Mai fare un "unescape" generico del testo libero: `C:\new\tab` in un testo libero decodificato è già corretto. Il rischio `\n`/`\t` esiste solo se si applica un unescape a qualcosa che non era escapato (es. si "decodifica" due volte) o si fa regex sulla riga raw. Euristica di guardia: se una stringa decodificata contiene caratteri di controllo dentro un candidato path, si tronca il candidato lì.
- Comandi shell: va gestito il quoting (`cd "C:\Program Files\x"`, `'...'`) e lo split su `&&`, `;`, `|`. Esistono tokenizer shell-words per Rust (crate `shell-words`/`shlex`, non verificati in questa ricerca), ma per i comandi PowerShell/cmd il quoting è diverso: è meglio estrarre sia i token quotati sia quelli non quotati.
- Mappatura: `^/([a-zA-Z])(/|$)` → `X:\` (Git Bash) e `^/mnt/([a-zA-Z])(/|$)` → `X:\` (WSL, root di default). Il primo pattern è ambiguo con path POSIX reali di una lettera (es. `/c/` su Linux): va applicato solo se il transcript proviene da Windows (cwd Windows).
- `file://` URI: percent-decoding (`%20` → spazio), rimozione della `/` iniziale in `/C:/...`, host non vuoto → UNC `\\host\...`. `Url::to_file_path` su host Linux produce un path POSIX, quindi per testare su Linux conviene un decoding manuale (o `percent-encoding`) seguito dal parsing con `typed-path`.
- Markdown: rimuovere backtick, `**`, parentesi di link `[..](..)` e punteggiatura finale prima della validazione.

### Gaps
- Non ho verificato i nomi esatti dei campi nei JSONL di Claude Code/Codex/OpenCode/Pi (presumibilmente coperto da altri ricercatori).
- La pagina wsl-config è citata dalle mie conoscenze, non ricontrollata in questa sessione: va verificata.

## 3. Regex ed euristiche usate da tool reali

### Takeaway
I tool maturi (VS Code, Zed, kitty, tmux-thumbs) usano classi di caratteri "esclusi" anziché grammatiche complete, gestiscono a parte i suffissi `:line:col`, escludono `"'` e le parentesi come delimitatori, e VS Code ha dovuto vietare `\\` nella classe POSIX per evitare backtracking catastrofico. Nessuno valida l'esistenza sul file system nel regex: la verifica avviene dopo.

### Cited Findings
- VS Code, path Windows (`winLocalLinkClause`): prefisso opzionale (`winDrivePrefix` | `\.\.?` | `~` | un primo segmento), poi uno o più `(?:\\|\/)` seguiti da caratteri in `[^\0<>\?\|\/\s!`&*()'":;]`. Il primo carattere esclude anche `[]` — [VS Code terminalLinkParsing.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkParsing.ts)
- VS Code, path POSIX: caratteri esclusi `[^\0<>\?\s!`&*()'":;\\]`, con il commento "'\":; are allowed in paths but they are often separators so ignore them. Also disallow \\ to prevent a catastropic backtracking case #24795" — [VS Code terminalLinkParsing.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkParsing.ts)
- VS Code esegue prima il rilevamento "via suffix" (path seguito da `:339:12`, `#339`, `(339,12)`, `", line 339, column 12"`, `on line 339` e altri), poi il rilevamento senza suffisso, e unisce i range non in conflitto. Per i path con suffisso usa `(?<path>(?:file:\/\/\/)?[^\s\|<>\[\({][^\s\|<>]*)$`. Rimuove i prefissi git diff `a/`, `b/` (anche `[abciow12]/` con mnemonicPrefix) — [VS Code terminalLinkParsing.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkParsing.ts)
- Zed (editor in Rust), `terminal.path_hyperlink_regexes` di default: (a) diagnostica Python `File "(?<path>[^"]+)", line (?<line>[0-9]+)`; (b) regex generica: il primo carattere non è un delimitatore di apertura, uno spazio o un box-drawing `\u2500-\u257F`; nei caratteri centrali `:` e `(` sono ammessi solo se non seguiti da cifra/parentesi/spazio; l'ultimo carattere non è `()}]>"'`.,;:` né spazio; segue il suffisso opzionale `(:+[0-9]+(:[0-9]+)?|:?\([0-9]+([,:]?[0-9]+)?\))?`. Il timeout di default è `path_hyperlink_timeout_ms: 1` — [Zed assets/settings/default.json](https://github.com/zed-industries/zed/blob/main/assets/settings/default.json)
- Zed ha corretto il riconoscimento dei path verbatim Windows (`\\?\C:\Over\here.rs`) nella stessa PR che ha introdotto le regex configurabili (#40305) — [Zed commit e76b485](https://git.secluded.site/zed/commit/e76b485de3cb85978d5e5300f004a65b4f7ff968)
- kitty hints: `path_regex = (?:\S*?/[\r\S]+)|(?:\S[\r\S]*FILE_EXTENSION)\b` con `FILE_EXTENSION = \.(?:[a-zA-Z0-9]{2,7}|[ahcmo])(?:\b|[^.])`; linenum: `(?P<path>…):(?P<line>\d+)`; espande `~` — [kitty kittens/hints/marks.go](https://github.com/kovidgoyal/kitty/blob/master/kittens/hints/marks.go)
- tmux-thumbs (Rust): `path = (?P<match>([.\w\-@$~\[\]]+)?(/[.\w\-@$\[\]]+)+)`; pattern separati per URL (`https?://|git@|git://|ssh://|ftp://|file:///`), `--- a/`, `+++ b/`, sha, IP, UUID. Prima del matching esclude le sequenze ANSI `[[:cntrl:]]\[([0-9]{1,2};)?([0-9]{1,2})?m` — [tmux-thumbs src/state.rs](https://github.com/fcsonline/tmux-thumbs/blob/master/src/state.rs)
- par-term (emulatore Rust) usa regex per i path con suffissi `file.rs:42` e `file.rs:42:10` — [par-term detector.rs su docs.rs](https://docs.rs/crate/par-term/0.40.0/source/src/url_detection/detector.rs)

### Inferences
- Il crate `regex` di Rust non supporta look-around né backreference (lo dicono i suoi docs; è una conoscenza generale non riverificata qui), ma garantisce tempo lineare, quindi il problema di backtracking di VS Code (#24795) non si pone. I vincoli "l'ultimo carattere non è punteggiatura" vanno implementati con classi negate come fa Zed, oppure con un post-processing (trim di `.,;:)]}>"'`` ` finali, con bilanciamento delle parentesi).
- Proposta di regex di estrazione per Windows assoluti (crate `regex`, modalità `(?x)`):
  `(?:\\\\\?\\(?:UNC\\)?|\\\\\.\\)?(?:[A-Za-z]:[\\/]|\\\\[^\\/\s"'<>|?*]+[\\/][^\\/\s"'<>|?*]+)(?:[^\\/\x00-\x1f"<>|?*\n]+[\\/]?)*`
  poi trim della punteggiatura finale e gestione degli spazi. Gli spazi sono il problema principale: senza virgolette non si sa dove finisce `C:\Program Files\x`. Strategia: se il candidato è racchiuso tra virgolette o backtick, si prende tutto fino al delimitatore; altrimenti si tronca al primo spazio e, se possibile, si estende verificando l'esistenza sul file system segmento per segmento.
- Lookbehind emulato: richiedere che prima di `X:` non ci sia una lettera o una cifra (`(?:^|[^A-Za-z0-9])`) per non agganciare `abc:\` o il `s:` di `https://`.
- Dopo il match: parse con `typed-path` (WindowsPath), scarto se non `is_absolute()`, scarto di componenti con caratteri riservati o nomi riservati, scarto se il "path" è solo `C:\`.

### Gaps
- Windows Terminal: non ho trovato prove che riconosca path in modo nativo (sembra limitarsi agli URL); non verificato.
- iTerm2 Semantic History, GitHub/GitLab linkifier, problemMatchers di VS Code: non consultati per limiti di tempo. I problemMatchers usano regex definite per tool (es. `$tsc`) e non hanno una regex di path generica (dato non verificato qui).

## 4. Crate Rust per normalizzazione e confronto

### Takeaway
Per un tool Windows che deve essere testabile su Linux: `typed-path` per il parsing e la normalizzazione lessicale di path Windows su qualsiasi OS; `dunce::simplified` per togliere `\\?\` quando è sicuro (solo su Windows: altrove è no-op); `normpath` (`normalize`, `expand`) o `std::path::absolute` per la normalizzazione nativa su Windows. Evitare `std::fs::canonicalize` per le chiavi di confronto: restituisce `\\?\` e fallisce sui path inesistenti.

### Cited Findings
- Stato dei crate (ultima release secondo l'API di crates.io, verificato 2026-11-13): `dunce` 1.0.5 (2024-08-04); `normpath` 1.5.2 (2026-09-19); `path-clean` 1.0.1 (2023-02-24); `path-absolutize` 4.0.1 (2026-07-11); `sugar_path` 3.0.0 (2026-07-12); `typed-path` 0.13.0 (2026-10-20); `relative-path` 2.0.3 (2026-10-21); `camino` 1.2.6 (2026-09-15); `path-slash` 0.2.1 (2022-08-06); `omnipath` 0.1.6 (2023-05-12) — [crates.io API](https://crates.io/api/v1/crates/typed-path) (stessa query per ogni crate)
- `std::fs::canonicalize` usa `realpath` su Unix e `CreateFile` + `GetFinalPathNameByHandle` su Windows. Su Windows "converts the path to use extended length path syntax", che "may be incompatible with other applications". Restituisce errore se il path non esiste — [std::fs::canonicalize](https://doc.rust-lang.org/std/fs/fn.canonicalize.html)
- `std::path::absolute` (stabile dalla 1.79.0): su Windows, per i path verbatim "will simply return the path as given", altrimenti equivale a `GetFullPathNameW` (es. `C:\foo//test\..\./bar.rs` → `C:\foo\bar.rs`). Su POSIX "keep[s] `..` components and trailing separators". Non risolve i symlink e "may succeed even if the path does not exist" — [std::path::absolute](https://doc.rust-lang.org/std/path/fn.absolute.html)
- `std::path::Path` interpreta `C:\path\to\file.txt` in modo diverso a seconda dell'OS: su Unix è un'unica componente `Normal("C:\\path\\to\\file.txt")` — [typed-path README](https://github.com/chipsenkbeil/typed-path)
- `typed-path` fornisce `WindowsPath`/`UnixPath` (byte) e `Utf8WindowsPath`/`Utf8UnixPath` che fanno il parsing in modo identico su tutte le piattaforme (`Prefix(Disk(67))`, `RootDir`, `Normal(..)`). Supporta no_std+alloc dalla 0.7 e serde dalla 0.13; ha `push_checked`/`join_checked` contro il path traversal — [typed-path README](https://github.com/chipsenkbeil/typed-path)
- `typed-path` ha `normalize()` lessicale (es. `C:..\foo\..` → `C:`, i `..` non risolvibili in un path relativo vengono scartati), `absolutize()` (prepende la cwd e normalizza), `with_encoding::<U>()` per convertire tra Unix e Windows. Nell'encoding Windows tratta `/` come separatore: `/` compare tra i caratteri di separazione/riservati in `windows/constants.rs` — [typed-path 0.13.0 sorgente src/common/utf8/path.rs, src/windows/constants.rs](https://docs.rs/typed-path/0.13.0/typed_path/)
- `dunce::simplified` converte `\\?\C:\Windows` → `C:\Windows`, ma lascia `\\?\C:\COM` così com'è; "does not perform any I/O"; "On non-Windows this is no-op". Toglie il prefisso solo per `VerbatimDisk` e solo se ogni componente è un nome valido (≤255 caratteri, nessun carattere `0..=31 < > : " / \ | ? *`, nessun `.` o spazio finale), non è un nome riservato (`AUX NUL PRN CON COM1-9 LPT1-9`, anche con estensione) e la lunghezza totale è ≤260. Non converte `\\?\UNC\...` — [dunce 1.0.5 src/lib.rs](https://docs.rs/dunce/1.0.5/dunce/) (sorgente del crate)
- `normpath`: nato per correggere i bug ricorrenti causati da `fs::canonicalize` su Windows (rust-lang issues #45067, #48249, #52440, #55812, #58613, #59107, #74327); "Normalization is usually a better choice unless you specifically need a canonical path". MSRV 1.81 — [normpath README](https://github.com/dylni/normpath)
- `normpath::PathExt`: `normalize` (su Windows `GetFullPathNameW`, il path deve esistere; su Unix `fs::canonicalize`; i path verbatim non vengono modificati; le junction non vengono risolte); `normalize_virtually` (solo Windows, senza accesso al file system, i path inesistenti sono ok); `expand` (`GetLongPathNameW`, risolve i nomi 8.3, il path deve esistere); `shorten` (`GetShortPathNameW`); `localize_name` — [docs.rs normpath PathExt](https://docs.rs/normpath/latest/normpath/trait.PathExt.html)
- `sugar_path` 3: "Host-native lexical path manipulation"; non tocca il file system né risolve symlink; "Windows drive-letter spelling is likewise preserved" (non mette in maiuscolo il drive) — [sugar_path README](https://github.com/hyf0/sugar_path)
- `path-absolutize`: `absolutize` "does not care about whether the file exists"; ha `absolutize_virtually(root)`, che dà errore se il path esce dalla root virtuale — [path-absolutize README](https://github.com/magiclen/path-absolutize)

### Inferences
- `sugar_path`, `path-clean`, `path-absolutize`, `dunce` e `normpath` lavorano sul `Path` nativo, quindi su un host Linux NON interpretano `C:\...`: i test unitari di estrazione e normalizzazione su CI Linux richiedono `typed-path` (o un normalizzatore scritto a mano sulle stringhe). Raccomandazione: implementare la chiave di confronto con `Utf8WindowsPath` di typed-path (cross-platform), e usare `dunce`, `normpath::expand` o `std::path::absolute` solo nello step "verifica esistenza" eseguito su Windows.
- Chiave canonica proposta: (1) parse `Utf8WindowsPath`; (2) mappa `VerbatimDisk` → `Disk` e `VerbatimUNC` → `UNC` (come `dunce`, ma cross-platform); (3) `normalize()`; (4) drive maiuscolo; (5) rimozione del separatore finale; (6) chiave = `to_lowercase()` (o case folding); valore visualizzato = forma originale normalizzata. Opzionale su Windows: `GetLongPathNameW` (normpath `expand`) per gli alias 8.3 e `fs::canonicalize` + `dunce::simplified` solo per deduplicare symlink e junction quando il path esiste.
- Attenzione: il case folding di NTFS usa una tabella upcase propria del volume; `to_lowercase` di Rust è un'approssimazione (fonte non verificata in questa ricerca). Per directory ASCII non cambia nulla.
- `camino` (Utf8PathBuf) è utile per un'API UTF-8 ma resta host-native; `relative-path` è per path relativi portabili e non serve qui.
- `path-slash` (2022) e `omnipath` (2023) non hanno release recenti: da evitare come dipendenze core.

### Gaps
- Non ho verificato se `typed-path` 0.13 offra un confronto case-insensitive integrato (probabilmente no).
- Il crate `windows`/`windows-sys` per chiamare direttamente `GetLongPathNameW` non è stato esaminato.

## 5. Falsi positivi e casi limite

### Takeaway
I falsi positivi tipici sono URL (`https://`), orari (`12:30`), token tipo drive (`a:b`, `s:` di URL), regex e escape (`\d+\s*`), path troncati o con `...`, path con punteggiatura finale, placeholder (`C:\path\to\file`, `C:\Users\<user>`), e path di esempio in documentazione/codice. Vanno filtrati con regole sintattiche e, alla fine, con la verifica di esistenza su disco.

### Cited Findings
- VS Code esclude da tutti i path i caratteri `!`&*()'":;` e i tag `<>`, perché "often separators" — [VS Code terminalLinkParsing.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkParsing.ts)
- Zed vieta `:` seguito da cifra all'interno del path (per non inglobare `:line`) e vieta come ultimo carattere `.,;:` e le parentesi chiuse — [Zed default.json](https://github.com/zed-industries/zed/blob/main/assets/settings/default.json)
- tmux-thumbs rimuove le sequenze ANSI prima del matching (il caso di test `path: [32m/var/log/nginx.log[m`) — [tmux-thumbs src/state.rs](https://github.com/fcsonline/tmux-thumbs/blob/master/src/state.rs)
- Un path `C:` senza separatore è relativo al drive, non assoluto — [MS Learn, File path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats)
- Gli stessi doc MSYS2 avvertono che la conversione automatica "converts arguments that look like Unix paths while they are not" (es. opzioni `/c` di cmd) — [MSYS2, Filesystem Paths](https://www.msys2.org/docs/filesystem-paths/)
- Nomi che finiscono con spazio o punto e caratteri riservati non sono validi nei nomi Windows — [MS Learn, Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)

### Inferences
- Filtri consigliati (in ordine):
  1. Contesto precedente: la lettera di drive non deve essere preceduta da `[A-Za-z0-9_]`, e non deve esserci `://` subito prima (URL); scartare se il match è dentro un URL `scheme://`.
  2. Dopo `X:` serve obbligatoriamente `\` o `/` (esclude `12:30`, `a:b`, `C:foo`).
  3. Path in regex o escape: scartare i candidati con componenti come `\d`, `\s`, `\w`, `\b`, `.*`, `[^`, `(?`, `$` finale, o con `*?|<>"` (caratteri riservati). `C:\new` è invece valido: in testo già decodificato `\n` è letterale, e il problema nasce solo se si applica un unescape scorretto. Se il testo decodificato contiene un newline reale, si spezza lì.
  4. Placeholder: `path\to`, `<...>`, `{...}`, `%USERPROFILE%`/`$HOME`/`~` (questi ultimi espandibili con le env var se si vuole), `...` e `…` (troncamento), `C:\Users\username`, `C:\Users\you`, `xxx`.
  5. Trim finale di `.,;:!?)]}>'"`` ` e degli `**` Markdown. Rimozione dei suffissi `:line(:col)?` e `(line,col)` come fanno VS Code e Zed.
  6. Path di file vs cartella: per le "cartelle candidate" si prende il parent se l'ultima componente ha un'estensione o se il path esiste come file. Poi si risale fino alla root del progetto (es. `.git`); è una scelta di design, non una fonte.
  7. Verifica finale: `Path::is_dir()` su Windows. Il ranking per frequenza e recenza attenua il rumore residuo.
- Path troncati: gli agenti abbreviano spesso con `…` o con larghezza fissa nelle UI TUI; un candidato che termina con `…`/`...` o che contiene `~` di abbreviazione della home (`~\proj`) va espanso o scartato.
- POSIX su Windows: nei transcript Windows possono comparire anche `/c/...` e `/mnt/c/...`. Un path POSIX puro (`/home/x`) proveniente da sessioni WSL/remote non corrisponde a cartelle Windows, salvo `\\wsl.localhost\<distro>\home\x` / `\\wsl$\<distro>\...` (forma UNC di WSL: conoscenza generale, non verificata qui).

### Gaps
- Non ho una misura empirica dei tassi di falsi positivi sui transcript reali: va validata con un corpus di prova.
- La mappatura `\\wsl.localhost\` / `\\wsl$\` non è stata verificata con una fonte in questa sessione.
