import test from 'node:test';
import assert from 'node:assert/strict';
import {
  compilePattern,
  parseIgnore,
  ignored,
  treeKeyOf,
  computeTreeKey,
  resultKey,
  commitKey,
  patternsFor,
  treeSource,
  DEFAULT_IGNORE,
  LIMITS,
} from '../src/treekey.js';
// The tree key (docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md §3).
// FALSIFIER: a newly ignored existing file keeps the key; a new file added with its exclusion
// changes it; a pattern outside the subset, a truncated listing or a fetch error yields a key
// instead of null; the order of the listing changes the key.

const sha = (n) => n.toString(16).padStart(40, '0');
const base = [
  { path: 'Cargo.toml', mode: '100644', sha: sha(1) },
  { path: 'crates/kf-a/src/lib.rs', mode: '100644', sha: sha(2) },
  { path: 'docs/a.md', mode: '100644', sha: sha(3) },
  { path: 'scripts/run.sh', mode: '100755', sha: sha(4) },
  { path: 'third_party/ogkm', mode: '160000', sha: sha(5) },
  { path: 'CLAUDE.md', mode: '120000', sha: sha(6) },
];
const P = (text) => parseIgnore(text);

test('treekey: newly ignoring an existing file changes the key', async () => {
  const before = await treeKeyOf(base, P('/docs/\n'));
  const after = await treeKeyOf(base, P('/docs/\n/scripts/\n'));
  assert.match(before, /^[0-9a-f]{64}$/);
  assert.notEqual(before, after);
});

test('treekey: a new file added together with its exclusion keeps the key', async () => {
  const before = await treeKeyOf(base, P('/docs/\n'));
  const added = [...base, { path: 'notes/new.txt', mode: '100644', sha: sha(9) }];
  assert.equal(await treeKeyOf(added, P('/docs/\n/notes/\n')), before);
  // ...and without the exclusion it changes.
  assert.notEqual(await treeKeyOf(added, P('/docs/\n')), before);
});

test('treekey: edits to ignored files and listing order never change the key', async () => {
  const k = await treeKeyOf(base, P('/docs/\n'));
  const edited = base.map((e) => (e.path === 'docs/a.md' ? { ...e, sha: sha(99) } : e));
  assert.equal(await treeKeyOf(edited, P('/docs/\n')), k);
  assert.equal(await treeKeyOf([...base].reverse(), P('/docs/\n')), k);
  // An edit to a kept file, a mode change or a rename changes it.
  const kept = base.map((e) => (e.path === 'Cargo.toml' ? { ...e, sha: sha(98) } : e));
  assert.notEqual(await treeKeyOf(kept, P('/docs/\n')), k);
  const mode = base.map((e) => (e.path === 'scripts/run.sh' ? { ...e, mode: '100644' } : e));
  assert.notEqual(await treeKeyOf(mode, P('/docs/\n')), k);
  const moved = base.map((e) => (e.path === 'Cargo.toml' ? { ...e, path: 'Cargo2.toml' } : e));
  assert.notEqual(await treeKeyOf(moved, P('/docs/\n')), k);
});

test('treekey: the key is the documented hash', async () => {
  const { createHash } = await import('node:crypto');
  const entries = [
    { path: 'b', mode: '100644', sha: sha(2) },
    { path: 'a', mode: '100755', sha: sha(1) },
  ];
  const expected = createHash('sha256')
    .update('kf-tree-v1\na\x00' + '100755\x00' + sha(1) + '\nb\x00100644\x00' + sha(2) + '\n')
    .digest('hex');
  assert.equal(await treeKeyOf(entries, []), expected);
  // Byte order, not UTF-16 order: U+FF5E sorts before U+10000 in UTF-16, after it in UTF-8.
  const wide = [
    { path: '\u{10000}', mode: '100644', sha: sha(1) },
    { path: '～', mode: '100644', sha: sha(2) },
  ];
  const bytes = createHash('sha256')
    .update('kf-tree-v1\n～\x00100644\x00' + sha(2) + '\n\u{10000}\x00100644\x00' + sha(1) + '\n')
    .digest('hex');
  assert.equal(await treeKeyOf(wide, []), bytes);
});

test('treekey: commit keys are domain-separated from tree keys', async () => {
  const c = await commitKey(sha(7));
  assert.match(c, /^[0-9a-f]{64}$/);
  assert.notEqual(c, await treeKeyOf([], []));
  assert.notEqual(c, await commitKey(sha(8)));
  await assert.rejects(() => commitKey('x'));
});

