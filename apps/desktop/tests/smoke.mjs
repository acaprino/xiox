// Smoke test: the page on Vite with the mocked commands (src/mock.ts), screenshots under
// tests/out/ (git-ignored), token / fit / error checks and the mock round trips (a click goes to the
// model and comes back as a view). Playwright is a pinned devDependency (1.63.0); its Chromium is
// the one already in %LOCALAPPDATA%\ms-playwright.
//   npm run smoke
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { mkdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { findItalian } from '../../../scripts/italian-check.mjs';

const here = fileURLToPath(new URL('.', import.meta.url));
const out = `${here}out/`;
mkdirSync(out, { recursive: true });

// Every state of the fixture is drawn and fit-checked; the ones below also get the click checks.
const fixture = JSON.parse(readFileSync(`${here}../fixtures/views.json`, 'utf8'));
export const SECTIONS = Object.entries(fixture.views).flatMap(([section, states]) => Object.keys(states).map((state) => [section, state]));
const ANALYZED = new Set(['cleanup:analyzed', 'cleanup:interrupted', 'registry:analyzed', 'developers:projects', 'developers:worktrees', 'developers:sessions']);
const VIEWPORTS = [[1000, 640], [1440, 900]];
// The narrow sizes of the breakpoints are checked on their own (narrowChecks).

/** What the root says per look (Imago's `applyLook`): the app, the look and the theme drawn, whatever Windows says. */
const LOOKS = {
  smooth: { look: 'smooth', theme: 'dark' },
  'smooth-light': { look: 'smooth', theme: 'light' },
  oni: { look: 'oni', theme: 'dark' },
};

export async function checkTokens(page, skin = 'smooth') {
  const found = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    const token = (name) => root.getPropertyValue(name).trim().toLowerCase();
    const attr = (name) => document.documentElement.getAttribute(name);
    return {
      app: attr('data-im-app'), look: attr('data-im-look'), theme: attr('data-im-theme'), scheme: document.documentElement.style.colorScheme,
      ground: token('--im-ground'), text: token('--im-text'), accent: token('--im-accent'), hue: token('--im-hue'),
      oldSkin: document.documentElement.hasAttribute('data-skin'),
    };
  });
  const want = LOOKS[skin];
  const bad = [];
  if (found.app !== 'xiox') bad.push(`tokens (${skin}): data-im-app is ${found.app}, expected xiox`);
  if (found.look !== want.look) bad.push(`tokens (${skin}): data-im-look is ${found.look}, expected ${want.look}`);
  if (found.theme !== want.theme) bad.push(`tokens (${skin}): data-im-theme is ${found.theme}, expected ${want.theme}`);
  if (found.scheme !== want.theme) bad.push(`tokens (${skin}): color-scheme is ${found.scheme}, expected ${want.theme}`);
  if (found.hue !== '75') bad.push(`tokens (${skin}): --im-hue is ${found.hue}, expected 75 (Xiox's amber)`);
  if (!found.accent.includes(' 75)') && !found.accent.startsWith('#')) bad.push(`tokens (${skin}): --im-accent is ${found.accent}, not of hue 75`);
  if (found.ground === '' || found.text === '') bad.push(`tokens (${skin}): --im-ground or --im-text is empty (Imago's stylesheet is not loaded)`);
  if (found.oldSkin) bad.push(`tokens (${skin}): the root still carries data-skin`);
  return bad;
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
      if (node.scrollWidth > node.clientWidth + 1 && !(node.closest('[title]')?.getAttribute('title') || '').trim()) {
        bad.push(`${where}: truncated without a title ${name(node)}`);
      }
    }
    if (document.documentElement.scrollWidth > innerWidth) bad.push(`${where}: page scrolls sideways (${document.documentElement.scrollWidth} > ${innerWidth})`);
    for (const sel of ['#rows .xi-list', '#sidebar-list', '.im-inspector:not([hidden]) .im-panel-body']) {
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
    // The title bar: the brand, the tabs (or the Views menu), the command button and the window buttons are all inside it, and none on another.
    const bar = document.getElementById('top');
    if (bar) {
      const box = bar.getBoundingClientRect();
      const parts = [...bar.querySelectorAll('.im-brand, .im-tabs, .im-views-menu, .im-titlebar-command, .im-window-buttons')].filter((node) => node.getBoundingClientRect().width > 0);
      for (const node of parts) {
        const r = node.getBoundingClientRect();
        if (r.left < box.left - 0.5 || r.right > box.right + 0.5 || r.top < box.top - 0.5 || r.bottom > box.bottom + 0.5) bad.push(`${where}: ${name(node)} sticks out of the title bar`);
      }
      if (bar.scrollWidth > bar.clientWidth + 1) bad.push(`${where}: the title bar scrolls sideways`);
      overlaps(parts);
    }
    // The action bar: its buttons do not collide (the hidden sentences that say why one is disabled are not drawn).
    overlaps([...document.querySelectorAll('#actions > .im-btn')]);
    // A running job: the bar is wide enough to read and its pieces do not collide.
    const job = document.getElementById('job');
    if (job && !job.hidden) {
      const track = job.querySelector('.im-progress');
      if (!track || track.getBoundingClientRect().width < 100) bad.push(`${where}: the progress bar is narrower than 100 px`);
      if (job.scrollWidth > job.clientWidth + 1) bad.push(`${where}: the job row overflows`);
      overlaps([...job.children]);
    }
    // The confirmation: inside the window, its parts stacked, nothing sideways.
    const dlg = document.querySelector('dialog.dlg');
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
      for (const row of report.querySelectorAll('.rp-row')) overlaps([...row.children].filter((n) => !n.classList.contains('xi-caret-gap')));
    }
    // Inside a row: name, cells, level and size each keep their own column.
    for (const row of document.querySelectorAll('#rows tbody tr[data-row]:not(.xi-section)')) {
      overlaps([...row.querySelectorAll(':scope > td')]);
    }
    // The inspector is docked from 1000 up: it pushes the list aside and never runs over it (below 1000 it is a drawer, over the content by design).
    const grid = document.querySelector('#rows .xi-list');
    const inspector = document.querySelector('.im-inspector:not([hidden])');
    if (grid && inspector && innerWidth >= 1000 && grid.getBoundingClientRect().right > inspector.getBoundingClientRect().left + 1) {
      bad.push(`${where}: the list runs under the inspector`);
    }
    return bad;
  }, label);
}

/**
 * Under 620 the tabs are one Views menu: it lists the sections and Settings, marks the open one, and a choice closes it and opens that view
 * (the page's own `open`, as the tab does).
 */
async function viewsMenuChecks(page, width) {
  const failures = [];
  const menu = page.locator('[role=menu]');
  const labels = () => page.locator('[role=menu] [role=menuitemradio] > span:first-child').allInnerTexts();
  await page.locator('.im-views-menu').click();
  await menu.waitFor({ timeout: 2000 }).catch(() => failures.push(`${width}: the Views button did not open its menu`));
  const items = await labels();
  if (items.join() !== 'Cleanup,Registry,Developers,Settings') failures.push(`${width}: the Views menu lists ${items.join()}, expected the three sections and Settings`);
  const checked = await page.locator('[role=menu] [role=menuitemradio][aria-checked="true"]').allInnerTexts();
  if (checked.length !== 1 || !checked[0].startsWith('Cleanup')) failures.push(`${width}: the Views menu does not mark Cleanup as the open view (${JSON.stringify(checked)})`);
  await page.locator('[role=menu] [role=menuitemradio]', { hasText: 'Registry' }).click();
  await menu.waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${width}: a choice in the Views menu did not close it`));
  await page.locator('#sections [data-section="registry"][aria-current="page"]').waitFor({ state: 'attached', timeout: 2000 }).catch(() => failures.push(`${width}: the Views menu did not open Registry`));
  await page.locator('.im-views-menu').click();
  await page.locator('[role=menu] [role=menuitemradio]', { hasText: 'Settings' }).click();
  await page.locator('#rows .xi-settings').waitFor({ timeout: 2000 }).catch(() => failures.push(`${width}: the Views menu did not open Settings`));
  return failures;
}

/**
 * The window at its narrow sizes, the three breakpoints of the shell (1000, 850 and 620) and the minimum of 500 and under it
 * (200 % zoom on a small screen): nothing scrolls sideways, and the window buttons, the sidebar toggle, the Views menu and the
 * primary action are all inside the shell.
 */
async function narrowChecks(browser, url, out) {
  const failures = [];
  for (const width of [1160, 900, 700, 500, 250]) {
    const page = await browser.newPage({ viewport: { width, height: 700 } });
    await page.goto(url);
    // Under 620 the tabs are not drawn (the Views menu is): the page is there when they are in the DOM.
    await page.locator('#sections [data-section="cleanup"][aria-selected="true"]').waitFor({ state: 'attached' });
    await page.evaluate(() => window.__xioxMock.go('cleanup', 'analyzed'));
    await page.waitForTimeout(250);
    const found = await page.evaluate(() => {
      const shell = document.getElementById('app').getBoundingClientRect();
      const inside = (selector) => {
        const node = document.querySelector(selector);
        if (!node) return `${selector} is missing`;
        const r = node.getBoundingClientRect();
        const shown = r.width > 0 && getComputedStyle(node).visibility !== 'hidden';
        if (!shown) return null;
        return r.left >= shell.left - 0.5 && r.right <= shell.right + 0.5 && r.top >= shell.top - 0.5 && r.bottom <= shell.bottom + 0.5 ? null : `${selector} sticks out of the shell`;
      };
      const bad = ['[data-im-window="minimize"]', '[data-im-window="maximize"]', '[data-im-window="close"]', '.im-sidebar-toggle', '.im-views-menu', '#actions .im-btn--primary'].map(inside).filter(Boolean);
      if (document.documentElement.scrollWidth > innerWidth) bad.push(`the page scrolls sideways (${document.documentElement.scrollWidth} > ${innerWidth})`);
      const content = document.getElementById('main');
      if (content && content.scrollWidth > content.clientWidth + 1) bad.push('the content scrolls sideways');
      return bad;
    });
    for (const line of found) failures.push(`${width}: ${line}`);
    if (width === 500) failures.push(...await viewsMenuChecks(page, width));
    await page.screenshot({ path: `${out}narrow-${width}.png` });
    await page.close();
  }
  return failures;
}

/** Every section back on a state without a dialog or a report (a dimmed page cannot be clicked). */
const resetAll = (page) => page.evaluate(() => {
  for (const [s, st] of [['developers', 'idle'], ['registry', 'idle'], ['cleanup', 'idle']]) window.__xioxMock.go(s, st);
});

/** The list takes the focus when there is one (the keys of the list are its own: arrows, Space, Enter), else nothing keeps it. */
const focusTable = (page) => page.evaluate(() => {
  const table = document.querySelector('#rows .xi-table');
  if (table instanceof HTMLElement) table.focus();
  else if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
});
/** Chromium starts Tab from where the last focus was: the starting point goes back to the top of the page. */
const fromTop = (page) => page.evaluate(() => { document.body.tabIndex = -1; document.body.focus(); document.body.removeAttribute('tabindex'); });

/**
 * The inspector open on `kind` ('details' or 'options'): the content header's button opens it, unless it is open on that already.
 * True when it is there; the callers say so when it is not.
 */
async function openInspector(page, kind) {
  const title = kind === 'details' ? 'Details' : 'Options';
  const shown = page.locator('.im-inspector:not([hidden]) .im-panel-title', { hasText: title });
  try {
    if (await shown.count() === 0) await page.locator(`#list-head [data-key="inspect:${kind}"]`).click({ timeout: 2000 });
    await shown.first().waitFor({ timeout: 2000 });
    return true;
  } catch {
    return false;
  }
}
/** The inspector closed, by its own button (what the user does with the mouse). */
async function closeInspector(page) {
  if (await page.locator('.im-inspector:not([hidden])').count() === 0) return;
  await page.locator('.im-inspector [data-key="inspector:close"]').click({ timeout: 2000 }).catch(() => undefined);
  await page.waitForFunction(() => document.querySelector('.im-inspector')?.hidden === true, null, { timeout: 2000 }).catch(() => undefined);
}

