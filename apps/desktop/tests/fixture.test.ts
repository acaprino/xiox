// The fixture is what the Rust engine's views look like (the drift test in src-tauri compares the two): these
// checks pin the engine strings the page must be drawn with, so a hand edit of views.json cannot lose them.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import type { Fixture } from '../src/mockmodel.ts';
import type { NoticeView, SectionId, SectionView } from '../src/types.ts';

const fixture = JSON.parse(readFileSync(new URL('../fixtures/views.json', import.meta.url), 'utf8')) as Fixture;
const views = Object.entries(fixture.views).flatMap(([section, states]) => Object.entries(states).map(([state, view]) => [`${section}:${state}`, view] as const));

/** Every string the page draws from a view that is not a path: where a count may hide. */
function counts(view: SectionView): string[] {
  const texts = [view.status.text, view.subtitle, view.footer ?? '', ...view.nav.map((n) => n.note ?? ''), ...view.rows.flatMap((r) => r.cells)];
  return texts;
}

test('About carries the attribution Winapp2 asks for, in the engine words', () => {
  assert.match(fixture.about.winapp2, /^Winapp2: (version \d+|loading|not downloaded yet)$/);
  assert.equal(
    fixture.about.attribution,
    'The cleaning rules for applications come from Winapp2 (https://github.com/MoscaDotTo/Winapp2), licence CC-BY-SA-4.0, downloaded at every start.',
  );
});

test('counts are grouped with a comma, never bare digits of four or more', () => {
  for (const [name, view] of views) {
    for (const text of counts(view)) {
      assert.ok(!/(?<![0-9.,:\-\\])\b\d{4,}\b(?![0-9\-:])/.test(text), `${name}: ${JSON.stringify(text)} has an ungrouped count`);
    }
  }
  assert.equal(fixture.views.cleanup.analyzed.status.text, '13.48 GB to delete (46,999 files)');
  assert.deepEqual(fixture.views.cleanup.analyzed.rows.slice(1, 4).map((r) => r.cells[0]), ['2,311', '9,870', '18,204']);
});

test('a count of one agrees with its word', () => {
  for (const [name, view] of views) {
    const notices = [...view.notices, ...(view.report?.notes ?? [])];
    const texts = [view.status.text, view.subtitle, view.footer ?? '', view.empty?.text ?? '', ...view.nav.map((n) => n.note ?? ''), ...notices.map((n) => n.text)];
    for (const text of texts) {
      assert.ok(!/(?<![0-9,.])\b1 (items|files|rules|folders|issues|projects|worktrees|sessions)\b/.test(text), `${name}: ${JSON.stringify(text)} says "1" with a plural`);
    }
  }
});

test('a partial failure is a warning, not a success', () => {
  const partial = (section: SectionId, state: string): NoticeView => {
    const notice = fixture.views[section][state]?.notices[0];
    assert.ok(notice, `${section}:${state} has a notice`);
    return notice;
  };
  assert.equal(partial('registry', 'partial').tone, 'warn');
  assert.match(partial('registry', 'partial').text, /\d+ not fixed/);
  assert.equal(partial('registry', 'fixed').tone, 'ok', 'a clean fix stays a success');
});

test('the data files the page names are the English ones', () => {
  const text = JSON.stringify(fixture);
  for (const old of ['cookie_da_conservare', 'backup_registro', 'backup_regole']) assert.ok(!text.includes(old), `${old} is gone`);
  assert.match(text, /registry-backup-\d{8}_\d{6}\.reg/);
  assert.ok(text.includes('cookie-keep-list.txt'));
});

test('a button that confirms says its verb and what it acts on, never a bare "Continue", "Yes" or "Permanently"', () => {
  let confirming = 0;
  for (const [name, view] of views) {
    const dialog = view.dialog;
    if (dialog === null) continue;
    for (const button of dialog.buttons.filter((b) => b.primary)) {
      confirming += 1;
      assert.doesNotMatch(button.label, /^(Continue|Yes\b.*|OK|Permanently|Fix without backup)$/, `${name}: ${button.label}`);
      if (dialog.permanent) {
        assert.match(button.label, /^(Clean|Delete|Remove) [\d,]+ \S+/, `${name}: a permanent delete names its verb and object: ${button.label}`);
      }
      if (button.id === 'permanent') assert.match(button.label, / permanently$/, `${name}: the way out of a full Recycle Bin says it is for good`);
    }
    if (dialog.id === 'backup') {
      assert.deepEqual(dialog.buttons.map((b) => b.id), ['backup', 'nobackup', 'cancel']);
      assert.match(dialog.buttons[0]?.label ?? '', /^Back up and fix [\d,]+ issues?$/);
      assert.match(dialog.buttons[1]?.label ?? '', /^Fix [\d,]+ issues? without a backup$/);
    }
  }
  assert.ok(confirming >= 3, 'the fixture holds the Cleanup, Registry and Developers confirmations');
});
