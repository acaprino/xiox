# xiox: the project cleanup command line

`xiox` finds everything on a Windows PC that belongs to **one project** (build folders, git
worktrees, copies of the repository, log files, archived coding-agent sessions) and removes
what you name. `xiox analyze` only looks and reports sizes. `xiox clean` removes only what its
flags name, after a summary and a confirmation.

It is a single executable. It never asks for administrator rights (no UAC prompt, ever), it
changes nothing on its own, and it is written to be used by people as well as by scripts and AI
agents (`--json`, stable exit codes).

- [Install](#install)
- [Quick start](#quick-start)
- [What belongs to a project](#what-belongs-to-a-project)
- [Levels and blocks](#levels-and-blocks)
- [`analyze` reference](#analyze-reference)
- [`clean` reference](#clean-reference)
- [For scripts and agents](#for-scripts-and-agents)
- [Safety](#safety)
- [Limits](#limits)
- [Troubleshooting](#troubleshooting)

## Install

There is no installer. Build the release executable from the workspace root (the folder that holds the top-level
`Cargo.toml`). You need a Rust toolchain (`cargo`):

```powershell
cargo build --release --locked -p xiox-cli
```

The result is `target\release\xiox.exe` (about 3.2 MB, one file). Copy it anywhere, or put its
folder on your `PATH`. `xiox --version` prints the version, `xiox --help` the full usage.

Requirements: Windows, and `git` on the `PATH` for everything that involves git (worktrees,
copies, log files, branches). Without git only build folders, name matches and sessions are
looked up (see [Troubleshooting](#troubleshooting)).

**Elevation.** `xiox` never asks to be elevated (its manifest is `asInvoker`). If you start the terminal yourself with
"Run as administrator", the only change is how copies and name-matched folders are searched:
the NTFS file-system index is read, which is faster and not limited to three folder levels
(see [Limits](#limits)). What can be removed does not change.

## Quick start

```powershell
cd C:\src\my-project
xiox analyze                       # look: nothing is removed
xiox clean --builds --worktrees    # remove the recommended build folders and worktrees
```

`<project>` is any folder of the project; without it the current folder is used. If the folder
is inside a git repository, or is one of its linked worktrees, the project is the repository's
main working tree. Otherwise the folder itself is the project.

The examples in this guide come from real runs on a throwaway project whose files were created
only for this purpose (the user name in the paths is replaced by `alice`):

| Folder | What it is |
|---|---|
| `...\w\sampleapp` | the project: a git repository with `Cargo.toml`, a `target` folder (11 MB) and an ignored 2 MiB `debug.log`; its `origin` is `https://github.com/acme/sampleapp.git` |
| `...\w\sampleapp-feature` | a clean linked worktree (branch `feature`) |
| `...\w\sampleapp-wip` | a linked worktree (branch `wip`) with an uncommitted file |
| `...\w\sampleapp-clone` | a clone of the project with nothing of its own |
| `...\w\sampleapp-experiment` | a clone with one commit the project does not have |
| `...\w\sampleapp-bench` | a plain folder called like the project |

`...\w` stands for `C:\Users\alice\AppData\Local\Temp\w`. Archived agent sessions are not part
of the examples.

```text
> xiox analyze C:\Users\alice\AppData\Local\Temp\w\sampleapp
Project  sampleapp   C:\Users\alice\AppData\Local\Temp\w\sampleapp   (git, origin github.com/acme/sampleapp)
Search   fixed drives to 3 levels (deeper folders may be missed; an elevated terminal uses the MFT index)

BUILDS                                                                       11 MB
  recommended      11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target      Rust build

WORKTREES                                                                     2 MB
  blocked           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-wip         uncommitted changes
  recommended       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature     clean, on branch feature

COPIES                                                                        2 MB
  blocked           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-experiment  has commits or changes the project does not have
  caution           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-clone       copy of the repository: same origin and first commit, no own work

NAMED                                                                         1 MB
  caution           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-bench       matched by name only

LOGS                                                                          2 MB
  recommended       2 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\debug.log   log file, ignored by git

BRANCHES (not removed)
  feature   merged
  old-idea  merged
  wip       merged

Total 18 MB · recommended 14 MB
Clean with: xiox clean "C:\Users\alice\AppData\Local\Temp\w\sampleapp" --builds --worktrees --logs
```

(Run without a folder argument inside `sampleapp`, `xiox analyze` prints exactly the same.)
Progress and errors go to stderr (`Analyzing <folder> ...`); the listing goes to stdout.

Read it as a shopping list. The last line is a ready-made command for the recommended part:
`xiox clean "<project>" --builds --worktrees --logs`. Everything else is listed so that you
can see it, but needs a deliberate choice, or cannot be removed at all.

## What belongs to a project

Each item in the listing is in one category. The category decides how it is found and how it
can be selected.

**`builds`: build and dependency folders.** Folders that tools regenerate: `target` (Rust),
`node_modules`, `.venv`, `__pycache__`, `.gradle`, `obj`, `dist`, `build` and the others of the
folder catalog of the desktop app. They are looked for inside the project folder and inside each of its
linked worktrees. Cache and build-output folders are `recommended`; dependency folders
(`node_modules`, `.venv`) are `optional`; a folder is `caution` when git tracks something in
it, when it is not confirmed to be generated, or when its name is generic (`dist`, `build`) and
git does not ignore it. Catalog folders that may hold your own data (`.hypothesis`, `wandb`,
`mlruns`, `runs`, `checkpoint-*`, Unity's `UserSettings`, Unreal's `Saved` and `Binaries`) are
always `caution`, labelled `Personal data (<name>)`. Selected with `--builds` (the recommended
ones, so not `node_modules`, and never the personal-data ones) or `--include <path>`.

**`worktrees`: linked git worktrees.** The linked worktrees git lists for the repository
(`git worktree list`), wherever they are on disk. A worktree is `recommended` when it is clean
and its HEAD commit is also contained in a remote branch or in a local branch other than the
one checked out there, so no commit can be lost. Otherwise it is blocked (see the table
below); so it is when an agent session is open in it. A registered worktree whose folder no longer exists is not an item: it appears as a
`note:` ("git still lists a worktree whose folder is gone"), and `git worktree prune` clears
it. Selected with `--worktrees` or `--include <path>`.

**`copies`: other copies of the repository.** Other git repositories on the PC with the same
identity as the project: they share at least one root (first) commit with it, or their
`origin` URL is the same after normalisation (case and a trailing `.git` are ignored, and
`git@host:a/b` equals `https://host/a/b`). They are found by identity, whatever the folder is
called. A fork has the same identity too, so a copy is **always `caution`**. The listing says
whether the copy has work of its own:

- `no own work`: it is clean, and its HEAD, every local branch, every tag and every stash
  entry is a commit that some branch, tag or remote branch of the project's repository
  reaches (not merely a commit that the project's object store still holds, for example after
  a deleted branch);
- `has own work`, shown as blocked (`unsaved-work`): it has uncommitted changes, or a commit
  (on a branch, a tag, a stash or HEAD) that the project's repository does not reach.
  **It cannot be removed, not even with `--include`.**
- `own work not checked`, shown as blocked (`unsaved-work-unknown`): git could not tell (it
  cannot read the repository, or gave no answer). **It cannot be removed, not even with
  `--include`**: when in doubt, `xiox` refuses.

A copy with an agent session open in it (a live session whose working folder is inside the
copy) is blocked too (`live-session`). A folder inside the project, inside another git
repository, or containing the project is never listed. Copies can only be removed with
`--include <path>`, one folder at a time.

**`named`: folders named like the project.** Folders outside the project whose name is the
project name followed by `-`, `_`, `.` or a space and at least one more character
(case-insensitive): `sampleapp-bench`, `sampleapp_tmp`. Nothing but the name links them to the
project, so they are **always `caution`** ("matched by name only"). They are never taken by a
category flag, only by `--include <path>`.

A named folder is blocked when an agent session is open inside it (`live-session`), and when it
is, or holds, a git repository that git cannot read (`unsaved-work-unknown`: nothing can show
that it has no unsaved work), or when it is itself a repository with uncommitted changes or
commits that no remote branch has (`unsaved-work`). A named folder that holds a git repository is
also checked again when it is removed: see [Safety](#safety).

**`scratch`: folders that agent sessions made.** Folders outside the project that the session
history of your coding agents (`claude`, `codex`, `opencode`, `pi`) shows an agent made, or wrote
files into, while working on the project: a run folder, a benchmark output, a scratch copy. A
folder is proposed only when the history holds evidence at least at the level `written` (a file
was written in it, or the folder itself was made, copied, moved or cloned); a folder that was
only read, listed or entered is never proposed. The listing says what the sessions did (`made by
a session`, `files written here by a session`), how many distinct sessions touched the folder,
the date of the last one (`no date` when no session gave one), `from a script` when the evidence
comes from a script that a session ran, and the **risks**: `no git`, `weak evidence`,
`partial history` and `recent activity` (a session touched it in the last 24 hours). The category
makes no promise that the content is useless: a session may have put something of yours there.
**`xiox` cannot remove a scratch folder yet**: the category flags do not take it and `--include`
refuses it. The listing is for review. Copies and named folders that the history also shows get
`; seen in <n> sessions` added to their description.

**`logs`: log and temporary files.** Files (not folders) ending in `.log`, `.tmp`, `.bak` or
`.dmp`, at least 1 MiB, inside the project and its worktrees (not inside a build folder that is
already listed), that git does not track. `recommended` when git ignores them, `caution` when
they are merely untracked. Needs a git repository. Links are never listed. Selected with
`--logs` (the recommended ones) or `--include <path>`.

**`sessions`: archived agent sessions.** Archived sessions of coding agents (`claude`,
`codex`, `opencode`, `pi`) whose working folder is the project or one of its worktrees (also a
worktree whose folder is gone). They have no path, so they are shown as `claude session <id>`
with the session title. They are `optional`. `--sessions` takes every one that is not blocked;
they cannot be named with `--include`.

Besides the items, the listing shows **branches**: local branches already merged into the
current branch of the main working tree, or whose worktree folder is gone. They are
information only: `xiox` never deletes a branch. Each line says `merged` or `not merged`,
and adds `its worktree folder is gone` when that is the case (so a line can read `not merged,
its worktree folder is gone`). Branches that are checked out in a linked worktree also appear
(`feature` and `wip` above).

## Levels and blocks

Every item has a **level**: how strongly the tool recommends removing it.

| Level | Meaning | Taken by |
|---|---|---|
| `recommended` | Regenerates, or carries nothing that would be lost. | its category flag, or `--include` |
| `optional` | Can be lost, but you may still want it (dependency folders, sessions). | `--sessions` for sessions; `--include` for folders |
| `caution` | Needs your judgement: only the project's name or identity links it, or git tracks something in it. | `--include` only |
| `blocked` | Cannot be removed. Shown in the level column in place of the level. | nothing, not even `--include` |

An item is blocked when it has at least one **block**. In the listing the reason replaces the
description; in JSON the `blocked` array holds the value of the first column.

| JSON value | Listing text | Applies to |
|---|---|---|
| `live-session` | an agent session is open | builds, worktrees, copies, named folders |
| `dirty-git` | uncommitted changes (also when git cannot tell) | worktrees |
| `unpushed-commits` | commits not kept anywhere else (also when git cannot tell) | worktrees |
| `unreachable` | could not be checked | builds |
| `session-locked` | session locked by its harness | sessions |
| `unsaved-work` | has commits or changes the project does not have | copies, named folders that are repositories |
| `unsaved-work-unknown` | could not be checked for unsaved work | copies, named folders that are or hold a repository |
| `unknown` | unknown reason | reserved: the current version never produces it |

**Top-level folders.** A folder directly under a drive or share root (such as `D:\sampleapp-old`)
is listed normally, but its description ends with `; cannot be removed: top-level folder`. The
safety guard of the cleaner refuses such folders, so `clean` skips them (the summary warns
about it before the question, and the run exits with 3). Remove them by hand if you are sure.
The `--json` output does not carry this marker.

## `analyze` reference

```text
xiox analyze [<project>] [--json] [--no-drives] [--no-history]
```

| Flag | Meaning |
|---|---|
| `<project>` | Any folder of the project. Default: the current folder. |
| `--json` | Print one JSON object on stdout instead of the listing (see [JSON](#json)). |
| `--no-drives` | Search copies and name-matched folders only next to the project (the immediate children of the project's parent folder), not on every fixed drive. Much faster on a PC with big drives. |
| `--no-history` | Do not read the session history: no `scratch` items and no `History` line. The archived sessions of the project are listed all the same. |
| `-h`, `--help` | Show the usage. |
| `-V`, `--version` | Show the version. |

`analyze` removes nothing and writes nothing, apart from the small cache of the session
listing that the desktop app already keeps. It exits 0 whenever the analysis worked, whatever it
found.

Reading the output (see the example above):

- `Project <name>   <root>   (<kind>)`: the project that was resolved. `<kind>` is
  `git, origin <host/path>`, `git, no origin` or `not a git repository`.
- `Search ...`: how copies and name-matched folders were searched: `fixed drives to 3 levels
  (...)` (not elevated), `all fixed drives, MFT index` (elevated terminal, NTFS drives) or
  `project's parent only` (`--no-drives`).
- `History  <n> sessions read of <m> related`: only when the session history was asked for and
  could be listed. `<m>` is the number of listed sessions that look related to the project. It
  continues with `; <f> could not be read` when some sessions failed (a timeout, an output over
  the cap, a store without a reader) and with `; stopped at the session limit`, `tool call limit`
  or `time limit` for every cap that was reached. The reading is bounded: at most **250
  sessions**, **250,000 tool calls** and **240 seconds** in all, and **120 seconds** for one
  session. A session is read, its folders are taken and it is released; one that fails is
  counted and the others are still read.
- One block per non-empty category, with its total size on the title line (an item inside
  another one is counted once). Each line is `level`, `size`, `path`, `description`. The
  description says why the item belongs to the project, or gives the block reason.
- `BRANCHES (not removed)`: see above.
- `note:` lines, if any: the project is not a git repository; git was not found; the project
  name is too short to match folders by name; some drives were searched only to 3 levels;
  a registered worktree is gone; the analysis reported warnings and the listing may be
  incomplete; the session listing of this PC could not be read (a live agent session may be
  missed; `clean` then leaves worktrees, copies and named folders alone); git could not list the
  files of the project or of a worktree (its log files are not in the listing); the folder is a
  drive root, your user profile or holds several projects (see `clean`); the session history was
  read only in part (the `History` line says what stopped it, so folders made by agents may be
  missing); the session history could not be listed at all (no folder was taken from it).
- `Total <all> · recommended <n>`: the sum of everything listed, and of the recommended,
  unblocked, removable items. Then the `Clean with:` command. If nothing is found the output
  ends with `Nothing found for this project.`

With `--no-drives` the same project gives the same listing (everything is next to the project)
and only the search line changes:

```text
> xiox analyze C:\Users\alice\AppData\Local\Temp\w\sampleapp --no-drives
Project  sampleapp   C:\Users\alice\AppData\Local\Temp\w\sampleapp   (git, origin github.com/acme/sampleapp)
Search   project's parent only
...
```

## `clean` reference

```text
xiox clean [<project>] [--builds] [--worktrees] [--logs] [--sessions]
           [--include <path>]... [--permanent] [--yes] [--dry-run] [--json] [--no-drives]
```

`clean` first runs the same analysis as `analyze`, then removes **only what the flags name**.
Called without any category flag and without `--include` it removes nothing: it prints the
listing and `Nothing selected. Name what to clean: --builds --worktrees --logs --sessions
--include <path>` (exit 0).

| Flag | Meaning |
|---|---|
| `--builds` | The `recommended` build folders that are not blocked. |
| `--worktrees` | The `recommended` linked worktrees. |
| `--logs` | The `recommended` log files. |
| `--sessions` | Every archived session of the project that is not blocked. |
| `--include <path>` | One more item of the listing, of any level except blocked ones, by its path. Repeatable; `--include=<path>` works too. The only way to take copies, name-matched folders and `optional` or `caution` folders and files. |
| `--permanent` | Delete for good instead of moving to the Recycle Bin. |
| `--yes` | Do not ask for confirmation. |
| `--dry-run` | Do every check, remove nothing, say what would be freed. |
| `--json` | One JSON object on stdout, see [JSON](#json). |
| `--no-drives` | As for `analyze`. |

**One project per run.** The category flags (`--builds`, `--worktrees`, `--logs`, `--sessions`)
are refused, with exit code 2, when the folder is a drive root, your user profile or a folder
above it, or is not a git repository and holds more than one repository (or more than one
project) below it. Otherwise they would take the builds and the archived sessions of every
project under it. `analyze` still works and prints a `note:`; `--include <path>` of one listed
item still works:

```text
> xiox clean D:\Projects --builds --yes
error: refusing to clean everything under D:\Projects: run it on one project
```

There are no `--copies` or `--named` flags, on purpose: copies and name-matched folders are
never taken as a group. Name each folder with `--include`:

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --copies
error: --copies/--named do not exist: name each folder with --include <path>

Usage:
  xiox analyze [<project>] [--json] [--no-drives]
  xiox clean   [<project>] [--builds] [--worktrees] [--logs] [--sessions]
               [--include <path>]... [--permanent] [--yes] [--dry-run] [--json] [--no-drives]
  xiox observe [<project>] [--interval <seconds>] [--grow-threshold <size>]
               [--worktree-threshold <n>] [--no-drives] [--mcp]
  xiox --help | --version
```

(exit code 2)

**What `--include` accepts.** A path that appears in the listing (absolute, or relative to the
current folder; case does not matter). A path the analysis did not produce is an error, not a
deletion. A blocked item is refused with its reason, and so is a folder that holds a blocked item
(removing the folder would remove it). When any part of a request is refused, nothing at all
is removed (exit code 2):

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --include C:\Users\alice\AppData\Local\Temp\w\sampleapp-experiment --yes
error: cannot be removed (has commits or changes the project does not have): C:\Users\alice\AppData\Local\Temp\w\sampleapp-experiment

> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --include C:\Users\alice\AppData\Local\Temp\w\sampleapp-wip --yes
error: cannot be removed (uncommitted changes): C:\Users\alice\AppData\Local\Temp\w\sampleapp-wip

> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --include C:\Users\alice\AppData\Local\Temp\w\nothing --yes
error: not found in the analysis of this project: C:\Users\alice\AppData\Local\Temp\w\nothing

> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --include C:\Users\alice\AppData\Local\Temp\w\sampleapp-old --yes
error: cannot be removed: it holds C:\Users\alice\AppData\Local\Temp\w\sampleapp-old\copy (has commits or changes the project does not have): C:\Users\alice\AppData\Local\Temp\w\sampleapp-old
```

### Summary, confirmation and `--yes`

Before anything is removed `xiox` prints a summary: the chosen items by category, the total,
where they go (`to the Recycle Bin` or `permanently`), a line for every folder named with
`--include` that deserves a second look, a warning for top-level folders that will be skipped,
and a warning for a Recycle Bin that is too small. Then it asks a question of this form:

```text
Remove 2 items (12 MB)? [y/N]
```

When worktrees are chosen without `--permanent` the question also says that they go for good:
`Remove 2 items (12 MB; worktrees are removed by git for good, not to the Recycle Bin)? [y/N]`.

Only `y` or `yes` (any case) confirms. Anything else, an empty line or the end of the input is
a no: `Cancelled.` is printed, nothing is removed, and the exit code is 0.

When stdin is redirected or is not a console (a pipe, a file, a scheduled task, an agent) there
is nobody to ask, so `clean` refuses instead of guessing. The check is only on stdin: a script
run from an interactive console still has a terminal and **would prompt**, with the question on
stderr (invisible after `2>$null`). A script must therefore **always pass `--yes` or
`--dry-run`**. With stdin redirected and no `--yes`, the same command as above gives:

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --builds --worktrees
Analyzing C:\Users\alice\AppData\Local\Temp\w\sampleapp ...
To be removed:

BUILDS                                                       11 MB
      11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target   Rust build

WORKTREES                                                     1 MB
       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature  clean, on branch feature

Total 12 MB in 2 items, to the Recycle Bin.
Worktrees are removed by git and do not go to the Recycle Bin.
error: not a terminal: pass --yes to remove without asking
```

(exit code 2)

`--yes` skips the question. Use it in scripts, after you have looked at the listing or at a
`--dry-run`.

**Where the text goes.** When `clean` is going to ask (no `--yes`, no `--dry-run`, no
`--json`), the summary, the question and `Cancelled.` go to **stderr**, so that a redirected
stdout can never hide what is about to be removed. Every other text (the listing, `Nothing to
clean.`, the summary of a `--yes` or `--dry-run` run, the report) goes to stdout, or to stderr
with `--json`. `Analyzing ...` and errors always go to stderr.

### Dry run

`--dry-run` does everything except removing, including the re-check described under
[Safety](#safety), and tells you what a real run would do. A dry run always exits 0, even when
some items come out as `skipped`: read the `skipped` lines (or `outcome.items` with `--json`)
before you run it for real. It uses the real mode, so a dry run
without `--permanent` says `to the Recycle Bin`:

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --builds --worktrees --dry-run
To be removed:

BUILDS                                                       11 MB
      11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target   Rust build

WORKTREES                                                     1 MB
       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature  clean, on branch feature

Total 12 MB in 2 items, to the Recycle Bin.
Worktrees are removed by git and do not go to the Recycle Bin.
Dry run: nothing was removed.
  would remove      11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target
  would remove       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature
Would free 12 MB
```

### Recycle Bin or `--permanent`

By default folders and files go to the Recycle Bin: you can restore them, but **the disk space
is freed only when the bin is emptied**. The report says so, in the form
`Freed 13 MB (13 MB in the Recycle Bin: freed when it is emptied)`. Before removing anything
`xiox` checks that the bin of each drive has room for what will be moved there. If it does not,
it prints `Recycle Bin on C: is too small (3 MB to move, 1 MB free). Use --permanent, or empty
the bin.` for that drive, then `error: not enough room in the Recycle Bin: nothing was
removed`, and exits with 1 (a dry run exits with 1 too, so a script finds out in advance).

`--permanent` deletes for good: no bin, no undo.

Sessions follow the same switch. Claude and Pi sessions are files (moved to the bin, or
deleted). Codex and OpenCode sessions are removed through the agent's own command, which must
be installed; in the default mode a copy of the session is put in the Recycle Bin first.

### Worktrees are always removed by git

A worktree is removed with `git worktree remove` (never `--force`) followed by a prune, in
both modes. It never goes to the Recycle Bin, even without `--permanent`: the summary says
`Worktrees are removed by git and do not go to the Recycle Bin.` If git refuses (for example
because the worktree changed after the analysis), the item is reported as `failed` and its
local changes stay where they are. The branch of a removed worktree is not deleted.

### A real run

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --builds --logs --yes --permanent
To be removed:

BUILDS                                                         11 MB
      11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target     Rust build

LOGS                                                            2 MB
       2 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\debug.log  log file, ignored by git

Total 13 MB in 2 items, permanently.
  removed           11 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\target
  removed            2 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp\debug.log
Freed 13 MB
```

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --worktrees --yes --permanent
To be removed:

WORKTREES                                                     1 MB
       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature  clean, on branch feature

Total 1 MB in 1 item, permanently.
  removed            1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-feature
Freed 1 MB
```

A name-matched folder (a copy is named the same way), taken with `--include`:

```text
> xiox clean C:\Users\alice\AppData\Local\Temp\w\sampleapp --include C:\Users\alice\AppData\Local\Temp\w\sampleapp-bench --yes --permanent
To be removed:

NAMED                                                       1 MB
       1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-bench  matched by name only

Named with --include (check each one before you go on):
  C:\Users\alice\AppData\Local\Temp\w\sampleapp-bench  matched by name only: nothing else shows it belongs to the project

Total 1 MB in 1 item, permanently.
  removed            1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-bench
Freed 1 MB
```

Afterwards the listing shows what is left: the blocked worktree and copy, the unblocked copy
and the branches (the branch `feature` of the removed worktree still exists, as `xiox` never
deletes branches):

```text
> xiox analyze C:\Users\alice\AppData\Local\Temp\w\sampleapp
Project  sampleapp   C:\Users\alice\AppData\Local\Temp\w\sampleapp   (git, origin github.com/acme/sampleapp)
Search   fixed drives to 3 levels (deeper folders may be missed; an elevated terminal uses the MFT index)

WORKTREES                                                                     1 MB
  blocked           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-wip         uncommitted changes

COPIES                                                                        2 MB
  blocked           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-experiment  has commits or changes the project does not have
  caution           1 MB  C:\Users\alice\AppData\Local\Temp\w\sampleapp-clone       copy of the repository: same origin and first commit, no own work

BRANCHES (not removed)
  feature   merged
  old-idea  merged
  wip       merged

Total 3 MB · recommended 0 B
```

The report has one line per item: `removed`, `skipped` (the item was left alone; the reason
follows in parentheses) or `failed`. If any item was skipped or failed, the last line says
`Not everything was removed: <n> items skipped or failed.` and the exit code is 3. An item that
sits inside another chosen item (a log inside a removed worktree) is reported as skipped with
`already removed with <path>`; that is not counted as a failure.

## For scripts and agents

### JSON

`--json` (on `analyze` and on `clean`) prints **one JSON object on one line to stdout** and
nothing else there; every message for a person goes to stderr. The object has `"schema": 1`;
field names are camelCase and enum values are kebab-case. Every text in it is English.
Within one `schema`, new fields and new values may appear; a changed meaning increments it.
A script should ignore the fields and the values it does not know. The session-history step only
adds: the category `scratch`, the `why` type `session-folder`, `items[].provenance`, the
top-level `history` and two notes; `schema` stays 1.

- `analyze --json` prints the **scan** (below).
- `clean --json` prints `{"schema":1,"scan":<the scan>,"outcome":<the outcome>|null}`: the scan
  as it was before removing, and what happened. `outcome` is `null` when nothing was run (no
  flag named anything, or nothing matched).
- `clean --json` never asks, so it **needs `--yes` to remove anything**; without it, it exits
  with 2 (`error: --json needs --yes to remove anything`). `--dry-run --json` works without
  `--yes`.
- On exit code 1 or 2 stdout is empty: the reason is on stderr.

A real `analyze --json` run (formatted and shortened: four items of seven are shown):

```jsonc
{
  "schema": 1,
  "project": {
    "root": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp",
    "name": "sampleapp",
    "isRepo": true,
    "origin": "https://github.com/acme/sampleapp.git",
    "worktrees": [
      "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-feature",
      "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-wip"
    ]
  },
  "items": [
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp\\target",
      "category": "builds",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\\target",
      "bytes": 11538432,
      "level": "recommended",
      "blocked": [],
      "why": {
        "type": "catalog-folder",
        "labelEn": "Rust build"
      },
      "target": {
        "type": "dir",
        "kind": "rust-target"
      }
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-wip",
      "category": "worktrees",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-wip",
      "bytes": 1048800,
      "level": "caution",
      "blocked": [
        "dirty-git"
      ],
      "why": {
        "type": "linked-worktree",
        "branch": "wip",
        "head": "66ff1f8a40c43fddf02199426b8771ab0c6096b6"
      },
      "target": {
        "type": "worktree",
        "repo": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp"
      }
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-experiment",
      "category": "copies",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-experiment",
      "bytes": 1098640,
      "level": "caution",
      "blocked": [
        "unsaved-work"
      ],
      "why": {
        "type": "same-repository",
        "sharedRootCommit": true,
        "sameOrigin": true,
        "hasOwnWork": true
      },
      "target": {
        "type": "dir",
        "kind": "project-folder"
      }
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp-bench",
      "category": "named",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp-bench",
      "bytes": 1048600,
      "level": "caution",
      "blocked": [],
      "why": {
        "type": "name-only"
      },
      "target": {
        "type": "dir",
        "kind": "project-folder"
      }
    },
    // ... three more items: sampleapp-feature, sampleapp-clone, debug.log
  ],
  "branches": [
    {
      "name": "feature",
      "merged": true,
      "worktreeMissing": false
    },
    // ... two more branches
  ],
  "notes": [
    {
      "type": "walk-may-miss-deep-folders"
    }
  ],
  "search": "bounded-walk",
  "scannedAtMs": 1794584999615,
  "elapsedMs": 2673
}
```

**The scan**

| Field | Type | Meaning |
|---|---|---|
| `schema` | number | Shape version, `1`. |
| `project.root` | string | Main working tree (or the folder itself). |
| `project.name` | string | Last component of `root`. |
| `project.isRepo` | bool | Is it a git repository. |
| `project.origin` | string or null | `remote.origin.url` without the credentials of its authority (`user:token@` is dropped). |
| `project.worktrees` | string[] | Existing linked worktrees. |
| `items[]` | array | The listing, ordered by category, then size descending. |
| `items[].key` | string | Stable id: the lower-cased normalized path; `session:<agent>:<id>` for sessions. |
| `items[].category` | string | `builds`, `worktrees`, `copies`, `named`, `scratch`, `logs`, `sessions`. |
| `items[].path` | string or null | Absolute path; `null` for sessions. |
| `items[].bytes` | number or null | Size in bytes (the command line always measures). |
| `items[].level` | string | `recommended`, `optional`, `caution`. A blocked worktree or copy is `caution`: check `blocked`. |
| `items[].blocked` | string[] | Block values; non-empty means it cannot be removed. |
| `items[].why` | object | `type` is `catalog-folder` (`labelEn`), `linked-worktree` (`branch`, `head`; either may be null), `same-repository` (`sharedRootCommit`, `sameOrigin`, `hasOwnWork`: true, false or null), `name-only`, `log-file` (`ignored`) `archived-session` (`agent`, `title`) or `session-folder` (`strength`: `used`, `written` or `created`; `fromScript`: bool; `risks`: `no-git`, `weak-evidence`, `partial-coverage`, `recent-activity`; `activity`: `active` or `unknown`, never "inactive": no source proves that a session is over). |
| `items[].provenance` | object or null | The session evidence behind a `scratch`, `copies` or `named` item; `null` otherwise. `strength`, `lastActivityMs` and `sessions[]`, the distinct sessions (by store, harness and id), most recent first. A session has `agent`, `harness`, `store`, `id`, `parent`, `lastActivityMs`, `strength`, `listedCwd` (its own listing names the folder as its working folder), `operations[]` (references into the session: `source`, `outcome`, `strength`, `entry`, `call`, `time`, `cwd`; never the text of a command), `omitted` (operations left out to keep the result small), `failed` (operations that failed) and `gaps` (shell calls that the parsers could not fully read). |
| `items[].target` | object | What removal means: `type` is `dir` (`kind`, e.g. `rust-target`, `node-modules`, `project-folder`), `file`, `worktree` (`repo`: where `git worktree remove` runs) or `session` (`agent`, `id`). |
| `branches[]` | array | `name`, `merged`, `worktreeMissing`. Information only. |
| `notes[]` | array | `type` is one of `not-a-repository`, `git-missing`, `name-too-short`, `walk-may-miss-deep-folders`, `prunable-worktree` (with `path`), `engine-warning`, `sessions-not-read`, `logs-not-checked`, `container-root`, `history-partial` (a cap was reached or a session could not be read: see `history`), `history-not-read` (no store could be listed). |
| `history` | object or null | What the session history read; `null` with `--no-history` or when no store could be asked. `sources[]` (per agent: `agent`, `listingFailed`, `listed`, `related`, `read`, `failed` as a count by `no-reader`, `timed-out`, `too-large` or `failed`), `sessionsRead`, `calls`, `shell` (counters of the shell calls and of what the parsers could not read: `shellCalls`, `unresolved`, `looped`, `script`, `truncated`, `exhausted`), `excluded` (folders the history named and the scan left out, counted by reason, never entered) and `limits[]` (`type` `sessions`, `calls` or `duration`, with its cap). |
| `search` | string | `mft-index`, `bounded-walk` or `parent-only`. |
| `scannedAtMs`, `elapsedMs` | number | Unix time of the analysis in milliseconds; how long it took. |

**The outcome** (`clean --json`, field `outcome`)

| Field | Type | Meaning |
|---|---|---|
| `schema` | number | `1`. |
| `mode` | string | `recycle` or `permanent`. |
| `dryRun` | bool | `true` when nothing was removed. |
| `items[]` | array | One entry per chosen item, in the order of the listing. |
| `items[].key`, `path`, `category` | | As in the scan (`path` is `null` for sessions). |
| `items[].outcome` | object | `type` is `removed` (`bytesFreed`), `skipped` (`reason`) or `failed` (`error`). In a dry run `removed` means "would be removed". |
| `bytesFreed` | number | Sum of the removed bytes, what went to the Recycle Bin included. |
| `recycledBytes` | number | The part of `bytesFreed` that went (in a dry run: would go) to the Recycle Bin, freed only when it is emptied. |
| `elapsedMs` | number | Duration of the removal. |

A real dry-run result (shortened to the `outcome`):

```jsonc
// clean --json prints {"schema":1,"scan":{...as above...},"outcome":{...}}
// the outcome:
{
  "schema": 1,
  "mode": "recycle",
  "dryRun": true,
  "items": [
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp\\target",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\\target",
      "category": "builds",
      "outcome": {
        "type": "removed",
        "bytesFreed": 11538432
      }
    },
    {
      "key": "c:\\users\\alice\\appdata\\local\\temp\\w\\sampleapp\\debug.log",
      "path": "C:\\Users\\alice\\AppData\\Local\\Temp\\w\\sampleapp\\debug.log",
      "category": "logs",
      "outcome": {
        "type": "removed",
        "bytesFreed": 2097152
      }
    }
  ],
  "bytesFreed": 13635584,
  "recycledBytes": 13635584,
  "elapsedMs": 2501
}
```

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Done. This includes any `analyze`, any `--dry-run` (even one whose outcome has `skipped` items: check them), `Nothing selected`, `Nothing to clean`, and an answer of no (`Cancelled.`). |
| `1` | Error: the path does not exist or is not a folder; not enough room in the Recycle Bin; the result could not be written (for example a path JSON cannot carry). Nothing was removed, except when the report itself could not be written after a removal (it then goes to stderr when possible). |
| `2` | Wrong usage: unknown command or flag, a `clean` flag given to `analyze`, `--copies` or `--named`, `--include` without a value, a second project, an `--include` that is not in the analysis or is blocked, no terminal and no `--yes`, `--json` without `--yes`. Nothing was removed. |
| `3` | Partly cleaned: at least one item was skipped or failed. Read the report, or `outcome.items`. |

### PowerShell example

```powershell
$project = 'C:\src\my-project'

# 1. Look (read-only) and decide on the facts.
$scan = xiox analyze $project --json 2>$null | ConvertFrom-Json
$ready = $scan.items | Where-Object { $_.level -eq 'recommended' -and $_.blocked.Count -eq 0 }
"{0} recommended items, {1:N0} MB" -f $ready.Count, (($ready | Measure-Object bytes -Sum).Sum / 1MB)

# 2. Rehearse (same mode as the real run, nothing is removed), then remove for good.
$rehearsal = xiox clean $project --builds --worktrees --logs --dry-run --permanent --json 2>$null | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$skipped = @($rehearsal.outcome.items | Where-Object { $_.outcome.type -ne 'removed' })
if ($skipped.Count -gt 0) { $skipped | Format-List; exit 3 }
$result = xiox clean $project --builds --worktrees --logs --yes --permanent --json 2>$null | ConvertFrom-Json
switch ($LASTEXITCODE) {
    0 { "Freed {0:N0} MB" -f ($result.outcome.bytesFreed / 1MB) }
    3 { $result.outcome.items | Where-Object { $_.outcome.type -ne 'removed' } | Format-List; exit 3 }
    default { Write-Error "xiox failed with exit code $LASTEXITCODE"; exit $LASTEXITCODE }
}
```

If the paths contain non-ASCII characters, make PowerShell read the output as UTF-8 first:
`[Console]::OutputEncoding = [Text.Encoding]::UTF8` (`xiox` writes UTF-8).

### For an AI agent

You can call `xiox` as a tool. A safe routine:

1. Before you finish your work on a project, run `xiox analyze <project> --json` (read-only) and
   look at what is left on the PC: build folders, worktrees you created, logs, sessions.
2. Report the items and their sizes to the user. Remove with category flags only when the user
   agrees: `xiox clean <project> --builds --worktrees --logs --yes --json`, after a `--dry-run`
   if in doubt. A `recommended` item is one that regenerates or has no commit to lose, but
   removing it still deletes files git does not keep: a removed worktree takes its ignored
   files with it (`.env`, local databases, editor settings), and an ignored `*.bak` file is
   recommended too. Look at the listing before you remove.
3. **Never pass `--include` for a `caution` item** (a copy, a name-matched folder, a build folder
   or log that git tracks) **without asking the user**, and never use `--permanent` unless the
   user asked for it. Do not try to get around a `blocked` item: it is blocked because removing
   it would lose work or because a session is using it.
4. Check the exit code. `3` means something was not removed: read `outcome.items` and tell the
   user. Never remove the folder you are working in.

**Watching a project over time.** `xiox observe` is a separate, read-only command for a process
that stays alive while you or an agent work on a project: it takes a snapshot, checks again at
intervals and reports what appeared since (new worktrees, copies, build folders, growth), as JSON
lines or as an MCP server, with a ready `xiox clean` command for the new recommended items. It
removes nothing. See [xiox-observe.md](xiox-observe.md).

## Safety

`xiox` never removes:

- **a blocked item** (open agent session, uncommitted changes, commits that exist nowhere else,
  a copy with unsaved work or whose work could not be checked, a locked session), not even with
  `--include`; and not a folder that holds one: listed ones for every folder, and git repositories
  that are not listed for copies and name-matched folders. A build folder or a worktree that holds
  an ignored nested repository is not scanned for one (see below);
- **anything that is not in the analysis**: `--include` takes only paths the listing shows. The
  project's own folder, and any folder that is, contains or overlaps the project or one of its
  worktrees, are refused;
- **through a link**: symbolic links and junctions are not followed, are never listed as copies,
  name-matched folders or logs, and an item that has become a link since the analysis is
  skipped (`now a link: not followed`);
- **a protected location**: the safety guard of the cleaner (the same one the desktop
  app uses) refuses system locations and any folder directly under a drive or share root;
- **branches** or git history: `git worktree remove` is never forced, and `git gc` is never run.

**Re-validation.** The listing is a proposal, not a promise. Right before it removes anything,
`clean` analyses the project again and keeps only the chosen items that are still listed, in
the same category, unblocked and not more suspect than before (an item that now needs a manual
review, or a copy that gained commits or changes, is skipped). The item that is removed is the
fresh one, and a folder is checked once more right before it goes (still there, still a real
folder and not a link, not overlapping the project, and with nothing in it that is blocked).
A copy must have a fresh, positive answer that it has no work of its own; no answer is a refusal.
A chosen worktree, copy or named folder is also left alone when the session listing of this PC
cannot be read now (`the sessions of this PC could not be read, so a live agent session cannot
be ruled out`). An item that no longer passes is reported
as `skipped` with the reason (`no longer there`, `now blocked: uncommitted changes`, `now has
commits or changes the project does not have`, ...). A dry run makes the same checks.

**What is inside a folder.** Removing a folder removes everything in it, so a folder is skipped
when it holds the project or one of its worktrees, an item the analysis lists as blocked, or
(for a build folder) a worktree, copy or named folder of its own. A copy or a named folder is
also skipped when it holds a git repository that the fresh analysis does not list as removable:
the search is bounded (three levels, one level with `--no-drives`), so a repository it did not
list cannot be vouched for. The reason is `it holds the git repository <path>, which is not
listed as removable`. A named folder that holds a copy with unsaved work is refused by
`--include` before anything runs.

## Limits

- **Where copies and name-matched folders are searched.** Not elevated: the project's parent
  folder (its immediate children), each fixed drive from its root down to **three levels**
  (`D:\a\b\c` is looked at, `D:\a\b\c\d` is not) and your home folder down to three levels.
  Never inside system folders, `AppData`, links, another git repository or the project itself;
  a git repository that is found is examined but not entered. Elevated terminal: NTFS drives
  are searched by name through the file-system index, at any depth. The `Search` line says
  which mode was used. In the index mode a copy whose folder name is unrelated to the project is
  found only if it sits next to the project or on a drive that is not read through the index
  (the index offers names, not repository identity). `--no-drives` limits the search to the
  immediate children of the project's parent folder.
- **Names shorter than 4 characters** (`app`, `web`): no search by name, so no `named` items
  (they would attribute other people's folders to the project). Copies are still found, by
  repository identity. The listing says so in a `note:`.
- **Branches are listed only.** `xiox` has no way to delete a branch.
- **One project per run.** There is no `--all`, and the category flags refuse a drive root, your
  user profile or a folder with several projects (`--include <path>` still works there).
- Log files need a git repository: a folder that is not a repository has only `builds`,
  `named` and `sessions`.
- There is no search by file content, only git identity and folder name.
- A build folder that carries a blocked status of the analysis (a project in a dirty or
  unpushed agent worktree) is not turned into a block: the listing marks the builds of a
  project with an open session (`live-session`) and of an unreachable one (`unreachable`) only.
- **The session history is evidence, not a census.** A session that began somewhere else and was
  never linked to the project (a different working folder, no path of the project in its calls)
  can escape. A folder is never called inactive: `activity` is `unknown` at best. The 50 Pi
  sessions that share an ambiguous id and 7 chains of Claude Code subagents are not read. A
  command the parsers cannot read in full leaves its folders unresolved, and an unresolved path
  is never a candidate. The `History` line and `history` in the JSON say what was left out.
- `clean --include` of a `scratch` item is refused until the confirmations that will go with it
  exist.
- `--include` cannot name a session: sessions are taken together with `--sessions`.

## Troubleshooting

**`git was not found on this PC: worktrees and copies were not checked`.** Install Git for
Windows (it must be on the `PATH` of the terminal). Meanwhile the listing still covers build
folders, name-matched folders and sessions.

**`not a git repository, so worktrees and copies of the repository cannot be identified`.**
The folder you gave is not inside a git repository. Pass the project's folder, or run `xiox`
from inside it. A folder that is not a repository is still a project: its build folders,
name-matched folders and sessions are listed.

**`error: refusing to clean everything under <folder>: run it on one project`** (exit code 2).
The folder is a drive root, your user profile (or a folder above it) or holds several
projects, and a category flag would take the builds and sessions of all of them. Pass the one
project's folder, or name items with `--include <path>`.

**`note: the sessions of this PC could not be read ...`.** The list of agent sessions could not
be read, so an open session cannot be ruled out. `analyze` shows it; `clean` skips worktrees,
copies and named folders (reason `the sessions of this PC could not be read, so a live agent
session cannot be ruled out`) until a run can read them.

**`error: path not found: ...` or `error: not a directory: ...`** (exit code 1). Check the path;
it must be a folder. Quote paths that contain spaces.

**`error: not a terminal: pass --yes to remove without asking`** (exit code 2). `clean` was
started with its input redirected. Run it in an interactive terminal, or add `--yes`.

**`error: --json needs --yes to remove anything`** (exit code 2). Add `--yes`, or `--dry-run`.

**`Recycle Bin on C: is too small ...`** (exit code 1, nothing removed). The Recycle Bin of that
drive cannot take what you chose. Empty it, or add `--permanent`.

**Exit code 3.** Not everything was removed. Each `skipped` or `failed` line of the report has a
reason: the item changed since the analysis (run `xiox analyze` again); it is in use (`in use
by: <program>`); `git refused to remove the worktree (not forced; local changes are never
discarded)` (commit or discard the changes yourself, then run again); or the safety guard
refuses it. The guard refuses a folder directly under a drive or share root, `C:\Users\<name>`
and the `Users` folder, the user-profile folder and any folder that contains it, system
locations (`C:\Windows`, `Program Files`, `ProgramData`, ...), and names that are never a
removable artifact. The reason is shown as, for example, `protected system location (...)`.
The listing marks only the first of these (the top-level rule); the others show up when the
item is skipped.

**The analysis is slow.** It measures every folder and searches the drives. Use `--no-drives`
when the copies and name-matched folders you care about are next to the project, or run it from
an elevated terminal to use the file-system index.

**The `·` in the `Total` line looks wrong.** `xiox` writes UTF-8: switch the console to it
(`chcp 65001`), or use `--json`.
