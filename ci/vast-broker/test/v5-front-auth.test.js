import test from 'node:test';
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { env, clock, jwt, jwk } from './helpers.js';
import * as source from '../src/index.js';
// FALSIFIER: a token which fails stateless signature/claims checks reaches
// the Durable Object; a slow JWKS request prevents an owner cleanup call.
for (const kind of ['garbage', 'forged'])
  test('front auth rejects before Durable Object ' + kind, async () => {
    assert.equal(typeof source.createGateway, 'function');
    let entries = 0,
      reads = 0;
    const gateway = source.createGateway({
      fetcher: async () => {
        reads++;
        return Response.json({ keys: [jwk] });
      },
      now: () => clock,
    });
    let token = 'garbage';
    if (kind === 'forged') {
      const k = await webcrypto.subtle.generateKey(
        {
          name: 'RSASSA-PKCS1-v1_5',
          modulusLength: 2048,
          publicExponent: new Uint8Array([1, 0, 1]),
          hash: 'SHA-256',
        },
        false,
        ['sign', 'verify'],
      );
      token = await jwt({}, {}, k.privateKey);
    }
    const bindings = {
      ...env,
      BROKER_PATH_PREFIX: 'g'.repeat(32),
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async () => {
            entries++;
            return new Response('unexpected');
          },
        }),
      },
    };
    const r = await gateway.fetch(
      new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + '/status/123-1', {
        headers: { Authorization: 'Bearer ' + token },
      }),
      bindings,
    );
    assert.equal(r.status, 401);
    assert.equal(entries, 0);
    assert.equal(reads, kind === 'garbage' ? 0 : 1);
  });
for (const kind of ['repository', 'ref'])
  test('front auth validates claims before Durable Object ' + kind, async () => {
    assert.equal(typeof source.createGateway, 'function');
    let entries = 0;
    const gateway = source.createGateway({
      fetcher: async () => Response.json({ keys: [jwk] }),
      now: () => clock,
    });
    const token = await jwt(
      kind === 'repository' ? { repository: 'other/repo' } : { ref: 'refs/heads/evil' },
    );
    const bindings = {
      ...env,
      BROKER_PATH_PREFIX: 'g'.repeat(32),
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async () => {
            entries++;
            return new Response('unexpected');
          },
        }),
      },
    };
    assert.equal(
      (
        await gateway.fetch(
          new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + '/status/123-1', {
            headers: { Authorization: 'Bearer ' + token },
          }),
          bindings,
        )
      ).status,
      401,
    );
    assert.equal(entries, 0);
  });
