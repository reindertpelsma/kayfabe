import test from 'node:test';
import assert from 'node:assert/strict';
import { OfferBook, offerCountry } from '../src/offers.js';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { DEFAULTS } from '../src/defaults.js';
import { setup, env, clock, MemoryStorage } from './helpers.js';
// Owner ruling 2026-10-24 item 5: boxes in CN, HK and MO repeatedly had broken or very slow
// CDN/Ubuntu-mirror access. FALSIFIER: an offer there is admitted (even when cheapest or
// preferred), an alias is not recognised, an invalid override is accepted, or unknown
// location handling changes.
const base = {
  id: 1,
  verified: true,
  vms_enabled: true,
  rentable: true,
  rented: false,
  num_gpus: 1,
  gpu_name: 'RTX 3060',
  gpu_arch: 'nvidia',
  compute_cap: 860,
  machine_id: 100001,
  driver_version: '580.95.05',
  dlperf: 12,
  cpu_cores_effective: 16,
  cpu_ghz: 3,
  cpu_ram: 65536,
  inet_down: 1000,
  inet_up: 100,
  disk_bw: 500,
  disk_space: 200,
  reliability: 0.999,
  dph_base: 0.1,
  storage_cost: 0.12,
  dph_total: 0.12,
  inet_down_cost: 0.001,
  inet_up_cost: 0.001,
};
const offer = (id, geolocation, extra = {}) => ({ ...base, id, geolocation, ...extra });
const pricier = { dph_base: 0.2, dph_total: 0.22 };
async function evaluate(rows, extra = {}) {
  const cfg = configuration({
    ...env,
    MAX_PRICE_USD_HOUR: '1',
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
    ...extra,
  });
  const book = new OfferBook(() => clock),
    ledger = new Ledger(new MemoryStorage()),
    evaluation = await book.evaluate({ offers: async () => rows }, cfg, ledger, 'check');
  return { book, cfg, ledger, evaluation };
}
test('item 5: default deny list is CU IR KP SY CN HK MO', () => {
  assert.deepEqual(DEFAULTS.deniedCountries, ['CU', 'IR', 'KP', 'SY', 'CN', 'HK', 'MO']);
  assert.deepEqual(configuration(env).deniedCountries, DEFAULTS.deniedCountries);
});
for (const [geolocation, code] of [
  ['Beijing, CN', 'CN'],
  ['Shanghai, China', 'CN'],
  ['Kowloon, HK', 'HK'],
  ['Hong Kong, Hong Kong', 'HK'],
  ['Macau, MO', 'MO'],
  ['Taipa, Macau', 'MO'],
  ['Taipa, MACAO', 'MO'],
  ['Beijing, china', 'CN'],
  ['China', 'CN'],
])
  test('item 5: ' + JSON.stringify(geolocation) + ' is ' + code + ' and denied', async () => {
    assert.equal(offerCountry({ geolocation }), code);
    // The denied offer is the cheapest and its country the preferred one: still rejected.
    const { book, cfg, ledger, evaluation } = await evaluate(
      [offer(1, geolocation), offer(2, 'Austin, US', pricier)],
      { PREFERRED_COUNTRIES: JSON.stringify([code, 'US']) },
    );
    assert.deepEqual(
      evaluation.allowed.map((x) => x.id),
      [2],
    );
    assert.equal(evaluation.view.rejected.COUNTRY, 1);
    assert.equal((await book.select(evaluation, cfg, ledger, 'check')).offer.id, 2);
  });
for (const code of ['CN', 'HK', 'MO'])
  test('item 5: a rental in ' + code + ' alone makes no Vast create', async () => {
    const h = await setup({ PREFERRED_COUNTRIES: JSON.stringify([code]) });
    h.cloud.offer.geolocation = 'Region, ' + code;
    const response = await h.call();
    assert.equal(response.status, 503);
    assert.equal(h.cloud.creates, 0);
  });
for (const value of ['["cn"]', '["CN","CN"]', '["CHN"]', '"CN"', '[1]'])
  test('item 5: DENIED_COUNTRIES override stays validated ' + value, () => {
    assert.throws(() => configuration({ ...env, DENIED_COUNTRIES: value }));
  });
