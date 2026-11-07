import test from 'node:test';
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { applyProfile } from '../src/tier.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { dieLegs, planRelease } from '../src/release.js';
import { cleanClaimsReport, buildReport } from '../src/runreport.js';
import { setup, env, MemoryStorage, MockCloud, sha } from './helpers.js';

// FALSIFIERS: any one preferred dimension can be omitted; price outranks the
// preferred tier; an unaffordable preferred offer prevents an affordable floor
// fallback; or provider offer facts are lost/replaced by guest report claims.
const cfgFor = (tier = 'small', extra = {}) =>
  applyProfile(
    configuration({
      ...env,
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      MAX_PRICE_USD_HOUR: '2',
      RELEASE_ENABLED: 'true',
      MAX_CONCURRENT_RENTALS: '9',
      ...extra,
    }),
    tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
    tier,
  );
function offer(id, patch = {}, cfg = cfgFor()) {
  const o = {
    ...new MockCloud().offer,
    id,
    machine_id: 80000 + id,
    host_id: 90000 + id,
    cpu_cores_effective: 16,
    disk_bw: 500,
    inet_down: 10000,
    dlperf: 60,
    geolocation: 'Texas, US',
    disk_space: 400,
    storage_cost: 0.12,
    inet_up_cost: 0,
    inet_down_cost: 0,
    dph_base: 0.3,
    ...patch,
  };
  o.dph_total = o.dph_base + (o.storage_cost * cfg.diskGb) / 720;
  return o;
}
async function evaluate(rows, cfg = cfgFor()) {
  const ledger = new Ledger(new MemoryStorage()),
    book = new OfferBook(() => 0);
  const profile = cfg.profile ?? 'check';
  const evaluation = await book.evaluate(null, cfg, ledger, profile, { market: rows });
  return { book, ledger, evaluation, cfg, profile };
}
async function select(x, options = {}) {
  return x.book.select(x.evaluation, x.cfg, x.ledger, x.profile, undefined, options);
}
test('preferred data: every execution profile explicitly proposes all four dimensions', () => {
  for (const tier of ['small', 'middle', 'large', 'adhoc'])
    assert.deepEqual(DEFAULTS.preferredHostQuality[tier], {
      status: 'PROPOSED',
      preferredMedianRatio: 1,
      preferredCpuCores: 12,
      preferredDiskMbps: 150,
      preferredDownMbps: 300,
    });
});
// Feasibility now implies more than the default 300 Mbps preferred floor.
// Raise the preferred threshold in these fixtures to exercise that dimension independently.
for (const [axis, low] of [
  ['dlperf', 30],
  ['cpu_cores_effective', 8],
  ['disk_bw', 100],
  ['inet_down', 5000],
]) {
  for (const mode of ['selection', 'release'])
    test(
      (mode === 'selection'
        ? 'preferred beats cheaper floor: '
        : 'release uses preferred before cheap floor: ') + axis,
      async () => {
        const tier = mode === 'release' ? 'large' : 'small';
        const old = DEFAULTS.preferredHostQuality[tier].preferredDownMbps;
        if (axis === 'inet_down') DEFAULTS.preferredHostQuality[tier].preferredDownMbps = 6000;
        try {
          const cfg = cfgFor(tier);
          const x = await evaluate(
            [offer(1, { [axis]: low, dph_base: 0.1 }, cfg), offer(2, {}, cfg)],
            cfg,
          );
          assert.equal(x.evaluation.allowed.length, 2);
          const id =
            mode === 'release'
              ? dieLegs(x.evaluation.allowed, cfg)[0].offer_id
              : (await select(x)).offer.id;
          assert.equal(id, 2);
          assert.equal(x.evaluation.allowed[0].offer_facts.performance_tier, 'floor');
          assert.equal(x.evaluation.allowed[1].offer_facts.performance_tier, 'preferred');
        } finally {
          DEFAULTS.preferredHostQuality[tier].preferredDownMbps = old;
        }
      },
    );
}
for (const tier of ['small', 'middle', 'large', 'adhoc'])
  test('preferred profile and cheapest preferred: ' + tier, async () => {
    const cfg = cfgFor(tier);
    const x = await evaluate(
      [offer(1, { dph_base: 0.4 }, cfg), offer(2, { dph_base: 0.2 }, cfg)],
      cfg,
    );
    const chosen = await select(x, { die: 'AD104' });
    assert.equal(chosen.offer.id, 2);
    assert.equal(chosen.offer.offer_facts.resource_profile, tier);
    assert.equal(chosen.offer.offer_facts.performance_tier, 'preferred');
  });
