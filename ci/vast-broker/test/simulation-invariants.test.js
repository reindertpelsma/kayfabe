import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { matrixTest as test, capablePayload } from './release-fixture.js';
import { setup, env, sha, clock, MemoryStorage, MockCloud } from './helpers.js';
import { configuration, offerPrice, profileCaps, gpuClass } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { DEFAULTS } from '../src/defaults.js';
import { planRelease } from '../src/release.js';
import { RELEASE_MATRIX } from '../src/release-matrix.js';
import { Ledger } from '../src/ledger.js';
import { OfferBook } from '../src/offers.js';
const vars = {
  MAX_CONCURRENT_RENTALS: '9',
  RELEASE_ENABLED: 'true',
  HARD_RUNTIME_MINUTES: '300',
  BOOT_LIMIT_MINUTES: '75',
  ROLLING_SPEND_USD: '200',
  DAILY_SPEND_USD: '40',
  ACCOUNT_DAILY_USD: '100',
};
async function harness(extra = {}) {
  const h = capablePayload(await setup({ ...vars, ...extra }));
  h.cloud.credit = 500;
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 900 + i);
  const rows = JSON.parse(
    readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
  ).offers;
  const [first, ...rest] = rows;
  h.cloud.offer = first;
  h.cloud.price = first.dph_total;
  h.cloud.extraOffers = rest.map((o) => ({ ...o, price: o.dph_total }));
  return h;
}
for (const cap of [2, 3])
  test('simulation invariant: no rental above release cap ' + cap, async () => {
    const h = await harness({ RELEASE_COST_CAP_USD: String(cap) });
    await assert.rejects(
      () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
      (e) => {
        assert.equal(e.reason, 'PLAN_REFUSED');
        assert.ok(e.plan.refusals.includes('COST_CAP'));
        return true;
      },
    );
    assert.equal(h.cloud.creates, 0);
    assert.equal((await new Ledger(h.storage).openRecords()).length, 0);
    assert.equal(h.cloud.instances.size, 2);
  });
for (const version of ['580.65.06', '580.88'])
  test('simulation invariant: executable UNPINNED driver never rents ' + version, async () => {
    const p = RELEASE_MATRIX.driver_installers.find((p) => p.version === version);
    p.status = 'UNPINNED';
    p.sha256 = null;
    p.size = null;
    const h = await harness();
    await assert.rejects(
      () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
      (e) => e.reason === 'MATRIX_INVALID',
    );
    assert.equal(h.cloud.creates, 0);
    assert.equal((await new Ledger(h.storage).openRecords()).length, 0);
    assert.equal(h.cloud.instances.size, 2);
  });
function cfg() {
  return applyProfile(configuration({ ...env, ...vars }), 'release', 'large');
}
function record(n, machine = 10000 + n) {
  return {
    key: String(800000 + n) + '-1',
    run: String(800000 + n),
    attempt: 1,
    sha,
    branch: 'simulation',
    profile: 'check',
    budgetClass: 'release',
    started: clock,
    ttl: 60000,
    ended: null,
    id: null,
    machineId: machine,
    hostId: 20000 + n,
    price: 0.1,
    bandwidthCost: 0,
  };
}
for (const limit of [2, 3])
  test('simulation invariant: atomic concurrency ' + limit, async () => {
    const c = cfg();
    c.maxConcurrent = limit;
    c.maxConcurrentProfile.release = limit;
    c.releaseDailyCount = 64;
    const l = new Ledger(new MemoryStorage());
    for (let n = 0; n < limit; n++)
      await l.reserve(record(n), { cfg: c, now: clock, cost: 0.1 / 60, credit: 500 });
    await assert.rejects(
      () => l.reserve(record(limit), { cfg: c, now: clock, cost: 0.1 / 60, credit: 500 }),
      (e) => e.state === 'BUSY',
    );
    assert.equal((await l.openRecords()).length, limit);
    assert.equal(await l.get('record:' + record(limit).key), undefined);
  });
