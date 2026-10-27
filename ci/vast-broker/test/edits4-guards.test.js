import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, rentBody } from './helpers.js';
// Review of pass 3 (2026-10-24), tests T1, T2 and T3: guards that had no failing test when
// removed or moved.
// T1: the spent-run-key refusal comes before the supersede, so a spent key closes no older box.
// T2/T3: see the country tests below.
// FALSIFIER: a spent run key supersedes (destroys) an older run of its branch before it is
// refused; a format character hides a denied country when unknown locations are allowed.
const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
function runAs(h, n, { branch = 'branch-' + n, sha = hex(n) } = {}) {
  h.cloud.runs.set(String(n), {
    ...h.cloud.run,
    id: n,
    head_sha: sha,
    head_branch: branch,
    pull_requests: [],
  });
  return { run_id: String(n) };
}
async function rentAs(h, n, run = {}, extra = {}) {
  const claims = runAs(h, n, run);
  const body = rentBody(h.cloud.runs.get(String(n)).head_sha, extra);
  const response = await h.call('/rent', { claims, body });
  return { status: response.status, data: await response.json() };
}
const market = (h) => {
  h.cloud.extraOffers = [
    { ...h.cloud.offer, id: 11, machine_id: 200011 },
    { ...h.cloud.offer, id: 12, machine_id: 200012 },
  ];
  h.cloud.contracts = [901, 902, 903, 904];
};

test('T1: a spent run key closes no older run of its branch', async () => {
  const h = await setup();
  market(h);
  const older = await rentAs(h, 201, { branch: 'b' }, { remaining_seconds: 12000 });
  assert.equal(older.status, 200);
  assert.equal(h.cloud.creates, 1);
  // Run 202 is newer, on the same branch with another commit, so an admitted 202 would
  // supersede 201. Its spend row exists but its record is gone (retention): the key is spent.
  await h.storage.put('cost:202-1', {
    profile: 'check',
    started: h.time.value - 7 * 3600000,
    ended: h.time.value - 6 * 3600000,
    price: 0.2,
    bandwidthCost: 0,
    ttl: 1,
  });
  const spent = await rentAs(h, 202, { branch: 'b' }, { remaining_seconds: 12000 });
  assert.deepEqual([spent.status, spent.data.state], [409, 'UNAVAILABLE_BUDGET']);
  assert.deepEqual(h.cloud.deletes, []);
  assert.equal((await h.storage.get('record:201-1')).ended, null);
  assert.equal(h.cloud.creates, 1);
  assert.equal(await h.storage.get('record:202-1'), undefined);
});
