import test from 'node:test';
import assert from 'node:assert/strict';
import {
  recordRun,
  requirementsMet,
  normaliseRow,
  runTag,
  verifyRunTag,
  canonicalRun,
  recordRequirementSnapshot,
  TokenRedactor,
} from '../src/results.js';
import { commitKey } from '../src/treekey.js';
import { FakeD1, d1Skip } from './d1.js';
// The result store (recordRun / requirementsMet), against the real migrations on node:sqlite.
// FALSIFIER: a second recordRun with the same run_id changes a final verdict or duplicates rows;
// a RETRY/UNATTRIBUTED or untagged/forged row satisfies a requirement; a FAIL is not cached; an
// in-flight run is not offered for joining; a token-shaped or oversized row is stored; a run
// token split across log chunks survives redaction.

const KEY = 'k'.repeat(40);
const TREE = 'a'.repeat(64);
const SHA = 'c'.repeat(40);
const T0 = Date.UTC(2026, 10, 5, 10, 0, 0);
const env = (db) => ({ DB: db, ROW_TAG_KEY: KEY });
const row = (patch = {}) => ({
  run_id: 'vast-100-1-0',
  commit_sha: SHA,
  tree_hash: TREE,
  tier: 'middle',
  profile: 'check',
  die: 'GA102',
  family: 'ampere',
  platform: 'both',
  verdict: 'PASS',
  stages: [
    { name: 'gates', rc: 0, duration_ms: 1000, result: 'PASS' },
    { name: 'windows_gate', rc: 0, duration_ms: 2000, result: 'PASS' },
  ],
  gpu_name: 'RTX_3090',
  driver_versions: { host: '580.95.05', guest: '580.159.04' },
  machine_id: 12345,
  price_usd_h: 0.4,
  cost_usd: 1.25,
  started_at: T0,
  ended_at: T0 + 3600000,
  log_key: null,
  ...patch,
});
const REQ = `version = 1
[[require]]
tier = "middle"
family = "any"
dies = ["any"]
platforms = ["linux", "windows"]
stages = ["gates", "windows_gate"]
`;
const check = (db, patch = {}) =>
  requirementsMet(env(db), {
    commit_sha: SHA,
    tree_hash: TREE,
    tier: 'middle',
    requirements: REQ,
    now: T0 + 4 * 3600000,
    ...patch,
  });

test(
  'store: insert, duplicate and conflict are idempotent on run_id',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    assert.equal((await recordRun(env(db), row())).state, 'INSERTED');
    assert.equal((await recordRun(env(db), row())).state, 'DUPLICATE');
    const conflict = await recordRun(env(db), row({ verdict: 'FAIL', stages: [] }));
    assert.equal(conflict.state, 'CONFLICT');
    assert.equal(conflict.stored, false);
    const runs = db.rows('SELECT verdict, tag FROM ci_runs');
    assert.equal(runs.length, 1);
    assert.equal(runs[0].verdict, 'PASS', 'a final verdict never changes');
    assert.equal(db.rows('SELECT * FROM ci_run_stages').length, 2);
    assert.equal(db.rows('SELECT * FROM ci_host_facts').length, 1);
    assert.deepEqual(db.rows('SELECT machine_id, price_usd_h, cost_usd FROM ci_runs_private'), [
      { machine_id: 12345, price_usd_h: 0.4, cost_usd: 1.25 },
    ]);
    // Same run id, other identity: conflict, nothing written.
    assert.equal(
      (await recordRun(env(db), row({ die: 'AD104', family: 'ada' }))).state,
      'CONFLICT',
    );
  },
);

