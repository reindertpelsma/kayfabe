import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce, callbackToken } from './helpers.js';
// FALSIFIER: a provider outage suppresses the required MANUAL cron status.
for (const retry of [false, true])
  test('V5 MANUAL cron line survives provider errors ' + retry, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    assert.equal((await h.call()).status, 200);
    await h.report({ seq: 1, kind: 'log', text: '' });
    const r = await h.storage.get('record:123-1');
    assert.equal((await h.call('/hold/123-1', { token: callbackToken(r) })).status, 200);
    h.cloud.vastError = 503;
    if (retry) {
      await h.service.sweep();
      h.restart();
    }
    const before = (await h.storage.get('record:123-1')).logChunks;
    h.time.value += 60000;
    await h.service.sweep();
    const current = await h.storage.get('record:123-1');
    let added = '';
    for (let i = before; i < current.logChunks; i++) added += await h.storage.get('log:123-1:' + i);
    assert.match(added, /\[Worker\] MANUAL/);
  });
