import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { api, seededDatabase, MIGRATION } from './helpers.js';
import { syntheticRows } from '../scripts/seed-data.mjs';
// Run reports, the log identity and stats on the public API (migration 0004). FALSIFIER: a run
// page without its subruns, report or log identity when they exist, or with invented ones when they
// do not; a report that carries the run's identity from anywhere but the row, a SPECviewperf datum,
// a token shape or an unbounded string; /v1/logs/<run>/log.txt that redirects for a run with no
// published log or off kayfabe.dev; stats that mix runs of different dies or drivers, show a
// non-final run, or return more than the limit; an old database (no 0004 tables) that takes the
// run page down; a way to write.
const rows = syntheticRows();
const BASE = 'https://logs.kayfabe.dev';
const withBase = () => api({ env: { LOGS_BASE: BASE } });
const reported = rows.ci_run_reports.find((r) => r.run_id === 'syn-ci-p-8');
const plain = rows.ci_runs.find(
  (r) => !rows.ci_run_reports.some((p) => p.run_id === r.run_id) && r.verdict === 'PASS',
);

test('a run with a report: subruns, report and log identity; the identity fields come from the row', async () => {
  const { call } = withBase();
  const r = await call('/v1/ci/runs/syn-ci-p-8');
  assert.equal(r.status, 200);
  const run = r.body.run;
  assert.deepEqual(
    run.subruns.map((s) => s.sub),
    [
      'host',
      'linux-580.95.05',
      'linux-575.64.05',
      'windows-580.88',
      'windows-576.80',
      'windows-572.83',
    ],
  );
  assert.ok(
    run.subruns.every((s) => ['host_bare', 'linux_guest', 'windows_guest'].includes(s.kind)),
  );
  assert.deepEqual(run.log, {
    bytes: reported.log_bytes,
    sha256: reported.log_sha256,
    url: BASE + '/syn-ci-p-8/log.txt',
  });
  assert.equal(run.report.schema, 1);
  assert.equal(run.report.run_id, 'syn-ci-p-8');
  assert.deepEqual(run.report.log, run.log);
  assert.equal(run.report.subruns.length, run.subruns.length);
  assert.ok(run.report.tests.length > 100 && run.report.perf.length > 5);
  assert.ok(run.report.perf.some((p) => p.ref_ratio !== null && p.higher_is_better === true));
  assert.deepEqual(run.report.apps, []);
  assert.equal(r.headers.get('cache-control'), 'public, max-age=31536000, immutable');
});

test('without LOGS_BASE the log identity has no url; the report still carries its own log without one', async () => {
  const { call } = api({ env: { LOGS_BASE: '' } });
  const r = await call('/v1/ci/runs/syn-ci-p-8');
  assert.deepEqual(Object.keys(r.body.run.log).sort(), ['bytes', 'sha256']);
  assert.ok(!('url' in r.body.run.report.log));
  const evil = api({ env: { LOGS_BASE: 'https://evil.test' } });
  assert.ok(!('url' in (await evil.call('/v1/ci/runs/syn-ci-p-8')).body.run.log));
});

test('a run without a report answers null and [], never invented rows', async () => {
  const { call } = withBase();
  const r = await call('/v1/ci/runs/' + plain.run_id);
  assert.equal(r.status, 200);
  assert.equal(r.body.run.report, null);
  assert.equal(r.body.run.log, null);
  assert.deepEqual(r.body.run.subruns, []);
  const running = rows.ci_runs.find((x) => x.verdict === null);
  const live = await call('/v1/ci/runs/' + running.run_id);
  assert.deepEqual(
    [live.body.run.report, live.body.run.log, live.body.run.subruns],
    [null, null, []],
  );
});

