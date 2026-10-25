import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { onstart } from '../src/onstart.js';
// MEASURED 2026-10-26 on a real Vast box (RTX 3060, slow link): the blobless clone fetched in
// ~56 s and then spent ~15 MINUTES in the checkout, because it downloaded every blob of the tree
// (traces/ alone is ~0.5 GB). The onstart now writes a sparse-checkout list BEFORE the fetch and
// checkout, so only what the box runs is downloaded.
// FALSIFIER: the HEAD==sha check is gone; the list excludes a path a box script reads; the list
// is written after the checkout; or traces/ and archive/ are not really excluded.
const root = fileURLToPath(new URL('../../../', import.meta.url));
const SHA = 'a'.repeat(40);
const REPO = 'reindertpelsma/kayfabe';
const text = () => onstart(SHA, REPO);
const git = (cwd, ...args) =>
  execFileSync('git', args, { cwd, encoding: 'utf8', maxBuffer: 256 * 2 ** 20 });
const haveGit = spawnSync('git', ['--version']).status === 0;
const tracked = haveGit
  ? git(root, 'ls-files', '-z')
      .split('\0')
      .filter(Boolean)
      // Gitlinks (submodules) are not files; the box never initialises them.
      .filter((p) => !p.startsWith('third_party/ogkm'))
  : [];
const skip = haveGit && tracked.length > 1000 ? false : 'needs a full git checkout';

// ---- the sparse list, as the generated script writes it -----------------------------------------
function block(script) {
  const m = /<<'KF_SPARSE'\n([\s\S]*?)\nKF_SPARSE\n/.exec(script);
  return m ? m[1].split('\n') : null;
}
// The patterns use three shapes only: `/*`, `/dir/` (a directory and all below) and `/path`
// (a file or directory). The last matching line wins, like git's own non-cone matching.
export function included(patterns, path) {
  let ok = false;
  for (const line of patterns) {
    const neg = line.startsWith('!');
    const p = neg ? line.slice(1) : line;
    const hit =
      p === '/*'
        ? true
        : p.endsWith('/')
          ? ('/' + path).startsWith(p)
          : '/' + path === p || ('/' + path).startsWith(p + '/');
    if (hit) ok = !neg;
  }
  return ok;
}

// ---- what the box reads --------------------------------------------------------------------------
const ROOTS =
  '(?:crates|cuda|qemu|scripts|tools|firmware|broker|docs|traces|archive|third_party|ci)';
