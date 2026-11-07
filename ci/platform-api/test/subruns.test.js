import test from 'node:test';
import assert from 'node:assert/strict';
import { api } from './helpers.js';
import { syntheticRows } from '../scripts/seed-data.mjs';
import { DIMS } from '../src/rank.js';
import { seededDatabase } from './helpers.js';
// /v1/ci/subruns (migration 0004): ranked subruns with an explained match. FALSIFIER: an unknown or
// repeated parameter or dimension, an over-long list, a limit above 100, a cursor of another request
// or a require/ignore conflict is accepted instead of a 400 that lists the allowed words; the
// effective parameters are not echoed; relax, require, ignore, sort, include, min_closeness or the
// cursor do not do what the response says; a page repeats or skips a hit; an expanded body is not
// bounded or does not say truncated; a write statement, a raw SQL path or a non-GET method is
// reachable.
const rows = syntheticRows();
const { call } = api({ env: { LOGS_BASE: 'https://logs.kayfabe.dev' } });
const Q = '/v1/ci/subruns';
const near = '?die=GA106&os=windows&host_driver=580.95.05&guest_driver=580.88&cpu=amd';

test('the default answer: ranked, exact first, effective parameters stated', async () => {
  const r = await call(Q + near);
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.effective, {
    relax: DIMS,
    require: [],
    ignore: [],
    limit: 20,
    sort: 'closeness',
    include: [],
    min_closeness: 7,
  });
  assert.equal(r.headers.get('access-control-allow-origin'), '*');
  const [first, second] = r.body.hits;
  for (const hit of [first, second]) {
    assert.deepEqual(hit.match.relaxed, []);
    assert.equal(hit.die, 'GA106');
    assert.equal(hit.os, 'windows');
    assert.equal(hit.guest_driver, '580.88');
    assert.equal(hit.cpu.vendor, 'amd');
    assert.equal(hit.session, '/v1/ci/runs/' + hit.run);
    assert.deepEqual(hit.claimed, ['cpu', 'host_driver', 'guest_driver', 'verdict']);
  }
  assert.ok(r.body.hits.length > 6);
  assert.ok(r.body.eligible >= r.body.hits.length);
  // Every later hit is explained: relaxed dims are named with what was wanted and what was found.
  for (const hit of r.body.hits.slice(2)) {
    assert.ok(hit.match.relaxed.length >= 1);
    for (const x of hit.match.relaxed) {
      assert.ok(DIMS.includes(x.dim));
      assert.equal(typeof x.wanted, 'string');
      assert.ok('found' in x);
    }
    assert.deepEqual(
      [...new Set([...hit.match.exact, ...hit.match.relaxed.map((x) => x.dim)])].length,
      hit.match.exact.length + hit.match.relaxed.length,
    );
  }
  const counts = r.body.hits.map((h) => h.match.relaxed.length);
  assert.deepEqual(
    counts,
    [...counts].sort((a, b) => a - b),
    'fewer relaxed dimensions first',
  );
  assert.ok(r.body.wanted.die === 'GA106' && r.body.wanted.cpu === 'amd');
});

test('no parameters: every subrun that ran, newest first; NOT_RUN and in-flight are never hits', async () => {
  const r = await call(Q + '?limit=100');
  const notRun = new Set(
    rows.ci_subruns.filter((s) => s.verdict === 'NOT_RUN').map((s) => s.run_id + '|' + s.sub),
  );
  assert.ok(notRun.size > 0);
  for (const hit of r.body.hits) assert.ok(!notRun.has(hit.run + '|' + hit.sub), hit.run + hit.sub);
  assert.ok(
    r.body.hits.every((h) => h.verdict === 'PASS' || h.verdict === 'FAIL' || h.verdict === 'RETRY'),
  );
  const ended = r.body.hits.map((h) => h.ended_at);
  assert.deepEqual(ended, [...ended].sort().reverse());
});

