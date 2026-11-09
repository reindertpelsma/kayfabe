import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setup, nonce, sha, callbackToken, env as baseEnv } from './helpers.js';
import { FakeD1, d1Skip, migrations } from './d1.js';
import { createGateway } from '../src/index.js';
import { routeKind } from '../src/routes.js';
import { Ledger } from '../src/ledger.js';
import { publishTick, MAX_ATTEMPTS, logsBase } from '../src/publish.js';
import { publishTimeouts } from '../src/publiclog.js';
import {
  REPORT_MAX_BYTES,
  LIMITS,
  buildReport,
  cleanClaimsReport,
  fitReport,
} from '../src/runreport.js';
import { syntheticRows } from '../../platform-api/scripts/seed-data.mjs';
// Run report and full public log, end to end. FALSIFIER: a report, a log line or a storage fault
// changes a verdict, a deadline, the budget or the teardown; the box can set the identity of a run
// (id, commit, tier, die, verdict) or a SPECviewperf datum, a token-shaped string or an unknown key
// reaches the stored report; a second report replaces the first; a run without the live token, the
// result store or the bucket gets anything but a fixed refusal; the log is published unredacted,
// truncated below the cap, twice, or not at all when only D1 fails; a bucket or database outage is
// louder than one log line or delays the sweep.
const TAG_KEY = 't'.repeat(48);
const hex = (n) => n.toString(16).padStart(40, '0');
const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };
const passSuites = ['provision', 'gates', 'raw_suite'].map((name) => ({
  name,
  pass: 1,
  fail: 0,
  skip: 0,
}));

class Bucket {
  objects = new Map();
  puts = [];
  fail = false;
  hang = false;
  async put(key, value, options = {}) {
    this.puts.push(key);
    if (this.hang) return new Promise(() => {});
    if (this.fail) throw new Error('R2 down: secret-bucket-detail');
    this.objects.set(key, {
      text: typeof value === 'string' ? value : Buffer.from(value).toString(),
      options,
    });
    return { key };
  }
  text(key) {
    return this.objects.get(key)?.text;
  }
}

