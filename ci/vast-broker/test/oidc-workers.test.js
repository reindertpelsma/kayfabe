import test from 'node:test';
import assert from 'node:assert/strict';
import { OidcVerifier } from '../src/oidc.js';
import { jwt, jwk, env, clock } from './helpers.js';
// jose skips in-flight fetch sharing on Cloudflare Workers (I/O may not cross requests); it
// detects Workers by the WebSocketPair global, set only for this test (test/index.js imports
// every test file into one process).
// FALSIFIER (key-fetch flood, review N2): on the Workers code path an unknown-kid flood makes
// more key-set fetches than requests, or any fetch while the cooldown holds.
test('oidc on Workers: key-set fetches never exceed requests, none during cooldown', async (t) => {
  globalThis.WebSocketPair = class {};
  t.after(() => delete globalThis.WebSocketPair);
  let calls = 0;
  const v = new OidcVerifier(
    async () => {
      calls++;
      await new Promise((resolve) => setTimeout(resolve, 20));
      return Response.json({ keys: [jwk] });
    },
    () => clock,
  );
  const good = await jwt();
  await Promise.all(Array.from({ length: 20 }, () => v.verify(good, env.WORKER_URL)));
  // MEASURED 2026-10-23: a cold isolate fetches once per request in flight (no amplification).
  assert.ok(calls >= 1 && calls <= 20, 'cold fetches ' + calls);
  calls = 0;
  const stranger = await jwt({}, { kid: 'stranger' });
  const flood = await Promise.allSettled(
    Array.from({ length: 200 }, () => v.verify(stranger, env.WORKER_URL)),
  );
  assert.ok(flood.every((x) => x.status === 'rejected'));
  assert.equal(calls, 0);
});
