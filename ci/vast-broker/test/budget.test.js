import test from 'node:test';
import assert from 'node:assert/strict';
import { Ledger } from '../src/ledger.js';
import { BrokerLedger } from '../src/index.js';
import { configuration } from '../src/policy.js';
import { env, clock, MemoryStorage, setup, rentBody } from './helpers.js';

// FALSIFIER WK1: confirmed no-instance attempts consume daily rental slots; reasons/numbers
// are missing; settled costs use TTL; profiles mix; credit counts elapsed spend twice.
const cfg = () => ({ ...configuration(env), profile: 'check', maxConcurrent: 4 });
const record = (n, patch = {}) => ({
  key: `${n}-1`,
  run: String(n),
  attempt: '1',
  sha: String(n).padStart(40, '0'),
  branch: 'branch-' + n,
  profile: 'check',
  machineId: n,
  started: clock,
  ended: null,
  price: 0.5,
  bandwidthCost: 0,
  ttl: 3600000,
  ...patch,
});
const spent = (patch = {}) => ({
  profile: 'check',
  started: clock - 600000,
  ended: clock,
  price: 0.5,
  bandwidthCost: 0,
  ttl: 10800000,
  ...patch,
});
for (const variant of [0, 1]) {
  for (const reason of [
    'cost-key',
    'sha-gap',
    'no-credit',
    'daily-count',
    'daily-usd',
    'rolling-usd',
    'credit-reserve',
    'ledger-full',
    'invalid-cost',
  ])
    test(`budget reserve diagnostic ${reason} ${variant}`, async () => {
      const storage = new MemoryStorage(),
        ledger = new Ledger(storage),
        config = cfg();
      const r = record(100 + variant);
      let credit = 20,
        cost = 0.1;
      if (reason === 'cost-key') await storage.put('cost:' + r.key, spent());
      if (reason === 'sha-gap') await storage.put('sha:' + r.sha, { started: clock - variant });
      if (reason === 'no-credit') credit = variant ? undefined : NaN;
      if (reason === 'invalid-cost') cost = variant ? -1 : NaN;
      if (reason === 'daily-count') {
        config.dailyCount = 1;
        await storage.put('cost:old', spent());
      }
      if (reason === 'daily-usd') config.daily = 0.01 + variant * 0.01;
      if (reason === 'rolling-usd') config.rolling = 0.01 + variant * 0.01;
      if (reason === 'credit-reserve') credit = config.reserve + cost - 0.01 - variant * 0.01;
      if (reason === 'ledger-full')
        for (let i = 0; i < 1000; i++) await storage.put('cost:' + i, spent());
      await assert.rejects(ledger.reserve(r, { cfg: config, now: clock, cost, credit }), (e) => {
        assert.equal(e.state, 'UNAVAILABLE_BUDGET');
        assert.equal(e.reason, 'BUDGET:' + reason);
        assert.equal(e.status, 409);
        assert.ok(e.budget && Object.keys(e.budget).length);
        assert.ok(Object.values(e.budget).every((v) => v === null || Number.isFinite(v)));
        return true;
      });
      assert.equal(await storage.get('record:' + r.key), undefined);
    });
  test('budget settled never-created attempts release daily slots ' + variant, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      config = cfg();
    config.dailyCount = 1;
    const r = record(200 + variant, { id: null, closed: 'GONE', price: 0 });
    await ledger.settle(r, clock + 1);
    await ledger.reserve(record(300 + variant, { started: clock + 1 }), {
      cfg: config,
      now: clock + 1,
      cost: 0.1,
      credit: 20,
    });
    assert.ok(await storage.get('cost:' + r.key), 'one-attempt receipt is retained');
    assert.equal((await storage.get('cost:' + r.key)).created, false);
  });
  test('budget legacy no-instance evidence releases daily slots ' + variant, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      config = cfg();
    config.dailyCount = 1;
    await storage.put('cost:old', spent({ price: 0 }));
    await storage.put('record:old', { id: null, ended: clock, closed: 'GONE' });
    await ledger.budgets(config, clock, 0.1);
  });
  test('budget uncertain or real free rental still consumes slot ' + variant, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      config = cfg();
    config.dailyCount = 1;
    await storage.put('cost:old', spent({ price: 0, created: variant ? true : null }));
    await assert.rejects(
      ledger.budgets(config, clock, 0.1),
      (e) => e.reason === 'BUDGET:daily-count',
    );
  });
  test(
    'budget settlement uses elapsed compute and full transfer allowance ' + variant,
    async () => {
      const storage = new MemoryStorage(),
        ledger = new Ledger(storage);
      const r = record(400 + variant, { id: '900', started: clock - 600000, bandwidthCost: 0.02 });
      await ledger.settle(r, clock);
      const s = await ledger.budgetSummary(cfg(), clock);
      assert.equal(s.spend_rows[0].billed_usd, 0.5 / 6 + 0.02);
      assert.equal(s.outstanding_usd, 0);
      assert.equal(s.profiles.check.count_today, 1);
    },
  );
  test('budget profile and remaining-credit boundaries are independent ' + variant, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      config = cfg();
    const profile = variant ? 'check' : 'sweep';
    await storage.put(
      'cost:old',
      spent({
        profile,
        ended: null,
        started: clock - 1800000,
        ttl: 3600000,
        bandwidthCost: 0.02,
      }),
    );
    const s = await ledger.budgetSummary(config, clock);
    assert.equal(s.rolling_usd, 0.52);
    assert.equal(s.profiles[profile].daily_usd, 0.52);
    assert.equal(s.profiles[variant ? 'sweep' : 'check'].daily_usd, 0);
    assert.equal(s.outstanding_usd, 0.27);
    const credit = config.reserve + 0.1 + 0.27;
    assert.equal(await ledger.budgets(config, clock, 0.1, null, { credit }), 0.27);
    await assert.rejects(
      ledger.budgets(config, clock, 0.1, null, { credit: credit - 0.01 }),
      (e) => e.reason === 'BUDGET:credit-reserve',
    );
  });
  test('budget admin summary is bounded, numeric and read-only ' + variant, async () => {
    const storage = new MemoryStorage();
    const now = Date.now();
    for (let i = 1; i <= 25; i++)
      await storage.put(
        'cost:' + i + '-1',
        spent({
          started: now - i * 1000,
          ended: now,
          secret: 'PRIVATE-SENTINEL',
        }),
      );
    const before = JSON.stringify([...(await storage.list())]);
    const response = await new BrokerLedger({ storage }, env).fetch(
      new Request('https://internal.invalid/admin/list'),
    );
    assert.equal(response.status, 200);
    const data = await response.json();
    assert.equal(data.budget.spend_rows.length, 20);
    assert.equal(data.budget.spend_rows[0].key, '1-1');
    assert.equal(data.budget.profiles.check.count_today, 25);
    assert.equal(data.budget.outstanding_rows.length, 0);
    assert.equal(JSON.stringify(data).includes('PRIVATE-SENTINEL'), false);
    assert.equal(JSON.stringify([...(await storage.list())]), before);
  });
  test('budget explicit failed creates do not exhaust daily rentals ' + variant, async () => {
    const h = await setup({ DAILY_RENTALS: '1' });
    const original = h.cloud.fetch;
    h.cloud.fetch = (url, init) =>
      String(url).includes('/asks/')
        ? Response.json({ success: false }, { status: variant ? 400 : 200 })
        : original(url, init);
    h.restart();
    assert.equal((await h.call()).status, 503);
    assert.equal((await h.storage.get('cost:123-1')).created, false);
    h.cloud.fetch = original;
    h.cloud.runs.set('124', { ...h.cloud.run, id: 124, head_sha: 'c'.repeat(40) });
    h.restart();
    const response = await h.call('/rent', {
      claims: { run_id: '124' },
      body: rentBody('c'.repeat(40)),
    });
    assert.equal(response.status, 200, JSON.stringify(await response.json()));
  });
}