test('store: an in-flight row is finalised once', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  const inflight = row({ verdict: null, ended_at: null, stages: [] });
  assert.equal((await recordRun(env(db), inflight)).state, 'INSERTED');
  assert.equal((await recordRun(env(db), inflight)).state, 'DUPLICATE');
  assert.deepEqual(db.rows('SELECT verdict, tag, sort_at FROM ci_runs'), [
    { verdict: null, tag: null, sort_at: '2026-11-05T10:00:00Z' },
  ]);
  assert.equal((await recordRun(env(db), row({ verdict: 'FAIL', stages: [] }))).state, 'FINALIZED');
  assert.equal((await recordRun(env(db), row())).state, 'CONFLICT');
  assert.deepEqual(db.rows('SELECT verdict, ended_at, sort_at FROM ci_runs'), [
    { verdict: 'FAIL', ended_at: '2026-11-05T11:00:00Z', sort_at: '2026-11-05T11:00:00Z' },
  ]);
  // Finalising with another identity is refused.
  const db2 = new FakeD1();
  await recordRun(env(db2), inflight);
  assert.equal((await recordRun(env(db2), row({ platform: 'linux' }))).state, 'CONFLICT');
  assert.equal(db2.rows('SELECT verdict FROM ci_runs')[0].verdict, null);
});

test('store: a batch is atomic', { skip: d1Skip }, async () => {
  const db = new FakeD1({ failOn: /INSERT INTO ci_run_stages/ });
  await assert.rejects(() => recordRun(env(db), row()));
  assert.equal(db.rows('SELECT * FROM ci_runs').length, 0);
});

test('store: without DB or tag key nothing is written', async () => {
  assert.equal((await recordRun({}, row())).state, 'NO_DB');
  const fake = {
    prepare() {
      throw Error('no call expected');
    },
  };
  assert.equal((await recordRun({ DB: fake, ROW_TAG_KEY: 'short' }, row())).state, 'NO_TAG_KEY');
});

test('store: rows are validated and bounded', async () => {
  const refuse = async (patch, field) =>
    assert.rejects(() => normaliseRow(row(patch)), new RegExp('BAD_ROW:' + field));
  await refuse({ run_id: 'X' }, 'run_id');
  await refuse({ commit_sha: 'z' }, 'commit_sha');
  await refuse({ tree_hash: 'q' }, 'tree_hash');
  await refuse({ tier: 'huge' }, 'tier');
  await refuse({ die: 'gpu0' }, 'die');
  await refuse({ family: 'ada' }, 'family'); // GA102 is Ampere
  await refuse({ platform: 'mac' }, 'platform');
  await refuse({ verdict: 'PROVIDER_FAULT' }, 'verdict');
  await refuse({ verdict: 'PASS', stages: [{ name: 'x', result: 'FAIL' }] }, 'verdict.stages');
  await refuse({ stages: [{ name: 'Bad Name', result: 'PASS' }] }, 'stage.name');
  await refuse(
    {
      stages: [
        { name: 'a', result: 'PASS' },
        { name: 'a', result: 'PASS' },
      ],
    },
    'stage.name',
  );
  await refuse(
    { stages: Array.from({ length: 65 }, (_, i) => ({ name: 's' + i, result: 'PASS' })) },
    'stages',
  );
  await refuse({ stages: [{ name: 'a', rc: 1e9, result: 'PASS' }] }, 'stage.rc');
  await refuse({ gpu_name: 'RTX\n3090' }, 'gpu_name');
  await refuse({ driver_versions: { host: 'rm -rf /' } }, 'driver_versions');
  await refuse({ machine_id: -1 }, 'machine_id');
  await refuse({ cost_usd: Infinity }, 'cost_usd');
  await refuse({ ended_at: null }, 'ended_at'); // final without an end
  await refuse({ verdict: null }, 'ended_at'); // in flight with an end
  await refuse({ ended_at: T0 - 1 }, 'ended_at');
  await refuse({ started_at: 'yesterday' }, 'started_at');
  await refuse({ log_key: 'runs/other-run-0/box.log' }, 'log_key');
  await refuse({ log_key: 'runs/vast-100-1-0/../../x.log' }, 'log_key');
  await refuse({ source: 'reported' }, 'source');
  await refuse({ gpu_name: 'eyJhbGciOiJIUzI1NiJ9' }, 'token-shaped');
  await refuse({ profile: 'x'.repeat(20000) }, 'size');
  // Accepted forms.
  const ok = await normaliseRow(
    row({ tier: 'large', family: undefined, log_key: 'runs/vast-100-1-0/box.log' }),
  );
  assert.equal(ok.tier, 'largest');
  assert.equal(ok.family, 'ampere');
  const iso = await normaliseRow(row({ started_at: '2026-11-05T10:00:00.123Z' }));
  assert.equal(iso.started_at, '2026-11-05T10:00:00Z');
});

