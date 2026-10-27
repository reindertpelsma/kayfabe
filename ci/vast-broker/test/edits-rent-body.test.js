import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup, sha } from './helpers.js';
// Owner ruling 2026-10-24 item 1. FALSIFIER: the Worker still needs or accepts pr_number, the
// status object still carries pr, or the runner still sends pr_number / accepts a pr field.
const body = {
  runner_nonce: 'f'.repeat(64),
  tested_sha: sha,
  profile: 'check',
  remaining_seconds: 12000,
};
test('item 1: rent body without pr_number is admitted', async () => {
  const h = await setup();
  const response = await h.call('/rent', { body });
  assert.equal(response.status, 200);
  assert.equal(h.cloud.creates, 1);
});
for (const pr of [7, null])
  test('item 1: rent body with pr_number ' + pr + ' is refused before any Vast call', async () => {
    const h = await setup();
    const response = await h.call('/rent', { body: { ...body, pr_number: pr } });
    assert.equal(response.status, 400);
    assert.equal((await response.json()).state, 'BAD_REQUEST');
    assert.equal(h.cloud.creates, 0);
  });
for (const missing of ['runner_nonce', 'tested_sha', 'profile'])
  test('item 1: rent body without ' + missing + ' is refused', async () => {
    const h = await setup();
    const partial = { ...body };
    delete partial[missing];
    assert.equal((await h.call('/rent', { body: partial })).status, 400);
    assert.equal(h.cloud.creates, 0);
  });
test('item 1: status object has no pr field (rent, status, stop)', async () => {
  const h = await setup();
  const rented = await (await h.call('/rent', { body })).json();
  assert.equal('pr' in rented, false);
  const status = await (await h.call('/status/123-1')).json();
  assert.equal('pr' in status, false);
  const stopped = await (await h.call('/stop/123-1')).json();
  assert.equal('pr' in stopped, false);
  assert.equal(
    [...h.storage.data.keys()].some((k) => k.startsWith('run-pr:')),
    false,
  );
});
test('item 1: runner sends no pr_number and refuses a status with pr', () => {
  const p = spawnSync(
    'python3',
    ['-I', new URL('edits_runner_fixture.py', import.meta.url).pathname, 'rent-body'],
    { encoding: 'utf8' },
  );
  assert.equal(p.status, 0, p.stderr + p.stdout);
});
