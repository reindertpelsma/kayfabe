// Behaviour of the public API over a READ-ONLY seeded SQLite file (synthetic rows).
// FALSIFIER: a non-GET method is served or gets CORS; an unvalidated parameter reaches SQL; a
// private column, row tag or SPECviewperf score appears in any response; pagination skips or
// repeats a run; the matrix disagrees with an independent computation over the seed rows.
import test from 'node:test';
import assert from 'node:assert/strict';
import { api, seededDatabase, FakeD1 } from './helpers.js';
import { syntheticRows } from '../scripts/seed-data.mjs';
import { QUERIES } from '../src/queries.js';
import { handle } from '../src/api.js';
import { encodeCursor } from '../src/validate.js';
import { Db } from '../src/db.js';

const rows = syntheticRows();
const log = [];
const { call, db } = api({ env: { LOGS_BASE: 'https://logs.kayfabe.dev' }, log });
const finalRun = rows.runs.find((r) => r.verdict === 'PASS' && r.suite === 'linux-gate');
const runningRun = rows.runs.find((r) => r.verdict === null);
const specRun = rows.runs.find((r) => r.suite === 'windows-specviewperf' && r.scores_json);
const docsCommit = rows.commits.find((c) => c.message.endsWith('docs only'));
const byTime = [...rows.runs].sort((a, b) =>
  a.sort_at === b.sort_at ? (a.id < b.id ? 1 : -1) : a.sort_at < b.sort_at ? 1 : -1,
);

const ENDPOINTS = () => [
  '/v1/health',
  '/v1/schema',
  '/v1/dies',
  '/v1/hosts',
  '/v1/matrix',
  '/v1/matrix?family=ada&cpu=intel&nested=1&driver=580.159.04&branch=next',
  '/v1/matrix?commit=' + docsCommit.sha,
  '/v1/runs',
  '/v1/runs?verdict=RUNNING',
  '/v1/runs?die=AD104&suite=linux-gate&tier=middle&source=reproduced&limit=5',
  '/v1/runs/' + finalRun.id,
  '/v1/runs/' + runningRun.id,
  '/v1/runs/' + specRun.id,
  '/v1/cells/AD104/linux-gate',
  '/v1/cells/GH100/windows-gate',
  '/v1/commits/' + docsCommit.sha + '/requirements',
];

test('GET endpoints answer 200 JSON with CORS *, nosniff and a cache policy', async () => {
  for (const path of ENDPOINTS()) {
    const r = await call(path);
    assert.equal(r.status, 200, path);
    assert.equal(r.headers.get('access-control-allow-origin'), '*', path);
    assert.equal(r.headers.get('x-content-type-options'), 'nosniff', path);
    assert.equal(r.headers.get('content-type'), 'application/json; charset=utf-8', path);
    assert.match(r.headers.get('content-security-policy'), /default-src 'none'/, path);
    assert.ok(r.headers.get('cache-control'), path);
    assert.equal(typeof r.body, 'object', path);
  }
});

test('every method other than GET is 405 with Allow: GET and no CORS header', async () => {
  for (const method of ['POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS', 'HEAD']) {
    for (const path of ['/v1/runs', '/v1/health', '/v1/logs/' + finalRun.id + '/box.log']) {
      const init = { method };
      if (method !== 'HEAD' && method !== 'OPTIONS') init.body = 'x';
      const r = await call(path, init);
      assert.equal(r.status, 405, method + ' ' + path);
      assert.equal(r.headers.get('allow'), 'GET');
      assert.equal(r.headers.get('access-control-allow-origin'), null);
    }
  }
});

