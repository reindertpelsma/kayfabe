import test from 'node:test';
import assert from 'node:assert/strict';
import { AccessPolicy } from '../src/access.js';
import { OidcVerifier } from '../src/oidc.js';
import { clock, jwt, jwk, env } from './helpers.js';
const team = 'unit.cloudflareaccess.com';
const audience = 'unit-audience';
const bindings = {
  ACCESS_TEAM_DOMAIN: team,
  ACCESS_AUD: audience,
  ACCESS_RUNNER_CLIENT_ID: 'a'.repeat(32) + '.access',
  ACCESS_BOX_CLIENT_ID: 'b'.repeat(32) + '.access',
};
// FALSIFIER H14: a correctly signed Access audience array refuses, or GitHub accepts one.
async function authorize(role, aud) {
  const policy = new AccessPolicy(
    async () => Response.json({ keys: [jwk] }),
    () => clock,
  );
  const token = await jwt({
    iss: 'https://' + team,
    aud,
    common_name: bindings[role === 'runner' ? 'ACCESS_RUNNER_CLIENT_ID' : 'ACCESS_BOX_CLIENT_ID'],
  });
  return policy.authorize(
    new Request(env.WORKER_URL + (role === 'runner' ? '/availability' : '/report/123-1'), {
      headers: { 'Cf-Access-Jwt-Assertion': token },
    }),
    bindings,
  );
}
for (const role of ['runner', 'box']) {
  for (const aud of [[audience], ['other-app', audience]])
    test('round Access accepts bound audience array ' + role + ' ' + aud.length, async () => {
      await authorize(role, aud);
    });
  for (const [name, aud] of [
    ['empty', []],
    ['wrong', ['other-app']],
    ['number', [audience, 1]],
    ['null', [audience, null]],
    ['object', [{}]],
    ['oversize', Array(17).fill(audience)],
  ])
    test('round Access rejects audience array ' + role + ' ' + name, async () => {
      await assert.rejects(authorize(role, aud), (error) => error.status === 403);
    });
}
for (const aud of [[env.WORKER_URL], ['other', env.WORKER_URL]])
  test('round GitHub audience stays scalar ' + aud.length, async () => {
    const verifier = new OidcVerifier(
      async () => Response.json({ keys: [jwk] }),
      () => clock,
    );
    await assert.rejects(verifier.verify(await jwt({ aud }), env.WORKER_URL));
  });