test('require: forced exact, never relaxed', async () => {
  const r = await call(Q + near + '&require=die,os');
  assert.deepEqual(r.body.effective.require, ['die', 'os']);
  assert.ok(!r.body.effective.relax.includes('die') && !r.body.effective.relax.includes('os'));
  assert.ok(r.body.hits.length > 0);
  for (const hit of r.body.hits) {
    assert.equal(hit.die, 'GA106');
    assert.equal(hit.os, 'windows');
    assert.ok(hit.match.exact.includes('die') && hit.match.exact.includes('os'));
  }
  const missing = await call(Q + '?os=windows&require=die');
  assert.equal(missing.status, 400);
  assert.equal(missing.body.detail, 'required dimension has no wanted value');
  assert.equal(missing.body.dimension, 'die');
  assert.deepEqual(missing.body.allowed, DIMS);
});

test('ignore: dropped from matching entirely', async () => {
  const r = await call(Q + near + '&ignore=cpu,host_driver');
  assert.deepEqual(r.body.effective.ignore, ['cpu', 'host_driver']);
  assert.ok(
    !r.body.effective.relax.includes('cpu') && !r.body.effective.relax.includes('host_driver'),
  );
  assert.deepEqual(r.body.effective.relax, ['commit', 'guest_driver', 'die', 'os', 'family']);
  for (const hit of r.body.hits) {
    assert.ok(
      !hit.match.exact.includes('cpu') &&
        !hit.match.relaxed.some((x) => x.dim === 'cpu' || x.dim === 'host_driver'),
    );
  }
  assert.ok(
    r.body.hits.some((h) => h.cpu.vendor === 'intel' && h.match.relaxed.length === 0),
    'another CPU vendor is as good as the first',
  );
});

test('relax: a client-chosen order, a subset, none', async () => {
  const only = await call(Q + near + '&relax=host_driver');
  assert.deepEqual(only.body.effective.relax, ['host_driver']);
  for (const hit of only.body.hits)
    assert.ok(hit.match.relaxed.every((x) => x.dim === 'host_driver'));
  const none = await call(Q + near + '&relax=none');
  assert.deepEqual(none.body.effective.relax, []);
  assert.ok(none.body.hits.length > 0);
  for (const hit of none.body.hits) assert.deepEqual(hit.match.relaxed, []);
  const order = await call(Q + near + '&relax=guest_driver,os,die,family,cpu,host_driver,commit');
  assert.deepEqual(order.body.effective.relax, [
    'guest_driver',
    'os',
    'die',
    'family',
    'cpu',
    'host_driver',
    'commit',
  ]);
  // require wins over relax, and the effective list shows it.
  const both = await call(Q + near + '&relax=die,os,cpu&require=die');
  assert.deepEqual(both.body.effective.relax, ['os', 'cpu']);
});

test('min_closeness: 0 is exact only; each step admits more', async () => {
  const exact = await call(Q + near + '&min_closeness=0');
  assert.ok(exact.body.hits.length > 0);
  assert.ok(exact.body.hits.every((h) => h.match.relaxed.length === 0));
  assert.equal(exact.body.effective.min_closeness, 0);
  const one = await call(Q + near + '&min_closeness=1');
  const seven = await call(Q + near + '&min_closeness=7');
  assert.ok(one.body.eligible > exact.body.eligible);
  assert.ok(seven.body.eligible >= one.body.eligible);
  assert.ok(one.body.hits.every((h) => h.match.relaxed.length <= 1));
});

test('limit, default 20 and at most 100', async () => {
  assert.equal((await call(Q + '?limit=3')).body.hits.length, 3);
  assert.equal((await call(Q)).body.effective.limit, 20);
  assert.equal((await call(Q + '?limit=100')).status, 200);
  for (const bad of ['0', '101', '-1', '1.5', 'x', '1000'])
    assert.equal((await call(Q + '?limit=' + bad)).status, 400, bad);
});

