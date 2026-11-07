import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce, sha, callbackToken, env as baseEnv } from './helpers.js';
import { FakeD1, d1Skip } from './d1.js';
import { publishTimeouts, buildPublicLog, exactValues, LOG_MAX_BYTES } from '../src/publiclog.js';
import { REPORT_MAX_BYTES, buildReport, cleanClaimsReport, storeReport } from '../src/runreport.js';
// The second angle on the same rules (every mutant of report-mutants.mjs needs two killers): the
// validator through the ROUTE (what the ledger really stores), the redaction and the cap through the
// whole publish path, and the lifecycle edges (closed rental, replacement leg, in-flight row, queue).
// FALSIFIER: see public-log.test.js and run-report.test.js; here the same rules are checked on what
// is stored and published, not on the helper functions.
const TAG_KEY = 't'.repeat(48);
const hex = (n) => n.toString(16).padStart(40, '0');
const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };
const pass = ['provision', 'gates', 'raw_suite'].map((name) => ({
  name,
  pass: 1,
  fail: 0,
  skip: 0,
}));
const fail = [{ name: 'gates', pass: 0, fail: 1, skip: 0 }];

class Bucket {
  objects = new Map();
  puts = [];
  hang = false;
  async put(key, value) {
    this.puts.push(key);
    if (this.hang) return new Promise(() => {});
    this.objects.set(key, typeof value === 'string' ? value : Buffer.from(value).toString());
  }
}
async function harness(overrides = {}, { store = true } = {}) {
  const db = store ? new FakeD1() : null;
  const bucket = new Bucket();
  const h = await setup({
    ...(store ? { DB: db, ROW_TAG_KEY: TAG_KEY } : {}),
    LOGS_BUCKET: bucket,
    ...overrides,
  });
  const base = h.cloud.fetch;
  h.cloud.fetch = async (input, init = {}) => {
    const u = new URL(String(input));
    if (u.hostname === 'api.github.com') {
      const p = u.pathname.replace(/^\/repos\/[^/]+\/[^/]+\//, '');
      if (p === 'git/commits/' + sha) return Response.json({ tree: { sha: hex(100) } });
      if (p === 'git/trees/' + hex(100))
        return Response.json({
          truncated: false,
          tree: [{ path: 'Cargo.toml', mode: '100644', type: 'blob', sha: hex(1) }],
        });
      if (/^git\//.test(p) || /^contents\/ci\/requirements\.toml$/.test(p))
        return new Response('', { status: 404 });
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
const post = async (h, body, { token, raw, run = '123-1' } = {}) => {
  const record = await h.storage.get('record:' + run);
  return h.service.fetch(
    new Request(h.bindings.WORKER_URL + '/run-report/' + run, {
      method: 'POST',
      headers: {
        Authorization: 'Bearer ' + (token ?? callbackToken(record, h.bindings.RUN_TOKEN_SECRET)),
      },
      body: raw ?? JSON.stringify(body),
    }),
  );
};
const stored = async (h, run = '123-1') => {
  const record = await h.storage.get('record:' + run);
  const parts = [];
  for (let i = 0; i < (record.runReport?.chunks ?? 0); i++)
    parts.push(await h.storage.get('rreport:' + run + ':0:' + i));
  return parts.length ? JSON.parse(parts.join('')) : null;
};
const base = (patch = {}) => ({
  schema: 1,
  guest_drivers: { linux: '580.159.03', windows: null },
  stages: [],
  suites: [{ name: 'gates', pass: 9, fail: 0, skip: 0 }],
  subruns: [
    {
      sub: 'host',
      kind: 'host_bare',
      os: 'host',
      host_driver: '580.159.04',
      verdict: 'PASS',
      stages: ['gates'],
      log_stages: ['cuda_host'],
      counts: { pass: 9, fail: 0, skip: 0 },
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
  ],
  tests: [{ sub: 'host', suite: 'raw_suite', name: 'timer', result: 'PASS', ms: 5 }],
  apps: [],
  perf: [
    {
      sub: 'host',
      name: 'cup8bench N=2048 gflops',
      value: 1,
      unit: 'GFLOPS',
      higher_is_better: true,
      suite: 'cuda_host',
      ref_value: null,
      ref_ratio: null,
    },
  ],
  windows: { gate: null, install_s: null, probes: [], resets: null, live_dumps_193: null },
  ...patch,
});

// ---- the validator, through the route ---------------------------------------------------------

test(
  'route stores only the clean claims: hostile body in, bounded shape out',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const hostile = base({
      run_id: 'vast-9-9-9',
      verdict: 'PASS',
      extra: { deep: [1, 2, 3] },
      tests: [
        { sub: 'host', suite: 'raw_suite', name: 'ok-é', result: 'PASS', ms: 1 },
        { sub: 'ghost', suite: 'raw_suite', name: 'unknown subrun', result: 'PASS', ms: 1 },
        {
          sub: 'host',
          suite: 'raw_suite',
          name: 'SPECviewperf maya score 99',
          result: 'PASS',
          ms: 1,
        },
        {
          sub: 'host',
          suite: 'raw_suite',
          name: 'tok eyJhbGciOiJIUzI1NiJ9.abc',
          result: 'PASS',
          ms: 1,
        },
        { sub: 'host', suite: 'raw_suite', name: 'u https://x.test/p', result: 'PASS', ms: 1 },
        { sub: 'host', suite: 'raw_suite', name: 'x'.repeat(300), result: 'PASS', ms: 1 },
      ],
      perf: [
        {
          sub: 'host',
          name: 'huge',
          value: 1e300,
          unit: 'ms',
          higher_is_better: false,
          suite: 'cuda_host',
        },
        {
          sub: 'host',
          name: 'text',
          value: 'NaN',
          unit: 'ms',
          higher_is_better: false,
          suite: 'cuda_host',
        },
        {
          sub: 'host',
          name: 'fine',
          value: 2.5,
          unit: 'ms',
          higher_is_better: false,
          suite: 'cuda_host',
        },
      ],
      suites: [
        { name: 'gates', pass: 9, fail: 0, skip: 0 },
        { name: 'specviewperf', pass: 1, fail: 0, skip: 0 },
      ],
      subruns: [
        ...base().subruns,
        { ...base().subruns[0] },
        { sub: 'host', kind: 'linux_guest', os: 'linux', verdict: 'PASS' },
        {
          sub: 'linux-570.1.1',
          kind: 'linux_guest',
          os: 'linux',
          guest_driver: '580.159.03',
          verdict: 'PASS',
        },
        {
          sub: 'windows-580.88',
          kind: 'windows_guest',
          os: 'windows',
          guest_driver: '580.88',
          verdict: 'NOT_RUN',
        },
      ],
    });
    assert.equal((await post(h, hostile)).status, 200);
    const doc = await stored(h);
    assert.deepEqual(Object.keys(doc).sort(), [
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
    assert.deepEqual(
      doc.tests.map((t) => t.name),
      ['ok-?', 'x'.repeat(120)],
      'a long name is cut, not kept whole',
    );
    assert.deepEqual(
      doc.perf.map((p) => p.name),
      ['fine'],
    );
    assert.deepEqual(
      doc.suites.map((s) => s.name),
      ['gates'],
    );
    assert.deepEqual(
      doc.subruns.map((s) => s.sub),
      ['host', 'linux-580.159.03', 'windows-580.88'],
    );
    assert.equal(doc.subruns[2].reason, 'not reported');
    assert.doesNotMatch(JSON.stringify(doc), /viewperf|eyJ|https?:|x{200}|[^\x20-\x7e]/i);
  },
);

test(
  'route fits an oversized report into 256 KiB and says which arrays were cut',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const row = (i, pad) => ({
      sub: 'host',
      suite: 'raw_suite',
      name: 'arm-' + String(i).padStart(4, '0') + '-' + 'n'.repeat(pad),
      result: 'PASS',
    });
    const shell = JSON.stringify(base({ tests: [] })).length;
    const pad = Math.floor(
      (REPORT_MAX_BYTES - 2000 - shell) / 2000 - JSON.stringify(row(0, 0)).length - 1,
    );
    const tests = Array.from({ length: 2000 }, (_, i) => row(i, pad));
    const body = JSON.stringify(base({ tests }));
    assert.ok(
      body.length <= REPORT_MAX_BYTES,
      'the request itself is within the cap: ' + body.length,
    );
    assert.equal((await post(h, null, { raw: body })).status, 200);
    const doc = await stored(h);
    assert.ok(Buffer.byteLength(JSON.stringify(doc)) <= REPORT_MAX_BYTES);
    assert.deepEqual(doc.truncated, ['tests']);
    assert.ok(doc.tests.length > 800 && doc.tests.length < 2000);
  },
);

test('route: arrays are capped and a subrun id is never repeated', { skip: d1Skip }, async () => {
  const { h } = await harness();
  await h.call();
  const tests = Array.from({ length: 2300 }, (_, i) => ({
    sub: 'host',
    suite: 'raw_suite',
    name: 'a' + i,
    result: 'PASS',
    ms: 1,
  }));
  const perf = Array.from({ length: 260 }, (_, i) => ({
    sub: 'host',
    name: 'p' + i,
    value: i,
    unit: 'ms',
    higher_is_better: false,
    suite: 'cuda_host',
  }));
  const subs = [...base().subruns, ...base().subruns, ...base().subruns];
  const r = await post(h, base({ tests: tests.slice(0, 1200), perf, subruns: subs }));
  assert.equal(r.status, 200);
  const doc = await stored(h);
  assert.equal(doc.perf.length, 200);
  assert.equal(doc.subruns.length, 2);
  assert.ok(doc.tests.length <= 2000);
});

test(
  'route: the schema is required and anything else is a fixed 400 that stores nothing',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    for (const body of [{ ...base(), schema: 2 }, { ...base(), schema: '1' }, [], 'x', null]) {
      const r = await post(h, null, { raw: JSON.stringify(body) });
      assert.equal(r.status, 400);
      assert.equal((await r.json()).state, 'BAD_REQUEST');
    }
    assert.equal((await h.storage.get('record:123-1')).runReport, undefined);
  },
);

test(
  "route: a bad token, another rental's token and a missing record are each a 401",
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const record = await h.storage.get('record:123-1');
    assert.equal((await post(h, base(), { token: 'bad.token.value' })).status, 401);
    assert.equal(
      (await post(h, base(), { token: callbackToken({ ...record, sha: 'e'.repeat(40) }) })).status,
      401,
    );
    assert.equal(
      (await post(h, base(), { token: callbackToken({ ...record, nonce: 'f'.repeat(32) }) }))
        .status,
      401,
    );
    const r = await post(h, base(), {
      run: '999-1',
      token: callbackToken({ ...record, run: '999', key: '999-1' }),
    });
    assert.equal(r.status, 401);
    assert.equal((await h.storage.get('record:123-1')).runReport, undefined);
  },
);

test('route: a closed rental and an ended rental are refused', { skip: d1Skip }, async () => {
  const { h } = await harness();
  await h.call();
  const record = await h.storage.get('record:123-1');
  await h.storage.put('record:123-1', { ...record, closed: 'STOPPED' });
  assert.equal((await post(h, base(), { token: callbackToken(record) })).status, 401);
  await h.storage.put('record:123-1', { ...record, ended: h.time.value });
  assert.equal((await post(h, base(), { token: callbackToken(record) })).status, 401);
  await h.storage.put('record:123-1', record);
  assert.equal((await post(h, base())).status, 200);
});

test(
  'route: one report per leg; the second answers REPORT_EXISTS and the first stays',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    assert.equal((await post(h, base())).status, 200);
    const second = await post(h, base({ tests: [] }));
    assert.equal(second.status, 409);
    assert.equal((await second.json()).state, 'REPORT_EXISTS');
    assert.equal((await stored(h)).tests.length, 1);
  },
);

test('route: a body above the cap is a 413 and never read whole', { skip: d1Skip }, async () => {
  const { h } = await harness();
  await h.call();
  const r = await post(h, null, {
    raw: JSON.stringify(base({ pad: 'x'.repeat(REPORT_MAX_BYTES) })),
  });
  assert.equal(r.status, 413);
  assert.equal((await r.json()).state, 'PAYLOAD_TOO_LARGE');
});

test('no result store: KF_RUN_REPORT is not set even with a bucket, and the route says REPORTS_DISABLED', async () => {
  const { h } = await harness({}, { store: false });
  await h.call();
  const create = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
  assert.ok(!('KF_RUN_REPORT' in create.body.env));
  const r = await post(h, base());
  assert.equal(r.status, 404);
  assert.equal((await h.storage.get('record:123-1')).runReport, undefined);
});

// ---- the whole publish path -------------------------------------------------------------------

const SHAPES = {
  jwt: 'a=eyJhbGciOiJIUzI1NiJ9.eyJydW4iOiIxMjMifQ.' + 'S'.repeat(43),
  key: '-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----',
  github: 'ghp_' + 'a1'.repeat(15),
  credentials: 'git clone https://user:hunter2hunter2@h.test/r',
  named: 'db_password=correcthorsebattery',
  access: 'CF-Access-Client-Secret: abcdef0123456789abcdef',
  vast: '{"instance_api_key": "' + 'A1'.repeat(16) + '"}',
};
const PIECES = {
  jwt: /eyJhbGci|SSSSSSSS/,
  key: /b3BlbnNzaC1rZXk|PRIVATE KEY/,
  github: /ghp_/,
  credentials: /hunter2/,
  named: /correcthorse/,
  access: /abcdef0123456789/,
  vast: /A1A1A1/,
};

async function runWith(
  lines,
  { verdict = 'PASS', suites = pass, report = base(), overrides = {}, mutate } = {},
) {
  const x = await harness(overrides);
  const { h } = x;
  assert.equal((await h.call()).status, 200);
  await cron(h, 2);
  const record = await h.storage.get('record:123-1');
  let seq = 1;
  for (const line of typeof lines === 'function'
    ? lines(record, callbackToken(record, h.bindings.RUN_TOKEN_SECRET))
    : lines)
    await h.report({ seq: seq++, kind: seq === 2 ? 'progress' : 'log', text: line });
  if (report) assert.equal((await post(h, report)).status, 200);
  if (mutate) await mutate(h);
  await h.report({ seq: seq++, kind: 'result', result: { nonce, verdict, suites, hardware } });
  h.time.value += 11 * 60000;
  await cron(h, 2);
  await h.service.publishRuns();
  return { ...x, record };
}

test(
  'publish: every secret shape is redacted in the published log (and nothing else is)',
  { skip: d1Skip },
  async () => {
    const keep =
      'KEEP 203.0.113.9 GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001 \u001b[31mred\u001b[0m café ☃';
    const { bucket } = await runWith([keep + '\n', ...Object.values(SHAPES).map((s) => s + '\n')]);
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    for (const [name, pattern] of Object.entries(PIECES)) assert.doesNotMatch(log, pattern, name);
    assert.ok(log.includes(keep), 'the rest of the log is untouched, ANSI sequences included');
    assert.ok(log.includes('[redacted]'));
  },
);

test(
  'publish: a token cut in two by two posts, and fragments of it, never reach the bucket',
  { skip: d1Skip },
  async () => {
    const { bucket, h } = await runWith((record, token) => [
      'first part ' + token.slice(0, 40),
      token.slice(40) + ' second part\n',
      'fragment ' + token.slice(0, 20) + ' and ' + token.slice(-20) + '\n',
      'nonce ' + record.nonce + ' and ' + record.nonce.slice(0, 16) + '\n',
    ]);
    const record = await h.storage.get('record:123-1');
    const token = callbackToken(record);
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    for (const piece of [
      token.slice(0, 20),
      token.slice(-20),
      token.slice(10, 40),
      record.nonce,
      record.nonce.slice(0, 16),
    ])
      assert.ok(!log.includes(piece), piece);
  },
);

test(
  'publish: a short Worker secret is not redacted by value (it would shred ordinary words)',
  { skip: d1Skip },
  async () => {
    const { bucket } = await runWith(['a table with ab and ci and dev\n'], {
      overrides: { BOX_ACCESS_ID: 'ab', BOX_ACCESS_SECRET: 'ci' },
    });
    assert.ok(
      bucket.objects.get('vast-123-1-0/log.txt').includes('a table with ab and ci and dev'),
    );
  },
);

test(
  'publish: Worker secrets by value (a long env secret in a box line) are redacted',
  { skip: d1Skip },
  async () => {
    const { bucket } = await runWith(
      [
        'leak ' + 'ownerStopSecret_'.repeat(3) + ' end\n',
        'leak2 ' + 'rowTagKeyValue.'.repeat(3) + '\n',
      ],
      {
        overrides: {
          OWNER_STOP_TOKEN: 'ownerStopSecret_'.repeat(3),
          BOX_ACCESS_SECRET: 'rowTagKeyValue.'.repeat(3),
        },
      },
    );
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    assert.doesNotMatch(log, /ownerStopSecret|rowTagKeyValue/);
  },
);

test(
  'publish: bytes the ledger dropped are named in the log (its own cap), the Worker lines stay',
  { skip: d1Skip },
  async () => {
    const { bucket } = await runWith(['line\n'], {
      mutate: async (h) => {
        const record = await h.storage.get('record:123-1');
        record.plogDropped = 4242;
        await h.storage.put('record:123-1', record);
      },
    });
    assert.match(
      bucket.objects.get('vast-123-1-0/log.txt'),
      /\[kayfabe-log\] 4242 further bytes were not stored/,
    );
  },
);

test(
  "publish: a failed run publishes with the Worker's verdict, not the box's claim",
  { skip: d1Skip },
  async () => {
    const claims = base({
      verdict: 'PASS',
      tier: 'largest',
      die: 'GB202',
      run_id: 'vast-1-1-0',
      commit: 'f'.repeat(40),
    });
    const { bucket, db } = await runWith(['x\n'], {
      verdict: 'FAIL',
      suites: fail,
      report: claims,
    });
    const doc = JSON.parse(bucket.objects.get('vast-123-1-0/report.json'));
    assert.equal(doc.verdict, 'FAIL');
    assert.equal(doc.tier, 'middle');
    assert.equal(doc.die, 'AD104');
    assert.equal(doc.commit, sha);
    assert.equal(doc.run_id, 'vast-123-1-0');
    assert.equal(db.rows('SELECT verdict FROM ci_runs')[0].verdict, 'FAIL');
  },
);

test(
  'publish: the report is not written while the D1 row is in flight, and says why in one line',
  { skip: d1Skip },
  async () => {
    const { h, db, bucket } = await runWith(['x\n']);
    db.database.exec(
      'DELETE FROM ci_run_reports; DELETE FROM ci_run_tests; DELETE FROM ci_run_perf; DELETE FROM ci_subruns',
    );
    db.database.exec('UPDATE ci_runs SET verdict = NULL, ended_at = NULL, tag = NULL');
    bucket.objects.delete('vast-123-1-0/report.json');
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 1,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    const lines = [];
    const original = console.log;
    console.log = (l) => lines.push(String(l));
    try {
      await h.service.publishRuns();
    } finally {
      console.log = original;
    }
    assert.equal(db.rows('SELECT * FROM ci_run_reports').length, 0);
    assert.ok(!bucket.objects.has('vast-123-1-0/report.json'));
    assert.ok(
      lines.some((l) => /"why":"row_missing"/.test(l)),
      lines.join('\n'),
    );
  },
);

test(
  'storeReport replaces the previous rows of the run, never mixes old and new',
  { skip: d1Skip },
  async () => {
    const { h, db } = await runWith(['x\n']);
    const first = db.rows('SELECT COUNT(*) AS n FROM ci_run_tests')[0].n;
    assert.ok(first >= 1);
    const doc = {
      ...buildReport({
        claims: cleanClaimsReport(
          base({
            tests: [{ sub: 'host', suite: 'raw_suite', name: 'only-one', result: 'FAIL', ms: 1 }],
            perf: [],
          }),
        ),
        row: db.rows('SELECT * FROM ci_runs')[0],
        runId: 'vast-123-1-0',
        log: null,
        offsets: null,
        logsBase: null,
      }),
    };
    assert.equal(
      await storeReport(h.bindings, {
        runId: 'vast-123-1-0',
        doc,
        hostDriver: '580.159.04',
        log: null,
        now: Date.now(),
      }),
      'STORED',
    );
    assert.deepEqual(db.rows('SELECT name, result FROM ci_run_tests'), [
      { name: 'only-one', result: 'FAIL' },
    ]);
    assert.equal(db.rows('SELECT COUNT(*) AS n FROM ci_run_perf')[0].n, 0);
    assert.equal(db.rows('SELECT COUNT(*) AS n FROM ci_run_reports')[0].n, 1);
  },
);

test(
  'publish: the sweep is not queued behind a stuck bucket write (the publisher is off the serial queue)',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await runWith(['x\n']);
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    bucket.hang = true;
    const was = publishTimeouts.r2;
    publishTimeouts.r2 = 700;
    try {
      const stuck = h.service.publishRuns();
      const t0 = Date.now();
      await h.service.sweep();
      assert.ok(Date.now() - t0 < 400, 'the sweep waited ' + (Date.now() - t0) + ' ms');
      await stuck;
    } finally {
      publishTimeouts.r2 = was;
    }
  },
);

test(
  'publish: two publisher ticks never run at once in one isolate',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await runWith(['x\n']);
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    const before = bucket.puts.length;
    bucket.hang = true;
    const was = publishTimeouts.r2;
    publishTimeouts.r2 = 300;
    try {
      await Promise.all([
        h.service.publishRuns(),
        h.service.publishRuns(),
        h.service.publishRuns(),
      ]);
    } finally {
      publishTimeouts.r2 = was;
    }
    assert.equal(bucket.puts.length - before, 1, 'one attempt, not three');
  },
);

test(
  'replacement leg: the log chain continues, leg 0 keeps its own log',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await harness();
    Object.assign(h.cloud.offer, { host_id: 100 });
    h.cloud.contracts = [900, 901];
    assert.equal((await h.call()).status, 200);
    await cron(h, 2);
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 },
    ];
    const r0 = await h.storage.get('record:123-1');
    await h.report({ seq: 1, kind: 'log', text: 'LEG-ZERO-MARK\n' });
    await h.report({
      seq: 2,
      kind: 'result',
      result: {
        nonce: r0.nonce,
        verdict: 'PROVIDER_FAULT',
        suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
      },
    });
    assert.equal((await h.call()).status, 200);
    const r1 = await h.storage.get('record:123-1');
    assert.equal(r1.providerRetries, 1);
    assert.ok(r1.plogChunks >= 1, 'the chain continues into leg 1');
    await h.report({ seq: 3, kind: 'log', text: 'LEG-ONE-MARK\n' });
    await h.report({
      seq: 4,
      kind: 'result',
      result: { nonce: r1.nonce, verdict: 'PASS', suites: pass, hardware },
    });
    h.time.value += 11 * 60000;
    await cron(h, 3);
    await h.service.publishRuns();
    await h.service.publishRuns();
    const leg0 = bucket.objects.get('vast-123-1-0/log.txt');
    const leg1 = bucket.objects.get('vast-123-1-1/log.txt');
    assert.ok(leg0.includes('LEG-ZERO-MARK') && !leg0.includes('LEG-ONE-MARK'));
    assert.ok(leg1.includes('LEG-ZERO-MARK') && leg1.includes('LEG-ONE-MARK'));
  },
);