for (const machine of [10101, 20202])
  test('simulation invariant: atomic duplicate machine ' + machine, async () => {
    const c = cfg(),
      l = new Ledger(new MemoryStorage());
    await l.reserve(record(0, machine), { cfg: c, now: clock, cost: 0.1 / 60, credit: 500 });
    await assert.rejects(() =>
      l.reserve(record(1, machine), { cfg: c, now: clock, cost: 0.1 / 60, credit: 500 }),
    );
    assert.equal((await l.openRecords()).length, 1);
    assert.equal(await l.get('record:' + record(1).key), undefined);
  });
for (const tier of ['small', 'large'])
  for (const rate of [0.004, 0.005])
    test('simulation price: network changes cheapest compute ' + tier + ' ' + rate, async () => {
      const profile = tier === 'large' ? 'release' : 'check',
        c = applyProfile(configuration({ ...env, ...vars }), profile, tier),
        cloud = new MockCloud(),
        l = new Ledger(new MemoryStorage()),
        b = new OfferBook(() => clock);
      const rows = [
        { id: 1, compute: 0.1, rate },
        { id: 2, compute: 0.19, rate: 0 },
      ].map((x) => ({
        ...cloud.offer,
        id: x.id,
        machine_id: 100 + x.id,
        host_id: 200 + x.id,
        inet_down: 10000,
        disk_space: 400,
        storage_cost: 0.12,
        dph_base: x.compute,
        dph_total: x.compute + (0.12 * c.diskGb) / 720,
        inet_down_cost: x.rate,
        inet_up_cost: 0,
      }));
      const e = await b.evaluate(null, c, l, profile, { market: rows });
      assert.equal(e.allowed.length, 2);
      const choice = await b.select(e, c, l, profile, undefined, { die: 'AD104' });
      assert.equal(choice.offer.id, tier === 'small' ? 1 : 2);
      assert.equal(rows[0].dph_base < rows[1].dph_base, true);
    });

for (const tier of ['small', 'large']) {
  test('simulation invariant: per-GB cap ' + tier, async () => {
    const profile = tier === 'large' ? 'release' : 'check',
      c = applyProfile(
        configuration({
          ...env,
          ...vars,
          MAX_PRICE_USD_HOUR: '2',
          RELEASE_MAX_PRICE_USD_HOUR: '2',
        }),
        profile,
        tier,
      ),
      base = new MockCloud().offer;
    const o = {
      ...base,
      id: 1,
      disk_space: 400,
      inet_down: 10000,
      storage_cost: 0.12,
      dph_base: 0.01,
      dph_total: 0.01 + (0.12 * c.diskGb) / 720,
      inet_down_cost: DEFAULTS.payloadProfiles[tier].maxInetUsdPerGb + 0.000001,
      inet_up_cost: 0,
    };
    assert.ok(offerPrice(o, c, profile).effective < profileCaps(c, profile).price);
    const e = await new OfferBook(() => clock).evaluate(
      null,
      c,
      new Ledger(new MemoryStorage()),
      profile,
      { market: [o] },
    );
    assert.equal(e.allowed.length, 0);
  });
  test('simulation invariant: hourly cap ' + tier, async () => {
    const profile = tier === 'large' ? 'release' : 'check',
      c = applyProfile(configuration({ ...env, ...vars }), profile, tier),
      base = new MockCloud().offer;
    const o = {
      ...base,
      id: 1,
      disk_space: 400,
      inet_down: 10000,
      storage_cost: 0.12,
      dph_base: 3,
      dph_total: 3 + (0.12 * c.diskGb) / 720,
      inet_down_cost: 0,
      inet_up_cost: 0,
    };
    const e = await new OfferBook(() => clock).evaluate(
      null,
      c,
      new Ledger(new MemoryStorage()),
      profile,
      { market: [o] },
    );
    assert.equal(e.allowed.length, 0);
  });
  test('simulation invariant: deterministic offer ties ' + tier, async () => {
    const profile = tier === 'large' ? 'release' : 'check',
      c = applyProfile(configuration({ ...env, ...vars }), profile, tier),
      base = new MockCloud().offer;
    const rows = [1, 2].map((id) => ({
      ...base,
      id,
      machine_id: 100 + id,
      host_id: 200 + id,
      disk_space: 400,
      inet_down: 10000,
      storage_cost: 0.12,
      dph_base: 0.1,
      dph_total: 0.1 + (0.12 * c.diskGb) / 720,
      inet_down_cost: 0,
      inet_up_cost: 0,
    }));
    const b = new OfferBook(() => clock),
      l = new Ledger(new MemoryStorage());
    for (const market of [rows, [...rows].reverse()]) {
      const e = await b.evaluate(null, c, l, profile, { market });
      const chosen = await b.select(e, c, l, profile, undefined, { die: 'AD104' });
      assert.equal(chosen.offer.id, 1);
    }
  });
}
for (const die of ['TU116', 'GB205'])
  test('simulation invariant: deterministic release ties ' + die, async () => {
    const c = cfg(),
      l = new Ledger(new MemoryStorage()),
      b = new OfferBook(() => clock);
    const marketRows = JSON.parse(
      readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
    ).offers;
    const original = marketRows.find((o) => gpuClass(o, c) === die);
    const rows = marketRows.filter((o) => gpuClass(o, c) !== die);
    rows.push(original, {
      ...original,
      id: original.id + 10000,
      machine_id: original.machine_id + 10000,
    });
    for (const market of [rows, [...rows].reverse()]) {
      const e = await b.evaluate(null, c, l, 'release', { market });
      const p = planRelease(e.allowed, c);
      const leg = p.families
        .flatMap((f) => f.cells.flatMap((c) => c.legs))
        .find((l) => l.release_matrix.die === die);
      assert.equal(leg.offer_id, original.id);
    }
  });

