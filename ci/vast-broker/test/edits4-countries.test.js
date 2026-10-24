import test from 'node:test';
import assert from 'node:assert/strict';
import { OfferBook, offerCountry } from '../src/offers.js';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { env, clock, MemoryStorage } from './helpers.js';
// Review of pass 3 (2026-10-24), T2 and T3 (format characters in country names). Characters
// are written as \u escapes in this source on purpose.
// T3: a Cc/Cf character in or around a denied country must not turn it into an admitted UNKNOWN
// location when ALLOW_UNKNOWN_COUNTRY=true (the owner opt-in): normal() strips them before the
// denied-name match. At the default the format-character guard rejects it anyway, so the opt-in
// is the only configuration that tests the strip.
// T2: the invisible-character strip in clean() has no safety effect on a denied country (normal()
// strips Cf too); what it decides is whether an ALLOWED code is read, so the test pins that.
// FALSIFIER: a denied country with a format or control character is admitted under the opt-in.
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
const DENIED = [
  ['Beijing, \u2066CN\u2069', 'CN'], // left-to-right isolate ... pop directional isolate
  ['Beijing, \u200eCN', 'CN'], // left-to-right mark
  ['Beijing, CN\u200f', 'CN'], // right-to-left mark
  ['Beijing, C\u2062N', 'CN'], // invisible times
  ['Beijing, CN\u0007', 'CN'], // a control character (BEL)
  ['Beijing, \u202aCN\u202c', 'CN'], // embedding controls
  ['Tehran, \u2066IR\u2069', 'IR'],
  ['\u2066China\u2069', 'CN'], // a long name
  ['Hong\u200e Kong', 'HK'],
];
for (const [geolocation, code] of DENIED) {
  test('T3: ' + JSON.stringify(geolocation) + ' is ' + code, () => {
    assert.equal(offerCountry({ geolocation }), code);
  });
  test(
    'T3: ' + JSON.stringify(geolocation) + ' is rejected, also when unknown is allowed',
    async () => {
      for (const allow of ['false', 'true']) {
        const result = await evaluate(geolocation, { ALLOW_UNKNOWN_COUNTRY: allow });
        assert.equal(result.allowed.length, 0, allow);
        assert.deepEqual(result.view.rejected, { COUNTRY: 1 }, allow);
      }
    },
  );
}
