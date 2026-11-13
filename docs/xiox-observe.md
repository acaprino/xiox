# xiox observe: watch one project while you work on it

`xiox observe` is a long-running process that you start **for one project**. It takes a
snapshot of the project when it starts, checks again at intervals, and tells whoever started
it what has **appeared since**: new git worktrees, copies of the repository, backups, build
folders, logs, and how much space all of it takes. It answers either with one JSON object per
line on stdout (the default), or, with `--mcp`, as a read-only MCP server on stdio that an AI
agent can query.

It only watches. It never removes, moves or writes anything (apart from the cache of the
session listing that `xiox analyze` writes too), and it never asks for administrator rights. To
clean up, you run `xiox clean` yourself (see [`xiox-cli.md`](xiox-cli.md)); the observer's
report carries that command ready to run.

- [What it is, and what it is not](#what-it-is-and-what-it-is-not)
- [Quick start](#quick-start)
- [Reference](#reference)
- [Events (JSON lines)](#events-json-lines)
- [The report and `cleanCommand`](#the-report-and-cleancommand)
- [Thresholds and re-arming](#thresholds-and-re-arming)
- [MCP mode](#mcp-mode)
- [Configuring an MCP client](#configuring-an-mcp-client)
- [Limits](#limits)
- [Troubleshooting](#troubleshooting)

## What it is, and what it is not

- **It is started by whoever needs it**, for that one project: an agent harness, a script, an
  application, or you in a terminal. Nothing starts it on its own, nothing starts with Windows,
  and it does not discover projects.
- **One process per project.** Two observers on the same project are independent: each has its
  own snapshot and its own idea of what is new. There is no shared service, no network port, no
  state file between instances.
- **"New" means new since this process started.** Whatever existed at the start (the first
  snapshot) is `new: false` for as long as the process lives, even if it grows. The observer
  does not know who created something (which agent, which session): it never reads sessions to
  decide that.
- **It ends when its caller goes away**: when stdin closes, on a `stop` line (default mode), or
  when the pipe it writes to is closed. It is not a daemon.
- **It is light.** One analysis at a time per process. A check is the same bounded analysis that
  `xiox analyze` does (`git worktree list`, a bounded search by name and by repository
  identity), with sizes measured only for what is new and for the build folders that were there
  at the start. It never walks whole disks at intervals.
- **It is read-only.** It has no path to the code that removes things.
- **Output is English only.** Field names, event names, errors.

## Quick start

From a terminal, to try it by hand (stdin is your keyboard; type `report` and Enter to get a
report, `stop` to end it):

```powershell
xiox observe C:\src\my-project
```

From a program, start it as a child process with stdin and stdout redirected, read the lines,
write `report` when you want the full picture, and close its stdin (or write `stop`) when you
are done:

```powershell
$psi = [Diagnostics.ProcessStartInfo]::new('xiox', 'observe "C:\src\my-project"')
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.StandardOutputEncoding = [Text.Encoding]::UTF8
$psi.UseShellExecute = $false
$p = [Diagnostics.Process]::Start($psi)
$started = $p.StandardOutput.ReadLine() | ConvertFrom-Json        # the "started" event
$p.StandardInput.WriteLine('report')
do { $e = $p.StandardOutput.ReadLine() | ConvertFrom-Json } until ($e.event -eq 'report')
"{0} items, growth {1} bytes, attention: {2}" -f $e.items.Count, $e.growthBytes, ($e.attention -join ',')
$p.StandardInput.WriteLine('stop')
$p.WaitForExit()                                                   # exit code 0
```

For an AI agent harness, use [MCP mode](#mcp-mode) instead.

`<project>` is any folder of the project, as for `xiox analyze`: a subfolder of a git
repository, or one of its linked worktrees, means the repository's main working tree. Without
it, the current folder is used.

The examples in this guide come from real runs of the program on a throwaway project. The
checks were made much more often than the default so that a run is short, and the paths are
shortened (`...\w` stands for `C:\Users\alice\AppData\Local\Temp\w`):

| Folder | What it is |
|---|---|
| `...\w\sampleapp` | the project: a git repository with `Cargo.toml` and a `target` folder (3 MiB) |
| `...\w\sampleapp-feature` | a linked worktree (branch `feature`) that existed before the observer started |
| `...\w\sampleapp-spike1`, `...\w\sampleapp-spike2` | two linked worktrees created while the observer was running |

On stderr, a person sees what the process is doing:

```text
Observing C:\Users\alice\AppData\Local\Temp\w\sampleapp every 600s (stdin: report, stop). Nothing is removed.
```

On stdout, the first line is the snapshot (`started`); the JSON is formatted here, in the real
output it is one line:

```json
{
  "schema": 1,
  "event": "started",
  "time": "2026-11-13T20:30:52.871Z",
  "project": {
    "root": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp",
    "name": "sampleapp",
    "isRepo": true,
    "origin": "https://github.com/acme/sampleapp.git",
    "worktrees": [
      "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-feature"
    ]
  },
  "params": {
    "growThreshold": 2147483648,
    "worktreeThreshold": 3
  },
  "totals": {
    "byCategory": {
      "builds": 3145772,
      "worktrees": 172
    },
    "items": 2,
    "recommendedBytes": 3145944
  }
}
```

The lines below come from a second run on the same tree, started with
`--grow-threshold 1MB --worktree-threshold 1` so that both thresholds trip (its `started` line
only differs in `params`). While the observer runs, the two worktrees are created and a 2 MiB
file is written in `target`. The next check sees them: a `changed` event (one of the two added
items is shown), and an `attention` event for each threshold that is now exceeded:

```json
{
  "schema": 1,
  "event": "changed",
  "time": "2026-11-13T20:30:22.707Z",
  "added": [
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-spike1",
      "category": "worktrees",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1",
      "bytes": 171,
      "level": "recommended",
      "blocked": [],
      "why": {
        "type": "linked-worktree",
        "branch": "spike1",
        "head": "f0aae281f60de1b1a1e20a5fe2f80878b731cd2e"
      },
      "target": {
        "type": "worktree",
        "repo": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp"
      },
      "new": true,
      "firstSeenMs": 1794601822706,
      "measuredAtMs": 1794601822706
    }
  ],
  "removed": [],
  "growthBytes": 2097494
}
```

```json
{"schema":1,"event":"attention","time":"2026-11-13T20:30:22.709Z","kind":"growth","value":2097494,"threshold":1048576,"cleanCommand":"xiox clean \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\" --include \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1\" --include \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike2\""}
{"schema":1,"event":"attention","time":"2026-11-13T20:30:22.709Z","kind":"worktrees","value":2,"threshold":1,"cleanCommand":"xiox clean \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\" --include \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1\" --include \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike2\""}
```

A line that is not `report` or `stop` gets an `error` event, and the observer goes on:

```json
{"schema":1,"event":"error","time":"2026-11-13T20:30:22.868Z","message":"unknown command: hello (expected report or stop)"}
```

After `git worktree remove` of one of the worktrees, the next check says it is gone (the removed
entries are item keys, the normalised paths) and the growth is recomputed:

```json
{"schema":1,"event":"changed","time":"2026-11-13T20:30:32.967Z","added":[],"removed":["c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-spike1"],"growthBytes":2097323}
```

Finally `report` (a fresh check, then the complete picture; see
[The report](#the-report-and-cleancommand)) and `stop`: the process exits with code 0.

## Reference

```text
xiox observe [<project>] [--interval <seconds>] [--grow-threshold <size>]
             [--worktree-threshold <n>] [--no-drives] [--mcp]
```

| Flag | Default | What it does |
|---|---|---|
| `<project>` | the current folder | The project to watch. It must exist (`error: path not found: ...`, exit code 1). |
| `--interval <seconds>` | `600` | Seconds between two checks. A whole number from `60` to `2592000` (30 days); anything else is a usage error (exit code 2). The timer restarts after every check, including the one a `report` runs. Does not apply with `--mcp`. |
| `--grow-threshold <size>` | `2GB` | Growth since the start that raises an `attention` event. Units `B`, `KB`, `MB`, `GB`, `TB` (binary: `1 KB` = `1 KiB` = 1024 bytes; `KiB`, `MiB`, ... are the same), any case, optional space, decimals allowed: `500MB`, `1.5GiB`, `1073741824`. Zero or garbage is a usage error. |
| `--worktree-threshold <n>` | `3` | New worktrees (more than this many) that raise an `attention` event. A whole number, at least `1`. |
| `--no-drives` | off | Search for copies and name-matched folders only next to the project, not on every fixed drive (as in `xiox analyze`). |
| `--mcp` | off | Be a read-only MCP server on stdio instead of printing JSON lines. See [MCP mode](#mcp-mode). |
| `--flag=value` | | The flags that take a value also accept `--flag=value`. |

`--json`, `--builds`, `--worktrees`, `--logs`, `--sessions`, `--include`, `--permanent`, `--yes`
and `--dry-run` do not exist for `observe` (a usage error, exit code 2).

**Stdin (default mode).** One command per line, in lower case; blank lines are ignored:

| Line | What happens |
|---|---|
| `report` | A fresh check, then its events (`changed`, `attention`) if it found any, then a `report` event. |
| `stop` | The process exits with code 0, without writing anything more. |
| anything else | An `error` event: `unknown command: <the first 80 characters> (expected report or stop)`. |

End of input does the same as `stop`. A line longer than 64 KiB is dropped as it is read (it is
never kept in memory) and answered with an `error` event.

**Stdout** carries only the JSON lines (or, with `--mcp`, only MCP messages). Each line is one
complete JSON object, written and flushed at once; the characters U+2028 and U+2029 are written
as `\u2028` and `\u2029`, so that a reader that splits lines on them cannot break an object.
Everything meant for a person goes to **stderr**.

**Exit codes**

| Code | Meaning |
|---|---|
| `0` | Ended normally: `stop`, end of stdin, or the reader of stdout went away (closed pipe). |
| `1` | Error: the project path does not exist or is not a folder; in default mode, the first analysis failed (stdout is then empty and the reason is on stderr); stdout cannot be written; stdin cannot be read; the reader thread cannot start. An internal error (a panic) is reported on stderr and ends the process with a non-zero code. |
| `2` | Wrong usage: unknown flag, a flag of another command, a value out of range, a second project. Nothing is analysed. |

## Events (JSON lines)

Every line is an object with `"schema": 1`, `"event"` and `"time"` (RFC 3339, UTC, to the
millisecond), followed by the fields of the event. Field names are camelCase, enum values are
kebab-case. Fields that end in `Ms` (`firstSeenMs`, `startedAtMs`, ...) are Unix times in
milliseconds.

| Event | When | Fields |
|---|---|---|
| `started` | The first snapshot is complete (the first line; it comes after the first analysis, which measures every item, so on a big project it can take a while). | `project` (`root`, `name`, `isRepo`, `origin`, `worktrees`), `params` (`growThreshold` in bytes, `worktreeThreshold`), `totals` (`byCategory` in measured bytes, `items`, `recommendedBytes`). |
| `changed` | A check finds an item that the previous check did not have, or one that is gone. Growth alone does not produce it. | `added`: the items (below); `removed`: the keys of the items that are gone; `growthBytes`: growth since the start, in bytes. |
| `attention` | A threshold is exceeded (once per crossing, see [Thresholds](#thresholds-and-re-arming)). | `kind` (`growth` or `worktrees`), `value` (bytes, or the number of new worktrees), `threshold`, `cleanCommand` (a string, or `null`). |
| `report` | After a `report` line on stdin. | The [report](#the-report-and-cleancommand). |
| `error` | A check failed (the reason starts with `check failed:`), or a stdin line was unknown or too long. The process goes on. | `message`. |

An **item** has the fields of an item of `xiox analyze --json` (`key`, `category`, `path`,
`bytes`, `level`, `blocked`, `why`, `target`; see [`xiox-cli.md`](xiox-cli.md#json)) plus three
that only the observer knows:

| Field | Meaning |
|---|---|
| `new` | `true` when the item was not in the first snapshot. |
| `firstSeenMs` | When the observer first saw it. For an item of the first snapshot: when the process started. |
| `measuredAtMs` | When `bytes` was last measured, or `null` if it never was. |

`bytes` is `null` while the item has not been measured: it has no folder to measure, or the
measuring hit a limit or could not read it (see [Limits](#limits)).

If a check fails (for example the project folder was moved or deleted), the observer writes an `error` event
and the same text on stderr, keeps the last good picture, and tries again at the next tick. A
`report` after a failed check is answered with the last good picture and `"stale": true`.

## The report and `cleanCommand`

The `report` event, and the MCP tool result, have the shape of `xiox analyze --json` (the same
`schema`, `project`, `items`, `notes`, `branches`, `search`, `scannedAtMs`, `elapsedMs`) with
the observer's fields added. This is the report of a real MCP call, made after a worktree was
created (`sampleapp-spike1`): the first item is the build folder and the second the worktree that
existed at the start, both `new: false`, the third is new.

```json
{
  "schema": 1,
  "project": {
    "root": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp",
    "name": "sampleapp",
    "isRepo": true,
    "origin": "https://github.com/acme/sampleapp.git",
    "worktrees": [
      "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-feature",
      "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1"
    ]
  },
  "items": [
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp\\target",
      "category": "builds",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\\target",
      "bytes": 3145772,
      "level": "recommended",
      "blocked": [],
      "why": {
        "type": "catalog-folder",
        "labelEn": "Rust build"
      },
      "target": {
        "type": "dir",
        "kind": "rust-target"
      },
      "new": false,
      "firstSeenMs": 1794601865194,
      "measuredAtMs": 1794601871137
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-feature",
      "category": "worktrees",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-feature",
      "bytes": 172,
      "level": "recommended",
      "blocked": [],
      "why": {
        "type": "linked-worktree",
        "branch": "feature",
        "head": "f0aae281f60de1b1a1e20a5fe2f80878b731cd2e"
      },
      "target": {
        "type": "worktree",
        "repo": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp"
      },
      "new": false,
      "firstSeenMs": 1794601865194,
      "measuredAtMs": 1794601865194
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-spike1",
      "category": "worktrees",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1",
      "bytes": 171,
      "level": "recommended",
      "blocked": [],
      "why": {
        "type": "linked-worktree",
        "branch": "spike1",
        "head": "f0aae281f60de1b1a1e20a5fe2f80878b731cd2e"
      },
      "target": {
        "type": "worktree",
        "repo": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp"
      },
      "new": true,
      "firstSeenMs": 1794601871137,
      "measuredAtMs": 1794601871137
    }
  ],
  "notes": [
    {
      "type": "walk-may-miss-deep-folders"
    }
  ],
  "branches": [
    {
      "name": "feature",
      "merged": true,
      "worktreeMissing": false
    },
    {
      "name": "spike1",
      "merged": true,
      "worktreeMissing": false
    }
  ],
  "search": "bounded-walk",
  "scannedAtMs": 1794601867932,
  "elapsedMs": 3204,
  "totals": {
    "byCategory": {
      "builds": 3145772,
      "worktrees": 343
    },
    "items": 3,
    "recommendedBytes": 3146115
  },
  "growthBytes": 171,
  "attention": [],
  "stale": false,
  "cleanCommand": "xiox clean \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\" --include \"C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-spike1\"",
  "cleanCommandTruncated": false,
  "startedAtMs": 1794601865194,
  "checkedAtMs": 1794601871137
}
```

The `report` event is the same object with `"event"` and `"time"` after `"schema"`.

| Field | Meaning |
|---|---|
| `items` | Every item of the latest check, with `new`, `firstSeenMs`, `measuredAtMs`. |
| `totals` | `byCategory`: measured bytes per category (a category with nothing measured is absent; the categories overlap: a build folder inside a worktree is in both, so do not add them up); `items`: how many; `recommendedBytes`: measured bytes of the items `xiox clean` would take as recommended. An item inside another item is counted once. |
| `growthBytes` | Growth since the start, in measured bytes: for a new item all its bytes, for an item of the snapshot what it grew by (shrinking counts 0). An item that was never measured counts 0. An item inside another that already counts is not counted twice. |
| `attention` | The thresholds that are exceeded right now: `growth`, `worktrees`. |
| `stale` | `true` when the fresh check failed and this is the last good picture. |
| `cleanCommand`, `cleanCommandTruncated` | See below. |
| `startedAtMs`, `checkedAtMs` | When the snapshot was taken, and when the latest check ended. |

**`cleanCommand`** is a ready-made `xiox clean` command, or `null`. It is built from
`--include <path>` entries, **one for each new item that `xiox clean` would accept as
recommended** (level `recommended`, not blocked, with a folder, not directly under a drive
root, not holding a blocked item), in the order of the report:

```text
xiox clean "C:\Users\alice\AppData\Local\Temp\w\sampleapp" --include "C:\Users\alice\AppData\Local\Temp\w\sampleapp-spike1"
```

It uses `--include` and not `--worktrees` or `--builds` because the category flags would take
**every** recommended item of the category, including the ones that were already there when the
observer started and that the observer never vouched for. The command names only what appeared
since the start. At most **25** items go into one command; when there are more,
`cleanCommandTruncated` is `true` (run the command, then ask for a new report). Copies and
name-matched folders are `caution` items, so they are never in it: they need your own decision
and `--include`. Archived sessions have no folder and are never in it.

The observer never runs this command and a program should not either without asking the
person: `xiox clean` shows a summary, asks for confirmation (unless `--yes`), and re-analyses
the project right before removing anything, skipping what no longer qualifies. See the Safety
section of [`xiox-cli.md`](xiox-cli.md#safety).

## Thresholds and re-arming

Two thresholds raise an `attention` event; both are compared at every check:

| `kind` | Exceeded when | `value` |
|---|---|---|
| `growth` | the growth since the start is **more than** `--grow-threshold` bytes | the growth, in bytes |
| `worktrees` | the number of new worktrees is **more than** `--worktree-threshold` (the default `3` fires at the fourth) | the number of new worktrees |

An `attention` event is sent **once per crossing**. It is armed again only when the value is back
at or below the threshold (growth shrinks, worktrees are removed); the next crossing sends a new
event. While it stays above, no event is repeated; the `attention` array of a report always says
which thresholds are exceeded at that moment. Both events carry the same `cleanCommand` as a
report would. The observer decides nothing from them: whoever reads the event decides whether
to show it to a person, pass it to an agent, or ignore it.

Growth is computed from **measured bytes only**. A new item whose size is not known yet counts 0
and is measured again at every check, so it is counted as soon as it can be measured.

## MCP mode

`xiox observe <project> --mcp` speaks the Model Context Protocol on stdio, so that an agent
harness can start it as one of the agent's MCP servers and the agent can ask it directly. The
protocol is written by hand and is small: `initialize`, `ping`, `tools/list`, `tools/call`
(protocol versions `2025-06-18`, `2025-03-26` and `2024-11-05` are accepted; when the client
asks for another one, the server answers `2025-06-18`). One JSON-RPC message per line in each direction.

- **One tool, `project_leftovers`, read-only.** It takes no arguments and is marked read-only
  (`readOnlyHint: true`, `destructiveHint: false`). There is no tool that removes anything.
- **No background timer.** In MCP mode nothing runs between calls: a check is made when the tool
  is called, so `--interval` does not apply. The server ends when stdin closes.
- **`instructions`.** The `initialize` reply tells the agent to call `project_leftovers` before
  it declares its work finished and again after it creates a worktree or a copy of the
  repository, that `new: true` means "appeared since this server started", and to show the
  `cleanCommand` to the person instead of running it.
- **The snapshot is taken at once, when the process starts**, on its own thread, so that
  `initialize`, `ping` and `tools/list` are answered immediately even when the first analysis
  is slow. "New" therefore means new since the process started, not since the first call.

A real session. The client writes these lines (one per line, nothing else is needed):

```text
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"demo","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_leftovers","arguments":{}}}
```

The server answers the requests with an `id` (a notification gets no answer), one line each.
The replies, formatted (in the real output each is one line):

The reply to `initialize`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2025-06-18",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "xiox",
      "version": "0.1.0"
    },
    "instructions": "Xiox watches one project on this PC for the leftovers that agent work creates: build folders, git worktrees, copies of the repository, folders named like the project, logs and archived agent sessions. Call the project_leftovers tool before you declare your work finished, and again after you create a git worktree or a copy of the repository. Items with new: true appeared since this server started. The tool only reports and never removes anything; to clean up, show the cleanCommand of the report to the person instead of running it yourself."
  }
}
```

The reply to `tools/list`:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "project_leftovers",
        "description": "Lists what this project has left on this PC since this server started: build folders, linked worktrees, copies of the repository, folders named like the project, log files and archived agent sessions, each with its path, size, level and reason, and whether it is new since the start. Also returns totals, the growth in bytes since the start, attention (thresholds exceeded), stale (true when the fresh check failed and the last report is returned) and cleanCommand, a ready xiox clean command for the new recommended items. Runs a fresh check on every call. Read-only: it only reports and never removes anything.",
        "inputSchema": {
          "type": "object",
          "properties": {},
          "additionalProperties": false
        },
        "annotations": {
          "readOnlyHint": true,
          "destructiveHint": false,
          "idempotentHint": true,
          "openWorldHint": false
        }
      }
    ]
  }
}
```

The reply to `tools/call` carries the [report](#the-report-and-cleancommand) twice, as the
text of the content (one line of JSON, for clients that only show text) and as
`structuredContent` (an object); here both are replaced by a placeholder, the report is the one
shown [above](#the-report-and-cleancommand):

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "<the report below, as one line of JSON>"
      }
    ],
    "structuredContent": "<the same report as an object>",
    "isError": false
  }
}
```

**What a call does.** Every `tools/call` runs a fresh check first and then answers.

- If the check fails, the answer is the last good report with `"stale": true` (the cause is on
  stderr); it is not an error result.
- If the first analysis is still running, the call waits for it for up to **30 seconds**. After
  that it answers a tool result with `isError: true` and the text
  `The first analysis of the project is still running. Call project_leftovers again in a moment.`
  The agent can call again; the snapshot is not lost.
- If the first analysis **failed**, the server stays up and reports it on stderr. Each later
  call tries the first analysis again, and while that fails the result is `isError: true` with
  `The project could not be analysed: <reason>. No report is available yet.` **Caveat:** the
  snapshot is then taken by the first call that succeeds, so everything that existed before
  that call is `new: false`, including what the agent created in the meantime. `startedAtMs`
  in the report is when that snapshot was taken: compare it with the moment the work began.
- A closed stdin ends the server (exit code 0); a stdout that cannot be written ends it with 1.

**Protocol errors** follow JSON-RPC: `-32700` for a line that is not JSON or is longer than
64 KiB (`id: null`), `-32600` for JSON that is not a request, `-32601` for an unknown method,
`-32602` for an unknown tool or invalid parameters. Notifications are never answered. The
server keeps running after any of them.

## Configuring an MCP client

An MCP client starts the server with a command and its arguments, over stdio. The entry is the
same everywhere:

```json
{"command": "xiox", "args": ["observe", "C:\\path\\to\\project", "--mcp"]}
```

Name the project explicitly: the folder the client uses as the working directory of the
server may not be the project. In JSON the backslashes of a Windows path are doubled. If `xiox`
is not on the `PATH` of the client, give the full path of `xiox.exe` as `command`. Add one entry
per project, with its own name (the observer is a separate process per project).

**Claude Code** reads servers from a `.mcp.json` file in a project folder (and from the
`mcpServers` object of `.claude.json` in your home folder):

```json
{
  "mcpServers": {
    "xiox-observe": {
      "type": "stdio",
      "command": "xiox",
      "args": ["observe", "C:\\src\\my-project", "--mcp"]
    }
  }
}
```

**Codex** reads servers from `config.toml` in its configuration folder (`.codex` in your home
folder), one `[mcp_servers.<name>]` table each:

```toml
[mcp_servers.xiox-observe]
command = "xiox"
args = ["observe", 'C:\src\my-project', "--mcp"]
```

For any other client (OpenCode, Pi, an editor, your own harness): use your client's way of
adding a stdio MCP server with this command. Check that it works by running the same command in
a terminal and typing an `initialize` request; the server must answer with one line of JSON
and nothing else on stdout.

## Limits

- **Where copies and name-matched folders are searched.** As in `xiox analyze` (see its
  [Limits](xiox-cli.md#limits)): not elevated, the project's parent folder, each fixed drive to
  three levels and your home folder to three levels; started from an elevated terminal, NTFS
  drives are searched by name through the file-system index. The observer never asks for
  elevation. The `search` field of a report says which mode was used (`bounded-walk`,
  `mft-index`, `parent-only`), and `notes` says when the walk may have missed deeper folders
  (`walk-may-miss-deep-folders`). `--no-drives` limits the search to the project's parent folder.
- **Sizes are measured with caps.** Each folder is measured by walking it, for at most
  200,000 entries and 10 seconds. Over either, or when the folder cannot be read, its `bytes`
  is `null` and it counts 0 in the growth. Links and junctions are not followed. A subfolder that
  cannot be read is skipped, so the size can be under-counted. The measurer is the same at the
  start and at every check, so growth is never the difference between two different measurers.
- **Only new items and build folders are measured again.** An item of the first snapshot that
  is not a build folder keeps the size it had at the start (its `measuredAtMs` says when); only
  the `builds` items of the snapshot are measured at every check, because their growth is what is
  watched. A folder that could not be measured at the start takes its first later measurement as
  its starting size.
- **Checks, not events.** There is no real-time watching of files. A worktree created and
  removed between two checks is never seen. Between checks nothing happens.
- **A change of category is not an event.** An item is identified by its path. If the same folder
  is listed in another category by a later check, no `changed` event is written: the report
  always shows the current category. (Seen in a real run: a worktree that was created while a
  check was running was listed as a copy by that check and as a worktree by the next.)
- **No attribution.** The observer says what appeared since it started. It does not say which
  agent, session or harness created it, and it does not read sessions for that. Two agents
  working in the same project are seen together.
- **One analysis at a time.** A `report` line or a tool call that arrives while a check is
  running is answered when that check ends.
- **Large reports.** The report lists every item; the MCP result carries it twice (text and
  structured). A project with many items gives a large reply.
- **A caller that vanishes without closing the pipes** is noticed at the next write to stdout or
  at the end of stdin (Windows closes both when a process dies, so the usual orphan case is
  covered).
- **The interval** is between 60 seconds and 30 days.

## Troubleshooting

**Nothing appears on stdout.** In default mode the first line (`started`) is written when the
first analysis ends, which includes measuring every item: wait for it (the stderr line
`Observing ...` comes just after it). After that, a line appears only when something changes or
a threshold is crossed; send `report` to get the full picture at once. In MCP mode the server
writes only replies: it is silent until the client sends a request.

**It exits immediately, with code 0.** Its stdin was closed: it ends when the caller's end of
the pipe closes. Make sure the program that starts it keeps the stdin pipe open for as long
as it wants the observer to live (for example, started with input redirected from `NUL` it
exits after printing `started`). Closing stdin is also the way to stop it.

**`error: path not found: <path>` (exit code 1).** The project folder does not exist or is not
a folder. Stdout is empty.

**An error about usage (exit code 2).** The message says which flag is wrong, for example
`--interval must be at least 60 seconds, not 30`, `--interval must be at most 2592000 seconds
(30 days), not 3000000`, `--worktree-threshold needs a whole number of at least 1, not "0"`,
`--grow-threshold: invalid size "0" (examples: 500MB, 2GB, 1.5GiB, 1073741824)`, or
`--json is not valid for observe (it always prints JSON lines)`. The usage text follows.

**`error` events with `check failed: ...`.** A check could not analyse the project (for example
the folder was moved or deleted). The observer stays up and tries again; reports are
marked `stale: true` until a check succeeds. The same text goes to stderr.

**The first analysis fails.** Default mode: the reason is on stderr, stdout is empty, exit code
1. MCP mode: the server stays up and each call tries again (see
[MCP mode](#mcp-mode), including the caveat about `new`).

**A parser breaks on stdout.** It should not: stdout carries only JSON lines. Everything meant
for a person (the `Observing ...` line, `error: ...` lines, the cause of a failed check) goes
to stderr. If you capture both streams into one, separate them. A panic does not write to
stdout either: it prints `error: xiox observe stopped on an internal error: ...` on stderr and
ends the process with a non-zero code.

**MCP: the client says the tool is still running or times out at start.** `initialize` is
answered at once; only `tools/call` waits (at most 30 seconds) for the first analysis, and then
answers `The first analysis of the project is still running. Call project_leftovers again in a
moment.`: call it again.

**`git was not found` and similar `notes`.** The observer uses the same analysis as
`xiox analyze`, so the same notes appear in the report and the same remedies apply: see
[Troubleshooting in `xiox-cli.md`](xiox-cli.md#troubleshooting).
