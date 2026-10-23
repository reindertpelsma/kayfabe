import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, callbackToken } from './helpers.js';
// FALSIFIER: a stale kill or a saturated own record holds the cleanup/control queue.
async function heldStatus(h) {
  let entered, release;
  const ready = new Promise((resolve) => {
    entered = resolve;
  });
  const held = new Promise((resolve) => {
    release = resolve;
  });
  const get = h.storage.get.bind(h.storage);
  let armed = true;
  h.storage.get = async (key) => {
    if (armed && key === 'run:123-1') {
      armed = false;
      entered();
      await held;
    }
    return get(key);
  };
  const pending = h.call('/status/123-1');
  await ready;
  return { pending, release };
}
for (const result of [false, true])
  test('round ended kill receipt skips busy queue ' + result, async () => {
    const h = await setup();
    await h.call();
    const r = await h.storage.get('record:123-1');
    const ended = {
      ...r,
      key: '124-1',
      run: '124',
      label: 'kf-ci-124-1',
      ended: h.time.value,
      closed: 'STOPPED',
      boxKilled: true,
      ...(result ? { resultAt: h.time.value } : {}),
    };
    await h.storage.put('record:124-1', ended);
    const held = await heldStatus(h);
    const response = h.call('/kill/124-1', { token: callbackToken(ended) });
    let timeout;
    try {
      const first = await Promise.race([
        response,
        new Promise((resolve) => {
          timeout = setTimeout(() => resolve(null), 500);
        }),
      ]);
      assert.ok(first, 'ended kill waited on the queue');
      assert.equal(first.status, 200);
      assert.deepEqual(await first.json(), { state: 'NONE' });
    } finally {
      clearTimeout(timeout);
      held.release();
      await held.pending;
      await response;
    }
  });
for (const route of ['/status/123-1', '/logs/123-1?cursor=0'])
  test('round own record queue is bounded ' + route, async () => {
    const h = await setup();
    await h.call();
    const held = await heldStatus(h);
    const pending = Array.from({ length: 5 }, () => h.call(route, { method: 'GET' }));
    let timeout;
    try {
      const first = await Promise.race([
        ...pending,
        new Promise((resolve) => {
          timeout = setTimeout(() => resolve(null), 500);
        }),
      ]);
      assert.ok(first, 'record queue accepted every request');
      assert.equal(first.status, 409);
      assert.equal((await first.json()).state, 'BUSY');
    } finally {
      clearTimeout(timeout);
      held.release();
      await held.pending;
      await Promise.all(pending);
    }
  });
for (const grace of ['5', '10'])
  test('round callback lifetime follows hard limit ' + grace, async () => {
    const h = await setup({ FAIL_GRACE_MINUTES: grace });
    await h.call();
    const r = await h.storage.get('record:123-1');
    assert.ok(r.reportExpiry * 1000 <= r.hardDeadline + 11 * 60000);
    assert.ok(r.reportExpiry * 1000 >= r.hardDeadline);
  });
for (const field of ['inet_up_cost', 'inet_down_cost'])
  test('round provider query carries per GB ceiling ' + field, async () => {
    const h = await setup();
    await h.call();
    const request = h.cloud.calls.find((x) => x.url.endsWith('/bundles/')).body;
    assert.equal(request[field].lte, 0.005);
  });
for (const fallback of [undefined, 'deprecated-asset-token'])
  test('round primary GitHub token authorizes asset metadata ' + String(fallback), async () => {
    const { mintBlob } = await import('../src/blobs.js');
    const sentinel = 'worker-readonly-sentinel';
    const headers = [];
    const fetcher = async (url, init) => {
      headers.push(new Headers(init.headers));
      return url.includes('/tags/')
        ? Response.json({ assets: [{ id: 1, name: 'asset', size: 2 }] })
        : new Response(null, {
            status: 302,
            headers: {
              Location: 'https://release-assets.githubusercontent.com/asset?sig=short-lived',
            },
          });
    };
    const env = {
      GITHUB_API_TOKEN: sentinel,
      BLOB_GITHUB_TOKEN: fallback,
      BLOB_REPO: 'owner/store',
      BLOB_ALLOWED_TAG_PREFIXES: '["allowed-"]',
    };
    const r = await mintBlob(
      fetcher,
      env,
      { get: async () => 0, save: async () => {}, put: async () => {} },
      {},
      'allowed-v1',
      'asset',
      1800000000000,
    );
    assert.equal(headers.length, 2);
    assert.ok(headers.every((x) => x.get('Authorization') === 'Bearer ' + sentinel));
    assert.ok(!JSON.stringify(r).includes(sentinel));
  });
