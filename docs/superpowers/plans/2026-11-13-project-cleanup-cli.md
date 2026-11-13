# Project Cleanup CLI (`xiox analyze` / `xiox clean`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Given one project, list everything on the PC that belongs to it (builds, worktrees, copies, name-matched folders, logs, archived sessions) and remove only what the command-line flags name.

**Architecture:** A new library crate `dc-project` does the targeted analysis and the cleaning on top of the existing engine crates (`dc-git`, `dc-engine`, `dc-scanner`, `dc-cleaner`, `dc-app`). A new binary crate `xiox-cli` (binary `xiox`, console subsystem, `asInvoker`) holds only argument parsing, printing and the confirmation prompt. The egui app is not touched.

**Tech Stack:** Rust 2024, the workspace's existing crates, `serde`/`serde_json` (already in the build), `embed-resource` (already a build dependency of the root package), `tempfile` for tests. No new runtime dependency.

**Spec:** `docs/superpowers/specs/2026-11-13-project-cleanup-cli-design.md` (Italian). Read it before any task.

## Global Constraints

- **English only** in every string the user can see: command names, flags, help, output, JSON field names, error messages. No Italian reaches stdout/stderr. The engine's Italian strings (`ProjectReport::reason`, `Artifact::note`, `Artifact::label`, `ArchivedSession::blocked`, `DevScan::warnings`) are never printed: build English text from structured fields.
- `cargo` always with `-j 1` and `CARGO_INCREMENTAL=0`; never two cargo processes at once (other sessions build on this machine: if `cargo`/`rustc` is already running, wait).
- Never run the analysis or the cleaning on the user's real projects or sessions. Tests build fake trees and fake git repositories under `tempfile::tempdir()`.
- The product rule: the app proposes, the user chooses. `clean` removes nothing a flag does not name. Nothing is hidden: blocked and `caution` items are listed.
- Safety equal to today: no link is followed, every item is re-validated right before removal, worktrees go through `PlanAction::GitWorktreeRemove` (never `--force`), protected locations stay refused (`dc_cleaner::guard`).
- `xiox` never asks for elevation. `analyze` writes nothing except the session listing cache the engine already writes (`dc_app::cache_dir()`).
- Do not change the behaviour of the egui app (`src/`), and do not touch `vendor/mukade`.
- Commits: only the files the task names, message in Italian like the rest of the history (`feat(xiox): …`), ending with the attribution lines the dispatcher gives.

## Review Focus

1. A folder whose name starts with the project's name but belongs to someone else (`app-data` next to project `app`, `mukade-chunked` being another repo): must be `caution`, never taken by a category flag. → Task 3 tests.
2. A worktree with uncommitted changes, or whose HEAD is reachable from no other ref: must be blocked, and `--include` must not override. → Task 2 and Task 5 tests.
3. The item changes between analysis and removal (folder replaced by a junction, worktree becomes dirty): removal must skip it and exit 3. → Task 5 tests.
4. Path given is a subfolder, a linked worktree, a non-repository folder, or does not exist: the project resolves to the main repository / the folder itself / exit 1 with a clear message. → Task 2 and Task 6 tests.
5. `--json` with anything else on stdout, or `clean --json` without `--yes`: stdout must be exactly one JSON object and nothing is deleted. → Task 6 and Task 7 tests.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/dc-git/src/project.rs` (new) + `lib.rs` re-export | git queries the targeted analysis needs |
| `crates/dc-project/Cargo.toml`, `src/lib.rs` | public types and `analyze_project` |
| `crates/dc-project/src/resolve.rs` | from a path to the project identity |
| `crates/dc-project/src/owned.rs` | builds, worktrees, sessions (engine reuse) |
| `crates/dc-project/src/search.rs` | copies and name-matched folders on the drives |
| `crates/dc-project/src/logs.rs` | log/temp files and stale branches |
| `crates/dc-project/src/clean.rs` | selection, re-validation, removal, outcome |
| `crates/dc-project/src/tests/` | one test module per source file, shared `fixtures.rs` |
| `crates/dc-model/src/lib.rs`, `crates/dc-cleaner/src/guard.rs` | new `ArtifactKind::ProjectFolder` |
| `crates/xiox-cli/Cargo.toml`, `build.rs`, `xiox.rc`, `xiox.manifest` | binary `xiox`, `asInvoker` |
| `crates/xiox-cli/src/main.rs`, `args.rs`, `render.rs`, `json.rs`, `run.rs` | CLI |
| `docs/xiox-cli.md`, `README.md` | user guide (English) |

---

### Task 1: git queries for a project (`dc-git`)

**Files:**
- Create: `crates/dc-git/src/project.rs`
- Modify: `crates/dc-git/src/lib.rs` (`mod project; pub use project::*;`; make the private `git()` helper and `resolve()` visible to the module with `pub(crate)`)
- Test: in `crates/dc-git/src/project.rs` (`#[cfg(test)] mod tests`), using real `git` in temp dirs like the existing tests at `crates/dc-git/src/lib.rs:540`

**Interfaces:**
- Produces:

