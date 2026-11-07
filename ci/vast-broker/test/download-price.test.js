import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration, downloadTiming, offerPrice, offerRejections } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { applyProfile } from '../src/tier.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { dieLegs } from '../src/release.js';
import { env, MockCloud, MemoryStorage, setup, shippedPayloadProfiles } from './helpers.js';
// These model/admission tests use shipped profile data, independently of the
// faster origin explicitly simulated by the unrelated protocol fixtures.
for (const [name, profile] of Object.entries(shippedPayloadProfiles))
  Object.assign(DEFAULTS.payloadProfiles[name], profile);
const cfgFor = (tier = 'small', extra = {}) =>
  applyProfile(
    configuration({
      ...env,
      MAX_PRICE_USD_HOUR: '2',
      MAX_CONCURRENT_RENTALS: '9',
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      ...extra,
    }),
    tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
    tier,
  );
const offer = (speed, compute = 0.5, id = 1) => ({
  ...new MockCloud().offer,
  id,
  machine_id: 80000 + id,
  host_id: 90000 + id,
  inet_down: speed,
  dph_base: compute,
  dph_total: compute,
  storage_cost: 0,
  disk_space: 400,
  inet_down_cost: 0.002,
  inet_up_cost: 0.001,
});
const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} != ${b}`);
// FALSIFIERS: slow links receive no time premium; fast links receive a negative
// discount; the model ignores data; or planning/admission bypasses feasibility.
for (const compute of [0.5, 0.8])
  test('download time premium uses extra leased hours ' + compute, () => {
    const cfg = cfgFor('small', { BANDWIDTH_DOWN_GB: '100' });
    const price = offerPrice(offer(1000, compute), cfg);
    near(price.download.reference_seconds, (100 * 8000) / 445);
    near(price.download.expected_seconds, (100 * 8000) / 300);
    near(price.downloadTimeCost, (((100 * 8000) / 300 - (100 * 8000) / 445) / 3600) * compute);
    near(price.bandwidth, 0.202 + price.downloadTimeCost);
    near(price.effective, compute + price.bandwidth / 5);
    near(price.total, compute * (5 + 1 / 60) + price.bandwidth);
  });
for (const speed of [4000, 10000])
  test('fast link has zero extra-time charge ' + speed, () => {
    const cfg = cfgFor('small', { BANDWIDTH_DOWN_GB: '100' });
    const p = offerPrice(offer(speed), cfg);
    assert.equal(p.download.extra_seconds, 0);
    assert.equal(p.downloadTimeCost, 0);
    near(p.bandwidth, 0.202);
  });
for (const [tier, speed] of [
  ['large', 500],
  ['middle', 1500],
])
  test('feasibility rejects over-profile download time ' + tier, () => {
    const cfg = cfgFor(tier),
      profile = tier === 'large' ? 'release' : 'check';
    const reasons = offerRejections(offer(speed), cfg, { profile });
    assert.ok(reasons.includes('SLOW_NETWORK'));
    assert.ok(!reasons.includes('PRICE'));
  });
for (const tier of ['small', 'large'])
  test('advertised 200 Mbps floor is not feasible ' + tier, () => {
    const cfg = cfgFor(tier),
      profile = tier === 'large' ? 'release' : 'check';
    assert.ok(offerRejections(offer(200), cfg, { profile }).includes('SLOW_NETWORK'));
  });
for (const speed of [0, -1, Number.MIN_VALUE, 1e-320, undefined, NaN, Infinity])
  test('missing or invalid speed cannot be priced ' + speed, () => {
    assert.equal(offerPrice(offer(speed), cfgFor()), null);
  });
for (const mode of ['price', 'guard'])
  test('derate is inferred data and changes ' + mode, () => {
    const original = DEFAULTS.downloadModel.derate;
    try {
      const cfg = cfgFor('large');
      const before = offerPrice(offer(500), cfg);
      DEFAULTS.downloadModel.derate = 0.6;
      const after = offerPrice(offer(500), cfg);
      if (mode === 'price')
        near(after.download.expected_seconds, before.download.expected_seconds / 2);
      else
        assert.ok(
          !offerRejections(offer(500), cfg, { profile: 'release' }).includes('SLOW_NETWORK'),
        );
    } finally {
      DEFAULTS.downloadModel.derate = original;
    }
  });
for (const tier of ['small', 'large'])
  test('R2 reference is profile data ' + tier, () => {
    const resource = DEFAULTS.payloadProfiles[tier],
      old = resource.r2ReferenceMbps;
    const other = tier === 'large' ? 'small' : 'large';
    try {
      const cfg = cfgFor(tier),
        before = offerPrice(offer(1000), cfg);
      const otherBefore = downloadTiming(offer(1000), cfgFor(other));
      resource.r2ReferenceMbps = 890;
      const after = offerPrice(offer(1000), cfg);
      near(after.download.reference_seconds, before.download.reference_seconds / 2);
      assert.ok(after.downloadTimeCost > before.downloadTimeCost);
      near(
        downloadTiming(offer(1000), cfgFor(other)).reference_seconds,
        otherBefore.reference_seconds,
      );
    } finally {
      resource.r2ReferenceMbps = old;
    }
  });
for (const mode of ['selection', 'release'])
  test('download time participates in effective-price ordering ' + mode, async () => {
    const cfg = cfgFor('large'),
      book = new OfferBook(() => 0),
      ledger = new Ledger(new MemoryStorage());
    const market = [offer(1000, 0.5, 1), offer(10000, 0.52, 2)];
    const evaluation = await book.evaluate(null, cfg, ledger, 'release', { market });
    assert.equal(evaluation.allowed.length, 2);
    const id =
      mode === 'release'
        ? dieLegs(evaluation.allowed, cfg)[0].offer_id
        : (await book.select(evaluation, cfg, ledger, 'release', undefined, { die: 'AD104' })).offer
            .id;
    assert.equal(id, 2);
  });
for (const speed of [200, 300])
  test('live contract uses the same feasibility guard ' + speed, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '30' });
    h.cloud.run.event = 'push';
    assert.equal((await h.call('/rent', { claims: { event_name: 'push' } })).status, 200);
    h.cloud.instances.get('900').inet_down = speed;
    await h.service.sweep();
    assert.equal((await h.storage.get('record:123-1')).closed, 'CONTRACT_SLOW_NETWORK');
  });
for (const missing of [undefined, null])
  test('live contract can reuse its recorded advertised speed ' + missing, async () => {
    const h = await setup({ HARD_RUNTIME_MINUTES: '30' });
    h.cloud.run.event = 'push';
    assert.equal((await h.call('/rent', { claims: { event_name: 'push' } })).status, 200);
    h.cloud.instances.get('900').inet_down = missing;
    await h.service.sweep();
    assert.equal((await h.storage.get('record:123-1')).closed, null);
  });
for (const tier of ['small', 'large'])
  test('profile download maximum is data-driven ' + tier, () => {
    const original = DEFAULTS.payloadProfiles[tier].maxDownloadSeconds;
    try {
      DEFAULTS.payloadProfiles[tier].maxDownloadSeconds = 10;
      const cfg = cfgFor(tier);
      assert.ok(
        offerRejections(offer(10000), cfg, {
          profile: tier === 'large' ? 'release' : 'check',
        }).includes('SLOW_NETWORK'),
      );
    } finally {
      DEFAULTS.payloadProfiles[tier].maxDownloadSeconds = original;
    }
  });
for (const rate of [0.12, 0.24])
  test('download time charges the storage-inclusive hourly rate ' + rate, () => {
    const cfg = cfgFor('large'),
      row = { ...offer(1000), storage_cost: rate, dph_total: 0.5 + (rate * 300) / 720 };
    const p = offerPrice(row, cfg);
    near(p.downloadTimeCost, (p.download.extra_seconds / 3600) * row.dph_total);
    assert.ok(p.downloadTimeCost > (p.download.extra_seconds / 3600) * row.dph_base);
  });
for (const key of ['derate', 'status'])
  test('invalid download model data refuses configuration ' + key, () => {
    const original = DEFAULTS.downloadModel[key];
    try {
      DEFAULTS.downloadModel[key] = 0;
      assert.throws(
        () => cfgFor(),
        (e) => e.state === 'CONFIG_VAR:DOWNLOAD_MODEL',
      );
    } finally {
      DEFAULTS.downloadModel[key] = original;
    }
  });
for (const tier of ['small', 'large'])
  test('invalid download limit refuses configuration ' + tier, () => {
    const original = DEFAULTS.payloadProfiles[tier].maxDownloadSeconds;
    try {
      DEFAULTS.payloadProfiles[tier].maxDownloadSeconds = 0;
      assert.throws(
        () => cfgFor(tier),
        (e) => e.state === 'CONFIG_VAR:DOWNLOAD_LIMIT',
      );
    } finally {
      DEFAULTS.payloadProfiles[tier].maxDownloadSeconds = original;
    }
  });
for (const speed of [1000, 1200])
  test('release leg reserves the shared time-inclusive estimate ' + speed, async () => {
    const cfg = cfgFor('large'),
      row = offer(speed),
      book = new OfferBook(() => 0),
      ledger = new Ledger(new MemoryStorage());
    const evaluation = await book.evaluate(null, cfg, ledger, 'release', { market: [row] });
    const leg = dieLegs(evaluation.allowed, cfg)[0];
    near(leg.est_usd, Math.round(offerPrice(row, cfg).total * 10000) / 10000);
  });

for (const [tier, minutes, speed] of [
  ['small', 20, 1000],
  ['middle', 11, 3000],
])
  test('download feasibility also fits a shortened lease ' + tier, () => {
    const cfg = cfgFor(tier, { HARD_RUNTIME_MINUTES: String(minutes), BOOT_LIMIT_MINUTES: '5' });
    assert.ok(offerRejections(offer(speed), cfg).includes('SLOW_NETWORK'));
  });

// FALSIFIERS: a fast advertisement exceeds R2's configured ceiling, streams change
// the single-stream reference, a profile borrows another profile's reference, or
// the feasibility guard ignores the origin's irreducible transfer duration.
for (const speed of [4000, 10000])
  test('R2 single-stream ceiling bounds fast offers ' + speed, () => {
    const p = offerPrice(offer(speed), cfgFor('large'));
    near(p.download.expected_seconds, (130 * 8000) / 445);
    near(p.download.reference_seconds, (130 * 8000) / 445);
    assert.equal(p.download.effective_down_mbps, 445);
    assert.equal(p.download.streams_factor, 1);
    assert.equal(p.download.r2_reference_mbps, 445);
    assert.equal(p.download.reference_status, 'PROPOSED');
    assert.equal(p.downloadTimeCost, 0);
  });
for (const tier of ['small', 'large'])
  test('streams factor is profile data with nonnegative premium ' + tier, () => {
    const resource = DEFAULTS.payloadProfiles[tier],
      old = resource.streamsFactor;
    try {
      const cfg = cfgFor(tier),
        before = downloadTiming(offer(10000), cfg);
      const other = tier === 'small' ? 'large' : 'small';
      const otherBefore = downloadTiming(offer(10000), cfgFor(other));
      resource.streamsFactor = 2;
      const p = offerPrice(offer(10000), cfg);
      near(p.download.expected_seconds, before.expected_seconds / 2);
      near(p.download.reference_seconds, before.reference_seconds);
      assert.equal(p.download.streams_factor, 2);
      near(
        downloadTiming(offer(10000), cfgFor(other)).expected_seconds,
        otherBefore.expected_seconds,
      );
      assert.equal(p.download.extra_seconds, 0);
      assert.equal(p.downloadTimeCost, 0);
    } finally {
      resource.streamsFactor = old;
    }
  });
for (const tier of ['small', 'large'])
  test('slower profile R2 ceiling can reject an otherwise fast link ' + tier, () => {
    const resource = DEFAULTS.payloadProfiles[tier],
      old = resource.r2ReferenceMbps;
    try {
      resource.r2ReferenceMbps = 100;
      const cfg = cfgFor(tier);
      assert.ok(
        offerRejections(offer(10000), cfg, {
          profile: tier === 'large' ? 'release' : 'check',
        }).includes('SLOW_NETWORK'),
      );
    } finally {
      resource.r2ReferenceMbps = old;
    }
  });
for (const tier of ['small', 'large'])
  for (const key of ['r2ReferenceMbps', 'streamsFactor'])
    for (const invalid of [0, NaN, Infinity, -1])
      test(
        'invalid profile download data refuses configuration ' + [tier, key, invalid].join('/'),
        () => {
          const resource = DEFAULTS.payloadProfiles[tier],
            old = resource[key];
          try {
            resource[key] = invalid;
            assert.throws(
              () => cfgFor(tier),
              (e) => e.state === 'CONFIG_VAR:DOWNLOAD_PROFILE',
            );
          } finally {
            resource[key] = old;
          }
        },
      );
for (const tier of ['small', 'middle', 'large', 'adhoc'])
  test('each shipped profile has proposed single-stream R2 data ' + tier, () => {
    const resource = DEFAULTS.payloadProfiles[tier];
    assert.equal(resource.r2ReferenceMbps, 445);
    assert.equal(resource.r2ReferenceStatus, 'PROPOSED');
    assert.equal(resource.streamsFactor, 1);
    assert.equal(resource.streamsFactorStatus, 'PROPOSED');
  });
for (const speed of [4000, 10000])
  test('middle origin minimum exceeds unchanged twenty-minute limit ' + speed, () => {
    const cfg = cfgFor('middle');
    near(downloadTiming(offer(speed), cfg).expected_seconds, (87 * 8000) / 445);
    assert.ok(offerRejections(offer(speed), cfg).includes('SLOW_NETWORK'));
  });
for (const speed of [4000, 10000])
  test('short twenty-minute small lease cannot fit the R2 minimum ' + speed, () => {
    const cfg = cfgFor('small', { HARD_RUNTIME_MINUTES: '20', BOOT_LIMIT_MINUTES: '5' });
    assert.ok(offerRejections(offer(speed), cfg).includes('SLOW_NETWORK'));
  });

for (const tier of ['small', 'large'])
  test('R2 expected ceiling stays within its profile ' + tier, () => {
    const resource = DEFAULTS.payloadProfiles[tier],
      old = resource.r2ReferenceMbps;
    const other = tier === 'small' ? 'large' : 'small';
    try {
      const before = downloadTiming(offer(10000), cfgFor(tier));
      const otherBefore = downloadTiming(offer(10000), cfgFor(other));
      resource.r2ReferenceMbps = 890;
      near(
        downloadTiming(offer(10000), cfgFor(tier)).expected_seconds,
        before.expected_seconds / 2,
      );
      near(
        downloadTiming(offer(10000), cfgFor(other)).expected_seconds,
        otherBefore.expected_seconds,
      );
    } finally {
      resource.r2ReferenceMbps = old;
    }
  });
