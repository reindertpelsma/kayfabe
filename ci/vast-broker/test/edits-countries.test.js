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
for (const geolocation of [undefined, '', 'Region, CHN', 'Region, Atlantis', 'Region, us'])
  test('item 5: unknown location handling unchanged ' + JSON.stringify(geolocation), async () => {
    assert.equal(offerCountry({ geolocation }), 'UNKNOWN');
    assert.equal((await evaluate([offer(1, geolocation)])).evaluation.allowed.length, 1);
    const strict = await evaluate([offer(1, geolocation)], { ALLOW_UNKNOWN_COUNTRY: 'false' });
    assert.equal(strict.evaluation.allowed.length, 0);
  });