test(
  'expiry removes the log pieces, the report chunks and the publish entry of a run',
  { skip: d1Skip },
  async () => {
    const { h } = await runWith(['piece\n']);
    assert.ok([...h.storage.data.keys()].some((k) => k.startsWith('plog:')));
    assert.ok([...h.storage.data.keys()].some((k) => k.startsWith('rreport:')));
    h.time.value += 7 * 3600 * 1000;
    for (let i = 0; i < 3; i++) await h.service.sweep();
    assert.deepEqual(
      [...h.storage.data.keys()].filter((k) => k.startsWith('plog:')),
      [],
    );
    assert.deepEqual(
      [...h.storage.data.keys()].filter((k) => k.startsWith('rreport:')),
      [],
    );
    assert.deepEqual(
      [...h.storage.data.keys()].filter((k) => k.startsWith('plogq:')),
      [],
    );
  },
);

test(
  'expiry also removes the publish entry of a run that was never published',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    await h.report({ seq: 1, kind: 'log', text: 'x\n' });
    await h.report({
      seq: 2,
      kind: 'result',
      result: { nonce, verdict: 'PASS', suites: pass, hardware },
    });
    h.time.value += 11 * 60000;
    await h.service.sweep();
    assert.ok(await h.storage.get('plogq:123-1:0'));
    h.time.value += 7 * 3600 * 1000;
    for (let i = 0; i < 3; i++) await h.service.sweep();
    assert.equal(await h.storage.get('plogq:123-1:0'), undefined);
    assert.equal(await h.storage.get('plog:123-1:0'), undefined);
  },
);

