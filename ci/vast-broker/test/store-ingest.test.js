import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { setup, nonce, sha, jwt, clock, env as baseEnv, MemoryStorage } from './helpers.js';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { FakeD1, d1Skip } from './d1.js';
import { treeKeyOf, patternsFor, commitKey } from '../src/treekey.js';
import {
  storeVerdict,
  storeTierOf,
  storeTierOfRecord,
  rowFromRecord,
  treeKeyFor,
  storeTick,
  TREE_ATTEMPTS,
} from '../src/ingest.js';
// Ledger -> D1 ingest and the read-only requirements check, through the real service.
// FALSIFIER: the store changes a ledger verdict or rental; a PASS row appears without a verified
// box PASS; a final row is missing after settlement; a FAIL is not recorded; the check rents,
// queues, or answers without OIDC; nothing is queued/written when D1 or ROW_TAG_KEY is unset.

const TAG_KEY = 't'.repeat(48);
const hex = (n) => n.toString(16).padStart(40, '0');
const FILES = [
  { path: 'Cargo.toml', mode: '100644', sha: hex(1) },
  { path: 'crates/kf-a/src/lib.rs', mode: '100644', sha: hex(2) },
];
const REQ = `version = 1
[[require]]
tier = "middle"
family = "any"
dies = ["any"]
platforms = ["linux", "windows"]
stages = ["gates"]
`;

