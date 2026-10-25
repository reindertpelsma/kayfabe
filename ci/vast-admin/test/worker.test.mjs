import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { jwt, jwk, clock } from '../../vast-broker/test/helpers.js';

// FALSIFIER: missing/invalid Access identity can read or stop, the wrong owner can enter,
// or any accepted API request uses raw storage or calls the bound DO more than once.
const issuer = 'https://test-team.cloudflareaccess.com';
const audience = 'test-admin-audience';
const email = 'owner@example.com';
const token = (patch = {}, header = {}) =>
  jwt(
    {
      iss: issuer,
      aud: [audience],
      email,
      ...patch,
    },
    header,
  );
const fixture = async (patch = {}) => {
  const { createAdmin } = await import('../worker.js');
  const calls = [];
  const keyCalls = [];
  const names = [];
  const bindings = {
    ACCESS_TEAM_DOMAIN: 'test-team.cloudflareaccess.com',
    ACCESS_AUDIENCE: audience,
    OWNER_EMAIL: email,
    LEDGER: {
      idFromName: (name) => (names.push(name), name),
      get: () => ({
        fetch: async (request) => {
          calls.push(request);
          return Response.json(
            new URL(request.url).pathname === '/admin/list'
              ? { runs: [{ run: '123-1', state: 'BOOTING', ended_at: null }] }
              : { state: 'NONE' },
          );
        },
      }),
    },
    ...patch,
  };
  const worker = createAdmin({
    now: () => clock,
    fetcher: async (url, init) => {
      keyCalls.push(String(url));
      assert.equal(String(url), issuer + '/cdn-cgi/access/certs');
      assert.equal(init.redirect, 'manual');
      return Response.json({ keys: [jwk] });
    },
  });
  return {
    calls,
    keyCalls,
    names,
    async call(path = '/api/list', { bearer, body, headers = {}, method } = {}) {
      return worker.fetch(
        new Request('https://admin.example' + path, {
          method: method ?? (body === undefined ? 'GET' : 'POST'),
          headers: {
            ...(bearer === undefined ? {} : { 'Cf-Access-Jwt-Assertion': bearer }),
            ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
            ...headers,
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        }),
        bindings,
      );
    },
  };
};

for (const path of ['/', '/api/list', '/api/stop'])
  test('admin UI no JWT fails closed ' + path, async () => {
    const h = await fixture();
    const response = await h.call(path);
    assert.equal(response.status, 401);
    assert.equal(h.calls.length, 0);
    assert.equal(h.keyCalls.length, 0);
  });

for (const path of ['/', '/api/list', '/api/stop'])
  test('admin UI valid signature with wrong email is forbidden ' + path, async () => {
    const h = await fixture();
    const response = await h.call(path, { bearer: await token({ email: 'other@example.com' }) });
    assert.equal(response.status, 403);
    assert.equal(h.calls.length, 0);
  });

for (const patch of [
  { email: undefined },
  { aud: 'other' },
  { iss: 'https://other.example' },
  { exp: clock / 1000 - 1 },
  { nbf: clock / 1000 + 60 },
  { exp: undefined },
])
  test('admin UI rejects invalid Access claims ' + Object.keys(patch), async () => {
    const h = await fixture();
    const response = await h.call('/api/list', { bearer: await token(patch) });
    assert.equal(response.status, patch.email === undefined && 'email' in patch ? 403 : 401);
    assert.equal(h.calls.length, 0);
  });

for (const patch of [
  { OWNER_EMAIL: '' },
  { ACCESS_AUDIENCE: '' },
  { ACCESS_TEAM_DOMAIN: '' },
  { ACCESS_TEAM_DOMAIN: 'https://test-team.cloudflareaccess.com' },
  { ACCESS_TEAM_DOMAIN: 'attacker.example' },
])
  test('admin UI unconfigured/invalid policy fails closed ' + JSON.stringify(patch), async () => {
    const h = await fixture(patch);
    assert.equal((await h.call('/api/list', { bearer: await token() })).status, 503);
    assert.equal(h.calls.length, 0);
    assert.equal(h.keyCalls.length, 0);
  });

test('admin UI rejects corrupted signature and oversized assertion', async () => {
  const h = await fixture();
  const signed = await token();
  const parts = signed.split('.');
  parts[2] = (parts[2][0] === 'a' ? 'b' : 'a') + parts[2].slice(1);
  for (const bearer of [parts.join('.'), 'x'.repeat(8193)])
    assert.equal((await h.call('/api/list', { bearer })).status, 401);
  assert.equal(h.calls.length, 0);
});

test('admin UI never trusts email header or Access cookie without a JWT', async () => {
  const h = await fixture();
  assert.equal(
    (
      await h.call('/api/list', {
        headers: {
          'Cf-Access-Authenticated-User-Email': email,
          Cookie: 'CF_Authorization=' + (await token()),
        },
      })
    ).status,
    401,
  );
  assert.equal(h.calls.length, 0);
});

test('admin UI list forwards to named DO once and returns list shape', async () => {
  const h = await fixture();
  const response = await h.call('/api/list', { bearer: await token() });
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    runs: [{ run: '123-1', state: 'BOOTING', ended_at: null }],
  });
  assert.equal(response.headers.get('cache-control'), 'no-store');
  assert.deepEqual(h.names, ['owner-shared-vast-account-enam-v1']);
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].url, 'https://internal.invalid/admin/list');
  assert.equal(h.calls[0].method, 'GET');
  assert.equal(h.calls[0].headers.has('Cf-Access-Jwt-Assertion'), false);
  assert.equal(h.calls[0].headers.has('Authorization'), false);
});