/** One click on the first enabled check box must come back flipped from the mock model. */
async function checkToggle(page, label) {
  const box = page.locator('#rows tbody tr[data-row] [role=checkbox]:not([aria-disabled="true"])').first();
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
  const after = await page.locator(`#rows [data-row="${esc(id)}"] [role=checkbox]`).getAttribute('aria-checked');
  return after === want ? [] : [`${label}: ${id} went ${before} -> ${after}, expected ${want}`];
}

/**
 * Every row the fixture state has: its check box is disabled exactly when the row is, a disabled one says
 * why in its title, and a blocked row (disabled and muted) is listed, dimmed, with its reason on the row
 * and, once selected, in the inspector's details.
 */
async function checkRows(page, label, view) {
  const failures = [];
  for (const row of view.rows) {
    if (row.kind === 'header') continue;
    const node = page.locator(`#rows [data-row="${esc(row.id)}"]`);
    if (await node.count() === 0) continue; // a child under a collapsed parent
    const box = node.locator('[role=checkbox]');
    if (row.check === 'none') continue;
    const disabled = await box.isDisabled();
    const frozen = view.job !== null; // Rust never greys a row for a job: the page does not offer the tick
    if (disabled !== (!row.enabled || frozen)) failures.push(`${label}: ${row.id} check box disabled=${disabled}, row enabled=${row.enabled}, job=${frozen}`);
    if (!row.enabled && row.why && !(await box.getAttribute('title'))) failures.push(`${label}: disabled ${row.id} has no title`);
    if (row.muted && !row.enabled && (await node.getAttribute('aria-disabled')) !== 'true') failures.push(`${label}: blocked ${row.id} is not aria-disabled`);
    if (row.muted && !row.enabled && row.why && view.dialog === null) {
      const shown = await node.locator('[data-blocked]').textContent();
      if (shown !== row.why) failures.push(`${label}: blocked ${row.id} does not show its reason on the row`);
      await node.locator('td.xi-name').click({ force: true });
      await page.waitForTimeout(80);
      if (!await openInspector(page, 'details')) failures.push(`${label}: the Details button did not open the inspector`);
      const detail = await page.locator('.im-inspector').textContent();
      if (!detail.includes(row.why)) failures.push(`${label}: the details of blocked ${row.id} do not give the reason`);
      if ((await box.getAttribute('aria-checked')) !== 'false') failures.push(`${label}: blocked ${row.id} is checked`);
      await page.screenshot({ path: `${out}${label}-blocked-${row.id.replace(/[^a-z0-9]+/gi, '_')}.png` });
      shots += 1;
      await closeInspector(page);
    }
  }
  return failures;
}

/**
 * The selected row is highlighted and marked; the inspector's details describe it. A category or a sub-heading has nothing to add to its
 * own line, so the check clicks the first row that is an item (the selection starts on the first row of the list).
 */
async function checkSelection(page, label) {
  const failures = [];
  const selected = await page.locator('#rows tr[aria-selected="true"]').count();
  if (selected !== 1) failures.push(`${label}: ${selected} selected rows, expected 1`);
  const item = page.locator('#rows tbody tr[data-row]:not(.xi-group):not(.xi-heading):not(.xi-section) td.xi-name').first();
  if (await item.count() === 0) failures.push(`${label}: no item row to select`);
  else {
    await item.click();
    await page.waitForTimeout(80);
    if (await page.locator('#rows tr[aria-selected="true"]').count() !== 1) failures.push(`${label}: the clicked row is not the one selected row`);
    if (!await openInspector(page, 'details')) failures.push(`${label}: the Details button did not open the inspector`);
    else if (await page.locator('.im-inspector .im-facts').count() === 0) failures.push(`${label}: the details of the selected row are empty`);
    await closeInspector(page);
  }
  return failures;
}