test(
  'a settled run with nothing to publish queues nothing; a log-only run queues its log',
  { skip: d1Skip },
  async () => {
    const x = await harness({}, { store: false });
    await x.h.call();
    await x.h.report({
      seq: 1,
      kind: 'result',
      result: { nonce, verdict: 'PASS', suites: pass, hardware },
    });
    x.h.time.value += 11 * 60000;
    await x.h.service.sweep();
    const entry = await x.h.storage.get('plogq:123-1:0');
    assert.equal(entry.rchunks, 0);
    assert.ok(entry.chunks >= 1);
    await x.h.service.publishRuns();
    assert.ok(
      x.bucket.objects.has('vast-123-1-0/log.txt'),
      'the log needs the bucket only, not D1',
    );
    assert.ok(!x.bucket.objects.has('vast-123-1-0/report.json'));
    assert.equal(await x.h.storage.get('plogq:123-1:0'), undefined);
  },
);

// ---- the cap, with literal numbers (no import of the constant under test) ---------------------

test('the cap is 16 MiB exactly and a cut character is never half a character', async () => {
  assert.equal(LOG_MAX_BYTES, 16777216);
  const line = 'x'.repeat(1023) + '\n';
  const total = 17 * 1024 * 1024;
  const parts = Array.from({ length: Math.ceil(total / line.length) }, () => line);
  const built = await buildPublicLog(parts, []);
  assert.ok(built.bytes <= 16 * 1024 * 1024 && built.bytes > 16 * 1024 * 1024 - 65536);
  assert.ok(built.cut > 1024 * 1024 - 8192);
  const snow = await buildPublicLog(['☃'.repeat(1_000_000)], [], { max: 2 * 1024 * 1024 + 1 });
  assert.doesNotMatch(snow.text, /�/);
  assert.ok(snow.bytes <= 2 * 1024 * 1024 + 1);
  assert.match(snow.text, /CUT: [0-9]+ bytes omitted/);
  assert.ok(snow.text.endsWith('☃'));
  assert.match(
    (await buildPublicLog(['y'.repeat(5000)], [], { max: 4096 + 1024 * 1024 + 100 + 1000 })).text,
    /^y+/,
  );
});