async function harness({ store = true, logs = true, bucket = new Bucket() } = {}) {
  const db = store ? new FakeD1() : null;
  const extra = {
    ...(store ? { DB: db, ROW_TAG_KEY: TAG_KEY } : {}),
    ...(logs ? { LOGS_BUCKET: bucket } : {}),
  };
  const h = await setup(extra);
  const base = h.cloud.fetch;
  h.cloud.fetch = async (input, init = {}) => {
    const u = new URL(String(input));
    if (u.hostname === 'api.github.com') {
      const p = u.pathname.replace(/^\/repos\/[^/]+\/[^/]+\//, '');
      if (/^git\//.test(p) || /^contents\/ci\/requirements\.toml$/.test(p)) {
        if (p === 'git/commits/' + sha) return Response.json({ tree: { sha: hex(100) } });
        if (p === 'git/trees/' + hex(100))
          return Response.json({
            truncated: false,
            tree: [{ path: 'Cargo.toml', mode: '100644', type: 'blob', sha: hex(1) }],
          });
        return new Response('', { status: 404 });
      }
    }
    return base(input, init);
  };
  h.restart();
  return { h, db, bucket };
}
const cron = async (h, n = 1) => {
  for (let i = 0; i < n; i++) {
    await h.service.sweep();
    await h.service.storeTrees();
  }
};
async function postReport(h, body, { token, run = '123-1', raw } = {}) {
  const record = await h.storage.get('record:' + run);
  return h.service.fetch(
    new Request(h.bindings.WORKER_URL + '/run-report/' + run, {
      method: 'POST',
      headers: {
        Authorization: 'Bearer ' + (token ?? callbackToken(record, h.bindings.RUN_TOKEN_SECRET)),
        'Content-Type': 'application/json',
      },
      body: raw ?? JSON.stringify(body),
    }),
  );
}
const goodReport = (patch = {}) => ({
  schema: 1,
  guest_drivers: { linux: '580.159.03', windows: '580.88' },
  stages: [
    { name: 'provision', result: 'PASS', rc: 0, duration_ms: 1000 },
    { name: 'merge_bar', result: 'PASS', rc: 0, duration_ms: 2000 },
  ],
  suites: [{ name: 'gates', pass: 9, fail: 0, skip: 0 }],
  subruns: [
    {
      sub: 'host',
      kind: 'host_bare',
      os: 'host',
      guest_driver: null,
      host_driver: '580.159.04',
      verdict: 'PASS',
      reason: null,
      stages: ['cuda_host', 'raw_suite'],
      log_stages: ['cuda_host'],
      counts: { pass: 34, fail: 0, skip: 0 },
      duration_ms: 700,
    },
    {
      sub: 'linux-580.159.03',
      kind: 'linux_guest',
      os: 'linux',
      guest_driver: '580.159.03',
      host_driver: '580.159.04',
      verdict: 'PASS',
      stages: ['thin_suite'],
      log_stages: ['merge_bar'],
      counts: { pass: 30, fail: 0, skip: 0 },
    },
    {
      sub: 'windows-580.88',
      kind: 'windows_guest',
      os: 'windows',
      guest_driver: '580.88',
      host_driver: '580.159.04',
      verdict: 'NOT_RUN',
      stages: ['windows_gate'],
      log_stages: ['windows_gate'],
      counts: { pass: 0, fail: 0, skip: 0 },
    },
  ],
  tests: [
    { sub: 'host', suite: 'raw_suite', name: 'timer', result: 'PASS', ms: 120 },
    { sub: 'linux-580.159.03', suite: 'thin_suite', name: 'timer', result: 'PASS', ms: 9000 },
  ],
  apps: [],
  perf: [
    {
      sub: 'host',
      name: 'cup8bench N=2048 gflops',
      value: 2500.5,
      unit: 'GFLOPS',
      higher_is_better: true,
      suite: 'cuda_host',
      ref_value: null,
      ref_ratio: null,
    },
    {
      sub: 'linux-580.159.03',
      name: 'cup8bench N=2048 gflops',
      value: 2400,
      unit: 'GFLOPS',
      higher_is_better: true,
      suite: 'cuda_fat',
      ref_value: 2500.5,
      ref_ratio: 0.96,
    },
  ],
  windows: { gate: null, install_s: null, probes: [], resets: null, live_dumps_193: null },
  ...patch,
});

// ---- the validator ---------------------------------------------------------------------------

test('cleanClaimsReport: shape kept, unknown keys dropped, identity never taken from the box', () => {
  const out = cleanClaimsReport({
    ...goodReport(),
    run_id: 'vast-9-9-9',
    commit: 'f'.repeat(40),
    verdict: 'PASS',
    die: 'GB202',
    extra: { deep: true },
  });
  assert.deepEqual(Object.keys(out).sort(), [
    'apps',
    'guest_drivers',
    'perf',
    'schema',
    'stages',
    'subruns',
    'suites',
    'tests',
    'windows',
  ]);
  assert.equal(out.subruns.length, 3);
  assert.equal(out.tests.length, 2);
  assert.equal(cleanClaimsReport({ ...goodReport(), schema: 2 }), null);
  assert.equal(cleanClaimsReport([]), null);
  assert.equal(cleanClaimsReport(null), null);
});

test('cleanClaimsReport: rows of an unknown subrun, bad enums and non-finite numbers are dropped', () => {
  const r = goodReport();
  r.tests.push(
    { sub: 'nope', suite: 'raw_suite', name: 'x', result: 'PASS', ms: 1 },
    { sub: 'host', suite: 'raw_suite', name: 'y', result: 'MAYBE', ms: 1 },
    { sub: 'host', suite: 'Raw Suite', name: 'z', result: 'PASS', ms: 1 },
    { sub: 'host', suite: 'raw_suite', name: 'ok', result: 'SKIP', ms: -4 },
  );
  r.perf.push(
    {
      sub: 'host',
      name: 'inf',
      value: Infinity,
      unit: 'ms',
      higher_is_better: false,
      suite: 'cuda_host',
    },
    {
      sub: 'host',
      name: 'nan',
      value: 'NaN',
      unit: 'ms',
      higher_is_better: false,
      suite: 'cuda_host',
    },
    {
      sub: 'host',
      name: 'big',
      value: 1e300,
      unit: 'ms',
      higher_is_better: false,
      suite: 'cuda_host',
    },
    {
      sub: 'host',
      name: 'unit',
      value: 1,
      unit: 'a very long unit string',
      higher_is_better: false,
      suite: 'cuda_host',
    },
  );
  const out = cleanClaimsReport(r);
  assert.deepEqual(
    out.tests.map((t) => t.name),
    ['timer', 'timer', 'ok'],
  );
  assert.equal(out.tests.at(-1).ms, null);
  assert.equal(out.perf.length, 2);
  // JSON cannot carry Infinity or NaN, but a hostile box can send strings and huge numbers.
  assert.ok(out.perf.every((p) => Number.isFinite(p.value)));
});

test('cleanClaimsReport: SPECviewperf, tokens, URLs and identifiers never survive in any string', () => {
  const r = goodReport();
  r.tests.push(
    {
      sub: 'host',
      suite: 'raw_suite',
      name: 'SPECviewperf 2020 maya score',
      result: 'PASS',
      ms: 1,
    },
    { sub: 'host', suite: 'raw_suite', name: 'eyJhbGciOiJIUzI1NiJ9abcdef', result: 'PASS', ms: 1 },
    { sub: 'host', suite: 'raw_suite', name: 'see https://x.test/a', result: 'PASS', ms: 1 },
    { sub: 'host', suite: 'raw_suite', name: 'a'.repeat(40), result: 'PASS', ms: 1 },
    {
      sub: 'host',
      suite: 'raw_suite',
      name: 'fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001',
      result: 'PASS',
      ms: 1,
    },
  );
  r.perf.push({
    sub: 'host',
    name: 'viewperf maya',
    value: 3.2,
    unit: 'fps',
    higher_is_better: true,
    suite: 'cuda_host',
  });
  r.subruns[2].reason = 'see https://evil.test';
  r.suites.push({ name: 'specviewperf', pass: 1, fail: 0, skip: 0 });
  const out = cleanClaimsReport(r);
  assert.deepEqual(
    out.tests.map((t) => t.name),
    ['timer', 'timer'],
  );
  assert.equal(out.perf.length, 2);
  assert.equal(out.subruns[2].reason, 'not reported');
  assert.deepEqual(
    out.suites.map((s) => s.name),
    ['gates'],
  );
  assert.doesNotMatch(JSON.stringify(out), /specviewperf|viewperf|eyJ|https?:|[0-9a-f]{40}/i);
});

test('cleanClaimsReport: non-ASCII becomes ?, strings are bounded, arrays are capped', () => {
  const r = goodReport();
  r.tests = Array.from({ length: LIMITS.tests + 50 }, (_, i) => ({
    sub: 'host',
    suite: 'raw_suite',
    name: 'arm-' + i + '-é☃',
    result: 'PASS',
    ms: i,
  }));
  r.perf = Array.from({ length: LIMITS.perf + 9 }, (_, i) => ({
    sub: 'host',
    name: 'p' + i,
    value: i,
    unit: 'ms',
    higher_is_better: false,
    suite: 'cuda_host',
  }));
  r.subruns[0].reason = 'x'.repeat(5000);
  r.windows.probes = Array.from({ length: 30 }, (_, i) => 'probe' + i);
  const out = cleanClaimsReport(r);
  assert.equal(out.tests.length, LIMITS.tests);
  assert.equal(out.perf.length, LIMITS.perf);
  assert.match(out.tests[3].name, /^arm-3-\?\?$/);
  assert.equal(out.subruns[0].reason.length, 200);
  assert.equal(out.windows.probes.length, LIMITS.probes);
});

test('cleanClaimsReport: a repeated id, a verdict outside the vocabulary and a missing kind are dropped', () => {
  const r = goodReport();
  r.subruns.push(
    { ...r.subruns[0] },
    { ...r.subruns[0], sub: 'host-2' },
    { ...r.subruns[0], verdict: 'GREAT' },
  );
  assert.deepEqual(
    cleanClaimsReport(r).subruns.map((s) => s.sub),
    ['host', 'linux-580.159.03', 'windows-580.88'],
  );
});

test('cleanClaimsReport: a subrun whose OS contradicts its kind is dropped', () => {
  const r = goodReport();
  r.subruns = [{ ...r.subruns[0], os: 'linux' }, r.subruns[1]];
  assert.deepEqual(
    cleanClaimsReport(r).subruns.map((s) => s.sub),
    ['linux-580.159.03'],
  );
});

test('cleanClaimsReport: a subrun whose id names another OS than its kind is dropped', () => {
  const r = goodReport();
  const [host, linux] = r.subruns;
  r.subruns = [{ ...host, sub: 'host', kind: 'linux_guest', os: 'linux' }, linux];
  assert.deepEqual(
    cleanClaimsReport(r).subruns.map((s) => s.sub),
    ['linux-580.159.03'],
  );
  r.subruns = [{ ...linux, sub: 'windows-580.88' }, host];
  assert.deepEqual(
    cleanClaimsReport(r).subruns.map((s) => s.sub),
    ['host'],
  );
});

test("cleanClaimsReport: the driver in the id is the subrun's guest driver", () => {
  const r = goodReport();
  r.subruns = [{ ...r.subruns[1], sub: 'linux-570.1.1' }, r.subruns[0]];
  assert.deepEqual(
    cleanClaimsReport(r).subruns.map((s) => s.sub),
    ['host'],
  );
});

test('cleanClaimsReport: NOT_RUN always carries a reason', () => {
  const r = goodReport();
  const out = cleanClaimsReport(r);
  assert.equal(out.subruns[2].reason, 'not reported');
  r.subruns[2].reason = 'Windows stage not requested for this profile';
  assert.equal(
    cleanClaimsReport(r).subruns[2].reason,
    'Windows stage not requested for this profile',
  );
});

test('fitReport: stays within 256 KiB by halving tests, apps and perf, and says so', () => {
  const r = goodReport();
  r.tests = Array.from({ length: 2000 }, (_, i) => ({
    sub: 'host',
    suite: 'raw_suite',
    name: 'arm-' + String(i).padStart(4, '0') + '-' + 'n'.repeat(90),
    result: 'PASS',
    ms: i,
  }));
  r.apps = Array.from({ length: 500 }, (_, i) => ({
    sub: 'host',
    name: 'app-' + i + '-' + 'a'.repeat(100),
    os: 'linux',
    result: 'PASS',
    note: 'n'.repeat(190),
  }));
  const claims = cleanClaimsReport(r);
  assert.ok(new TextEncoder().encode(JSON.stringify(claims)).length > REPORT_MAX_BYTES);
  const fitted = fitReport(claims);
  assert.ok(new TextEncoder().encode(JSON.stringify(fitted)).length <= REPORT_MAX_BYTES);
  assert.ok(fitted.truncated.includes('tests') || fitted.truncated.includes('apps'));
  assert.equal(fitReport(cleanClaimsReport(goodReport())).truncated, undefined);
});

test('buildReport: the Worker sets the identity and finds each subrun in the published log', () => {
  const claims = cleanClaimsReport(goodReport());
  const offsets = new Map([
    ['cuda_host', { start_byte: 10, end_byte: 90 }],
    ['merge_bar', { start_byte: 100, end_byte: 400 }],
  ]);
  const doc = buildReport({
    claims,
    row: {
      commit_sha: sha,
      tree_hash: 'c'.repeat(64),
      tree_kind: 'tree',
      tier: 'middle',
      die: 'AD104',
      family: 'ada',
      gpu_name: 'RTX_4070',
      host_driver: '580.159.04',
      verdict: 'PASS',
      started_at: '2026-11-07T00:00:00Z',
      ended_at: '2026-11-07T01:00:00Z',
    },
    runId: 'vast-123-1-0',
    log: { bytes: 500, sha256: 'd'.repeat(64) },
    offsets,
    logsBase: 'https://logs.kayfabe.dev',
  });
  assert.equal(doc.run_id, 'vast-123-1-0');
  assert.equal(doc.commit, sha);
  assert.equal(doc.tree, 'c'.repeat(64));
  assert.equal(doc.verdict, 'PASS');
  assert.deepEqual(doc.subruns[0].log_range, { start_byte: 10, end_byte: 90 });
  assert.deepEqual(doc.subruns[1].log_range, { start_byte: 100, end_byte: 400 });
  assert.equal(doc.subruns[2].log_range, null, 'no marker, no range, never a guessed one');
  assert.ok(doc.subruns.every((s) => !('log_stages' in s) && !('ord' in s)));
  assert.equal(doc.subruns[0].duration_ms, 700);
  assert.deepEqual(doc.log, {
    url: 'https://logs.kayfabe.dev/vast-123-1-0/log.txt',
    bytes: 500,
    sha256: 'd'.repeat(64),
  });
  const commitOnly = buildReport({
    claims,
    row: {
      commit_sha: sha,
      tree_hash: 'c'.repeat(64),
      tree_kind: 'commit',
      tier: 'small',
      die: 'AD104',
      family: 'ada',
      verdict: 'FAIL',
      started_at: 'a',
      ended_at: 'b',
    },
    runId: 'vast-1-1-0',
    log: null,
    offsets: null,
    logsBase: null,
  });
  assert.equal(commitOnly.tree, null);
  assert.equal(commitOnly.log, null);
  assert.equal(commitOnly.verdict, 'FAIL', "the verdict is the row's, whatever the box claimed");
  assert.equal(commitOnly.tier, 'small');
  assert.equal(commitOnly.run_id, 'vast-1-1-0');
});

test('logsBase accepts only a https origin on kayfabe.dev', () => {
  assert.equal(logsBase({}), 'https://logs.kayfabe.dev');
  assert.equal(logsBase({ LOGS_BASE: 'https://logs.kayfabe.dev/' }), 'https://logs.kayfabe.dev');
  for (const bad of [
    'http://logs.kayfabe.dev',
    'https://evil.test',
    'https://kayfabe.dev.evil.test',
    'https://u:p@logs.kayfabe.dev',
    'https://logs.kayfabe.dev/x',
    'https://logs.kayfabe.dev:8443',
    'nonsense',
  ])
    assert.equal(logsBase({ LOGS_BASE: bad }), null, bad);
});

// ---- the route -------------------------------------------------------------------------------

test('route: stored once, per rental, with the live run token only', { skip: d1Skip }, async () => {
  const { h } = await harness();
  await h.call();
  const bad = await postReport(h, goodReport(), { token: 'x.y.z' });
  assert.equal(bad.status, 401);
  const other = await postReport(h, goodReport(), {
    token: callbackToken({ ...(await h.storage.get('record:123-1')), nonce: 'e'.repeat(32) }),
  });
  assert.equal(other.status, 401);
  const ok = await postReport(h, goodReport());
  assert.equal(ok.status, 200);
  assert.equal((await ok.json()).state, 'STORED');
  const again = await postReport(h, goodReport());
  assert.equal(again.status, 409);
  assert.equal((await again.json()).state, 'REPORT_EXISTS');
  const record = await h.storage.get('record:123-1');
  assert.equal(record.runReport.chunks, 1);
  assert.equal(record.verdict, null, 'a report decides nothing');
  assert.equal(record.closed, null);
});

test(
  'route: refusals are fixed words and a bad body costs the rental nothing',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const before = JSON.stringify(await h.storage.get('record:123-1'));
    for (const [raw, status, state] of [
      ['not json', 400, 'BAD_REQUEST'],
      ['[]', 400, 'BAD_REQUEST'],
      [JSON.stringify({ schema: 2 }), 400, 'BAD_REQUEST'],
      ['x'.repeat(REPORT_MAX_BYTES + 1), 413, 'PAYLOAD_TOO_LARGE'],
    ]) {
      const r = await postReport(h, null, { raw });
      assert.equal(r.status, status, raw.slice(0, 20));
      assert.equal((await r.json()).state, state);
    }
    const after = await h.storage.get('record:123-1');
    assert.equal(after.runReport, undefined);
    assert.equal(after.verdict, null);
    assert.equal(after.closed, null);
    assert.ok(JSON.stringify(after).length >= before.length - 10);
    // After the failure the box may still send a good one.
    assert.equal((await postReport(h, goodReport())).status, 200);
  },
);

