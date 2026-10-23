import test from 'node:test';
import { webcrypto } from 'node:crypto';
import assert from 'node:assert/strict';
import { createGateway } from '../src/index.js';
import { clock, env, jwt, jwk } from './helpers.js';
const access = {
  ACCESS_TEAM_DOMAIN: 'unit.cloudflareaccess.com',
  ACCESS_AUD: 'unit-audience',
  ACCESS_RUNNER_CLIENT_ID: 'a'.repeat(32) + '.access',
  ACCESS_BOX_CLIENT_ID: 'b'.repeat(32) + '.access',
};
const runner = [
  '/rent',
  '/availability',
  '/status/123-1',
  '/stop/123-1',
  '/stop-all',
  '/logs/123-1',
  '/stop-timer/123-1',
];
const box = ['/report/123-1', '/hold/123-1', '/kill/123-1', '/blob-url/123-1/v1/asset'];
const issuer = 'https://' + access.ACCESS_TEAM_DOMAIN;
async function probe(path, mode, patch = {}, keys = [jwk]) {
  let entries = 0;
  const calls = [];
  const gateway = createGateway({
    now: () => clock,
    authorize: async () => {},
    fetcher: async (url) => {
      calls.push(String(url));
      return Response.json({ keys });
    },
  });
  const bindings = {
    ...env,
    ...access,
    ...patch,
    BROKER_PATH_PREFIX: 'g'.repeat(32),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async () => {
          entries++;
          return new Response('admitted');
        },
      }),
    },
  };
  let assertion;
  if (mode !== 'none')
    assertion = await jwt({
      iss: issuer,
      aud: access.ACCESS_AUD,
      common_name: mode.endsWith('box')
        ? access.ACCESS_BOX_CLIENT_ID
        : access.ACCESS_RUNNER_CLIENT_ID,
      ...(mode === 'aud' ? { aud: 'wrong' } : {}),
      ...(mode === 'issuer' ? { iss: 'https://other.cloudflareaccess.com' } : {}),
      ...(mode === 'no-iat' ? { iat: undefined } : {}),
      ...(mode === 'future-iat' ? { iat: clock / 1000 + 61 } : {}),
      ...(mode === 'expired' ? { iat: clock / 1000 - 300, exp: clock / 1000 - 1 } : {}),
      ...(mode.startsWith('no-nbf') ? { nbf: undefined } : {}),
      ...(mode === 'unknown' ? { common_name: 'unknown.access' } : {}),
    });
  if (mode === 'forged') assertion = assertion.slice(0, -10) + 'A'.repeat(10);
  const result = await gateway.fetch(
    new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + path, {
      headers: assertion ? { 'Cf-Access-Jwt-Assertion': assertion } : {},
    }),
    bindings,
  );
  return { result, entries, calls };
}
// FALSIFIER H8: the wrong Access identity reaches any route's ledger.
for (const path of [...runner, ...box])
  for (const mode of [
    'runner',
    'box',
    'none',
    'forged',
    'aud',
    'expired',
    'unknown',
    'issuer',
    'no-iat',
    'future-iat',
  ])
    test('round Access route matrix ' + path + ' ' + mode, async () => {
      const h = await probe(path, mode);
      const allowed =
        (runner.includes(path) && mode === 'runner') || (box.includes(path) && mode === 'box');
      assert.equal(h.result.status, allowed ? 200 : 403);
      assert.equal(h.entries, allowed ? 1 : 0);
      assert.ok(h.calls.every((x) => x === issuer + '/cdn-cgi/access/certs'));
    });
for (const key of Object.keys(access))
  test('round partial Access public config ' + key, async () => {
    const h = await probe('/rent', 'runner', { [key]: undefined });
    assert.equal(h.entries, 0);
    assert.equal(h.result.status, 503);
    assert.ok((await h.result.text()).includes(key));
  });

for (const role of ['runner', 'box'])
  test('round Access permits valid assertion without nbf ' + role, async () => {
    const h = await probe(role === 'runner' ? '/rent' : '/report/123-1', 'no-nbf-' + role);
    assert.equal(h.result.status, 200);
    assert.equal(h.entries, 1);
  });

const decoyPair = await webcrypto.subtle.generateKey(
  {
    name: 'RSASSA-PKCS1-v1_5',
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
    hash: 'SHA-256',
  },
  true,
  ['sign', 'verify'],
);
const decoy = {
  ...(await webcrypto.subtle.exportKey('jwk', decoyPair.publicKey)),
  kid: 'decoy',
  alg: 'RS256',
  use: 'sig',
};
for (const role of ['runner', 'box'])
  test('round Access selects signing key by kid ' + role, async () => {
    const h = await probe(role === 'runner' ? '/rent' : '/report/123-1', role, {}, [decoy, jwk]);
    assert.equal(h.result.status, 200);
    assert.equal(h.entries, 1);
  });
for (const path of ['/availability', '/bad/path/availability'])
  test('round owner Access config diagnostic survives malformed prefix ' + path, async () => {
    const bindings = {
      ...env,
      ...access,
      BROKER_PATH_PREFIX: 'bad/path=',
      LEDGER: {
        idFromName: () => {
          throw Error('diagnostics must not enter ledger');
        },
      },
    };
    const gateway = createGateway({
      now: () => clock,
      fetcher: async () => Response.json({ keys: [jwk] }),
    });
    const assertion = await jwt({
      iss: issuer,
      aud: access.ACCESS_AUD,
      common_name: access.ACCESS_RUNNER_CLIENT_ID,
    });
    const response = await gateway.fetch(
      new Request(env.WORKER_URL + path, {
        headers: {
          Authorization: 'Bearer ' + (await jwt()),
          'Cf-Access-Jwt-Assertion': assertion,
        },
      }),
      bindings,
    );
    assert.equal(response.status, 503);
    assert.ok((await response.text()).includes('CONFIG_SECRET_FORMAT:BROKER_PATH_PREFIX'));
  });
