import test from 'node:test';
import assert from 'node:assert/strict';
import { OfferBook, offerCountry } from '../src/offers.js';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { setup, env, clock, MemoryStorage } from './helpers.js';
// Edits3 C (delta review C1): unknown or unparseable geolocation is rejected by default
// (ALLOW_UNKNOWN_COUNTRY defaults to false, fail closed). The parser NFKC-normalises, strips
// invisible characters, accepts exactly Vast's `region, CC` (CC two ASCII capitals) plus the
// known long names of denied countries, and calls anything else UNKNOWN.
// FALSIFIER: a measured spelling of a denied country is admitted; a look-alike or invisible
// character turns a code into an admitted one; an UNKNOWN location is admitted by default.
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
async function evaluate(geolocation, extra = {}) {
  const cfg = configuration({
    ...env,
    MAX_PRICE_USD_HOUR: '1',
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
    ...extra,
  });
  const offer = { ...base, ...(geolocation === undefined ? {} : { geolocation }) };
  const book = new OfferBook(() => clock);
  return book.evaluate(
    { offers: async () => [offer] },
    cfg,
    new Ledger(new MemoryStorage()),
    'check',
  );
}
// The review's measured spellings (delta_probe G), with what each must now be.
const MEASURED = [
  ['P.R. China', 'CN'],
  ['PR China', 'CN'],
  ['China (Mainland)', 'CN'],
  ['Beijing, China.', 'CN'],
  ['Hong Kong (China)', 'HK'],
  ['HongKong', 'HK'],
  ['Hong-Kong', 'HK'],
  ['Persia', 'IR'],
  ['IRAN.', 'IR'],
  ['Korea, DPR', 'KP'],
  ['Syrian Arab Rep.', 'SY'],
  ['Beijing, C\u039d', 'UNKNOWN'], // Greek capital Nu: a look-alike, not a code
  ['Beijing, \uff23\uff2e', 'CN'], // full-width CN folds under NFKC
  ['Beijing, CN\u200b', 'CN'], // zero-width space stripped
  ['Beijing,\u00a0CN', 'CN'], // NBSP is a space
  ['US', 'UNKNOWN'], // bare code: not the Vast form
  ['Taiwan, Province of China', 'UNKNOWN'],
  ['Korea, Republic of', 'UNKNOWN'],
];
const MORE = [
  ['\ufeffBeijing, CN', 'CN'], // byte-order mark
  ['Beijing, C\u200dN', 'CN'], // zero-width joiner inside the code
  ['Beijing, C\u00adN', 'CN'], // soft hyphen inside the code
  ['Beijing, C\u2060N', 'CN'], // word joiner
  ['Beijing, \u202eNC', 'UNKNOWN'], // bidi override: a reordered code is never trusted
  ['Beijing, \u0421N', 'UNKNOWN'], // Cyrillic Es
  ['Texas, U\u0405', 'UNKNOWN'], // Cyrillic Dze looks like S
  ['Region, us', 'UNKNOWN'], // lower case is not the Vast form
  ['Region, USA', 'UNKNOWN'],
  [', US', 'UNKNOWN'], // no region
  ['Texas,\tUS', 'US'],
  ['Texas, US', 'US'],
  ['Seoul, KR', 'KR'],
  ['Taiwan, TW', 'TW'],
  ['Tehran, \u0131r', 'IR'], // dotless i: denied matching folds case (fail closed)
  ['Tex\u202eas, US', 'UNKNOWN'], // a bidi control anywhere: never an allowed code
  ['Texas\u0007, US', 'UNKNOWN'], // a control character anywhere
];
for (const [geolocation, code] of [...MEASURED, ...MORE])
  test('C: ' + JSON.stringify(geolocation) + ' is ' + code, () => {
    assert.equal(offerCountry({ geolocation }), code);
  });
test('C: ALLOW_UNKNOWN_COUNTRY defaults to false', () => {
  assert.equal(configuration(env).allowUnknownCountry, false);
  assert.equal(configuration({ ...env, ALLOW_UNKNOWN_COUNTRY: 'true' }).allowUnknownCountry, true);
  assert.throws(() => configuration({ ...env, ALLOW_UNKNOWN_COUNTRY: 'yes' }));
});
for (const [geolocation, code] of [...MEASURED, ...MORE, [undefined, 'UNKNOWN'], ['', 'UNKNOWN']])
  test('C: ' + JSON.stringify(geolocation) + ' admission follows ' + code, async () => {
    const admitted = !['UNKNOWN', 'CU', 'IR', 'KP', 'SY', 'CN', 'HK', 'MO'].includes(code);
    const strict = await evaluate(geolocation);
    assert.equal(strict.allowed.length, admitted ? 1 : 0);
    if (!admitted) assert.deepEqual(strict.view.rejected, { COUNTRY: 1 });
    // The owner may opt back in to unknown locations; denied ones stay denied.
    const lax = await evaluate(geolocation, { ALLOW_UNKNOWN_COUNTRY: 'true' });
    assert.equal(lax.allowed.length, admitted || code === 'UNKNOWN' ? 1 : 0);
  });
test('C: a rent for an offer without a location is refused by default', async () => {
  const h = await setup();
  delete h.cloud.offer.geolocation;
  const r = await h.call();
  assert.equal(r.status, 503);
  assert.equal(h.cloud.creates, 0);
});