const KEY_LABELS = { Enter: '↵', Escape: 'Esc' };
/** The key cap of a binding as Imago draws it: Enter is the return symbol, a letter is a capital. */
const keyCap = (key) => KEY_LABELS[key] ?? (key.length === 1 ? key.toUpperCase() : key);
const FOOTER = {
  cleanup: '↑↓ move · space toggle · s sort · a analyze · ↵ clean',
  registry: '↑↓ move · space toggle · s sort · a find issues · ↵ fix',
  developers: '↑↓ move · space toggle · s sort · a analyze · ↵ clean',
};
const SECTION_NAMES = [['cleanup', 'Cleanup'], ['registry', 'Registry'], ['developers', 'Developers']];
/** The tabs of the title bar: the three sections, then Settings. */
const TAB_NAMES = [...SECTION_NAMES, ['settings', 'Settings']];
const count = (n) => n.toLocaleString('en-US');
function bytesText(n) {
  if (n >= 2 ** 30) return `${(n / 2 ** 30).toFixed(2)} GB`;
  if (n >= 2 ** 20) return `${(n / 2 ** 20).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}
/** A value inside a quoted attribute selector: the backslash of a Windows path and the quote are escaped. */
const esc = (value) => value.replace(/[\\"]/g, (c) => '\\' + c);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The copy on screen is the copy of the view: action labels and keys, badge, section names, footer
 * hints, notices, empty state, progress bar, dialog and report. Sentence case here: CSS makes capitals.
 */
async function checkCopy(page, label, section, view) {
  if (view.report === null && (view.empty !== null || view.rows.length === 0) && view.job !== null) {
    await page.locator('#rows .im-skeleton').first().waitFor({ timeout: 2000 }).catch(() => undefined);
  }
  const found = await page.evaluate(() => {
    const text = (sel) => document.querySelector(sel)?.textContent ?? null;
    const all = (sel) => [...document.querySelectorAll(sel)];
    const dlg = document.querySelector('[role=dialog]');
    const rowsBox = dlg?.querySelector('.dlg-rows');
    const ack = dlg?.querySelector('.ack');
    const report = document.querySelector('#rows .report');
    const track = document.querySelector('#job .im-progress');
    return {
      badge: text('#badge'),
      actions: all('#actions .im-btn').map((b) => [b.querySelector('[data-fit]')?.textContent ?? '', b.querySelector('.im-key')?.textContent ?? null, b.disabled]),
      sections: all('#sections .im-tab[data-section]').map((b) => [b.dataset.section, b.textContent]),
      hint: all('#foot .im-footer-keys > span').map((n) => n.textContent), footLeft: text('#foot .foot-left'), title: text('#list-head .im-panel-title'),
      notices: all('#notices .im-note').map((n) => n.querySelector('.ellipsis, b')?.textContent),
      empty: document.querySelector('#rows .im-empty') ? [text('#rows .im-empty > strong'), text('#rows .im-empty > span')] : null,
      loading: document.querySelector('#rows .im-skeleton') !== null,
      job: document.getElementById('job').hidden ? null : {
        label: text('#job .xi-job-label'), now: track?.getAttribute('aria-valuenow'), pct: text('#job .im-progress-value'),
        width: track?.style.getPropertyValue('--im-progress'), detail: text('#job .xi-job-detail'),
      },
      dialog: dlg ? {
        modal: dlg.getAttribute('aria-modal'), title: document.getElementById(dlg.getAttribute('aria-labelledby'))?.textContent,
        text: [...dlg.querySelectorAll('.dlg-text')].map((n) => n.textContent),
        error: dlg.querySelector('.dlg-error')?.textContent ?? null,
        summary: [...dlg.querySelectorAll('.dlg-sum')].map((n) => n.textContent),
        rows: [...dlg.querySelectorAll('.dr')].map((r) => [r.children[0].textContent, r.children[1].textContent]),
        rowsMax: rowsBox ? getComputedStyle(rowsBox).maxHeight : null, rowsOverflow: rowsBox ? getComputedStyle(rowsBox).overflowY : null,
        warnTitle: dlg.querySelector('.dlg-warn b')?.textContent ?? null, warnings: [...dlg.querySelectorAll('.dlg-warn li')].map((n) => n.textContent),
        cautionTitle: dlg.querySelector('.dlg-caut > b')?.textContent ?? null,
        cautions: [...dlg.querySelectorAll('.dlg-caut li')].map((li) => [li.querySelector('.cl').textContent, li.querySelector('small').textContent]),
        ack: ack ? [String(ack.querySelector('input').checked), ack.textContent, ack.querySelector('input').type] : null,
        buttons: [...dlg.querySelectorAll('.dlg-buttons .im-btn')].map((b) => [b.dataset.button, b.querySelector('[data-fit]').textContent, !b.disabled, b.classList.contains('im-btn--primary') || b.classList.contains('im-btn--confirm-danger')]),
        behind: ['sidebar', 'status', 'body', 'actions', 'foot'].every((id) => document.getElementById(id).closest('[inert]') !== null) && document.getElementById('top').closest('[inert]') === null,
      } : null,
      report: report ? {
        title: text('#rp-title'), freed: text('.rp-freed'), lines: [...report.querySelectorAll('.rp-sum p')].map((p) => p.textContent),
        subs: [...report.querySelectorAll('.rp-sub')].map((n) => n.textContent),
        skipped: [...report.querySelectorAll('.rp-lines')].map((t) => [...t.querySelectorAll('.rp-row')].map((r) => [r.children[0].textContent, r.children[1].textContent, r.querySelector('.rp-path')?.textContent ?? ''])),
        cats: [...report.querySelectorAll('.rp-cats .rp-row')].map((r) => [r.children[0].textContent, r.children[1].textContent]),
        items: [...report.querySelectorAll('.rp-items .rp-item')].map((r) => [...r.children].map((c) => c.textContent)),
        buttons: [...report.querySelectorAll('.rp-buttons .im-btn')].map((b) => b.textContent),
        saved: report.querySelector('.rp-saved')?.textContent ?? null, grid: document.querySelector('#rows .xi-table') !== null,
        notices: [...report.querySelectorAll('.im-note .ellipsis, .im-note b')].map((n) => n.textContent),
      } : null,
    };
  });
  const bad = [];
  const want = (what, got, expected) => { if (!same(got, expected)) bad.push(`${label}: ${what} is ${JSON.stringify(got)}, expected ${JSON.stringify(expected)}`); };
  want('badge', found.badge, view.status.badge);
  want('actions', found.actions, view.actions.map((a) => [a.label, a.key === null ? null : keyCap(a.key), !a.enabled]));
  want('section tabs', found.sections, TAB_NAMES.map(([id, name]) => [id, name]));
  want('footer hint', found.hint, ['↑↓Move', 'SpaceToggle', 'SOrder by the next column', ...view.actions.filter((a) => a.key !== null && !['Esc', 'Escape'].includes(a.key)).map((a) => (a.key === 'Enter' ? '↵' : a.key.toUpperCase()) + a.label)]);
  want('footer text', found.footLeft, view.footer ?? '');
  want('list title', found.title, view.title);
  want('notices', found.notices, view.notices.map((n) => n.text));
  // The panel's states (setPanelState, render/shell.ts): a list with nothing in it is the empty state (its title and its words, or "Nothing to
  // show." when Rust sent none); with a job running it is the loading state instead (a skeleton, which shows after 300 ms), and no words.
  const showsEmpty = view.report === null && (view.empty !== null || view.rows.length === 0);
  const loading = showsEmpty && view.job !== null;
  want('empty state', found.empty, showsEmpty && !loading ? [view.empty?.title || null, view.empty?.text ?? 'Nothing to show.'] : null);
  want('loading state', found.loading, loading);
  if (view.job === null) want('job bar', found.job, null);
  else {
    const pct = Math.round(view.job.fraction * 100);
    want('job bar', found.job, { label: view.job.label, now: String(pct), pct: `${pct}%`, width: String(pct), detail: view.job.detail });
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
const ringOf = (page) => page.evaluate(() => (document.activeElement && getComputedStyle(document.activeElement).outlineStyle !== 'none' && parseFloat(getComputedStyle(document.activeElement).outlineWidth) > 0 ? 'outline' : 'none'));
const badgeOf = (page) => page.locator('#badge').textContent();
const blur = (page) => page.evaluate(() => { if (document.activeElement instanceof HTMLElement) document.activeElement.blur(); });
async function goTo(page, section, state) {
  await resetAll(page); // no dialog or report left over from the last flow
  await blur(page);
  const digit = String(['cleanup', 'registry', 'developers'].indexOf(section) + 1);
  if (await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).count() === 0) await page.keyboard.press(digit);
  await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).waitFor({ timeout: 2000 });
  await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, state]);
  await page.waitForTimeout(150);
  await focusTable(page);
}

/**
 * Digits switch sections; Enter opens the confirmation; Enter on Cancel closes it WITHOUT cleaning; Tab is trapped; Esc closes. The list's own
 * keys (arrows, Space, Enter) are the list's: `goTo` leaves the focus on it, as the window does at its start.
 */
async function keyboardFlows(page, size, expect, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };

  // 1-3 open the sections in the order of the bar.
  await page.evaluate(() => window.__xioxMock.pause(false));
  await blur(page);
  for (const [key, id] of [['2', 'registry'], ['3', 'developers'], ['1', 'cleanup']]) {
    await page.keyboard.press(key);
    await page.locator(`#sections [data-section="${id}"][aria-current="page"]`).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: key ${key} did not open ${id}`));
    check(await page.locator('#sections [aria-current]').count() === 1, `after key ${key} more than one section is current`);
  }

  // The keyboard focus ring is visible on Tab, from the top of the page.
  await goTo(page, 'cleanup', 'analyzed');
  // Chromium starts Tab from where the last focus was, not from the top: put the starting point on the body.
  await page.evaluate(() => { document.body.tabIndex = -1; document.body.focus(); document.body.removeAttribute('tabindex'); });
  const stops = [];
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.press('Tab');
    const [tag, ring, isBody] = await page.evaluate(() => [document.activeElement?.tagName, document.activeElement ? (getComputedStyle(document.activeElement).outlineStyle !== 'none' ? 'outline' : 'none') : 'none', document.activeElement === document.body]);
    stops.push(tag);
    check(!isBody, `Tab ${i + 1} left the focus on the page body`);
    check(ring !== 'none', `Tab ${i + 1} (${tag}): the focus ring is not visible (box-shadow ${ring})`);
  }
  await blur(page);

  // Enter opens the confirmation (never a clean); Cancel has the focus and its ring.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the confirmation`));
  check(await badgeOf(page) !== 'Cleaning', 'Enter went straight to a clean');
  check(await focusIn(page, '[role=dialog]'), 'the focus is not inside the dialog after it opened');
  // A permanent delete starts on Cancel (a second Enter never approves it); Shift+Tab reaches the primary button.
  check(await focusKey(page) === 'dlg:cancel', `focus on open is ${await focusKey(page)}, expected Cancel for a permanent delete`);
  await page.keyboard.press('Shift+Tab');
  check(await focusKey(page) === 'dlg:continue', `Shift+Tab from Cancel is ${await focusKey(page)}, expected the primary button`);
  check(await ringOf(page) !== 'none', 'the focused button of the dialog has no visible ring');
  await page.screenshot({ path: `${shot}cleanup-confirm-keyboard-${size}.png` });

  // Enter on Cancel closes without confirming: no job, the analysis is still there. (Past the 300 ms in which a dialog's buttons ignore Enter.)
  await page.waitForTimeout(350);
  await page.locator('[data-key="dlg:cancel"]').focus();
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Enter on Cancel did not close the dialog`));
  await page.waitForTimeout(700); // longer than three ticks of the mock: a confirmed clean would show by now
  check(await badgeOf(page) === 'Analysis done', `after Enter on Cancel the badge is ${await badgeOf(page)}, expected Analysis done (a clean may have started)`);
  check(await page.locator('#job:not([hidden])').count() === 0, 'Enter on Cancel started a job');
  check(await page.locator('dialog.dlg').count() === 0, 'the dialog is still in the page after it closed');
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
  // The title bar stays free (it moves and closes the window): a click on it does not close the dialog.
  await page.mouse.click(4, 4);
  check(await page.locator('[role=dialog]').count() === 1, 'a click on the title bar closed the dialog');
  // Escape closes the dialog; the focus goes back inside the list (the opener was the page body).
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the dialog`));
  check(await focusIn(page, '#rows'), `after Escape the focus is on ${await focusKey(page)}, expected inside #rows`);
  check(await badgeOf(page) === 'Analysis done', 'Escape on a dialog changed the section state');

  // A press on the scrim is the dialog's own Cancel: it closes it, the inert page behind never gets the press, and nothing is confirmed.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the confirmation (scrim test)`));
  await clearCalls(page);
  const view = page.viewportSize();
  await page.mouse.click(view.width - 6, view.height - 6);
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: a press on the scrim did not close the dialog`));
  check(!(await sent(page, 'cleanup', 'confirm')) && await badgeOf(page) === 'Analysis done', 'a press on the scrim confirmed something or changed the section state');

  // The opener gets the focus back when it is still there (the Clean button, opened with the mouse).
  await goTo(page, 'cleanup', 'analyzed');
  await page.locator('#actions [data-action="clean"]').click();
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the Clean button did not open the dialog`));
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  check(await focusKey(page) === 'action:clean', `after closing, the focus is on ${await focusKey(page)}, expected the Clean button that opened it`);

  // Enter guard: the Enter that opens a confirmation, pressed again at once, held down, or repeated, never confirms it.
  await goTo(page, 'cleanup', 'analyzed');
  await clearCalls(page);
  await page.keyboard.press('Enter');
  await page.keyboard.press('Enter'); // at once: the dialog may not even be drawn yet, or is under 300 ms old
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the confirmation (guard test)`));
  await page.waitForTimeout(40);
  check(await page.locator('[role=dialog]').count() === 1, 'a second Enter right after the first confirmed or closed the dialog');
  check(await badgeOf(page) !== 'Cleaning', 'a second Enter right after the first started a clean');
  await page.keyboard.press('Enter'); // still under 300 ms since the dialog opened: no
  await page.waitForTimeout(60);
  check(await page.locator('[role=dialog]').count() === 1 && await badgeOf(page) !== 'Cleaning', 'Enter under 300 ms after the dialog opened activated its primary button');
  await page.waitForTimeout(400);
  await page.locator('[data-key="dlg:continue"]').focus(); // the dialog opened on Cancel: the primary button is one Shift+Tab away
  await page.keyboard.down('Enter'); // a fresh press: the dialog is armed
  await page.keyboard.down('Enter'); // the second keydown of a held key carries repeat: true
  await page.keyboard.up('Enter');
  await page.waitForTimeout(150);
  // The first of the two downs is a real press and confirms; what matters is that a held key does not press twice.
  check(await page.locator('dialog.dlg').count() === 0, 'Enter on the armed primary button did not close the dialog');
  check(await sent(page, 'cleanup', 'confirm') && (await callsOf(page)).filter(([c, k]) => c === 'cleanup' && k === 'confirm').length === 1, 'the held Enter sent confirm more than once, or not at all');
  await clearCalls(page);
  // A held Enter alone, from a closed dialog: the first press opens it, the repeats do nothing more.
  await goTo(page, 'cleanup', 'analyzed');
  await clearCalls(page);
  await page.keyboard.down('Enter');
  for (let i = 0; i < 6; i += 1) await page.keyboard.down('Enter');
  await page.keyboard.up('Enter');
  await page.waitForTimeout(500);
  check(await page.locator('[role=dialog]').count() === 1, 'a held Enter did not leave the confirmation open');
  check(!(await sent(page, 'cleanup', 'confirm')), 'a held Enter confirmed the clean');
  check((await callsOf(page)).filter(([c, k]) => c === 'cleanup' && k === 'openConfirm').length === 1, 'a held Enter opened the confirmation more than once');
  await page.keyboard.press('Escape'); // Escape always works
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the dialog after a held Enter`));

  // Developers: the primary button stays disabled until the warnings are read. Every confirmation opens on Cancel (a second Enter never
  // approves), this one too: the ack box is reached on purpose, and Space ticks it.
  await goTo(page, 'developers', 'projects');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the Developers dialog`));
  check(await focusKey(page) === 'dlg:cancel', `Developers dialog focus is ${await focusKey(page)}, expected Cancel (primary disabled)`);
  check(await page.locator('[data-button="clean"]').isDisabled(), 'Clean is enabled before the warnings are read');
  await page.screenshot({ path: `${shot}developers-confirm-keyboard-${size}.png` });
  await page.waitForTimeout(350);
  await page.locator('[data-key="dlg:ack"]').focus();
  await page.keyboard.press('Space'); // the ack box is a native check box: Space ticks it
  await page.waitForFunction(() => document.querySelector('.ack input')?.checked === true, null, { timeout: 2000 }).catch(() => failures.push(`${size}: the ack box did not tick`));
  check(!(await page.locator('[data-button="clean"]').isDisabled()), 'Clean stayed disabled after the warnings were read');
  check(await focusKey(page) === 'dlg:ack', `after ticking, the focus is on ${await focusKey(page)}, expected it to stay on the ack box`);
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  check(await badgeOf(page) === 'Analysis done', 'Escape in the Developers dialog changed the state');

  // Escape cancels a running clean when no dialog is open (it ends in the cancelled report, the analysis dropped), and closes a report (the section is empty again).
  await page.evaluate(() => window.__xioxMock.pause(true));
  await goTo(page, 'cleanup', 'cleaning');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Cleanup cancelled' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not cancel the running clean`));
  check(await page.locator('#rows .report .rp-cancelled').count() === 1, 'the cancelled clean shows no report');
  check(await page.locator('[data-key="action:analyze"]').isEnabled() && await page.locator('[data-key="action:clean"]').isDisabled(), 'the cancelled report must offer Analyze and not Clean');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the cancelled report`));
  await goTo(page, 'cleanup', 'report');
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the report`));
  check(await page.locator('#rows .report').count() === 0, 'the report is still on screen after Escape');
  check(await page.locator('#rows tbody tr[data-row]').count() > 0, 'the list did not come back after the report closed');
  await page.evaluate(() => window.__xioxMock.pause(false));

  // Keys are ignored while typing; Escape leaves the field. (Cleanup's search box looks through every category.)
  await goTo(page, 'cleanup', 'analyzed');
  const search = page.locator('#rows .xi-search input');
  await search.focus();
  await page.keyboard.type('a1 ');
  await page.waitForTimeout(450);
  check(await search.inputValue() === 'a1 ', 'typing in the search box lost characters to shortcuts');
  check(await badgeOf(page) === 'Analysis done' && await page.locator('#sections [data-section="cleanup"][aria-current="page"]').count() === 1, 'typing in the search box acted as a shortcut');
  check(await page.locator('#rows [data-row]').count() > 0 && await page.locator('[role=dialog]').count() === 0, 'typing in the search box opened or closed something');
  await page.keyboard.press('Escape');
  check(!(await page.evaluate(() => document.activeElement?.closest('.xi-search') != null)), 'Escape did not leave the search box');
  await search.fill('');
  await page.waitForTimeout(300);

  // Arrows move the selection, Space ticks the selected row.
  await goTo(page, 'cleanup', 'analyzed');
  const selected = () => page.evaluate(() => document.querySelector('#rows tr[aria-selected="true"]')?.dataset.row ?? null);
  const ids = await page.locator('#rows tbody tr[data-row]:not(.xi-section)').evaluateAll((nodes) => nodes.map((n) => n.dataset.row));
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
  const tickBefore = await page.locator(`#rows [data-row="${esc(second)}"] [role=checkbox]`).getAttribute('aria-checked');
  await page.keyboard.press('Space');
  await page.waitForFunction(([row, was]) => document.querySelector(`#rows [data-row="${CSS.escape(row)}"] [role=checkbox]`)?.getAttribute('aria-checked') !== was, [second, tickBefore], { timeout: 2000 })
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
  await page.locator('#rows [data-row="p:1"] td.xi-name').click();
  await page.keyboard.press('ArrowLeft');
  await page.waitForTimeout(80);
  check(await page.locator('#rows [data-row="a:1"]').count() === 0, 'ArrowLeft did not close the selected parent');
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(80);
  check(await page.locator('#rows [data-row="a:1"]').count() === 1, 'ArrowRight did not open the selected parent');

  // Registry: its own keys press its own actions.
  await goTo(page, 'registry', 'idle');
  await page.keyboard.press('a');
  await page.locator('#badge', { hasText: 'Scanning' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key a did not start the scan`));
  await page.locator('#badge', { hasText: 'Scan done' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: the scan did not land`));
  await focusTable(page); // the list has just come: Enter is its own key
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not open the backup dialog`));
  check(await page.locator('[role=dialog] .dlg-buttons .im-btn').count() === 3, 'the backup dialog does not have its three buttons');
  check(await focusKey(page) === 'dlg:cancel', `the backup dialog opens with the focus on ${await focusKey(page)}, expected Cancel`);
  check(await page.locator('.dlg-buttons .im-btn').evaluateAll((b) => b.map((n) => n.dataset.button).join()) === 'cancel,nobackup,backup', 'the backup dialog buttons are not Cancel first, the confirming one last');
  await page.screenshot({ path: `${shot}registry-backup-keyboard-${size}.png` });
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});

  return failures;
}

/** The report: Save as text, an item's lines, Close; the progress bar of a running job. */
async function reportFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  await page.evaluate(() => window.__xioxMock.pause(true));
  await goTo(page, 'cleanup', 'report');
  const caret = page.locator('#rows .report .xi-caret').first();
  await caret.click();
  await page.waitForTimeout(100);
  check(await page.locator('#rows .report .rp-open li').count() === 2, `the first item's lines did not open (${await page.locator('#rows .report .rp-open li').count()})`);
  check(await page.locator('#rows .report .xi-caret').first().getAttribute('aria-expanded') === 'true', 'the caret does not say expanded');
  failures.push(...await checkFit(page, `report-expanded-${size}`));
  await page.screenshot({ path: `${shot}cleanup-report-expanded-${size}.png` });
  await page.locator('#rows .report [data-key="report:save"]').click();
  await page.waitForTimeout(150);
  check(await page.locator('.im-notices .im-notice[data-im-tone="err"]').count() === 0, 'Save as text… ended in an error');
  check(await page.locator('#rows .report').count() === 1, 'Save as text… closed the report');
  check((await page.locator('#rows .report .rp-saved').textContent().catch(() => null)) === 'Report saved.', 'the saved text is not shown after Save as text…');
  await page.locator('#rows .report [data-key="report:close"]').click();
  await page.locator('#badge', { hasText: 'Ready' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Close did not leave the report`));
  check(await page.locator('#rows .report').count() === 0, 'the report is still there after Close');
  // The progress bar of a running job: a fraction in the middle.
  await goTo(page, 'cleanup', 'analyzing');
  await page.evaluate(() => window.__xioxMock.pause(false));
  await page.locator('#job .im-progress[aria-valuenow="33"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the bar never showed a third`));
  await page.evaluate(() => window.__xioxMock.pause(true));
  const bar = await page.evaluate(() => {
    const t = document.querySelector('#job .im-progress');
    return t ? { now: Number(t.getAttribute('aria-valuenow')), width: t.getBoundingClientRect().width, fill: parseFloat(getComputedStyle(t, '::before').width), role: t.getAttribute('role') } : null;
  });
  check(bar !== null && bar.role === 'progressbar', 'no progress bar while a job runs');
  if (bar) check(Math.abs(bar.fill / bar.width * 100 - bar.now) < 8, `the bar is filled ${(bar.fill / bar.width * 100).toFixed(0)} % but says ${bar.now} %`);
  await page.screenshot({ path: `${shot}cleanup-analyzing-bar-${size}.png` });
  await page.evaluate(() => window.__xioxMock.pause(false));
  return failures;
}

