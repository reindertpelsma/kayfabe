import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULTS } from '../src/defaults.js';
import { configuration, resourceCaps, offerPrice } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { classifyLeg, releaseVerdict, requiredStageRows, planRelease } from '../src/release.js';
import { env, setup, sha, MockCloud } from './helpers.js';
import { executableMatrix, receipts, subrunSuites } from './release-fixture.js';
// FALSIFIER RV2: drift bypasses attribution, malformed SPEC policy certifies PASS,
// or malformed resource fields admit any non-finite/over-cap rental.
const cfg = () =>
  applyProfile(
    configuration({
      ...env,
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      MAX_PRICE_USD_HOUR: '2',
    }),
    'release',
    'large',
  );
for (const control of ['wrong die', 'wrong compute capability', 'stopped'])
  for (const matrix of [false, true])
    test('RV2 drift respects ' + control + ' matrix=' + matrix, () => {
      const c = cfg(),
        m = executableMatrix(),
        row = m.dies.find((r) => r.die === 'AD104');
      if (!matrix) c.profile = 'check';
      const r = {
        ended: 1,
        verdict: 'FAIL',
        hardware: {
          gpu_name: 'RTX_4070',
          compute_cap: 890,
          driver_version: row.drivers.host_driver,
        },
        result: {
          verdict: 'FAIL',
          host_reference: 'GREEN',
          suites: [
            { name: 'windows_subrun_580', pass: 0, fail: 1 },
            { name: 'driver_media_drift', pass: 0, fail: 1 },
          ],
        },
      };
      if (control === 'wrong die') r.hardware.gpu_name = 'RTX_4090';
      if (control === 'wrong compute capability') r.hardware.compute_cap = 860;
      if (control === 'stopped') r.closed = 'STOPPED';
      const opts = {
        die: 'AD104',
        family: 'ada',
        cfg: c,
        windowsRuns: 2,
        ...(matrix ? { releaseMatrix: row } : {}),
      };
      const out = classifyLeg(r, opts);
      assert.equal(out.verdict, control === 'stopped' ? 'UNATTRIBUTED' : 'RETRY');
      assert.equal(out.reason, control === 'stopped' ? 'STOPPED' : 'HARDWARE_MISMATCH');
    });
for (const field of ['requiredRowsPerDie', 'lanes'])
  for (const mode of ['missing', 'empty', 'duplicate', 'wrong'])
    test('RV2 SPEC policy refuses ' + field + ' ' + mode, () => {
      const slot = DEFAULTS.stageSlots.specviewperf,
        saved = structuredClone(slot);
      try {
        if (mode === 'missing') delete slot[field];
        else
          slot[field] =
            mode === 'empty'
              ? []
              : mode === 'duplicate'
                ? [saved[field][0], saved[field][0]]
                : ['unplanned'];
        assert.throws(() => requiredStageRows(['specviewperf']));
        assert.throws(() => planRelease([], { ...cfg(), releaseStages: ['specviewperf'] }));
        assert.throws(() =>
          releaseVerdict({
            tier: 'large',
            windowsRunsPerDie: 2,
            stages: ['specviewperf'],
            releaseDies: { ada: ['AD104'] },
            diesPerFamily: 1,
            legs: [],
          }),
        );
      } finally {
        for (const k of Object.keys(slot)) delete slot[k];
        Object.assign(slot, saved);
      }
    });
for (const tier of ['small', 'middle', 'large', 'adhoc'])
  for (const field of ['diskGb', 'bandwidthDownGb', 'bandwidthUpGb', 'maxInetUsdPerGb'])
    test('RV2 profile rejects malformed ' + tier + '/' + field, () => {
      const profile = DEFAULTS.payloadProfiles[tier],
        saved = { ...profile };
      try {
        for (const value of [undefined, null, NaN, Infinity, -1, '1', false]) {
          if (value === undefined) delete profile[field];
          else profile[field] = value;
          assert.throws(() =>
            resourceCaps(
              { hard: 300 * 60000, cronPeriod: 60000 },
              tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
              tier,
            ),
          );
        }
      } finally {
        for (const k of Object.keys(profile)) delete profile[k];
        Object.assign(profile, saved);
      }
    });
for (const mode of ['missing', 'NaN', 'Infinity'])
  test('RV2 invalid middle cap makes zero rentals ' + mode, async () => {
    const p = DEFAULTS.payloadProfiles.middle,
      saved = { ...p };
    try {
      if (mode === 'missing') delete p.maxInetUsdPerGb;
      else p.maxInetUsdPerGb = mode === 'Infinity' ? Infinity : NaN;
      const h = await setup({
        MAX_PRICE_USD_HOUR: '2',
        DAILY_SPEND_USD: '100',
        ROLLING_SPEND_USD: '200',
        ACCOUNT_DAILY_USD: '100',
        HARD_RUNTIME_MINUTES: '300',
        BOOT_LIMIT_MINUTES: '75',
      });
      h.cloud.offer.inet_down_cost = 0.006;
      h.cloud.offer.inet_up_cost = 0;
      await assert.rejects(() => h.service.adminStart({ commit_sha: sha, tier: 'middle' }));
      assert.equal(h.cloud.creates, 0);
    } finally {
      Object.assign(p, saved);
    }
  });