test('cache policy: final run immutable, running run and lists short, errors no-store', async () => {
  assert.equal(
    (await call('/v1/runs/' + finalRun.id)).headers.get('cache-control'),
    'public, max-age=31536000, immutable',
  );
  assert.equal(
    (await call('/v1/runs/' + runningRun.id)).headers.get('cache-control'),
    'public, max-age=30',
  );
  assert.equal((await call('/v1/runs')).headers.get('cache-control'), 'public, max-age=30');
  assert.equal((await call('/v1/matrix')).headers.get('cache-control'), 'public, max-age=30');
  assert.equal((await call('/v1/schema')).headers.get('cache-control'), 'public, max-age=300');
  assert.equal((await call('/v1/health')).headers.get('cache-control'), 'no-store');
  assert.equal((await call('/v1/runs/no-such-run')).headers.get('cache-control'), 'no-store');
});

const BAD = [
  ['/v1/runs?unknown=1', 400],
  ['/v1/runs?die=AD104&die=AD104', 400],
  ['/v1/health?x=1', 400],
  ['/v1/dies?family=ada', 400],
  ['/v1/runs?commit=ABC', 400],
  ['/v1/runs?commit=' + 'a'.repeat(41), 400],
  ['/v1/runs?die=ad104', 400],
  ['/v1/runs?die=AD104%27%20OR%201%3D1--', 400],
  ['/v1/runs?suite=linux', 400],
  ['/v1/runs?verdict=pass', 400],
  ['/v1/runs?tier=huge', 400],
  ['/v1/runs?source=community', 400],
  ['/v1/runs?limit=0', 400],
  ['/v1/runs?limit=101', 400],
  ['/v1/runs?limit=abc', 400],
  ['/v1/runs?limit=', 400],
  ['/v1/runs?cursor=***', 400],
  ['/v1/runs?cursor=' + 'A'.repeat(201), 400],
  ['/v1/runs?cursor=' + btoa('{"s":"x","i":"y"}'), 400],
  ['/v1/runs?cursor=' + btoa('{"s":"2026-10-01","i":"syn-1-ab"}'), 400],
  ['/v1/runs?cursor=' + btoa('{"s":"2026-10-01T00:00:00Z","i":"x"}'), 400],
  ['/v1/runs?cursor=' + btoa('{"s":"2026-10-01T00:00:00Z","i":"syn-1-x","z":1}'), 400],
  ['/v1/runs?cursor=' + btoa('["2026-10-01T00:00:00Z","syn-1-ab"]'), 400],
  ['/v1/matrix?driver=latest', 400],
  ['/v1/matrix?cpu=arm', 400],
  ['/v1/matrix?nested=true', 400],
  ['/v1/matrix?family=Ada', 400],
  ['/v1/matrix?branch=main', 400],
  ['/v1/matrix?commit=' + 'f'.repeat(40), 404],
  ['/v1/cells/ad104/linux-gate', 400],
  ['/v1/cells/ZZ999/linux-gate', 404],
  ['/v1/cells/AD104/linux-unknown', 404],
  ['/v1/cells/AD104/linux-gate?limit=1000', 400],
  ['/v1/cells/AD104/linux-gate?cursor=abc', 400],
  ['/v1/runs/UPPER-case', 400],
  ['/v1/runs/syn-no-such-run', 404],
  ['/v1/runs/' + finalRun.id + '/', 404],
  ['/v1/commits/xyz/requirements', 400],
  ['/v1/commits/' + '0'.repeat(40) + '/requirements', 404],
  ['/v1/logs/' + finalRun.id + '/../secret.log', 404],
  ['/v1/logs/' + finalRun.id + '/box.sh', 400],
  ['/v1/logs/' + finalRun.id + '/other.log', 404],
  ['/v1/logs/syn-no-such-run/box.log', 404],
  ['/v2/runs', 404],
  ['/', 404],
  ['/v1/sql?q=select%201', 404],
];

