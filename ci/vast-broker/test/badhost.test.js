import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setup, env } from './helpers.js';
import { configuration } from '../src/policy.js';

// FALSIFIER: this exact pre-product provider summary already gets a bounded other-host retry.
const summary = readFileSync(new URL('./fixtures/provider-57945969.txt', import.meta.url), 'utf8');
const hardware = { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' };
async function fixture() {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '40' });
  h.cloud.offer.host_id = 5000;
  h.cloud.contracts = [900, 901];
  assert.equal((await h.call()).status, 200);
  h.cloud.extraOffers = [
    { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 5000, price: 0.1 },
    { ...h.cloud.offer, id: 7, machine_id: 100003, host_id: 6000, price: 0.11 },
  ];
  return h;
}
async function report(h, text, reason = 'host_xid', verdict = 'FAIL') {
  const r = await h.storage.get('record:123-1');
  assert.equal((await h.report({ seq: 1, kind: 'log', text })).status, 200);
  return h.report({
    seq: 2,
    kind: 'result',
    result: {
      nonce: r.nonce,
      verdict,
      hardware,
      suites: [{ name: reason, pass: 0, fail: 1, skip: 0 }],
    },
  });
}
for (const reason of ['host_xid', 'provision'])
  for (const branch of ['initialization', 'unusable', 'not-ready', 'kvm'])
    test('badhost classifies and retries ' + reason + ' ' + branch, async () => {
      const h = await fixture();
      let text = summary.replaceAll('host_xid', reason);
      if (branch === 'initialization') text = text.replace('GPU unusable', 'GPU unavailable');
      if (branch === 'unusable')
        text = text.replaceAll(/RmInitAdapter|rm_init_adapter/g, 'adapter');
      if (branch === 'not-ready')
        text = text
          .replace('line=DRIVER_RC=8', 'line=NOT_READY GPU unusable')
          .replace(/^cause .*\n/m, '');
      if (branch === 'kvm')
        text = text.replace('line=DRIVER_RC=8', 'line=no /dev/kvm').replace(/^cause .*\n/m, '');
      assert.equal((await report(h, text, reason)).status, 200);
      const first = await h.storage.get('record:123-1');
      assert.equal(first.verdict, 'PROVIDER_FAULT');
      assert.equal(h.cloud.instances.has('900'), false);
      assert.equal((await h.storage.get('bad-machine:100001')).hostId, 5000);
      assert.match(first.log, /provider fault: machine 100001 host 5000, not a kayfabe verdict/);
      h.restart();
      const reply = await h.call();
      assert.equal(reply.status, 200);
      assert.equal((await reply.json()).host_id, 6000);
      assert.equal(h.cloud.created.length, 2);
      assert.deepEqual(h.cloud.deletes, ['900']);
    });
for (const variant of [1, 2]) {
  test('badhost two distinct machines learn one durable host ' + variant, async () => {
    const h = await fixture();
    await h.storage.put('bad-machine:' + variant, {
      reason: 'PROVIDER_FAULT',
      at: h.time.value - 1,
      hostId: 5000,
    });
    assert.equal((await report(h, '', 'provider_kvm_missing', 'PROVIDER_FAULT')).status, 200);
    const host = await h.storage.get('bad-host:5000');
    assert.equal(host.reason, 'PROVIDER_FAULT');
    assert.deepEqual(
      host.machines.sort((a, b) => a - b),
      [variant, 100001],
    );
    h.restart();
    const availability = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal(availability.status, 200);
    assert.deepEqual(
      (await availability.json()).offers.map((o) => o.host_id),
      [6000],
    );
  });
  test('badhost dynamic host denies cached offers ' + variant, async () => {
    const h = await fixture();
    const host = 6000 + variant;
    h.cloud.extraOffers = [{ ...h.cloud.extraOffers[1], host_id: host }];
    h.time.value += 60001;
    const before = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal((await before.json()).offers[0].host_id, host);
    await h.storage.put('bad-host:' + host, { reason: 'PROVIDER_FAULT' });
    const response = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).offers.length, 0);
  });
  test('badhost overflow refuses offers ' + variant, async () => {
    const h = await fixture();
    for (let i = 1; i <= 256 + variant; i++)
      await h.storage.put('bad-host:' + i, { reason: 'PROVIDER_FAULT' });
    const response = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal(response.status, 503);
  });
  test('badhost retry requires a different known host ' + variant, async () => {
    const h = await fixture();
    h.cloud.extraOffers = [h.cloud.extraOffers[0]];
    if (variant === 2) delete h.cloud.extraOffers[0].host_id;
    assert.equal((await report(h, '', 'provider_kvm_missing', 'PROVIDER_FAULT')).status, 200);
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).verdict, 'PROVIDER_FAULT');
    assert.equal(h.cloud.created.length, 1);
  });
  test('badhost static seed excludes known failing host ' + variant, () => {
    assert.ok(configuration(env).badHosts.includes(224600));
  });
}
for (const change of ['product-ran', 'no-driver-proof', 'no-fault-proof', 'truncated', 'ordinary'])
  test('badhost preserves non-provider or incomplete failure ' + change, async () => {
    const h = await fixture();
    let text = summary;
    if (change === 'product-ran')
      text = text.replace(
        'cuda_host result=NOT_RUN',
        'cuda_host rc=1 duration_ms=1 result=FAIL reason=host_xid',
      );
    if (change === 'no-driver-proof') text = text.replace('DRIVER_RC=8', 'DRIVER_RC=0');
    if (change === 'no-fault-proof') text = text.replace(/^cause .*\n/m, '');
    if (change === 'truncated') text = text.replace('KF_FINAL_SUMMARY_END', '');
    if (change === 'ordinary') text = 'DRIVER_RC=8\n';
    assert.equal((await report(h, text)).status, 200);
    assert.equal((await h.storage.get('record:123-1')).verdict, 'FAIL');
    assert.equal(await h.storage.get('bad-machine:100001'), undefined);
  });
