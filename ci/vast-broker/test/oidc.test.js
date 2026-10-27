import test from 'node:test';
import assert from 'node:assert/strict';
import { createHmac, webcrypto } from 'node:crypto';
import { OidcVerifier } from '../src/oidc.js';
import { jwt, jwk, env, clock, baseClaims, setup } from './helpers.js';
// GitHub OIDC acceptance (owner decision 2026-10-23 19:35: verified by jose, not by hand).
// FALSIFIER: any token below that is not the valid one reaches the claim checks or a rental.
const now = clock / 1000;
const verifier = (keys = [jwk], counter = { calls: 0 }) =>
  new OidcVerifier(
    async () => {
      counter.calls++;
      return Response.json({ keys });
    },
    () => clock,
  );
const b64 = (value) => Buffer.from(JSON.stringify(value)).toString('base64url');
const refused = async (token, audience = env.WORKER_URL) => {
  await assert.rejects(verifier().verify(token, audience), (e) => e.status === 401);
};
test('oidc: a valid GitHub-shaped RS256 token is accepted', async () => {
  const claims = await verifier().verify(await jwt(), env.WORKER_URL);
  assert.equal(claims.repository, baseClaims.repository);
  assert.equal(claims.actor_id, '42');
});
test('oidc: alg none is refused', async () => {
  await refused(b64({ alg: 'none', kid: 'test-key' }) + '.' + b64(baseClaims) + '.');
});
test('oidc: HS256 keyed with the public RSA key material is refused', async () => {
  const input = b64({ alg: 'HS256', kid: 'test-key' }) + '.' + b64(baseClaims);
  for (const secret of [jwk.n, JSON.stringify(jwk)]) {
    const tag = createHmac('sha256', secret).update(input).digest('base64url');
    await refused(input + '.' + tag);
  }
});
for (const [name, patch] of [
  ['wrong aud', { aud: 'https://other.example' }],
  ['aud with a trailing slash', { aud: env.WORKER_URL + '/' }],
  ['wrong iss', { iss: 'https://token.actions.githubusercontent.com.evil.example' }],
  ['expired', { exp: now - 120 }],
  ['not yet valid', { nbf: now + 120, iat: now }],
  ['issued far in the future', { iat: now + 600, nbf: now - 1 }],
  ['issued too long ago', { iat: now - 1200, nbf: now - 1200, exp: now + 300 }],
  ['no nbf', { nbf: undefined }],
  ['no exp', { exp: undefined }],
])
  test('oidc: refused ' + name, async () => {
    await refused(await jwt(patch));
  });
test('oidc: refused for an unknown kid', async () => {
  await refused(await jwt({}, { kid: 'not-in-the-jwks' }));
});
test('oidc: refused for a signature by another key', async () => {
  const other = await webcrypto.subtle.generateKey(
    {
      name: 'RSASSA-PKCS1-v1_5',
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: 'SHA-256',
    },
    true,
    ['sign'],
  );
  await refused(await jwt({}, {}, other.privateKey));
});
test('oidc: refused for a garbage or oversized bearer', async () => {
  for (const token of ['', 'a.b', 'a.b.c', 'x'.repeat(9000), (await jwt()) + 'x'])
    await refused(token);
});
test('oidc: an unknown-kid flood fetches the key set at most once per cooldown', async () => {
  const counter = { calls: 0 };
  const v = verifier([jwk], counter);
  await v.verify(await jwt(), env.WORKER_URL);
  const stranger = await jwt({}, { kid: 'stranger' });
  const flood = Array.from({ length: 50 }, () => v.verify(stranger, env.WORKER_URL));
  for (const result of await Promise.allSettled(flood)) assert.equal(result.status, 'rejected');
  assert.ok(counter.calls <= 2, 'JWKS fetches: ' + counter.calls);
});
const vastCalls = (h) => h.cloud.calls.filter((x) => x.url.startsWith('https://console.vast.ai'));
const repo = baseClaims.repository;
for (const [name, patch] of [
  ['repository', { repository: 'reindertpelsma/other' }],
  ['repository id', { repository_id: '7654321' }],
  ['owner id', { repository_owner_id: '43' }],
  ['subject', { sub: 'repo:' + repo + ':ref:refs/heads/master' }],
  ['environment', { sub: 'repo:' + repo + ':environment:other' }],
  ['ref', { ref: 'refs/heads/feature' }],
  [
    'job workflow ref',
    { job_workflow_ref: repo + '/.github/workflows/other.yml@refs/heads/master' },
  ],
  ['workflow ref', { workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/feature' }],
  ['event name', { event_name: 'workflow_run' }],
  ['event name pull_request', { event_name: 'pull_request' }],
  ['run id', { run_id: '../1' }],
])
  test('oidc: verified claims refuse a wrong ' + name, async () => {
    const h = await setup();
    const response = await h.call('/rent', { claims: patch });
    assert.equal(response.status, 401);
    assert.equal(h.cloud.creates, 0);
    assert.deepEqual(vastCalls(h), []);
  });
test('oidc: the valid token rents', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  assert.equal(h.cloud.creates, 1);
});