```rust
pub struct WorktreeEntry {
    pub path: PathBuf,          // normalized (dc_model::normalize_path)
    pub head: Option<String>,   // full sha
    pub branch: Option<String>, // short name; None when detached
    pub is_main: bool,          // first entry of the list
    pub prunable: bool,         // git marks it prunable, or the folder does not exist
}
/// Main working tree of the repository `path` is in (`path` may be a subfolder or a linked worktree).
pub fn repo_main_root(path: &Path) -> Option<PathBuf>;
/// `git worktree list --porcelain -z` from `repo` (fallback without `-z` as `main_worktree` does).
pub fn list_worktrees(repo: &Path) -> Option<Vec<WorktreeEntry>>;
/// `git rev-list --max-parents=0 HEAD`, sorted. Empty when there is no commit.
pub fn root_commits(repo: &Path) -> Vec<String>;
pub fn origin_url(repo: &Path) -> Option<String>;
/// Lowercase, no trailing `.git` or `/`, `git@host:a/b` and `ssh://git@host/a/b` and `https://host/a/b` -> `host/a/b`.
pub fn normalize_remote(url: &str) -> String;
/// Some(true) when HEAD of `worktree` is contained in a remote branch or in a local branch other than
/// the one checked out there (`git branch -a --contains HEAD`); None when git cannot answer or no HEAD.
pub fn head_kept_elsewhere(worktree: &Path) -> Option<bool>;
/// Untracked files relative to `root`: `ignored = true` -> `git ls-files -z --others --ignored --exclude-standard`,
/// `false` -> `git ls-files -z --others --exclude-standard`. None when not a repository.
pub fn untracked_files(root: &Path, ignored: bool) -> Option<Vec<PathBuf>>;
pub struct BranchInfo { pub name: String, pub merged: bool, pub worktree_missing: bool }
/// Local branches other than the main worktree's current one that are merged into it
/// (`git branch --merged`) or checked out in a prunable worktree.
pub fn stale_branches(repo: &Path) -> Vec<BranchInfo>;
```

- [ ] **Step 1: Write the failing tests** (each creates repos with `git init`, `git commit --allow-empty`, `git worktree add`):

```rust
#[test] fn main_root_from_subfolder_and_from_linked_worktree()   // both give the main repo root
#[test] fn worktrees_listed_with_main_first_branch_and_detached() // 3 entries: main, branch `feat`, detached
#[test] fn worktree_whose_folder_was_deleted_is_prunable()
#[test] fn root_commits_equal_for_a_clone_and_differ_for_an_unrelated_repo()
#[test] fn normalize_remote_forms() {
    for u in ["git@GitHub.com:Acme/Mukade.git", "https://github.com/acme/mukade/", "ssh://git@github.com/acme/mukade.git"] {
        assert_eq!(normalize_remote(u), "github.com/acme/mukade");
    }
}
#[test] fn head_kept_elsewhere_true_for_detached_at_main_false_for_a_new_commit_on_detached()
#[test] fn head_kept_elsewhere_false_for_a_branch_with_its_own_unmerged_commit()
#[test] fn untracked_files_split_ignored_from_not_ignored() // `a.log` ignored by .gitignore, `b.tmp` not
#[test] fn stale_branches_lists_merged_and_skips_unmerged()
#[test] fn not_a_repository_gives_none_everywhere()
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p dc-git project::` — FAIL (functions missing).
- [ ] **Step 3:** Implement the functions with the crate's existing `git(root, args)` helper (it already sets literal pathspecs, timeout and `CREATE_NO_WINDOW`). `head_kept_elsewhere`: parse `git branch -a --contains HEAD --format=%(refname)`; true if any `refs/remotes/*`, or any `refs/heads/X` where X is not the branch checked out in this worktree.
- [ ] **Step 4:** Run the same command — PASS; then `cargo test -j 1 -p dc-git` — all green.
- [ ] **Step 5:** Commit `crates/dc-git/src/project.rs crates/dc-git/src/lib.rs` — `feat(dc-git): elenco dei worktree e identità del repository`.

---

### Task 2: `dc-project` — model, project resolution, builds, worktrees, sessions

**Files:**
- Create: `crates/dc-project/Cargo.toml` (deps: `dc-model`, `dc-git`, `dc-engine`, `dc-scanner`, `dc-app`, `dc-mukade`, `dc-cleaner`, `dc-disktree`, `serde`, `serde_json`; dev: `tempfile = "3.27.0"`), `src/lib.rs`, `src/resolve.rs`, `src/owned.rs`, `src/tests/mod.rs`, `src/tests/fixtures.rs`, `src/tests/owned.rs`
- (The workspace `members = [".", "crates/*"]` picks the crate up.)

**Interfaces:**
- Consumes: Task 1 functions; `dc_engine::analyze(cfg, scanner, index, sources, git)`; `dc_app::load_sessions`, `dc_app::cache_dir`, `dc_app::now_ms`; `dc_model::{path_key, normalize_path, Level, RemoveMode, Agent}`; `dc_mukade::ArchivedSession`.
- Produces (all `Serialize`, `#[serde(rename_all = "camelCase")]`, enums `kebab-case`):

```rust
pub enum Category { Builds, Worktrees, Copies, Named, Logs, Sessions }
pub enum Block { LiveSession, DirtyGit, UnpushedCommits, Unreachable, SessionLocked, Unknown }
pub enum Why {                                   // why the item belongs to the project
    CatalogFolder { label_en: String },          // English label from ArtifactKind/Class, e.g. "Rust build"
    LinkedWorktree { branch: Option<String>, head: Option<String> },
    SameRepository { shared_root_commit: bool, same_origin: bool, has_own_work: Option<bool> },
    NameOnly,
    LogFile { ignored: bool },
    ArchivedSession { agent: Agent, title: Option<String> },
}
pub enum Target {
    Dir { kind: dc_model::ArtifactKind },
    File,
    Worktree { repo: PathBuf },
    Session { agent: Agent, id: String },
}
pub struct Item {
    pub key: String,            // path_key(path); for sessions "session:<agent>:<id>"
    pub category: Category,
    pub path: Option<PathBuf>,  // None for sessions
    pub bytes: Option<u64>,     // None when not measured (light mode)
    pub level: Level,           // Recommended / Optional / Caution
    pub blocked: Vec<Block>,    // non-empty = cannot be removed at all
    pub why: Why,
    pub target: Target,
}
pub struct Project { pub root: PathBuf, pub name: String, pub is_repo: bool, pub origin: Option<String>, pub worktrees: Vec<PathBuf> }
pub enum SearchMode { MftIndex, BoundedWalk, ParentOnly }
pub enum Note { NotARepository, GitMissing, NameTooShort, WalkMayMissDeepFolders, PrunableWorktree { path: PathBuf }, EngineWarning }
pub struct StaleBranch { pub name: String, pub merged: bool, pub worktree_missing: bool }
pub struct ProjectScan {
    pub schema: u32,            // 1
    pub project: Project,
    pub items: Vec<Item>,       // sorted by category order above, then bytes descending
    pub branches: Vec<StaleBranch>,
    pub notes: Vec<Note>,
    pub search: SearchMode,
    pub scanned_at_ms: i64,
    pub elapsed_ms: u64,
}
pub struct Options { pub drives: bool /* default true */, pub measure: bool /* default true */, pub admin: bool }
pub enum Error { PathNotFound(PathBuf), NotADirectory(PathBuf) }   // Display in English
pub fn resolve_project(path: &Path) -> Result<Project, Error>;
pub fn analyze_project(path: &Path, opts: &Options) -> Result<ProjectScan, Error>;
// test seam: everything outside the fake tree is injected
pub(crate) struct Env<'a> { pub sessions: &'a [dc_app::Preloaded], pub archived: &'a [dc_mukade::ArchivedSession], pub search_roots: Option<Vec<PathBuf>>, pub now_ms: i64 }
pub(crate) fn analyze_with(path: &Path, opts: &Options, env: &Env) -> Result<ProjectScan, Error>;
```

Rules this task implements (spec section «Cosa appartiene a un progetto»):
- **Project:** `repo_main_root(path)` if git answers, else `path` itself (`Note::NotARepository`, or `GitMissing` when `!git_available()`). `name` = last component of the root.
- **Builds:** run `dc_engine::analyze` with `roots` = project root + every non-prunable worktree, `container_roots` empty, `inactive_days` from `dc_app::default_settings()`, `ScanOptions::default()`, `dc_scanner::DefaultScanner`, no index, the injected sessions. One `Item` per artifact of the reports whose root is one of those roots. `level` = the artifact's level **recomputed as if the project were inactive** (project age does not gate here): take `artifact.class.base_level()`, then `Caution` if `!artifact.verified` or (`artifact.ambiguous` and git does not ignore it) or git tracks it — the same conditions as `dc_engine::assess` (`crates/dc-engine/src/lib.rs:467`); read that function and mirror it, do not change it. `blocked` = `LiveSession` when the report's status is `InUse`; `Unreachable` when `Unreachable`.
- **Worktrees:** every `WorktreeEntry` that is not main. Prunable → `Note::PrunableWorktree`, no item. Otherwise `Target::Worktree { repo: project.root }`, bytes = measured size of the folder, blocked: `DirtyGit` if `GitCli.state(path).dirty != Some(false)`; `UnpushedCommits` if `head_kept_elsewhere(path) != Some(true)`; `LiveSession` if the engine report for that root is `InUse`. Level `Recommended` when not blocked, else `Caution`.
- **Sessions:** archived sessions whose `cwd` key equals or is under the project root or one of its worktrees. `blocked = [SessionLocked]` when `ArchivedSession::blocked.is_some()`. Level `Optional`.
- `opts.measure == false`: no folder is measured; `bytes` is `None` everywhere except sessions.

- [ ] **Step 1: Write the failing tests** in `src/tests/owned.rs` with `fixtures.rs` helpers `repo(dir) -> PathBuf`, `commit(repo, file)`, `worktree(repo, dest, branch)`, `fill(dir, files, bytes)`, `env()` (empty sessions):

```rust
#[test] fn subfolder_and_linked_worktree_resolve_to_the_main_repository()
#[test] fn plain_folder_is_its_own_project_with_a_not_a_repository_note()
#[test] fn missing_path_is_path_not_found()
#[test] fn rust_target_and_node_modules_are_builds_recommended_even_if_the_project_was_used_today()
    // Cargo.toml + target/CACHEDIR.TAG ; a session with last activity = now in env.sessions
