import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
import { Vast } from '../src/vast.js';
import { IMAGE } from '../src/policy.js';
// FALSIFIER: either direct provider creation or the admitted rent path uses
// entrypoint/args mode, drops the fixed onstart, or supplies account SSH keys.
test('provider launch uses SSH with fixed image and onstart, inheriting account keys', async () => {
  let body;
  const vast = new Vast(
    async (_url, init) => {
      body = JSON.parse(init.body);
      return Response.json({ success: true, new_contract: 900 });
    },
    {},
    'mock-key',
  );
  await vast.create({ id: 5 }, { label: 'kf-ci-123-1' }, 'fixed-script');
  assert.equal(body.runtype, 'ssh_direct');
  assert.equal(body.image, IMAGE);
  assert.equal(body.onstart, 'fixed-script');
  assert.equal(body.vm, true);
  assert.equal('args' in body, false);
  assert.equal('ssh_key' in body, false);
});
test('admitted rent uses SSH onstart without Worker-managed SSH credentials', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  const body = h.cloud.calls.find((c) => c.url.endsWith('/asks/5/')).body;
  assert.equal(body.runtype, 'ssh_direct');
  assert.match(body.onstart, /exec bash scripts\/ci\/vast_onstart.sh/);
  assert.match(body.env.KF_RUN_TOKEN, /^eyJ[\w-]+\.eyJ[\w-]+\.[\w-]{43}$/);
  assert.equal(body.image, IMAGE);
  assert.equal('args' in body, false);
  assert.doesNotMatch(JSON.stringify(body), /ssh_public_key|ssh_private_key|authorized_keys/);
});
