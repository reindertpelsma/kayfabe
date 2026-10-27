import test from 'node:test';
import assert from 'node:assert/strict';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';
import { clock, env, MemoryStorage } from './helpers.js';
// FALSIFIER: a retroactively corrected failure starts counting again after its record expires.
for (const mode of ['admission', 'expiry'])
  for (const id of [null, undefined])
    test(`silent legacy proof survives retention ${mode} ${id}`, async () => {
      const storage = new MemoryStorage(),
        ledger = new Ledger(storage);
      const cfg = { ...configuration(env), logRetention: 1 };
      await storage.put('record:100-1', {
        key: '100-1',
        started: clock - 1000,
        ended: clock,
        id,
        closed: 'GONE',
        price: 0,
      });
      await storage.put('cost:100-1', {
        started: clock - 1000,
        ended: clock,
        price: 0.5,
        ttl: 3600000,
        bandwidthCost: 0,
      });
      await storage.put('ended:100-1', clock);
      if (mode === 'admission') {
        await ledger.budgets({ ...cfg, dailyCount: 1 }, clock, 0.1);
        assert.equal((await storage.get('cost:100-1')).created, false);
      }
      await ledger.expire(clock + 2, cfg);
      assert.equal(await storage.get('record:100-1'), undefined);
      const summary = await ledger.budgetSummary(cfg, clock + 2);
      assert.equal(summary.profiles.check.count_today, 0);
      assert.equal(summary.rolling_usd, 0);
      assert.equal((await storage.get('cost:100-1')).created, false);
    });