test('preferred fallback: offer above effective price cap is excluded before selection', async () => {
  const x = await evaluate([
    offer(1, { cpu_cores_effective: 8, dph_base: 0.1 }),
    offer(2, { dph_base: 3 }),
  ]);
  assert.equal(x.evaluation.allowed.length, 1);
  assert.equal((await select(x)).offer.id, 1);
  assert.equal(x.evaluation.allowed[0].offer_facts.performance_tier, 'floor');
});
test('preferred fallback: release excludes over-cap preferred offer', async () => {
  const cfg = cfgFor('large');
  const x = await evaluate(
    [offer(1, { disk_bw: 100, dph_base: 0.1 }, cfg), offer(2, { dph_base: 3 }, cfg)],
    cfg,
  );
  assert.equal(dieLegs(x.evaluation.allowed, cfg)[0].offer_id, 1);
  assert.equal(x.evaluation.allowed[0].offer_facts.performance_tier, 'floor');
});
test('effective price precedes preferred country within the preferred tier', async () => {
  const x = await evaluate([
    offer(1, { dph_base: 0.4 }),
    offer(2, { dph_base: 0.2, geolocation: 'Paris, FR' }),
  ]);
  assert.equal((await select(x)).offer.id, 2);
  assert.equal(dieLegs(x.evaluation.allowed, x.cfg)[0].offer_id, 2);
});
test('offer facts contain provider values, die median and ratio at selection time', async () => {
  const x = await evaluate([offer(1, { dlperf: 30 }), offer(2, { dlperf: 60 })]);
  const facts = (await select(x)).offer.offer_facts;
  assert.equal(facts.dlperf, 60);
  assert.equal(facts.die_median_dlperf, 45);
  assert.equal(facts.median_ratio, 60 / 45);
  assert.equal(facts.cpu_cores_effective, 16);
  assert.equal(facts.disk_bw, 500);
  assert.equal(facts.inet_down, 10000);
});
for (const quality of ['preferred', 'floor'])
  test('ledger records provider offer facts for ' + quality, async () => {
    const h = await setup({ LOGS_BUCKET: { put: async () => ({}) } });
    if (quality === 'floor') h.cloud.offer.disk_bw = 100;
    assert.equal((await h.call()).status, 200);
    const r = await h.storage.get('record:123-1');
    assert.equal(r.offerFacts.performance_tier, quality);
    assert.equal(r.offerFacts.disk_bw, quality === 'floor' ? 100 : h.cloud.offer.disk_bw);
    assert.equal(r.offerFacts.dlperf, h.cloud.offer.dlperf);
    assert.equal(r.offerFacts.median_ratio, 1);
    await h.report({ seq: 1, kind: 'log', text: 'offer calibration fixture\n' });
    await h.call('/stop/123-1');
    const queue = await h.storage.get('plogq:123-1:0');
    assert.deepEqual(queue.offerFacts, r.offerFacts);
  });
for (const quality of ['preferred', 'floor'])
  test('report uses Worker facts and ignores guest replacement for ' + quality, async () => {
    const x = await evaluate([offer(1, quality === 'floor' ? { disk_bw: 100 } : {})]);
    const facts = x.evaluation.allowed[0].offer_facts;
    const claims = cleanClaimsReport({
      schema: 1,
      offer_facts: { performance_tier: 'guest-forged' },
    });
    const doc = buildReport({
      claims,
      row: { verdict: 'PASS' },
      runId: 'vast-123-1-0',
      offerFacts: facts,
    });
    assert.deepEqual(doc.offer_facts, facts);
    assert.equal(doc.offer_facts.performance_tier, quality);
  });

for (const bad of [{ status: 'unmarked' }, { preferredCpuCores: 0 }])
  test('preferred data rejects invalid metadata ' + JSON.stringify(bad), () => {
    const original = DEFAULTS.preferredHostQuality.small;
    try {
      DEFAULTS.preferredHostQuality.small = { ...original, ...bad };
      assert.throws(
        () => cfgFor(),
        (e) => e.state === 'CONFIG_VAR:PREFERRED_HOST_QUALITY',
      );
    } finally {
      DEFAULTS.preferredHostQuality.small = original;
    }
  });
for (const mode of ['select', 'release'])
  test('preferred ranking includes expected network: ' + mode, async () => {
    const cfg = cfgFor('large');
    const x = await evaluate(
      [offer(1, { dph_base: 0.1, inet_down_cost: 0.005 }, cfg), offer(2, { dph_base: 0.2 }, cfg)],
      cfg,
    );
    const id =
      mode === 'select'
        ? (await select(x, { die: 'AD104' })).offer.id
        : dieLegs(x.evaluation.allowed, cfg)[0].offer_id;
    assert.equal(id, 2);
  });
for (const cap of ['0.1', '0.15'])
  test('preferred fallback respects remaining run budget ' + cap, async () => {
    const h = await setup({ DAILY_SPEND_USD: cap });
    h.cloud.offer = { ...h.cloud.offer, cpu_cores_effective: 8 };
    h.cloud.extraOffers = [
      { ...h.cloud.offer, id: 6, machine_id: 100002, cpu_cores_effective: 16, price: 0.45 },
    ];
    h.cloud.price = 0.2;
    assert.equal((await h.call()).status, 200);
    const r = await h.storage.get('record:123-1');
    assert.equal(r.machineId, 100001);
    assert.equal(r.offerFacts.performance_tier, 'floor');
  });

