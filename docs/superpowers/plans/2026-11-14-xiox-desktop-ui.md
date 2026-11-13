# Xiox Desktop UI (Tauri, Oni style) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the egui window of Xiox (`xiox-ui.exe`) with a Tauri 2 app in the Oni style of the approved mock-up (two sidebars, dark only, English only) that drives the same engine through the same guards, then delete the egui app.

**Architecture:** The egui-free modules of `src/` and the state machines that live today in `src/main.rs` and `src/dev.rs` move into a new library crate `crates/xiox-core`. Each section's state accepts typed *intents* (refusing whatever the egui UI would have disabled) and builds one serializable `SectionView` snapshot; worker threads wake the UI through a `Notifier` trait instead of `egui::Context`. A Tauri crate `apps/desktop/src-tauri` (package `xiox-ui`) puts a `Backend` trait in front of it (real: `xiox_core::app::Core` behind a mutex; fake: scripted fixtures, debug only) with a few commands and one `xiox:view` event. A framework-free TypeScript page renders whatever `SectionView` it receives and sends intents. The egui app keeps compiling on top of `xiox-core` until Task 19 deletes it.

**Tech Stack:** Rust (xiox-core edition 2021 like the code it receives — 2024 would silently change `if let` drop order; Tauri crate edition 2024 like mukade), Tauri `=2.12.5`, `tauri-build =2.7.4`, `tauri-plugin-dialog =2.8.4`, serde, windows-sys 0.61; TypeScript 7.0.2, Vite 8.3.5, `@tauri-apps/api` / `@tauri-apps/cli` 2.12.4, `node --experimental-strip-types --test`, Playwright 1.63.0 (Chromium on Vite, like mukade's `smoke.mjs`).

**Spec:** `docs/superpowers/specs/2026-11-14-xiox-desktop-ui-design.md` (Italian, authoritative; its section «Decisioni dopo il controllo del builder» is binding). Mock-up: `docs/assets/branding/xiox-ui-preview.png`; its source `.superpowers/brainstorm/17805-1794570359/content/oni-xiox-2barre-en.html`. Reference product to copy structure and pins from: `D:\Projects\mukade` (root `package.json`, `package-lock.json`, `tsconfig.json`, `apps/desktop/**`, `Cargo.lock`).

## Global Constraints

- **English only** for all visible text and all new identifiers of the new UI. The engine's Italian strings are translated **at the source** only in Tasks 16–18 (with the tests that assert them); until then moved code keeps its strings and its tests byte for byte. Every string this plan adds is English from the start.
- **Rust owns all state and every safety decision.** TypeScript renders `SectionView` and sends intents (clicks, keys); it never decides what is selectable or removable. An intent the egui UI could not have produced (button disabled, row greyed, dialog not open, job running) is refused in Rust with an English reason (`CmdError { kind: "refused" }`). Every removal goes through the existing engine paths unchanged: `xiox_cleaner::guard`, `xiox_app::build_plan` / `execute_plan` (stale-scan refusal) / `revalidate_archived`, `xiox_rules::Guard`, registry backup-before-fix, `services::set_start` on non-protected services only.
- **No path from the page.** No command argument or intent carries a filesystem path. The folder picker and the save dialog open in Rust (`tauri_plugin_dialog::DialogExt` inside the command, as mukade's `pick`); the capability grants the page no `dialog:`, `fs:`, `shell:` or `http:` permission. Capability `main` = `["core:event:allow-listen", "core:event:allow-unlisten"]`. CSP = mukade's `tauri.conf.json` string verbatim (it starts with `default-src 'self'`). No network from the UI; the Winapp2 update stays inside `rules::start`, as today.
- **Machine limits.** Every cargo run is `CARGO_INCREMENTAL=0 cargo <cmd> -j 1 …`; the tauri CLI runs with `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0`. One cargo **or** npm build at a time, never two (os error 1455). `node_modules` exists once, at the repo root (mukade keeps `package.json` at its root). No second target dir, no copy of the repo; tests that need scratch space use `tempfile` / the OS temp dir.
- **Nothing real is touched by tests, smoke scripts or demos**: no real system, projects, registry, services or Recycle Bin. Rust tests go through the worker seams of Tasks 6, 8, 9 and the `FakeBackend` (Task 11); the page is screenshot on Vite with `src/mock.ts`; the fake window exists only in debug builds with `XIOX_UI_FAKE=1` (`cfg!(debug_assertions)`). Single exception: the `#[ignore]` end-to-end test of Task 12, which reads session stores read-only and deletes only inside its own temp tree with `RemoveMode::Permanent`.
- **Always green.** Every commit leaves `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade` green (baseline at plan time: 1013 passed, 0 failed, 20 ignored, plus the `xiox observe` tests another task is adding in parallel; record the totals before starting a task and state the delta in the report). Totals: `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade 2>&1 | awk '/^test result/{p+=$4;f+=$6;i+=$8} END{print p" passed, "f" failed, "i" ignored"}'`. From Task 1 on, `npm test` and `npm run build` stay green too.
- **Hands off:** never edit `crates/xiox-observer`, `crates/xiox-cli`, `vendor/mukade`; never change the behaviour of `crates/xiox-project`. If one of their tests breaks, stop and report instead of editing them.
- **Commits:** Italian subject (given per task), only the files the task names (`git mv` for moves; `Cargo.lock` / `package-lock.json` when dependencies change), and the message ends with exactly this final paragraph (use a second `-m`):
  ```
  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Cd6KB14fHjjb98MiWyaNmx
  ```
- **Pins (copied from mukade).** npm: `@tauri-apps/api` 2.12.4, `@tauri-apps/cli` 2.12.4, `typescript` 7.0.2, `vite` 8.3.5, plus `playwright` 1.63.0 (exact). Rust: `tauri = "=2.12.5"`, `tauri-build = "=2.7.4"`, `tauri-plugin-dialog = "=2.8.4"`; after the first resolve set the Tauri family to mukade's `Cargo.lock` with `cargo update -p <name> --precise <version>`: `tauri-runtime 2.12.2`, `tauri-runtime-wry 2.12.3`, `tauri-utils 2.10.2`, `tauri-codegen 2.7.4`, `tauri-macros 2.7.4`, `tauri-plugin 2.7.2`, `tauri-plugin-fs 2.7.1`, `tauri-winres 0.3.7`, `wry 0.57.2`, `tao 0.37.2`, `webview2-com 0.39.1`, `webview2-com-sys 0.39.1`, `muda 0.20.0`, `rfd 0.17.2` (the dialog plugin's; the egui app's `rfd 0.15` coexists until Task 19).
- **Privileges as today.** The release exe embeds `requireAdministrator`; debug builds and every test binary embed `asInvoker`; `apps/desktop/src-tauri/build.rs` chooses by `PROFILE`, like the root `build.rs` does today. The status bar shows `administrator` or `standard user`.
- **Window:** minimum 1000 x 640; system title bar, dark (`"theme": "Dark"`); remembers size and position; one instance; text 13 px; Bahnschrift for titles, Cascadia Mono for numbers and paths; `prefers-reduced-motion` respected and no continuous animation; every control reachable with Tab, with a visible focus ring.
- **Copy fixed by the spec (verbatim):** sections `Cleanup` `1`, `Registry` `2`, `Services & RAM` `3`, `Developers` `4`; Developers categories `Projects`, `Worktrees`, `Archived sessions`; levels `RECOMMENDED` (ice), `OPTIONAL` (grey-blue), `CAUTION` (orange); detail keys `item`, `files`, `size`, `level`, `mode`, `why`; modes `to the Recycle Bin`, `permanently`; `Only delete Temp files older than 24 hours`; `I have read the warnings`; `Save as text…`; `Disable`, `Set to Manual`, `Restore originals`; `Recommended` / `All`; buttons `ANALYZE a`, `CLEAN <n> ITEMS ↵` (Developers: `Clean 3 folders`, `Remove 1 worktree`…); status `ANALYSIS DONE`; `administrator` / `standard user · RAM <n> GB`; footer `↑↓ move · space toggle · a analyze · ↵ clean`. Rust sends sentence case (`Analysis done`, `Clean 3 items`); CSS `text-transform: uppercase` makes the mock-up's capitals.
- **No new features** (spec «Non si fa»): no AUTO button, no light theme, no language switch, no installer, no tray icon, no auto-update, no mascot, no restore of registry backups inside the app.

## Review Focus

1. **A removal without the confirmation.** Enter pressed twice, a scripted `confirm` intent with no dialog open, a toggle while the dialog is open (the warnings on screen no longer describe what will run), the ack box bypassed. → Task 6 `confirm_needs_an_open_ready_dialog`, `nothing_changes_while_busy_or_with_the_dialog_open`; Task 8 `cleanup_confirm_needs_the_open_dialog`, `selection_is_frozen_while_busy_or_with_the_dialog_open`; Task 9 `fix_and_apply_need_their_dialog`; Task 3 smoke "Enter on Cancel closes without confirming"; Task 12 smoke "Clean stays disabled until the warnings are read".
2. **A stale analysis cleaned.** The 30-minute limit (`xiox_app::MAX_SCAN_AGE_MS`) or the confirmation's own scan lost in the port; the PATH preview reused from a view no longer on screen. → moved tests of Task 5 (`an_old_analysis_does_not_clean_projects_or_worktrees`, `a_dialog_left_open_past_half_an_hour_does_not_clean`, `the_confirmation_uses_the_chosen_mode_and_keeps_its_scan`); Task 6 `confirm_after_half_an_hour_only_warns`; Task 8 `previews_only_from_the_analysis_on_screen`.
3. **A dead or cancelled job leaves a half-state** (spinner forever, Clean enabled after a partial clean) now that no frame loop polls every frame. → Task 5 `outbox_drop_disconnects_before_notifying`, `a_panicking_worker_still_notifies`; Task 11 `pump_publishes_the_view_after_a_worker_dies`; moved `a_dead_cleanup_discards_the_scan_and_disables_the_button`, `dead_cleaning_thread_still_builds_the_report`, `dead_or_cancelled_analysis_is_marked_interrupted`.
4. **Italian left on screen, or text clipped / overlapping at 1000 x 640.** → Task 1 and Task 3 smoke fit check on every fixture state at 1000x640 and 1440x900; Tasks 16–18 `scripts/italian-check.mjs` over the crates, the embedded cleaners, the page and the fixtures; Task 18 smoke scan of the rendered text.
5. **Keyboard focus lost or trapped.** Focus vanishing when a dialog closes, Tab escaping an open dialog, focus ring cut off by `clip-path`, keys acting while typing in a search box. → Task 3 `keys.test.ts` and smoke focus checks; Task 15 smoke Tab audit.

## Decisions taken by this plan (★ = controller should check)

- ★ **One generic view.** All four sections produce the same `SectionView` shape, so one TypeScript renderer draws them and the section tasks are mostly Rust view builders plus fixtures. Rust writes every sentence and button label (the egui code already computes them and its tests assert them); TypeScript keeps only static headings, key hints, byte/duration formatting, sorting and row expansion.
- ★ **Transitional binary name.** The new package is `xiox-ui` but its `[[bin]]` is `xiox-tauri` until Task 19, so it never collides with the egui `[[bin]] xiox-ui` (cargo "output filename collision"). Task 19 renames it to `xiox-ui`.
- **Fields `pub` for the egui adapter.** State structs moved into xiox-core expose their fields `pub` so the egui drawing code keeps working with mechanical edits; Task 19 makes them private again where no outside user remains.
- **`Outbox<M>`** (sender + notifier, notifies on drop *after* closing the channel) replaces `tx.send(..); ctx.request_repaint()` in every worker, so a panicking worker still wakes the UI and `poll` sees `Disconnected`.
- **Worker seams** (`DevWorkers`, `CleanupWorkers`, `RegistryWorkers`, `ServicesWorkers`: plain `fn` pointers, real by default) let tests drive `confirm` → start end to end without ever reaching the engine.
- **One fixture file** `apps/desktop/fixtures/views.json` feeds both the TypeScript mock and the Rust `FakeBackend`, with one shared transition table (Task 2); a Rust test round-trips it through the real types.
- ★ **Playwright** is a pinned devDependency (1.63.0, whose Chromium 1243 is already in `%LOCALAPPDATA%\ms-playwright`): no machine-wide Playwright exists here (mukade's `smoke.mjs` assumes one).
- ★ **No single-instance / window-state plugins.** Own code over `windows-sys` (named mutex + `FindWindowW`) and a `window.json` next to the Developers `settings.json`, to avoid unpinned plugin versions.
- ★ **Recycle Bin row mode** reads `empties the Recycle Bin` (the spec says `to the Recycle Bin`, but that row deletes for good and «Nessun dato inventato» wins); every other Cleanup row reads `permanently`.
- ★ **Dates** in the English UI are ISO `YYYY-MM-DD` (Task 18 renames `dates_are_day_month_year` to `dates_are_iso`).
- ★ **Tests that disappear with egui (Task 19):** `column_widths_give_the_rest_to_the_flexible_column` (egui layout), `demo_runs_only_on_a_fake_tree` (egui `--dev-demo`), `sections_from_the_command_line` (egui `--dev-section`). Every other test of `main.rs` / `dev.rs` moves to xiox-core; `levels_have_their_words_and_colors` keeps its words and loses its egui colours (renamed `levels_have_their_words`).
- Sorting: rows with sizes are shown largest first (mock-up «largest first»), children stay under their parent, header rows keep their group; rows without sizes keep the engine order.
- Manifests: `apps/desktop/src-tauri/xiox-ui.manifest` / `xiox-ui.dev.manifest` are the root ones plus the Common-Controls v6 `<dependency>` that Tauri requires when an app supplies its own manifest.
- The Tauri app keeps `--only-root <dir>` (Developers restricted to one folder, settings in memory), drops `--screenshot`, `--tab`, `--analyze`, `--about`, `--dev-section`, `--dev-demo`, `--clean-demo` (egui screenshot aids, replaced by the fake and the smoke script).

## File map (end state)

```
package.json, package-lock.json, tsconfig.json          repo root, like mukade
apps/desktop/index.html, vite.config.ts
apps/desktop/fixtures/views.json                         shared fixed data (mock + FakeBackend)
apps/desktop/public/xiox-wordmark.png                    small copy of the logo for the top bar
apps/desktop/scripts/make_icon.py                        first X of the logo -> square PNG for `tauri icon`
apps/desktop/src/{main,api,types,mock,mockmodel,state,format,rows,intents,keys,dom}.ts
apps/desktop/src/render/{shell,bar2,list,detail,dialog,report,about}.ts
apps/desktop/src/styles/{tokens,frame,shell,list,dialog}.css
apps/desktop/tests/{format,rows,state,intents,keys,mockmodel}.test.ts, smoke.mjs
apps/desktop/src-tauri/{Cargo.toml,build.rs,tauri.conf.json,xiox-ui.manifest,xiox-ui.dev.manifest}
apps/desktop/src-tauri/capabilities/main.json, icons/{icon.ico,icon.png,32x32.png,128x128.png,128x128@2x.png}
apps/desktop/src-tauri/src/{main,backend,real,fake,commands,error,files,window_state,single_instance}.rs
crates/xiox-core/src/{lib,notify,view,app,pump,regstate,svcstate}.rs
crates/xiox-core/src/dev/{mod,view,tests,intent_tests}.rs
crates/xiox-core/src/cleanup/{mod,view,tests,intent_tests}.rs
crates/xiox-core/src/{account,browserdb,cleaner,envpath,live_checks,native,netcreds,regbackup,registry,rules,services,sys,sysclean,winold,wipe}.rs  (+ their */tests.rs)
scripts/italian-check.mjs, scripts/italian-allow.txt, scripts/italian-check.test.mjs
docs/design/oni-xiox-2barre-en.html                      living copy of the approved mock-up source
```

## The view contract (Rust → TypeScript)

`apps/desktop/src/types.ts` (Task 1) is the source of truth; `crates/xiox-core/src/view.rs` (Task 6) mirrors it field for field: Rust snake_case; every struct `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(rename_all = "camelCase", deny_unknown_fields)]`; string unions are unit enums `#[serde(rename_all = "camelCase")]`; tagged unions `#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]`; `[a, b]` is `(A, B)`; `T | null` is `Option<T>`; bytes, counts and ms are `u64`, days `u32`, indices `usize`, `fraction` `f32`. No `#[serde(default)]`: a missing field is an error.

```ts
export type SectionId = 'cleanup' | 'registry' | 'services' | 'developers';
export type Tone = 'info' | 'ok' | 'warn' | 'error';
export type LevelView = 'recommended' | 'optional' | 'caution';
export type Check = 'on' | 'off' | 'mixed' | 'none';
export type RowKind = 'item' | 'header' | 'child';
export type ColumnKind = 'name' | 'cell' | 'level' | 'size';

export interface AppInfo { version: string; admin: boolean; ramTotalBytes: number; profileMismatch: string | null; fake: boolean }
export interface AboutView { version: string; winapp2: string; attribution: string }
export interface JobView { label: string; fraction: number; detail: string | null }
export interface StatusView { badge: string; text: string }
export interface NavItem { id: string; name: string; note: string | null; bytes: number | null; active: boolean }
export interface ChoiceItem { id: string; label: string }
export type OptionView =
  | { kind: 'toggle'; id: string; label: string; on: boolean; enabled: boolean; hint: string | null }
  | { kind: 'number'; id: string; label: string; value: number; min: number; max: number; enabled: boolean }
  | { kind: 'choice'; id: string; label: string; value: string; choices: ChoiceItem[]; enabled: boolean }
  | { kind: 'list'; id: string; label: string; items: string[]; selected: number | null; enabled: boolean; addLabel: string; removeLabel: string; empty: string }
  | { kind: 'button'; id: string; label: string; enabled: boolean; hint: string | null }
  | { kind: 'meter'; id: string; label: string; usedBytes: number; totalBytes: number; tone: Tone }
  | { kind: 'text'; text: string; tone: Tone; hint: string | null };
export interface ColumnView { title: string; kind: ColumnKind }
export interface RowView {
  id: string; kind: RowKind; parent: string | null; check: Check; enabled: boolean;
  name: string; sub: string | null; level: LevelView | null; cells: string[];
  bytes: number | null; sizeText: string | null; muted: boolean; why: string | null;
  detail: [string, string][]; lines: string[];
}
export interface NoticeView { tone: Tone; text: string; more: string[] }
export interface ActionView { id: string; label: string; key: string | null; enabled: boolean; primary: boolean }
export interface ButtonView { id: string; label: string; enabled: boolean; primary: boolean }
export interface DialogView {
  id: string; title: string; text: string[]; summary: string[]; rows: [string, string][];
  warningsTitle: string | null; warnings: string[]; cautionTitle: string | null; cautions: [string, string][];
  ack: boolean | null; error: string | null; buttons: ButtonView[];
}
export interface ReportLine { item: string; why: string; path: string }
export interface ReportItem { name: string; freedBytes: number; removed: number; left: number; note: string | null; lines: string[] }
export interface SavedView { text: string; ok: boolean }
export interface ReportView {
  title: string; freedBytes: number; recycledBytes: number; files: number; dirs: number; elapsedMs: number;
  cancelled: boolean; notes: NoticeView[]; byCategory: [string, number][]; skipped: ReportLine[]; failed: ReportLine[];
  items: ReportItem[]; canSave: boolean; saved: SavedView | null;
}
export interface EmptyView { title: string; text: string }
export interface SearchView { placeholder: string; value: string }
export interface SectionView {
  section: SectionId; rev: number; job: JobView | null; status: StatusView;
  nav: NavItem[]; options: OptionView[]; title: string; subtitle: string;
  columns: ColumnView[]; rows: RowView[]; bulk: boolean; empty: EmptyView | null;
  notices: NoticeView[]; actions: ActionView[]; search: SearchView | null;
  dialog: DialogView | null; report: ReportView | null; footer: string | null;
}
export interface CmdError { kind: 'refused' | 'failed'; message: string }

export type CleanupIntent =
  | { kind: 'select'; category: string } | { kind: 'toggle'; row: string } | { kind: 'setOnlyOld'; on: boolean }
  | { kind: 'search'; text: string } | { kind: 'analyze' } | { kind: 'openConfirm' } | { kind: 'confirm' }
  | { kind: 'closeDialog' } | { kind: 'cancel' } | { kind: 'openKeepList' };
export type RegistryIntent =
  | { kind: 'toggleCategory'; index: number } | { kind: 'toggle'; row: string } | { kind: 'selectAll'; on: boolean }
  | { kind: 'scan' } | { kind: 'openFix' } | { kind: 'fix'; backup: boolean } | { kind: 'closeDialog' };
export type ServicesIntent =
  | { kind: 'setAll'; all: boolean } | { kind: 'search'; text: string } | { kind: 'toggle'; row: string } | { kind: 'refresh' }
  | { kind: 'openConfirm'; action: 'disable' | 'manual' | 'restore' } | { kind: 'confirm' } | { kind: 'closeDialog' };
export type DevIntent =
  | { kind: 'select'; category: 'projects' | 'worktrees' | 'sessions' } | { kind: 'toggle'; row: string } | { kind: 'selectAll'; on: boolean }
  | { kind: 'setInactiveDays'; days: number } | { kind: 'setRemoveMode'; mode: 'recycle' | 'permanent' } | { kind: 'setScanRoots'; on: boolean }
  | { kind: 'selectRoot'; index: number | null } | { kind: 'removeRoot' } | { kind: 'analyze' } | { kind: 'openConfirm' }
  | { kind: 'setAck'; on: boolean } | { kind: 'confirm' } | { kind: 'permanent' } | { kind: 'closeDialog' } | { kind: 'cancel' };
export type AnyIntent = CleanupIntent | RegistryIntent | ServicesIntent | DevIntent;
```

**Commands** (Tauri, Task 11; the same names are answered by `src/mock.ts` from Task 1): `app_info() -> AppInfo`, `about() -> AboutView`, `show({ section }) -> SectionView`, `view({ section }) -> SectionView`, `cleanup({ intent })`, `registry({ intent })`, `services({ intent })`, `developers({ intent })` → `SectionView` or `CmdError`, `developers_add_root() -> SectionView` (folder picker in Rust), `save_report({ section }) -> SectionView` (save dialog in Rust). **Event** `xiox:view`, payload `SectionView`. The page keeps, per section, the view with the highest `rev`.

**Row ids:** Cleanup `cat:<index>` (built-in category), `native:<id>`, `<cleaner id>/<option id>` (`rules::key`), headers `section:<Winapp2 section>`; Registry `i:<index>`; Services `svc:<service name>`; Developers `p:<project id>`, `a:<artifact id>`, `w:<worktree report id>`, `o:<other worktree id>`, `s:<agent>:<session id>` (`agent` = `claude` | `codex` | `opencode` | `pi`), header `h:others`.

**Action ids** (top bar) and the intent the page sends: `analyze` → Cleanup/Developers `analyze`; `scan` → Registry `scan`; `refresh` → Services `refresh`; `clean` → Cleanup/Developers `openConfirm`; `fix` → Registry `openFix`; `disable` / `manual` / `restore` → Services `openConfirm { action }`; `cancel` → `cancel`. **Dialog button ids:** `cancel` → `closeDialog`; `continue` → Cleanup/Services `confirm`; `clean` → Developers `confirm`; `permanent` → Developers `permanent`; `backup` → Registry `fix { backup: true }`; `nobackup` → Registry `fix { backup: false }`. **Option ids:** Cleanup `onlyOld` (toggle → `setOnlyOld`), `keepList` (button → `openKeepList`); Registry `cat:<i>` (toggle → `toggleCategory { index }`); Services `filter` (choice `recommended` | `all` → `setAll`); Developers `inactiveDays` (number → `setInactiveDays`), `removeMode` (choice `recycle` | `permanent` → `setRemoveMode`), `scanRoots` (toggle → `setScanRoots`), `roots` (list: select → `selectRoot`, add → command `developers_add_root`, remove → `removeRoot`).

---

### Task 1: Web shell scaffold (Vite + TypeScript, tokens, frames, mock, smoke)

**Files:**
- Create: `package.json`, `package-lock.json`, `tsconfig.json` (repo root); `apps/desktop/index.html`, `apps/desktop/vite.config.ts`; `apps/desktop/src/{main,api,types,mock,mockmodel,state,format,dom}.ts`, `apps/desktop/src/render/shell.ts`; `apps/desktop/src/styles/{tokens,frame,shell}.css`; `apps/desktop/public/xiox-wordmark.png`; `apps/desktop/fixtures/views.json`; `apps/desktop/tests/format.test.ts`, `apps/desktop/tests/smoke.mjs`; `docs/design/oni-xiox-2barre-en.html` (copy of the mock-up source)
- Modify: `.gitignore` (add `node_modules/`, `apps/desktop/dist/`, `apps/desktop/tests/out/`, `apps/desktop/src-tauri/gen/`)

**Interfaces:**
- Consumes: mukade's `package.json`, `package-lock.json`, `tsconfig.json`, `apps/desktop/vite.config.ts`, `src/api.ts`, `src/dom.ts`, `tests/smoke.mjs` (read them first).
- Produces: `types.ts` = the view contract above, verbatim. `api.ts` (mukade pattern: real `invoke` inside Tauri, `mock.ts` only when `import.meta.env.DEV`, else throw): `api.appInfo()`, `api.about()`, `api.show(s)`, `api.view(s)`, `api.intent(s: SectionId, i: AnyIntent): Promise<SectionView>` (dispatches to the section's command), `api.addRoot()`, `api.saveReport(s)`, `onView(handler: (v: SectionView) => void): void` (`listen('xiox:view')` in Tauri, the mock's emitter outside). `format.ts`: `formatBytes(n: number): string`, `formatDuration(ms: number): string`, `ramText(bytes: number): string`. `mockmodel.ts`: `export interface Fixture { app: AppInfo; about: AboutView; views: Record<SectionId, Record<string, SectionView>> }` and `class MockModel { constructor(f: Fixture); show(s): SectionView; view(s): SectionView; intent(s, i): SectionView; tick(): SectionView[]; go(s, state: string): SectionView }` (Task 1: `show`/`view` only, `intent` returns the current view with a new rev; Task 2 adds the table). `mock.ts`: `answer(command, args)` over one `MockModel` built from `fixtures/views.json` (`import … with { type: 'json' }`), an emitter for `onView`, `setInterval(tick, 200)`, and `window.__xioxMock = { go }` for the smoke script. Smoke helpers in `smoke.mjs`: `checkFit(page, label)`, `checkTokens(page)`.

- [ ] **Step 1: Root npm project.** Copy mukade's `package.json` with `"name": "xiox"`, its seven scripts unchanged (they already point at `apps/desktop`) and the four pinned deps; copy mukade's `package-lock.json`, set both `name` fields to `xiox`; then `npm install --save-dev --save-exact playwright@1.63.0`. Copy mukade's `tsconfig.json` and add `"resolveJsonModule": true`, `"include": ["apps/desktop/src"]`. Copy `vite.config.ts` with `port: 1422`.
Run: `npm ls --depth=0` → exactly `@tauri-apps/api@2.12.4`, `@tauri-apps/cli@2.12.4`, `playwright@1.63.0`, `typescript@7.0.2`, `vite@8.3.5`. Run `node -e "require('playwright').chromium.executablePath()"` from the repo root; if the browser is missing, `npx playwright install chromium` once.

- [ ] **Step 2: Write the failing test** `apps/desktop/tests/format.test.ts`:

```ts
test('formatBytes matches the engine sizes in English', () => {
  assert.equal(formatBytes(0), '0 B'); assert.equal(formatBytes(512), '512 B'); assert.equal(formatBytes(1023), '1023 B');
  assert.equal(formatBytes(2048), '2 KB'); assert.equal(formatBytes(3 * 2 ** 20), '3.0 MB'); assert.equal(formatBytes(10 * 2 ** 30), '10.00 GB');
});
test('formatDuration', () => {
  assert.equal(formatDuration(850), '850 ms'); assert.equal(formatDuration(12_300), '12 s');
  assert.equal(formatDuration(125_000), '2 min 05 s'); assert.equal(formatDuration(3_720_000), '1 h 02 min');
});
test('ramText rounds to whole GB', () => { assert.equal(ramText(33_930_000_000), 'RAM 32 GB'); });
```

- [ ] **Step 3: Run it.** `npm test` → FAIL (module `../src/format.ts` not found).

- [ ] **Step 4: Implement `format.ts`.** Same thresholds as `xiox_model::fmt_size` (GB 2 decimals, MB 1, KB 0, else `B`) with `.`; `formatDuration` same steps as `xiox_model::fmt_duration`; `ramText` = `RAM ${Math.round(bytes / 2 ** 30)} GB`.

- [ ] **Step 5: Run it.** `npm test` → 3 pass.

- [ ] **Step 6: Tokens and frames.** `styles/tokens.css` copies the mock-up's variables under `:root` (`color-scheme: dark`):

```css
--stage:#0b0416; --chrome:#10081f; --surface:#1b2c52; --pane:#0d0a22; --pane-fill:#0d0a22cc;
--line:#3f6fb5; --line-strong:#6aa0e6; --line-soft:#3f6fb555; --ice:#9cc4f5; --live:#8fd6ff;
--text:#ffb21e; --text-2:#f0a020; --text-hi:#ffd257; --muted:#7f9fd0; --warn:#ff6a3a; --ok:#7fe0a0;
--sel:#2a4f8f; --sel-soft:#2a4f8f99; --button:#24457a; --button-primary:#3566b0;
--grid:#2a4a8018; --glow:#2b4f9a40; --inner:#6aa0e633; --box:#1b2c5266; --box-line:#3f6fb588;
--font-display:Bahnschrift,'Segoe UI',sans-serif; --font-body:'Segoe UI',sans-serif;
--font-mono:'Cascadia Mono','Cascadia Code',Consolas,monospace; --text-size:13px;
```

`body { margin:0; background:var(--stage); color:var(--text); font:600 var(--text-size)/1.45 var(--font-body) }`. `styles/frame.css`: `.frame` = 2 px `--line` border, `--pane-fill` background, `box-shadow: inset 0 0 0 1px var(--inner)`, and the mock-up's clip-path verbatim `polygon(0 14px,14px 0,calc(50% - 30px) 0,calc(50% - 22px) 6px,calc(50% + 22px) 6px,calc(50% + 30px) 0,calc(100% - 14px) 0,100% 14px,100% calc(100% - 14px),calc(100% - 14px) 100%,14px 100%,0 calc(100% - 14px))`; `.btn` = `polygon(8px 0,100% 0,100% calc(100% - 8px),calc(100% - 8px) 100%,0 100%,0 8px)`, 2 px `--line` border, `--button` fill, `--text` colour; `.btn.primary` = `--button-primary`, border `--line-strong`, colour `--text-hi`; `#app::before` = the grid background (`linear-gradient(90deg,var(--grid) 1px,transparent 1px) 0 0/22px 22px, linear-gradient(0deg,var(--grid) 1px,transparent 1px) 0 0/22px 22px, radial-gradient(700px 360px at 80% 60%,var(--glow),transparent 70%)`). Focus ring: because `clip-path` cuts outlines, every focusable gets `:focus-visible { outline:none; box-shadow: inset 0 0 0 2px var(--live) }`. `@media (prefers-reduced-motion: reduce) { *, *::before { transition:none !important; animation:none !important } }`; no keyframe animation anywhere. Titles (`.hd`, nav, badge, actions) use `--font-display` 700 13 px, `--ice` for pane titles, `text-transform: uppercase`; numbers and paths `--font-mono`.

- [ ] **Step 7: Shell layout.** `index.html` (lang `en`, `<title>Xiox</title>`, no inline script/style): `<main id="app">` with `<header id="top">` (wordmark `<img src="/xiox-wordmark.png" alt="Xiox">`, `#version`, spring, `#actions`), `<div id="status">` (`#badge`, `#status-text`, `#status-right`), `<div id="profile" hidden>`, `<div id="body">` (`<nav id="sections" class="frame">`, `<aside id="bar2" class="frame">`, `<section id="main" class="frame">` holding `#notices`, `#list-head`, `#rows`, `#detail`), `<footer id="foot">`. `shell.css`: `#app` grid rows `38px 34px auto 1fr 26px`, `#body` columns `170px 200px minmax(0,1fr)`, gap 8 px, padding `0 8px 8px`; `#detail` at the bottom of `#main` (mock-up `.det` / `.kv`, max 34 % of the pane). Make `public/xiox-wordmark.png` from `docs/assets/branding/xiox-logo.png` with Pillow: resize to height 48 px, keep the ratio (shown at 24 px).

- [ ] **Step 8: Fixture and mock (shell level).** `fixtures/views.json` = `{ "app": AppInfo, "about": AboutView, "views": { "cleanup": { "analyzed": … }, "registry": { "analyzed": … }, "services": { "loaded": … }, "developers": { "projects": … } } }` with complete `SectionView`s (every field present). Content follows the mock-up: Cleanup nav `System` (active, note `6 items · 4 selected`), `Explorer`, `Advanced`, `Windows`, `Browser`, `Development & AI`, `Applications`; rows `Recycle Bin`, `User temporary files`, `Windows temporary files`, `Memory dumps`, `Windows log files`, `Windows Error Reporting` with sizes like the mock-up and one `caution` row; actions `analyze` (`Analyze`, key `a`) and `clean` (`Clean 4 items`, key `Enter`, primary); badge `Analysis done`, text `12.4 GB to delete (18 204 files)`. App: `version "0.1.0"`, `admin true`, `ramTotalBytes 33930000000`, `profileMismatch null`, `fake true`. Plausible English data for the other three sections (Developers rows include one project with two children).

- [ ] **Step 9: Render the shell.** `render/shell.ts`: `renderShell(app: AppInfo, view: SectionView): void` draws the top bar (version, `view.actions` as `.btn` with `<kbd>` showing `a` / `↵` / `Esc`), the status row (`badge`, `text`; right side `administrator` / `standard user` + ` · ` + `ramText`), the sections bar (`SECTIONS` header with count 4; `Cleanup 1`, `Registry 2`, `Services & RAM 3`, `Developers 4`, the open one `.on`), the bar-2 header (`CATEGORIES` when `nav` is non-empty, else `OPTIONS`) with nav names, the list header (`title`, `subtitle`) and the footer (`view.footer` left, `↑↓ move · space toggle · a analyze · ↵ clean` right). `main.ts`: `appInfo()`, `show('cleanup')`, render; clicking a section calls `show`. Every label that must never clip carries `data-fit`; long free text (paths, reasons) gets `.ellipsis` and a `title` with the full text. `dom.ts` copies mukade's `h`, `fill`, `button`, `kbd`.

- [ ] **Step 10: Smoke script.** `tests/smoke.mjs` (mukade's structure: Vite `createServer` on port 1432, `strictPort: false`, `chromium.launch()`, fail on `pageerror` and console errors). For viewports `1000x640` and `1440x900`, for each section (press `1`…`4` is Task 3; here click the section buttons) screenshot to `tests/out/<section>-<state>-<w>x<h>.png`. `checkTokens(page)`: `getComputedStyle(document.documentElement)` gives `--stage` `#0b0416`, `--line` `#3f6fb5`, `--text` `#ffb21e`, `--ice` `#9cc4f5`, and `document.body` background `rgb(11, 4, 22)`. `checkFit(page, label)`: every `[data-fit]` has `scrollWidth <= clientWidth + 1` and `scrollHeight <= clientHeight + 1`; every truncated `.ellipsis` has a non-empty `title`; `document.documentElement.scrollWidth <= innerWidth`; the bounding boxes of `#actions > *` do not intersect each other or `#version`. Collect failures, exit 1 with the list.

- [ ] **Step 11: Run.** `npm run smoke` → `smoke ok: 8 screenshots`. Open two of them with the Read tool next to `docs/assets/branding/xiox-ui-preview.png`: same palette, cut-corner frames with the top notch, grid background, amber bold text, ice titles. `npm run build` → tsc and Vite succeed.

- [ ] **Step 12: Commit** `feat(xiox-ui): guscio web in stile Oni con dati finti e prove di schermata` — the files above.

### Task 2: List, detail, bar 2 and intents on the mock

**Files:**
- Create: `apps/desktop/src/{rows,intents}.ts`, `apps/desktop/src/render/{bar2,list,detail}.ts`, `apps/desktop/src/styles/list.css`, `apps/desktop/tests/{rows,state,intents,mockmodel}.test.ts`
- Modify: `apps/desktop/src/{state,mockmodel,mock,main}.ts`, `apps/desktop/src/render/shell.ts`, `apps/desktop/fixtures/views.json` (add the states the table below names for Cleanup: `idle`, `analyzing`, `confirm`, `cleaning`, `report`; for the others: Registry `idle`, `scanning`, `backup`, `fixing`, `fixed`; Services `loading`, `confirm`; Developers `idle`, `analyzing`, `worktrees`, `sessions`, `confirm`, `shortfall`, `cleaning`, `report` — dialogs/reports may stay minimal until Task 3 and Tasks 12–14 enrich them)

**Interfaces:**
- Consumes: Task 1 `types.ts`, `api`, `MockModel`, `Fixture`.
- Produces: `rows.ts`: `sortRows(rows: RowView[]): RowView[]`, `visibleRows(rows: RowView[], expanded: ReadonlySet<string>): RowView[]`, `gridTemplate(columns: ColumnView[], checks: boolean): string`. `state.ts`: `interface UiState { section: SectionId; views: Partial<Record<SectionId, SectionView>>; focus: Partial<Record<SectionId, string>>; expanded: Set<string> }`, `acceptView(ui: UiState, v: SectionView): boolean`, `moveFocus(rows: RowView[], current: string | null, delta: number): string | null`. `intents.ts`: `actionIntent(s: SectionId, id: string): AnyIntent | null`, `buttonIntent(s, id): AnyIntent | null`, `optionIntent(s, o: OptionView, value: boolean | number | string | null): AnyIntent | null`, `toggleIntent(s, row: string): AnyIntent`, `selectIntent(s, nav: string): AnyIntent | null`, `searchIntent(s, text: string): AnyIntent | null` — exactly the id tables of the view contract. `MockModel` implements the **transition table** (the Rust `FakeBackend` of Task 11 implements the same one):

| Intent (any section unless named) | From state | To |
|---|---|---|
| `show(s)` | — | the section's current state; initial `cleanup:idle`, `registry:idle`, `services:loaded`, `developers:idle` |
| `analyze` / `scan` / `refresh` | not busy | busy state (`analyzing`, `scanning`, `loading`) for 3 ticks with `job.fraction` 1/3, 2/3, 1, then `analyzed` (Developers `projects`, Services `loaded`) |
| `select{category}` Developers | `projects` / `worktrees` / `sessions` | that state |
| `select{category}` other | any | same state, `nav[].active` moved to that id |
| `toggle{row}` | any | same state, that row's `check` `on`↔`off` (`mixed`→`on`); nothing else recounted |
| `openConfirm` / `openFix` | analyzed-like | `confirm` (Registry `backup`) |
| `setAck{on}` | `confirm` | same, `dialog.ack = on`, the primary button `enabled = on` |
| `confirm` / `fix` / `permanent` | `confirm` / `backup` / `shortfall` | busy (`cleaning`, `fixing`) 3 ticks, then `report` (Registry `fixed`); Services `confirm` goes straight back to `loaded` with an `ok` notice |
| `closeDialog` | a dialog state | the state it was opened from |
| `cancel` | a busy state | the state before the job |
| anything else | — | unchanged, new `rev` |

`rev` grows by one on every view the model returns. `go(s, state)` jumps to a named state (smoke only).

- [ ] **Step 1: Write the failing tests.**
  - `rows.test.ts`: `sortRows puts the largest group first and keeps children with their parent` (input `[p1 100 {c1 10, c2 90}, p2 300]` → `p2, p1, c2, c1`); `sortRows keeps the engine order without sizes`; `headers keep their rows` (a `header` row moves with the items after it up to the next header, groups ordered by summed bytes); `gridTemplate adds the check column only when rows have checks` (`[name, level, size]`, checks → `'16px minmax(0,1fr) 96px 74px'`; without checks → `'minmax(0,1fr) 96px 74px'`; each `cell` column → `minmax(64px,max-content)`).
  - `state.test.ts`: `an older view never replaces a newer one` (rev 5 accepted, then rev 4 refused, rev 6 accepted); `moveFocus skips header rows and clamps at the ends`; `children of a collapsed parent are not visible`.
  - `intents.test.ts`: `actions, buttons and options map to their section's intent` — one assertion per row of the id tables (e.g. `actionIntent('registry','fix')` → `{ kind: 'openFix' }`, `buttonIntent('registry','nobackup')` → `{ kind: 'fix', backup: false }`, `optionIntent('developers', removeModeOption, 'permanent')` → `{ kind: 'setRemoveMode', mode: 'permanent' }`, unknown ids → `null`).
  - `mockmodel.test.ts` (reads the fixture with `fs`): `the fixture holds every state the table names`; `analyze passes three ticks and lands on analyzed`; `toggle flips one row only`; `closeDialog returns to the state the dialog was opened from`; `cancel returns to the state before the job`.

- [ ] **Step 2: Run** `npm test` → the new tests FAIL.

- [ ] **Step 3: Implement** `rows.ts`, `state.ts`, `intents.ts`, the table in `mockmodel.ts`, the timer in `mock.ts`.

- [ ] **Step 4: Run** `npm test` → all pass.

- [ ] **Step 5: Render.** `render/bar2.ts`: nav items (`name`, `note` small muted, `bytes` right in mono; `active` = `--sel` + `--text-hi`), then each `OptionView` kind (toggle = checkbox styled like the mock-up `.ck`; number = `<input type=number>` with min/max, sent on change/blur; choice = radio group; list = selectable items + `addLabel` / `removeLabel` buttons; button; meter = bar with `usedBytes / totalBytes`, tone colour; text = paragraph in tone colour with `title = hint`). `render/list.ts`: column header from `columns` (`gridTemplate`), rows from `visibleRows(sortRows(rows), expanded)` — checkbox `role="checkbox"` with `aria-checked` `true`/`false`/`mixed`, disabled when `!enabled` with `title = why`; name + `sub` (muted, `.ellipsis`); `cells` mono; level word in caps with colour (`recommended` `--ice`, `optional` `--muted`, `caution` `--warn`); size right-aligned mono = `formatBytes(bytes)` or `sizeText` or `—`; `muted` rows dim; `child` rows indented 14 px, their parent shows a caret that toggles `expanded`; `header` rows in `--ice`. `bulk` shows `All` / `None` buttons in the list header (→ `selectAll`); `search` shows an input above the rows (→ `searchIntent`, 150 ms debounce). `render/detail.ts`: the focused row's `detail` pairs as the mock-up `.kv` grid, then `lines` as a list; nothing focused → empty box. Clicking a row focuses it; clicking its checkbox sends `toggleIntent`; clicking a nav item sends `selectIntent`.

- [ ] **Step 6: Smoke.** Extend `smoke.mjs`: after each screenshot of an analyzed state, click the first enabled checkbox and assert its `aria-checked` flipped (mock round trip). Run `npm run smoke` and `npm run build` → ok.

- [ ] **Step 7: Commit** `feat(xiox-ui): elenco, dettaglio, seconda barra e intenzioni sul finto`.

### Task 3: Dialogs, reports, progress, notices, empty states and the keyboard

**Files:**
- Create: `apps/desktop/src/keys.ts`, `apps/desktop/src/render/{dialog,report}.ts`, `apps/desktop/src/styles/dialog.css`, `apps/desktop/tests/keys.test.ts`
- Modify: `apps/desktop/src/{main,state}.ts`, `apps/desktop/src/render/{shell,list}.ts`, `apps/desktop/fixtures/views.json` (Cleanup `confirm` with 3 warnings, `cleaning` with a job, `report` with skipped/failed lines and items), `apps/desktop/tests/smoke.mjs`

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `keys.ts`:

```ts
export interface KeyLike { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean }
export type KeyAction =
  | { kind: 'section'; section: SectionId } | { kind: 'move'; delta: number } | { kind: 'toggle' }
  | { kind: 'expand'; open: boolean } | { kind: 'action'; key: 'a' | 'Enter' | 'Esc' } | { kind: 'closeDialog' };
export function keyAction(e: KeyLike, ctx: { inField: boolean; dialog: boolean }): KeyAction | null;
export function actionForKey(view: SectionView, key: 'a' | 'Enter' | 'Esc'): ActionView | null; // enabled action whose `key` matches
```

Rules: with modifiers (Ctrl/Alt/Meta) → `null`. `Escape` → `closeDialog` when a dialog is open, else `action Esc`. Inside a dialog every other key → `null` (the browser activates the focused button with Enter/Space; Tab is trapped by the dialog). In a text field only `Escape` (blur) acts. Otherwise: `1`–`4` → section in bar-1 order; `ArrowDown`/`ArrowUp` → move ±1, `PageDown`/`PageUp` ±10, `Home`/`End` → ∓/± `Infinity`; `' '` → toggle; `ArrowRight`/`ArrowLeft` → expand open/close; `a` → `action a`; `Enter` → `action Enter`.

- [ ] **Step 1: Write the failing tests** `keys.test.ts`: `digits switch sections`; `space toggles and arrows move outside fields and dialogs`; `Enter and a press the view's action with that key, and nothing when it is disabled` (`actionForKey` over a view whose `clean` is disabled → `null`); `Escape closes a dialog before it cancels a job`; `keys are ignored while typing, except Escape`; `inside a dialog only Escape is ours`.

- [ ] **Step 2: Run** `npm test` → FAIL. **Step 3: Implement** `keys.ts`. **Step 4: Run** → pass.

- [ ] **Step 5: Render the rest.** `render/dialog.ts`: modal frame over a dimmed page (`role="dialog"`, `aria-modal="true"`, `aria-labelledby` the title): `title`, `text` paragraphs, `error` in `--warn`, `summary` lines in `--ice`, `rows` (label + size, scrolls past 180 px), `warningsTitle` + `warnings` in `--warn`, `cautionTitle` + `cautions` (label, muted note), the ack checkbox `I have read the warnings` when `ack !== null` (→ `setAck`), `buttons` in order (primary styled). Focus on open: the primary button if enabled, else the ack checkbox, else `Cancel`; Tab and Shift+Tab cycle inside; on close focus returns to the element that had it before (or `#rows`). `render/report.ts` replaces the list area when `report` is set: title, `Space freed: …`, recycled line when > 0, `Files deleted: … · Folders deleted: … · Duration: …`, `notes`, `byCategory` table, `Skipped (n)` and `Failed (n)` tables (item + why, `title = path`), `items` table (name, freed, removed, left, note; `lines` expandable), `Save as text…` button when `canSave` (→ `api.saveReport`), `saved.text` in `--ok`/`--warn`. `job` shows a progress bar under the status row with `label` and `detail`; no animation. `notices` are strips above the list (tone colour, `more` in `title`). `empty` replaces the rows with a centred title and text.

- [ ] **Step 6: Smoke additions.** States `cleanup:idle`, `cleanup:analyzing`, `cleanup:confirm`, `cleanup:report` at both sizes through `__xioxMock.go`, each passing `checkFit`. Keyboard: press `1`–`4` and check the open section; on `cleanup:analyzed` press `Enter` → dialog shown; focus `Cancel`, press `Enter` → no dialog and the view is not `cleaning` (**Enter on Cancel closes without confirming**); open again, press Tab 12 times → `document.activeElement` stays inside `[role=dialog]`; `Escape` → dialog gone and focus inside `#rows`; the focused button's computed `box-shadow` is not `none`. Run `npm run smoke`, `npm test`, `npm run build` → ok.

- [ ] **Step 7: Commit** `feat(xiox-ui): conferme, resoconto, avanzamento e tastiera`.

### Task 4: `crates/xiox-core` with the egui-free modules

**Files:**
- Create: `crates/xiox-core/Cargo.toml`, `crates/xiox-core/src/lib.rs`
- Move (`git mv`): `src/{account,browserdb,cleaner,envpath,live_checks,native,netcreds,regbackup,registry,rules,services,sys,sysclean,winold,wipe}.rs` and the dirs `src/{browserdb,envpath,netcreds,winold,wipe}/` → `crates/xiox-core/src/`
- Modify: `src/main.rs:3-20` (module list), `Cargo.toml` (root dependencies)

**Interfaces:**
- Produces: `xiox_core::{account, browserdb, cleaner, envpath, native, netcreds, regbackup, registry, rules, services, sys, sysclean, winold, wipe}` as `pub mod`, unchanged; `#[cfg(test)] mod live_checks;` stays test-only.

- [ ] **Step 1: Record the baseline totals** (one-liner of Global Constraints) and `grep -c '#\[test\]'` over the files to move (expected 255 with their `tests.rs`).

- [ ] **Step 2: Create the crate.** `Cargo.toml`: `name = "xiox-core"`, `edition = "2021"`, `license = "MIT"`; dependencies = the ones the moved files use, copied from the root manifest with the same versions, features and comments (`xiox-cleaner`, `xiox-model`, `xiox-rules`, `winreg = "0.56"`, `rusqlite = { version = "=0.40.2", features = ["bundled"] }`, `serde_json` with `float_roundtrip` + `preserve_order`, `idna = "=1.1.0"`, `windows-sys = "0.61"` with the full feature list); dev-dependency `tempfile = "3.27.0"`. `lib.rs`: the `pub mod` list. Move the files with `git mv`.

- [ ] **Step 3: Rewire the egui app.** In `src/main.rs` replace the moved `mod` lines with `use xiox_core::{account, browserdb, cleaner, envpath, native, netcreds, regbackup, registry, rules, services, sys, sysclean, winold, wipe};` (crate-root imports keep every `crate::sys`, `crate::rules` path in `src/dev.rs` valid); keep `mod dev; mod theme;`. Root `Cargo.toml`: add `xiox-core = { path = "crates/xiox-core" }`, remove dependencies no file left in `src/` uses (`grep` each: expected `winreg`, `rusqlite`, `serde_json`, `idna`, `windows-sys`).

- [ ] **Step 4: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core` → every moved test runs (passed + ignored = Step 1 count); then the workspace totals → identical to Step 1. `CARGO_INCREMENTAL=0 cargo build -j 1 -p xiox-desktop` → builds, no new warnings.

- [ ] **Step 5: Commit** `refactor: crate xiox-core con i moduli del motore senza egui` — `crates/xiox-core/**`, the moves, `src/main.rs`, `Cargo.toml`, `Cargo.lock`.

### Task 5: Notifier, Outbox and the Developers state in xiox-core

**Files:**
- Create: `crates/xiox-core/src/notify.rs`, `crates/xiox-core/src/dev/mod.rs`, `crates/xiox-core/src/dev/tests.rs`
- Modify: `crates/xiox-core/src/lib.rs`, `crates/xiox-core/Cargo.toml` (+ `xiox-app`, `xiox-mukade`, `xiox-sessions`, `chrono` as in the root manifest, `serde = { version = "1", features = ["derive"] }`), `src/dev.rs` (drawing only), `src/main.rs`, `Cargo.toml` (drop `chrono` if `src/` no longer uses it)

**Interfaces:**
- Consumes: Task 4.
- Produces (`notify.rs`):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionId { Cleanup, Registry, Services, Developers }
pub trait Notifier: Send + Sync { fn notify(&self, section: SectionId); }
pub type SharedNotifier = std::sync::Arc<dyn Notifier>;
pub struct NoNotifier; // impl Notifier: does nothing
/// The sending end of a worker: `send` = `tx.send(m)` (closed receiver ignored) then `notify`.
/// `Drop` drops the sender FIRST, then notifies, so the poller sees `Disconnected`.
pub struct Outbox<M: Send> { /* tx: Option<Sender<M>>, notifier, section */ }
impl<M: Send> Outbox<M> {
    pub fn new(tx: std::sync::mpsc::Sender<M>, notifier: SharedNotifier, section: SectionId) -> Self;
    pub fn send(&self, m: M);
}
```

- `xiox_core::dev`: everything of `src/dev.rs` that is not egui — `Section`, `DevMsg`, `DevCleaned`, `View`, `Confirm`, `DevState`, `Demo`, `STALE_SCAN`, `CLEANUP_DIED`, and the pure helpers (`locked_why` … `fmt_elapsed`, `report_view` excluded). Field and helper visibility becomes `pub` where `src/dev.rs` needs it. Signature changes (the only ones): `DevState::start_scan(&mut self, notifier: SharedNotifier, admin: bool)`, `DevState::start_clean(&mut self, notifier: SharedNotifier, c: Confirm)`, `DevState::run_demo(&mut self, notifier: SharedNotifier)`; each worker builds `Outbox::new(tx, notifier, SectionId::Developers)` inside its thread and uses `out.send(..)` where it did `tx.send(..); ctx.request_repaint()`. `level_text(Level) -> (&str, Color32)` becomes `level_word(Level) -> &'static str` in core and `level_color(Level) -> Color32` in `src/dev.rs`.

- [ ] **Step 1: Write the failing tests** in `notify.rs` (`#[cfg(test)] mod tests`): `outbox_drop_disconnects_before_notifying` — a test notifier holding the `Receiver<u8>` records `try_recv()` at each `notify`; `send(1)` then drop → records `[Ok(1), Err(Disconnected)]`; `a_panicking_worker_still_notifies` — move the outbox into a thread that panics; `join()` is `Err`; the notifier was called and the receiver is `Disconnected`.

- [ ] **Step 2: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core notify` → FAIL (no `notify` module). **Step 3: Implement** `notify.rs`. **Step 4: Run** → 2 pass.

- [ ] **Step 5: Move the Developers state.** `dev/mod.rs` = the non-drawing items above with `ctx` replaced as stated; `dev/tests.rs` = the test module of `src/dev.rs` (`#[cfg(test)] mod tests;` in `dev/mod.rs`) unchanged except: `levels_have_their_words_and_colors` → `levels_have_their_words` asserting `level_word`; `demo_runs_only_on_a_fake_tree` builds its scratch dir under `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")` instead of `current_dir().join("target")` (no `crates/xiox-core/target`); `column_widths_give_the_rest_to_the_flexible_column` stays in `src/dev.rs` with `col_widths`. `src/dev.rs` keeps the drawing (`dev_tab`, `options`/`body`/`scanned_view`/`cleaned_view` as free functions over `&mut DevState`, `confirm_dialog`, tables, strips, `report_view`, `save_report_text`, `col_widths`, `level_color`) and imports `xiox_core::dev::*`. `src/main.rs`: `struct Repaint(egui::Context); impl Notifier for Repaint { fn notify(&self, _: SectionId) { self.0.request_repaint() } }` and passes `Arc::new(Repaint(ctx.clone()))` wherever a `ctx` went to `DevState`.

- [ ] **Step 6: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core dev::` → the 36 moved tests pass; workspace totals = before + 2. `CARGO_INCREMENTAL=0 cargo build -j 1 -p xiox-desktop` → ok.

- [ ] **Step 7: Commit** `refactor: stato di Sviluppatori in xiox-core con notifica al posto di egui`.

### Task 6: Developers intents, worker seam and `DevView`; the Rust view types

**Files:**
- Create: `crates/xiox-core/src/view.rs`, `crates/xiox-core/src/dev/view.rs`, `crates/xiox-core/src/dev/intent_tests.rs`
- Modify: `crates/xiox-core/src/dev/mod.rs`, `crates/xiox-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 5 (`DevState`, `Outbox`, `SharedNotifier`), the view contract.
- Produces: `view.rs` = every type of the view contract (`AppInfo` … `SectionView`, `CmdError` excluded — it belongs to the Tauri crate) plus `CleanupIntent`, `RegistryIntent`, `ServicesIntent { …, OpenConfirm { action: ServiceAction } }`, `enum ServiceAction { Disable, Manual, Restore }`, `DevIntent { …, SetRemoveMode { mode: RemoveModeView } }`, `enum RemoveModeView { Recycle, Permanent }`, `enum DevCategory { Projects, Worktrees, Sessions }`, `SectionId` re-exported from `notify`. In `dev/mod.rs`:

```rust
pub struct DevCleanJob { pub scan: Arc<DevScan>, pub plan: Option<CleanPlan>, pub sessions: Vec<(Agent, String)>, pub mode: RemoveMode,
                         pub plan_labels: HashMap<String, String>, pub session_labels: HashMap<(Agent, String), String>, pub cancel: Cancel }
#[derive(Clone, Copy)] pub struct DevWorkers { pub scan: fn(Settings, bool, Outbox<DevMsg>), pub clean: fn(DevCleanJob, Outbox<DevMsg>) }
impl DevWorkers { pub const REAL: DevWorkers; }          // the bodies of today's two thread closures
impl DevState {
    pub fn with_workers(self, w: DevWorkers) -> Self;
    pub fn select_category(&mut self, c: DevCategory) -> Result<(), String>;
    pub fn toggle(&mut self, row: &str) -> Result<(), String>;          // row ids of the contract
    pub fn select_all(&mut self, on: bool) -> Result<(), String>;
    pub fn set_inactive_days(&mut self, days: u32) -> Result<(), String>; // clamps to 1..=365, saves
    pub fn set_remove_mode(&mut self, mode: RemoveMode) -> Result<(), String>;
    pub fn set_scan_roots(&mut self, on: bool) -> Result<(), String>;
    pub fn select_root(&mut self, index: Option<usize>) -> Result<(), String>;
    pub fn add_root(&mut self, dir: PathBuf) -> Result<(), String>;      // ignores a duplicate by path_key, saves
    pub fn remove_root(&mut self) -> Result<(), String>;                 // the selected one
    pub fn analyze(&mut self, notifier: SharedNotifier, admin: bool) -> Result<(), String>;
    pub fn request_clean(&mut self) -> Result<(), String>;               // can_clean() then open_confirm()
    pub fn set_ack(&mut self, on: bool) -> Result<(), String>;
    pub fn take_ready_confirm(&mut self, permanent: bool, now_ms: i64) -> Result<Option<Confirm>, String>;
    pub fn confirm(&mut self, notifier: SharedNotifier, permanent: bool) -> Result<(), String>; // take_ready_confirm(now) + start_clean
    pub fn close_confirm(&mut self);
    pub fn cancel(&mut self) -> Result<(), String>;
    pub fn report_text(&self) -> Option<String>;                          // Some(to_text()) when idle with a non-empty result
    pub fn set_saved(&mut self, text: String, ok: bool);
    pub fn view(&self, admin: bool) -> SectionView;                       // in dev/view.rs; rev 0 (Core sets it)
}
```

Refusal rules (English messages, constants in `dev/mod.rs`: `BUSY = "Wait for the running job to finish."`, `DIALOG_OPEN = "Close the confirmation first."`, `NO_DIALOG = "No confirmation is open."`, `NOT_READY = "Read the warnings and tick “I have read the warnings” first."`): while `busy()` every intent except `cancel` and `close_confirm` → `BUSY`; while a confirmation is open, selection and settings intents → `DIALOG_OPEN`; toggles and `select_all` need `View::Scanned`; a row the table shows disabled is refused (`toggle_project`/artifact on a project `!status_allows`, worktree `!selectable`, session `blocked`, other worktrees, headers, unknown ids). `take_ready_confirm`: no dialog → `NO_DIALOG`; `!confirm_ready` → `NOT_READY` (dialog stays open); `permanent` must equal `!shortfall.is_empty()` (the only button shown); `permanent` → `make_permanent` (its error goes into `c.error`, dialog stays); then `accept_confirm(c, now_ms)` (stale → `Ok(None)` with the notice, as today).

**`DevView` mapping** (`dev/view.rs`; strings from moved helpers stay as they are until Task 18, the new ones are English):

| Field | Value |
|---|---|
| `status.badge` | no scan `Ready`; scanning `Analyzing`; cleaning `Cleaning` / `Cancelling`; scanned `Analysis done`; cleaned `cleaned_title(..)` |
| `status.text` | scanned: `<fmt_size(selected bytes)> to free · <selected> of <total> selected · <fmt_elapsed> · <how>` (how = `sizes from the disk MFT` / `folders measured one by one`); cleaned: `Space freed: <size>` |
| `job` | busy with `tick`: label `Cleaning…`/`Cancelling…`, `tick_fraction`, `tick_text`; busy without: `text`, `progress`, `null` |
| `nav` | `projects` / `worktrees` / `sessions`, name `section.name()`, note `selected_text` when > 0, bytes = the section total (Σ `reclaimable_bytes` / Σ session `bytes`), `null` without a scan |
| `options` | number `inactiveDays` `Days of inactivity` 1–365; choice `removeMode` `Deletion` (`recycle` `To the Recycle Bin (recoverable)`, `permanent` `Permanently`); text `The Recycle Bin frees no space until you empty it. “Recycle Bin” in Cleanup empties it.`; toggle `scanRoots` `Also analyze these folders`; list `roots` `Extra folders`, add `Add folder…`, remove `Remove`, empty `No folders.`; text (error) `Settings not saved: <e>` when `settings_err`; all `enabled = !busy` |
| `title` / `subtitle` | section name / the egui hint line of the section (projects, worktrees; empty for sessions) |
| `columns` | projects `[project · folder (name), last use (cell), level, size]`; worktrees `[worktree (name), repository, state (cells), size]`; sessions `[title (name), harness, project, date (cells), size]` |
| `rows` | projects: `p:` row (check `on`/`off`/`mixed` from `tri_state`, `none` without artifacts; enabled = free; sub `reason`; cells `[date]`; bytes `reclaimable_bytes`; muted/why from `locked_why`; detail `item`, `folder`, `last use`, `size`, `why`) then its `a:` children (`parent` = the `p:` id; sub `note`; level; bytes `effective_bytes`; detail `item` (`plan`-style “project · label”), `folder`, `size`, `level`, `mode`, `why`); worktrees `w:` (enabled `selectable`, cells `[repo, nothing_to_remove or reason]`, detail with `mode` `permanently (git)`), then header `h:others` `Other worktrees (created by hand, not removed from here)` and `o:` rows (`check none`, muted); sessions `s:` (enabled `blocked.is_none()`, cells `[agent_label, project folder, date]`, bytes `Some(bytes)` when a file exists else `sizeText` `session_size`, why `blocked`) |
| `mode` text | `to the Recycle Bin` / `permanently` from `settings.remove_mode` |
| `bulk` | `View::Scanned && !busy` |
| `empty` | View `Empty`: busy → `Analyzing` / `The analysis can take a minute: meanwhile you can use the other sections.`; idle → `Developer cleanup` / `Press “Analyze”: the projects come from the Claude Code, Codex, OpenCode and Pi sessions.`; scanned with no rows → `empty_text(section, failed)` |
| `notices` | `notice` (warn); `ordered_warnings` (first two as warn, the `ANALYSIS_FAILED` one followed by ` The projects and worktrees were not analyzed: press “Analyze” to try again.`, the rest in `more`); `error` (error); cleaned with `errors`: each (error) + `That part was not touched. Press “Analyze” to analyze again before retrying.`; `interrupted`: `An internal error stopped the cleanup: other items may have been touched without appearing here. Press “Analyze” to see what is left.` |
| `actions` | `analyze` `Analyze` key `a` enabled `!busy`; `cancel` `Cancel` / `Cancelling…` key `Esc` while a cancellable cleanup runs; `clean` `clean_button(section, selected)` key `Enter`, primary, enabled `can_clean()` |
| `dialog` | `confirm` → id `confirm`, `confirm_title`, text `[confirm_text(section, shown_mode), others, shortfall_text]` (present ones), `summary`, `rows`, `cautionTitle` `caution_title(n)` + `cautions` when any, `ack` `Some(c.ack)` only with cautions, `error`; buttons: no shortfall → `clean` `confirm_button(c)` primary enabled `confirm_ready`, `cancel` `Cancel`; shortfall → `permanent` `Permanently` primary enabled `confirm_ready`, `cancel` |
| `report` | View `Cleaned`, idle, `cleaned` set: title `cleaned_title`, the `CleanReport` numbers, notes (cancelled → info `Cancelled: the items not yet started stayed where they were.`; empty without errors → info `Nothing to clean.`), `byCategory`, skipped/failed with `item = c.label(path)`, `path` = display, `items` empty, `canSave = !c.is_empty()`, `saved` |
| `footer` | with a scan: `Selected in <section name>: <size> · <selection_text>` |

- [ ] **Step 1: Write the failing tests** (`dev/intent_tests.rs`, test workers: `fn scan_none(_: Settings, _: bool, _: Outbox<DevMsg>) {}` and a `clean` worker that pushes the job's `mode` and plan item count into a `static CALLS: Mutex<Vec<(RemoveMode, usize)>>` then sends `DevMsg::Cleaned(DevCleaned::new())`):
  - `toggles_follow_the_tables` — on `three_sections()`: `a:` of project `b` (Blocked) → Err; `p:a` → Ok and its artifacts flip; `w:w` → Ok; `s:claude:1` blocked session (build one) → Err; `o:`/`h:others`/`x:1` → Err.
  - `nothing_changes_while_busy_or_with_the_dialog_open` — with `job = Some(rx)` every selection/settings intent → `BUSY`; with a confirmation open → `DIALOG_OPEN`, and `close_confirm` reopens them.
  - `settings_intents_clamp_dedupe_and_stay_in_memory_with_only_root` — `only_root(tmp)`: days 0 → 1, 999 → 365; `add_root` the same dir twice → one entry; `select_root(Some(0))` + `remove_root` → gone; no settings file appears under `%APPDATA%` (persist false: `settings_err` stays `None`, nothing written — assert by `xiox_app::settings_path()` mtime unchanged or absent).
  - `confirm_needs_an_open_ready_dialog` — no dialog → `NO_DIALOG`; caution plan (`levels_state()` + dist) without ack → `NOT_READY` and `confirm` still `Some`; `permanent = true` without shortfall → Err; with ack → `Ok(Some(_))`; with a shortfall (`open_confirm_with(.., |p| vec![('D', p.expected_bytes, 0)])`) `confirm(false)` → Err, `confirm(true)` → job started and `CALLS` holds `(Permanent, 1)`.
  - `confirm_after_half_an_hour_only_warns` — `take_ready_confirm(false, t + 31 * 60 * 1000)` → `Ok(None)`, notice `STALE_SCAN`, not busy.
  - `dev_view_lists_sections_rows_and_the_clean_button` — `three_sections()`: nav counts and active, row ids `p:a`, `a:…`, children carry `parent`, `p:b` muted and disabled with its `why`, primary action label `clean_button(Projects, 1)`, `bulk` true, footer present.
  - `dev_view_dialog_shows_cautions_ack_and_shortfall` — caution confirm → `ack == Some(false)`, `clean` button disabled; after `set_ack(true)` enabled; shortfall confirm → buttons `[permanent, cancel]`.
  - `dev_view_report_after_cleaning_and_save_text` — after a `DevMsg::Cleaned` with freed 10 → `report.freedBytes == 10`, `canSave`, `report_text()` contains the `to_text()` header; `set_saved("x", true)` shows in `report.saved`.
  - `views_round_trip_through_json` (in `view.rs`) — `serde_json::to_value(view)` then `from_value` equals; the JSON has `camelCase` keys (`"sizeText"`, `"warningsTitle"`).

- [ ] **Step 2: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core dev::intent_tests view::` → FAIL.
- [ ] **Step 3: Implement** `view.rs`, the seam (move the two thread bodies into `DevWorkers::REAL` fns; `start_scan` / `start_clean` spawn a thread that calls them), the intents, `dev/view.rs`.
- [ ] **Step 4: Run** the same filter → pass; workspace totals = before + 9. `cargo build -p xiox-desktop` still ok (egui untouched).
- [ ] **Step 5: Commit** `feat(xiox-core): intenzioni, vista e punto di innesto dei lavori di Developers`.

### Task 7: Cleanup state into xiox-core (move only)

**Files:**
- Create: `crates/xiox-core/src/cleanup/mod.rs`, `crates/xiox-core/src/cleanup/tests.rs`
- Modify: `crates/xiox-core/src/lib.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Task 5 `Outbox`, `SharedNotifier`.
- Produces: `xiox_core::cleanup::{CleanMsg, CleanEnd, CleanView, Chosen, CLEAN_DIED, clean_report, analysis_title, stop_note, CleanupState}`. `CleanupState` holds, as `pub` fields with the same names, `src/main.rs:101-142` (`clean_sel` … `confirm_warnings`, `rules`, `rules_rx`, `winapp2_update`, `rule_sel`, `rule_found`, `rule_cleaned`, `rule_search`, `natives`, `native_found`, `native_cleaned`, `processes`). Methods moved from `App`: `pub fn new() -> Self` (the initialisers of `App::new`), `pub fn start_rules(&mut self, notifier: SharedNotifier, from_process: bool, update: bool)`, `pub fn chosen_rules(&self)`, `pub fn start(&mut self, also_clean: bool, notifier: SharedNotifier)` (was `start_clean`), `pub fn finish(&mut self, died: bool)`, `pub fn cancel(&self)`, `pub fn cancelling(&self) -> bool`, `pub fn busy(&self) -> bool`, `pub fn rule_warnings(&self) -> Vec<String>`, `pub fn any_selected(&self) -> bool` (the `any` of `cleaner_tab`), `pub fn poll(&mut self)` (the `rules_rx` and `clean_job` parts of `App::poll`). Worker and rules thread use `Outbox::new(.., SectionId::Cleanup)`.

- [ ] **Step 1: Move.** Cut the items above out of `src/main.rs` into `cleanup/mod.rs` (Italian strings untouched); move the four tests of `src/main.rs` into `cleanup/tests.rs` (`test_app()` becomes a `CleanupState::new()`; assertions unchanged). In `src/main.rs`, `App` gets `cleanup: CleanupState` and the drawing code reads `self.cleanup.<field>`; `clean_demo` stays in `src/main.rs` (debug only) writing `self.cleanup.*`; `App::poll` calls `self.cleanup.poll()`; `start_rules` passes `Arc::new(Repaint(ctx.clone()))`, `rules::rules_env_from_process(fake_profile)` and `rules::should_update(fake_profile)`.
- [ ] **Step 2: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core cleanup::` → the 4 moved tests pass; workspace totals unchanged; `cargo build -p xiox-desktop` ok.
- [ ] **Step 3: Commit** `refactor: stato di Pulizia file in xiox-core`.

### Task 8: Cleanup intents, worker seam and `CleanupView`

**Files:**
- Create: `crates/xiox-core/src/cleanup/view.rs`, `crates/xiox-core/src/cleanup/intent_tests.rs`
- Modify: `crates/xiox-core/src/cleanup/mod.rs`

**Interfaces:**
- Consumes: Task 6 view types; Task 7 `CleanupState`.
- Produces:

```rust
pub struct CleanJob { pub cats: Vec<usize>, pub natives: Vec<NativeItem>, pub natives_last: Vec<NativeItem>, pub dynamic: Vec<(Cleaner, Opt)>,
                      pub env: Option<Env>, pub previews: HashMap<String, NativeFound>, pub only_old: bool, pub also_clean: bool,
                      pub files_cancel: xiox_cleaner::Cancel, pub cancel: Arc<AtomicBool> }
#[derive(Clone, Copy)] pub struct CleanupWorkers { pub job: fn(CleanJob, Outbox<CleanMsg>) }
impl CleanupWorkers { pub const REAL: CleanupWorkers; }  // today's thread body of `start`
impl CleanupState {
    pub fn with(natives: Vec<NativeItem>, processes: Vec<String>, workers: CleanupWorkers) -> Self; // `new()` = with(native::all(), rules::processes(), REAL)
    pub fn select_category(&mut self, id: &str) -> Result<(), String>;   // nav ids below
    pub fn toggle(&mut self, row: &str) -> Result<(), String>;
    pub fn set_only_old(&mut self, on: bool) -> Result<(), String>;
    pub fn set_search(&mut self, text: &str);
    pub fn analyze(&mut self, notifier: SharedNotifier) -> Result<(), String>;      // any_selected && !busy → start(false)
    pub fn request_clean(&mut self) -> Result<(), String>;                          // any_selected && !busy → warnings + dialog
    pub fn confirm(&mut self, notifier: SharedNotifier) -> Result<(), String>;      // dialog open && !busy → close it, start(true)
    pub fn close_confirm(&mut self);
    pub fn cancel_job(&mut self) -> Result<(), String>;
    pub fn open_keep_list(&self) -> Result<(), String>;                             // browserdb::open_keep_list, English error
    pub fn report_text(&self) -> Option<String>;
    pub fn set_saved(&mut self, text: String, ok: bool);
    pub fn view(&self, admin: bool) -> SectionView;                                 // cleanup/view.rs
}
pub fn winapp2_matches(rs: &RuleSet, needle: &str) -> Vec<(String, Vec<usize>)>;  // the filter of egui `rules_panel`
```

Same refusal rules and constants as Task 6 (`BUSY`, `DIALOG_OPEN`, `NO_DIALOG`; `NOTHING = "Select something to analyze or clean first."`): toggles refused while busy, with the dialog open, for unknown ids and for rows `rules::blocked` / `rules::native_blocked` returns `Some`. A new field `nav: String` holds the open category (default the first).

**`CleanupView` mapping:**

| Field | Value |
|---|---|
| `nav` | one item per built-in group in `cleaner::CATEGORIES` order (id `builtin:<group>`), then each `rules::panel_groups(rules.groups or [], natives)` group (id `rules:<group>`), then `winapp2` (name `rules::WINAPP2_GROUP`) when rules are loaded and have sections; note `<n> items · <k> selected`; bytes = Σ analysed bytes of the group, `null` before an analysis |
| rows, built-in `cat:<i>` | name `c.name`, sub `c.help`, level `default_on` → `recommended` else `optional`, cells `[files]` after analysis, bytes `found[i].bytes`, why `Needs administrator: some folders are skipped` when `needs_admin && !admin` (and `found.skipped_dirs > 0` note in detail), detail `item`, `files`, `size`, `level`, `mode` (`empties the Recycle Bin` for `Kind::RecycleBin`, else `permanently`), `why` = help |
| rows, native `native:<id>` | name `label`, sub `description`, level `caution` if `warning` else by `default_on`, enabled = not blocked, why = blocked reason or warning, cells `[files or —]`, bytes, `lines` = `found.detail`, detail adds `warning` |
| rows, option `<c>/<o>` | name `rules::row_label`, sub `o.description` or `c.description` (absent → `null`), level `caution` if `o.warning` else by `o.default_on`, enabled = `rules::blocked(..).is_none()`, why = blocked reason, cells `[count]`, bytes, detail `why` only when a description exists; skipped → detail `left` `<n> items left (protected or unreadable)` and `lines` = `skipped_reasons` |
| Winapp2 | rows of `winapp2_matches(rs, rule_search)`: a `section:<name>` header (`<name> (<n>)`) then its options; `search` = `{ placeholder: "Search", value: rule_search }` only in that category; option text `rules::status_line` with `hint` = reason + `rs.warnings` |
| `options` | toggle `onlyOld` `Only delete Temp files older than 24 hours` (`cleaner::TEMP_MIN_AGE_HOURS`); button `keepList` `Edit cookie keep list` (hint: the egui hover text) when the open category holds `browserdb::COOKIES_ID` |
| `columns` | `[item (name), level, files (cell), size]` |
| `status` | badge `Ready` / `Analyzing` / `Cleaning` / `Cancelling` / `analysis_title(..).0` / `Cleanup done` or `Cleanup cancelled`; text after analysis `<total> to delete (<files> files)` |
| `notices` | `analysis_title(..).1` (error) |
| `actions` | `analyze` `Analyze` `a` enabled `!busy && any_selected`; `cancel` while busy (`Cancel` / `Cancelling…`, `Esc`); `clean` `Clean <n> items` (`Clean 1 item`, `Clean` at 0; n = selected categories + `active_natives` + `active_choices`) `Enter` primary, enabled `!busy && any_selected` |
| `dialog` | id `confirm`, title `Start cleaning?`, text `The selected files will be deleted permanently. Files in use are left where they are.`, `warningsTitle` `Warnings for the chosen options` + `confirm_warnings`, buttons `continue` `Continue` (primary) and `cancel` `Cancel` |
| `report` | from `clean_report` and `CleanView::Cleaned` (idle): numbers, `cancelled`, `items` = one per category / native / option (`cleaned_detail`: name, freed, removed, left = failed, note = first note or backup line, `lines` = native `done` + `notes`), `canSave`, `saved`; while cleaning the running totals come through `status.text` `<freed> freed` |

- [ ] **Step 1: Write the failing tests** (`cleanup/intent_tests.rs`; the test worker records each `CleanJob`'s `also_clean`, `cats` and `previews` keys in a `static JOBS: Mutex<Vec<…>>` and sends `CleanMsg::Done`; build states with `CleanupState::with(natives_for_test, vec![], TEST)` where `natives_for_test` keeps `native::all()` items whose `id` is `"env_path"` (needs_preview) and `"clipboard"`, and a hand-made `RuleSet` with one embedded cleaner with two options, one `warning`, one `unsupported`):
  - `rows_are_keyed_and_unknown_or_blocked_rows_are_refused`
  - `selection_is_frozen_while_busy_or_with_the_dialog_open`
  - `analyze_needs_something_selected`
  - `cleanup_confirm_needs_the_open_dialog` — `confirm` without dialog → Err and `JOBS` empty; `request_clean` then `confirm` → one job with `also_clean == true`, dialog closed
  - `previews_only_from_the_analysis_on_screen` — with `clean_view = Analyzed` and a `native_found` entry for `native:env_path`, the job's `previews` has that key; with `clean_view = Cleaned` it is empty
  - `cleanup_view_groups_levels_and_modes` — nav ids/order, `cat:0` mode `empties the Recycle Bin`, a default-off category `optional`, the warning option `caution`, the unsupported option disabled with its `why`, an option without description has no `why` detail entry
  - `cleanup_view_confirm_lists_the_warnings_of_the_open_dialog`
  - `cleanup_view_report_and_items_after_cleaning` — feed `CleanMsg::Cleaned`, `Native`, `Rule`, `Done` through a channel as the moved tests do
  - `winapp2_search_matches_section_cleaner_or_option`
- [ ] **Step 2: Run** `… -p xiox-core cleanup::intent_tests` → FAIL. **Step 3: Implement** (seam: move the thread body of `start` into `CleanupWorkers::REAL.job`; `start` builds the `CleanJob` exactly as today and spawns `(self.workers.job)(job, outbox)`). **Step 4: Run** → pass; workspace totals = before + 9; egui builds.
- [ ] **Step 5: Commit** `feat(xiox-core): intenzioni, vista e punto di innesto dei lavori di Cleanup`.

### Task 9: Registry and Services & RAM state, intents and views

**Files:**
- Create: `crates/xiox-core/src/regstate.rs`, `crates/xiox-core/src/svcstate.rs` (each with `#[cfg(test)] mod tests`)
- Modify: `crates/xiox-core/src/lib.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Tasks 5–6.
- Produces:

```rust
// regstate.rs — moved from src/main.rs: RegMsg, the reg_* fields (pub), start_reg_scan / start_reg_fix / the reg part of poll
#[derive(Clone, Copy)] pub struct RegistryWorkers { pub scan: fn(Vec<usize>, Outbox<RegMsg>), pub fix: fn(Vec<registry::Issue>, bool, Outbox<RegMsg>) }
pub struct RegistryState { pub sel: Vec<bool>, pub issues: Vec<registry::Issue>, pub job: Option<Receiver<RegMsg>>, pub text: String,
                           pub progress: f32, pub scanned: bool, pub confirm: bool, pub msg: String, workers: RegistryWorkers }
impl RegistryState { pub fn new() -> Self; pub fn with_workers(self, w: RegistryWorkers) -> Self; pub fn busy(&self) -> bool; pub fn poll(&mut self);
    pub fn start_scan(&mut self, n: SharedNotifier); pub fn start_fix(&mut self, with_backup: bool, n: SharedNotifier);
    pub fn toggle_category(&mut self, i: usize) -> Result<(), String>; pub fn toggle(&mut self, row: &str) -> Result<(), String>;
    pub fn select_all(&mut self, on: bool) -> Result<(), String>; pub fn scan(&mut self, n: SharedNotifier) -> Result<(), String>;
    pub fn open_fix(&mut self) -> Result<(), String>; pub fn fix(&mut self, backup: bool, n: SharedNotifier) -> Result<(), String>;
    pub fn close_confirm(&mut self); pub fn view(&self) -> SectionView; }
// svcstate.rs — moved: SvcMsg, send_svc, the svc_* fields (pub), load_services / apply_services / the svc part of poll
#[derive(Clone, Copy)] pub struct ServicesWorkers { pub load: fn(Outbox<SvcMsg>), pub apply: fn(Vec<services::Service>, Option<u32>, Outbox<SvcMsg>),
                                                   pub saved: fn() -> usize, pub memory: fn() -> sys::Memory }
pub struct ServicesState { pub services: Vec<services::Service>, pub job: Option<Receiver<SvcMsg>>, pub loaded: bool, pub all: bool,
                           pub filter: String, pub msg: String, pub confirm: Option<u32>, workers: ServicesWorkers }
impl ServicesState { pub fn new() -> Self; pub fn with_workers(self, w: ServicesWorkers) -> Self; pub fn busy(&self) -> bool; pub fn poll(&mut self, n: SharedNotifier);
    pub fn load(&mut self, n: SharedNotifier); pub fn apply(&mut self, start: Option<u32>, n: SharedNotifier);
    pub fn visible(&self) -> Vec<usize>;   // the egui filter: (all || suggestion) && (display or name contains filter, lowercase)
    pub fn set_all(&mut self, all: bool); pub fn set_search(&mut self, text: &str); pub fn toggle(&mut self, row: &str) -> Result<(), String>;
    pub fn refresh(&mut self, n: SharedNotifier) -> Result<(), String>; pub fn open_confirm(&mut self, action: ServiceAction) -> Result<(), String>;
    pub fn confirm(&mut self, n: SharedNotifier) -> Result<(), String>; pub fn close_confirm(&mut self); pub fn view(&self) -> SectionView; }
```

Rules: registry toggles refused while busy or with the dialog open; `scan` needs a ticked category and no job; `open_fix` needs a selected issue; `fix` needs the dialog. Services: a `protected` row is refused; `open_confirm(Restore)` needs `saved() > 0`, the others a non-protected selection; `confirm` needs the dialog; `apply` keeps today's `!protected` filter.

**Views.** Registry: no nav; options = one toggle `cat:<i>` per `registry::CATEGORIES` + text `Before fixing you can save a backup: double-click the .reg file to go back.`; columns `[issue (name), registry key (cell)]`; rows `i:<n>` name `problem`, sub `data`, cells `[fix.key_text()]`; `bulk` when issues exist and idle; empty before a scan `Registry cleanup` / `Press “Find issues” to find entries that point to files or programs that no longer exist.`, after an empty scan `No issues found`; notice `msg` (`ok`, or `error` when it reports a refused backup); job from `text` / `progress`; actions `scan` `Find issues` `a`, `fix` `Fix selected (<n>)…` `Enter` primary; dialog `backup`: `Save a registry backup?`, `Recommended: before the changes a copy (.reg) is saved in the Documents\Xiox folder.`, buttons `backup` `Yes, back up and fix` (primary), `nobackup` `Fix without backup`, `cancel` `Cancel`. Services: options = meter `ram` `RAM in use` (`memory()`; tone `error` > 85 %, `warn` > 65 %, else `ok`), choice `filter` (`recommended` `Recommended`, `all` `All`); `search` `Search…`; columns `[service (name), state, startup, memory (cells)]`; rows `svc:<name>` name `display`, sub `name`, cells `[Running|Stopped, start_text(), memory text (`–`, `<size> (shared)`)]`, enabled `!protected`, why `Needed by Windows: protected` or the suggestion, detail adds `why turn it off`; footer `Selected: <n> · Freeable RAM (estimate): <size>`; empty while loading the first time `Reading services…`; notice `msg` (`ok`); actions `refresh` `Refresh` `a`, `manual` `Set to Manual`, `restore` `Restore originals (<saved>)`, `disable` `Disable` `Enter` primary; dialogs `confirm` with the three title/text pairs of egui `services_tab` in English (`Restore the services?` / `Every service Xiox changed goes back to how it was.`; `Set to Manual?` / `The services stop now and Windows starts them only when needed.`; `Disable the services?` / `The services stop now and will not start again. You can always use “Restore originals”.`), buttons `continue` `Continue`, `cancel` `Cancel`. The egui strings moved in this task stay Italian until Task 18.

- [ ] **Step 1: Write the failing tests.** `regstate`: `registry_toggles_are_refused_while_busy_or_with_the_dialog_open`; `scan_needs_a_category`; `fix_and_apply_need_their_dialog` (registry half: the test `fix` worker records `with_backup`; `fix` without `open_fix` → Err, nothing recorded; `open_fix` + `fix(true)` → recorded `true`); `a_refused_backup_fixes_nothing_and_says_why` (poll `RegMsg::Fixed { refused: Some(..) }` → notice tone `error`, issues kept); `registry_view_lists_issues_and_the_backup_dialog`. `svcstate`: `protected_services_cannot_be_ticked`; `fix_and_apply_need_their_dialog` (services half: the `apply` worker records the chosen names — a protected-but-selected service is never among them); `restore_needs_a_saved_backup`; `the_filter_shows_recommended_or_all_and_searches_both_names`; `a_message_reloads_the_list` (poll `SvcMsg::Message` → `load` worker called again); `services_view_meter_footer_and_dialog_texts`.
- [ ] **Step 2: Run** `… -p xiox-core regstate svcstate` → FAIL. **Step 3: Implement**; `src/main.rs` keeps the egui drawing over `self.registry.*` / `self.services.*` and calls the moved starters with `Arc::new(Repaint(..))`. **Step 4: Run** → pass; workspace totals = before + 11; egui builds.
- [ ] **Step 5: Commit** `feat(xiox-core): stato, intenzioni e viste di Registry e Services`.

### Task 10: Tauri crate skeleton — manifests, icon, capability, config

**Files:**
- Create: `apps/desktop/src-tauri/{Cargo.toml, build.rs, tauri.conf.json, xiox-ui.manifest, xiox-ui.dev.manifest}`, `apps/desktop/src-tauri/capabilities/main.json`, `apps/desktop/src-tauri/icons/{icon.ico, icon.png, 32x32.png, 128x128.png, 128x128@2x.png}`, `apps/desktop/src-tauri/src/{main.rs, error.rs}`, `apps/desktop/scripts/make_icon.py`
- Modify: `Cargo.toml` (root `[workspace] members = [".", "crates/*", "apps/desktop/src-tauri"]`), `Cargo.lock`

**Interfaces:**
- Produces: package `xiox-ui` (edition 2024, `publish = false`, `[[bin]] name = "xiox-tauri" path = "src/main.rs"`, `[features] custom-protocol = ["tauri/custom-protocol"]`; deps `xiox-core` (path), `tauri =2.12.5`, `tauri-plugin-dialog =2.8.4`, `serde` (derive), `serde_json`, `windows-sys 0.61` features `Win32_Foundation`, `Win32_System_Threading`, `Win32_UI_WindowsAndMessaging`; build-dep `tauri-build =2.7.4`; dev-dep `tempfile`). `error.rs`: `#[derive(Serialize, Debug, Clone, PartialEq)] pub struct CmdError { pub kind: &'static str, pub message: String }` with `refused(msg)`, `failed(msg)`; `pub type CmdResult<T> = Result<T, CmdError>`.

- [ ] **Step 1: Manifests.** Copy `xiox-ui.manifest` and `xiox-ui.dev.manifest` from the repo root and add inside `<assembly>`:

```xml
<dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
```

`build.rs`: `release = PROFILE == "release"`; manifest file by profile; `println!("cargo:rerun-if-changed=…")` for both; `tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new().app_manifest(std::fs::read_to_string(file)?)))`.

- [ ] **Step 2: Config and capability.** `tauri.conf.json`: `productName "Xiox"`, `version "0.1.0"`, `identifier "dev.xiox.desktop"`, build `beforeDevCommand "npm run dev"`, `beforeBuildCommand "npm run build"`, `devUrl "http://localhost:1422"`, `frontendDist "../dist"`; one window `label "main"`, `title "Xiox"`, `width 1280`, `height 800`, `minWidth 1000`, `minHeight 640`, `theme "Dark"`, `backgroundColor "#0b0416"`, `visible false`, `center true`, decorations default (system title bar); `security.csp` = mukade's string, `security.capabilities ["main"]`; `bundle.active false`. `capabilities/main.json`: identifier `main`, windows `["main"]`, permissions exactly the two of Global Constraints, description `The Xiox window: the app's own commands and its view events. No shell, filesystem, HTTP or dialog permissions for the page: folder and save dialogs open from Rust.`

- [ ] **Step 3: Icon.** `scripts/make_icon.py` (Pillow; paths as arguments): open `docs/assets/branding/xiox-logo.png`, multiply its alpha by a mask polygon `[(150,0),(694,0),(612,412),(150,412)]` (keeps the first X, cuts the slanted edge of the I), crop to `(150,0,700,412)` then to the alpha bounding box, centre it on a square of side `1.12 × max(w,h)` filled `#0B0416`, resize to 1024 px, save to the OS temp dir. Then `npx tauri icon <that png> -o apps/desktop/src-tauri/icons` and delete everything it wrote except the five files listed. Check: `python -c "from PIL import Image; print(sorted(Image.open('apps/desktop/src-tauri/icons/icon.ico').info['sizes']))"` lists at least `(16,16) (24,24) (32,32) (48,48) (64,64) (256,256)`; look at `128x128.png` with the Read tool: one X on dark, no piece of the I.

- [ ] **Step 4: Write the failing test** in `error.rs`: `cmd_error_serializes_kind_and_message` → `serde_json::to_string(&CmdError::refused("No."))` == `{"kind":"refused","message":"No."}`. `main.rs` for now: `#![cfg_attr(windows, windows_subsystem = "windows")]`, `mod error;`, a `fn main()` that builds `tauri::Builder::default().plugin(tauri_plugin_dialog::init())`, shows the `main` window in `setup`, runs `generate_context!()`.

- [ ] **Step 5: Build and pin.** `CARGO_INCREMENTAL=0 cargo build -j 1 -p xiox-ui` (first build is long); then the `cargo update -p … --precise …` list of Global Constraints, rebuild. If tauri-build asks for `../dist`, run `npm run build` once (git-ignored). Check the debug manifest: `python -c "d=open('target/debug/xiox-tauri.exe','rb').read(); print(b'asInvoker' in d, b'requireAdministrator' in d)"` → `True False`.

- [ ] **Step 6: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-ui` → 1 pass; workspace totals = before + 1.

- [ ] **Step 7: Commit** `feat(xiox-ui): crate Tauri con manifest, icona e permessi minimi`.

### Task 11: Core aggregate, pump, `Backend` (real and fake) and the commands

**Files:**
- Create: `crates/xiox-core/src/app.rs`, `crates/xiox-core/src/pump.rs`; `apps/desktop/src-tauri/src/{backend,real,fake,commands,files}.rs`
- Modify: `crates/xiox-core/src/lib.rs`, `apps/desktop/src-tauri/src/main.rs`

**Interfaces:**
- Consumes: Tasks 6, 8, 9 states and views, Task 10 crate.
- Produces:

```rust
// xiox-core/src/app.rs
pub struct CoreOptions { pub version: String, pub admin: bool, pub only_root: Option<PathBuf>, pub load_rules: bool, pub update_rules: bool }
pub struct Core { pub cleanup: CleanupState, pub registry: RegistryState, pub services: ServicesState, pub dev: DevState,
                  admin: bool, mismatch: Option<(String, String)>, version: String, shown: SectionId, rev: u64, notifier: SharedNotifier }
impl Core {
    pub fn new(o: CoreOptions, notifier: SharedNotifier) -> Core;            // starts the rules thread when load_rules
    pub fn with_states(o: CoreOptions, notifier: SharedNotifier, cleanup: CleanupState, registry: RegistryState,
                       services: ServicesState, dev: DevState) -> Core;    // tests
    pub fn app_info(&self, fake: bool) -> AppInfo;  // profileMismatch: "Xiox is cleaning the profile of “<p>”, not that of “<s>”, who opened this Windows session."
    pub fn about(&self) -> AboutView;               // winapp2: "Winapp2: loading" | "Winapp2: version <v>" | "Winapp2: not downloaded yet"; attribution rules::ABOUT_RULES
    pub fn show(&mut self, s: SectionId) -> SectionView;  // leaving Developers → dev.release(); first Services → services.load()
    pub fn view(&mut self, s: SectionId) -> SectionView;  // poll that section, build its view, rev = ++self.rev
    pub fn cleanup(&mut self, i: CleanupIntent) -> Result<SectionView, String>;
    pub fn registry(&mut self, i: RegistryIntent) -> Result<SectionView, String>;
    pub fn services(&mut self, i: ServicesIntent) -> Result<SectionView, String>;
    pub fn developers(&mut self, i: DevIntent) -> Result<SectionView, String>;
    pub fn add_root(&mut self, dir: PathBuf) -> Result<SectionView, String>;
    pub fn report_text(&self, s: SectionId) -> Option<String>;
    pub fn set_saved(&mut self, s: SectionId, text: String, ok: bool) -> SectionView;
}
// xiox-core/src/pump.rs
pub trait ViewSink: Send + Sync + 'static { fn send(&self, view: &SectionView); }
pub const PUMP_COALESCE_MS: u64 = 100;
pub struct PumpNotifier { /* Sender<SectionId> */ }   // impl Notifier
pub fn pump_channel() -> (PumpNotifier, Receiver<SectionId>);
/// Thread: recv a section, wait PUMP_COALESCE_MS, drain the rest, then for each distinct section
/// lock the core (poison-tolerant), `view(s)` and `sink.send(&v)`. Ends when every sender is gone.
pub fn spawn_pump(core: Arc<Mutex<Core>>, rx: Receiver<SectionId>, sink: Arc<dyn ViewSink>) -> JoinHandle<()>;
```

```rust
// src-tauri/src/backend.rs
pub trait Backend: Send + Sync + 'static {
    fn app_info(&self) -> AppInfo; fn about(&self) -> AboutView;
    fn show(&self, s: SectionId) -> SectionView; fn view(&self, s: SectionId) -> SectionView;
    fn cleanup(&self, i: CleanupIntent) -> CmdResult<SectionView>; fn registry(&self, i: RegistryIntent) -> CmdResult<SectionView>;
    fn services(&self, i: ServicesIntent) -> CmdResult<SectionView>; fn developers(&self, i: DevIntent) -> CmdResult<SectionView>;
    fn add_root(&self, dir: PathBuf) -> CmdResult<SectionView>; fn report_text(&self, s: SectionId) -> Option<String>;
    fn set_saved(&self, s: SectionId, text: String, ok: bool) -> SectionView; fn is_fake(&self) -> bool;
}
pub struct BackendState(pub Arc<dyn Backend>);   // tauri managed state
// real.rs: RealBackend { core: Arc<Mutex<Core>> }, String errors → CmdError::refused
// fake.rs (#[cfg(debug_assertions)] only): FakeModel = Rust port of the Task 2 transition table over
//   include_str!("../../fixtures/views.json"); FakeBackend { model: Mutex<FakeModel>, sink } with a 200 ms ticker thread
// files.rs: pub fn write_report(path: &Path, text: &str) -> (String, bool)  // ("Report saved to <path>", true) | ("Report not saved: <e>", false)
// commands.rs: async commands of the contract, each `tauri::async_runtime::spawn_blocking` (mukade `blocking`);
//   developers_add_root: fake → backend.add_root("D:\\Work\\extra"); real → dialog().file().set_title("Folder to analyze").blocking_pick_folder()
//   save_report: report_text → refused "Nothing to save." if None; fake → set_saved("Report saved to C:\\Users\\demo\\Documents\\Xiox report.txt", true);
//   real → set_title("Save the report").set_file_name("Xiox report.txt").add_filter("Text", &["txt"]).blocking_save_file() → write_report → set_saved
```

`main.rs`: parse `--only-root <dir>`; fake when `cfg!(debug_assertions) && std::env::var("XIOX_UI_FAKE").as_deref() == Ok("1")`; in `setup`: real → unless `only_root`, `xiox_core::wipe::remove_leftovers_in_background()`; `pump_channel()`, `Core::new(CoreOptions { version: env!("CARGO_PKG_VERSION"), admin: sys::is_admin(), only_root, load_rules: true, update_rules: rules::should_update(false) }, notifier)`, `spawn_pump(core, rx, Arc::new(app.handle().clone()))` (`impl ViewSink for AppHandle` emits `xiox:view`); `app.manage(BackendState(..))`; show the window. Register the ten commands.

- [ ] **Step 1: Write the failing tests.** `app.rs`: `leaving_developers_releases_the_analysis`; `services_load_once_on_first_show` (test `load` worker counts calls); `views_carry_increasing_revs`; `about_names_winapp2_and_the_licence`; `refusals_are_english` (`developers(Confirm)` with no dialog → `Err` equal to `NO_DIALOG`; `cleanup(Toggle { row: "x" })` → an English message, no `«`); `pump.rs`: `pump_coalesces_and_publishes_views` (recording sink; 5 notifies in 10 ms → one view per section); `pump_publishes_the_view_after_a_worker_dies` (Developers `scan` test worker that panics → within 2 s the sink gets a Developers view with `job == None` and an `error` notice); `fake.rs`: `fixture_round_trips_and_holds_every_state` (deserialize into `Fixture { app, about, views: BTreeMap<SectionId, BTreeMap<String, SectionView>> }`, re-serialize, `serde_json::Value` equal; every state name of the table present); `fake_analyze_ticks_then_lands_on_analyzed`; `fake_cancel_returns_to_the_state_before_the_job`; `fake_never_reaches_the_engine` (`include_str!("fake.rs")` contains neither `xiox_core::cleaner` nor `xiox_core::dev::DevState` nor `Core::`); `files.rs`: `write_report_says_where_or_why_not` (tempdir; a path inside a missing dir fails with `Report not saved:`).
- [ ] **Step 2: Run** `… -p xiox-core app:: pump::` and `… -p xiox-ui` → FAIL.
- [ ] **Step 3: Implement** in the order `app.rs`, `pump.rs`, `backend.rs`, `real.rs`, `files.rs`, `fake.rs`, `commands.rs`, `main.rs`.
- [ ] **Step 4: Run** the filters → pass; workspace totals = before + 12; `npm run build` ok.
- [ ] **Step 5 (optional, only with a display): the fake window.** `XIOX_UI_FAKE=1 CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 npm run desktop` starts Vite and the debug window on the fixtures; check it shows the Cleanup view, close it. Never start the real backend by hand.
- [ ] **Step 6: Commit** `feat(xiox-ui): backend vero e finto, comandi ed eventi della vista`.

### Task 12: Developers end to end

**Files:**
- Modify: `apps/desktop/fixtures/views.json` (Developers states rich enough for every element: `projects` with a blocked project, a caution child and a mixed parent; `worktrees` with an other-worktrees header; `sessions` with a blocked one and a database-only one; `confirm` with cautions and `ack: false`; `shortfall`; `cleaning` with a job and the `cancel` action; `report` with skipped and failed lines and `canSave`), `apps/desktop/src/render/{list,bar2}.ts`, `apps/desktop/src/main.ts`, `apps/desktop/tests/smoke.mjs`, `crates/xiox-core/src/app.rs` (the ignored end-to-end test)

**Interfaces:**
- Consumes: Tasks 2–3 renderer, Task 11 `api.addRoot`, `api.saveReport`, `Core`.

- [ ] **Step 1: Write the failing test** `developers_clean_a_temp_project_end_to_end` (`#[ignore = "real engine on a temp tree; reads the session stores read-only"]`) in `app.rs`: temp tree `<tmp>/proj/Cargo.toml` + `<tmp>/proj/target/debug/blob.bin` (1 MiB); `Core::new(CoreOptions { only_root: Some(tmp), load_rules: false, update_rules: false, admin: false, .. }, NoNotifier)`; `developers(SetRemoveMode { mode: Permanent })`, `Analyze`, poll `view(Developers)` until not busy (≤ 120 s); toggle the `a:` row whose detail `folder` ends in `target` (if no project is listed, `git init` the folder and analyze again); `OpenConfirm`, `Confirm`; poll until a report; assert `report.freedBytes >= 1 MiB`, `target` gone, `Cargo.toml` present.
- [ ] **Step 2: Run** `CARGO_INCREMENTAL=0 cargo test -j 1 -p xiox-core app::developers_clean_a_temp_project_end_to_end -- --ignored` → PASS (the pipeline exists since Task 11; if it fails, fix the cause in core, never loosen the test or a guard).
- [ ] **Step 3: Page specifics.** Caret on parents (click, `→` / `←`); mixed checkbox (`aria-checked="mixed"`); the `roots` list (select → `selectRoot`, `Add folder…` → `api.addRoot()`, `Remove` → `removeRoot`); the number field sends on change and on Enter; `Save as text…` → `api.saveReport('developers')`.
- [ ] **Step 4: Smoke** (both sizes, `checkFit` each): `developers:projects` (Space on a project row → its children flip in the mock), `worktrees`, `sessions`, `confirm` — **Clean stays disabled until the warnings are read** (primary `[disabled]`; tick `I have read the warnings` → enabled), `shortfall` (buttons `Permanently`, `Cancel`), `cleaning` (Escape sends `cancel` → back to `projects`), `report` (`Save as text…` → saved text shown).
- [ ] **Step 5: Run** `npm test`, `npm run smoke`, `npm run build`, workspace totals (= before + 1 ignored).
- [ ] **Step 6: Commit** `feat(xiox-ui): sezione Developers completa`.

### Task 13: Cleanup end to end

**Files:**
- Modify: `apps/desktop/fixtures/views.json` (Cleanup `idle`, `analyzing`, `analyzed` for `builtin:System` and an `applications` state = nav `winapp2` active with headers, `search`, the Winapp2 status option; `confirm` with 3 warnings; `cleaning`; `report` with items whose `lines` are filled), `apps/desktop/src/render/{list,detail,bar2}.ts`, `apps/desktop/src/mockmodel.ts` (Cleanup `select` to `winapp2` goes to `applications`), `apps/desktop/tests/{mockmodel.test.ts,smoke.mjs}`

**Interfaces:**
- Consumes: Task 8 view; Tasks 2–3 renderer.

- [ ] **Step 1: Write the failing test** `mockmodel.test.ts` `selecting Applications shows its sections and the search box`.
- [ ] **Step 2: Run** `npm test` → FAIL. **Step 3: Implement** the mock branch; search input only when `view.search` is set, keeps its text and caret across re-renders (render does not replace a focused input), Escape blurs it; native `lines` in the detail box scroll; the `keepList` button. **Step 4: Run** → pass.
- [ ] **Step 5: Smoke** (both sizes, `checkFit`): `cleanup:idle`, `analyzing` (badge `ANALYZING`, progress bar), `analyzed`, `applications` (type `chro` → the mock receives `search` with `chro` after the debounce; the input still holds `chro` after the next view), `confirm` (3 warnings visible, `Continue` primary), `report` (`Skipped`, `Failed`, item lines).
- [ ] **Step 6: Run** `npm test`, `npm run smoke`, `npm run build`.
- [ ] **Step 7: Commit** `feat(xiox-ui): sezione Cleanup completa`.

### Task 14: Registry and Services & RAM end to end

**Files:**
- Modify: `apps/desktop/fixtures/views.json` (Registry `idle`, `scanning`, `analyzed` with 40 issues, `backup`, `fixing`, `fixed` with an ok notice; Services `loading`, `loaded` with protected and running rows, meter at 72 % `warn`, `confirm`), `apps/desktop/src/{main,state}.ts` (Services refresh timer), `apps/desktop/tests/{state.test.ts,smoke.mjs}`

**Interfaces:**
- Produces: `state.ts` `shouldPollServices(ui: UiState, visible: boolean): boolean` — true when the open section is `services`, the document is visible and no dialog is open; `main.ts` calls `api.view('services')` every 2000 ms while true (the RAM meter, as egui's 2 s repaint).

- [ ] **Step 1: Write the failing test** `state.test.ts` `services refresh only while shown, visible and without a dialog`. **Step 2: Run** → FAIL. **Step 3: Implement**. **Step 4: Run** → pass.
- [ ] **Step 5: Smoke** (both sizes, `checkFit`): `registry:idle`, `analyzed` (All / None buttons send `selectAll`), `backup` (three buttons, `Yes, back up and fix` focused), `fixed`; `services:loaded` (protected row checkbox disabled with its `title`; filter `All` → `setAll`), `confirm`.
- [ ] **Step 6: Run** `npm test`, `npm run smoke`, `npm run build`.
- [ ] **Step 7: Commit** `feat(xiox-ui): sezioni Registry e Services & RAM complete`.

### Task 15: About, profile strip, status, window memory, single instance, keyboard audit

**Files:**
- Create: `apps/desktop/src/render/about.ts`, `apps/desktop/src-tauri/src/{window_state,single_instance}.rs`
- Modify: `apps/desktop/src/{main,keys}.ts`, `apps/desktop/src/render/shell.ts`, `apps/desktop/src-tauri/src/main.rs`, `apps/desktop/src/mock.ts` (`__xioxMock.setMismatch(text)`), `apps/desktop/tests/{keys.test.ts,smoke.mjs}`

**Interfaces:**
- Produces:

```rust
// window_state.rs
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)] pub struct Geometry { pub x: i32, pub y: i32, pub width: u32, pub height: u32, pub maximized: bool }
pub fn path() -> PathBuf;                                   // xiox_app::settings_path().with_file_name("window.json")
pub fn load(path: &Path) -> Option<Geometry>;
pub fn save(path: &Path, g: &Geometry) -> Result<(), String>;   // write <file>.tmp then rename
pub fn fits(g: &Geometry, monitors: &[(i32, i32, u32, u32)]) -> bool; // ≥ 1000x640 and its top-left 64x64 inside a monitor
// single_instance.rs
pub struct InstanceGuard(/* HANDLE */);                    // CloseHandle on drop
pub fn acquire(name: &str) -> Option<InstanceGuard>;       // CreateMutexW; None on ERROR_ALREADY_EXISTS
pub fn focus_existing(title: &str);                        // FindWindowW(null, title) → ShowWindow(SW_RESTORE) + SetForegroundWindow
```

`main.rs`: before the builder, `acquire("Local\\XioxUi")` (fake: `Local\\XioxUiFake`); on `None` call `focus_existing("Xiox")` and return. In `setup`, real mode only: restore `load(path())` when `fits` against `available_monitors()` (else centre), then show; on `WindowEvent::CloseRequested` save the outer position, inner size and `is_maximized`. Fake mode reads and writes nothing.
Page: the sections bar ends with an `About` button (key `?` → `keyAction` `{ kind: 'about' }`, added to `KeyAction`) opening `render/about.ts` (`api.about()`: `Xiox`, `Version <version>`, `winapp2`, `attribution`, `Close`; Escape closes, focus returns). `#profile` shows `app.profileMismatch` as a warn strip above the body when non-null. Status right: `administrator` / `standard user` · `ramText`.

- [ ] **Step 1: Write the failing tests.** `window_state.rs`: `geometry_round_trips_through_its_file`; `a_window_off_every_monitor_is_not_restored`; `a_window_below_the_minimum_is_not_restored`. `single_instance.rs`: `the_second_holder_of_a_name_is_refused` (name with the process id; first `Some`, second `None`, after drop `Some` again). `keys.test.ts`: `question mark opens About outside fields and dialogs`.
- [ ] **Step 2: Run** `… -p xiox-ui` and `npm test` → FAIL. **Step 3: Implement**. **Step 4: Run** → pass; workspace totals = before + 4.
- [ ] **Step 5: Smoke** (both sizes): `about` (fits, Escape closes, focus back on `About`); profile strip via `setMismatch` (fits at 1000x640); **Tab audit** from `document.body`: pressing Tab visits, in order, the section buttons, About, the bar-2 controls, the list (rows container, `tabindex=0`; arrows move inside), the detail, the top-bar actions; every stop has a computed `box-shadow` or `outline` that is not `none`, and none is hidden behind `clip-path` (the ring is inset).
- [ ] **Step 6: Run** `npm test`, `npm run smoke`, `npm run build`, workspace totals.
- [ ] **Step 7: Commit** `feat(xiox-ui): About, avviso di profilo, finestra ricordata, istanza unica e tastiera`.

### Task 16: English at the source — model, app, engine, scanner, sessions, mukade, cleaner; the Italian check

**Files:**
- Create: `scripts/italian-check.mjs`, `scripts/italian-allow.txt`, `scripts/italian-check.test.mjs`
- Modify: user-visible string literals in `crates/{xiox-model,xiox-app,xiox-engine,xiox-scanner,xiox-sessions,xiox-mukade,xiox-cleaner,xiox-disktree,xiox-git}/src/**`, and every test anywhere in the workspace (outside `xiox-cli`, `xiox-observer`, `xiox-project`) that asserts one of them — notably `crates/xiox-app/src/tests.rs`, `crates/xiox-engine/src/tests.rs`, `crates/xiox-scanner/src/tests.rs`, `crates/xiox-core/src/dev/tests.rs`, `crates/xiox-core/src/cleanup/tests.rs` (sizes)

**Interfaces:**
- Produces: `scripts/italian-check.mjs` — `export function findItalian(text: string, kind: 'rust' | 'xml' | 'ts' | 'json'): { line: number; literal: string }[]` and a CLI `node scripts/italian-check.mjs <path>...` printing `path:line: "literal"` and exiting 1 on any hit. Rust: string literals (`"…"`, `r"…"`, `r#"…"#`) outside comments and outside `#[cfg(test)]` modules / `tests.rs` / `tests/`; XML: text of `<label>`, `<description>`, `<warning>`; TS/JSON: string literals. A literal is Italian when it matches, case-insensitive, `[àèéìòù«»]` or a whole word of: `il lo gli della delle degli dei nel nella sono non perché già più nessun nessuna nessuno questa questo cartella cartelle pulizia analisi sessione sessioni elimina eliminati cestino attenzione consigliato facoltativo annulla annullato salva ripara servizio servizi avvio chiudi errore riuscito trovati`. `italian-allow.txt`: exact literals allowed (data file names such as `cookie_da_conservare.txt`, Italian test-input paths), one per line.

**Glossary** (Tasks 16–18 use it everywhere): Pulisci → Clean, Rimuovi → Remove, Cancella → Delete, Analizza → Analyze, Consigliato → Recommended, Facoltativo → Optional, Attenzione → Caution, Cestino → Recycle Bin, Definitiva → Permanently, Progetti → Projects, Worktree → Worktrees, Sessioni archiviate → Archived sessions, cartella → folder, voce → item, resoconto → report, Saltati → Skipped, Non riusciti → Failed, Annullato → Cancelled; `«x»` → `“x”`; decimal comma → point; `byte` → `B`. Sentences pinned by tests: `STALE_ANALYSIS` = `The analysis is too old: press “Analyze” again.`; `ANALYSIS_FAILED` = `Internal error while analyzing the projects.`; `CleanReport::to_text` header `Cleanup report`, lines `Cancelled: the cleanup stopped before the end`, `Space freed: <size>`, `In the Recycle Bin: <size> (freed by emptying the Recycle Bin)`, `Files deleted: <n>`, `Folders deleted: <n>`, `Duration: <d>`, `By category:`, `Skipped (<n>):`, `Failed (<n>):`; `fmt_size` → `512 B`, `2 KB`, `3.0 MB`, `2.00 GB` (identical to the page's `formatBytes`). Comments, identifiers of the engine, rule ids, registry paths, process names and data file names do not change.

- [ ] **Step 1: Write the failing test** `scripts/italian-check.test.mjs` (`node --test scripts/`): flags `"Pulizia completata"` and `"«Analizza»"`; ignores `// la pulizia`, `"Cleanup done"`, a literal inside `#[cfg(test)] mod tests { … }`; XML `<label>Cache di Cargo</label>` flagged; allow-listed literal ignored.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** the script. **Step 4: Run** → pass.
- [ ] **Step 5: Translate** the listed crates (glossary), run `node scripts/italian-check.mjs crates/xiox-model/src crates/xiox-app/src crates/xiox-engine/src crates/xiox-scanner/src crates/xiox-sessions/src crates/xiox-mukade/src crates/xiox-cleaner/src crates/xiox-disktree/src crates/xiox-git/src` → no hit; update the asserting tests (same meaning, English text).
- [ ] **Step 6: Run** workspace totals → identical to before; any failure in `xiox-cli` / `xiox-observer` / `xiox-project` → stop and report.
- [ ] **Step 7: Commit** `refactor: testi in inglese nel motore e controllo dell'italiano`.

### Task 17: English at the source — xiox-rules and the embedded cleaners

**Files:**
- Modify: user-visible literals in `crates/xiox-rules/src/*.rs` (updater outcomes and reasons, guard / regguard / safedelete refusals, collect notes, Winapp2 parse warnings, `unsupported` reasons), the `<label>`, `<description>`, `<warning>` texts of `cleaners/*.xml` (61 files; ids, paths, `running` and actions untouched), `crates/xiox-rules/tests/*.rs` and any core test asserting those strings

- [ ] **Step 1: Translate** (glossary of Task 16). **Step 2: Run** `node scripts/italian-check.mjs crates/xiox-rules/src cleaners` → no hit. **Step 3: Run** workspace totals → identical. **Step 4: Commit** `refactor: testi in inglese in xiox-rules e nei cleaner inglobati`.

### Task 18: English at the source — xiox-core and the page

**Files:**
- Modify: user-visible literals in `crates/xiox-core/src/**` (category names `System` / `Explorer` / `Advanced` and their entries — `Recycle Bin`, `User temporary files`, `Windows temporary files`, `Memory dumps`, `Windows log files`, `Windows Error Reporting`, `Thumbnail cache`, `Old Prefetch data`, `Delivery Optimization files`; `EMBEDDED_GROUP` `Development & AI`; `WINAPP2_GROUP` `Applications`; `ABOUT_RULES`; `NO_PREVIEW`; registry categories and problems; service suggestions and `start_text` `System` / `Automatic (delayed)` / `Automatic` / `Manual` / `Disabled`; native items; `rules::status_text`; the strings moved from egui into `dev`, `cleanup`, `regstate`, `svcstate`), their tests, `apps/desktop/tests/smoke.mjs`
- Remove: `rules::REASON_IT` and `rules::reason_it` (the updater speaks English after Task 17) and their test assertions; `stop_note` matches the new English prefixes (`Skipped`, `interrupted`, `Option not supported`) **in the same commit** as the notes that produce them
- `fmt_date` → `%Y-%m-%d`; test `dates_are_day_month_year` → `dates_are_iso`

- [ ] **Step 1: Translate** (glossary). Pinned: `CLEAN_DIED` = `Cleanup stopped by an internal error: some items may already be gone. Press “Analyze” to see what is left.`; dev `CLEANUP_DIED` = `An internal error stopped the cleanup: some items may already have been removed. Press “Analyze” to see what is left.`; `analysis_title` → `Analysis in progress…`, `Analysis cancelled` (+ `Analysis cancelled: the missing items were not analyzed.`), `Analysis interrupted` (+ `An internal error stopped the analysis: the missing items were not analyzed.`), `Analysis done`; `cleaned_title` → `Cleaning…`, `Cleanup interrupted`, `Cleanup not run`, `Cleanup cancelled`, `Cleanup done`; `clean_button` → `Clean 3 folders`, `Remove 1 worktree`, `Delete 2 sessions`; `caution_title` → `1 folder at the Caution level:`; `shortfall_text` → `The Recycle Bin of D: has 0 B free, the cleanup moves 100 B. Delete permanently this time?`; `tick_text` → `512 B of 2 KB · 1 of 4 items`; `fmt_elapsed` → `Analysis in 16 s`.
- [ ] **Step 2: Smoke Italian scan.** `smoke.mjs` imports `findItalian` and fails when `document.body.innerText` of any screenshot state has a hit.
- [ ] **Step 3: Run** `node scripts/italian-check.mjs crates apps/desktop/src apps/desktop/fixtures cleaners` → no hit (allow-list only for data names); workspace totals identical; `npm test`, `npm run smoke`, `npm run build` ok.
- [ ] **Step 4: Commit** `refactor: testi in inglese in xiox-core e nella pagina`.

### Task 19: Delete the egui app; production build; docs

**Files:**
- Delete: `src/main.rs`, `src/dev.rs`, `src/theme.rs`, root `build.rs`, `xiox-ui.rc`, `xiox-ui.dev.rc`, `xiox-ui.manifest`, `xiox-ui.dev.manifest`
- Modify: `Cargo.toml` (virtual workspace: `[workspace] resolver = "3"`, `members = ["crates/*", "apps/desktop/src-tauri"]`, keep `[profile.release]`), `Cargo.lock`, `apps/desktop/src-tauri/Cargo.toml` (`[[bin]] name = "xiox-ui"`), `crates/xiox-core/src/dev/{mod,tests}.rs` (remove `Demo`, `DEMO_MARKER`, `demo_allowed`, `run_demo`, `demo_pending`, the `demo`/`demo_frames`/`screenshot` fields, `set_screenshot`, `Section::parse`, tests `demo_runs_only_on_a_fake_tree` and `sections_from_the_command_line`), fields and helpers of `dev`, `cleanup`, `regstate`, `svcstate` back to private where nothing outside their module uses them, `README.md`, `docs/xiox-cli.md` (only if it mentions the egui app), `THIRD_PARTY_NOTICES.md` (drop eframe/egui/rfd/image if listed, add Tauri — MIT or Apache-2.0)

- [ ] **Step 1: Delete and rewire**, then `CARGO_INCREMENTAL=0 cargo test -j 1 --workspace --exclude mukade` → totals = before − 3 (the three removed tests of the Decisions list).
- [ ] **Step 2: Leftovers.** `git grep -n -e egui -e eframe -e "rfd::" -e "xiox-desktop" -- ':!docs/superpowers'` → nothing.
- [ ] **Step 3: Production build.** `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 npm run desktop:release` → `target/release/xiox-ui.exe`; `python -c "d=open('target/release/xiox-ui.exe','rb').read(); print(b'requireAdministrator' in d, b'asInvoker' in d)"` → `True False` (do not launch it: it asks for UAC). Debug build check again → `asInvoker` only.
- [ ] **Step 4: Docs.** README: the app section describes the Tauri app (four sections, keyboard, `administrator` at start as today); build: `npm ci`, then `CARGO_BUILD_JOBS=1 npm run desktop:release` → `target\release\xiox-ui.exe`; `cargo build --release --locked -p xiox-cli` for `xiox.exe`; tests `cargo test --workspace --exclude mukade`, `npm test`, `npm run smoke`; tick the README roadmap line «A **Xiox** desktop app: Tauri 2…».
- [ ] **Step 5: Run** `npm test`, `npm run smoke`, `npm run build`, `node scripts/italian-check.mjs crates apps/desktop/src apps/desktop/fixtures cleaners`.
- [ ] **Step 6: Commit** `refactor: via l'app egui; build di produzione Tauri e documentazione`.

### Final: whole-branch review

A fresh reviewer reads the spec, this plan and `git diff <base>..HEAD` and checks the five Review Focus lines against the code (not only the tests), that no command or intent carries a path, that `capabilities/main.json` and the CSP are exactly as stated, that every egui behaviour listed in the spec table has a view row and an intent, and that `cargo test`, `npm test`, `npm run smoke` and the release manifest check pass on the final commit.
