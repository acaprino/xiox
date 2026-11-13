# `xiox observe` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `xiox observe [<project>]` is a long-running process that a harness, an agent or a program starts for ONE project: it takes a snapshot, re-checks at intervals, and tells its caller what appeared since it started (JSON lines on stdout, or a read-only MCP server on stdio with `--mcp`). It never removes anything.

**Architecture:** A new library crate `xiox-observer` holds the pure state machine (snapshot, diff, growth, thresholds with hysteresis, report, a bounded size measurer). The binary crate `xiox-cli` gets the `observe` command: argument parsing, the event loop (timer + stdin reader thread), JSON-lines output, and a hand-written MCP stdio server (one read-only tool). All analysis comes from `xiox-project` (`analyze_project*` with `Options { measure: false, .. }`).

**Tech Stack:** Rust (workspace crates `xiox-*`), `serde_json`, `chrono` (default-features off, `std`+`clock`, already in the workspace) for RFC 3339 times. No async runtime, no `rmcp`/`axum`: the MCP subset needed (initialize, ping, tools/list, tools/call) is written by hand.

**Spec:** `docs/superpowers/specs/2026-11-13-project-observer-mcp-design.md` (Italian; authoritative). Read it first. Deviations decided in this plan (record them in the spec «Esito» at the end):
- Single-threaded loop: a `report` request or an MCP call that arrives during a scheduled check is answered after that check ends (no 20-second wait rule). If the fresh analysis fails, the last result is returned with `stale: true` and an `error` event is emitted.
- The clean command in a report is built from `--include <path>` entries of the NEW recommended unblocked items (so it never takes older items), at most 25, with `cleanCommandTruncated: true` when there are more.

## Global Constraints

- **English only** in every string the user or a program sees: flags, event names, field names, tool name/description, instructions, errors. No Italian reaches stdout/stderr.
- stdout carries ONLY JSON lines (default mode) or ONLY MCP messages (`--mcp`); all human text goes to stderr.
- The observer is **read-only**: it never calls anything from `xiox_project::clean*`, `xiox_cleaner` removal, or git write commands, and exposes no write path. It writes nothing to disk except the session-listing cache the engine already writes.
- It never asks for administrator rights; if the process is not elevated it uses the bounded walk (`Options.admin` from `xiox_project::is_elevated()`).
- Light on resources: one analysis at a time per process; no full-disk walk at intervals (the existing bounded search only); size measuring is bounded (see Task 1) and only for new items and known `builds` folders.
- `cargo` always `-j 1` with `CARGO_INCREMENTAL=0`; one cargo process at a time (os error 1455 otherwise); an idle cargo.exe may exist, wait only if cargo/rustc is actually compiling; disk nearly full: no extra target dirs.
- Tests use fake trees/repos under `tempfile` only; binary tests run with the debug-only switches `XIOX_NO_SESSIONS=1`, `XIOX_SEARCH_ROOTS=<fake parent>`, `GIT_CEILING_DIRECTORIES` and remove `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`; never real projects or sessions. New debug-only switch (same `cfg!(debug_assertions)` gating as the others): `XIOX_OBSERVE_INTERVAL_MS` overrides `--interval`.
- Do not change the egui app (`src/`), `vendor/mukade`, or the behaviour/output of `xiox analyze`/`xiox clean` (their existing tests are the guard).
- Commits: only the files the task names (+ `Cargo.lock` when dependencies change), Italian message (`feat(xiox): …`), ending with exactly:
  `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01Cd6KB14fHjjb98MiWyaNmx`

## Review Focus

1. **"New" must mean new since the process started.** Everything in the first snapshot is `new: false` forever, even if it grows or moves category; an item that disappears and reappears is new again only if it was absent from the baseline. → Task 1 tests.
2. **stdout purity.** Any stray `println!`/panic message/progress line on stdout breaks the caller's parser; a panic must not print to stdout and the process must exit non-zero with the reason on stderr. → Tasks 2–3 tests that parse every stdout line / message.
3. **Lifetime.** The process must exit when stdin closes or on `stop` (default mode) and on EOF (MCP), never linger as an orphan, and not busy-loop while waiting. → Tasks 2–3 tests.
4. **Bounded work.** A huge or unreadable tree must not make a check run forever or eat memory: measuring is capped (entries and time), failures degrade to `bytes: null`, not to a hang or a crash. → Task 1 tests.
5. **Thresholds.** An attention event fires once per crossing, re-arms only after returning at or below the threshold, and growth is computed from measured bytes only (an unmeasured new item must not count as 0 forever: it is measured at the next check). → Task 1 tests.

---

