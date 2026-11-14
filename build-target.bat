@echo off
rem Build di produzione di Xiox: xiox-ui.exe (finestra Tauri, senza installer) e xiox.exe
rem (riga di comando), raccolti con mukade.exe in target\dist\Xiox\ e in uno zip accanto.
rem
rem   build-target.bat             npm ci, build completa, cartella e zip
rem   build-target.bat --no-ci     salta npm ci (usa node_modules com'e')
rem   build-target.bat --jobs 8    compilazioni Rust in parallelo (predefinito: CARGO_BUILD_JOBS, altrimenti 4)
rem
rem Il limite ai job c'e' perche' la build release di Tauri, con tutti i core, esaurisce la memoria
rem (os error 1450). Su una macchina piccola: --jobs 1.
rem
rem Gli errori escono sempre da :fail o :usage: con setlocal attivo, "exit /b 1" dentro un blocco
rem tra parentesi restituisce 0 a chi ha lanciato lo script.

setlocal EnableExtensions
set "ROOT=%~dp0"
cd /d "%ROOT%" || goto :fail

set "NPM_CI=1"
if not defined CARGO_BUILD_JOBS set "CARGO_BUILD_JOBS=4"

:args
if "%~1"=="" goto :args_done
if /i "%~1"=="--no-ci" (set "NPM_CI=0" & shift & goto :args)
if /i not "%~1"=="--jobs" goto :usage
if "%~2"=="" goto :usage
set "CARGO_BUILD_JOBS=%~2"
shift & shift & goto :args
:args_done

rem --- Strumenti e file richiesti --------------------------------------------------------------
for %%T in (cargo node npm git) do (
  where %%T >nul 2>nul || (echo [ERRORE] %%T non trovato nel PATH. & goto :fail)
)
if not exist "vendor\mukade\mukade.exe" (
  echo [ERRORE] vendor\mukade\mukade.exe mancante: senza, la sezione delle sessioni non funziona.
  goto :fail
)

for /f "usebackq delims=" %%V in (`node -p "require('./package.json').version"`) do set "VERSION=%%V"
if not defined VERSION (echo [ERRORE] versione non letta da package.json. & goto :fail)
for /f "usebackq delims=" %%C in (`git rev-parse --short HEAD`) do set "COMMIT=%%C"
set "DIRTY="
git diff --quiet HEAD -- 2>nul || set "DIRTY= (modifiche non registrate)"

echo.
echo === Xiox %VERSION% - build di produzione, commit %COMMIT%%DIRTY%, %CARGO_BUILD_JOBS% job
echo.

rem --- 1. Dipendenze della pagina ---------------------------------------------------------------
if "%NPM_CI%"=="0" goto :skip_ci
echo [1/4] npm ci
call npm ci || goto :fail
goto :ci_done
:skip_ci
echo [1/4] npm ci saltato
if not exist "node_modules\.bin\tauri.cmd" (
  echo [ERRORE] node_modules incompleto: lancia senza --no-ci.
  goto :fail
)
:ci_done

rem --- 2. Finestra desktop: la pagina (beforeBuildCommand: tsc + vite) e xiox-ui.exe -------------
echo.
echo [2/4] xiox-ui.exe (Tauri, release, custom-protocol)
pushd "apps\desktop" || goto :fail
call "%ROOT%node_modules\.bin\tauri.cmd" build --no-bundle --features custom-protocol -- --locked
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" goto :fail

rem --- 3. Riga di comando ------------------------------------------------------------------------
echo.
echo [3/4] xiox.exe (xiox-cli, release)
cargo build --release --locked -p xiox-cli || goto :fail

rem --- 4. Cartella di distribuzione e zip --------------------------------------------------------
echo.
echo [4/4] target\dist
set "REL=%ROOT%target\release"
set "DIST=%ROOT%target\dist\Xiox"
set "ZIP=%ROOT%target\dist\xiox-%VERSION%-windows-x64.zip"

for %%F in (xiox-ui.exe xiox.exe) do (
  if not exist "%REL%\%%F" (echo [ERRORE] %REL%\%%F non prodotto. & goto :fail)
)

if exist "%DIST%" rmdir /s /q "%DIST%" || goto :fail
mkdir "%DIST%" || goto :fail
copy /y "%REL%\xiox-ui.exe" "%DIST%\" >nul || goto :fail
copy /y "%REL%\xiox.exe" "%DIST%\" >nul || goto :fail
rem mukade.exe accanto agli eseguibili: e' il primo posto dove xiox-mukade lo cerca.
copy /y "vendor\mukade\mukade.exe" "%DIST%\" >nul || goto :fail
copy /y "vendor\mukade\LICENSE" "%DIST%\LICENSE-mukade.txt" >nul || goto :fail

if exist "%ZIP%" del /q "%ZIP%"
powershell -NoProfile -NonInteractive -Command "Compress-Archive -Path '%DIST%' -DestinationPath '%ZIP%' -Force" || goto :fail

echo.
echo === Fatto: Xiox %VERSION% (%COMMIT%%DIRTY%)
for %%F in ("%DIST%\*") do echo     %%~nxF  %%~zF byte
echo     cartella  %DIST%
echo     zip       %ZIP%
endlocal
exit /b 0

:usage
echo [ERRORE] argomenti non validi: %*
echo Uso: build-target.bat [--no-ci] [--jobs N]
endlocal
exit /b 1

:fail
echo.
echo [ERRORE] build interrotta.
endlocal
exit /b 1
