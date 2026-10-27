import test from 'node:test';
import assert from 'node:assert/strict';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';
import { env, clock, MemoryStorage } from './helpers.js';
// FALSIFIER: a failed cost/ended-index write leaves the durable record ended with an open
// spend row, or a legacy interrupted settlement bills forever despite durable end evidence.
const record = () => ({
  key: '123-1',
  run: '123',
  sha: 'a'.repeat(40),
  id: '900',
  profile: 'check',
  started: clock - 600000,
  ended: null,
  price: 0.5,
  ttl: 10800000,
});
for (const failAt of ['cost:', 'ended:'])
  test('budget settlement is atomic at ' + failAt, async () => {
    const storage = new MemoryStorage(),
      r = record();
    await storage.put('record:' + r.key, r);
    await storage.put('cost:' + r.key, { ...r });
    await storage.put('open:' + r.key, true);
    const before = [...(await storage.list())];
    const failing = (s) => ({
      get: (...a) => s.get(...a),
      list: (...a) => s.list(...a),
      delete: (...a) => s.delete(...a),
      put: async (key, value) => {
        await s.put(key, value);
        if (key.startsWith(failAt)) throw Error('injected settlement failure');
      },
      transaction: (action) => s.transaction((tx) => action(failing(tx))),
    });
    await assert.rejects(new Ledger(failing(storage)).settle(r, clock));
    assert.deepEqual([...(await storage.list())], before);
  });
for (const uncreated of [false, true])
  test('budget legacy interrupted settlement uses durable end ' + uncreated, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      r = record();
    const closed = {
      ...r,
      ended: clock,
      closed: uncreated ? 'GONE' : 'STOPPED',
      id: uncreated ? null : r.id,
      price: uncreated ? 0 : r.price,
    };
    await storage.put('record:' + r.key, closed);
    await storage.put('cost:' + r.key, { ...r });
    const before = [...(await storage.list())];
    const summary = await ledger.budgetSummary(configuration(env), clock + 3600000);
    assert.equal(summary.outstanding_usd, 0);
    assert.equal(summary.rolling_usd, uncreated ? 0 : 0.5 / 6);
    assert.equal(summary.profiles.check.count_today, uncreated ? 0 : 1);
    assert.deepEqual([...(await storage.list())], before, 'summary recovery stays read-only');
  });
