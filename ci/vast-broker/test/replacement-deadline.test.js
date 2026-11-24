import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env, rentBody } from './helpers.js';
import { configuration } from '../src/policy.js';
import { replacementExtension, validExtension } from '../src/provider-policy.js';

// A replacement leg exists only because the legs before it never posted. The payload's stage budget counts from the first
// rental (KF_SOFT_DEADLINE), so every minute a faulted box wasted was taken from the apps stage, which starts only with 245
// minutes left (scripts/ci/vast_payload.py: apps 100 + Windows gate 75 + Windows apps 70). Measured on the replacement of
// the first attempt of the last batch PR gate: the box was rented 3 min 49 s before its first stage, and provision,
// cuda_host, merge_bar and cuda_guest took 573 + 12 + 1172 + 283 s before `apps` was due. Without a replacement
// extension, a replacement rented more than 7 minutes after the first rental already misses the apps stage.
// FALSIFIER: before this rule the replacement kept the first leg's deadlines (the 5 minute test at the end fails).
const MIN = 60000;
const APPS_NEEDS = 245 * MIN;
const BOX_READY = (3 * 60 + 49) * 1000;
const BEFORE_APPS = (573 + 12 + 1172 + 283) * 1000;
const policy = { replacementExtensionMinutes: 25 };
const MARGIN = 5 * MIN;

test('the extension is the time lost, up to the cap', () => {
  const prior = { started: 1000, hardDeadline: 1000 + 300 * MIN };
  const at = (minutes) => 1000 + minutes * MIN;
  assert.equal(replacementExtension(prior, at(0), policy, undefined, MARGIN), 0);
  assert.equal(replacementExtension(prior, at(4), policy, undefined, MARGIN), 4 * MIN);
  assert.equal(replacementExtension(prior, at(25), policy, undefined, MARGIN), 25 * MIN);
  assert.equal(replacementExtension(prior, at(75), policy, undefined, MARGIN), 25 * MIN);
});
test('a later replacement is measured from the first rental, never summed over legs', () => {
  const first = 1000,
    hard = first + 300 * MIN;
  const second = { started: first + 4 * MIN, originalStarted: first, deadlineExtension: 4 * MIN };
  second.hardDeadline = hard + 4 * MIN;
  assert.equal(replacementExtension(second, first + 10 * MIN, policy, undefined, MARGIN), 10 * MIN);
  assert.equal(replacementExtension(second, first + 90 * MIN, policy, undefined, MARGIN), 25 * MIN);
  // never below what the previous leg already had
  assert.equal(replacementExtension(second, first + 1 * MIN, policy, undefined, MARGIN), 4 * MIN);
});
test('the extension stays inside the runner wait budget less the admission margin', () => {
  const prior = { started: 0, hardDeadline: 300 * MIN };
  // the runner has 330 minutes at the start; 20 minutes in, 310 remain: hard remaining 280 + margin 5 leaves 25
  assert.equal(replacementExtension(prior, 20 * MIN, policy, 310 * MIN, MARGIN), 20 * MIN);
  assert.equal(replacementExtension(prior, 30 * MIN, policy, 300 * MIN, MARGIN), 25 * MIN);
  // a sweep runner (200 minute budget, 180 minute hard limit) gets only its own 15 minute room
  const sweep = { started: 0, hardDeadline: 180 * MIN };
  assert.equal(replacementExtension(sweep, 20 * MIN, policy, 180 * MIN, MARGIN), 15 * MIN);
  assert.equal(replacementExtension(prior, 20 * MIN, policy, 270 * MIN, MARGIN), 0);
});
test('a record without the policy field gets no extension', () => {
  const prior = { started: 0, hardDeadline: 300 * MIN };
  assert.equal(replacementExtension(prior, 20 * MIN, {}, undefined, MARGIN), 0);
});

test('the reservation accepts exactly the extension rule', () => {
  const prior = { started: 0, hardDeadline: 300 * MIN, softDeadline: 290 * MIN };
  const record = (extension, patch = {}) => ({
    started: 6 * MIN,
    deadlineExtension: extension,
    hardDeadline: prior.hardDeadline + extension,
    softDeadline: prior.softDeadline + extension,
    ...patch,
  });
  assert.equal(validExtension(record(6 * MIN), prior, policy), true);
  assert.equal(validExtension(record(0), prior, policy), true);
  assert.equal(validExtension(record(7 * MIN), prior, policy), false, 'more than the time lost');
  assert.equal(
    validExtension(record(30 * MIN, { started: 60 * MIN }), prior, policy),
    false,
    'over the cap',
  );
  assert.equal(
    validExtension(record(6 * MIN, { hardDeadline: prior.hardDeadline }), prior, policy),
    false,
  );
  assert.equal(
    validExtension(record(6 * MIN, { softDeadline: prior.softDeadline }), prior, policy),
    false,
  );
  assert.equal(validExtension(record(-1), prior, policy), false);
  assert.equal(validExtension(record(NaN), prior, policy), false);
  assert.equal(validExtension(record(6 * MIN), prior, {}), false, 'a policy without the field');
});

