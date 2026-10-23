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
  { actor_id: '43' },
  { actor_id: undefined },
  { triggering_actor: '43' },
  { repository: 'other/repo' },
  { event_name: 'workflow_run' },
  { event_name: 'merge_group' },
  { ref: 'refs/tags/owner-pr' },
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
  'actor',
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
      head_branch: 'master',
      event: 'push',
      run_attempt: 2,
      id: 124,
      repository: { full_name: 'elsewhere/repo' },
    }[change];
    assert.notEqual((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 0);
  });
}
for (const change of ['old-head', 'fork', 'different-branch', 'closed', 'ambiguous', 'unknown']) {
  test('PR identity rejects ' + change, async () => {
    const h = await setup();
    const p = h.cloud.prs[0];
    if (change === 'old-head') p.head.sha = 'd'.repeat(40);
    if (change === 'fork') p.head.repo.full_name = 'fork/repo';
    if (change === 'different-branch') p.head.ref = 'other';
    if (change === 'closed') p.state = 'closed';
    if (change === 'ambiguous') {
      h.cloud.prs.push({ ...p, number: 8 });
      h.cloud.run.event = 'workflow_dispatch';
    }
    if (change === 'unknown') h.cloud.prs = [];
    assert.notEqual(
      (
        await h.call(
          '/rent',
          change === 'ambiguous'
            ? { claims: { event_name: 'workflow_dispatch', sha, ref: 'refs/heads/owner-pr' } }
            : {},
        )
      ).status,
      200,
    );
    assert.equal(h.cloud.creates, 0);
  });
}
for (const scenario of ['none', 'old-head', 'wrong-id', 'dismissed', 'changes-requested']) {
  test('review_only rejects ' + scenario, async () => {
    const h = await setup({ APPROVAL_MODE: 'review_only' });
    h.cloud.reviews =
      scenario === 'none'
        ? []
        : [
            {
              id: 1,
              user: { id: scenario === 'wrong-id' ? 43 : 42 },
              state: 'APPROVED',
              commit_id: scenario === 'old-head' ? 'd'.repeat(40) : sha,
            },
          ];
    if (scenario === 'dismissed' || scenario === 'changes-requested')
      h.cloud.reviews.push({
        id: 2,
        user: { id: 42 },
        state: scenario === 'dismissed' ? 'DISMISSED' : 'CHANGES_REQUESTED',
        commit_id: sha,
      });
    const r = await h.call();
    assert.equal((await r.json()).state, 'NEEDS_APPROVAL');
    assert.equal(h.cloud.creates, 0);
  });
}
test('review_or_author accepts owner author', async () => {
  const h = await setup({ APPROVAL_MODE: 'review_or_author' });
  assert.equal((await h.call()).status, 200);
});
test('review mode accepts exact head approval by numeric owner', async () => {
  const h = await setup({ APPROVAL_MODE: 'review_only' });
  h.cloud.prs[0].user.id = 43;
  h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: sha }];
  assert.equal((await h.call()).status, 200);
});
test('owner author does not override latest dismissal', async () => {
  const h = await setup();
  h.cloud.reviews = [{ id: 2, user: { id: 42 }, state: 'DISMISSED', commit_id: sha }];
  assert.equal((await (await h.call()).json()).state, 'NEEDS_APPROVAL');
});
test('author does not override changes requested', async () => {
  const h = await setup();
  h.cloud.reviews = [{ id: 2, user: { id: 42 }, state: 'CHANGES_REQUESTED', commit_id: sha }];
  assert.equal((await (await h.call()).json()).state, 'NEEDS_APPROVAL');
});
test('actor_only is explicitly selectable', async () => {
  const h = await setup({ APPROVAL_MODE: 'actor_only' });
  h.cloud.prs[0].user.id = 43;
  assert.equal((await h.call()).status, 200);
});
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
      'NEEDS_APPROVAL',
    );
    assert.equal(h.cloud.creates, 0);
  });
test('nonowner push with exact-head owner review authorizes', async () => {
  const h = await setup();
  h.cloud.run.actor.id = 43;
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: sha }];
  assert.equal((await h.call('/rent', { claims: { actor_id: '43' } })).status, 200);
});
test('nonowner rerun cannot inherit owner push approval', async () => {
  const h = await setup();
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.run.run_attempt = 2;
  assert.equal(
    (await (await h.call('/rent', { claims: { run_attempt: '2' } })).json()).state,
    'NEEDS_APPROVAL',
  );
});
test('owner_push mode refuses even reviewed nonowner event', async () => {
  const h = await setup({ APPROVAL_MODE: 'owner_push' });
  h.cloud.run.actor.id = 43;
  h.cloud.run.triggering_actor.id = 43;
  h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: sha }];
  assert.equal(
    (await (await h.call('/rent', { claims: { actor_id: '43' } })).json()).state,
    'NEEDS_APPROVAL',
  );
});
test('dispatch from exact PR branch is also accepted for owner', async () => {
  const h = await setup();
  h.cloud.run.event = 'workflow_dispatch';
  h.cloud.run.pull_requests = [];
  assert.equal(
    (
      await h.call('/rent', {
        claims: { event_name: 'workflow_dispatch', sha, ref: 'refs/heads/owner-pr' },
      })
    ).status,
    200,
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
    'NEEDS_APPROVAL',
  );
});
