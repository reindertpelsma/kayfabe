import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { RELEASE_MATRIX } from '../src/release-matrix.js';
import { classifyLeg, releaseVerdict, wantsRerun } from '../src/release.js';
import { configuration } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { setup, env, sha, reporterSource } from './helpers.js';
import { Github } from '../src/github.js';
import {
  matrixTest,
  capablePayload,
  receipts,
  receiptParts,
  subrunSuites,
} from './release-fixture.js';
const vars = {
  MAX_CONCURRENT_RENTALS: '9',
  RELEASE_ENABLED: 'true',
  HARD_RUNTIME_MINUTES: '300',
  BOOT_LIMIT_MINUTES: '75',
  ROLLING_SPEND_USD: '200',
  DAILY_SPEND_USD: '40',
  ACCOUNT_DAILY_USD: '100',
};
async function harness(extra = {}) {
  const h = await setup({ ...vars, ...extra });
  h.cloud.credit = 500;
  capablePayload(h);
  h.cloud.extraOffers = JSON.parse(
    readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
  ).offers.map((o) => ({ ...o, price: o.dph_total }));
  h.cloud.contracts = Array.from({ length: 40 }, (_, i) => 900 + i);
  return h;
}
const cfgFor = (extra) =>
  applyProfile(configuration({ ...env, ...vars, ...extra }), 'release', 'large');
const goodRows = () =>
  Object.entries({ cuda_fat: 4, windows_gate: 1, windows_gate_2: 1 }).map(([name, pass]) => ({
    name,
    pass,
    fail: 0,
    skip: 0,
  }));
// FALSIFIERS: a proposed matrix starts any mock rental; ordinary receipts can
// certify matrix coverage; a real failing planned Windows row becomes a retry;
// or one SPEC lane can certify both required lane rows.
test('proposed release plan is not admitted despite affordable offers', async () => {
  const h = await harness();
  const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
  assert.equal(plan.execution_ready, false);
  assert.equal(plan.admitted, false);
  assert.ok(plan.refusals.includes('MATRIX_NOT_EXECUTABLE'));
  assert.equal(h.cloud.creates, 0);
});
test('trusted Worker refuses proposed matrix before any create', async () => {
  const h = await harness();
  await assert.rejects(
    () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
    (e) => e.reason === 'MATRIX_NOT_EXECUTABLE',
  );
  assert.equal(h.cloud.creates, 0);
});
for (const depth of [0, 1])
  test('proposed persisted release cannot rent depth ' + depth, async () => {
    const h = await harness();
    const row = structuredClone(RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'));
    const id = '8123456789012345678';
    const batch = {
      id,
      sha,
      tier: 'large',
      profile: 'release',
      state: 'RUNNING',
      created: h.time.value,
      updated: h.time.value,
      runnerNonce: 'f'.repeat(64),
      costCapUsd: 30,
      estimateUsd: 1,
      windowsRunsPerDie: 2,
      diesPerFamily: 1,
      releaseDies: { ada: ['AD104'] },
      stages: [],
      reruns: 1,
      legs: [
        {
          seq: 1,
          key: id + '-1',
          family: 'ada',
          die: 'AD104',
          windowsRuns: 2,
          releaseMatrix: row,
          state: 'PENDING',
          avoidMachines: [],
          avoidHosts: [],
          rerunDepth: depth,
          queued: h.time.value,
        },
      ],
      verdict: null,
    };
    await h.storage.put('batch:' + id, batch);
    await h.storage.put('batch-open:' + id, id);
    await h.service.sweep();
    assert.equal(h.cloud.creates, 0);
    assert.notEqual((await h.storage.get('batch:' + id)).legs[0].state, 'STARTED');
  });
for (const die of ['AD104', 'TU116'])
  test('attributed planned Windows subrun FAIL stays FAIL ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const cfg = cfgFor();
    const rule = cfg.dieRules.find((r) => r.die === die);
    const gpu = die === 'AD104' ? 'RTX_4070' : 'GTX_1660';
    const result = {
      verdict: 'FAIL',
      host_reference: 'GREEN',
      suites: [...goodRows(), { name: 'windows_subrun_580', pass: 0, fail: 1, skip: 0 }],
    };
    const outcome = classifyLeg(
      {
        ended: 1,
        verdict: 'FAIL',
        hardware: { gpu_name: gpu, compute_cap: rule.cc, driver_version: '580.159.04' },
        result,
      },
      { die, family: row.die_family, cfg, windowsRuns: 2, releaseMatrix: row },
    );
    assert.equal(outcome.verdict, 'FAIL');
    assert.equal(wantsRerun({ outcome }, 1), false);
  });
