import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { configuration, offerRejections } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { parseVars } from '../deploy.mjs';
import { setup, env, nonce, sampleResult } from './helpers.js';
// Owner rulings 2026-10-24 items 10 and 10b: for VM (KVM) instances the host's reported Vast
// driver_version is irrelevant (same KVM image everywhere; the guest loads its own driver) and
// misleading when shown as the guest's. OFFER_DRIVER_TAGS and the DRIVER rejection go; the
// status driver_version is only the box-reported guest driver (`unknown` before a result); the
// selection does not use the offer's host driver. Owner 2026-10-26 now asks for its separate
// public diagnostic label, which must never be confused with the box-reported driver.
// FALSIFIER: an offer with an unlisted or missing host driver_version is rejected; a DRIVER
// code or OFFER_DRIVER_TAGS remains in the Worker, the runner or the deploy helper; the status,
// guest-driver status or selection semantics change when the public host label is added.
const broker = new URL('../', import.meta.url);
const read = (path) => readFile(new URL(path, broker), 'utf8');
const hostDriver = '570.124.06'; // Public offer-side label, separate from the guest field.
const availability = (h) => h.call('/availability?profile=check', { method: 'GET' });

for (const value of ['999.1.1', '1.0', undefined, null, 570, 'garbage'])
  test(
    'item 10: an offer with host driver_version ' + JSON.stringify(value) + ' is admitted',
    async () => {
      const h = await setup();
      if (value === undefined) delete h.cloud.offer.driver_version;
      else h.cloud.offer.driver_version = value;
      const data = await (await availability(h)).json();
      assert.deepEqual(data.rejected, {});
      assert.equal(data.offers.length, 1);
      assert.equal((await h.call()).status, 200);
      assert.equal(h.cloud.creates, 1);
    },
  );
test('item 10: offerRejections never reads the host driver', () => {
  const cfg = configuration(env);
  assert.equal('driverTags' in cfg, false);
  assert.equal('driverTags' in DEFAULTS, false);
  const offer = new Proxy(
    { driver_version: '999.9.9' },
    {
      get(target, key) {
        assert.notEqual(key, 'driver_version', 'selection read the host driver');
        return target[key];
      },
    },
  );
  offerRejections(offer, cfg);
});
test('item 10: OFFER_DRIVER_TAGS is gone from the Worker, the helper and the docs', async () => {
  // A leftover var in an old deployment is ignored, never a configuration error.
  assert.doesNotThrow(() => configuration({ ...env, OFFER_DRIVER_TAGS: 'not json' }));
  assert.throws(() => parseVars('OWNER_GITHUB_ID=229738317\nOFFER_DRIVER_TAGS=["610.43.02"]\n'));
  for (const path of [
    'src/policy.js',
    'src/offers.js',
    'src/index.js',
    'defaults.json',
    'deploy.mjs',
    'wrangler.toml',
    'DEPLOY.md',
    'README.md',
    'runner.py',
  ]) {
    const text = await read(path);
    assert.doesNotMatch(text, /OFFER_DRIVER_TAGS|driverTags/, path);
    assert.doesNotMatch(text, /'DRIVER'|"DRIVER"/, path);
  }
});
test('item 10: the runner knows no DRIVER rejection code', () => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('edits2_identity_fixture.py', import.meta.url).pathname, 'codes'],
    { encoding: 'utf8' },
  );
  assert.equal(p.status, 0, p.stderr);
  assert.equal(JSON.parse(p.stdout).includes('DRIVER'), false);
});
test('item 10b: status says unknown until the box result, then the guest driver', async () => {
  const h = await setup();
  h.cloud.offer.driver_version = hostDriver;
  const rented = await (await h.call()).json();
  assert.equal(rented.driver_version, 'unknown');
  assert.equal((await (await h.call('/status/123-1')).json()).driver_version, 'unknown');
  const result = { seq: 1, kind: 'result', result: { nonce, verdict: 'PASS', ...sampleResult() } };
  assert.equal((await h.report(result)).status, 200);
  const after = await (await h.call('/status/123-1')).json();
  assert.equal(after.driver_version, sampleResult().hardware.driver_version);
});
test('public host label is separate from the guest driver and selection', async () => {
  const h = await setup();
  h.cloud.offer.driver_version = hostDriver;
  assert.ok(!(await (await availability(h)).text()).includes(hostDriver));
  for (const path of ['/rent', '/status/123-1', '/stop/123-1']) {
    const data = await (await h.call(path)).json();
    assert.equal(data.host_driver_version, hostDriver);
    assert.equal(data.driver_version, 'unknown');
    assert.equal('driver' in data, false);
  }
  const record = await h.storage.get('record:123-1');
  assert.equal(record.hostDriverVersion, hostDriver);
  assert.equal('driver' in record, false);
});
test('item 10b: availability rows carry no driver field', async () => {
  const h = await setup();
  h.cloud.offer.driver_version = hostDriver;
  h.cloud.extraOffers = [{ ...h.cloud.offer, id: 6, machine_id: 100002, reliability: 0.5 }];
  const data = await (await availability(h)).json();
  assert.equal(data.offers.length, 1);
  assert.equal(data.rejected_offers.length, 1);
  for (const row of [...data.offers, ...data.rejected_offers])
    assert.equal(
      Object.keys(row).some((k) => /driver/i.test(k)),
      false,
    );
});
