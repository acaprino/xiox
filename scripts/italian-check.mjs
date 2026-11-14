// Finds Italian text in the user-visible literals of the workspace.
//
//   node scripts/italian-check.mjs <path>...
//
// Prints `path:line: "literal"` for every hit and exits with 1 when there is one.
// Rust: string literals outside comments, `#[cfg(test)]` items and `tests.rs`.
// Any file under an `examples/` or a `tests/` directory is skipped, whatever its kind.
// XML: the text of <label>, <description> and <warning>.  TS / JSON: string literals.
// `scripts/italian-allow.txt` lists the literals that are data, not text for the user.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ALLOW = path.join(HERE, 'italian-allow.txt');

const WORDS = (
  'il lo gli della delle degli dei di del dal dalla nel nella sono non perché già più nessun nessuna nessuno ' +
  'questa questo cartella cartelle pulizia analisi sessione sessioni elimina eliminati cestino ' +
  'attenzione consigliato facoltativo annulla annullato salva ripara servizio servizi avvio ' +
  'chiudi errore riuscito trovati'
).split(' ');

// A word is whole when no letter, digit, underscore or hyphen touches it.
const EDGE_BEFORE = '(?<![\\p{L}\\p{N}_-])';
const EDGE_AFTER = '(?![\\p{L}\\p{N}_-])';
const ITALIAN = new RegExp(
  `[àèéìòù«»]|${EDGE_BEFORE}(?:${WORDS.join('|')})${EDGE_AFTER}`,
  'iu',
);

export function isItalian(literal) {
  return ITALIAN.test(literal);
}

export function loadAllow(file = DEFAULT_ALLOW) {
  const out = new Set();
  if (!fs.existsSync(file)) return out;
  for (const raw of fs.readFileSync(file, 'utf8').split('\n')) {
    const line = raw.replace(/\r$/, '');
    if (line === '' || line.startsWith('#')) continue;
    out.add(line);
  }
  return out;
}

let defaultAllow = null;
function allowOrDefault(allow) {
  if (allow) return allow;
  defaultAllow ??= loadAllow();
  return defaultAllow;
}

class Lines {
  constructor(text) {
    this.starts = [0];
    for (let i = 0; i < text.length; i++) if (text[i] === '\n') this.starts.push(i + 1);
  }
  at(index) {
    let lo = 0;
    let hi = this.starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.starts[mid] <= index) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  }
}

const isIdent = (c) => c !== undefined && /[\p{L}\p{N}_]/u.test(c);

