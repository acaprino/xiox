# Transcript formats of coding agents and fast incremental JSONL scanning in Rust

Research date: 2026-11-13. Source code was read from shallow clones of the GitHub repos at their HEAD on that date: openai/codex commit 4413deac (2026-11-13), sst/opencode commit e6fc5888 (2026-11-13, `dev` branch), and badlogic/pi-mono (whose docs now point to `earendil-works/pi`). Claude Code facts come from the official docs plus a real transcript on this machine (Claude Code, Nov 2026). The Claude Code JSONL record schema is **not officially documented**; the field-level details below are reverse-engineered.

## Q1. Where file paths appear in structured fields of agent transcripts

### Takeaway
Each agent stores a session-level working directory in a dedicated structured field: Claude Code `cwd` on every message record, Codex `session_meta.payload.cwd` plus `turn_context.payload.cwd`, Pi's header `cwd`, and OpenCode `session.directory` / `project.worktree`. Tool-call arguments add file-level paths, for example Claude `file_path`/`path`, Pi `path`, Codex `workdir` plus paths inside the apply_patch text, and OpenCode `filePath`. All of these sit inside known JSON keys, so they can be extracted without free-text heuristics. Two traps: Codex stores function-call `arguments` as a JSON-encoded *string* (double-decode needed), and Codex now compresses cold rollouts to `.zst`.

### Cited Findings