for (const die of ['AD104', 'TU116'])
  test('ordinary passing rows do not certify matrix coverage ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const cfg = cfgFor();
    const rule = cfg.dieRules.find((r) => r.die === die);
    const result = { verdict: 'PASS', host_reference: 'GREEN', suites: goodRows() };
    const outcome = classifyLeg(
      {
        ended: 1,
        verdict: 'PASS',
        hardware: {
          gpu_name: die === 'AD104' ? 'RTX_4070' : 'GTX_1660',
          compute_cap: rule.cc,
          driver_version: row.drivers.host_driver,
        },
        result,
      },
      { die, family: row.die_family, cfg, windowsRuns: 2, releaseMatrix: row },
    );
    assert.notEqual(outcome.verdict, 'PASS');
  });
for (const filter of ['none', 'one-per-gpu'])
  test('enabled SPEC plans two lane rentals ' + filter, async () => {
    const h = await harness({ RELEASE_STAGES: '["specviewperf"]' });
    if (filter === 'one-per-gpu') {
      const seen = new Set();
      h.cloud.extraOffers = h.cloud.extraOffers.filter((o) => {
        const k = o.gpu_name;
        if (seen.has(k)) return false;
        seen.add(k);
        return true;
      });
    }
    const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
    assert.equal(plan.legs, 16);
    for (const f of plan.families)
      for (const c of f.cells) {
        assert.deepEqual(
          c.legs.map((l) => l.spec),
          ['lanea', 'laneb'],
        );
        assert.ok(c.legs.every((l) => l.est_usd > 0));
        assert.equal(
          c.est_usd,
          Math.round(c.legs.reduce((s, l) => s + l.est_usd, 0) * 10000) / 10000,
        );
      }
    assert.deepEqual(plan.required_stage_rows_per_die, {
      specviewperf: ['specviewperf_lanea', 'specviewperf_laneb'],
    });
    assert.equal(h.cloud.creates, 0);
  });

// Each capability is a separate pre-allocation boundary; protocol 7 alone is
// deliberately insufficient. These use the existing boxProtocol API so the
// pre-fix implementation accepts the bad declarations and fails the assertions.
for (const text of [
  'BOX_PROTOCOL = "7"\n',
  'BOX_PROTOCOL = "7"\nBOX_RELEASE_MATRIX_RECEIPTS = 0\n',
])
  test('matrix mode rejects receipt-incapable reporter ' + text.length, async () => {
    const h = await harness();
    h.cloud.reporterText = text;
    const gh = new Github(h.cloud.fetch, cfgFor());
    await assert.rejects(
      () => gh.boxProtocol(sha, { matrix: true }),
      (e) => e.reason === 'MATRIX_REPORTER_CAPABILITY',
    );
    assert.equal(h.cloud.creates, 0);
  });
for (const text of ['# old payload\n', 'PAYLOAD_RELEASE_MATRIX_EXECUTOR = 0\n'])
  test('matrix mode rejects executor-incapable payload ' + text.length, async () => {
    const h = await harness();
    h.cloud.payloadText = text;
    const gh = new Github(h.cloud.fetch, cfgFor());
    await assert.rejects(
      () => gh.boxProtocol(sha, { matrix: true }),
      (e) => e.reason === 'MATRIX_PAYLOAD_CAPABILITY',
    );
    assert.equal(h.cloud.creates, 0);
  });
