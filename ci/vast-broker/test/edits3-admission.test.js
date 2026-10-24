import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { DEFAULTS } from '../src/defaults.js';
import { setup, rentBody } from './helpers.js';
// Delta review 2026-10-24, findings 1-3 (B1, R1, B2).
// B1: a BUSY run waits at most until its remaining budget no longer covers the hard limit plus
// the admission margin; the runner stops asking, and the Worker refuses a run that declares
// less (NOT_ELIGIBLE, reason RUNNER_TIME), before it supersedes anything.
// R1: the one-attempt rule outlives retention: a run key whose spend row exists never rents
// again, even after its record expired.
// B2: the 24 h same-commit rule is checked before the concurrency cap, so a repeated commit is
// refused at once (UNAVAILABLE_BUDGET) instead of waiting BUSY, and closes no older run.
// FALSIFIER: a run with too little time is admitted or closes another run; the runner keeps
// waiting past the threshold after a definite BUSY or stops after an ambiguous reply; a
// refused-create key rents again after retention; a repeated commit gets BUSY or supersedes.
const fixture = new URL('edits3_runner_fixture.py', import.meta.url).pathname;
const py = (args, input) =>
  spawnSync('python3', ['-I', fixture, ...args], { encoding: 'utf8', input, timeout: 20000 });
const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
// The harness hard limit is 20 minutes (helpers.js); the margin is DEFAULTS'.
const need = (20 + 5) * 60;
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

test('B1: the admission margin is 5 minutes', () => {
  assert.equal(DEFAULTS.admissionMarginMinutes, 5);
});
for (const bad of [undefined, '1500', -1, 1.5, 86401, null, true])
  test('B1: remaining_seconds ' + JSON.stringify(bad) + ' is a bad request', async () => {
    const h = await setup();
    const body = rentBody(h.cloud.run.head_sha, { remaining_seconds: bad });
    if (bad === undefined) delete body.remaining_seconds;
    const r = await h.call('/rent', { body });
    assert.equal(r.status, 400);
    assert.equal((await r.json()).state, 'BAD_REQUEST');
    assert.equal(h.cloud.creates, 0);
  });
test('B1: a run declaring less than the hard limit plus margin is refused, no rental', async () => {
  const h = await setup();
  const r = await h.call('/rent', {
    body: rentBody(h.cloud.run.head_sha, { remaining_seconds: need - 1 }),
  });
  assert.equal(r.status, 409);
  const data = await r.json();
  assert.deepEqual([data.state, data.reason], ['NOT_ELIGIBLE', 'RUNNER_TIME']);
  assert.equal(h.cloud.creates, 0);
  assert.equal(await h.storage.get('record:123-1'), undefined);
  assert.equal(await h.storage.get('cost:123-1'), undefined);
  // The real runner accepts the refusal as final.
  const p = py(['validate-refusal'], JSON.stringify(data));
  assert.equal(p.status, 0, p.stderr + p.stdout);
  const ok = await h.call('/rent', {
    body: rentBody(h.cloud.run.head_sha, { remaining_seconds: need }),
  });
  assert.equal(ok.status, 200);
  assert.equal(h.cloud.creates, 1);
});
test('B1: a run without time left closes no older run of its branch', async () => {
  const h = await setup();
  market(h);
  assert.equal((await rentAs(h, 201, { branch: 'b' }, { remaining_seconds: 12000 })).status, 200);
  const late = await rentAs(h, 202, { branch: 'b' }, { remaining_seconds: need - 1 });
  assert.deepEqual([late.status, late.data.state], [409, 'NOT_ELIGIBLE']);
  assert.deepEqual(h.cloud.deletes, []);
  assert.equal((await h.storage.get('record:201-1')).ended, null);
});
test('B1: an admitted run gets its record back whatever time it declares', async () => {
  const h = await setup();
  const body = (s) => rentBody(h.cloud.run.head_sha, { remaining_seconds: s });
  const first = await (await h.call('/rent', { body: body(need) })).json();
  const again = await h.call('/rent', { body: body(0) });
  assert.equal(again.status, 200);
  assert.equal((await again.json()).id, first.id);
  assert.equal(h.cloud.creates, 1);
});
for (const name of ['need-constant', 'busy-wait-ends', 'ambiguous-keeps-asking'])
  test('B1: runner ' + name, () => {
    const p = py([name]);
    assert.equal(p.status, 0, p.stderr + p.stdout);
  });