**Claude Code (`~/.claude/projects/<project>/<session>.jsonl`)**
- Official doc layout: `projects/<project>/<session>.jsonl` holds the "Conversation transcript: messages, tool calls, and tool results, plus the system prompt, tool definitions, and parameters". Subagent transcripts are at `projects/<project>/<session>/subagents/` in the same format. Large tool outputs spill to `projects/<project>/<session>/tool-results/`. Set-aside copies are named `<session>.orphaned-<timestamp>-<suffix>.jsonl` and `<session>.jsonl.superseded-<timestamp>`. `~/.claude/history.jsonl` holds "Prompts you've typed, with timestamp and project path". With `CLAUDE_CONFIG_DIR` set, everything moves there; on Windows the root is `%USERPROFILE%\.claude` — [Claude Code docs: Explore the .claude directory](https://code.claude.com/docs/en/claude-directory)
- Project folder encoding: the docs give the rule for the parallel temp/scratchpad tree as "`<project>` is your working directory path with every character other than letters and digits replaced by `-`, such as `-Users-you-my-project`. On Windows, `<project>` is a 16-character hash of that string". The same page's `claude purge` example maps `/home/user/work/my-repo` → `~/.claude/projects/-home-user-work-my-repo` — [Claude Code docs](https://code.claude.com/docs/en/claude-directory). Note that the Windows-hash statement is made for the temp tree, so it is unclear whether it also applies to `~/.claude/projects`. Because the encoding is lossy (`/`, `.`, `_`, `-` and spaces all become `-`), the folder name cannot be decoded reliably. Use the `cwd` field inside the records instead.
- Transcripts are retained for `cleanupPeriodDays`, default 30 days and minimum 1, so older history is deleted — [Claude Code docs](https://code.claude.com/docs/en/claude-directory)
- Observed on a real transcript on this machine (reverse-engineered, Nov 2026):
  - Top-level `type` values: `user`, `assistant`, `attachment`, `system`, `ai-title`, `last-prompt`, `queue-operation`, `tools-used`, `atis-latch`.
  - The `cwd` field (along with `sessionId`, `gitBranch`, `version`, `timestamp`, `uuid`, `parentUuid`, `isSidechain`, `entrypoint`) appears on every `user`, `assistant`, `attachment` and `system` record. It does not appear on bookkeeping records (`ai-title`, `last-prompt`, `queue-operation`, `tools-used`).
  - Tool calls are `message.content[]` blocks with `{"type":"tool_use","name":...,"input":{...}}` inside `assistant` records. Results are `{"type":"tool_result",...}` blocks inside `user` records, and those records also carry a structured top-level `toolUseResult` object (for Bash: `stdout`, `stderr`, `interrupted`, `isImage`, `noOutputExpected`).
  - Source: local inspection of `~/.claude/projects/-home-user-xiox/*.jsonl` (no public URL; this is first-hand observation, not documentation).
- Claude Code tool input field names, from the current tool schemas exposed to the model in this session (first-hand, not a public doc):
  - `Read`, `Edit`, `Write`: `file_path` (absolute).
  - `Glob`, `Grep`: optional `path` (search directory), plus `pattern`/`glob`.
  - `Bash`: `command` (free-text shell).
  - `NotebookEdit`: `notebook_path` (recalled from training; not re-verified here).

**Codex CLI (`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl`)**
- File naming in `codex-rs/rollout/src/recorder.rs`: the doc comment uses `~/.codex/sessions/rollout-2025-05-07T17-24-21-<uuid>.jsonl`, the code says "Resolve ~/.codex/sessions/YYYY/MM/DD path", and a revert variant is named `rollout-<timestamp>-<conversation_id>_<rollout_id>.jsonl`. There is also an archived sessions directory — [openai/codex recorder.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/recorder.rs)
- **New in 2026**: `codex-rs/rollout/src/compression.rs` defines `COMPRESSED_SUFFIX = ".zst"`, with a "best-effort background job that compresses cold local rollout files". A scanner therefore has to handle both `rollout-*.jsonl` and `rollout-*.jsonl.zst` — [openai/codex compression.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/compression.rs)
- Each line is a `RolloutLine { timestamp, ordinal?, #[serde(flatten)] item: RolloutItem }` — [codex-rs/history/src/lib.rs](https://github.com/openai/codex/blob/main/codex-rs/history/src/lib.rs). The item is tagged `#[serde(tag = "type", rename_all = "snake_case")]` with `payload`, and its variants are `session_meta`, `dynamic_tools_updated`, `response_item`, `inter_agent_communication`, `inter_agent_communication_metadata`, `compacted`, `turn_context`, `token_usage_record`, `world_state`, `retained_context`, `security_risk_score`, `event_msg` and `realtime_item` — [codex-rs/history/src/rollout_payload.rs](https://github.com/openai/codex/blob/main/codex-rs/history/src/rollout_payload.rs)
- `SessionMeta` has `cwd: PathBuf`, along with `session_id`, `id`, `forked_from_id`, `parent_thread_id` and `timestamp`. `SessionMetaLine` flattens it and adds `git: Option<GitInfo>`. `TurnContextItem` has `cwd: AbsolutePathBuf` (per turn, so it can change mid-session), plus `approval_policy`, `model` and others — [codex-rs/protocol/src/protocol.rs](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs)
- `ResponseItem::FunctionCall { name, namespace?, arguments: String, call_id }`. The source comment says: "The Responses API returns the function call arguments as a *string* that contains JSON… We keep it as a raw string". `LocalShellCall { call_id, status, action: LocalShellAction }` and `CustomToolCall { call_id, name, input }` also exist (custom/freeform tools such as apply_patch carry raw text input) — [codex-rs/protocol/src/models.rs](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/models.rs)
- The unified exec tool args are `ExecCommandArgs { cmd: String, shell?, login?, tty, yield_time_ms, timeout_ms?, max_output_tokens?, sandbox_permissions?, justification?, prefix_rule?, workdir: Option<String> }`, with the comment "resolved against the selected environment cwd, not the process cwd" — [codex-rs/core/src/tools/handlers/unified_exec.rs](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/handlers/unified_exec.rs)
- apply_patch paths are not separate JSON fields. They are embedded in the patch envelope text as `*** Add File: <path>` / `*** Update File: <path>` / `*** Delete File: <path>` (handled in `codex-rs/core/src/tools/handlers/apply_patch*.rs`) — [openai/codex tools/handlers](https://github.com/openai/codex/tree/main/codex-rs/core/src/tools/handlers). Extracting them requires a line-regex over the decoded patch string.

**Pi coding agent (badlogic/pi-mono, now earendil-works/pi)**
- Location: `~/.pi/agent/sessions/--<path>--/<timestamp>_<session-id>.jsonl`. "For `<path>`, Pi removes the leading path separator and replaces `/`, `\\`, and `:` with `-`" (lossy, like Claude's) — [pi session-format.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/session-format.md)
- The first line is the header `{"type":"session","version":3,"id":"uuid","timestamp":...,"cwd":"/path/to/project"}`, with an optional `parentSession` path for forks. Other entries are `message`, `model_change`, `thinking_level_change`, `usage`, `compaction`, `context_edit`, branch summary, custom, label and session info, linked into a tree by `id`/`parentId`. System messages can carry `sections.cwd` — [pi session-format.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/session-format.md)
- `compaction` and branch-summary entries have `details: { readFiles: string[], modifiedFiles: string[] }`. This is a high-signal, pre-aggregated file list — [pi session-format.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/session-format.md)
- Tool calls are assistant content blocks `{type:"toolCall", ..., arguments: Record<string,any>}` (an object, not a string). Results are `role:"toolResult"` messages with `toolCallId` and `toolName` — [pi message-types.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/message-types.md)
- Tool parameter names in `packages/coding-agent/src/core/tools/*.ts`:
  - `read` / `edit` / `write`: `path` ("relative or absolute").
  - `bash`: `command`.
  - `grep`: `pattern` and optional `path`.
  - `find`: `pattern` and optional `path`.
  - `ls`: optional `path`.
  - Relative paths resolve against the session cwd.
  - Source: [pi tools](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent/src/core/tools)

**OpenCode (SQLite)**
- No official schema reference was found. Community sources say the DB lives at `~/.local/share/opencode/opencode.db`, runs in WAL mode (with `-wal`/`-shm` sidecars), and has tables `project` 1─* `session` 1─* `message` 1─* `part`. `message.data` and `part.data` are JSON columns. Column names vary by version (`time_created` vs `created_at`; one source mentions a `session_v2` table) — [Tachikoma opencode-database notes](https://nirvaxstiel.github.io/Tachikoma-Agent-Skills/internals/opencode-database.html), [openoxide directory layout](https://openoxide.pages.dev/features/sessions/directory-layout) (both third-party, so lower confidence)
- The third-party Rust reader `toolpath-opencode` 0.4.0 models:
  - `Session { id, project_id, directory: PathBuf, title, ... }`
  - `Project { id, worktree: PathBuf }`
  - `Part { id, message_id, session_id, time_created, time_updated, data }`
  - `ToolPart { tool, callID, state }`, where `state.input` is untyped JSON.
  - Source: [toolpath-opencode types.rs](https://docs.rs/crate/toolpath-opencode/0.4.0/source/src/types.rs)
- Older OpenCode versions (pre-SQLite) stored JSON files under `~/.local/share/opencode/storage/{session,message,part}/...`. This comes from training knowledge and was not re-verified. A scanner should probe for both layouts.
- The repo was refactored heavily by Nov 2026: `sst/opencode` HEAD no longer has `packages/opencode/src/session/session.sql.ts`. There is a new `packages/core/src/database` built on drizzle. I could not locate the current table definitions in the time available.

### Inferences
- **High-signal fields** (structured, written by the harness rather than the model):
  - Claude `cwd` (per record).
  - Codex `session_meta.payload.cwd`, `turn_context.payload.cwd` and `git` info.
  - Pi header `cwd`, plus compaction `details.readFiles` / `details.modifiedFiles`.
  - OpenCode `session.directory` / `project.worktree`.
  - Claude `gitBranch` and Codex `git` help tell repos apart.
- **Medium-high signal** (model-chosen but structured):
  - Write/edit paths: Claude `Edit`/`Write` `file_path`, Pi `edit`/`write` `path`, Codex apply_patch headers, OpenCode edit/write `filePath`. A modification is stronger evidence of "worked in" than a read.
  - Read/search paths: Read `file_path`, Glob/Grep `path`, Codex exec `workdir`.
  - Take the parent directory of the file and walk up to the repo root (`.git`).
- **Medium-low signal**: Bash/`cmd` strings. They need shell tokenizing (e.g. `cd X && ...`, absolute args) and are noisy (pipes to `/dev/null`, `/tmp`, system paths).
- **Low signal**:
  - Free-text assistant/user messages.
  - Tool *outputs* (`ls`, `git status`, `find`, grep hits, `toolUseResult.stdout`, Codex `function_call_output`). These mention many paths the user never worked in.
  - Claude's system prompt and attachments (they list CLAUDE.md paths, skills directories and so on).
- Paths can be relative: Pi says so explicitly, and Codex `workdir` is relative to cwd. Resolve them against the record's/turn's cwd before aggregating.
- Prefilter keys for a byte-level scan are `"cwd"`, `"file_path"`, `"notebook_path"`, `"path"`, `"filePath"`, `"workdir"`, `"session_meta"`, `"turn_context"` and `*** Update File:`. Codex arguments are double-encoded, so inside the line the key appears as `\"workdir\"`.

### Gaps
- Claude Code's JSONL record schema has no official public spec. The field list is observed only and may change between versions. I could not verify whether `~/.claude/projects` uses the Windows 16-char hash.
- The current OpenCode (Nov 2026) SQLite table/column definitions and exact tool input keys (`filePath`, `workdir` for bash) were not confirmed from source. They are recalled from training and third-party readers.
- Codex `event_msg` subtypes that may also carry cwd (e.g. exec begin events) were not enumerated.

## Q2. Fast scanning of GB-scale JSONL in Rust

### Takeaway
The fastest practical pipeline:
1. Read big chunks.
2. Split on `\n` with `memchr`.
3. Reject most lines cheaply with a `memmem`/aho-corasick prefilter on key names.
4. Only fully parse surviving lines, and only partially: serde with borrowed `&str`/`RawValue` and ignored fields, or sonic-rs `get`/`LazyValue`.
5. Parallelize per file with rayon.

Avoid mmap for files that agents are actively appending to.

### Cited Findings
- `memchr` provides SIMD (SSE2, plus AVX2 detected at runtime, NEON, wasm) search for 1–3 bytes and `memmem` substring search (Rabin-Karp / Generic SIMD / Two-Way with prefilter). In rebar benchmarks, memchr's geometric-mean speed ratio is 1.03 against std's 6.50 across 53 "prebuilt" benchmarks — [memchr README](https://github.com/BurntSushi/memchr)
- sonic-rs (SIMD JSON):
  - Struct deserialize on twitter.json: 827.74 µs, against simd-json 1.0872 ms and serde_json 2.2895 ms (~2.8x faster than serde_json).
  - Untyped Value: 556 µs against 3.80 ms for serde_json (~6.8x).
  - `get_from_str` by JSON pointer: 434.62 µs (against gjson 363.14 µs). `get_unchecked_from_str`: 76.8 µs.
  - APIs: `get`/`get_unchecked` return `LazyValue` (a raw slice), plus `to_object_iter`.
  - It needs x86_64/aarch64 and `-C target-cpu=native` to be fast; other architectures fall back "maybe very slower".
  - Source: [sonic-rs README](https://github.com/cloudwego/sonic-rs)
- grep-searcher (ripgrep's library): `MmapChoice::auto()` is `unsafe` because the "caller guarantees that the underlying file won't be mutated". Maps are disabled by default, and CLI tools accept the risk of "`SIGBUS` occurring while attempting to read a memory map". The searcher falls back to read calls when it can't map — [grep-searcher MmapChoice](https://docs.rs/grep-searcher/latest/grep_searcher/struct.MmapChoice.html)
- memmap2: "All file-backed memory map constructors are marked `unsafe` because of the potential for Undefined Behavior… if the underlying file is subsequently modified, in or out of process" — [memmap2 Mmap docs](https://docs.rs/memmap2/latest/memmap2/struct.Mmap.html)
- Windows: before `SetEndOfFile` (truncate) can be called on a file with a mapping object, "UnmapViewOfFile must be called first to unmap all views and call CloseHandle to close the file mapping object". A reader holding a map can therefore make the agent's truncate/rewrite fail — [Microsoft Learn SetEndOfFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setendoffile)
- The following come from training knowledge (docs.rs pages not fetched in this session):
  - `serde_json::StreamDeserializer` iterates concatenated values and exposes `byte_offset()`.
  - `serde_json::value::RawValue` (feature `raw_value`) defers parsing of a subtree.
  - `#[serde(borrow)] &'a str` and `Cow<'a, str>` avoid allocations, and unknown fields are skipped by default.
  - `simd-json` needs a mutable buffer (in-place parse).
  - `jiter` (pydantic) is an iterator/lazy JSON parser.
  - `struson` is a streaming pull reader.
  - `regex::bytes::RegexSet` reports which of many patterns matched.
  - `aho-corasick` does multi-literal search, with a Teddy SIMD prefilter for small pattern sets.

### Inferences
- In these transcripts, most bytes are tool outputs and file contents, and the path-bearing fields are a small fraction. A `memmem` prefilter for `"cwd"` / `"file_path"` / etc. before any JSON parse should make the scan close to I/O-bound. Literal multi-search over cached data typically runs at multiple GB/s per core; this is general knowledge and was not benchmarked here.
- Claude lines repeat `cwd` on nearly every record. Instead of parsing them, the scanner can grab `"cwd":"` with `memmem` and read up to the closing quote (handling `\\` escapes, especially on Windows paths). Unescape only on the slow path.
- Codex `arguments` strings require a second `serde_json::from_str` on the decoded string. `.zst` rollouts require streaming decompression (`zstd` crate) and cannot be resumed by byte offset; cache those by (size, mtime) only.
- Prefer buffered `read` (e.g. 64 KiB–1 MiB `BufReader` / `read_until(b'\n')` or manual memchr chunking) over mmap on live files. This avoids SIGBUS/UB on concurrent truncation and Windows mapping conflicts, and mmap gives little benefit for single sequential passes.
- Parallelism: rayon `par_iter` over files (thousands of sessions) is simplest. Within-file parallel chunking only matters for single multi-GB files.

### Gaps
- No published head-to-head benchmark of memchr-prefilter + partial parse on agent transcripts specifically. Throughput would need local measurement.
- I did not fetch BurntSushi's ripgrep blog post or jiter/struson docs for exact figures in this session.

## Q3. Incremental caching and prior art

### Takeaway
Use a per-file cache entry: identity = path + (dev, inode / Windows file index) + a fingerprint of the first N bytes, with validity = (size, mtime). The append-only fast path resumes from the last complete newline offset. If size has decreased or the head fingerprint has changed, discard and rescan. This is exactly the model mature log shippers use: Vector (checksum of first lines), and Filebeat (fingerprint of the first 1024 bytes, offsets in an append-only registry).

### Cited Findings
- Vector `file` source:
  - Checkpoints are saved "after each successful read" into `data_dir`.
  - `fingerprint.strategy` is `checksum` (default: "Read lines from the beginning of the file and compute a checksum", `fingerprint.lines` default 1, `ignored_header_bytes`) or `device_and_inode`, with the caveat that inodes can be reused.
  - Gzip files with an existing checkpoint are not re-read.
  - `read_from` is `beginning`/`end` for new files.
  - Source: [Vector file source docs](https://vector.dev/docs/reference/configuration/sources/file/)
- Filebeat `filestream`:
  - Default identity is `fingerprint` over `offset: 0`, `length: 1024` (min 64). Changing it re-ingests everything.
  - `growing: true` (9.5+) handles files smaller than the fingerprint length.
  - Alternatives: `native` (inode+device, which "can change, which risks duplication"), `path`, and `inode_marker` (not on Windows).
  - The registry is an append-only log of offset updates, with GC via `clean_inactive` / `clean_removed`.
  - With fingerprint identity (9.5+), copytruncate rotation is detected automatically.
  - Source: [Filebeat filestream docs](https://www.elastic.co/docs/reference/beats/filebeat/filebeat-input-filestream)
- Claude Code itself may replace transcripts: `<session>.jsonl.superseded-<ts>` / `.orphaned-` copies are made "instead of overwriting or deleting". The retention sweep deletes files after `cleanupPeriodDays`. A cache must therefore handle files that disappear and files that are rewritten rather than appended — [Claude Code docs](https://code.claude.com/docs/en/claude-directory)
- Codex compresses cold rollouts to `.zst` in the background. The `.jsonl` disappears and a `.jsonl.zst` appears, so the identity must survive a rename plus re-encoding. Keying by the session id in the filename is more robust than by path — [codex compression.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/compression.rs)
- Pi entries form a tree via `id`/`parentId` and are appended in place ("enabling in-place branching without creating new files"), so files are append-only in normal operation — [pi session-format.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/session-format.md)
- For OpenCode SQLite in WAL mode, there is no byte offset to resume from. The incremental strategy is to query `WHERE time_updated > last_seen` (column per third-party readers), open the DB read-only, and tolerate `SQLITE_BUSY` — [toolpath-opencode types.rs](https://docs.rs/crate/toolpath-opencode/0.4.0/source/src/types.rs)

### Inferences
- Recommended cache record per file:
  - Fields: `{path, session_key, dev/inode (or Windows volume serial + file index), head_hash(first 1 KiB), size, mtime_ns, resume_offset (byte after last '\n'), tail_hash (hash of the 64 bytes before resume_offset), per-file aggregated results}`.
  - On rescan:
    1. If size and mtime are unchanged, skip.
    2. If size has grown and head_hash/tail_hash match, parse from `resume_offset`.
    3. Otherwise, do a full rescan.
  - Never commit an offset past a partial last line: agents may be mid-write.
- Persistence options:
  - A single `bincode`/`postcard` blob written atomically (write tmp + rename) is simplest and fastest for ~10^4 entries.
  - `redb`/sqlite only matter if the cache is huge or concurrent writers exist.
  - Version the cache with a schema/extractor version so extractor changes invalidate it.
- Since results are aggregations (folder → counts, last-seen), store per-file partial aggregates and merge them at query time. A changed file then only replaces its own contribution.
- Prior art for Claude logs: ccusage (ryoppippi/ccusage) and similar usage tools parse `~/.claude/projects/**/*.jsonl` (and Codex sessions) for token usage. To my knowledge from training, they rescan rather than offset-cache. Not verified in this session. ripgrep does no caching.

### Gaps
- I did not verify ccusage's or "mukade"'s current caching implementation. I found no source for "mukade".
- I found no published benchmark of offset-resume vs full rescan for agent transcripts.