test('treekey: malformed entries give null', async () => {
  for (const bad of [
    { path: '/abs', mode: '100644', sha: sha(1) },
    { path: 'a/../b', mode: '100644', sha: sha(1) },
    { path: 'a//b', mode: '100644', sha: sha(1) },
    { path: '.git/config', mode: '100644', sha: sha(1) },
    { path: 'a', mode: '040000', sha: sha(1) },
    { path: 'a', mode: '100644', sha: 'z'.repeat(40) },
    { path: 'a\nb', mode: '100644', sha: sha(1) },
  ])
    assert.equal(await treeKeyOf([bad], []), null, JSON.stringify(bad));
  const dup = { path: 'a', mode: '100644', sha: sha(1) };
  assert.equal(await treeKeyOf([dup, dup], []), null);
  assert.equal(await treeKeyOf([], null), null);
});

test('treekey: patterns outside the subset are refused (fail closed)', () => {
  for (const line of [
    '!keep.md',
    'a\\b',
    '[ab].md',
    'a b',
    'trail ',
    '/',
    '//',
    'a//b',
    '**',
    '**/',
    'a**',
    '**.md',
    'x/**b',
    '../x',
    'a/./b',
    'a#b',
    'x'.repeat(LIMITS.patternLength + 1),
  ])
    assert.equal(compilePattern(line), null, JSON.stringify(line));
  assert.equal(parseIgnore('ok/\r\n'), null, 'CR');
  assert.equal(parseIgnore('ok/\n!no\n'), null, 'one bad line poisons the file');
  assert.equal(parseIgnore('a\n'.repeat(LIMITS.patterns + 1)), null, 'too many patterns');
  assert.equal(parseIgnore('#'.repeat(LIMITS.ignoreBytes + 1)), null, 'too large');
  assert.equal(parseIgnore(undefined), null);
  assert.deepEqual(parseIgnore('# only a comment\n\n'), []);
});

test('treekey: anchoring, directory-only and wildcard rules read like git', () => {
  const cases = [
    // [pattern, path, isDirectoryEntry, ignored]
    ['docs/', 'docs/a.md', false, true],
    ['docs/', 'crates/x/docs/a.md', false, true],
    ['docs/', 'docs', false, false], // a FILE named docs: directory-only pattern
    ['/docs/', 'crates/x/docs/a.md', false, false],
    ['/docs/', 'docs/a.md', false, true],
    ['docs', 'docs', false, true],
    ['docs', 'a/docs', false, true],
    ['*.md', 'a/b/c.md', false, true],
    ['*.md', 'a.md/x.rs', false, true], // a directory called a.md
    ['*.md', 'a/b.mdx', false, false],
    ['a/*.md', 'a/b.md', false, true],
    ['a/*.md', 'a/b/c.md', false, false],
    ['a/*.md', 'x/a/b.md', false, false],
    ['?.rs', 'a.rs', false, true],
    ['?.rs', 'ab.rs', false, false],
    ['**/fixtures/', 'x/y/fixtures/f', false, true],
    ['**/fixtures/', 'fixtures/f', false, true],
    ['a/**/b', 'a/b', false, true],
    ['a/**/b', 'a/x/y/b', false, true],
    ['a/**', 'a/x', false, true],
    ['a/**', 'a', false, false],
    ['*', '.hidden', false, true],
    ['third_party/ogkm/', 'third_party/ogkm', true, true], // a submodule is a directory
    ['third_party/ogkm/', 'third_party/ogkm', false, false],
    ['/crates/kf-*/tests/', 'crates/kf-a/tests/t.rs', false, true],
    ['/crates/kf-*/tests/', 'crates/kayfabe-a/tests/t.rs', false, false],
    ['x.y', 'xay', false, false], // '.' is literal
    ['c++', 'c++', false, true],
  ];
  for (const [pattern, path, dir, expected] of cases)
    assert.equal(ignored(P(pattern), path, dir), expected, pattern + ' ~ ' + path);
});

test('treekey: the built-in default applies only without a .workerignore', () => {
  assert.deepEqual(DEFAULT_IGNORE, ['/archive/', '/docs/', '/site/']);
  assert.ok(ignored(patternsFor(undefined), 'docs/x.md'));
  assert.ok(!ignored(patternsFor(undefined), 'traces/driver_matrix/drmnv.tsv'));
  assert.ok(!ignored(patternsFor(''), 'docs/x.md'), 'an empty file ignores nothing');
  assert.equal(patternsFor('!x'), null);
});

// --- GitHub trees API ----------------------------------------------------------------------

