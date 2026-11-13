# Rebrand to Xiox: no old names left (plan)

Date: 2026-11-13. Status: approved by the user in chat ("tutto deve essere Xiox, perché restano ancora nomi vecchi?").

## Goal
Every name that belongs to this product is **Xiox**: crates, binaries, window title, data folders, temp names, registry value names, docs. The old names (`PulisciPC`, `pulisci-pc`, `pc-rules`, the `dc-*` crates that come from the old devclean project) disappear from the code. Existing users keep working: data written under the old names is still found, migrated or recognised (see "Persisted names").

## Names (decided)
| Old | New |
|---|---|
| package `pulisci-pc`, bin `PulisciPC` | package `xiox-desktop`, bin **`xiox-ui`** (the CLI is `xiox`; Windows file names are case-insensitive so the desktop exe cannot be `Xiox.exe`) |
| `PulisciPC.rc`, `PulisciPC.dev.rc`, `PulisciPC.manifest`, `PulisciPC.dev.manifest` | `xiox-ui.rc`, `xiox-ui.dev.rc`, `xiox-ui.manifest`, `xiox-ui.dev.manifest` (check build.rs and the `.rc` references) |
| window title, sidebar brand, About text, any user-visible "PulisciPC" | `Xiox` |
| `dc-app`, `dc-cleaner`, `dc-disktree`, `dc-engine`, `dc-git`, `dc-model`, `dc-mukade`, `dc-project`, `dc-scanner`, `dc-sessions` | `xiox-app`, `xiox-cleaner`, `xiox-disktree`, `xiox-engine`, `xiox-git`, `xiox-model`, `xiox-mukade`, `xiox-project`, `xiox-scanner`, `xiox-sessions` (directories under `crates/` renamed with `git mv`; Rust paths `dc_app::` → `xiox_app::` etc.) |
| `pc-rules` / `pc_rules` | `xiox-rules` / `xiox_rules` |
| data folders `%LOCALAPPDATA%\PulisciPC` (settings, cache, backups), `%PROGRAMDATA%\PulisciPC\winapp…`, `Documents\PulisciPC` (registry backups) | `…\Xiox` |
| temp names `PulisciPC_wipe_*`, `pulisci-curl-*`, `pulisci-worker-*`, env var `PULISCI_WINAPP*`, registry value `PulisciPC_StateFlags0099`, test keys `PulisciPC_Verify_*`, any `pulisci…` file-name prefix | `Xiox_wipe_*`, `xiox-curl-*`, `xiox-worker-*`, `XIOX_WINAPP*`, `Xiox_StateFlags0099`, `Xiox_Verify_*`, `xiox…` |

**Do NOT rename** third-party or historical references: `CCleaner Browser` (a browser the catalog knows), the Winapp2 URL (`Non-CCleaner`), `CCleaner`/`Winapp2` as sources in `docs/ccleaner-coverage.md`, `MECHANICS.md` (analysis of the original program, its source column), `THIRD_PARTY_NOTICES.md` entries for other products, `vendor/mukade` (the real Mukade product, vendored), the `mukade` crate and `dc-mukade`→`xiox-mukade` only renames OUR adapter crate, `docs/reference/devclean-*.md` (historical copies: keep file names; keep their content except the header note), the outer folder name `ccleaner64\app` (do not move the checkout), commit history, the GitHub remote names, and the Italian UI strings of the egui app (language is not part of this task). A word like `devclean` in a *comment* that describes history may stay if rewording changes meaning; prefer rewording to "the old devclean project" only when needed.

## Persisted names (safety: never lose a user's backups)
Existing installs have data under the old names. For every persisted or recognised name:
1. **Folders (settings, cache, backups, winapp cache, registry backups):** one small module (put it in `xiox-app`, e.g. `legacy.rs`) with `fn migrate_legacy_dir(old: &Path, new: &Path) -> PathBuf`: if `new` exists → use `new`; else if `old` exists → try `std::fs::rename(old, new)` (same parent, so same volume); if the rename succeeds use `new`; **if it fails, keep using `old`** (never copy-and-delete, never leave the user without their backups); else create/use `new`. Call it once at startup before any read/write of those folders, and make every path accessor go through it. For `%PROGRAMDATA%\PulisciPC\winapp…` (elevated-written data) the same rule applies; failure falls back to the old path.
2. **Recognised leftovers:** the wipe residue cleanup at start, the cleanmgr companion registry value, the env var and temp-file prefixes: new names are written; the **old names are still recognised and removed/read** (accept both `Xiox_wipe_*` and `PulisciPC_wipe_*`; remove both `Xiox_StateFlags0099` and `PulisciPC_StateFlags0099`; read `XIOX_WINAPP*` first then `PULISCI_WINAPP*`). Tests for each.
3. **Backup file names** (`backup_*` in the backups folder) are not renamed; the folder they live in is.
4. Settings JSON keys do not change.

## Method
- Order: (a) mechanical rename of crates/packages/bins/files (`git mv`, then identifier sweep); build + tests green; commit. (b) brand and data-name rename with the legacy handling and its tests; build + tests green; commit. (c) docs: README, `docs/xiox-cli.md` ("the PulisciPC desktop app" → "the Xiox desktop app", exe names), the specs/plans that name crates (`docs/superpowers/**`: update crate names in the two 2026-11-13 CLI/observer specs and plan only where they refer to current code; leave historical dated specs of 2026-11-10/12 untouched except a one-line note at the top of each saying names changed), `THIRD_PARTY_NOTICES.md` (our product name only), `.gitignore`/scripts if they name old crates. Commit.
- Final check: `git grep -i -n -E "pulisci|pc-rules|pc_rules|\bdc[-_](app|cleaner|disktree|engine|git|model|mukade|project|scanner|sessions)\b"` must show only the allowed leftovers (legacy-recognition code and its tests, historical docs, the `devclean` reference files); list every remaining hit with a one-line reason in the report.
- Build limits (this machine): `cargo` always `-j 1` with `CARGO_INCREMENTAL=0`, one build at a time (os error 1455 otherwise); an idle cargo.exe may exist, wait only if cargo/rustc is actually compiling. A rename forces full rebuilds: be patient, do not run two builds at once. Disk is nearly full: no extra target directories; `cargo clean -p <old crate>` is not needed — stale artifacts of old crate names in `target/` may be deleted with `rm -rf target/debug/deps/*dc_*` etc. only if disk runs out (say so).
- Tests: `cargo test -j 1 --workspace --exclude mukade` must pass (before: 991 passed, 0 failed, 20 ignored) plus the new legacy tests. A release build (`cargo build -j 1 --release --locked -p xiox-desktop -p xiox-cli`) must succeed at the end; report the exe names and sizes (`target\release\xiox-ui.exe`, `xiox.exe`).
- Never run the apps on the user's real data; tests use temp dirs only. The legacy migration must be tested with fake old/new folders (new exists; only old exists; rename fails → falls back to old).
- Commits: only changed files, messages in Italian (`refactor: …`), ending with:
  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Cd6KB14fHjjb98MiWyaNmx
