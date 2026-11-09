import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';

// FALSIFIER: default checks already choose the cheapest eligible die rather than list order.
// 2026-11-05 (wtier): that default is the small tier (a master push); a master PR is the middle
// tier and rotates families (tier.test.js), so these runs are pushes.
for (const gpu of ['RTX_4090', 'RTX_4070']) {
  test('green default check chooses cheaper die ' + gpu, async () => {
    const h = await setup();
    h.cloud.run = { ...h.cloud.run, event: 'push', head_branch: 'master', pull_requests: [] };
    h.cloud.offer.gpu_name = 'RTX_3060';
    h.cloud.offer.compute_cap = 860;
    h.cloud.price = 0.4;
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, gpu_name: gpu, compute_cap: 890, price: 0.1 },
    ];
    const response = await h.call('/rent', { claims: { event_name: 'push' } });
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
  test(
    'green check preferences remain ordered when the pin is disabled ' + preferences,
    async () => {
      const h = await setup({ PINNED_MIDDLE_DIES: '[]' });
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
    },
  );
}