test('a cut keeps the head and the tail around its marker and says how much was removed', async () => {
  const text = 'HEAD' + 'm'.repeat(3 * 1024 * 1024) + 'TAIL';
  const built = await buildPublicLog([text], [], { max: 2 * 1024 * 1024 });
  assert.ok(built.text.startsWith('HEADm'));
  assert.ok(built.text.endsWith('mTAIL'));
  const marker = /\n\[kayfabe-log\] CUT: ([0-9]+) bytes omitted here \(16 MiB cap\)\n/.exec(
    built.text,
  );
  assert.ok(marker);
  assert.equal(Number(marker[1]), built.cut);
  assert.equal(built.bytes + built.cut - Buffer.byteLength(marker[0]), Buffer.byteLength(text));
});

test('exact values are replaced by value, the fragments of a long one too, and nothing short', () => {
  const secret = 'Zq9-'.repeat(12);
  const values = exactValues([secret, 'short', 'seven77', 'eight888']);
  assert.ok(
    values.includes(secret) &&
      values.includes(secret.slice(0, 16)) &&
      values.includes(secret.slice(-16)),
  );
  assert.ok(values.includes('eight888'));
  assert.ok(!values.includes('short') && !values.includes('seven77'));
  assert.equal(values[0], secret, 'longest first');
});

