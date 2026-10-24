import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { parseVars } from '../deploy.mjs';
import { setup, env, nonce } from './helpers.js';
// Owner ruling 2026-10-24 item 8: concurrent rentals. A set of active runs replaces the single
// `active` slot; MAX_CONCURRENT_RENTALS (default 1, max 4) caps the account, and
// MAX_CONCURRENT_CHECK / MAX_CONCURRENT_SWEEP may lower it per profile.
// FALSIFIER: the cap admits one run too many or one too few; a second box dodges the daily,
// rolling or credit-reserve caps; one run's failure, close or stop touches another; stop-all,
// the cron or the reaper miss one of many runs; a push supersedes another branch's run or a
// newer run of its own branch; two runs take the same machine; the rotation state moves on a
// refused rent or lands on a machine another run holds.
const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
const sweepClaims = (sha) => ({
  event_name: 'workflow_dispatch',
  sha,
  ref: 'refs/heads/master',
  workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master',
});
// One GitHub run per id: its own branch and head commit (check) or master (sweep).
function runAs(h, n, { branch = 'branch-' + n, sha = hex(n), profile = 'check' } = {}) {
  const sweep = profile === 'sweep';
  h.cloud.runs.set(String(n), {
    ...h.cloud.run,
    id: n,
    head_sha: sha,
    head_branch: sweep ? 'master' : branch,
    event: sweep ? 'workflow_dispatch' : 'pull_request_target',
    pull_requests: [],
  });
  return { run_id: String(n), ...(sweep ? sweepClaims(sha) : {}) };
}
async function rentAs(h, n, { preferences, profile = 'check', ...run } = {}) {
  const claims = runAs(h, n, { profile, ...run });
  const response = await h.call('/rent', {
    claims,
    body: {
      runner_nonce: 'f'.repeat(64),
      tested_sha: h.cloud.runs.get(String(n)).head_sha,
      profile,
      ...(preferences ? { preferences } : {}),
    },
  });
  return { status: response.status, data: await response.json() };
}
const statusOf = async (h, n, profile = 'check') =>
  (
    await h.call('/status/' + n + '-1', {
      claims: {
        run_id: String(n),
        ...(profile === 'sweep' ? sweepClaims(h.cloud.runs.get(String(n)).head_sha) : {}),
      },
    })
  ).json();
const record = (h, n) => h.storage.get('record:' + n + '-1');
// The market: AD104 (base offer, machine 100001), GA106, GA102, AD102 and a second AD104.
const market = (h) => {
  h.cloud.offer.host_id = 3001;
  const at = (id, gpu_name, compute_cap, machine_id) => ({
    ...h.cloud.offer,
    id,
    gpu_name,
    compute_cap,
    machine_id,
    host_id: 3000 + id,
  });
  h.cloud.extraOffers = [
    at(11, 'RTX_3060', 860, 200011),
    at(12, 'RTX_3080', 860, 200012),
    at(13, 'RTX_4090', 890, 200013),
    at(14, 'RTX_4070', 890, 200014),
  ];
  h.cloud.contracts = [901, 902, 903, 904, 905, 906];
};
const named = (fn, name) =>
  assert.throws(fn, (e) => e.state === 'CONFIG_VAR:' + name && e.status === 503, name);

