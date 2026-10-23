import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// FALSIFIER: a Worker-authored status line bypasses the shared tail line/UTF-8 byte limits
// that apply to box logs.
for (const kind of ['lines', 'bytes'])
  test('V5 Worker-authored tail shares bounds ' + kind, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
    assert.equal((await h.call()).status, 200);
    const text =
      kind === 'lines' ? 'short\n'.repeat(200) : ('é'.repeat(495) + ' abcdefg\n').repeat(8);
    await h.report({ seq: 1, kind: 'log', text });
    for (const state of ['created', 'running', 'loading', 'running']) {
      const r = await h.storage.get('record:123-1');
      r.providerState = state === 'running' ? 'created' : 'running';
      await h.storage.put('record:123-1', r);
      h.time.value += 1000;
      assert.equal((await h.call('/status/123-1')).status, 200);
    }
    const tail = (await h.storage.get('record:123-1')).log;
    assert.ok(tail.trimEnd().split('\n').length <= 200);
    assert.ok(Buffer.byteLength(tail) <= 8192);
    assert.match(tail, /\[Worker\] provider=running/);
  });
