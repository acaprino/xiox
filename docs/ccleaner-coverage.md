# Copertura di CCleaner in PulisciPC (Fase 3)

Stato: inventario e tabella di copertura per la Fase 3 della spec `docs/superpowers/specs/2026-11-10-dev-mode-design.md` (sezione «Copertura nativa di CCleaner»). Decisione dell'utente: **tutto l'elenco**, anche le voci per programmi non installati su questo PC.

## 1. Premessa

**Il buco principale.** L'installazione di CCleaner citata dalla spec (`C:\Users\alfio\Desktop\CCleaner\CCleaner64.exe`, `Lang\lang-1040.dll`) non c'è più su questo PC: è stata cercata in Desktop, Download, `D:\Software` e Program Files. Quindi l'elenco **non** è stato riletto dall'eseguibile attuale. È stato ricostruito dalle fonti qui sotto. I nomi italiani delle voci sono tradotti da me dai nomi inglesi: quelli esatti di `lang-1040.dll` sono da verificare (tranne i 14 già citati nella spec).

**Fonti** (sigla usata nella colonna `Fonte`):

| Sigla | Fonte | Cosa dà | Limiti |
|---|---|---|---|
| F1 | `ccleaner64/investigation.cany/investigation/findings.md` (fnd_2), le prove in `investigation.cany/evidence/`, `app/MECHANICS.md` sez. 1 | Stringhe lette dall'exe vero (sha256 `30de2996…a5fe53`): categorie Rule `Temporary_Files 3142`, `Memory_Dumps 3143`, `Windows_Log_Files 3145`, `Old_Prefetch_Data 3147`, `Windows_Error_Reporting 3149`, `Thumbnail_Cache 3131`; opzioni «Only delete files in Windows Temp folders older than 24 hours» (0x141ab7df8), «… 48 hours» (0x1427284fc), «Only delete files in Recycle Bin older than 24 hours» (0x141ab7e70), «Wi&pe Cluster Tips» (0x1424db690), Ottimizzazione recapito (risorsa 0x1424e1476) | L'indagine cercava solo i file temporanei, il registro e i servizi: niente browser e niente elenco completo |
| F2 | Export di CCleaner `stored_ini__System_Cleaning.ini`, `Revision=2072`, © 2004-2018 Piriform (repo GitHub `vaginessa/eladkarako-software-mods`, cartella `CCleaner/resources`, commit del 2018-03-28) | **L'elenco completo della scheda Windows** di allora, con le sezioni (`LangSecRef` 3001 Internet Explorer, 3002 Esplora risorse, 3003 Sistema, 3004 Avanzate, 3005 Microsoft Edge), `Default=` e i percorsi o le routine interne (`SpecialKey1=N_…`) | Del 2018 (CCleaner 5.x): mancano le voci aggiunte dopo. Se `Default=` manca, l'ho letto come «spento» (vale per le password, che CCleaner spegne sempre): è una deduzione |
| F3 | Export `stored_ini__Application_Cleaning.ini`, `Revision=2184` (stesso repo) | Le voci dei browser della scheda Applicazioni (3026 Firefox, 3027 Opera, 3028 Safari, 3029 Chrome, 3030 Thunderbird) con i default; le sezioni 3021-3025 e 3031 Windows Store (98 + 65 + 81 + 91 + 10 + 10 voci) | Del 2018: niente Edge Chromium, Brave, Vivaldi |
| F4 | Documentazione ufficiale di CCleaner archiviata: `ccleaner.com/docs/ccleaner/ccleaner-rules/windows-tab/{internet-explorer-files, windows-explorer-files, system-files, advanced-windows-files}` e `…/applications-tab` (copie di web.archive.org del 2018-2020; le pagine vive rispondono 403 o rimandano a support.ccleaner.com) | Cosa fa ogni voce, con le parole di Piriform | Non dice i default |
| F5 | Synacktiv, «CCleaner forensics» (CCleaner 6.00.9727, 2022): https://www.synacktiv.com/publications/ccleaner-forensics | In 6.00 `Winapp.ini` ha 476 voci, `Winsys.ini` 62, `Winreg.ini` 15; `Edge Chromium - Session` sta in `Winsys.ini` con `LangSecRef=3006`, `Default=False`; la pulizia dei registri eventi li azzera (eventi 1102 e 104) | Solo esempi, non l'elenco |
| F6 | Forum CCleaner, «CCleaner 6.06 new cleaning option "Windows Web Cache"»: https://community.ccleaner.com/t/ccleaner-6-06-new-cleaning-option-windows-web-cache/77799 | Voce nuova in Windows → Sistema dalla 6.06, cartella `CryptnetUrlCache`; un utente la trova accesa di default | Il default è un'osservazione di un utente |
| F7 | Forum CCleaner, discussione 62304 sulla 5.90: «Windows Subsystems» spostata in Avanzate (accanto a «Windows Event Logs»), poi rinominata «Network Data Usage» (citata in F6) | Esistenza della voce | Il contenuto esatto è da verificare |
| F8 | Winapp2, tabella dei `LangSecRef`: https://github.com/MoscaDotTo/Winapp2/blob/master/CONTRIBUTING.md | 3001 Internet Explorer (deprecata), 3005 Microsoft Edge legacy (deprecata), 3006 Edge Chromium, 3021 Applicazioni, 3022 Internet, 3023 Multimedia, 3024 Utilità, 3025 Windows, 3026 Firefox, 3027 Opera, 3028 Safari, 3029 Google Chrome, 3030 Thunderbird, 3031 Windows Store, 3032 CCleaner Browser, 3033 Vivaldi, 3034 Brave, 3035 Opera GX, 3036 Spotify, 3037 Avast Secure Browser, 3038 AVG Secure Browser, 3039 Arc, 3040 iTunes, 3042 WhatsApp, 3043 Norton Private Browser, 3044 Avira Secure Browser (3041 non valido). La pagina dice che i valori sono «defined by CCleaner v6.39» | Elenca le sezioni, non le voci |
| W | `C:\ProgramData\PulisciPC\winapp2\Winapp2.ini` (variante `Non-CCleaner`, `Version: 260915`, 4068 voci) letto in sola lettura | Le voci Winapp2 citate in questa tabella esistono tutte nel file (nome tra backtick) | — |
| S | Le stringhe italiane già citate dalla spec (righe 154-156: Appunti, Cache DNS, Frammenti file di chkdsk, Documenti recenti, Esegui (nel Menu Start), Collegamenti nel menu Start, Collegamenti sul Desktop, Cronologia assistenza utente, Cache MUI, Cache Web di Windows, Password salvate, Compatta database, Cookie con «Cookie da conservare», Pulisci spazio libero) | Nomi italiani veri di quelle 14 voci | Non riverificabili senza la DLL |

**Cosa non si è potuto verificare.** (1) Le voci aggiunte dopo il 2018 oltre a quelle con fonte (Cache Web di Windows, Utilizzo dati di rete, Edge Chromium, Ottimizzazione recapito): ce ne possono essere altre. (2) I default nella versione 6.x installata dall'utente. (3) Il gruppo esatto di alcune voci nuove. (4) L'elenco delle voci di Brave, Vivaldi, Opera GX e degli altri browser «CCleaner only» (3032-3044): l'ho supposto uguale a quello di Chrome perché sono tutti Chromium. Ogni cella che dipende da questo dice «da verificare».

