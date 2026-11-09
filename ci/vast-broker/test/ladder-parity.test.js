import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { LIMITS, MatchBudget, PROTECTED, ignored, parseIgnore, treeKeyOf } from '../src/treekey.js';
import { skippedPath } from '../trigger-policy.mjs';
// The strip step (scripts/ci/ignore_ladder.py, `matches`) is the ONE matcher the box and the
// stripped-tree CI job use; the Worker's treekey.js reads the same file to key a commit. If the two
// ever disagree on one path, the box would test a different tree than the key names.
// FALSIFIER: a path on which `matches` and treekey.js `ignored` disagree for a pattern set both
// accept; a pattern set the strict side accepts and the Worker reads differently; a different tree
// key for the same entries; a PROTECTED root the floor neither covers nor lists; a path the gate
// skips today that .workerignore would not strip.
const root = fileURLToPath(new URL('../../../', import.meta.url));
const py = (args, input) =>
  spawnSync('python3', [root + 'scripts/ci/ignore_ladder.py', ...args], {
    input: input === undefined ? undefined : JSON.stringify(input),
    encoding: 'utf8',
    maxBuffer: 1 << 28,
    cwd: root,
  });
const have =
  spawnSync('python3', ['--version']).status === 0 && spawnSync('git', ['--version']).status === 0;
const tracked = have
  ? spawnSync('git', ['-C', root, 'ls-files', '-s', '-z'], { encoding: 'utf8', maxBuffer: 1 << 28 })
      .stdout.split('\0')
      .filter(Boolean)
      .map((r) => {
        const tab = r.indexOf('\t');
        const [mode, sha] = r.slice(0, tab).split(' ');
        return { path: r.slice(tab + 1), mode, sha };
      })
  : [];
const skip = have && tracked.length > 1000 ? false : 'needs a full git checkout';
const lines = (text) => text.split('\n').filter((l) => l && !l.startsWith('#'));

function both(patternLines, entries) {
  const run = py(['matchjson'], {
    patterns: patternLines,
    paths: entries.map((e) => e.path),
    entries: entries.map((e) => [e.mode, e.sha, e.path]),
  });
  assert.equal(run.status, 0, run.stderr + run.stdout);
  const mine = JSON.parse(run.stdout);
  const theirs = parseIgnore(patternLines.join('\n') + '\n');
  return { mine, theirs };
}

test(
  'the real ignore files: matches and treekey.js ignore exactly the same tracked paths',
  { skip },
  () => {
    for (const name of ['.workerignore', '.dockerignore', '.gitignore']) {
      const patternLines = lines(readFileSync(root + name, 'utf8'));
      const { mine, theirs } = both(patternLines, tracked);
      assert.ok(theirs, name + ' is outside the treekey.js subset');
      const diff = tracked
        .map((e, i) => [e, i])
        .filter(([e, i]) => mine.ignored[i] !== ignored(theirs, e.path, e.mode === '160000'));
      assert.deepEqual(
        diff.map(([e]) => e.path).slice(0, 5),
        [],
        name + ': matcher disagreement on ' + diff.length + ' paths',
      );
    }
  },
);

test(
  'the tree key is the same bytes in Python and in treekey.js (real tree, real ignore file)',
  { skip },
  async () => {
    const patternLines = lines(readFileSync(root + '.workerignore', 'utf8'));
    const { mine, theirs } = both(patternLines, tracked);
    assert.equal(mine.key, await treeKeyOf(tracked, theirs));
  },
);

test(
  'the Worker can still compute a key for .workerignore within 60 percent of its matcher budget',
  { skip },
  async () => {
    // Mirrors computeTreeKey: a wholly ignored top-level directory is never listed, every other
    // directory is. traces/ is partly kept, so its files are listed and every one of them is matched.
    // The matcher stops at the first matching line, so the order of the lines is part of the cost.
    const patterns = parseIgnore(readFileSync(root + '.workerignore', 'utf8'));
    const listed = tracked.filter(
      (e) => !e.path.includes('/') || !ignored(patterns, e.path.split('/')[0], true),
    );
    const budget = new MatchBudget();
    const key = await treeKeyOf(listed, patterns, budget);
    const used = LIMITS.steps - budget.left;
    assert.ok(key, 'no key: the matcher budget ran out (' + used + ' steps)');
    assert.ok(used < LIMITS.steps * 0.6, 'uses ' + used + ' of ' + LIMITS.steps + ' steps');
    assert.ok(patterns.length < LIMITS.patterns * 0.5, patterns.length + ' lines');
  },
);

