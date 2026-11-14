import { test } from 'node:test';
import assert from 'node:assert/strict';
import { findItalian } from './italian-check.mjs';

const none = new Set();

test('flags Italian string literals in Rust', () => {
  const src = 'fn a() {\n    let x = "Pulizia completata";\n    let y = "«Analizza»";\n}\n';
  const hits = findItalian(src, 'rust', none);
  assert.deepEqual(hits.map((h) => h.line), [2, 3]);
  assert.deepEqual(hits.map((h) => h.literal), ['Pulizia completata', '«Analizza»']);
});

test('ignores comments, English literals and test modules', () => {
  const src = [
    '// la pulizia della cartella',
    '/* il cestino /* annidato */ non */',
    '/// Doc: sessione archiviata',
    'fn a() { let x = "Cleanup done"; let c = \'à\'; }',
    '#[cfg(test)]',
    'mod tests {',
    '    fn t() { assert_eq!(f(), "Pulizia completata"); }',
    '}',
    'fn b() { let l: &\'static str = "Done"; }',
    '',
  ].join('\n');
  assert.deepEqual(findItalian(src, 'rust', none), []);
});

test('reads raw strings, escapes and keeps scanning after a test module', () => {
  const src = [
    'const A: &str = r"cartella";',
    'const B: &str = r#"say "non" here"#;',
    'const C: &str = "quote \\" è";',
    '#[cfg(test)]',
    'mod tests { const X: &str = "nel mezzo"; mod inner { const Y: &str = "{"; } }',
    'const D: &str = "più tardi";',
  ].join('\n');
  const hits = findItalian(src, 'rust', none);
  assert.deepEqual(hits.map((h) => h.line), [1, 2, 3, 6]);
});

test('whole words only, case-insensitive', () => {
  assert.equal(findItalian('const A: &str = "Il file";', 'rust', none).length, 1);
  assert.equal(findItalian('const A: &str = "NON riuscito";', 'rust', none).length, 1);
  assert.equal(findItalian('const A: &str = "Nonexistent, silo, folder, non-empty";', 'rust', none).length, 0);
});

test('flags XML label, description and warning text only', () => {
  const xml = [
    '<cleaner id="cargo" path="cartella">',
    '  <label>Cache di Cargo</label>',
    '  <description>Cleans the cache</description>',
    '  <warning>Attenzione: lento</warning>',
    '  <option id="x"><label>Cargo cache</label></option>',
    '</cleaner>',
  ].join('\n');
  const hits = findItalian(xml, 'xml', none);
  assert.deepEqual(hits.map((h) => [h.line, h.literal]), [
    [2, 'Cache di Cargo'],
    [4, 'Attenzione: lento'],
  ]);
});

test('flags TS and JSON string literals, skips comments', () => {
  const ts = '// la cartella\nconst a = "Cleanup";\nconst b = `Pulizia ${a}`;\nconst c = \'Annulla\';\n';
  assert.deepEqual(findItalian(ts, 'ts', none).map((h) => h.line), [3, 4]);
  assert.deepEqual(findItalian('{"label": "Cestino"}', 'json', none).map((h) => h.literal), ['Cestino']);
});

test('an allow-listed literal is ignored', () => {
  const src = 'const F: &str = "cartella da conservare";\nconst G: &str = "la cartella";\n';
  const hits = findItalian(src, 'rust', new Set(['cartella da conservare']));
  assert.deepEqual(hits.map((h) => h.literal), ['la cartella']);
});

test('the allow file skips comments and blank lines', async () => {
  const fs = await import('node:fs');
  const os = await import('node:os');
  const path = await import('node:path');
  const { loadAllow } = await import('./italian-check.mjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'italian-allow-'));
  const file = path.join(dir, 'allow.txt');
  fs.writeFileSync(file, '# a comment\n\ncookie_da_conservare.txt\r\nnon un commento\n');
  try {
    assert.deepEqual([...loadAllow(file)], ['cookie_da_conservare.txt', 'non un commento']);
    assert.deepEqual([...loadAllow(path.join(dir, 'missing.txt'))], []);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('the CLI prints path:line: "literal", exits 1 on a hit and skips tests.rs and tests/', async () => {
  const fs = await import('node:fs');
  const os = await import('node:os');
  const path = await import('node:path');
  const { spawnSync } = await import('node:child_process');
  const cli = path.join(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(\w:)/, '$1')), 'italian-check.mjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'italian-cli-'));
  fs.mkdirSync(path.join(dir, 'tests'));
  fs.writeFileSync(path.join(dir, 'tests.rs'), 'const A: &str = "Pulizia completata";\n');
  fs.writeFileSync(path.join(dir, 'tests', 'x.rs'), 'const A: &str = "Pulizia completata";\n');
  fs.writeFileSync(path.join(dir, 'ok.rs'), 'const A: &str = "Cleanup done";\n');
  try {
    let run = spawnSync(process.execPath, [cli, dir], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stdout);
    fs.writeFileSync(path.join(dir, 'bad.rs'), 'fn a() {}\nconst B: &str = "«Analizza»";\n');
    run = spawnSync(process.execPath, [cli, dir], { encoding: 'utf8' });
    assert.equal(run.status, 1);
    assert.match(run.stdout.trim(), /bad\.rs:2: "«Analizza»"$/);
    assert.equal(spawnSync(process.execPath, [cli], { encoding: 'utf8' }).status, 2);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('the CLI skips every file under examples/ and tests/, whatever its kind', async () => {
  const fs = await import('node:fs');
  const os = await import('node:os');
  const path = await import('node:path');
  const { spawnSync } = await import('node:child_process');
  const cli = path.join(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(\w:)/, '$1')), 'italian-check.mjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'italian-skip-'));
  const xml = '<cleaner><option><label>Pulizia completata</label></option></cleaner>\n';
  const rs = 'fn a() { let x = "Pulizia completata"; }\n';
  fs.mkdirSync(path.join(dir, 'examples'));
  fs.mkdirSync(path.join(dir, 'tests', 'fixtures'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'examples', 'peak.rs'), rs);
  fs.writeFileSync(path.join(dir, 'tests', 'fixtures', 'sample.xml'), xml);
  fs.writeFileSync(path.join(dir, 'tests', 'data.json'), '{"a": "Pulizia completata"}\n');
  try {
    let run = spawnSync(process.execPath, [cli, dir], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stdout);
    fs.writeFileSync(path.join(dir, 'sample.xml'), xml);
    run = spawnSync(process.execPath, [cli, dir], { encoding: 'utf8' });
    assert.equal(run.status, 1, 'the same file outside tests/ is still checked');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('knows the words that slipped through once', () => {
  const yes = ['account sconosciuto', 'Sconosciuta', 'senza permessi', 'scaricamento fallito', 'regole aggiornate', 'Aggiornato', 'impossibile aprire', 'servizi disattivati', 'riavvia il PC', 'Riavvio', 'connessione assente'];
  const no = ['unknown account', 'permissions', 'Updated', 'disabled', 'restart', 'connection lost'];
  for (const w of yes) assert.equal(findItalian(`const A: &str = "${w}";`, 'rust', none).length, 1, w);
  for (const w of no) assert.equal(findItalian(`const A: &str = "${w}";`, 'rust', none).length, 0, w);
});
