import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chordAction } from '../src/keys.ts';
import { nextZoom, storedZoom, WHEEL_NOTCH, wheelPixels, ZOOM_STEPS } from '../src/zoom.ts';

test('zoom steps up to 200 % and back, stopping at both ends', () => {
  assert.equal(nextZoom(1, 'in'), 1.1);
  assert.equal(nextZoom(1, 'out'), 0.9);
  assert.equal(nextZoom(1.75, 'in'), 2);
  assert.equal(nextZoom(2, 'in'), 2, 'WCAG 1.4.4: at least 200 %, and no further');
  assert.equal(nextZoom(0.67, 'out'), 0.67);
  assert.equal(nextZoom(1.6, 'in'), 1.75, 'a value between steps goes to the next larger one');
  assert.equal(nextZoom(1.6, 'out'), 1.5);
  assert.equal(nextZoom(1.75, 'reset'), 1);
  let factor = 1;
  for (let i = 0; i < ZOOM_STEPS.length; i += 1) factor = nextZoom(factor, 'in');
  assert.equal(factor, 2);
});

test('a stored zoom is used only when it is a usable factor', () => {
  assert.equal(storedZoom('1.5'), 1.5);
  assert.equal(storedZoom(null), 1);
  assert.equal(storedZoom('abc'), 1);
  assert.equal(storedZoom('5'), 1);
  assert.equal(storedZoom('0.1'), 1);
});

test('the app\'s chords: zoom on Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0, Ctrl+A outside fields', () => {
  const free = { inField: false };
  assert.deepEqual(chordAction({ key: '=', ctrlKey: true }, free), { kind: 'zoom', step: 'in' });
  assert.deepEqual(chordAction({ key: '+', ctrlKey: true }, free), { kind: 'zoom', step: 'in' }, 'Italian layout and numeric keypad');
  assert.deepEqual(chordAction({ key: '+', ctrlKey: true, shiftKey: true }, free), { kind: 'zoom', step: 'in' }, 'US layout: Ctrl+Shift+=');
  assert.deepEqual(chordAction({ key: '-', ctrlKey: true }, free), { kind: 'zoom', step: 'out' });
  assert.deepEqual(chordAction({ key: '0', ctrlKey: true }, free), { kind: 'zoom', step: 'reset' });
  assert.deepEqual(chordAction({ key: '-', ctrlKey: true }, { inField: true }), { kind: 'zoom', step: 'out' }, 'zoom holds in a field too');
  assert.deepEqual(chordAction({ key: 'a', ctrlKey: true }, free), { kind: 'selectAll' });
  assert.equal(chordAction({ key: 'a', ctrlKey: true }, { inField: true }), null, 'a field selects its own text');
  assert.equal(chordAction({ key: '=' }, free), null, 'no Ctrl, no chord');
  assert.equal(chordAction({ key: '=', ctrlKey: true, altKey: true }, free), null, 'AltGr (Ctrl+Alt) types characters');
  assert.equal(chordAction({ key: 'r', ctrlKey: true }, free), null, 'reload is not a command of this window');
});

test('one wheel notch is one step at any zoom (WebView2 shrinks pixel deltas as the page grows)', () => {
  // What WebView2 reported for one notch, measured in the window at 110 %, 100 % and 90 %.
  for (const [deltaY, factor] of [[90.909, 1.1], [100, 1], [111.111, 0.9], [50, 2]] as const) {
    const pixels = wheelPixels(deltaY, 0, factor);
    assert.ok(pixels >= WHEEL_NOTCH && pixels < 4 * WHEEL_NOTCH, `${deltaY} at ${factor}: ${pixels}`);
  }
  assert.ok(wheelPixels(3, 1, 2) >= WHEEL_NOTCH, 'three lines are a notch whatever the zoom');
  assert.ok(Math.abs(wheelPixels(4, 0, 1)) < WHEEL_NOTCH, 'a touchpad pinch delta is a fraction of a step');
});
