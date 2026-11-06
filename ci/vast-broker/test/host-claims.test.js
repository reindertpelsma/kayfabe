import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { checkedResult, cleanClaims, recordRun, CLAIM_KEYS, guestsOf } from '../src/results.js';
import { parseReport } from '../src/reports.js';
import { FakeD1, d1Skip } from './d1.js';
// Host claims (setup picker, 2026-11-06): the box's optional, bounded, unverified facts.
// FALSIFIER: a claim refuses or blocks a report or a row; a claim reaches the store unbounded or
// shaped like an identifier; an old box (no claims) or a result with an unknown top-level key
// behaves differently from before; a claim changes the HMAC tag or a verdict; the Worker and the
// box disagree on what a clean claim is (shared vectors).

const vectors = JSON.parse(
  readFileSync(new URL('../../../scripts/ci/host_claims_vectors.json', import.meta.url), 'utf8'),
);

test('claims: cleanClaims agrees with the shared vectors (the box checks the same)', () => {
  assert.ok(vectors.cases.length >= 25);
  for (const c of vectors.cases) {
    const want = Object.fromEntries(CLAIM_KEYS.map((k) => [k, null]));
    Object.assign(want, c.expect);
    assert.deepEqual(cleanClaims(c.input), want, c.name);
  }
});

test('claims: cleanClaims is total and never throws', () => {
  for (const hostile of [
    undefined,
    null,
    0,
    '',
    [],
    {},
    Object.create(null),
    {
      cpu_model: {
        toString() {
          throw Error('x');
        },
      },
    },
    JSON.parse('{"__proto__": {"cpu_vendor": "amd"}}'),
    {
      cpu_vendor: 'amd',
      get cpu_arch() {
        return 'x86_64';
      },
    },
  ])
    assert.doesNotThrow(() => cleanClaims(hostile));
  // __proto__ is data, not a prototype: it grants nothing.
  assert.equal(cleanClaims(JSON.parse('{"__proto__": {"cpu_vendor": "amd"}}')).cpu_vendor, null);
});

const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };
const report = (extra = {}) => ({
  nonce: 'a'.repeat(32),
  runner_nonce: 'b'.repeat(64),
  sha: 'c'.repeat(40),
  verdict: 'PASS',
  suites: [{ name: 'gates', pass: 1, fail: 0, skip: 0 }],
  hardware: { ...hardware },
  ...extra,
});
const body = (result) => JSON.stringify({ seq: 2, kind: 'result', result });

test('report: an old box (no claims) and a box with claims are both accepted', () => {
  assert.doesNotThrow(() => checkedResult(report()));
  assert.equal(checkedResult(report()).host_claims, undefined, 'nothing is invented');
  const claims = {
    cpu_vendor: 'amd',
    cpu_model: 'AMD EPYC 7763 64-Core Processor',
    cpu_arch: 'x86_64',
    linux_os_version: 'ubuntu-24.04',
    linux_driver: '580.159.04',
    windows_os_version: '11-ltsc-2024-26100',
    windows_driver: '580.88',
  };
  const parsed = parseReport(body(report({ host_claims: claims })));
  assert.equal(parsed.result.host_claims.cpu_vendor, 'amd');
  assert.equal(parsed.result.host_claims.windows_driver, '580.88');
  assert.equal(parsed.result.host_claims.host_driver, null);
  // The nesting limit of parseReport (depth 4) still holds with the claims block in place.
  assert.equal(Object.keys(parsed.result).includes('host_claims'), true);
});

test('report: a bad claim is dropped, it never refuses the report or a PASS', () => {
  for (const bad of [
    'amd',
    ['amd'],
    7,
    null,
    { cpu_vendor: 'via', cpu_model: 'x'.repeat(500), linux_driver: '<script>', machine_id: 'm1' },
    { cpu_model: 'GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001' },
  ]) {
    const result = checkedResult(report({ host_claims: bad }));
    assert.equal(result.verdict, 'PASS');
    assert.deepEqual(Object.keys(result.host_claims).sort(), [...CLAIM_KEYS].sort());
    assert.ok(
      Object.values(result.host_claims).every((v) => v === null),
      JSON.stringify(bad),
    );
  }
});

test('report: strictness elsewhere is unchanged (unknown keys and bad hardware still refuse)', () => {
  assert.throws(() => checkedResult(report({ extra: 1 })));
  assert.throws(() => checkedResult(report({ host_claim: {} })), 'a near-miss key name');
  assert.throws(() => checkedResult(report({ hardware: { ...hardware, cpu: 'amd' } })));
  assert.throws(() => checkedResult(report({ hardware: { gpu_name: 'RTX_4070' } })));
  // gpu_name and the PASS conditions are the Worker's old rules; claims cannot satisfy them.
  assert.throws(() =>
    checkedResult(
      report({
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
        host_claims: { cpu_vendor: 'amd', cpu_model: 'AMD EPYC 7763' },
      }),
    ),
  );
});