/** The commands the page sent since `clearCalls`: [command, intent kind or null, the intent]. */
const callsOf = (page) => page.evaluate(() => window.__xioxMock.calls.map((c) => [c.command, c.args?.intent?.kind ?? null, c.args?.intent ?? c.args ?? null]));
const clearCalls = (page) => page.evaluate(() => window.__xioxMock.clearCalls());
const sent = async (page, command, kind, match = () => true) => (await callsOf(page)).some(([c, k, a]) => c === command && k === kind && match(a));

/**
 * The sections end to end, from the briefs of Tasks 12 to 14: what a click or a key SENDS (read from the mock's log,
 * not from what is drawn), and the page's own behaviours (roots list, number field, search box, keep list, lines).
 * One page, one viewport.
 */
async function sectionFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  await page.evaluate(() => window.__xioxMock.pause(true));

  // ---- Developers: roots, number field, projects, worktrees, sessions, dialogs, report.
  await goTo(page, 'developers', 'projects');
  await clearCalls(page);
  // The options of Developers are in the inspector, under the content header's Options button.
  if (!await openInspector(page, 'options')) failures.push(`${size}: the Options button did not open the inspector`);
  await page.locator('[data-key="opt:roots:1"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'selectRoot', (a) => a.index === 1), 'selecting a root did not send selectRoot { index: 1 }');
  await page.locator('[data-key="opt:roots:add"]').click();
  await page.waitForTimeout(100);
  check((await callsOf(page)).some(([c, , a]) => c === 'developers_add_root' && (a === null || Object.keys(a).length === 0)), 'Add folder… did not call developers_add_root without arguments (a path never comes from the page)');
  await page.locator('[data-key="opt:roots:remove"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'removeRoot'), 'Remove did not send removeRoot');
  const days = page.locator('[data-key="opt:inactiveDays"]');
  await days.fill('45');
  await days.blur();
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'setInactiveDays', (a) => a.days === 45), 'leaving the number field did not send setInactiveDays { days: 45 }');
  await clearCalls(page);
  await page.locator('[data-key="opt:inactiveDays"]').fill('60');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'setInactiveDays', (a) => a.days === 60), 'Enter in the number field did not send setInactiveDays { days: 60 }');
  await clearCalls(page);
  await page.locator('[data-key="opt:removeMode:permanent"]').click({ force: true });
  await page.locator('[data-key="opt:scanRoots"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'setRemoveMode', (a) => a.mode === 'permanent') && await sent(page, 'developers', 'setScanRoots', (a) => a.on === false), 'the removal mode and the scan-folders box did not send their intents');
  await closeInspector(page);
  // The check box at the top of the list sends selectAll: on while something is left to tick, off once everything is ticked.
  await clearCalls(page);
  await page.locator('[data-key="bulk:master"]').click();
  await page.waitForFunction(() => document.querySelector('[data-key="bulk:master"]')?.getAttribute('aria-checked') === 'true', null, { timeout: 2000 })
    .catch(() => failures.push(`${size}: the top check box of Developers did not turn on after selectAll`));
  await page.locator('[data-key="bulk:master"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'developers', 'selectAll', (a) => a.on === true) && await sent(page, 'developers', 'selectAll', (a) => a.on === false), 'the top check box did not send selectAll on, then off');
  // A project row: Space ticks the parent (the mock changes that row only, as the script says), a blocked project and its child stay off.
  await goTo(page, 'developers', 'projects');
  await page.locator('#rows [data-row="p:2"] td.xi-name').click();
  await page.keyboard.press('Space');
  await page.waitForFunction(() => document.querySelector('#rows [data-row="p:2"] [role=checkbox]')?.getAttribute('aria-checked') === 'true', null, { timeout: 2000 })
    .catch(() => failures.push(`${size}: Space on a project row did not tick it`));
  check(await page.locator('#rows [data-row="p:3"] [role=checkbox]').isDisabled(), 'the checkbox of the project in use is not disabled');
  const setOpen = async (id, open) => {
    const caret = page.locator(`#rows [data-row="${esc(id)}"] .xi-caret`);
    // The state is the row's (aria-expanded), the caret is only the mouse's handle.
    if ((await page.locator(`#rows [data-row="${esc(id)}"]`).getAttribute('aria-expanded')) !== String(open)) await caret.click({ force: true });
  };
  await setOpen('p:1', false);
  check(await page.locator('#rows [data-row="a:3"]').count() === 0, 'a collapsed parent shows its caution child');
  await setOpen('p:1', true);
  check(await page.locator('#rows [data-row="a:3"]  [data-level="caution"]').count() === 1, 'the caution child is not drawn at the caution level');
  await setOpen('p:3', true);
  check(await page.locator('#rows [data-row="a:5"] [role=checkbox]').isDisabled(), 'the child of the project in use is not disabled');
  // The projects' tree: a folder folds everything inside it and opens again (its id holds a backslash).
  const projectsFolder = 'h:folder:D:\\Projects';
  await setOpen(projectsFolder, false);
  check(await page.locator('#rows [data-row="p:1"]').count() === 0 && await page.locator('#rows [data-row="a:3"]').count() === 0, 'a folded folder still shows what is inside it');
  check(await page.locator('#rows [data-row="p:3"]').count() === 1, 'folding a folder hid the projects of its sibling');
  await setOpen(projectsFolder, true);
  check(await page.locator('#rows [data-row="p:1"]').count() === 1, 'an opened folder does not show its projects');
  failures.push(...await checkFit(page, `developers-projects-all-open-${size}`));
  await page.screenshot({ path: `${shot}developers-projects-all-open-${size}.png` });
  // Worktrees: the other-worktrees header, rows without a box, the blocked one.
  await goTo(page, 'developers', 'worktrees');
  check(await page.locator('#rows [data-row="h:others"]').count() === 1, 'the other-worktrees header is missing');
  check(await page.locator('#rows [data-row="o:1"] [role=checkbox]').count() === 0, 'a worktree made by hand has a box');
  // Sessions: the database-only row says so instead of a size.
  await goTo(page, 'developers', 'sessions');
  check((await page.locator('#rows [data-row="s:opencode:91d2a0"] .xi-size').textContent()) === 'database row only', 'the database-only session does not say so');
  // The dialogs: Clean waits for the warnings; the shortfall has Permanently first, then Cancel.
  await goTo(page, 'developers', 'confirm');
  check(await page.locator('[data-button="clean"]').isDisabled(), 'Clean is enabled before the warnings are read');
  await page.locator('[data-key="dlg:ack"]').click();
  await page.waitForTimeout(100);
  check(!(await page.locator('[data-button="clean"]').isDisabled()), 'Clean stayed disabled after the warnings were read');
  await goTo(page, 'developers', 'shortfall');
  check(await page.locator('.dlg-buttons .im-btn').evaluateAll((b) => b.map((n) => n.textContent).join()) === 'Cancel,Clean 2 folders permanently', 'the shortfall dialog does not offer Cancel, then the verb and object with permanently');
  await page.screenshot({ path: `${shot}developers-shortfall-${size}.png` });
  // Cleaning: Escape sends cancel and lands on the cancelled report.
  await goTo(page, 'developers', 'cleaning');
  await clearCalls(page);
  await page.keyboard.press('Escape');
  await page.locator('#badge', { hasText: 'Cleanup cancelled' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not cancel the Developers clean`));
  check(await sent(page, 'developers', 'cancel'), 'Escape did not send cancel');
  // Report: Save as text… shows its answer.
  await goTo(page, 'developers', 'report');
  await page.locator('#rows .report [data-key="report:save"]').click();
  await page.locator('#rows .report .rp-saved').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Save as text… showed no answer in Developers`));
  check((await page.locator('#rows .report .rp-lines').count()) === 2, 'the Developers report does not list its skipped and failed lines');

  // ---- Cleanup: every category in one list (group rows, the menu scrolls, folding), Applications (search box, headers, blocked row, long lines), Browser (keep list, native lines), the partial analysis.
  await goTo(page, 'cleanup', 'analyzed');
  check(await page.locator('#rows tr.xi-group').count() === 7, 'the Cleanup list does not hold its seven categories');
  check(await page.locator('#rows tr.xi-group[data-row="winapp2"][aria-expanded="false"]').count() === 1, 'the long Applications category does not start folded');
  check(await page.locator('#rows [data-row="opera/cache"]').count() === 0, 'a folded category still draws its rows');
  check(await page.locator('#rows [data-row="cat:0"]').count() === 1 && await page.locator('#rows [data-row="npm/cache"]').count() === 1, 'the short categories are not open');
  // The menu scrolls to a category and marks the one the list is at.
  await page.evaluate(() => window.__xioxMock.pause(true));
  await page.locator('[data-key="nav:rules:Browser"]').click();
  await page.waitForTimeout(900);
  const atTop = await page.evaluate(() => {
    const grid = document.querySelector('#rows .xi-list');
    const anchor = document.querySelector('#rows .xi-anchor[data-anchor="rules:Browser"]');
    const head = grid.querySelector('thead th').getBoundingClientRect().bottom;
    return Math.abs(anchor.getBoundingClientRect().top - head);
  });
  check(atTop < 4, `the menu did not scroll Browser to the top of the list (${atTop}px off)`);
  check((await page.locator('#sidebar [data-key="nav:rules:Browser"][aria-current="true"]').count()) === 1 && await page.locator('#sidebar .xi-row[aria-current]').count() === 1, 'the menu does not mark Browser as the category the list is at');
  // Scrolling by hand moves the mark.
  await page.locator('#rows .xi-list').evaluate((n) => { n.scrollTop = 0; });
  await page.waitForTimeout(250);
  check((await page.locator('#sidebar [data-key="nav:builtin:System"][aria-current="true"]').count()) === 1, 'the menu did not follow the list back to System');
  // A group box ticks what it may: System is mixed (4 of 6), one click ticks all six, another clears them.
  await clearCalls(page);
  await page.locator('#rows [data-key="ck:builtin:System"]').click();
  await page.waitForFunction(() => document.querySelector('#rows [data-key="ck:builtin:System"]')?.getAttribute('aria-checked') === 'true', null, { timeout: 2000 })
    .catch(() => failures.push(`${size}: the System box did not turn on`));
  check(await sent(page, 'cleanup', 'toggle', (a) => a.row === 'builtin:System'), 'the group box did not send toggle { row: builtin:System }');
  check(await page.locator('#rows [data-row="cat:5"] [role=checkbox]').getAttribute('aria-checked') === 'true', 'the group box did not tick the rows of its category');
  // The caret and a click on the header fold it; Fold all / Open all do every category.
  await page.locator('#rows tr.xi-group[data-row="builtin:System"] .xi-caret').click();
  check(await page.locator('#rows [data-row="cat:0"]').count() === 0, 'the caret of a category did not fold it');
  await page.locator('#rows tr.xi-group[data-row="builtin:System"] td.xi-name').click();
  check(await page.locator('#rows [data-row="cat:0"]').count() === 1, 'a click on the category header did not open it again');
  await page.locator('[data-key="fold:all"]').click();
  check(await page.locator('#rows tbody tr[data-row]:not(.xi-group):not(.xi-section)').count() === 0 && (await page.locator('[data-key="fold:all"]').textContent()) === 'Open all', 'Fold all did not fold every category');
  await page.locator('[data-key="fold:all"]').click();
  check(await page.locator('#rows [data-row="opera/cache"]').count() === 1, 'Open all did not open Applications');
  await page.locator('[data-key="nav:winapp2"]').click();
  check((await page.locator('#sidebar [data-key="nav:winapp2"]').count()) === 1, 'the Applications entry is gone from the menu');
  check(await page.locator('#rows tr.xi-section, #rows tr.xi-heading').count() === 6, 'Applications does not list its six Winapp2 sections');
  const search = page.locator('#rows .xi-search input');
  await clearCalls(page);
  await search.click();
  await page.keyboard.type('chrome', { delay: 20 });
  await page.waitForTimeout(450);
  check(await sent(page, 'cleanup', 'search', (a) => a.text === 'chrome'), 'the search box did not send search { text: "chrome" } after the pause');
  await search.evaluate((n) => n.setSelectionRange(2, 2));
  await page.evaluate(() => window.__xioxMock.go('cleanup', 'analyzed')); // another view comes back (its search value is empty)
  await page.waitForTimeout(150);
  check(await search.inputValue() === 'chrome', 'the search box lost its text when the next view came back');
  check(await search.evaluate((n) => [n.selectionStart, n.selectionEnd, document.activeElement === n].join()) === '2,2,true', 'the search box lost its caret or focus when the next view came back');
  await page.keyboard.press('Escape');
  check(!(await page.evaluate(() => document.activeElement?.closest('.xi-search') != null)), 'Escape did not leave the search box');
  await search.fill('');
  // A blocked row, and a row whose long list of lines scrolls inside the detail box.
  await page.locator('#rows [data-row="spotify/cache"] td.xi-name').click();
  await page.waitForTimeout(100);
  if (!await openInspector(page, 'details')) failures.push(`${size}: the Details button did not open the inspector`);
  // The lines scroll natively inside the inspector's body, and every one is reachable: scrolled to the end, the last line is inside the box.
  const scroll = await page.locator('.im-inspector .im-panel-body').evaluate((n) => {
    n.scrollTop = n.scrollHeight;
    const last = n.querySelector('.xi-lines li:last-child');
    const end = last ? last.getBoundingClientRect().bottom : Infinity;
    return [getComputedStyle(n).overflowY, n.querySelectorAll('.xi-lines li').length, end <= n.getBoundingClientRect().bottom + 1, n.scrollHeight <= n.clientHeight || n.scrollTop > 0];
  });
  check(scroll[0] === 'auto' && scroll[1] === 31 && scroll[2] && scroll[3], `the details do not scroll their lines natively (${JSON.stringify(scroll)})`);
  failures.push(...await checkFit(page, `cleanup-applications-lines-${size}`));
  await page.screenshot({ path: `${shot}cleanup-applications-lines-${size}.png` });
  // Browser: the keep list button sends its intent and explains itself.
  await page.locator('[data-key="nav:rules:Browser"]').click();
  await page.waitForTimeout(900);
  if (!await openInspector(page, 'options')) failures.push(`${size}: the Options button did not open the inspector`);
  await page.locator('[data-key="opt:keepList"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the options show no keep list button`));
  check(((await page.locator('[data-key="opt:keepList"]').getAttribute('title')) ?? '').length > 20, 'the keep list button has no explanation');
  await clearCalls(page);
  await page.locator('[data-key="opt:keepList"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'cleanup', 'openKeepList'), 'the keep list button did not send openKeepList');
  await page.locator('#rows [data-row="native:cookies"] td.xi-name').click();
  if (!await openInspector(page, 'details')) failures.push(`${size}: the Details button did not open the inspector`);
  check(await page.locator('.im-inspector .xi-lines li').count() === 36, 'the cookies row does not show its 36 lines in the details');
  await page.screenshot({ path: `${shot}cleanup-browser-${size}.png` });
  await closeInspector(page);
  // The badge is capitals by CSS and sentence case in the copy; the partial analysis keeps Clean on.
  await goTo(page, 'cleanup', 'interrupted');
  // The badge's words are sentence case in the copy; the look sets them (ONI: capitals by --im-case; Smooth: as written).
  const look = await page.evaluate(() => document.documentElement.getAttribute('data-im-look'));
  check(await page.locator('#badge').evaluate((n) => getComputedStyle(n).textTransform) === (look === 'oni' ? 'uppercase' : 'none'), `the badge is not set as the look says (${look})`);
  check(await page.locator('#notices .im-note[data-im-tone="err"]').count() === 1 && await page.locator('[data-key="action:clean"]').isEnabled(), 'the cancelled analysis does not show its notice with Clean enabled');
  await page.screenshot({ path: `${shot}cleanup-interrupted-${size}.png` });
  // The confirmation: three warnings, Continue first and primary.
  await goTo(page, 'cleanup', 'confirm');
  check(await page.locator('.dlg-warn li').count() === 3, 'the Cleanup confirmation does not show its three warnings');
  check(await page.locator('.dlg-buttons .im-btn').evaluateAll((b) => b.map((n) => `${n.textContent}:${n.classList.contains('im-btn--confirm-danger')}`).join()) === 'Cancel:false,Clean 8 items:true', 'the Cleanup dialog is not Cancel, Clean 8 items (confirming)');
  // The report: its notes, its skipped and failed lines, item lines; Save as text….
  await goTo(page, 'cleanup', 'report');
  check(await page.locator('#rows .report .rp-lines').count() === 2, 'the Cleanup report does not list Skipped and Failed');
  await page.locator('#rows .report [data-key="report:save"]').click();
  await page.locator('#rows .report .rp-saved').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Save as text… showed no answer in Cleanup`));

  // ---- Registry: bulk buttons, the backup dialog, the ok notice with its extra line.
  await goTo(page, 'registry', 'analyzed');
  await clearCalls(page);
  await page.locator('[data-key="bulk:master"]').click();
  await page.locator('[data-key="bulk:master"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'registry', 'selectAll', (a) => a.on === true) && await sent(page, 'registry', 'selectAll', (a) => a.on === false), 'the Registry top check box did not send selectAll on, then off');
  await page.locator('[data-key="opt:cat:10"]').click();
  await page.waitForTimeout(100);
  check(await sent(page, 'registry', 'toggleCategory', (a) => a.index === 10), 'a category box did not send toggleCategory { index: 10 }');
  check(await page.locator('#rows [data-row="i:39"]').count() === 1 || (await page.locator('#rows .xi-list').evaluate((n) => n.scrollHeight > n.clientHeight)), 'the 40 issues are not all reachable in the list');
  await goTo(page, 'registry', 'fixed');
  check((await page.locator('#notices [data-key^="more:"]').textContent().catch(() => '')) === 'Show 1 more line', 'the fixed notice does not say it has one more line');
  await page.screenshot({ path: `${shot}registry-fixed-${size}.png` });
  await page.evaluate(() => window.__xioxMock.pause(false));
  return failures;
}

/**
 * The final fix wave: what a notice hides is reachable by keyboard, a skipped path is on the page, All / None wait for a
 * job, a double click sends once, the Options of Developers reach their folder buttons, the focus does not fall to nothing
 * when a report closes.
 */
async function fixWaveFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  await page.evaluate(() => window.__xioxMock.pause(true));

  // The extra lines of a notice: a button, reached with Tab, opened with Enter, closed with Space; the focus stays on it.
  await goTo(page, 'registry', 'fixed');
  const more = page.locator('#notices [data-key^="more:"]');
  check(await more.count() === 1 && await more.getAttribute('aria-expanded') === 'false' && await page.locator('#notices .xi-more').count() === 0, 'the fixed notice does not start closed with a +1 more button');
  check(await more.evaluate((n) => n.tagName === 'BUTTON' && !n.closest('[title]')), 'the extra lines are behind something that is not a plain button (a tooltip is not reachable)');
  await fromTop(page);
  let reached = false;
  for (let i = 0; i < 80 && !reached; i += 1) {
    await page.keyboard.press('Tab');
    reached = ((await focusKey(page)) ?? '').startsWith('more:');
  }
  check(reached, 'Tab never reaches the +1 more button');
  await page.keyboard.press('Enter');
  await page.locator('#notices .xi-more li').first().waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Enter on +1 more did not open the line`));
  check((await page.locator('#notices .xi-more').innerText().catch(() => '')).includes('registry-backup-'), 'the opened line does not show the backup path');
  check(((await focusKey(page)) ?? '').startsWith('more:') && await page.locator('#notices [data-key^="more:"]').getAttribute('aria-expanded') === 'true', 'the button lost the focus or does not say expanded after Enter');
  failures.push(...await checkFit(page, `registry-fixed-open-${size}`));
  await page.screenshot({ path: `${shot}registry-fixed-notice-open-${size}.png` });
  await page.keyboard.press('Space');
  await page.locator('#notices .xi-more').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Space on the open notice did not close it`));
  check(await page.locator('#notices [data-key^="more:"]').getAttribute('aria-expanded') === 'false', 'the button does not say collapsed after Space');
  // A partial failure is a warning, not a success.
  await goTo(page, 'registry', 'partial');
  check(await page.locator('#notices .im-note[data-im-tone="warn"]').count() === 1 && await page.locator('#notices .im-note[data-im-tone="ok"]').count() === 0, 'the partial fix is not shown as a warning');
  await page.screenshot({ path: `${shot}registry-partial-${size}.png` });

  // Skipped / failed: the full path is on the page (a line of its own), not in a tooltip.
  await goTo(page, 'cleanup', 'report');
  const paths = await page.locator('#rows .report .rp-lines .rp-path').allInnerTexts();
  check(paths.filter(Boolean).length > 0 && paths.filter(Boolean).every((p) => p.length > 8), `the skipped / failed paths are not on the page (${JSON.stringify(paths)})`);
  check(await page.locator('#rows .report .rp-lines .rp-row[title]').count() === 0, 'a report row still hides its path in a tooltip');
  check(await page.locator('#rows .report table td, #rows .report table th').count() > 0, 'the report tables have no cells');
  await page.screenshot({ path: `${shot}cleanup-report-paths-${size}.png` });

  // The top check box waits while a job runs (checked on a view Rust would not send: bulk with a job).
  await page.evaluate(() => window.__xioxMock.patch('registry', 'analyzed', { job: { label: 'Fixing…', fraction: 0.5, detail: null } }));
  await goTo(page, 'registry', 'analyzed');
  await page.waitForTimeout(100);
  check(await page.locator('[data-key="bulk:master"]').isDisabled(), 'the top check box is enabled while a job runs');
  await page.evaluate(() => window.__xioxMock.patch('registry', 'analyzed', { job: null }));
  await goTo(page, 'registry', 'analyzed');
  await page.waitForTimeout(100);
  check(await page.locator('[data-key="bulk:master"]').isEnabled(), 'the top check box stays disabled after the job');

  // A double click on a dialog button sends its intent once (the second one is dropped while the first is in flight).
  await goTo(page, 'cleanup', 'confirm');
  await clearCalls(page);
  await page.evaluate(() => {
    const button = document.querySelector('[data-button="continue"]');
    button.click();
    button.click();
  });
  await page.waitForTimeout(200);
  const confirms = (await callsOf(page)).filter(([c, k]) => c === 'cleanup' && k === 'confirm').length;
  check(confirms === 1, `a double click sent confirm ${confirms} times`);
  check(await page.locator('.im-notices .im-notice[data-im-tone="err"]').count() === 0, 'a double click on Continue left an error notice');

  // The report closing does not leave the focus on nothing.
  await goTo(page, 'cleanup', 'report');
  await page.locator('#rows .report [data-key="report:close"]').focus();
  await page.keyboard.press('Enter');
  await page.locator('#rows .report').waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Close did not leave the report`));
  check(await page.evaluate(() => document.activeElement?.classList.contains('xi-table') ?? false), `after the report closed the focus is on ${await focusKey(page)}, expected the list`);

  // The folder buttons of Developers are in its Options (the inspector): the panel is a native scroller, and each button can be brought
  // into it (a window of 640 is not tall enough for every option at once).
  await goTo(page, 'developers', 'projects');
  if (!await openInspector(page, 'options')) failures.push(`${size}: the Options button did not open the inspector`);
  const panel = await page.evaluate(() => {
    const body = document.querySelector('.im-inspector .im-panel-body');
    const reach = (id) => {
      const node = document.querySelector(`.im-inspector [data-key^="opt:"][data-key$=":${id}"]`);
      if (!node) return false;
      node.scrollIntoView({ block: 'nearest' });
      const box = body.getBoundingClientRect();
      const r = node.getBoundingClientRect();
      return r.left >= box.left - 0.5 && r.right <= box.right + 0.5 && r.top >= box.top - 0.5 && r.bottom <= box.bottom + 0.5;
    };
    return { overflow: getComputedStyle(body).overflowY, add: reach('add'), remove: reach('remove') };
  });
  check(panel.overflow === 'auto', `the options panel does not scroll natively (${panel.overflow})`);
  check(panel.add && panel.remove, 'Add folder… / Remove are not reachable in the options panel of Developers');
  await page.screenshot({ path: `${shot}developers-options-${size}.png` });
  await closeInspector(page);
  await page.evaluate(() => window.__xioxMock.pause(false));
  return failures;
}