for (const depth of [0, 1])
  matrixTest('invalid runtime matrix blocks persisted pending rental ' + depth, async () => {
    const h = await harness();
    const row = structuredClone(RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'));
    const id = '8123456789012345678';
    const batch = {
      id,
      sha,
      tier: 'large',
      profile: 'release',
      state: 'RUNNING',
      created: h.time.value,
      updated: h.time.value,
      runnerNonce: 'f'.repeat(64),
      costCapUsd: 30,
      estimateUsd: 1,
      windowsRunsPerDie: 2,
      diesPerFamily: 1,
      releaseDies: { ada: ['AD104'] },
      stages: [],
      reruns: 1,
      legs: [
        {
          seq: 1,
          key: id + '-1',
          family: 'ada',
          die: 'AD104',
          windowsRuns: 2,
          releaseMatrix: row,
          state: 'PENDING',
          avoidMachines: [],
          avoidHosts: [],
          rerunDepth: depth,
          queued: h.time.value,
        },
      ],
      verdict: null,
    };
    await h.storage.put('batch:' + id, batch);
    await h.storage.put('batch-open:' + id, id);
    RELEASE_MATRIX.schema_version = 2;
    await h.service.sweep();
    assert.equal(h.cloud.creates, 0);
    const updated = await h.storage.get('batch:' + id);
    assert.equal(updated.legs[0].lastRefusal, 'MATRIX_INVALID');
    assert.notEqual(updated.legs[0].state, 'STARTED');
  });
for (const axis of ['linux_guest_driver', 'windows_guest_driver'])
  matrixTest('driver receipts must match selected ' + axis, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'),
      proof = receipts(row);
    proof.rows[0].drivers[axis] = '590.48.01';
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: proof,
      suites: [...goodRows(), ...subrunSuites(row)],
    };
    const record = {
      ended: 1,
      verdict: 'PASS',
      hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: row.drivers.host_driver },
      result,
    };
    assert.notEqual(
      classifyLeg(record, {
        windowsRuns: 2,
        die: row.die,
        family: row.die_family,
        cfg: cfgFor(),
        releaseMatrix: row,
      }).verdict,
      'PASS',
    );
  });
for (const os of ['linux', 'windows'])
  matrixTest('every planned cross receipt is required ' + os, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'),
      proof = receipts(row);
    proof.rows = proof.rows.filter((r) => r.id !== os + '_cross');
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: proof,
      suites: [...goodRows(), ...subrunSuites(row)],
    };
    const record = {
      ended: 1,
      verdict: 'PASS',
      hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: row.drivers.host_driver },
      result,
    };
    assert.notEqual(
      classifyLeg(record, {
        windowsRuns: 2,
        die: row.die,
        family: row.die_family,
        cfg: cfgFor(),
        releaseMatrix: row,
      }).verdict,
      'PASS',
    );
  });
for (const os of ['linux', 'windows'])
  matrixTest('every planned subrun suite is required ' + os, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104');
    const suites = [...goodRows(), ...subrunSuites(row)].filter(
      (r) => !r.name.startsWith(os + '_subrun_'),
    );
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: receipts(row),
      suites,
    };
    const record = {
      ended: 1,
      verdict: 'PASS',
      hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: row.drivers.host_driver },
      result,
    };
    assert.notEqual(
      classifyLeg(record, {
        windowsRuns: 2,
        die: row.die,
        family: row.die_family,
        cfg: cfgFor(),
        releaseMatrix: row,
      }).verdict,
      'PASS',
    );
  });
for (const die of ['AD104', 'TU116'])
  matrixTest('complete selected receipts can pass ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die),
      cfg = cfgFor();
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: receipts(row),
      suites: [...goodRows(), ...subrunSuites(row)],
    };
    const record = {
      ended: 1,
      verdict: 'PASS',
      hardware: {
        gpu_name: die === 'AD104' ? 'RTX_4070' : 'GTX_1660',
        compute_cap: cfg.dieRules.find((r) => r.die === die).cc,
        driver_version: row.drivers.host_driver,
      },
      result,
    };
    assert.equal(
      classifyLeg(record, { windowsRuns: 2, die, family: row.die_family, cfg, releaseMatrix: row })
        .verdict,
      'PASS',
    );
  });
