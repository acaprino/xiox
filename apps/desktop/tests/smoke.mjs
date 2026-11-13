// Smoke test: the page on Vite with the mocked commands (src/mock.ts), screenshots under
// tests/out/ (git-ignored), token / fit / error checks and the mock round trips (a click goes to the
// model and comes back as a view). Playwright is a pinned devDependency (1.63.0); its Chromium is
// the one already in %LOCALAPPDATA%\ms-playwright.
//   npm run smoke
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { mkdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const out = `${here}out/`;
mkdirSync(out, { recursive: true });

// Every state of the fixture is drawn and fit-checked; the ones below also get the click checks.
const fixture = JSON.parse(readFileSync(`${here}../fixtures/views.json`, 'utf8'));
export const SECTIONS = Object.entries(fixture.views).flatMap(([section, states]) => Object.keys(states).map((state) => [section, state]));
const ANALYZED = new Set(['cleanup:analyzed', 'registry:analyzed', 'services:loaded', 'developers:projects', 'developers:worktrees', 'developers:sessions']);
const VIEWPORTS = [[1000, 640], [1440, 900]];

/** Tokens the whole look rests on: a wrong value here means the palette drifted from the mock-up. */
export async function checkTokens(page) {
  const found = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    const token = (name) => root.getPropertyValue(name).trim().toLowerCase();
    return {
      '--stage': token('--stage'), '--line': token('--line'), '--text': token('--text'), '--ice': token('--ice'),
      body: getComputedStyle(document.body).backgroundColor,
    };
  });
  const want = { '--stage': '#0b0416', '--line': '#3f6fb5', '--text': '#ffb21e', '--ice': '#9cc4f5', body: 'rgb(11, 4, 22)' };
  return Object.entries(want).filter(([key, value]) => found[key] !== value)
    .map(([key, value]) => `tokens: ${key} is ${JSON.stringify(found[key])}, expected ${value}`);
}

/** Nothing clipped, nothing overlapping: the failures as text lines. */
export async function checkFit(page, label) {
  return page.evaluate((where) => {
    const bad = [];
    const name = (node) => `${node.tagName.toLowerCase()}${node.id ? `#${node.id}` : ''}${node.className ? `.${String(node.className).split(' ')[0]}` : ''} "${(node.textContent || '').trim().slice(0, 30)}"`;
    for (const node of document.querySelectorAll('[data-fit]')) {
      if (node.scrollWidth > node.clientWidth + 1 || node.scrollHeight > node.clientHeight + 1) {
        bad.push(`${where}: clipped ${name(node)} (${node.scrollWidth}x${node.scrollHeight} in ${node.clientWidth}x${node.clientHeight})`);
      }
    }
    for (const node of document.querySelectorAll('.ellipsis')) {
      if (node.scrollWidth > node.clientWidth + 1 && !(node.getAttribute('title') || '').trim()) {
        bad.push(`${where}: truncated without a title ${name(node)}`);
      }
    }
    if (document.documentElement.scrollWidth > innerWidth) bad.push(`${where}: page scrolls sideways (${document.documentElement.scrollWidth} > ${innerWidth})`);
    for (const sel of ['#rows .grid', '#bar2 .items', '#detail']) {
      const node = document.querySelector(sel);
      if (node && node.scrollWidth > node.clientWidth + 1) bad.push(`${where}: ${sel} scrolls sideways (${node.scrollWidth} > ${node.clientWidth})`);
    }
    const overlaps = (boxes) => {
      const rects = boxes.map((node) => [node, node.getBoundingClientRect()]);
      for (let i = 0; i < rects.length; i += 1) {
        for (let j = i + 1; j < rects.length; j += 1) {
          const [a, ra] = rects[i];
          const [b, rb] = rects[j];
          if (ra.width && rb.width && ra.left < rb.right - 1 && rb.left < ra.right - 1 && ra.top < rb.bottom - 1 && rb.top < ra.bottom - 1) {
            bad.push(`${where}: overlap ${name(a)} / ${name(b)}`);
          }
        }
      }
    };
    const top = [...document.querySelectorAll('#actions > *')];
    const version = document.getElementById('version');
    if (version) top.push(version);
    // The logo, the version and the actions share one 38 px bar: all inside it, none on another.
    const wordmark = document.getElementById('wordmark');
    const bar = document.getElementById('top');
    if (wordmark && bar) {
      if (!wordmark.complete || wordmark.naturalWidth === 0) bad.push(`${where}: the wordmark did not load`);
      top.push(wordmark);
      const box = bar.getBoundingClientRect();
      for (const node of top) {
        const r = node.getBoundingClientRect();
        if (r.left < box.left - 0.5 || r.right > box.right + 0.5 || r.top < box.top - 0.5 || r.bottom > box.bottom + 0.5) bad.push(`${where}: ${name(node)} sticks out of the top bar`);
      }
      if (bar.scrollWidth > bar.clientWidth + 1) bad.push(`${where}: the top bar scrolls sideways`);
    }
    overlaps(top);
    // A running job: the bar is wide enough to read and its pieces do not collide.
    const job = document.getElementById('job');
    if (job && !job.hidden) {
      const track = job.querySelector('.track');
      if (!track || track.getBoundingClientRect().width < 100) bad.push(`${where}: the progress bar is narrower than 100 px`);
      if (job.scrollWidth > job.clientWidth + 1) bad.push(`${where}: the job row overflows`);
      overlaps([...job.children]);
    }
    // The confirmation: inside the window, its parts stacked, nothing sideways.
    const dlg = document.querySelector('#modal:not([hidden]) .dlg');
    if (dlg) {
      const r = dlg.getBoundingClientRect();
      if (r.left < 0 || r.top < 0 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) bad.push(`${where}: the dialog sticks out of the window`);
      const inner = dlg.querySelector('.dlg-body');
      if (inner && inner.scrollWidth > inner.clientWidth + 1) bad.push(`${where}: the dialog scrolls sideways`);
      overlaps([...dlg.children]);
      for (const row of dlg.querySelectorAll('.dr')) overlaps([...row.children]);
    }
    // The report: nothing sideways, the columns of an item row apart.
    const report = document.querySelector('#rows .report');
    if (report) {
      const inner = report.querySelector('.rp-body');
      if (inner && inner.scrollWidth > inner.clientWidth + 1) bad.push(`${where}: the report scrolls sideways`);
      for (const row of report.querySelectorAll('.rp-row')) overlaps([...row.children].filter((n) => !n.classList.contains('caret-gap')));
    }
    // Inside a row: name, cells, level and size each keep their own column.
    for (const row of document.querySelectorAll('#rows .r:not(.h):not(.sec)')) {
      overlaps([...row.querySelectorAll(':scope > .nm, :scope > .cell, :scope > .lv, :scope > .sz, :scope > .ck')]);
    }
    // The list never runs under the detail panel.
    const grid = document.querySelector('#rows .grid');
    const detail = document.getElementById('detail');
    if (grid && detail && detail.children.length && grid.getBoundingClientRect().bottom > detail.getBoundingClientRect().top + 1) {
      bad.push(`${where}: the list runs under the detail panel`);
    }
    return bad;
  }, label);
}

