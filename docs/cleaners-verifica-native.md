# Verifica dei percorsi: lotto «native» (Task 9, Parte B)

Ogni percorso dei cleaner di questo lotto è stato controllato sulla documentazione o sul codice ufficiale dello strumento (sito, repository GitHub dello strumento). Data della verifica: 2026-11-10. Stato: **tenuta** (documentata), **cambiata** (la fonte dice altro dalla tabella della spec), **tolta** (non documentata o non raggiungibile dal motore).

Variabili d'ambiente: tutte quelle usate dai cleaner sono nella lista di `Env::for_rules` (`TOOL_VARS` più `userprofile`, `appdata`, `localappdata`, `temp`, `username`). Nessuna variabile mancante. Variabili che la documentazione cita ma che **non** sono nella lista e che quindi non si usano: `HEX_HOME`, `JULIA_DEPOT_PATH`, `PLATFORMIO_CORE_DIR` (e `PLATFORMIO_CACHE_DIR`, `PLATFORMIO_PACKAGES_DIR`, `PLATFORMIO_PLATFORMS_DIR`), `SCOOP_CACHE`, `CABAL_DIR`, `PULUMI_HOME`, `REBAR_CACHE_DIR`, `VCPKG_DOWNLOADS`, `BUNDLE_USER_CACHE`, `HOME` (Bazel). Con una di queste impostate, la cartella spostata non si trova (nessun danno: la voce non trova nulla).

## Cargo (`cleaners/bbag_cargo.xml`, modificato)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cargo home | `%CARGO_HOME%`, poi `%USERPROFILE%\.cargo` | tenuta | https://doc.rust-lang.org/cargo/reference/environment-variables.html (`%USERPROFILE%\.cargo` su Windows) |
| `registry\cache` (esistente) | `…\registry\cache`, `walk.files` | tenuta, ora con `%CARGO_HOME%` prima | https://doc.rust-lang.org/cargo/guide/cargo-home.html |
| `registry\src` | `…\registry\src` | tenuta | stessa pagina: «unpacked into `registry/src`», ricreata da archivio |
| `registry\index` | `…\registry\index` | tenuta | stessa pagina |
| `git\checkouts` | `…\git\checkouts` | tenuta | stessa pagina |
| `git\db` | `…\git\db` | tenuta | stessa pagina |
| `bin`, `config.toml`, `credentials.toml`, `.crates.toml`, `.crates2.json` | — | mai toccati (🔒 e regola 2) | stessa pagina |

La pagina dice anche: «In theory, you can always remove any part of the cache and Cargo will do its best to restore sources».

**Limite del motore (annotato, non bloccante).** Il guard protegge ogni componente `.git`. Nelle voci `registry\index` (vecchio indice git, non lo sparse di crates.io) e `git\checkouts` (ogni copia di lavoro ha un `.git`) quei file si scartano e si contano come «percorso protetto»: lo spazio liberato è parziale e restano le cartelle con il loro `.git`. Non supportato: svuotare i `.git`. L'indice sparse (predefinito di crates.io) e `git\db` (repository bare) si puliscono per intero.

**Effetto su `embedded.rs`.** Il test `xml_structure_unchanged` si aspetta `("bbag_cargo", 1, 1, &[])` e `("bbag_go", 1, 1, &["build"])`. Ora sono 5 opzioni e 5 azioni (`<action>`) per Cargo, 2 opzioni e 2 azioni per Go (id `build` invariato). Lo aggiorna il controller.

## rustup (`cleaners/rustup.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Rustup home | `%RUSTUP_HOME%`, poi `%USERPROFILE%\.rustup` | tenuta | https://rust-lang.github.io/rustup/environment-variables.html («default: `~/.rustup` or `%USERPROFILE%/.rustup`») |
| `downloads` | `…\downloads` | tenuta | https://github.com/rust-lang/rustup/blob/master/src/config.rs (`rustup_dir.join("downloads")`) |
| `tmp` | `…\tmp` | tenuta | issue ufficiali https://github.com/rust-lang/rustup/issues/2430 e https://github.com/rust-lang/rustup/issues/2246 (la cartella `~/.rustup/tmp` è quella dei temporanei di rustup; dalla 1.24.3 non si cancellano le cartelle in sé, il motore non le cancella: `walk.all` lascia la radice) |
| `toolchains`, `update-hashes`, `settings.toml` | — | non scritte (regola 2) | — |