async function harness({ store = true, treeFails = false, requirements = REQ } = {}) {
  const db = store ? new FakeD1() : null;
  const h = await setup(store ? { DB: db, ROW_TAG_KEY: TAG_KEY } : {});
  const base = h.cloud.fetch;
  const gh = [];
  h.cloud.fetch = async (input, init = {}) => {
    const u = new URL(String(input));
    if (u.hostname === 'api.github.com') {
      const p = u.pathname.replace(/^\/repos\/[^/]+\/[^/]+\//, '');
      if (/^git\//.test(p) || /^contents\/ci\/requirements\.toml$/.test(p)) {
        gh.push(p + u.search);
        if (treeFails && /^git\//.test(p)) return new Response('', { status: 502 });
        if (p === 'git/commits/' + sha) return Response.json({ tree: { sha: hex(100) } });
        if (p === 'git/trees/' + hex(100))
          return Response.json({
            truncated: false,
            tree: [
              { path: 'Cargo.toml', mode: '100644', type: 'blob', sha: hex(1) },
              { path: 'crates', mode: '040000', type: 'tree', sha: hex(101) },
            ],
          });
        if (p === 'git/trees/' + hex(101))
          return Response.json({
            truncated: false,
            tree: [
              { path: 'kf-a', mode: '040000', type: 'tree', sha: hex(102) },
              { path: 'kf-a/src', mode: '040000', type: 'tree', sha: hex(103) },
              { path: 'kf-a/src/lib.rs', mode: '100644', type: 'blob', sha: hex(2) },
            ],
          });
        if (p === 'contents/ci/requirements.toml') {
          if (requirements === null) return new Response('{}', { status: 404 });
          return Response.json({
            type: 'file',
            encoding: 'base64',
            content: Buffer.from(requirements).toString('base64'),
          });
        }
        return new Response('', { status: 404 });
      }
    }
    return base(input, init);
  };
  h.restart();
  return { h, db, gh };
}
// One cron tick: the serial sweep, then the tree keys outside the queue (BrokerLedger /cron).
const cron = async (h, n = 1) => {
  for (let i = 0; i < n; i++) {
    await h.service.sweep();
    await h.service.storeTrees();
  }
};
const storeKeys = (h) => [...h.storage.data.keys()].filter((k) => k.startsWith('store:'));
const check = (h, body = { commit_sha: sha, tier: 'middle' }, opts = {}) =>
  h.call('/v1/requirements/check', { body, ...opts });
const passSuites = (extra = []) =>
  ['provision', 'gates', 'raw_suite', ...extra].map((name) => ({
    name,
    pass: 1,
    fail: 0,
    skip: 0,
  }));
const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };

test('ingest: no D1 or no tag key -> nothing queued, no tree fetch', async () => {
  for (const overrides of [{}, { DB: new FakeD1() }, { ROW_TAG_KEY: TAG_KEY }]) {
    if (overrides.DB === undefined && d1Skip && overrides.ROW_TAG_KEY) continue;
    const h = await setup(overrides);
    assert.equal((await h.call()).status, 200);
    await h.service.sweep();
    assert.deepEqual(storeKeys(h), []);
    assert.ok(!h.cloud.calls.some((c) => c.url.includes('/git/')));
  }
});

test(
  'ingest: in-flight row, then the final PASS row after settlement',
  { skip: d1Skip },
  async () => {
    const { h, db } = await harness();
    assert.equal((await h.call()).status, 200);
    assert.deepEqual(storeKeys(h), ['store:123-1:0']);
    await cron(h, 2); // the first tick computes the tree key, the second records the row
    const key = await treeKeyOf(FILES, patternsFor(undefined));
    const inflight = db.rows(
      'SELECT run_id, verdict, tree_hash, tree_kind, tier, die, family, platform FROM ci_runs',
    );
    assert.deepEqual(inflight, [
      {
        run_id: 'vast-123-1-0',
        verdict: null,
        tree_hash: key,
        tree_kind: 'tree',
        tier: 'middle',
        die: 'AD104',
        family: 'ada',
        platform: 'both',
      },
    ]);
    // The check offers it for joining and rents nothing.
    const creates = h.cloud.creates;
    const joined = await (await check(h)).json();
    assert.equal(joined.met, false);
    assert.deepEqual(joined.inflight, ['vast-123-1-0']);
    assert.equal(joined.tree_hash, key);
    assert.equal(h.cloud.creates, creates);
    // Box PASS, grace, cleanup, settlement, then the store tick records the final row.
    await h.report({ seq: 1, kind: 'progress', text: 'started' });
    const r = await h.report({
      seq: 2,
      kind: 'result',
      result: { nonce, verdict: 'PASS', suites: passSuites(), hardware },
    });
    assert.equal(r.status, 200);
    h.time.value += 6 * 60000;
    await cron(h);
    const record = await h.storage.get('record:123-1');
    assert.equal(record.verdict, 'PASS', 'the ledger verdict is untouched');
    assert.notEqual(record.ended, null);
    if (storeKeys(h).length) await cron(h);
    assert.deepEqual(storeKeys(h), []);
    assert.deepEqual(db.rows('SELECT verdict, tier FROM ci_runs'), [
      { verdict: 'PASS', tier: 'middle' },
    ]);
    assert.deepEqual(
      db.rows('SELECT name, result FROM ci_run_stages ORDER BY ord').map((s) => s.name),
      ['provision', 'gates', 'raw_suite'],
    );
    assert.deepEqual(db.rows('SELECT gpu_name, driver_versions_json FROM ci_host_facts'), [
      { gpu_name: 'RTX_4070', driver_versions_json: '{"host":"580.159.04"}' },
    ]);
    assert.equal(db.rows('SELECT machine_id FROM ci_runs_private')[0].machine_id, 100001);
    const met = await (await check(h)).json();
    assert.equal(met.met, true);
    assert.match(met.requirements_sha256, /^[0-9a-f]{64}$/);
    assert.equal(db.rows('SELECT base_ref FROM ci_requirement_snapshots')[0].base_ref, 'master');
  },
);

test('ingest: a box FAIL is recorded (cached) as FAIL', { skip: d1Skip }, async () => {
  const { h, db } = await harness();
  await h.call();
  await h.report({ seq: 1, kind: 'progress', text: 'started' });
  await h.report({
    seq: 2,
    kind: 'result',
    result: {
      nonce,
      verdict: 'FAIL',
      suites: [{ name: 'gates', pass: 0, fail: 1, skip: 0 }],
      hardware,
    },
  });
  h.time.value += 11 * 60000;
  await cron(h, 2);
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'FAIL' }]);
  const answer = await (await check(h)).json();
  assert.equal(answer.met, false);
  assert.equal(answer.missing[0].reason, 'FAIL');
});