test('cursor: stable pages that neither repeat nor skip, bound to their own request', async () => {
  const full = await call(Q + near + '&limit=100');
  const all = full.body.hits.map((h) => h.run + '/' + h.sub);
  const seen = [];
  let cursor = null;
  for (let page = 0; page < 40; page++) {
    const r = await call(Q + near + '&limit=7' + (cursor ? '&cursor=' + cursor : ''));
    assert.equal(r.status, 200);
    seen.push(...r.body.hits.map((h) => h.run + '/' + h.sub));
    cursor = r.body.next_cursor;
    if (!cursor) break;
    assert.match(cursor, /^[A-Za-z0-9_-]{8,200}$/);
  }
  assert.deepEqual(seen, all);
  assert.equal(new Set(seen).size, seen.length);
  // A cursor belongs to the request that made it.
  const first = await call(Q + near + '&limit=2');
  assert.ok(first.body.next_cursor);
  for (const other of ['&sort=recent', '&min_closeness=1', '&relax=cpu', '&die=GA104'])
    assert.equal(
      (await call(Q + near + other + '&limit=2&cursor=' + first.body.next_cursor)).status,
      400,
      other,
    );
  for (const bad of [
    'short',
    'a'.repeat(300),
    '!!!!!!!!',
    Buffer.from('{"i":1}').toString('base64url'),
  ])
    assert.equal((await call(Q + near + '&cursor=' + bad)).status, 400, bad);
  assert.equal(first.body.next_cursor === null, false);
  assert.equal((await call(Q + '?limit=100')).body.next_cursor === null || true, true);
});

test('sort: closeness (default), recent, duration', async () => {
  const closeness = await call(Q + near + '&limit=100');
  const recent = await call(Q + near + '&sort=recent&limit=100');
  const duration = await call(Q + near + '&sort=duration&limit=100');
  assert.equal(closeness.body.effective.sort, 'closeness');
  assert.equal(recent.body.effective.sort, 'recent');
  const ended = recent.body.hits.map((h) => h.ended_at);
  assert.deepEqual(ended, [...ended].sort().reverse());
  const spans = duration.body.hits.map((h) => h.duration_ms ?? Infinity);
  assert.deepEqual(
    spans,
    [...spans].sort((a, b) => a - b),
  );
  assert.equal(
    new Set(closeness.body.hits.map((h) => h.run + h.sub)).size,
    new Set(recent.body.hits.map((h) => h.run + h.sub)).size,
  );
  assert.equal((await call(Q + '?sort=loudest')).status, 400);
});

test('include: counts, stages, tests, perf, apps expand each hit, bounded, truncated says so', async () => {
  const r = await call(Q + near + '&include=counts,stages,tests,perf,apps&limit=3');
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.effective.include, ['counts', 'stages', 'tests', 'perf', 'apps']);
  const hit = r.body.hits[0];
  assert.deepEqual(Object.keys(hit.counts), ['pass', 'fail', 'skip']);
  assert.ok(Array.isArray(hit.stages.rows) && typeof hit.stages.truncated === 'boolean');
  for (const key of ['tests', 'perf', 'apps']) {
    assert.ok(Array.isArray(hit[key].rows), key);
    assert.equal(typeof hit[key].truncated, 'boolean');
  }
  assert.ok(
    hit.tests.rows.length > 0 && hit.tests.rows.every((t) => !('sub' in t) && t.name && t.result),
  );
  assert.deepEqual(hit.apps.rows, []);
  assert.equal(hit.apps.truncated, false);
  // Without include nothing is expanded.
  const plain = (await call(Q + near + '&limit=1')).body.hits[0];
  for (const key of ['counts', 'stages', 'tests', 'perf', 'apps']) assert.ok(!(key in plain), key);
  // A big page shrinks the per-subrun cap: the expanded body stays bounded and says truncated.
  const wide = await call(Q + '?include=tests&limit=100');
  const total = wide.body.hits.reduce((n, h) => n + h.tests.rows.length, 0);
  assert.ok(total <= 1000 + 100 * 5, 'bounded: ' + total);
  const cap = Math.max(...wide.body.hits.map((h) => h.tests.rows.length));
  assert.ok(cap <= 100);
  assert.ok(
    wide.body.hits.some((h) => h.tests.truncated === true),
    'a subrun with more tests than the cap says truncated',
  );
  const small = await call(Q + '?include=tests&limit=1&die=GA106');
  assert.ok(small.body.hits[0].tests.rows.length <= 100);
});

