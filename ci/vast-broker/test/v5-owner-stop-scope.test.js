import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
for (const actor of ['42', '43'])
  test('OIDC identity never grants owner stop-all ' + actor, async () => {
    const h = await setup();
    await h.call();
    assert.equal((await h.call('/stop-all', { claims: { actor_id: actor } })).status, 401);
    assert.equal(h.cloud.deletes.length, 0);
  });
