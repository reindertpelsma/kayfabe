import test from 'node:test';
import assert from 'node:assert/strict';
import { createGateway } from '../src/index.js';
import { OidcVerifier } from '../src/oidc.js';
import { setup, env, sha, repo, clock, callbackToken, jwt, jwk } from './helpers.js';
// FALSIFIER H-AUTH (owner 19:25): the Worker authorizes anything beyond signed identity,
// the pinned workflow, triggering_actor == owner and requested sha == run head_sha.
const base = 'd'.repeat(40);
const pulls = (h) =>
  h.cloud.calls.filter((x) => /\/pulls(?:\/|$|\?)/.test(new URL(x.url).pathname));
function collaboratorRun(h) {
  h.cloud.prs[0].user = { id: 99 };
  h.cloud.run.actor = { id: 99 };
}
function pushRun(h, branch = 'master') {
  Object.assign(h.cloud.run, { event: 'push', head_branch: branch, pull_requests: [] });
}
const pushClaims = (branch = 'master') => ({
  event_name: 'push',
  ref: 'refs/heads/' + branch,
  workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + branch,
});
test('simplify owner-triggered collaborator PR rents without PR lookups', async () => {
  const h = await setup();
  collaboratorRun(h);
  const response = await h.call('/rent', { claims: { actor_id: '99' } });
  assert.equal(response.status, 200);
  assert.equal((await response.json()).state, 'BOOTING');
  assert.equal(h.cloud.creates, 1);
  assert.deepEqual(pulls(h), []);
});
for (const review of [[], [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: sha }]])
  test('simplify non-owner trigger refuses reviews=' + review.length, async () => {
    const h = await setup();
    collaboratorRun(h);
    h.cloud.run.triggering_actor = { id: 99 };
    h.cloud.reviews = review;
    const response = await h.call('/rent', { claims: { actor_id: '99' } });
    assert.notEqual(response.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const tested of ['e'.repeat(40), base])
  test('simplify requested sha must equal run head ' + tested[0], async () => {
    const h = await setup();
    const response = await h.call('/rent', {
      body: { runner_nonce: 'f'.repeat(64), tested_sha: tested, pr_number: 7 },
    });
    assert.notEqual(response.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const claim of ['sha', 'workflow_sha'])
  test('simplify PR target base-commit OIDC ' + claim + ' rents the run head', async () => {
    // MEASURED shape (public github/docs runs): pull_request_target run.head_sha is the PR
    // head; the OIDC sha/workflow_sha claims name the BASE commit.
    const h = await setup();
    const response = await h.call('/rent', { claims: { sha: base, workflow_sha: base } });
    assert.equal(response.status, 200, claim);
    assert.equal(h.cloud.calls.find((x) => x.url.includes('/asks/')).body.env.KF_SHA, sha);
  });
for (const branch of ['master', 'feature'])
  test('simplify check profile refuses workflow_dispatch on ' + branch, async () => {
    const h = await setup();
    Object.assign(h.cloud.run, { event: 'workflow_dispatch', head_branch: branch });
    h.cloud.run.pull_requests = [];
    const response = await h.call('/rent', {
      claims: { event_name: 'workflow_dispatch', ref: 'refs/heads/' + branch },
    });
    assert.notEqual(response.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const fork of ['fork/kayfabe', null])
  test('simplify foreign head repository refuses ' + fork, async () => {
    const h = await setup();
    h.cloud.run.head_repository = fork === null ? null : { full_name: fork };
    assert.notEqual((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const name of [
  'APPROVAL_MODE',
  'RENT_DRAFTS',
  'ALLOWED_BASE_BRANCHES',
  'PATH_SKIP_PATTERNS',
  'MANUAL_CEILING_MINUTES',
])
  test('simplify retired variable is ignored ' + name, async () => {
    const h = await setup({ [name]: 'retired value' });
    assert.equal((await h.call()).status, 200);
    assert.equal(h.cloud.creates, 1);
  });
for (const branch of ['feature', 'release'])
  test('simplify owner branch workflow copy refuses ' + branch, async () => {
    const h = await setup();
    pushRun(h, branch);
    const response = await h.call('/rent', {
      claims: pushClaims(branch),
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: null },
    });
    assert.notEqual(response.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const trigger of ['42', '99'])
  test('simplify push to master rents only for owner trigger ' + trigger, async () => {
    const h = await setup();
    pushRun(h);
    h.cloud.run.triggering_actor = { id: Number(trigger) };
    const response = await h.call('/rent', {
      claims: pushClaims(),
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: null },
    });
    assert.equal(response.status === 200, trigger === '42');
    assert.equal(h.cloud.creates, trigger === '42' ? 1 : 0);
  });
for (const route of ['hold', 'stop-timer'])
  test('simplify retired manual route is not found ' + route, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    const record = await h.storage.get('record:123-1');
    await h.report({ seq: 1, kind: 'result', result: { verdict: 'PASS', nonce: record.nonce } });
    const token = route === 'hold' ? callbackToken(record) : env.OWNER_STOP_TOKEN;
    const response = await h.call('/' + route + '/123-1', { token, body: undefined });
    assert.equal(response.status, 404);
    assert.equal((await h.storage.get('record:123-1')).phase, 'GRACE');
  });
for (const path of ['/rent', '/status/123-1'])
  test('simplify status has no manual deadline ' + path, async () => {
    const h = await setup();
    let response = await h.call();
    if (path !== '/rent') response = await h.call(path);
    const body = await response.json();
    assert.equal(response.status, 200);
    assert.ok(!Object.hasOwn(body, 'manual_deadline_at'));
  });
for (const patch of [
  { ACCESS_AUD: 'only-one-variable' },
  {
    ACCESS_TEAM_DOMAIN: 'unit.cloudflareaccess.com',
    ACCESS_AUD: 'unit-audience',
    ACCESS_RUNNER_CLIENT_ID: 'a'.repeat(32) + '.access',
    ACCESS_BOX_CLIENT_ID: 'b'.repeat(32) + '.access',
  },
])
  test('simplify gateway ignores Access variables ' + Object.keys(patch).length, async () => {
    let entries = 0;
    const calls = [];
    const gateway = createGateway({
      now: () => clock,
      authorize: async () => {},
      fetcher: async (url) => {
        calls.push(String(url));
        return new Response('unexpected', { status: 500 });
      },
    });
    const response = await gateway.fetch(new Request(env.WORKER_URL + '/status/123-1'), {
      ...env,
      ...patch,
      RENTING_ENABLED: 'false',
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async () => {
            entries++;
            return new Response('admitted');
          },
        }),
      },
    });
    assert.equal(await response.text(), 'admitted');
    assert.equal(entries, 1);
    assert.deepEqual(calls, []);
  });
for (const change of ['closed', 'moved'])
  test('simplify lifecycle performs no PR lookups ' + change, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    assert.equal((await h.call()).status, 200);
    if (change === 'closed') h.cloud.prs[0].state = 'closed';
    else h.cloud.prs[0].head.sha = 'e'.repeat(40);
    h.time.value += 120000;
    await h.service.sweep();
    assert.deepEqual(pulls(h), []);
    assert.deepEqual(h.cloud.deletes, []);
  });
// GitHub publishes several keys; the verifier selects by kid, never by position.
const extraKeys = await Promise.all(
  ['rotated-a', 'rotated-b'].map(async (kid) => {
    const { webcrypto } = await import('node:crypto');
    const pair = await webcrypto.subtle.generateKey(
      {
        name: 'RSASSA-PKCS1-v1_5',
        modulusLength: 2048,
        publicExponent: new Uint8Array([1, 0, 1]),
        hash: 'SHA-256',
      },
      true,
      ['sign', 'verify'],
    );
    return {
      ...(await webcrypto.subtle.exportKey('jwk', pair.publicKey)),
      kid,
      alg: 'RS256',
      use: 'sig',
    };
  }),
);
for (const count of [1, 2])
  test('simplify GitHub OIDC selects the signing key by kid ' + count, async () => {
    const verifier = new OidcVerifier(
      async () => Response.json({ keys: [...extraKeys.slice(0, count), jwk] }),
      () => clock,
    );
    assert.equal((await verifier.verify(await jwt(), env.WORKER_URL)).actor_id, '42');
  });
