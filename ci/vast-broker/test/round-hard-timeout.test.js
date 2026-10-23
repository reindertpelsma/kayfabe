import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// FALSIFIER: with MANUAL gone, nothing but heartbeat loss ends a live, chatty box; the cron
// hard deadline must end it while heartbeats are still fresh.
for (const restart of [false, true])
  test('round cron hard deadline ends a box with fresh heartbeats ' + restart, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    assert.equal((await h.call()).status, 200);
    let seq = 0;
    for (let minute = 0; minute < 180; minute += 9) {
      h.time.value = 1800000000000 + minute * 60000;
      assert.equal((await h.report({ seq: ++seq, kind: 'log', text: '' })).status, 200);
    }
    h.time.value = 1800000000000 + 181 * 60000;
    if (restart) h.restart();
    await h.service.sweep();
    const r = await h.storage.get('record:123-1');
    assert.equal(r.closed, 'HARD_TIMEOUT');
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
