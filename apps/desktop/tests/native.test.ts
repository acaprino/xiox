// The desktop rules a later edit could quietly undo, checked on the files without running the window: the
// arrow cursor, chrome that is not text, the first frame, contrast themes, the shared webview policy in Rust and
// native dialogs owned by the window. What the running window does is checked by hand (docs/xiox-desktop.md).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { SELECTABLE } from '../src/keys.ts';

const file = (path: string): string => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const styles = readdirSync(new URL('../src/styles/', import.meta.url)).filter((name) => name.endsWith('.css')).map((name) => [name, file(`src/styles/${name}`)] as const);
const sources = readdirSync(new URL('../src/render/', import.meta.url)).map((name) => [name, file(`src/render/${name}`)] as const);

test('buttons, menu entries and rows keep the arrow cursor: no hand, no "not allowed"', () => {
  for (const [name, text] of [...styles, ...sources, ['index.html', file('index.html')] as const]) {
    assert.doesNotMatch(text, /cursor:\s*(pointer|not-allowed)|cursor-pointer/, `${name} sets a web cursor`);
  }
  // The arrow and the chrome that is not text are Imago's base, which the shell carries: the app is an im-shell.
  assert.match(file('index.html'), /id="app"[^>]*class="im-shell"/, 'the arrow cursor comes from the im-shell base');
});