for (const missing of ['specviewperf_lanea', 'specviewperf_laneb'])
  matrixTest('both SPEC rows are independently required ' + missing, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104');
    const outcome = {
      verdict: 'PASS',
      linux: 'PASS',
      windows: ['PASS', 'PASS'],
      stages: { specviewperf: 'PASS' },
      stageRows: { specviewperf_lanea: 'PASS', specviewperf_laneb: 'PASS' },
      matrix: receiptParts(row),
    };
    delete outcome.stageRows[missing];
    const batch = {
      tier: 'large',
      releaseDies: { ada: ['AD104'] },
      diesPerFamily: 1,
      windowsRunsPerDie: 2,
      stages: ['specviewperf'],
      legs: [
        { key: 'x', family: 'ada', die: row.die, state: 'ENDED', releaseMatrix: row, outcome },
      ],
    };
    assert.equal(releaseVerdict(batch).verdict, 'INCOMPLETE');
    outcome.stageRows[missing] = 'PASS';
    assert.equal(releaseVerdict(batch).verdict, 'PASS');
  });
for (const die of ['AD104', 'TU116'])
  matrixTest('matrix coverage cannot be omitted from a release cell ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const batch = {
      tier: 'large',
      releaseDies: { [row.die_family]: [die] },
      diesPerFamily: 1,
      windowsRunsPerDie: 2,
      stages: [],
      legs: [
        {
          key: 'x',
          family: row.die_family,
          die,
          state: 'ENDED',
          releaseMatrix: row,
          outcome: { verdict: 'PASS', linux: 'PASS', windows: ['PASS', 'PASS'], stages: {} },
        },
      ],
    };
    assert.equal(releaseVerdict(batch).verdict, 'INCOMPLETE');
    batch.legs[0].outcome.matrix = receiptParts(row);
    assert.equal(releaseVerdict(batch).verdict, 'PASS');
  });

for (const extra of ['', '# second approved fixture\n'])
  test('current reporter carries matrix receipts ' + extra.length, async () => {
    const h = await harness();
    h.cloud.reporterText = reporterSource;
    h.cloud.payloadText = 'PAYLOAD_RELEASE_MATRIX_EXECUTOR = 1\n' + extra;
    await new Github(h.cloud.fetch, cfgFor()).boxProtocol(sha, { matrix: true });
  });
for (const label of ['first', 'second'])
  test('current payload does not advertise an unbuilt executor ' + label, async () => {
    const h = await harness();
    h.cloud.reporterText = reporterSource;
    delete h.cloud.payloadText;
    await assert.rejects(
      () => new Github(h.cloud.fetch, cfgFor()).boxProtocol(sha, { matrix: true }),
      (e) => e.reason === 'MATRIX_PAYLOAD_CAPABILITY',
    );
    assert.equal(h.cloud.creates, 0);
  });

// FALSIFIER: the bounded HTTP parser rejects a valid receipt, or a receipt
// exception permits arbitrary nesting elsewhere in the ordinary report.
for (const die of ['AD104', 'TU116'])
  matrixTest('HTTP parser accepts bounded matrix receipts ' + die, async () => {
    const { parseReport } = await import('../src/reports.js');
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const result = {
      nonce: 'a'.repeat(32),
      runner_nonce: 'b'.repeat(64),
      sha,
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: receipts(row),
      suites: [...goodRows(), ...subrunSuites(row)],
      hardware: {
        gpu_name: die === 'AD104' ? 'RTX_4070' : 'GTX_1660',
        compute_cap: die === 'AD104' ? 890 : 750,
        driver_version: row.drivers.host_driver,
      },
    };
    assert.deepEqual(
      parseReport(JSON.stringify({ seq: 1, kind: 'result', result })).result,
      result,
    );
  });

