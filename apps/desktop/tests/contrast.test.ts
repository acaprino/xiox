// WCAG AA (4.5:1) for the text that carries information, computed from the CSS the page ships: a token or an
// opacity that drifts under it fails here, not in the eye of whoever looks at a screenshot.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = (name: string): string => readFileSync(new URL(`../src/styles/${name}`, import.meta.url), 'utf8');
const tokens = css('tokens.css');
const list = css('list.css');

type Rgb = [number, number, number];

function token(name: string): string {
  const match = new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6,8})`).exec(tokens);
  assert.ok(match, `--${name} is defined in tokens.css`);
  return match[1] as string;
}

const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
const alpha = (hex: string): number => (hex.length === 9 ? parseInt(hex.slice(7, 9), 16) / 255 : 1);
const mix = (fg: Rgb, bg: Rgb, a: number): Rgb => fg.map((v, i) => v * a + (bg[i] as number) * (1 - a)) as Rgb;
const linear = (c: number): number => { const v = c / 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
const luminance = ([r, g, b]: Rgb): number => 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
function ratio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** The opacity a rule gives: the first `opacity` in the block whose selector contains `selector`. */
function opacityOf(selector: string): number {
  const at = list.indexOf(selector);
  assert.ok(at >= 0, `list.css has ${selector}`);
  const match = /opacity:\s*([0-9.]+)/.exec(list.slice(at, list.indexOf('}', at)));
  assert.ok(match, `${selector} sets an opacity`);
  return Number(match[1]);
}

const pane = rgb(token('pane'));
const selected = mix(rgb(token('sel-soft')), pane, alpha(token('sel-soft')));

test('the primary button: its text keeps 4.5:1 on its background', () => {
  const r = ratio(rgb(token('text-hi')), rgb(token('button-primary')));
  assert.ok(r >= 4.5, `--text-hi on --button-primary is ${r.toFixed(2)}:1`);
});

test('the ordinary button: its text keeps 4.5:1 on its background', () => {
  const r = ratio(rgb(token('text')), rgb(token('button')));
  assert.ok(r >= 4.5, `--text on --button is ${r.toFixed(2)}:1`);
});

test('a blocked row stays readable: name, cells, size and level on the pane, dimmed', () => {
  const dim = opacityOf('.r.muted > :not(.nm)');
  for (const name of ['text', 'text-2', 'ice']) {
    const r = ratio(mix(rgb(token(name)), pane, dim), pane);
    assert.ok(r >= 4.5, `--${name} at opacity ${dim} on the pane is ${r.toFixed(2)}:1`);
  }
});

test('a blocked row that is selected stays readable too (its ground is lighter)', () => {
  const dim = opacityOf('.r.muted.sel > :not(.nm)');
  for (const name of ['text', 'text-2', 'ice']) {
    const r = ratio(mix(rgb(token(name)), selected, dim), selected);
    assert.ok(r >= 4.5, `--${name} at opacity ${dim} on the selected row is ${r.toFixed(2)}:1`);
  }
});

test('the text colours that are not dimmed keep 4.5:1 on the pane', () => {
  for (const name of ['text', 'text-2', 'text-hi', 'muted', 'ice', 'warn', 'ok', 'err']) {
    const r = ratio(rgb(token(name)), pane);
    assert.ok(r >= 4.5, `--${name} on the pane is ${r.toFixed(2)}:1`);
  }
});

test('the ratio helper matches known values', () => {
  assert.equal(ratio([0, 0, 0], [255, 255, 255]).toFixed(1), '21.0');
  assert.equal(ratio([119, 119, 119], [255, 255, 255]).toFixed(2), '4.48');
});
