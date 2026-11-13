# Prior art: noisy path mentions -> ranked project folders -> repo roots -> clone/copy identity

## Q1. Directory-history tools (zoxide, autojump, fasd, z.sh, z.lua): frecency, aging, deleted dirs, exclusions, data stores; other folder-mention sources (atuin, VS Code, JetBrains)

### Takeaway
All mainstream "jump" tools use the same pattern: a per-path counter (rank) plus a last-access timestamp, combined at query time with a coarse recency multiplier. They bound the DB by multiplying every rank by a factor (0.9 / 0.99 / computed k) once the total passes a cap, then drop entries below 1. Deleted directories are filtered at query time and lazily pruned. Exclusions are prefix or glob lists, and `$HOME` and `/` are excluded by default. These rules port directly to a Rust ranker over path mentions mined from logs/transcripts.

### Cited Findings
**zoxide (Rust)**
- Score starts at 1 and each access adds 1. At query time it is multiplied by 4 if the last access was within 1 hour, by 2 within 1 day, divided by 2 within 1 week, and divided by 4 otherwise. — [zoxide wiki: Algorithm](https://github.com/ajeetdsouza/zoxide/wiki/Algorithm)
- Aging: when the total score exceeds `_ZO_MAXAGE`, every score is divided by a factor k so the total becomes about 90% of `_ZO_MAXAGE`. Entries with a score below 1 are removed. The entry count is "theoretically" at most 4 × `_ZO_MAXAGE`. — [zoxide wiki: Algorithm](https://github.com/ajeetdsouza/zoxide/wiki/Algorithm)
- Deleted dirs are pruned lazily, and only if they no longer exist AND are older than 90 days. — [zoxide wiki: Algorithm](https://github.com/ajeetdsouza/zoxide/wiki/Algorithm)
- `_ZO_MAXAGE` defaults to 10000. `_ZO_EXCLUDE_DIRS` is a list of glob patterns separated by `:` on Unix and `;` on Windows, and defaults to `$HOME`. `_ZO_RESOLVE_SYMLINKS=1` resolves symlinks before adding. The DB lives in `_ZO_DATA_DIR`, which defaults to `%LOCALAPPDATA%\zoxide` on Windows. — [zoxide README](https://github.com/ajeetdsouza/zoxide)
- `zoxide import` can import from autojump, z, atuin, fasd, z.lua and zsh-z, and "auto-detects the data file using each plugin's standard conventions". This is a ready-made list of extra sources of folder mentions. — [zoxide README](https://github.com/ajeetdsouza/zoxide)

**z.sh (rupa/z)**
- Formula: `frecent = int(10000 * rank * (3.75/((0.0001 * dx + 1) + 0.25)))`, where dx is the age in seconds. This is a smooth hyperbolic decay rather than buckets. — [z.sh source](https://raw.githubusercontent.com/rupa/z/master/z.sh)
- A new path gets rank 1, and each revisit adds 1 and refreshes the timestamp. When the sum of ranks exceeds `_Z_MAX_SCORE` (default 9000), every rank is multiplied by 0.99 and entries with rank < 1 are dropped. — [z.sh source](https://raw.githubusercontent.com/rupa/z/master/z.sh)
- Data file `~/.z` (override `_Z_DATA`) has the format `path|rank|epoch_seconds`. — [z.sh source](https://raw.githubusercontent.com/rupa/z/master/z.sh)
- `_Z_EXCLUDE_DIRS` uses **prefix** matching (`case "$*" in "$exclude"*)`), so it excludes whole subtrees. `$HOME` and `/` are never recorded. — [z.sh source](https://raw.githubusercontent.com/rupa/z/master/z.sh)
- Nonexistent dirs: `_z_dirs` emits only entries where `[ -d path ]`. Every add rewrites the file from that output, so missing dirs disappear on the next write. — [z.sh source](https://raw.githubusercontent.com/rupa/z/master/z.sh)

**fasd**
- Recency buckets: multiply by 6 if dx < 3600, by 4 if < 86400, by 2 if < 604800, else by 1. Score = `times * frecent(last_access)`. Recent-only mode uses `sqrt(100000/(1+t-la))`. — [fasd source](https://raw.githubusercontent.com/clvv/fasd/master/fasd)
- `_FASD_MAX` defaults to 2000. Above that, ranks are multiplied by **0.9**. `_FASD_BLACKLIST` defaults to `--help`. The data file `~/.fasd` uses the format `path|rank|time`. Paths are recorded only if `[ -e path ]`, and queries re-check existence. — [fasd source](https://raw.githubusercontent.com/clvv/fasd/master/fasd)

**autojump**
- Weight update `w = sqrt(w_old^2 + inc^2)` with a default inc of 10. This gives diminishing returns, so repeated visits grow the weight sublinearly. Decrease: `max(0, w - inc)`. — [autojump bin](https://raw.githubusercontent.com/wting/autojump/master/bin/autojump)
- `purge_missing_paths` filters entries with `os.path.exists`, and `find_matches` filters out non-existing paths and the cwd. — [autojump bin](https://raw.githubusercontent.com/wting/autojump/master/bin/autojump)
- The data file has one `weight<TAB>path` per line, and the backup is refreshed when it is older than 24h. — [autojump_data.py](https://raw.githubusercontent.com/wting/autojump/master/bin/autojump_data.py)

**Other folder-mention sources**
- atuin stores every command in SQLite (`~/.local/share/atuin/history.db`, path configurable). The `history` table has a `cwd` column, timestamps in nanoseconds, hostname, a `deleted_at` soft-delete flag, `unique(timestamp, cwd, command)`, and an index `idx_history_cwd_timestamp ... where deleted_at is null`. — [atuin-client database.rs (docs.rs)](https://docs.rs/atuin-client/latest/src/atuin_client/database.rs.html); [SANS ISC diary "Linux Shell Forensic: Let's Dive Into Atuin"](https://isc.sans.edu/diary/Linux%20Shell%20Forensic:%20Let%3Fs%20Dive%20Into%20Atuin!/33226)
- VS Code keeps recently opened folders, files and workspaces in SQLite `state.vscdb`, table `ItemTable`, key `history.recentlyOpenedPathsList`. The value is a JSON array of entries. The usual Windows location is `%APPDATA%\Code\User\globalStorage\state.vscdb`. `storage.json` (with `windowsState`) is in the same user-data area. Paths differ by version and fork (Cursor uses `.../Cursor/User/globalStorage/state.vscdb`). — [Targeting VS Code for macOS (ajpc500)](https://ajpc500.github.io/macos/Targeting-Visual-Studio-Code-For-macOS); [vscodehelper crate README](https://docs.rs/crate/vscodehelper/0.2.0/source/README.md); [Fixing VS Code forgetting recents on Windows](https://cpcom-blog.pages.dev/posts/fixing-vs-code-forgetting-recents-workspace-trust-and-github-sign-in-on-windows/). These are community sources and they disagree on exact paths.
- JetBrains IDEs: `%APPDATA%\JetBrains\<ProductVersion>\options\recentProjects.xml`, with component `RecentProjectsManager` and `<option name="recentPaths">` / `<entry>`. Older versions used `options/other.xml`. Rider uses `recentSolutions.xml`, and WebStorm may use `RecentDirectoryProjectsManager`. The IDE drops entries whose project no longer exists. — [codegenes guide](https://www.codegenes.net/blog/intellij-delete-list-of-previously-opened-projects/); [JetBrains support forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/206941345/comments/206626785). This is third-party and forum material, not official docs.

### Inferences
- A reasonable scoring for mentions mined from transcripts is zoxide-style: count each distinct mention event (deduplicated per session/command), keep the latest timestamp, and apply bucket multipliers (4/2/0.5/0.25) or z.sh's smooth decay at query time. Autojump's sqrt accumulation stops a single noisy log that repeats one path 1000× from dominating. Something like log1p(count) or sqrt-of-sum-of-squares is advisable.
- For deleted paths, follow zoxide: keep but down-rank missing paths, and prune only after a grace period (90 days). Unlike z.sh/fasd, do not hard-drop them immediately. Moved or deleted folders still carry signal for "where copies used to be".
- Exclusions should match by **path-component prefix**, as z.sh does, not plain substring. They should be case-insensitive on Windows and normalized for `\` vs `/`.
- Sources of folder mentions on Windows, beyond agent transcripts: zoxide DB, atuin `history.cwd`, VS Code `state.vscdb` recently-opened list (plus forks such as Cursor and Windsurf), JetBrains `recentProjects.xml`, and PowerShell PSReadLine history. Mentions from IDE recents are high-precision (they are already folder roots), so they deserve a higher prior weight than paths found in free text.

### Gaps
- z.lua's exact formula was not fetched (it is reportedly z.sh-compatible), and neither was the GitHub Desktop repo-list storage.
- No official Microsoft documentation was found for `state.vscdb` paths or schema. The JetBrains `additionalInfo` / `RecentProjectMetaInfo` element names were not verified.
- The PSReadLine history location (`ConsoleHost_history.txt`) is from memory, not from a source fetched in this session.

## Q2. Secret-scanner techniques transferable to path extraction

### Takeaway
Secret scanners use the same pipeline needed here. Steps: (1) path-based skip lists and .gitignore respect; (2) a cheap keyword/literal prefilter before the expensive regex; (3) one combined regex pass per file; (4) post-match validation (entropy or probabilistic randomness); (5) allowlists and stopwords on the extracted token; (6) stable fingerprints for dedup and baselines.

### Cited Findings
- gitleaks: rule `keywords` are "used for pre-regex check filtering", meaning a cheap substring check must pass before the regex runs. `entropy` is the minimum Shannon entropy of a regex group. — [gitleaks README](https://github.com/gitleaks/gitleaks)
- gitleaks allowlists: a finding is ignored if ANY allowlist matches. Criteria (`commits`, `paths`, `regexes`, `stopwords`) are OR'd by default, and `condition = "AND"` requires all of them. `regexTarget` can be `secret` (default), `match` or `line`. Stopwords apply to the extracted secret, not the whole match. — [gitleaks README](https://github.com/gitleaks/gitleaks)
- gitleaks fingerprints: each finding has a Fingerprint, and adding it to `.gitleaksignore` suppresses it (marked experimental). `--baseline-path` reports only new findings. — [gitleaks README](https://github.com/gitleaks/gitleaks)
- ripsecrets (Rust): respects `.gitignore` by default, and has `.secretsignore` (gitignore syntax plus a `[secrets]` section) and `# pragma: allowlist secret` comments. It combines all patterns into **one regex** so each file is read once, and runs it via ripgrep's engine. Instead of entropy it uses a probability test: a string is not a secret if its chance of occurring randomly is below 1 in 10,000. On the Sentry repo it reports 0.32s versus trufflehog 31.2s and detect-secrets 73.5s. — [ripsecrets README](https://github.com/sirwart/ripsecrets)

### Inferences
- Prefilter for path extraction: before running a Windows-path regex on a line, check cheaply for `:\`, `:/`, `\\` (UNC), `/c/`, `~/` or `%USERPROFILE%`, using `memchr`/`aho-corasick`. Most transcript lines have no path, so this saves most of the regex cost.
- "Validation" equivalents for paths: filesystem existence, the nearest existing ancestor, and a repo-root lookup. These replace entropy as the false-positive filter.
- Allowlist/denylist targets should mirror gitleaks' `regexTarget`. Match the normalized extracted path (default), or the whole line (e.g. ignore lines from tool-call echo of the agent's own config dir).
- Fingerprint = (normalized lowercase path, source file, line, session id) gives dedup and incremental "only new mentions since baseline" scanning.

### Gaps
- detect-secrets plugin/filter architecture and trufflehog's detector keyword prefilter were not fetched directly. Whether ripsecrets uses the `ignore` crate internally was not confirmed.

## Q3. Noise-reduction exclusion lists (Windows system/user dirs, dependency caches, agent dirs)

### Takeaway
No single curated "non-project folders" list was found. The existing tools default to excluding only `$HOME` and `/`. A tool has to compose its own denylist from well-known Windows known-folders, package-manager caches and build-output dirs. The github/gitignore templates and ripgrep's .gitignore handling are the closest curated sources.

### Cited Findings
- zoxide excludes `$HOME` by default, and z.sh never records `$HOME` or `/`. — [zoxide README](https://github.com/ajeetdsouza/zoxide); [z.sh](https://raw.githubusercontent.com/rupa/z/master/z.sh)
- ripsecrets and ripgrep-family tools respect `.gitignore` by default, so the project's own ignore rules mark build/dependency dirs (target/, node_modules/) as non-source. — [ripsecrets README](https://github.com/sirwart/ripsecrets)
- `GIT_CEILING_DIRECTORIES` exists so discovery never climbs into certain dirs, "useful for excluding slow-loading network directories". — [git(1)](https://git-scm.com/docs/git)

### Inferences (proposed list, not sourced from a single curated list)
- Windows system: `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`, `C:\ProgramData`, `C:\$Recycle.Bin`, `C:\System Volume Information`.
- Per-user: `%APPDATA%` (Roaming), `%LOCALAPPDATA%` (including `Temp`, `Packages`, `Microsoft`), `%USERPROFILE%\AppData\LocalLow`, and `%TEMP%`. Also the bare `%USERPROFILE%` itself, but not its children, mirroring zoxide's `$HOME` rule.
- Dependency/caches: `node_modules`, `.cargo\registry`, `.cargo\git`, `.rustup`, `site-packages`, `.venv`/`venv`, `__pycache__`, `target`, `.gradle`, `.m2`, `go\pkg\mod`, `.nuget\packages`, `bin`/`obj` (.NET), `dist`, `build`, `.next`, `.tox`, `.pytest_cache`.
- Agent dirs: `.claude`, `.codex`, `.cursor`, `.vscode`, `.idea`, and `.git` internals.
- Treat these as **cut points**: when a path contains such a component, truncate it to the ancestor before that component (e.g. `X:\proj\node_modules\foo` maps to `X:\proj`) rather than dropping it. Dependency paths still reveal the owning project.

### Gaps
- No source was fetched for the github/gitignore templates, ripgrep's built-in ignore defaults, or Everything's default exclusions. These remain unverified.

## Q4. Mapping a path to its repository root

### Takeaway
Walk up the ancestors looking for `.git`, which may be a directory, or a *file* containing `gitdir: <path>` for worktrees and submodules. Respect ceiling dirs. In Rust, `gix_discover::upwards_opts` does this with ceiling directories, ownership-based trust and a `dot_git_only` fast mode. For paths that no longer exist, first climb to the nearest existing ancestor. Cache results per directory so shared ancestors are resolved once.

### Cited Findings
- A `.git` *file* at the worktree root containing `gitdir: <path>` (a "gitfile") points to the real repo dir. It is used by `git submodule` and `git worktree`. — [gitrepository-layout](https://git-scm.com/docs/gitrepository-layout)
- `commondir`: if present, GIT_COMMON_DIR is taken from it (relative to $GIT_DIR). "The repository with commondir is incomplete without the repository pointed by 'commondir'." `worktrees/<id>/gitdir` holds the absolute path back to the linked worktree's `.git` file and is used to detect manually removed worktrees. `modules/` "Contains the git-repositories of the submodules". — [gitrepository-layout](https://git-scm.com/docs/gitrepository-layout)
- `GIT_CEILING_DIRECTORIES`: absolute paths Git should not chdir up into while looking for a repo. An empty entry tells Git the following entries need no symlink resolution. By default Git does not cross filesystem boundaries (`GIT_DISCOVERY_ACROSS_FILESYSTEM`). — [git(1)](https://git-scm.com/docs/git)
- gix-discover 0.57.1 (Oct 2026): `upwards` and `upwards_opts` search a dir and its parents and return the path plus an "ownership-derived trust level". `is_git` guesses the repo kind from the files present, `is_bare` checks for a bare repo, and `into_repository_and_work_tree_directories()` splits the result. — [docs.rs gix-discover](https://docs.rs/gix-discover/latest/gix_discover/)
- `upwards::Options`:
  - `trust` defaults to `Required(Reduced)`. `Required(Full)` returns only repos owned by the current user, and `Assume` skips the trust computation.
  - `ceiling_dirs` are made absolute and lexically normalized, without resolving symlinks.
  - `match_ceiling_dir_or_error` defaults to true: with ceilings set, one ceiling must contain the search dir or discovery errors.
  - `cross_fs` is Unix-only.
  - `dot_git_only` limits discovery to `.git` dirs "for better performance".
  - `current_dir` is an optimization "for processes that discover many repositories".
  
  — [docs.rs gix-discover Options](https://docs.rs/gix-discover/latest/gix_discover/upwards/struct.Options.html)

### Inferences
- Recommended algorithm:
  1. Normalize the mention: strip quotes and trailing punctuation, unify separators, expand env vars and `~`, lowercase for keys on Windows, strip a `\\?\` prefix.
  2. Find the nearest existing ancestor.
  3. Look the ancestor up in a `HashMap<PathBuf, Option<RepoRoot>>` cache. On a miss, call `gix_discover::upwards_opts` with `dot_git_only = true`, `current_dir` preset and ceiling dirs at the drive root or user profile. Record the result for every directory visited on the way up, including negative results.
  4. If the `.git` found is a gitfile, classify it as a linked worktree (gitdir under `.../.git/worktrees/<id>`) or a submodule (gitdir under `.../.git/modules/...`). Map it to the common dir for identity purposes, but keep the worktree root as the "folder".
- Set `match_ceiling_dir_or_error = false` when supplying generic ceilings. Otherwise paths on other drives would error.
- On Windows, ownership-trust checks may reduce trust for repos on external or other-user drives. Use `TrustPolicy::Assume`, or accept Reduced, since the tool only reads.

### Gaps
- No benchmark was found for gix-discover vs `git rev-parse --show-toplevel` (spawning git.exe) on Windows. Process-spawn overhead on Windows is widely reported as high, but this session found no citation.

## Q5. Repo identity / clone detection

### Takeaway
The most robust identity signal is the set of **root commits** (`git rev-list --max-parents=0 --all`), which is shared by clones and forks regardless of origin URL. Normalized origin URLs are a secondary, weaker signal. Alternates, commondir and worktree metadata reveal structural sharing (borrowed objects, linked worktrees) and should be classified separately from independent clones. Shallow clones, rewritten histories and multi-root repos need fallbacks.

### Cited Findings
- `--max-parents=0` lists root commits. `git rev-list --max-parents=0 HEAD` gives the current branch's root, and `--all` covers all refs. A repo can have several roots, for example via `git switch --orphan`. — [git rev-list docs](https://git-scm.com/docs/rev-list); [adamj.eu: Show the initial (root) commit](https://adamj.eu/tech/2023/09/30/git-show-initial-root-commit/); [ACM@UIUC TIL](https://til.acm.illinois.edu/git/history/find-the-initial-commit/)
- `git clone --local` copies HEAD, objects and refs and **hardlinks** `.git/objects` where possible, without alternates. `--shared` sets up `.git/objects/info/alternates` so the clone "starts out without any object of its own". `--reference` also uses alternates. `--dissociate` copies the borrowed objects and stops borrowing. — [git-clone](https://git-scm.com/docs/git-clone)
- `objects/info/alternates` lists alternate object stores, one per line, with paths relative to the object DB. — [gitrepository-layout](https://git-scm.com/docs/gitrepository-layout)
- The `shallow` file is maintained by shallow clones (`--depth`), similar to `info/grafts`. With `--depth`, history is truncated, which implies `--single-branch`. — [gitrepository-layout](https://git-scm.com/docs/gitrepository-layout); [git-clone](https://git-scm.com/docs/git-clone)
- ghq lays repos out as `<root>/<host>/<user>/<repo>`, derived from the remote URL. It normalizes SCP-style `git@host:path` to `ssh://git@host/path`, defaults bare `owner/project` to `github.com` (`ghq.defaultHost`), and reuses an existing clone under any configured root. Repo identity is therefore URL-based. — [ghq README](https://github.com/x-motemen/ghq)
- gix-url 0.40.1 parses git remote URLs, including the SCP-like form, and provides `Url` with `host()`/`user()`, serialization via `to_bstring`, and `expand_path`. The docs do not describe a canonicalization helper. — [docs.rs gix-url](https://docs.rs/gix-url/latest/gix_url/)

### Inferences
- Proposed decision procedure for "is folder F a clone or copy of project P":
  1. **Same object store?** If F's commondir resolves to P's git dir, F is a *linked worktree*, not a clone. If `objects/info/alternates` points into P's objects, it is a *shared/reference clone*.
  2. **Root-commit match:** compute `roots(F)` and `roots(P)` over all refs. A non-empty intersection means same lineage (clone or fork). Identical sets plus the same normalized origin means a plain clone. A shared root with a different origin means a fork, or a clone with a re-pointed remote.
  3. **Shallow:** if `.git/shallow` exists, the parentless commits are graft boundaries, not true roots. Instead, test whether any of P's commits, or the shallow boundary commits listed in `.git/shallow`, exist in the other repo's object DB. Alternatively use the normalized origin URL as the identity.
  4. **No .git (plain copy/export):** fall back to content fingerprinting, such as hashing the tracked files' blob ids from P's HEAD tree and comparing them against F's files. Or compare manifest files (Cargo.toml `name`, package.json `name`).
  5. **Has own work:** compare refs. If F's branch tips are not reachable from P (and vice versa), compute ahead/behind with a merge-base (`git rev-list --left-right --count A...B`). Uncommitted changes can be found via a status/index diff.
- Normalize origin URLs to `host/path` in lowercase. Strip `.git`, the user and the scheme, map `git@host:path` to `host/path`, and treat SSH, HTTPS and git as equivalent. Before use, check whether gix-url or the `git-url-parse` crate offers this normalization.
- Edge cases:
  - filter-branch or filter-repo rewrites change every SHA including roots. Fall back to the URL or content.
  - Multi-root repos (merged unrelated histories) should compare *sets* of roots.
  - Bare repos have no worktree and should be reported separately.
  - Submodules have their own roots and should not be conflated with the superproject.
  - Hardlinked `--local` clones are full independent clones with the same roots.
- Reading roots with gix avoids spawning git.exe: open the repo with `gix::open`, then rev-walk from all refs and keep commits with zero parents. For large repos this walk is O(history), so cache the result keyed by `(git_dir, packed-refs mtime, refs mtime)`. A cheaper approximation walks only from HEAD and the default branch.

### Gaps
- Tools such as `mr`, `gita`, `git-find-repos`, `gr` and GitHub Desktop were not researched directly in this session (budget limit). ghq's `list` scanning markers were not documented on its README.
- No measured comparison of gix rev-walk vs `git rev-list` process spawn on Windows was found.
- The `git-url-parse` crate API and normalization behavior were not verified.
- The exact gix API names for the rev-walk and root filtering (e.g. `repo.rev_walk(...)`) were not fetched from docs.rs. They should be verified before implementation.
