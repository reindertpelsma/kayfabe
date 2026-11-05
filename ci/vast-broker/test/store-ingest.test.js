import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce, sha, jwt, clock } from './helpers.js';
import { FakeD1, d1Skip } from './d1.js';
import { treeKeyOf, patternsFor, commitKey } from '../src/treekey.js';
import { storeVerdict, storeTierOf, rowFromRecord } from '../src/ingest.js';
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
const storeKeys = (h) => [...h.storage.data.keys()].filter((k) => k.startsWith('store:'));
const check = (h, body = { commit_sha: sha, base: 'master', tier: 'middle' }, opts = {}) =>
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
    await h.service.sweep();
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
    await h.service.sweep();
    const record = await h.storage.get('record:123-1');
    assert.equal(record.verdict, 'PASS', 'the ledger verdict is untouched');
    assert.notEqual(record.ended, null);
    if (storeKeys(h).length) await h.service.sweep();
    assert.deepEqual(storeKeys(h), []);
    assert.deepEqual(db.rows('SELECT verdict, tier FROM ci_runs'), [
      { verdict: 'PASS', tier: 'middle' },
    ]);
    assert.deepEqual(
      db.rows('SELECT name, result FROM ci_run_stages ORDER BY ord').map((s) => s.name),
      ['provision', 'gates', 'raw_suite'],
    );
    assert.deepEqual(db.rows('SELECT gpu_name, driver_versions_json FROM ci_host_facts'), [
      { gpu_name: 'RTX_4070', driver_versions_json: '{"guest":"580.159.04","host":"580.95.05"}' },
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
  await h.service.sweep();
  await h.service.sweep();
  assert.deepEqual(db.rows('SELECT verdict FROM ci_runs'), [{ verdict: 'FAIL' }]);
  const answer = await (await check(h)).json();
  assert.equal(answer.met, false);
  assert.equal(answer.missing[0].reason, 'FAIL');
});

test('ingest: a box that never reports is UNATTRIBUTED, never PASS', { skip: d1Skip }, async () => {
  const { h, db } = await harness();
  await h.call();
  h.time.value += 11 * 60000; // past BOOT_LIMIT_MINUTES=10
  await h.service.sweep();
  await h.service.sweep();
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
    for (let i = 0; i < 3; i++) await h.service.sweep();
    assert.deepEqual(db.rows('SELECT tree_hash, tree_kind FROM ci_runs'), [
      { tree_hash: await commitKey(sha), tree_kind: 'commit' },
    ]);
    const answer = await (await check(h)).json();
    assert.equal(answer.tree_kind, 'commit');
  },
);

test('check: OIDC, body and store are required; it never rents', { skip: d1Skip }, async () => {
  const { h } = await harness();
  const before = [...h.storage.data.keys()].sort();
  assert.equal((await check(h, undefined, { token: 'not-a-jwt' })).status, 401);
  assert.equal((await check(h, undefined, { protocol: null })).status, 409);
  for (const body of [
    { commit_sha: sha, base: 'feature', tier: 'middle' },
    { commit_sha: sha, base: 'master', tier: 'adhoc' },
    { commit_sha: 'x', base: 'master', tier: 'middle' },
    { commit_sha: sha, base: 'master', tier: 'middle', extra: 1 },
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
  assert.deepEqual([...h.storage.data.keys()].sort(), before, 'no ledger write');
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
    await h.service.sweep(); // in-flight row of leg 0
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
    await h.service.sweep();
    assert.deepEqual(db.rows('SELECT run_id, verdict FROM ci_runs ORDER BY run_id'), [
      { run_id: 'vast-123-1-0', verdict: 'RETRY' },
      { run_id: 'vast-123-1-1', verdict: null },
    ]);
    assert.deepEqual(storeKeys(h), ['store:123-1:1']);
  },
);