**Come si chiude il buco.** Con l'exe di nuovo sul PC: (a) `CCleaner64.exe /EXPORT` scrive `Winapp.ini`, `Winsys.ini` e `Winreg.ini` nella cartella del programma (F5), cioè l'elenco completo con i default, senza pulire niente; (b) dalle stringhe di `lang-1040.dll` si prendono i nomi italiani; (c) confronto a campione su 10 voci, CCleaner solo in **Analizza**, come vuole la spec. Voci proposte per il campione: Cestino, File temporanei, Dump di memoria, File di log di Windows, Cache miniature, Jump list, Frammenti chkdsk, Cache Web di Windows, Chrome/Edge Cache, Firefox Cache.

**Vincoli di PulisciPC che cambiano la copertura** (letti nel codice, non supposti):
- `crates/pc-rules/src/guard.rs:115-141`: le regole esterne (Winapp2 e CleanerML) non toccano mai il contenuto di `System32`, `SysWOW64`, `WinSxS`, `Vault`, `Credentials`, `Crypto`, né il Cestino nella radice dei dischi. Quindi le voci Winapp2 che puntano lì vengono scartate (per esempio `%WinDir%\System32\winevt\logs` di `Windows Event Logs *`).
- `crates/pc-rules/src/regguard.rs`: solo `HKCU\Software` e `HKLM\Software`. `HKU\…` non è una radice supportata: `Windows Recent Documents *` di Winapp2 tocca solo `HKU\.DEFAULT` e i profili di sistema, quindi **non** pulisce i documenti recenti dell'utente.
- Tutte le voci Winapp2 partono spente (`winapp2.rs`, test `everything_is_off_by_default`).
- Un XML nuovo in `cleaners/` va anche aggiunto all'elenco `include_str!` di `crates/pc-rules/src/embedded.rs`.
- `rusqlite =0.40.2` con `bundled` è già nel build (`crates/dc-mukade/Cargo.toml:14`): la compattazione SQLite e i cookie da conservare non aggiungono dipendenze.

## 2. Scheda Windows

`Default CCleaner`: valore di `Default=` in F2 (2018) salvo altra fonte; «No (dedotto)» = chiave assente in F2. Le proposte `Codice nativo nuovo` (N1…N12) sono descritte una volta sola in 2.6. Le voci `Nuovo XML` vanno tutte in un solo file nuovo, `cleaners/windows_extra.xml` (id `windows_extra`), un'`<option>` per voce: il nome dell'opzione è nella colonna `Come`.

### 2.1 Internet Explorer (`LangSecRef` 3001)

Internet Explorer 11 non si apre più su Windows 11, ma le sue cartelle (WinINet) restano e le usano ancora app e modalità IE di Edge. Winapp2 non ha più voci per IE (sezione deprecata, F8).

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| File temporanei di Internet | File: cache WinINet (routine interna `N_INT_TEMP`) | Sì | Nuovo XML | opzione `ie_cache`: `%LocalAppData%\Microsoft\Windows\INetCache\IE` e `…\INetCache\Low\IE`, `walk.files`. Il percorso è lo standard di Windows 10/11, non quello di CCleaner (interno): da verificare a campione | Basso: file in uso saltati | F2, F4 |
| Cronologia | File: cartella History di WinINet; registro `HKCU\Software\Microsoft\Internet Explorer\TabbedBrowsing\NewTabPage` | Sì | Nuovo XML | opzione `ie_history`: `%LocalAppData%\Microsoft\Windows\History` (`walk.files`) + `winreg` sulla chiave `NewTabPage` | Basso. `WebCache\WebCacheV01.dat` resta (sempre aperto da un processo di sistema) | F2, F4 |
| Cookie | File: `INetCookies` (routine `N_INT_COOKIES`, rispetta «Cookie da conservare») | Sì | Nuovo XML | opzione `ie_cookies`: `%LocalAppData%\Microsoft\Windows\INetCookies` (`walk.files`), **senza** lista da conservare | Medio: disconnette dai siti nelle app che usano WinINet. Spenta di default qui (come tutti i cookie, spec riga 163) | F2, F4 |
| URL digitati di recente | Registro: `HKCU\Software\Microsoft\Internet Explorer\TypedURLs`, `TypedURLsTime`, `Explorer Bars\{C4EE31F3-…}\FilesNamedMRU`, `…\ContainingTextMRU` | Sì | Nuovo XML | opzione `ie_typed_urls`: 4 azioni `winreg` con le chiavi di F2 | Basso | F2, F4 |
| File Index.dat | File: `index.dat` (routine `N_INT_INDEXDAT`) | Sì | Non fattibile | — | Da Windows 8 `index.dat` non esiste più; il sostituto `WebCacheV01.dat` è sempre aperto da `taskhostw`/`dllhost` | F2, F4 |
| Ultimo percorso di download | Registro: valori `HKCU\Software\Microsoft\Internet Explorer\Download Directory` e `…\Main\Save Directory` | Sì | Nuovo XML | opzione `ie_last_download`: 2 azioni `winreg` con `name=` | Basso | F2, F4 |
| Cronologia moduli con completamento automatico | Registro IntelliForms (routine interna `N_INT_AUTOCOMPLETE`) | No (dedotto) | Nuovo XML | opzione `ie_autocomplete`: `winreg` su `HKCU\Software\Microsoft\Internet Explorer\IntelliForms\Storage1` — chiave **da verificare** (F2 non la scrive, usa la routine interna) | Medio: F4 avverte che con IE questa pulizia cancella anche le password salvate | F2, F4 |
| Password salvate | Credenziali «Web» nel Vault di Windows (routine `N_INT_PASSWORD`) | No (dedotto) | Non fattibile | — | Il Vault è protetto dalla guardia (`guard.rs:126`) e si svuota solo con l'API WinRT `PasswordVault`, che vale per tutte le app: rischio alto, valore basso | F2, F4 |

### 2.2 Microsoft Edge

