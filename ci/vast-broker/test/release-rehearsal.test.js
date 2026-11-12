import test from 'node:test';
import assert from 'node:assert/strict';
import { RELEASE_MATRIX } from '../src/release-matrix.js';
import { classifyLeg, releaseVerdict, planRehearsal, dieLegs } from '../src/release.js';
import { rowFromRecord } from '../src/ingest.js';
import { configuration } from '../src/policy.js';
import { applyProfile, tierEnvironment, middleAppsRequest } from '../src/tier.js';
import { setup, sha, env } from './helpers.js';
import { FakeD1, d1Skip } from './d1.js';
// 2026-11-12: the named release REHEARSAL scope (start body {tier:'large', die, rehearsal:true}).
// FALSIFIERS (written before the code): a rehearsal clears or hides a matrix blocker; a rehearsal ends in the release
// verdict PASS or counts toward a release record; `die` for large works without `rehearsal:true`; a rehearsal rents
// with a payload that does not declare it; a real matrix leg (no rehearsal) is rented, or sent without the matrix env,
// because the rehearsal exists; a large leg does not carry the middle apps pair; the rehearsal leg carries driver env.
const PAYLOAD_REHEARSAL = 'PAYLOAD_RELEASE_REHEARSAL = 1\n';
const vars = {
  MAX_CONCURRENT_RENTALS: '2',
  ROLLING_SPEND_USD: '200',
  RELEASE_ENABLED: 'true',
  RELEASE_COST_CAP_USD: '12',
  RELEASE_RERUNS_PER_LEG: '0',
  MIDDLE_APPS_ENABLED: undefined, // the test setup turns apps off; production sends them unless the var is 'false'
};
const body = (extra = {}) => ({
  commit_sha: sha,
  tier: 'large',
  die: 'AD102',
  rehearsal: true,
  ...extra,
});
async function harness(extra = {}, { payload = PAYLOAD_REHEARSAL } = {}) {
  const h = await setup({ ...vars, ...extra });
  h.cloud.reporterText = 'BOX_PROTOCOL = "7"\n';
  h.cloud.payloadText = payload;
  h.cloud.offer = {
    ...h.cloud.offer,
    id: 11,
    gpu_name: 'RTX_4090',
    compute_cap: 890,
    machine_id: 200011,
    host_id: 3011,
    gpu_ram: 24576,
    disk_space: 400,
  };
  h.cloud.price = 0.4;
  h.cloud.credit = 500;
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 901 + i);
  return h;
}
const creates = (h) =>
  h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).map((c) => c.body);
const batchOf = (h, id) => h.storage.get('batch:' + id);
const LINUX = { provision: 1, gates: 9, thin_suite: 30, cuda_host: 4, cuda_fat: 4 };
const hardware = {
  gpu_name: 'NVIDIA GeForce RTX 4090',
  compute_cap: 890,
  driver_version: '580.159.04',
};
async function finish(h, key, verdict, spec) {
  const suites = Object.entries(spec).map(([name, v]) => ({
    name,
    pass: v > 0 ? v : 0,
    fail: v < 0 ? 1 : 0,
    skip: 0,
  }));
  const record = await h.storage.get('record:' + key);
  const r = await h.report(
    {
      seq: 1,
      kind: 'result',
      result: { nonce: record.nonce, verdict, suites, hardware, host_reference: 'GREEN' },
    },
    { run: key },
  );
  assert.equal(r.status, 200, key + ' ' + (await r.text()));
}
const PASSING = { ...LINUX, apps: 66, windows_gate: 1, windows_gate_2: 1, windows_apps: 10 };
const tick = async (h) => {
  h.time.value += 11 * 60000;
  await h.service.sweep();
};
const storeKeys = (h) => [...h.storage.data.keys()].filter((k) => k.startsWith('store:'));