test('route: without a result store the route answers 404 REPORTS_DISABLED and the env var is absent', async () => {
  const { h } = await harness({ store: false });
  await h.call();
  const r = await postReport(h, goodReport());
  assert.equal(r.status, 404);
  assert.equal((await r.json()).state, 'REPORTS_DISABLED');
  const create = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
  assert.ok(!('KF_RUN_REPORT' in create.body.env));
  assert.equal(create.body.env.KF_HOST_CLAIMS, '1');
});

test('route: a closed, ended or expired rental is refused', { skip: d1Skip }, async () => {
  const { h } = await harness();
  await h.call();
  const record = await h.storage.get('record:123-1');
  const token = callbackToken(record);
  await h.storage.put('record:123-1', { ...record, closed: 'STOPPED' });
  assert.equal((await postReport(h, goodReport(), { token })).status, 401, 'closed');
  await h.storage.put('record:123-1', { ...record, ended: h.time.value });
  assert.equal((await postReport(h, goodReport(), { token })).status, 401, 'ended');
  await h.storage.put('record:123-1', record);
  h.time.value += 3 * 3600 * 1000;
  assert.equal((await postReport(h, goodReport(), { token })).status, 401, 'expired');
});

test(
  'KF_RUN_REPORT=1 reaches the box only from a Worker that has the result store',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const create = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
    assert.equal(create.body.env.KF_RUN_REPORT, '1');
    assert.match(
      readFileSync(new URL('../../../scripts/ci/vast_report.py', import.meta.url), 'utf8'),
      /"KF_RUN_REPORT", False/,
    );
  },
);

