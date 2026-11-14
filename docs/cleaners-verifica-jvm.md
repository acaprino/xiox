> Internal working notes, written in Italian.

# Verifica dei percorsi: lotto «jvm» (Task 9, Parte B)

Regola 4 della spec `docs/superpowers/specs/2026-11-10-catalogo-sviluppo-design.md`: ogni percorso è controllato sulla documentazione ufficiale dello strumento (sito o GitHub dello strumento). Dove la documentazione non dà il percorso letterale, si cita il sorgente ufficiale che lo definisce. Data della verifica: 2026-11-10.

Tutte le opzioni usano `walk.all` sulla cartella indicata (si tolgono i figli, la cartella radice resta); fa eccezione `gradle/daemon_logs` (`glob`). Nessuna voce usa funzioni che il motore non conosce: **nessun «non supportato»**.

## Voci tolte

| Strumento | Percorso | Motivo |
|---|---|---|
| Conda | `%USERPROFILE%\mambaforge\pkgs` | Non documentato: il README di Miniforge dice solo che la cartella di installazione predefinita di Mambaforge è diversa da quella di Miniforge, senza darla, ed è deprecato (ritirato da gennaio 2025). Tolta. Restano `miniconda3`, `anaconda3`, `miniforge3` e `%CONDA_PKGS_DIRS%`. |

## Voci ristrette rispetto alla tabella

| Strumento | Tabella | Scritto | Motivo |
|---|---|---|---|
| Coursier | `%LOCALAPPDATA%\Coursier\Cache` | `%LOCALAPPDATA%\Coursier\Cache\v1` | La documentazione ufficiale dà il percorso predefinito come `%LOCALAPPDATA%\Coursier\Cache\v1`; la cartella padre non è documentata. È un sottopercorso di quello della tabella, quindi più stretto. |

## Fonti per strumento

### uv (`cleaners/uv.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `cache` | `%UV_CACHE_DIR%`, `%LOCALAPPDATA%\uv\cache` | https://docs.astral.sh/uv/concepts/cache/ («`%LOCALAPPDATA%\uv\cache` on Windows»; `UV_CACHE_DIR` ha la precedenza) |

Python e tool installati da uv (regola 2): non scritti.

### Poetry (`cleaners/poetry.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `cache` | `%LOCALAPPDATA%\pypoetry\Cache\cache` e `\artifacts` | https://python-poetry.org/docs/configuration/ (cache-dir su Windows: `C:\Users\<username>\AppData\Local\pypoetry\Cache`); le due sottocartelle sono definite in https://github.com/python-poetry/poetry/blob/main/src/poetry/config/config.py (`repository_cache_directory` = cache-dir/`cache`/`repositories`, `artifacts_cache_directory` = cache-dir/`artifacts`) |
| `virtualenvs` | `%LOCALAPPDATA%\pypoetry\Cache\virtualenvs` | https://python-poetry.org/docs/configuration/ (`virtualenvs.path`, predefinito `{cache-dir}/virtualenvs`) |

`POETRY_CACHE_DIR` non è nella tabella né in `TOOL_VARS`: non usata. Il file `%APPDATA%\pypoetry\config.toml` non si tocca.

### Pipenv (`cleaners/pipenv.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `virtualenvs` | `%WORKON_HOME%`, `%USERPROFILE%\.virtualenvs` | https://pipenv.pypa.io/en/latest/virtualenv.html (Windows: `%USERPROFILE%\.virtualenvs\`; `WORKON_HOME` cambia la cartella) |

