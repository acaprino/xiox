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
    overlaps(top);
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
    if (row.muted && !row.enabled && row.why) {
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
      await page.locator(`#sections [data-section="${section}"]`).click();
      await page.locator(`#sections [data-section="${section}"][aria-current="page"]`).waitFor();
      await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, stateName]);
      await page.waitForTimeout(150);
      failures.push(...await checkFit(page, label));
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
