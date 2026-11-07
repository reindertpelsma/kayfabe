import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration, offerPrice, offerRejections, resourceCaps } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { Vast } from '../src/vast.js';
import { DEFAULTS } from '../src/defaults.js';
import { env, MemoryStorage, MockCloud } from './helpers.js';
const cfgFor = (tier) =>
  applyProfile(
    configuration({
      ...env,
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      MAX_PRICE_USD_HOUR: '2',
      MAX_CONCURRENT_RENTALS: '9',
    }),
    tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
    tier,
  );
const offer = (id, compute, inet, disk = 120) => ({
  ...new MockCloud().offer,
  id,
  machine_id: 80000 + id,
  gpu_name: 'RTX_4070',
  compute_cap: 890,
  geolocation: 'Texas, US',
  dph_base: compute,
  storage_cost: 0.12,
  dph_total: compute + (0.12 * disk) / 720,
  disk_space: 400,
  inet_down_cost: inet,
  inet_up_cost: 0,
});
test('profile resources: four data profiles include conservative stage allowances', () => {
  for (const tier of ['small', 'middle', 'large', 'adhoc']) {
    const r = resourceCaps(
      cfgFor(tier),
      tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
      tier,
    );
    assert.deepEqual(r, DEFAULTS.payloadProfiles[tier]);
  }
  assert.equal(cfgFor('large').diskGb, 300);
  assert.equal(cfgFor('large').bandwidthDown, 130);
  assert.equal(cfgFor('large').maxInet, 0.02);
  assert.equal(cfgFor('small').maxInet, 0.005);
  assert.equal(cfgFor('middle').maxInet, 0.005);
});
for (const tier of ['small', 'large'])
  test('profile ranking: compute/network tradeoff ' + tier, async () => {
    const cfg = cfgFor(tier),
      disk = cfg.diskGb,
      rows = [offer(1, 0.1, 0.005, disk), offer(2, 0.2, 0, disk)];
    const book = new OfferBook(() => 0),
      ledger = new Ledger(new MemoryStorage());
    const profile = tier === 'large' ? 'release' : 'check';
    const evaluation = await book.evaluate(null, cfg, ledger, profile, { market: rows });
    assert.equal(evaluation.allowed.length, 2);
    const selected = await book.select(evaluation, cfg, ledger, profile, undefined, {
      tierName: tier,
      die: 'AD104',
    });
    assert.equal(selected.offer.id, tier === 'large' ? 2 : 1);
    const prices = rows.map((o) => offerPrice(o, cfg));
    assert.equal(prices[0].effective < prices[1].effective, tier === 'small');
    assert.equal(prices[0].bandwidth, cfg.bandwidthDown * 0.005);
  });
for (const tier of ['small', 'large'])
  test('network cap survives effective hourly headroom ' + tier, () => {
    const cfg = cfgFor(tier),
      profile = tier === 'large' ? 'release' : 'check';
    const high = offer(1, 0.1, cfg.maxInet + 0.00001, cfg.diskGb);
    assert.ok(offerPrice(high, cfg).effective < 2);
    assert.ok(offerRejections(high, cfg, { profile }).includes('PRICE'));
    const low = offer(2, 0.1, 0.0075, cfg.diskGb);
    assert.equal(offerRejections(low, cfg, { profile }).includes('PRICE'), tier === 'small');
  });
for (const tier of ['small', 'large'])
  test('provider request uses profile disk and per-GB caps ' + tier, async () => {
    const cfg = cfgFor(tier),
      bodies = [];
    const vast = new Vast(
      async (url, init) => {
        bodies.push(JSON.parse(init.body));
        return Response.json({ offers: [] });
      },
      cfg,
      'test-only',
    );
    await vast.offers();
    assert.equal(bodies[0].allocated_storage, cfg.diskGb);
    assert.equal(bodies[0].disk_space.gte, cfg.diskGb);
    assert.equal(bodies[0].inet_down_cost.lte, cfg.maxInet);
    assert.equal(bodies[0].inet_up_cost.lte, cfg.maxInet);
  });

