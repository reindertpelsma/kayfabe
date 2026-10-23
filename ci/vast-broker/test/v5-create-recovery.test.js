import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha } from './helpers.js';
// FALSIFIER: a create that never reached Vast pins the active ledger forever;
// a create that did reach Vast must still reconcile only its exact own box.
for (const stopFirst of [false, true])
  test(
    'ambiguous absent create settles after spaced boot-expiry observations ' + stopFirst,
    async () => {
      const h = await setup();
      const base = h.cloud.fetch;
      h.cloud.fetch = async (url, init) => {
        if (String(url).includes('/asks/')) throw Error('ambiguous transport');
        return base(url, init);
      };
      h.restart();
      assert.equal((await h.call()).status, 503);
      const r = await h.storage.get('record:123-1');
      if (stopFirst)
        assert.equal(
          (await h.call('/stop/123-1', { token: h.bindings.OWNER_STOP_TOKEN })).status,
          200,
        );
      h.time.value = r.bootDeadline + 1;
      await h.service.sweep();
      if (!stopFirst) assert.equal((await h.storage.get('record:123-1')).ended, null);
      h.time.value += 60001;
      await h.service.sweep();
      const settled = await h.storage.get('record:123-1');
      assert.notEqual(settled.ended, null);
      assert.equal(await h.storage.get('active'), null);
      assert.equal((await h.storage.get('cost:123-1')).price, 0);
      assert.deepEqual(h.cloud.deletes, []);
    },
  );