test(
  'store: a null tree hash stores the per-commit key (no fall-through)',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    const r = await recordRun(env(db), row({ tree_hash: null }));
    assert.equal(r.tree_kind, 'commit');
    assert.equal(r.tree_hash, await commitKey(SHA));
    await assert.rejects(() => normaliseRow(row({ tree_kind: 'commit' })), /tree_hash/);
    const met = await check(db, { tree_hash: null });
    assert.equal(met.met, true, 'the same commit is answered by its own key');
    assert.equal((await check(db, { tree_hash: null, commit_sha: 'd'.repeat(40) })).met, false);
  },
);

test('store: the HMAC tag covers the row and its stages', async () => {
  const r = await normaliseRow(row());
  const tag = await runTag(KEY, r, r.stages);
  assert.match(tag, /^[A-Za-z0-9_-]{43}$/);
  assert.ok(await verifyRunTag(KEY, r, r.stages, tag));
  assert.ok(!(await verifyRunTag('x'.repeat(40), r, r.stages, tag)), 'other key');
  assert.ok(!(await verifyRunTag(KEY, { ...r, verdict: 'FAIL' }, r.stages, tag)), 'verdict');
  assert.ok(!(await verifyRunTag(KEY, { ...r, tree_hash: 'b'.repeat(64) }, r.stages, tag)));
  const stages = r.stages.map((s) => (s.name === 'windows_gate' ? { ...s, result: 'SKIP' } : s));
  assert.ok(!(await verifyRunTag(KEY, r, stages, tag)), 'stages');
  assert.ok(!(await verifyRunTag(KEY, r, r.stages, null)));
  assert.match(await canonicalRun(r, r.stages), /^kf-ci-run-v1\nvast-100-1-0\n/);
});

test(
  'requirements: PASS meets, FAIL is cached, RETRY/UNATTRIBUTED never pass',
  { skip: d1Skip },
  async () => {
    for (const verdict of ['RETRY', 'UNATTRIBUTED']) {
      const db = new FakeD1();
      await recordRun(env(db), row({ verdict, stages: [] }));
      const r = await check(db);
      assert.equal(r.met, false, verdict);
      assert.equal(r.missing[0].reason, 'NO_RESULT');
    }
    const db = new FakeD1();
    await recordRun(env(db), row({ verdict: 'FAIL', stages: [{ name: 'gates', result: 'FAIL' }] }));
    const failed = await check(db);
    assert.equal(failed.met, false);
    assert.equal(failed.missing[0].reason, 'FAIL', 'a FAIL is an answer: no automatic retry');
    // A newer PASS (an owner re-run) answers instead.
    await recordRun(
      env(db),
      row({ run_id: 'vast-101-1-0', started_at: T0 + 3600000, ended_at: T0 + 7200000 }),
    );
    assert.deepEqual(await check(db), { met: true, missing: [], inflight: [] });
  },
);

test('requirements: an untagged or forged row never counts', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  await recordRun(env(db), row());
  db.database.exec('UPDATE ci_runs SET tag = NULL');
  assert.equal((await check(db)).met, false);
  const db2 = new FakeD1();
  await recordRun(env(db2), row({ verdict: 'FAIL', stages: [] }));
  db2.database.exec("UPDATE ci_runs SET verdict = 'PASS'"); // forged in the database
  assert.equal((await check(db2)).met, false);
  const db3 = new FakeD1();
  await recordRun(env(db3), row());
  db3.database.exec("UPDATE ci_run_stages SET result = 'PASS' WHERE name = 'gates'");
  db3.database.exec("UPDATE ci_run_stages SET name = 'other' WHERE name = 'windows_gate'");
  assert.equal((await check(db3)).met, false, 'stage rows are covered by the tag');
  // A different key does not verify.
  const db4 = new FakeD1();
  await recordRun(env(db4), row());
  const r = await requirementsMet(
    { DB: db4, ROW_TAG_KEY: 'z'.repeat(40) },
    {
      commit_sha: SHA,
      tree_hash: TREE,
      tier: 'middle',
      requirements: REQ,
      now: T0 + 4 * 3600000,
    },
  );
  assert.equal(r.met, false);
});