test('invalid input is refused with fixed details that never echo the request', async () => {
  for (const [path, status] of BAD) {
    const r = await call(path + (path.includes('?') ? '&' : '?') + 'marker=Q9ZQ');
    assert.ok(r.status === status || (status === 404 && r.status === 400), path + ' ' + r.status);
    const exact = await call(path);
    assert.equal(exact.status, status, path);
    assert.equal(typeof exact.body.error, 'string', path);
    assert.doesNotMatch(exact.text, /Q9ZQ|OR 1=1|AD104'|secret|select 1/i, path);
    assert.equal(exact.headers.get('cache-control'), 'no-store', path);
  }
});

test('private columns, row tags, host addresses and SPECviewperf scores never leave the API', async () => {
  const forbidden = [/192\.0\.2\./, /synthetic-tag-/, /box_token/, /synthetic-machine/, /"tag"/];
  const paths = [...ENDPOINTS()];
  for (const run of rows.runs) paths.push('/v1/runs/' + run.id);
  for (const path of paths) {
    const r = await call(path);
    for (const pattern of forbidden) assert.doesNotMatch(r.text, pattern, path);
    if (r.body?.run?.suite === 'windows-specviewperf') {
      assert.equal(r.body.run.scores, null);
      assert.equal(r.body.run.scores_hidden, 'SPEC_RESULTS.md');
    }
  }
  const list = await call('/v1/runs?suite=windows-specviewperf&limit=100');
  assert.ok(list.body.runs.length > 0);
  for (const run of list.body.runs) assert.equal(run.scores, null);
  const spec = await call('/v1/runs/' + specRun.id);
  assert.doesNotMatch(spec.text, /snx-04/);
});

test('run pagination walks every run exactly once, newest first', async () => {
  const seen = [];
  let cursor = null;
  for (let page = 0; page < 1000; page++) {
    const r = await call('/v1/runs?limit=7' + (cursor ? '&cursor=' + cursor : ''));
    assert.equal(r.status, 200);
    assert.ok(r.body.runs.length <= 7);
    seen.push(...r.body.runs.map((x) => x.id));
    cursor = r.body.next_cursor;
    if (!cursor) break;
  }
  assert.deepEqual(
    seen,
    byTime.map((r) => r.id),
  );
});

test('cell history pages with before= and stays inside the cell', async () => {
  const expected = byTime.filter((r) => r.die === 'GA106' && r.suite === 'linux-gate');
  assert.ok(expected.length > 2);
  const seen = [];
  let before = null;
  do {
    const r = await call(
      '/v1/cells/GA106/linux-gate?limit=2' + (before ? '&before=' + before : ''),
    );
    assert.equal(r.body.die.family, 'ampere');
    for (const run of r.body.runs) assert.equal(run.die + run.suite, 'GA106linux-gate');
    seen.push(...r.body.runs.map((x) => x.id));
    before = r.body.next_before;
  } while (before);
  assert.deepEqual(
    seen,
    expected.map((r) => r.id),
  );
});

test('a forged cursor moves the page but cannot inject anything', async () => {
  const cursor = encodeCursor('2026-10-05T00:00:00Z', 'zzzzzz-forged');
  const r = await call('/v1/runs?limit=100&cursor=' + cursor);
  assert.equal(r.status, 200);
  for (const run of r.body.runs)
    assert.ok(run.finished_at === null || run.finished_at <= '2026-10-05T00:00:00Z');
});

function independentMatrix(branch = 'next') {
  const seqOf = new Map(
    rows.branch_commits.filter((b) => b.branch === branch).map((b) => [b.sha, b.seq]),
  );
  const boundary = new Map(rows.commits.map((c) => [c.sha, c.boundary_hash]));
  const head = Math.max(...seqOf.values());
  const latest = new Map();
  const pass = new Map();
  for (const run of byTime) {
    const key = run.die + '|' + run.suite;
    if (!latest.has(key)) latest.set(key, run);
    if (run.verdict === 'PASS' && seqOf.has(run.commit_sha)) {
      const prior = pass.get(key);
      if (!prior || seqOf.get(run.commit_sha) > seqOf.get(prior.commit_sha)) pass.set(key, run);
    }
  }
  const changes = (run) => {
    const from = seqOf.get(run.commit_sha);
    const hashes = new Set();
    for (const [sha, seq] of seqOf)
      if (seq > from && seq <= head && boundary.get(sha) !== run.boundary_hash)
        hashes.add(boundary.get(sha));
    return hashes.size;
  };
  return { latest, pass, changes };
}

test('matrix cells match an independent computation over the seed rows', async () => {
  const r = await call('/v1/matrix');
  const { latest, pass, changes } = independentMatrix();
  assert.equal(r.body.families.length, 5);
  let cells = 0;
  for (const family of r.body.families) {
    for (const die of family.dies) {
      assert.deepEqual(
        Object.keys(die.cells),
        r.body.suites.map((s) => s.id),
      );
      for (const [suite, cell] of Object.entries(die.cells)) {
        cells++;
        const key = die.id + '|' + suite;
        assert.equal(cell.latest?.run ?? null, latest.get(key)?.id ?? null, key);
        assert.equal(cell.last_pass?.run ?? null, pass.get(key)?.id ?? null, key);
        if (cell.last_pass) assert.equal(cell.last_pass.changes_since, changes(pass.get(key)), key);
      }
    }
  }
  assert.equal(cells, 15 * 6);
  const hopper = r.body.families.find((f) => f.id === 'hopper').dies[0].cells;
  assert.equal(hopper['windows-gate'].latest, null, 'no Windows rows for Hopper in the seed');
});

test('matrix filters restrict families, platforms and the commit boundary', async () => {
  const ada = await call('/v1/matrix?family=ada');
  assert.deepEqual(
    ada.body.families.map((f) => f.id),
    ['ada'],
  );
  const bare = await call('/v1/matrix?nested=0');
  for (const f of bare.body.families)
    for (const d of f.dies) for (const c of Object.values(d.cells)) assert.equal(c.latest, null);
  const amd = await call('/v1/matrix?cpu=amd');
  for (const f of amd.body.families)
    for (const d of f.dies)
      for (const c of Object.values(d.cells)) {
        if (!c.latest) continue;
        const run = (await call('/v1/runs/' + c.latest.run)).body.run;
        assert.equal(run.platform.cpu_vendor, 'amd');
      }
  const one = await call('/v1/matrix?commit=' + docsCommit.sha);
  assert.equal(one.body.filters.commit, docsCommit.sha);
  let found = 0;
  for (const f of one.body.families)
    for (const d of f.dies)
      for (const c of Object.values(d.cells)) {
        if (!c.latest) continue;
        found++;
        const run = rows.runs.find((x) => x.id === c.latest.run);
        assert.equal(run.boundary_hash, docsCommit.boundary_hash);
        assert.notEqual(
          run.commit_sha,
          docsCommit.sha,
          'docs-only commit answered by an earlier run',
        );
      }
  assert.ok(found > 0);
});

test('a docs-only commit carries requirement answers from its boundary twin', async () => {
  const r = await call('/v1/commits/' + docsCommit.sha + '/requirements');
  assert.equal(r.body.commit.sha, docsCommit.sha);
  assert.ok(r.body.requirements.length > 0);
  for (const q of r.body.requirements) {
    assert.match(q.state, /^(PASS|FAIL|PENDING|UNAVAILABLE|NOT_NEEDED)$/);
    assert.ok(q.run.id.startsWith('syn-'));
  }
  assert.deepEqual(
    r.body.branches.map((b) => b.branch),
    ['next'],
  );
});

test('log redirect: 302 only for a listed log, only to a valid kayfabe.dev https origin', async () => {
  const r = await call('/v1/logs/' + finalRun.id + '/box.log');
  assert.equal(r.status, 302);
  assert.equal(
    r.headers.get('location'),
    'https://logs.kayfabe.dev/runs/' + finalRun.id + '/box.log',
  );
  const run = (await call('/v1/runs/' + finalRun.id)).body.run;
  assert.deepEqual(run.logs[0], {
    name: 'box.log',
    path: '/v1/logs/' + finalRun.id + '/box.log',
    direct: 'https://logs.kayfabe.dev/runs/' + finalRun.id + '/box.log',
  });
  for (const base of [
    '',
    'http://logs.kayfabe.dev',
    'https://logs.kayfabe.dev.evil.example',
    'https://evil.example',
    'https://logs.kayfabe.dev/path',
    'https://user:pw@logs.kayfabe.dev',
    'https://logs.kayfabe.dev:8443',
    'https://logs.kayfabe.dev?x=1',
    'not a url',
  ]) {
    const other = api({ database: db, env: { LOGS_BASE: base } });
    const redirect = await other.call('/v1/logs/' + finalRun.id + '/box.log');
    assert.equal(redirect.status, 404, base);
    const page = (await other.call('/v1/runs/' + finalRun.id)).body.run;
    assert.equal(page.logs[0].direct, undefined, base);
  }
});

test('every executed statement is a QUERIES entry bound with exactly its parameters', async () => {
  for (const path of ENDPOINTS()) await call(path);
  await call('/v1/logs/' + finalRun.id + '/box.log');
  const known = new Set(Object.values(QUERIES));
  assert.ok(log.length > 20);
  for (const { sql, params } of log) {
    assert.ok(known.has(sql), 'unknown SQL executed');
    const max = Math.max(0, ...[...sql.matchAll(/\?(\d+)/g)].map((m) => Number(m[1])));
    assert.equal(params.length, max, sql.slice(0, 60));
  }
  const used = new Set(log.map((e) => e.sql));
  for (const [name, sql] of Object.entries(QUERIES))
    assert.ok(used.has(sql), 'untested statement ' + name);
});

test('the test database really is read-only (so a write statement would fail)', () => {
  assert.throws(
    () => db.prepare("INSERT INTO audit (at, actor, action) VALUES ('a','b','c')").run(),
    /readonly/i,
  );
});

test('a database failure is a fixed 500 without details', async () => {
  const broken = {
    prepare() {
      throw new Error('D1_ERROR: no such table: runs SECRETDETAIL');
    },
  };
  const r = await handle(new Request('https://api.test.invalid/v1/runs'), { DB: broken });
  assert.equal(r.status, 500);
  const text = await r.text();
  assert.doesNotMatch(text, /SECRETDETAIL|D1_ERROR/);
  const missing = await handle(new Request('https://api.test.invalid/v1/runs'), {});
  assert.equal(missing.status, 500);
});

test('an empty database answers every list with empty results, not errors', async () => {
  const empty = api({ database: seededDatabase({ schemaOnly: true }).reader });
  for (const path of ['/v1/matrix', '/v1/runs', '/v1/dies', '/v1/hosts', '/v1/health']) {
    const r = await empty.call(path);
    assert.equal(r.status, 200, path);
  }
  assert.equal((await empty.call('/v1/matrix')).body.reference.seq, 0);
});

test('host counts carry no identifiers', async () => {
  const r = await call('/v1/hosts');
  for (const row of r.body.counts)
    assert.deepEqual(Object.keys(row).sort(), ['class', 'die', 'hosts', 'status']);
  assert.equal(
    r.body.counts.reduce((n, row) => n + row.hosts, 0),
    rows.hosts.length,
  );
});

test('the database wrapper refuses anything but a statement name and primitive bindings', async () => {
  const calls = [];
  const wrapper = new Db(new FakeD1(db, calls));
  for (const name of ['SELECT 1', 'constructor', '__proto__', 'toString', ''])
    await assert.rejects(wrapper.all(name), /unknown statement/, name);
  for (const value of [undefined, {}, [], true, 1n, NaN, Infinity])
    await assert.rejects(wrapper.all('dieById', [value]), /binding type/);
  assert.equal(calls.length, 0, 'nothing reached the database');
  assert.throws(() => new Db(undefined), /D1 binding/);
});
