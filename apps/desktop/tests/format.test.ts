import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, formatDuration, ramText } from '../src/format.ts';

test('formatBytes matches the engine sizes in English', () => {
  assert.equal(formatBytes(0), '0 B'); assert.equal(formatBytes(512), '512 B'); assert.equal(formatBytes(1023), '1023 B');
  assert.equal(formatBytes(2048), '2 KB'); assert.equal(formatBytes(3 * 2 ** 20), '3.0 MB'); assert.equal(formatBytes(10 * 2 ** 30), '10.00 GB');
});
test('formatDuration', () => {
  assert.equal(formatDuration(850), '850 ms'); assert.equal(formatDuration(12_300), '12 s');
  assert.equal(formatDuration(125_000), '2 min 05 s'); assert.equal(formatDuration(3_720_000), '1 h 02 min');
});
test('ramText rounds to whole GB', () => { assert.equal(ramText(33_930_000_000), 'RAM 32 GB'); });