test('gateway: run-report is a box route and reaches the ledger with its path', async () => {
  assert.equal(routeKind('/run-report/123-1'), 'box');
  assert.equal(routeKind('/run-report/123-1/x'), null);
  assert.equal(routeKind('/run-report/abc'), null);
  const gateway = createGateway({
    authorize: async () => {},
    fetcher: async () => new Response(''),
  });
  const seen = [];
  const env = {
    ...baseEnv,
    BROKER_PATH_PREFIX: 'p'.repeat(40),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({ fetch: async (r) => (seen.push(new URL(r.url).pathname), new Response('{}')) }),
    },
  };
  const body = JSON.stringify(goodReport());
  const r = await gateway.fetch(
    new Request('https://broker.example/' + 'p'.repeat(40) + '/run-report/123-1', {
      method: 'POST',
      headers: { Authorization: 'Bearer a.b.c' },
      body,
    }),
    env,
  );
  assert.equal(r.status, 200);
  assert.deepEqual(seen, ['/' + 'p'.repeat(40) + '/run-report/123-1']);
});

test('gateway: a 200 KB report reaches the ledger whole, a larger one is cut at the cap plus one byte', async () => {
  const gateway = createGateway({ fetcher: async () => new Response('') });
  const seen = [];
  const environment = {
    ...baseEnv,
    BROKER_PATH_PREFIX: 'p'.repeat(40),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async (r) => (
          seen.push([new URL(r.url).pathname, (await r.text()).length]),
          new Response('{}')
        ),
      }),
    },
  };
  const record = {
    run: '123',
    attempt: '1',
    sha,
    nonce,
    key: '123-1',
    label: 'kf-ci-123-1',
    reportExpiry: Math.floor(Date.now() / 1000) + 3600,
  };
  const good = callbackToken(record, baseEnv.RUN_TOKEN_SECRET);
  const url = 'https://broker.example/' + 'p'.repeat(40) + '/run-report/';
  const big = JSON.stringify({ schema: 1, pad: 'x'.repeat(200000) });
  const send = (key, token, body = big) =>
    gateway.fetch(
      new Request(url + key, {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + token },
        body,
      }),
      environment,
    );
  assert.equal((await send('123-1', good)).status, 200);
  assert.deepEqual(seen, [['/' + 'p'.repeat(40) + '/run-report/123-1', big.length]]);
  const over = await send('123-1', good, 'y'.repeat(REPORT_MAX_BYTES + 5000));
  assert.equal(
    seen.at(-1)[1] <= REPORT_MAX_BYTES + 1,
    true,
    'the gateway buffers at most the cap plus one byte',
  );
  assert.equal(over.status, 200);
});