// Calls `emit(literal, startIndex)` for every string literal that is not suppressed.
function scanRust(text, emit) {
  const n = text.length;
  // 0 = reporting, 1 = a cfg(test) item starts here, 2 = inside it, 3 = the whole file
  let mode = 0;
  let depth = 0;
  let i = 0;
  const report = (lit, at) => {
    if (mode === 0) emit(lit, at);
  };
  while (i < n) {
    const c = text[i];
    const d = text[i + 1];
    if (c === '/' && d === '/') {
      while (i < n && text[i] !== '\n') i++;
    } else if (c === '/' && d === '*') {
      let level = 1;
      i += 2;
      while (i < n && level > 0) {
        if (text[i] === '/' && text[i + 1] === '*') {
          level++;
          i += 2;
        } else if (text[i] === '*' && text[i + 1] === '/') {
          level--;
          i += 2;
        } else {
          i++;
        }
      }
    } else if (c === '#') {
      const m = /^#(!?)\[\s*cfg\s*\(\s*test\s*\)\s*\]/.exec(text.slice(i, i + 64));
      if (m) {
        if (mode === 0) mode = m[1] ? 3 : 1;
        i += m[0].length;
      } else {
        i++;
      }
    } else if (
      (c === 'r' || ((c === 'b' || c === 'c') && d === 'r')) &&
      !isIdent(text[i - 1]) &&
      /^(?:br|cr|r)#*"/.test(text.slice(i, i + 12))
    ) {
      const m = /^(?:br|cr|r)(#*)"/.exec(text.slice(i, i + 12));
      const close = '"' + m[1];
      const start = i + m[0].length;
      const end = text.indexOf(close, start);
      const stop = end === -1 ? n : end;
      report(text.slice(start, stop), start);
      i = stop + close.length;
    } else if (c === '"') {
      let j = i + 1;
      while (j < n && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      report(text.slice(i + 1, j), i + 1);
      i = j + 1;
    } else if (c === "'") {
      if (d === '\\') {
        let j = i + 2;
        while (j < n && text[j] !== "'") j++;
        i = j + 1;
      } else if (text[i + 2] === "'") {
        i += 3;
      } else if (d >= '\ud800' && d <= '\udbff' && text[i + 3] === "'") {
        i += 4;
      } else {
        i++; // a lifetime
      }
    } else if (c === '{') {
      if (mode === 1) {
        mode = 2;
        depth = 1;
      } else if (mode === 2) {
        depth++;
      }
      i++;
    } else if (c === '}') {
      if (mode === 2 && --depth === 0) mode = 0;
      i++;
    } else if (c === ';') {
      if (mode === 1) mode = 0;
      i++;
    } else {
      i++;
    }
  }
}

const REGEX_AFTER = new Set(['', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '<', '>', '~', '^']);

function scanScript(text, emit) {
  const n = text.length;
  let i = 0;
  let prev = '';
  const readString = (quote) => {
    let j = i + 1;
    while (j < n && text[j] !== quote && text[j] !== '\n') j += text[j] === '\\' ? 2 : 1;
    emit(text.slice(i + 1, j), i + 1);
    i = j + 1;
  };
  const readTemplate = () => {
    let j = i + 1;
    let braces = 0;
    while (j < n) {
      const ch = text[j];
      if (ch === '\\') j += 2;
      else if (ch === '$' && text[j + 1] === '{') {
        braces++;
        j += 2;
      } else if (ch === '}' && braces > 0) {
        braces--;
        j++;
      } else if (ch === '`' && braces === 0) break;
      else j++;
    }
    emit(text.slice(i + 1, j), i + 1);
    i = j + 1;
  };
  while (i < n) {
    const c = text[i];
    const d = text[i + 1];
    if (c === '/' && d === '/') {
      while (i < n && text[i] !== '\n') i++;
    } else if (c === '/' && d === '*') {
      const end = text.indexOf('*/', i + 2);
      i = end === -1 ? n : end + 2;
    } else if (c === '"' || c === "'") {
      readString(c);
      prev = c;
    } else if (c === '`') {
      readTemplate();
      prev = c;
    } else if (c === '/' && REGEX_AFTER.has(prev)) {
      let j = i + 1;
      let inClass = false;
      while (j < n && text[j] !== '\n' && (inClass || text[j] !== '/')) {
        if (text[j] === '\\') j++;
        else if (text[j] === '[') inClass = true;
        else if (text[j] === ']') inClass = false;
        j++;
      }
      i = j + 1;
      prev = '/';
    } else {
      if (!/\s/.test(c)) prev = c;
      i++;
    }
  }
}

function scanXml(text, emit) {
  const blank = text.replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, ' '));
  const re = /<(label|description|warning)\b[^>]*>([\s\S]*?)<\/\1\s*>/g;
  let m;
  while ((m = re.exec(blank))) {
    const body = m[2];
    const lead = body.length - body.trimStart().length;
    const literal = body.trim();
    const bodyStart = m.index + m[0].length - body.length - m[1].length - 3;
    if (literal) emit(literal, bodyStart + lead);
  }
}

/** @returns {{line: number, literal: string}[]} */
export function findItalian(text, kind, allow) {
  const allowed = allowOrDefault(allow);
  const lines = new Lines(text);
  const hits = [];
  const emit = (literal, at) => {
    if (allowed.has(literal) || !isItalian(literal)) return;
    hits.push({ line: lines.at(at), literal });
  };
  if (kind === 'rust') scanRust(text, emit);
  else if (kind === 'xml') scanXml(text, emit);
  else if (kind === 'ts' || kind === 'json') scanScript(text, emit);
  else throw new Error(`unknown kind: ${kind}`);
  return hits;
}

const KINDS = { '.rs': 'rust', '.xml': 'xml', '.ts': 'ts', '.tsx': 'ts', '.mjs': 'ts', '.js': 'ts', '.json': 'json' };
const SKIP_DIRS = new Set(['node_modules', 'target', 'dist', '.git']);

function* walk(target) {
  const stat = fs.statSync(target);
  if (!stat.isDirectory()) {
    yield target;
    return;
  }
  for (const entry of fs.readdirSync(target, { withFileTypes: true })) {
    if (entry.isDirectory() && SKIP_DIRS.has(entry.name)) continue;
    yield* walk(path.join(target, entry.name));
  }
}

// Developer examples, tests and their fixtures are not text for the user.
function skipped(file, kind) {
  const parts = file.split(/[\\/]/);
  const dirs = parts.slice(0, -1);
  if (dirs.includes('examples') || dirs.includes('tests')) return true;
  return kind === 'rust' && parts.at(-1) === 'tests.rs';
}

function main(args) {
  if (args.length === 0) {
    console.error('usage: node scripts/italian-check.mjs <path>...');
    return 2;
  }
  let found = 0;
  for (const arg of args) {
    for (const file of walk(arg)) {
      const kind = KINDS[path.extname(file)];
      if (!kind || skipped(file, kind)) continue;
      for (const hit of findItalian(fs.readFileSync(file, 'utf8'), kind)) {
        console.log(`${file.replaceAll('\\', '/')}:${hit.line}: ${JSON.stringify(hit.literal)}`);
        found++;
      }
    }
  }
  return found > 0 ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = main(process.argv.slice(2));
}