// --- the store ----------------------------------------------------------------------------

const KEY = 'k'.repeat(40);
const T0 = Date.UTC(2026, 10, 6, 10, 0, 0);
const env = (db) => ({ DB: db, ROW_TAG_KEY: KEY });
const row = (patch = {}) => ({
  run_id: 'vast-300-1-0',
  commit_sha: 'c'.repeat(40),
  tree_hash: 'a'.repeat(64),
  tier: 'middle',
  profile: 'check',
  die: 'GA106',
  family: 'ampere',
  platform: 'both',
  verdict: 'PASS',
  stages: [
    { name: 'gates', rc: 0, duration_ms: 1000, result: 'PASS' },
    { name: 'windows_gate', rc: 0, duration_ms: 2000, result: 'PASS' },
  ],
  gpu_name: 'RTX_3060',
  driver_versions: {},
  claims: {
    cpu_vendor: 'amd',
    cpu_model: 'AMD EPYC 7763 64-Core Processor',
    cpu_arch: 'x86_64',
    host_driver: '580.95.05',
    linux_os_version: 'ubuntu-24.04',
    linux_driver: '580.159.04',
    windows_os_version: '11-ltsc-2024-26100',
    windows_driver: '580.88',
  },
  machine_id: 4242,
  price_usd_h: 0.4,
  cost_usd: 1.25,
  started_at: T0,
  ended_at: T0 + 3600000,
  log_key: null,
  ...patch,
});
const claimRows = (db) =>
  db.rows(
    'SELECT guest_os, guest_os_version, guest_driver, host_driver, cpu_vendor, cpu_model, cpu_arch, claimed FROM ci_run_claims ORDER BY guest_os',
  );

test(
  'store: one claims row per guest OS the run covered, marked as claimed',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    assert.equal((await recordRun(env(db), row())).state, 'INSERTED');
    assert.deepEqual(claimRows(db), [
      {
        guest_os: 'linux',
        guest_os_version: 'ubuntu-24.04',
        guest_driver: '580.159.04',
        host_driver: '580.95.05',
        cpu_vendor: 'amd',
        cpu_model: 'AMD EPYC 7763 64-Core Processor',
        cpu_arch: 'x86_64',
        claimed: 1,
      },
      {
        guest_os: 'windows',
        guest_os_version: '11-ltsc-2024-26100',
        guest_driver: '580.88',
        host_driver: '580.95.05',
        cpu_vendor: 'amd',
        cpu_model: 'AMD EPYC 7763 64-Core Processor',
        cpu_arch: 'x86_64',
        claimed: 1,
      },
    ]);
    // Linux-only and Windows-only runs: one row, the guest values of that OS.
    await recordRun(env(db), row({ run_id: 'vast-300-2-0', platform: 'linux', stages: [] }));
    await recordRun(env(db), row({ run_id: 'vast-300-3-0', platform: 'windows', stages: [] }));
    const one = db.rows(
      "SELECT run_id, guest_os, guest_driver FROM ci_run_claims WHERE run_id <> 'vast-300-1-0' ORDER BY run_id",
    );
    assert.deepEqual(one, [
      { run_id: 'vast-300-2-0', guest_os: 'linux', guest_driver: '580.159.04' },
      { run_id: 'vast-300-3-0', guest_os: 'windows', guest_driver: '580.88' },
    ]);
    assert.deepEqual(
      [guestsOf('both'), guestsOf('linux'), guestsOf('x')],
      [['linux', 'windows'], ['linux'], []],
    );
  },
);

test(
  'store: claims never change the tag, never block a row, never add private data',
  { skip: d1Skip },
  async () => {
    const a = new FakeD1();
    const b = new FakeD1();
    await recordRun(env(a), row());
    await recordRun(env(b), row({ claims: undefined }));
    assert.equal(
      a.rows('SELECT tag FROM ci_runs')[0].tag,
      b.rows('SELECT tag FROM ci_runs')[0].tag,
      'the tag covers the verdict fields, not the claims',
    );
    assert.deepEqual(claimRows(b), [], 'no claims, no rows (an older box)');
    // Hostile claims: the row is still recorded and the claims are null, not an error.
    const c = new FakeD1();
    const hostile = {
      cpu_vendor: 'amd; DROP TABLE ci_runs',
      cpu_model: 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9',
      host_driver: '10.0.0.7',
      linux_os_version: 'x'.repeat(200),
      windows_driver: 'machine 12345',
    };
    assert.equal((await recordRun(env(c), row({ claims: hostile }))).state, 'INSERTED');
    assert.deepEqual(claimRows(c), []);
    assert.equal(c.rows('SELECT verdict FROM ci_runs')[0].verdict, 'PASS');
    // The claims table holds no machine, price or cost.
    const columns = a
      .rows("SELECT name FROM pragma_table_info('ci_run_claims')")
      .map((r) => r.name);
    assert.ok(
      !columns.some((n) => /machine|price|cost|token|address|host(?!_driver)|serial|uuid/i.test(n)),
      columns.join(),
    );
    const text = JSON.stringify(a.rows('SELECT * FROM ci_run_claims'));
    assert.doesNotMatch(text, /4242|0\.4|1\.25/);
  },
);