/** Every section back on a state without a dialog or a report (a dimmed page cannot be clicked). */
const resetAll = (page) => page.evaluate(() => {
  for (const [s, st] of [['developers', 'idle'], ['services', 'loaded'], ['registry', 'idle'], ['cleanup', 'idle']]) window.__xioxMock.go(s, st);
});

/** One click on the first enabled check box must come back flipped from the mock model. */
async function checkToggle(page, label) {
  const box = page.locator('#rows .r[data-row] [role=checkbox]:not([disabled])').first();
  if (await box.count() === 0) return [`${label}: no enabled check box to click`];
  const id = await box.evaluate((node) => node.closest('[data-row]').dataset.row);
  const before = await box.getAttribute('aria-checked');
  await box.click();
  const want = before === 'on' || before === 'true' ? 'false' : 'true';
  try {
    await page.waitForFunction(([row, was]) => {
      const now = document.querySelector(`#rows [data-row="${CSS.escape(row)}"] [role=checkbox]`)?.getAttribute('aria-checked');
      return now !== undefined && now !== null && now !== was;
    }, [id, before], { timeout: 2000 });
  } catch {
    return [`${label}: clicking the check box of ${id} did not change aria-checked (still ${before})`];
  }
  const after = await page.locator(`#rows [data-row="${id}"] [role=checkbox]`).getAttribute('aria-checked');
  return after === want ? [] : [`${label}: ${id} went ${before} -> ${after}, expected ${want}`];
}

/**
 * Every row the fixture state has: its check box is disabled exactly when the row is, a disabled one says
 * why in its title, and a blocked row (disabled and muted) is listed, dimmed, with its reason on the row
 * and, once selected, in the detail panel.
 */
async function checkRows(page, label, view) {
  const failures = [];
  for (const row of view.rows) {
    if (row.kind === 'header') continue;
    const node = page.locator(`#rows [data-row="${row.id}"]`);
    if (await node.count() === 0) continue; // a child under a collapsed parent
    const box = node.locator('[role=checkbox]');
    if (row.check === 'none') continue;
    const disabled = await box.isDisabled();
    if (disabled !== !row.enabled) failures.push(`${label}: ${row.id} check box disabled=${disabled}, row enabled=${row.enabled}`);
    if (!row.enabled && row.why && !(await box.getAttribute('title'))) failures.push(`${label}: disabled ${row.id} has no title`);
    if (row.muted) {
      const opacity = await node.locator('.nm-text .t').evaluate((n) => Number(getComputedStyle(n).opacity));
      if (!(opacity < 1)) failures.push(`${label}: muted ${row.id} is not dimmed (opacity ${opacity})`);
    }
    if (row.muted && !row.enabled && row.why && view.dialog === null) {
      const shown = await node.locator('.s.blocked').textContent();
      if (shown !== row.why) failures.push(`${label}: blocked ${row.id} does not show its reason on the row`);
      await node.locator('.nm').click();
      await page.waitForTimeout(80);
      const detail = await page.locator('#detail').textContent();
      if (!detail.includes(row.why)) failures.push(`${label}: the detail of blocked ${row.id} does not give the reason`);
      if ((await box.getAttribute('aria-checked')) !== 'false') failures.push(`${label}: blocked ${row.id} is checked`);
      await page.screenshot({ path: `${out}${label}-blocked-${row.id.replace(/[^a-z0-9]+/gi, '_')}.png` });
      shots += 1;
    }
  }
  return failures;
}