## Go (`cleaners/bbag_go.xml`, modificato)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache di compilazione | `%GOCACHE%`, poi `%LocalAppData%\go-build` | tenuta (l'esistente, con la variabile prima) | https://pkg.go.dev/cmd/go («Build and test caching»: sottocartella `go-build` della cartella cache utente) e https://pkg.go.dev/os#UserCacheDir («On Windows, it returns %LocalAppData%») |
| Moduli | `%GOMODCACHE%`, poi `%GOPATH%\pkg\mod`, poi `%USERPROFILE%\go\pkg\mod` | tenuta; **aggiunto `%GOPATH%\pkg\mod`** | https://go.dev/ref/mod#module-cache («default `$GOPATH/pkg/mod`»; «`GOMODCACHE`… defaults to `$GOPATH/pkg/mod`»); GOPATH predefinito `%USERPROFILE%\go` su Windows: https://pkg.go.dev/cmd/go («GOPATH environment variable») |
| File in sola lettura | avviso nell'opzione `modules` | tenuta | https://go.dev/ref/mod#module-cache («creates module source files and directories in the cache with read-only permissions») |
| `go\bin` | — | non scritta (regola 2) | — |

`%GOPATH%` è nella lista di `Env::for_rules`; un valore con `;` (più cartelle) lo scarta già `Env::for_rules`. L'aggiunta è documentata ma non è nella tabella della spec: si può togliere senza altro effetto.

## ccache e sccache (`cleaners/ccache.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| ccache | `%CCACHE_DIR%`, poi `%LOCALAPPDATA%\ccache`; si toccano solo le sottocartelle di una cifra esadecimale (`<dir>\?`) | tenuta, **ristretta** | https://ccache.dev/manual/latest.html (`cache_dir`: «On Windows, the default is `%USERPROFILE%\.ccache` if such a directory exists, otherwise `%LOCALAPPDATA%\ccache`»); struttura `cache_dir/<cifra hex>/…` dal sorgente https://github.com/ccache/ccache (`src/ccache/storage/local/localstorage.cpp`, `FMT("{:x}/{:x}", l1_index, l2_index)`) |
| sccache | `%SCCACHE_DIR%`, poi `%LOCALAPPDATA%\Mozilla\sccache\cache` | tenuta | https://github.com/mozilla/sccache/blob/main/docs/Local.md (`SCCACHE_DIR`; disco in `%LOCALAPPDATA%\Mozilla\sccache`) e `src/config.rs` (`ProjectDirs::from("", "Mozilla", "sccache").cache_dir()`, che su Windows è `…\Mozilla\sccache\cache`) |
| `<running>` | `sccache.exe` | come la tabella | — |

**Perché ristretta.** Il manuale dice che `ccache.conf` sta nella cartella della cache («Clear the entire cache, removing all cached files, but keeping the configuration file») e il CleanerML non sa escludere un file. Con `glob` su `<dir>\?` si prendono le sole cartelle `0`..`f` e restano `ccache.conf`, `tmp`, `lock`. Il manuale non descrive la struttura delle cartelle: si appoggia al sorgente.

Percorsi del manuale non usati (non nella tabella): `%USERPROFILE%\.ccache` (cartella storica, solo se esiste) e `%APPDATA%\ccache` (versioni vecchie).