/**
 * Settings, the fourth view: its tab, `4` and `?` open it; it offers the look and the theme and says what Rust says (About); it is no
 * dialog (nothing is inert behind it); a section's tab or its digit brings the section back.
 */
async function settingsFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  const about = fixture.about;
  const settings = page.locator('#rows .xi-settings');
  const tab = (id) => page.locator(`#sections [data-section="${id}"]`);
  const theme = () => page.evaluate(() => document.documentElement.getAttribute('data-im-theme'));
  const leave = async () => {
    await tab('cleanup').click();
    await settings.waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: a section's tab did not leave Settings`));
  };
  await goTo(page, 'cleanup', 'analyzed');
  await tab('settings').click();
  await settings.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the Settings tab did not open Settings`));
  check(await page.locator('#sections [data-section="settings"][aria-current="page"]').count() === 1 && await page.locator('#sections [aria-current]').count() === 1, 'Settings is not the one current tab');
  const text = (await settings.innerText().catch(() => '')).replace(/\s+/g, ' ');
  for (const part of ['Look', 'Smooth', 'ONI mode', 'Theme', 'Windows', 'Light', 'Dark', 'About Xiox', `Version ${about.version}`, about.winapp2, about.attribution]) {
    check(text.toLowerCase().includes(part.toLowerCase()), `Settings does not show ${JSON.stringify(part)}: ${text}`);
  }
  check((await page.locator('#list-head .im-panel-title').textContent()) === 'Settings', 'the content header of Settings does not say Settings');
  check((await page.locator('#sidebar .im-panel-title').textContent()) === 'Settings' && await page.locator('#sidebar [data-key^="settings:"]').count() === 2, 'the sidebar of Settings is not its Look and About rows');
  check(await focusKey(page) === 'settings:settings-look', `Settings opens with the focus on ${await focusKey(page)}, expected its first sidebar row`);
  check(await page.evaluate(() => !document.getElementById('body').inert && document.querySelector('[role=dialog]') === null), 'Settings is a dialog: the page behind it is inert');
  check(await badgeOf(page) === 'Analysis done', 'opening Settings changed the section state');
  failures.push(...await checkFit(page, `settings-${size}`));
  await page.screenshot({ path: `${shot}settings-${size}.png` });
  // A row of the sidebar scrolls to its part.
  await page.locator('[data-key="settings:settings-about"]').click();
  await page.waitForTimeout(500);
  check(await page.evaluate(() => {
    const heading = document.getElementById('settings-about').getBoundingClientRect();
    const box = document.querySelector('.xi-settings').getBoundingClientRect();
    return heading.top >= box.top - 1 && heading.bottom <= box.bottom + 1;
  }), 'the About row of the sidebar did not bring About into view');
  // The theme: each choice is drawn at once; Windows' own colours (dark in this pass) come back with the first one.
  const choose = (key) => page.locator(`[data-key="${key}"]`).click({ force: true });
  await choose('theme:light');
  check(await theme() === 'light' && await page.locator('[data-key="theme:light"]:checked').count() === 1, 'the Light theme was not drawn');
  await choose('theme:dark');
  check(await theme() === 'dark' && await page.locator('[data-key="theme:dark"]:checked').count() === 1, 'the Dark theme was not drawn');
  await choose('theme:system');
  check(await theme() === 'dark' && await page.locator('[data-key="theme:system"]:checked').count() === 1, 'the Windows theme was not drawn');
  // A digit brings a section back, whatever has the focus but a field.
  await page.locator('[data-key="settings:settings-look"]').focus();
  await page.keyboard.press('1');
  await settings.waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: the key 1 did not leave Settings`));
  check(await page.locator('#sections [data-section="cleanup"][aria-current="page"]').count() === 1 && (await page.locator('#sidebar .im-panel-title').textContent()) === 'Categories', 'the section did not come back with its own sidebar');
  check(await badgeOf(page) === 'Analysis done', 'leaving Settings changed the section state');
  // `?` and `4` open it from anywhere outside a field; a second one opens nothing more.
  await blur(page);
  await page.keyboard.press('Shift+Slash');
  await settings.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key ? did not open Settings`));
  await page.keyboard.press('Shift+Slash');
  check(await page.locator('#rows .xi-settings').count() === 1, 'a second ? opened a second Settings');
  await leave();
  await blur(page);
  await page.keyboard.press('4');
  await settings.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the key 4 did not open Settings`));
  await leave();
  // A question mark typed in the search box is typing.
  const search = page.locator('.xi-search input');
  if (await search.count() > 0) {
    await search.focus();
    await page.keyboard.type('?');
    await page.waitForTimeout(80);
    check(await settings.count() === 0, 'a ? typed in the search box opened Settings');
    await search.fill('');
    await blur(page);
  }
  // A confirmation keeps the keyboard: ? does not open Settings behind it.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Enter');
  await page.locator('[role=dialog]').waitFor({ timeout: 2000 }).catch(() => {});
  await page.keyboard.press('Shift+Slash');
  await page.waitForTimeout(100);
  check(await settings.count() === 0, 'a ? opened Settings over a confirmation');
  await page.keyboard.press('Escape');
  await page.locator('[role=dialog]').waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  return failures;
}

