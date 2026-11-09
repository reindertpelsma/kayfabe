import test from 'node:test';
import assert from 'node:assert/strict';
import { simulateProviderSeed } from './provider-market-model.js';
const count = Number(process.env.KF_PROVIDER_SIM_SEEDS ?? 1024);
const start = Number(process.env.KF_PROVIDER_SIM_START ?? 9001);
assert.ok(Number.isSafeInteger(count) && count >= 1 && count <= 10000);
assert.ok(Number.isSafeInteger(start) && start >= 1 && start <= 1000000);
for (const offset of [0, 100000])
  test(
    'provider market invariants seeds ' + (start + offset) + '..' + (start + offset + count - 1),
    async () => {
      let rentals = 0;
      for (let seed = start + offset; seed < start + offset + count; seed++)
        rentals += await simulateProviderSeed(seed);
      assert.ok(rentals > 0);
    },
  );