test('gateway: the run token is checked statelessly (another rental, garbage, tampered)', async () => {
  const gateway = createGateway({ fetcher: async () => new Response('') });
  const seen = [];
  const environment = {
    ...baseEnv,
    BROKER_PATH_PREFIX: 'p'.repeat(40),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async (r) => (
          seen.push([new URL(r.url).pathname, (await r.text()).length]),
          new Response('{}')
        ),
      }),
    },
  };
  const record = {
    run: '123',
    attempt: '1',
    sha,
    nonce,
    key: '123-1',
    label: 'kf-ci-123-1',
    reportExpiry: Math.floor(Date.now() / 1000) + 3600,
  };
  const good = callbackToken(record, baseEnv.RUN_TOKEN_SECRET);
  const url = 'https://broker.example/' + 'p'.repeat(40) + '/run-report/';
  const big = JSON.stringify({ schema: 1, pad: 'x'.repeat(200000) });
  const send = (key, token, body = big) =>
    gateway.fetch(
      new Request(url + key, {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + token },
        body,
      }),
      environment,
    );
  assert.equal((await send('123-1', good, '{}')).status, 200);
  assert.equal((await send('123-2', good)).status, 401, 'a token of another rental');
  assert.equal((await send('123-1', 'a.b.c')).status, 401);
  assert.equal((await send('123-1', good + 'x')).status, 401);
  assert.equal(seen.length, 1, 'only the good token reached the ledger');
});

// ---- publishing ------------------------------------------------------------------------------

const LOG_LINES = [
  'KF_PAYLOAD_STAGE_START name=cuda_host\n',
  'café host 203.0.113.9 GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001 commit ' + 'a'.repeat(40) + '\n',
  'KF_PAYLOAD_STAGE_END name=cuda_host rc=0 duration_ms=5\n',
  'KF_PAYLOAD_STAGE_START name=merge_bar\nlong line https://example.test/x?y=1\n',
  'KF_PAYLOAD_STAGE_END name=merge_bar rc=0 duration_ms=9\n',
];