test('ingest: a box that never reports is UNATTRIBUTED, never PASS', { skip: d1Skip }, async () => {
  const { h, db } = await harness();
  await h.call();
  h.time.value += 11 * 60000; // past BOOT_LIMIT_MINUTES=10
  await cron(h, 2);
  const record = await h.storage.get('record:123-1');
  assert.notEqual(record.ended, null);
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'UNATTRIBUTED' }]);
  assert.equal((await (await check(h)).json()).missing[0].reason, 'NO_RESULT');
});

test(
  'ingest: a tree that cannot be listed falls back to the per-commit key',
  { skip: d1Skip },
  async () => {
    const { h, db } = await harness({ treeFails: true });
    await h.call();
    await cron(h, 5); // 3 failed attempts, the per-commit key, then the row
    assert.deepEqual(db.rows('SELECT tree_hash, tree_kind FROM ci_runs'), [
      { tree_hash: await commitKey(sha), tree_kind: 'commit' },
    ]);
    const answer = await (await check(h)).json();
    assert.equal(answer.tree_kind, 'commit');
  },
);

test('check: OIDC, body and store are required; it never rents', { skip: d1Skip }, async () => {
  const { h } = await harness();
  const ledgerKeys = () => [...h.storage.data.keys()].filter((k) => !k.startsWith('treekey:'));
  const before = ledgerKeys().sort();
  assert.equal((await check(h, undefined, { token: 'not-a-jwt' })).status, 401);
  assert.equal((await check(h, undefined, { protocol: null })).status, 409);
  for (const body of [
    { commit_sha: sha, base: 'next', tier: 'middle' }, // the Worker picks the base, not the caller
    { commit_sha: sha, tier: 'adhoc' },
    { commit_sha: 'x', tier: 'middle' },
    { commit_sha: sha, tier: 'middle', extra: 1 },
  ])
    assert.equal((await check(h, body)).status, 400, JSON.stringify(body));
  // The run must be triggered by the owner, as for /availability and /rent.
  const actor = h.cloud.run.triggering_actor;
  h.cloud.run.triggering_actor = { id: 999 };
  assert.equal((await check(h)).status, 401);
  h.cloud.run.triggering_actor = actor;
  const bad = await h.call('/v1/requirements/check', { method: 'GET' });
  assert.equal(bad.status, 404);
  const ok = await check(h);
  assert.equal(ok.status, 200);
  assert.equal((await ok.json()).base, 'master');
  assert.deepEqual(ledgerKeys().sort(), before, 'no ledger write besides the tree-key cache');
  const small = await (await check(h, { commit_sha: sha, tier: 'small' })).json();
  assert.equal(small.base, 'next');
  assert.equal(h.cloud.creates, 0);
  // A missing requirements file is not met.
  const none = await harness({ requirements: null });
  assert.equal((await (await check(none.h)).json()).missing[0].reason, 'REQUIREMENTS_INVALID');
  // Without the store the check is unavailable, not "met".
  const off = await setup();
  assert.equal((await check(off)).status, 503);
});