### Conda (`cleaners/conda.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `pkgs` | `%CONDA_PKGS_DIRS%` | https://docs.conda.io/projects/conda/en/stable/user-guide/configuration/settings.html («The `CONDA_PKGS_DIRS` environment variable overwrites the `pkgs_dirs` setting») |
| `pkgs` | `<radice>\pkgs`: la cache dell'installazione base è `root_dir/pkgs` | stessa pagina (`pkgs_dirs`: «for the standard envs directory in the root directory ... the normal `root_dir/pkgs` is used») |
| `pkgs` | radici `%USERPROFILE%\miniconda3`, `%USERPROFILE%\anaconda3` | https://www.anaconda.com/docs/getting-started/working-with-conda/reference/faq (installazione predefinita su Windows: `C:\Users\<YOUR_USERNAME>\Anaconda3\` e `\Miniconda3\`; Windows non distingue le maiuscole) |
| `pkgs` | radice `%USERPROFILE%\miniforge3` | https://github.com/conda-forge/miniforge (README: `C:\Users\myusername\miniforge3`) |

`envs` (regola 2): non scritta. `%USERPROFILE%\mambaforge\pkgs`: tolta (vedi sopra).

### virtualenv (`cleaners/virtualenv.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `app_data` | `%LOCALAPPDATA%\pypa\virtualenv` | La documentazione (https://virtualenv.pypa.io/en/latest/explanation.html) dice solo «user application data directory» e `VIRTUALENV_OVERRIDE_APP_DATA`, senza percorso Windows. Il percorso viene dal sorgente ufficiale https://github.com/pypa/virtualenv/blob/main/src/virtualenv/app_data/cache_migration.py: `user_cache_dir` e `user_data_dir` di platformdirs con `appname="virtualenv"` e `appauthor="pypa"`; su Windows platformdirs dà `%LOCALAPPDATA%\pypa\virtualenv` (dati, versioni precedenti) e `%LOCALAPPDATA%\pypa\virtualenv\Cache` (cache, versioni recenti), https://github.com/tox-dev/platformdirs/blob/main/src/platformdirs/windows.py. Si prende la cartella padre, che contiene entrambe le disposizioni. |

`VIRTUALENV_OVERRIDE_APP_DATA` non è nella tabella né in `TOOL_VARS`: non usata.

### pre-commit (`cleaners/precommit.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `cache` | `%PRE_COMMIT_HOME%`, `%USERPROFILE%\.cache\pre-commit` | https://pre-commit.com/#managing-ci-caches («by default places its repository store in `~/.cache/pre-commit`», `PRE_COMMIT_HOME`; l'esempio AppVeyor dello stesso sito usa `%USERPROFILE%\.cache\pre-commit`) |

### pyenv-win (`cleaners/pyenv.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `install_cache` | `%USERPROFILE%\.pyenv\pyenv-win\install_cache` | https://github.com/pyenv-win/pyenv-win/blob/master/pyenv-win/libexec/libs/pyenv-lib.vbs (`strDirCache = strPyenvHome & "\install_cache"`, con `strPyenvHome` = cartella `pyenv-win`); l'installazione in `%USERPROFILE%\.pyenv\pyenv-win` è in https://github.com/pyenv-win/pyenv-win/blob/master/docs/installation.md |

`versions` (regola 2): non scritta.

### pytest (`cleaners/pytest.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `tmp` | `%TEMP%\pytest-of-*` | https://docs.pytest.org/en/stable/how-to/tmp_path.html (`{temproot}/pytest-of-{user}/pytest-{num}/`, `temproot` = `tempfile.gettempdir()`; si tengono le ultime 3 esecuzioni) |

### Maven (`cleaners/maven.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `repository` | `%USERPROFILE%\.m2\repository` | https://maven.apache.org/settings.html (`localRepository`: «The default value is `${user.home}/.m2/repository`»). Avviso sugli artefatti di `mvn install`: https://maven.apache.org/guides/introduction/introduction-to-repositories.html (il repository locale «contains temporary build artifacts that you have not yet released») |
| `wrapper_dists` | `%USERPROFILE%\.m2\wrapper\dists` | https://github.com/apache/maven-wrapper/blob/master/maven-wrapper-distribution/src/resources/only-mvnw.cmd (`$MAVEN_WRAPPER_DISTS = "$MAVEN_M2_PATH/wrapper/dists"`, con `MAVEN_M2_PATH` = `$HOME/.m2`); https://maven.apache.org/tools/wrapper/ (base `$HOME/.m2`, `MAVEN_USER_HOME`) |