test('the production matrix is still proposed with blockers (the rehearsal does not change data)', () => {
  assert.notEqual(RELEASE_MATRIX.status, 'executable');
  assert.ok(RELEASE_MATRIX.dies.every((d) => d.blockers.length > 0));
});
test('rehearsal plan: one die, scope stated, blockers kept, execution_ready false, admitted', async () => {
  const h = await harness();
  const { plan, renting } = await h.service.adminPlan(body());
  assert.equal(renting, false);
  assert.equal(h.cloud.creates, 0);
  assert.equal(plan.rehearsal, true);
  assert.equal(plan.scope, 'REHEARSAL');
  assert.equal(plan.driver_subrun_matrix, 'NOT_EXECUTED');
  assert.equal(plan.max_verdict, 'REHEARSAL_PASS');
  assert.equal(plan.counts_toward_release, false);
  assert.equal(plan.execution_ready, false);
  assert.equal(plan.matrix_execution_problem, 'MATRIX_NOT_EXECUTABLE');
  assert.deepEqual(
    plan.matrix_blockers.map((b) => b.die + ':' + b.reason).sort(),
    RELEASE_MATRIX.dies.flatMap((d) => d.blockers.map((b) => d.die + ':' + b)).sort(),
  );
  assert.deepEqual(plan.refusals, [], JSON.stringify(plan.refusals));
  assert.equal(plan.admitted, true);
  assert.equal(plan.legs, 1);
  const [leg] = plan.families[0].cells[0].legs;
  assert.equal(plan.families[0].cells[0].die, 'AD102');
  assert.equal(leg.rehearsal, true);
  for (const key of ['release_matrix', 'drivers', 'driver_pins', 'subruns', 'cross_subrun'])
    assert.ok(!(key in leg), key + ' must not be claimed by a rehearsal leg');
});
test('without RELEASE_ENABLED the rehearsal plan says RELEASE_DISABLED and start refuses', async () => {
  const h = await harness({ RELEASE_ENABLED: undefined });
  const { plan } = await h.service.adminPlan(body());
  assert.equal(plan.admitted, false);
  assert.ok(plan.refusals.includes('RELEASE_DISABLED'));
  await assert.rejects(
    () => h.service.adminStart(body()),
    (e) => e.state === 'RELEASE_DISABLED',
  );
  assert.equal(h.cloud.creates, 0);
});
test('cost gates still apply to a rehearsal (cap, slots)', async () => {
  const h = await harness({ RELEASE_COST_CAP_USD: '0.5' });
  const { plan } = await h.service.adminPlan(body());
  assert.ok(plan.refusals.includes('COST_CAP'), JSON.stringify(plan.refusals));
  const slots = await harness({ MAX_CONCURRENT_RENTALS: '1' });
  assert.ok((await slots.service.adminPlan(body())).plan.refusals.includes('NO_SLOTS'));
});
test('the daily, account, rolling and credit-reserve gates still refuse a rehearsal plan', async () => {
  const refusals = async (extra, credit) => {
    const h = await harness(extra);
    if (credit !== undefined) h.cloud.credit = credit;
    return (await h.service.adminPlan(body())).plan.refusals;
  };
  assert.deepEqual(await refusals({}), []);
  assert.ok((await refusals({ ACCOUNT_DAILY_USD: '1' })).includes('ACCOUNT_DAILY_USD'));
  assert.ok((await refusals({ ROLLING_SPEND_USD: '1' })).includes('ROLLING_USD'));
  assert.ok((await refusals({}, 9)).includes('CREDIT_RESERVE'));
  assert.ok(
    (await refusals({ RELEASE_DAILY_USD: '12', RELEASE_COST_CAP_USD: '1' })).includes('COST_CAP'),
  );
});
test('a die for large needs rehearsal:true; rehearsal needs large and a die', async () => {
  const h = await harness();
  for (const bad of [
    { commit_sha: sha, tier: 'large', die: 'AD102' },
    { commit_sha: sha, tier: 'large', rehearsal: true },
    { commit_sha: sha, tier: 'large', die: 'AD102', rehearsal: 'true' },
    { commit_sha: sha, tier: 'large', die: 'AD102', rehearsal: false },
    { commit_sha: sha, tier: 'middle', die: 'AD102', rehearsal: true },
    { commit_sha: sha, tier: 'small', rehearsal: true },
  ]) {
    await assert.rejects(
      () => h.service.adminPlan(bad),
      (e) => e.state === 'BAD_REQUEST',
      JSON.stringify(bad),
    );
    await assert.rejects(
      () => h.service.adminStart(bad),
      (e) => e.state === 'BAD_REQUEST',
      JSON.stringify(bad),
    );
  }
  // a die outside the release set is refused too
  await assert.rejects(
    () => h.service.adminStart(body({ die: 'GA106' })),
    (e) => e.state === 'BAD_REQUEST',
  );
  assert.equal(h.cloud.creates, 0);
});
test('a plain large Start is unchanged: still refused for the proposed matrix', async () => {
  const h = await harness();
  const plan = (await h.service.adminPlan({ commit_sha: sha, tier: 'large' })).plan;
  assert.ok(plan.refusals.includes('MATRIX_NOT_EXECUTABLE'));
  assert.equal(plan.rehearsal, undefined);
  await assert.rejects(
    () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
    (e) => e.reason === 'MATRIX_NOT_EXECUTABLE',
  );
  assert.equal(h.cloud.creates, 0);
});
test('a payload that does not declare the rehearsal is not rented', async () => {
  for (const payload of [
    '# old\n',
    'PAYLOAD_RELEASE_REHEARSAL = 0\n',
    'PAYLOAD_RELEASE_MATRIX_EXECUTOR = 1\n',
  ]) {
    const h = await harness({}, { payload });
    await assert.rejects(
      () => h.service.adminStart(body()),
      (e) => e.reason === 'REHEARSAL_PAYLOAD_CAPABILITY',
      payload,
    );
    assert.equal(h.cloud.creates, 0);
  }
});
test('start: one leg, rehearsal env, apps pair, no driver env, record flagged, never queued for the store', async () => {
  const h = await harness();
  const started = await h.service.adminStart(body());
  assert.equal(started.rehearsal, true);
  assert.equal(started.scope, 'REHEARSAL');
  assert.equal(started.driver_subrun_matrix, 'NOT_EXECUTED');
  assert.equal(started.legs.length, 1);
  assert.equal(started.legs[0].state, 'STARTED', JSON.stringify(await batchOf(h, started.batch)));
  const [create] = creates(h);
  assert.equal(create.env.KF_TIER, 'large');
  assert.equal(create.env.KF_RELEASE_REHEARSAL, '1');
  assert.equal(create.env.KF_LINUX_APPS, '1');
  assert.equal(create.env.KF_WINAPPS, 'mid');
  assert.equal(create.env.KF_WINDOWS_RUNS, '2');
  assert.equal(create.env.KF_MERGE_WINDOWS, '1');
  for (const k of Object.keys(create.env))
    assert.ok(
      !/^KF_(HOST_DRIVER|LINUX_GUEST_DRIVER|WINDOWS_GUEST_DRIVER|WINGATE_GUEST_DRIVER|RELEASE_SUBRUNS|RELEASE_DRIVER_PINS)$/.test(
        k,
      ),
      k,
    );
  const batch = await batchOf(h, started.batch);
  assert.equal(batch.rehearsal, true);
  const record = await h.storage.get('record:' + batch.legs[0].key);
  assert.equal(record.rehearsal, true);
  assert.equal(record.releaseMatrix, undefined);
  assert.equal(record.middleAppsRequested, true);
  assert.equal(storeKeys(h).length, 0);
});
test('a rehearsal leg that passes everything ends REHEARSAL_PASS with the matrix cell NOT_EXECUTED, never PASS', async () => {
  const h = await harness();
  const { batch: id } = await h.service.adminStart(body());
  await finish(h, (await batchOf(h, id)).legs[0].key, 'PASS', PASSING);
  await tick(h);
  const batch = await batchOf(h, id);
  assert.equal(batch.state, 'DONE');
  assert.equal(batch.verdict, 'REHEARSAL_PASS', JSON.stringify(batch.cells));
  assert.notEqual(batch.verdict, 'PASS');
  const matrix = batch.cells.find((c) => c.platform === 'matrix:driver_subruns');
  assert.equal(matrix.verdict, 'NOT_EXECUTED');
  assert.equal(h.cloud.creates, 1);
});
test('apps are mandatory in a large leg: a PASS without the 66 Linux apps is not a pass', async () => {
  const h = await harness();
  const { batch: id } = await h.service.adminStart(body());
  const { apps, ...without } = PASSING;
  await finish(h, (await batchOf(h, id)).legs[0].key, 'PASS', without);
  await tick(h);
  const batch = await batchOf(h, id);
  assert.notEqual(batch.verdict, 'REHEARSAL_PASS');
  assert.notEqual(batch.verdict, 'PASS');
});
test('a failed rehearsal leg is FAIL and is not queued for the store', async () => {
  const h = await harness();
  const { batch: id } = await h.service.adminStart(body());
  await finish(h, (await batchOf(h, id)).legs[0].key, 'FAIL', { ...LINUX, windows_gate: -1 });
  await tick(h);
  assert.equal((await batchOf(h, id)).verdict, 'FAIL');
  assert.equal(storeKeys(h).length, 0);
});
test('releaseVerdict: only batch.rehearsal===true can yield REHEARSAL_PASS; a release batch never does', () => {
  const ok = { linux: 'PASS', windows: ['PASS', 'PASS'], stages: {}, stageRows: {} };
  const leg = (extra = {}) => ({
    key: '1-1',
    family: 'ada',
    die: 'AD102',
    state: 'ENDED',
    outcome: { verdict: 'PASS', ...ok },
    ...extra,
  });
  const base = {
    tier: 'large',
    windowsRunsPerDie: 2,
    stages: [],
    releaseDies: { ada: ['AD102'] },
    legs: [leg()],
  };
  assert.equal(releaseVerdict({ ...base, rehearsal: true }).verdict, 'REHEARSAL_PASS');
  const real = releaseVerdict(base);
  assert.notEqual(real.verdict, 'REHEARSAL_PASS');
  assert.notEqual(real.verdict, 'PASS');
  for (const rehearsal of [false, 'true', 1, undefined])
    assert.notEqual(releaseVerdict({ ...base, rehearsal }).verdict, 'REHEARSAL_PASS');
  const view = releaseVerdict({ ...base, rehearsal: true });
  assert.ok(view.cells.length > 0);
  for (const c of view.cells) {
    assert.equal(c.scope, 'REHEARSAL');
    assert.equal(c.driver_subrun_matrix, 'NOT_EXECUTED');
    assert.equal(c.counts_toward_release, false);
    assert.ok(c.platform === 'matrix:driver_subruns' ? c.verdict === 'NOT_EXECUTED' : true);
  }
  assert.equal(view.complete, false);
});
test('classifyLeg: rehearsal drops matrix receipts only; a real release leg still needs them', () => {
  const cfg = applyProfile(configuration({ ...env, ...vars }), 'release', 'large');
  const rows = Object.entries(PASSING).map(([name, pass]) => ({ name, pass, fail: 0, skip: 0 }));
  const record = {
    sha,
    ended: 1,
    verdict: 'PASS',
    hardware,
    result: { verdict: 'PASS', suites: rows, host_reference: 'GREEN', hardware },
  };
  const opts = { windowsRuns: 2, die: 'AD102', family: 'ada', cfg };
  assert.equal(classifyLeg(record, { ...opts, rehearsal: true }).verdict, 'PASS');
  assert.notEqual(classifyLeg(record, opts).verdict, 'PASS');
  assert.notEqual(classifyLeg(record, { ...opts, rehearsal: false }).verdict, 'PASS');
});
test('a rehearsal record can never become a stored release PASS row (the large guard is what stops it)', () => {
  const cfg = applyProfile(configuration({ ...env, ...vars }), 'release', 'large');
  const rows = Object.entries(PASSING).map(([name, pass]) => ({ name, pass, fail: 0, skip: 0 }));
  const record = {
    key: '1-1',
    sha,
    die: 'AD102',
    family: 'ada',
    tier: 'middle',
    budgetClass: 'check',
    windowsRuns: 2,
    stages: [],
    ended: 1,
    resultAt: 1,
    verdict: 'PASS',
    hardware,
    result: { verdict: 'PASS', suites: rows, host_reference: 'GREEN', hardware },
  };
  const row = (r) => rowFromRecord(r, 1, { key: 'k', kind: 'tree' }, 0, cfg);
  // control: the same record as a middle row IS a stored PASS, so the check below can fail
  assert.equal(row(record).verdict, 'PASS');
  // as a large rehearsal record, or as a large record without a matrix row, it is not
  assert.equal(
    row({ ...record, tier: 'large', budgetClass: 'release', rehearsal: true }).verdict,
    'UNATTRIBUTED',
  );
  assert.equal(row({ ...record, tier: 'large' }).verdict, 'UNATTRIBUTED');
  assert.equal(row({ ...record, budgetClass: 'release' }).verdict, 'UNATTRIBUTED');
});
test('dieLegs: a rehearsal skips the host-driver filter and carries no matrix fields; a normal call is unchanged', () => {
  const cfg = applyProfile(configuration({ ...env, ...vars }), 'release', 'large');
  const offer = {
    id: 1,
    die: 'AD102',
    machine_id: 5,
    host_id: 6,
    country: 'US',
    gpu_name: 'RTX 4090',
    dph_total: 0.5,
    pricing: { total: 2, effective: 0.5 },
    host_capabilities: { can_swap_host_driver: false, host_driver: '999.99' },
  };
  assert.equal(
    dieLegs([offer], cfg).length,
    0,
    'normal: a host that cannot provide drivers is filtered',
  );
  const [leg] = dieLegs([offer], cfg, undefined, { rehearsal: true });
  assert.equal(leg.rehearsal, true);
  assert.ok(!('release_matrix' in leg) && !('drivers' in leg));
  assert.equal(planRehearsal([offer], cfg, { die: 'AD102' }).legs, 1);
  assert.ok(planRehearsal([offer], cfg, { die: 'NOPE' }).refusals.includes('DIE_NOT_IN_MATRIX'));
});
test('tierEnvironment: the marker is large/release only and never beside a matrix row; large carries the apps pair', () => {
  assert.deepEqual(middleAppsRequest('release', 'large', true), {
    linuxApps: true,
    winApps: 'mid',
  });
  assert.deepEqual(middleAppsRequest('release', 'large', false), {});
  // 2026-11-12 (release shape v2): a release MIDDLE leg asks for the apps pair too.
  assert.deepEqual(middleAppsRequest('release', 'middle', true), {
    linuxApps: true,
    winApps: 'mid',
  });
  assert.deepEqual(middleAppsRequest('release', 'small', true), {});
  assert.deepEqual(middleAppsRequest('check', 'large', true), {});
  const e = tierEnvironment('release', {
    tierName: 'large',
    rehearsal: true,
    linuxApps: true,
    winApps: 'mid',
  });
  assert.equal(e.KF_RELEASE_REHEARSAL, '1');
  assert.equal(e.KF_LINUX_APPS, '1');
  assert.throws(
    () =>
      tierEnvironment('release', {
        tierName: 'large',
        rehearsal: true,
        releaseMatrix: RELEASE_MATRIX.dies[0],
      }),
    /rehearsal/,
  );
  for (const [profile, tierName] of [
    ['check', 'middle'],
    ['sweep', 'adhoc'],
    ['release', 'middle'],
  ])
    assert.throws(() => tierEnvironment(profile, { tierName, rehearsal: true }), /rehearsal/);
  assert.ok(!('KF_RELEASE_REHEARSAL' in tierEnvironment('release', { tierName: 'large' })));
});

