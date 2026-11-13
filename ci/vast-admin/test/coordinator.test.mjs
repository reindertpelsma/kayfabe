import test from 'node:test';
import assert from 'node:assert/strict';
import { jwt, jwk, clock } from '../../vast-broker/test/helpers.js';

// FALSIFIER: the coordinator service identity (owner decision 2026-11-13) reaches a route outside
// the table/log/plan/start/stop set, a user JWT or a wrong/absent Client ID takes the service path,
// or the feature is on while COORDINATOR_CLIENT_ID is empty or malformed.
const issuer = 'https://test-team.cloudflareaccess.com';
const audience = 'test-admin-audience';
const owner = 'owner@example.com';
const cid = 'e367826f93b8d71185e03fe518aff3b4.access';
const sha = 'a'.repeat(40);
const service = (patch = {}) =>
  jwt({
    iss: issuer,
    aud: [audience],
    type: 'app',
    common_name: cid,
    sub: '',
    ...patch,
  });
const user = (patch = {}) => jwt({ iss: issuer, aud: [audience], email: owner, ...patch });
const fixture = async (vars = { COORDINATOR_CLIENT_ID: cid }) => {
  const { createAdmin } = await import('../worker.js');
  const ledger = [];
  const hosts = [];
  const env = {
    ACCESS_TEAM_DOMAIN: 'test-team.cloudflareaccess.com',
    ACCESS_AUDIENCE: audience,
    OWNER_EMAIL: owner,
    LEDGER: {
      idFromName: (n) => n,
      get: () => ({
        fetch: async (r) => {
          ledger.push(new URL(r.url).pathname);
          const p = new URL(r.url).pathname;
          return Response.json(p === '/admin/log' ? { text: 'x', next_cursor: 0 } : { ok: 1 });
        },
      }),
    },
    HOSTS: {
      idFromName: (n) => n,
      get: () => ({
        fetch: async (r) => (hosts.push(r.url), Response.json({ ok: 1 })),
      }),
    },
    ...vars,
  };
  const worker = createAdmin({
    now: () => clock,
    fetcher: async () => Response.json({ keys: [jwk] }),
  });
  return {
    ledger,
    hosts,
    call: (path, method, bearer, body, headers = {}) =>
      worker.fetch(
        new Request('https://admin.example' + path, {
          method,
          headers: {
            'Cf-Access-Jwt-Assertion': bearer,
            ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
            ...headers,
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        }),
        env,
      ),
  };
};
const ALLOWED = [
  ['/', 'GET'],
  ['/api/list', 'GET'],
  ['/log?run=1-1', 'GET'],
  ['/api/plan', 'POST', { commit_sha: sha, tier: 'small' }],
  ['/api/start', 'POST', { commit_sha: sha, tier: 'small' }],
  ['/api/stop', 'POST', { run: '1-1' }],
  ['/api/stop', 'POST', { batch: '8' + '0'.repeat(18) }],
  ['/api/stop', 'POST', { all: true }],
];
const FORBIDDEN = [
  ['/api/hosts', 'GET'],
  ['/api/hosts/box1', 'GET'],
  ['/api/hosts/box1', 'PUT', { a: 1 }],
  ['/api/hosts/box1', 'DELETE'],
  ['/api/hosts/box1/token', 'POST'],
  ['/api/hosts/box1/enrol', 'POST'],
  ['/api/hosts/box1/cmd', 'POST', { c: 1 }],
  ['/api/hosts/box1/commands-until', 'PUT', { t: 1 }],
  ['/api/list', 'POST', {}],
  ['/api/stop', 'GET'],
  ['/api/plan', 'GET'],
  ['/other', 'GET'],
];

test('owner path unchanged and marked owner', async () => {
  const h = await fixture();
  const r = await h.call('/api/list', 'GET', await user());
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('x-kf-identity'), 'owner');
});
test('owner keeps full access incl. hosts routes with the var set', async () => {
  const h = await fixture();
  assert.equal((await h.call('/api/hosts', 'GET', await user())).status, 200);
  assert.equal(h.hosts.length, 1);
});
test('owner still works with the var empty or malformed', async () => {
  for (const v of ['', 'junk'])
    assert.equal(
      (await (await fixture({ COORDINATOR_CLIENT_ID: v })).call('/api/list', 'GET', await user()))
        .status,
      200,
    );
});
for (const [path, method, body] of ALLOWED)
  test('coordinator allowed ' + method + ' ' + path + ' ' + JSON.stringify(body), async () => {
    const h = await fixture();
    const r = await h.call(path, method, await service(), body);
    assert.equal(r.status, 200);
    assert.equal(r.headers.get('x-kf-identity'), 'coordinator');
    assert.equal(h.ledger.length, path === '/' ? 0 : 1);
  });
