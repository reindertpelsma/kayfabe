import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
// Final review 3b: checks whose removal no plain test noticed. Each test below fails when its
// named check is deleted (hand-removal re-run 2026-10-24, see the edits report).
const instance = (h) => h.cloud.instances.get('900');

// S07: the post-create contract recheck enforces MAX_INET_USD_PER_GB on the instance row. The
// 0.0051 USD/GB below keeps the effective hourly price under the cap, so only S07 can refuse.
for (const field of ['inet_down_cost', 'inet_up_cost']) {
  test('S07 a contract repriced above the per-GB cap is destroyed on update ' + field, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    instance(h)[field] = 0.0051;
    assert.equal((await h.call('/status/123-1')).status, 503);
    const record = await h.storage.get('record:123-1');
    assert.equal(record.closed, 'CONTRACT_PRICE');
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
  test('S07 a create whose row exceeds the per-GB cap is destroyed at once ' + field, async () => {
    const h = await setup();
    const fetch = h.cloud.fetch;
    h.cloud.fetch = async (input, init) => {
      const response = await fetch(input, init);
      if (/\/api\/v0\/asks\//.test(String(input))) instance(h)[field] = 0.0051;
      return response;
    };
    h.restart();
    assert.equal((await h.call()).status, 503);
    assert.equal((await h.storage.get('record:123-1')).closed, 'CONTRACT_PRICE');
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
}

// S08: a contract repriced upwards (still under the hourly cap) is re-budgeted against the daily
// cap and the prepaid credit; a repricing that no longer fits is destroyed, not kept running.
const reprice = (h, total) => {
  Object.assign(instance(h), { dph_total: total, dph_base: total - (0.12 * 120) / 720 });
};
for (const [label, overrides, credit] of [
  ['daily cap', { DAILY_SPEND_USD: '0.1' }, 20],
  ['credit reserve', {}, 8.1],
])
  test('S08 a repriced contract over the ' + label + ' is destroyed', async () => {
    const h = await setup(overrides);
    h.cloud.credit = credit;
    assert.equal((await h.call()).status, 200);
    reprice(h, 0.45);
    assert.equal((await h.call('/status/123-1')).status, 409);
    assert.equal((await h.storage.get('record:123-1')).closed, 'CONTRACT_BUDGET');
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
test('S08 a repriced contract that still fits is re-budgeted and kept', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  reprice(h, 0.45);
  assert.equal((await h.call('/status/123-1')).status, 200);
  const record = await h.storage.get('record:123-1');
  assert.equal(record.closed, null);
  assert.ok(Math.abs(record.computeStoragePrice - 0.45) < 1e-9);
  assert.ok(Math.abs((await h.storage.get('cost:123-1')).price - 0.45) < 1e-9);
  assert.deepEqual(h.cloud.deletes, []);
});

// R04-R06: the reaper's second snapshot, complete-list and margin-floor checks.
const now = 1800000000000;
const old = (id, label = 'kf-ci-' + id + '-1') => ({
  id,
  label,
  start_date: (now - 999 * 60000) / 1000,
});
async function reaper(listing, env = {}) {
  const { createReaper } = await import('../../vast-reaper/worker.js');
  const calls = [],
    logs = [];
  let lists = 0;
  const worker = createReaper({
    now: () => now,
    report: (x) => logs.push(x),
    fetcher: async (input, init = {}) => {
      const url = new URL(input);
      calls.push((init.method ?? 'GET') + ' ' + url.pathname);
      if (url.pathname === '/api/v1/instances/') return Response.json(listing(++lists));
      return Response.json({ success: true });
    },
  });
  await worker.sweep({
    REAPER_VAST_API_KEY: 'reaper-mock-key',
    REAPER_HARD_MINUTES: '180',
    REAPER_MARGIN_MINUTES: '15',
    ...env,
  });
  return { calls, logs, deletes: calls.filter((x) => x.startsWith('DELETE ')) };
}
const listed = (rows, extra = {}) => ({ success: true, instances: rows, ...extra });
test('R04 the reaper takes a second snapshot before any DELETE', async () => {
  const r = await reaper((n) => listed(n === 3 ? [] : [old(1)]));
  assert.deepEqual(r.calls, [
    'GET /api/v1/instances/',
    'GET /api/v1/instances/',
    'DELETE /api/v0/instances/1/',
    'GET /api/v1/instances/',
  ]);
});
for (const [label, second] of [
  ['gone', []],
  ['replaced by another id', [old(2)]],
  ['relabelled', [old(1, 'owner-dev')]],
])
  test('R04 a row ' + label + ' in the second snapshot is not deleted', async () => {
    const r = await reaper((n) => listed(n === 1 ? [old(1)] : second));
    assert.equal(r.deletes.includes('DELETE /api/v0/instances/1/'), false);
  });
for (const [label, page] of [
  ['next_token', listed([old(1)], { next_token: 'more' })],
  ['next', listed([old(1)], { next: 'more' })],
  ['100 rows', listed(Array.from({ length: 100 }, (_, i) => old(i + 1)))],
  ['duplicate id', listed([old(1), old(1)])],
  ['invalid id', listed([old(1), { ...old(2), id: 'x' }])],
  ['not success', { success: false, instances: [old(1)] }],
])
  test('R05 an incomplete or invalid list aborts the sweep: ' + label, async () => {
    const r = await reaper(() => page);
    assert.deepEqual(r.deletes, []);
    assert.deepEqual(r.logs, [
      {
        event: 'reaper_unavailable',
        reason: ['duplicate id', 'invalid id'].includes(label)
          ? 'malformed_list'
          : 'incomplete_list',
      },
    ]);
  });
test('R05 a complete 99-row list is accepted', async () => {
  const rows = Array.from({ length: 99 }, (_, i) => old(i + 1));
  const r = await reaper((n) => listed(n <= 2 ? rows : []));
  assert.equal(r.deletes.length, 20);
});
for (const margin of ['14', '1', '0'])
  test('R06 a margin below 15 minutes refuses the whole sweep ' + margin, async () => {
    const r = await reaper(() => listed([old(1)]), { REAPER_MARGIN_MINUTES: margin });
    assert.deepEqual(r.calls, []);
    assert.deepEqual(r.logs, [{ event: 'reaper_unavailable', reason: 'configuration' }]);
  });
test('R06 the 15-minute margin floor itself is accepted', async () => {
  const r = await reaper((n) => listed(n === 3 ? [] : [old(1)]), { REAPER_MARGIN_MINUTES: '15' });
  assert.deepEqual(r.deletes, ['DELETE /api/v0/instances/1/']);
});
