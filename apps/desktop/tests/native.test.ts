// The desktop rules a later edit could quietly undo, checked on the files without running the window: the shell is Imago's (the plugin
// of imago-shell and the page of @acaprino/imago), the screens are Sigrid's parts with no class, style or stylesheet of the app's own,
// the window's capabilities, native dialogs owned by the window, and the title and taskbar the page tells the shell. What the running
// window does is checked by hand (docs/xiox-desktop.md) and by the Verify workflow.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';

const file = (path: string): string => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const exists = (path: string): boolean => existsSync(new URL(`../${path}`, import.meta.url));

/** Every file under a folder of the app, by relative path. */
function tree(path: string): string[] {
  return readdirSync(new URL(`../${path}`, import.meta.url)).flatMap((name) => {
    const inner = `${path}/${name}`;
    return statSync(new URL(`../${inner}`, import.meta.url)).isDirectory() ? tree(inner) : [inner];
  });
}
const sources = tree('src').filter((name) => /\.(tsx?|css)$/.test(name));
const screens = sources.filter((name) => name.endsWith('.tsx'));

const config = JSON.parse(file('src-tauri/tauri.conf.json')) as {
  app: { windows: { label: string; create?: boolean; decorations?: boolean; transparent?: boolean; backgroundColor?: string; windowEffects?: unknown; visible?: boolean; minWidth: number; minHeight: number }[]; security: { capabilities: string[] } };
};

