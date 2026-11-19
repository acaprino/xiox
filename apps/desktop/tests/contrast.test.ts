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

/** A tint of tokens.css: `--name: color-mix(in srgb, var(--a) N%, var(--b))`, computed the way the browser mixes it. */
function derived(name: string): Rgb {
  const match = new RegExp(String.raw`--${name}:\s*color-mix\(in srgb,\s*var\(--([a-z-]+)\)\s*(\d+)%,\s*var\(--([a-z-]+)\)\)`).exec(tokens);
  assert.ok(match, `--${name} is a color-mix of two tokens in tokens.css`);
  return mix(rgb(token(match[1] as string)), rgb(token(match[3] as string)), Number(match[2]) / 100);
}
const solidSel = rgb(token('sel'));
const box = mix(rgb(token('surface')), pane, alpha(token('box')));
const shell = css('shell.css');

test('on the solid --sel (the active section and the active category) the texts that sit there keep 4.5:1', () => {
  for (const name of ['text-hi']) {
    const r = ratio(rgb(token(name)), solidSel);
    assert.ok(r >= 4.5, `--${name} on --sel is ${r.toFixed(2)}:1`);
  }
  // The pairs that fail there are never used on it: --muted (2.97), --text-2 (3.73), --warn (2.82), --ice (4.45).
  assert.match(shell, /\.nv\.on kbd\s*\{\s*color:\s*var\(--text-hi\)/, 'the key of the active section is --text-hi');
  assert.match(shell, /\.it\.on small,\s*\.it\.on b\s*\{\s*color:\s*var\(--text-hi\)/, 'the note and the size of the active category are --text-hi');
});

test('the muted notes keep 4.5:1 on every ground they sit on', () => {
  const grounds: Record<string, Rgb> = {
    box, band: rgb(token('band')), 'band-hi': rgb(token('band-hi')), 'selected row': selected,
    hover: mix(rgb(token('sel')), pane, alpha(token('hover'))),
  };
  for (const [name, ground] of Object.entries(grounds)) {
    const r = ratio(rgb(token('muted')), ground);
    assert.ok(r >= 4.5, `--muted on ${name} is ${r.toFixed(2)}:1`);
  }
  // On the selected category header (--band-sel) --muted is 3.49:1, so its note is --ice there.
  assert.ok(ratio(rgb(token('ice')), rgb(token('band-sel'))) >= 4.5, '--ice on --band-sel');
  assert.match(list, /\.r\.grp\.sel \.nm-text \.s\s*\{\s*color:\s*var\(--ice\)/);
});

test('buttons keep 4.5:1 in their hover and pressed states, the danger variant included', () => {
  const pairs: [string, string, Rgb][] = [
    ['text', 'hover-btn', rgb(token('text'))], ['text', 'press-btn', rgb(token('text'))],
    ['text-hi', 'hover-primary', rgb(token('text-hi'))], ['text-hi', 'press-primary', rgb(token('text-hi'))],
    ['text-hi', 'danger-bg', rgb(token('text-hi'))], ['text-hi', 'danger-hover', rgb(token('text-hi'))], ['text-hi', 'danger-press', rgb(token('text-hi'))],
  ];
  for (const [fg, bg, color] of pairs) {
    const r = ratio(color, derived(bg));
    assert.ok(r >= 4.5, `--${fg} on --${bg} is ${r.toFixed(2)}:1`);
  }
});

test('the error colour and the hover row text keep 4.5:1 where fields and rows use them', () => {
  assert.ok(ratio(rgb(token('err')), box) >= 4.5, '--err on a field');
  const hover = mix(rgb(token('sel')), pane, alpha(token('hover')));
  for (const name of ['text', 'text-2', 'text-hi', 'ice']) assert.ok(ratio(rgb(token(name)), hover) >= 4.5, `--${name} on the hovered row`);
});

test('the secondary text is 12 px at the least: no font size below the scale is left in the styles', () => {
  for (const name of ['shell.css', 'list.css', 'dialog.css', 'frame.css', 'tokens.css', 'smooth.css']) {
    const small = [...css(name).matchAll(/font(?:-size)?:[^;}]*?\b(\d+(?:\.\d+)?)px/g)].filter((m) => Number(m[1]) < 12);
    assert.deepEqual(small.map((m) => m[0]), [], `${name} sets a font below 12px`);
  }
});

// ---- Smooth: its own tokens (smooth.css), the same AA floor.

const smooth = css('smooth.css');
function smoothToken(name: string): string {
  const match = new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6,8})`).exec(smooth);
  assert.ok(match, `--${name} is defined in smooth.css`);
  return match[1] as string;
}
const smoothPane = rgb(smoothToken('pane'));

test('Smooth: words, numbers and states keep 4.5:1 on the pane, the category band and the selected row', () => {
  const grounds: [string, Rgb][] = [
    ['pane', smoothPane], ['band', rgb(smoothToken('band'))],
    ['selected', mix(rgb(smoothToken('sel-soft')), smoothPane, alpha(smoothToken('sel-soft')))],
  ];
  for (const name of ['text', 'text-2', 'muted', 'amber', 'ice']) {
    for (const [ground, colour] of grounds) {
      const r = ratio(rgb(smoothToken(name)), colour);
      assert.ok(r >= 4.5, `Smooth --${name} on the ${ground} is ${r.toFixed(2)}:1`);
    }
  }
});

test('Smooth: the primary button keeps 4.5:1 at rest, hovered and pressed', () => {
  for (const ground of ['amber', 'hover-primary', 'press-primary']) {
    const r = ratio(rgb(smoothToken('on-amber')), rgb(smoothToken(ground)));
    assert.ok(r >= 4.5, `Smooth --on-amber on --${ground} is ${r.toFixed(2)}:1`);
  }
});

test('Smooth: a blocked row, dimmed by the shared rule, stays readable', () => {
  const dim = opacityOf('.r.muted > :not(.nm)');
  for (const name of ['text', 'text-2', 'amber']) {
    const r = ratio(mix(rgb(smoothToken(name)), smoothPane, dim), smoothPane);
    assert.ok(r >= 4.5, `Smooth --${name} at opacity ${dim} is ${r.toFixed(2)}:1`);
  }
});

// ---- Smooth in the light (it follows the Windows theme): the values of its light block, the same floor.

const lightBlock = smooth.slice(smooth.indexOf('&[data-theme="light"]'));
function lightToken(name: string): string {
  const match = new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6,8})`).exec(lightBlock);
  return match ? match[1] as string : smoothToken(name);
}
const lightPane = rgb(lightToken('pane'));

