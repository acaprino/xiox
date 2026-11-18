import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { effectiveTheme, otherSkin, storedSkin, storedTheme, windowTheme } from '../src/skin.ts';

test('the window opens in Smooth unless ONI MODE was chosen', () => {
  assert.equal(storedSkin(null), 'smooth');
  assert.equal(storedSkin(''), 'smooth');
  assert.equal(storedSkin('ONI'), 'smooth', 'only the exact stored word');
  assert.equal(storedSkin('oni'), 'oni');
  assert.equal(otherSkin('smooth'), 'oni');
  assert.equal(otherSkin('oni'), 'smooth');
});

test('the first frame is already Smooth, and the switch is in the page with its state', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  assert.match(html, /<html[^>]*data-skin="smooth"/);
  assert.match(html, /id="skin"[^>]*role="switch"[^>]*aria-checked="false"/);
});

test('every Smooth rule is scoped to the Smooth root, so ONI MODE sees none of them', () => {
  const css = readFileSync(new URL('../src/styles/smooth.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  // Top-level blocks: the scoped root, the root rule of the switch and the keyframes. Nothing else.
  let depth = 0;
  const tops: string[] = [];
  let start = 0;
  for (let i = 0; i < css.length; i += 1) {
    if (css[i] === '{') { if (depth === 0) tops.push(css.slice(start, i).trim()); depth += 1; }
    if (css[i] === '}') { depth -= 1; if (depth === 0) start = i + 1; }
  }
  for (const selector of tops) assert.match(selector, /^(:root\[data-skin="smooth"\]|@keyframes |@font-face)/, `unscoped: ${selector}`);
});

test('Smooth follows Windows unless light or dark was chosen; ONI MODE asks for dark whatever the choice', () => {
  assert.equal(storedTheme(null), 'system');
  assert.equal(storedTheme('sepia'), 'system');
  assert.equal(storedTheme('light'), 'light');
  assert.equal(storedTheme('dark'), 'dark');
  assert.equal(windowTheme('smooth', 'system'), 'system');
  assert.equal(windowTheme('smooth', 'light'), 'light');
  assert.equal(windowTheme('oni', 'light'), 'dark');
});

test('the palette is the chosen theme, or the one of Windows for Auto', () => {
  assert.equal(effectiveTheme('system', true), 'light');
  assert.equal(effectiveTheme('system', false), 'dark');
  assert.equal(effectiveTheme('light', false), 'light');
  assert.equal(effectiveTheme('dark', true), 'dark');
});
