import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { configuration, offerRejections } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { parseVars } from '../deploy.mjs';
import { setup, env } from './helpers.js';
// Owner ruling 2026-10-24 item 6: a hoster-level blacklist. FALSIFIER: a listed host_id is
// admitted on any of its machines, an unlisted host is rejected, an invalid BAD_HOST_IDS is
// accepted or refused without naming the variable, the runner refuses the BAD_HOST count, or
// the owner cannot set the list through the deploy helper.
const offer = (id, host_id, machine_id) => ({
  id,
  host_id,
  machine_id,
  verified: true,
  vms_enabled: true,
  rentable: true,
  rented: false,
  num_gpus: 1,
  gpu_name: 'RTX 3060',
  gpu_arch: 'nvidia',
  compute_cap: 860,
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
});
const cfg = (extra = {}) =>
  configuration({
    ...env,
    MAX_PRICE_USD_HOUR: '1',
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
    ...extra,
  });
test('item 6: badHosts defaults to an empty list', () => {
  assert.deepEqual(DEFAULTS.badHosts, []);
  assert.deepEqual(cfg().badHosts, []);
});
test('item 6: a listed host is rejected with all its machines', () => {
  const c = cfg({ BAD_HOST_IDS: '[4242, 77]' });
  for (const machine of [100001, 100002, 100003])
    assert.ok(offerRejections(offer(1, 4242, machine), c).includes('BAD_HOST'), String(machine));
  assert.ok(offerRejections(offer(2, 77, 100004), c).includes('BAD_HOST'));
});
test('item 6: an unlisted host (or none given) is unaffected', () => {
  const c = cfg({ BAD_HOST_IDS: '[4242]' });
  assert.deepEqual(offerRejections(offer(1, 4243, 100001), c), []);
  assert.deepEqual(offerRejections(offer(1, undefined, 100001), c), []);
  assert.deepEqual(offerRejections(offer(1, 4242, 100001), cfg()), []);
});
for (const value of [
  '[0]',
  '[-1]',
  '[1.5]',
  '["4242"]',
  '{}',
  'x',
  '',
  JSON.stringify(Array.from({ length: 257 }, (_, i) => i + 1)),
])
  test('item 6: invalid BAD_HOST_IDS ' + value.slice(0, 12) + ' names the variable', async () => {
    assert.throws(
      () => cfg({ BAD_HOST_IDS: value }),
      (e) => e.state === 'CONFIG_VAR:BAD_HOST_IDS',
    );
    const h = await setup({ BAD_HOST_IDS: value });
    const response = await h.call();
    assert.equal(response.status, 503);
    assert.equal((await response.json()).state, 'CONFIG_VAR:BAD_HOST_IDS');
    assert.equal(h.cloud.creates, 0);
  });
test('item 6: 256 host ids are accepted', () => {
  const ids = Array.from({ length: 256 }, (_, i) => i + 1);
  assert.deepEqual(cfg({ BAD_HOST_IDS: JSON.stringify(ids) }).badHosts, ids);
});
test('item 6: a rental never lands on a listed host; availability counts BAD_HOST', async () => {
  const h = await setup({ BAD_HOST_IDS: '[4242]' });
  h.cloud.offer.host_id = 4242;
  const view = await (await h.call('/availability?profile=check', { method: 'GET' })).json();
  assert.equal(view.rejected.BAD_HOST, 1);
  assert.equal((await h.call()).status, 503);
  assert.equal(h.cloud.creates, 0);
  h.cloud.offer.host_id = 4243;
  h.restart();
  assert.equal((await h.call()).status, 200);
  assert.equal(h.cloud.creates, 1);
});
test('item 6: the runner accepts a BAD_HOST rejection count', () => {
  const code = `import importlib.util,json,sys
s=importlib.util.spec_from_file_location('runner',sys.argv[1]);m=importlib.util.module_from_spec(s)
sys.modules['runner']=m;s.loader.exec_module(m)
scope={};exec(open(sys.argv[2]).read(),scope)
data=scope['availability']();data['rejected']={'BAD_HOST':3,'MACHINE':1}
data['rejected_offers']=[{'machine_id':9,'host_id':4242,'reasons':['BAD_HOST','MACHINE']}]
data['rejected_offers']+=[{'machine_id':i,'host_id':4242,'reasons':['BAD_HOST']} for i in (7,8)]
m.validate_availability(data,'check')
`;
  const p = spawnSync(
    'python3',
    [
      '-I',
      '-c',
      code,
      new URL('../runner.py', import.meta.url).pathname,
      new URL('v5_fixture_data.py', import.meta.url).pathname,
    ],
    { encoding: 'utf8' },
  );
  assert.equal(p.status, 0, p.stderr);
});
test('item 6: the deploy helper takes the owner-edited public lists', () => {
  const vars = parseVars(
    'OWNER_GITHUB_ID=229738317\nBAD_HOST_IDS=[4242]\nBAD_MACHINE_IDS=[1,2]\n' +
      'LOG_RETENTION_HOURS=6\n',
  );
  assert.equal(vars.get('BAD_HOST_IDS'), '[4242]');
  assert.equal(vars.get('BAD_MACHINE_IDS'), '[1,2]');
  assert.equal(vars.get('LOG_RETENTION_HOURS'), '6');
});