### Task 1: `xiox-observer` — snapshot, diff, thresholds, report, bounded measurer

**Files:**
- Create: `crates/xiox-observer/Cargo.toml` (deps: `xiox-project`, `xiox-model`, `serde`, `serde_json`; dev: `tempfile`), `src/lib.rs`, `src/state.rs`, `src/measure.rs`, `src/tests/` (one test module per source file + fixtures)
- Modify: `crates/xiox-project/src/lib.rs` (+ a new `src/hint.rs`): make `too_shallow(&Item) -> bool` and `is_recommended(&Item) -> bool` public and move them out of `crates/xiox-cli/src/render.rs`, which calls the public ones (behaviour-preserving: the existing `render` tests must pass unchanged); `Cargo.toml` workspace members already use `crates/*`
- Test: `crates/xiox-observer/src/tests/`

**Interfaces:**
- Consumes: `xiox_project::{ProjectScan, Item, Category, Level, Why, Note, Project, SCHEMA}`, `xiox_model::path_key`.
- Produces (all `Serialize`, `rename_all = "camelCase"`, enums `kebab-case`):

```rust
pub struct Params { pub grow_threshold: u64 /* bytes, default 2 GiB */, pub worktree_threshold: usize /* default 3 */ }
pub struct ItemView {            // an Item plus observer facts; `#[serde(flatten)] item`
    pub item: xiox_project::Item,
    pub new: bool,
    pub first_seen_ms: i64,
    pub measured_at_ms: Option<i64>,
}
pub enum AttentionKind { Growth, Worktrees }
pub struct Totals { pub by_category: std::collections::BTreeMap<String, u64> /* bytes, measured only */, pub items: usize, pub recommended_bytes: u64 }
pub struct Report {
    pub schema: u32,                         // xiox_project::SCHEMA
    pub project: xiox_project::Project,
    pub items: Vec<ItemView>,
    pub notes: Vec<xiox_project::Note>,
    pub totals: Totals,
    pub growth_bytes: i64,                   // since the baseline, measured bytes only
    pub attention: Vec<AttentionKind>,       // thresholds currently exceeded
    pub stale: bool,
    pub clean_command: Option<String>,       // `xiox clean "<root>" --include "<p1>" --include "<p2>" ...` for NEW recommended unblocked items that are not `too_shallow`
    pub clean_command_truncated: bool,       // more than 25 such items
    pub started_at_ms: i64, pub checked_at_ms: i64,
}
pub enum Event {                         // serde tag = "event", kebab-case; the CLI adds "schema" and "time"
    Started { project: xiox_project::Project, params: Params, totals: Totals },
    Changed { added: Vec<ItemView>, removed: Vec<String> /* item keys */, growth_bytes: i64 },
    Attention { kind: AttentionKind, value: u64, threshold: u64, clean_command: Option<String> },
    Report(Report),
    Error { message: String },
}
pub struct Observer { /* private */ }
impl Observer {
    /// Everything in `first` is the baseline: never `new`.
    pub fn new(first: ProjectScan, params: Params, now_ms: i64) -> Self;
    pub fn started(&self) -> Event;
    /// Feed the result of a re-check. `scan.items[*].bytes` are the measured values (None = not measured).
    /// Returns `Changed` (only if something was added/removed) and then any `Attention` events.
    pub fn update(&mut self, scan: ProjectScan, now_ms: i64) -> Vec<Event>;
    /// Keys of items whose bytes should be measured at the next check: new items without bytes and baseline `builds` items.
    pub fn keys_to_measure(&self) -> Vec<String>;
    pub fn report(&self, stale: bool) -> Report;
}
/// Bounded recursive size of `path`: never follows links/reparse points; stops after `max_entries` entries
/// or `max_time`; returns `None` when it hit a limit or could not read the root.
pub fn measure_bytes(path: &std::path::Path, max_entries: usize, max_time: std::time::Duration) -> Option<u64>;
```

Rules: growth = Σ bytes of current items not in the baseline + Σ max(0, current − baseline) for items in both with both measured; `Growth` attention fires when growth > `grow_threshold` (value = growth), `Worktrees` when (number of `Category::Worktrees` items not in the baseline) > `worktree_threshold`; each fires once when crossing, re-arms when back at or below the threshold. `Changed` lists items by key difference only (not size changes). `removed` keys that were in the baseline are reported once; a baseline item that disappears and later reappears is **not** new (it was in the baseline) — it appears in `added` with `new: false`.

- [ ] **Step 1: Write the failing tests** (module names in `src/tests/`): `state.rs`: `baseline_items_are_never_new_even_when_they_grow`, `a_new_item_is_added_with_first_seen_and_new_true`, `a_baseline_item_that_disappears_is_removed_and_when_back_is_added_not_new`, `no_change_gives_no_events`, `growth_counts_new_items_and_growth_of_baseline_items_but_not_shrinking`, `growth_attention_fires_once_then_rearms_after_dropping_below`, `worktree_attention_counts_only_worktrees_not_in_the_baseline`, `unmeasured_new_item_is_listed_for_measuring_and_counts_when_measured`, `report_marks_new_items_and_builds_clean_command_from_new_recommended_unblocked_items`, `clean_command_skips_blocked_and_too_shallow_items_and_truncates_at_25`, `report_stale_flag_is_passed_through`, `events_serialize_with_kebab_case_event_tag`. `measure.rs`: `sums_nested_files`, `does_not_follow_a_junction_or_symlink` (create with `mklink /J`/`symlink_dir`; skip with a printed reason only if refused), `entry_cap_returns_none`, `time_cap_returns_none` (cap of 0 ms), `missing_root_is_none`. In `crates/xiox-cli`: the existing render tests must pass after the helper move.
- [ ] **Step 2:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-observer` → FAIL (nothing exists).
- [ ] **Step 3: Implement** `state.rs`/`measure.rs`/`lib.rs` per the Interfaces block; move `too_shallow`/`is_recommended` into `xiox-project` as `pub` (`hint.rs`), re-export from `lib.rs`, and call them from `render.rs`. `measure_bytes` uses `symlink_metadata`, skips entries whose attributes include the reparse-point bit (0x400), iterative (no deep recursion).
- [ ] **Step 4:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-observer -p xiox-project -p xiox-cli` → PASS; no warnings.
- [ ] **Step 5: Commit** `crates/xiox-observer crates/xiox-project crates/xiox-cli/src/render.rs Cargo.lock` — `feat(xiox-observer): fotografia, differenze, soglie e misura limitata`.

---

### Task 2: `xiox observe` — JSON-lines mode

**Files:**
- Modify: `crates/xiox-cli/Cargo.toml` (deps `xiox-observer`, `chrono`), `src/args.rs` (`Command::Observe`), `src/main.rs`, `src/run.rs` (dispatch), `crates/xiox-cli/tests/cli.rs`
- Create: `crates/xiox-cli/src/observe.rs` (loop + JSON lines), `src/size.rs` (size parser)
- Test: unit tests in `args.rs`, `size.rs`, `observe.rs`; binary tests in `tests/cli.rs`

**Interfaces:**
- Consumes: Task 1 (`Observer`, `Params`, `Event`, `measure_bytes`), `xiox_project::{analyze_project, analyze_project_in, analyze_project_without_sessions, Options, is_elevated}`, the CLI's `write_result` helper and exit-code constants in `run.rs`, the debug-only switch helper (`test_switches()`).
- Produces:

```rust
// args.rs
Command::Observe { project: Option<PathBuf>, interval_secs: u64 /* default 600, min 60 */, grow_threshold: u64 /* default 2 GiB */,
                   worktree_threshold: usize /* default 3 */, no_drives: bool, mcp: bool }
