# Vendored: mukade (prebuilt executable)

- Upstream: https://github.com/acaprino/mukade (MIT, same author as Xiox).
- What is here: `mukade.exe`, the release build of upstream commit `f189a70` (2026-11-13), built
  from the local checkout `D:\Projects\mukade` (`target/release/mukade.exe`), and upstream's `LICENSE`.
  SHA-256 `efb02f7c71a88a01969e1991e06b02ff2277b68837dcf950c8641da7bab6b05a`.
- **No mukade source is in this repository and none is compiled into Xiox.** `crates/xiox-mukade`
  runs the executable as a separate program and reads its JSON output.
- What is run: only `mukade list <harness>` (`--children` for Codex, OpenCode, Pi, and for the
  archived listing of each), with the store passed in the harness's own variable (`CLAUDE_CONFIG_DIR`,
  `CODEX_HOME`, `OPENCODE_DB`, `PI_CODING_AGENT_DIR`). Nothing is written to the stores; `dump`,
  `hop` and `write` are never run.
- The only writes: mukade's data folder, set with `MUKADE_DATA_DIR` to the folder Xiox passes in
  (`app_cache_dir()/mukade`): the Claude Code listing cache and the intent prompts mukade installs on
  first run. Never `mukade-data` beside the executable.
- Which executable runs: `XIOX_MUKADE_EXE`, else `mukade.exe` beside the Xiox executable (the
  `xiox-mukade` build script copies this file into `target/<profile>/`), else this file.

To update: build upstream in release (`cargo build --release`), copy `target/release/mukade.exe`
here, and update the commit and the SHA-256 above.