test('item 5: a valid DENIED_COUNTRIES override replaces the default', async () => {
  const { evaluation } = await evaluate([offer(1, 'Kowloon, HK'), offer(2, 'Beijing, CN')], {
    DENIED_COUNTRIES: '["CN"]',
  });
  assert.deepEqual(
    evaluation.allowed.map((x) => x.id),
    [1],
  );
});
// Edits3 C supersedes "unknown location handling unchanged": UNKNOWN is rejected by default
// and admitted only when the owner sets ALLOW_UNKNOWN_COUNTRY=true.
for (const geolocation of [undefined, '', 'Region, XYZ', 'Region, Atlantis', 'Region, us'])
  test(
    'item 5: unknown location is rejected by default ' + JSON.stringify(geolocation),
    async () => {
      assert.equal(offerCountry({ geolocation }), 'UNKNOWN');
      assert.equal((await evaluate([offer(1, geolocation)])).evaluation.allowed.length, 0);
      const lax = await evaluate([offer(1, geolocation)], { ALLOW_UNKNOWN_COUNTRY: 'true' });
      assert.equal(lax.evaluation.allowed.length, 1);
    },
  );
// Review V13 (2026-10-23): a long country name must be recognised and denied, never admitted as
// UNKNOWN. FALSIFIER: any form below maps to UNKNOWN or the wrong code, or a non-denied place
// (Taiwan) is denied.
for (const [geolocation, code] of [
  ['Damascus, Syrian Arab Republic', 'SY'],
  ['Damascus, SYR', 'SY'],
  ['Tehran, Iran, Islamic Republic of', 'IR'],
  ['Tehran, Islamic Republic of Iran', 'IR'],
  ['Tehran, Iran (Islamic Republic of)', 'IR'],
  ["Pyongyang, Korea, Democratic People's Republic of", 'KP'],
  ['Pyongyang, Korea, Democratic People’s Republic of', 'KP'],
  ["Pyongyang, Democratic People's Republic of Korea", 'KP'],
  ['Pyongyang, DPRK', 'KP'],
  ['Havana, Republic of Cuba', 'CU'],
  ["Beijing, People's Republic of China", 'CN'],
  ["Beijing, China, People's Republic of", 'CN'],
  ['Beijing, CHN', 'CN'],
  ['Central, Hong Kong SAR', 'HK'],
  ['Central, Hong Kong SAR, China', 'HK'],
  ['Central, hong kong sar china', 'HK'],
  ['Kowloon, HKG', 'HK'],
  ['Taipa, Macao', 'MO'],
  ['Taipa, Macao SAR', 'MO'],
  ['Taipa, Macao SAR, China', 'MO'],
  ['Taipa, Macau Special Administrative Region', 'MO'],
  ['  Taipa ,   macao   sar  ', 'MO'],
])
  test('V13: ' + JSON.stringify(geolocation) + ' is ' + code + ' and denied', async () => {
    assert.equal(offerCountry({ geolocation }), code);
    const { evaluation } = await evaluate([offer(1, geolocation)]);
    assert.equal(evaluation.allowed.length, 0);
    assert.equal(evaluation.view.rejected.COUNTRY, 1);
  });
for (const [geolocation, code] of [
  ['Taipei, TW', 'TW'],
  ['Taipei, Taiwan', 'UNKNOWN'],
  ['Seoul, KR', 'KR'],
  ['Seoul, Korea, Republic of', 'UNKNOWN'],
])
  test('V13: ' + JSON.stringify(geolocation) + ' stays ' + code + ', not denied', async () => {
    assert.equal(offerCountry({ geolocation }), code);
    // Edits3 C: an UNKNOWN location is admitted only with ALLOW_UNKNOWN_COUNTRY=true.
    const extra = code === 'UNKNOWN' ? { ALLOW_UNKNOWN_COUNTRY: 'true' } : {};
    assert.equal((await evaluate([offer(1, geolocation)], extra)).evaluation.allowed.length, 1);
  });
