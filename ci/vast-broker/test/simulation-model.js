import assert from 'node:assert/strict';
import { MemoryStorage, MockCloud, env, sha, clock } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { Vast } from '../src/vast.js';
import { OfferBook } from '../src/offers.js';
import { configuration, resourceCaps, profileCaps, offerPrice } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { planRelease } from '../src/release.js';
import { RELEASE_MATRIX, releaseEnvironment } from '../src/release-matrix.js';
import { DEFAULTS } from '../src/defaults.js';
import { executableMatrix } from './release-fixture.js';
const vars = {
  MAX_CONCURRENT_RENTALS: '9',
  RELEASE_ENABLED: 'true',
  HARD_RUNTIME_MINUTES: '300',
  BOOT_LIMIT_MINUTES: '75',
  ROLLING_SPEND_USD: '1000',
  DAILY_SPEND_USD: '100',
  ACCOUNT_DAILY_USD: '1000',
};
const dies = [
  ['TU106', 'RTX_2060', 750],
  ['TU116', 'GTX_1660', 750],
  ['GA102', 'RTX_3090', 860],
  ['GA104', 'RTX_3070', 860],
  ['AD102', 'RTX_4090', 890],
  ['AD104', 'RTX_4070', 890],
  ['GB202', 'RTX_5090', 1200],
  ['GB205', 'RTX_5070', 1200],
];
export function rng(seed) {
  return () => {
    let t = (seed += 0x6d2b79f5);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
export function randomMarket(seed) {
  const next = rng(seed),
    template = new MockCloud().offer,
    rows = [];
  let id = 1000 + seed * 100;
  for (const [die, gpu, cc] of dies)
    for (let n = 0; n < 8; n++) {
      const anchor = n === 0;
      const offer = {
        ...template,
        id: ++id,
        machine_id: 200000 + id,
        host_id: 300000 + Math.floor(id / 2),
        gpu_name: gpu,
        compute_cap: cc,
        dlperf: anchor ? 120 : 20 + next() * 180,
        cpu_cores_effective: anchor ? 16 : 4 + Math.floor(next() * 24),
        disk_space: anchor ? 400 : 150 + next() * 300,
        disk_bw: anchor ? 500 : 50 + next() * 500,
        reliability: anchor ? 0.999 : 0.94 + next() * 0.06,
        inet_down: anchor ? 2500 : 100 + next() * 3500,
        inet_up: anchor ? 1000 : 10 + next() * 1500,
        inet_down_cost: anchor ? 0 : next() * 0.03,
        inet_up_cost: anchor ? 0 : next() * 0.015,
        dph_base: anchor ? 0.09 : 0.01 + next() * 0.8,
        storage_cost: 0.12,
        geolocation: anchor
          ? 'Texas, US'
          : ['City, US', 'City, DE', 'City, FR', 'City, CN'][Math.floor(next() * 4)],
        vms_enabled: anchor || next() > 0.15,
        rentable: true,
        gpu_ram: 24576,
      };
      if (n === 2) offer.machine_id = rows.at(-1).machine_id;
      if (n === 3) offer.host_id = 99991;
      if (n === 4) offer.machine_id = 99992;
      if (n === 6) offer.machine_id = 99994;
      if (n === 7) offer.host_id = 99993;
      if (n === 5) offer[['inet_down_cost', 'reliability', 'disk_bw', 'inet_up'][seed % 4]] = null;
      rows.push(offer);
    }
  return rows;
}
const legsOf = (plan) => plan.families.flatMap((f) => f.cells.flatMap((c) => c.legs));
export async function simulateSeed(seed) {
  const raw = randomMarket(seed);
  let rentals = 0;
  for (const tier of ['small', 'middle', 'large', 'adhoc']) {
    const profile = tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check';
    const cfg = applyProfile(
      configuration({
        ...env,
        ...vars,
        BAD_MACHINE_IDS: '[99992]',
        BAD_HOST_IDS: '[99991]',
        RELEASE_COST_CAP_USD: String(10 + (seed % 3) * 10),
      }),
      profile,
      tier,
    );
    const market = raw.map((o) => ({
      ...o,
      price: o.dph_base + (o.storage_cost * cfg.diskGb) / 720,
      dph_total: o.dph_base + (o.storage_cost * cfg.diskGb) / 720,
    }));
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      cloud = new MockCloud();
    const [first, ...rest] = market;
    cloud.offer = first;
    cloud.price = first.price;
    cloud.extraOffers = rest;
    cloud.credit = 500;
    cloud.contracts = Array.from({ length: 20 }, (_, n) => 9000000 + seed * 100 + n);
    await storage.put('bad-machine:99994', { reason: 'SIMULATED', at: clock });
    await storage.put('bad-host:99993', { reason: 'SIMULATED', at: clock });
    const vast = new Vast(cloud.fetch, cfg, 'synthetic-test-key'),
      book = new OfferBook(() => clock);
    const evaluation = await book.evaluate(null, cfg, ledger, profile, { market });
    const limits = DEFAULTS.payloadProfiles[tier];
    for (const o of evaluation.allowed) {
      assert.ok(o.vms_enabled === true);
      for (const field of [
        'dph_base',
        'dph_total',
        'storage_cost',
        'inet_down_cost',
        'inet_up_cost',
        'inet_down',
        'inet_up',
        'disk_space',
        'disk_bw',
        'reliability',
        'dlperf',
        'cpu_cores_effective',
      ])
        assert.ok(
          typeof o[field] === 'number' && Number.isFinite(o[field]) && o[field] >= 0,
          'typed offer field ' + field,
        );
      const quality = cfg.quality[profile];
      assert.ok(o.reliability >= quality.reliability && o.reliability <= 1);
      assert.ok(o.cpu_cores_effective >= quality.cores && o.disk_bw >= quality.diskMbps);
      assert.ok(o.inet_down >= quality.downMbps && o.inet_up >= quality.upMbps);
      assert.notEqual(o.machine_id, 99992);
      assert.notEqual(o.host_id, 99991);
      assert.notEqual(o.host_id, 99993);
      assert.notEqual(o.machine_id, 99994);
      assert.ok(
        o.inet_down_cost <= limits.maxInetUsdPerGb && o.inet_up_cost <= limits.maxInetUsdPerGb,
      );
      const compute = o.dph_base + (o.storage_cost * limits.diskGb) / 720;
      const network =
        limits.bandwidthDownGb * o.inet_down_cost + limits.bandwidthUpGb * o.inet_up_cost;
      assert.ok(
        Math.abs(o.pricing.effective - (compute + network / (cfg.hard / 3600000))) < 1e-8,
        'independent effective-price oracle',
      );
    }

    const reverse = await new OfferBook(() => clock).evaluate(null, cfg, ledger, profile, {
      market: [...market].reverse().map((o) => Object.fromEntries(Object.entries(o).reverse())),
    });
    let selected = [];
    if (tier === 'large') {
      const a = planRelease(evaluation.allowed, cfg),
        b = planRelease(reverse.allowed, cfg);
      assert.equal(
        new Set(legsOf(a).map((l) => l.machine_id)).size,
        legsOf(a).length,
        'plan machine uniqueness',
      );
      assert.deepEqual(
        legsOf(a).map((l) => l.offer_id),
        legsOf(b).map((l) => l.offer_id),
        'deterministic release ranking',
      );
      if (a.admitted) {
        assert.ok(a.estimate_usd <= a.cost_cap_usd);
        selected = legsOf(a).map((l) => evaluation.allowed.find((o) => o.id === l.offer_id));
      }
    } else if (evaluation.allowed.length) {
      const a = await book.select(evaluation, cfg, ledger, profile),
        b = await book.select(reverse, cfg, ledger, profile);
      assert.equal(a.offer.id, b.offer.id, 'deterministic profile ranking');
      selected = [a.offer];
    }
    if (['small', 'middle', 'large', 'adhoc'][seed % 4] !== tier) selected = [];
    if (selected.length > 1) selected = [selected[seed % selected.length]];
    const owned = [];
    let quoted = 0;
    for (const [index, o] of selected.entries()) {
      const q = resourceCaps(cfg, profile);
      assert.ok(o.pricing.effective <= profileCaps(cfg, profile).price);
      assert.ok(o.inet_down_cost <= q.maxInetUsdPerGb && o.inet_up_cost <= q.maxInetUsdPerGb);
      assert.notEqual(o.machine_id, 99992);
      assert.notEqual(o.host_id, 99991);
      assert.notEqual(o.host_id, 99993);
      assert.notEqual(o.machine_id, 99994);
      assert.ok(!owned.some((r) => r.machineId === o.machine_id));
      const run = String(100000000 + seed * 100 + index),
        record = {
          key: run + '-1',
          run,
          attempt: 1,
          sha,
          branch: 'simulation-' + seed,
          profile: profile === 'release' ? 'check' : profile,
          ...(profile === 'release' ? { budgetClass: 'release' } : {}),
          started: clock,
          ttl: cfg.hard,
          billingTtl: cfg.hard + cfg.cronPeriod,
          ended: null,
          id: null,
          machineId: o.machine_id,
          hostId: o.host_id,
          price: o.pricing.effective,
          computeStoragePrice: o.pricing.computeStorage,
          bandwidthCost: o.pricing.bandwidth,
          label: 'kf-ci-simulation-' + run,
          nonce: 'a'.repeat(32),
        };
      await ledger.reserve(record, { cfg, now: clock, cost: o.pricing.total, credit: 500 });
      record.id = await vast.create(
        o,
        record,
        'simulation',
        tier === 'large'
          ? releaseEnvironment(RELEASE_MATRIX.dies.find((r) => r.die === o.die))
          : {},
      );
      record.idSource = 'create';
      await ledger.save(record);
      const create = cloud.calls.filter((c) => /\/asks\//.test(c.url)).at(-1);
      assert.equal(create.body.disk, DEFAULTS.payloadProfiles[tier].diskGb);
      if (tier === 'large') {
        const pins = JSON.parse(Buffer.from(create.body.env.KF_RELEASE_DRIVER_PINS, 'base64url'));
        for (const p of pins) {
          const expected = RELEASE_MATRIX.driver_installers.find(
            (e) => e.os === p.os && e.version === p.version,
          );
          assert.deepEqual(p, expected);
          assert.equal(p.status, 'PINNED');
        }
      }
      owned.push(record);
      rentals++;
      quoted += o.pricing.total;
      assert.ok((await ledger.openRecords()).length <= cfg.maxConcurrentProfile[profile]);
      if (tier === 'large') assert.ok(quoted <= cfg.releaseCostCap + 1e-8);
    }
    for (const record of owned) {
      await vast.destroy(record);
      record.closed = 'STOPPED';
      await ledger.settle(record, clock + 60000);
      assert.equal(cloud.instances.has(record.id), false);
    }
    assert.equal((await ledger.openRecords()).length, 0);
    assert.equal(cloud.instances.size, 2);
    assert.deepEqual([...new Set(cloud.deletes)].sort(), owned.map((r) => r.id).sort());
  }
  return rentals;
}