test(
  'requirements: in-flight runs of the same tree are offered for joining',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    await recordRun(env(db), row({ verdict: null, ended_at: null, stages: [] }));
    const r = await check(db, { now: T0 + 60000 });
    assert.equal(r.met, false);
    assert.deepEqual(r.inflight, ['vast-100-1-0']);
    assert.equal(r.missing[0].reason, 'INFLIGHT');
    // Another tree does not see it; a stale in-flight row is not offered.
    assert.deepEqual(
      (await check(db, { tree_hash: 'b'.repeat(64), now: T0 + 60000 })).inflight,
      [],
    );
    assert.deepEqual((await check(db, { now: T0 + 9 * 3600000 })).inflight, []);
  },
);

test('requirements: tier rank, stages, platforms and missing files', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  await recordRun(env(db), row({ tier: 'small', run_id: 'small-run-1' }));
  assert.equal((await check(db)).met, false, 'a small-tier run does not meet middle');
  await recordRun(env(db), row({ platform: 'linux', run_id: 'linux-run-1' }));
  const r = await check(db);
  assert.equal(r.met, false, 'linux only: Windows not covered');
  await recordRun(
    env(db),
    row({ run_id: 'large-run-1', tier: 'large', stages: [{ name: 'gates', result: 'PASS' }] }),
  );
  assert.equal((await check(db)).missing[0].reason, 'STAGE', 'windows_gate stage missing');
  await recordRun(env(db), row({ run_id: 'large-run-2', tier: 'large', ended_at: T0 + 3700000 }));
  assert.equal((await check(db)).met, true, 'a larger tier satisfies a smaller one');
  for (const bad of ['', 'version = 2', 'version = 1\n[[require]]\ntier = "middle"\n', 'garbage'])
    assert.equal((await check(db, { requirements: bad })).met, false, JSON.stringify(bad));
  const none = await check(db, { requirements: 'version = 1\n' });
  assert.equal(none.missing[0].reason, 'NO_REQUIREMENTS');
  assert.equal(none.met, false);
  await assert.rejects(() => check(db, { tier: 'adhoc' }), /BAD_CHECK:tier/);
});

test('store: requirement snapshots are content-addressed', { skip: d1Skip }, async () => {
  const db = new FakeD1();
  const a = await recordRequirementSnapshot(env(db), { base_ref: 'master', content: REQ, now: T0 });
  const b = await recordRequirementSnapshot(env(db), {
    base_ref: 'master',
    content: REQ,
    now: T0 + 1,
  });
  assert.equal(a, b);
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(db.rows('SELECT * FROM ci_requirement_snapshots').length, 1);
  assert.equal(await recordRequirementSnapshot(env(db), { base_ref: 'pr', content: REQ }), null);
});

test('redaction: a run token split across chunks is still removed', () => {
  const token = 'eyJhbGciOiJIUzI1NiJ9.eyJydW4iOiIxIn0.c2lnbmF0dXJlLXNpZ25hdHVyZQ';
  const text = 'start ' + token + ' middle ' + token + ' end\n';
  for (let cut = 1; cut < text.length; cut += 3) {
    const r = new TokenRedactor([token]);
    const out = r.push(text.slice(0, cut)) + r.push(text.slice(cut)) + r.end();
    assert.equal(out, 'start [redacted] middle [redacted] end\n', 'cut ' + cut);
  }
  const bytewise = new TokenRedactor([token]);
  let out = '';
  for (const c of text) out += bytewise.push(c);
  out += bytewise.end();
  assert.ok(!out.includes(token.slice(0, 20)));
  assert.equal(new TokenRedactor([]).push('abc'), 'abc');
});
