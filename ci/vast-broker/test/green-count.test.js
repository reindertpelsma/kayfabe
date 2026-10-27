import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { env, MemoryStorage, clock } from './helpers.js';

// FALSIFIER: a count ceiling above20 is already configurable without relaxing dollar caps.
for (const count of [21, 40])
  test('green count setting accepts ' + count, () => {
    assert.equal(configuration({ ...env, DAILY_RENTALS: String(count) }).dailyCount, count);
  });
for (const count of [41, 100])
  test('green count setting rejects above40 ' + count, () => {
    assert.throws(() => configuration({ ...env, DAILY_RENTALS: String(count) }));
  });
for (const kind of ['daily-usd', 'rolling-usd', 'credit-reserve'])
  test('green higher count retains ' + kind, async () => {
    const cfg = configuration({ ...env, DAILY_RENTALS: '40' });
    const storage = new MemoryStorage();
    const ledger = new Ledger(storage);
    cfg.profile = 'check';
    if (kind !== 'credit-reserve')
      await storage.put('cost:old', {
        started: clock - 3600000,
        ended: clock,
        price: kind === 'daily-usd' ? 3 : 30,
        created: true,
        profile: kind === 'daily-usd' ? 'check' : 'sweep',
      });
    await assert.rejects(
      ledger.budgets(cfg, clock, 0.1, null, { credit: kind === 'credit-reserve' ? 8 : 100 }),
      (e) => e.reason === 'BUDGET:' + kind,
    );
  });