#[test] fn builds_inside_a_linked_worktree_are_listed_too()
#[test] fn clean_worktree_at_a_commit_main_has_is_recommended_and_not_blocked()
#[test] fn dirty_worktree_is_blocked_dirty_git()
#[test] fn worktree_with_a_commit_nobody_else_has_is_blocked_unpushed_commits()
#[test] fn deleted_worktree_folder_gives_a_prunable_note_and_no_item()
#[test] fn archived_session_started_in_a_worktree_belongs_to_the_project_and_one_elsewhere_does_not()
#[test] fn light_mode_measures_nothing()            // opts.measure = false -> every non-session bytes is None
#[test] fn items_are_ordered_by_category_then_size()
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p dc-project` — FAIL.
- [ ] **Step 3:** Implement. `analyze_project` builds the real `Env` (`dc_app::load_sessions(Some(cache_dir()))`, archived sessions as `dc_app::run_scan` loads them — expose the private `load_archived` of `crates/dc-app/src/lib.rs` as `pub fn load_archived_sessions(cache: Option<PathBuf>) -> (Vec<ArchivedSession>, Vec<String>)` if needed, without changing its behaviour) and calls `analyze_with`. Engine warnings are not copied (they are Italian): one `Note::EngineWarning` if any.
- [ ] **Step 4:** Run — PASS.
- [ ] **Step 5:** Commit the new crate (and `crates/dc-app/src/lib.rs` if touched) — `feat(dc-project): analisi mirata, build worktree e sessioni di un progetto`.

---

### Task 3: `dc-project` — copies and name-matched folders

**Files:**
- Create: `crates/dc-project/src/search.rs`, `src/tests/search.rs`
- Modify: `crates/dc-project/src/lib.rs` (call it from `analyze_with`)

**Interfaces:**
- Consumes: Task 1 (`root_commits`, `origin_url`, `normalize_remote`, `GitCli.state`), Task 2 types, `dc_model::{is_system_dir, DirIndex}`, `dc_disktree::{fixed_drives, build_for_roots}`.
- Produces:

```rust
/// True when `folder` is `project` + one of `-` `_` `.` ` ` + at least one more character, case-insensitive.
pub(crate) fn name_matches_project(project: &str, folder: &str) -> bool;
/// Candidate folders outside the project, by name only. `roots`: (root, max_depth).
pub(crate) fn walk_candidates(project: &Project, roots: &[(PathBuf, usize)]) -> Vec<PathBuf>;
pub(crate) fn classify(project: &Project, project_roots: &[String], project_origin: Option<&str>, candidate: &Path) -> Option<(Category, Why)>;
```

Rules:
- Project name shorter than 4 characters → no search, `Note::NameTooShort`.
- Where: `opts.drives == false` → the project's parent only, depth 1 (`SearchMode::ParentOnly`). Otherwise, with `opts.admin` → MFT index over the NTFS fixed drives, as `dc_app::run_scan` builds it, and `DirIndex::find_dirs_named` cannot do prefix search: list candidates from the index if it offers a way, **otherwise fall back to the walk** — decide by reading `crates/dc-disktree` and say which in the report. Without admin → bounded walk: project's parent depth 1, every fixed drive root depth 3, the user's home depth 3 (`SearchMode::BoundedWalk` + `Note::WalkMayMissDeepFolders`). `env.search_roots` replaces the drives and the home in tests.
- Never descend into, or accept: system folders (`is_system_dir`), any `AppData`, the project and its worktrees, a folder that is a git repository (it is tested as a candidate itself but not descended into), links (symlink/junction: never followed, never a candidate).
- A candidate that is a git repository (not a registered worktree) sharing a root commit with the project **or** with the same normalized origin → `Category::Copies`, `Why::SameRepository { .. , has_own_work }` where `has_own_work = Some(true)` if it is dirty or has commits the project's repository does not contain (`git -C <project> cat-file -e <copy HEAD>` fails), `Some(false)` if neither, `None` if git cannot tell. Repositories are found by identity **even when the name does not match** only inside the walk roots at the same depths.
- A candidate matching by name and not a copy and not a registered worktree → `Category::Named`, `Why::NameOnly`.
- Both categories: `level = Caution`, `Target::Dir { kind: ArtifactKind::ProjectFolder }` (the variant is added in Task 5; until then use `ArtifactKind::Other`), `blocked` empty.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn name_rule() {
    assert!(name_matches_project("mukade", "mukade-test-scratch"));
    assert!(name_matches_project("mukade", "Mukade_bench"));
    assert!(name_matches_project("mukade", "mukade.bak"));
    assert!(name_matches_project("mukade", "mukade old"));
    assert!(!name_matches_project("mukade", "mukade"));
    assert!(!name_matches_project("mukade", "mukade-"));
    assert!(!name_matches_project("mukade", "mukadesk"));
    assert!(!name_matches_project("mukade", "my-mukade-x"));
}
#[test] fn sibling_folder_with_the_project_name_is_named_caution()
#[test] fn clone_elsewhere_is_a_copy_even_with_another_name()            // git clone into search root as "zzz"
#[test] fn clone_with_an_extra_commit_has_own_work_true_and_plain_clone_false()
#[test] fn unrelated_repository_with_a_matching_name_is_named_not_copy()
#[test] fn registered_worktree_is_never_listed_as_copy_or_named()
#[test] fn nothing_is_taken_from_system_folders_appdata_or_inside_another_repository()
#[test] fn junction_with_a_matching_name_is_ignored()                    // std::os::windows::fs::symlink_dir or `mklink /J`; skip with a printed reason if not permitted
#[test] fn deeper_than_the_depth_limit_is_not_found()
#[test] fn three_letter_project_name_searches_nothing_and_says_so()
#[test] fn no_drives_option_looks_only_at_the_parent()
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p dc-project search` — FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -j 1 -p dc-project` — PASS.
- [ ] **Step 5:** Commit `crates/dc-project/src/search.rs src/tests/search.rs src/lib.rs src/tests/mod.rs` — `feat(dc-project): copie del repository e cartelle con il nome del progetto`.

---

### Task 4: `dc-project` — log/temp files and stale branches

**Files:**
- Create: `crates/dc-project/src/logs.rs`, `src/tests/logs.rs`
- Modify: `crates/dc-project/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 `untracked_files`, `stale_branches`; Task 2 types.
- Produces: `pub(crate) fn log_items(project: &Project, build_dirs: &[PathBuf]) -> Vec<Item>`; `ProjectScan::branches` filled.

