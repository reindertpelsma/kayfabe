import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { setup, sha, env, clock, MemoryStorage } from './helpers.js';
import { configuration, validOffer, gpuClass } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
const fixture = JSON.parse(
  await readFile(new URL('fixtures/v5-offers.json', import.meta.url), 'utf8'),
);
const good = {
  ...fixture.offers.find((o) => o.gpu_name === 'RTX 3060'),
  id: 5,
  machine_id: 100001,
  driver_version: '580.95.05',
  gpu_name: 'RTX 3060',
  compute_cap: 860,
  dlperf: 12,
  cpu_ram: 65536,
  inet_down: 1000,
  inet_up: 100,
  dph_base: 0.1,
  storage_cost: 0.12,
  dph_total: 0.12,
  inet_down_cost: 0.001,
  inet_up_cost: 0.001,
};
const cfg = () =>
  configuration({
    ...env,
    MAX_PRICE_USD_HOUR: '1',
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
  });
const body = (profile = 'check', preferences) => ({
  runner_nonce: 'f'.repeat(64),
  tested_sha: sha,
  profile,
  ...(preferences === undefined ? {} : { preferences }),
});
async function harness(offers = [good], bindings = {}) {
  const h = await setup({
      MAX_PRICE_USD_HOUR: '1',
      HARD_RUNTIME_MINUTES: '180',
      BOOT_LIMIT_MINUTES: '40',
      DAILY_SPEND_USD: '6',
      ...bindings,
    }),
    base = h.cloud.fetch;
  h.cloud.fetch = async (u, i) =>
    new URL(String(u)).pathname.replace(/\/$/, '') === '/api/v0/bundles'
      ? (h.cloud.calls.push({ url: String(u), body: JSON.parse(i.body) }),
        Response.json({ offers }))
      : base(u, i);
  h.restart();
  return h;
}
async function available(h, profile = 'check', claims = {}) {
  return h.call('/availability?profile=' + profile, { method: 'GET', claims });
}
const sweepClaims = {
  event_name: 'workflow_dispatch',
  sha,
  ref: 'refs/heads/master',
  workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master',
};
function sweep(h) {
  h.cloud.run.event = 'workflow_dispatch';
  h.cloud.run.head_branch = 'master';
  h.cloud.run.pull_requests = [];
  h.cloud.prs = [];
}
// FALSIFIER: any missing/ambiguous quality or price fact admits a paid offer,
// or CI preferences/profile labels can widen the Worker-enforced allowed set.
for (const name of ['GPU class', 'recorded prices'])
  test('V5 complete recorded offer accepted ' + name, () =>
    assert.equal(validOffer(good, cfg()), true),
  );
const bad = [
  ['compute_cap', undefined],
  ['compute_cap', 7.5],
  ['compute_cap', '860'],
  ['compute_cap', 610],
  ['gpu_arch', 'amd'],
  ['gpu_name', 'A100'],
  ['verification', undefined],
  ['verification', 'unverified'],
  ['verified', false],
  ['reliability', 0.97],
  ['reliability', undefined],
  ['cpu_ram', 16000],
  ['cpu_ram', undefined],
  ['cpu_cores_effective', 4],
  ['cpu_ghz', 1],
  ['dlperf', 0],
  ['inet_down', 10],
  ['inet_up', 1],
  ['disk_bw', 10],
  // Owner ruling 2026-10-24 item 10: the host driver_version is no longer a filter.
  ['storage_cost', undefined],
  ['storage_cost', -1],
  ['inet_down_cost', undefined],
  ['inet_up_cost', '0.001'],
  ['dph_base', undefined],
  ['dph_total', 0.01],
  ['machine_id', 42636],
  ['vms_enabled', false],
  ['rented', true],
  ['num_gpus', 2],
];
for (const model of ['RTX 4070', 'RTX 4090'])
  for (const [key, value] of bad)
    test('V5 rejects odd/missing forced field ' + key + ' ' + value + ' ' + model, () =>
      assert.equal(
        validOffer({ ...good, gpu_name: model, compute_cap: 890, [key]: value }, cfg()),
        false,
      ),
    );
