import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, nonce } from './helpers.js';
// Owner ruling 2026-10-24 item 7, "no secrets on vast". FALSIFIER: the create request carries
// any key outside the pinned allow-list (args, ssh keys, extra env), or any value that equals or
// contains a Worker secret, the runner's Access token A or the owner stop token. Allowed by
// design (leakable, accepted): the per-run JWT, KF_BROKER_URL with the path prefix, the box
// Access pair B, nonce/sha/deadlines.
const secret = (name) => name.toLowerCase().replaceAll('_', '') + 'Sentinel' + 'q'.repeat(24);
const forbidden = {
  VAST_API_KEY: secret('VAST_API_KEY'),
  GITHUB_API_TOKEN: secret('GITHUB_API_TOKEN'),
  BLOB_GITHUB_TOKEN: secret('BLOB_GITHUB_TOKEN'),
  RUN_TOKEN_SECRET: secret('RUN_TOKEN_SECRET'),
  OWNER_STOP_TOKEN: secret('OWNER_STOP_TOKEN'),
  // Token A belongs to the runner (GitHub environment); a Worker binding of that name, set by
  // mistake, must still never travel to Vast.
  VAST_BROKER_ACCESS_ID: secret('VAST_BROKER_ACCESS_ID'),
  VAST_BROKER_ACCESS_SECRET: secret('VAST_BROKER_ACCESS_SECRET'),
};
const pairB = { BOX_ACCESS_ID: 'boxid.access', BOX_ACCESS_SECRET: 'boxsecret' + 'b'.repeat(32) };
const BODY_KEYS = [
  'cancel_unavail',
  'disk',
  'env',
  'image',
  'label',
  'onstart',
  'runtype',
  'target_state',
  'vm',
];
const ENV_KEYS = [
  'KF_ACCESS_ID',
  'KF_ACCESS_SECRET',
  'KF_BOOT_DEADLINE',
  'KF_BROKER_URL',
  'KF_GRACE_FAIL',
  'KF_GRACE_PASS',
  'KF_HARD_DEADLINE',
  'KF_HEARTBEAT_SECONDS',
  'KF_NONCE',
  'KF_PROTOCOL',
  'KF_REPO',
  'KF_RUNNER_NONCE',
  'KF_RUN_ID',
  'KF_RUN_TOKEN',
  'KF_SHA',
  'KF_SOFT_DEADLINE',
];
for (const withPair of [true, false])
  test('item 7: the Vast create request is the exact allow-list, pair B ' + withPair, async () => {
    const h = await setup({ ...forbidden, ...(withPair ? pairB : {}) });
    assert.equal((await h.call()).status, 200);
    const create = h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
    assert.equal(create.length, 1);
    const body = create[0].body;
    assert.deepEqual(Object.keys(body).sort(), BODY_KEYS);
    assert.deepEqual(
      Object.keys(body.env).sort(),
      ENV_KEYS.filter((k) => withPair || !k.startsWith('KF_ACCESS_')),
    );
    assert.equal(body.image, 'docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16');
    assert.equal(body.vm, true);
    assert.equal(body.disk, 120);
    assert.equal(body.runtype, 'ssh_direct');
    assert.equal(body.label, 'kf-ci-123-1');
    assert.equal(body.target_state, 'running');
    assert.equal(body.cancel_unavail, true);
    // Allowed by design, and only these.
    const record = await h.storage.get('record:123-1');
    assert.equal(body.env.KF_RUN_ID, '123-1');
    assert.equal(body.env.KF_NONCE, nonce);
    assert.equal(body.env.KF_SHA, record.sha);
    assert.equal(body.env.KF_RUNNER_NONCE, record.runnerNonce);
    assert.equal(
      body.env.KF_BROKER_URL,
      h.bindings.WORKER_URL + '/' + h.bindings.BROKER_PATH_PREFIX,
    );
    assert.match(body.env.KF_RUN_TOKEN, /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/);
    if (withPair) {
      assert.equal(body.env.KF_ACCESS_ID, pairB.BOX_ACCESS_ID);
      assert.equal(body.env.KF_ACCESS_SECRET, pairB.BOX_ACCESS_SECRET);
    }
    const text = JSON.stringify(body);
    for (const [name, value] of Object.entries(forbidden))
      assert.equal(text.includes(value), false, name + ' reached the Vast create request');
    for (const value of Object.values(body.env))
      for (const [name, s] of Object.entries(forbidden))
        assert.equal(String(value).includes(s), false, name);
  });
test('item 7: every Vast request carries no secret but the API key header', async () => {
  const h = await setup({ ...forbidden, ...pairB });
  assert.equal((await h.call()).status, 200);
  await h.call('/status/123-1');
  await h.call('/stop/123-1');
  const vast = h.cloud.calls.filter((c) => new URL(c.url).hostname === 'console.vast.ai');
  assert.ok(vast.length >= 4);
  for (const call of vast) {
    assert.equal(call.headers.get('Authorization'), 'Bearer ' + forbidden.VAST_API_KEY);
    const visible = call.url + JSON.stringify(call.body ?? null);
    for (const [name, value] of Object.entries(forbidden))
      assert.equal(visible.includes(value), false, name + ' in ' + call.method + ' ' + call.url);
  }
});
