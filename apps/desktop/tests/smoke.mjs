// Smoke test: the page on Vite with the mocked commands (src/mock.ts), screenshots under
// tests/out/ (git-ignored), token / fit / error checks. Playwright is a pinned devDependency
// (1.63.0); its Chromium is the one already in %LOCALAPPDATA%\ms-playwright.
//   npm run smoke
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const out = `${here}out/`;
mkdirSync(out, { recursive: true });

export const SECTIONS = [
  ['cleanup', 'analyzed'],
  ['registry', 'analyzed'],
  ['services', 'loaded'],
  ['developers', 'projects'],
];
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
    const boxes = [...document.querySelectorAll('#actions > *')];
    const version = document.getElementById('version');
    if (version) boxes.push(version);
    const rects = boxes.map((node) => [node, node.getBoundingClientRect()]);
    for (let i = 0; i < rects.length; i += 1) {
      for (let j = i + 1; j < rects.length; j += 1) {
        const [a, ra] = rects[i];
        const [b, rb] = rects[j];
        if (ra.left < rb.right && rb.left < ra.right && ra.top < rb.bottom && rb.top < ra.bottom) {
          bad.push(`${where}: overlap ${name(a)} / ${name(b)}`);
        }
      }
    }
    return bad;
  }, label);
}

const server = await createServer({ configFile: `${here}../vite.config.ts`, server: { port: 1432, strictPort: false }, logLevel: 'error' });
await server.listen();
const url = server.resolvedUrls.local[0];
// 'chromium' = the full Chromium build in the cache (the headless shell is not installed here).
const browser = await chromium.launch({ channel: 'chromium' });
const failures = [];
let shots = 0;
try {
  for (const [width, height] of VIEWPORTS) {
    const page = await browser.newPage({ viewport: { width, height } });
    const size = `${width}x${height}`;
    page.on('pageerror', (error) => failures.push(`${size}: page error ${error}`));
    page.on('console', (message) => { if (message.type() === 'error') failures.push(`${size}: console error ${message.text()}`); });
    await page.goto(url);
    await page.locator('#sections [data-section="cleanup"].on').waitFor();
    failures.push(...await checkTokens(page));
    for (const [section, stateName] of SECTIONS) {
      await page.locator(`#sections [data-section="${section}"]`).click();
      await page.locator(`#sections [data-section="${section}"].on`).waitFor();
      await page.evaluate(([s, st]) => window.__xioxMock.go(s, st), [section, stateName]);
      await page.waitForTimeout(150);
      failures.push(...await checkFit(page, `${section}-${stateName}-${size}`));
      await page.screenshot({ path: `${out}${section}-${stateName}-${size}.png` });
      shots += 1;
    }
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