test('ingest: verdict attribution never turns a non-result into PASS', () => {
  const base = { ended: 1, verdict: 'PASS', result: { verdict: 'PASS' }, resultAt: 1 };
  assert.equal(storeVerdict(base), 'PASS');
  assert.equal(storeVerdict({ ...base, ended: null }), null);
  assert.equal(storeVerdict({ ...base, resultAt: undefined }), 'UNATTRIBUTED');
  assert.equal(storeVerdict({ ...base, result: { verdict: 'FAIL' } }), 'UNATTRIBUTED');
  assert.equal(storeVerdict({ ended: 1, verdict: 'PROVIDER_FAULT' }), 'RETRY');
  assert.equal(storeVerdict({ ended: 1, verdict: 'UNAVAILABLE' }), 'UNATTRIBUTED');
  assert.equal(storeVerdict({ ended: 1, verdict: 'FAIL', result: { verdict: 'FAIL' } }), 'FAIL');
  for (const closed of ['HARD_TIMEOUT', 'HEARTBEAT_TIMEOUT', 'LIMIT', 'TTL'])
    assert.equal(storeVerdict({ ended: 1, verdict: 'FAIL', closed, firstReportAt: 1 }), 'FAIL');
  for (const closed of ['STOPPED', 'SUPERSEDED', 'RUN_GONE', 'ENDED', 'HARD_TIMEOUT'])
    assert.equal(storeVerdict({ ended: 1, verdict: 'FAIL', closed }), 'UNATTRIBUTED', closed);
  assert.equal(storeTierOf('check'), 'middle');
  assert.equal(storeTierOf('sweep'), 'adhoc');
  assert.equal(storeTierOf('large'), 'largest');
  assert.equal(storeTierOf('mystery'), 'adhoc');
  const row = rowFromRecord(
    { key: '5-1', sha, profile: 'sweep', die: 'GA102', started: clock, ended: null, price: 0.3 },
    0,
    { key: 'f'.repeat(64), kind: 'tree' },
    clock + 1000,
  );
  assert.equal(row.platform, 'linux');
  assert.equal(row.verdict, null);
  assert.equal(row.run_id, 'vast-5-1-0');
});

test(
  'ingest: a provider-fault replacement keeps the first leg as RETRY',
  { skip: d1Skip },
  async () => {
    const { h, db } = await harness();
    Object.assign(h.cloud.offer, { host_id: 100 });
    h.cloud.contracts = [900, 901];
    assert.equal((await h.call()).status, 200);
    await cron(h, 2); // in-flight row of leg 0
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 },
    ];
    const r0 = await h.storage.get('record:123-1');
    const fault = await h.report({
      seq: 1,
      kind: 'result',
      result: {
        nonce: r0.nonce,
        verdict: 'PROVIDER_FAULT',
        suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
      },
    });
    assert.equal(fault.status, 200);
    assert.equal((await h.storage.get('record:123-1')).verdict, 'PROVIDER_FAULT');
    // The replacement leg overwrites record:123-1 before any sweep could record leg 0's end.
    assert.equal((await h.call()).status, 200);
    assert.equal((await h.storage.get('record:123-1')).providerRetries, 1);
    assert.deepEqual(storeKeys(h).sort(), ['store:123-1:0', 'store:123-1:1']);
    await cron(h);
    assert.deepEqual(db.rows('SELECT run_id, verdict FROM ci_runs ORDER BY run_id'), [
      { run_id: 'vast-123-1-0', verdict: 'RETRY' },
      { run_id: 'vast-123-1-1', verdict: null },
    ]);
    assert.deepEqual(storeKeys(h), ['store:123-1:1']);
  },
);

// --- review of #258: H1 (attempt first, budgets, expiry first), M1 (outside the queue), L1, L2 ---

