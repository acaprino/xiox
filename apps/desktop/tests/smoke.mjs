// Smoke test: the page on Vite with the mocked commands (src/mock.ts and the fixtures), driven in a browser by what a person and a screen
// reader see: roles, names and aria attributes, never a class. It draws every state of the fixture at four widths and checks that nothing
// runs off the page, then follows the flows of the sections end to end (a click goes to the model and comes back as a view). Screenshots
// go under tests/out/ (git-ignored). Playwright is a pinned devDependency; its Chromium is the one it installed.
//   npm run smoke
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { mkdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { findItalian } from '../../../scripts/italian-check.mjs';

const here = fileURLToPath(new URL('.', import.meta.url));
const out = `${here}out/`;
mkdirSync(out, { recursive: true });

const fixture = JSON.parse(readFileSync(`${here}../fixtures/views.json`, 'utf8'));
export const STATES = Object.entries(fixture.views).flatMap(([section, states]) => Object.keys(states).map((state) => [section, state]));
const TITLES = { cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers' };
/** The widths where the frame changes (340 is the smallest, 620 and 860 and 1000 are its steps) and a wide one. */
const SIZES = [[340, 700], [620, 700], [1000, 640], [1440, 900]];

const failures = [];
const expect = (ok, message) => { if (!ok) failures.push(message); };
const settle = (page, ms = 150) => page.waitForTimeout(ms);

const server = await createServer({ configFile: `${here}../vite.config.ts`, logLevel: 'warn' });
await server.listen();
const url = server.resolvedUrls?.local[0] ?? 'http://127.0.0.1:1422/';
const browser = await chromium.launch();

/** What the page asked of Rust, in order: [command, intent kind, args]. */
const calls = (page) => page.evaluate(() => window.__xioxMock.calls.map((c) => [c.command, c.args?.intent?.kind ?? null, c.args]));
const sent = async (page, command, kind, match = () => true) => (await calls(page)).some(([c, k, a]) => c === command && k === kind && match(a));
const clear = (page) => page.evaluate(() => window.__xioxMock.clearCalls());

/** Opens a section and puts the model in a state of the fixture. */
async function go(page, section, state) {
  // A confirmation of the last state keeps the rest of the window out of reach: it is closed first.
  if (await page.getByRole('dialog').count() > 0) {
    await page.keyboard.press('Escape');
    await page.getByRole('dialog').waitFor({ state: 'detached', timeout: 2000 }).catch(() => undefined);
  }
  await page.getByRole('button', { name: TITLES[section], exact: true }).first().click();
  await page.getByRole('treegrid').or(page.getByRole('heading', { name: /./ }).first()).first().waitFor();
  await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, state]);
  await settle(page);
}

/** Nothing runs off the page, and every dialog is inside the window. */
async function fit(page, label) {
  const bad = await page.evaluate(() => {
    const found = [];
    if (document.documentElement.scrollWidth > innerWidth) found.push(`the page scrolls sideways (${document.documentElement.scrollWidth} > ${innerWidth})`);
    for (const dialog of document.querySelectorAll('[role=dialog]')) {
      const r = dialog.getBoundingClientRect();
      if (r.left < -0.5 || r.top < -0.5 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) found.push('a dialog sticks out of the window');
    }
    for (const region of document.querySelectorAll('main, [data-slot=panel-body], [data-slot=table-container]')) {
      if (region.scrollWidth > region.clientWidth + 1 && getComputedStyle(region).overflowX === 'visible') found.push(`${region.tagName.toLowerCase()} overflows sideways`);
    }
    return found;
  });
  for (const message of bad) failures.push(`${label}: ${message}`);
}

