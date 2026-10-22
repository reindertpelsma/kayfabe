import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// H7 FALSIFIER: admission uses legacy balance rather than prepaid credit,
// or missing/malformed/insufficient credit can bypass reserve + full TTL cost.
async function account(shape) {
  const h = await setup(),
    base = h.cloud.fetch;
  h.cloud.fetch = async (url, init) =>
    new URL(url).pathname.replace(/\/$/, '') === '/api/v0/users/current'
      ? Response.json(shape)
      : base(url, init);
  h.restart();
  return h;
}
for (const credit of [39.5, 9])
  test('prepaid credit admits with zero legacy balance ' + credit, async () => {
    const h = await account({ credit, balance: 0 });
    assert.equal((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 1);
  });
for (const credit of [3, 8, 8.06])
  test('prepaid credit must cover reserve plus TTL ' + credit, async () => {
    const h = await account({ credit, balance: 100 });
    const r = await h.call();
    assert.equal(r.status, 409);
    assert.equal((await r.json()).state, 'UNAVAILABLE_BUDGET');
    assert.equal(h.cloud.creates, 0);
  });
for (const credit of [undefined, null, '39.5', -1, false, {}, []])
  test('missing or malformed prepaid credit fails closed ' + JSON.stringify(credit), async () => {
    const h = await account({ credit, balance: 39.5 });
    const r = await h.call();
    assert.equal(r.status, 409);
    assert.equal((await r.json()).state, 'UNAVAILABLE_BUDGET');
    assert.equal(h.cloud.creates, 0);
  });