async function finishLeg(h, leg, { fail = false, spec = leg.spec } = {}) {
  const record = await h.storage.get('record:' + leg.key);
  const row = leg.releaseMatrix;
  const offer = [h.cloud.offer, ...h.cloud.extraOffers].find(
    (o) => o.machine_id === record.machineId,
  );
  const suites = [
    ...goodRows().filter((r) => leg.windowsRuns === 2 || r.name !== 'windows_gate_2'),
    ...subrunSuites(row),
  ];
  if (spec) suites.push({ name: 'specviewperf_' + spec, pass: 1, fail: 0, skip: 0 });
  if (fail) {
    const old = suites.find((r) => r.name === 'windows_subrun_580');
    old.pass = 0;
    old.fail = 1;
  }
  const response = await h.report(
    {
      seq: 1,
      kind: 'result',
      result: {
        nonce: record.nonce,
        verdict: fail ? 'FAIL' : 'PASS',
        host_reference: 'GREEN',
        suites,
        matrix_receipts: receipts(row),
        hardware: {
          gpu_name: offer.gpu_name,
          compute_cap: offer.compute_cap,
          driver_version: row.drivers.host_driver,
        },
      },
    },
    { run: leg.key },
  );
  assert.equal(response.status, 200, await response.text());
}
for (const die of ['AD104', 'TU116'])
  matrixTest('accepted Windows subrun failure cannot be laundered by cron ' + die, async () => {
    const h = await harness();
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id),
      leg = batch.legs.find((l) => l.die === die);
    await finishLeg(h, leg, { fail: true });
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const after = await h.storage.get('batch:' + id);
    assert.equal(after.legs.find((l) => l.key === leg.key).outcome.verdict, 'FAIL');
    assert.equal(after.legs.filter((l) => l.rerunOf === leg.key).length, 0);
    assert.equal(h.cloud.creates, 8);
    assert.equal(after.verdict, 'FAIL');
  });
for (const die of ['AD104', 'TU116'])
  matrixTest('persisted pre-fix downgraded failure is regraded ' + die, async () => {
    const h = await harness();
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id),
      leg = batch.legs.find((l) => l.die === die);
    await finishLeg(h, leg, { fail: true });
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const stale = await h.storage.get('batch:' + id),
      old = stale.legs.find((l) => l.key === leg.key);
    old.outcome = { verdict: 'RETRY', linux: 'PASS', windows: ['PASS', 'PASS'], stages: {} };
    await h.storage.put('batch:' + id, stale);
    await h.service.sweep();
    const after = await h.storage.get('batch:' + id);
    assert.equal(after.legs.find((l) => l.key === leg.key).outcome.verdict, 'FAIL');
    assert.equal(after.verdict, 'FAIL');
    assert.equal(h.cloud.creates, 8);
  });
for (const single of [false, true])
  matrixTest(
    'SPEC lane B waits for settlement and has its own rental window ' + single,
    async () => {
      const h = await harness({ RELEASE_STAGES: '["specviewperf"]' });
      if (single) {
        const seen = new Set([h.cloud.offer.gpu_name]);
        h.cloud.extraOffers = h.cloud.extraOffers.filter((o) => {
          if (seen.has(o.gpu_name)) return false;
          seen.add(o.gpu_name);
          return true;
        });
      }
      const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
      const batch = await h.storage.get('batch:' + id);
      // A persisted batch may have waited most of its 360-minute start window
      // before A was allocated. Its actual rental is still only 31 minutes old.
      batch.created -= 350 * 60000;
      await h.storage.put('batch:' + id, batch);
      assert.equal(h.cloud.creates, 8);
      assert.ok(batch.legs.filter((l) => l.state === 'STARTED').every((l) => l.spec === 'lanea'));
      assert.ok(batch.legs.filter((l) => l.spec === 'laneb').every((l) => l.state === 'PENDING'));
      h.time.value += 20 * 60000;
      for (const leg of batch.legs.filter((l) => l.spec === 'lanea')) await finishLeg(h, leg);
      h.time.value += 11 * 60000;
      await h.service.sweep();
      const after = await h.storage.get('batch:' + id);
      assert.equal(h.cloud.creates, 16);
      const b = after.legs.filter((l) => l.spec === 'laneb');
      assert.equal(b.length, 8);
      assert.ok(b.every((l) => l.state === 'STARTED'));
      for (const leg of b) {
        const record = await h.storage.get('record:' + leg.key);
        const create = h.cloud.calls.find((c) => c.body?.label === record.label);
        assert.equal(create.body.env.KF_SPEC, 'laneb');
        assert.equal(record.spec, 'laneb');
      }
      for (const leg of b) await finishLeg(h, leg);
      h.time.value += 11 * 60000;
      await h.service.sweep();
      const completed = await h.storage.get('batch:' + id);
      assert.equal(completed.verdict, 'PASS');
      assert.equal(completed.legs.filter((l) => l.state === 'ENDED').length, 16);
      assert.equal(h.cloud.creates, 16);
    },
  );