test('Smooth light: words, amber text and the focus ring keep their ratios on the sheet, the band and the selected row', () => {
  const grounds: [string, Rgb][] = [
    ['sheet', lightPane], ['band', rgb(lightToken('band'))], ['ground', rgb(lightToken('stage'))],
    ['selected', mix(rgb(lightToken('sel-soft')), lightPane, alpha(lightToken('sel-soft')))],
  ];
  for (const name of ['text', 'text-2', 'muted', 'amber-ink', 'ice', 'warn', 'err']) {
    for (const [ground, colour] of grounds) {
      const r = ratio(rgb(lightToken(name)), colour);
      assert.ok(r >= 4.5, `Smooth light --${name} on the ${ground} is ${r.toFixed(2)}:1`);
    }
  }
  for (const [name, floor] of [['live', 3], ['line-strong', 3]] as const) {
    const r = ratio(rgb(lightToken(name)), lightPane);
    assert.ok(r >= floor, `Smooth light --${name} (a ring, a box) on the sheet is ${r.toFixed(2)}:1`);
  }
});

test('Smooth light: a blocked row, dimmed less on white, stays readable', () => {
  const dim = Number(/r\.muted > :not\(\.nm\)[^{]*\{\s*opacity:\s*([0-9.]+)/.exec(lightBlock)?.[1]);
  assert.ok(dim > 0, 'the light block sets its own dimming');
  for (const name of ['text', 'text-2']) {
    const r = ratio(mix(rgb(lightToken(name)), lightPane, dim), lightPane);
    assert.ok(r >= 4.5, `Smooth light --${name} at opacity ${dim} is ${r.toFixed(2)}:1`);
  }
});
