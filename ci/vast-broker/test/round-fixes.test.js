import test from 'node:test';
import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';
import { createGateway } from '../src/index.js';
import { OidcVerifier } from '../src/oidc.js';
import { RunTokens } from '../src/tokens.js';
import { setup, env, clock, nonce, sha, jwt, jwk } from './helpers.js';

// FALSIFIER H1: a forged/expired capability enters the DO or stale state enters its queue.
const prefix = 'round-fixture-' + 'p'.repeat(32);
function tokenV2(patch = {}) {
  const r = { run: '123', attempt: '1', expiry: clock / 1000 + 120, nonce, sha, ...patch };
  const label = 'kf-ci-' + r.run + '-' + r.attempt;
  const message = ['v2', r.run, r.attempt, r.sha, label, r.expiry, r.nonce].join('|');
  const tag = createHmac('sha256', env.RUN_TOKEN_SECRET).update(message).digest('base64url');
  return ['v2', r.run, r.attempt, r.expiry, r.nonce, r.sha, tag].join('.');
}
async function ingress(path, token, count = 1) {
  let entries = 0;
  const gateway = createGateway({ now: () => clock });
  const bindings = {
    ...env,
    BROKER_PATH_PREFIX: prefix,
    LEDGER: {
      idFromName: () => 0,
      get: () => ({
        fetch: () => {
          entries++;
          return Response.json({ state: 'ACCEPTED' });
        },
      }),
    },
  };
  let response;
  for (let i = 0; i < count; i++) {
    response = await gateway.fetch(
      new Request(env.WORKER_URL + '/' + prefix + path, {
        method: path.startsWith('/blob-url') ? 'GET' : 'POST',
        headers: { Authorization: 'Bearer ' + token },
      }),
      bindings,
    );
  }
  return { response, entries };
}
for (const path of ['/report/123-1', '/kill/123-1', '/blob-url/123-1/release-v1/asset']) {
  test('round front rejects forged legacy bearer without DO ' + path, async () => {
    const result = await ingress(path, 'v1.123.1.1800009000.' + 'A'.repeat(43), 100);
    assert.equal(result.entries, 0);
    assert.equal(result.response.status, 401);
  });
}
for (const path of ['/report/123-1', '/blob-url/123-1/release-v1/asset']) {
  test('round front verifies nonce MAC before DO ' + path, async () => {
    const result = await ingress(path, tokenV2().slice(0, -4) + 'AAAA');
    assert.equal(result.entries, 0);
    assert.equal(result.response.status, 401);
  });
  test('round front accepts valid stateless capability ' + path, async () => {
    const result = await ingress(path, tokenV2());
    assert.equal(result.entries, 1);
    assert.equal(result.response.status, 200);
  });
  test('round front rejects expired capability ' + path, async () => {
    const result = await ingress(path, tokenV2({ expiry: clock / 1000 - 1 }));
    assert.equal(result.entries, 0);
    assert.equal(result.response.status, 401);
  });
}
for (const changed of ['nonce', 'sha']) {
  test('round token includes immutable rental field ' + changed, async () => {
    const h = await setup();
    await h.call();
    const record = await h.storage.get('record:123-1');
    const token = await new RunTokens(env.RUN_TOKEN_SECRET).issue(record);
    assert.ok(token.startsWith('v2.'));
    assert.ok(token.includes('.' + record[changed] + '.'));
  });
}
for (const closed of ['stopped', 'closed', 'unknown']) {
  test('round stale authenticated flood never queues ' + closed, async () => {
    const h = await setup();
    await h.call();
    const r = await h.storage.get('record:123-1');
    const token = await new RunTokens(env.RUN_TOKEN_SECRET).issue(r);
    if (closed === 'closed') h.cloud.vastError = 500;
    await h.call('/stop/123-1');
    h.cloud.vastError = 0;
    if (closed === 'unknown') await h.storage.delete('record:123-1');
    const base = h.storage.get.bind(h.storage);
    let release, entered;
    const started = new Promise((resolve) => (entered = resolve));
    const gate = new Promise((resolve) => (release = resolve));
    h.storage.get = async (key) => {
      if (key === 'run:123-1') {
        entered();
        await gate;
      }
      return base(key);
    };
    const blocker = h.call('/status/123-1');
    await started;
    const calls = Array.from({ length: 40 }, () =>
      h.report({ seq: 1, kind: 'log', text: 'stale' }, { token }),
    );
    let result;
    try {
      result = await Promise.race([
        Promise.all(calls).then((values) => values.every((x) => x.status === 401)),
        new Promise((resolve) => setTimeout(() => resolve(false), 1000)),
      ]);
    } finally {
      release();
      await blocker;
      await Promise.all(calls);
    }
    assert.equal(result, true);
  });
}
for (const patch of [
  { iat: undefined },
  { iat: 1 },
  { iat: clock / 1000 + 61 },
  { iat: clock / 1000 - 3601 },
  { iat: clock / 1000, exp: clock / 1000 + 3601 },
]) {
  test('round OIDC rejects stale or unbounded issue time ' + JSON.stringify(patch), async () => {
    const verifier = new OidcVerifier(
      async () => Response.json({ keys: [jwk] }),
      () => clock,
    );
    await assert.rejects(verifier.verify(await jwt(patch), env.WORKER_URL));
  });
}
for (const path of ['/rent', '/availability?profile=check']) {
  test('round pull_request cannot admit or inspect market ' + path, async () => {
    const h = await setup();
    const response = await h.call(path, {
      method: path.startsWith('/availability') ? 'GET' : 'POST',
      claims: { event_name: 'pull_request' },
    });
    assert.equal(response.status, 401);
    assert.equal(h.cloud.creates, 0);
  });
}
// FALSIFIER H4/H5: labels block cleanup; transfer/export policy admits forbidden boxes.
for (const action of ['stop', 'cron']) {
  test('round create-owned ID survives label change ' + action, async () => {
    const h = await setup();
    await h.call();
    h.cloud.instances.get('900').label = 'changed-owner-label';
    if (action === 'stop') await h.call('/stop/123-1');
    else {
      h.time.value += 181 * 60000;
      await h.service.sweep();
    }
    assert.deepEqual(h.cloud.deletes, ['900']);
    assert.equal(await h.storage.get('active'), null);
    assert.ok(h.cloud.instances.has('888'));
  });
}
for (const field of ['inet_up_cost', 'inet_down_cost']) {
  test('round transfer price capped per GB ' + field, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180', MAX_PRICE_USD_HOUR: '1' });
    h.cloud.offer[field] = 0.1;
    await h.call();
    assert.equal(h.cloud.creates, 0);
  });
}
for (const field of ['dph_total', 'inet_down_cost']) {
  test('round changed contract price destroys new owned box ' + field, async () => {
    const h = await setup();
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      const response = await base(url, init);
      if (String(url).includes('/asks/')) {
        h.cloud.instances.get('900')[field] = 9;
      }
      return response;
    };
    h.restart();
    assert.notEqual((await h.call()).status, 200);
    assert.deepEqual(h.cloud.deletes, ['900']);
    assert.ok(h.cloud.instances.has('888'));
  });
}
for (const country of ['CU', 'IR', 'KP', 'SY']) {
  test('round denied country cannot rent ' + country, async () => {
    const h = await setup({ PREFERRED_COUNTRIES: JSON.stringify([country]) });
    h.cloud.offer.geolocation = 'fixture, ' + country;
    await h.call();
    assert.equal(h.cloud.creates, 0);
  });
}
for (const location of [undefined, 'malformed']) {
  test('round unknown country obeys owner config ' + String(location), async () => {
    const h = await setup({ ALLOW_UNKNOWN_COUNTRY: 'false' });
    h.cloud.offer.geolocation = location;
    await h.call();
    assert.equal(h.cloud.creates, 0);
  });
}
for (const prefixes of [undefined, '[]']) {
  test('round blob tag scope defaults denied ' + String(prefixes), async () => {
    const { mintBlob } = await import('../src/blobs.js');
    const h = await setup({
      BLOB_REPO: 'example/private',
      BLOB_GITHUB_TOKEN: 'mock-only',
      BLOB_ALLOWED_TAG_PREFIXES: prefixes,
    });
    let calls = 0;
    const fetcher = async () => {
      calls++;
      return new Response('{}');
    };
    await assert.rejects(
      mintBlob(
        fetcher,
        h.bindings,
        { get: async () => 0, save: async () => {}, put: async () => {} },
        { key: '123-1' },
        'release',
        'asset',
        clock,
      ),
    );
    assert.equal(calls, 0);
  });
}
for (const path of ['/rent', '/status/123-1'])
  test('round pull_request fails at front before ledger ' + path, async () => {
    let entries = 0;
    const gateway = createGateway({
      now: () => clock,
      fetcher: async () => Response.json({ keys: [jwk] }),
    });
    const bindings = {
      ...env,
      BROKER_PATH_PREFIX: 'g'.repeat(32),
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
    const result = await gateway.fetch(
      new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + path, {
        method: path === '/rent' ? 'POST' : 'GET',
        headers: { Authorization: 'Bearer ' + (await jwt({ event_name: 'pull_request' })) },
      }),
      bindings,
    );
    assert.equal(result.status, 401);
    assert.equal(entries, 0);
  });