Rules: files (not directories, not links) with extension `log`, `tmp`, `bak`, `dmp` (case-insensitive), size ≥ 1 MiB (1_048_576), under the project root or a worktree, **not** under any folder already listed in `builds`, that git reports as untracked. Ignored → `Recommended`, `Why::LogFile { ignored: true }`; untracked but not ignored → `Caution`, `ignored: false`. `Target::File`. Two git calls per root (ignored, not ignored), not one per file. Not a repository → no log items.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn ignored_big_log_is_recommended_and_untracked_big_tmp_is_caution()
#[test] fn tracked_log_and_small_log_are_not_listed()        // 1 MiB - 1 byte is not listed, exactly 1 MiB is
#[test] fn log_inside_a_build_folder_is_not_listed_twice()   // target/x.log
#[test] fn log_in_a_linked_worktree_is_listed()
#[test] fn plain_folder_has_no_log_items()
#[test] fn merged_branch_is_reported_as_stale_and_unmerged_is_not()
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p dc-project logs` — FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -j 1 -p dc-project` — PASS.
- [ ] **Step 5:** Commit — `feat(dc-project): log, file temporanei e branch vecchi`.

---

### Task 5: `dc-project` — selection, re-validation, removal

**Files:**
- Create: `crates/dc-project/src/clean.rs`, `src/tests/clean.rs`
- Modify: `crates/dc-project/src/lib.rs`, `src/search.rs` (use the new kind), `crates/dc-model/src/lib.rs` (`ArtifactKind::ProjectFolder`), `crates/dc-cleaner/src/guard.rs` (`name_plausible` accepts any name for `ProjectFolder`; every other check unchanged), plus any exhaustive `match` on `ArtifactKind` the compiler points at (give it the same arm as `Other`)