test('jwt-shaped needs three parts of the shapes of a token; a file name is left alone', async () => {
  const keepers = [
    'eyJabc.def.ghi',
    'eyJabcdefghij.txt',
    'eyJabcdefghij.kkkkkk.short',
    'xeyJ.not.a.token',
  ];
  const built = await buildPublicLog([keepers.join('\n')], []);
  assert.equal(built.text, keepers.join('\n'));
});

// ---- second angles, part 2 --------------------------------------------------------------------

import { readRunRow } from '../src/runreport.js';
import { publishTick, MAX_ATTEMPTS } from '../src/publish.js';
import { Ledger } from '../src/ledger.js';

test(
  'the env var KF_RUN_REPORT=1 reaches the box of a Worker that has the store',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const create = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
    assert.equal(create.body.env.KF_RUN_REPORT, '1');
    assert.equal(create.body.env.KF_HOST_CLAIMS, '1');
  },
);

test('a bucket without a result store: the log is kept and published, no report is asked for', async () => {
  const x = await harness({}, { store: false });
  await x.h.call();
  const create = x.h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
  assert.ok(!('KF_RUN_REPORT' in create.body.env));
  await x.h.report({ seq: 1, kind: 'log', text: 'only a log\n' });
  assert.ok(await x.h.storage.get('plog:123-1:0'));
});

test(
  'a result store without a bucket keeps no public log pieces at all',
  { skip: d1Skip },
  async () => {
    const x = await harness({ LOGS_BUCKET: undefined });
    await x.h.call();
    await x.h.report({ seq: 1, kind: 'log', text: 'text\n' });
    await x.h.report({
      seq: 2,
      kind: 'result',
      result: { nonce, verdict: 'PASS', suites: pass, hardware },
    });
    x.h.time.value += 11 * 60000;
    await cron(x.h, 2);
    assert.deepEqual(
      [...x.h.storage.data.keys()].filter((k) => k.startsWith('plog')),
      [],
    );
  },
);

