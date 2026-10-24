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
      const operation =
        action === 'cron'
          ? h.service.sweep()
          : h
              .call('/stop-all', { token: h.bindings.OWNER_STOP_TOKEN })
              .then((r) => assert.equal(r.status, 200));
      const ready = await Promise.race([
        operation.then(() => true),
        new Promise((resolve) => setTimeout(() => resolve(false), 100)),
      ]);
      assert.equal(ready, true, 'cleanup waited for unauthorized JWKS');
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
    // Item 8: the set of active runs (open: index) replaced the single `active` slot.
    assert.equal(await h.storage.get('open:123-1'), undefined);
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

for (const clients of [5, 10])
  test('security concurrent valid tokens share JWKS flight ' + clients, async () => {
    let calls = 0;
    let release;
    const gate = new Promise((r) => (release = r));
    const { OidcVerifier } = await import('../src/oidc.js');
    const { jwk, clock, env } = await import('./helpers.js');
    const v = new OidcVerifier(
      async () => {
        calls++;
        await gate;
        return Response.json({ keys: [jwk] });
      },
      () => clock,
    );
    const token = await jwt();
    const batch = Array.from({ length: clients }, () => v.verify(token, env.WORKER_URL));
    release();
    assert.equal((await Promise.all(batch)).length, clients);
    assert.equal(calls, 1);
  });

for (const requests of [100, 1000])
  test('security unknown-kid cooldown has bounded fetches ' + requests, async () => {
    const { OidcVerifier } = await import('../src/oidc.js');
    const { env, clock, jwk } = await import('./helpers.js');
    let calls = 0;
    const v = new OidcVerifier(
      async () => {
        calls++;
        return Response.json({ keys: [jwk] });
      },
      () => clock,
    );
    const good = await jwt();
    await v.verify(good, env.WORKER_URL);
    for (let i = 0; i < requests; i++) {
      const parts = good.split('.');
      parts[0] = Buffer.from(JSON.stringify({ alg: 'RS256', kid: 'unknown-' + i })).toString(
        'base64url',
      );
      await assert.rejects(v.verify(parts.join('.'), env.WORKER_URL));
    }
    assert.equal(calls, 1);
  });
