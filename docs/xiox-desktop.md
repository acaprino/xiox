# Xiox: the desktop app

Xiox is a Windows cleaner for people who code. The desktop app (`xiox-ui.exe`) is one window
with three sections: **Cleanup** (temporary files, caches, browser data, logs), **Registry**
(entries that point to files or programs that no longer exist) and **Developers** (build folders,
git worktrees and archived coding-agent sessions of every project on the PC).

It proposes, you choose. Every section lists what it found with a level and a reason, some
entries are blocked and cannot be removed at all, and nothing happens until you press a
button and answer a confirmation. The window only draws and sends your clicks and keys; every
decision about what may be removed is made by the same engine that the command line (`xiox`)
uses.

- [Starting it](#starting-it)
- [The window](#the-window)
- [Cleanup](#cleanup)
- [Registry](#registry)
- [Developers](#developers)
- [Levels and blocked entries](#levels-and-blocked-entries)
- [Confirmations, cancel and the report](#confirmations-cancel-and-the-report)
- [Safety](#safety)
- [Keyboard](#keyboard)
- [Settings](#settings)
- [Where things are stored](#where-things-are-stored)
- [Troubleshooting](#troubleshooting)
- [The command line and the observer](#the-command-line-and-the-observer)

## Starting it

There is no installer: you run `xiox-ui.exe`. Build instructions are in the
[README](../README.md#build).

**Administrator rights.** The production build asks for them once, when it starts: Windows
shows its usual prompt (the executable carries a `requireAdministrator` manifest). Without them
the app cannot clean system folders or fix `HKLM` registry keys. A
development build (a debug build, as made by `cargo run`) does not ask and runs as you. The
status row always says which one you got: `administrator` or `standard user`.

**One window at a time.** Starting the app a second time does not open another window: it
brings the first one to the front (and restores it if it was minimized) and exits.

**The window remembers where it was.** Its size, position and whether it was maximized are
saved when you close it and restored at the next start. If the saved place no longer fits your
screens (a monitor was unplugged), the window opens in the centre instead. The smallest size is
1000 x 640 pixels. The window is always dark and has the system title bar.

**`--only-root <folder>`.** An optional argument that restricts the Developers section to one
folder. In this mode nothing is read from or written to `pref.json`: the Developers options and the
Cleanup choices stay in memory, the window's place and theme are not kept, and
the Winapp2 rules are neither loaded nor downloaded. The argument needs a folder: without one
the app prints a message and exits with code 2, instead of falling back to your real profile.

At start the app also loads the cleaning rules (and updates Winapp2 in the background, see
[Settings](#settings)) and removes the leftovers of a free-space wipe that a power cut or a crash
interrupted (see [Cleanup](#cleanup)).

## The window

The window is the shell of the suite (Imago, on Sigrid's parts), with Xiox's own screens in it: the amber of Xiox is its only
accent, the views are tabs in the title bar and the buttons of the view are under the content. The title bar is the shell's: drag it
to move the window, double-click it to maximize, use its minimize, maximize and close buttons (hovering maximize offers the Windows
11 snap layouts), the switch of light and dark, and `Commands` (`Ctrl`+`K`). The window is opaque, in the ground of the look, until
you choose **glass** (Windows' acrylic behind the page) in [Settings](#settings) or with the command `Glass` of the palette: it
changes at once, with no restart, and the next start opens as you left it. A browser has no glass. Everything else (the lists, the
boxes, the keys, what each button does) is the same either way.

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ XIOX   CLEANUP  REGISTRY  DEVELOPERS  SETTINGS          ☀  Commands Ctrl K │  title bar: the views as tabs
│ ANALYSIS DONE   13.48 GB to delete (46,999 files)  administrator · RAM 32 GB │  status row
│ Cleaning…  ████████░░░░  64%  Windows temporary files                        │  progress (while a job runs)
│ Xiox is cleaning the profile of “…”, not that of “…” …                       │  profile strip (when it applies)
├────────────┬───────────────────────────────────────────────────────────────┤
│ Categories │ title ................................  Details i  Options o │
│ System     │ ☐  ITEM ▲          LEVEL        FILES        SIZE ▼            │  list
│ Explorer   │ ☑  SYSTEM                       30,389     11.29 GB            │
│ …          │   ☑ Windows temporary files     18,204      4.10 GB            │
│            │                                    │ Details (on request)      │  inspector
│            │                              [ Analyze ] [ Clean 8 items ]     │  buttons
├────────────┴───────────────────────────────────────────────────────────────┤
│ ● ↑↓ move · space toggle · s sort · a analyze · ↵ clean                     │  footer
└────────────────────────────────────────────────────────────────────────────┘
```

The title bar's mark is the initial of the name on the amber of Xiox, and the executables' icons are the same mark. Open the window
menu (`Alt`+`Space`) for `Reload interface`, which reloads the page when it hangs. A window narrower than 860 px puts the views in a
row under the bar, and a sidebar narrower than its place folds into a drawer or a rail; nothing runs off the page from 340 px.

**Title bar and buttons.** The Xiox wordmark, the views (`Cleanup`, `Registry`, `Developers`, `Settings`, each also a command of the palette) and `Commands`. The buttons
of the open section (`Analyze`, `Clean 8 items`, `Find issues`, `Refresh`, ...) sit under the list,
the main one last and in the accent colour. A small key next to a
label says which key presses it. A button that cannot be used right now is greyed out; the chapters
below say when. A reason a button is greyed out is its hint.

**Status row.** On the left a badge (`Ready`, `Analyzing`, `Analysis done`, `Cleaning`,
`Cleanup done`, ...) and a sentence with the figures (`13.48 GB to delete (46,999 files)`,
`40 issues found · 2 selected`). On the right who you are running as, `administrator` or
`standard user`, and the PC's total memory (`RAM 32 GB`).

**Progress bar.** While a job runs (an analysis, a scan, a cleaning) a bar shows what it is
doing: its label, a percentage and the item it is on. It does not animate by itself.

**Profile strip.** Shown only when it applies: *Xiox is cleaning the profile of “A”, not that of
“B”, who opened this Windows session.* It appears when the program runs under a different account
from the one that opened the Windows session, typically because the administrator prompt was
approved with another administrator's password. "Your" Temp folder, browser data and the other
per-user locations are then those of account A, not of the person sitting at the screen. Close
the app and start it again from the right account if that is not what you want.

**Views.** Cleanup, Registry, Developers and `Settings`: tabs in the title bar, with a line under the open one. Each is also a
command of the palette (`Go to Cleanup`, ...).

**Categories bar (the sidebar).** In Registry its heading is `Look for`: the kinds of issue a scan
looks through. The categories of the open section, each with its
item count and, after an analysis, its size (`System`, `Explorer`, ...; for Developers
`Projects`, `Worktrees`, `Archived sessions`). Under them, `Options`: the choices that belong to
the section. Registry has no categories: for it this bar is only
`Options`. In Cleanup the list shows every category one after another, so the bar is a menu: a
click scrolls the list to that category (opening it if it was folded) and the entry of the
category the list is at is highlighted as you scroll. A small box in front of each entry shows
at a glance how much of the category is ticked. In Developers each category is a list of its own
and a click opens it.

**List.** The rows of the section, largest first unless you choose another order: a click on a
column title (`Item`, `Size`, `Date`, `Project`, ...) orders the list by that column, a second click
turns the order round, and an arrow shows the column and the direction (`s` and `S` do the same
from the keyboard). Rows without a value go last either way, rows inside a parent stay under it,
the rows of a group stay under its heading, and the categories of Cleanup keep their place. Each row has a box (ticked, empty, or
half-ticked when only some of its children are), a name with a second line that says what it is
or, for a blocked row, why it cannot be chosen, and the columns of the section. The whole cell
around the box is the button, so you do not have to aim at the square; a click on the rest of the
row selects it (the detail panel follows). The box in the header of the check column ticks or
clears everything that can be ticked, in every section that has boxes (it is half-ticked when
only some rows are on); items marked `caution` are never ticked by it, nor by a category box,
only by their own box. A caret folds and unfolds the rows under a parent. In Cleanup, which has
many rows, a search box above the list filters them.

**Groups of rows.** The rows that belong together sit under a heading with its own box, which
ticks or clears every row under it that can be ticked (never a blocked one, never one marked
`caution`) and is half-ticked when only some are: the sections of Winapp2 in Cleanup
(`Internet`, `Utilities`, ...), the kinds of issue in Registry (`Missing shared DLLs`, ...), the
archived sessions by project and the worktrees by repository in Developers. A heading also says
how many rows it holds and what they add up to. The worktrees made by hand keep a heading without
a box: Xiox does not remove them.

**Categories in the list (Cleanup).** Each category starts with a header row that stays at the
top while its rows scroll: its own box (ticks or clears the whole category), a caret (folds it),
how many items are ticked, and the files and size found. A click on the header folds or opens
it. A category with more than 40 rows (`Applications`) starts folded; `Fold all` / `Open all`
above the list does every category at once, and a search opens the categories it found
something in.

**Detail.** In the inspector at the right of the list, when you ask for it with `Details` (above the list) or `i`: what the selected row says about itself, as `item`, `files`,
`size`, `level`, `mode` (`permanently`, `empties the Recycle Bin`, `to the Recycle Bin`,
`permanently (git)`), `why` and, when there is one, `warning`. A blocked row also gets a
`blocked` line with the reason. Some rows add lines below, for example which browser profile
holds how many cookies.

**Notices.** Coloured one-line strips above the list: a result, a warning, or an error or
refusal. A strip that has more to say holds a `Show 2 more lines` row: reach it with `Tab` and press
`Enter` or `Space` to open the extra lines under it, and again to close them. A result that
only half worked (some keys not fixed) is shown in the warning colour,
not as a success.

**Footer.** The keys of the open section that work now (`↑↓ move · space tick · ←→ fold · s order · a analyze · ↵ clean · ctrl f search · f6 next region`) and,
in some sections, a line about the selection (`Selected in Projects: 6.80 GB · 2 folders`).

While a job runs the lists are frozen: you cannot tick or untick anything, because the
selection is what the job is working on.

**Windows and the taskbar.** The window title names the open section (`Cleanup - Xiox`), so Alt+Tab
and the taskbar tell the sections apart. While a job runs the taskbar button shows its progress, and
when it ends while Xiox is behind other windows the button flashes once. The window never opens
larger than the work area of the screen it opens on (the saved size included), and the smallest size
(500 x 540, title bar included) fits the work area of a 1366 x 768 laptop at 125 % scaling. A folder dragged onto the window is offered to
Developers: a message asks `Add "<folder>" to the folders Developers analyzes?` and nothing is added
until you answer `Add`; anything that is not a folder only gets a message saying so.

**Theme.** Light or dark with the switch of the title bar, or with the three commands of the palette (`Ctrl`+`K`): `Light`, `Dark` and
the system's look, which follows the Windows theme, title bar included; the choice is kept for the next start. The accents are fixed
(the amber of Xiox): a deliberate choice, not a gap. The palette is the suite's (its contrasts are tested in Imago), so the window
does not follow the system's accent colour. What follows the system is what accessibility needs: the high-contrast (forced colours)
themes, reduced motion and the text size.

## Cleanup

Temporary files, caches, browser data, logs and the other things Windows and your programs
leave behind. This is the section the app opens in.

**Categories.** `System`, `Explorer`, `Advanced`, `Windows`, `Browser`, `Development & AI` and
`Applications`. Each shows `n items · m selected` and, after an analysis, the size found. The
rows of a category are its entries: the built-in ones (`Recycle Bin`, `User temporary files`,
`Windows temporary files`, `Memory dumps`, `Windows log files`, ...), a few special entries
described below and, in `Applications`, the rules of the Winapp2 catalog, grouped under headers
such as `Internet (3)`. All of them are in one list, one category after another. The search box
above the list looks through every category (name and description).

**How it works.** Three steps, in this order:

1. **Tick** what you want to look at. The entries that are on by default (the `Recommended`
   ones) start ticked.
2. **Analyze** (`a`). The app measures what is ticked and writes the files and the size next to
   each row. Nothing is deleted. You can stop it with `Cancel` (`Esc`): the rows it did not get
   to stay without a figure, and a notice says so.
3. **Clean** (`Enter`; the button reads `Clean 8 items`, with the number of ticked entries).
   The button is greyed out until an analysis is on screen: *Clean works on the analysis on
   screen.* It opens a confirmation; only its confirming button (`Clean 8 items`) starts the removal.

**Columns.** `Item`, `Level`, `Files` and `Size`. The level comes from the rule set:
`Recommended` when the entry is on by default, `Optional` when it is off by default, `Caution`
when it carries a warning (signing out of websites, losing history, ...).

**Everything here is deleted permanently**, except that the `Recycle Bin` entry empties the
Recycle Bin (its `mode` says `empties the Recycle Bin`, and the confirmation repeats that this
cannot be undone). This section has no choice between the Recycle Bin and permanent removal:
that choice belongs to [Developers](#developers).

**Options.** `Only delete Temp files older than 24 hours`: when it is on, temporary folders are
cleaned of files at least a day old, so a program that is writing a file right now keeps it. In
`Browser`, `Edit cookie keep list` opens `Documents\Xiox\cookie-keep-list.txt` in Notepad
(it creates the file if it is missing): one domain per line, and the cookies of those sites are
not removed. In `Applications`, a line says whether the Winapp2 rules are up to date.

**Your choices are kept.** What you tick and untick, and `Only delete Temp files older than 24
hours`, come back at the next start. Only the choices that differ from the defaults are saved, so
an item that a later version adds starts at its own default. A file that cannot be written is
reported under the options (`Choices not saved: ...`).

**Special entries.** Besides the file rules the section has entries that act on something other
than plain files. Each has a `Caution` warning that you read in the confirmation:

- *Cookies (with a keep list)*, *compact databases*, *Firefox and Thunderbird history*, *last
  download path* (Browser). Removing cookies signs you out of the websites you use, except the
  ones on your keep list.
- *Clipboard*, *DNS cache*, *Windows event logs*, *invalid shortcuts*.
- *Environment PATH* (removes entries that point to folders that no longer exist, and
  duplicates; a `.reg` backup goes to `Documents\Xiox`), *Network passwords* (saved credentials
  for network drives, shares, Remote Desktop and the FTP accounts of File Explorer; browser and
  Microsoft account passwords are not touched) and *old Windows data* (`Windows.old`: afterwards
  you cannot go back to the earlier version of Windows). These three act only on what **the last
  analysis showed**: without an analysis the entry is skipped with *Run Analyze first to see the
  items to remove*.
- *Clean free space*: overwrites the free space of the **system disk only** so that deleted
  files cannot be recovered. It can take hours, leaves at least 1 GB free while it writes (other
  programs may fail to write, so close them first), does not touch the MFT, other disks or the
  shadow copies, and **refuses on an SSD**. The files it writes live in a hidden folder that is
  always removed, even if the run is cancelled; if Windows crashes mid-run, the app removes the
  leftover folder the next time it starts.

**Blocked entries.** An entry whose program is running cannot be ticked: *Close Google Chrome
to clean*. Close the program and analyze again. An entry that needs administrator rights shows
*Needs administrator: some folders are skipped* when you are not elevated, and its detail says
how many folders were skipped.

**After the clean.** The [report](#confirmations-cancel-and-the-report) replaces the list. When
you close it, the analysis goes with it (its figures are about files that are no longer there)
and your ticks stay: press `Analyze` again to see what is left.

## Registry

Registry entries that point to a file or a program that no longer exists: shared DLLs, file
extensions, ActiveX/COM classes, type libraries, fonts, application paths, help files, installer
folders, obsolete software, startup entries and the MUI cache.

**Options (second sidebar).** One box per kind of problem: `Missing shared DLLs`, `Unused file
extensions`, `ActiveX and class problems`, `Type libraries`, `Fonts`, `Application paths`,
`Help files`, `Installer folders`, `Obsolete software`, `Run at startup` and `MUI cache`. All
start ticked. At least one must be ticked (*Tick at least one category to look through.*).

**Find issues** (`a`). Scans the ticked kinds, one at a time (the progress bar says which). It
does not change anything. The list shows `Issue` and `Registry key`; the detail adds the `data`
(the missing path). **Every issue the scan finds starts ticked**: untick what you want to keep,
or press `None` and tick only what you want. Nothing is changed until you press
`Fix selected` and answer the question.

**Fix selected (n)…** (`Enter`). Opens a question, *Save a registry backup?*, with three buttons:

| Button | What it does |
|---|---|
| `Back up and fix N issues` | saves a `.reg` copy of the keys it is about to change in `Documents\Xiox` (the file is named `registry-backup-<date>_<time>.reg`, and an existing file is never overwritten), then fixes |
| `Fix N issues without a backup` | fixes without saving anything |
| `Cancel` | closes the question, nothing happens |

If the backup cannot be written, **nothing is fixed** and a notice says why (*Registry backup
failed (...): no repair was done.*).

**The result** is a notice (`Fixed 2 issues.`, with the path of the backup on the line behind
`+1 more`). If some keys could not be changed it adds how many and why (*1 not fixed
(administrator rights are needed)*: start the app as administrator), and the strip turns to the
warning colour. When everything was fixed
the fixed issues leave the list; when something failed they stay.

**There is no restore button in the app.** To go back, double-click the `.reg` file of the
backup and let Windows merge it (the section says so in its options).

## Developers

One view over every project on the PC: build folders, git worktrees and archived sessions of
the coding agents (Claude Code, Codex, OpenCode and Pi). The projects are found from the folders
those agents worked in; you can add more folders to look through.

**Options.**

| Option | Meaning |
|---|---|
| `Layout` | how `Projects` is listed: `Tree` (default), the projects in the folders they sit in; `Flat`, one list of every project with its full path under its name, so the order by size runs over the whole list |
| `Days of inactivity` | how long a project must have been untouched to be suggested for cleaning (1 to 365, default 14) |
| `Deletion` | `To the Recycle Bin (recoverable)` (default) or `Permanently` |
| `Also analyze these folders` | also look through the folders in the list below |
| `Extra folders` | the folders you add with `Add folder…` (a folder picker opens) and take away with `Remove` (select one first) |

These options are saved (see [Where things are stored](#where-things-are-stored)). A note under
`Deletion` reminds you that the Recycle Bin frees no space until you empty it, and that the
`Recycle Bin` entry of Cleanup empties it. If the settings cannot be saved a red line says so
(`Settings not saved: ...`) and the options only last until you close the app.

**Analyze** (`a`). Looks at the projects (the progress bar says `Looking at projects…`), then
shows the result with the time it took and how the sizes were measured (`sizes from the disk
MFT` or `folders measured one by one`). Until you analyze, the list says what to do. Warnings
come as notices (*OpenCode sessions could not be read: some projects may be missing.*).
**Leaving the Developers section drops its analysis** (it can be large): analyze again when you
come back.

**Categories.** Each has its own list, its own ticks and its own button:

| Category | Columns | The row is | The button |
|---|---|---|---|
| `Projects` | `Project · folder`, `Last use`, `Level`, `Size` | a project, with its build and dependency folders under it | `Clean 2 folders` |
| `Worktrees` | `Worktree`, `Repository`, `State`, `Size` | a git worktree created by an agent | `Remove 2 worktrees` |
| `Archived sessions` | `Title`, `Harness`, `Project`, `Date`, `Size` | one archived agent session | `Delete 2 sessions` |

The category entries show how many items are ticked and the size the analysis found. The button
acts on **the open category only**; what you ticked in the other two is not touched, and the
confirmation says so (*Also chosen in the other lists (not cleaned now): 2 worktrees · 2
sessions.*). Dates are written `YYYY-MM-DD`.

**Projects.** The projects sit in a tree of the folders they are in, as in Explorer: `D:\` holds
`Projects` and `Work`, `Projects` holds `acme-api`. A folder with no project of its own and a single
folder inside it is one row (`D:\Projects`, not `D:\` then `Projects`). Each folder shows what its
projects add up to, folds and opens with its arrow, a click or ←/→ (it starts open), and has a box
that ticks every project inside it that is not in use. The sort works inside each folder.
A project inside another project (`admin-ui` inside `tri-llm-gateway`) is not a folder of its own: it
sits under that project, named by its path below it, and is always in sight, even when the project
above is closed. The project above shows what it and the projects inside it add up to (its detail
has both figures, `size` and `with sub-projects`); its box and its arrow are about its own folders
only, which come before the projects inside it. A project at the root of a drive (`D:\`) never holds
the others. With `Layout: Flat` there are no folders: every project, those inside others too, is a
row of the list with its full path as its second line and its own size only.
Each project has a box that ticks all its folders (half-ticked when only some are)
and, under it, the folders themselves (`target`, `node_modules`, `.venv`, ...), each with its own
box, level, size and reason (*Build output that cargo recreates.*, *Restored by npm install.*).
The folders of an inactive project that the engine recommends start ticked; a recently used
project's folders are up to you. `All` ticks every folder of every project that is not in use,
whatever its level. A project an agent is using right now cannot be ticked: its row is greyed out
with *The project is in use by an agent: its folders are cleaned when the session is closed.*

**Worktrees.** The git worktrees that the agents created. They are removed by `git worktree
remove`, never forced, and **always permanently**, even with `To the Recycle Bin` selected (the
confirmation says so; the build folders inside a worktree do go to the Recycle Bin). The `State`
column says why one is safe to remove (*Merged into main*, *No changes since the last commit*) or
why it stays (*Unsaved changes: the worktree stays*; such rows are greyed out and cannot be
ticked). Worktrees created by hand are listed under `Other worktrees (created by hand, not
removed from here)`: they have no box and are information only.

**Archived sessions.** The sessions the agents have archived: title, which agent, which project,
date and size (`database row only` for a session that lives in a database). They start unticked.
A session that is open in a running agent is greyed out (*In use by a running Claude Code
session*). With `To the Recycle Bin` selected, a copy of the session file goes to the Recycle Bin
before the session is deleted, when the file exists.

**Clean.** The button opens the confirmation. It says what will be removed and where it goes
(*The selected items go to the Recycle Bin.* or *The selected items will be deleted
permanently.*), then the figures (`2 folders · 6.80 GB`) and one line per folder with its size.

- **Caution folders.** If you ticked folders at the `Caution` level the confirmation names them
  (`1 folder at the Caution level:`, with the reason of each) and shows a box, `I have read the
  warnings`. The `Clean` button stays greyed out until you tick it.
- **The Recycle Bin is too small.** When the Recycle Bin of a drive cannot take what you chose,
  the confirmation says so (*The Recycle Bin of D: has 0 B free, the cleanup moves 6.80 GB.
  Delete permanently this time?*) and the `Clean` button is replaced by the same words with `permanently` (`Delete 3 sessions permanently`). Press it
  to delete for good for this run only (the `Deletion` option does not change), or `Cancel` and
  make room. The Recycle Bin is never skipped silently.
- **A stale analysis.** An analysis older than 30 minutes is refused (*The analysis is too old:
  press “Analyze” again.*): the disk may have changed since.
- **Archived sessions are checked again** right before they are deleted: a session that has been
  reopened, unarchived or deleted since the analysis is skipped with a reason, never deleted.

**Space freed.** A cleaning that goes to the Recycle Bin frees no space yet: the report says
`Sent to the Recycle Bin` with the size, and the status row `Space freed: 0 B`. The space comes
back when you empty the Recycle Bin (the `Recycle Bin` entry of [Cleanup](#cleanup) does that).

## Levels and blocked entries

Every entry has a level, the same words as in the command line:

| Level | Meaning |
|---|---|
| `Recommended` | Regenerates, or carries nothing that would be lost (caches, build output). |
| `Optional` | Can be lost, but you may still want it (dependencies, sessions, entries that are off by default). |
| `Caution` | Needs your judgement: it may sign you out, delete history, or take long to rebuild. Read its warning. |

The level is a colour and a word on the row, in the detail box and in the confirmation. The page
does not decide levels: they come from the engine.

A **blocked** entry is greyed out, its box cannot be ticked, `All` skips it, and the second line
of the row says why. The reason is a fact about your PC right now:

| Where | Reason shown |
|---|---|
| Cleanup | *Close Google Chrome to clean* (the program is running); *... : this option cannot be used* (the rule is not supported on this system); *Needs administrator: some folders are skipped* |
| Projects | *The project is in use by an agent: its folders are cleaned when the session is closed.*, or the reason it is locked or unreachable |
| Worktrees | the `State` reason, for example *Unsaved changes: the worktree stays* |
| Archived sessions | *In use by a running Claude Code session* (or another agent) |

A blocked entry cannot be removed from the app by any means. To clean it, change the fact (close
the program, close the agent session, commit or discard the changes) and analyze again.

## Confirmations, cancel and the report

**The confirmation.** Every removal and every registry fix goes through
one: a window in front of the page, with the title of the action (`Start cleaning?`, `Clean the
selected folders?`, `Save a registry backup?`), what will happen, the list
of what will be touched, the warnings and its buttons. While it is open the rest of the window is
inert: you cannot click or Tab behind it, and nothing can be ticked or unticked (what the
warnings describe must be what runs). `Esc` or `Cancel` closes it and nothing runs.

- **Cleanup** asks `Start cleaning?` with *The selected files will be deleted permanently. Files in
  use are left where they are.*, followed by `Warnings for the chosen options`: one line for each
  chosen entry that has a warning (cookies, emptying the Recycle Bin, an application cache that
  has to be downloaded again, ...). The confirming button (`Clean 8 items`) starts.
- **Developers** asks about folders, worktrees or sessions, as described above.
- **Registry** is described in its chapter.

When a confirmation opens, the focus goes to its main button or, if that one is greyed out until
you tick `I have read the warnings`, to that box. The buttons ignore `Enter` and `Space` for 300
milliseconds after the confirmation opens, and ignore a key that is held down: the `Enter` that
opened the confirmation (or that you are still holding) must never also confirm it. A mouse click
is never held back.

**Cancel.** While an analysis or a cleaning runs in Cleanup or Developers, a `Cancel` button
(`Esc`) appears in the top bar. It stops before the next file or item; it does not undo what was
already done. An analysis that is cancelled keeps what it had found (the other rows have no
figure and a notice says so); a cleaning that is cancelled ends with a report of what was done,
marked as cancelled (*Cancelled: the items not yet started stayed where they were.*). While it is
stopping the button reads `Cancelling…`. Registry has no cancel: its jobs are short.

**The report.** When a cleaning ends (done, cancelled, or stopped by an internal error) a report
replaces the list:

| Part | What it says |
|---|---|
| title | `Cleanup done`, `Cleanup cancelled`, `Cleanup not run`, `Cleanup interrupted` (the status badge says the same) |
| `Space freed` | what was deleted for good |
| `Sent to the Recycle Bin` | what went to the Recycle Bin (shown only when something did) |
| the counts | `Files deleted`, `Folders deleted`, `Duration` |
| notes | for example *Some files could not be removed. They are listed below.* |
| `By category` | the freed size per category, largest first |
| `Skipped (n)` | what was left alone, one line each: the item and the reason (*In use by another program*, *Left: protected or unreadable*, *Not started: the cleanup was cancelled*); the full path is on a line of its own under it |
| `Failed (n)` | what was tried and refused (*Access denied*) |
| `Items (n)` | in Cleanup: for each entry the size freed, how many were removed, how many were left and a note; a caret opens the details (which cookies were kept, what stayed in use, ...) |

**Skipped** means left alone on purpose, or in use, or changed: nothing went wrong, it is simply
still there. **Failed** means a removal was attempted and refused by Windows.

`Save as text…` (shown when the report can be saved) opens the Windows save dialog, proposing
`Xiox report.txt`, and writes the report as plain text. The dialog belongs to the program, not to
the page. The report then says *Report saved to <path>* or *Report not saved: <why>*; cancelling
the dialog changes nothing. `Close` (or `Esc`) closes the report.

## Safety

The engine is written so that a mistake costs a skipped entry, not your work. In plain words:

- **Nothing is hidden.** Everything the section found is in the list, with its level and its
  reason. What cannot be removed is shown too, with the reason, not left out.
- **Nothing runs by itself.** Opening the app, switching section or analyzing never deletes
  anything. A removal needs, in this order, an analysis on screen (Cleanup and Developers), a press
  of the main button and a click on the confirmation. Some entries start ticked (the
  `Recommended` ones in Cleanup, the engine's recommendation in Developers); the ticks are in plain
  sight and the confirmation lists exactly what it is about to do.
- **Pressing a key is not confirming.** `Enter` only opens the confirmation. Inside it, `Enter`
  and `Space` are ignored for the first 300 ms and while held.
- **What you read is what runs.** The ticks are frozen while a job runs or a confirmation is
  open, and the removal works on the analysis you were looking at, not on a newer one.
- **The analysis is checked again.** In Developers an analysis older than 30 minutes is refused,
  only items the analysis itself would plan are accepted, and archived sessions are looked up
  again right before they are deleted. The Cleanup entries that need an analysis (PATH, network
  passwords, old Windows data) act only on what that analysis showed.
- **Unsaved work is never the price.** A worktree with uncommitted changes, a project an agent is
  using and a session that is open are blocked. Worktrees go through `git worktree remove`, never
  forced.
- **Protected locations are refused.** The cleaner's safety guard refuses system locations,
  drive roots and folders directly under a drive root, and no link (symbolic link or junction) is
  followed.
- **Windows keeps what it needs.** Before fixing the registry the app offers a `.reg` backup,
  and a fix that was asked to make a backup and could not make it does not run.
- **The window cannot name a path.** The page that draws the window sends clicks and keys, never a
  path: the folder picker and the save dialog are opened by the program itself, and the page has
  no access to your files, to the network or to a shell. The only network use is the download of
  the Winapp2 rules, done by the engine.
- **A refusal says why.** If something is refused, the reason appears in a red strip above the
  list (see [Troubleshooting](#troubleshooting)). A refusal changes nothing.

## Keyboard

Every function can be reached from the keyboard. The keys below work when no text field has the
focus (so typing in a search box never presses anything), no confirmation is open (unless noted)
and no `Ctrl`, `Alt` or `Windows` key is held. They press only what the screen offers: a button
that is greyed out ignores its key.

| Key | What it does |
|---|---|
| `Ctrl`+`K` | the command palette: every command with its key, the views, and the look commands (`Light`, `Dark`, the system's, `Glass`) |
| `Ctrl`+`F` | the search box of the list |
| `F6` `Shift`+`F6` | move between the sidebar, the content and the inspector |
| `↑` `↓` | move the selection one row |
| `PageUp` `PageDown` | move ten rows |
| `Home` `End` | first and last row |
| `→` `←` | unfold and fold the rows under a project or an entry |
| `Space` | tick or untick the selected row, or every row under the selected heading (not while a job runs, not on a blocked row) |
| `s` / `S` | order the list by the next column / turn the order round |
| `i` / `o` | open or close the inspector on the details of the selected row / on the options of the section |
| `a` | `Analyze` in Cleanup and Developers, `Find issues` in Registry |
| `Enter` | in the list, the section's main button: `Clean` or `Fix selected`; it opens the confirmation (a focused button presses itself) |
| `Esc` | in this order: the open menu, palette or inspector; the confirmation (its `Cancel`); leave a text field; close the report; cancel the running job |
| `Tab` `Shift`+`Tab` | next and previous control |

A key held down does not repeat an action: one `Enter` is one press, so a held `Enter` cannot run
on past the confirmation it just opened. The arrows do repeat, that is how a long list is crossed.
The footer shows the keys of the open section, taken from the command registry, so what it says
and what the keys do agree. The commands are Sigrid's registry (`useCommand`): `Ctrl`+`K` lists them all, with the actions of the open section.

**Tab order.** The title bar (the views; the switch of light and dark; the `Commands` button; the window buttons), the categories of
the sidebar, then the main panel (`Details` and `Options`, the search box, the list with its top check box), the inspector when it is
open, and last the buttons of the section. The list is **one** Tab stop: once in it, move with the
arrows and tick with `Space`; the boxes and carets of its rows are for the mouse. The inspector
can be reached too (`F6`), so a long reason can be scrolled with the arrows. A visible ring shows
where the focus is.

**In a confirmation.** The focus starts on `Cancel`. `Tab` and `Shift`+`Tab` cycle through its controls and never leave it;
`Enter` or `Space` press the button that has the focus; `Esc` or a click outside closes it; the title bar stays free, so the window can be moved and closed. Because of the guard
described above, the buttons ignore `Enter` and `Space` for 300 ms after the confirmation opens
and while the key is held down. When it closes, the focus goes back to where it was.

**Zoom and copying.** These work everywhere, in a text field and in a confirmation too:

| Key | What it does |
|---|---|
| `Ctrl`+`+` (or `Ctrl`+`=`), `Ctrl`+`-` | make the window's content larger or smaller, from 50 % to 200 % in steps of 10 % |
| `Ctrl`+wheel | the same, one step per notch |
| `Ctrl`+`0` | back to 100 % |
| `Ctrl`+`A` | in a text field, its text |
| `Ctrl`+`C` | copy what is selected |

The zoom is remembered for the next start (a level saved by an earlier version, 67 %, 125 % or 175 %, reads as the nearest step: 70 %, 130 %, 180 %). The window is not a web page: buttons, rows and titles
cannot be selected and keep the arrow pointer, the right button opens a menu only on selected text
(`Copy`) and in a text field (the editing commands), and the browser keys (`F5`, `Ctrl`+`R`,
`Ctrl`+`P`, `Ctrl`+`F`, `F12`, `Alt`+`←`, the mouse's back button) do nothing. What is worth copying
can be selected with the mouse: the paths and reasons of the detail panel, the report, the notices
and the text of a confirmation.

## Settings

Settings is a view, the fourth tab: open it with the `Settings` tab or the command `Go to Settings`. Its sidebar lists its two
sections, `Appearance` and `About`, and a click scrolls to one. `Appearance` says where the theme is (the switch of the title bar and
the palette: there is no switch of its own here) and, in the window and not in a browser, has the switch `Glass window`: it changes
at once and the window keeps the choice for the next start (the same choice is the command `Glass` of the palette).
`About Xiox`: the version (`Version 0.1.0`), the state of the
cleaning rules and where they come from:

- `Winapp2: version 260915` when the rules are loaded, `Winapp2: loading` while they are being
  read, `Winapp2: not downloaded yet` when no copy is available.
- The attribution that the Winapp2 licence asks for: *The cleaning rules for applications come
  from Winapp2 (https://github.com/MoscaDotTo/Winapp2), licence CC-BY-SA-4.0, downloaded at every
  start.*

Leave Settings by opening a section. Settings does not open while a confirmation is up.

The Winapp2 rules are downloaded when the app starts and stored outside the executable (see the
next chapter). Their licence stays their own.

## Where things are stored

| What | Where |
|---|---|
| The settings of the app: the Developers options (`developers`) and the Cleanup choices (`cleanup`), one section each | `pref.json` in the program's folder, beside `xiox-ui.exe` |
| The window's size and position, its theme for the first frame of the next start, and its glass | `window.json`, `window-theme` and `window-material` in the program's folder (Imago's files) |
| The look, the theme, the folded sidebar and the zoom of the page | the window's own storage (Imago keeps them there) |
| Registry backups | `Documents\Xiox\registry-backup-<date>_<time>.reg` |
| Backup of the registry keys an `Applications` rule is about to delete | `Documents\Xiox\rules-backup-<date>_<time>.reg` |
| The cookie keep list | `Documents\Xiox\cookie-keep-list.txt` |
| `.reg` backup of an Environment PATH change | `Documents\Xiox` |
| The Winapp2 rules | `%ProgramData%\Xiox\winapp2\` |
| Cache of the agent-session listing | `%LOCALAPPDATA%\Xiox\mukade` |
| Reports | nowhere, unless you press `Save as text…` and choose a place |

`Documents` is your Documents folder (`%USERPROFILE%\Documents`). The app writes nothing else
outside the places you choose. A window saved on a monitor that is gone comes back inside the monitors there are now; the fixtures window
and `--only-root` keep neither the position nor the theme; a settings file
that cannot be written is reported in the window (`Settings not saved: ...`, `Choices not saved: ...`).

The settings and the window's files sit beside the program, so the portable copy keeps them in its own folder; an update keeps
them. The first start of a version that keeps them there takes what an older version kept in `%APPDATA%\Xiox` (`settings.json`,
`window.json`, `window-theme`, `window-material`) and leaves those files where they are; the Developers options are read from the
old `settings.json` until you change one. A `pref.json` broken by hand is kept as `pref.json.bad` before a new one is written.

## Troubleshooting

**A button is greyed out.** It has a reason, and it is one of these: a job is running (wait for
it, or `Cancel`); a confirmation is open (answer it); nothing is ticked; there is no analysis yet
(Cleanup: *Analyze first: Clean works on the analysis on screen.*); or the report is open (close
it).

**A row cannot be ticked.** It is blocked: read the second line of the row, or the `blocked` line
of the detail. See [Levels and blocked entries](#levels-and-blocked-entries).

**A red strip above the list.** The last thing you asked was refused, and it says why. The ones
you are most likely to meet:

| Message | What to do |
|---|---|
| *Wait for the running job to finish.* | the app is busy; wait, or `Cancel` |
| *Close the confirmation first.* | answer the open confirmation |
| *Select something to analyze or clean first.* / *Nothing is selected in this list.* | tick something |
| *Analyze first: Clean works on the analysis on screen.* | press `Analyze` |
| *The analysis is too old: press “Analyze” again.* | analyze again (the limit is 30 minutes) |
| *Read the warnings and tick “I have read the warnings” first.* | tick the box in the confirmation |
| *Tick at least one category to look through.* | Registry: tick a kind of problem |
| *Registry backup failed (…): no repair was done.* | fix the cause (disk full, no rights) and try again |

**Registry: `n not fixed (administrator rights are needed)`.** You are running as a standard
user, so keys under `HKLM` cannot be changed. Start the production build (it asks for
administrator rights) and scan again.

**Cleanup: `n folders skipped: they need administrator rights`.** Same cause: some system folders
could not be read or cleaned as a standard user.

**The report lists Skipped or Failed items.** `Skipped` items are still there: they were in use
(close the program and run again), changed since the analysis, or protected. `Failed` ones were
refused by Windows. Analyze again to see what is left; items that are still blocked say why.

**Space was not freed after a Developers cleaning.** It went to the Recycle Bin. Empty it with
the `Recycle Bin` entry of Cleanup, or choose `Permanently` next time.

**`OpenCode sessions could not be read: some projects may be missing.`** The list of that
agent's sessions was not readable, so the projects only that agent knows are not in this
analysis. The warning names what was missing; the rest of the analysis is valid.

**`Winapp2: not downloaded yet` in Settings.** No copy of the Winapp2 rules is available yet. The
built-in entries still work; the Winapp2 entries of `Applications` appear once the rules are
there.

**The window opens in the middle, not where I left it.** The saved position did not fit your
screens any more, so it was ignored.

**Starting the app again does nothing.** There is already a window: the second start brings it
to the front. Look for it in the taskbar.

**The window went blank, or a message says it stopped working.** The component Windows uses to
draw the window (Microsoft Edge WebView2) failed. Xiox reloads the window by itself, up to three
times a minute; after that it asks: `Reload` or `Quit`. If the component stopped altogether the
message offers `Restart`, which closes Xiox and opens it again. A reload does not touch a running
job: it goes on, and the window shows it again. A restart ends it as closing the window would:
analyze again to see where things are. The same message, with `Try again`, appears if the window
could not be loaded at all.

## The command line and the observer

The desktop app and the `xiox` command line are two programs on the same engine: the same safety
rules, the same levels, the same guard that refuses protected locations. They differ in who they
are for and in what they cover.

| | Desktop app | `xiox` |
|---|---|---|
| Administrator | asks once, at start (production build) | never asks |
| Scope | the whole PC: files, registry and **every** project | **one** project at a time |
| Categories for projects | `Projects`, `Worktrees`, `Archived sessions` | builds, worktrees, copies of the repository, named folders, logs, sessions |
| Output | a window | text, or JSON for scripts and agents |
| Confirmation | a dialog | a summary and a y/N question (`--yes` skips it) |

Use the app to see and clean up after your projects as a whole; use `xiox` for one project
(`xiox analyze`, `xiox clean`), inside a script, or from an AI agent. The full guide is
[`xiox-cli.md`](xiox-cli.md).

`xiox observe` is a third program: a process that an agent harness or a script starts for one
project, which reports what has appeared since it started and never removes anything (see
[`xiox-observe.md`](xiox-observe.md)). The desktop app does not start or use observers.
