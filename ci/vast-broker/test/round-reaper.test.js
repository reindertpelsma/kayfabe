import test from 'node:test';
import assert from 'node:assert/strict';
const now = 1800000000000;
const epoch = (minutes) => (now - minutes * 60000) / 1000;
const row = (id, minutes, label = 'kf-ci-' + id + '-1') => ({
  id,
  label,
  start_date: epoch(minutes),
});
async function harness(rows, mutate) {
  const { createReaper } = await import('../../vast-reaper/worker.js');
  const records = new Map(rows.map((x) => [String(x.id), { ...x }]));
  const calls = [],
    logs = [];
  let lists = 0;
  const worker = createReaper({
    now: () => now,
    report: (x) => logs.push(x),
    fetcher: async (input, init = {}) => {
      const url = new URL(input);
      assert.equal(url.origin, 'https://console.vast.ai');
      assert.equal(new Headers(init.headers).get('Authorization'), 'Bearer reaper-mock-key');
      assert.equal(init.redirect, 'error');
      calls.push({ path: url.pathname, method: init.method ?? 'GET' });
      if (url.pathname === '/api/v1/instances/') {
        lists++;
        if (mutate) mutate(records, lists);
        return Response.json({ success: true, instances: [...records.values()] });
      }
      const match = /^\/api\/v0\/instances\/([1-9][0-9]*)\/$/.exec(url.pathname);
      assert.ok(match);
      assert.equal(init.method, 'DELETE');
      records.delete(match[1]);
      return Response.json({ success: true });
    },
  });
  const env = {
    REAPER_VAST_API_KEY: 'reaper-mock-key',
    REAPER_HARD_MINUTES: '180',
    REAPER_MARGIN_MINUTES: '15',
  };
  return {
    worker,
    env,
    records,
    calls,
    logs,
    deletes: () => calls.filter((x) => x.method === 'DELETE').map((x) => x.path),
  };
}
// FALSIFIER H3: a young/foreign/unknown-age box is deleted, or main broker uptime is needed.
for (const age of [194, 195, 196]) {
  test('round reaper deletes only strictly overdue owned namespace ' + age, async () => {
    const h = await harness([
      row(1, age),
      row(2, 999, 'owner-dev'),
      row(3, 999, 'kf-ci-unrecorded'),
    ]);
    await h.worker.sweep(h.env);
    assert.equal(h.records.has('1'), age <= 195);
    assert.ok(h.records.has('2') && h.records.has('3'));
    assert.equal(h.deletes().length, age > 195 ? 1 : 0);
  });
}
for (const date of [undefined, NaN, 0, epoch(-1), 'old']) {
  test('round reaper rejects unknown or future provider age ' + String(date), async () => {
    const h = await harness([{ ...row(1, 999), start_date: date }]);
    await h.worker.sweep(h.env);
    assert.equal(h.deletes().length, 0);
  });
}
for (const changed of ['label', 'start_date']) {
  test('round reaper revalidates snapshot before deletion ' + changed, async () => {
    const h = await harness([row(1, 999)], (records, n) => {
      if (n === 2) records.get('1')[changed] = changed === 'label' ? 'owner-dev' : epoch(1);
    });
    await h.worker.sweep(h.env);
    assert.equal(h.deletes().length, 0);
  });
}
for (const path of ['/stop-all', '/rent']) {
  test('round reaper exposes no public control path ' + path, async () => {
    const h = await harness([row(1, 999)]);
    const result = await h.worker.fetch(new Request('https://reaper.example' + path), h.env);
    assert.equal(result.status, 404);
    assert.equal(h.calls.length, 0);
  });
}
for (const why of ['absent', 'unavailable']) {
  test('round reaper has no broker or ledger dependency ' + why, async () => {
    const h = await harness([row(1, 999)]);
    await h.worker.sweep(h.env);
    assert.equal(h.deletes().length, 1);
    assert.ok(!JSON.stringify(h.logs).includes('reaper-mock-key'));
  });
}
