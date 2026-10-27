import test from 'node:test';
import assert from 'node:assert/strict';
// FALSIFIER V9: one undeletable overdue box aborts the sweep, hides the other failures, or is
// never retried on a later tick.
const now = 1800000000000;
const row = (id) => ({ id, label: 'kf-ci-' + id + '-1', start_date: (now - 999 * 60000) / 1000 });
async function harness(failing, fault) {
  const { createReaper } = await import('../../vast-reaper/worker.js');
  const records = new Map([1, 2, 3].map((id) => [String(id), row(id)]));
  const logs = [];
  const state = { failing: new Set(failing) };
  const worker = createReaper({
    now: () => now,
    report: (x) => logs.push(x),
    fetcher: async (input, init = {}) => {
      const url = new URL(input);
      if (url.pathname === '/api/v1/instances/')
        return Response.json({ success: true, instances: [...records.values()] });
      const id = /^\/api\/v0\/instances\/([1-9][0-9]*)\/$/.exec(url.pathname)[1];
      assert.equal(init.method, 'DELETE');
      if (state.failing.has(Number(id))) {
        if (fault === 'throw') throw Error('network');
        if (fault === 'http') return new Response('upstream', { status: 500 });
        return Response.json({ success: false });
      }
      records.delete(id);
      return Response.json({ success: true });
    },
  });
  const env = { REAPER_VAST_API_KEY: 'reaper-mock-key' };
  return { worker, env, records, logs, state };
}
for (const fault of ['ack', 'http', 'throw'])
  for (const failing of [[1], [2], [3], [1, 3]])
    test('round reaper continues past failed delete ' + fault + ' ' + failing, async () => {
      const h = await harness(failing, fault);
      await h.worker.sweep(h.env);
      assert.deepEqual(
        [...h.records.keys()].map(Number).sort(),
        [...failing].sort(),
        'every deletable overdue box is gone after one tick',
      );
      assert.deepEqual(h.logs, [
        {
          event: 'reaper_complete',
          deleted: 3 - failing.length,
          unconfirmed: 0,
          failed: failing.length,
        },
      ]);
      h.state.failing.clear();
      await h.worker.sweep(h.env);
      assert.equal(h.records.size, 0, 'the next tick retries the failed box');
      assert.deepEqual(h.logs.at(-1), {
        event: 'reaper_complete',
        deleted: failing.length,
        unconfirmed: 0,
        failed: 0,
      });
    });
