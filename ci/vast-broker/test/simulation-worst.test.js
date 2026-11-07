import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { matrixTest as test, capablePayload } from './release-fixture.js';
import { setup, sha } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { configuration, gpuClass } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
const vars = {
  MAX_CONCURRENT_RENTALS: '9',
  RELEASE_ENABLED: 'true',
  HARD_RUNTIME_MINUTES: '300',
  BOOT_LIMIT_MINUTES: '75',
  ROLLING_SPEND_USD: '200',
  DAILY_SPEND_USD: '40',
  ACCOUNT_DAILY_USD: '100',
  RELEASE_DAILY_RENTALS: '64',
};
const market = JSON.parse(
  readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
).offers;
async function fleet({ changeMarket = (m) => m, overrides = {}, fault, tier = 'small' } = {}) {
  const h = capablePayload(await setup({ ...vars, ...overrides }));
  const cfg = applyProfile(configuration(h.bindings), tier === 'large' ? 'release' : 'check', tier);
  const rows = changeMarket(structuredClone(market)).map((o) => ({
    ...o,
    dph_total: o.dph_base + (o.storage_cost * cfg.diskGb) / 720,
  }));
  const [first, ...rest] = rows;
  h.cloud.offer = first;
  h.cloud.price = first.dph_total;
  h.cloud.extraOffers = rest.map((o) => ({ ...o, price: o.dph_total }));
  h.cloud.credit = 500;
  h.cloud.contracts = Array.from({ length: 80 }, (_, i) => 900 + i);
  const original = h.cloud.fetch,
    owned = new Set(),
    outsideDestroyed = new Set(),
    state = { fault, armed: true };
  h.cloud.fetch = async (input, init = {}) => {
    const u = new URL(input),
      method = init.method ?? 'GET';
    if (u.hostname === 'console.vast.ai' && method === 'DELETE')
      assert.ok(
        owned.has(u.pathname.split('/').filter(Boolean).at(-1)),
        'exact-id teardown must be owned',
      );
    if (state.armed && state.fault) {
      const intercepted = await state.fault({ h, u, method, init, owned });
      if (intercepted) return intercepted;
    }
    const response = await original(input, init);
    if (u.hostname === 'console.vast.ai' && /\/asks\//.test(u.pathname)) {
      const created = h.cloud.created.at(-1);
      owned.add(String(created.contract));
      assert.ok(
        [...owned].filter((id) => h.cloud.instances.has(id)).length <= (tier === 'large' ? 8 : 1),
        'fleet concurrency',
      );
      const records = [...(await h.storage.list({ prefix: 'record:' })).values()];
      const estimate = records.reduce(
        (sum, r) =>
          sum +
          (r.closed === 'GONE'
            ? 0
            : ((r.computeStoragePrice ?? r.price) *
                Math.max(0, (r.ended ?? r.started + (r.billingTtl ?? r.ttl)) - r.started)) /
                3600000 +
              (r.bandwidthCost ?? 0)),
        0,
      );
      assert.ok(estimate <= 30 + 1e-6, 'fleet estimated cap after create');
    }
    return response;
  };
  h.restart();
  return { h, owned, outsideDestroyed, state, tier };
}
const records = async (h) => [...(await h.storage.list({ prefix: 'record:' })).values()];
async function start(f) {
  const b = await f.h.service.adminStart({ commit_sha: sha, tier: f.tier });
  f.batch = b.batch;
  return b;
}
async function stopAndCheck(f, expected = 'STOPPED') {
  const { h, owned, state } = f;
  state.armed = false;
  if (f.batch) await h.service.adminStopBatch(f.batch);
  for (let i = 0; i < 4 && (await records(h)).some((r) => r.ended === null); i++) {
    h.time.value += 360 * 60000;
    await h.service.sweep();
  }
  assert.equal(
    (await new Ledger(h.storage).openRecords()).length,
    0,
    'no ledger liability after release ends',
  );
  for (const id of owned) assert.equal(h.cloud.instances.has(id), false, 'no rental leak: ' + id);
  assert.equal(h.cloud.instances.size, 2, 'foreign instances remain');
  assert.ok(
    h.cloud.deletes.every((id) => owned.has(id)),
    'never destroy a guessed or foreign id',
  );
  const rs = await records(h);
  assert.ok(
    rs.every((r) => r.verdict !== 'PASS'),
    'failed or absent box never becomes PASS',
  );
  assert.ok(
    rs.every((r) => Number.isFinite(r.ended)),
    'every rental settles',
  );
  if (f.batch) assert.equal((await h.storage.get('batch:' + f.batch)).state, expected);
}
async function heartbeat(f) {
  const r = (await records(f.h)).find((r) => r.id);
  assert.equal((await f.h.report({ seq: 1, kind: 'log', text: '' }, { run: r.key })).status, 200);
  return await f.h.storage.get('record:' + r.key);
}
// FALSIFIER: one provider status word certifies absence, a fault erases liability,
// a retry changes owned ids, or a finished batch leaves any paid instance live.
test('simulation worst: running box offline once (#278)', async () => {
  const f = await fleet();
  await start(f);
  const r = await heartbeat(f);
  f.h.cloud.instances.get(r.id).actual_status = 'offline';
  f.h.time.value += 1000;
  await f.h.service.sweep();
  const held = await f.h.storage.get('record:' + r.key);
  assert.equal(held.ended, null);
  assert.equal(held.closed, null);
  assert.ok(!f.h.cloud.deletes.includes(r.id));
  f.h.cloud.instances.get(r.id).actual_status = 'running';
  await f.h.service.sweep();
  await stopAndCheck(f);
});
test('simulation worst: create succeeds but remains created for twelve minutes', async () => {
  const f = await fleet();
  await start(f);
  const r = (await records(f.h))[0];
  f.h.cloud.instances.get(r.id).actual_status = 'created';
  f.h.time.value += 12 * 60000;
  await f.h.service.sweep();
  assert.equal((await f.h.storage.get('record:' + r.key)).ended, null);
  f.h.time.value = r.bootDeadline + 1;
  await f.h.service.sweep();
  assert.equal((await f.h.storage.get('record:' + r.key)).closed, 'BOOT_TIMEOUT');
  await stopAndCheck(f, 'DONE');
});
test('simulation worst: offer disappears between search and create', async () => {
  const f = await fleet({
    fault: ({ u }) =>
      /\/asks\//.test(u.pathname) ? Response.json({ success: false }, { status: 404 }) : null,
  });
  await start(f);
  assert.equal(f.h.cloud.creates, 0);
  const rs = await records(f.h);
  assert.equal(rs.length, 1);
  assert.equal(rs[0].closed, 'GONE');
  assert.ok(Number.isFinite(rs[0].ended));
  await stopAndCheck(f);
});
test('simulation worst: no VM-capable offer rents nothing', async () => {
  const f = await fleet({
    tier: 'large',
    changeMarket: (m) => m.map((o) => ({ ...o, vms_enabled: false })),
  });
  await assert.rejects(
    () => start(f),
    (e) => e.reason === 'PLAN_REFUSED',
  );
  assert.equal(f.h.cloud.creates, 0);
  await stopAndCheck(f);
});
test('simulation worst: a die with no offer is not offered, not FAIL', async () => {
  const cfg = applyProfile(
    configuration({ ...vars, ...(await setup()).bindings }),
    'release',
    'large',
  );
  const f = await fleet({
    tier: 'large',
    changeMarket: (m) => m.filter((o) => gpuClass(o, cfg) !== 'TU116'),
  });
  const { plan } = await f.h.service.adminPlan({ commit_sha: sha, tier: 'large' });
  assert.ok(
    plan.families
      .flatMap((f) => f.missing)
      .some((m) => m.die === 'TU116' && m.reason === 'NO_OFFER'),
  );
  assert.equal(f.h.cloud.creates, 0);
  await stopAndCheck(f);
});
test('simulation worst: ssh never reachable produces boot timeout', async () => {
  const f = await fleet();
  await start(f);
  f.h.cloud.logReady = false;
  const r = (await records(f.h))[0];
  f.h.time.value = r.bootDeadline + 1;
  await f.h.service.sweep();
  const ended = await f.h.storage.get('record:' + r.key);
  assert.equal(ended.closed, 'BOOT_TIMEOUT');
  assert.equal(ended.verdict, 'UNAVAILABLE');
  await stopAndCheck(f, 'DONE');
});
test('simulation worst: host reboots after its first heartbeat', async () => {
  const f = await fleet();
  await start(f);
  const r = await heartbeat(f);
  f.h.cloud.instances.get(r.id).actual_status = 'created';
  f.h.time.value = r.hardDeadline + 1;
  await f.h.service.sweep();
  assert.equal((await f.h.storage.get('record:' + r.key)).closed, 'HARD_TIMEOUT');
  await stopAndCheck(f, 'DONE');
});
test('simulation worst: third party destroys one exact instance', async () => {
  const f = await fleet();
  await start(f);
  const r = (await records(f.h))[0];
  f.h.cloud.instances.delete(r.id);
  f.outsideDestroyed.add(r.id);
  await f.h.service.sweep();
  assert.ok(Number.isFinite((await f.h.storage.get('record:' + r.key)).ended));
  assert.ok(!f.h.cloud.deletes.includes('777'));
  await stopAndCheck(f, 'DONE');
});
for (const fault of ['500', 'timeout', '429'])
  test(
    'simulation worst: API ' + fault + (fault === '429' ? ' with Retry-After' : ''),
    async () => {
      const f = await fleet();
      await start(f);
      const before = f.h.cloud.creates;
      f.state.fault = ({ u, method }) => {
        if (u.hostname !== 'console.vast.ai' || method !== 'GET') return null;
        if (fault === 'timeout') throw new DOMException('synthetic timeout', 'TimeoutError');
        return new Response('', {
          status: Number(fault),
          headers: fault === '429' ? { 'Retry-After': '120' } : {},
        });
      };
      await f.h.service.sweep();
      assert.equal(f.h.cloud.creates, before);
      assert.ok(
        (await records(f.h)).every((r) => r.ended === null),
        'failed API read retains liability',
      );
      await stopAndCheck(f);
    },
  );