test('item 8: the cap defaults to 1 and is validated (1 to 4, per profile at most the cap)', () => {
  assert.equal(DEFAULTS.maxConcurrentRentals, 1);
  const cfg = configuration(env);
  assert.equal(cfg.maxConcurrent, 1);
  assert.deepEqual(cfg.maxConcurrentProfile, { check: 1, sweep: 1 });
  const four = configuration({ ...env, MAX_CONCURRENT_RENTALS: '4', MAX_CONCURRENT_SWEEP: '2' });
  assert.equal(four.maxConcurrent, 4);
  assert.deepEqual(four.maxConcurrentProfile, { check: 4, sweep: 2 });
  for (const name of ['MAX_CONCURRENT_RENTALS', 'MAX_CONCURRENT_CHECK', 'MAX_CONCURRENT_SWEEP'])
    for (const bad of ['0', '5', '1.5', 'x', '', ' 2', '02', '-1'])
      named(() => configuration({ ...env, MAX_CONCURRENT_RENTALS: '4', [name]: bad }), name);
  named(() => configuration({ ...env, MAX_CONCURRENT_CHECK: '2' }), 'MAX_CONCURRENT_CHECK');
  named(
    () => configuration({ ...env, MAX_CONCURRENT_RENTALS: '2', MAX_CONCURRENT_SWEEP: '3' }),
    'MAX_CONCURRENT_SWEEP',
  );
});
test('item 8: the deploy helper takes the three concurrency vars', () => {
  const vars = parseVars(
    'OWNER_GITHUB_ID=229738317\nMAX_CONCURRENT_RENTALS=4\nMAX_CONCURRENT_CHECK=4\n' +
      'MAX_CONCURRENT_SWEEP=1\n',
  );
  assert.deepEqual(
    [...vars.keys()],
    ['OWNER_GITHUB_ID', 'MAX_CONCURRENT_RENTALS', 'MAX_CONCURRENT_CHECK', 'MAX_CONCURRENT_SWEEP'],
  );
});
test('item 8a: at the default cap a second run on another branch gets 409 BUSY', async () => {
  const h = await setup();
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  const second = await rentAs(h, 202);
  assert.equal(second.status, 409);
  assert.equal(second.data.state, 'BUSY');
  assert.equal(h.cloud.creates, 1);
  assert.equal(await record(h, 202), undefined);
});
test('item 8a: raising the cap to 2 admits exactly two', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  assert.equal((await rentAs(h, 202)).status, 200);
  const third = await rentAs(h, 203);
  assert.deepEqual([third.status, third.data.state], [409, 'BUSY']);
  assert.equal(h.cloud.creates, 2);
  assert.deepEqual(
    h.cloud.created.map((x) => x.label),
    ['kf-ci-201-1', 'kf-ci-202-1'],
  );
});
test('item 8a: a per-profile cap lowers the account cap for that profile only', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2', MAX_CONCURRENT_CHECK: '1' });
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  const check = await rentAs(h, 202);
  assert.deepEqual([check.status, check.data.state], [409, 'BUSY']);
  assert.equal((await rentAs(h, 203, { profile: 'sweep', preferences: ['GA102'] })).status, 200);
  assert.equal(h.cloud.creates, 2);
});
test('item 8a: the account cap holds across profiles', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  assert.equal((await rentAs(h, 202)).status, 200);
  const sweep = await rentAs(h, 203, { profile: 'sweep', preferences: ['GA102'] });
  assert.deepEqual([sweep.status, sweep.data.state], [409, 'BUSY']);
  assert.equal(h.cloud.creates, 2);
});
// Harness cost of one rental: 0.2 USD/h for 20 min + one cron period = 0.07 USD, no transfer.
for (const [name, bindings, credit] of [
  ['daily', { DAILY_SPEND_USD: '0.1' }, 20],
  ['rolling', { ROLLING_SPEND_USD: '0.1' }, 20],
  ['credit reserve', {}, 8.1],
])
  test('item 8b: the ' + name + ' cap sums the liabilities of all active runs', async () => {
    const h = await setup({ MAX_CONCURRENT_RENTALS: '2', ...bindings });
    market(h);
    h.cloud.credit = credit;
    assert.equal((await rentAs(h, 201)).status, 200);
    const second = await rentAs(h, 202);
    assert.deepEqual([second.status, second.data.state], [409, 'UNAVAILABLE_BUDGET']);
    assert.equal(h.cloud.creates, 1);
    // Control: once the first run has ended (after no billed time), the same request fits.
    assert.equal((await h.call('/stop/201-1', { token: h.bindings.OWNER_STOP_TOKEN })).status, 200);
    assert.equal((await rentAs(h, 203)).status, 200);
  });
