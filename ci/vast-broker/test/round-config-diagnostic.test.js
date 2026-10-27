import test from 'node:test';
import assert from 'node:assert/strict';
import { createGateway } from '../src/index.js';
import { clock, env, jwt, jwk } from './helpers.js';
// FALSIFIER: an owner cannot learn which secret is malformed when the prefix itself is bad.
for (const path of ['/availability', '/bad/path/availability'])
  test('round owner config diagnostic survives malformed prefix ' + path, async () => {
    const bindings = {
      ...env,
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
    const response = await gateway.fetch(
      new Request(env.WORKER_URL + path, {
        headers: { Authorization: 'Bearer ' + (await jwt()) },
      }),
      bindings,
    );
    assert.equal(response.status, 503);
    assert.ok((await response.text()).includes('CONFIG_SECRET_FORMAT:BROKER_PATH_PREFIX'));
  });