async function finishedRun(options = {}, { report = goodReport(), secretLine = true } = {}) {
  const x = await harness(options);
  const { h } = x;
  assert.equal((await h.call()).status, 200);
  await cron(h, 2);
  const record = await h.storage.get('record:' + '123-1');
  const token = callbackToken(record, h.bindings.RUN_TOKEN_SECRET);
  let seq = 1;
  await h.report({ seq: seq++, kind: 'progress', text: LOG_LINES[0] });
  for (const line of LOG_LINES.slice(1, 3)) await h.report({ seq: seq++, kind: 'log', text: line });
  if (secretLine)
    await h.report({
      seq: seq++,
      kind: 'log',
      text:
        'oops ' +
        token +
        ' and nonce ' +
        record.nonce +
        ' and Bearer ' +
        'q'.repeat(30) +
        ' and https://b.test/o?X-Amz-Signature=' +
        'ab'.repeat(32) +
        '&X-Amz-Credential=AK end\n',
    });
  for (const line of LOG_LINES.slice(3)) await h.report({ seq: seq++, kind: 'log', text: line });
  if (report) assert.equal((await postReport(h, report)).status, 200);
  assert.equal(
    (
      await h.report({
        seq: seq++,
        kind: 'result',
        result: { nonce, verdict: 'PASS', suites: passSuites, hardware },
      })
    ).status,
    200,
  );
  h.time.value += 6 * 60000;
  await cron(h, 2);
  return { ...x, token, record, seq };
}

test(
  'publish: a settled run gets log.txt and report.json in the bucket and rows in D1',
  { skip: d1Skip },
  async () => {
    const { h, db, bucket, token, record } = await finishedRun();
    assert.deepEqual(bucket.puts, [], 'nothing is written before the publisher runs');
    const final = await h.storage.get('record:123-1');
    assert.notEqual(final.ended, null);
    assert.equal(final.verdict, 'PASS');
    await h.service.publishRuns();
    assert.deepEqual(bucket.puts.sort(), ['vast-123-1-0/log.txt', 'vast-123-1-0/report.json']);
    const log = bucket.text('vast-123-1-0/log.txt');
    // Whole text, with secret and endpoint redaction.
    assert.ok(
      log.includes(
        'café host 203.0.113.0 GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001 commit ' + 'a'.repeat(40),
      ),
    );
    assert.ok(log.includes('long line https://example.test/x?y=1'));
    assert.ok(log.includes('[Worker] '), "the Worker's own lines are part of the log");
    // Redacted: the run token, the nonce, a bearer header, a presigned URL.
    for (const secret of [token, record.nonce, 'q'.repeat(30), 'ab'.repeat(32)])
      assert.ok(!log.includes(secret), secret.slice(0, 8));
    assert.equal(
      bucket.objects.get('vast-123-1-0/log.txt').options.httpMetadata.contentType,
      'text/plain; charset=utf-8',
    );
    const doc = JSON.parse(bucket.text('vast-123-1-0/report.json'));
    assert.equal(doc.schema, 1);
    assert.equal(doc.run_id, 'vast-123-1-0');
    assert.equal(doc.verdict, 'PASS');
    assert.equal(doc.die, 'AD104');
    assert.equal(doc.family, 'ada');
    assert.equal(doc.host_driver, '580.159.04');
    assert.equal(doc.log.bytes, Buffer.byteLength(log));
    assert.match(doc.log.url, /^https:\/\/logs\.kayfabe\.dev\/vast-123-1-0\/log\.txt$/);
    // log_range: byte offsets of the log THE WORKER WROTE.
    const bytes = Buffer.from(log);
    const host = doc.subruns.find((s) => s.sub === 'host').log_range;
    assert.ok(
      bytes
        .subarray(host.start_byte)
        .toString()
        .startsWith('KF_PAYLOAD_STAGE_START name=cuda_host'),
    );
    assert.ok(bytes.subarray(0, host.end_byte).toString().endsWith('duration_ms=5\n'));
    assert.equal(doc.subruns.find((s) => s.sub === 'windows-580.88').log_range, null);
    // D1.
    const row = db.rows(
      'SELECT report_json, host_driver, log_bytes, log_sha256 FROM ci_run_reports',
    )[0];
    assert.equal(row.host_driver, '580.159.04');
    assert.equal(row.log_bytes, Buffer.byteLength(log));
    assert.equal(JSON.parse(row.report_json).run_id, 'vast-123-1-0');
    assert.deepEqual(
      db.rows('SELECT sub, kind, verdict FROM ci_subruns ORDER BY ord').map((s) => s.sub),
      ['host', 'linux-580.159.03', 'windows-580.88'],
    );
    assert.equal(db.rows('SELECT * FROM ci_run_tests').length, 2);
    assert.deepEqual(
      db.rows('SELECT sub, name, ref_ratio, higher_is_better FROM ci_run_perf ORDER BY ord'),
      [
        { sub: 'host', name: 'cup8bench N=2048 gflops', ref_ratio: null, higher_is_better: 1 },
        {
          sub: 'linux-580.159.03',
          name: 'cup8bench N=2048 gflops',
          ref_ratio: 0.96,
          higher_is_better: 1,
        },
      ],
    );
    assert.deepEqual(
      db
        .rows('SELECT sub, log_start IS NOT NULL AS has FROM ci_subruns ORDER BY ord')
        .map((r) => r.has),
      [1, 1, 0],
    );
    // Done: the entry is gone and a second tick writes nothing.
    assert.deepEqual(
      [...h.storage.data.keys()].filter((k) => k.startsWith('plogq:')),
      [],
    );
    const puts = bucket.puts.length;
    await h.service.publishRuns();
    assert.equal(bucket.puts.length, puts);
  },
);

test(
  'publish: a run with no box report still gets its log, and D1 records only the log identity',
  { skip: d1Skip },
  async () => {
    const { h, db, bucket } = await finishedRun({}, { report: null });
    await h.service.publishRuns();
    assert.deepEqual(bucket.puts, ['vast-123-1-0/log.txt']);
    const row = db.rows('SELECT report_json, log_bytes FROM ci_run_reports')[0];
    assert.equal(row.report_json, null);
    assert.ok(row.log_bytes > 0);
    assert.equal(db.rows('SELECT * FROM ci_subruns').length, 0);
  },
);

