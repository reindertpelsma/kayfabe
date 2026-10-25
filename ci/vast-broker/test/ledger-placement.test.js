import test from 'node:test';
import assert from 'node:assert/strict';
import { createGateway, BrokerLedger } from '../src/index.js';
import { env, clock, jwt, jwk, MemoryStorage } from './helpers.js';

// FALSIFIER: either real entry point selects the old identity, omits enam, or accesses
// the namespace before rejecting a request. No live namespace or provider is used.
const name = 'owner-shared-vast-account-enam-v1';
const prefix = 'placement-test-' + 'p'.repeat(32);
function fixture() {
  const names = [],
    gets = [],
    requests = [];
  const id = {};
  let sweeps = 0;
  const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
  object.service = {
    sweep: async () => sweeps++,
    fetch: async (request) => {
      requests.push(request);
      return new Response('ledger response', { status: 202 });
    },
  };
  const bindings = {
    ...env,
    BROKER_PATH_PREFIX: prefix,
    LEDGER: {
      idFromName: (value) => (names.push(value), id),
      get: (...args) => (gets.push(args), object),
    },
  };
  const gateway = createGateway({
    now: () => clock,
    fetcher: async () => Response.json({ keys: [jwk] }),
  });
  return { gateway, bindings, names, gets, id, requests, sweeps: () => sweeps };
}
function placed(h, count) {
  assert.deepEqual(h.names, Array(count).fill(name));
  assert.deepEqual(
    h.gets,
    Array.from({ length: count }, () => [h.id, { locationHint: 'enam' }]),
  );
}
async function cleanup(h) {
  const pending = [];
  h.gateway.scheduled({}, h.bindings, { waitUntil: (p) => pending.push(p) });
  assert.equal(pending.length, 1);
  await Promise.all(pending);
}
for (const path of ['/status/123-1', '/logs/123-1']) {
  test('placement: authenticated forwarding and cron share the enam ledger ' + path, async () => {
    const h = fixture();
    const request = new Request(env.WORKER_URL + '/' + prefix + path, {
      headers: { Authorization: 'Bearer ' + (await jwt()) },
    });
    const response = await h.gateway.fetch(request, h.bindings);
    assert.equal(response.status, 202);
    assert.equal(await response.text(), 'ledger response');
    assert.equal(h.requests[0], request);
    placed(h, 1);
    await cleanup(h);
    placed(h, 2);
    assert.equal(h.sweeps(), 1);
    assert.equal(h.requests.length, 1);
  });
}
for (const repetitions of [1, 2]) {
  test('placement: scheduled cleanup alone selects enam ledger ' + repetitions, async () => {
    const h = fixture();
    for (let i = 0; i < repetitions; i++) await cleanup(h);
    placed(h, repetitions);
    assert.equal(h.sweeps(), repetitions);
    assert.equal(h.requests.length, 0);
  });
}
for (const [suffix, token, status] of [
  ['/status/123-1', undefined, 401],
  ['/logs/123-1', 'garbage', 401],
  ['/cron', undefined, 404],
  ['/unknown', undefined, 404],
]) {
  test('placement: refusal never resolves or gets ledger ' + suffix + status, async () => {
    const h = fixture();
    const response = await h.gateway.fetch(
      new Request(env.WORKER_URL + '/' + prefix + suffix, {
        headers: token ? { Authorization: 'Bearer ' + token } : {},
      }),
      h.bindings,
    );
    assert.equal(response.status, status);
    placed(h, 0);
    assert.equal(h.requests.length, 0);
    assert.equal(h.sweeps(), 0);
  });
}
for (const badPrefix of ['wrong-' + 'w'.repeat(32), 'short']) {
  test('placement: wrong prefix never resolves or gets ledger ' + badPrefix.length, async () => {
    const h = fixture();
    const response = await h.gateway.fetch(
      new Request(env.WORKER_URL + '/' + badPrefix + '/status/123-1'),
      h.bindings,
    );
    assert.equal(response.status, 404);
    placed(h, 0);
  });
}
