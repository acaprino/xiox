import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { LOOK_KEY, readLook, readTheme, THEME_KEY, windowThemeFor } from '../src/look.ts';

const storing = (values: Record<string, string | null>): Pick<Storage, 'getItem'> => ({ getItem: (key) => values[key] ?? null });

test('every look and theme Xiox 0.2.2 stored reads back as the same look and theme', () => {
  assert.equal(LOOK_KEY, 'xiox.skin', 'the key of 0.2.2');
  assert.equal(THEME_KEY, 'xiox.theme', 'the key of 0.2.2');
  for (const look of ['smooth', 'oni'] as const) assert.equal(readLook(storing({ [LOOK_KEY]: look })), look);
  for (const theme of ['system', 'light', 'dark'] as const) assert.equal(readTheme(storing({ [THEME_KEY]: theme })), theme);
});

test('nothing stored, garbage or no storage is Smooth and the theme of Windows', () => {
  for (const raw of [null, '', 'ONI', 'sepia', 'undefined']) {
    assert.equal(readLook(storing({ [LOOK_KEY]: raw })), 'smooth', `look ${JSON.stringify(raw)}`);
    assert.equal(readTheme(storing({ [THEME_KEY]: raw })), 'system', `theme ${JSON.stringify(raw)}`);
  }
  assert.equal(readLook(null), 'smooth');
  assert.equal(readTheme(null), 'system');
  const refusing = { getItem: () => { throw new Error('blocked'); } };
  assert.equal(readLook(refusing), 'smooth', 'a webview that refuses its storage');
  assert.equal(readTheme(refusing), 'system');
});

test("the window is asked for the theme the user chose in both looks (ONI MODE has a light theme); ONI MODE with Windows' theme is dark", () => {
  for (const theme of ['system', 'light', 'dark'] as const) assert.equal(windowThemeFor('smooth', theme), theme, `smooth ${theme}`);
  assert.equal(windowThemeFor('oni', 'light'), 'light');
  assert.equal(windowThemeFor('oni', 'dark'), 'dark');
  assert.equal(windowThemeFor('oni', 'system'), 'dark', 'ONI is a dark look: it has no light backdrop to follow');
});

test('the first frame is the page: the root carries no look of its own, Imago writes it before the first paint', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  assert.doesNotMatch(html, /data-skin|data-theme=|name="color-scheme"|<style>/, 'no look, no colour-scheme meta and no inline stage colour in the page');
  assert.match(html, /id="app"[^>]*class="im-shell"/, 'the app is an im-shell, so the package\'s base applies');
  const main = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
  assert.match(main, /import '@acaprino\/imago\/css\/imago\.css';\r?\nimport '\.\/styles/, 'the package stylesheet is imported first');
  assert.match(main, /applyLook\(document\.documentElement,\s*\{\s*app: 'xiox'/, 'the look is written on the root');
});
