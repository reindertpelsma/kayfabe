import test from 'node:test';
import assert from 'node:assert/strict';
import { providerPolicy, configuration } from '../src/policy.js';
import { fatalSignature } from '../src/provider-policy.js';
import { setup, env } from './helpers.js';

// A box that sat in `created` with the daemon error below waited the whole boot timeout (75 min) before it was destroyed,
// because the message was not a fatal signature (batch PR gate, attempt 1: docs/CI_V3.md, "gpu-vast failure causes").
// FALSIFIER: the signature is not in defaults.json, so a box that shows only this status message is closed at the boot deadline.
const MESSAGE =
  'Error response from daemon: error gathering device information while adding custom device "/dev/kvm"';
const policy = providerPolicy();

for (const [name, text] of [
  ['verbatim', MESSAGE],
  ['upper case', MESSAGE.toUpperCase()],
  ['prefixed by the host', 'host 12: ' + MESSAGE + ': no such file or directory'],
]) {
  test('kvm device error is a fatal status signature, ' + name, () => {
    assert.equal(fatalSignature({ status_msg: text }, '', '1', policy), 'KVM_DEVICE_ERROR');
  });
}
for (const [name, text] of [
  ['another device', 'error gathering device information while adding custom device "/dev/fuse"'],
  [
    'another device path',
    'Error response from daemon: error gathering device information while adding custom device "/dev/nvidiactl"',
  ],
  ['the kvm path alone', 'waiting for /dev/kvm'],
  ['the gpu start error keeps its own reason', 'GPU error, unable to start instance'],
]) {
  test('kvm device signature does not match ' + name, () => {
    assert.notEqual(fatalSignature({ status_msg: text }, '', '1', policy), 'KVM_DEVICE_ERROR');
  });
}
test('kvm device error is only a status signature, not a boot log one', () => {
  assert.equal(fatalSignature({}, MESSAGE, '1', policy), null);
});

// The end to end rule: the box is closed as a provider fault shortly after the confirmation window, not at the boot
// deadline, and the run is offered a replacement.
test('a box stuck on the kvm device error is closed within the confirmation window', async () => {
  const h = await setup({});
  assert.equal((await h.call()).status, 200);
  const first = await h.storage.get('record:123-1');
  Object.assign(h.cloud.instances.get(first.id), { actual_status: 'created', status_msg: MESSAGE });
  const limit = first.started + configuration(env).provider.confirmationSeconds * 1000 + 60000;
  while (h.time.value < limit) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  const r = await h.storage.get('record:123-1');
  assert.equal(r.verdict, 'PROVIDER_FAULT');
  assert.equal(r.faultReason, 'KVM_DEVICE_ERROR');
  assert.ok(
    r.ended !== null && r.ended <= limit,
    'closed inside the confirmation window, not at the boot deadline',
  );
  assert.ok(limit < r.started + 5 * 60000);
  assert.ok(await h.storage.get('provider-machine:' + first.machineId));
});