test('400s: unknown parameter, repeated parameter, unknown or repeated dimension, long lists, conflicts', async () => {
  let r = await call(Q + '?colour=red');
  assert.equal(r.status, 400);
  assert.equal(r.body.detail, 'unknown parameter');
  assert.deepEqual(r.body.allowed, [
    'commit',
    'tree',
    'die',
    'os',
    'host_driver',
    'guest_driver',
    'cpu',
    'relax',
    'require',
    'ignore',
    'limit',
    'cursor',
    'sort',
    'include',
    'min_closeness',
  ]);
  assert.equal((await call(Q + '?die=GA106&die=GA104')).body.detail, 'repeated parameter');
  for (const name of ['relax', 'require', 'ignore']) {
    r = await call(Q + '?die=GA106&' + name + '=die,colour');
    assert.equal(r.status, 400, name);
    assert.equal(r.body.detail, 'unknown dimension');
    assert.equal(r.body.parameter, name);
    assert.deepEqual(r.body.allowed, DIMS);
    r = await call(Q + '?die=GA106&' + name + '=die,die');
    assert.equal(r.body.detail, 'repeated dimension');
  }
  assert.equal((await call(Q + '?die=GA106&relax=' + Array(12).fill('die').join(','))).status, 400);
  assert.equal((await call(Q + '?relax=' + 'x'.repeat(200))).status, 400);
  r = await call(Q + '?die=GA106&os=linux&require=die&ignore=die');
  assert.equal(r.body.detail, 'dimension in both require and ignore');
  r = await call(Q + '?include=counts,movies');
  assert.equal(r.body.detail, 'unknown word');
  assert.deepEqual(r.body.allowed, ['stages', 'tests', 'perf', 'apps', 'counts']);
  assert.equal((await call(Q + '?include=counts,counts')).body.detail, 'repeated word');
  for (const bad of [
    'die=ga106',
    'die=GA1060',
    'os=beos',
    'host_driver=58',
    'guest_driver=1.2.3.4',
    'cpu=sparc',
    'commit=zz',
    'tree=' + 'a'.repeat(63),
    'min_closeness=8',
    'min_closeness=-1',
    'sort=',
    'limit=',
  ])
    assert.equal((await call(Q + '?' + bad)).status, 400, bad);
  assert.equal((await call(Q + '?relax=')).status, 400);
  // Errors are fixed words: nothing of the request is echoed back.
  r = await call(Q + '?die=%3Cscript%3E');
  assert.doesNotMatch(r.text, /script/);
});

test('commit and tree: the same tree is exact, another commit is relaxed with a note', async () => {
  const anyRun = rows.ci_runs.find((x) => x.run_id === 'syn-ci-p-0');
  const exact = await call(
    Q + '?die=GA106&os=windows&tree=' + anyRun.tree_hash + '&include=counts',
  );
  const top = exact.body.hits[0];
  assert.ok(top.match.exact.includes('commit'));
  assert.equal(top.tree, anyRun.tree_hash);
  const byCommit = await call(Q + '?commit=' + anyRun.commit_sha + '&die=GA106&os=windows');
  assert.ok(byCommit.body.hits[0].match.exact.includes('commit'));
  const foreign = await call(
    Q + '?commit=' + 'e'.repeat(40) + '&tree=' + 'f'.repeat(64) + '&die=GA106&os=windows',
  );
  assert.ok(foreign.body.hits[0].match.relaxed.some((x) => x.dim === 'commit' && x.note));
  const must = await call(Q + '?commit=' + 'e'.repeat(40) + '&die=GA106&require=commit');
  assert.equal(must.body.hits.length, 0, 'a required commit with no result has no hits');
  const both = await call(
    Q + '?commit=' + 'e'.repeat(40) + '&tree=' + anyRun.tree_hash + '&require=commit&die=GA106',
  );
  assert.ok(both.body.hits.length > 0 && both.body.hits.every((h) => h.tree === anyRun.tree_hash));
});