for (const rate of [null, '0.001'])
  test('simulation invariant: malformed inbound cost ' + String(rate), async () => {
    const c = cfg(),
      base = new MockCloud().offer;
    const o = {
      ...base,
      id: 1,
      disk_space: 400,
      inet_down: 10000,
      storage_cost: 0.12,
      dph_base: 0.1,
      dph_total: 0.1 + (0.12 * c.diskGb) / 720,
      inet_down_cost: rate,
      inet_up_cost: 0,
    };
    const e = await new OfferBook(() => clock).evaluate(
      null,
      c,
      new Ledger(new MemoryStorage()),
      'release',
      { market: [o] },
    );
    assert.equal(e.allowed.length, 0);
  });
for (const kind of ['machine', 'host'])
  for (const id of [101, 202]) {
    test('simulation invariant: candidate bad ' + kind + ' ' + id, async () => {
      const c = applyProfile(
          configuration({
            ...env,
            ...vars,
            ...(kind === 'machine'
              ? { BAD_MACHINE_IDS: JSON.stringify([id]) }
              : { BAD_HOST_IDS: JSON.stringify([id]) }),
          }),
          'release',
          'large',
        ),
        base = new MockCloud().offer;
      const o = {
        ...base,
        id: 1,
        machine_id: kind === 'machine' ? id : 99901,
        host_id: kind === 'host' ? id : 99902,
        disk_space: 400,
        inet_down: 10000,
        storage_cost: 0.12,
        dph_base: 0.1,
        dph_total: 0.1 + (0.12 * c.diskGb) / 720,
        inet_down_cost: 0,
        inet_up_cost: 0,
      };
      const l = new Ledger(new MemoryStorage()),
        e = await new OfferBook(() => clock).evaluate(null, c, l, 'release', { market: [o] });
      assert.equal(e.allowed.length, 0);
    });
    for (const persisted of [false, true])
      test(
        'simulation invariant: atomic bad ' + kind + ' ' + id + ' persisted=' + persisted,
        async () => {
          const c = cfg(),
            l = new Ledger(new MemoryStorage()),
            r = record(0, kind === 'machine' ? id : 99901);
          r.hostId = kind === 'host' ? id : 99902;
          if (persisted) await l.put('bad-' + kind + ':' + id, { reason: 'SIMULATED', at: clock });
          else c[kind === 'machine' ? 'badMachines' : 'badHosts'] = [id];
          await assert.rejects(() =>
            l.reserve(r, { cfg: c, now: clock, cost: 0.1 / 60, credit: 500 }),
          );
          assert.equal((await l.openRecords()).length, 0);
          assert.equal(await l.get('record:' + r.key), undefined);
        },
      );
  }
