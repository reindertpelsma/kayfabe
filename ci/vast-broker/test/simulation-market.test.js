import assert from 'node:assert/strict';
import { matrixTest as test } from './release-fixture.js';
import { simulateSeed } from './simulation-model.js';
const count = Number(process.env.KF_SIM_SEEDS ?? 200),
  start = Number(process.env.KF_SIM_START ?? 1337);
assert.ok(
  Number.isSafeInteger(count) && count >= 1 && count <= 10000,
  'KF_SIM_SEEDS must be 1..10000',
);
assert.ok(
  Number.isSafeInteger(start) && start >= 1 && start <= 1000000,
  'KF_SIM_START must be 1..1000000',
);
// Fixed Mulberry32 seeds; all plans and every mock rental recheck independent invariants.
for (let seed = start; seed < start + count; seed++)
  test('simulation market seed ' + seed, async () => {
    await simulateSeed(seed);
  });

import * as helperPolicy from './helpers.js';
import { DEFAULTS } from '../src/defaults.js';
for (const seed of [1337, 1338])
  test('simulation shipped-profile seed ' + seed, async () => {
    const saved = structuredClone(DEFAULTS.payloadProfiles);
    try {
      Object.assign(
        DEFAULTS.payloadProfiles,
        structuredClone(helperPolicy.shippedPayloadProfiles ?? saved),
      );
      const rentals = await simulateSeed(seed);
      if (
        seed === 1337 &&
        DEFAULTS.downloadModel &&
        DEFAULTS.payloadProfiles.middle.r2ReferenceMbps === 445
      )
        assert.equal(rentals, 0, 'the unchanged middle limit remains infeasible');
    } finally {
      Object.assign(DEFAULTS.payloadProfiles, saved);
    }
  });