**Interfaces:**
- Consumes: Tasks 2–4; `dc_cleaner::{ParallelRemover, worker_count, remove_files_parallel, Cancel, Tick}`; `dc_cleaner::guard::validate`; `dc_app::{revalidate_archived, delete_archived, recycle_shortfall}`; `dc_model::{CleanPlan, PlanItem, PlanAction, RemoveMode, ItemOutcome}`.
- Produces:

```rust
pub struct CleanRequest { pub categories: Vec<Category>, pub include: Vec<PathBuf>, pub mode: RemoveMode }
pub enum SelectError {
    NotInAnalysis(PathBuf),                 // --include of a path the analysis did not produce
    Blocked { path: PathBuf, why: Vec<Block> },
    CategoryNeedsInclude(Category),         // Copies / Named passed as a category
}
pub struct Chosen { pub keys: Vec<String>, pub bytes: u64, pub permanent_anyway: Vec<String> /* worktree keys */ }
/// Category flags take the `Recommended`, unblocked items (for Sessions: every unblocked one).
/// `include` adds any unblocked item of any level except sessions.
pub fn select(scan: &ProjectScan, req: &CleanRequest) -> Result<Chosen, SelectError>;
pub enum Outcome { Removed { bytes_freed: u64 }, Skipped { reason: String }, Failed { error: String } }
pub struct CleanOutcome {
    pub schema: u32, pub mode: RemoveMode, pub dry_run: bool,
    pub items: Vec<(String /*key*/, Option<PathBuf>, Category, Outcome)>,
    pub bytes_freed: u64, pub recycled_bytes: u64, pub elapsed_ms: u64,
}
impl CleanOutcome { pub fn partial(&self) -> bool; }   // any Skipped or Failed
/// Volumes whose Recycle Bin cannot take the chosen items: (drive, bytes moved, room).
pub fn recycle_shortfall(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode) -> Vec<(char, u64, u64)>;
pub fn clean(scan: &ProjectScan, chosen: &Chosen, mode: RemoveMode, dry_run: bool) -> CleanOutcome;
```

