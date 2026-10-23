import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { setup } from './helpers.js';
// Owner ruling 2026-10-24 item 9: every /availability offer row (offers[] and the rejected
// rows with their reason codes) and the status object carry the public Vast machine_id and
// host_id, so a bad hoster can be put on BAD_MACHINE_IDS / BAD_HOST_IDS from the public log.
// No IPs, no hostnames, no offer (ask) id.
// FALSIFIER: an id is missing or wrong, an IP or hostname crosses, the runner accepts a
// malformed id or reason, or the Worker can emit a rejection code the runner refuses.
const fixture = new URL('edits2_identity_fixture.py', import.meta.url).pathname;
const py = (args, input) =>
  spawnSync('python3', ['-I', fixture, ...args], { encoding: 'utf8', input, timeout: 10000 });
const availability = async (h, profile = 'check') =>
  h.call('/availability?profile=' + profile, { method: 'GET' });
const hostile = { public_ipaddr: '203.0.113.7', hostname: 'box7.hoster.example' };
const leaks = /203\.0\.113|hoster\.example|public_ipaddr|hostname/;

test('item 9: availability offer rows carry the representative machine_id and host_id', async () => {
  const h = await setup();
  Object.assign(h.cloud.offer, { host_id: 3001 }, hostile);
  h.cloud.extraOffers = [
    // Same die, cheaper: it is the row's representative (the price the row shows).
    { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 3002, price: 0.15 },
    { ...h.cloud.offer, id: 7, machine_id: 100003, host_id: 3003, reliability: 0.5 },
  ];
  const r = await availability(h);
  assert.equal(r.status, 200);
  const data = await r.json();
  assert.deepEqual(
    data.offers.map((o) => [o.die, o.count, o.machine_id, o.host_id]),
    [['AD104', 2, 100002, 3002]],
  );
  assert.deepEqual(data.rejected_offers, [
    { machine_id: 100003, host_id: 3003, reasons: ['RELIABILITY'] },
  ]);
  assert.deepEqual(data.rejected, { RELIABILITY: 1 });
  assert.doesNotMatch(JSON.stringify(data), leaks);
  assert.doesNotMatch(JSON.stringify(data), /"(?:id|offer_id)":/);
  const p = py(['validate', 'check'], JSON.stringify(data));
  assert.equal(p.status, 0, p.stderr + p.stdout);
});
test('item 9: a missing or malformed provider id is shown as null, never invented', async () => {
  const h = await setup();
  delete h.cloud.offer.host_id;
  h.cloud.extraOffers = [{ ...h.cloud.offer, id: 6, machine_id: 'x', host_id: -4 }];
  const data = await (await availability(h)).json();
  assert.deepEqual(
    data.offers.map((o) => [o.machine_id, o.host_id]),
    [[100001, null]],
  );
  assert.deepEqual(data.rejected_offers, [
    { machine_id: null, host_id: null, reasons: ['MACHINE'] },
  ]);
  const p = py(['validate', 'check'], JSON.stringify(data));
  assert.equal(p.status, 0, p.stderr + p.stdout);
});
test('item 9: the status object carries machine_id and host_id next to id', async () => {
  const h = await setup();
  Object.assign(h.cloud.offer, { host_id: 3001 }, hostile);
  const rent = await h.call();
  assert.equal(rent.status, 200);
  for (const data of [await rent.json(), await (await h.call('/status/123-1')).json()]) {
    assert.equal(data.id, '900');
    assert.equal(data.machine_id, 100001);
    assert.equal(data.host_id, 3001);
    const keys = Object.keys(data);
    assert.deepEqual(keys.slice(keys.indexOf('id'), keys.indexOf('id') + 3), [
      'id',
      'machine_id',
      'host_id',
    ]);
    assert.doesNotMatch(JSON.stringify(data), leaks);
  }
  const stopped = await (await h.call('/stop/123-1')).json();
  assert.deepEqual([stopped.machine_id, stopped.host_id], [100001, 3001]);
});
test('item 9: a status without a host id says null', async () => {
  const h = await setup();
  const data = await (await h.call()).json();
  assert.deepEqual([data.machine_id, data.host_id], [100001, null]);
});
for (const name of ['status-ids', 'availability-ids', 'display'])
  test('item 9: runner ' + name, () => {
    const p = py([name]);
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
// One offer mutation per rejection code. Each must reach /availability as a count and as a row,
// and the runner's strict validator must accept the real response.
const CODES = {
  VERIFICATION: (h) => (h.cloud.offer.verified = false),
  CAPABILITY: (h) => (h.cloud.offer.num_gpus = 2),
  MACHINE: (h) => (h.cloud.offer.machine_id = 42636), // on the default BAD_MACHINE_IDS
  BAD_HOST: (h) => (h.cloud.offer.host_id = 77),
  DIE: (h) => (h.cloud.offer.gpu_name = 'RTX_3060'), // CC_89: not a check die
  RELIABILITY: (h) => (h.cloud.offer.reliability = 0.5),
  CPU_RAM: (h) => (h.cloud.offer.cpu_ram = 1000),
  PERFORMANCE: (h) => (h.cloud.offer.dlperf = 0),
  NETWORK: (h) => (h.cloud.offer.inet_down = 10),
  DISK_BW: (h) => (h.cloud.offer.disk_bw = 10),
  DRIVER: (h) => (h.cloud.offer.driver_version = '999.1.1'),
  PRICE_FIELDS: (h) => (h.cloud.offer.storage_cost = -1),
  COUNTRY: (h) => (h.cloud.offer.geolocation = 'Beijing, CN'),
  PRICE: (h) => (h.cloud.price = 0.9), // MAX_PRICE_USD_HOUR is 0.5 in the harness
  // Item 8: the machine of an active run is held (reserved while CREATING and active).
  IN_USE: async (h) => assert.equal((await h.call()).status, 200),
};
for (const [code, mutate] of Object.entries(CODES))
  test('item 9: rejection code table ' + code, async () => {
    const h = await setup({ BAD_HOST_IDS: '[77]' });
    h.cloud.offer.host_id = 3001;
    await mutate(h);
    const r = await availability(h);
    assert.equal(r.status, 200);
    const data = await r.json();
    assert.ok(data.rejected[code] >= 1, JSON.stringify(data.rejected));
    assert.ok(
      data.rejected_offers.some((row) => row.reasons.includes(code)),
      JSON.stringify(data.rejected_offers),
    );
    const p = py(['validate', 'check'], JSON.stringify(data));
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });
test('item 9: every code the Worker can emit is in the table and accepted by the runner', async () => {
  const source = ['policy.js', 'offers.js']
    .map((f) => readFile(new URL('../src/' + f, import.meta.url), 'utf8'))
    .map(async (x) => [...(await x).matchAll(/reasons\.push\('([A-Z_]+)'\)/g)].map((m) => m[1]));
  const emitted = [...new Set((await Promise.all(source)).flat())].sort();
  assert.ok(emitted.length >= 14, emitted.join());
  assert.deepEqual(Object.keys(CODES).sort(), emitted);
  const p = py(['codes']);
  assert.equal(p.status, 0, p.stderr);
  assert.deepEqual(JSON.parse(p.stdout), emitted);
});