`MAVEN_USER_HOME` non è nella tabella né in `TOOL_VARS`: non usata. `settings.xml` non si tocca.

### Gradle (`cleaners/gradle.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| tutte | `%GRADLE_USER_HOME%`, `%USERPROFILE%\.gradle` | https://docs.gradle.org/current/userguide/directory_layout.html («`C:\Users\<USERNAME>\.gradle` on Windows», `GRADLE_USER_HOME`) |
| `caches` | `<home>\caches` | stessa pagina (`caches/`) |
| `wrapper_dists` | `<home>\wrapper\dists` | stessa pagina («Distributions downloaded by the Gradle Wrapper») |
| `jdks` | `<home>\jdks` | stessa pagina («JDKs downloaded by the toolchain support») |
| `native` | `<home>\native` | Non è nella guida. Sorgente ufficiale: https://github.com/gradle/gradle/blob/master/platforms/core-runtime/native/src/main/java/org/gradle/internal/nativeintegration/services/NativeServices.java (`getNativeServicesDir`: `new File(userHomeDir, "native")`) |
| `daemon_logs` | `<home>\daemon\*\*.out.log` | https://docs.gradle.org/current/userguide/gradle_daemon.html («stored in the Gradle User Home directory under `daemon/<gradle-version>/`», «named `daemon-<pid>.out.log`») |

🔒 `gradle.properties` e `init.d` (pagina `directory_layout.html`): non entrano in nessuna opzione.

**Daemon.** `<running>` è solo `gradle.exe`: il daemon gira come `java.exe`, e `java.exe` non basta a dire che c'è un daemon Gradle (regola del lotto). Il dubbio resta al builder («Come riconoscere un daemon Gradle attivo», spec, «Lasciato al builder»): la pulizia di `caches` può cadere su file in uso da un daemon; i file bloccati si saltano e si contano. L'avviso di `caches` dice di fermare i daemon con `gradle --stop`.

### Kotlin/Native (`cleaners/kotlin_native.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `data` | `%KONAN_DATA_DIR%`, `%USERPROFILE%\.konan` | https://kotlinlang.org/docs/native-improving-compilation-time.html («`$USER_HOME/.konan` directory»; su Windows «`.konan` ... located in `%USERPROFILE%` by default»); `KONAN_DATA_DIR`: https://github.com/JetBrains/kotlin/blob/master/kotlin-native/HACKING.md («changes `.konan` local data directory location (`$HOME/.konan` by default)») |

### sbt, Ivy, Coursier (`cleaners/sbt.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `boot` | `%USERPROFILE%\.sbt\boot` | https://www.scala-sbt.org/1.x/docs/Launcher-Configuration.html (`directory: ${sbt.boot.directory-${sbt.global.base-${user.home}/.sbt}/boot/}`) |
| `ivy_cache` | `%USERPROFILE%\.ivy2\cache` | https://www.scala-sbt.org/1.x/docs/Launcher-Configuration.html (`ivy-home` predefinito `~/.ivy2`, «where the Ivy cache is stored»); la cartella `cache` è nominata in https://www.scala-sbt.org/1.x/docs/Cached-Resolution.html (`~/.ivy2/cache/com.ning/...`) |
| `coursier_cache` | `%COURSIER_CACHE%`, `%LOCALAPPDATA%\Coursier\Cache\v1` | https://get-coursier.io/docs/cache (Windows: `%LOCALAPPDATA%\Coursier\Cache\v1`; «`COURSIER_CACHE` in the environment is looked at if it is set») |

`.ivy2\local` (Attenzione, pacchetti pubblicati in locale): **non inclusa**, come da richiesta. Documentata come repository locale in https://www.scala-sbt.org/1.x/docs/Launcher-Configuration.html («`local` - the local Ivy repository `~/.ivy2/local`»).

