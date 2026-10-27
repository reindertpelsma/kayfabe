import test from 'node:test';
import assert from 'node:assert/strict';
import { createGateway } from '../src/index.js';
import { setup, env, clock, nonce, sha, callbackToken } from './helpers.js';
// Plain tests for checks that, when removed by hand (2026-10-23 jose round, mutation tooling
// deleted), failed no other test except the bundle hash.
// FALSIFIER: the hourly offer cap, the contract price recheck or the front gate's run/path
// binding can be deleted without a test failing.
test('spend cap: an offer above the hourly cap only through transfer cost is refused', async () => {
  const h = await setup();
  // 0.25 $/h + 15 GB down and 2 GB up at 0.005 $/GB over 20 minutes = 0.505 $/h effective,
  // while the whole-reservation total (0.1725 $) is still under 0.5 $/h * 21 minutes.
  h.cloud.price = 0.25;
  h.cloud.offer.inet_down_cost = 0.005;
  h.cloud.offer.inet_up_cost = 0.005;
  assert.notEqual((await h.call()).status, 200);
  assert.equal(h.cloud.creates, 0);
});
test('spend cap: a contract repriced above the cap is destroyed on the next update', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  const instance = h.cloud.instances.get('900');
  instance.dph_total = 0.9;
  instance.dph_base = 0.9 - (instance.storage_cost * 120) / 720;
  h.time.value += 60000;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['900']);
  assert.equal((await h.storage.get('record:123-1')).closed, 'CONTRACT_PRICE');
});
test('run token: the front gate refuses a valid token on another run path', async () => {
  const prefix = 'guarantee-fixture-' + 'p'.repeat(32);
  let entries = 0;
  const response = await createGateway({ now: () => clock }).fetch(
    new Request(env.WORKER_URL + '/' + prefix + '/report/123-1', {
      method: 'POST',
      headers: {
        Authorization:
          'Bearer ' +
          callbackToken({ run: '124', attempt: '1', reportExpiry: clock / 1000 + 120, nonce, sha }),
      },
    }),
    {
      ...env,
      BROKER_PATH_PREFIX: prefix,
      LEDGER: {
        idFromName: () => 0,
        get: () => ({ fetch: () => (entries++, Response.json({ state: 'ACCEPTED' })) }),
      },
    },
  );
  assert.equal(response.status, 401);
  assert.equal(entries, 0);
});
