import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce, callbackToken, clock } from './helpers.js';
// FALSIFIER: a community host can hold an unfinished test, extend its own
// billing window, or suppress cancellation checks without owner confirmation.
for (const progress of [false, true])
  test('box hold requires a recorded result ' + progress, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    await h.call();
    if (progress) await h.report({ seq: 1, kind: 'log', text: '' });
    assert.equal(
      (await h.call('/hold/123-1', { token: callbackToken(await h.storage.get('record:123-1')) }))
        .status,
      401,
    );
    assert.notEqual((await h.storage.get('record:123-1')).phase, 'MANUAL');
  });
for (const ended of ['run', 'pr'])
  test('unconfirmed MANUAL retains cancellation and bounded hold ' + ended, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    await h.call();
    await h.report({ seq: 1, kind: 'result', result: { nonce, verdict: 'PASS' } });
    assert.equal(
      (await h.call('/hold/123-1', { token: callbackToken(await h.storage.get('record:123-1')) }))
        .status,
      200,
    );
    assert.equal((await h.storage.get('record:123-1')).manualDeadline, clock + 30 * 60000);
    if (ended === 'run') h.cloud.run.status = 'completed';
    else h.cloud.prs[0].state = 'closed';
    await h.service.sweep();
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