test('the stored report is re-bounded: schema, object, no script, run id and log from the row', async () => {
  const rows2 = syntheticRows();
  const target = rows2.ci_run_reports.find((r) => r.run_id === 'syn-ci-p-2');
  const doc = JSON.parse(target.report_json);
  doc.run_id = 'vast-forged-1-0';
  doc.log = { url: 'https://evil.test/x', bytes: 1, sha256: 'x' };
  target.report_json = JSON.stringify(doc);
  const bad = rows2.ci_run_reports.find((r) => r.run_id === 'syn-ci-p-0');
  bad.report_json = '{not json';
  const { call } = api({
    database: seededDatabase({ rows: rows2 }).reader,
    env: { LOGS_BASE: BASE },
  });
  const r = await call('/v1/ci/runs/syn-ci-p-2');
  assert.equal(r.body.run.report.run_id, 'syn-ci-p-2');
  assert.deepEqual(r.body.run.report.log, r.body.run.log);
  assert.doesNotMatch(r.text, /evil\.test/);
  const broken = await call('/v1/ci/runs/syn-ci-p-0');
  assert.equal(broken.status, 200);
  assert.equal(broken.body.run.report, null);
  assert.ok(broken.body.run.subruns.length > 0, 'the subruns are rows, not the report');
});

test('strings out of the database are bounded again (a hostile row cannot reach a page)', async () => {
  const rows2 = syntheticRows();
  const s = rows2.ci_subruns.find((x) => x.run_id === 'syn-ci-p-2' && x.os === 'windows');
  s.reason = 'visit https://evil.test/' + 'x'.repeat(100);
  const t = rows2.ci_run_tests.find((x) => x.run_id === 'syn-ci-p-2');
  t.name = 'SPECviewperf maya 12.5';
  const { call } = api({ database: seededDatabase({ rows: rows2 }).reader });
  const r = await call('/v1/ci/subruns?die=GA106&os=windows&include=tests&limit=100');
  assert.doesNotMatch(r.text, /evil\.test|SPECviewperf/i);
  const run = await call('/v1/ci/runs/syn-ci-p-2');
  assert.equal(run.body.run.subruns.find((x) => x.os === 'windows').reason, null);
});

test('/v1/logs/<run>/log.txt and report.json redirect for a published run only', async () => {
  const { call } = withBase();
  const log = await call('/v1/logs/syn-ci-p-8/log.txt');
  assert.equal(log.status, 302);
  assert.equal(log.headers.get('location'), BASE + '/syn-ci-p-8/log.txt');
  assert.equal(log.headers.get('access-control-allow-origin'), '*');
  const rep = await call('/v1/logs/syn-ci-p-8/report.json');
  assert.equal(rep.status, 302);
  assert.equal(rep.headers.get('location'), BASE + '/syn-ci-p-8/report.json');
  for (const path of [
    '/v1/logs/' + plain.run_id + '/log.txt',
    '/v1/logs/' + plain.run_id + '/report.json',
    '/v1/logs/vast-unknown-1-0/log.txt',
    '/v1/logs/syn-ci-p-8/other.json',
    '/v1/logs/syn-ci-p-8/..%2Fx.txt',
  ])
    assert.equal([404, 400].includes((await call(path)).status), true, path);
  const none = api({ env: { LOGS_BASE: '' } });
  assert.equal((await none.call('/v1/logs/syn-ci-p-8/log.txt')).status, 404);
  const wrong = api({ env: { LOGS_BASE: 'https://evil.test' } });
  assert.equal((await wrong.call('/v1/logs/syn-ci-p-8/log.txt')).status, 404);
  assert.equal((await call('/v1/logs/syn-ci-p-8/log.txt', { method: 'POST' })).status, 405);
});

test('the legacy /v1/logs path for the 0001 runs still works', async () => {
  const legacy = rows.runs.find((r) => r.logs_json && JSON.parse(r.logs_json).length);
  if (!legacy) return;
  const { call } = withBase();
  const name = JSON.parse(legacy.logs_json)[0];
  const r = await call('/v1/logs/' + legacy.id + '/' + name);
  assert.equal(r.status, 302);
  assert.equal(r.headers.get('location'), BASE + '/runs/' + legacy.id + '/' + name);
});

