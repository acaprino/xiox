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
const tokens = file('src/styles/tokens.css');

test('buttons, menu entries and rows keep the arrow cursor: no hand, no "not allowed"', () => {
  for (const [name, text] of [...styles, ...sources, ['index.html', file('index.html')] as const]) {
    assert.doesNotMatch(text, /cursor:\s*(pointer|not-allowed)|cursor-pointer/, `${name} sets a web cursor`);
  }
  assert.match(tokens, /html\s*\{[^}]*cursor:\s*default/, 'the window starts from the arrow');
});

test('the chrome is not selectable, and every place Ctrl+A selects is', () => {
  assert.match(tokens, /html\s*\{[^}]*(?<!-)user-select:\s*none/, 'html is not selectable');
  const rule = /([^{}]+)\{[^}]*(?<!-)user-select:\s*text/.exec(tokens);
  assert.ok(rule, 'tokens.css has the selectable opt-in');
  const selectors = (rule[1] as string).split(',').map((s) => s.trim());
  for (const [, select] of SELECTABLE) assert.ok(selectors.includes(select), `${select} (Ctrl+A) is selectable in tokens.css`);
  assert.ok(selectors.includes('input'), 'fields stay selectable');
});

test('the first frame is the stage colour: window, inline style and tokens agree', () => {
  const stage = /--stage:\s*(#[0-9a-f]{6})/i.exec(tokens)?.[1]?.toLowerCase();
  const config = JSON.parse(file('src-tauri/tauri.conf.json')) as { app: { windows: { backgroundColor: string; visible: boolean }[] } };
  const inline = /<style>[^<]*background:\s*(#[0-9a-f]{6})/i.exec(file('index.html'))?.[1]?.toLowerCase();
  const main = config.app.windows[0];
  assert.ok(stage && main);
  assert.equal(main.backgroundColor.toLowerCase(), stage, 'tauri.conf.json backgroundColor');
  assert.equal(inline, stage, 'the inline style in index.html, before any stylesheet');
  assert.match(file('index.html'), /<meta name="color-scheme" content="dark">/);
});

test('the window is shown by the page\'s first frame, with a fallback, never straight from setup', () => {
  const main = file('src-tauri/src/main.rs');
  assert.doesNotMatch(main, /window\.show\(\)/, 'main.rs shows the window only through desktop::show_when_drawn');
  assert.match(main, /desktop::show_when_drawn\(&window\)/);
  assert.match(main, /desktop::window_ready/, 'the command is registered');
  assert.match(file('src/main.ts'), /api\.ready\(\)/, 'the page signals its first frame');
  assert.match(file('src-tauri/src/desktop.rs'), /const SHOW_ANYWAY: Duration/);
});

test('focus ring and selection survive contrast themes (forced colors drop box-shadow and backgrounds)', () => {
  const forced = /@media \(forced-colors: active\) \{([\s\S]*?)\n\}/.exec(tokens)?.[1] ?? '';
  assert.match(forced, /:focus-visible\s*\{[^}]*outline:\s*2px solid Highlight/);
  for (const selected of ['.r.sel', '.nv.on', '.it.on']) assert.ok(forced.includes(selected), `${selected} has an outline`);
});

test('every webview gets the desktop policy from one plugin, registered on the builder', () => {
  const desktop = file('src-tauri/src/desktop.rs');
  for (const call of ['SetAreBrowserAcceleratorKeysEnabled(false)', 'SetIsBuiltInErrorPageEnabled(false)', 'add_ContextMenuRequested', 'add_ProcessFailed', 'add_NavigationCompleted', '.on_navigation(', '.on_webview_ready(']) {
    assert.ok(desktop.includes(call), `desktop.rs: ${call}`);
  }
  assert.match(file('src-tauri/src/main.rs'), /\.plugin\(desktop::plugin\(\)\)/);
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