### Android (`cleaners/android.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `user_cache` | `%ANDROID_USER_HOME%\cache`, `%USERPROFILE%\.android\cache` | `ANDROID_USER_HOME` (predefinito `$HOME/.android/`): https://developer.android.com/tools/variables . La sottocartella `cache` non è nelle guide: sorgente ufficiale AOSP `platform/tools/base`, `sdklib/.../legacy/remote/internal/DownloadCache.java` (`new File(androidFolder, SdkConstants.FD_CACHE)`) e `common/.../SdkConstants.java` (`FD_CACHE = "cache"`), https://android.googlesource.com/platform/tools/base/+/refs/heads/mirror-goog-studio-main/ |
| `sdk_temp` | `<SDK>\.temp`, `<SDK>\.downloadIntermediates`, con `<SDK>` = `%ANDROID_HOME%`, `%ANDROID_SDK_ROOT%`, `%LOCALAPPDATA%\Android\Sdk` | `ANDROID_HOME`, `ANDROID_SDK_ROOT` (deprecata ma letta): https://developer.android.com/tools/variables ; SDK predefinito su Windows (`%USERPROFILE%\AppData\Local\Android\SDK`, Windows non distingue le maiuscole): https://developer.android.com/studio/intro/studio-config . `.temp` e `.downloadIntermediates` non sono nelle guide: sorgente ufficiale AOSP `repository/.../impl/installer/AbstractPackageOperation.java` (`REPO_TEMP_DIR_FN = "." + "temp"`, `DOWNLOAD_INTERMEDIATES_DIR_FN = "." + "downloadIntermediates"`, create sotto `repoManager.getLocalPath()`, cioè la radice dell'SDK), stessa base URL |
| `studio_caches` | `%LOCALAPPDATA%\Google\AndroidStudio*\caches` | La cartella di sistema `%LOCALAPPDATA%\Google\AndroidStudioVERSIONE` è in https://developer.android.com/studio/troubleshoot («System (idea.system.path)»). La sottocartella `caches` non è nelle guide di Android: viene dalla piattaforma IntelliJ su cui Android Studio è costruito, sorgente ufficiale https://github.com/JetBrains/intellij-community/blob/master/platform/vfs-impl/src/com/intellij/openapi/vfs/newvfs/persistent/FSRecords.java (`Path.of(PathManager.getSystemPath(), "caches")`); la guida JetBrains https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html dice che la cartella di sistema «contains caches and local history files» |
| `studio_logs` | `%LOCALAPPDATA%\Google\AndroidStudio*\log` | https://developer.android.com/studio/troubleshoot («Logs (idea.log.path): `%LOCALAPPDATA%\Google\AndroidStudioVERSION\log`») |

🔒 `debug.keystore` e `adbkey`: stanno in `.android` accanto a `cache`, non dentro; non entrano in nessuna opzione. `LocalHistory` (sorella di `caches` nella cartella di sistema): non inclusa. Emulatori (`avd`), `platforms`, `platform-tools`, `system-images`: parte C / regola 2, non scritti.

### Pub (`cleaners/pub.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `hosted`, `git` | `%PUB_CACHE%`, `%LOCALAPPDATA%\Pub\Cache` | https://dart.dev/tools/pub/environment-variables (`%LOCALAPPDATA%\Pub\Cache` su Windows, `PUB_CACHE`) |
| `hosted`, `git` | `<cache>\hosted`, `<cache>\git` | Le due cartelle non sono nelle guide. Sorgente ufficiale https://github.com/dart-lang/pub/blob/master/lib/src/system_cache.dart (`rootDirForSource(source)` = `p.join(rootDir, source.name)`) con `name = 'hosted'` in `lib/src/source/hosted.dart` e `name = 'git'` in `lib/src/source/git.dart` |

`bin` e `global_packages` (regola 2): non scritte. `hosted-hashes` (cartella sorella di `hosted` nel sorgente): non nella tabella, non scritta.

