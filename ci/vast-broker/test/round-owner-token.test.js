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