test('an old database (no 0004 tables) still answers the run page', () => {
  const old = new DatabaseSync(':memory:');
  old.exec(MIGRATION.split('-- kayfabe-platform D1 schema, version 4')[0]);
  old.exec(
    "INSERT INTO ci_runs (run_id, commit_sha, tree_hash, tree_kind, tier, profile, family, die, platform, verdict, started_at, ended_at, sort_at, recorded_at) VALUES ('vast-1-1-0', '" +
      'a'.repeat(40) +
      "', '" +
      'b'.repeat(64) +
      "', 'tree', 'middle', 'check', 'ada', 'AD104', 'both', 'PASS', '2026-11-01T00:00:00Z', '2026-11-01T01:00:00Z', '2026-11-01T01:00:00Z', '2026-11-01T01:00:00Z')",
  );
  const { call } = api({ database: old });
  return call('/v1/ci/runs/vast-1-1-0').then(async (r) => {
    assert.equal(r.status, 200);
    assert.deepEqual([r.body.run.report, r.body.run.log, r.body.run.subruns], [null, null, []]);
    assert.equal((await call('/v1/logs/vast-1-1-0/log.txt')).status, 404);
    assert.equal(
      (await call('/v1/ci/stats')).status,
      500,
      'the new endpoint needs the migration (fixed 500)',
    );
  });
});

// ---- stats -------------------------------------------------------------------------------------

test('stats: the latest reported final session per die and host driver, with counts and perf', async () => {
  const { call } = withBase();
  const r = await call('/v1/ci/stats');
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.effective, {
    die: null,
    tier: null,
    driver: null,
    os: null,
    limit: 20,
    sort: 'recent',
  });
  const keys = r.body.groups.map((g) => g.die + '/' + g.host_driver);
  assert.equal(new Set(keys).size, keys.length, 'one group per die and driver');
  assert.ok(r.body.groups.length >= 4);
  for (const g of r.body.groups) {
    const final = rows.ci_runs.find((x) => x.run_id === g.run);
    assert.ok(final.verdict !== null, 'final only');
    assert.equal(g.die, final.die);
    assert.equal(g.session, '/v1/ci/runs/' + g.run);
    const mine = rows.ci_subruns.filter((s) => s.run_id === g.run);
    assert.equal(
      g.counts.pass,
      mine.reduce((n, s) => n + s.pass_count, 0),
    );
    assert.equal(
      g.counts.fail,
      mine.reduce((n, s) => n + s.fail_count, 0),
    );
    assert.equal(g.subruns.length, mine.length);
    assert.ok(g.perf.rows.length <= 50);
    assert.deepEqual(g.claimed, ['host_driver', 'perf', 'counts']);
  }
  // The newest reported run of the group wins.
  const ga106 = r.body.groups.filter((g) => g.die === 'GA106' && g.host_driver === '580.95.05');
  assert.equal(ga106.length, 1);
  const candidates = rows.ci_runs.filter(
    (x) =>
      x.die === 'GA106' &&
      x.verdict !== null &&
      rows.ci_run_reports.some((p) => p.run_id === x.run_id && p.host_driver === '580.95.05'),
  );
  const newest = candidates.sort((a, b) =>
    a.sort_at < b.sort_at ? 1 : a.sort_at > b.sort_at ? -1 : a.run_id < b.run_id ? 1 : -1,
  )[0];
  assert.equal(ga106[0].run, newest.run_id);
});