Due sezioni diverse: Edge legacy (EdgeHTML, 3005, in F2) ed Edge Chromium (3006, in `Winsys.ini` dalla 6.x, F5).

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Edge legacy: Cache Internet, Cronologia Internet, Cookie, Cronologia download, Ultimo percorso di download, Sessione, Schede messe da parte, URL digitati di recente, Informazioni moduli salvate, Password salvate (10 voci) | File e registro del pacchetto `Microsoft.MicrosoftEdge_8wekyb3d8bbwe` (routine `N_EDGE_*` e chiavi `AppContainer\Storage\microsoft.microsoftedge_…`) | Sì per Cache, Cronologia, Cookie, Cronologia download, Ultimo percorso, URL digitati; No per Sessione, Schede messe da parte, Moduli, Password | Non fattibile | — | Il programma non esiste più: Edge legacy è stato tolto da Windows 10 nel 2021 e non c'è in Windows 11. Le chiavi di F2 sarebbero un XML da 10 righe se servisse un PC Windows 10 vecchio | F2 |
| Edge Chromium: Cache Internet | File: `%LocalAppData%\Microsoft\Edge*\User Data\*\Cache…` | Sì (come Chrome in F3; da verificare) | Winapp2 | `Microsoft Edge Caches *` | Basso | F3 (analogia), F5, W |
| Edge Chromium: Cronologia Internet | File: database `History` e affini | Sì (da verificare) | Winapp2 | `Microsoft Edge Web Browsing History *` (cancella `History*`, `Top Sites*`, `Visited Links*`, `shortcuts*`…) | Medio: il file `History` contiene anche la cronologia download; browser aperto = file saltati | F3 (analogia), W |
| Edge Chromium: Cookie | File: `Network\Cookies` | Sì (da verificare) | Codice nativo nuovo | N9 (cookie con lista da conservare); senza lista c'è già `Microsoft Edge Web Browsing Cookies *` | Medio: disconnette dai siti | F3 (analogia), S, W |
| Edge Chromium: Cronologia download | File | Sì (da verificare) | Winapp2 | `Microsoft Edge Download History *` (`DownloadMetadata`, `shared_proto_db`) | Parziale: la tabella `downloads` resta dentro `History` | F3 (analogia), W |
| Edge Chromium: Ultimo percorso di download | JSON: file `Preferences` | Sì (da verificare) | Codice nativo nuovo | N12 | Medio: un `Preferences` rovinato azzera le impostazioni del browser | F3 (analogia) |
| Edge Chromium: Sessione | File: `Sessions`, `Extension State` | **No** | Winapp2 | `Microsoft Edge Web Browsing Session *` | Medio: si perdono le schede aperte | F5, W |
| Edge Chromium: Informazioni moduli salvate | File: `Web Data` | No (da verificare) | Winapp2 | `Microsoft Edge Autofill Data & Search Engine Preferences *` | Medio: `Web Data` contiene anche i motori di ricerca | F3 (analogia), W |
| Edge Chromium: Password salvate | File: `Login Data` | No (da verificare) | Winapp2 | `Microsoft Edge Saved Usernames & Passwords *` | **Alto**: password perse se non sincronizzate. Spenta, con avviso | F3 (analogia), S, W |
| Edge Chromium: Compatta database | SQLite: `VACUUM` dei database del profilo | No (da verificare) | Codice nativo nuovo | N10 | Basso se il browser è chiuso | F3 (analogia), S |

### 2.3 Esplora risorse (`LangSecRef` 3002)

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Documenti recenti | Registro `HKCU\…\Explorer\RecentDocs` + collegamenti in `Recent` (routine `N_EX_RECENTDOCS`) | No | Nuovo XML | opzione `recent_docs`: `%AppData%\Microsoft\Windows\Recent` `*.lnk` (`search="glob"`, solo primo livello) + `winreg` su `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs`. `Windows Recent Documents *` di Winapp2 non basta: tocca solo `HKU\…`, radice non supportata | Basso: si svuota «Recenti» di Esplora e di Start | F2, F4, S, W |
| Esegui (nel menu Start) | Registro `HKCU\…\Explorer\RunMRU` | Sì | Nuovo XML | opzione `run_mru`: `winreg` sulla chiave (nessuna voce Winapp2 la tocca) | Basso | F2, F4, S |
| Completamento automatico Assistente ricerca | Registro `HKCU\Software\Microsoft\Search Assistant\ACMru` | Sì | Nuovo XML | opzione `search_assistant`: `winreg` sulla chiave. La ricerca di Esplora moderna (`WordWheelQuery`) è già in `Windows Search *` | Basso; la chiave esiste solo su Windows vecchi | F2 |
| Altri MRU di Esplora | Registro: `Explorer\ComDlg32`, `FindComputerMRU`, `PrnPortsMRU`, `Map Network Drive MRU`, `ComputerDescriptions`, valore `HKLM\Software\Microsoft\Direct3D\MostRecentApplication\Name` | No | Nuovo XML | opzione `explorer_mrus`: 6 azioni `winreg` con le chiavi di F2. Non uso `Windows Shell - Folder/Dialog Box View Settings *`: cancella anche `BagMRU`/`Bags` (le viste delle cartelle), cioè più di CCleaner | Basso | F2, F4, W |
| Cache miniature | File: `%LocalAppData%\Microsoft\Windows\Explorer\thumbcache_*.db` e `ThumbCacheToDelete\thm*.tmp` | Sì | Nativa esistente | «Cache delle miniature» (`src/cleaner.rs:48`). Da aggiungere `ThumbCacheToDelete\thm*.tmp` | Basso. Differenza: PulisciPC la tiene spenta | F1, F2, F4 |
| Jump list della barra delle applicazioni | File: `Recent\AutomaticDestinations`, `CustomDestinations` (routine `N_EX_JUMP_LISTS`) | Sì | Winapp2 | `Windows Taskbar *` | Medio: si perdono anche gli elementi **aggiunti** alle jump list | F2, F4, W |
| Password di rete | Registro `HKCU\Software\Microsoft\Ftp\Accounts` + credenziali di Windows (routine `N_EX_NETWORK_PASSWORDS`) | No | Codice nativo nuovo | N8 | **Alto**: si perdono le password delle unità di rete | F2, F4 |

### 2.4 Sistema (`LangSecRef` 3003)

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Svuota Cestino | Non-file: `SHEmptyRecycleBinW` | Sì | Nativa esistente | «Cestino» (`cleaner.rs:42`) | Basso. CCleaner ha anche l'opzione «solo file più vecchi di 24 ore» (F1, 0x141ab7e70): PulisciPC no | F1, F2, F4 |
| File temporanei | File: `%TEMP%`, `%WinDir%\Temp` (routine `N_TEMP_DIRS`), opzione 24/48 ore | Sì | Nativa esistente | «File temporanei dell'utente» + «File temporanei di Windows» (`cleaner.rs:43-44`, 24 ore). Le cartelle Temp dei profili di servizio sono in `Windows Temporary Files *`, ma stanno in parte sotto `System32` e la guardia le scarta | Basso | F1, F2, F4 |
| Appunti | Non-file: contenuto degli Appunti | Sì | Codice nativo nuovo | N1 | Basso: si perde quello che si è copiato | F2, F4, S |
| Dump di memoria | File: `%WinDir%\memory.dmp`, `Minidump\*.dmp`, `LiveKernelReports\*.dmp`, `%LocalAppData%\CrashDumps\*.dmp` | Sì | Nativa esistente | «Dump di memoria» (`cleaner.rs:45`): gli stessi 4 percorsi di F2 | Basso | F1, F2, F4 |
| Frammenti file di chkdsk | File: `%SystemDrive%\File*.chk` | Sì | Nuovo XML | opzione `chkdsk`: `%SystemDrive%\FILE*.CHK` e `%SystemDrive%\FOUND.*\FILE*.CHK` (`search="glob"`). F2 dice solo `File*.chk` nella radice; le cartelle `FOUND.nnn` sono dove chkdsk li mette di solito | Basso | F2, F4, S |
| File di log di Windows | File: `wbem\Logs\*.log;*.lo_`, `%WinDir%\*.log;*.bak;*log.txt`, `Dr Watson\*.log` | Sì | Nativa esistente | «File di log di Windows» (`cleaner.rs:46`). Da aggiungere `%WinDir%\*.bak`, `*log.txt` e `System32\wbem\Logs` (codice nativo, non passa dalla guardia di `pc-rules`) | Basso | F1, F2, F4 |
| Segnalazione errori di Windows | File: `WER\ReportArchive`, `ReportQueue`, `%LocalAppData%\ElevatedDiagnostics`; registro `Windows Error Reporting\Debug\StoreLocation` | No | Nativa esistente | «Segnalazione errori di Windows» (`cleaner.rs:47`). Mancano `ElevatedDiagnostics` e il valore di registro | Basso. Differenza: PulisciPC la tiene **accesa**, CCleaner (2018) spenta | F1, F2, F4 |
| Cache DNS | Non-file: cache del resolver | No | Codice nativo nuovo | N2 | Basso | F2, F4, S |
| Cache dei caratteri | File: `System32\FNTCACHE.DAT`, `%LocalAppData%\GDIPFONTCACHEV1.DAT`, `ServiceProfiles\LocalService\AppData\Local\FontCache*.dat` | No (dedotto) | Winapp2 | `Windows Font Cache *` (`ServiceProfiles\LocalService\…\FontCache*.dat`). `FNTCACHE.DAT` sta in `System32`: escluso | Basso; file aperti dal servizio Cache dei caratteri → saltati finché gira | F2, F4, W |
| Collegamenti nel menu Start | File `.lnk` **non validi** (destinazione inesistente) nel menu Start (`F_STARTMENU`) | No | Codice nativo nuovo | N4 | Medio: una destinazione su disco di rete o USB staccato sembra mancante | F2, F4, S |
| Collegamenti sul Desktop | File `.lnk` non validi sul Desktop (`F_DESKTOP`) | No | Codice nativo nuovo | N4 | Medio, come sopra | F2, F4, S |
| Cache Web di Windows | File: `%UserProfile%\AppData\LocalLow\Microsoft\CryptnetUrlCache` (dalla 6.06) | Sì (osservato da un utente) | Winapp2 | `Windows Cryptographic Services *` (`…\CryptnetUrlCache\Content`) | Basso: Windows riscarica i certificati | F6, S, W |

