import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// FALSIFIER: the failed intent's SHA cache denies a later workflow retry even
// after the provider proved no contract was created.
for (const status of [200, 400])
  test('definitive refusal permits the next same-SHA workflow attempt ' + status, async () => {
    const h = await setup();
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) =>
      String(url).includes('/asks/')
        ? Response.json({ success: false }, { status })
        : base(url, init);
    h.restart();
    assert.equal((await h.call()).status, 503);
    h.cloud.fetch = base;
    h.cloud.run.run_attempt = 2;
    h.restart();
    assert.equal((await h.call('/rent', { claims: { run_attempt: '2' } })).status, 200);
    assert.equal(h.cloud.creates, 1);
  });