test('the ledger cap holds at 16 MiB with its default and counts what it drops', async () => {
  const data = new Map();
  const ledger = new Ledger({ put: async (k, v) => data.set(k, v), get: async (k) => data.get(k) });
  const record = { key: '1-1', plogChunks: 3, plogBytes: 16 * 1024 * 1024 - 10 };
  await ledger.appendPublicLog(record, 'x'.repeat(10));
  assert.deepEqual(
    [record.plogChunks, record.plogBytes, record.plogDropped],
    [4, 16 * 1024 * 1024, undefined],
  );
  await ledger.appendPublicLog(record, 'y'.repeat(11));
  assert.deepEqual(
    [record.plogChunks, record.plogBytes, record.plogDropped],
    [4, 16 * 1024 * 1024, 11],
  );
  assert.equal(data.has('plog:1-1:4'), false);
});

test(
  'Worker secrets by value: the run token secret and the Access id are redacted too',
  { skip: d1Skip },
  async () => {
    const secret = 'runSecretOnly_' + 'Kq7'.repeat(12);
    const { bucket } = await runWith(
      ['leak ' + secret + ' end\n', 'leak ' + 'accessIdValue.access'.repeat(2) + '\n'],
      {
        overrides: { RUN_TOKEN_SECRET: secret, BOX_ACCESS_ID: 'accessIdValue.access'.repeat(2) },
      },
    );
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    assert.doesNotMatch(log, /accessIdValue|runSecretOnly|Kq7Kq7/);
    assert.ok(log.includes('leak [redacted] end'));
  },
);

test(
  'Worker secrets by value: the GitHub, Vast and R2 secrets and the secret path are redacted',
  { skip: d1Skip },
  async () => {
    const values = {
      GITHUB_API_TOKEN: 'github-api-token-value-123456',
      VAST_API_KEY: 'vast-key-value-abcdefghijkl',
      R2_READ_SECRET_ACCESS_KEY: 'r2-read-secret-value-0123456789',
      ROW_TAG_KEY: 'x'.repeat(48),
    };
    const prefix = 'unit-fixture-' + 'p'.repeat(32);
    const { bucket } = await runWith(
      [
        Object.values(values).join(' ') + ' ' + 'runSecretTwo_' + 'Zp4'.repeat(12) + '\n',
        'path ' + prefix + '\n',
        'url ' + baseEnv.WORKER_URL + '/' + prefix + '/x\n',
      ],
      {
        overrides: { ...values, RUN_TOKEN_SECRET: 'runSecretTwo_' + 'Zp4'.repeat(12) },
      },
    );
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    for (const v of [...Object.values(values), prefix, 'runSecretTwo_' + 'Zp4'.repeat(12)])
      assert.ok(!log.includes(v), v.slice(0, 12));
    assert.ok(log.includes('path [redacted]'));
  },
);

test(
  'a published report names the log by LOGS_BASE only when it is a https kayfabe.dev origin',
  { skip: d1Skip },
  async () => {
    const good = await runWith(['x\n']);
    assert.match(
      JSON.parse(good.bucket.objects.get('vast-123-1-0/report.json')).log.url,
      /^https:\/\/logs\.kayfabe\.dev\//,
    );
    const evil = await runWith(['x\n'], { overrides: { LOGS_BASE: 'https://evil.test' } });
    const doc = JSON.parse(evil.bucket.objects.get('vast-123-1-0/report.json'));
    assert.ok(!('url' in doc.log));
    assert.doesNotMatch(JSON.stringify(doc), /evil\.test/);
  },
);

test(
  'in-flight rows are not read as identity (readRunRow answers null until the run is final)',
  { skip: d1Skip },
  async () => {
    const { h, db } = await runWith(['x\n']);
    const row = await readRunRow(h.bindings, 'vast-123-1-0');
    assert.equal(row.verdict, 'PASS');
    assert.equal(row.host_driver, '580.159.04');
    assert.equal(row.tree_kind, 'tree');
    db.database.exec('UPDATE ci_runs SET verdict = NULL, ended_at = NULL, tag = NULL');
    assert.equal(await readRunRow(h.bindings, 'vast-123-1-0'), null);
    assert.equal(await readRunRow(h.bindings, 'vast-999-1-0'), null);
  },
);

test(
  'storeReport is idempotent: the same report twice leaves the same rows',
  { skip: d1Skip },
  async () => {
    const { h, db } = await runWith(['x\n']);
    const doc = JSON.parse(db.rows('SELECT report_json FROM ci_run_reports')[0].report_json);
    const count = () =>
      ['ci_subruns', 'ci_run_tests', 'ci_run_perf', 'ci_run_reports'].map(
        (t) => db.rows('SELECT COUNT(*) AS n FROM ' + t)[0].n,
      );
    const before = count();
    for (let i = 0; i < 2; i++)
      assert.equal(
        await storeReport(h.bindings, {
          runId: 'vast-123-1-0',
          doc,
          hostDriver: '580.159.04',
          log: null,
          now: Date.now(),
        }),
        'STORED',
      );
    assert.deepEqual(count(), before);
  },
);

test('a log identity is recorded only for a log that was published', { skip: d1Skip }, async () => {
  const { h, db, bucket } = await runWith(['x\n']);
  assert.ok(db.rows('SELECT log_bytes FROM ci_run_reports')[0].log_bytes > 0);
  // The bucket binding is removed after the run (a rollback): the entry has no logPut.
  db.database.exec(
    'DELETE FROM ci_run_reports; DELETE FROM ci_run_tests; DELETE FROM ci_run_perf; DELETE FROM ci_subruns',
  );
  await h.storage.put('plogq:123-1:0', {
    attempts: 0,
    chunks: 1,
    dropped: 0,
    rchunks: 1,
    leg: 0,
    key: '123-1',
    run: '123',
    attempt: '1',
    sha,
    nonce,
    runnerNonce: null,
    reportExpiry: 2000000000,
    at: 0,
  });
  const { LOGS_BUCKET, ...rest } = h.bindings;
  void LOGS_BUCKET;
  const ledger = new Ledger(h.storage);
  await publishTick({ ledger, env: rest, now: Date.now() });
  const row = db.rows('SELECT log_bytes, log_sha256 FROM ci_run_reports')[0];
  assert.deepEqual(row, { log_bytes: null, log_sha256: null });
  void bucket;
});

