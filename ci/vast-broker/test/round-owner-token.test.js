import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha } from './helpers.js';
// FALSIFIER H9: client credentials become authority or the Worker token escapes.
for (const path of ['/rent', '/availability?profile=check'])
  test('round Worker GitHub credential wins over caller ' + path, async () => {
    const sentinel = 'worker-readonly-sentinel-' + 'q'.repeat(40);
    const h = await setup({ GITHUB_API_TOKEN: sentinel });
    const r = await h.call(path, {
      method: path === '/rent' ? 'POST' : 'GET',
      ghToken: 'caller-token',
    });
    assert.equal(r.status, 200);
    const calls = h.cloud.calls.filter((x) => x.url.startsWith('https://api.github.com/'));
    assert.ok(calls.length > 0);
    assert.ok(
      calls.every((x) => new Headers(x.headers).get('Authorization') === 'Bearer ' + sentinel),
    );
    assert.ok(!(await r.text()).includes(sentinel));
    assert.ok(!JSON.stringify([...h.storage.data]).includes(sentinel));
    const creation = h.cloud.calls.find((x) => x.url.includes('/asks/'));
    assert.ok(!JSON.stringify(creation?.body?.env ?? {}).includes(sentinel));
  });
for (const path of ['/rent', '/availability?profile=check'])
  test('round caller GitHub credential is not forwarded ' + path, async () => {
    const h = await setup();
    assert.equal(
      (
        await h.call(path, {
          method: path === '/rent' ? 'POST' : 'GET',
          ghToken: 'untrusted-caller-token',
        })
      ).status,
      200,
    );
    const calls = h.cloud.calls.filter((x) => x.url.startsWith('https://api.github.com/'));
    assert.ok(calls.every((x) => !new Headers(x.headers).has('Authorization')));
  });
for (const key of ['RUN_TOKEN_SECRET', 'OWNER_STOP_TOKEN', 'BROKER_PATH_PREFIX'])
  test('round owner secret format diagnostic ' + key, async () => {
    const invalid = 'bad-format+/=' + 'z'.repeat(32);
    const h = await setup({ [key]: invalid });
    const r = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal(r.status, 503);
    const text = await r.text();
    assert.ok(text.includes('CONFIG_SECRET_FORMAT:' + key));
    assert.ok(!text.includes(invalid));
  });
for (const field of ['dph_total', 'inet_up_cost'])
  test('round recovered create repricing is checked before rent response ' + field, async () => {
    const h = await setup();
    h.cloud.ambiguous = true;
    assert.equal((await h.call()).status, 503);
    const instance = [...h.cloud.instances.values()].find((x) => x.label === 'kf-ci-123-1');
    instance[field] = 100;
    const response = await h.call();
    assert.notEqual(response.status, 200);
    assert.ok(h.cloud.deletes.includes(String(instance.id)));
  });
for (const route of ['/status/123-1', '/logs/123-1?cursor=0'])
  test('round Worker GitHub secret redacted from box reflection ' + route, async () => {
    const sentinel = 'worker-read-sentinel-' + 'q'.repeat(40);
    const h = await setup({ GITHUB_API_TOKEN: sentinel });
    await h.call();
    assert.equal((await h.report({ seq: 1, kind: 'log', text: sentinel })).status, 200);
    const response = await h.call(route, { method: 'GET' });
    assert.equal(response.status, 200);
    assert.ok(!(await response.text()).includes(sentinel));
    assert.ok(!JSON.stringify([...h.storage.data]).includes(sentinel));
  });
for (const encoded of [false, true])
  test('round blob redirect cannot embed Worker credential ' + encoded, async () => {
    const { mintBlob } = await import('../src/blobs.js');
    const sentinel = 'worker/read+credential=';
    const env = {
      GITHUB_API_TOKEN: sentinel,
      BLOB_REPO: 'owner/store',
      BLOB_ALLOWED_TAG_PREFIXES: '["allowed-"]',
    };
    const ledger = { get: async () => 0, put: async () => {}, save: async () => {} };
    const fetcher = async (url) =>
      url.includes('/tags/')
        ? Response.json({ assets: [{ id: 1, name: 'asset', size: 1 }] })
        : new Response(null, {
            status: 302,
            headers: {
              Location:
                'https://release-assets.githubusercontent.com/asset?credential=' +
                (encoded ? encodeURIComponent(sentinel) : sentinel),
            },
          });
    await assert.rejects(mintBlob(fetcher, env, ledger, {}, 'allowed-v1', 'asset', 1800000000000));
  });
for (const path of ['/status/123-1', '/stop/123-1'])
  test('round changed create label emits a fixed ownership event ' + path, async () => {
    const h = await setup();
    await h.call();
    h.cloud.instances.get('900').label = 'untrusted-label-must-not-be-logged';
    const lines = [], original = console.log;
    console.log = line => lines.push(String(line));
    try { assert.equal((await h.call(path)).status,200); }
    finally { console.log = original; }
    assert.ok(lines.includes('{"event":"OWNERSHIP_MISMATCH"}'));
    assert.ok(!lines.join('').includes('untrusted-label-must-not-be-logged'));
  });