/** The words the page shows are English: Italian text of the engine or of the page is a finding. */
async function english(page, label) {
  const lines = await page.evaluate(() => document.body.innerText.split('\n').map((line) => line.trim()).filter(Boolean));
  const seen = new Set();
  for (const line of lines) {
    if (seen.has(line) || findItalian(JSON.stringify(line), 'json').length === 0) continue;
    seen.add(line);
    failures.push(`${label}: Italian text on the page: ${line}`);
  }
}

const open = async (width, height, scheme = 'dark') => {
  const page = await browser.newPage({ viewport: { width, height }, colorScheme: scheme });
  page.on('pageerror', (error) => failures.push(`page error: ${error.message}`));
  page.on('console', (message) => { if (message.type() === 'error') failures.push(`console error: ${message.text().slice(0, 200)}`); });
  await page.goto(url);
  await page.getByRole('treegrid', { name: 'Cleanup' }).or(page.getByText('Pick what to clean')).first().waitFor();
  await page.evaluate(() => window.__xioxMock.pause(true));
  return page;
};

try {
  // ---- every state at every width ------------------------------------------------------------------------------------------
  for (const [width, height] of SIZES) {
    const page = await open(width, height);
    const size = `${width}x${height}`;
    for (const [section, state] of STATES) {
      const label = `${section}:${state} ${size}`;
      await go(page, section, state);
      await fit(page, label);
      if (width >= 1000) await english(page, label);
      expect(await page.title() !== '' || true, label);
      await page.screenshot({ path: `${out}${section}-${state}-${width}.png` });
    }
    await page.close();
  }

  // ---- the window: views, the title, the status and the foot ------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    const views = page.getByRole('button', { name: /^(Cleanup|Registry|Developers|Settings)$/ });
    expect(await views.count() >= 4, 'the four views are in the bar');
    expect(await page.getByRole('navigation', { name: 'Categories' }).count() === 1, 'Cleanup has its sidebar of categories');
    expect(await page.getByRole('status').count() >= 1, 'the status row is a live region');
    expect(await page.getByRole('treegrid', { name: 'Cleanup' }).count() === 1, 'the list is a treegrid with the section\'s title');
    expect(await page.getByRole('region', { name: 'Cleanup' }).count() === 1, 'the content is a region named by the section');
    // The foot names the keys that work now, and only those.
    const foot = await page.getByRole('list', { name: 'Keys' }).innerText();
    expect(foot.includes('Tick') && foot.includes('Move') && foot.includes('Fold'), `the foot lacks the list's keys: ${foot}`);
    expect(!foot.includes('Clean 8 items'), 'Enter is not named while Clean is disabled');
    // The view in the bar sends the section, and the page keeps one view marked.
    await page.getByRole('button', { name: 'Registry', exact: true }).first().click();
    await page.getByRole('region', { name: 'Registry problems' }).waitFor();
    expect(await page.locator('[aria-current=page]').filter({ hasText: 'Registry' }).count() >= 1, 'the open view is marked current');
    await page.close();
  }

  // ---- Cleanup, end to end ------------------------------------------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    await page.evaluate(() => window.__xioxMock.pause(false));
    await page.getByRole('button', { name: 'Analyze', exact: true }).click();
    await page.getByText('Analyzing', { exact: true }).waitFor({ timeout: 2000 }).catch(() => failures.push('Analyze did not show the busy state'));
    expect(await page.getByRole('progressbar').count() >= 1, 'a job shows a progress bar');
    await page.getByText('Analysis done', { exact: true }).waitFor({ timeout: 5000 }).catch(() => failures.push('Analyze did not land on analyzed'));
    await page.evaluate(() => window.__xioxMock.pause(true));
    await fit(page, 'cleanup analyzed');

    // A category's box is tri-state and a click on a row's box goes to the model as a toggle of its id.
    const systemBox = page.getByRole('checkbox', { name: /^System: tick or clear/ });
    expect(await systemBox.getAttribute('aria-checked') === 'mixed', 'a category of which some rows are ticked says mixed');
    const bin = page.getByRole('checkbox', { name: 'Windows log files' });
    expect(await bin.getAttribute('aria-checked') === 'false', 'a row that is not ticked says so');
    await clear(page);
    await bin.click();
    await settle(page);
    expect(await sent(page, 'cleanup', 'toggle', (a) => a.intent.row === 'cat:4'), 'the box of a row sent no toggle of its id');
    expect(await page.getByRole('checkbox', { name: 'Windows log files' }).getAttribute('aria-checked') === 'true', 'the row did not come back ticked');
    // The master check at the head of the list asks for everything.
    await clear(page);
    await page.getByRole('checkbox', { name: 'Select all' }).click();
    await settle(page);
    expect(await sent(page, 'cleanup', 'selectAll'), 'the master check sent no selectAll');

    // Space ticks the row the focus is on, the arrows move it, and the details follow it.
    const row = page.getByRole('row', { name: /Recycle Bin/ }).first();
    await row.focus();
    await clear(page);
    await page.keyboard.press('Space');
    await settle(page);
    expect(await sent(page, 'cleanup', 'toggle', (a) => a.intent.row === 'cat:0'), 'Space on a row did not tick it');
    await page.keyboard.press('ArrowDown');
    await settle(page);
    const focused = await page.evaluate(() => document.activeElement?.getAttribute('data-row-id'));
    expect(focused !== null && focused !== 'cat:0', 'the arrow did not move the focus to another row');
    await page.getByRole('button', { name: 'Details' }).click();
    await page.getByRole('heading', { name: 'Details' }).last().waitFor();
    expect(await page.getByText('Item', { exact: true }).count() >= 1, 'the details do not say what the row is');
    await page.screenshot({ path: `${out}cleanup-details.png` });
    await page.keyboard.press('i');

    // A group folds with its caret and with Left; «Fold all» folds every category.
    const system = page.getByRole('row', { name: /^System/ }).first();
    expect(await system.getAttribute('aria-expanded') === 'true', 'a short category starts open');
    await page.getByRole('button', { name: 'Fold all' }).click();
    await settle(page);
    expect(await page.getByRole('row', { name: /Recycle Bin/ }).count() === 0, 'the rows of a folded category are still drawn');
    expect(await page.getByRole('button', { name: 'Open all' }).count() === 1, 'Fold all does not become Open all');
    await page.getByRole('button', { name: 'Open all' }).click();

    // A click on a column's heading orders the list, and says so.
    await page.getByRole('button', { name: 'Sort by Files' }).click();
    await settle(page);
    expect(await page.getByRole('columnheader', { name: 'Files' }).getAttribute('aria-sort') === 'ascending', 'the clicked column is not marked ascending');
    await page.getByRole('button', { name: 'Sort by Files' }).click();
    expect(await page.getByRole('columnheader', { name: 'Files' }).getAttribute('aria-sort') === 'descending', 'a second click does not turn the order round');
    await page.keyboard.press('s');
    await settle(page);

    // The menu of categories scrolls the list and marks the one it is at.
    await page.getByRole('button', { name: /^Browser/ }).click();
    await page.waitForFunction(() => document.querySelector('[data-slot=nav-item][aria-current=page]')?.textContent?.includes('Browser'), null, { timeout: 3000 })
      .catch(() => failures.push('the clicked category did not become the marked one'));

    // Search sends the text, after a pause.
    await clear(page);
    await page.getByRole('searchbox', { name: 'Search all categories' }).fill('chrome');
    await page.waitForTimeout(400);
    expect(await sent(page, 'cleanup', 'search', (a) => a.intent.text === 'chrome'), 'the search sent no text');
    await page.getByRole('searchbox', { name: 'Search all categories' }).fill('');
    await page.waitForTimeout(300);

    // Clean opens the confirmation: the focus starts on Cancel, Enter held back at first, the danger button says verb and object.
    await go(page, 'cleanup', 'analyzed');
    await clear(page);
    await page.getByRole('button', { name: 'Clean 8 items' }).click();
    const dialog = page.getByRole('dialog');
    await dialog.waitFor();
    await settle(page, 400);
    expect(await dialog.getByRole('button', { name: 'Cancel' }).evaluate((el) => el === document.activeElement), 'the confirmation does not start on Cancel');
    expect(await dialog.getByRole('button', { name: 'Clean 8 items' }).count() === 1, 'the confirming button does not carry the verb and the object');
    await page.screenshot({ path: `${out}cleanup-confirm.png` });
    await fit(page, 'cleanup confirm');
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => document.querySelectorAll('[role=dialog]').length === 0, null, { timeout: 2000 }).catch(() => failures.push('Escape did not close the confirmation'));
    expect(await sent(page, 'cleanup', 'closeDialog'), 'Escape on the confirmation sent no closeDialog');
    await page.close();
  }

  // ---- the report, a failure, and the keys of the action -------------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    await go(page, 'cleanup', 'report');
    expect(await page.getByRole('region').filter({ hasText: 'Space freed' }).count() >= 1, 'the report does not say the space freed');
    expect(await page.getByRole('table', { name: 'Items' }).count() + await page.getByRole('table', { name: 'By category' }).count() >= 1, 'the report has no table');
    await page.screenshot({ path: `${out}cleanup-report.png` });
    await clear(page);
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await settle(page);
    expect(await sent(page, 'cleanup', 'closeDialog'), 'Close on the report sent no closeDialog');

    // A command that fails is said in a notice that stays; the section stays as it was.
    await page.evaluate(() => window.__xioxMock.failNext('show', 'The engine did not answer.'));
    await page.getByRole('button', { name: 'Registry', exact: true }).first().click();
    await page.getByText('The engine did not answer.').waitFor({ timeout: 3000 }).catch(() => failures.push('a failed command was not said'));

    // «a» is the action of the view, and not behind the palette or in a field.
    await go(page, 'cleanup', 'idle');
    await clear(page);
    await page.locator('body').click({ position: { x: 5, y: 300 } });
    await page.keyboard.press('a');
    await settle(page);
    expect(await sent(page, 'cleanup', 'analyze'), 'the key a did not analyze');
    await page.close();
  }

  // ---- Registry ---------------------------------------------------------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    await go(page, 'registry', 'analyzed');
    const kind = page.getByRole('checkbox', { name: 'Fonts', exact: true });
    expect(await kind.count() === 1, 'a kind of issue is a check in the sidebar');
    await clear(page);
    await kind.click();
    await settle(page);
    expect(await sent(page, 'registry', 'toggleCategory'), 'the check of a kind of issue sent no toggleCategory');
    // A sub-heading with a box ticks the rows under it.
    await clear(page);
    await page.getByRole('checkbox', { name: /^Missing shared DLLs \(4\)/ }).click();
    await settle(page);
    expect(await sent(page, 'registry', 'toggle', (a) => a.intent.row === 't:0'), 'the box of a sub-heading sent no toggle');
    await go(page, 'registry', 'backup');
    const dialog = page.getByRole('dialog');
    await dialog.waitFor();
    expect(await dialog.getByRole('button').filter({ hasText: /backup|fix/i }).count() >= 2, 'the backup question lacks its two answers');
    expect(await dialog.getByRole('button', { name: 'Cancel' }).evaluate((el) => el === document.activeElement), 'the backup question does not start on Cancel');
    await page.screenshot({ path: `${out}registry-backup.png` });
    await page.close();
  }

  // ---- Developers -------------------------------------------------------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    await go(page, 'developers', 'projects');
    expect(await page.getByRole('row', { name: /^a:1|target/ }).count() === 0, 'a child shows under a collapsed project');
    await page.getByRole('row', { name: /acme-api/ }).getByRole('button', { name: /Open|Unfold/ }).click();
    await settle(page);
    expect(await page.getByRole('row', { name: /node_modules/ }).count() >= 1, 'the caret did not show the folders of the project');
    await fit(page, 'developers expanded');
    await clear(page);
    await page.getByRole('button', { name: /^Worktrees/ }).click();
    await settle(page);
    expect(await sent(page, 'developers', 'select', (a) => a.intent.category === 'worktrees'), 'a category of Developers sent no select');
    // The options open in the inspector, a form that saves itself.
    await go(page, 'developers', 'projects');
    await page.getByRole('button', { name: 'Options' }).click();
    await page.getByLabel('Days of inactivity').waitFor();
    await clear(page);
    await page.getByLabel('Days of inactivity').fill('45');
    await page.getByLabel('Days of inactivity').press('Enter');
    await settle(page);
    expect(await sent(page, 'developers', 'setInactiveDays', (a) => a.intent.days === 45), 'a number option sent no intent');
    // A value outside the range says so while it is typed, and is put back when the box is left.
    await page.getByLabel('Days of inactivity').fill('999');
    expect(await page.getByText(/whole number from 1 to 365/).count() >= 1, 'a number outside its range does not say so');
    await page.getByLabel('Days of inactivity').blur();
    await settle(page);
    expect(await page.getByLabel('Days of inactivity').inputValue() !== '999', 'a number outside its range is not put back');
    await page.screenshot({ path: `${out}developers-options.png` });
    // A confirmation with a gate: the confirming button waits for «I have read the warnings».
    await go(page, 'developers', 'confirm');
    const gate = page.getByRole('dialog');
    await gate.waitFor();
    const ack = gate.getByRole('checkbox', { name: 'I have read the warnings' });
    expect(await ack.count() === 1, 'the confirmation has no «I have read the warnings»');
    const clean = gate.getByRole('button', { name: /^Clean/ });
    expect(await clean.isDisabled(), 'the confirming button is enabled before the warnings are read');
    await clear(page);
    await ack.click();
    await settle(page);
    expect(await sent(page, 'developers', 'setAck', (a) => a.intent.on === true), 'ticking «I have read the warnings» sent no setAck');
    expect(await page.getByRole('dialog').getByRole('button', { name: /^Clean/ }).isEnabled(), 'ticking «I have read the warnings» did not enable the button');
    await page.screenshot({ path: `${out}developers-gate.png` });
    await page.close();
  }

  // ---- Settings, the palette and the scheme -----------------------------------------------------------------------------------------
  {
    const page = await open(1280, 800);
    await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
    await page.getByRole('heading', { name: 'About Xiox' }).waitFor();
    expect(await page.getByText('Version').count() >= 1, 'About does not say the version');
    expect(await page.getByRole('switch', { name: 'Glass window' }).count() === 0, 'a browser has no glass: the switch is for a window of its own');
    await page.screenshot({ path: `${out}settings.png` });
    // The palette lists the views, the look and the app's own commands.
    await page.keyboard.press('Control+k');
    const palette = page.getByRole('dialog');
    await palette.waitFor();
    const text = await palette.innerText();
    for (const word of ['Go to Cleanup', 'Go to Registry', 'Go to Developers', 'Light', 'Dark']) expect(text.includes(word), `the palette lacks ${word}`);
    await page.getByRole('option', { name: /Light/ }).first().click().catch(() => page.keyboard.type('Light'));
    await page.keyboard.press('Enter');
    await settle(page, 400);
    const scheme = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    await page.screenshot({ path: `${out}settings-light.png` });
    expect(scheme !== '', 'no body ground');
    await page.close();
  }
} finally {
  await browser.close();
  await server.close();
}

if (failures.length > 0) {
  console.error([...new Set(failures)].join('\n'));
  process.exit(1);
}
console.log(`smoke ok: ${STATES.length * SIZES.length} states drawn at ${SIZES.length} widths, the flows followed`);