const never = () => new Promise(() => {});
const hungDb = () => ({
  prepare: () => {
    const statement = {
      bind: () => statement,
      first: never,
      all: never,
      run: never,
    };
    return statement;
  },
  batch: never,
});
// Wait until the code under test has reached a state; a fixed 30 ms is not enough on a loaded CI
// runner (the same one-tick assumption that made the evidence cap test flaky, #279).
async function until(condition, what, ms = 20000) {
  const end = Date.now() + ms;
  for (;;) {
    if (await condition()) return;
    if (Date.now() > end) throw new Error('timed out waiting for ' + what);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

test('ingest: a tree key that never returns counts its attempt and gives up', async () => {
  const ledger = new Ledger(new MemoryStorage());
  for (let i = 1; i <= TREE_ATTEMPTS; i++) {
    assert.equal(await treeKeyFor({ ledger, sha, now: clock, treeKey: never, budgetMs: 10 }), null);
    assert.deepEqual(await ledger.get('treekey:' + sha), {
      kind: 'pending',
      attempts: i,
      at: clock,
    });
  }
  const fallback = await treeKeyFor({ ledger, sha, now: clock, treeKey: never, budgetMs: 10 });
  assert.deepEqual(fallback, { key: await commitKey(sha), kind: 'commit', at: clock });
  // The attempt is saved before the computation starts.
  const other = 'e'.repeat(40);
  treeKeyFor({ ledger, sha: other, now: clock, treeKey: never, budgetMs: 300 });
  await until(async () => (await ledger.get('treekey:' + other))?.attempts === 1, 'the attempt');
  assert.equal((await ledger.get('treekey:' + other)).attempts, 1);
});

async function queued(db) {
  const ledger = new Ledger(new MemoryStorage());
  await ledger.put('record:5-1', {
    key: '5-1',
    sha,
    profile: 'check',
    die: 'AD104',
    id: 900,
    started: clock,
    ended: null,
    price: 0.2,
  });
  await ledger.put('store:5-1:0', { attempts: 0, inflight: false });
  await ledger.put('treekey:' + sha, { key: 'f'.repeat(64), kind: 'tree', at: clock });
  return { ledger, env: { DB: db, ROW_TAG_KEY: TAG_KEY } };
}

test('ingest: the store tick counts its attempt before any D1 call', async () => {
  const { ledger, env } = await queued(hungDb());
  storeTick({ ledger, env, now: clock });
  await until(async () => (await ledger.get('store:5-1:0')).attempts === 1, 'the counted attempt');
  assert.equal((await ledger.get('store:5-1:0')).attempts, 1);
});

test('ingest: the store tick stops at its time budget', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  const { ledger, env } = await queued(db);
  let t = 0;
  await storeTick({ ledger, env, now: clock, clock: () => (t += 1000), budgetMs: 500 });
  assert.equal(db.rows('SELECT * FROM ci_runs').length, 0);
  assert.equal((await ledger.get('store:5-1:0')).attempts, 0);
  await storeTick({ ledger, env, now: clock });
  assert.equal(db.rows('SELECT * FROM ci_runs').length, 1);
});

test('ingest: a hung store never blocks cleanup expiry', async () => {
  const h = await setup({ DB: hungDb(), ROW_TAG_KEY: TAG_KEY });
  const old = clock - 7 * 3600000; // past LOG_RETENTION_HOURS (6)
  await h.storage.put('record:999-1', {
    key: '999-1',
    sha,
    profile: 'check',
    die: 'AD104',
    id: 5,
    started: old - 60000,
    ended: old,
    price: 0.2,
    verdict: 'PASS',
  });
  await h.storage.put('ended:999-1', old);
  await h.storage.put('store:999-1:0', { attempts: 0, inflight: true });
  await h.storage.put('treekey:' + sha, { key: 'f'.repeat(64), kind: 'tree', at: clock });
  h.service.sweep();
  await until(async () => (await h.storage.get('ended:999-1')) === undefined, 'the expiry');
  assert.equal(await h.storage.get('ended:999-1'), undefined, 'expired before the store work');
});

test(
  'ingest: the check and tree keys run outside the serial queue',
  { skip: d1Skip, timeout: 10000 },
  async () => {
    const { h } = await harness();
    const inner = h.cloud.fetch;
    let hang = false,
      hung = 0;
    h.cloud.fetch = (input, init) =>
      hang && new URL(String(input)).hostname === 'console.vast.ai'
        ? (hung++, never())
        : inner(input, init);
    h.restart();
    assert.equal((await h.call()).status, 200);
    hang = true;
    h.service.sweep(); // holds the serial queue on a provider call that never returns
    await until(() => hung > 0, 'the sweep to reach its hung provider call');
    const answer = await check(h);
    assert.equal(answer.status, 200);
    await h.service.storeTrees();
    assert.equal((await h.storage.get('treekey:' + sha)).kind, 'tree');
  },
);

test('check: its own admission counter never starves /availability', { skip: d1Skip }, async () => {
  const { h } = await harness();
  // /availability first, so the shared window is open when the checks run.
  assert.equal((await h.call('/availability?profile=check', { method: 'GET' })).status, 200);
  let limited = 0;
  for (let i = 0; i < 31; i++) if ((await check(h)).status === 429) limited++;
  assert.equal(limited, 1);
  assert.equal((await h.call('/availability?profile=check', { method: 'GET' })).status, 200);
});

test('ingest: a dropped queue entry closes its in-flight row', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  const { ledger, env } = await queued(db);
  await storeTick({ ledger, env, now: clock });
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: null }]);
  await ledger.storage.delete('record:5-1'); // e.g. expired
  await storeTick({ ledger, env, now: clock + 60000 });
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'UNATTRIBUTED' }]);
});