### 2.5 Avanzate (`LangSecRef` 3004)

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Registri eventi di Windows | Non-file: canali del Visualizzatore eventi (`N_EX_WINDOWS_EVENT_LOGS`) | No | Codice nativo nuovo | N3. `Windows Event Logs *` di Winapp2 cancella i file `.evtx` in `System32\winevt\logs`: la guardia li scarta (ed è il modo sbagliato, i file sono aperti dal servizio) | Alto per la diagnosi: si perde la storia degli errori | F2, F5, F7 |
| Vecchi dati Prefetch | File: `%WinDir%\Prefetch\*.pf` vecchi (`N_INT_PREFETCH`) | No (dedotto) | Nativa esistente | «Vecchi dati Prefetch» (`cleaner.rs:49`, 14 giorni stimati) | Basso | F1, F2, F4 |
| Cache ordine menu | Registro `HKCU\…\Explorer\MenuOrder` | No (dedotto) | Nuovo XML | opzione `menu_order`: `winreg` sulla chiave | Basso | F2, F4 |
| Cache notifiche area di sistema | Registro: valori `IconStreams`, `PastIconsStream` in `HKCU\…\Explorer\TrayNotify` e `HKCU\Software\Classes\Local Settings\…\TrayNotify` | No (dedotto) | Nuovo XML | opzione `tray_notify`: 4 azioni `winreg` con `name=` | Basso: le icone nascoste/visibili tornano come all'inizio; Explorer riscrive i valori alla chiusura | F2, F4 |
| Cache dimensione/posizione finestre | Registro `HKCU\…\Explorer\StreamMRU`, `…\Explorer\Streams` | No (dedotto) | Nuovo XML | opzione `window_cache`: 2 azioni `winreg` | Basso | F2, F4 |
| Percorso ambiente | Non-file: la variabile `PATH` (`N_EX_ENVIRONMENT_PATH`; la doc scrive «TBA») | No (dedotto) | Codice nativo nuovo | N5 — comportamento **da verificare**: l'ipotesi è «toglie dal PATH le cartelle che non esistono» | **Alto**: un PATH sbagliato rompe strumenti e build | F2, F4 |
| Cronologia assistenza utente | Registro `HKCU\…\Explorer\UserAssist\{GUID}\Count` (4 GUID) | No (dedotto) | Nuovo XML | opzione `user_assist`: 4 azioni `winreg` con i GUID di F2 | Basso: si azzera «più usati» del menu Start | F2, F4, S |
| File di log di IIS | File: `%SystemDirectory%\LogFiles`, `%SystemDrive%\inetpub\logs\LogFiles` | No (dedotto) | Nuovo XML | opzione `iis_logs`: `%SystemDrive%\inetpub\logs\LogFiles` (`walk.files`). `System32\LogFiles` è escluso dalla guardia | Basso | F2, F4 |
| Programmi di disinstallazione degli hotfix | File: `$NtUninstallKB…$` (`N_EX_HOTFIX`) | No (dedotto) | Non fattibile | — | Esistono solo su Windows XP (F4: «XP only»); da Windows Vista gli aggiornamenti stanno in `WinSxS`, intoccabile | F2, F4 |
| Vecchia installazione di Windows | File: `C:\Windows.old` (`N_EX_PREVIOUS_WINDOWS_INSTALLATION`) | No | Codice nativo nuovo | N7 | **Alto**: non si torna più alla versione di Windows precedente | F2 |
| Cartelle personalizzate | La lista «Includi» delle opzioni di CCleaner (`N_EX_CUSTOMFOLDERS`) | No (dedotto) | Non fattibile | — | Non è una posizione di Windows ma una funzione di CCleaner (cartelle scelte dall'utente). Vedi punto aperto 3 | F2, F4 |
| Pulisci spazio libero | Non-file: spazio libero del disco e della MFT (`N_EX_WIPEFREESPACE`, `N_EX_WIPE_MFT_FREE_SPACE`); opzione «Wipe Cluster Tips» | No | Codice nativo nuovo | N6 | **Alto**: ore di lavoro, disco pieno per un momento, usura degli SSD (su SSD non serve: TRIM) | F1, F2, F4, S |
| Utilizzo dati di rete (ex «Windows Subsystems») | File: database SRU `%WinDir%\System32\sru` | Da verificare | Non fattibile | — | Il file sta in `System32` (escluso dalla guardia) ed è aperto dal servizio Criteri di diagnostica; servirebbe fermarlo. `Windows Subsystems *` di Winapp2 lo ha, ma viene scartato | F7, W |
| Ottimizzazione recapito | File: cache di Delivery Optimization | Da verificare (gruppo da verificare) | Nativa esistente | «File di Ottimizzazione recapito» (`cleaner.rs:50`) | Basso | F1 |
| Cache MUI | Registro `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache` | Da verificare (anche il gruppo: forse è la voce del Registro `R_MUICACHE`) | Nativa esistente | Scheda Registro, «Cache MUI» (`src/registry.rs:80`), che toglie solo le voci di programmi mancanti; per svuotarla tutta c'è `Windows Subsystems *` (RegKey `…\Shell\MuiCache`) | Basso | F1 (fnd_1), S, W |

### 2.6 Proposte di codice nativo nuovo

| Id | Voce | Proposta tecnica | Cosa si può rompere | Admin | Default PulisciPC |
|---|---|---|---|---|---|
| N1 | Appunti | `OpenClipboard(null)` → `EmptyClipboard()` → `CloseClipboard()` (windows-sys `Win32_System_DataExchange`). Riprova 3 volte se un'altra app tiene aperti gli Appunti. Anteprima: «Appunti con contenuto» se `CountClipboardFormats() > 0`. La cronologia di Win+V non si tocca | Si perde il contenuto copiato | No | Acceso (come CCleaner) |
| N2 | Cache DNS | `DnsFlushResolverCache()` da `dnsapi.dll` (export non documentato: caricare con `GetProcAddress`, se manca usare `ipconfig /flushdns`). Nessuna anteprima di dimensione: la voce mostra «1 operazione» | Niente di permanente: la cache si riempie da sola | Da verificare (si pensa di no) | Spento (come CCleaner) |
| N3 | Registri eventi | `EvtOpenChannelEnum`/`EvtNextChannelPath` + `EvtClearLog` (wevtapi, `Win32_System_EventLog`) per ogni canale con eventi; prima, a scelta, `EvtExportLog` in `Documents\PulisciPC\eventi\`. Anteprima: numero di canali e dimensione da `EvtGetLogInfo` | Si perde la storia per la diagnosi; il registro Sicurezza scrive l'evento 1102 (F5) | Sì | Spento |
| N4 | Collegamenti non validi (Start, Desktop) | Elencare i `.lnk` in `%AppData%\Microsoft\Windows\Start Menu`, `%ProgramData%\Microsoft\Windows\Start Menu`, Desktop dell'utente e pubblico; leggere la destinazione con `IShellLinkW::GetPath` (COM, `SLGP_RAWPATH`); proporre solo se la destinazione è su un disco **fisso presente** e manca con errore NotFound (stessa trappola di `handoff.md`: non usare `Path::exists`). Mai le destinazioni di rete, USB o le shell (`shell:`, URL) | Collegamenti a dischi staccati o in app pacchettizzate creduti rotti | Sì per `%ProgramData%` e Desktop pubblico | Spento (come CCleaner) |
| N5 | Percorso ambiente | Leggere `PATH` da `HKCU\Environment` e `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`; espandere `%var%` solo per il controllo; togliere le voci la cui cartella manca (NotFound, disco presente) e i doppioni; riscrivere con lo **stesso tipo** (`REG_EXPAND_SZ`) dopo il backup `.reg` di `regbackup.rs`; inviare `WM_SETTINGCHANGE` "Environment" | PATH di sistema rovinato; cartelle che compaiono più tardi (dischi esterni, SDK non ancora installati) | Sì per HKLM | Spento. Nota: è fuori da `Software`, quindi fuori dal `regguard` delle regole esterne: codice dedicato |
| N6 | Pulisci spazio libero | Per ogni volume scelto: cartella temporanea nel volume, scrivere file di zeri senza cache (`FILE_FLAG_NO_BUFFERING` + `FILE_FLAG_WRITE_THROUGH`) finché lo spazio è finito, poi tanti file piccoli per la MFT; cancellare tutto in ogni caso (anche su errore o annullamento). Rifiutare i volumi SSD (`IOCTL_STORAGE_QUERY_PROPERTY`, `IncursSeekPenalty = false`) | Disco pieno per minuti/ore (altre app falliscono a scrivere); usura degli SSD | No per le cartelle utente, sì nella radice di C: | Spento, con avviso e conferma |
| N7 | Vecchia installazione di Windows | Usare il gestore di Pulizia disco: impostare `StateFlags0099=2` sotto `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches\Previous Installations` e lanciare `cleanmgr /sagerun:99`; mai cancellare `Windows.old` a mano (permessi di TrustedInstaller). Anteprima: dimensione di `C:\Windows.old` | Niente più ritorno alla versione precedente | Sì | Spento |
| N8 | Password di rete | `CredEnumerateW` + `CredDeleteW` per le credenziali di tipo `CRED_TYPE_DOMAIN_PASSWORD` (unità di rete, Desktop remoto) e valori in `HKCU\Software\Microsoft\Ftp\Accounts`; l'anteprima elenca i nomi di destinazione, mai le password | Password perse, accessi a NAS/server da rifare | No | Spento. Vedi punto aperto 2 |
| N9 | Cookie con lista da conservare (tutti i browser) | Con `rusqlite` sul file `Cookies` (Chromium, tabella `cookies`, colonna `host_key`) e `cookies.sqlite` (Gecko, `moz_cookies.host`): `DELETE … WHERE` host non in lista. Lista in `Documents\PulisciPC\cookie_da_conservare.txt`, con i domini dell'account Microsoft/Google proposti al primo avvio. Solo a browser chiuso (`<running>` sul processo) | File bloccato o rovinato se il browser è aperto: si controlla prima e si salta | No | Spento (spec riga 163) |
| N10 | Compatta database | `VACUUM` con `rusqlite` sui database SQLite del profilo (Chromium: `History`, `Cookies`, `Web Data`, `Favicons`, `Top Sites`…; Gecko: `places.sqlite`, `cookies.sqlite`, `formhistory.sqlite`…), solo a browser chiuso, solo se `PRAGMA integrity_check` dà `ok`. Anteprima: spazio recuperabile = `freelist_count × page_size` | Database rovinato se scritto mentre il browser è aperto | No | Spento (come CCleaner per Chrome, Firefox, Opera, F3) |
| N11 | Cronologia e cronologia download di Firefox/Thunderbird | `places.sqlite` contiene anche i segnalibri, quindi non si cancella il file: `DELETE FROM moz_historyvisits`, poi `moz_places` senza segnalibri (`foreign_count = 0`); per i download `moz_annos` con `anno_attribute_id` dei download. A browser chiuso. Winapp2 non ha voci per questo | Segnalibri persi se la query è sbagliata: test su copia del profilo | No | Cronologia accesa, download accesi (come F3), solo dopo test |
| N12 | Ultimo percorso di download (Chromium, Firefox) | Chromium: togliere `savefile.default_directory` e `download.default_directory` da `Preferences` (JSON) a browser chiuso — chiavi **da verificare** sul browser vero. Firefox/Thunderbird: togliere `browser.download.lastDir` da `prefs.js` | `Preferences` scritto male = impostazioni del browser azzerate | No | Spento finché non è verificato (CCleaner: acceso) |

## 3. Scheda Applicazioni

### 3.1 Browser

Le voci e i default vengono da F3 (CCleaner 2018). Edge Chromium è nella scheda Windows (2.2), come in CCleaner. Un browser aperto tiene bloccati i suoi file: le voci sotto vanno marcate `<running>` sul processo del browser, come fa CleanerML (spec riga 143).

**Famiglia Chromium.** Google Chrome (3029) e i browser che Winapp2 elenca per CCleaner: Opera (3027), Opera GX (3035), Brave (3034), Vivaldi (3033), CCleaner Browser (3032), Avast Secure Browser (3037), AVG Secure Browser (3038), Arc (3039), Norton Private Browser (3043; in Winapp2 si chiama «Norton Secure Browser», corrispondenza da verificare), Avira Secure Browser (3044). Le voci Winapp2 hanno lo stesso schema per tutti: `<prefisso> <voce> *`, con prefisso `Google Chrome`, `Opera`, `Opera GX`, `Brave`, `Vivaldi`, `CCleaner Browser`, `Avast Secure Browser`, `AVG Secure Browser`, `Arc`, `Norton Secure Browser`, `Avira Secure Browser`. Ho controllato nel file che tutte le 8 voci usate qui sotto esistano per tutti gli 11 prefissi. Per i 10 browser diversi da Chrome e Opera l'elenco delle voci di CCleaner e i default non sono in F3: li suppongo uguali a Chrome (da verificare).

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Cache Internet | File: `Cache`, `Code Cache`, `GPUCache`… del profilo | Sì | Winapp2 | `<prefisso> Caches *` (per esempio `Google Chrome Caches *`, `Brave Caches *`) | Basso | F3, F4, W |
| Cronologia Internet | File: `History`, `Top Sites`, `Visited Links`, `Shortcuts`, `Network Action Predictor` | Sì | Winapp2 | `<prefisso> Web Browsing History *` | Medio: Winapp2 cancella il file `History` intero, quindi va via anche la cronologia download (CCleaner le separa) | F3, F4, W |
| Cronologia download | Righe `downloads` dentro `History` + `DownloadMetadata` | Sì | Winapp2 | `<prefisso> Download History *` (`DownloadMetadata`, `shared_proto_db`). Copertura **parziale**: le righe in `History` restano; per toglierle sole serve un `DELETE FROM downloads` (estensione di N10, opzionale) | Basso | F3, F4, W |
| Ultimo percorso di download | JSON: file `Preferences` | Sì | Codice nativo nuovo | N12 | Medio (vedi N12) | F3 |
| Cookie | SQLite: `Network\Cookies` | Sì | Codice nativo nuovo | N9 (con «Cookie da conservare»). Senza lista basta `<prefisso> Web Browsing Cookies *`: lo si usa finché N9 non c'è | Medio: disconnette da tutti i siti | F3, F4, S, W |
| Informazioni moduli salvate | SQLite: `Web Data` (compilazione automatica) | No | Winapp2 | `<prefisso> Autofill Data & Search Engine Preferences *` | Medio: `Web Data` contiene anche i motori di ricerca personalizzati e (Chrome) le carte salvate | F3, F4, W |
| Password salvate | SQLite: `Login Data` | No | Winapp2 | `<prefisso> Saved Usernames & Passwords *` | **Alto**: password perse se non sincronizzate. Spenta, con avviso | F3, F4, S, W |
| Sessione | File: `Sessions`, `Extension State` | No | Winapp2 | `<prefisso> Web Browsing Session *` | Medio: si perdono le schede aperte | F3, F4, W |
| Compatta database | SQLite: `VACUUM` | No | Codice nativo nuovo | N10 | Basso a browser chiuso | F3, F4, S |
| Opera: URL digitati di recente | Righe `keyword_search_terms`/`typed_count` in `History` | Sì | Winapp2 | `Opera Web Browsing History *` / `Opera GX Web Browsing History *` (cancella `History` intero: copre, con lo stesso effetto collaterale della cronologia) | Medio | F3, W |
| Opera: Icone dei siti web | File: `Favicons` | Sì | Winapp2 | `Opera Bookmark Favicons *` / `Opera GX Bookmark Favicons *` | Basso | F3, F4, W |

**Famiglia Gecko.** Firefox (3026) e Thunderbird (3030) hanno le stesse 10 voci in F3, con le routine `N_MOZ_*` e `N_THUNDERBIRD_*`. Prefissi Winapp2: `Mozilla Firefox` e `Thunderbird`.

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Cache Internet | File: `%LocalAppData%\Mozilla\Firefox\Profiles\*\cache2` e simili | Sì | Winapp2 | `Mozilla Firefox Caches *`, `Thunderbird Caches *` | Basso | F3, F4, W |
| Cronologia Internet | SQLite: tabelle di `places.sqlite` (che contiene anche i **segnalibri**) | Sì | Codice nativo nuovo | N11. Winapp2 non ha una voce di cronologia per i browser Gecko (controllato: nessuna voce tocca `places.sqlite`) | Alto se fatto male: segnalibri persi | F3, F4, W |
| Cronologia download | SQLite: annotazioni dei download in `places.sqlite` | Sì | Codice nativo nuovo | N11 | Come sopra | F3, F4 |
| Cookie | SQLite: `cookies.sqlite` | Sì | Codice nativo nuovo | N9; senza lista c'è `Mozilla Firefox Web Browsing Cookies *` / `Thunderbird Web Browsing Cookies *` | Medio: disconnette dai siti; in Thunderbird anche da alcuni account di posta via web | F3, F4, S, W |
| Informazioni moduli salvate | SQLite: `formhistory.sqlite` | No | Winapp2 | `Mozilla Firefox Autocomplete History *` (`formhistory*`), `Thunderbird Autocomplete History *` | Basso | F3, F4, W |
| Ultimo percorso di download | `prefs.js`: `browser.download.lastDir` | Sì | Codice nativo nuovo | N12 | Medio: `prefs.js` scritto male = impostazioni perse; Firefox lo riscrive alla chiusura, quindi solo a browser chiuso | F3 |
| Password salvate | File: `logins.json`, `key4.db` | No (dedotto) | Winapp2 | `Mozilla Firefox Saved Usernames & Passwords *`, `Thunderbird Saved Usernames & Passwords *` | **Alto**: `key4.db` contiene anche i certificati client; in Thunderbird si perdono le password **degli account di posta**. Spenta, con avviso | F3, F4, S, W |
| Sessione | File: `sessionstore.jsonlz4`, `sessionstore-backups` | No | Winapp2 | `Mozilla Firefox Web Browsing Session *` + `Mozilla Firefox Web Browsing Session Backups *` (e le due di Thunderbird) | Medio: si perdono le schede aperte | F3, F4, W |
| Preferenze dei siti | SQLite: `permissions.sqlite`, `content-prefs.sqlite` | No | Winapp2 | `Mozilla Firefox Website Permissions *`, `Thunderbird Website Permissions *` | Basso: i permessi (fotocamera, notifiche, zoom) vanno richiesti di nuovo | F3, W |
| Compatta database | SQLite: `VACUUM` | No | Codice nativo nuovo | N10 | Basso a browser chiuso | F3, F4, S |

**Safari per Windows** (3028). Apple non lo aggiorna dal 2012 (versione 5.1.7); CCleaner lo rileva da `%ProgramFiles%\Safari\Safari.exe`. Winapp2 non ha voci Safari (controllato).

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Cache Internet | File: `%LocalAppData%\Apple Computer\Safari\Cache.db` | Sì | Nuovo XML | `cleaners/safari.xml` (id `safari`), opzione `cache`: il percorso di F3 | Basso | F3 |
| Cronologia Internet | File: `%AppData%\Apple Computer\Safari\History.plist`, `LastSession.plist`, `TopSites.plist`, `Downloads.plist`, `%LocalAppData%\Apple Computer\Safari\History\*`, `…\Webpage Previews` | Sì | Nuovo XML | opzione `history`: i 6 percorsi di F3 (F3 aggiunge anche la routine `N_SAFARI_HISTORY`, contenuto sconosciuto) | Basso | F3 |
| Cookie | Routine interna `N_SAFARI_COOKIES` | Sì | Nuovo XML | opzione `cookies`: `%AppData%\Apple Computer\Safari\Cookies\Cookies.binarycookies` — percorso **da verificare** (F3 non lo scrive) | Basso; nessuna lista da conservare (formato binario Apple) | F3 |
| Informazioni moduli salvate | File: `%AppData%\Apple Computer\Safari\Form Values.plist` | No | Nuovo XML | opzione `forms`: il percorso di F3 | Basso | F3 |
| Password salvate | Routine `N_SAFARI_PASSWORD` (portachiavi Apple) | No (dedotto) | Non fattibile | — | Formato del portachiavi Apple non documentato, browser abbandonato | F3 |

### 3.2 Altre sezioni

Qui CCleaner mette una casella per programma («Applications, Internet, Multimedia, Utilities, Windows … the cleaning process will typically clear out document history (MRUs), visited URLs, and temporary files», F4). Nel 2018 CCleaner aveva 345 voci sue in queste sezioni (F3: 98 + 65 + 81 + 91 + 10) e 476 nella 6.00 (F5): l'elenco di oggi non è disponibile. Winapp2 copre le stesse sezioni con molte più voci; dove CCleaner ha un programma che Winapp2 non ha, non lo so dire senza l'export (vedi 1). I conteggi sono sul file W: «voci» = sezioni `[… *]`, `FileKey`/`RegKey` = righe di regola reali. La guardia può scartarne una parte: il numero di regole davvero attive su questo PC lo dà il rapporto di `pc-rules` all'avvio, non questa tabella.

| Voce (come in CCleaner, italiano) | Cosa tocca | Default CCleaner | Copertura PulisciPC | Come | Rischio/avvisi | Fonte |
|---|---|---|---|---|---|---|
| Sezione Applicazioni (3021) | File e registro: MRU, temporanei, log dei programmi | Per voce (F3: `Default=True` per la maggior parte, es. WinRAR; `False` per alcune) | Winapp2 | 491 voci, 1570 `FileKey`, 949 `RegKey`. Esempi: `Microsoft Office *`, `LibreOffice *`, `Notepad++ *`, `Microsoft OneDrive *`, `GIMP *` | Basso-medio: alcune voci toccano file non rigenerabili (es. `Notepad++ - Backups & Unsaved Files *`, `Microsoft Office Unsaved Files *`): restano spente come tutte le voci Winapp2 | F3, F4, F8, W |
| Sezione Internet (3022) | File e registro di client di chat, posta, download | Per voce | Winapp2 | 271 voci, 780 `FileKey`, 150 `RegKey`. Esempi: `Discord *`, `Telegram Desktop *`, `Zoom *`, `WhatsApp *` (in CCleaner sezione propria 3042), `Windows Mail *` | Basso | F3, F4, F8, W |
| Sezione Multimedia (3023) | File e registro di lettori, editor, convertitori | Per voce | Winapp2 | 451 voci, 1457 `FileKey`, 741 `RegKey`. Esempi: `VLC Media Player *`, `Spotify *` (in CCleaner sezione propria 3036), `Apple iTunes *` (in CCleaner 3040), `Audacity *`, `4K Video Downloader *` | Medio per `Spotify - Downloaded Songs *` (brani scaricati) | F3, F4, F8, W |
| Sezione Utilità (3024) | File e registro di archiviatori, antivirus, strumenti di sistema | Per voce (F5: `Windows Defender` `Default=False`) | Winapp2 | 944 voci, 2619 `FileKey`, 1319 `RegKey`. Esempi: `7-Zip *`, `WinRAR *`, `Malwarebytes *`, `NVIDIA App *`, `BOINC Manager *` | Basso | F3, F4, F5, F8, W |
| Sezione Windows (3025) | File e registro delle app di Windows (Paint, Blocco note, Store…) | Per voce | Winapp2 | 74 voci, 536 `FileKey`, 140 `RegKey`. Esempi: `.NET Framework *`, `Windows Notepad *`, `Windows Photos *`, `Windows Snipping Tool *`, `Microsoft Store *`. Le voci che fanno da doppione alla scheda Windows sono già nella sezione 2 | Medio per `Windows Activity History *` (ha `Warning=`) e `Windows Credential Manager Saved Credentials *` (`Vault`, scartata dalla guardia) | F3, F4, F8, W |
| Sezione Windows Store (3031) | File delle app UWP | Per voce (F3: `True`) | Winapp2 | Winapp2 non usa più 3031: le app del Microsoft Store sono voci normali nelle sezioni 3021-3025 (es. `Universal Windows Platform *`, `Microsoft Store *`, `Windows Calculator *`, `Windows Camera *`, `Microsoft Sticky Notes *`). Le 10 voci di CCleaner 2018 (Skype Metro, Bing News…) sono app tolte da Windows | Basso | F3, F8, W |
| Sezione Adobe | File e registro dei programmi Adobe | Per voce | Winapp2 | `Section=Adobe`: 31 voci, 422 `FileKey`, 109 `RegKey`. Esempi: `Adobe Acrobat Reader *`, `Adobe Photoshop *`, `Adobe Illustrator *`, `Adobe Audition *`, `Adobe Dreamweaver *` | Basso | F8, W |
| Sezione Giochi | File di launcher e giochi (log, cache, installer) | Per voce | Winapp2 | `Section=Games`: 521 voci, 1326 `FileKey`, 19 `RegKey`. Esempi: `Steam *`, `Epic Games Launcher *`, `Minecraft *`, `Battle.net *`, `EA App *` | Medio per `Steam Installers *` e `Ubisoft Connect Installers *` (hanno `Warning=`: installer da riscaricare) | F8, W |
| Altre sezioni Winapp2 senza equivalente in CCleaner | File di PowerToys e dei browser che CCleaner non conosce | — | Winapp2 | `Section=Microsoft PowerToys` 30 voci (es. `Microsoft PowerToys *`, `Microsoft PowerToys Hosts File Editor *`), `Section=Command Palette` 1 voce, 50 sezioni di altri browser con 962 voci (le sezioni browser sono 64 con 1254 voci; le 14 di 3.1 e 2.2 ne hanno 292; es. `Yandex Caches *`, `Maxthon Caches *`, `LibreWolf Caches *`, `Waterfox Caches *`, `Zen Caches *`) | Come le righe dei browser in 3.1 | W |
| Strumenti di IA (categoria nuova in CCleaner, «AI Tools») | File di ChatGPT, Claude e altre app di IA | Da verificare | CleanerML inglobato | `ai_claude_code`, `ai_codex`, `ai_gemini_cli`, `ai_opencode` + Winapp2 `Anthropic Claude *`, `Microsoft Copilot *`. L'elenco delle app nella categoria di CCleaner è **da verificare** (nota di rilascio vista solo in un risultato di ricerca, non letta); per ChatGPT desktop Winapp2 non ha voci: eventuale `Nuovo XML` | Medio: le cartelle degli agenti contengono credenziali, già escluse dalla guardia (`guard.rs:137-141`) | ricerca web (nota di rilascio CCleaner, da verificare), W |

## 4. Riepilogo con numeri

### 4.1 Righe per valore di copertura

Una riga = una voce di CCleaner, salvo due casi: la riga «Edge legacy» raccoglie 10 voci, e ogni riga della famiglia Chromium vale per 11 browser (ogni riga Gecko per 2).

| Copertura | Scheda Windows (52 righe) | Browser (26 righe) | Altre sezioni (10 righe) | Totale (88) |
|---|---|---|---|---|
| Nativa esistente | 9 | 0 | 0 | 9 |
| Winapp2 | 9 | 13 | 9 | 31 |
| CleanerML inglobato | 0 | 0 | 1 | 1 |
| Nuovo XML | 16 | 4 | 0 | 20 |
| Codice nativo nuovo | 12 | 8 | 0 | 20 |
| Non fattibile | 6 | 1 | 0 | 7 |

Le 20 righe «Codice nativo nuovo» si riducono a **12 funzioni** (N1-N12, sezione 2.6), perché la stessa funzione serve più browser. Le 20 righe «Nuovo XML» sono **2 file**: `cleaners/windows_extra.xml` (16 opzioni) e `cleaners/safari.xml` (4 opzioni). Le 7 «Non fattibile»: Index.dat, password di IE, Edge legacy (10 voci), programmi di disinstallazione degli hotfix, cartelle personalizzate, utilizzo dati di rete, password di Safari.

Ritocchi alle categorie native già esistenti emersi dal confronto con F2 (piccoli, non contati come righe nuove): «Cache delle miniature» + `ThumbCacheToDelete\thm*.tmp`; «File di log di Windows» + `%WinDir%\*.bak`, `%WinDir%\*log.txt`, `System32\wbem\Logs\*.log;*.lo_`; «Segnalazione errori di Windows» + `%LocalAppData%\ElevatedDiagnostics` e il valore `HKCU\Software\Microsoft\Windows\Windows Error Reporting\Debug\StoreLocation`.

### 4.2 Lavoro vero della Fase 3, in ordine di rischio

| Ordine | Lavoro | Rischio | Stima |
|---|---|---|---|
| 1 | N6 Pulisci spazio libero | Alto (disco pieno, SSD) | 6 h |
| 2 | N5 Percorso ambiente (PATH) | Alto (rompe strumenti; comportamento CCleaner da verificare) | 3 h |
| 3 | N11 Cronologia e download di Firefox/Thunderbird (`places.sqlite`) | Alto (segnalibri) | 4 h |
| 4 | N8 Password di rete | Alto (credenziali) | 2 h |
| 5 | N7 Vecchia installazione di Windows (via `cleanmgr /sagerun`) | Alto (niente ritorno indietro) | 2 h |
| 6 | N3 Registri eventi | Alto per la diagnosi, basso per il sistema | 2 h |
| 7 | N9 Cookie con lista da conservare | Medio | 4 h |
| 8 | N12 Ultimo percorso di download (`Preferences`, `prefs.js`) | Medio | 2 h |
| 9 | N4 Collegamenti non validi (Start, Desktop) | Medio | 3 h |
| 10 | N10 Compatta database | Basso a browser chiuso | 2 h |
| 11 | `cleaners/windows_extra.xml` (16 opzioni: IE, MRU, Esplora, chkdsk, IIS) | Basso (registro con backup `.reg`) | 2,5 h |
| 12 | Ritocchi alle categorie native (4.1) | Basso | 1 h |
| 13 | `cleaners/safari.xml` | Basso | 0,5 h |
| 14 | N1 Appunti | Basso | 0,5 h |
| 15 | N2 Cache DNS | Basso | 0,5 h |
| — | **Totale** | — | **35 h** |

La spec stimava 6-8 ore (riga 173): il conto è più alto perché Winapp2 copre meno del previsto (cronologia Gecko, cookie con lista, compattazione, voci di registro di Esplora e IE che la variante Non-CCleaner non ha più). Senza N6, N5, N7 e N8 (punto aperto 4) si scende a **22 h**.

### 4.3 Task proposti per il piano (raggruppati per file)

1. **Regole nuove e ritocchi** — `cleaners/windows_extra.xml`, `cleaners/safari.xml`, `crates/pc-rules/src/embedded.rs` (due righe `include_str!`), `src/cleaner.rs` (ritocchi di 4.1). Verifica: l'anteprima trova le chiavi `RunMRU`, `UserAssist`, `TypedURLs` su questo PC; i test di `pc-rules` passano con i due XML. **4 h.**
2. **Azioni di sistema non-file** — nuovo `src/sysclean.rs` con N1 Appunti, N2 DNS, N3 registri eventi, N4 collegamenti non validi; feature di `windows-sys` in `Cargo.toml` (`Win32_System_DataExchange`, `Win32_System_EventLog`, `Win32_System_Com`, `Win32_Security_Credentials` se entra N8); righe nuove in `CATEGORIES` di `src/cleaner.rs` con un `Kind` nuovo per le azioni che non sono file. **6 h.**
3. **Database dei browser** — nuovo `src/browserdb.rs` con N9, N10, N11, N12 (`rusqlite =0.40.2` `bundled`, la stessa versione di `crates/dc-mukade/Cargo.toml:14`, aggiunta al `Cargo.toml` radice), lista `Documents\PulisciPC\cookie_da_conservare.txt`, controllo del processo del browser prima di aprire i file, test su copie di profili finti in `tempfile`. **12 h.**
4. **Voci pericolose** (solo se il punto aperto 4 le conferma) — N5 PATH con backup `.reg` (`src/regbackup.rs`), N7 Windows.old, N8 password di rete, ognuna spenta, con conferma esplicita e messaggio di avviso. **7 h.**
5. **Pulisci spazio libero** (rimandabile) — nuovo `src/wipe.rs`, N6 con rilevamento SSD, annullamento e pulizia garantita dei file di riempimento. **6 h.**

## 5. Punti aperti per l'utente

1. **Recuperare CCleaner per chiudere il buco?** Con l'exe sul PC, `CCleaner64.exe /EXPORT` dà l'elenco e i default della versione attuale, e `lang-1040.dll` i nomi italiani veri. Senza, restano «da verificare» i default 6.x, i nomi italiani (salvo 14) e le voci nuove dopo il 2018 che non ho trovato.
2. **Credenziali**: fare N8 (password di rete, tocca Gestione credenziali di Windows)? Le password dei browser invece passano da Winapp2 e restano spente con avviso.
3. **«Cartelle personalizzate»** di CCleaner (Includi/Escludi scelti dall'utente) non è una voce di Windows: la segno «Non fattibile» come voce. Vuoi una funzione di PulisciPC per aggiungere cartelle proprie (sarebbe lavoro in più, fuori da questa tabella)?
4. **Budget**: 35 h contro le 6-8 della spec. Proposta: in v1 i task 1-3 (22 h), poi N6, N5, N7, N8 solo se servono. N5 (PATH) in particolare ha un comportamento in CCleaner che non ho potuto verificare.
5. **Default diversi da CCleaner** già oggi: PulisciPC accende «Segnalazione errori» (CCleaner 2018: spenta) e spegne «Cache delle miniature» (CCleaner: accesa). Allineare a CCleaner o tenere le scelte attuali?
