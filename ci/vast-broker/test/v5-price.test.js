import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { Ledger } from '../src/ledger.js';
import { env, setup, MemoryStorage } from './helpers.js';
const defaults = () => {
  const b = { ...env };
  for (const k of [
    'MAX_PRICE_USD_HOUR',
    'DAILY_SPEND_USD',
    'ROLLING_SPEND_USD',
    'CREDIT_RESERVE_USD',
    'HARD_RUNTIME_MINUTES',
    'BOOT_LIMIT_MINUTES',
  ])
    delete b[k];
  return b;
};
// FALSIFIER: the shipped cap is not $1/hour, admission exceeds it, or the
// three-hour liability plus cron margin is hidden to claim two $3 runs fit.
for (const source of ['configuration', 'admission'])
  test('V5 owner one dollar hourly cap ' + source, async () => {
    const b = defaults();
    if (source === 'configuration') {
      const c = configuration(b);
      assert.deepEqual([c.price, c.daily, c.rolling, c.reserve], [1, 6, 30, 8]);
    } else {
      const h = await setup(b);
      h.bindings.MAX_PRICE_USD_HOUR = undefined;
      h.cloud.price = 0.99;
      assert.equal((await h.call()).status, 200);
    }
  });
for (const price of [1, 1.00001])
  test('V5 default hourly boundary ' + price, async () => {
    const h = await setup(defaults());
    h.bindings.MAX_PRICE_USD_HOUR = undefined;
    h.cloud.price = price;
    assert.equal((await h.call()).status, price === 1 ? 200 : 503);
  });
// Lane 2, 2026-11-04: the three-hour arithmetic below is a 180-minute limit (the sweep default);
// the check profile's 300-minute master gate is pinned after it.
for (const price of [1, 0.99])
  test('V5 daily liability includes cron margin ' + price, async () => {
    const cfg = configuration({ ...defaults(), HARD_RUNTIME_MINUTES: '180' }),
      now = Date.UTC(2026, 9, 22, 12),
      storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      cost = (price * (cfg.hard + cfg.cronPeriod)) / 3600000;
    assert.equal(cfg.price, 1);
    assert.equal(cfg.hard / 3600000, 3);
    assert.equal(cfg.daily, 6);
    await storage.put('cost:first', { started: now - 4 * 3600000, ended: now - 3600000, price });
    if (price === 1)
      await assert.rejects(ledger.budgets(cfg, now, cost), (e) => e.state === 'UNAVAILABLE_BUDGET');
    else await ledger.budgets(cfg, now, cost);
    assert.equal(2 * 3 * 1, 6); // Work time alone: the positive cron liability is additional.
  });

// Lane 2, 2026-11-04. FALSIFIER: the 300-minute check reservation at the $1/h cap is hidden, or two
// full-price master gates fit the default $6 day. At the cap one run reserves 1 x 301/60 = 5.0167
// USD (hard limit plus one cron period): it is admitted on an empty day and refused once 1 USD of
// the day is spent (1 + 5.0167 > 6). At a typical 0.5 USD/h it reserves 2.508 USD.
for (const [prior, ok] of [
  [0, true],
  [1, false],
])
  test('V5 check (Windows master gate) reservation at the cap, prior ' + prior, async () => {
    const cfg = applyProfile(configuration(defaults()), 'check'),
      now = Date.UTC(2026, 10, 4, 12),
      storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      cost = (1 * (cfg.hard + cfg.cronPeriod)) / 3600000;
    assert.equal(cfg.hard / 60000, 300);
    assert.ok(Math.abs(cost - 5.0167) < 0.001, cost);
    if (prior)
      await storage.put('cost:first', { started: now - 2 * 3600000, ended: now - 3600000, price: prior });
    if (ok) await ledger.budgets(cfg, now, cost);
    else await assert.rejects(ledger.budgets(cfg, now, cost), (e) => e.state === 'UNAVAILABLE_BUDGET');
  });
