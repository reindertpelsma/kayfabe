import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DEFAULT_IGNORE, ignored, parseIgnore, patternsFor, treeKeyOf } from '../src/treekey.js';
// Parity of the Worker's matcher with git on this repository's real paths: the reference
// (treekey_ref.py) uses `git ls-files -s` and `git check-ignore --no-index`.
// FALSIFIER: any path on which the two disagree, or a different key, for the built-in default,
// the checkout's own .workerignore (when present) and a pattern set using every rule.
const broker = fileURLToPath(new URL('../', import.meta.url));
const root = fileURLToPath(new URL('../../../', import.meta.url));
const git = spawnSync('git', ['-C', root, 'ls-files', '-s', '-z'], { maxBuffer: 1 << 28 });
const skip =
  git.status !== 0
    ? 'not a git checkout'
    : spawnSync('python3', ['--version']).status !== 0
      ? 'python3 missing'
      : false;

function entries() {
  const out = [];
  for (const record of git.stdout.toString('utf8').split('\0')) {
    if (!record) continue;
    const tab = record.indexOf('\t');
    const [mode, sha, stage] = record.slice(0, tab).split(' ');
    assert.equal(stage, '0');
    out.push({ path: record.slice(tab + 1), mode, sha });
  }
  return out;
}

const RICH = [
  '# every rule of the subset',
  '*.md',
  'docs/',
  '/traces/**/*.json',
  '**/fixtures/',
  '/crates/kf-*/tests/',
  'scripts/ci/*_mutants.py',
  '?ARGO.lock',
  '/third_party/ogkm/',
  'LICENSE*',
  '.github/',
  '/archive/**',
  'ci/**/test',
  '',
].join('\n');

function reference(patterns) {
  const dir = mkdtempSync(join(tmpdir(), 'kf-treekey-parity-'));
  try {
    const file = join(dir, 'patterns');
    writeFileSync(file, patterns);
    const run = spawnSync(
      'python3',
      [join(broker, 'treekey_ref.py'), '--patterns', file, '--repo', root, '--list'],
      { maxBuffer: 1 << 28, encoding: 'utf8' },
    );
    assert.equal(run.status, 0, run.stderr);
    const lines = run.stdout.trimEnd().split('\n');
    const key = lines.pop();
    const states = new Map(lines.map((l) => [l.slice(l.indexOf('\t') + 1), l.split('\t')[0]]));
    return { key, states };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function parity(name, text) {
  const patterns = parseIgnore(text);
  assert.ok(patterns, name + ': inside the subset');
  const list = entries();
  const ref = reference(text);
  assert.equal(ref.states.size, list.length, name + ': same listing');
  let disagreements = 0,
    ignoredCount = 0;
  for (const e of list) {
    const mine = ignored(patterns, e.path, e.mode === '160000');
    if (mine) ignoredCount++;
    if (mine !== (ref.states.get(e.path) === 'ignored')) {
      disagreements++;
      if (disagreements <= 5) assert.fail(name + ': ' + e.path + ' js=' + mine);
    }
  }
  assert.equal(disagreements, 0);
  assert.equal(await treeKeyOf(list, patterns), ref.key, name + ': key');
  return ignoredCount;
}

test('treekey parity: built-in default list', { skip }, async () => {
  const n = await parity('default', DEFAULT_IGNORE.join('\n') + '\n');
  assert.ok(n > 0);
});

test('treekey parity: every rule of the subset', { skip }, async () => {
  const n = await parity('rich', RICH);
  assert.ok(n > 0);
});

const own = join(root, '.workerignore');
test(
  "treekey parity: the checkout's own .workerignore",
  { skip: skip || (existsSync(own) ? false : 'no .workerignore in this checkout') },
  async () => {
    await parity('.workerignore', readFileSync(own, 'utf8'));
  },
);

// Review of PR #269, R1 (2026-11-05): the committed .workerignore says that it is read by this
// Worker and that it yields no tree key today (it covers the PROTECTED root ci/vast-broker). If the
// file changes so that a key is produced, this fails: update the header (and decide on carry-over).
test(
  "treekey: the checkout's own .workerignore yields no tree key, as its header says",
  { skip: existsSync(own) ? false : 'no .workerignore in this checkout' },
  () => {
    const text = readFileSync(own, 'utf8');
    assert.ok(parseIgnore(text), 'inside the subset');
    assert.equal(patternsFor(text), null, 'refused: covers a protected code root');
    assert.ok(ignored(parseIgnore(text), 'ci/vast-broker', true));
    assert.match(text, /produces NO tree key today/);
    assert.doesNotMatch(text, /Not read by anything/);
  },
);