for (const reason of ['account-limit', 'credit', 'daily-cap'])
  test('simulation worst: five of eight created then ' + reason, async () => {
    const f = await fleet({
      tier: 'large',
      fault: async ({ h, u }) => {
        if (h.cloud.creates < 5) return null;
        if (reason === 'account-limit' && /\/asks\//.test(u.pathname))
          return Response.json({ success: false, error: 'account limit' }, { status: 400 });
        if (reason === 'credit') h.cloud.credit = 0;
        if (reason === 'daily-cap' && !(await h.storage.get('cost:history-0'))) {
          for (let i = 0; i < 16; i++)
            await h.storage.put('cost:history-' + i, {
              attemptKey: 'history-' + i,
              attemptStarted: h.time.value - 3600000,
              profile: 'check',
              budgetClass: 'release',
              started: h.time.value - 3600000,
              ended: h.time.value,
              created: true,
              price: 0.5,
              bandwidthCost: 2.6,
              ttl: 3600000,
            });
        }
        return null;
      },
    });
    await start(f);
    assert.equal(f.h.cloud.creates, 5);
    assert.equal((await new Ledger(f.h.storage).openRecords()).length, 5);
    await stopAndCheck(f);
  });
test('simulation worst: cron after restart retains persisted ownership', async () => {
  const f = await fleet({ tier: 'large' });
  await start(f);
  const ids = [...f.owned];
  f.h.restart();
  await f.h.service.sweep();
  assert.equal(f.h.cloud.creates, 8);
  assert.deepEqual([...f.owned], ids);
  await stopAndCheck(f);
});
test('simulation worst: same machine has two offer ids', async () => {
  const f = await fleet({
    tier: 'large',
    changeMarket: (m) => [
      ...m,
      {
        ...m[0],
        id: 99001,
        dph_base: m[0].dph_base / 2,
        dph_total: m[0].dph_base / 2 + (m[0].storage_cost * 300) / 720,
      },
    ],
  });
  await start(f);
  const ids = (await records(f.h)).map((r) => r.machineId);
  assert.equal(new Set(ids).size, ids.length);
  await stopAndCheck(f);
});
test('simulation worst: duplicate and late report posts', async () => {
  const f = await fleet();
  await start(f);
  const r = await heartbeat(f);
  const body = { seq: 1, kind: 'log', text: '' };
  assert.equal((await f.h.report(body, { run: r.key })).status, 200);
  assert.equal((await f.h.storage.get('record:' + r.key)).lastSeq, 1);
  await stopAndCheck(f);
  assert.equal((await f.h.report({ seq: 2, kind: 'log', text: '' }, { run: r.key })).status, 401);
  assert.equal((await f.h.storage.get('record:' + r.key)).lastSeq, 1);
});
test('simulation worst: clock moves backwards then reaches the hard deadline', async () => {
  const f = await fleet();
  await start(f);
  const r = (await records(f.h))[0];
  f.h.time.value -= 60000;
  await f.h.service.sweep();
  assert.equal((await f.h.storage.get('record:' + r.key)).ended, null);
  assert.equal(f.h.cloud.deletes.length, 0);
  f.h.time.value = r.hardDeadline + 1;
  await f.h.service.sweep();
  assert.equal((await f.h.storage.get('record:' + r.key)).closed, 'HARD_TIMEOUT');
  await stopAndCheck(f, 'DONE');
});