test('stats filters: die, tier, driver, os; limit; sort; every filter is stated back', async () => {
  const { call } = withBase();
  const die = await call('/v1/ci/stats?die=GA106');
  assert.ok(die.body.groups.length >= 1 && die.body.groups.every((g) => g.die === 'GA106'));
  assert.equal(die.body.effective.die, 'GA106');
  const drv = await call('/v1/ci/stats?driver=575.64.05');
  assert.ok(drv.body.groups.every((g) => g.host_driver === '575.64.05'));
  const tier = await call('/v1/ci/stats?tier=largest');
  assert.ok(tier.body.groups.every((g) => g.tier === 'largest'));
  const lin = await call('/v1/ci/stats?os=linux');
  assert.ok(lin.body.groups.length > 0);
  for (const g of lin.body.groups) assert.ok(g.subruns.every((s) => s.os === 'linux'));
  const host = await call('/v1/ci/stats?os=host');
  for (const g of host.body.groups)
    assert.deepEqual(
      [...new Set(g.perf.rows.map((p) => p.suite))].filter((x) => x !== 'cuda_host'),
      [],
    );
  const win = await call('/v1/ci/stats?os=windows');
  assert.ok(win.body.groups.length > 0);
  for (const g of win.body.groups)
    assert.ok(
      g.subruns.length > 0 &&
        g.subruns.every((s) => s.os === 'windows') &&
        g.perf.rows.length === 0,
    );
  const linuxOnly = rows.ci_runs
    .filter((x) => x.platform === 'linux' && rows.ci_run_reports.some((p) => p.run_id === x.run_id))
    .map((x) => x.run_id);
  assert.ok(linuxOnly.length > 0);
  const unfiltered = await call('/v1/ci/stats?limit=100');
  assert.ok(unfiltered.body.groups.length >= win.body.groups.length);
  assert.ok(
    !win.body.groups.some((g) => linuxOnly.includes(g.run)),
    'a session with no Windows subrun has no Windows stats',
  );
  assert.equal((await call('/v1/ci/stats?limit=2')).body.groups.length, 2);
  const byDie = await call('/v1/ci/stats?sort=die&limit=100');
  const dies = byDie.body.groups.map((g) => g.die);
  assert.deepEqual(dies, [...dies].sort());
  const byDriver = await call('/v1/ci/stats?sort=driver&limit=100');
  const drivers = byDriver.body.groups.map((g) => g.host_driver);
  assert.deepEqual(drivers, [...drivers].sort());
  const recent = await call('/v1/ci/stats?sort=recent&limit=100');
  const at = recent.body.groups.map((g) => g.ended_at);
  assert.deepEqual(at, [...at].sort().reverse());
  for (const bad of [
    'die=x',
    'tier=huge',
    'driver=1',
    'os=beos',
    'limit=0',
    'limit=101',
    'sort=loud',
    'colour=1',
    'die=GA106&die=GA104',
  ])
    assert.equal((await call('/v1/ci/stats?' + bad)).status, 400, bad);
  const unknown = await call('/v1/ci/stats?colour=1');
  assert.deepEqual(unknown.body.allowed, ['die', 'tier', 'driver', 'os', 'limit', 'sort']);
  assert.equal((await call('/v1/ci/stats', { method: 'POST' })).status, 405);
});

test('stats never shows an in-flight run, a run without a report, or a SPECviewperf row', async () => {
  const { call } = withBase();
  const r = await call('/v1/ci/stats?limit=100');
  for (const g of r.body.groups)
    assert.ok(rows.ci_run_reports.some((p) => p.run_id === g.run && p.report_json !== null));
  assert.doesNotMatch(r.text, /specviewperf|viewperf/i);
  assert.ok(!r.body.groups.some((g) => g.run === plain.run_id));
});

test('/v1/schema documents the new endpoints and their parameters', async () => {
  const { call } = withBase();
  const r = await call('/v1/schema');
  const sub = r.body.endpoints.find((e) => e.path === '/v1/ci/subruns');
  assert.deepEqual(Object.keys(sub.params), [
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
  assert.ok(r.body.endpoints.some((e) => e.path === '/v1/ci/stats'));
  assert.match(
    r.body.endpoints.find((e) => e.path === '/v1/logs/{run}/{name}').about,
    /report\.json/,
  );
});

test('stats cap the perf rows of a group at 50 and say truncated', async () => {
  const rows2 = syntheticRows();
  const target = rows2.ci_run_perf.filter((p) => p.run_id === 'syn-ci-p-8');
  const last = target[target.length - 1];
  for (let i = 0; i < 70; i++)
    rows2.ci_run_perf.push({ ...last, ord: last.ord + 1 + i, name: 'extra-' + i });
  const { call } = api({ database: seededDatabase({ rows: rows2 }).reader });
  const group = (await call('/v1/ci/stats?die=GA104&driver=580.95.05')).body.groups.find(
    (g) => g.run === 'syn-ci-p-8',
  );
  assert.equal(group.perf.rows.length, 50);
  assert.equal(group.perf.truncated, true);
  const small = (await call('/v1/ci/stats?die=GA106')).body.groups[0];
  assert.equal(small.perf.truncated, false);
});