async function replaced(faultAfterSweeps) {
  const h = await setup({ HARD_RUNTIME_MINUTES: '300', BOOT_LIMIT_MINUTES: '75' });
  Object.assign(h.cloud.offer, { host_id: 100 });
  h.cloud.contracts = [900, 901, 902];
  assert.equal((await h.call()).status, 200);
  h.cloud.extraOffers = [{ ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 }];
  const original = await h.storage.get('record:123-1');
  Object.assign(h.cloud.instances.get(original.id), {
    actual_status: 'created',
    status_msg:
      'Error response from daemon: error gathering device information while adding custom device "/dev/kvm"',
  });
  for (let i = 0; i < faultAfterSweeps; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  const second = await h.call();
  assert.equal(second.status, 200);
  const replacement = await h.storage.get('record:123-1');
  return { h, original, replacement };
}

test('a replacement after a fast boot fault keeps the apps stage reachable', async () => {
  const { original, replacement } = await replaced(9);
  assert.equal(replacement.providerRetries, 1);
  const lost = replacement.started - original.started;
  assert.ok(lost >= 90000 && lost <= 5 * MIN, 'the fault is confirmed within minutes: ' + lost);
  assert.equal(replacement.deadlineExtension, lost);
  assert.equal(replacement.hardDeadline, original.hardDeadline + lost);
  assert.equal(replacement.softDeadline, original.softDeadline + lost);
  assert.equal(replacement.sessionDeadline, original.sessionDeadline);
  assert.equal(replacement.runBudgetUsd, original.runBudgetUsd);
  // apps is due this long after the replacement was rented; the payload starts it only with 245 minutes left
  const appsDue = replacement.started + BOX_READY + BEFORE_APPS;
  assert.ok(replacement.softDeadline - appsDue >= APPS_NEEDS, 'apps starts');
});

test('a replacement rented 8 minutes in misses apps only without the extension', () => {
  const soft = 290 * MIN,
    started = 8 * MIN,
    appsDue = started + BOX_READY + BEFORE_APPS;
  assert.ok(soft - appsDue < APPS_NEEDS, 'unextended');
  assert.ok(soft + 8 * MIN - appsDue >= APPS_NEEDS, 'extended');
});

test('the runner wait budget limits the extension of a late replacement', async () => {
  const h = await setup({ HARD_RUNTIME_MINUTES: '300', BOOT_LIMIT_MINUTES: '75' });
  h.cloud.contracts = [900, 901, 902];
  Object.assign(h.cloud.offer, { host_id: 100 });
  assert.equal((await h.call()).status, 200);
  h.cloud.extraOffers = [{ ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 }];
  const original = await h.storage.get('record:123-1');
  Object.assign(h.cloud.instances.get(original.id), {
    actual_status: 'created',
    status_msg: 'GPU error, unable to start instance',
  });
  for (let i = 0; i < 9; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  const sha = original.sha;
  // the runner has only the hard limit plus the admission margin left: no room for any extension
  const left = (original.hardDeadline - h.time.value) / 1000 + 5 * 60;
  const response = await h.call('/rent', { body: rentBody(sha, { remaining_seconds: left }) });
  assert.equal(response.status, 200);
  const replacement = await h.storage.get('record:123-1');
  assert.equal(replacement.providerRetries, 1);
  assert.equal(replacement.deadlineExtension, 0);
  assert.equal(replacement.hardDeadline, original.hardDeadline);
  assert.equal(replacement.softDeadline, original.softDeadline);
  assert.ok(configuration(env).provider.replacementExtensionMinutes > 0);
});

test('a third leg is measured from the first rental, not summed over the legs', async () => {
  const h = await setup({ HARD_RUNTIME_MINUTES: '300', BOOT_LIMIT_MINUTES: '75' });
  Object.assign(h.cloud.offer, { host_id: 100 });
  h.cloud.contracts = [900, 901, 902];
  assert.equal((await h.call()).status, 200);
  h.cloud.extraOffers = [
    { ...h.cloud.offer, id: 6, machine_id: 100002, host_id: 200, price: 0.1 },
    { ...h.cloud.offer, id: 7, machine_id: 100003, host_id: 300, price: 0.1 },
  ];
  const first = await h.storage.get('record:123-1');
  const stuck = async (id, sweeps) => {
    Object.assign(h.cloud.instances.get(id), {
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance',
    });
    for (let i = 0; i < sweeps; i++) {
      h.time.value += 20000;
      await h.service.sweep();
    }
    assert.equal((await h.call()).status, 200);
    return h.storage.get('record:123-1');
  };
  const second = await stuck('900', 9);
  assert.equal(second.providerRetries, 1);
  const third = await stuck('901', 9);
  assert.equal(third.providerRetries, 2);
  const lost = third.started - first.started;
  assert.ok(lost > second.deadlineExtension);
  assert.equal(third.deadlineExtension, lost);
  assert.equal(third.hardDeadline, first.hardDeadline + lost);
  assert.equal(third.softDeadline, first.softDeadline + lost);
});