import { offlinePlan } from '../release-plan.mjs';
import { readFileSync } from 'node:fs';
import { setup, sha } from './helpers.js';
test('release estimate reserves the large allowance on every full leg', async () => {
  const market = JSON.parse(
    readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
  ).offers;
  const plan = await offlinePlan(market, { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' });
  assert.equal(plan.legs, 8);
  const cfg = cfgFor('large');
  const expected = plan.families
    .flatMap((f) => f.cells.flatMap((c) => c.legs))
    .reduce(
      (sum, l) =>
        sum +
        offerPrice(
          market.find((o) => o.id === l.offer_id),
          cfg,
        ).total,
      0,
    );
  assert.ok(Math.abs(plan.estimate_usd - expected) < 0.002);
  assert.equal(plan.resource_profile.bandwidthDownGb, 130);
  assert.equal(plan.resource_profile.diskGb, 300);
});
test('a shared offer cache is refreshed across resource query boundaries', async () => {
  const book = new OfferBook(() => 0),
    ledger = new Ledger(new MemoryStorage());
  let calls = 0;
  const vast = {
    offers: async () => {
      calls++;
      return [];
    },
  };
  await book.evaluate(vast, cfgFor('small'), ledger, 'check');
  await book.evaluate(vast, cfgFor('large'), ledger, 'release');
  assert.equal(calls, 2);
});
for (const changed of ['inet_down_cost', 'inet_up_cost'])
  test('live instance rechecks per-GB cap ' + changed, async () => {
    const h = await setup({
      MAX_PRICE_USD_HOUR: '2',
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      DAILY_SPEND_USD: '20',
      ROLLING_SPEND_USD: '200',
    });
    assert.equal((await h.call()).status, 200);
    h.cloud.instances.get('900')[changed] = 0.00501;
    await h.service.sweep();
    const record = await h.storage.get('record:123-1');
    assert.equal(record.closed, 'CONTRACT_PRICE');
    assert.ok(h.cloud.deletes.includes('900') || h.cloud.deletes.includes(900));
  });

for (const tier of ['small', 'large'])
  test('outbound unit cap is independent of price ' + tier, () => {
    const cfg = cfgFor(tier),
      profile = tier === 'large' ? 'release' : 'check';
    const o = { ...offer(1, 0.1, 0, cfg.diskGb), inet_up_cost: cfg.maxInet + 0.00001 };
    assert.ok(offerPrice(o, cfg).effective < 2);
    assert.ok(offerRejections(o, cfg, { profile }).includes('PRICE'));
  });
test('large storage price uses 300 GB in the hourly component', () => {
  const cfg = cfgFor('large');
  const p = offerPrice(offer(1, 0.1, 0, 300), cfg);
  assert.ok(p);
  assert.ok(Math.abs(p.computeStorage - 0.15) < 1e-12);
});
test('large storage refuses a total quoted for only 120 GB', () => {
  assert.equal(offerPrice(offer(1, 0.1, 0, 120), cfgFor('large')), null);
});
test('provider create reserves the large-profile disk', async () => {
  const cfg = cfgFor('large');
  let body;
  const vast = new Vast(
    async (url, init) => {
      body = JSON.parse(init.body);
      return Response.json({ success: true, new_contract: 123 });
    },
    cfg,
    'test-only',
  );
  await vast.create({ id: 1 }, { label: 'test-only' }, 'echo test', {});
  assert.equal(body.disk, 300);
});

import { dieLegs } from '../src/release.js';
for (const tier of ['small', 'large'])
  test('release leg ordering uses the profile effective price ' + tier, () => {
    const cfg = cfgFor(tier);
    const allowed = [offer(1, 0.1, 0.005, cfg.diskGb), offer(2, 0.2, 0, cfg.diskGb)].map((o) => ({
      ...o,
      die: 'AD104',
      country: 'US',
      pricing: offerPrice(o, cfg),
    }));
    assert.equal(dieLegs(allowed, cfg)[0].offer_id, tier === 'large' ? 2 : 1);
  });
test('release leg ordering beats cheaper compute for a second large network fixture', () => {
  const cfg = cfgFor('large');
  const allowed = [offer(3, 0.15, 0.004, cfg.diskGb), offer(4, 0.23, 0, cfg.diskGb)].map((o) => ({
    ...o,
    die: 'AD104',
    country: 'US',
    pricing: offerPrice(o, cfg),
  }));
  assert.equal(dieLegs(allowed, cfg)[0].offer_id, 4);
});

async function largeService() {
  const h = await setup({
    MAX_CONCURRENT_RENTALS: '9',
    RELEASE_ENABLED: 'true',
    ROLLING_SPEND_USD: '200',
    ACCOUNT_DAILY_USD: '50',
  });
  h.cloud.credit = 500;
  const market = JSON.parse(
    readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
  ).offers;
  h.cloud.extraOffers = market.map((o) => ({ ...o, price: o.dph_total }));
  h.cloud.contracts = Array.from({ length: 30 }, (_, i) => 900 + i);
  const batch = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  assert.equal(h.cloud.creates, 8);
  const stored = await h.storage.get('batch:' + batch.batch);
  const first = await h.storage.get('record:' + stored.legs[0].key);
  return { h, first };
}
for (const field of ['inet_down_cost', 'inet_up_cost'])
  test('large live-instance cap remains independently enforced ' + field, async () => {
    const { h, first } = await largeService();
    h.cloud.instances.get(String(first.id))[field] = 0.02001;
    await h.service.sweep();
    assert.equal((await h.storage.get('record:' + first.key)).closed, 'CONTRACT_PRICE');
  });
test('large live-instance recheck permits rates above the small cap', async () => {
  const { h, first } = await largeService();
  h.cloud.instances.get(String(first.id)).inet_down_cost = 0.0075;
  await h.service.sweep();
  assert.equal((await h.storage.get('record:' + first.key)).closed, null);
});

// FALSIFIER: the generous sanity cap excludes a rate below .02, or ranking
// treats dear network as an automatic loser despite a lower total effective price.
test('large sanity cap admits dear inbound when its effective price wins', async () => {
  const cfg = cfgFor('large');
  const rows = [offer(81, 0.1, 0.015, cfg.diskGb), offer(82, 0.6, 0, cfg.diskGb)];
  const book = new OfferBook(() => 0),
    ledger = new Ledger(new MemoryStorage());
  const evaluation = await book.evaluate(null, cfg, ledger, 'release', { market: rows });
  assert.equal(evaluation.allowed.length, 2);
  const chosen = await book.select(evaluation, cfg, ledger, 'release', undefined, { die: 'AD104' });
  assert.equal(chosen.offer.id, 81);
  assert.ok(chosen.offer.pricing.total < offerPrice(rows[1], cfg).total);
});
test('large release plan permits the cheaper box with dear inbound', async () => {
  const cfg = cfgFor('large');
  const rows = [offer(83, 0.1, 0.019, cfg.diskGb), offer(84, 0.7, 0, cfg.diskGb)];
  const evaluation = await new OfferBook(() => 0).evaluate(
    null,
    cfg,
    new Ledger(new MemoryStorage()),
    'release',
    { market: rows },
  );
  assert.equal(evaluation.allowed.length, 2);
  assert.equal(dieLegs(evaluation.allowed, cfg)[0].offer_id, 83);
});