for (const [path, method, body] of FORBIDDEN)
  test('coordinator forbidden ' + method + ' ' + path, async () => {
    const h = await fixture();
    assert.equal((await h.call(path, method, await service(), body)).status, 403);
    assert.equal(h.hosts.length + h.ledger.length, 0);
  });
test('coordinator POST: mismatching Origin refused, missing Origin ok, JSON type required', async () => {
  const h = await fixture();
  const b = { commit_sha: sha, tier: 'small' };
  assert.equal(
    (
      await h.call('/api/plan', 'POST', await service(), b, {
        Origin: 'https://evil.example',
      })
    ).status,
    403,
  );
  assert.equal((await h.call('/api/plan', 'POST', await service(), b)).status, 200);
  assert.equal(
    (
      await h.call('/api/plan', 'POST', await service(), b, {
        'Content-Type': 'text/plain',
      })
    ).status,
    400,
  );
  assert.equal(h.ledger.length, 1);
});
for (const email of ['other@example.com', 'x@example.org'])
  test(
    'service-shaped JWT with the right common_name but email ' + email + ' is not the coordinator',
    async () => {
      const h = await fixture();
      assert.equal((await h.call('/api/list', 'GET', await service({ email }))).status, 403);
      assert.equal(h.ledger.length, 0);
    },
  );
test('user JWT carrying common_name of the coordinator is not the coordinator', async () => {
  const h = await fixture();
  const r = await h.call(
    '/api/list',
    'GET',
    await user({ email: 'x@example.com', common_name: cid }),
  );
  assert.equal(r.status, 403);
});
for (const patch of [
  { common_name: '0'.repeat(32) + '.access' },
  { common_name: undefined },
  { common_name: cid.toUpperCase() },
  { type: 'user' },
  { type: undefined },
  { sub: 'user-123' },
  { sub: undefined },
])
  test('wrong common_name or type refused ' + JSON.stringify(patch), async () => {
    const h = await fixture();
    assert.equal((await h.call('/api/list', 'GET', await service(patch))).status, 403);
    assert.equal(h.ledger.length, 0);
  });
for (const v of [{ COORDINATOR_CLIENT_ID: '' }, {}, { COORDINATOR_CLIENT_ID: undefined }])
  for (const patch of [{ common_name: '' }, {}, { common_name: undefined }])
    test('empty var = feature off ' + JSON.stringify(v) + JSON.stringify(patch), async () => {
      const h = await fixture(v);
      assert.equal((await h.call('/api/list', 'GET', await service(patch))).status, 403);
      assert.equal(h.ledger.length, 0);
    });
for (const v of [
  'junk',
  'E367826F93B8D71185E03FE518AFF3B4.access',
  cid + 'x',
  cid.slice(1),
  ' ' + cid,
  'a.access',
])
  test('malformed var refused even when common_name equals it ' + JSON.stringify(v), async () => {
    const h = await fixture({ COORDINATOR_CLIENT_ID: v });
    assert.equal((await h.call('/api/list', 'GET', await service({ common_name: v }))).status, 403);
    assert.equal(h.ledger.length, 0);
  });
test('expired, wrong audience, wrong issuer, no JWT still refused for a service JWT', async () => {
  for (const [patch, status] of [
    [{ exp: clock / 1000 - 1 }, 401],
    [{ aud: 'other' }, 401],
    [{ iss: 'https://other.example' }, 401],
  ]) {
    const h = await fixture();
    assert.equal((await h.call('/api/list', 'GET', await service(patch))).status, status);
    assert.equal(h.ledger.length, 0);
  }
  const h = await fixture();
  assert.equal((await h.call('/api/list', 'GET', '')).status, 401);
});