test('the chrome is not selectable, and every place Ctrl+A selects is (im-content-text, the package\'s own opt-in)', () => {
  const places: Record<string, [string, RegExp]> = {
    '.im-content-text': ['src/render/inspector.ts', /im-panel-body im-content-text/],
    '#notices': ['src/render/frame.ts', /id="notices" class="im-content-text"/],
    '.rp-body': ['src/render/report.ts', /rp-body[^']*im-content-text/],
    '.dlg-body': ['src/render/dialog.ts', /im-content-text im-scroll dlg-body/],
  };
  for (const [, select] of SELECTABLE) {
    const place = places[select];
    assert.ok(place, `${select} (Ctrl+A) has a place in this test`);
    assert.match(file(place[0]), place[1], `${select} (Ctrl+A) is selectable: it carries im-content-text`);
  }
});

test('the first frame is the page: a transparent glass window, hidden until drawn, the colours Imago\'s (no inline style, no scheme meta)', () => {
  const config = JSON.parse(file('src-tauri/tauri.conf.json')) as { app: { windows: { backgroundColor: string; visible: boolean; transparent: boolean }[] } };
  const main = config.app.windows[0];
  assert.ok(main);
  // Smooth is glass over Windows' acrylic: the window paints nothing of its own, and it is shown once the page has drawn.
  assert.equal(main.transparent, true, 'tauri.conf.json transparent');
  assert.equal(main.backgroundColor.toLowerCase(), '#00000000', 'tauri.conf.json backgroundColor');
  assert.equal(main.visible, false, 'tauri.conf.json visible');
  assert.doesNotMatch(file('index.html'), /<style>|name="color-scheme"/, 'the look is written on the root by applyLook, not by the page');
});

test('the window is shown by the page\'s first frame, with a fallback, never straight from setup', () => {
  const main = file('src-tauri/src/main.rs');
  assert.doesNotMatch(main, /window\.show\(\)/, 'main.rs shows the window only through desktop::show_when_drawn');
  assert.match(main, /desktop::show_when_drawn\(&window\)/);
  assert.match(main, /desktop::window_ready/, 'the command is registered');
  assert.match(file('src/main.ts'), /api\.ready\(\)/, 'the page signals its first frame');
  assert.match(file('src-tauri/src/desktop.rs'), /const SHOW_ANYWAY: Duration/);
});

test('focus ring and selection survive contrast themes: the focus ring is Imago\'s outline, and the app draws none with box-shadow', () => {
  for (const [name, text] of styles) assert.doesNotMatch(text, /:focus-visible[^{]*\{[^}]*box-shadow/, `${name} draws a focus ring a contrast theme drops`);
});

test('the window is built from the glass config of imago-shell, hidden, with the guard on its builder and the crate\'s wiring after it', () => {
  const config = JSON.parse(file('src-tauri/tauri.conf.json')) as { app: { windows: { label: string; create?: boolean; visible: boolean }[] } };
  const main = config.app.windows.find((window) => window.label === 'main');
  assert.equal(main?.create, false, 'the window is built in setup, once every service is managed');
  assert.equal(main?.visible, false, 'and opens hidden');
  const rust = file('src-tauri/src/main.rs');
  assert.match(rust, /glass_window_config\(base, data_dir\.and_then\(saved_theme\)\)/, 'the config of the glass window with the theme of last time');
  assert.match(rust, /guard_navigation_with_scheme\(WebviewWindowBuilder::from_config\(app, &config\)\?/, 'the navigation guard on the builder');
  assert.match(rust, /desktop::wire\(app, &window\)\?/, 'the wiring of the window right after it is built');
  assert.match(rust, /\.plugin\(tauri_plugin_dialog::init\(\)\)/);
  assert.doesNotMatch(rust, /desktop::plugin|native_frame|window_state::/, 'no policy plugin, frame or place of the app\'s own is left');
  const desktop = file('src-tauri/src/desktop.rs');
  for (const call of ['disable_browser_accelerators(window)', 'trim_context_menu(window)', 'watch_process_failures(', 'install_snap_layouts(window)', 'install_reload_menu(window, "Reload interface"', 'recovery_action(']) {
    assert.ok(desktop.includes(call), `desktop.rs: ${call}`);
  }
  assert.match(desktop, /if !tauri::is_dev\(\) \{\s*disable_browser_accelerators\(window\)\?;\s*trim_context_menu\(window\)\?;/, 'both in a release build only: a development build keeps reload, DevTools and the whole menu');
  assert.match(desktop, /Mutex<HashMap<String, Recovery>>/, 'a Recovery for each window label');
  assert.match(file('src-tauri/src/shell.rs'), /Mutex<HashMap<String, SnapLayouts>>/, 'and a Snap Layouts overlay for each label');
  assert.doesNotMatch(desktop, /menu_keep|add_ContextMenuRequested|webview2/, 'the context menu and WebView2 settings are the crate\'s');
});

test('a failure while the application or the window closes asks nothing: every Recovery is closed on exit and on close', () => {
  const desktop = file('src-tauri/src/desktop.rs');
  assert.match(desktop, /RunEvent::ExitRequested \{ \.\. \} \| RunEvent::Exit => close_recoveries\(app, None\)/);
  assert.match(desktop, /WindowEvent::CloseRequested \{ \.\. \} \| WindowEvent::Destroyed/);
  assert.match(file('src-tauri/src/main.rs'), /\.run\(\|app, event\| desktop::on_run_event\(app, event\)\)/, 'the run loop hands the events to it');
});

test('the window theme is asked of the crate with the look and the theme drawn, kept only when the run remembers', () => {
  const shell = file('src-tauri/src/shell.rs');
  assert.match(shell, /fn set_window_theme\(window: tauri::WebviewWindow, dir: State<'_, DataDir>, theme: String, look: Option<String>, resolved: Option<String>\)/);
  assert.match(shell, /drawn_by_page\(theme, look\.as_deref\(\), resolved\.as_deref\(\)\)/);
  assert.match(shell, /apply_theme\(&window, theme, look, resolved\)/);
  assert.match(shell, /fn data_dir\(fake: bool, only_root: bool\) -> Option<PathBuf>/, 'no data folder for the fixtures window and for --only-root');
  assert.match(shell, /Some\(dir\) => save_theme\(dir, theme\)/);
  const page = file('src/main.ts');
  assert.match(page, /setWindowTheme: inTauri \? \(word, detail\) => api\.setWindowTheme\(word, detail\.look, detail\.resolved\)/, 'applyLook tells the window');
  assert.match(file('src/api.ts'), /invoke<void>\('set_window_theme', \{ theme, look, resolved \}\)/);
  assert.match(file('src/api.ts'), /invoke<void>\('window_chrome', \{ section, job \}\)/, 'the section and the job are all the chrome command takes');
  assert.match(file('src-tauri/src/main.rs'), /shell::set_window_theme,\s*shell::window_caption_maximize,/, 'both commands are registered');
});

test('native dialogs are owned by the window that opens them', () => {
  const commands = file('src-tauri/src/commands.rs');
  const opened = commands.match(/\.dialog\(\)/g)?.length ?? 0;
  assert.ok(opened > 0);
  assert.equal(commands.match(/\.set_parent\(&window\)/g)?.length ?? 0, opened, 'each file dialog has set_parent');
  const desktop = file('src-tauri/src/desktop.rs');
  assert.equal(desktop.match(/\.message\(/g)?.length, desktop.match(/\.parent\(&window\)/g)?.length, 'each message has a parent');
});

test('zoom is the page\'s job: engine zoom off, and the one webview permission it needs', () => {
  assert.doesNotMatch(file('src-tauri/tauri.conf.json'), /zoomHotkeysEnabled/);
  const capability = JSON.parse(file('src-tauri/capabilities/main.json')) as { permissions: string[] };
  assert.ok(capability.permissions.includes('core:webview:allow-set-webview-zoom'));
});

/** The text of the call that opens at `from` (the `(` of `h(`): up to its matching parenthesis. */
function callAt(text: string, from: number): string {
  let depth = 0;
  for (let i = from; i < text.length; i++) {
    if (text[i] === '(') depth++;
    else if (text[i] === ')' && --depth === 0) return text.slice(from, i + 1);
  }
  return text.slice(from);
}

test('the window is placed through the crate, inside the work area of its monitor, and its minimum fits a 1366x768 laptop at 125 %', () => {
  const rust = file('src-tauri/src/main.rs');
  const shell = file('src-tauri/src/shell.rs');
  assert.match(shell, /work_area\(\)/, 'shell.rs reads the work area, not the monitor size');
  assert.match(shell, /clamp_to_screens\(/, 'the restored and the first-launch size go through the clamp of the crate');
  assert.match(rust, /shell::place\(saved, size, &shell::screens\(window\), scale, frame_of\(window\)\)/);
  assert.match(rust, /state_to_save\(/, 'what is remembered of the window is the crate\'s too');
  assert.match(rust, /save_state\(dir, &state\)/);
  const config = JSON.parse(file('src-tauri/tauri.conf.json')) as { app: { windows: { minWidth: number; minHeight: number }[] } };
  const main0 = config.app.windows[0];
  assert.ok(main0);
  // 1366x768 at 125 %: 1093 logical wide and, less the 48 px taskbar, 574 high, of which the title bar and borders take about 31.
  assert.ok(main0.minWidth <= 1093 - 14 && main0.minHeight + 31 <= 574, `${main0.minWidth}x${main0.minHeight}`);
  assert.equal(Number(/pub const MIN_WIDTH: f64 = (\d+)/.exec(shell)?.[1]), main0.minWidth, 'shell.rs MIN_WIDTH, which place() passes to the clamp, follows tauri.conf.json');
  assert.equal(Number(/pub const MIN_HEIGHT: f64 = (\d+)/.exec(shell)?.[1]), main0.minHeight, 'shell.rs MIN_HEIGHT follows tauri.conf.json');
});

test('the ready signal does not depend on requestAnimationFrame alone (a hidden window may never run one)', () => {
  const main = file('src/main.ts');
  const signal = /function signalReady[\s\S]*?\n\}/.exec(main)?.[0] ?? '';
  assert.match(signal, /requestAnimationFrame/, 'the double frame is still there: the first frame is drawn');
  assert.match(signal, /setTimeout\(send, READY_FALLBACK_MS\)/, 'a timer races the frames');
  assert.ok(Number(/const READY_FALLBACK_MS = (\d+)/.exec(main)?.[1]) >= 100, 'a fallback, not a zero timer that would always win and make the paint wait dead code');
  assert.match(main, /Math\.min\(100, Math\.max\(0,/, 'the percent sent to the taskbar is clamped to 0-100');
  assert.match(signal, /let sent|if \(sent\)|sent = true/, 'whichever comes first, the signal goes once');
});

test('the first focus is the list, and a native dialog gives the focus back to the page', () => {
  assert.match(file('src/main.ts'), /async function start[\s\S]*?focusList\(\);\s*signalReady\(\)/, 'the page focuses the list after the first render');
  assert.match(file('src/main.ts'), /function focusList[\s\S]*?#rows \.xi-table/, 'the list is the table of #rows');
  assert.match(file('src/render/dialog.ts'), /d\.permanent/, 'the danger variant comes from the view\'s flag');
  assert.doesNotMatch(file('src/render/policy.ts'), /permanent\|cannot be undone|IRREVERSIBLE/, 'no reading of the dialog\'s English');
  assert.match(file('src/main.ts'), /addEventListener\('focus'/, 'the window regaining the focus puts it back in the page');
  const desktop = file('src-tauri/src/desktop.rs');
  assert.match(desktop, /pub fn give_back_focus/);
  const commands = file('src-tauri/src/commands.rs');
  assert.equal(commands.match(/blocking_(pick_folder|save_file)/g)?.length, commands.match(/give_back_focus\(/g)?.length, 'every blocking dialog of commands.rs hands the focus back');
  assert.match(desktop, /give_back_focus\(&webview\)/, 'so does the recovery message');
  assert.match(file('src-tauri/src/dropped.rs'), /give_back_focus\(/, 'and the question about a dropped folder');
});

test('every button of the page has a name: a text or an aria-label, icon-only ones included', () => {
  let buttons = 0;
  for (const [name, text] of sources) {
    for (const m of text.matchAll(/h\('button'|(?<![\w.])button\(/g)) {
      const call = callAt(text, m.index + m[0].length - 1);
      buttons++;
      const named = /aria-label|aria-labelledby|h\('span'|, '[^']|`[^`]+`\)|\bkeyCaps\(/.test(call) || /^\(\s*h\('(?!i')/.test(call);
      assert.ok(named, `${name}: a button without text or aria-label: ${call.slice(0, 80)}`);
    }
  }
  assert.ok(buttons > 10, 'the scan found the buttons');
});

test('the tick state of a category is said in words, not by a picture', () => {
  const bar = file('src/render/bar2.ts');
  for (const words of ['all ticked', 'some ticked', 'none ticked']) assert.ok(bar.includes(words), words);
  assert.doesNotMatch(bar, /pip/, 'no picture of the state is left');
});

test('dragging a folder onto the window is an explicit choice: Rust takes the drop and asks first', () => {
  const config = file('src-tauri/tauri.conf.json');
  assert.doesNotMatch(config, /dragDropEnabled":\s*false/, 'drag and drop stays on, so the drop reaches Rust and the page is never navigated');
  const dropped = file('src-tauri/src/dropped.rs');
  assert.match(dropped, /DragDropEvent::Drop/);
  assert.match(dropped, /\.message\(/, 'the person is asked');
  assert.match(dropped, /\.parent\(window\)/, 'by a message owned by the window');
  assert.match(dropped, /add_root/, 'and the folder goes through the Developers flow');
  assert.match(file('src-tauri/src/main.rs'), /dropped::/, 'wired in main.rs');
});

test('the title says the section, and the single-instance lookup does not need the exact title', () => {
  assert.match(file('src-tauri/src/single_instance.rs'), /fn is_title_of/);
  assert.match(file('src-tauri/src/chrome.rs'), /fn title\(/);
  assert.match(file('src/main.ts'), /api\.chrome\(/, 'the page tells Rust the section and the job');
});

test('every scroll container gets the thin themed scrollbar from Imago: it is an im-panel-body or an im-scroll', () => {
  assert.match(file('src/render/inspector.ts'), /class: 'im-panel-body im-content-text'/, 'the details scroll in the inspector panel body');
  assert.match(file('src/render/dialog.ts'), /im-scroll dlg-rows|dlg-rows[^\n]*im-scroll/, 'the items of a confirmation too');
  assert.match(file('src/render/list.ts'), /im-panel-body xi-list/, 'the list');
  assert.match(file('src/render/bar2.ts'), /im-panel-body xi-side/, 'and the sidebar');
  for (const [name, text] of styles) assert.doesNotMatch(text, /scrollbar-width/, `${name} sets a scrollbar of its own`);
});

test('the reason an action is disabled is read as its description, never as part of its name', () => {
  const shell = file('src/render/shell.ts');
  const body = /function actionButton[\s\S]*?\n\}/.exec(shell)?.[0] ?? '';
  const start = body.indexOf('button(');
  assert.ok(start > 0, 'actionButton builds a button');
  const call = callAt(body, start + 'button'.length);
  assert.doesNotMatch(call, /sr-only/, 'the hidden reason is not a child of the button');
  assert.match(body, /h\('span', \{ class: 'sr-only', id: reasonId \}/, 'it is a sibling');
  assert.match(call, /'aria-describedby': reason === null \? null : reasonId/);
});

test('the list is announced as a grid only when it has rows, and its helpers are out of the accessibility tree', () => {
  const list = file('src/render/list.ts');
  assert.match(list, /role: 'presentation', 'data-anchor'/, 'the empty anchor between rows has no role');
  assert.match(list, /role: 'treegrid'/, 'the table is a treegrid');
  assert.match(list, /if \(empty\) \{[\s\S]*?return;/, 'an empty list draws no table at all');
  const carets = [...list.matchAll(/class: 'xi-caret'[^\n]*/g)].map((m) => m[0]);
  assert.ok(carets.length >= 1);
  for (const caret of carets) {
    assert.match(caret, /'aria-hidden': 'true'/, 'a caret is the mouse\'s: the row carries the state');
    assert.match(caret, /title:/, 'and says what it does in its tooltip');
    assert.doesNotMatch(caret, /aria-expanded/, 'aria-expanded is on the row only');
  }
  assert.match(list, /'aria-expanded': expandable \? String\(expanded\) : null/, 'the row has it');
});