for (const key of ['storage_cost', 'inet_up_cost', 'inet_down_cost'])
  test('V5 effective cap includes ' + key, () =>
    assert.equal(
      validOffer(
        {
          ...good,
          gpu_name: 'RTX 4070',
          compute_cap: 890,
          [key]: key === 'storage_cost' ? 100 : 0.5,
        },
        configuration({
          ...env,
          MAX_PRICE_USD_HOUR: '1',
          HARD_RUNTIME_MINUTES: '20',
          BOOT_LIMIT_MINUTES: '10',
          MAX_INET_USD_PER_GB: '1',
        }),
      ),
      false,
    ),
  );
for (const profile of ['check', 'sweep'])
  test('V5 availability exposes only policy fields ' + profile, async () => {
    const h = await harness([
      good,
      { ...good, id: 6, machine_id: 100002, gpu_name: 'RTX 4060', compute_cap: 890 },
    ]);
    if (profile === 'sweep') sweep(h);
    const response = await available(h, profile, profile === 'sweep' ? sweepClaims : {});
    assert.equal(response.status, 200);
    const r = await response.json();
    assert.equal(r.profile, profile);
    assert.ok(Array.isArray(r.offers));
    assert.ok(r.offers.some((x) => x.die === 'GA106'));
    if (profile === 'check') assert.ok(!r.offers.some((x) => x.die === 'AD107'));
    // Owner ruling 2026-10-24 item 9 supersedes the machine_id ban: the public machine_id and
    // host_id are shown; the offer (ask) id, IPs and hostnames still never cross.
    assert.doesNotMatch(JSON.stringify(r), /public_ip|offer_id|hostname|"id":/);
    assert.equal(h.cloud.creates, 0);
    assert.ok(r.rejected && r.thresholds && r.verified_counts);
  });
for (const profile of ['check', 'sweep'])
  test('V5 availability caches provider reads ' + profile, async () => {
    const h = await harness();
    if (profile === 'sweep') sweep(h);
    for (let n = 0; n < 4; n++)
      assert.equal(
        (await available(h, profile, profile === 'sweep' ? sweepClaims : {})).status,
        200,
      );
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 1);
    h.time.value += 60001;
    assert.equal((await available(h, profile, profile === 'sweep' ? sweepClaims : {})).status, 200);
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 2);
  });
for (const kind of ['no-auth', 'non-owner', 'PR', 'fork'])
  test('V5 sweep availability refuses ' + kind, async () => {
    const h = await harness();
    sweep(h);
    const claims = { ...sweepClaims };
    if (kind === 'non-owner') {
      claims.actor_id = '99';
      h.cloud.run.triggering_actor = { id: 99 };
    }
    if (kind === 'PR') {
      claims.event_name = 'pull_request';
      claims.ref = 'refs/pull/7/merge';
    }
    if (kind === 'fork') claims.repository = 'foreign/repo';
    const r =
      kind === 'no-auth'
        ? await h.call('/availability?profile=sweep', { method: 'GET', token: 'bad' })
        : await available(h, 'sweep', claims);
    assert.notEqual(r.status, 200);
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 0);
  });
for (const preferences of [
  ['AD107', 'GA106'],
  ['UNKNOWN', 'GA106'],
])
  test('V5 preference intersection is forced ' + preferences, async () => {
    const h = await harness([
      good,
      {
        ...good,
        id: 6,
        machine_id: 100002,
        gpu_name: 'RTX 4060',
        compute_cap: 890,
        dph_base: 0.05,
        dph_total: 0.07,
      },
    ]);
    const r = await h.call('/rent', { body: body('check', preferences) });
    assert.equal(r.status, 200);
    const record = await h.storage.get('record:123-1');
    assert.equal(record.die, 'GA106');
    assert.equal(record.preferencesIgnored, 1);
    assert.match(record.log, /Worker.*ignored=1/);
  });