/** The selected row is highlighted, marked and described by the detail panel. */
async function checkSelection(page, label) {
  const failures = [];
  const selected = await page.locator('#rows .r.sel').count();
  if (selected !== 1) failures.push(`${label}: ${selected} selected rows, expected 1`);
  if (await page.locator('#detail .kv').count() === 0) failures.push(`${label}: the detail panel is empty`);
  if (await page.locator('#rows .r.sel[aria-current="true"]').count() !== 1) failures.push(`${label}: the selected row has no aria-current`);
  return failures;
}

const KEY_LABELS = { Enter: '↵', Escape: 'Esc' };
const FOOTER = {
  cleanup: '↑↓ move · space toggle · a analyze · ↵ clean',
  registry: '↑↓ move · space toggle · s scan · ↵ fix',
  services: '↑↓ move · space toggle · r refresh',
  developers: '↑↓ move · space toggle · a analyze · ↵ clean',
};
const SECTION_NAMES = [['cleanup', 'Cleanup'], ['registry', 'Registry'], ['services', 'Services & RAM'], ['developers', 'Developers']];
const count = (n) => n.toLocaleString('en-US');
function bytesText(n) {
  if (n >= 2 ** 30) return `${(n / 2 ** 30).toFixed(2)} GB`;
  if (n >= 2 ** 20) return `${(n / 2 ** 20).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The copy on screen is the copy of the view: action labels and keys, badge, section names, footer
 * hints, notices, empty state, progress bar, dialog and report. Sentence case here: CSS makes capitals.
 */
async function checkCopy(page, label, section, view) {
  const found = await page.evaluate(() => {
    const text = (sel) => document.querySelector(sel)?.textContent ?? null;
    const all = (sel) => [...document.querySelectorAll(sel)];
    const dlg = document.querySelector('[role=dialog]');
    const rowsBox = dlg?.querySelector('.dlg-rows');
    const ack = dlg?.querySelector('.ack');
    const report = document.querySelector('#rows .report');
    const track = document.querySelector('#job .track');
    return {
      badge: text('#badge'),
      actions: all('#actions .btn').map((b) => [b.querySelector('[data-fit]')?.textContent ?? '', b.querySelector('kbd')?.textContent ?? null, b.disabled]),
      sections: all('#sections .nv').map((b) => [b.dataset.section, b.querySelector('[data-fit]')?.textContent, b.querySelector('kbd')?.textContent]),
      sectionsHead: text('#sections .hd-title'),
      hint: text('#foot .foot-right'), footLeft: text('#foot .foot-left'), title: text('#list-head .hd-title'),
      notices: all('#notices .notice').map((n) => n.querySelector('.ellipsis')?.textContent),
      empty: document.querySelector('#rows .empty') ? [text('#rows .empty-title'), text('#rows .empty span')] : null,
      job: document.getElementById('job').hidden ? null : {
        label: text('#job .job-label'), now: track?.getAttribute('aria-valuenow'), pct: text('#job .job-pct'),
        width: track?.firstElementChild?.style.width, detail: text('#job .job-detail'),
      },
      dialog: dlg ? {
        modal: dlg.getAttribute('aria-modal'), title: document.getElementById(dlg.getAttribute('aria-labelledby'))?.textContent,
        text: [...dlg.querySelectorAll('.dlg-text')].map((n) => n.textContent),
        error: dlg.querySelector('.dlg-error')?.textContent ?? null,
        summary: [...dlg.querySelectorAll('.dlg-sum')].map((n) => n.textContent),
        rows: [...dlg.querySelectorAll('.dr')].map((r) => [r.children[0].textContent, r.children[1].textContent]),
        rowsMax: rowsBox ? getComputedStyle(rowsBox).maxHeight : null, rowsOverflow: rowsBox ? getComputedStyle(rowsBox).overflowY : null,
        warnTitle: dlg.querySelector('.dlg-warn .dlg-sub')?.textContent ?? null, warnings: [...dlg.querySelectorAll('.dlg-warn li')].map((n) => n.textContent),
        cautionTitle: dlg.querySelector('.dlg-caut .dlg-sub')?.textContent ?? null,
        cautions: [...dlg.querySelectorAll('.dlg-caut li')].map((li) => [li.querySelector('.cl').textContent, li.querySelector('small').textContent]),
        ack: ack ? [ack.getAttribute('aria-checked'), ack.textContent, ack.getAttribute('role')] : null,
        buttons: [...dlg.querySelectorAll('.dlg-buttons .btn')].map((b) => [b.dataset.button, b.querySelector('[data-fit]').textContent, !b.disabled, b.classList.contains('primary')]),
        behind: ['top', 'status', 'body', 'foot'].every((id) => document.getElementById(id).inert),
      } : null,
      report: report ? {
        title: text('#rp-title'), freed: text('.rp-freed'), lines: [...report.querySelectorAll('.rp-sum p')].map((p) => p.textContent),
        subs: [...report.querySelectorAll('.rp-sub')].map((n) => n.textContent),
        skipped: [...report.querySelectorAll('.rp-lines')].map((t) => [...t.querySelectorAll('.rp-row')].map((r) => [r.children[0].textContent, r.children[1].textContent, r.title])),
        cats: [...report.querySelectorAll('.rp-cats .rp-row')].map((r) => [r.children[0].textContent, r.children[1].textContent]),
        items: [...report.querySelectorAll('.rp-items .rp-item')].map((r) => [...r.children].map((c) => c.textContent)),
        buttons: [...report.querySelectorAll('.rp-buttons .btn')].map((b) => b.textContent),
        saved: report.querySelector('.rp-saved')?.textContent ?? null, grid: document.querySelector('#rows .grid') !== null,
        notices: [...report.querySelectorAll('.notice .ellipsis')].map((n) => n.textContent),
      } : null,
    };
  });
  const bad = [];
  const want = (what, got, expected) => { if (!same(got, expected)) bad.push(`${label}: ${what} is ${JSON.stringify(got)}, expected ${JSON.stringify(expected)}`); };
  want('badge', found.badge, view.status.badge);
  want('actions', found.actions, view.actions.map((a) => [a.label, a.key === null ? null : (KEY_LABELS[a.key] ?? a.key), !a.enabled]));
  want('section bar', found.sections, SECTION_NAMES.map(([id, name], i) => [id, name, String(i + 1)]));
  want('section bar heading', found.sectionsHead, 'Sections');
  want('footer hint', found.hint, FOOTER[section]);
  want('footer text', found.footLeft, view.footer ?? '');
  want('list title', found.title, view.title);
  want('notices', found.notices, view.notices.map((n) => n.text));
  const showsEmpty = view.report === null && (view.empty !== null || view.rows.length === 0);
  want('empty state', found.empty, showsEmpty && view.empty ? [view.empty.title, view.empty.text] : null);
  if (view.job === null) want('job bar', found.job, null);
  else {
    const pct = Math.round(view.job.fraction * 100);
    want('job bar', found.job, { label: view.job.label, now: String(pct), pct: `${pct}%`, width: `${pct}%`, detail: view.job.detail });
  }
  if (view.dialog === null) want('dialog', found.dialog, null);
  else {
    const d = view.dialog;
    want('dialog', found.dialog, {
      modal: 'true', title: d.title, text: d.text, error: d.error, summary: d.summary, rows: d.rows,
      rowsMax: d.rows.length ? '180px' : null, rowsOverflow: d.rows.length ? 'auto' : null,
      warnTitle: d.warningsTitle, warnings: d.warnings, cautionTitle: d.cautionTitle, cautions: d.cautions,
      ack: d.ack === null ? null : [String(d.ack), 'I have read the warnings', 'checkbox'],
      buttons: d.buttons.map((b) => [b.id, b.label, b.enabled, b.primary]), behind: true,
    });
  }
  if (view.report === null) want('report', found.report, null);
  else {
    const r = view.report;
    const got = found.report;
    const expected = {
      title: r.title, freed: `Space freed: ${bytesText(r.freedBytes)}`,
      subs: [...(r.byCategory.length ? ['By category'] : []), ...(r.skipped.length ? [`Skipped (${r.skipped.length})`] : []), ...(r.failed.length ? [`Failed (${r.failed.length})`] : []), ...(r.items.length ? [`Items (${r.items.length})`] : [])],
      cats: r.byCategory.map(([n, b]) => [n, bytesText(b)]),
      items: r.items.map((i) => [i.name, bytesText(i.freedBytes), count(i.removed), count(i.left), i.note ?? '']),
      buttons: [...(r.canSave ? ['Save as text…'] : []), 'Close'],
      saved: r.saved === null ? null : r.saved.text, grid: false, notices: r.notes.map((n) => n.text),
    };
    const counts = got.lines.find((l) => l.startsWith('Files deleted: '));
    if (!counts || !counts.startsWith(`Files deleted: ${count(r.files)} · Folders deleted: ${count(r.dirs)} · Duration: `)) bad.push(`${label}: report counts line is ${JSON.stringify(counts)}`);
    if (r.recycledBytes > 0 !== got.lines.some((l) => l.startsWith('Sent to the Recycle Bin: '))) bad.push(`${label}: the recycled line does not follow recycledBytes`);
    if (r.cancelled !== got.lines.some((l) => l.startsWith('Cancelled'))) bad.push(`${label}: the cancelled line does not follow cancelled`);
    const tables = [r.skipped, r.failed].filter((l) => l.length).map((l) => l.map((x) => [x.item, x.why, x.path]));
    want('report skipped/failed tables', got.skipped, tables);
    for (const key of Object.keys(expected)) want(`report ${key}`, got[key], expected[key]);
  }
  return bad;
}

// ---- Keyboard, focus and motion flows (one page, one viewport size).

const focusKey = (page) => page.evaluate(() => document.activeElement?.getAttribute('data-key') ?? null);
const focusIn = (page, selector) => page.evaluate((s) => Boolean(document.activeElement?.closest(s)), selector);
const ringOf = (page) => page.evaluate(() => (document.activeElement ? getComputedStyle(document.activeElement).boxShadow : 'none'));
const badgeOf = (page) => page.locator('#badge').textContent();
const blur = (page) => page.evaluate(() => { if (document.activeElement instanceof HTMLElement) document.activeElement.blur(); });
async function goTo(page, section, state) {
  await resetAll(page); // no dialog or report left over from the last flow
  await blur(page);
  const digit = String(['cleanup', 'registry', 'services', 'developers'].indexOf(section) + 1);
  if (await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).count() === 0) await page.keyboard.press(digit);
  await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).waitFor({ timeout: 2000 });
  await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, state]);
  await page.waitForTimeout(150);
  await blur(page);
}

/** Digits switch sections; Enter opens the confirmation; Enter on Cancel closes it WITHOUT cleaning; Tab is trapped; Esc closes. */
async function keyboardFlows(page, size, expect, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };

  // 1-4 open the sections in the order of the bar.
  await page.evaluate(() => window.__xioxMock.pause(false));
  await blur(page);
  for (const [key, id] of [['2', 'registry'], ['3', 'services'], ['4', 'developers'], ['1', 'cleanup']]) {
    await page.keyboard.press(key);
    await page.locator(`#sections [data-section="${id}"][aria-current="page"]`).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: key ${key} did not open ${id}`));
    check(await page.locator('#sections [aria-current]').count() === 1, `after key ${key} more than one section is current`);
  }

  // The keyboard focus ring is visible on Tab, from the top of the page.
  await goTo(page, 'cleanup', 'analyzed');
  const stops = [];
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.press('Tab');
    const [tag, ring, isBody] = await page.evaluate(() => [document.activeElement?.tagName, document.activeElement ? getComputedStyle(document.activeElement).boxShadow : 'none', document.activeElement === document.body]);
    stops.push(tag);
    check(!isBody, `Tab ${i + 1} left the focus on the page body`);
    check(ring !== 'none', `Tab ${i + 1} (${tag}): the focus ring is not visible (box-shadow ${ring})`);
  }
  await blur(page);

  // Enter opens the confirmation (never a clean); the primary button has the focus and its ring.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the confirmation`));
  check(await badgeOf(page) !== 'Cleaning', 'Enter went straight to a clean');
  check(await focusIn(page, '[role=dialog]'), 'the focus is not inside the dialog after it opened');
  check(await focusKey(page) === 'dlg:continue', `focus on open is ${await focusKey(page)}, expected the enabled primary button`);
  check(await ringOf(page) !== 'none', 'the focused button of the dialog has no visible ring');
  await page.screenshot({ path: `${shot}cleanup-confirm-keyboard-${size}.png` });

  // Enter on Cancel closes without confirming: no job, the analysis is still there.
  await page.locator('[data-key="dlg:cancel"]').focus();
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Enter on Cancel did not close the dialog`));
  await page.waitForTimeout(700); // longer than three ticks of the mock: a confirmed clean would show by now
  check(await badgeOf(page) === 'Analysis done', `after Enter on Cancel the badge is ${await badgeOf(page)}, expected Analysis done (a clean may have started)`);
  check(await page.locator('#job:not([hidden])').count() === 0, 'Enter on Cancel started a job');
  check(await page.locator('#modal[hidden]').count() === 1, 'the modal host is not hidden after the dialog closed');
  check(await page.evaluate(() => !document.getElementById('body').inert), 'the page stays inert after the dialog closed');

  // Tab never leaves the dialog (12 presses, then 4 Shift+Tab), and the ring is visible each time.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => {});
  const seen = new Set();
  for (let i = 0; i < 12; i += 1) {
    await page.keyboard.press('Tab');
    check(await focusIn(page, '[role=dialog]'), `Tab ${i + 1} left the dialog (focus on ${await focusKey(page)})`);
    check(await ringOf(page) !== 'none', `Tab ${i + 1} in the dialog: no visible ring`);
    seen.add(await focusKey(page));
  }
  for (let i = 0; i < 4; i += 1) {
    await page.keyboard.press('Shift+Tab');
    check(await focusIn(page, '[role=dialog]'), `Shift+Tab ${i + 1} left the dialog`);
  }
  check(seen.size >= 2, `Tab visited ${[...seen].join(',')}: it must reach every control of the dialog`);
  // Clicking the dimmed page does nothing: it is inert and the dialog stays.
  await page.mouse.click(4, 4);
  check(await page.locator('[role=dialog]').count() === 1, 'a click on the dimmed page closed the dialog');
  // Escape closes the dialog; the focus goes back inside the list (the opener was the page body).
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the dialog`));
  check(await focusIn(page, '#rows'), `after Escape the focus is on ${await focusKey(page)}, expected inside #rows`);
  check(await badgeOf(page) === 'Analysis done', 'Escape on a dialog changed the section state');

  // The opener gets the focus back when it is still there (the Clean button, opened with the mouse).
  await goTo(page, 'cleanup', 'analyzed');
  await page.locator('#top [data-action="clean"]').click();
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the Clean button did not open the dialog`));
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  check(await focusKey(page) === 'action:clean', `after closing, the focus is on ${await focusKey(page)}, expected the Clean button that opened it`);

  // Developers: the primary button stays disabled until the warnings are read; the ack box has the focus.
  await goTo(page, 'developers', 'projects');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the Developers dialog`));
  check(await focusKey(page) === 'dlg:ack', `Developers dialog focus is ${await focusKey(page)}, expected the ack box (primary disabled)`);
  check(await page.locator('[data-button="clean"]').isDisabled(), 'Clean is enabled before the warnings are read');
  await page.screenshot({ path: `${shot}developers-confirm-keyboard-${size}.png` });
  await page.keyboard.press('Enter'); // the ack box is a button: Enter presses it
  await page.waitForFunction(() => document.querySelector('.ack')?.getAttribute('aria-checked') === 'true', null, { timeout: 2000 }).catch(() => failures.push(`${size}: the ack box did not tick`));
  check(!(await page.locator('[data-button="clean"]').isDisabled()), 'Clean stayed disabled after the warnings were read');
  check(await focusKey(page) === 'dlg:ack', `after ticking, the focus is on ${await focusKey(page)}, expected it to stay on the ack box`);
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  check(await badgeOf(page) === 'Analysis done', 'Escape in the Developers dialog changed the state');

  // Escape cancels a running clean when no dialog is open (it ends in the cancelled report, the analysis dropped), and closes a report (the section is empty again).
  await page.evaluate(() => window.__xioxMock.pause(true));
  await goTo(page, 'cleanup', 'cleaning');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Clean cancelled' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not cancel the running clean`));
  check(await page.locator('#rows .report .rp-cancelled').count() === 1, 'the cancelled clean shows no report');
  check(await page.locator('[data-key="action:analyze"]').isEnabled() && await page.locator('[data-key="action:clean"]').isDisabled(), 'the cancelled report must offer Analyze and not Clean');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the cancelled report`));
  await goTo(page, 'cleanup', 'report');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the report`));
  check(await page.locator('#rows .report').count() === 0, 'the report is still on screen after Escape');
  check(await page.locator('#rows .grid .r[data-row]').count() > 0, 'the list did not come back after the report closed');
  await page.evaluate(() => window.__xioxMock.pause(false));

  // Keys are ignored while typing; Escape leaves the field.
  await goTo(page, 'cleanup', 'analyzed');
  const search = page.locator('#rows input.search');
  await search.focus();
  await page.keyboard.type('a1 ');
  await page.waitForTimeout(450);
  check(await search.inputValue() === 'a1 ', 'typing in the search box lost characters to shortcuts');
  check(await badgeOf(page) === 'Analysis done' && await page.locator('#sections [data-section="cleanup"][aria-current="page"]').count() === 1, 'typing in the search box acted as a shortcut');
  await page.keyboard.press('Escape');
  check(!(await page.evaluate(() => document.activeElement?.classList.contains('search'))), 'Escape did not leave the search box');
  await search.fill('');
  await page.waitForTimeout(300);

  // Arrows move the selection, Space ticks the selected row.
  await goTo(page, 'cleanup', 'analyzed');
  const selected = () => page.evaluate(() => document.querySelector('#rows .r.sel')?.dataset.row ?? null);
  const ids = await page.locator('#rows .grid .r[data-row]:not(.sec)').evaluateAll((nodes) => nodes.map((n) => n.dataset.row));
  await page.keyboard.press('Home');
  await page.waitForTimeout(80);
  check(await selected() === ids[0], `Home selected ${await selected()}, expected the first row ${ids[0]}`);
  await page.keyboard.press('ArrowUp');
  await page.waitForTimeout(80);
  check(await selected() === ids[0], 'ArrowUp on the first row moved the selection');
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(80);
  const second = await selected();
  check(second === ids[1], `ArrowDown selected ${second}, expected ${ids[1]}`);
  const tickBefore = await page.locator(`#rows [data-row="${second}"] [role=checkbox]`).getAttribute('aria-checked');
  await page.keyboard.press('Space');
  await page.waitForFunction(([row, was]) => document.querySelector(`#rows [data-row="${row}"] [role=checkbox]`)?.getAttribute('aria-checked') !== was, [second, tickBefore], { timeout: 2000 })
    .catch(() => failures.push(`${size}: Space did not tick the selected row ${second}`));
  await page.keyboard.press('End');
  await page.waitForTimeout(80);
  check(await selected() === ids[ids.length - 1], `End selected ${await selected()}, expected the last row ${ids[ids.length - 1]}`);
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(80);
  check(await selected() === ids[ids.length - 1], 'ArrowDown on the last row moved the selection');
  // a / Enter press the view's actions: a analyzes (a job shows), the dialog opens only with Enter.
  await page.keyboard.press('a');
  await page.locator('#badge', { hasText: 'Analyzing' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key a did not start the analysis`));
  await page.keyboard.press('Enter');
  check(await page.locator('[role=dialog]').count() === 0, 'Enter opened a dialog while Clean was disabled');
  await page.locator('#badge', { hasText: 'Analysis done' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: the analysis started by a did not land`));

  // Developers: the right and left arrows open and close the selected parent.
  await goTo(page, 'developers', 'projects');
  await page.locator('#rows [data-row="p:1"] .nm').click();
  await page.keyboard.press('ArrowLeft');
  await page.waitForTimeout(80);
  check(await page.locator('#rows [data-row="a:1"]').count() === 0, 'ArrowLeft did not close the selected parent');
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(80);
  check(await page.locator('#rows [data-row="a:1"]').count() === 1, 'ArrowRight did not open the selected parent');

  // Registry and Services: their own keys press their own actions.
  await goTo(page, 'registry', 'idle');
  await page.keyboard.press('s');
  await page.locator('#badge', { hasText: 'Scanning' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key s did not start the scan`));
  await page.locator('#badge', { hasText: 'Scan done' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: the scan did not land`));
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the backup dialog`));
  check(await page.locator('[role=dialog] .dlg-buttons .btn').count() === 3, 'the backup dialog does not have its three buttons');
  await page.screenshot({ path: `${shot}registry-backup-keyboard-${size}.png` });
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  await goTo(page, 'services', 'loaded');
  await page.keyboard.press('r');
  await page.locator('#badge', { hasText: 'Loading' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key r did not refresh`));
  await page.locator('#badge', { hasText: 'Loaded' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: the refresh did not land`));

  return failures;
}

/** The report: Save as text, an item's lines, Close; the progress bar of a running job. */
async function reportFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  await page.evaluate(() => window.__xioxMock.pause(true));
  await goTo(page, 'cleanup', 'report');
  const caret = page.locator('#rows .report .caret').first();
  await caret.click();
  await page.waitForTimeout(100);
  check(await page.locator('#rows .report .rp-open li').count() === 2, `the first item's lines did not open (${await page.locator('#rows .report .rp-open li').count()})`);
  check(await page.locator('#rows .report .caret').first().getAttribute('aria-expanded') === 'true', 'the caret does not say expanded');
  failures.push(...await checkFit(page, `report-expanded-${size}`));
  await page.screenshot({ path: `${shot}cleanup-report-expanded-${size}.png` });
  await page.locator('#rows .report [data-key="report:save"]').click();
  await page.waitForTimeout(150);
  check(await page.locator('#notices .tone-error').count() === 0, 'Save as text… ended in an error');
  check(await page.locator('#rows .report').count() === 1, 'Save as text… closed the report');
  await page.locator('#rows .report [data-key="report:close"]').click();
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Close did not leave the report`));
  check(await page.locator('#rows .report').count() === 0, 'the report is still there after Close');
  // The progress bar of a running job: a fraction in the middle.
  await goTo(page, 'cleanup', 'analyzing');
  await page.evaluate(() => window.__xioxMock.pause(false));
  await page.locator('#job .track[aria-valuenow="33"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the bar never showed a third`));
  await page.evaluate(() => window.__xioxMock.pause(true));
  const bar = await page.evaluate(() => {
    const t = document.querySelector('#job .track');
    return t ? { now: Number(t.getAttribute('aria-valuenow')), width: t.getBoundingClientRect().width, fill: t.firstElementChild.getBoundingClientRect().width, role: t.getAttribute('role') } : null;
  });
  check(bar !== null && bar.role === 'progressbar', 'no progress bar while a job runs');
  if (bar) check(Math.abs(bar.fill / bar.width * 100 - bar.now) < 8, `the bar is filled ${(bar.fill / bar.width * 100).toFixed(0)} % but says ${bar.now} %`);
  await page.screenshot({ path: `${shot}cleanup-analyzing-bar-${size}.png` });
  await page.evaluate(() => window.__xioxMock.pause(false));
  return failures;
}

