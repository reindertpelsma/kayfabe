import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { OfferBook } from '../src/offers.js';
import { configuration } from '../src/policy.js';
import { Ledger } from '../src/ledger.js';
import { setup, env, clock, MemoryStorage } from './helpers.js';
// FALSIFIER: location admits a failing host, excludes all non-US hosts, overrides
// forced die preferences, or carries unchecked provider text into the runner.
const fixture = JSON.parse(
  await readFile(new URL('fixtures/v5-offers.json', import.meta.url), 'utf8'),
);
function offer(id, country, extra = {}) {
  return {
    ...fixture.offers.find((o) => o.gpu_name === 'RTX 3060'),
    id,
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
    geolocation: 'Region, ' + country,
    ...extra,
  };
}
async function choose(rows, extra = {}) {
  const cfg = configuration({
    ...env,
    MAX_PRICE_USD_HOUR: '1',
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
    ...extra,
  });
  const book = new OfferBook(() => clock);
  const ledger = new Ledger(new MemoryStorage());
  const evaluation = await book.evaluate({ offers: async () => rows }, cfg, ledger, 'check');
  return (await book.select(evaluation, cfg, ledger, 'check')).offer;
}
for (const [preferred, other] of [
  ['US', 'CA'],
  ['CA', 'GB'],
])
  test('country ranks qualifying ' + preferred + ' before ' + other, async () => {
    const selected = await choose([
      offer(1, other),
      offer(2, preferred, { dph_base: 0.11, dph_total: 0.13 }),
    ]);
    assert.equal(selected.id, 2);
  });
for (const reversed of [false, true])
  test('country order is configurable ' + reversed, async () => {
    const rows = [offer(1, 'US'), offer(2, 'CA', { dph_base: 0.11, dph_total: 0.13 })];
    if (reversed) rows.reverse();
    assert.equal((await choose(rows, { PREFERRED_COUNTRIES: '["CA","US"]' })).id, 2);
  });
for (const country of ['CA', 'GB'])
  test('qualifying non-US fallback ' + country, async () => {
    assert.equal((await choose([offer(1, country)])).id, 1);
  });
for (const [field, value] of [
  ['verified', false],
  ['reliability2', 0.1],
  ['cpu_cores_effective', 1],
  ['inet_down', 1],
  ['disk_bw', 1],
  ['has_vm', false],
  ['dlperf', 0],
  ['dph_total', 2],
  ['driver_version', '1.0'],
])
  test('US never bypasses required ' + field, async () => {
    const selected = await choose([offer(1, 'US', { [field]: value }), offer(2, 'CA')]);
    assert.equal(selected.id, 2);
  });
for (const country of ['US', 'CA'])
  test('chosen country is structured metadata ' + country, async () => {
    const h = await setup();
    h.cloud.offer.geolocation = 'Region, ' + country;
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).country, country);
  });
for (const value of ['["us"]', '["US","US"]', 'false'])
  test('country config fails closed ' + value, async () => {
    const h = await setup({ PREFERRED_COUNTRIES: value });
    assert.equal((await h.call()).status, 503);
    assert.equal(h.cloud.creates, 0);
  });
for (const country of ['https://attacker.invalid', 'US\n::error::x'])
  test('runner refuses hostile country ' + JSON.stringify(country), async () => {
    const h = await setup();
    const response = await h.call();
    const data = await response.json();
    const code = `import importlib.util,json,sys
s=importlib.util.spec_from_file_location('runner',sys.argv[1]);m=importlib.util.module_from_spec(s);sys.modules['runner']=m;s.loader.exec_module(m)
d=json.loads(sys.stdin.read());ctx={'run':d['run'],'sha':d['sha'],'pr':d['pr'],'nonce':d['runner_nonce']};manifest={k:d[k] for k in ['protocol_version','worker_version','bundle_sha256']};m.validate_status(d,ctx,manifest,200);d['country']=sys.argv[2]
try:m.validate_status(d,ctx,manifest,200)
except m.Invalid:pass
else:raise AssertionError('hostile country accepted')
`;
    const p = spawnSync(
      'python3',
      ['-I', '-c', code, new URL('../runner.py', import.meta.url).pathname, country],
      { input: JSON.stringify(data), encoding: 'utf8' },
    );
    assert.equal(p.status, 0, p.stderr);
  });
