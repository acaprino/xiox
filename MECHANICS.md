# Xiox – regole ricavate dal programma originale

Fonte: CCleaner64.exe (sha256 30de2996…a5fe53), letto con il decompilatore integrato
(Ghidra non è partito). Ogni valore ha la sua fonte; "stimato" = cercato nel codice ma non trovato.

## 1. Pulizia file temporanei

| Regola | Valore | Fonte |
|---|---|---|
| Cartella temporanei utente | `%TEMP%` (espanso) | sub_140412ce0 ("Unable to expand %TEMP%…"), stringa 0x141afe920 |
| Cartella temporanei Windows | `%WINDIR%\Temp` | stringa `\Temp` 0x141afe990, `\TEMP` 0x141ae23f0 |
| Opzione età minima nelle cartelle Temp | 24 ore | stringa 0x141ab7df8 "Only delete files in Windows Temp folders older than 24 hours" |
| Cestino | SHQueryRecycleBinW / SHEmptyRecycleBinW | import 0x141acd920, 0x141f53d76; regola N_TEMP_RECYCLEBIN id 0x35 (sub_140206850) |
| Categorie di sistema | Cestino, File temporanei, Dump di memoria, File di log di Windows, Vecchi dati Prefetch, Cache miniature, Segnalazione errori Windows, Ottimizzazione recapito | risorse 0x1424e10e2, 0x1424e1126, 0x1424e116c, 0x1424e11ac, 0x1424e1002, 0x1424e1476; tabella Rule (Temporary_Files 3142, Memory_Dumps 3143, Windows_Log_Files 3145, Old_Prefetch_Data 3147, Windows_Error_Reporting 3149, Thumbnail_Cache 3131) |
| Prefetch | `%WINDIR%\Prefetch` | stringa 0x141ae1210 |
| Schemi file | `*.tmp`, `*.log` | stringhe 0x141ae3028, 0x141ae21e8 |
| Età "vecchi" Prefetch | 14 giorni | **stimato** (non trovato) |
| Percorsi dump/log/WER/recapito | cartelle standard di Windows | **stimato** (le stringhe esatte non sono state trovate) |

Ordine: Analizza = conta file e dimensioni; Avvia pulizia = elimina i file trovati, salta quelli in uso,
poi rimuove le sottocartelle vuote (mai la cartella principale). Collegamenti/junction non vengono seguiti.

## 2. Registro di sistema

sub_140206850 associa i nomi delle categorie a un numero interno:

| Categoria | id | Chiave controllata | Problema | Riparazione |
|---|---|---|---|---|
| DLL condivise mancanti (R_SHARED_DLLS) | 1 | `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\SharedDlls` (0x141aea040) | il file del nome valore non esiste | elimina valore |
| Estensioni file inutilizzate (R_FILE_EXTS) | 2 | `HKCU\Software\Classes\.ext` | ProgID inesistente | elimina chiave |
| ActiveX e classi (R_ACTIVEX) | 3 | `…\Classes\CLSID\{…}\InProcServer32 / LocalServer32` (0x141adb7b8, 0x141ae5cd0) | file del server mancante | elimina chiave CLSID |
| Librerie dei tipi (TypeLib) | 3615 | `HKLM\SOFTWARE\Classes\TypeLib` (0x141aaa528) | tutti i file win32/win64 mancanti | elimina versione |
| Caratteri (R_FONTS) | 5 | `Software\Microsoft\Windows NT\CurrentVersion\Fonts` (0x141ae9ec0) | file font mancante | elimina valore |
| Percorsi applicazioni (R_APP_PATHS) | 6 | `…\CurrentVersion\App Paths` (0x141ae9f30) | programma mancante | elimina chiave |
| File della guida (R_HELP) | 7 | `Software\Microsoft\Windows\Help`, `HTML Help` (0x141aea480, 0x141aea2f0) | file guida mancante | elimina valore |
| Cartelle installer (R_INSTALLER) | 9 | `…\Installer\Folders` (0x141aea340) | cartella mancante | elimina valore |
| Software obsoleto (R_OLDSOFTWARE) | 12 | `…\CurrentVersion\Uninstall` (0x141ac7cf0) | programma di disinstallazione mancante | elimina chiave |
| Esecuzione all'avvio (R_RUNSTARTUP) | 14 | `Software\Microsoft\Windows\CurrentVersion\Run` (0x141abff20) | programma mancante | elimina valore |
| Cache MUI (R_MUICACHE) | 16 | `Software\Classes\Local Settings\…\Shell\MuiCache` (0x141ae9df0) | programma mancante | elimina valore |
| Servizi Windows (R_WINDOWS_SERVICES) | 19 | `HKLM\SYSTEM\CurrentControlSet\Services` | file del servizio mancante | elimina chiave (spento di default) |

Non inclusi in questa versione: R_APP_OPENWITH (4), R_STARTMENUORDER (15), R_INTERFACE (17), R_SOUND_EVENTS (18).
Le regole esatte di ogni controllo non sono leggibili (le stringhe non hanno riferimenti risolti): **stimate**
come "il percorso indicato non esiste su un disco presente". Prima di riparare si offre il backup `.reg` (come l'originale).

## 3. Servizi (meno RAM)

L'originale non ha un tool per disattivare i servizi: il suo "Performance Optimizer" mette in sospensione
i programmi tramite un proprio servizio (stringhe 0x1424f4f70, 0x141abafd0) e usa
OpenSCManager / ChangeServiceConfigW / EnumServicesStatusExW (import 0x141f51662, 0x141f51728, 0x141f5198c).
Qui: elenco servizi da `HKLM\SYSTEM\CurrentControlSet\Services`, stato e memoria da `tasklist`,
modifica dell'avvio con `sc config` (Disattivato = 4, Manuale = 3). L'elenco dei servizi consigliati
e quello dei servizi protetti sono **stimati** (scelta nostra, prudente). Lo stato originale di ogni
servizio viene salvato prima della prima modifica, per poterlo ripristinare.