Rules:
- `clean` first re-runs the analysis for the project in light mode and keeps only the chosen keys that are still present, in the same category, and unblocked; the others become `Skipped { reason }` (English: "no longer there", "now blocked: uncommitted changes", …).
- Dirs and worktrees → one `CleanPlan` (`RemoveDir` / `GitWorktreeRemove { repo }`), each item through `guard::validate` (a refusal is `Skipped` with the guard's reason), executed with `ParallelRemover { workers: worker_count() }.execute_with(&plan, dry_run, &|_| {}, &Cancel::default())` (check the real signature in `crates/dc-cleaner/src/parallel.rs`). A `ProjectFolder` item is additionally refused if its key contains or is contained in the project root or any worktree.
- Files: `symlink_metadata` says regular file, the parent is validated like a directory item's parent (reuse the guard's protected-location check; expose a small `pub fn validate_file(path:&Path)->Result<PathBuf,String>` in `guard.rs` if nothing fits), then Recycle Bin or permanent delete according to `mode` — read how «Pulizia file» does both (`remove_files_parallel` and the recycle path in `crates/dc-cleaner/src/lib.rs`) and reuse it.
- Sessions: `dc_app::revalidate_archived` then `dc_app::delete_archived` exactly as `src/dev.rs` calls them; read that call site first. Map each `SessionOutcome` to an `Outcome`.
- Worktrees are removed permanently in both modes; `Chosen::permanent_anyway` lists them so the CLI can say so.
- `dry_run`: every check runs, nothing is touched, outcomes are `Removed { bytes_freed: expected }`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn no_category_and_no_include_chooses_nothing()
#[test] fn builds_flag_takes_recommended_builds_and_leaves_caution_ones()
#[test] fn copies_or_named_as_a_category_is_refused()                       // CategoryNeedsInclude
#[test] fn include_takes_a_named_folder_and_a_caution_build()
#[test] fn include_of_a_path_not_in_the_analysis_is_an_error_and_nothing_is_chosen()
#[test] fn include_of_a_dirty_worktree_is_refused_as_blocked()
#[test] fn sessions_flag_takes_unblocked_sessions_only()
#[test] fn clean_permanent_removes_the_build_folder_and_reports_its_bytes()
#[test] fn clean_removes_a_clean_worktree_with_git_and_the_main_repo_no_longer_lists_it()
#[test] fn worktree_that_became_dirty_after_the_analysis_is_skipped_and_outcome_is_partial()
#[test] fn folder_replaced_by_a_junction_after_the_analysis_is_skipped_and_the_target_survives()
#[test] fn log_file_is_removed_and_a_file_that_vanished_is_skipped()
#[test] fn dry_run_touches_nothing_and_reports_what_it_would_free()
#[test] fn guard_still_refuses_a_project_folder_in_a_protected_location()    // unit test in dc-cleaner guard tests
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p dc-project clean` — FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -j 1 -p dc-project`, then `cargo test -j 1 -p dc-cleaner -p dc-model -p dc-engine -p dc-app` — PASS, no warning.
- [ ] **Step 5:** Commit the files above — `feat(dc-project): pulizia mirata con riconvalida prima di cancellare`.

---

### Task 6: `xiox` binary — arguments and `analyze`