for (const cap of ['9', '9.25'])
  test('preferred release plan reserves remaining dies within total cap ' + cap, async () => {
    const cfg = cfgFor('large', { RELEASE_COST_CAP_USD: cap });
    const rows = JSON.parse(
      readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
    ).offers.map((o) => ({
      ...o,
      dph_base: 0.15,
      storage_cost: 0.12,
      dph_total: 0.2,
      inet_down_cost: 0,
      inet_up_cost: 0,
      cpu_cores_effective: 8,
      inet_down: 10000,
    }));
    const first = rows[0];
    rows.push({
      ...first,
      id: 50000,
      machine_id: 850000,
      host_id: 950000,
      dph_base: 0.45,
      dph_total: 0.5,
      cpu_cores_effective: 16,
    });
    const x = await evaluate(rows, cfg);
    const plan = planRelease(x.evaluation.allowed, cfg);
    assert.equal(plan.admitted, true, JSON.stringify(plan.refusals));
    assert.equal(plan.legs, 8);
    assert.ok(plan.estimate_usd <= Number(cap));
    assert.equal(plan.families[0].cells[0].legs[0].offer_id, first.id);
  });

for (const cap of ['9', '9.25'])
  test('preferred release admission preserves pending leg budgets ' + cap, async () => {
    const h = await setup({
      MAX_CONCURRENT_RENTALS: '9',
      RELEASE_ENABLED: 'true',
      RELEASE_COST_CAP_USD: cap,
      ROLLING_SPEND_USD: '200',
    });
    const rows = JSON.parse(
      readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
    ).offers.map(({ dph_base, dph_total, ...o }) => ({
      ...o,
      price: 0.2,
      storage_cost: 0.12,
      inet_down_cost: 0,
      inet_up_cost: 0,
      cpu_cores_effective: 8,
      inet_down: 10000,
    }));
    rows.push({
      ...rows[0],
      id: 50000,
      machine_id: 850000,
      host_id: 950000,
      price: 0.5,
      cpu_cores_effective: 16,
    });
    h.cloud.extraOffers = rows;
    h.cloud.credit = 500;
    h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 900 + i);
    const result = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    assert.equal(h.cloud.creates, 8);
    const batch = await h.storage.get('batch:' + result.batch);
    const first = batch.legs.find((l) => l.die === 'TU106');
    const record = await h.storage.get('record:' + first.key);
    assert.equal(record.offerFacts.performance_tier, 'floor');
    assert.notEqual(record.machineId, 850000);
  });

for (const kind of ['rolling', 'account', 'credit'])
  for (const cap of [0.1, 0.15])
    test('preferred fallback respects ' + kind + ' headroom ' + cap, async () => {
      const overrides =
        kind === 'rolling'
          ? { ROLLING_SPEND_USD: String(cap) }
          : kind === 'account'
            ? { ACCOUNT_DAILY_USD: String(cap) }
            : {};
      const h = await setup(overrides);
      if (kind === 'credit') h.cloud.credit = 8 + cap;
      h.cloud.offer.cpu_cores_effective = 8;
      h.cloud.extraOffers = [
        { ...h.cloud.offer, id: 6, machine_id: 100002, cpu_cores_effective: 16, price: 0.45 },
      ];
      assert.equal((await h.call()).status, 200);
      const r = await h.storage.get('record:123-1');
      assert.equal(r.machineId, 100001);
      assert.equal(r.offerFacts.performance_tier, 'floor');
    });

// FALSIFIER: a host exactly meeting a proposed minimum is treated as floor-only.
for (const mode of ['select', 'release'])
  test('preferred minimum boundary is inclusive: ' + mode, async () => {
    const old = DEFAULTS.preferredHostQuality.large.preferredDownMbps;
    DEFAULTS.preferredHostQuality.large.preferredDownMbps = 6000;
    try {
      const cfg = cfgFor('large');
      const x = await evaluate(
        [
          offer(1, { dph_base: 0.1, cpu_cores_effective: 11, disk_bw: 149, inet_down: 5999 }, cfg),
          offer(2, { cpu_cores_effective: 12, disk_bw: 150, inet_down: 6000 }, cfg),
        ],
        cfg,
      );
      const id =
        mode === 'select'
          ? (await select(x, { die: 'AD104' })).offer.id
          : dieLegs(x.evaluation.allowed, cfg)[0].offer_id;
      assert.equal(id, 2);
      assert.equal(x.evaluation.allowed[1].offer_facts.performance_tier, 'preferred');
    } finally {
      DEFAULTS.preferredHostQuality.large.preferredDownMbps = old;
    }
  });