### NuGet (`cleaners/nuget.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `packages` | `%NUGET_PACKAGES%`, `%USERPROFILE%\.nuget\packages` | https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders (global-packages, `NUGET_PACKAGES`) |
| `http_cache` | `%NUGET_HTTP_CACHE_PATH%`, `%LOCALAPPDATA%\NuGet\v3-cache` | stessa pagina (http-cache, `NUGET_HTTP_CACHE_PATH`) |
| `plugins_cache` | `%LOCALAPPDATA%\NuGet\plugins-cache` | stessa pagina (plugins-cache, NuGet 4.8+) |
| `scratch` | `%TEMP%\NuGetScratch` | stessa pagina (temp) |

`NUGET_PLUGINS_CACHE_PATH` e `NUGET_SCRATCH` (stessa pagina) non sono nella tabella né in `TOOL_VARS`: non usate. `NuGet.Config` non si tocca.

### Visual Studio (`cleaners/visualstudio.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `component_cache` | `%LOCALAPPDATA%\Microsoft\VisualStudio\*\ComponentModelCache` | https://learn.microsoft.com/en-us/visualstudio/extensibility/managed-extensibility-framework-in-the-editor (`%localappdata%\Microsoft\VisualStudio[yourVSVersion]\ComponentModelCache`); https://learn.microsoft.com/en-us/visualstudio/gamedev/unity/troubleshooting/troubleshooting-and-known-issues-visual-studio-tools-for-unity (togliere la cartella per azzerare la cache MEF, con Visual Studio chiuso) |

### .NET a file singolo (`cleaners/dotnet_bundle.xml`)

| Voce | Percorso | Fonte |
|---|---|---|
| `extract` | `%DOTNET_BUNDLE_EXTRACT_BASE_DIR%`, `%TEMP%\.net` | https://learn.microsoft.com/en-us/dotnet/core/deploying/single-file/overview (se la variabile è impostata i file si estraggono sotto quel percorso, altrimenti su Windows sotto `%TEMP%/.net`) |

## Variabili d'ambiente

Tutte quelle usate dalle XML sono in `TOOL_VARS` o sono cartelle note: `UV_CACHE_DIR`, `WORKON_HOME`, `CONDA_PKGS_DIRS`, `PRE_COMMIT_HOME`, `GRADLE_USER_HOME`, `KONAN_DATA_DIR`, `COURSIER_CACHE`, `ANDROID_USER_HOME`, `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `PUB_CACHE`, `NUGET_PACKAGES`, `NUGET_HTTP_CACHE_PATH`, `DOTNET_BUNDLE_EXTRACT_BASE_DIR`, più `%USERPROFILE%`, `%LocalAppData%`, `%TEMP%`. **Nessuna variabile mancante.** Variabili che gli strumenti riconoscono ma la tabella non usa, quindi non scritte: `POETRY_CACHE_DIR`, `VIRTUALENV_OVERRIDE_APP_DATA`, `MAVEN_USER_HOME`, `NUGET_PLUGINS_CACHE_PATH`, `NUGET_SCRATCH`, `ANDROID_SDK_HOME`.

## Note per il controller

- **Variabile e percorso predefinito insieme.** Con `<var>` il motore tiene ogni valore la cui variabile è definita: se `GRADLE_USER_HOME` è definita, entrano sia `%GRADLE_USER_HOME%\caches` sia `%USERPROFILE%\.gradle\caches`. Non si può dire «il predefinito solo se la variabile manca». Di solito il predefinito non esiste e non fa nulla.
- **Rilevamento con i jolly.** `collect::present` prende come base la parte del percorso prima del primo componente con jolly. `%TEMP%\pytest-of-*` ha base `%TEMP%`, che esiste sempre: la categoria pytest risulta sempre presente. `%LocalAppData%\Google\AndroidStudio*\caches` ha base `%LocalAppData%\Google`, che esiste anche con solo Chrome: la categoria Android risulta presente anche senza Android Studio (le altre azioni di Android, `.android\cache` e l'SDK, hanno basi precise). Il caso di `visualstudio` è corretto (base `...\Microsoft\VisualStudio`). Una correzione possibile nel motore: per le azioni con jolly, chiedere almeno una corrispondenza reale.
- **Gradle e daemon**: vedi sopra, `<running>` solo `gradle.exe`.