const TOKEN = new RegExp('(?<![A-Za-z0-9_.-])' + ROOTS + '/[A-Za-z0-9_./-]*[A-Za-z0-9_]', 'g');
const read = (p) => readFileSync(join(root, p), 'utf8');
const code = (p) =>
  read(p)
    .split('\n')
    .filter((l) => !/^\s*(#|\/\/|\*|\/\*)/.test(l));
const resolve = (token) => tracked.filter((p) => p === token || p.startsWith(token + '/'));
const files = (dir, pred) => tracked.filter((p) => p.startsWith(dir + '/') && pred(p));

function needed() {
  const need = new Map(); // path -> why
  const add = (paths, why) => paths.forEach((p) => need.set(p, why));
  // Every file of the source dirs the build, the gates and the suites live in, and the root files.
  for (const d of 'crates cuda qemu firmware scripts tools broker docs third_party .cargo .github ci'.split(
    ' ',
  ))
    add(
      files(d, () => true),
      'source dir ' + d,
    );
  add(
    tracked.filter((p) => !p.includes('/')),
    'root file',
  );
  // Paths named in the box scripts (code lines only) and the dependency-cache inputs.
  const boxScripts = [
    ...tracked.filter((p) =>
      /^scripts\/ci\/(vast_[^/]*\.(sh|py)|setup_cache_build\.py|setup-cache\.lock\.json)$/.test(p),
    ),
    ...tracked.filter((p) =>
      /^scripts\/(bench\/(box\/[^/]*\.sh|cuda_ladder\.sh|build_kf3\.sh|v3_gates\.sh|provision_[^/]*\.sh|host_xid_watch\.sh)|fastguest\/[^/]*\.sh)$/.test(
        p,
      ),
    ),
  ];
  for (const s of boxScripts)
    for (const line of code(s).filter((l) => !/^\s*(echo|printf)\b/.test(l)))
      for (const t of line.match(TOKEN) ?? []) add(resolve(t), 'named in ' + s);
  // Test inputs read at runtime by the crates the payload runs (`cargo test -p kf-*`).
  const rel = /(?:\.\.\/|join\(")(traces\/[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_./-]*[A-Za-z0-9_])?)/g;
  for (const f of tracked.filter(
    (p) => /^crates\/kf-[^/]*\/.*\.rs$/.test(p) && !p.includes('/gen/'),
  ))
    for (const line of code(f)) {
      for (const m of line.matchAll(rel)) add(resolve(m[1]), 'read by ' + f);
      for (const m of line.matchAll(/"(real_[a-z0-9]+\/[A-Za-z0-9_.-]+)"/g))
        add(resolve('traces/' + m[1]), 'read by ' + f);
    }
  // Evidence paths the per-die constant tables must resolve (crates/kf-rm/tests/consttab_data.rs).
  for (const f of files('crates/kf-rm/data', (p) => p.endsWith('.tsv')))
    for (const m of read(f).matchAll(/(?:traces|docs)\/[^\s:;,"']+/g))
      add(resolve(m[0]), 'evidence in ' + f);
  // The raw-client CUDA ladder compiles this one archived source.
  add(['archive/nvkvm/tests/mode2/cup2.c'], 'scripts/bench/cuda_ladder.sh');
  return need;
}

// ---- the checks ----------------------------------------------------------------------------------
function analyze(script, sha = SHA) {
  const bad = [];
  const lines = script.split('\n');
  const at = (re) => lines.findIndex((l) => re.test(l));
  const i = {
    clone: at(
      /^git clone --filter=blob:none --no-checkout https:\/\/github\.com\/[-\w.]+\/[-\w.]+\.git \/workspace\/kf-ci-src$/,
    ),
    cfg: at(/^git config core\.sparseCheckout true$/),
    list: at(/^cat >.*info\/sparse-checkout <<'KF_SPARSE'$/),
    fetch: lines.indexOf(`git fetch --depth=1 origin ${sha}`),
    checkout: lines.indexOf(`git checkout --detach ${sha}`),
    verify: lines.indexOf(`[ "$(git rev-parse HEAD)" = '${sha}' ]`),
    exec: at(/^exec bash scripts\/ci\/vast_onstart\.sh$/),
  };
  for (const [k, v] of Object.entries(i)) if (v < 0) bad.push('missing ' + k);
  const order = ['clone', 'cfg', 'list', 'fetch', 'checkout', 'verify', 'exec'];
  for (let n = 1; n < order.length; n++)
    if (i[order[n - 1]] >= 0 && i[order[n]] >= 0 && i[order[n - 1]] >= i[order[n]])
      bad.push(order[n - 1] + ' must come before ' + order[n]);
  if (!/^set -eu$/m.test(script)) bad.push('set -eu');
  const patterns = block(script);
  if (!patterns) return [...bad, 'no quoted sparse heredoc'];
  for (const p of patterns) if (!/^!?\/[A-Za-z0-9_./*-]*$/.test(p)) bad.push('bad pattern ' + p);
  if (patterns[0] !== '/*') bad.push('the list must start from everything: /*');
  for (const [p, why] of needed())
    if (!included(patterns, p)) bad.push('excludes ' + p + ' (' + why + ')');
  for (const d of [
    'traces/windows_matrix_20261011/x.log',
    'traces/new_dir/x',
    'archive/nvkvm/tests/mode2/cup3.c',
    'archive/crates/x',
  ])
    if (included(patterns, d)) bad.push(d + ' is not excluded');
  return bad;
}

test('onstart sparse: the real script passes every check', { skip }, () => {
  assert.deepEqual(analyze(text()), []);
});

test('onstart sparse: order is clone, sparse list, fetch, checkout, HEAD check, exec', () => {
  const t = text().split('\n');
  const idx = (re) => t.findIndex((l) => re.test(l));
  const seq = [
    /^git clone --filter=blob:none --no-checkout /,
    /^git config core\.sparseCheckout true$/,
    /^cat >.*info\/sparse-checkout/,
    /^git fetch --depth=1 origin /,
    /^git checkout --detach /,
    /^\[ "\$\(git rev-parse HEAD\)" = /,
    /^exec bash /,
  ].map(idx);
  assert.deepEqual(
    seq,
    [...seq].sort((a, b) => a - b),
  );
  assert.ok(!seq.includes(-1), String(seq));
});

test('onstart sparse: only the validated sha and repo are interpolated', () => {
  const a = onstart('a'.repeat(40), 'o/r');
  const b = onstart('b'.repeat(40), 'p-q/s.t');
  assert.equal(
    a.replaceAll('a'.repeat(40), 'SHA').replaceAll('o/r', 'REPO'),
    b.replaceAll('b'.repeat(40), 'SHA').replaceAll('p-q/s.t', 'REPO'),
  );
  assert.ok(!/\$\{|\$\(?[A-Za-z_]/.test(text().replace('$(git rev-parse HEAD)', '')));
  for (const bad of ['A'.repeat(40), 'a'.repeat(39), 'a'.repeat(40) + ';id', '', undefined])
    assert.throws(() => onstart(bad, REPO), /Invalid fixed onstart inputs/);
  for (const bad of ['o/r;id', 'o/r r', 'o', 'o/r/s', '$(id)/r', 'o/r\n', ''])
    assert.throws(() => onstart(SHA, bad), /Invalid fixed onstart inputs/);
});

test(
  'onstart sparse: traces and archive are excluded, docs stay (three gates read them)',
  { skip },
  () => {
    const patterns = block(text());
    assert.ok(patterns);
    const size = Object.fromEntries(
      git(root, 'ls-tree', '-r', '-l', 'HEAD')
        .split('\n')
        .filter(Boolean)
        .map((l) => [l.split('\t')[1], Number(l.slice(0, l.indexOf('\t')).split(/\s+/)[3]) || 0]),
    );
    const sum = (f) => tracked.filter(f).reduce((n, p) => n + (size[p] ?? 0), 0);
    const keptTraces = sum((p) => p.startsWith('traces/') && included(patterns, p));
    const allTraces = sum((p) => p.startsWith('traces/'));
    assert.ok(keptTraces <= 64 * 2 ** 20, 'kept traces bytes ' + keptTraces);
    assert.ok(
      allTraces - keptTraces >= 100 * 2 ** 20,
      'excluded traces bytes ' + (allTraces - keptTraces),
    );
    assert.ok(tracked.filter((p) => p.startsWith('archive/') && included(patterns, p)).length <= 1);
    assert.ok(tracked.filter((p) => p.startsWith('docs/')).every((p) => included(patterns, p)));
    assert.ok(included(patterns, 'archive/nvkvm/tests/mode2/cup2.c'));
  },
);

// ---- mutants: each must be caught ----------------------------------------------------------------
const mutants = {
  'drops the HEAD==sha check': (s) => s.replace(/^\[ "\$\(git rev-parse HEAD\)".*\n/m, ''),
  'excludes a needed directory (crates)': (s) =>
    s.replace('\nKF_SPARSE\n', '\n!/crates/\nKF_SPARSE\n'),
  'excludes a needed directory (scripts)': (s) =>
    s.replace('\nKF_SPARSE\n', '\n!/scripts/bench/\nKF_SPARSE\n'),
  'forgets the archived cup2.c the CUDA ladder compiles': (s) =>
    s.replace(/^\/archive\/nvkvm\/tests\/mode2\/cup2\.c\n/m, ''),
  'forgets the cap1 recordings kf-crec reads': (s) =>
    s.replace(/^\/traces\/cap1_coldboot_hermetic\.rec\n/m, ''),
  'forgets the constant evidence traces': (s) =>
    s.replace(/^\/traces\/constants_measure_20261014b\/\n/m, ''),
  'does not exclude traces': (s) => s.replace(/^!\/traces\/\n/m, ''),
  'does not exclude archive': (s) => s.replace(/^!\/archive\/\n/m, ''),
  'writes the sparse list after the checkout': (s) => {
    const m = /(git config core\.sparseCheckout true\n[\s\S]*?\nKF_SPARSE\n)/.exec(s)[1];
    return s.replace(m, '').replace(/(git checkout --detach .*\n)/, '$1' + m);
  },
  'writes the sparse list after the fetch': (s) => {
    const m = /(git config core\.sparseCheckout true\n[\s\S]*?\nKF_SPARSE\n)/.exec(s)[1];
    return s.replace(m, '').replace(/(git fetch --depth=1 .*\n)/, '$1' + m);
  },
  'checks out a branch instead of the detached sha': (s) => s.replace('--detach ', ''),
  'verifies a different revision': (s) => s.replace('rev-parse HEAD', 'rev-parse HEAD~0^{tree}'),
  'unquoted heredoc': (s) => s.replace("<<'KF_SPARSE'", '<<KF_SPARSE'),
};
for (const [name, mutate] of Object.entries(mutants))
  test('onstart sparse mutant is caught: ' + name, { skip }, () => {
    const mutated = mutate(text());
    assert.notEqual(mutated, text(), 'the mutation applied');
    assert.notDeepEqual(analyze(mutated), []);
  });

// ---- a real git run against a local file:// remote -----------------------------------------------
const TREE = {
  'Cargo.toml': 'x',
  'rust-toolchain.toml': 'x',
  'crates/kf-a/Cargo.toml': 'x',
  'scripts/ci/vast_onstart.sh': 'x',
  'tools/drivermatrix/tags.txt': 'x',
  'docs/design/d.md': 'x',
  'archive/old/a.txt': 'archive-only',
  'archive/nvkvm/tests/mode2/cup2.c': 'cup2',
  'archive/nvkvm/tests/mode2/cup3.c': 'cup3-excluded',
  'traces/windows_matrix_20261011/big.log': 'big-excluded',
  'traces/driver_matrix/ranges.tsv': 'ranges',
  'traces/cap1_coldboot_hermetic.rec': 'rec',
  'traces/some_new_dir/x.log': 'new-excluded',
  'scripts/bench/box/merge_check.sh': readFileSync(
    join(root, 'scripts/bench/box/merge_check.sh'),
    'utf8',
  ),
};
function remote(dir) {
  const work = join(dir, 'work');
  mkdirSync(work);
  const g = (...a) => git(work, '-c', 'user.name=t', '-c', 'user.email=t@example.invalid', ...a);
  g('init', '-q', '-b', 'main');
  for (const [p, body] of Object.entries(TREE)) {
    mkdirSync(join(work, p, '..'), { recursive: true });
    writeFileSync(join(work, p), body);
  }
  g('add', '-A');
  g('commit', '-q', '-m', 'x');
  const sha = g('rev-parse', 'HEAD').trim();
  const bare = join(dir, 'remote.git');
  git(dir, 'clone', '-q', '--bare', work, bare);
  git(bare, 'config', 'uploadpack.allowFilter', 'true');
  git(bare, 'config', 'uploadpack.allowAnySHA1InWant', 'true');
  return { sha, bare };
}
const walk = (dir, base = dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.name === '.git'
      ? []
      : e.isDirectory()
        ? walk(join(dir, e.name), base)
        : [join(dir, e.name).slice(base.length + 1)],
  );
function runOnstart(mutate = (s) => s) {
  const dir = mkdtempSync(join(tmpdir(), 'kf-sparse-'));
  const { sha, bare } = remote(dir);
  const dest = join(dir, 'src');
  const script = mutate(onstart(sha, REPO))
    .replace(`https://github.com/${REPO}.git`, 'file://' + bare)
    .replaceAll('/workspace/kf-ci-src', dest)
    .replace(/^exec bash .*$/m, 'echo ONSTART_REACHED_EXEC');
  const r = spawnSync('bash', ['-c', script], { encoding: 'utf8', timeout: 60000 });
  return { dir, dest, sha, bare, r, script };
}
const expectedTree = (patterns) =>
  Object.keys(TREE)
    .filter((p) => included(patterns, p))
    .sort();

test(
  'onstart sparse: a real clone, fetch and checkout leaves out traces/archive and their blobs',
  { skip: !haveGit },
  () => {
    const t = runOnstart();
    try {
      assert.equal(t.r.status, 0, t.r.stderr);
      assert.match(t.r.stdout, /ONSTART_REACHED_EXEC/);
      assert.equal(git(t.dest, 'rev-parse', 'HEAD').trim(), t.sha);
      const got = walk(t.dest).sort();
      assert.deepEqual(got, expectedTree(block(onstart(t.sha, REPO))));
      assert.ok(got.includes('archive/nvkvm/tests/mode2/cup2.c'));
      assert.ok(
        !got.some((p) => p.startsWith('traces/windows_matrix') || p === 'archive/old/a.txt'),
      );
      // The excluded blobs were never downloaded: the partial clone still lists them as missing.
      const missing = git(t.dest, 'rev-list', '--objects', '--missing=print', 'HEAD')
        .split('\n')
        .filter((l) => l.startsWith('?'));
      assert.ok(missing.length >= 4, 'missing blobs: ' + missing.length);
      assert.equal(
        git(t.dest, 'status', '--porcelain', '--untracked-files=no').trim(),
        '',
        'not dirty',
      );
    } finally {
      rmSync(t.dir, { recursive: true, force: true });
    }
  },
);

test(
  'onstart sparse mutant is caught by a real run: list written after the checkout',
  { skip: !haveGit },
  () => {
    const t = runOnstart(mutants['writes the sparse list after the checkout']);
    try {
      // Either the run stops, or the full tree (the 15 minute download) was checked out.
      const full =
        t.r.status === 0 && walk(t.dest).includes('traces/windows_matrix_20261011/big.log');
      assert.ok(t.r.status !== 0 || full);
    } finally {
      rmSync(t.dir, { recursive: true, force: true });
    }
  },
);

test(
  'onstart sparse mutant is caught by a real run: HEAD check removed cannot hide a wrong sha',
  { skip: !haveGit },
  () => {
    const good = runOnstart();
    try {
      const wrong = good.script.replace(`'${good.sha}' ]`, `'${'0'.repeat(40)}' ]`);
      const r = spawnSync('bash', ['-c', wrong.replaceAll(good.dest, good.dest + '-2')], {
        encoding: 'utf8',
      });
      assert.notEqual(r.status, 0);
      assert.doesNotMatch(r.stdout, /ONSTART_REACHED_EXEC/);
    } finally {
      rmSync(good.dir, { recursive: true, force: true });
    }
  },
);

// ---- the second checkout on the box: scripts/bench/box/merge_check.sh ---------------------------------
// The payload points `origin` at the local sparse checkout, which does not hold the excluded blobs.
// merge_check adds a detached worktree of the same revision: it must be sparse too (git < 2.36 does
// not copy the patterns), or the checkout would ask `origin` -- itself -- for blobs it never had.
test('merge_check: its worktree is created without checkout and given the same sparse list', () => {
  const s = read('scripts/bench/box/merge_check.sh');
  assert.match(s, /worktree add --detach --no-checkout "\$CHECKOUT" "\$REV"/);
  const add = s.indexOf('worktree add');
  const copy = s.indexOf('info/sparse-checkout');
  const populate = s.indexOf('reset -q --hard "$REV"');
  assert.ok(add > 0 && copy > add && populate > copy, [add, copy, populate].join());
});

test(
  'merge_check: a real run leaves a sparse worktree of the sparse source',
  { skip: !haveGit },
  () => {
    const t = runOnstart();
    try {
      assert.equal(t.r.status, 0, t.r.stderr);
      const bench = join(t.dir, 'bench');
      const prov = join(t.dir, 'prov');
      const r = spawnSync(
        'bash',
        [join(t.dest, 'scripts/bench/box/merge_check.sh'), 'main', 'sparsetag'],
        {
          encoding: 'utf8',
          timeout: 60000,
          env: {
            ...process.env,
            BENCH_DIR: bench,
            KF_PROV_DIR: prov,
            CARGO_TARGET_DIR: join(t.dir, 'tgt'),
          },
        },
      );
      const log = readFileSync(join(prov, 'sparsetag.log'), 'utf8');
      if (/another merge check is running/.test(log)) return; // the owner's own run holds the lock
      // It stops at the crate census (the fixture has one crate); the worktree is already made.
      assert.equal(r.status, 2, log);
      assert.match(log, /v3 crate census truncated|HEAD=/);
      const [name] = readdirSync(join(bench, 'verify-worktrees'));
      const wt = join(bench, 'verify-worktrees', name);
      assert.equal(git(wt, 'rev-parse', 'HEAD').trim(), t.sha);
      assert.deepEqual(walk(wt).sort(), expectedTree(block(onstart(t.sha, REPO))));
    } finally {
      rmSync(t.dir, { recursive: true, force: true });
    }
  },
);