test(
  'publish: the report waits for the final D1 row and the log does not',
  { skip: d1Skip },
  async () => {
    const { h, db, bucket } = await finishedRun();
    db.database.exec(
      'DELETE FROM ci_run_stages; DELETE FROM ci_host_facts; DELETE FROM ci_runs_private; DELETE FROM ci_run_claims; DELETE FROM ci_runs',
    );
    const lines = [];
    const original = console.log;
    console.log = (l) => lines.push(String(l));
    try {
      await h.service.publishRuns();
    } finally {
      console.log = original;
    }
    assert.ok(
      lines.some((l) => /"why":"row_missing"/.test(l)),
      'one line says why: ' + lines.join('|'),
    );
    assert.deepEqual(
      bucket.puts,
      ['vast-123-1-0/log.txt'],
      'the log is published, the report waits',
    );
    assert.equal(db.rows('SELECT * FROM ci_run_reports').length, 0);
    assert.deepEqual(
      [...h.storage.data.keys()].filter((k) => k.startsWith('plogq:')),
      ['plogq:123-1:0'],
    );
    // The row appears later: the next tick finishes, and does not write the log twice.
    await cron(h);
  },
);

test(
  'fail open: a bucket outage never touches the verdict, the teardown or the budget, and is one line',
  { skip: d1Skip },
  async () => {
    const bucket = new Bucket();
    bucket.fail = true;
    const { h, db } = await finishedRun({ bucket });
    const record = await h.storage.get('record:123-1');
    assert.equal(record.verdict, 'PASS');
    assert.notEqual(record.ended, null, 'the rental was torn down before any publish');
    assert.equal(h.cloud.deletes.length, 1);
    const lines = [];
    const orig = console.log;
    console.log = (line) => lines.push(String(line));
    try {
      await h.service.publishRuns();
    } finally {
      console.log = orig;
    }
    const mine = lines.filter((l) => l.includes('public_log'));
    assert.equal(mine.length, 1);
    assert.match(mine[0], /"why":"error"/);
    assert.doesNotMatch(lines.join('\n'), /secret-bucket-detail|vast-123|R2 down/);
    assert.equal(db.rows('SELECT * FROM ci_run_reports').length, 0);
    assert.equal((await h.storage.get('plogq:123-1:0')).attempts, 1);
    // The bucket recovers: the next tick publishes everything.
    bucket.fail = false;
    h.time.value += 10 * 60000; // the entry backs off after a failure
    await h.service.publishRuns();
    assert.deepEqual([...bucket.objects.keys()].sort(), [
      'vast-123-1-0/log.txt',
      'vast-123-1-0/report.json',
    ]);
    assert.equal(db.rows('SELECT * FROM ci_run_reports').length, 1);
  },
);

test(
  'fail open: a hung bucket call is cut after its bound and the sweep is never delayed by it',
  { skip: d1Skip },
  async () => {
    const bucket = new Bucket();
    bucket.hang = true;
    const { h } = await finishedRun({ bucket });
    const was = publishTimeouts.r2;
    publishTimeouts.r2 = 30;
    try {
      const t0 = Date.now();
      const seen = [];
      const original = console.log;
      console.log = (l) => seen.push(String(l));
      try {
        await h.service.publishRuns();
      } finally {
        console.log = original;
      }
      assert.ok(Date.now() - t0 < 2000);
      assert.ok(
        seen.some((l) => /"why":"timeout"/.test(l)),
        seen.join('|'),
      );
      // A sweep is a different queue: it answers while a publish is stuck.
      bucket.hang = true;
      const stuck = h.service.publishRuns();
      await h.service.sweep();
      await stuck;
    } finally {
      publishTimeouts.r2 = was;
    }
  },
);

test(
  'fail open: a D1 outage after the log is stored repeats only what is missing',
  { skip: d1Skip },
  async () => {
    const { h, db, bucket } = await finishedRun();
    db.failOn = /INSERT INTO ci_run_reports/;
    const seen = [];
    const original = console.log;
    console.log = (l) => seen.push(String(l));
    try {
      await h.service.publishRuns();
    } finally {
      console.log = original;
    }
    assert.ok(
      seen.some((l) => /"why":"error"/.test(l)) && !seen.join('').includes('injected D1 fault'),
    );
    assert.deepEqual(bucket.puts.sort(), ['vast-123-1-0/log.txt', 'vast-123-1-0/report.json']);
    const entry = await h.storage.get('plogq:123-1:0');
    assert.equal(entry.logPut, true);
    assert.equal(entry.reportPut, true);
    assert.notEqual(entry.d1, true);
    db.failOn = null;
    h.time.value += 10 * 60000;
    await h.service.publishRuns();
    assert.equal(bucket.puts.length, 2, 'no object is written twice');
    assert.equal(db.rows('SELECT * FROM ci_run_reports').length, 1);
  },
);

test('publish: attempts are bounded and the entry is then dropped', { skip: d1Skip }, async () => {
  const bucket = new Bucket();
  bucket.fail = true;
  const { h } = await finishedRun({ bucket });
  const entry = await h.storage.get('plogq:123-1:0');
  entry.attempts = MAX_ATTEMPTS;
  await h.storage.put('plogq:123-1:0', entry);
  await h.service.publishRuns();
  assert.equal(await h.storage.get('plogq:123-1:0'), undefined);
});

