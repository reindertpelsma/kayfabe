import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, jwt, sha } from './helpers.js';
// FALSIFIER: unauthenticated I/O holds the billing ledger, an offer refusal
// leaves liability active forever, or rejects exhaust authenticated admission.
for (const action of ['cron', 'stop-all'])
  test('security unauthenticated JWKS cannot starve ' + action, async () => {
    const h = await setup();
    await h.call();
    const base = h.cloud.fetch;
    let release, started;
    const began = new Promise((r) => (started = r)),
      gate = new Promise((r) => (release = r));
    h.cloud.fetch = async (url, init) => {
      if (String(url).includes('/.well-known/jwks')) {
        started();
        await gate;
      }
      return base(url, init);
    };
    h.restart();
    const attack = h.call('/status/123-1', { token: await jwt({}, { kid: 'forged' }) });
    await began;
    try {
      h.time.value += 10 * 3600000;
      if (action === 'cron') await h.service.sweep();
      else
        assert.equal(
          (await h.call('/stop-all', { token: h.bindings.OWNER_STOP_TOKEN })).status,
          200,
        );
      assert.deepEqual(h.cloud.deletes, ['900']);
    } finally {
      release();
      await attack;
    }
  });
for (const code of [200, 400])
  test('security definitive rental refusal releases reservation ' + code, async () => {
    const h = await setup();
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) =>
      String(url).includes('/asks/')
        ? Response.json({ success: false, error: 'no_such_ask' }, { status: code })
        : base(url, init);
    h.restart();
    assert.equal((await h.call()).status, 503);
    const r = await h.storage.get('record:123-1');
    assert.notEqual(r.ended, null);
    assert.equal(await h.storage.get('active'), null);
    assert.equal((await h.call('/stop/123-1', { token: h.bindings.OWNER_STOP_TOKEN })).status, 200);
  });
for (const path of ['/rent', '/availability?profile=check'])
  test('security unauthorized requests cannot spend valid admission ' + path, async () => {
    const h = await setup();
    for (let i = 0; i < 31; i++)
      await h.service.fetch(
        new Request(h.bindings.WORKER_URL + path, { method: path === '/rent' ? 'POST' : 'GET' }),
      );
    assert.equal((await h.call()).status, 200);
  });