for (const preferences of [['AD107'], []])
  test('V5 empty allowed preference intersection refuses ' + preferences.length, async () => {
    const h = await harness();
    const r = await h.call('/rent', { body: body('check', preferences) });
    assert.equal(r.status, 503);
    assert.equal((await r.json()).reason, 'NO_ALLOWED_PREFERENCE');
    assert.equal(h.cloud.creates, 0);
  });
for (const cap of [1, 2.5])
  test('V5 sweep higher cap is separate ' + cap, async () => {
    const o = { ...good, dph_base: 1.5, dph_total: 1.52 };
    const h = await harness([o]);
    sweep(h);
    if (cap === 1) h.cloud.run.event = 'push';
    const r = await h.call('/rent', {
      claims:
        cap === 1
          ? {
              ...sweepClaims,
              event_name: 'push',
              workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast.yml@refs/heads/master',
            }
          : sweepClaims,
      body: { ...body(cap === 1 ? 'check' : 'sweep', ['GA106']) },
    });
    assert.equal(r.status, cap === 1 ? 503 : 200);
  });
for (const previous of ['check', 'sweep'])
  test('V5 profile daily budgets stay separate ' + previous, async () => {
    const cfg2 = { ...cfg(), profile: 'sweep', sweepDaily: 8 },
      storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      now = Date.UTC(2026, 9, 22, 12);
    await storage.put('cost:old', {
      started: now - 3600000,
      ended: now,
      price: 7,
      profile: previous,
    });
    if (previous === 'check') await ledger.budgets(cfg2, now, 2);
    else
      await assert.rejects(ledger.budgets(cfg2, now, 2), (e) => e.state === 'UNAVAILABLE_BUDGET');
  });
for (const binding of ['ROLLING_SPEND_USD', 'CREDIT_RESERVE_USD'])
  test('V5 sweep keeps common ' + binding, async () => {
    const h = await harness([good], {
      [binding]: binding === 'ROLLING_SPEND_USD' ? '0.01' : '100',
    });
    sweep(h);
    assert.equal(
      (await h.call('/rent', { claims: sweepClaims, body: { ...body('sweep') } })).status,
      409,
    );
    assert.equal(h.cloud.creates, 0);
  });

for (const die of ['GA106', 'AD104'])
  test('V5 relative performance floor within class ' + die, async () => {
    const o = {
      ...good,
      gpu_name: die === 'GA106' ? 'RTX 3060' : 'RTX 4070',
      compute_cap: die === 'GA106' ? 860 : 890,
    };
    const h = await harness([o, { ...o, id: 6, machine_id: 100002, dlperf: 1 }]);
    const r = await (await available(h)).json();
    assert.equal(r.offers[0].count, 1);
    assert.equal(r.rejected.PERFORMANCE, 1);
  });
for (const order of [false, true])
  test('V5 sweep deterministic rotation persists ' + order, async () => {
    const rows = [
      good,
      { ...good, id: 6, machine_id: 100002, gpu_name: 'RTX 4070', compute_cap: 890 },
    ];
    if (order) rows.reverse();
    const h = await harness(rows);
    sweep(h);
    await h.storage.put('rotation:lastDie', 'GA106');
    const r = await h.call('/rent', {
      claims: sweepClaims,
      body: { ...body('sweep', ['rotate']) },
    });
    assert.equal(r.status, 200);
    assert.equal((await h.storage.get('record:123-1')).die, 'AD104');
    assert.equal(await h.storage.get('rotation:lastDie'), 'AD104');
  });
for (const n of [31, 40])
  test('V5 admission rate limits and bounds provider calls ' + n, async () => {
    const h = await harness();
    let limited = 0;
    for (let i = 0; i < n; i++) {
      const r = await available(h);
      if (r.status === 429) limited++;
    }
    assert.equal(limited, n - 30);
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 1);
  });