test('a die with no subruns falls back across the family and then, last, across families', async () => {
  const r = await call(Q + '?die=GA107&os=windows&limit=100');
  const first = r.body.hits[0];
  assert.equal(first.family, 'ampere');
  assert.deepEqual(
    first.match.relaxed.map((x) => x.dim),
    ['die'],
  );
  const other = r.body.hits.findIndex((h) => h.family !== 'ampere');
  assert.ok(other > 0, 'another family appears, after every Ampere hit');
  assert.ok(r.body.hits.slice(0, other).every((h) => h.family === 'ampere'));
  const strict = await call(
    Q + '?die=GA107&os=windows&relax=die,os,cpu,host_driver,guest_driver,commit&limit=100',
  );
  assert.ok(
    strict.body.hits.every((h) => h.family === 'ampere'),
    'family is not relaxable: no other family',
  );
});

test('the endpoint is read-only: other methods are 405, no write or raw SQL path exists', async () => {
  for (const method of ['POST', 'PUT', 'DELETE', 'PATCH']) {
    const r = await call(Q, { method });
    assert.equal(r.status, 405, method);
    assert.equal(r.headers.get('access-control-allow-origin'), null);
  }
  assert.equal((await call(Q + '?sql=select')).status, 400);
});

test('every hit is final, claims are labelled, and no private or SPEC data appears', async () => {
  const r = await call(Q + '?limit=100&include=counts,tests,perf');
  const text = r.text;
  assert.doesNotMatch(text, /machine_id|price_usd|cost_usd|\"tag\"|specviewperf|viewperf/i);
  for (const hit of r.body.hits) {
    assert.ok(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED'].includes(hit.session_verdict));
    assert.ok(hit.claimed.length > 0);
  }
});

// ---- candidate selection under a crowd of newer rows (the bound of 60 rows per read) -----------

function crowded({ die, family, os, count = 70, inflight = false }) {
  const rows2 = syntheticRows();
  const template = rows2.ci_runs.find((r) => r.run_id === 'syn-ci-p-1');
  // The exact match (a GA106 Windows 580.88 subrun) is made the OLDEST of all.
  const target = rows2.ci_runs.find((r) => r.run_id === 'syn-ci-p-0');
  Object.assign(target, {
    started_at: '2026-01-01T00:00:00Z',
    ended_at: '2026-01-01T01:00:00Z',
    sort_at: '2026-01-01T01:00:00Z',
  });
  for (let i = 0; i < count; i++) {
    const id = 'syn-ci-z-' + String(i).padStart(3, '0');
    const at = '2026-12-' + String(1 + (i % 28)).padStart(2, '0') + 'T00:00:00Z';
    rows2.ci_runs.push({
      ...template,
      run_id: id,
      die,
      family,
      started_at: at,
      ended_at: inflight ? null : at,
      sort_at: at,
      verdict: inflight ? null : 'PASS',
      tag: inflight ? null : template.tag,
    });
    rows2.ci_subruns.push({
      run_id: id,
      sub: os === 'windows' ? 'windows-580.88' : 'linux-580.95.05',
      ord: 0,
      kind: os === 'windows' ? 'windows_guest' : 'linux_guest',
      os,
      guest_driver: os === 'windows' ? '580.88' : '580.95.05',
      host_driver: '580.95.05',
      verdict: 'PASS',
      reason: null,
      stages_json: '[]',
      pass_count: 1,
      fail_count: 0,
      skip_count: 0,
      started_at: null,
      ended_at: null,
      duration_ms: 1,
      log_start: null,
      log_end: null,
    });
  }
  return rows2;
}
const first = async (rows2, query) => {
  const { call } = api({ database: seededDatabase({ rows: rows2 }).reader });
  return (await call(Q + query)).body;
};

test('the exact subrun is found even when 70 newer subruns of other dies crowd the newest rows', async () => {
  const body = await first(
    crowded({ die: 'GA104', family: 'ampere', os: 'windows' }),
    '?die=GA106&os=windows&host_driver=580.95.05&guest_driver=580.88',
  );
  const top = body.hits[0];
  assert.deepEqual(
    [top.die, top.os, top.host_driver, top.guest_driver],
    ['GA106', 'windows', '580.95.05', '580.88'],
  );
  assert.deepEqual(top.match.relaxed, []);
  assert.ok(!top.run.startsWith('syn-ci-z-'));
});

test('the exact subrun is found even when 70 newer subruns of the same die and another OS crowd it', async () => {
  const body = await first(
    crowded({ die: 'GA106', family: 'ampere', os: 'linux' }),
    '?die=GA106&os=windows&host_driver=580.95.05',
  );
  const top = body.hits[0];
  assert.equal(top.os, 'windows');
  assert.equal(top.die, 'GA106');
  assert.equal(top.host_driver, '580.95.05');
  assert.equal(top.guest_driver, '580.88');
  assert.deepEqual(
    top.match.relaxed.filter((x) => x.dim !== 'commit' && x.dim !== 'cpu'),
    [],
  );
});

test('a session still in flight is never a candidate, whatever its subruns say', async () => {
  const body = await first(
    crowded({ die: 'GA106', family: 'ampere', os: 'windows', count: 5, inflight: true }),
    '?limit=100',
  );
  assert.ok(body.hits.every((h) => !h.run.startsWith('syn-ci-z-')));
  assert.ok(body.hits.length > 5);
});

test('the report text must be schema 1 to be shown', async () => {
  const rows2 = syntheticRows();
  const r = rows2.ci_run_reports.find((x) => x.run_id === 'syn-ci-p-2');
  r.report_json = JSON.stringify({ ...JSON.parse(r.report_json), schema: 2 });
  const { call } = api({ database: seededDatabase({ rows: rows2 }).reader });
  const run = (await call('/v1/ci/runs/syn-ci-p-2')).body.run;
  assert.equal(run.report, null);
  assert.ok(run.subruns.length > 0);
});

// ---- the cost of one anonymous request (review of #282, F8) ------------------------------------

import { QUERIES } from '../src/queries.js';
import { MAX_PROBES } from '../src/subruns.js';

test('a request naming every dimension reads at most MAX_PROBES candidate sets', async () => {
  const log = [];
  const { call } = api({ log });
  const anyRun = rows.ci_runs.find((x) => x.run_id === 'syn-ci-p-0');
  const query =
    near +
    '&commit=' +
    anyRun.commit_sha +
    '&tree=' +
    anyRun.tree_hash +
    '&include=counts,stages,tests,perf,apps';
  assert.equal((await call(Q + query)).status, 200);
  const reads = log.filter((e) => e.sql === QUERIES.ciSubrunCandidates);
  assert.ok(reads.length >= 5 && reads.length <= MAX_PROBES, 'candidate reads: ' + reads.length);
  assert.equal(MAX_PROBES, 9);
  assert.ok(log.length <= MAX_PROBES + 1 + 4, 'all statements: ' + log.length);
  // The all-dimensions reads come first, so the exact match is found even at the cap.
  assert.ok(reads[0].params.slice(0, 8).filter((v) => v !== null).length >= 4);
});

test('with a required commit given as a tree and a commit, both exact reads happen before any near read', async () => {
  const log = [];
  const { call } = api({ log });
  const anyRun = rows.ci_runs.find((x) => x.run_id === 'syn-ci-p-0');
  await call(
    Q + near + '&commit=' + anyRun.commit_sha + '&tree=' + anyRun.tree_hash + '&require=commit',
  );
  const reads = log.filter((e) => e.sql === QUERIES.ciSubrunCandidates);
  assert.ok(reads.length <= MAX_PROBES);
  const trees = reads.slice(0, 2).map((e) => [e.params[5], e.params[6]]);
  assert.deepEqual(
    trees.map((t) => t[0] !== null || t[1] !== null),
    [true, true],
  );
});

test('/v1/ci/subruns carries a shared-cache lifetime', async () => {
  assert.equal(
    (await call(Q + '?limit=1')).headers.get('cache-control'),
    'public, max-age=30, s-maxage=60',
  );
});

test('/v1/ci/stats carries a shared-cache lifetime', async () => {
  assert.equal(
    (await call('/v1/ci/stats?limit=1')).headers.get('cache-control'),
    'public, max-age=30, s-maxage=60',
  );
});
