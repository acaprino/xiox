# Verifica dei percorsi: lotto «web» (Task 9, Parte B)

Ogni voce ha la sua fonte (documentazione o repository ufficiale dello strumento). Dove la fonte dice `~` si intende `%USERPROFILE%`. File 🔒 non toccati: `token`, `stored_tokens` (Hugging Face), `keras.json`, `settings.json` e `keybindings.json` (editor), `auth.ini` di pnpm.

Convenzione: «tolta» = voce non documentata, non scritta nel cleaner. «Variabile non nella lista consentita» = nome non presente in `TOOL_VARS` di `crates/pc-rules/src/model.rs`; il percorso è scritto senza quel valore.

## npm (`cleaners/bbag_npm.xml`, opzione `custom_cache`)

| Percorso | Fonte |
|---|---|
| `%LocalAppData%\npm-cache` (predefinito, già nel file) | https://docs.npmjs.com/cli/v10/configuring-npm/folders («`%LocalAppData%/npm-cache` on Windows») e https://docs.npmjs.com/cli/v10/using-npm/config#cache |
| `%npm_config_cache%` | https://docs.npmjs.com/cli/v10/using-npm/config: «Any environment variables that start with `npm_config_` will be interpreted as a configuration parameter». La pagina non ha un esempio con `npm_config_cache`: la voce si ricava da questa regola |
| `\_cacache` | https://github.com/npm/cli/blob/latest/docs/lib/content/commands/npm-cache.md («an opaque directory within the configured `cache`, named `_cacache`») |
| `\_logs` | https://docs.npmjs.com/cli/v10/using-npm/config#logs-dir («A directory named `_logs` inside the cache») |
| `\_npx` | https://github.com/npm/cli/blob/latest/workspaces/libnpmexec/README.md (`npxCache: '~/.npm/_npx'`) |

Scelta: l'opzione nuova `custom_cache` compare solo se `npm_config_cache` è definita (un `<var>` senza valori fa sparire le azioni). Le tre opzioni esistenti non cambiano.

## pnpm (`cleaners/pnpm.xml`)

| Percorso | Fonte |
|---|---|
| `%LocalAppData%\pnpm\store` | https://pnpm.io/settings/store, `storeDir`: «On Windows: ~/AppData/Local/pnpm/store» |
| `%LocalAppData%\pnpm-cache` | https://pnpm.io/settings/other, `cacheDir`: «On Windows: ~/AppData/Local/pnpm-cache» |