// size.rs
pub fn parse_size(text: &str) -> Result<u64, String>;   // "2GB", "2 GiB", "500MB", "1.5GB", "1024"; binary units; KB/KiB/MB/MiB/GB/GiB/TB/TiB/B; case-insensitive; rejects 0, negatives, garbage
// observe.rs
pub fn observe(cmd: ObserveArgs, stdin: impl BufRead + Send + 'static, out: &mut dyn Write, err: &mut dyn Write, deps: &Deps) -> i32;
pub struct Deps { pub analyze: Box<dyn Fn() -> Result<ProjectScan, String>>, pub measure: Box<dyn Fn(&Path) -> Option<u64>>, pub now_ms: Box<dyn Fn() -> i64>, pub interval: Duration }
```
Behaviour: usage `xiox observe [<project>] [--interval <seconds>] [--grow-threshold <size>] [--worktree-threshold <n>] [--no-drives] [--mcp]`; `--interval` below 60 → usage error (exit 2) (the debug-only `XIOX_OBSERVE_INTERVAL_MS` bypasses the minimum); each stdout line is one compact JSON object `{"schema":1,"event":"<kebab>","time":"<RFC 3339 UTC>", …event fields}`; first the initial light analysis (`Options { measure: false, drives: !no_drives, admin: is_elevated() }`), then bytes measured for all its items that have none (via `measure_bytes`, 200 000 entries / 10 s caps), baseline built, `started` emitted; the loop waits `interval` on a channel fed by a stdin-reader thread; on each tick: analyze (light), measure `keys_to_measure()`, `update`, emit events; stdin line `report` → fresh check then a `report` event; `stop` or EOF → exit 0 with nothing more on stdout; a failed check → `error` event and the loop continues (report requests answer with the last result, `stale: true`); the first analysis failing → message on stderr, exit 1, nothing on stdout. Unknown stdin lines → `error` event ("unknown command"). Panics: set a panic hook that writes to stderr only.

- [ ] **Step 1: Write the failing tests:** `args.rs`: `observe_defaults`, `observe_all_flags`, `observe_interval_below_60_is_usage_error`, `observe_bad_size_and_zero_worktree_threshold_are_usage_errors`, `observe_accepts_project_path_and_rejects_second_positional`. `size.rs`: `parses_units_case_insensitively_and_binary`, `rejects_zero_negative_empty_and_garbage`. `observe.rs` (fake `Deps`, `Cursor` stdin, in-memory out/err): `started_is_the_first_line_and_all_lines_are_valid_json_with_schema_time_event`, `nothing_changes_gives_no_changed_event`, `a_new_worktree_appears_in_a_changed_event_and_a_report_marks_it_new`, `report_command_on_stdin_emits_a_report_event`, `stop_and_eof_exit_zero_with_no_extra_output`, `failed_check_emits_error_and_keeps_going_and_report_is_stale`, `first_analysis_failure_exits_1_with_empty_stdout`, `unknown_stdin_line_is_an_error_event`, `nothing_but_json_on_stdout` (stderr carries human text). `tests/cli.rs` (debug-only, fake repo, `XIOX_OBSERVE_INTERVAL_MS=200`, piped stdin): `observe_binary_prints_started_then_a_changed_event_after_a_worktree_is_added`, `observe_binary_exits_when_stdin_closes`, `observe_binary_interval_below_60_exits_2`.
- [ ] **Step 2:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli observe` → FAIL.
- [ ] **Step 3: Implement** the files above; reuse `write_result` for every write (a failed stdout write ends the loop with exit 1; BrokenPipe = caller gone = exit 0).
- [ ] **Step 4:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli -p xiox-observer` → PASS, no warnings (the pre-existing `analyze`/`clean` tests unchanged).
- [ ] **Step 5: Commit** `crates/xiox-cli Cargo.lock` — `feat(xiox): comando observe con righe JSON`.

---

### Task 3: `xiox observe --mcp` — read-only MCP server on stdio

**Files:**
- Create: `crates/xiox-cli/src/mcp.rs`, tests in the same file and in `tests/cli.rs`
- Modify: `crates/xiox-cli/src/observe.rs` (the `--mcp` branch shares the loop's analysis/measure/update step: factor it into one function used by both modes, no duplication), `src/run.rs`

**Interfaces:**
- Consumes: Task 2's shared check step, `Observer::report`, `Report` serialization.
- Produces: an MCP server over **stdio** (newline-delimited JSON-RPC 2.0, one message per line, no embedded newlines; stdout carries only protocol messages), implementing: `initialize` (reply with `protocolVersion` = the client's if it is one of `"2025-06-18"`, `"2025-03-26"`, `"2024-11-05"`, else `"2025-06-18"`; `capabilities: { tools: {} }`; `serverInfo: { name: "xiox", version: <cargo pkg version> }`; `instructions` English text telling the agent to call `project_leftovers` before declaring work finished and after creating worktrees or copies, and that the tool only reports and never removes), notification `notifications/initialized` (no reply), `ping` (empty result), `tools/list` (one tool), `tools/call`, and a JSON-RPC error for anything else (`-32601` method not found, `-32602` invalid params, `-32700` parse error with `id: null`). The tool: name `project_leftovers`, description English (read-only; what it returns), `inputSchema: {"type":"object","properties":{},"additionalProperties":false}`, `annotations: {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}`; `tools/call` runs a fresh check (analyze → measure → update) then returns `{"content":[{"type":"text","text":"<the Report as compact JSON>"}],"structuredContent":<the Report>,"isError":false}`; if the fresh check fails: the last report with `"stale": true` and `isError: false`, and if there is no report yet `isError: true` with an English message. Unknown tool name → JSON-RPC error `-32602`. No periodic `attention` push exists in this mode (MCP only answers): the report carries `attention`. The process still re-checks on the interval in the background? **No**: in `--mcp` mode it checks only when called (single-threaded, no timer), and exits on EOF.

- [ ] **Step 1: Write the failing tests** (`mcp.rs`, in-memory reader/writer + fake `Deps`): `initialize_negotiates_the_protocol_version_and_has_instructions`, `initialize_with_an_unknown_version_answers_the_latest`, `initialized_notification_gets_no_reply`, `tools_list_has_exactly_one_read_only_tool`, `tools_call_returns_the_report_as_text_and_structured_content`, `a_new_worktree_between_two_calls_shows_up_as_new_in_the_second`, `failed_check_returns_the_last_report_stale`, `failed_check_with_no_report_is_an_error_result`, `unknown_tool_unknown_method_and_garbage_line_give_jsonrpc_errors_and_the_server_keeps_running`, `ping_works`, `every_output_line_is_a_single_json_object`, `eof_ends_the_server`. `tests/cli.rs`: `mcp_binary_answers_initialize_list_and_call_over_stdio` (spawn, write the three requests, read three lines, close stdin, expect exit 0; debug-only switches as in Task 2).
- [ ] **Step 2:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli mcp` → FAIL.
- [ ] **Step 3: Implement** `mcp.rs` and the shared check function.
- [ ] **Step 4:** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli -p xiox-observer` → PASS, no warnings.
- [ ] **Step 5: Commit** `crates/xiox-cli` — `feat(xiox): osservatore come servizio MCP su stdio`.

---

### Task 4: Documentation, help text, release check

**Files:**
- Create: `docs/xiox-observe.md` (English guide)
- Modify: `crates/xiox-cli/src/args.rs` (the `--help` text: add `observe` and a short MCP configuration example), `README.md` (Roadmap: tick the observer box, link the guide; a short "Observe a project" subsection), `docs/xiox-cli.md` (one-paragraph pointer), `docs/superpowers/specs/2026-11-13-project-observer-mcp-design.md` (append «Esito» in Italian: what was built, the two deviations of this plan, test totals, exe size, limits)

**Interfaces:** consumes the real behaviour of Tasks 1–3 (read the code; document only what it does).

Guide content (English, for people and AI agents; examples from REAL runs of a debug build on a fake tree with `XIOX_NO_SESSIONS=1`, `XIOX_SEARCH_ROOTS`, `XIOX_OBSERVE_INTERVAL_MS` — never on real projects; the debug-only switches must NOT appear in the guide): what it is and is not (never removes; one process per project; who starts it); quick start (start it, read the JSON lines, ask for a `report`, `stop`); every flag with defaults and the minimum interval; the event table (fields, when); the report and `cleanCommand` (why `--include` and the 25 cap); thresholds and re-arming; MCP mode (what it exposes, `instructions`, the tool, results, no background timer); configuration snippets for MCP clients as JSON with a `command`/`args` stanza (`{"command":"xiox","args":["observe","C:\\path\\to\\project","--mcp"]}`) and, ONLY for clients whose format you can verify from files present on this machine or from the harness's own documentation you can read, the client-specific file location — otherwise say plainly "use your client's way of adding a stdio MCP server with this command"; limits (bounded search without elevation, measuring caps, `bytes: null`, no attribution to sessions); troubleshooting (nothing appears on stdout, exits immediately, exit codes). Italian-words check: `grep -n -i -E "consigliat|attenzione|cartell|pulisc|sessione" docs/xiox-observe.md` prints nothing; the guide mentions no `XIOX_*` switch.

- [ ] **Step 1:** Build debug: `CARGO_INCREMENTAL=0 cargo build -j 1 -p xiox-cli`; create a fake tree (repo with a `target`, one linked worktree) under `%TEMP%` and run the real examples (default mode with `XIOX_OBSERVE_INTERVAL_MS=300` feeding `report`/`stop` on stdin; add a worktree between ticks; MCP initialize/list/call over a pipe). Save outputs.
- [ ] **Step 2:** Write `docs/xiox-observe.md` from the real outputs; update help text, README, `docs/xiox-cli.md`.
- [ ] **Step 3:** Italian grep check (above) and `grep -i -E "XIOX_" docs/xiox-observe.md` (must print nothing); re-read the guide against the code once.
- [ ] **Step 4:** Whole suite `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade` (before: 1013 passed, 0 failed, 20 ignored + the new tests) and release build `CARGO_INCREMENTAL=0 cargo build -j 1 --release --locked -p xiox-desktop -p xiox-cli`; note `xiox.exe` size.
- [ ] **Step 5:** Append the spec «Esito»; commit `docs/xiox-observe.md README.md docs/xiox-cli.md crates/xiox-cli/src/args.rs docs/superpowers/specs/2026-11-13-project-observer-mcp-design.md` — `docs(xiox): guida dell'osservatore e relazione`.