/** No continuous animation; and with prefers-reduced-motion: reduce nothing animates or transitions at all. */
async function motionFlows(page, size) {
  const failures = [];
  const running = () => page.evaluate(() => document.getAnimations().map((a) => `${a.constructor.name}:${a.animationName ?? a.transitionProperty ?? ''}:${a.effect?.getComputedTiming().iterations}`));
  const quiet = () => page.evaluate(() => [...document.querySelectorAll('*')].filter((n) => {
    const s = getComputedStyle(n);
    return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0);
  }).slice(0, 5).map((n) => `${n.tagName}.${n.className}`));
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.waitForTimeout(300);
  const infinite = (await running()).filter((a) => a.endsWith(':Infinity'));
  if (infinite.length) failures.push(`${size}: continuous animation: ${infinite.join(', ')}`);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.waitForTimeout(200);
  if (!(await page.evaluate(() => matchMedia('(prefers-reduced-motion: reduce)').matches))) failures.push(`${size}: the reduced-motion emulation did not apply`);
  const moving = await running();
  if (moving.length) failures.push(`${size}: with reduced motion something still animates: ${moving.join(', ')}`);
  const styled = await quiet();
  if (styled.length) failures.push(`${size}: with reduced motion these elements still have an animation or transition: ${styled.join(', ')}`);
  await page.emulateMedia({ reducedMotion: null });
  return failures;
}