for (const id of ['full', 'windows_cross'])
  matrixTest('Windows ABI receipt binds the selected ' + id, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'),
      proof = receipts(row);
    proof.rows.find((r) => r.id === id).windows_guest_abi = '590.48.01';
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: proof,
      suites: [...goodRows(), ...subrunSuites(row)],
    };
    assert.notEqual(
      classifyLeg(
        {
          ended: 1,
          verdict: 'PASS',
          result,
          hardware: {
            gpu_name: 'RTX_4070',
            compute_cap: 890,
            driver_version: row.drivers.host_driver,
          },
        },
        { die: row.die, family: row.die_family, cfg: cfgFor(), releaseMatrix: row, windowsRuns: 2 },
      ).verdict,
      'PASS',
    );
  });

for (const die of ['AD104', 'TU116'])
  matrixTest('result store cannot certify ordinary receipts as matrix release ' + die, async () => {
    const { rowFromRecord } = await import('../src/ingest.js');
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const record = {
      key: '8123456789012345678-1',
      sha,
      started: 1,
      ended: 2,
      resultAt: 1,
      tier: 'large',
      profile: 'check',
      budgetClass: 'release',
      family: row.die_family,
      die,
      windowsRuns: 2,
      stages: [],
      releaseMatrix: row,
      verdict: 'PASS',
      result: {
        verdict: 'PASS',
        suites: [...goodRows(), ...subrunSuites(row)],
        host_reference: 'GREEN',
      },
      hardware: {
        gpu_name: die === 'AD104' ? 'RTX_4070' : 'GTX_1660',
        compute_cap: die === 'AD104' ? 890 : 750,
        driver_version: row.drivers.host_driver,
      },
    };
    assert.equal(
      rowFromRecord(record, 0, { key: null, kind: 'commit' }, 2, cfgFor()).verdict,
      'UNATTRIBUTED',
    );
    record.result.matrix_receipts = receipts(row);
    assert.equal(
      rowFromRecord(record, 0, { key: null, kind: 'commit' }, 2, cfgFor()).verdict,
      'PASS',
    );
  });

async function pendingBatch(
  h,
  { depth = 0, row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104') } = {},
) {
  const id = '8123456789012345678';
  const batch = {
    id,
    sha,
    tier: 'large',
    profile: 'release',
    state: 'RUNNING',
    created: h.time.value,
    updated: h.time.value,
    runnerNonce: 'f'.repeat(64),
    costCapUsd: 30,
    estimateUsd: 1,
    windowsRunsPerDie: 2,
    diesPerFamily: 1,
    releaseDies: { ada: ['AD104'] },
    stages: [],
    reruns: 1,
    legs: [
      {
        seq: 1,
        key: id + '-1',
        family: 'ada',
        die: 'AD104',
        windowsRuns: 2,
        releaseMatrix: structuredClone(row),
        state: 'PENDING',
        avoidMachines: [],
        avoidHosts: [],
        rerunDepth: depth,
        queued: h.time.value,
      },
    ],
    verdict: null,
  };
  await h.storage.put('batch:' + id, batch);
  await h.storage.put('batch-open:' + id, id);
  return id;
}
for (const kind of ['reporter', 'payload'])
  for (const path of ['admin', 'cron'])
    matrixTest('paid ' + path + ' refuses old matrix ' + kind, async () => {
      const h = await harness();
      if (kind === 'reporter') h.cloud.reporterText = 'BOX_PROTOCOL = "7"\n';
      else h.cloud.payloadText = '# protocol-7 executor not implemented\n';
      const reason =
        kind === 'reporter' ? 'MATRIX_REPORTER_CAPABILITY' : 'MATRIX_PAYLOAD_CAPABILITY';
      if (path === 'admin')
        await assert.rejects(
          () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
          (e) => e.reason === reason,
        );
      else {
        const id = await pendingBatch(h, { depth: 1 });
        await h.service.sweep();
        assert.equal((await h.storage.get('batch:' + id)).legs[0].lastRefusal, reason);
      }
      assert.equal(h.cloud.creates, 0);
    });
for (const axis of ['linux_guest_driver', 'windows_guest_driver'])
  matrixTest('persisted selection cannot change trusted ' + axis, async () => {
    const h = await harness(),
      row = structuredClone(RELEASE_MATRIX.dies.find((r) => r.die === 'AD104'));
    row.drivers[axis] = '590.48.01';
    const id = await pendingBatch(h, { row });
    await h.service.sweep();
    assert.equal(h.cloud.creates, 0);
    assert.equal((await h.storage.get('batch:' + id)).legs[0].lastRefusal, 'MATRIX_PLAN_CHANGED');
  });
for (const die of ['AD104', 'TU116'])
  matrixTest('later clean leg cannot erase an earlier matrix FAIL ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const outcome = {
      verdict: 'PASS',
      linux: 'PASS',
      windows: ['PASS', 'PASS'],
      stages: {},
      matrix: receiptParts(row),
    };
    const failed = structuredClone(outcome);
    failed.verdict = 'FAIL';
    failed.matrix.full = 'FAIL';
    const batch = {
      tier: 'large',
      releaseDies: { [row.die_family]: [die] },
      diesPerFamily: 1,
      windowsRunsPerDie: 2,
      stages: [],
      legs: [failed, outcome].map((o, i) => ({
        key: 'x' + i,
        die,
        family: row.die_family,
        state: 'ENDED',
        releaseMatrix: row,
        outcome: o,
      })),
    };
    assert.equal(releaseVerdict(batch).verdict, 'FAIL');
  });
