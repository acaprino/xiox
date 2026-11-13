> Copia di `D:\Projects\KKK\devcleanendor\mukade\VENDORED.md`, presa il 2026-11-12 prima di eliminare la cartella. La copia di mukade usata da Xiox (prima PulisciPC) è descritta in `vendor/mukade/VENDORED.md`. Dal 2026-11-13 i crate `dc-*` si chiamano `xiox-*` (`dc-mukade` -> `xiox-mukade`); il resto del testo è quello di allora.

# Vendored: mukade

- Upstream: https://github.com/acaprino/mukade (MIT, same author as devclean)
- Taken from commit `97d827f` (2026-11-08), local checkout `D:\Projects\mukade`. Previously `55635a2` (2026-11-04).
- Why vendored: devclean must be self-contained. The library is compiled **into the devclean executable**: no external binary, no network, no path outside this repository.
- What is used (`crates/dc-mukade`): only the listings, for all four harnesses. `reader::claude_code::list_cached` for Claude Code (transcripts, `history.jsonl`, `sessions/<pid>.json` liveness) and `Reader::list_all` for Codex (rollouts + `state_<n>.sqlite` threads), OpenCode (every `opencode*.db` beside the selected one) and Pi. Nothing is written to the stores; `Reader::read` (which for OpenCode would spawn the `opencode` CLI) is never called.
- The only write: the Claude Code listing cache (what each session file said, keyed by path, size and mtime), in the folder devclean passes in (`app_cache_dir()/mukade`), never mukade's own `mukade-data/cache` beside the executable.
- `process.rs` checks whether a registered Claude Code process is still running: on Windows two `kernel32` calls (`OpenProcess`, `GetProcessTimes`) declared by hand, no crate.

## Differences from upstream

- Library only: removed `src/bin/`, `mukade-tui/`, `tests/`, `fixtures/`, `docs/`, `build.rs`, `assets/windows/` (icon and manifest of the CLI binary).
- `Cargo.toml`: dropped `[[bin]]`, `[workspace]`, `[build-dependencies]` (`embed-resource`) and `build = "build.rs"`; added `publish = false`.
- Sources under `src/`, `catalog/` and `assets/intent-prompts/` are unchanged; `README.md` and `LICENSE` are upstream's.

To update: re-copy `src/` (minus `src/bin/`), `catalog/`, `assets/intent-prompts/`, `README.md` and `LICENSE` from upstream and keep this Cargo.toml. Keep `rusqlite` pinned to the same version as `crates/dc-mukade` (two versions cannot coexist because of `libsqlite3-sys`), then update the commit here and in `THIRD_PARTY_NOTICES.md`.
