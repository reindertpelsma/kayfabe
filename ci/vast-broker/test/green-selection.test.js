import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';

// FALSIFIER: default checks already choose the cheapest eligible die rather than list order.
for (const gpu of ['RTX_4090', 'RTX_4070']) {
  test('green default check chooses cheaper die ' + gpu, async () => {
    const h = await setup();
    h.cloud.offer.gpu_name = 'RTX_3060';
    h.cloud.offer.compute_cap = 860;
    h.cloud.price = 0.4;
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, gpu_name: gpu, compute_cap: 890, price: 0.1 },
    ];
    const response = await h.call();
    assert.equal(response.status, 200);
    const data = await response.json();
    assert.equal(data.machine_id, 100002);
    assert.equal(data.die, gpu === 'RTX_4090' ? 'AD102' : 'AD104');
    assert.equal(h.cloud.created[0].ask, 6);
  });
}
for (const preferences of [
  ['GA106', 'AD102'],
  ['AD102', 'GA106'],
]) {
  test('green explicit check preferences remain ordered ' + preferences, async () => {
    const h = await setup();
    h.cloud.offer.gpu_name = 'RTX_3060';
    h.cloud.offer.compute_cap = 860;
    h.cloud.price = 0.4;
    h.cloud.extraOffers = [
      {
        ...h.cloud.offer,
        id: 6,
        machine_id: 100002,
        gpu_name: 'RTX_4090',
        compute_cap: 890,
        price: 0.1,
      },
    ];
    const response = await h.call('/rent', {
      body: {
        runner_nonce: 'f'.repeat(64),
        tested_sha: h.cloud.run.head_sha,
        profile: 'check',
        remaining_seconds: 12000,
        preferences,
      },
    });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).die, preferences[0]);
  });
}
