import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clampZoom, stepZoom } from '@acaprino/imago/shell';

test('a zoom Xiox 0.2.2 saved reads as the nearest step of Imago: 50 to 200 % in tenths', () => {
  // 0.2.2 stepped through 67, 75, 80, 90, 100, 110, 125, 150, 175 and 200 %; the steps are Imago's now (the level is a factor).
  for (const [saved, read] of [[0.67, 0.7], [0.75, 0.8], [0.8, 0.8], [0.9, 0.9], [1, 1], [1.1, 1.1], [1.25, 1.3], [1.5, 1.5], [1.75, 1.8], [2, 2]] as const) {
    assert.equal(clampZoom(saved), read, `${saved}`);
  }
  assert.equal(clampZoom(5), 2, 'never past 200 %');
  assert.equal(clampZoom(0.1), 0.5, 'nor under 50 %');
});

test('Imago steps the zoom by tenths and stops at both ends', () => {
  assert.equal(stepZoom(1, 1), 1.1);
  assert.equal(stepZoom(1, -1), 0.9);
  assert.equal(stepZoom(2, 1), 2, 'WCAG 1.4.4: at least 200 %, and no further');
  assert.equal(stepZoom(0.5, -1), 0.5);
  assert.equal(stepZoom(1.7, 0), 1, 'reset');
});