test('R1: a refused-create run key never rents again, also after retention', async () => {
  const h = await setup();
  const base = h.cloud.fetch;
  h.cloud.fetch = async (url, init) =>
    String(url).includes('/asks/') ? Response.json({ success: false }) : base(url, init);
  h.restart();
  assert.equal((await h.call()).status, 503);
  h.cloud.fetch = base;
  h.restart();
  assert.ok(await h.storage.get('cost:123-1'));
  // Before retention: the prior (GONE) record comes back, nothing new is created.
  const prior = await h.call();
  assert.equal(prior.status, 200);
  assert.equal((await prior.json()).state, 'NONE');
  const creates = h.cloud.creates;
  h.time.value += 7 * 3600000; // past LOG_RETENTION_HOURS (6)
  await h.service.sweep();
  assert.equal(await h.storage.get('record:123-1'), undefined);
  assert.ok(await h.storage.get('cost:123-1'));
  const again = await h.call();
  assert.equal(again.status, 409);
  assert.equal((await again.json()).state, 'UNAVAILABLE_BUDGET');
  assert.equal(h.cloud.creates, creates);
  assert.equal(await h.storage.get('record:123-1'), undefined);
});
test('R1: another run key of the same commit may rent after a refused create', async () => {
  const h = await setup();
  const base = h.cloud.fetch;
  h.cloud.fetch = async (url, init) =>
    String(url).includes('/asks/') ? Response.json({ success: false }) : base(url, init);
  h.restart();
  assert.equal((await h.call()).status, 503);
  h.cloud.fetch = base;
  h.cloud.run.run_attempt = 2;
  h.restart();
  assert.equal((await h.call('/rent', { claims: { run_attempt: '2' } })).status, 200);
});

test('B2: a repeated commit on another branch is refused at once, not BUSY', async () => {
  const h = await setup();
  market(h);
  const sha = hex(7);
  assert.equal((await rentAs(h, 201, { branch: 'a', sha })).status, 200);
  const second = await rentAs(h, 202, { branch: 'b', sha });
  assert.deepEqual([second.status, second.data.state], [409, 'UNAVAILABLE_BUDGET']);
  assert.equal(h.cloud.creates, 1);
  assert.deepEqual(h.cloud.deletes, []);
});
test('B2: a repeated commit on the same branch is refused at once, not BUSY', async () => {
  const h = await setup();
  market(h);
  const sha = hex(7);
  assert.equal((await rentAs(h, 201, { branch: 'a', sha })).status, 200);
  const second = await rentAs(h, 202, { branch: 'a', sha });
  assert.deepEqual([second.status, second.data.state], [409, 'UNAVAILABLE_BUDGET']);
  assert.deepEqual(h.cloud.deletes, []);
});
test('B2: a repeated commit does not supersede the newer run of its branch', async () => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2' });
  market(h);
  assert.equal((await rentAs(h, 201, { branch: 'a', sha: hex(7) })).status, 200);
  assert.equal((await h.call('/stop/201-1', { claims: { run_id: '201' } })).status, 200);
  assert.equal((await rentAs(h, 202, { branch: 'a', sha: hex(8) })).status, 200);
  const deletes = h.cloud.deletes.length;
  const third = await rentAs(h, 203, { branch: 'a', sha: hex(7) });
  assert.deepEqual([third.status, third.data.state], [409, 'UNAVAILABLE_BUDGET']);
  assert.equal(h.cloud.deletes.length, deletes);
  assert.equal((await h.storage.get('record:202-1')).ended, null);
});
test('B2: a second run of the same commit after the gap is admitted (owner re-run)', async () => {
  const h = await setup();
  market(h);
  const sha = hex(7);
  assert.equal((await rentAs(h, 201, { branch: 'a', sha })).status, 200);
  assert.equal((await h.call('/stop/201-1', { claims: { run_id: '201' } })).status, 200);
  h.time.value += 25 * 3600000;
  const claims = { ...runAs(h, 202, { branch: 'a', sha }), run_attempt: '2' };
  h.cloud.runs.get('202').run_attempt = 2;
  const r = await h.call('/rent', { claims, body: rentBody(sha) });
  assert.equal(r.status, 200, JSON.stringify(await r.clone().json()));
});