// --- integration of #258 with #259 (review rev259 F7, 2026-11-05) ---
// FALSIFIER: a release leg (profile check + budgetClass release, tier large) is stored as middle or
// adhoc, or under profile check; a record's own tier is ignored; a PASS from a box that reported
// another die (or no known GPU/driver) is stored as PASS; a /rent leg is recorded twice.
const f7Record = (extra = {}) => ({
  key: '8000000000000000001-1',
  sha,
  profile: 'check',
  die: 'AD104',
  family: 'ada',
  id: 900,
  started: clock,
  ended: clock + 600000,
  resultAt: clock + 500000,
  price: 0.2,
  verdict: 'PASS',
  result: { verdict: 'PASS', suites: passSuites() },
  hardware: { gpu_name: 'NVIDIA GeForce RTX 4070', compute_cap: 890, driver_version: '580.159.04' },
  ...extra,
});
const f7Tree = { key: 'f'.repeat(64), kind: 'tree' };
const f7Row = (extra, cfg = configuration(baseEnv)) =>
  rowFromRecord(f7Record(extra), 0, f7Tree, clock + 700000, cfg);

test('F7: a release leg is stored as tier largest, profile release, platform both', () => {
  const row = f7Row({ budgetClass: 'release', tier: 'large' });
  assert.deepEqual(
    [row.tier, row.profile, row.platform, row.verdict, row.run_id],
    ['largest', 'release', 'both', 'PASS', 'vast-8000000000000000001-1-0'],
  );
  // Without the tier field the budget class still says release.
  assert.equal(f7Row({ budgetClass: 'release' }).tier, 'largest');
  assert.equal(storeTierOf('release'), 'largest');
});

test('F7: the record tier wins over the profile; old records keep the old mapping', () => {
  assert.equal(f7Row({ tier: 'small' }).tier, 'small');
  assert.equal(f7Row({ tier: 'middle' }).tier, 'middle');
  assert.equal(f7Row({ tier: 'adhoc' }).tier, 'adhoc');
  assert.equal(f7Row({}).tier, 'middle', 'no tier field: profile check -> middle');
  assert.equal(f7Row({ tier: 'bogus' }).tier, 'middle', 'unknown tier: the profile decides');
  const sweep = f7Row({ profile: 'sweep' });
  assert.deepEqual([sweep.tier, sweep.profile, sweep.platform], ['adhoc', 'sweep', 'linux']);
  assert.equal(storeTierOfRecord({ profile: 'sweep', tier: 'large' }), 'largest');
});

