import test from 'node:test';
import assert from 'node:assert/strict';
import { createGateway } from '../src/index.js';
import { setup, env, jwt, jwk, clock, repo } from './helpers.js';
// FALSIFIER: a signed but unbound identity reaches GitHub, or one client can
// enter the ledger beyond either cheap control budget, including across routes.
const expected = repo + '/.github/workflows/gpu-vast.yml@refs/heads/master';
for (const [field, wrong] of Object.entries({
  repository_id: '98765',
  repository_owner_id: '43',
  sub: 'repo:' + repo + ':pull_request',
  job_workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/evil',
}))
  for (const value of [undefined, wrong])
    test('identity fail closed ' + field + ' ' + String(value), async () => {
      const h = await setup();
      assert.equal((await h.call('/rent', { claims: { [field]: value } })).status, 401);
      assert.equal(
        h.cloud.calls.filter((c) => new URL(c.url).hostname === 'api.github.com').length,
        0,
      );
    });
for (const field of ['CHECK_JOB_WORKFLOW_REFS', 'SWEEP_JOB_WORKFLOW_REF'])
  test('identity expected config fails closed ' + field, async () => {
    const h = await setup({ [field]: '' });
    assert.equal((await h.call()).status, 503);
    assert.equal(h.cloud.creates, 0);
  });
for (const name of ['owner', 'reviewed'])
  test('identity exact BASE workflow accepted ' + name, async () => {
    const h = await setup();
    assert.equal((await h.call('/rent', { claims: { job_workflow_ref: expected } })).status, 200);
  });
async function controls(extra = {}) {
  let entries = 0;
  const prefix = 'control-test-' + 'q'.repeat(32);
  const gateway = createGateway({
    fetcher: async () => Response.json({ keys: [jwk] }),
    now: () => clock,
  });
  const bindings = {
    ...env,
    BROKER_PATH_PREFIX: prefix,
    CONTROL_IP_PER_MINUTE: '2',
    CONTROL_GLOBAL_PER_MINUTE: '3',
    ...extra,
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async () => {
          entries++;
          return Response.json({});
        },
      }),
    },
  };
  const token = await jwt();
  return {
    get entries() {
      return entries;
    },
    async call(path = '/status/123-1', ip = '192.0.2.1', auth = token) {
      return gateway.fetch(
        new Request(env.WORKER_URL + '/' + prefix + path, {
          method: path.startsWith('/stop') ? 'POST' : 'GET',
          headers: { Authorization: 'Bearer ' + auth, 'CF-Connecting-IP': ip },
        }),
        bindings,
      );
    },
  };
}
for (const route of ['/status/123-1', '/logs/123-1?cursor=0', '/stop/123-1'])
  test('control per-IP budget ' + route, async () => {
    const h = await controls();
    for (let i = 0; i < 2; i++) assert.equal((await h.call(route)).status, 200);
    assert.equal((await h.call(route)).status, 429);
    assert.equal(h.entries, 2);
  });
for (const route of ['/status/123-1', '/logs/123-1?cursor=0'])
  test('control global budget ' + route, async () => {
    const h = await controls();
    for (let i = 0; i < 3; i++)
      assert.equal((await h.call(route, '192.0.2.' + (i + 1))).status, 200);
    assert.equal((await h.call(route, '192.0.2.9')).status, 429);
    assert.equal(h.entries, 3);
  });
for (const route of ['/stop/123-1', '/stop-all'])
  test('owner cleanup bypasses exhausted control budget ' + route, async () => {
    const h = await controls();
    await h.call();
    await h.call();
    assert.equal((await h.call()).status, 429);
    assert.equal((await h.call(route, '192.0.2.1', env.OWNER_STOP_TOKEN)).status, 200);
  });
for (const route of ['/status/123-1', '/logs/123-1?cursor=0'])
  test('unauthorized control noise cannot spend verified budget ' + route, async () => {
    const h = await controls();
    for (let i = 0; i < 10; i++)
      assert.equal((await h.call(route, '192.0.2.1', 'garbage')).status, 401);
    for (let i = 0; i < 2; i++) assert.equal((await h.call(route)).status, 200);
    assert.equal(h.entries, 2);
  });