test('the screens are Sigrid\'s parts: no class, no style, no stylesheet and no name of the old look in the app', () => {
  assert.ok(screens.length >= 8, 'the screens were found');
  assert.ok(!sources.some((name) => name.endsWith('.css')), 'no stylesheet in src');
  assert.ok(!exists('src/styles') && !exists('src/render') && !exists('src/dom.ts') && !exists('src/look.ts'), 'the hand-made renderers and the looks are gone');
  for (const name of [...sources, 'index.html']) {
    const text = name === 'index.html' ? file(name) : file(name);
    assert.doesNotMatch(text, /\bclassName\s*=|\bclass\s*=|classList|setAttribute\('class'|\bstyle\s*=\s*[{"]/, `${name} writes a class or a style`);
    assert.doesNotMatch(text, /\bim-[a-z]|data-im-|--im-|@acaprino\/imago\/(css|ui|shell|tauri|graph|react|tokens)/, `${name} names the look of Imago 0.4.1`);
  }
  assert.doesNotMatch(file('index.html'), /<style>|<link[^>]*stylesheet/, 'the page has no style of its own');
  assert.match(file('src/main.tsx'), /import '@acaprino\/sigrid\/styles\.css'/, 'Sigrid\'s stylesheet is the page\'s only one');
});

test('a button, a field or a row is named: a text or an aria-label, never the reason of a disabled action inside its name', () => {
  const window = file('src/screens/xiox.tsx');
  assert.match(window, /aria-describedby=\{reason === null \? undefined : `why-\$\{action\.id\}`\}/, 'the reason an action is disabled is its description');
  assert.match(window, /<Text srOnly key=\{action\.id\} id=\{`why-\$\{action\.id\}`\}>/, 'it is a sibling of the button, not inside it');
  const sidebar = file('src/screens/sidebar.tsx');
  for (const words of ['all ticked', 'some ticked', 'none ticked']) assert.ok(sidebar.includes(words), `the tick state is said in words: ${words}`);
});

test('the window is the shell\'s: built by imago-shell from the window main, with no transparency and no effects of the app\'s own', () => {
  const main = config.app.windows.find((window) => window.label === 'main');
  assert.equal(main?.create, false, 'the plugin builds it in setup, once every service is managed');
  assert.equal(main?.decorations, false, 'the title bar is the shell\'s');
  for (const key of ['transparent', 'backgroundColor', 'windowEffects', 'visible'] as const) assert.equal(main?.[key], undefined, `${key} is the crate\'s: the material is Options::material, the visibility its own`);
  const rust = file('src-tauri/src/main.rs');
  assert.match(rust, /\.plugin\(plugin\(\)\)/, 'the plugin of imago-shell');
  assert.match(rust, /open_main\(app\.handle\(\), options\)\?/, 'the window is opened with its options');
  assert.match(rust, /material: MATERIAL/, 'its ground');
  assert.match(rust, /const MATERIAL: Material = Material::Opaque \{ dark: DARK_GROUND, light: LIGHT_GROUND \}/, 'is the look\'s, opaque until the person chooses glass');
  assert.doesNotMatch(rust, /glass: true/, 'glass is off until the person chooses it');
  assert.match(rust, /\.run\(on_run_event\)/, 'without the crate\'s run handler a window built again ends the app');
  assert.match(rust, /launch\(&instance_dir\(fake\), false\)/, 'one running copy, the crate\'s');
  assert.match(rust, /\.plugin\(tauri_plugin_dialog::init\(\)\)/, 'the folder and save dialogs open from Rust');
  for (const gone of ['shell.rs', 'chrome.rs', 'desktop.rs', 'single_instance.rs']) assert.ok(!exists(`src-tauri/src/${gone}`), `${gone} is the crate's now`);
  assert.doesNotMatch(rust, /glass_window_config|guard_navigation_with_scheme|load_state|save_state|(?<![\w])(desktop|shell|chrome)::/, 'no 0.4.1 glue is left');
});

test('the capabilities: the nine permissions of the shell\'s window and imago-shell:default, both listed in tauri.conf.json', () => {
  assert.deepEqual(config.app.security.capabilities, ['main', 'shell']);
  const main = JSON.parse(file('src-tauri/capabilities/main.json')) as { windows: string[]; permissions: string[] };
  assert.deepEqual(main.windows, ['main']);
  assert.deepEqual([...main.permissions].sort(), [
    'core:event:allow-listen', 'core:event:allow-unlisten', 'core:webview:allow-set-webview-zoom', 'core:window:allow-close',
    'core:window:allow-internal-toggle-maximize', 'core:window:allow-is-maximized', 'core:window:allow-minimize',
    'core:window:allow-start-dragging', 'core:window:allow-toggle-maximize',
  ]);
  const shell = JSON.parse(file('src-tauri/capabilities/shell.json')) as { windows: string[]; permissions: string[] };
  assert.deepEqual(shell.permissions, ['imago-shell:default']);
  assert.deepEqual(shell.windows, ['main']);
  assert.doesNotMatch(file('src-tauri/tauri.conf.json'), /zoomHotkeysEnabled/, 'zoom is the page\'s job (Sigrid\'s useZoom): engine zoom is off');
});

test('the shell and the app are at one version of Imago: the npm asset and the crate\'s tag', () => {
  const pack = JSON.parse(file('../../package.json')) as { dependencies: Record<string, string> };
  const npm = /\/download\/v([\d.]+)\/acaprino-imago-[\d.]+\.tgz$/.exec(pack.dependencies['@acaprino/imago'] ?? '')?.[1];
  const crate = /imago-shell = \{ git = "https:\/\/github\.com\/acaprino\/imago", tag = "v([\d.]+)" \}/.exec(file('src-tauri/Cargo.toml'))?.[1];
  assert.ok(npm && crate, 'both are pinned');
  assert.equal(npm, crate);
  assert.match(pack.dependencies['@acaprino/sigrid'] ?? '', /^https:\/\/github\.com\/acaprino\/sigrid\/releases\/download\/v[\d.]+\/acaprino-sigrid-[\d.]+\.tgz$/);
  assert.equal(pack.dependencies['react'], pack.dependencies['react-dom'], 'one React');
});

test('native dialogs are owned by the window that opens them, and give the keyboard back to the page', () => {
  const commands = file('src-tauri/src/commands.rs');
  const opened = commands.match(/\.dialog\(\)/g)?.length ?? 0;
  assert.ok(opened > 0);
  assert.equal(commands.match(/\.set_parent\(&window\)/g)?.length ?? 0, opened, 'each file dialog has set_parent');
  assert.equal(commands.match(/blocking_(pick_folder|save_file)/g)?.length, commands.match(/focus::give_back\(/g)?.length, 'every blocking dialog of commands.rs hands the focus back');
  assert.match(file('src-tauri/src/focus.rs'), /pub fn give_back/);
  assert.match(file('src-tauri/src/dropped.rs'), /focus::give_back\(/, 'and the question about a dropped folder');
  assert.match(file('src/screens/xiox.tsx'), /addEventListener\('focus'/, 'the window regaining the focus puts it back on the list');
});

test('dragging a folder onto the window is an explicit choice: Rust takes the drop and asks first', () => {
  assert.doesNotMatch(file('src-tauri/tauri.conf.json'), /dragDropEnabled":\s*false/, 'drag and drop stays on, so the drop reaches Rust and the page is never navigated');
  const dropped = file('src-tauri/src/dropped.rs');
  assert.match(dropped, /DragDropEvent::Drop/);
  assert.match(dropped, /\.message\(/, 'the person is asked');
  assert.match(dropped, /\.parent\(window\)/, 'by a message owned by the window');
  assert.match(dropped, /add_root/, 'and the folder goes through the Developers flow');
  assert.match(file('src-tauri/src/main.rs'), /dropped::on_event/, 'wired in main.rs');
});

test('the window\'s minimum fits a 1366x768 laptop at 125 %', () => {
  const main = config.app.windows.find((window) => window.label === 'main');
  assert.ok(main);
  // 1366x768 at 125 %: 1093 logical wide and, less the 48 px taskbar, 574 high, of which the title bar and borders take about 31.
  assert.ok(main.minWidth <= 1093 - 14 && main.minHeight + 31 <= 574, `${main.minWidth}x${main.minHeight}`);
});

test('the shell is told the view and the job: the title says the view, the taskbar button the percent (clamped), and nothing else is a command', () => {
  const window = file('src/screens/xiox.tsx');
  assert.match(window, /window_\.setTitle\(`\$\{TITLES\[open\]\} - Xiox`\)/, 'the title ends with the app\'s name');
  assert.match(window, /Math\.min\(100, Math\.max\(0, Math\.round\(job\.fraction \* 100\)\)\)/, 'the percent is clamped to 0-100');
  assert.match(window, /window_\.setProgress\(percent\)/);
  const api = file('src/api.ts');
  for (const gone of ['window_ready', 'window_chrome', 'set_window_theme']) assert.ok(!api.includes(gone), `${gone} is the plugin's`);
  assert.match(file('src-tauri/src/main.rs'), /commands::save_report,\s*\]\)/, 'the app\'s commands are the eight of the contract');
});

test('the first focus is the list, after the first view; the confirmation starts on Cancel and holds Enter for a moment', () => {
  const confirm = file('src/screens/confirm.tsx');
  assert.match(confirm, /autoFocus=\{b\.id === 'cancel'\}/, 'the focus starts on Cancel, for every confirmation');
  assert.match(confirm, /dialogKeySwallowed\(event, performance\.now\(\) - openedAt\.current\)/, 'the Enter that opened the dialog does not confirm it');
  assert.match(confirm, /variant=\{b\.primary \? \(permanent \? 'destructive' : 'default'\) : 'outline'\}/, 'the danger variant comes from the view\'s flag, not from reading its words');
  assert.doesNotMatch(file('src/policy.ts'), /permanent\|cannot be undone|IRREVERSIBLE/, 'no reading of the dialog\'s English');
});

test('the list is a tree with a choice: Sigrid\'s DataTable in the order the page chose, one primary action on Enter, the tick on Space', () => {
  const list = file('src/screens/list.tsx');
  for (const prop of ['rowLevel=', 'rowOpen=', 'onRowOpenChange=', 'isGroupRow=', 'groupCell=', 'selection=', 'onRowKeyDown=', 'containerRef=', 'fill']) assert.ok(list.includes(prop), `DataTable ${prop}`);
  assert.doesNotMatch(list, /onRowPress=\{[^}]*primary/, 'Enter is the primary action through onRowKeyDown, never through the press of a row');
  assert.match(list, /act\?\.kind === 'primary'[\s\S]*?c\.primary\(\)/);
});