test('F7: a PASS on hardware that is not the record die is UNATTRIBUTED', () => {
  const gpu = (gpu_name, compute_cap = 890, driver_version = '580.159.04') => ({
    hardware: { gpu_name, compute_cap, driver_version },
  });
  assert.equal(f7Row({ tier: 'large', ...gpu('NVIDIA GeForce RTX 4090') }).verdict, 'UNATTRIBUTED');
  assert.equal(f7Row(gpu('NVIDIA GeForce RTX 3070', 860)).verdict, 'UNATTRIBUTED');
  assert.equal(f7Row(gpu('unknown', 0, 'unknown')).verdict, 'UNATTRIBUTED');
  assert.equal(f7Row(gpu('NVIDIA GeForce RTX 4070', 890, 'unknown')).verdict, 'UNATTRIBUTED');
  assert.equal(f7Row({ hardware: undefined }).verdict, 'UNATTRIBUTED', 'no hardware report');
  assert.equal(f7Row({ family: 'ampere' }).verdict, 'UNATTRIBUTED', 'family of another die');
  assert.equal(f7Row({}, null).verdict, 'UNATTRIBUTED', 'no configuration: fail closed');
  // A record written before the family field: the die's family is derived, the die still checked.
  assert.equal(f7Row({ family: undefined }).verdict, 'PASS');
  assert.equal(
    f7Row({
      hardware: undefined,
      result: { verdict: 'PASS', suites: passSuites(), hardware: gpu('RTX_4070').hardware },
    }).verdict,
    'PASS',
  );
  // Only a PASS is demoted; a FAIL stays a FAIL.
  const fail = f7Row({
    verdict: 'FAIL',
    result: { verdict: 'FAIL', suites: [{ name: 'gates', pass: 0, fail: 1, skip: 0 }] },
    ...gpu('NVIDIA GeForce RTX 4090'),
  });
  assert.equal(fail.verdict, 'FAIL');
});

