import test from 'node:test';
import assert from 'node:assert/strict';
import { MemoryStorage, env, clock } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';

// FALSIFIER: counting legs or their start day already preserves one original UTC-day attempt.
for (const offset of [1000, 2000])
  for (const previousDay of [false, true])
    test(
      'green logical attempt UTC day offset=' + offset + ' previous=' + previousDay,
      async () => {
        const storage = new MemoryStorage();
        const day = Math.floor(clock / 86400000) * 86400000;
        const start = previousDay ? day - offset : day + offset;
        for (const [key, started, ended] of [
          ['cost:123-1:provider:0', start, day + 3000],
          ['cost:123-1', day + 4000, day + 5000],
        ])
          await storage.put(key, {
            attemptKey: '123-1',
            attemptStarted: start,
            started,
            ended,
            price: 0.2,
            created: true,
            profile: 'check',
          });
        const summary = await new Ledger(storage).budgetSummary(configuration(env), day + 6000);
        assert.equal(summary.profiles.check.count_today, previousDay ? 0 : 1);
        assert.equal(summary.spend_rows.length, 2);
        assert.ok(summary.rolling_usd > 0);
        assert.ok(summary.daily_usd > 0);
      },
    );
for (const marker of [{ attemptKey: '123-1' }, { attemptStarted: clock - 1000 }])
  test('green incomplete attempt marker refuses ' + JSON.stringify(marker), async () => {
    const storage = new MemoryStorage();
    await storage.put('cost:123-1', {
      started: clock - 1000,
      ended: clock,
      price: 0.2,
      created: true,
      ...marker,
    });
    await assert.rejects(new Ledger(storage).budgetSummary(configuration(env), clock));
  });