**Files:**
- Create: `crates/xiox-cli/Cargo.toml` (`[[bin]] name = "xiox"`, deps `dc-project`, `dc-model`, `serde_json`; build-dep `embed-resource` at the root package's version), `build.rs`, `xiox.rc`, `xiox.manifest` (`asInvoker`, copy `PulisciPC.dev.manifest` and its `.rc`), `src/main.rs`, `src/args.rs`, `src/render.rs`, `src/json.rs`, `src/run.rs`
- Test: unit tests in `args.rs`, `render.rs`, `json.rs`; `crates/xiox-cli/tests/cli.rs` runs the built binary (`env!("CARGO_BIN_EXE_xiox")`) on fake trees with `XIOX_SEARCH_ROOTS` set

**Interfaces:**
- Consumes: Task 2 `analyze_project`, `Options`, `ProjectScan`; Task 5 types (for the parser only).
- Produces:

```rust
pub enum Command {
    Analyze { project: Option<PathBuf>, json: bool, no_drives: bool },
    Clean { project: Option<PathBuf>, categories: Vec<Category>, include: Vec<PathBuf>,
            permanent: bool, yes: bool, dry_run: bool, json: bool, no_drives: bool },
    Help, Version,
}
pub fn parse(args: &[String]) -> Result<Command, String>;      // Err = usage message, exit 2
pub fn render_scan(scan: &ProjectScan) -> String;               // human text
pub fn scan_json(scan: &ProjectScan) -> serde_json::Value;      // {"schema":1,"project":…,"items":[…],…}
pub const EXIT_OK: i32 = 0; pub const EXIT_ERROR: i32 = 1; pub const EXIT_USAGE: i32 = 2; pub const EXIT_PARTIAL: i32 = 3;
```

Decisions:
- Hand-written parser, no dependency. Flags: `--json`, `--no-drives`, `--builds`, `--worktrees`, `--logs`, `--sessions`, `--include <path>` (repeatable), `--permanent`, `--yes`, `--dry-run`, `--help`/`-h`, `--version`/`-V`. `--copies` and `--named` are rejected with: `--copies/--named do not exist: name each folder with --include <path>`. Unknown flag, a second positional, or a missing value → usage error.
- No project argument → current directory.
- Elevation: detect it (move or copy the check from `src/sys.rs::is_elevated` into `dc-project` as `pub fn is_elevated() -> bool`; the egui app keeps its own) and pass `Options.admin`.
- `XIOX_SEARCH_ROOTS` (paths separated by `;`), read only in debug builds (`cfg!(debug_assertions)`), replaces the drives and the home — the tests need it; expose `dc_project::analyze_project_in(path, opts, search_roots)` for that.
- Human output: header `Project  <name>   <root>   (git, origin <normalized>)` or `(not a git repository)`; `Search   <all fixed drives, MFT index | fixed drives to 3 levels (not elevated: deeper folders may be missed) | project's parent only>`; one block per non-empty category: title in capitals and total right-aligned, then `level  size  path  reason`, blocked items shown with `blocked` in place of the level and the reason (`uncommitted changes`, `commits not kept anywhere else`, `an agent session is open`, `session locked by its harness`); a `BRANCHES (not removed)` block; notes; `Total <size> · recommended <size>`; last line `Clean with: xiox clean "<root>" <flags for the non-empty recommended categories>` only when something is recommended. Sizes with `dc_model::fmt_size` if its output is English, otherwise an own formatter (`6.0 GB`, `265 MB`, binary units, one decimal for GB).
- `--json`: stdout is exactly `scan_json` + newline; everything else to stderr.
- Progress and errors to stderr. Path not found → `error: path not found: <path>`, exit 1.

- [ ] **Step 1: Write the failing tests:**

```rust
// args.rs
#[test] fn analyze_defaults()                 // ["analyze"] -> Analyze{project:None,json:false,no_drives:false}
#[test] fn clean_with_everything()            // all flags + two --include
#[test] fn copies_flag_explains_include()     // Err contains "--include <path>"
#[test] fn unknown_flag_missing_value_second_positional_and_no_command_are_usage_errors()
// render.rs / json.rs  (build a ProjectScan by hand)
#[test] fn text_has_one_block_per_non_empty_category_and_the_clean_hint()
#[test] fn blocked_worktree_shows_blocked_and_its_reason_and_is_not_in_the_hint()
#[test] fn no_recommended_item_means_no_clean_hint()
#[test] fn json_has_schema_1_and_camel_case_kebab_case_fields()   // "schema":1, "category":"worktrees", "level":"recommended"
#[test] fn no_italian_in_the_output()         // none of: "Consigliato","Attenzione","cartella","sessione","Pulisci"
// tests/cli.rs
#[test] fn analyze_on_a_fake_project_exits_0_and_lists_its_target_folder()
#[test] fn analyze_json_stdout_is_one_json_object()               // serde_json::from_slice(stdout) ok
#[test] fn analyze_missing_path_exits_1_and_bad_flag_exits_2()
#[test] fn help_and_version_exit_0()
```

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli` — FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — PASS. Also `cargo build -j 1 -p xiox-cli` and check `target/debug/xiox.exe --version` prints `xiox <version>`.
- [ ] **Step 5:** Commit `crates/xiox-cli` (+ `crates/dc-project/src/lib.rs` if `is_elevated`/`analyze_project_in` were added, + `Cargo.lock`) — `feat(xiox): comando analyze`.

---

### Task 7: `xiox clean`

**Files:**
- Modify: `crates/xiox-cli/src/run.rs`, `src/render.rs`, `src/json.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: Task 5 `select`, `clean`, `recycle_shortfall`, `CleanOutcome`; Task 6 `Command::Clean`, exit codes.
- Produces: `pub fn render_summary(scan, chosen, mode, shortfall) -> String`, `pub fn render_outcome(o: &CleanOutcome) -> String`, `pub fn outcome_json(scan, o) -> serde_json::Value`.

Behaviour, in order:
1. Analyse. No category and no `--include` → print the analysis exactly as `analyze` does, then `Nothing selected. Name what to clean: --builds --worktrees --logs --sessions --include <path>`; exit 0.
2. `select`; a `SelectError` → message, exit 2, nothing removed. Nothing chosen (flags named but no eligible item) → `Nothing to clean.`, exit 0.
3. Summary: items by category with sizes, total, `to the Recycle Bin` or `permanently`; if worktrees are chosen without `--permanent`: `Worktrees are removed by git and do not go to the Recycle Bin.`; if `recycle_shortfall` is non-empty: `Recycle Bin on <X>: is too small (<moved> to move, <room> free). Use --permanent, or empty the bin.` and exit 1 unless `--permanent`.
4. `--dry-run` → run `clean(.., dry_run = true)`, print `Dry run: nothing was removed.` + the outcome; exit 0.
5. Confirmation: with `--yes` go on. Without it: `--json` → exit 2 with `--json needs --yes to remove anything`; stdin not a terminal (`std::io::IsTerminal`) → exit 2 with `not a terminal: pass --yes to remove without asking`; otherwise ask `Remove <n> items (<size>)? [y/N] ` on stderr and continue only on `y`/`yes` (case-insensitive); anything else → `Cancelled.`, exit 0.
6. `clean`; print the outcome (removed with bytes, skipped/failed with reason, `Freed <size>` and, in Recycle mode, `(<size> in the Recycle Bin: freed when it is emptied)`). Exit 3 if `outcome.partial()`, else 0.
7. `--json`: stdout is exactly one object `{"schema":1,"scan":…,"outcome":…}` (`"outcome": null` when nothing was run).

- [ ] **Step 1: Write the failing tests** (`tests/cli.rs`, binary on fake trees; stdin is not a terminal under the test harness):

```rust
#[test] fn clean_without_flags_lists_and_removes_nothing()
#[test] fn clean_builds_yes_permanent_removes_target_and_exits_0()
#[test] fn clean_without_yes_and_without_a_terminal_exits_2_and_removes_nothing()
#[test] fn clean_json_without_yes_exits_2_and_removes_nothing()
#[test] fn clean_dry_run_exits_0_and_the_folder_is_still_there()
#[test] fn clean_include_of_an_unknown_path_exits_2()
#[test] fn clean_include_removes_a_name_matched_folder()
#[test] fn clean_worktrees_removes_the_clean_one_and_says_git_removes_them_permanently()
#[test] fn clean_include_of_a_dirty_worktree_exits_2_and_it_survives()
#[test] fn clean_json_yes_stdout_is_one_object_with_scan_and_outcome()
```
plus unit tests in `render.rs`: `summary_mentions_the_recycle_bin_or_permanently`, `outcome_lists_skipped_items_with_their_reason`.

- [ ] **Step 2:** Run `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-cli` — new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -j 1 -p xiox-cli` — PASS; then the whole suite `cargo test -j 1 --workspace --exclude mukade` — green, no warning.
- [ ] **Step 5:** Commit — `feat(xiox): comando clean con conferma e codici di uscita`.

---

### Task 8: User guide and release check

**Files:**
- Create: `docs/xiox-cli.md` (English)
- Modify: `README.md` if the repository root has one (add a short "Command line" section linking the guide); `docs/superpowers/specs/2026-11-13-project-cleanup-cli-design.md` (append a section «Esito» with what was built, test counts, exe size, deviations)

**Interfaces:**
- Consumes: the real `xiox --help` output and behaviour from Tasks 6–7.

Content of `docs/xiox-cli.md` (English, written for a user who has never seen the tool; every command shown must have been run against a fake tree and its output pasted from the real run, with temp paths shortened):
1. What it is, in three lines; where the exe is; it never asks for administrator rights and what changes when the terminal is elevated.
2. Quick start: `xiox analyze`, read the list, `xiox clean --builds --worktrees`.
3. What belongs to a project: one short paragraph per category (`builds`, `worktrees`, `copies`, `named`, `logs`, `sessions`), how it is found, and its level.
4. Levels and blocks: `recommended`, `optional`, `caution`, `blocked` and each block reason, in a table.
5. `analyze` reference: every flag, an annotated example output.
6. `clean` reference: every flag; what a category flag takes; `--include`; Recycle Bin vs `--permanent`; worktrees are always removed by git; confirmation and `--yes`; `--dry-run`.
7. For scripts and agents: `--json`, the object's fields (table), exit codes 0/1/2/3, a PowerShell example and a one-paragraph note for an AI agent ("run `xiox analyze --json` before finishing; never pass `--include` for a `caution` item without asking the user").
8. Safety: what is never removed (blocked items, links, protected locations, anything not in the analysis), re-validation before removal.
9. Limits: name search depth without elevation, project names shorter than 4 characters, branches are listed only, one project per run.
10. Troubleshooting: git not installed, path is not a repository, Recycle Bin too small, exit code 3.

- [ ] **Step 1:** Build release: `CARGO_INCREMENTAL=0 cargo build -j 1 -p xiox-cli --release`; note the size of `target/release/xiox.exe`.
- [ ] **Step 2:** Build a fake tree in `%TEMP%` (a repo named `sampleapp` with `Cargo.toml` + `target/CACHEDIR.TAG` + files, one clean linked worktree, one dirty linked worktree, a sibling `sampleapp-bench`, an ignored 2 MiB `debug.log`) and run, with `XIOX_SEARCH_ROOTS` in a **debug** build: `xiox analyze`, `xiox analyze --json`, `xiox clean`, `xiox clean --builds --worktrees --dry-run`, `xiox clean --builds --logs --yes`, `xiox clean --include <sampleapp-bench> --yes --permanent`. Keep the outputs.
- [ ] **Step 3:** Write `docs/xiox-cli.md` from those outputs. Check: `grep -n -i -E "consigliat|attenzione|cartell|pulisc|sessione" docs/xiox-cli.md` prints nothing.
- [ ] **Step 4:** Append «Esito» to the spec.
- [ ] **Step 5:** Commit `docs/xiox-cli.md`, `README.md` (if changed), the spec — `docs(xiox): guida d'uso della riga di comando`.