import { spawnSync } from 'node:child_process';
const run = (reason, budget) => {
  const p = spawnSync(
    'python3',
    [
      new URL('./budget_runner_fixture.py', import.meta.url).pathname,
      JSON.stringify({ reason, budget }),
    ],
    { encoding: 'utf8' },
  );
  assert.equal(p.status, 0, p.stderr);
  return JSON.parse(p.stdout);
};
for (const reason of [
  'daily-count',
  'daily-usd',
  'rolling-usd',
  'credit-reserve',
  'cost-key',
  'sha-gap',
  'no-credit',
  'ledger-full',
  'invalid-cost',
  'credit-unavailable',
])
  test('budget actual runner prints bounded public reason ' + reason, () => {
    const r = run('BUDGET:' + reason, { daily_usd: 2, credit_usd: 26, cost_usd: 1.52 });
    assert.equal(r.valid, true);
    assert.match(r.text, new RegExp('BUDGET:' + reason));
    assert.match(r.text, /credit_usd=26/);
    assert.match(r.text, /cost_usd=1.52/);
  });
for (const [reason, budget] of [
  ['BUDGET:PRIVATE-SENTINEL', { cost_usd: 1 }],
  ['BUDGET:daily-usd', { secret: 2 }],
  ['BUDGET:daily-usd', { cost_usd: 'PRIVATE-SENTINEL' }],
  ['BUDGET:daily-usd', { cost_usd: -1 }],
  ['BUDGET:daily-usd', { cost_usd: { secret: 'PRIVATE-SENTINEL' } }],
])
  test(
    'budget runner rejects unknown reason or nonnumeric detail ' + JSON.stringify(budget),
    () => {
      assert.equal(run(reason, budget).valid, false);
    },
  );