test(
  'store: an in-flight row has no claims, a repeated final call adds none',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    await recordRun(
      env(db),
      row({ verdict: null, ended_at: null, stages: [], claims: row().claims }),
    );
    assert.deepEqual(claimRows(db), [], 'claims arrive with the result');
    await recordRun(env(db), row());
    assert.equal(claimRows(db).length, 2);
    assert.equal((await recordRun(env(db), row())).state, 'DUPLICATE');
    assert.equal(claimRows(db).length, 2);
    // A conflicting final call changes nothing, claims included.
    await recordRun(env(db), row({ verdict: 'FAIL', stages: [], claims: { cpu_vendor: 'intel' } }));
    assert.deepEqual(db.rows('SELECT DISTINCT cpu_vendor FROM ci_run_claims'), [
      { cpu_vendor: 'amd' },
    ]);
  },
);

test('store: the claims table was added by 0003 and 0001/0002 are untouched', () => {
  const dir = new URL('../../platform-api/migrations/', import.meta.url);
  const sql = readFileSync(new URL('0003_ci_run_claims.sql', dir), 'utf8').replace(/--[^\n]*/g, '');
  assert.doesNotMatch(sql, /\b(?:ALTER|DROP|DELETE|UPDATE|INSERT)\b/i);
  assert.match(sql, /CREATE TABLE ci_run_claims/);
});

// --- fail-open claims (review of #271, M1) ---------------------------------------------------
// FALSIFIER: a missing or failing ci_run_claims costs a run its row, its verdict or its finalisation.

async function captured(work) {
  const lines = [];
  const original = console.log;
  console.log = (...args) => lines.push(args.join(' '));
  try {
    return { value: await work(), lines };
  } finally {
    console.log = original;
  }
}

test(
  'store: without migration 0003 the run is still stored and one line names the config error',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1({ upTo: 2 }); // 0001 and 0002 only: no ci_run_claims
    const { value, lines } = await captured(() => recordRun(env(db), row()));
    assert.equal(value.state, 'INSERTED');
    assert.equal(value.stored, true);
    assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'PASS' }]);
    assert.equal(db.rows('SELECT * FROM ci_run_stages').length, 2);
    assert.equal(db.rows('SELECT * FROM ci_host_facts').length, 1);
    const says = lines.filter((l) => l.includes('CLAIMS_FAILED'));
    assert.equal(says.length, 1);
    assert.match(says[0], /table ci_run_claims is missing; apply ci\/platform-api migration 0003/);
    // An in-flight row is finalised the same way.
    const inflight = await recordRun(
      env(db),
      row({ run_id: 'vast-300-9-0', verdict: null, ended_at: null, stages: [] }),
    );
    assert.equal(inflight.state, 'INSERTED');
    const final = await captured(() => recordRun(env(db), row({ run_id: 'vast-300-9-0' })));
    assert.equal(final.value.state, 'FINALIZED');
    assert.deepEqual(db.rows("SELECT verdict FROM ci_runs WHERE run_id = 'vast-300-9-0'"), [
      { verdict: 'PASS' },
    ]);
  },
);

test(
  'store: any other failure of the claims batch also leaves the row stored',
  { skip: d1Skip },
  async () => {
    class Flaky extends FakeD1 {
      async batch(statements) {
        if (statements.some((s) => s.sql.includes('ci_run_claims')))
          throw new Error('D1_ERROR: boom');
        return super.batch(statements);
      }
    }
    const db = new Flaky();
    const { value, lines } = await captured(() => recordRun(env(db), row()));
    assert.equal(value.state, 'INSERTED');
    assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'PASS' }]);
    assert.deepEqual(db.rows('SELECT * FROM ci_run_claims'), []);
    assert.ok(lines.some((l) => l.includes('CLAIMS_FAILED')));
    assert.ok(!lines.join('\n').includes('boom'), 'the error text is not logged');
  },
);

test('store: a run without claims never touches the claims table', { skip: d1Skip }, async () => {
  const db = new FakeD1({ upTo: 2 });
  const { value, lines } = await captured(() => recordRun(env(db), row({ claims: undefined })));
  assert.equal(value.state, 'INSERTED');
  assert.deepEqual(
    lines.filter((l) => l.includes('CLAIMS')),
    [],
  );
});