Non si tocca `%LocalAppData%\pnpm\config` (contiene `auth.ini`, https://pnpm.io/npmrc). `PNPM_HOME` e `XDG_*` non sono nella tabella né nella lista consentita.

## Yarn (`cleaners/yarn.xml`)

| Percorso | Fonte |
|---|---|
| `%LocalAppData%\Yarn\Cache` (Classic) | https://github.com/yarnpkg/yarn/blob/master/src/util/user-dirs.js, `getCacheDir`: su win32 `path.join(LOCALAPPDATA\Yarn, 'Cache')`. La pagina https://classic.yarnpkg.com/lang/en/docs/cli/cache/ dà solo `yarn cache dir`, senza il percorso Windows |
| `%LocalAppData%\Yarn\Berry\cache` | https://github.com/yarnpkg/berry/blob/master/packages/yarnpkg-core/sources/folderUtils.ts (win32: `LOCALAPPDATA/Yarn/Berry`) e `Configuration.ts` riga 1417 (`cacheFolder = ${globalFolder}/cache` con `enableGlobalCache`) |
| `%UserProfile%\.yarn\berry\cache` | https://yarnpkg.com/configuration/yarnrc: `globalFolder` predefinita `${HOME}/.yarn/berry`, `enableGlobalCache: true` |

`YARN_CACHE_FOLDER` non è nella tabella né nella lista consentita. La `.yarn\cache` dei progetti non si tocca (Parte A).

## Bun (`cleaners/bun.xml`)

| Percorso | Fonte |
|---|---|
| `%BUN_INSTALL_CACHE_DIR%`, `%UserProfile%\.bun\install\cache` | https://bun.sh/docs/install/cache («global cache at `~/.bun/install/cache`, or the path set by the `BUN_INSTALL_CACHE_DIR` environment variable»). La pagina non ha un percorso specifico per Windows |

## Deno (`cleaners/deno.xml`)

| Percorso | Fonte |
|---|---|
| `%DENO_DIR%`, `%LocalAppData%\deno` | https://docs.deno.com/runtime/getting_started/installation/ («Cache location», Windows: `%LOCALAPPDATA%\deno`; override con `DENO_DIR`) |
| sottocartelle `remote`, `npm`, `gen`, `registries`, `dl` e file `*_cache_v2*` (`v8_code_cache_v2`, `check_cache_v2`, `dep_analysis_cache_v2`, `fast_check_cache_v2`, `node_analysis_cache_v2`, `fmt_incremental_cache_v2`, `lint_incremental_cache_v2`) | Sorgente ufficiale `cache/deno_dir.rs` (crate `deno_resolver` 0.77.0 di denoland): https://docs.rs/crate/deno_resolver/0.77.0/source/cache/deno_dir.rs |

Differenza dalla tabella (che elenca tutto `%DENO_DIR%`): si lasciano fuori `location_data` (dati di origine, cioè localStorage dei programmi lanciati con `--location`), `deno_history.txt` (cronologia del REPL) e `latest.txt`: sono dati dell'utente o di stato, non cache. Una cartella `deps` (nome usato da Deno 1.x) non compare nel sorgente attuale e non è inclusa.

## Electron (`cleaners/electron.xml`)

| Percorso | Fonte |
|---|---|
| `%LocalAppData%\electron\Cache` | https://www.electronjs.org/docs/latest/tutorial/installation, sezione Cache («Windows: `%LOCALAPPDATA%/electron/Cache`»); https://github.com/electron/get (How It Works) |
| `%ELECTRON_CACHE%` | Documentata solo nelle guide di Electron 8: https://github.com/electron/electron/blob/v8.0.0-beta.2/docs/tutorial/installation.md. La guida attuale usa `electron_config_cache`. **Variabile non nella lista consentita: electron_config_cache** |
| `%LocalAppData%\electron-builder\Cache` | https://www.electron.build/docs/tutorials/offline-air-gapped-builds («`%LOCALAPPDATA%\electron-builder\Cache` on Windows»). **Variabile non nella lista consentita: ELECTRON_BUILDER_CACHE** (la tabella non la usa) |

## Playwright (`cleaners/playwright.xml`)

| Percorso | Fonte |
|---|---|
| `%PLAYWRIGHT_BROWSERS_PATH%`, `%LocalAppData%\ms-playwright` | https://playwright.dev/docs/browsers («`%USERPROFILE%\AppData\Local\ms-playwright` on Windows»; `PLAYWRIGHT_BROWSERS_PATH`) |

Il valore `0` di `PLAYWRIGHT_BROWSERS_PATH` (cartella locale in `node_modules`) non è un percorso con lettera di unità: `Env::for_rules` lo scarta.

## Puppeteer (`cleaners/puppeteer.xml`)

| Percorso | Fonte |
|---|---|
| `%UserProfile%\.cache\puppeteer` | https://pptr.dev/guides/configuration («Starting in v19.0.0, Puppeteer stores browsers in `~/.cache/puppeteer`»). Nessun percorso specifico per Windows |
| `%PUPPETEER_CACHE_DIR%` | https://pptr.dev/troubleshooting («the `PUPPETEER_CACHE_DIR` variable») e https://pptr.dev/api/puppeteer.configuration.cachedirectory |

## Cypress (`cleaners/cypress.xml`)

| Percorso | Fonte |
|---|---|
| `%LocalAppData%\Cypress\Cache`, `%CYPRESS_CACHE_FOLDER%` | https://docs.cypress.io/app/references/advanced-installation, «Binary cache» (Windows: `/AppData/Local/Cypress/Cache`, relativo al profilo; `CYPRESS_CACHE_FOLDER`) |

## VS Code e derivati (`cleaners/vscode.xml`)

| Percorso | Fonte |
|---|---|
| `%AppData%\Code` | https://code.visualstudio.com/docs/setup/uninstall («Windows - Delete `%APPDATA%\Code`…») |
| `%AppData%\Code - Insiders` | https://code.visualstudio.com/docs/configure/profiles («If you are using the Insiders version, the intermediate folder name is `Code - Insiders`») |
| `%AppData%\VSCodium` | https://github.com/VSCodium/vscodium/blob/master/docs/migration.md («Windows: `%APPDATA%\VSCodium\User`») |
| `%AppData%\Cursor` | Solo la cartella `Cache` è documentata, da un messaggio del team Cursor sul forum ufficiale: https://forum.cursor.com/t/completely-reinstall-cursor-and-remove-all-settings/1743/13 (`%UserProfile%\AppData\Roaming\Cursor\Cache`). Le altre sottocartelle valgono per Cursor perché è un derivato di VS Code (stessa struttura): **dubbio, vedi il rapporto** |
| `%AppData%\Windsurf` | https://docs.devin.ai/desktop/devin-desktop-faq («Per-user IDE data»: `%APPDATA%\Windsurf\` (legacy) e `%APPDATA%\Devin\` (new); «This is the VS Code-derived user data directory»). La cartella nuova `Devin` non è nella tabella e non è inclusa |
| `logs`, `CachedExtensionVSIXs` | https://github.com/microsoft/vscode/blob/main/src/vs/platform/environment/common/environmentService.ts (`join(this.userDataPath, 'logs', key)`; `join(this.userDataPath, 'CachedExtensionVSIXs')`) |
| `CachedData` | https://github.com/microsoft/vscode/blob/main/src/mainImpl.ts riga 667 (`path.join(userDataPath, 'CachedData', commit)`) |
| `Cache`, `Code Cache`, `GPUCache` | Cartelle di Chromium nella cartella dei dati utente. Fonte indiretta: https://github.com/microsoft/vscode/issues/97315 (le elenca fra le cartelle di cache di `Code`; un maintainer risponde che molte sono «part of the Chromium user data directory»). Nessuna pagina ufficiale dice in modo esplicito che si possono cancellare: **dubbio, vedi il rapporto** |

Non si toccano (la spec le mette al livello Attenzione o 🔒): `User\History`, `User\workspaceStorage`, `Local Storage`, `Workspaces`, `Backups` (`environmentMainService.ts`: `join(this.userDataPath, 'Backups')`), `User\settings.json`, `User\keybindings.json`. I nomi dei processi di `<running>` (`Code.exe`, `Code - Insiders.exe`, `VSCodium.exe`, `Cursor.exe`, `Windsurf.exe`) non sono percorsi e non hanno una pagina di documentazione: non verificati.

## Firebase (`cleaners/firebase.xml`)

| Percorso | Fonte |
|---|---|
| `%UserProfile%\.cache\firebase\emulators` | https://github.com/firebase/firebase-tools/blob/master/src/emulator/downloadableEmulators.ts: `CACHE_DIR = process.env.FIREBASE_EMULATORS_PATH \|\| path.join(os.homedir(), ".cache", "firebase", "emulators")`. **Variabile non nella lista consentita: FIREBASE_EMULATORS_PATH** (la tabella non la usa) |

## Hugging Face (`cleaners/huggingface.xml`)

| Percorso | Fonte |
|---|---|
| `%HF_HUB_CACHE%`, `%HF_HOME%\hub`, `%UserProfile%\.cache\huggingface\hub` | https://huggingface.co/docs/huggingface_hub/package_reference/environment_variables (`HF_HOME` predefinita `~/.cache/huggingface`; `HF_HUB_CACHE` = `$HF_HOME/hub`) |
| `%UserProfile%\.cache\huggingface\xet`, `\assets` (e con `%HF_HOME%`) | stessa pagina (`HF_XET_CACHE` = `$HF_HOME/xet`; `HF_ASSETS_CACHE` = `$HF_HOME/assets`) |
| `%UserProfile%\.cache\huggingface\datasets` (e con `%HF_HOME%`) | https://huggingface.co/docs/datasets/cache («The default datasets cache directory is `~/.cache/huggingface/datasets`»; con `HF_HOME` diventa `<HF_HOME>/datasets`) |

Estensione della tabella: la tabella usa `%HF_HOME%` solo per `hub`; per `datasets`, `xet` e `assets` si aggiunge anche `%HF_HOME%\<cartella>` (regola 3, e la documentazione sopra dice che `HF_HOME` sposta anche queste cartelle). I sottopercorsi sono gli stessi della tabella. `token` e `stored_tokens` (in `$HF_HOME`, https://huggingface.co/docs/huggingface_hub/package_reference/environment_variables, `HF_TOKEN_PATH`) non sono in nessuna azione. `HF_DATASETS_CACHE`, `HF_XET_CACHE`, `HF_ASSETS_CACHE` non sono nella tabella né nella lista consentita.

## PyTorch (`cleaners/pytorch.xml`)

| Percorso | Fonte |
|---|---|
| `%TORCH_HOME%\hub`, `%UserProfile%\.cache\torch\hub` | https://github.com/pytorch/pytorch/blob/main/torch/hub.py (`_get_torch_home`: `TORCH_HOME` oppure `~/.cache/torch`; `get_dir` = `<torch_home>/hub`); documentazione https://docs.pytorch.org/docs/stable/hub.html |

## Keras (`cleaners/keras.xml`)

| Percorso | Fonte |
|---|---|
| `%UserProfile%\.keras\models` | https://keras.io/api/applications/ («They are stored at `~/.keras/models/`») |
| `%UserProfile%\.keras\datasets` | https://github.com/keras-team/keras/blob/master/keras/src/utils/file_utils.py (`get_file`: `cache_dir` `~/.keras`, `cache_subdir` `datasets`) |

`keras.json` (https://keras.io/api/applications/, `~/.keras/keras.json`) non è in nessuna azione. `KERAS_HOME` (https://github.com/keras-team/keras/blob/master/keras/src/backend/config.py) sposta tutta la cartella: **variabile non nella lista consentita: KERAS_HOME** (la tabella non la usa; chi l'ha definita non vede la sua cartella).

## Ollama (`cleaners/ollama.xml`)

| Percorso | Fonte |
|---|---|
| `%UserProfile%\.ollama\models`, `%OLLAMA_MODELS%` | https://docs.ollama.com/faq («Windows: `C:\Users\%username%\.ollama\models`»; `OLLAMA_MODELS`) e https://docs.ollama.com/windows |

`<running>` `ollama.exe`: indicato dalla tabella; la documentazione di Windows parla di «tray application» e di esecuzione in background senza citare il nome del processo: non verificato.

## CUDA (`cleaners/bbag_shader.xml`, opzione `cuda`)

| Percorso | Fonte |
|---|---|
| `%CUDA_CACHE_PATH%`, `%AppData%\NVIDIA\ComputeCache` | https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/environment-variables.html (§ `CUDA_CACHE_PATH`: «on Windows, `%APPDATA%\NVIDIA\ComputeCache`») |

## Voci tolte

Nessuna voce della tabella è stata tolta per intero. Tolte o ridotte:

- Deno: `location_data`, `deno_history.txt`, `latest.txt` (dati dell'utente, non cache) e il nome `deps` (non nel sorgente attuale).
- Variabili non scritte perché fuori dalla lista consentita: `electron_config_cache`, `ELECTRON_BUILDER_CACHE`, `FIREBASE_EMULATORS_PATH`, `KERAS_HOME`, `PNPM_HOME`, `YARN_CACHE_FOLDER`, `HF_DATASETS_CACHE`, `HF_XET_CACHE`, `HF_ASSETS_CACHE`.

## Non supportato dal motore

Nessuna voce.