for (const n of [2, 4])
  test('V5 failed offer reads are cached ' + n, async () => {
    const h = await setup(),
      base = h.cloud.fetch;
    let calls = 0;
    h.cloud.fetch = async (u, i) =>
      String(u).includes('/bundles/') ? (calls++, new Response('', { status: 503 })) : base(u, i);
    h.restart();
    for (let i = 0; i < n; i++) assert.equal((await available(h)).status, 503);
    assert.equal(calls, 1);
  });

for (const model of ['RTX 4070', 'RTX 4090'])
  test('V5 rejects pre-Turing capability classification ' + model, () =>
    assert.equal(gpuClass({ ...good, gpu_name: model, compute_cap: 610 }, cfg()), null),
  );
for (const [name, cc] of [
  ['GTX 1660 S', 750],
  ['RTX A6000', 860],
  ['RTX 4000Ada', 890],
  ['RTX PRO 5000', 1200],
])
  test('V5 sweep accepts consumer/workstation class ' + name, () =>
    assert.equal(
      validOffer({ ...good, gpu_name: name, compute_cap: cc }, cfg(), { profile: 'sweep' }),
      true,
    ),
  );
for (const model of ['RTX 4070', 'RTX 4090'])
  test('V5 denies configured GPU class ' + model, () =>
    assert.equal(
      validOffer(
        { ...good, gpu_name: model, compute_cap: 890 },
        { ...cfg(), gpuDeny: ['RTX_40*'] },
      ),
      false,
    ),
  );
for (const died of ['GA106', 'AD104'])
  test('V5 sweep skips recently passed class ' + died, async () => {
    const rows = [
        good,
        { ...good, id: 6, machine_id: 100002, gpu_name: 'RTX 4070', compute_cap: 890 },
      ],
      h = await harness(rows, { SWEEP_SKIP_PASSED_DAYS: '7' });
    sweep(h);
    await h.storage.put('rotation:lastDie', died === 'GA106' ? 'AD104' : 'GA106');
    await h.storage.put('passed:' + died, clock - 86400000);
    const r = await h.call('/rent', {
      claims: sweepClaims,
      body: { ...body('sweep', ['rotate']) },
    });
    assert.equal(r.status, 200);
    assert.equal((await h.storage.get('record:123-1')).die, died === 'GA106' ? 'AD104' : 'GA106');
  });
for (const restart of [false, true])
  test('V5 valid PR identity cannot use sweep ' + restart, async () => {
    const h = await harness();
    if (restart) h.restart();
    assert.equal((await available(h, 'sweep')).status, 401);
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 0);
  });
for (const model of ['RTX 4060', 'RTX A6000'])
  test('V5 check refuses non-listed class ' + model, () =>
    assert.equal(
      validOffer(
        { ...good, gpu_name: model, compute_cap: model === 'RTX 4060' ? 890 : 860 },
        cfg(),
      ),
      false,
    ),
  );
for (const old of ['check', 'sweep'])
  test('V5 opposite profile liability does not spend check daily ' + old, async () => {
    const cfg2 = { ...cfg(), profile: 'check' },
      storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      now = Date.UTC(2026, 9, 22, 12);
    await storage.put('cost:old', { started: now - 3600000, ended: now, price: 7, profile: old });
    if (old === 'sweep') await ledger.budgets(cfg2, now, 2);
    else await assert.rejects(ledger.budgets(cfg2, now, 2));
  });
for (const restart of [false, true])
  test('V5 owner dispatch cannot borrow check workflow for sweep ' + restart, async () => {
    const h = await harness();
    sweep(h);
    if (restart) h.restart();
    const r = await available(h, 'sweep', {
      ...sweepClaims,
      workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast.yml@refs/heads/master',
    });
    assert.equal(r.status, 401);
    assert.equal(h.cloud.calls.filter((x) => x.url.includes('/bundles')).length, 0);
  });