test('F7: only the ingest records runs (no second recordRun path for /rent legs)', () => {
  const src = new URL('../src/', import.meta.url);
  const callers = readdirSync(src)
    .filter((f) => f.endsWith('.js') && f !== 'results.js')
    .filter((f) => /\brecordRun\(/.test(readFileSync(new URL(f, src), 'utf8')));
  assert.deepEqual(callers, ['ingest.js']);
  const ingest = readFileSync(new URL('ingest.js', src), 'utf8');
  assert.equal(ingest.match(/\brecordRun\(/g).length, 1);
});

// --- setup picker claims (2026-11-06) ---
// FALSIFIER: the provider's offer label becomes the host driver; a claim rescues a PASS that the
// hardware check demoted; a box's host_claims are lost on the way to D1 or reach it unchecked.
const claimsBlock = {
  cpu_vendor: 'intel',
  cpu_model: 'Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz',
  cpu_arch: 'x86_64',
  linux_os_version: 'ubuntu-24.04',
  linux_driver: '580.159.04',
  windows_os_version: '11-ltsc-2024-26100',
  windows_driver: '580.88',
};

test('claims: the host driver is the box nvidia-smi driver, never the offer label', () => {
  const row = f7Row({
    hostDriverVersion: '535.54.03',
    hardware: {
      gpu_name: 'NVIDIA GeForce RTX 4070',
      compute_cap: 890,
      driver_version: '580.95.05',
    },
    result: { verdict: 'PASS', suites: passSuites(), host_claims: claimsBlock },
  });
  assert.equal(row.claims.host_driver, '580.95.05');
  assert.equal(row.claims.cpu_vendor, 'intel');
  assert.equal(row.claims.windows_driver, '580.88');
  assert.deepEqual(
    row.driver_versions,
    { host: '580.95.05' },
    'the offer label is not stored; the host driver is the box driver',
  );
  // No usable box driver: not reported, the offer label is not a substitute.
  const unknown = f7Row({
    hostDriverVersion: '535.54.03',
    hardware: { gpu_name: 'NVIDIA GeForce RTX 4070', compute_cap: 890, driver_version: 'unknown' },
  });
  assert.equal(unknown.claims.host_driver, null);
  // An old box: no claims block, the host driver alone.
  assert.equal(f7Row({}).claims.cpu_vendor, null);
  assert.equal(f7Row({}).claims.host_driver, '580.159.04');
});

test('claims: a claim never upgrades or changes a verdict', () => {
  const wrong = {
    gpu_name: 'NVIDIA GeForce RTX 4090',
    compute_cap: 890,
    driver_version: '580.159.04',
  };
  const claims = { ...claimsBlock, host_driver: '580.159.04' };
  assert.equal(
    f7Row({
      hardware: wrong,
      result: { verdict: 'PASS', suites: passSuites(), host_claims: claims },
    }).verdict,
    'UNATTRIBUTED',
  );
  assert.equal(
    f7Row({ result: { verdict: 'PASS', suites: passSuites(), host_claims: claims } }).verdict,
    'PASS',
  );
  // A hostile claims block neither throws nor changes the verdict.
  const hostile = { cpu_vendor: {}, cpu_model: 5, host_driver: [], linux_driver: null };
  assert.equal(
    f7Row({ result: { verdict: 'PASS', suites: passSuites(), host_claims: hostile } }).verdict,
    'PASS',
  );
});

test('claims: a box result with host_claims reaches ci_run_claims', { skip: d1Skip }, async () => {
  const { h, db } = await harness();
  assert.equal((await h.call()).status, 200);
  await cron(h, 2);
  await h.report({ seq: 1, kind: 'progress', text: 'started' });
  const r = await h.report({
    seq: 2,
    kind: 'result',
    result: {
      nonce,
      verdict: 'PASS',
      suites: passSuites(),
      hardware,
      host_claims: { ...claimsBlock, machine_id: '99999', cpu_arch: 'riscv64' },
    },
  });
  assert.equal(r.status, 200);
  h.time.value += 6 * 60000;
  await cron(h);
  if (storeKeys(h).length) await cron(h);
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'PASS' }]);
  assert.deepEqual(
    db.rows(
      'SELECT guest_os, guest_driver, host_driver, cpu_vendor, cpu_arch, claimed FROM ci_run_claims ORDER BY guest_os',
    ),
    [
      {
        guest_os: 'linux',
        guest_driver: '580.159.04',
        host_driver: '580.159.04',
        cpu_vendor: 'intel',
        cpu_arch: null,
        claimed: 1,
      },
      {
        guest_os: 'windows',
        guest_driver: '580.88',
        host_driver: '580.159.04',
        cpu_vendor: 'intel',
        cpu_arch: null,
        claimed: 1,
      },
    ],
  );
  assert.ok(!JSON.stringify(db.rows('SELECT * FROM ci_run_claims')).includes('99999'));
});

test(
  'claims: a box result without host_claims still records, with the host driver only',
  { skip: d1Skip },
  async () => {
    const { h, db } = await harness();
    await h.call();
    await cron(h, 2);
    await h.report({ seq: 1, kind: 'progress', text: 'started' });
    await h.report({
      seq: 2,
      kind: 'result',
      result: { nonce, verdict: 'PASS', suites: passSuites(), hardware },
    });
    h.time.value += 6 * 60000;
    await cron(h);
    if (storeKeys(h).length) await cron(h);
    assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'PASS' }]);
    assert.deepEqual(
      db.rows(
        'SELECT guest_os, guest_driver, host_driver, cpu_vendor FROM ci_run_claims ORDER BY guest_os',
      ),
      [
        { guest_os: 'linux', guest_driver: null, host_driver: '580.159.04', cpu_vendor: null },
        { guest_os: 'windows', guest_driver: null, host_driver: '580.159.04', cpu_vendor: null },
      ],
    );
  },
);