const server = await createServer({ configFile: `${here}../vite.config.ts`, server: { port: 1432, strictPort: false }, logLevel: 'error' });
await server.listen();
const url = server.resolvedUrls.local[0];
// 'chromium' = the full Chromium build in the cache (the headless shell is not installed here).
const browser = await chromium.launch({ channel: 'chromium' });
const failures = [];
let shots = 0;
const expect = (condition, message) => { if (!condition) failures.push(message); };
try {
  // A failing first command leaves an error in the window, not a blank page.
  {
    const page = await browser.newPage({ viewport: { width: 1000, height: 640 } });
    page.on('pageerror', (error) => failures.push(`failure page: page error ${error}`));
    await page.addInitScript(() => {
      window.__TAURI_INTERNALS__ = { invoke: () => Promise.reject({ kind: 'failed', message: 'the core did not answer' }), transformCallback: () => 0 };
    });
    await page.goto(url);
    await page.locator('#notices .notice.tone-error').waitFor({ timeout: 3000 }).catch(() => failures.push('failure page: no error notice in the window'));
    expect((await page.locator('#badge').textContent()) === 'Error', 'failure page: the badge does not say Error');
    await page.screenshot({ path: `${out}failure-1000x640.png` });
    await page.close();
  }

  for (const [width, height] of VIEWPORTS) {
    const page = await browser.newPage({ viewport: { width, height } });
    const size = `${width}x${height}`;
    page.on('pageerror', (error) => failures.push(`${size}: page error ${error}`));
    page.on('console', (message) => { if (message.type() === 'error') failures.push(`${size}: console error ${message.text()}`); });
    await page.goto(url);
    await page.locator('#sections [data-section="cleanup"].on').waitFor();
    failures.push(...await checkTokens(page));
    expect(await page.locator('#sections [data-section="cleanup"][aria-current="page"]').count() === 1, `${size}: the open section has no aria-current`);
    expect(await page.locator('#sections [aria-current]').count() === 1, `${size}: more than one section is marked current`);

    await page.evaluate(() => window.__xioxMock.pause(true)); // busy states stay put while they are inspected
    for (const [section, stateName] of SECTIONS) {
      const label = `${section}-${stateName}-${size}`;
      await resetAll(page);
      await page.locator(`#sections [data-section="${section}"]`).click();
      await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).waitFor();
      await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, stateName]);
      await page.waitForTimeout(150);
      failures.push(...await checkFit(page, label));
      failures.push(...await checkCopy(page, label, section, fixture.views[section][stateName]));
      await page.screenshot({ path: `${out}${label}.png` });
      shots += 1;
      failures.push(...await checkRows(page, label, fixture.views[section][stateName]));
      if (ANALYZED.has(`${section}:${stateName}`)) {
        failures.push(...await checkSelection(page, label));
        if (stateName === 'projects') {
          expect(await page.locator('#rows [data-row="p:1"] [aria-checked="mixed"]').count() === 1, `${size}: the parent of a half-selected project is not mixed`);
        }
        failures.push(...await checkToggle(page, label));
        await page.waitForTimeout(100);
        failures.push(...await checkFit(page, `${label}-after-toggle`));
      }
    }

    await page.evaluate(() => window.__xioxMock.pause(false));
    await resetAll(page);

    // Cleanup: a category click goes through the model and moves aria-current; a row click moves the selection.
    await page.locator('#sections [data-section="cleanup"]').click();
    await page.evaluate(() => window.__xioxMock.go('cleanup', 'analyzed'));
    await page.locator('#bar2 [data-key="nav:cat:browser"]').click();
    try {
      await page.locator('#bar2 [data-key="nav:cat:browser"][aria-current="true"]').waitFor({ timeout: 2000 });
      expect(await page.locator('#bar2 .it[aria-current]').count() === 1, `${size}: more than one category is marked current`);
    } catch { failures.push(`${size}: the clicked category did not become current`); }
    await page.locator('#rows [data-row="cat:3"] .nm').click();
    await page.locator('#rows [data-row="cat:3"].sel').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the clicked row is not selected`));
    expect((await page.locator('#detail .kv b').first().textContent()) === 'Windows log files', `${size}: the detail does not describe the clicked row`);
    await page.screenshot({ path: `${out}cleanup-row-selected-${size}.png` });
    shots += 1;

    // The check box keeps the keyboard focus through the round trip (the view is redrawn).
    const box = page.locator('#rows [data-row="cat:4"] [role=checkbox]');
    await box.focus();
    await page.keyboard.press('Space');
    await page.waitForFunction(() => document.querySelector('#rows [data-row="cat:4"] [role=checkbox]')?.getAttribute('aria-checked') === 'true', null, { timeout: 2000 })
      .catch(() => failures.push(`${size}: Space on a focused check box did not toggle it`));
    expect(await page.evaluate(() => document.activeElement?.getAttribute('data-key')) === 'ck:cat:4', `${size}: the check box lost the keyboard focus after the redraw`);

    // The search box keeps its text and focus while views come back.
    const search = page.locator('#rows input.search');
    await search.fill('temp');
    await page.waitForTimeout(450);
    expect(await search.inputValue() === 'temp' && await page.evaluate(() => document.activeElement?.classList.contains('search')),
      `${size}: the search box lost its text or focus`);

    // Analyze: the busy state shows, then the list lands on analyzed after the model's ticks.
    await page.evaluate(() => window.__xioxMock.go('cleanup', 'idle'));
    await page.locator('#top [data-action="analyze"]').click();
    await page.locator('#badge', { hasText: 'Analyzing' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Analyze did not show the busy state`));
    await page.locator('#badge', { hasText: 'Analysis done' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: Analyze did not land on analyzed`));

    // Developers: a collapsed parent hides its children until the caret opens it.
    await page.locator('#sections [data-section="developers"]').click();
    await page.evaluate(() => window.__xioxMock.go('developers', 'projects'));
    await page.locator('#rows [data-row="p:1"]').waitFor();
    expect(await page.locator('#rows [data-row="a:1"]').count() === 0, `${size}: a child shows under a collapsed parent`);
    await page.locator('#rows [data-row="p:1"] .caret').click();
    await page.locator('#rows [data-row="a:1"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the caret did not show the children`));
    failures.push(...await checkFit(page, `developers-expanded-${size}`));
    await page.screenshot({ path: `${out}developers-projects-expanded-${size}.png` });
    shots += 1;
    // A section's category changes the Developers state.
    await page.locator('#bar2 [data-key="nav:worktrees"]').click();
    await page.locator('#bar2 [data-key="nav:worktrees"][aria-current="true"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Worktrees did not become the current category`));

    // Keyboard, dialogs, report, progress bar and motion (Task 3).
    failures.push(...await keyboardFlows(page, size, expect, out));
    failures.push(...await reportFlows(page, size, out));
    shots += 5;
    failures.push(...await motionFlows(page, size));
    await page.close();
  }
} finally {
  await browser.close();
  await server.close();
}
if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}
console.log(`smoke ok: ${shots} screenshots`);