for (const die of ['AD104', 'TU116'])
  matrixTest('duplicate matrix receipt cannot certify ' + die, () => {
    const row = RELEASE_MATRIX.dies.find((r) => r.die === die),
      proof = receipts(row);
    proof.rows.push(structuredClone(proof.rows[0]));
    const result = {
      verdict: 'PASS',
      host_reference: 'GREEN',
      matrix_receipts: proof,
      suites: [...goodRows(), ...subrunSuites(row)],
    };
    assert.notEqual(
      classifyLeg(
        {
          ended: 1,
          verdict: 'PASS',
          result,
          hardware: {
            gpu_name: die === 'AD104' ? 'RTX_4070' : 'GTX_1660',
            compute_cap: die === 'AD104' ? 890 : 750,
            driver_version: row.drivers.host_driver,
          },
        },
        { die, family: row.die_family, cfg: cfgFor(), releaseMatrix: row, windowsRuns: 2 },
      ).verdict,
      'PASS',
    );
  });
for (const die of ['AD104', 'TU116'])
  matrixTest(
    'invalid matrix blocks a newly persisted retry while cleanup completes ' + die,
    async () => {
      const h = await harness();
      const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
      const batch = await h.storage.get('batch:' + id),
        leg = batch.legs.find((l) => l.die === die);
      const record = await h.storage.get('record:' + leg.key),
        row = leg.releaseMatrix;
      const offer = [h.cloud.offer, ...h.cloud.extraOffers].find(
        (o) => o.machine_id === record.machineId,
      );
      const response = await h.report(
        {
          seq: 1,
          kind: 'result',
          result: {
            nonce: record.nonce,
            verdict: 'FAIL',
            host_reference: 'GREEN',
            suites: [
              ...goodRows().filter((r) => !r.name.startsWith('windows_gate')),
              { name: 'windows_retry_media', pass: 0, fail: 1, skip: 0 },
            ],
            hardware: {
              gpu_name: offer.gpu_name,
              compute_cap: offer.compute_cap,
              driver_version: row.drivers.host_driver,
            },
          },
        },
        { run: leg.key },
      );
      assert.equal(response.status, 200, await response.text());
      RELEASE_MATRIX.schema_version = 2;
      h.time.value += 11 * 60000;
      await h.service.sweep();
      const after = await h.storage.get('batch:' + id),
        retry = after.legs.find((l) => l.rerunOf === leg.key);
      assert.notEqual((await h.storage.get('record:' + leg.key)).ended, null);
      assert.ok(retry);
      assert.equal(retry.state, 'MISSING');
      assert.equal(retry.lastRefusal, 'MATRIX_INVALID');
      assert.equal(h.cloud.creates, 8);
    },
  );