for (const body of [{ run: '123-1' }, { all: true }])
  test('admin UI stop calls the DO exactly once ' + JSON.stringify(body), async () => {
    const h = await fixture();
    const response = await h.call('/api/stop', { bearer: await token(), body });
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { state: 'NONE' });
    assert.equal(h.calls.length, 1);
    assert.equal(h.calls[0].url, 'https://internal.invalid/admin/stop');
    assert.equal(h.calls[0].method, 'POST');
    assert.deepEqual(await h.calls[0].json(), body);
    assert.equal(h.calls[0].headers.get('Content-Type'), 'application/json');
    assert.equal(h.calls[0].headers.has('Cf-Access-Jwt-Assertion'), false);
    assert.equal(h.calls[0].headers.has('Authorization'), false);
  });

for (const body of [{}, { run: '../cron' }, { all: true, run: '123-1' }, null, []])
  test('admin UI rejects invalid stop payload ' + JSON.stringify(body), async () => {
    const h = await fixture();
    assert.equal((await h.call('/api/stop', { bearer: await token(), body })).status, 400);
    assert.equal(h.calls.length, 0);
  });

for (const path of ['/api/list?all=1', '/api/stop?run=123-1', '/admin/list', '/other'])
  test('admin UI rejects unknown/query routes ' + path, async () => {
    const h = await fixture();
    assert.equal((await h.call(path, { bearer: await token() })).status, 404);
    assert.equal(h.calls.length, 0);
  });

test('admin UI rejects foreign-origin stop and wrong media type', async () => {
  const h = await fixture();
  for (const headers of [{ Origin: 'https://foreign.example' }, { 'Content-Type': 'text/plain' }]) {
    const response = await h.call('/api/stop', {
      bearer: await token(),
      body: { all: true },
      headers,
    });
    assert.equal(response.status, headers.Origin ? 403 : 400);
  }
  assert.equal(h.calls.length, 0);
});

test('admin UI reconstructs JWKS for each request', async () => {
  const h = await fixture();
  const bearer = await token();
  await h.call('/api/list', { bearer });
  await h.call('/api/list', { bearer });
  assert.equal(h.keyCalls.length, 2);
});

test('admin UI page is plain table with timed refresh and stop controls', async () => {
  const h = await fixture();
  const response = await h.call('/', { bearer: await token() });
  assert.equal(response.status, 200);
  assert.match(response.headers.get('Content-Type'), /text\/html/);
  const html = await response.text();
  assert.match(html, /<table/);
  assert.match(html, /Stop all/);
  assert.match(html, /confirm\(/);
  assert.match(html, /setInterval\(refresh, 10000\)/);
  assert.match(html, /textContent/);
  assert.doesNotMatch(html, /innerHTML|https:\/\/.*<script/);
  assert.equal(h.calls.length, 0);
});

test('admin UI binding references the existing broker DO without migrations or secrets', async () => {
  const config = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
  assert.match(config, /name = "LEDGER"/);
  assert.match(config, /class_name = "BrokerLedger"/);
  assert.match(config, /script_name = "kayfabe-vast-broker"/);
  assert.doesNotMatch(config, /migrations|VAST_API_KEY|OWNER_STOP_TOKEN/);
});