test(
  'attempts are bounded exactly: the last allowed attempt runs, the next one drops the entry',
  { skip: d1Skip },
  async () => {
    const bucket = new Bucket();
    const { h } = await runWith(['x\n'], { overrides: { LOGS_BUCKET: undefined } }).then((r) => ({
      h: r.h,
    }));
    void bucket;
    await h.storage.put('plogq:123-1:0', {
      attempts: MAX_ATTEMPTS - 1,
      chunks: 1,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    await h.storage.put('plog:123-1:0', 'x');
    const failing = {
      put: async () => {
        throw new Error('down');
      },
    };
    const env = { ...h.bindings, LOGS_BUCKET: failing };
    const ledger = new Ledger(h.storage);
    await publishTick({ ledger, env, now: Date.now() });
    assert.equal(
      (await h.storage.get('plogq:123-1:0')).attempts,
      MAX_ATTEMPTS,
      'still there after the last allowed attempt',
    );
    await publishTick({ ledger, env, now: Date.now() });
    assert.equal(await h.storage.get('plogq:123-1:0'), undefined);
  },
);

test(
  'the attempt is counted BEFORE the work: a call that never returns still counts',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await runWith(['x\n']);
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    bucket.hang = true;
    const was = publishTimeouts.r2;
    publishTimeouts.r2 = 800;
    try {
      const running = h.service.publishRuns();
      await new Promise((r) => setTimeout(r, 150));
      assert.equal(
        (await h.storage.get('plogq:123-1:0')).attempts,
        1,
        'saved while the bucket call was still in flight',
      );
      await running;
    } finally {
      publishTimeouts.r2 = was;
    }
  },
);

test(
  'a retry after a failed report write does not write the log again',
  { skip: d1Skip },
  async () => {
    const { h, bucket } = await runWith(['x\n']);
    bucket.objects.clear();
    bucket.puts.length = 0;
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 1,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    const original = bucket.put.bind(bucket);
    let failReport = true;
    bucket.put = async (key, value) => {
      if (failReport && key.endsWith('report.json')) {
        bucket.puts.push(key);
        throw new Error('down');
      }
      return original(key, value);
    };
    await h.service.publishRuns();
    failReport = false;
    await h.service.publishRuns();
    assert.equal(
      bucket.puts.filter((k) => k.endsWith('log.txt')).length,
      1,
      'the log object was written once',
    );
    assert.ok(bucket.objects.has('vast-123-1-0/report.json'));
  },
);

test('neither bucket nor store: the queue entry is left exactly as it was', async () => {
  const { h } = await harness({ LOGS_BUCKET: undefined }, { store: false });
  await h.storage.put('plogq:123-1:0', { attempts: 0, chunks: 1 });
  await h.service.publishRuns();
  assert.deepEqual(await h.storage.get('plogq:123-1:0'), { attempts: 0, chunks: 1 });
});

test(
  'a missing middle piece drops the entry and nothing is written',
  { skip: d1Skip },
  async () => {
    const x = await runWith(['a\n', 'b\n', 'c\n', 'd\n']);
    const { h, bucket } = x;
    bucket.objects.clear();
    bucket.puts.length = 0;
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 4,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    await h.storage.delete('plog:123-1:2');
    await h.service.publishRuns();
    assert.equal(bucket.puts.length, 0);
    assert.equal(await h.storage.get('plogq:123-1:0'), undefined);
  },
);

test(
  'a replacement leg continues the log chain: counts and numbering carry over',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    Object.assign(h.cloud.offer, { host_id: 100 });
    h.cloud.contracts = [900, 901];
    await h.call();
    await cron(h, 2);
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 },
    ];
    const r0 = await h.storage.get('record:123-1');
    await h.report({ seq: 1, kind: 'log', text: 'ZERO\n' });
    await h.report({
      seq: 2,
      kind: 'result',
      result: {
        nonce: r0.nonce,
        verdict: 'PROVIDER_FAULT',
        suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
      },
    });
    const settled = await h.storage.get('record:123-1');
    assert.ok(settled.plogChunks >= 1 && settled.plogBytes > 0);
    await h.call();
    const r1 = await h.storage.get('record:123-1');
    assert.ok(
      r1.plogChunks >= settled.plogChunks,
      'carried into the replacement record (plus its own Worker lines)',
    );
    assert.ok(r1.plogBytes >= settled.plogBytes);
    const before = r1.plogChunks;
    await h.report({ seq: 3, kind: 'log', text: 'ONE\n' });
    assert.equal(await h.storage.get('plog:123-1:' + before), 'ONE\n', 'numbering continues');
    assert.equal((await h.storage.get('plog:123-1:0')) !== undefined, true);
  },
);

test(
  'route reports with arrays other than tests are fitted too (apps and perf)',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const apps = Array.from({ length: 500 }, (_, i) => ({
      sub: 'host',
      name: 'app-' + String(i).padStart(3, '0') + '-' + 'a'.repeat(60),
      os: 'linux',
      result: 'PASS',
      note: 'n'.repeat(100),
    }));
    const perf = Array.from({ length: 200 }, (_, i) => ({
      sub: 'host',
      name: 'perf-' + i + '-' + 'p'.repeat(90),
      value: i + 0.123456789,
      unit: 'ms',
      higher_is_better: false,
      suite: 'cuda_host',
      ref_value: 1.5,
      ref_ratio: 0.5,
    }));
    const tests = Array.from({ length: 1000 }, (_, i) => ({
      sub: 'host',
      suite: 'raw_suite',
      name: 'arm-' + String(i).padStart(4, '0') + '-' + 'n'.repeat(60),
      result: 'PASS',
    }));
    const body = JSON.stringify(base({ apps, perf, tests }));
    const r = await post(h, null, { raw: body });
    if (body.length > REPORT_MAX_BYTES) assert.equal(r.status, 413);
    else {
      assert.equal(r.status, 200);
      const doc = await stored(h);
      assert.ok(Buffer.byteLength(JSON.stringify(doc)) <= REPORT_MAX_BYTES);
    }
  },
);