function fakeRepo({ ignore, truncated = false, files = base, ignoreMode = '100644' } = {}) {
  const blobs = new Map(),
    trees = new Map(),
    calls = [];
  let n = 1000;
  const id = () => sha(n++);
  const root = [];
  const top = new Map();
  for (const f of files) {
    const [head, ...rest] = f.path.split('/');
    if (!rest.length)
      root.push({
        path: head,
        mode: f.mode,
        type: f.mode === '160000' ? 'commit' : 'blob',
        sha: f.sha,
      });
    else {
      if (!top.has(head)) top.set(head, []);
      top.get(head).push({ ...f, path: rest.join('/') });
    }
  }
  for (const [dir, list] of top) {
    const t = id();
    root.push({ path: dir, mode: '040000', type: 'tree', sha: t });
    const listing = [];
    const dirs = new Set();
    for (const f of list) {
      const parts = f.path.split('/');
      for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
      listing.push({
        path: f.path,
        mode: f.mode,
        type: f.mode === '160000' ? 'commit' : 'blob',
        sha: f.sha,
      });
    }
    for (const d of dirs) listing.push({ path: d, mode: '040000', type: 'tree', sha: id() });
    trees.set(t, { tree: listing, truncated });
  }
  if (ignore !== undefined) {
    const b = id();
    blobs.set(b, ignore);
    root.push({ path: '.workerignore', mode: ignoreMode, type: 'blob', sha: b });
  }
  const rootSha = id();
  trees.set(rootSha, { tree: root, truncated: false });
  const commit = sha(7777);
  const get = async (path) => {
    calls.push(path);
    let m;
    if ((m = /^git\/commits\/([0-9a-f]{40})$/.exec(path)))
      return m[1] === commit ? { tree: { sha: rootSha } } : null;
    if ((m = /^git\/trees\/([0-9a-f]{40})(\?recursive=1)?$/.exec(path))) return trees.get(m[1]);
    if ((m = /^git\/blobs\/([0-9a-f]{40})$/.exec(path))) {
      const text = blobs.get(m[1]);
      return {
        encoding: 'base64',
        size: Buffer.byteLength(text),
        content: Buffer.from(text)
          .toString('base64')
          .replace(/(.{60})/g, '$1\n'),
      };
    }
    throw Error('unexpected ' + path);
  };
  return { get, calls, commit };
}

test('treekey: wholly ignored top-level directories are never listed', async () => {
  const repo = fakeRepo({ ignore: '/docs/\n/crates/\n' });
  const tree = await computeTreeKey(repo.get, repo.commit);
  assert.equal(tree.kind, 'tree');
  const listed = repo.calls.filter((c) => c.endsWith('?recursive=1'));
  // Top-level directories: crates, docs, scripts, third_party -> only scripts and third_party.
  assert.equal(listed.length, 2);
});

test('treekey: computeTreeKey matches the offline key and fails closed', async () => {
  const ignoreText = '/docs/\n.workerignore\n';
  const repo = fakeRepo({ ignore: ignoreText });
  const online = await computeTreeKey(repo.get, repo.commit);
  assert.equal(online.key, await treeKeyOf(base, P(ignoreText)));
  // Default patterns without a .workerignore.
  const plain = fakeRepo();
  assert.equal(
    (await computeTreeKey(plain.get, plain.commit)).key,
    await treeKeyOf(base, patternsFor(undefined)),
  );
  // Fail closed.
  assert.equal(await computeTreeKey(fakeRepo({ ignore: '!x\n' }).get, repo.commit), null);
  const link = fakeRepo({ ignore: '/docs/\n', ignoreMode: '120000' });
  assert.equal(await computeTreeKey(link.get, link.commit), null, 'a symlinked .workerignore');
  const t = fakeRepo({ truncated: true });
  assert.equal(await computeTreeKey(t.get, t.commit), null, 'truncated subtree');
  assert.equal(await computeTreeKey(repo.get, sha(1)), null, 'unknown commit');
  assert.equal(await computeTreeKey(repo.get, 'nope'), null);
  const failing = async () => {
    throw Error('network');
  };
  assert.equal(await computeTreeKey(failing, repo.commit), null);
  const fallback = await resultKey(failing, repo.commit);
  assert.deepEqual(fallback, { key: await commitKey(repo.commit), kind: 'commit' });
});

test('treekey: treeSource bounds paths and requests', async () => {
  const seen = [];
  const github = {
    cfg: { repo: 'o/r' },
    headers: { Accept: 'application/vnd.github+json' },
    fetcher: async (url) => {
      seen.push(url);
      return Response.json({ tree: [], truncated: false });
    },
  };
  const get = treeSource(github);
  assert.deepEqual(await get('git/trees/' + sha(1) + '?recursive=1'), {
    tree: [],
    truncated: false,
  });
  assert.equal(seen[0], 'https://api.github.com/repos/o/r/git/trees/' + sha(1) + '?recursive=1');
  await assert.rejects(() => get('contents/x'));
  await assert.rejects(() => get('git/trees/' + sha(1) + '/../x'));
  for (let i = 1; i < LIMITS.requests; i++) await get('git/blobs/' + sha(i));
  await assert.rejects(() => get('git/blobs/' + sha(2)), /tree requests/);
});