## vcpkg (`cleaners/vcpkg.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache binaria | `%VCPKG_DEFAULT_BINARY_CACHE%`, poi `%LOCALAPPDATA%\vcpkg\archives` | tenuta | https://learn.microsoft.com/en-us/vcpkg/users/binarycaching («Default binary cache», Windows) |
| `buildtrees` | `%VCPKG_ROOT%\buildtrees` | tenuta | https://learn.microsoft.com/en-us/vcpkg/commands/common-options (`--x-buildtrees-root`: «Defaults to `buildtrees/` under the vcpkg root folder») |
| `packages` | `%VCPKG_ROOT%\packages` | tenuta | stessa pagina (`--x-packages-root`) |
| `downloads` | `%VCPKG_ROOT%\downloads` | tenuta | stessa pagina (`--downloads-root`: «downloaded tools and source code archives»; predefinito `downloads/` sotto la radice, se `VCPKG_DOWNLOADS` non c'è) |
| `installed` | — | mai toccata | — |

Il terzo percorso della cache binaria (`%APPDATA%\vcpkg\archives`) esiste in documentazione ma non in tabella: non scritto. Con `VCPKG_ROOT` non definita le tre voci `buildtrees`, `packages`, `downloads` non compaiono.

## Conan (`cleaners/conan.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache | `%CONAN_HOME%\p`, poi `%USERPROFILE%\.conan2\p` | tenuta | https://docs.conan.io/2/reference/environment.html (`CONAN_HOME`, predefinita `<utente>/.conan2`); cartella `p`: https://docs.conan.io/2/reference/commands/cache.html e sorgente https://github.com/conan-io/conan (`conan/internal/cache/cache.py`: `os.path.join(cache_folder, "p")`) |

Nel sorgente il database `cache.sqlite3` sta dentro `p` per impostazione predefinita (`db_folder = self._store_folder`): la voce lo toglie insieme ai pacchetti, così il database non resta con riferimenti a cartelle sparite. Avviso presente («i pacchetti creati con `conan create` esistono solo qui»).

## Composer (`cleaners/composer.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache | `%COMPOSER_CACHE_DIR%`, poi `%LOCALAPPDATA%\Composer`: **solo** `files`, `repo`, `vcs` | tenuta, **ristretta** | https://getcomposer.org/doc/03-cli.md#composer-cache-dir («C:\Users\<user>\AppData\Local\Composer (or %LOCALAPPDATA%/Composer) on Windows») e https://getcomposer.org/doc/06-config.md (`cache-files-dir`, `cache-repo-dir`, `cache-vcs-dir`: `$cache-dir/files`, `/repo`, `/vcs`) |
| 🔒 `config.json`, `auth.json` | — | mai toccati | — |
| `%APPDATA%\Composer\vendor` | — | non scritta (regola 2) | — |

**Perché ristretta.** Sulle altre piattaforme la cache predefinita è dentro `COMPOSER_HOME` (`$COMPOSER_HOME/cache`), e chi imposta `COMPOSER_CACHE_DIR` sulla stessa cartella di `COMPOSER_HOME` avrebbe `config.json` accanto alla cache. Le tre sottocartelle sono le sole che la documentazione dà come cache: con una `walk.all` sulla radice `config.json` sarebbe finito in lista (`auth.json` lo ferma comunque il guard). Il test lo prova.

## Bundler (`cleaners/bundler.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache utente | `%USERPROFILE%\.bundle\cache` | tenuta | https://bundler.io/man/bundle-config.1.html (`user_cache` = `$BUNDLE_USER_HOME/cache`; `user_home` predefinita `~/.bundle`), riportata anche da https://guides.rubygems.org/command-reference/bundle-config/ («defaults to ~/.bundle/cache») |
| `.bundle\config` | — | mai toccato | — |

La pagina del manuale non si è lasciata leggere con lo strumento di lettura (contenuto vuoto): il testo è confermato dalla guida di RubyGems e dal sorgente del manuale (https://git.ruby-lang.org/ruby.git/tree/lib/bundler/man/bundle-config.1.ronn).

## Hex e rebar3 (`cleaners/hex.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Hex | `%USERPROFILE%\.hex\packages` | tenuta | home predefinita `~/.hex` (docs Hex 2.5.1, https://hex.hexdocs.pm/Hex.epub: «If none of the variables are set, the default directory ~/.hex will be used»; sorgente https://github.com/hexpm/hex `lib/hex/config.ex`: `Path.expand("~/.hex")`); `packages` da `lib/hex/scm.ex`: `Path.join([Hex.State.fetch!(:cache_home), @packages_dir, repo, filename])` con `@packages_dir "packages"` |
| rebar3 | `%USERPROFILE%\.cache\rebar3\hex` | **cambiata** (tabella: tutta `.cache\rebar3`) | https://rebar3.org/docs/configuration/configuration/ e sorgente https://github.com/erlang/rebar3 (`rebar_dir:global_cache_dir`: `<home>/.cache/rebar3`); sottocartella `hex` (pacchetti e indice): https://hex.pm/docs/rebar3-usage |

**Perché rebar3 è ristretto a `hex`.** https://rebar3.org/docs/getting-started/ dice che `rebar3 local install` mette il rebar3 compilato in `~/.cache/rebar3/lib` e il comando di avvio in `~/.cache/rebar3/bin` (da mettere nel `PATH`): prendere tutta `.cache\rebar3` toglierebbe il rebar3 installato (regola 2). Restano fuori anche i plugin globali.

## Cabal (`cleaners/cabal.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| `packages` | `%APPDATA%\cabal\packages` | tenuta (vecchia disposizione, `CABAL_DIR` predefinita su Windows) | guida Cabal 3.6: https://cabal.readthedocs.io/en/3.6/installing-packages.html |
| `packages` | `%LOCALAPPDATA%\cabal\packages` | **aggiunta** (disposizione attuale) | https://cabal.readthedocs.io/en/latest/config.html («cache directory… defaults to `%LOCALAPPDATA%/cabal` on Windows»; pacchetti in `$XDG_CACHE_HOME/cabal/packages/hackage.haskell.org`; «You can delete this directory and expect that its contents will be reconstructed as needed») |
| store | — | mai toccato: sta nella stessa cartella (`%LOCALAPPDATA%\cabal\store`) e «deleting this directory might cause installed programs to stop working» | stessa pagina |

La tabella dava solo `%APPDATA%\cabal\packages`; la documentazione attuale mette la cache sotto `%LOCALAPPDATA%\cabal`. Si tengono entrambi, sempre e solo la sottocartella `packages`.

## Julia (`cleaners/julia.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| `compiled` | `%USERPROFILE%\.julia\compiled` | tenuta | deposito predefinito `~/.julia`: https://docs.julialang.org/en/v1/base/constants/ (`DEPOT_PATH`); sottocartella: https://pkgdocs.julialang.org/v1/glossary/ («`compiled`: cached compiled package images (`.ji` files)») |

Solo `compiled`: `packages`, `registries`, `environments`, `config`, `dev`, `logs` restano.

## Zig (`cleaners/zig.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache globale | `%ZIG_GLOBAL_CACHE_DIR%`, poi `%LOCALAPPDATA%\zig` | tenuta | sorgente https://github.com/ziglang/zig: `src/introspect.zig` (`ZIG_GLOBAL_CACHE_DIR` poi `fs.getAppDataDir(allocator, "zig")`) e `lib/std/fs/get_app_data_dir.zig` (Windows: `LOCALAPPDATA` + `zig`). La pagina https://ziglang.org/documentation/master/ è troppo lunga per lo strumento di lettura |

## PlatformIO (`cleaners/platformio.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cartella core | `%USERPROFILE%\.platformio` | tenuta | https://docs.platformio.org/en/latest/projectconf/sections/platformio/options/directory/core_dir.html (Windows: `%HOMEPATH%\.platformio`); sorgente https://github.com/platformio/platformio-core `platformio/project/options.py` / `helpers.py` (`~/.platformio`) |
| `.cache` | `…\.platformio\.cache` | tenuta | sorgente `options.py`: `default=os.path.join("${platformio.core_dir}", ".cache")` (la pagina `cache_dir` dice `core_dir/cache` senza punto: il codice ha il punto, come la tabella) |
| `packages` | `…\.platformio\packages` | tenuta, opzione separata | `options.py` e https://docs.platformio.org/en/latest/projectconf/sections/platformio/options/directory/packages_dir.html |
| `platforms` | `…\.platformio\platforms` | tenuta, opzione separata | `options.py` e https://docs.platformio.org/en/latest/projectconf/sections/platformio/options/directory/platforms_dir.html |

Nota: per `get_default_core_dir` su Windows, se esiste `<unità>\.platformio` si usa quella al posto di `%USERPROFILE%\.platformio`: non coperta.

## Minikube (`cleaners/minikube.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Cache | `%MINIKUBE_HOME%\.minikube\cache`, poi `%USERPROFILE%\.minikube\cache` | tenuta | https://minikube.sigs.k8s.io/docs/handbook/config/ (`MINIKUBE_HOME`: «the final MINIKUBE_HOME will be `/path/to/somewhere/.minikube`»; predefinita `~/.minikube`); cartella `cache`: https://minikube.sigs.k8s.io/docs/handbook/offline/ («copy the contents of `~/.minikube/cache`») |

Il sorgente (`pkg/minikube/localpath/localpath.go`) non aggiunge un secondo `.minikube` se il nome della cartella di `MINIKUBE_HOME` è già `.minikube`: in quel caso la cache sta in `%MINIKUBE_HOME%\cache`, che non si cerca (nessun danno).

## Vagrant (`cleaners/vagrant.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| `boxes` | `%VAGRANT_HOME%\boxes`, poi `%USERPROFILE%\.vagrant.d\boxes` | tenuta | https://developer.hashicorp.com/vagrant/docs/other/environmental-variables (`VAGRANT_HOME`, predefinita `~/.vagrant.d`, «where things such as boxes are stored»); nome `boxes` dal sorgente https://github.com/hashicorp/vagrant `lib/vagrant/environment.rb` (`@boxes_path = @home_path.join("boxes")`) |

## kubectl e Helm (`cleaners/kubectl.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Helm | `%HELM_CACHE_HOME%`, poi `%TEMP%\helm` | tenuta | https://helm.sh/docs/helm/helm/ (tabella dei percorsi predefiniti: cache Windows `%TEMP%\helm`) |
| kubectl | `%USERPROFILE%\.kube\cache` | **tolta** | non supportato: la cartella `.kube` è protetta per intero dal guard (`crates/pc-rules/src/guard.rs`, `CREDENTIAL_CONTENT_DIRS`, perché contiene `config` con le credenziali dei cluster); `collect` scarterebbe ogni file come «percorso protetto». Se il controller allenta il guard per `.kube\cache`, si aggiunge `<option id="kubectl_cache">` con `walk.all` su `%USERPROFILE%\.kube\cache` |
| 🔒 `.kube\config` | — | mai toccato | — |

Il cleaner si chiama `kubectl` (id del lotto) ma oggi offre solo Helm: etichetta e descrizione lo dicono.

## Terraform e Pulumi (`cleaners/terraform.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Terraform | `%TF_PLUGIN_CACHE_DIR%` | tenuta | https://developer.hashicorp.com/terraform/cli/config/config-file (`plugin_cache_dir`, `TF_PLUGIN_CACHE_DIR`) |
| Terraform | `%APPDATA%\terraform.d\plugin-cache` | **tolta**: non documentato | la pagina dà solo l'esempio `$HOME/.terraform.d/plugin-cache` e dice che la cartella deve esistere prima (Terraform non la crea). Non c'è un percorso predefinito della cache su Windows. `%APPDATA%\terraform.d` compare solo per le cartelle `plugins` dei mirror locali impliciti |
| Pulumi | `%USERPROFILE%\.pulumi\plugins` | tenuta | https://www.pulumi.com/docs/iac/concepts/plugins/ («All plugins that are installed by the user… are cached in `~/.pulumi/plugins`»); `PULUMI_HOME` (`~/.pulumi`): https://www.pulumi.com/docs/iac/cli/environment-variables/ |

Senza `TF_PLUGIN_CACHE_DIR` l'opzione Terraform non compare; Pulumi sì (percorso predefinito).

## Scoop, Chocolatey e WinGet (`cleaners/scoop.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Scoop | `%SCOOP%\cache`, poi `%USERPROFILE%\scoop\cache` | tenuta | sorgente https://github.com/ScoopInstaller/Scoop `lib/core.ps1` (`$scoopdir` = `$env:SCOOP` o `~\scoop`; `$cachedir` = `"$scoopdir\cache"`); esiste anche `SCOOP_CACHE` (non usata, non nella lista) |
| Chocolatey | `%TEMP%\chocolatey` | tenuta | sorgente https://github.com/chocolatey/choco `src/chocolatey/infrastructure.app/builders/ConfigurationBuilder.cs` (`CombinePaths(GetTempPath(), "chocolatey")`); la guida https://docs.chocolatey.org/en-us/guides/usage/change-cache/ dice «uses the Temp Folder for downloads» e `cacheLocation` la sposta |
| WinGet, installer | `%TEMP%\WinGet` | tenuta | https://github.com/microsoft/winget-cli/discussions/1148 (risposta del manutentore: gli installer scaricati stanno in `%TEMP%\WinGet`, «Delete `%TEMP%\WinGet`»). Non è su Microsoft Learn |
| WinGet, registri | `%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\DiagOutputDir` | tenuta | https://learn.microsoft.com/en-us/windows/package-manager/winget/troubleshooting |
| 🔒 `settings.json` di WinGet | `…\LocalState\settings.json` | mai toccato (sta accanto a `DiagOutputDir`) | stessa pagina (`winget settings`) |
| Scoop `apps`, `persist`, versioni vecchie | — | non scritti (regola 2) | — |

## Bazel (`cleaners/bazel.xml`)

| Voce | Percorso | Stato | Fonte |
|---|---|---|---|
| Radice di output | `%USERPROFILE%\_bazel_%USERNAME%` | tenuta | https://bazel.build/remote/output-directories (`outputUserRoot` = `outputRoot/_bazel_$USER`; su Windows `outputRoot` è `%HOME%` se definita, altrimenti `%USERPROFILE%`) |
| `<running>` | `bazel.exe` | come la tabella | il server Bazel gira come processo Java: `bazel.exe` è solo il client e compare solo durante un comando. Non rileva un server inattivo; l'avviso dice di fare `bazel shutdown` |

`%USERNAME%` è nella lista (`Env::for_rules` lo prende da `GetUserNameW`); se manca, la voce sparisce (non si scrive `_bazel_` alla lettera). Se `HOME` è definita, Bazel usa `%HOME%\_bazel_<utente>`: non coperto (HOME non è nella lista).

## JetBrains (`cleaners/bbag_jetbrains.xml`)

Nessuna modifica. `LocalHistory` è livello Attenzione (dati unici dell'utente): non si include.

## Riepilogo delle differenze dalla tabella

- **Tolte:** kubectl `.kube\cache` (guard); Terraform `%APPDATA%\terraform.d\plugin-cache` (non documentato).
- **Ristrette:** rebar3 a `hex` (protegge il rebar3 installato); Composer a `files`/`repo`/`vcs` (protegge `config.json`); ccache alle cartelle esadecimali (protegge `ccache.conf`).
- **Aggiunte, documentate:** Go `%GOPATH%\pkg\mod`; Cabal `%LOCALAPPDATA%\cabal\packages`.
- **Limiti del motore:** cartelle `.git` dentro `registry\index` (vecchio indice) e `git\checkouts` di Cargo non si puliscono (guard); il CleanerML non ha esclusioni.