test('item 8b: a repriced contract is re-budgeted against the other runs too', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  h.cloud.credit = 8.15; // two runs fit: 8 + 0.07 + 0.07 = 8.14
  assert.equal((await rentAs(h, 201)).status, 200);
  assert.equal((await rentAs(h, 202)).status, 200);
  // 0.3 USD/h: 8 + 0.105 (this run) + 0.07 (run 201) = 8.175 > 8.15; alone it would fit.
  Object.assign(h.cloud.instances.get('902'), { dph_total: 0.3, dph_base: 0.3 - 0.02 });
  const repriced = await h.call('/status/202-1', { claims: { run_id: '202' } });
  assert.equal(repriced.status, 409);
  assert.equal((await repriced.json()).state, 'UNAVAILABLE_BUDGET');
  assert.deepEqual(h.cloud.deletes, ['902']);
  assert.equal((await record(h, 201)).ended, null);
  assert.equal((await statusOf(h, 201)).state, 'BOOTING');
});
test('item 8c: one run failing and closing never touches another', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  assert.equal((await rentAs(h, 202)).status, 200);
  const beforeB = await record(h, 202);
  assert.equal(
    (await h.report({ seq: 1, kind: 'log', text: 'b alive\n' }, { run: '202-1' })).status,
    200,
  );
  const fail = { seq: 1, kind: 'result', result: { nonce, verdict: 'FAIL' } };
  assert.equal((await h.report(fail, { run: '201-1' })).status, 200);
  h.time.value += 11 * 60000; // past the 10-minute fail grace of 201, inside 202's heartbeat
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes, ['901']);
  const a = await record(h, 201),
    b = await record(h, 202);
  assert.notEqual(a.ended, null);
  assert.equal(b.ended, null);
  assert.equal(b.closed, null);
  assert.equal(b.id, beforeB.id);
  assert.equal(b.machineId, beforeB.machineId);
  assert.equal((await statusOf(h, 202)).state, 'TESTING');
  // 201's slot is free; 202 still holds its own.
  assert.equal((await rentAs(h, 203)).status, 200);
  const over = await rentAs(h, 204);
  assert.deepEqual([over.status, over.data.state], [409, 'BUSY']);
});
test('item 8c: a refused create of one run leaves the other run untouched', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201)).status, 200);
  const base = h.cloud.fetch;
  h.cloud.fetch = async (url, init) =>
    String(url).includes('/asks/')
      ? Response.json({ success: false, error: 'no_such_ask' }, { status: 400 })
      : base(url, init);
  h.restart();
  assert.equal((await rentAs(h, 202)).status, 503);
  assert.notEqual((await record(h, 202)).ended, null);
  assert.equal((await record(h, 201)).ended, null);
  assert.equal((await statusOf(h, 201)).state, 'BOOTING');
  assert.deepEqual(h.cloud.deletes, []);
});
test('item 8d: stop-all stops every run; a failed destroy is retried by the cron', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '3' });
  market(h);
  for (const n of [201, 202, 203]) assert.equal((await rentAs(h, n)).status, 200);
  const base = h.cloud.fetch;
  let refuse = true;
  h.cloud.fetch = async (url, init = {}) =>
    refuse && init.method === 'DELETE' && String(url).includes('/instances/902')
      ? new Response('PRIVATE-UPSTREAM', { status: 500 })
      : base(url, init);
  h.restart();
  const all = await h.call('/stop-all', { token: h.bindings.OWNER_STOP_TOKEN });
  assert.equal(all.status, 503);
  assert.deepEqual(h.cloud.deletes.sort(), ['901', '903']);
  for (const n of [201, 203]) assert.notEqual((await record(h, n)).ended, null);
  assert.equal((await record(h, 202)).ended, null);
  assert.equal((await record(h, 202)).closed, 'STOPPED');
  refuse = false;
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes.sort(), ['901', '902', '903']);
  assert.notEqual((await record(h, 202)).ended, null);
  assert.equal((await h.call('/stop-all', { token: h.bindings.OWNER_STOP_TOKEN })).status, 200);
});
test('item 8d: one cron tick destroys every overdue run', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '4' });
  market(h);
  for (const n of [201, 202, 203, 204]) assert.equal((await rentAs(h, n)).status, 200);
  h.time.value += 21 * 60000; // past the 20-minute hard limit of all four
  await h.service.sweep();
  assert.deepEqual(h.cloud.deletes.sort(), ['901', '902', '903', '904']);
  for (const n of [201, 202, 203, 204]) assert.notEqual((await record(h, n)).ended, null);
});
test('item 8d: the reaper deletes every overdue CI box of many runs', async () => {
  const { createReaper } = await import('../../vast-reaper/worker.js');
  const now = 1800000000000,
    age = (minutes) => (now - minutes * 60000) / 1000;
  const rows = new Map(
    [
      ...[201, 202, 203, 204].map((n) => ({
        id: n,
        label: 'kf-ci-' + n + '-1',
        start_date: age(196),
      })),
      { id: 205, label: 'kf-ci-205-1', start_date: age(10) },
      { id: 206, label: 'owner-dev', start_date: age(999) },
    ].map((x) => [String(x.id), x]),
  );
  const reaper = createReaper({
    now: () => now,
    report: () => {},
    fetcher: async (input, init = {}) => {
      const url = new URL(input);
      if (url.pathname === '/api/v1/instances/')
        return Response.json({ success: true, instances: [...rows.values()] });
      rows.delete(/instances\/([0-9]+)\//.exec(url.pathname)[1]);
      return Response.json({ success: true });
    },
  });
  await reaper.sweep({
    REAPER_VAST_API_KEY: 'reaper-mock-key',
    REAPER_HARD_MINUTES: '180',
    REAPER_MARGIN_MINUTES: '15',
  });
  assert.deepEqual([...rows.keys()].sort(), ['205', '206']);
});
test('item 8e: a push supersedes only the older run of its own branch', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201, { branch: 'x' })).status, 200);
  assert.equal((await rentAs(h, 202, { branch: 'y' })).status, 200);
  const newer = await rentAs(h, 203, { branch: 'x' });
  assert.deepEqual([newer.status, newer.data.state], [409, 'BUSY']);
  assert.deepEqual(h.cloud.deletes, ['901']);
  assert.equal((await record(h, 201)).closed, 'SUPERSEDED');
  assert.equal((await record(h, 202)).ended, null);
  assert.equal((await rentAs(h, 203, { branch: 'x' })).status, 200);
  // A late request of an older run on branch x never closes the newer run.
  const late = await rentAs(h, 200, { branch: 'x' });
  assert.equal(late.status, 409);
  assert.equal(late.data.state, 'NOT_ELIGIBLE');
  assert.deepEqual(h.cloud.deletes, ['901']);
  assert.equal((await record(h, 203)).ended, null);
  assert.equal(h.cloud.creates, 3);
});
test('item 8f: status, logs and stop stay per run', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201, { preferences: ['AD104'] })).status, 200);
  assert.equal((await rentAs(h, 202, { preferences: ['GA106'] })).status, 200);
  await h.report({ seq: 1, kind: 'log', text: 'line of run a\n' }, { run: '201-1' });
  await h.report({ seq: 1, kind: 'log', text: 'line of run b\n' }, { run: '202-1' });
  const logs = async (n) =>
    (
      await (
        await h.call('/logs/' + n + '-1?cursor=0', { method: 'GET', claims: { run_id: String(n) } })
      ).json()
    ).log_increment_sanitised;
  assert.match(await logs(201), /line of run a/);
  assert.doesNotMatch(await logs(201), /line of run b/);
  assert.match(await logs(202), /line of run b/);
  assert.deepEqual(
    [(await statusOf(h, 201)).machine_id, (await statusOf(h, 202)).machine_id],
    [100001, 200011],
  );
  // Run 202's own token cannot read or stop run 201.
  for (const path of ['/status/201-1', '/logs/201-1?cursor=0'])
    assert.equal((await h.call(path, { method: 'GET', claims: { run_id: '202' } })).status, 401);
  assert.equal((await h.call('/stop/201-1', { claims: { run_id: '202' } })).status, 401);
  const stopped = await h.call('/stop/201-1', { claims: { run_id: '201' } });
  assert.equal((await stopped.json()).state, 'NONE');
  assert.deepEqual(h.cloud.deletes, ['901']);
  assert.equal((await statusOf(h, 202)).state, 'TESTING');
});
test('item 8g: four concurrent runs on four dies via preferences, one machine each', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '4' });
  market(h);
  const dies = ['GA106', 'AD104', 'GA102', 'AD102'];
  for (const [i, die] of dies.entries())
    assert.equal((await rentAs(h, 201 + i, { preferences: [die] })).status, 200, die);
  const views = [];
  for (const i of dies.keys()) views.push(await statusOf(h, 201 + i));
  assert.deepEqual(
    views.map((v) => [v.run, v.die, v.machine_id, v.state]),
    [
      ['201-1', 'GA106', 200011, 'BOOTING'],
      ['202-1', 'AD104', 100001, 'BOOTING'],
      ['203-1', 'GA102', 200012, 'BOOTING'],
      ['204-1', 'AD102', 200013, 'BOOTING'],
    ],
  );
  assert.deepEqual(
    h.cloud.created.map((x) => x.ask),
    [11, 5, 12, 13],
  );
  assert.equal(new Set(views.map((v) => v.id)).size, 4);
  const fifth = await rentAs(h, 205, { preferences: ['AD104'] });
  assert.deepEqual([fifth.status, fifth.data.state], [409, 'BUSY']);
});
test('item 8g: two runs never take the same machine; availability shows it IN_USE', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '3' });
  market(h);
  assert.equal((await rentAs(h, 201, { preferences: ['AD104'] })).status, 200);
  assert.equal((await rentAs(h, 202, { preferences: ['AD104'] })).status, 200);
  assert.deepEqual(
    h.cloud.created.map((x) => x.ask),
    [5, 14],
  );
  // Both AD104 machines are held: a third AD104 run finds no allowed offer (no create).
  const third = await rentAs(h, 203, { preferences: ['AD104'] });
  assert.equal(third.status, 503);
  assert.equal(third.data.reason, 'NO_ALLOWED_OFFERS');
  assert.equal(h.cloud.creates, 2);
  const view = await (
    await h.call('/availability?profile=check', { method: 'GET', claims: { run_id: '203' } })
  ).json();
  assert.equal(view.rejected.IN_USE, 2);
  assert.deepEqual(
    view.rejected_offers.filter((r) => r.reasons.includes('IN_USE')).map((r) => r.machine_id),
    [100001, 200014],
  );
  // Once 201 has ended its machine is free again.
  assert.equal((await h.call('/stop/201-1', { token: h.bindings.OWNER_STOP_TOKEN })).status, 200);
  assert.equal((await rentAs(h, 204, { preferences: ['AD104'] })).status, 200);
  assert.equal(h.cloud.created.at(-1).ask, 5);
});
for (const second of [true, false])
  test(
    'item 8g: the sweep rotation skips a machine another run holds (second AD104 ' + second + ')',
    async () => {
      const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
      market(h);
      if (!second) h.cloud.extraOffers = h.cloud.extraOffers.filter((x) => x.id !== 14);
      assert.equal((await rentAs(h, 201, { preferences: ['AD104'] })).status, 200);
      assert.equal(await h.storage.get('rotation:lastDie'), undefined); // check runs never rotate
      await h.storage.put('rotation:lastDie', 'AD106'); // next in rotation: AD104, AD103, AD102
      const sweep = await rentAs(h, 202, { profile: 'sweep' });
      assert.equal(sweep.status, 200);
      assert.equal(sweep.data.die, second ? 'AD104' : 'AD102');
      assert.equal(sweep.data.machine_id, second ? 200014 : 200013);
      assert.equal(await h.storage.get('rotation:lastDie'), sweep.data.die);
      // A refused rent (cap reached) leaves the rotation state alone.
      const third = await rentAs(h, 203, { profile: 'sweep', sha: hex(202) });
      assert.equal(third.status, 409);
      assert.equal(await h.storage.get('rotation:lastDie'), sweep.data.die);
    },
  );
