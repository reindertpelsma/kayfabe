import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, clock } from './helpers.js';
const message =
  "libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C.900'\n";
// FALSIFIER: boot expiry alone learns a bad host; only the exact owned-instance
// signature frozen across observations may produce a persistent deny entry.
for (const restart of [false, true])
  test('V5 learns frozen KVM failure only at boot expiry ' + restart, async () => {
    const h = await setup({ BOOT_LIMIT_MINUTES: '40', HARD_RUNTIME_MINUTES: '180' });
    assert.equal((await h.call()).status, 200);
    h.cloud.instances.get('900').actual_status = 'created';
    h.cloud.logText = message;
    h.time.value += 60000;
    await h.service.sweep();
    if (restart) h.restart();
    h.time.value += 180000;
    await h.service.sweep();
    assert.equal(await h.storage.get('bad-machine:100001'), undefined);
    h.time.value = clock + 39 * 60000;
    await h.service.sweep();
    h.time.value = clock + 40 * 60000;
    await h.service.sweep();
    const denied = await h.storage.get('bad-machine:100001');
    assert.equal(denied?.reason, 'KVM_DOMAIN_NOT_FOUND');
    assert.equal((await h.storage.get('record:123-1')).closed, 'PROVIDER_FAULT');
    h.restart();
    h.time.value += 60000;
    const r = await h.call('/availability?profile=check', { method: 'GET' });
    assert.equal(r.status, 200);
    assert.equal((await r.json()).offers.length, 0);
  });
for (const kind of ['plain-timeout', 'wrong-id', 'changed-log', 'running'])
  test('V5 never learns ambiguous boot failure ' + kind, async () => {
    const h = await setup({ BOOT_LIMIT_MINUTES: '40', HARD_RUNTIME_MINUTES: '180' });
    await h.call();
    h.cloud.instances.get('900').actual_status = kind === 'running' ? 'running' : 'created';
    h.cloud.logText =
      kind === 'plain-timeout'
        ? 'loading normally'
        : kind === 'wrong-id'
          ? message.replace('900', '901')
          : message;
    h.time.value += 60000;
    await h.service.sweep();
    if (kind === 'changed-log') h.cloud.logText += 'still changing\n';
    h.time.value = clock + 40 * 60000;
    await h.service.sweep();
    assert.equal(await h.storage.get('bad-machine:100001'), undefined);
  });