test(
  'with the result store enabled a rehearsal leg is still never queued',
  { skip: d1Skip },
  async () => {
    const h = await harness({ DB: new FakeD1(), ROW_TAG_KEY: 't'.repeat(48) });
    const { batch: id } = await h.service.adminStart(body());
    const batch = await batchOf(h, id);
    assert.equal(batch.legs[0].state, 'STARTED');
    assert.equal(storeKeys(h).length, 0);
    await finish(h, batch.legs[0].key, 'PASS', PASSING);
    await tick(h);
    assert.equal(storeKeys(h).length, 0);
  },
);
for (const rehearsal of [undefined, false])
  test(
    'a persisted non-rehearsal release batch without a matrix row never rents (' + rehearsal + ')',
    async () => {
      const h = await harness();
      const id = '8123456789012345678';
      await h.storage.put('batch:' + id, {
        id,
        sha,
        tier: 'large',
        profile: 'release',
        ...(rehearsal === undefined ? {} : { rehearsal }),
        state: 'RUNNING',
        created: h.time.value,
        updated: h.time.value,
        runnerNonce: 'f'.repeat(64),
        costCapUsd: 12,
        estimateUsd: 1,
        windowsRunsPerDie: 2,
        diesPerFamily: 1,
        releaseDies: { ada: ['AD102'] },
        stages: [],
        reruns: 0,
        legs: [
          {
            seq: 1,
            key: id + '-1',
            family: 'ada',
            die: 'AD102',
            windowsRuns: 2,
            state: 'PENDING',
            avoidMachines: [],
            avoidHosts: [],
            rerunDepth: 0,
            queued: h.time.value,
          },
        ],
        verdict: null,
      });
      await h.storage.put('batch-open:' + id, id);
      await h.service.sweep();
      assert.equal(h.cloud.creates, 0);
      assert.notEqual((await batchOf(h, id)).legs[0].state, 'STARTED');
    },
  );