for (const value of [NaN, Infinity])
  test('RV2 computed quote is finite ' + value, () => {
    const c = cfg();
    c.hard = value;
    assert.equal(
      offerPrice(
        { ...new MockCloud().offer, dph_base: 0.1, dph_total: 0.15, storage_cost: 0.12 },
        c,
      ),
      null,
    );
  });

for (const die of ['AD104', 'TU116'])
  test('RV2 empty SPEC rows cannot certify absent lane B ' + die, () => {
    const c = cfg(),
      m = executableMatrix(),
      row = m.dies.find((r) => r.die === die),
      family = row.die_family;
    c.releaseMatrix = m;
    const names = die === 'AD104' ? ['RTX_4070', 890] : ['GTX_1660', 750];
    const legs = ['lanea', 'laneb'].map((spec, i) => {
      const record = {
        ended: 1,
        verdict: 'PASS',
        hardware: {
          gpu_name: names[0],
          compute_cap: names[1],
          driver_version: row.drivers.host_driver,
        },
        result: {
          verdict: 'PASS',
          host_reference: 'GREEN',
          matrix_receipts: receipts(row),
          suites: [
            ...[
              'cuda_fat',
              'windows_gate',
              ...(spec === 'lanea' ? ['specviewperf_lanea'] : []),
            ].map((name) => ({ name, pass: 1, fail: 0, skip: 0 })),
            ...subrunSuites(row),
          ],
        },
      };
      return {
        key: 'spec-' + i,
        family,
        die,
        spec,
        state: 'ENDED',
        releaseMatrix: row,
        outcome: classifyLeg(record, {
          windowsRuns: 1,
          stages: ['specviewperf'],
          die,
          family,
          cfg: c,
          releaseMatrix: row,
          spec,
        }),
      };
    });
    const batch = {
      tier: 'large',
      windowsRunsPerDie: 2,
      stages: ['specviewperf'],
      releaseDies: { [family]: [die] },
      diesPerFamily: 1,
      legs,
    };
    assert.equal(releaseVerdict(batch, { releaseMatrix: m }).verdict, 'INCOMPLETE');
    const slot = DEFAULTS.stageSlots.specviewperf,
      saved = slot.requiredRowsPerDie;
    try {
      slot.requiredRowsPerDie = [];
      assert.throws(() => releaseVerdict(batch, { releaseMatrix: m }));
    } finally {
      slot.requiredRowsPerDie = saved;
    }
  });
for (const n of [1, 0.75])
  test('RV2 quote rejects computed overflow ' + n, () => {
    const c = cfg();
    c.hard = Number.MAX_VALUE * n;
    c.cronPeriod = Number.MAX_VALUE * n;
    assert.equal(
      offerPrice(
        { ...new MockCloud().offer, dph_base: 0.1, dph_total: 0.15, storage_cost: 0.12 },
        c,
      ),
      null,
    );
  });
for (const tier of ['middle', 'large'])
  test('RV2 resource keys are closed ' + tier, () => {
    const p = DEFAULTS.payloadProfiles[tier];
    try {
      p.diskGbs = 300;
      assert.throws(() =>
        resourceCaps(
          { hard: 300 * 60000, cronPeriod: 60000 },
          tier === 'large' ? 'release' : 'check',
          tier,
        ),
      );
    } finally {
      delete p.diskGbs;
    }
  });

for (const part of ['planner', 'reducer'])
  test('RV2 shipped SPEC contract remains complete ' + part, () => {
    assert.deepEqual(DEFAULTS.stageSlots.specviewperf.requiredRowsPerDie, [
      'specviewperf_lanea',
      'specviewperf_laneb',
    ]);
    assert.deepEqual(DEFAULTS.stageSlots.specviewperf.lanes, ['lanea', 'laneb']);
    assert.doesNotThrow(() => configuration({ ...env, RELEASE_STAGES: '["specviewperf"]' }));
  });

for (const value of [undefined, false])
  test('RV2 SPEC separate-rental contract refuses ' + String(value), () => {
    const slot = DEFAULTS.stageSlots.specviewperf,
      saved = slot.requiresSeparateRentals;
    try {
      if (value === undefined) delete slot.requiresSeparateRentals;
      else slot.requiresSeparateRentals = value;
      assert.throws(() => requiredStageRows(['specviewperf']));
      assert.throws(() => planRelease([], { ...cfg(), releaseStages: ['specviewperf'] }));
    } finally {
      slot.requiresSeparateRentals = saved;
    }
  });
