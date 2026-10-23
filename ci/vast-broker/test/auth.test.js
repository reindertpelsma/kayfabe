import test from 'node:test';
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { jwt, jwk, env, clock, baseClaims, setup, sha } from './helpers.js';
const auth = async () => {
  const { OidcVerifier } = await import('../src/oidc.js');
  return new OidcVerifier(
    async () => Response.json({ keys: [jwk] }),
    () => clock,
  );
};
test('RS256 authenticates locally generated GitHub-shaped JWT', async () => {
  const v = await auth();
  assert.equal((await v.verify(await jwt(), env.WORKER_URL)).actor_id, '42');
});
for (const patch of [
  { actor_id: undefined },
  { repository: 'other/repo' },
  { event_name: 'workflow_run' },
  { event_name: 'merge_group' },
  { ref: 'refs/tags/owner-pr' },
  { ref: 'refs/heads/feature' },
  { sha: '$(id)' },
  { run_id: '../1' },
  { run_attempt: '0' },
  { aud: 'wrong' },
  { iss: 'wrong' },
  { exp: clock / 1000 },
  { nbf: clock / 1000 + 1 },
  { nbf: undefined },
]) {
  test('unauthorized token cannot rent ' + JSON.stringify(patch), async () => {
    const h = await setup();
    const r = await h.call('/rent', { claims: patch });
    assert.notEqual(r.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
}
for (const header of [
  { alg: 'none' },
  { alg: 'HS256' },
  { kid: undefined },
  { jku: 'https://evil.example' },
]) {
  test('rejects hostile JWT header ' + JSON.stringify(header), async () => {
    const v = await auth();
    const jwtToken = await jwt({}, header);
    await assert.rejects(v.verify(jwtToken, env.WORKER_URL));
  });
}
test('rejects forged RS256 signature', async () => {
  const v = await auth();
  const k = await webcrypto.subtle.generateKey(
    {
      name: 'RSASSA-PKCS1-v1_5',
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: 'SHA-256',
    },
    true,
    ['sign', 'verify'],
  );
  await assert.rejects(v.verify(await jwt({}, {}, k.privateKey), env.WORKER_URL));
});
test('JWKS cache avoids repeated requests', async () => {
  const { OidcVerifier } = await import('../src/oidc.js');
  let calls = 0;
  const v = new OidcVerifier(
    async () => {
      calls++;
      return Response.json({ keys: [jwk] });
    },
    () => clock,
  );
  const t = await jwt();
  await v.verify(t, env.WORKER_URL);
  await v.verify(t, env.WORKER_URL);
  assert.equal(calls, 1);
});
test('unknown kid rotation refreshes after cooldown', async () => {
  const { OidcVerifier } = await import('../src/oidc.js');
  let calls = 0;
  let current = clock;
  const v = new OidcVerifier(
    async () => Response.json({ keys: [{ ...jwk, kid: ++calls === 1 ? 'test-key' : 'rotated' }] }),
    () => current,
  );
  await v.verify(await jwt(), env.WORKER_URL);
  await assert.rejects(v.verify(await jwt({}, { kid: 'rotated' }), env.WORKER_URL));
  current += 60001;
  await v.verify(await jwt({}, { kid: 'rotated' }), env.WORKER_URL);
  assert.equal(calls, 2);
});
test('same-kid rotation waits for scheduled key refresh', async () => {
  const { OidcVerifier } = await import('../src/oidc.js');
  const k = await webcrypto.subtle.generateKey(
    {
      name: 'RSASSA-PKCS1-v1_5',
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: 'SHA-256',
    },
    true,
    ['sign', 'verify'],
  );
  const rotated = {
    ...(await webcrypto.subtle.exportKey('jwk', k.publicKey)),
    kid: jwk.kid,
    alg: 'RS256',
    use: 'sig',
  };
  let key = jwk;
  let current = clock;
  const v = new OidcVerifier(
    async () => Response.json({ keys: [key] }),
    () => current,
  );
  await v.verify(await jwt(), env.WORKER_URL);
  key = rotated;
  await assert.rejects(v.verify(await jwt({}, {}, k.privateKey), env.WORKER_URL));
  current += 3600001;
  await v.verify(
    await jwt({ exp: current / 1000 + 300, nbf: current / 1000 - 1 }, {}, k.privateKey),
    env.WORKER_URL,
  );
});
for (const change of [
  'triggering_actor',
  'head_sha',
  'head_branch',
  'event',
  'run_attempt',
  'id',
  'repository',
]) {
  test('run API must bind ' + change, async () => {
    const h = await setup();
    h.cloud.run[change] = {
      actor: { id: 43 },
      triggering_actor: { id: 43 },
      head_sha: 'd'.repeat(40),
      head_branch: null,
      event: 'push',
      run_attempt: 2,
      id: 124,
      repository: { full_name: 'elsewhere/repo' },
    }[change];
    assert.notEqual((await h.call('/rent', { claims: { sha, workflow_sha: sha } })).status, 200);
    assert.equal(h.cloud.creates, 0);
  });
}
for (const code of [403, 404, 500])
  test('GitHub API fails closed ' + code, async () => {
    const h = await setup();
    h.cloud.githubError = code;
    const r = await h.call();
    assert.notEqual(r.status, 200);
    assert.equal(h.cloud.creates, 0);
    assert.doesNotMatch(await r.text(), /PRIVATE/);
  });
test('default owner PR event immediately authorizes the head', async () => {
  const h = await setup();
  h.cloud.prs[0].user.id = 99;
  assert.equal((await h.call()).status, 200);
});
for (const spoof of ['owner name', 'owner email'])
  test('nonowner push cannot spoof approval with ' + spoof, async () => {
    const h = await setup();
    h.cloud.run.actor.id = 43;
    h.cloud.run.triggering_actor.id = 43;
    h.cloud.prs[0].user.id = 42;
    h.cloud.prs[0].head.author = { name: 'Owner', email: 'owner@example.com' };
    assert.equal(
      (await (await h.call('/rent', { claims: { actor_id: '43' } })).json()).state,
      'UNAUTHORIZED',
    );
    assert.equal(h.cloud.creates, 0);
  });
test('nonowner rerun cannot inherit owner push approval', async () => {
  const h = await setup();
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.run.run_attempt = 2;
  assert.equal(
    (await (await h.call('/rent', { claims: { run_attempt: '2' } })).json()).state,
    'UNAUTHORIZED',
  );
});
test('dispatch by nonowner is refused even with review', async () => {
  const h = await setup();
  h.cloud.run.event = 'workflow_dispatch';
  h.cloud.run.actor.id = 43;
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: sha }];
  assert.notEqual(
    (
      await h.call('/rent', {
        claims: {
          actor_id: '43',
          event_name: 'workflow_dispatch',
          sha,
          ref: 'refs/heads/owner-pr',
        },
      })
    ).status,
    200,
  );
});
test('review cannot authorize a nonowner push at a newer head', async () => {
  const h = await setup();
  h.cloud.run.actor.id = 43;
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: 'e'.repeat(40) }];
  assert.equal(
    (await (await h.call('/rent', { claims: { actor_id: '43' } })).json()).state,
    'UNAUTHORIZED',
  );
});