// ---- review of PR #355 (2026-11-12): S1 apps only on the last SPEC lane leg, S2 APPS_DISABLED, S3 admission checks ----
const SPEC = { RELEASE_STAGES: '["specviewperf"]' };
const twoMachines = (h) => {
  h.cloud.extraOffers = [
    { ...h.cloud.offer, id: 12, machine_id: 200012, host_id: 3012, price: 0.4 },
  ];
};
const persisted = async (h, { leg = {}, batch = {} } = {}) => {
  const id = '8123456789012345678';
  await h.storage.put('batch:' + id, {
    id,
    sha,
    tier: 'large',
    profile: 'release',
    rehearsal: true,
    rehearsalDie: 'AD102',
    state: 'RUNNING',
    created: h.time.value,
    updated: h.time.value,
    runnerNonce: 'f'.repeat(64),
    costCapUsd: 12,
    estimateUsd: 1,
    windowsRunsPerDie: 2,
    diesPerFamily: 1,
    releaseDies: { ada: ['AD102'] },
    stages: [],
    reruns: 0,
    legs: [
      {
        seq: 1,
        key: id + '-1',
        family: 'ada',
        die: 'AD102',
        windowsRuns: 2,
        state: 'PENDING',
        avoidMachines: [],
        avoidHosts: [],
        rerunDepth: 0,
        queued: h.time.value,
        ...leg,
      },
    ],
    verdict: null,
    ...batch,
  });
  await h.storage.put('batch-open:' + id, id);
  return id;
};
test('S1: with SPEC only the LAST lane leg carries the apps pair (plan, env, record)', async () => {
  const h = await harness(SPEC);
  twoMachines(h);
  const { plan } = await h.service.adminPlan(body());
  const legs = plan.families[0].cells[0].legs;
  assert.deepEqual(
    legs.map((l) => [l.spec, l.apps]),
    [
      ['lanea', false],
      ['laneb', undefined],
    ],
  );
  const { batch: id } = await h.service.adminStart(body());
  const [a] = creates(h);
  assert.equal(a.env.KF_SPEC, 'lanea');
  assert.ok(!('KF_LINUX_APPS' in a.env) && !('KF_WINAPPS' in a.env));
  let batch = await batchOf(h, id);
  assert.equal((await h.storage.get('record:' + batch.legs[0].key)).middleAppsRequested, false);
  await finish(h, batch.legs[0].key, 'PASS', {
    ...LINUX,
    windows_gate: 1,
    specviewperf_lanea: 1,
  });
  await tick(h);
  batch = await batchOf(h, id);
  const b = creates(h).find((c) => c.env.KF_SPEC === 'laneb');
  assert.equal(b.env.KF_LINUX_APPS, '1');
  assert.equal(b.env.KF_WINAPPS, 'mid');
  assert.equal((await h.storage.get('record:' + batch.legs[1].key)).middleAppsRequested, true);
  // lane B without the apps rows is not a pass; with them the batch ends REHEARSAL_PASS
  await finish(h, batch.legs[1].key, 'PASS', {
    ...LINUX,
    apps: 66,
    windows_apps: 10,
    windows_gate: 1,
    specviewperf_laneb: 1,
  });
  await tick(h);
  assert.equal((await batchOf(h, id)).verdict, 'REHEARSAL_PASS');
});
test('S1: a SPEC lane B leg that skips the apps rows is not a pass', async () => {
  const h = await harness(SPEC);
  twoMachines(h);
  const { batch: id } = await h.service.adminStart(body());
  let batch = await batchOf(h, id);
  await finish(h, batch.legs[0].key, 'PASS', { ...LINUX, windows_gate: 1, specviewperf_lanea: 1 });
  await tick(h);
  batch = await batchOf(h, id);
  await finish(h, batch.legs[1].key, 'PASS', {
    ...LINUX,
    windows_gate: 1,
    specviewperf_laneb: 1,
  });
  await tick(h);
  assert.notEqual((await batchOf(h, id)).verdict, 'REHEARSAL_PASS');
});
test('S2: MIDDLE_APPS_ENABLED=false refuses a large plan, a rehearsal plan and a rehearsal start', async () => {
  const h = await harness({ MIDDLE_APPS_ENABLED: 'false' });
  const plain = (await h.service.adminPlan({ commit_sha: sha, tier: 'large' })).plan;
  assert.ok(plain.refusals.includes('APPS_DISABLED'), JSON.stringify(plain.refusals));
  const { plan } = await h.service.adminPlan(body());
  assert.ok(plan.refusals.includes('APPS_DISABLED'));
  assert.equal(plan.admitted, false);
  await assert.rejects(
    () => h.service.adminStart(body()),
    (e) => e.reason === 'PLAN_REFUSED',
  );
  assert.equal(h.cloud.creates, 0);
  // and the enabled case does not have the refusal
  const on = await harness();
  assert.ok(!(await on.service.adminPlan(body())).plan.refusals.includes('APPS_DISABLED'));
});
test('S2: a persisted pending rehearsal leg is not rented while the apps pair is off, and the batch ends', async () => {
  const h = await harness({ MIDDLE_APPS_ENABLED: 'false' });
  const id = await persisted(h);
  await h.service.sweep();
  assert.equal(h.cloud.creates, 0);
  const batch = await batchOf(h, id);
  assert.equal(batch.legs[0].state, 'MISSING');
  assert.equal(batch.legs[0].lastRefusal, 'APPS_DISABLED');
});
test('S3: a persisted rehearsal leg that carries a matrix row is refused at admission', async () => {
  const h = await harness();
  const row = structuredClone(RELEASE_MATRIX.dies.find((d) => d.die === 'AD102'));
  const id = await persisted(h, { leg: { releaseMatrix: row } });
  await h.service.sweep();
  assert.equal(h.cloud.creates, 0);
  assert.notEqual((await batchOf(h, id)).legs[0].state, 'STARTED');
});
test('S3: a persisted rehearsal leg whose die is not in the matrix is refused at admission', async () => {
  const h = await harness();
  // an offer exists for the die, so only the matrix membership check can stop the leg
  h.cloud.offer = { ...h.cloud.offer, gpu_name: 'RTX_3060', compute_cap: 860, gpu_ram: 12288 };
  const id = await persisted(h, { leg: { die: 'GA106', family: 'ampere' } });
  await h.service.sweep();
  assert.equal(h.cloud.creates, 0);
  assert.notEqual((await batchOf(h, id)).legs[0].state, 'STARTED');
});
test('S3: a Start whose matrix die is not in RELEASE_DIES is refused', async () => {
  const dies = JSON.stringify({
    turing: ['TU106', 'TU116'],
    ampere: ['GA102', 'GA104'],
    ada: ['AD104', 'AD103'],
    blackwell: ['GB202', 'GB205'],
  });
  const h = await harness({ RELEASE_DIES: dies });
  await assert.rejects(
    () => h.service.adminStart(body()),
    (e) => e.state === 'BAD_REQUEST',
  );
  assert.equal(h.cloud.creates, 0);
});
test('S3: the daily rental count cap applies to a rehearsal plan', async () => {
  const h = await harness({ ...SPEC, RELEASE_DAILY_RENTALS: '1' });
  twoMachines(h);
  const { plan } = await h.service.adminPlan(body());
  assert.equal(plan.legs, 2);
  assert.ok(plan.refusals.includes('DAILY_COUNT'), JSON.stringify(plan.refusals));
});