test('buildReport shows a tree only for a Worker-verified key (not a claimed or commit key)', () => {
  const claims = cleanClaimsReport(base());
  const row = {
    commit_sha: sha,
    tree_hash: 'c'.repeat(64),
    tier: 'middle',
    die: 'AD104',
    family: 'ada',
    verdict: 'PASS',
    started_at: 'a',
    ended_at: 'b',
  };
  for (const kind of ['claimed', 'commit'])
    assert.equal(
      buildReport({
        claims,
        row: { ...row, tree_kind: kind },
        runId: 'vast-1-1-0',
        log: null,
        offsets: null,
        logsBase: null,
      }).tree,
      null,
      kind,
    );
  assert.equal(
    buildReport({
      claims,
      row: { ...row, tree_kind: 'tree' },
      runId: 'vast-1-1-0',
      log: null,
      offsets: null,
      logsBase: null,
    }).tree,
    'c'.repeat(64),
  );
});

test(
  'an e2e log keeps a jwt-shaped file name and redacts the presigned url whole',
  { skip: d1Skip },
  async () => {
    const url =
      'https://b.test/o?X-Amz-Algorithm=A&X-Amz-Credential=AK%2Fx&X-Amz-Signature=' +
      'ab'.repeat(32);
    const { bucket } = await runWith([
      'file eyJabcdefghij.kkkkkk.short here\n',
      'u ' + url + ' end\n',
      'CF-Access-Client-Id: abcdef0123.access\n',
      '-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\nafter-key\n',
      'api_key=1234567 stays\n',
    ]);
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    assert.ok(log.includes('file eyJabcdefghij.kkkkkk.short here'));
    assert.ok(log.includes('u [redacted] end'));
    assert.ok(log.includes('CF-Access-Client-Id: [redacted]'));
    assert.ok(log.includes('[redacted]\nafter-key'));
    assert.ok(!log.includes('AAAA'));
    assert.ok(log.includes('api_key=1234567 stays'));
    assert.doesNotMatch(log, /b\.test|X-Amz/);
  },
);

test(
  'an e2e log: the split token is caught across two posts (one redaction, not two halves)',
  { skip: d1Skip },
  async () => {
    const { bucket, h } = await runWith((record, token) => [
      't ' + token.slice(0, 70),
      token.slice(70) + ' u\n',
    ]);
    const log = bucket.objects.get('vast-123-1-0/log.txt');
    assert.match(log, /t \[redacted\] u\n/);
    void h;
  },
);

test('a 100 KB report reaches the ledger whole through the gateway (the report limit, not the log limit)', async () => {
  const { createGateway } = await import('../src/index.js');
  const gateway = createGateway({ fetcher: async () => new Response('') });
  const seen = [];
  const environment = {
    ...baseEnv,
    BROKER_PATH_PREFIX: 'p'.repeat(40),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({ fetch: async (r) => (seen.push((await r.text()).length), new Response('{}')) }),
    },
  };
  const token = callbackToken(
    {
      run: '123',
      attempt: '1',
      sha,
      nonce,
      key: '123-1',
      label: 'kf-ci-123-1',
      reportExpiry: Math.floor(Date.now() / 1000) + 3600,
    },
    baseEnv.RUN_TOKEN_SECRET,
  );
  const body = JSON.stringify({ schema: 1, pad: 'y'.repeat(100000) });
  const r = await gateway.fetch(
    new Request('https://broker.example/' + 'p'.repeat(40) + '/run-report/123-1', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + token },
      body,
    }),
    environment,
  );
  assert.equal(r.status, 200);
  assert.deepEqual(seen, [body.length]);
});

test(
  'route fits a report made of other arrays too (linux subrun rows and apps)',
  { skip: d1Skip },
  async () => {
    const { h } = await harness();
    await h.call();
    const row = (i, pad) => ({
      sub: 'linux-580.159.03',
      suite: 'thin_suite',
      name: 'thin-' + String(i).padStart(4, '0') + '-' + 'q'.repeat(pad),
      result: 'PASS',
    });
    const app = (i) => ({ sub: 'host', name: 'app-' + i, os: 'linux', result: 'PASS' });
    const apps = Array.from({ length: 100 }, (_, i) => app(i));
    const shell = JSON.stringify(base({ tests: [], apps })).length;
    const pad = Math.floor(
      (REPORT_MAX_BYTES - 2000 - shell) / 2000 - JSON.stringify(row(0, 0)).length - 1,
    );
    const body = JSON.stringify(
      base({ tests: Array.from({ length: 2000 }, (_, i) => row(i, pad)), apps }),
    );
    assert.ok(body.length <= REPORT_MAX_BYTES);
    assert.equal((await post(h, null, { raw: body })).status, 200);
    const doc = await stored(h);
    assert.ok(Buffer.byteLength(JSON.stringify(doc)) <= REPORT_MAX_BYTES);
    assert.ok(doc.truncated.includes('tests'));
    assert.equal(doc.apps.length, 100, 'the smaller arrays are kept while the largest is cut');
  },
);

test(
  'a log identity is not recorded when only the report goes to D1 and the bucket is gone',
  { skip: d1Skip },
  async () => {
    const { h, db } = await runWith(['x\n']);
    db.database.exec(
      'DELETE FROM ci_run_reports; DELETE FROM ci_run_tests; DELETE FROM ci_run_perf; DELETE FROM ci_subruns',
    );
    await h.storage.put('plogq:123-1:0', {
      attempts: 0,
      chunks: 1,
      dropped: 0,
      rchunks: 0,
      leg: 0,
      key: '123-1',
      run: '123',
      attempt: '1',
      sha,
      nonce,
      runnerNonce: null,
      reportExpiry: 2000000000,
      at: 0,
    });
    const { LOGS_BUCKET, ...rest } = h.bindings;
    void LOGS_BUCKET;
    await publishTick({ ledger: new Ledger(h.storage), env: rest, now: Date.now() });
    assert.deepEqual(db.rows('SELECT report_json, log_bytes, log_sha256 FROM ci_run_reports'), [
      { report_json: null, log_bytes: null, log_sha256: null },
    ]);
  },
);