test(
  'publish: expired pieces drop the entry; the pieces and entries leave with the run',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await finishedRun();
    await h.storage.delete('plog:123-1:0');
    await h.service.publishRuns();
    assert.equal(bucket.puts.length, 0);
    assert.equal(await h.storage.get('plogq:123-1:0'), undefined);
    // expire(): the chunks of the log and of the report go with the record.
    const x = await finishedRun();
    h.time; // eslint-disable-line no-unused-expressions
    x.h.time.value += 7 * 3600 * 1000;
    for (let i = 0; i < 3; i++) await x.h.service.sweep();
    const left = [...x.h.storage.data.keys()].filter((k) => /^(plog|plogq|rreport):/.test(k));
    assert.deepEqual(left, []);
  },
);

test('publish: no bucket and no store -> nothing is stored, nothing is queued, nothing breaks', async () => {
  const { h } = await harness({ store: false, logs: false });
  assert.equal((await h.call()).status, 200);
  await cron(h, 2);
  await h.report({ seq: 1, kind: 'progress', text: 'hello\n' });
  await h.report({
    seq: 2,
    kind: 'result',
    result: { nonce, verdict: 'PASS', suites: passSuites, hardware },
  });
  h.time.value += 6 * 60000;
  await cron(h, 2);
  const keys = [...h.storage.data.keys()];
  assert.equal(
    keys.some((k) => /^(plog|plogq|rreport):/.test(k)),
    false,
  );
  await h.service.publishRuns();
  assert.equal((await h.storage.get('record:123-1')).verdict, 'PASS');
});

test('publish: the ledger cap is visible in the log (16 MiB of stored text, the rest counted)', async () => {
  const storage = new Map();
  const ledger = new Ledger({
    put: async (k, v) => storage.set(k, v),
    get: async (k) => storage.get(k),
  });
  const record = { key: '1-1' };
  await ledger.appendPublicLog(record, 'a'.repeat(100), 150);
  await ledger.appendPublicLog(record, 'b'.repeat(100), 150);
  await ledger.appendPublicLog(record, 'c'.repeat(40), 150);
  assert.deepEqual([record.plogChunks, record.plogBytes, record.plogDropped], [2, 140, 100]);
  assert.equal(storage.get('plog:1-1:1'), 'c'.repeat(40));
  await ledger.appendPublicLog(record, '', 150);
  await ledger.appendPublicLog(record, null, 150);
  assert.equal(record.plogChunks, 2);
});

test('publishTick does nothing when neither the bucket nor the store is configured', async () => {
  const storage = new Map([['plogq:1-1:0', { attempts: 0 }]]);
  await publishTick({
    ledger: {
      storage: { list: async () => storage, delete: async () => assert.fail('touched') },
      get: async () => null,
      put: async () => assert.fail('touched'),
    },
    env: {},
    now: 0,
  });
});

// ---- schema and seed -------------------------------------------------------------------------

test('migration 0004 only adds (no ALTER, DROP, DELETE, UPDATE, INSERT) and its tables exist', () => {
  const sql = migrations();
  const m4 = sql.find((s) => /CREATE TABLE ci_run_reports/.test(s));
  assert.ok(m4);
  assert.doesNotMatch(m4.replace(/--[^\n]*/g, ''), /\b(?:ALTER|DROP|DELETE|UPDATE|INSERT)\b/i);
  for (const table of [
    'ci_run_reports',
    'ci_subruns',
    'ci_run_tests',
    'ci_run_perf',
    'ci_run_apps',
  ])
    assert.match(m4, new RegExp('CREATE TABLE ' + table + ' \\('));
  assert.equal(sql.length, 4, 'four migrations in order');
  assert.doesNotMatch(m4.replace(/--[^\n]*/g, ''), /machine_id|price_usd|cost_usd|score/i);
});

test(
  'migration 0004 is additive: 0001-0003 alone still work and 0004 changes none of their tables',
  { skip: d1Skip },
  () => {
    const old = new FakeD1({ upTo: 3 });
    const names = old
      .rows("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
      .map((r) => r.name);
    assert.ok(!names.includes('ci_subruns'));
    const full = new FakeD1();
    for (const t of names) {
      const a = old.rows('PRAGMA table_info(' + t + ')');
      const b = full.rows('PRAGMA table_info(' + t + ')');
      assert.deepEqual(b, a, t + ' unchanged');
    }
  },
);

test('the synthetic seed reports pass the Worker validator (one shape in both packages)', () => {
  const rows = syntheticRows();
  assert.ok(rows.ci_run_reports.length >= 5);
  for (const r of rows.ci_run_reports) {
    const doc = JSON.parse(r.report_json);
    const clean = cleanClaimsReport(doc);
    assert.ok(clean, r.run_id);
    assert.equal(clean.subruns.length, doc.subruns.length, r.run_id + ' subruns');
    assert.equal(clean.tests.length, doc.tests.length, r.run_id + ' tests');
    assert.equal(clean.perf.length, doc.perf.length, r.run_id + ' perf');
    assert.equal(Buffer.byteLength(r.report_json) <= REPORT_MAX_BYTES, true);
  }
  assert.ok(rows.ci_subruns.some((s) => s.verdict === 'NOT_RUN' && s.reason));
  assert.ok(
    new Set(rows.ci_subruns.filter((s) => s.run_id === 'syn-ci-p-8').map((s) => s.sub)).size >= 4,
  );
});

test('the payload tier file and the docs name the same box variable', () => {
  const readme = readFileSync(
    new URL('../../../scripts/ci/vast_onstart.sh', import.meta.url),
    'utf8',
  );
  assert.match(readme, /KF_RUN_REPORT/);
});