/**
 * The window's chrome: Ctrl+K opens the command palette over the window (and the title bar's command button does too), it lists the commands
 * by their words, Enter runs the one it stands on and Escape closes it.
 */
async function paletteFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  const palette = page.locator('.im-palette');
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Control+k');
  await palette.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Ctrl+K did not open the command palette`));
  check(await page.evaluate(() => document.activeElement?.classList.contains('im-palette-input') ?? false), 'the palette does not have the focus in its field');
  await page.keyboard.type('registry');
  const titles = await page.locator('.im-palette-list [role=option] > span:first-child').allInnerTexts();
  check(titles.includes('Go to Registry'), `the palette does not list Go to Registry for "registry" (${JSON.stringify(titles)})`);
  await page.screenshot({ path: `${shot}palette-${size}.png` });
  await page.keyboard.press('Enter');
  await palette.waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Enter did not close the palette`));
  await page.locator('#sections [data-section="registry"][aria-current="page"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the palette's command did not open Registry`));
  // The command button of the title bar opens it too; Escape closes it and runs nothing.
  await page.locator('.im-titlebar-command').click();
  await palette.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the command button did not open the palette`));
  await page.keyboard.press('Escape');
  await palette.waitFor({ state: 'detached', timeout: 2000 }).catch(() => failures.push(`${size}: Escape did not close the palette`));
  check(await page.locator('#sections [data-section="registry"][aria-current="page"]').count() === 1, 'Escape in the palette changed the section');
  // The view's own actions are in it, under the words Rust wrote.
  await goTo(page, 'cleanup', 'analyzed');
  await page.keyboard.press('Control+k');
  await palette.waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Ctrl+K did not open the command palette again`));
  await page.keyboard.type('analy');
  const actions = await page.locator('.im-palette-list [role=option] > span:first-child').allInnerTexts();
  check(actions.includes(fixture.views.cleanup.analyzed.actions.find((a) => a.id === 'analyze').label), `the palette does not list the Analyze action (${JSON.stringify(actions)})`);
  await page.keyboard.press('Escape');
  await palette.waitFor({ state: 'detached', timeout: 2000 }).catch(() => {});
  return failures;
}

/** The warning for a profile that is not the user's (`app.profileMismatch`): a note under the content header, there only when Rust sends it, and everything still fits. */
async function profileFlows(page, size, shot) {
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(`${size}: ${message}`); };
  const ready = () => page.locator('#sections [data-section="cleanup"][aria-selected="true"]').waitFor();
  const note = page.locator('#notices > .im-note[title]');
  check(await note.count() === 0, 'the profile note shows without a mismatch');
  const message = 'Xiox is cleaning the profile of “Alfio”, not that of “Administrator”, who opened this Windows session.';
  const long = `Xiox is cleaning the profile of “${'Averylongusername'.repeat(8)}”, not that of “${'Anotherlongname'.repeat(8)}”, who opened this Windows session.`;
  for (const [label, text] of [['normal', message], ['long', long]]) {
    await page.evaluate((t) => window.__xioxMock.setMismatch(t), text);
    await page.reload();
    await ready();
    await page.waitForTimeout(150);
    const strip = await page.evaluate(() => {
      const node = document.querySelector('#notices > .im-note[title]');
      if (!node) return null;
      const r = node.getBoundingClientRect();
      const rect = (id) => document.getElementById(id).getBoundingClientRect();
      return {
        text: node.textContent, tone: node.getAttribute('data-im-tone'), role: node.getAttribute('role'), top: r.top, bottom: r.bottom,
        headBottom: rect('list-head').bottom, rowsTop: rect('rows').top, mainBottom: rect('main').bottom, actionsTop: rect('actions').top, footBottom: rect('foot').bottom, vh: innerHeight,
        cut: node.scrollWidth > node.clientWidth + 1 || node.scrollHeight > node.clientHeight + 1,
      };
    });
    check(strip !== null && strip.text === text, `the profile note (${label}) does not show the mismatch text`);
    if (strip === null) continue;
    check(strip.tone === 'warn' && strip.role === 'alert', `the profile note (${label}) is ${strip.tone}/${strip.role}, expected a warning that is announced`);
    check(strip.top >= strip.headBottom - 1 && strip.bottom <= strip.rowsTop + 1, `the profile note (${label}) is not between the content header and the list`);
    check(strip.mainBottom <= strip.actionsTop + 1 && strip.footBottom <= strip.vh + 1, `the content or the footer are pushed out of the window by the profile note (${label})`);
    check(!strip.cut, `the profile note (${label}) cuts its text instead of wrapping it`);
    failures.push(...await checkFit(page, `profile-${label}-${size}`));
    await page.screenshot({ path: `${shot}profile-${label}-${size}.png` });
  }
  await page.evaluate(() => window.__xioxMock.setMismatch(null));
  await page.reload();
  await ready();
  check(await note.count() === 0, 'the profile note stays after the mismatch is gone');
  return failures;
}

/**
 * Tab from the top of the page visits, in order: the skip link, the title bar (the sections and Settings, the command button, the window
 * buttons), the sidebar, the content header's buttons, the notices, the list's tools, the list (one stop), the inspector (its close button
 * and its body, one stop each), then the action bar. Every stop has a visible ring that the cut corners cannot hide.
 */
async function tabAudit(page, size, shot) {
  const failures = [];
  const REGIONS = ['.im-shell-skip-link', '#top', '#sidebar', '#list-head', '#notices', '#rows .xi-tools', '#rows .xi-table', '.im-inspector', '#actions'];
  const [SIDEBAR, GRID, INSPECTOR, ACTIONS] = [2, 6, 7, 8];
  for (const [section, state] of [['cleanup', 'analyzed'], ['developers', 'projects'], ['registry', 'analyzed']]) {
    const label = `${size} Tab audit ${section}:${state}`;
    await goTo(page, section, state);
    if (!await openInspector(page, 'details')) failures.push(`${label}: the Details button did not open the inspector`);
    await fromTop(page);
    const stops = [];
    for (let i = 0; i < 80; i += 1) {
      await page.keyboard.press('Tab');
      const stop = await page.evaluate((regions) => {
        const el = document.activeElement;
        if (!el || el === document.body || el === document.documentElement) return null;
        const style = getComputedStyle(el);
        const ring = style.boxShadow;
        const outlined = style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0;
        let clipped = false;
        for (let n = el; n; n = n.parentElement) if (getComputedStyle(n).clipPath !== 'none') clipped = true;
        return {
          key: el.getAttribute('data-key') ?? el.tagName.toLowerCase(), region: regions.findIndex((sel) => el.closest(sel) !== null),
          ring, outlined, clipped, tab: el.tabIndex,
        };
      }, REGIONS);
      if (stops.length > 0 && (stop === null || stop.key === stops[0].key)) break;
      if (stop === null) { failures.push(`${label}: Tab ${i + 1} left the focus on the page body`); break; }
      stops.push(stop);
      // What the rings look like, once per stop worth looking at (Cleanup only).
      if (section === 'cleanup' && (stop.key === 'section:settings' || stop.region === GRID || stop.region === INSPECTOR || stop.key === 'action:clean')) {
        await page.screenshot({ path: `${shot}tab-${stop.region}-${stop.key.replace(/[^a-z0-9]+/gi, "_")}-${size}.png` });
        shots += 1;
      }
    }
    const names = stops.map((s) => `${REGIONS[s.region] ?? '?'}:${s.key}`).join(' ');
    for (const s of stops) {
      if (s.region < 0) failures.push(`${label}: ${s.key} is a Tab stop outside every region`);
      if (s.ring === 'none' && !s.outlined) failures.push(`${label}: ${s.key} has no visible focus ring`);
      if (s.clipped && !s.outlined && !s.ring.includes('inset')) failures.push(`${label}: the ring of ${s.key} is drawn outside its box under a clip-path (${s.ring})`);
    }
    const order = stops.map((s) => s.region);
    if (order.some((r, i) => i > 0 && r < order[i - 1])) failures.push(`${label}: the Tab order goes back: ${names}`);
    const keys = stops.map((s) => s.key);
    if (keys.filter((k) => k.startsWith('section:')).join() !== 'section:cleanup,section:registry,section:developers,section:settings') failures.push(`${label}: Tab visits the tabs as ${keys.filter((k) => k.startsWith('section:')).join()}, expected the three sections, then Settings`);
    if (!stops.some((s) => s.region === SIDEBAR)) failures.push(`${label}: Tab never reached the sidebar`);
    if (stops.filter((s) => s.region === GRID).length !== 1) failures.push(`${label}: the list is not exactly one Tab stop (${stops.filter((s) => s.region === GRID).length})`);
    const inspector = stops.filter((s) => s.region === INSPECTOR);
    if (inspector.length !== 2 || !inspector.some((s) => s.key === 'inspector:close')) failures.push(`${label}: the inspector is not its close button and its body (${inspector.map((s) => s.key).join()})`);
    if (stops.length === 0 || stops[stops.length - 1].region !== ACTIONS) failures.push(`${label}: Tab does not end on the action bar: ${names}`);
    // Arrows move inside the list once it has the focus, and the stop does not change.
    if (state === 'analyzed' || state === 'projects') {
      await page.locator('#rows .xi-table').focus();
      await page.keyboard.press('Home');
      await page.waitForTimeout(80);
      const before = await page.locator('#rows tr[aria-selected="true"]').getAttribute('data-row');
      await page.keyboard.press('ArrowDown');
      await page.waitForTimeout(80);
      const after = await page.locator('#rows tr[aria-selected="true"]').getAttribute('data-row');
      if (before === after) failures.push(`${label}: ArrowDown in the list did not move the selection (${before} -> ${after}; rows ${await page.locator('#rows tbody tr[data-row]').count()}; focus ${await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 80))})`);
      if (!await focusIn(page, '#rows .xi-table')) failures.push(`${label}: the focus left the list on ArrowDown`);
    }
    await closeInspector(page);
    if (section === 'cleanup') console.log(`${label}: ${names}`);
  }
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

/** The window speaks English: before every screenshot, the visible text of the page is scanned for Italian. */
export function watchLanguage(page, label, failures) {
  const shoot = page.screenshot.bind(page);
  const seen = new Set();
  page.screenshot = async (options) => {
    const lines = await page.evaluate(() => document.body.innerText.split('\n').map((line) => line.trim()).filter(Boolean));
    for (const line of lines) {
      if (findItalian(JSON.stringify(line), 'json').length === 0 || seen.has(line)) continue;
      seen.add(line);
      failures.push(`${label}: Italian text on screen: ${JSON.stringify(line)} (${options && options.path ? options.path.replace(/^.*[\\/]/, '') : 'screenshot'})`);
    }
    return shoot(options);
  };
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
    watchLanguage(page, 'failure page', failures);
    page.on('pageerror', (error) => failures.push(`failure page: page error ${error}`));
    // The window's own calls (its handle, its events, its theme) answer; the first command of the app does not.
    await page.addInitScript(() => {
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
        invoke: (command) => (command === 'app_info' ? Promise.reject({ kind: 'failed', message: 'the core did not answer' }) : Promise.resolve(null)),
        transformCallback: () => 0,
      };
    });
    await page.goto(url);
    // The content panel's error state: the words and a Retry that asks again.
    await page.locator('#main .im-failure').waitFor({ timeout: 3000 }).catch(() => failures.push('failure page: no error state in the content panel'));
    expect(((await page.locator('#main .im-failure').textContent().catch(() => '')) ?? '').includes('the core did not answer'), 'failure page: the error state does not say what failed');
    expect(await page.locator('#main .im-failure button', { hasText: 'Retry' }).count() === 1, 'failure page: the error state has no Retry');
    expect((await page.locator('#badge').textContent()) === 'Error', 'failure page: the badge does not say Error');
    await page.screenshot({ path: `${out}failure-1000x640.png` });
    await page.close();
  }

  for (const [width, height] of VIEWPORTS) {
    // Smooth follows the Windows theme: the pass runs dark, and the light palette is checked on its own below.
    const page = await browser.newPage({ viewport: { width, height }, colorScheme: 'dark' });
    const size = `${width}x${height}`;
    watchLanguage(page, size, failures);
    page.on('pageerror', (error) => failures.push(`${size}: page error ${error}`));
    page.on('console', (message) => { if (message.type() === 'error') failures.push(`${size}: console error ${message.text()}`); });
    await page.goto(url);
    await page.locator('#sections [data-section="cleanup"][aria-selected="true"]').waitFor();
    failures.push(...await checkTokens(page));
    await page.emulateMedia({ colorScheme: 'light' });
    // The page follows Windows' colours through the media query's change event.
    await page.waitForFunction(() => document.documentElement.getAttribute('data-im-theme') === 'light', null, { timeout: 2000 }).catch(() => undefined);
    failures.push(...await checkTokens(page, 'smooth-light'));
    await page.screenshot({ path: `${out}smooth-light-${size}.png` });
    await page.emulateMedia({ colorScheme: 'dark' });
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

    // A click on a column header orders the list by it and says so; a sub-heading's box goes to the model as a toggle of its id.
    await resetAll(page);
    await page.locator('#sections [data-section="registry"]').click();
    await page.evaluate(() => window.__xioxMock.go('registry', 'analyzed'));
    await page.locator('#rows [data-key="sort:Issue"]').click();
    expect(await page.locator('#rows th[aria-sort="ascending"] [data-key="sort:Issue"]').count() === 1, `${size}: the clicked column header is not marked ascending`);
    await page.evaluate(() => window.__xioxMock.clearCalls());
    await page.locator('#rows [data-key="ck:t:2"]').click();
    await page.waitForTimeout(100);
    expect(await page.evaluate(() => window.__xioxMock.calls.some((c) => c.command === 'registry' && c.args.intent.kind === 'toggle' && c.args.intent.row === 't:2')), `${size}: the box of a kind of issue sent no toggle`);
    expect(await page.locator('#rows [data-row="t:2"] [aria-checked="true"]').count() === 1, `${size}: the box of a kind of issue did not come back ticked`);

    // ONI MODE: chosen in Settings, it turns the look, the choice outlives a reload, and every state is drawn and fit-checked in it too.
    const chooseLook = async (skin) => {
      await page.locator('#sections [data-section="settings"]').click();
      await page.locator(`#rows .xi-settings [data-key="look:${skin}"]`).click({ force: true });
      await page.locator('#sections [data-section="cleanup"]').click();
      await page.locator('#sections [data-section="cleanup"][aria-current="page"]').waitFor({ timeout: 2000 });
    };
    expect(await page.locator('#skin').count() === 0, `${size}: the top bar still has an ONI MODE switch`);
    await chooseLook('oni');
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-im-look')) === 'oni', `${size}: Settings did not turn ONI MODE on`);
    await page.reload();
    await page.locator('#sections [data-section="settings"]').click();
    expect(await page.locator('#rows .xi-settings [data-key="look:oni"]:checked').count() === 1, `${size}: Settings does not say ONI MODE is on after a reload`);
    await page.locator('#sections [data-section="cleanup"]').click();
    await page.locator('#sections [data-section="cleanup"][aria-current="page"]').waitFor();
    failures.push(...await checkTokens(page, 'oni'));
    await page.evaluate(() => window.__xioxMock.pause(true));
    for (const [section, stateName] of SECTIONS) {
      const label = `oni-${section}-${stateName}-${size}`;
      await resetAll(page);
      await page.locator(`#sections [data-section="${section}"]`).click();
      await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, stateName]);
      await page.waitForTimeout(150);
      failures.push(...await checkFit(page, label));
      await page.screenshot({ path: `${out}${label}.png` });
      shots += 1;
    }
    await chooseLook('smooth');
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-im-look')) === 'smooth', `${size}: Settings did not turn ONI MODE off`);
    await page.reload();
    await page.locator('#sections [data-section="cleanup"][aria-selected="true"]').waitFor();

    await page.evaluate(() => window.__xioxMock.pause(false));
    await resetAll(page);

    // Cleanup: a category click scrolls the list and moves aria-current; a row click moves the selection.
    await page.locator('#sections [data-section="cleanup"]').click();
    await page.evaluate(() => window.__xioxMock.go('cleanup', 'analyzed'));
    await page.locator('#sidebar [data-key="nav:rules:Development & AI"]').click();
    try {
      await page.locator('#sidebar [data-key="nav:rules:Development & AI"][aria-current="true"]').waitFor({ timeout: 2000 });
      expect(await page.locator('#sidebar .xi-row[aria-current]').count() === 1, `${size}: more than one category is marked current`);
    } catch { failures.push(`${size}: the clicked category did not become current`); }
    await page.waitForTimeout(900);
    await page.locator('#rows [data-row="cat:3"] td.xi-name').click();
    await page.locator('#rows [data-row="cat:3"][aria-selected="true"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the clicked row is not selected`));
    expect(await openInspector(page, 'details'), `${size}: the Details button did not open the inspector`);
    expect((await page.locator('.im-inspector .im-facts dd').first().textContent()) === 'Memory dumps', `${size}: the details do not describe the clicked row`);
    await page.screenshot({ path: `${out}cleanup-row-selected-${size}.png` });
    shots += 1;
    await closeInspector(page);

    // The check box keeps the keyboard focus through the round trip (the view is redrawn).
    const box = page.locator('#rows [data-row="cat:4"] [role=checkbox]');
    const boxBefore = await box.getAttribute('aria-checked');
    await box.focus();
    await page.keyboard.press('Space');
    await page.waitForFunction((was) => document.querySelector('#rows [data-row="cat:4"] [role=checkbox]')?.getAttribute('aria-checked') !== was, boxBefore, { timeout: 2000 })
      .catch(() => failures.push(`${size}: Space on a focused check box did not toggle it`));
    expect(await page.evaluate(() => document.activeElement?.getAttribute('data-key')) === 'ck:cat:4', `${size}: the check box lost the keyboard focus after the redraw`);

    // Analyze: the busy state shows, then the list lands on analyzed after the model's ticks.
    await page.evaluate(() => window.__xioxMock.go('cleanup', 'idle'));
    await page.locator('#actions [data-action="analyze"]').click();
    await page.locator('#badge', { hasText: 'Analyzing' }).waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Analyze did not show the busy state`));
    await page.locator('#badge', { hasText: 'Analysis done' }).waitFor({ timeout: 4000 }).catch(() => failures.push(`${size}: Analyze did not land on analyzed`));

    // Developers: a collapsed parent hides its children until the caret opens it.
    await page.locator('#sections [data-section="developers"]').click();
    await page.evaluate(() => window.__xioxMock.go('developers', 'projects'));
    await page.locator('#rows [data-row="p:1"]').waitFor();
    expect(await page.locator('#rows [data-row="a:1"]').count() === 0, `${size}: a child shows under a collapsed parent`);
    await page.locator('#rows [data-row="p:1"] .xi-caret').click();
    await page.locator('#rows [data-row="a:1"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: the caret did not show the children`));
    failures.push(...await checkFit(page, `developers-expanded-${size}`));
    await page.screenshot({ path: `${out}developers-projects-expanded-${size}.png` });
    shots += 1;
    // A section's category changes the Developers state.
    await page.locator('#sidebar [data-key="nav:worktrees"]').click();
    await page.locator('#sidebar [data-key="nav:worktrees"][aria-current="true"]').waitFor({ timeout: 2000 }).catch(() => failures.push(`${size}: Worktrees did not become the current category`));

    // The sections end to end (Tasks 12-14): Developers, Cleanup, Registry.
    failures.push(...await sectionFlows(page, size, out));
    shots += 6;

    // Keyboard, dialogs, report, progress bar and motion (Task 3).
    failures.push(...await keyboardFlows(page, size, expect, out));
    failures.push(...await fixWaveFlows(page, size, out));
    shots += 4;
    failures.push(...await reportFlows(page, size, out));
    shots += 5;
    // Settings, the palette, the Tab audit and the profile note.
    failures.push(...await settingsFlows(page, size, out));
    failures.push(...await paletteFlows(page, size, out));
    failures.push(...await tabAudit(page, size, out));
    failures.push(...await profileFlows(page, size, out));
    shots += 5;
    failures.push(...await motionFlows(page, size));
    await page.close();
  }
  failures.push(...await narrowChecks(browser, url, out));
} finally {
  await browser.close();
  await server.close();
}
if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}
console.log(`smoke ok: ${shots} screenshots`);