for (const die of ['AD104', 'TU116'])
  for (const condition of ['root-status', 'die-status', 'blocker', 'minimum', 'Windows-ABI'])
    matrixTest('executable policy still refuses ' + condition + ' ' + die, async () => {
      const h = await harness(),
        row = RELEASE_MATRIX.dies.find((r) => r.die === die);
      if (condition === 'root-status') RELEASE_MATRIX.status = 'proposed';
      else if (condition === 'die-status') row.status = 'proposed';
      else if (condition === 'blocker') row.blockers = ['DRIVER_CHOICES_PROPOSED'];
      else if (condition === 'minimum') row.minimum_drivers.linux_guest_driver = 'UNKNOWN';
      else {
        row.windows_guest_abi = 'UNKNOWN';
      }
      await assert.rejects(
        () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
        (e) => e.reason === 'MATRIX_NOT_EXECUTABLE',
      );
      assert.equal(h.cloud.creates, 0);
    });
for (const [spec, winApps] of [
  ['lanea', 'mid'],
  ['laneb', 'full'],
])
  test('merged environment retains matrix, SPEC and Windows apps ' + spec, async () => {
    const { tierEnvironment } = await import('../src/tier.js');
    const row = RELEASE_MATRIX.dies.find((r) => r.die === 'AD104');
    const selected = tierEnvironment('release', {
      tierName: 'large',
      stages: ['specviewperf'],
      spec,
      winApps,
      releaseMatrix: row,
    });
    assert.equal(selected.KF_SPEC, spec);
    assert.equal(selected.KF_WINAPPS, winApps);
    assert.equal(selected.KF_HOST_DRIVER, row.drivers.host_driver);
    assert.deepEqual(
      JSON.parse(Buffer.from(selected.KF_RELEASE_SUBRUNS, 'base64url')).subruns,
      row.subruns,
    );
  });
// FALSIFIER: one settled die frees a slot which a still-running die's lane B
// steals. Filling all eight slots initially must not mask the dependency rule.
for (const die of ['GA102', 'AD102'])
  matrixTest('only the settled die can start its SPEC lane B ' + die, async () => {
    const h = await harness({ RELEASE_STAGES: '["specviewperf"]' });
    // A live die has another offered physical machine. Reservation filtering
    // alone therefore cannot enforce the per-die A-before-B dependency.
    const template = h.cloud.extraOffers.find((o) => o.gpu_name.includes('2060'));
    assert.ok(template);
    h.cloud.extraOffers.push({ ...template, id: 99001, machine_id: 99001, host_id: 99001 });
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id);
    await finishLeg(
      h,
      batch.legs.find((l) => l.die === die && l.spec === 'lanea'),
    );
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const after = await h.storage.get('batch:' + id);
    assert.deepEqual(
      after.legs.filter((l) => l.spec === 'laneb' && l.state === 'STARTED').map((l) => l.die),
      [die],
    );
    assert.equal(h.cloud.creates, 9);
    const live = after.legs.find((l) => l.die === 'TU106' && l.spec === 'lanea');
    assert.equal((await h.storage.get('record:' + live.key)).ended, null);
  });
// A pre-fix cached PASS cannot survive as the displayed release verdict after
// the trusted receipt policy changes, even if the completed batch has no tick.
for (const die of ['AD104', 'TU116'])
  matrixTest('completed legacy PASS is not displayed without matrix coverage ' + die, async () => {
    const h = await harness(),
      row = RELEASE_MATRIX.dies.find((r) => r.die === die);
    const id = await pendingBatch(h, { row });
    const batch = await h.storage.get('batch:' + id);
    batch.state = 'DONE';
    batch.verdict = 'PASS';
    batch.releaseDies = { [row.die_family]: [die] };
    Object.assign(batch.legs[0], {
      die,
      family: row.die_family,
      state: 'ENDED',
      outcome: { verdict: 'PASS', linux: 'PASS', windows: ['PASS', 'PASS'], stages: {} },
    });
    await h.storage.put('batch:' + id, batch);
    assert.equal((await h.service.adminBatches())[0].verdict, 'INCOMPLETE');
    assert.equal(h.cloud.creates, 0);
  });