// ---- adversarial: seeded random pattern sets inside the Python subset ---------------------------
function rng(seed) {
  let s = seed >>> 0;
  return () => (s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32;
}
const SEGS = ['a', 'b', 'ab', 'aab', 'x.md', 'docs', 'test_a.py', 'a.b', 'c+d', 'abc'];
const GLOBS = ['*', 'a*', '*b', '*.md', 'test_*.py', 'a*b', '*a*', '**', 'docs', 'a', 'ab', 'x.md'];

function randomPattern(r) {
  const anchored = r() < 0.7;
  const n = 1 + Math.floor(r() * 3);
  const parts = [];
  for (let i = 0; i < n; i++) {
    const last = i === n - 1;
    let g = GLOBS[Math.floor(r() * GLOBS.length)];
    if (g === '**' && (last || (!anchored && i > 0 && false))) g = 'a*';
    parts.push(g);
  }
  return (anchored ? '/' : '**/') + parts.join('/');
}
function randomPath(r) {
  const n = 1 + Math.floor(r() * 4);
  return Array.from({ length: n }, () => SEGS[Math.floor(r() * SEGS.length)]).join('/');
}

test(
  'adversarial: 400 seeded pattern sets x 60 paths, matches == treekey.js ignored',
  { skip },
  async () => {
    const r = rng(20261109);
    let compared = 0;
    let refusedByWorker = 0;
    for (let round = 0; round < 400; round++) {
      const set = Array.from({ length: 1 + Math.floor(r() * 4) }, () => randomPattern(r));
      const entries = Array.from({ length: 60 }, (_, i) => ({
        path: randomPath(r),
        mode: '100644',
        sha: (i + 1).toString(16).padStart(40, '0'),
      }));
      const uniq = [...new Map(entries.map((e) => [e.path, e])).values()];
      const run = py(['matchjson'], {
        patterns: set,
        paths: uniq.map((e) => e.path),
        entries: uniq.map((e) => [e.mode, e.sha, e.path]),
      });
      if (run.status === 3) {
        // The strict side refuses the set: nothing is stripped. That is the safe direction.
        continue;
      }
      assert.equal(run.status, 0, set.join(' ') + run.stderr);
      const mine = JSON.parse(run.stdout);
      const theirs = parseIgnore(set.join('\n') + '\n');
      if (theirs === null) {
        // The Worker's own limits are stricter than the Python syntax in one way: at most
        // LIMITS.anyPerPattern `**` segments per line. It then refuses the file (no key): the safe side.
        const deep = set.some(
          (l) => l.split('/').filter((x) => x === '**').length > LIMITS.anyPerPattern,
        );
        assert.ok(deep, 'treekey.js refused a set the strict syntax accepts: ' + set.join(' '));
        refusedByWorker++;
        continue;
      }
      uniq.forEach((e, i) => {
        assert.equal(
          mine.ignored[i],
          ignored(theirs, e.path, false),
          'disagree on ' + e.path + ' under ' + set.join(' '),
        );
        compared++;
      });
      if (uniq.some((e) => !ignored(theirs, e.path)))
        assert.equal(mine.key, await treeKeyOf(uniq, theirs));
    }
    assert.ok(compared > 5000, 'only ' + compared + ' comparisons');
    assert.ok(refusedByWorker < 60, refusedByWorker + ' sets refused by the Worker');
  },
);

test(
  'a line the Worker reads differently from git is refused by the strict side (fail closed)',
  { skip },
  () => {
    // Bare names, trailing slashes, '?' and classes: treekey.js accepts some, docker or git read them
    // at a different depth. The strip step must refuse the whole file rather than guess.
    for (const bad of ['docs/', '*.md', 'docs', '/a?b', '/a[bc]', '!/keep', '/a/**', ' /x']) {
      const run = py(['matchjson'], { patterns: [bad], paths: ['x'] });
      assert.equal(run.status, 3, JSON.stringify(bad) + ' was accepted: ' + run.stdout);
    }
  },
);

test(
  'PROTECTED roots: each is covered by the strip floor or listed with a reason',
  { skip },
  () => {
    const floor = JSON.parse(py(['floorjson']).stdout);
    const covered = (path) =>
      floor.keep_dirs.some((d) => path === d || path.startsWith(d + '/')) ||
      floor.keep_files.includes(path);
    const drift = PROTECTED.map(([p]) => p).filter(
      (p) => !covered(p) && !floor.not_floored.includes(p),
    );
    assert.deepEqual(drift, [], 'PROTECTED roots the floor neither covers nor explains');
    // And the explanation list names only real PROTECTED roots.
    const names = PROTECTED.map(([p]) => p);
    assert.deepEqual(
      floor.not_floored.filter((p) => !names.includes(p)),
      [],
    );
  },
);

// ---- the gate's skip list is inside the strip set ------------------------------------------------
function gateSkipPatterns() {
  const text = readFileSync(root + '.github/workflows/gpu-vast.yml', 'utf8');
  const found = [...text.matchAll(/PATH_SKIP_PATTERNS: >-\n((?:\s+.*\n)+?)\s+run:/g)].map((m) =>
    JSON.parse(m[1].replace(/\n\s*/g, ' ').trim()),
  );
  return found;
}

test('gpu-vast.yml: both policy jobs carry the same skip list', { skip }, () => {
  const found = gateSkipPatterns();
  assert.equal(found.length, 2);
  assert.deepEqual(found[0], found[1]);
  assert.ok(found[0].length >= 7);
});

test(
  'every path the GPU gate skips today is stripped by .workerignore, except the kept traces data (skip set inside strip set)',
  { skip },
  () => {
    const [patterns] = gateSkipPatterns();
    const ignoreLines = lines(readFileSync(root + '.workerignore', 'utf8'));
    const probes = [
      'README.md',
      'a/b/c.md',
      'docs/x',
      'docs/design/a.txt',
      'site/index.html',
      'traces/a/b/c',
      'archive/crates/k/src/lib.rs',
      'LICENSE',
      'LICENSE-APACHE',
      'LICENSE-GPL',
      'LICENSE.txt',
      '.github/ISSUE_TEMPLATE/bug.yml',
      '.github/ISSUE_TEMPLATE/config/x.yml',
    ].map((path) => ({ path, mode: '100644', sha: '1'.repeat(40) }));
    const entries = [...tracked, ...probes];
    const { mine, theirs } = both(ignoreLines, entries);
    // FINDING (2026-11-09): the gate skips `traces/**` while the strip keeps the evidence directories that
    // tests read (they are listed in the header of .workerignore). A change confined to one of them is
    // skipped by the GPU gate and not by the stripped-tree CI job. The skip list is left as it is today (the
    // lane may not change gpu-vast.yml); this exception is parsed from the header, so a new divergence fails.
    const kept = /names the same data\): ([^\n]+)\./
      .exec(readFileSync(root + '.workerignore', 'utf8'))[1]
      .split(', ')
      .map((x) => 'traces/' + x);
    assert.ok(kept.length >= 10, 'the header lists the kept traces directories');
    const underKept = (path) => kept.some((k) => path === k || path.startsWith(k + '/'));
    let skipped = 0;
    let divergent = 0;
    entries.forEach((e, i) => {
      if (!skippedPath(e.path, patterns)) return;
      skipped++;
      if (!mine.ignored[i]) {
        assert.ok(underKept(e.path), 'the gate skips ' + e.path + ' but the strip step keeps it');
        divergent++;
        return;
      }
      assert.ok(ignored(theirs, e.path), 'the gate skips ' + e.path + ' but treekey.js keeps it');
    });
    assert.ok(skipped > 1000, 'only ' + skipped + ' skipped paths compared');
    assert.ok(divergent > 0 && divergent < 1500, divergent + ' skipped-but-kept evidence files');
  },
);