// ---- round 2 of the review of PR #355: R1 (a lane B rerun keeps the apps pair) and two NITs ----
const envOfLeg = async (h, key) => {
  const record = await h.storage.get('record:' + key);
  return { record, env: creates(h).find((c) => c.label === record.label).env };
};
test('R1: a rerun of lane B keeps the apps pair; a rerun of lane A still has none', async () => {
  const h = await harness({ ...SPEC, RELEASE_RERUNS_PER_LEG: '2', MAX_CONCURRENT_RENTALS: '5' });
  h.cloud.extraOffers = [12, 13, 14, 15].map((n) => ({
    ...h.cloud.offer,
    id: n,
    machine_id: 200000 + n,
    host_id: 3000 + n,
    price: 0.4,
  }));
  const { batch: id } = await h.service.adminStart(body());
  let batch = await batchOf(h, id);
  const [a1] = batch.legs;
  assert.equal(a1.spec, 'lanea');
  // lane A is retried (a retry row is not a verdict on the guest); lane B starts once A has ended
  await finish(h, a1.key, 'FAIL', { ...LINUX, windows_retry_media: -1 });
  await tick(h);
  batch = await batchOf(h, id);
  const a2 = batch.legs.find((l) => l.rerunOf === a1.key);
  const b1 = batch.legs.find((l) => l.spec === 'laneb' && !l.rerunOf);
  const states = JSON.stringify(batch.legs.map((l) => [l.key, l.spec, l.state]));
  assert.ok(a2 && a2.state === 'STARTED', states);
  assert.ok(b1 && b1.state === 'STARTED', states);
  const a2run = await envOfLeg(h, a2.key);
  assert.equal(a2run.env.KF_SPEC, 'lanea');
  assert.ok(!('KF_LINUX_APPS' in a2run.env) && !('KF_WINAPPS' in a2run.env));
  assert.equal(a2run.record.middleAppsRequested, false);
  assert.equal((await envOfLeg(h, b1.key)).env.KF_LINUX_APPS, '1');
  // lane B is retried: the rerun must still carry the apps (nothing else in the release requires them)
  // (a second test below covers a leg without SPEC)
  await finish(h, b1.key, 'FAIL', { ...LINUX, windows_retry_media: -1 });
  await finish(h, a2.key, 'PASS', { ...LINUX, windows_gate: 1, specviewperf_lanea: 1 });
  await tick(h);
  batch = await batchOf(h, id);
  const b2 = batch.legs.find((l) => l.rerunOf === b1.key);
  assert.ok(
    b2 && b2.state === 'STARTED',
    JSON.stringify(batch.legs.map((l) => [l.key, l.spec, l.state])),
  );
  assert.equal(b2.apps, undefined);
  const b2run = await envOfLeg(h, b2.key);
  assert.equal(b2run.env.KF_SPEC, 'laneb');
  assert.equal(b2run.env.KF_LINUX_APPS, '1');
  assert.equal(b2run.env.KF_WINAPPS, 'mid');
  assert.equal(b2run.record.middleAppsRequested, true);
});
test('R1b: the rerun of a rehearsal leg without SPEC keeps the apps pair', async () => {
  const h = await harness({ RELEASE_RERUNS_PER_LEG: '1', MAX_CONCURRENT_RENTALS: '3' });
  h.cloud.extraOffers = [12, 13].map((n) => ({
    ...h.cloud.offer,
    id: n,
    machine_id: 200000 + n,
    host_id: 3000 + n,
    price: 0.4,
  }));
  const { batch: id } = await h.service.adminStart(body());
  let batch = await batchOf(h, id);
  await finish(h, batch.legs[0].key, 'FAIL', { ...LINUX, windows_retry_media: -1 });
  await tick(h);
  batch = await batchOf(h, id);
  const rerun = batch.legs.find((l) => l.rerunOf === batch.legs[0].key);
  assert.ok(
    rerun && rerun.state === 'STARTED',
    JSON.stringify(batch.legs.map((l) => [l.key, l.state])),
  );
  const run = await envOfLeg(h, rerun.key);
  assert.equal(run.env.KF_LINUX_APPS, '1');
  assert.equal(run.env.KF_WINAPPS, 'mid');
  assert.equal(run.env.KF_RELEASE_REHEARSAL, '1');
  assert.equal(run.record.middleAppsRequested, true);
});
test(
  'N1: a rehearsal leg asks for no run report even when the result store is enabled',
  { skip: d1Skip },
  async () => {
    const h = await harness({ DB: new FakeD1(), ROW_TAG_KEY: 't'.repeat(48) });
    await h.service.adminStart(body());
    assert.ok(!('KF_RUN_REPORT' in creates(h)[0].env));
  },
);
test('N2: the shared credit rule is NaN- and string-safe (anything but a sufficient number is short)', async () => {
  for (const credit of [Number.NaN, 'lots', null]) {
    const h = await harness();
    h.cloud.credit = credit;
    const { plan } = await h.service
      .adminPlan(body())
      .catch((e) => ({ plan: { refusals: [e.state ?? 'ERR'] } }));
    assert.ok(plan.refusals.length > 0, String(credit));
  }
  // direct: the helper behind both plans, through the offline plan
  const cfg = applyProfile(configuration({ ...env, ...vars }), 'release', 'large');
  const budget = {
    profiles: { release: { daily_usd: 0, count_today: 0 } },
    daily_usd: 0,
    rolling_usd: 0,
    outstanding_usd: 0,
  };
  const offer = {
    id: 1,
    die: 'AD102',
    machine_id: 5,
    host_id: 6,
    country: 'US',
    gpu_name: 'RTX 4090',
    dph_total: 0.5,
    pricing: { total: 2, effective: 0.5 },
  };
  for (const credit of [Number.NaN, 'lots'])
    assert.ok(
      planRehearsal([offer], cfg, { die: 'AD102', budget, credit }).refusals.includes(
        'CREDIT_RESERVE',
      ),
      String(credit),
    );
  assert.ok(
    !planRehearsal([offer], cfg, { die: 'AD102', budget, credit: 500 }).refusals.includes(
      'CREDIT_RESERVE',
    ),
  );
});
