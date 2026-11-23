import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha, nonce, MemoryStorage } from './helpers.js';
import { unusedRental } from '../src/provider-policy.js';
import { Ledger } from '../src/ledger.js';

// The commit gap (SHA_GAP_HOURS) is a cost guard: one rental per commit. A rental that never became usable and cannot
// be replaced tested nothing, so a re-run of the commit is not a second rental of it. The rule is narrow on purpose:
// every other ending keeps the gap, and the spend rows (daily count, daily and rolling USD) are never released.
// FALSIFIER: before this rule a re-run of a commit whose only rental faulted at boot is refused with BUDGET:sha-gap.
const FAULT = 'GPU error, unable to start instance';
const reason = async (r) => (await r.json()).reason;

// One box, no replacement allowed: the run ends as an exhausted PROVIDER_FAULT without any box report.
async function faulted(overrides = {}, beforeFault = async () => {}) {
  const h = await setup({
    PROVIDER_MAX_ATTEMPTS: '1',
    PROVIDER_CONFIRM_SECONDS: '1',
    ...overrides,
  });
  assert.equal((await h.call()).status, 200);
  await beforeFault(h);
  Object.assign(h.cloud.instances.get('900'), { actual_status: 'created', status_msg: FAULT });
  for (let i = 0; i < 2; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  return h;
}
const rerun = (h) => {
  h.cloud.run.run_attempt = 2;
  return h.call('/rent', { claims: { run_attempt: '2' } });
};

test('unusedRental is true only for an exhausted provider fault that never reported', () => {
  const base = { verdict: 'PROVIDER_FAULT', replacementExhausted: true };
  assert.equal(unusedRental(base), true);
  for (const patch of [
    { verdict: 'FAIL' },
    { verdict: 'PASS' },
    { verdict: 'UNAVAILABLE' },
    { verdict: null },
    { firstReportAt: 1 },
    { replacementExhausted: false },
    { replacementExhausted: undefined },
  ])
    assert.equal(unusedRental({ ...base, ...patch }), false, JSON.stringify(patch));
});

test('a re-run of a commit whose only rental faulted at boot is admitted', async () => {
  const h = await faulted();
  const first = await h.storage.get('record:123-1');
  assert.equal(first.verdict, 'PROVIDER_FAULT');
  assert.equal(first.replacementExhausted, true);
  assert.equal(first.firstReportAt, undefined);
  assert.equal(await h.storage.get('sha:' + sha), undefined);
  const again = await rerun(h);
  assert.equal(again.status, 200);
  assert.equal(h.cloud.created.length, 2);
});

test('the freed commit does not free the spend rows or the daily rental count', async () => {
  const h = await faulted({ DAILY_RENTALS: '1' });
  assert.ok(await h.storage.get('cost:123-1'));
  const again = await rerun(h);
  assert.equal(again.status, 409);
  assert.equal(await reason(again), 'BUDGET:daily-count');
  assert.equal(h.cloud.created.length, 1);
});

test('the freed commit does not free the daily spend cap', async () => {
  const h = await faulted();
  h.bindings.DAILY_SPEND_USD = '0.01';
  h.restart();
  const again = await rerun(h);
  assert.equal(again.status, 409);
  assert.match(await reason(again), /^BUDGET:(daily-usd|rolling-usd|run-usd)/);
  assert.equal(h.cloud.created.length, 1);
});

test('a provider fault with a replacement still allowed keeps the commit gap', async () => {
  const h = await setup({ PROVIDER_CONFIRM_SECONDS: '1' });
  assert.equal((await h.call()).status, 200);
  Object.assign(h.cloud.instances.get('900'), { actual_status: 'created', status_msg: FAULT });
  for (let i = 0; i < 2; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  const first = await h.storage.get('record:123-1');
  assert.equal(first.verdict, 'PROVIDER_FAULT');
  assert.notEqual(first.replacementExhausted, true);
  assert.ok(await h.storage.get('sha:' + sha));
  const again = await rerun(h);
  assert.equal(again.status, 409);
  assert.equal(await reason(again), 'BUDGET:sha-gap');
});

for (const verdict of ['FAIL', 'PASS']) {
  test('a run that reported keeps the commit gap, verdict ' + verdict, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    const response = await h.report({ seq: 1, kind: 'result', result: { nonce, verdict } });
    assert.equal(response.status, 200);
    h.time.value = (await h.storage.get('record:123-1')).resultDeadline;
    await h.service.sweep();
    assert.ok(await h.storage.get('sha:' + sha));
    const answer = await (await rerun(h)).json();
    assert.equal(answer.state, 'UNAVAILABLE_BUDGET');
    assert.equal(answer.reason, 'BUDGET:sha-gap');
    assert.equal(h.cloud.created.length, 1);
  });
}

test('a rental that ended for another reason keeps the commit gap', async () => {
  const h = await setup();
  assert.equal((await h.call()).status, 200);
  await h.call('/stop/123-1');
  const closed = await h.storage.get('record:123-1');
  assert.notEqual(closed.verdict, 'PROVIDER_FAULT');
  const again = await rerun(h);
  assert.equal(await reason(again), 'BUDGET:sha-gap');
});

test('the marker of another run key is never released', async () => {
  const h = await faulted({}, (h) =>
    h.storage.put('sha:' + sha, { key: '999-1', started: h.time.value }),
  );
  assert.equal((await h.storage.get('sha:' + sha)).key, '999-1');
  const again = await rerun(h);
  assert.equal(await reason(again), 'BUDGET:sha-gap');
});

// The run's last chance to rent a replacement passes without one (no other machine is offered): the next request of
// the run ends the chain, and that end releases the marker like the end of a chain that was spent at once.
test('a replacement that never came ends the chain and releases the commit', async () => {
  const h = await setup({ PROVIDER_CONFIRM_SECONDS: '1' });
  assert.equal((await h.call()).status, 200);
  Object.assign(h.cloud.instances.get('900'), { actual_status: 'created', status_msg: FAULT });
  for (let i = 0; i < 2; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  await h.call();
  const waiting = await h.storage.get('record:123-1');
  assert.equal(waiting.verdict, 'PROVIDER_FAULT');
  assert.notEqual(waiting.replacementExhausted, true);
  assert.ok(await h.storage.get('sha:' + sha));
  h.time.value = waiting.sessionDeadline + 1000;
  await h.call();
  const ended = await h.storage.get('record:123-1');
  assert.equal(ended.replacementExhausted, true);
  assert.equal(await h.storage.get('sha:' + sha), undefined);
  assert.equal((await rerun(h)).status, 200);
});

// Two requests of one run can overlap (the runner retries): the one that still holds an earlier leg must not drop the
// marker of the leg the other has reserved since. Both legs have the run key, so only the leg's nonce tells them apart.
test('an earlier leg cannot release the marker of the live leg of its run', async () => {
  const storage = new MemoryStorage(),
    ledger = new Ledger(storage);
  const leg = (nonce, extra = {}) => ({
    key: '1-1',
    run: '1',
    sha,
    nonce,
    verdict: 'PROVIDER_FAULT',
    replacementExhausted: true,
    ...extra,
  });
  const stale = leg('a'.repeat(32)),
    live = leg('b'.repeat(32), { verdict: null, replacementExhausted: false });
  await ledger.save(live);
  await ledger.grant('1-1', live);
  await storage.put('sha:' + sha, { key: '1-1', started: 1 });
  await ledger.releaseUnusedMarker(stale);
  assert.ok(await storage.get('sha:' + sha), 'the live leg keeps its marker');
  // the same leg, once it is the current one, does release it
  await ledger.save({ ...live, verdict: 'PROVIDER_FAULT', replacementExhausted: true });
  await ledger.releaseUnusedMarker({
    ...live,
    verdict: 'PROVIDER_FAULT',
    replacementExhausted: true,
  });
  assert.equal(await storage.get('sha:' + sha), undefined);
});
