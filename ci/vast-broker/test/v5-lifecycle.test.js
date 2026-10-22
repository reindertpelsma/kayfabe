import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env, nonce, callbackToken, clock } from './helpers.js';
import { configuration } from '../src/policy.js';
// FALSIFIER: a restart/poll/hold moves an absolute deadline, heartbeat is lost,
// grace is counted rather than timestamped, or a box capability removes billing limits.
const minute = 60000;
const v5 = (overrides) =>
  setup({
    HARD_RUNTIME_MINUTES: '180',
    BOOT_LIMIT_MINUTES: '40',
    MANUAL_CEILING_MINUTES: '480',
    PASS_GRACE_MINUTES: '5',
    FAIL_GRACE_MINUTES: '10',
    ...overrides,
  });
const record = (h) => h.storage.get('record:123-1');
async function rent(h) {
  const r = await h.call();
  assert.equal(r.status, 200);
  return r.json();
}
async function progress(h) {
  assert.equal((await h.report({ seq: 1, kind: 'log', text: '' })).status, 200);
}
async function hold(h, token) {
  if (token === undefined && (await record(h)).resultAt === undefined) await finish(h, 'PASS');
  const r = await record(h);
  return h.call('/hold/123-1', {
    token: token ?? callbackToken(r, h.bindings.RUN_TOKEN_SECRET),
    ghToken: null,
  });
}
async function finish(h, verdict) {
  assert.equal(
    (await h.report({ seq: 2, kind: 'result', result: { nonce, verdict } })).status,
    200,
  );
}
for (const label of ['money', 'clock'])
  test('V5 one-place owner defaults ' + label, () => {
    const bindings = { ...env };
    for (const k of [
      'CREDIT_RESERVE_USD',
      'MAX_PRICE_USD_HOUR',
      'TTL_MINUTES',
      'HARD_RUNTIME_MINUTES',
      'BOOT_LIMIT_MINUTES',
      'DAILY_SPEND_USD',
      'ROLLING_SPEND_USD',
    ])
      delete bindings[k];
    const c = configuration(bindings);
    if (label === 'money')
      assert.deepEqual([c.price, c.daily, c.rolling, c.reserve], [1, 6, 30, 8]);
    else
      assert.deepEqual(
        [
          c.boot,
          c.hard,
          c.softMargin,
          c.heartbeat,
          c.heartbeatTimeout,
          c.passGrace,
          c.failGrace,
          c.manualCeiling,
        ].map((x) => x / minute),
        [40, 180, 10, 10, 15, 5, 10, 480],
      );
  });
for (const restart of [false, true])
  test('V5 BOOTING persists absolute creation clocks ' + restart, async () => {
    const h = await v5();
    const first = await rent(h);
    if (restart) h.restart();
    h.time.value += minute;
    await h.call('/status/123-1');
    const r = await record(h);
    assert.equal(first.state, 'BOOTING');
    assert.equal(r.phase, 'BOOTING');
    assert.equal(r.bootDeadline, clock + 40 * minute);
    assert.equal(r.hardDeadline, clock + 180 * minute);
    assert.equal(r.softDeadline, clock + 170 * minute);
    assert.equal(r.manualDeadline, clock + 480 * minute);
  });
for (const restart of [false, true])
  test('V5 boot gets forty minutes before first callback ' + restart, async () => {
    const h = await v5();
    await rent(h);
    if (restart) h.restart();
    h.time.value += 39 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).ended, null);
    h.time.value = clock + 40 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).closed, 'BOOT_TIMEOUT');
    assert.equal(h.cloud.instances.has('900'), false);
  });
for (const restart of [false, true])
  test('V5 empty log heartbeat advances last-post timestamp ' + restart, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    if (restart) h.restart();
    h.time.value += 10 * minute;
    assert.equal((await h.report({ seq: 2, kind: 'log', text: '' })).status, 200);
    h.time.value += 14 * minute;
    await h.service.sweep();
    const r = await record(h);
    assert.equal(r.phase, 'TESTING');
    assert.equal(r.lastPostAt, clock + 10 * minute);
    assert.equal(r.ended, null);
  });
for (const restart of [false, true])
  test('V5 missing heartbeat destroys at fifteen minutes ' + restart, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    if (restart) h.restart();
    h.time.value += 15 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).closed, 'HEARTBEAT_TIMEOUT');
    assert.equal(h.cloud.instances.has('900'), false);
  });
for (const verdict of ['PASS', 'FAIL'])
  test('V5 result grace is stored per verdict ' + verdict, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    await finish(h, verdict);
    h.restart();
    const until = clock + (verdict === 'PASS' ? 5 : 10) * minute;
    assert.equal((await record(h)).resultDeadline, until);
    h.time.value = until - 1;
    await h.service.sweep();
    assert.equal((await record(h)).ended, null);
    h.time.value = until;
    await h.service.sweep();
    assert.equal((await record(h)).ended, until);
    const status = await (await h.call('/status/123-1')).json();
    assert.equal(status.state, 'NONE');
    assert.equal(status.verdict, verdict);
  });
for (const verdict of ['PASS', 'FAIL'])
  test('V5 authenticated hold stops grace and heartbeat ' + verdict, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    await finish(h, verdict);
    assert.equal((await hold(h)).status, 200);
    h.restart();
    h.time.value += 20 * minute;
    await h.service.sweep();
    const r = await record(h);
    assert.equal(r.phase, 'MANUAL');
    assert.equal(r.ended, null);
    assert.equal(r.resultDeadline, null);
    assert.equal(r.hardDeadline, clock + 180 * minute);
  });
for (const restart of [false, true])
  test('V5 repeated hold cannot extend hard or manual ceiling ' + restart, async () => {
    const h = await v5({ HARD_RUNTIME_MINUTES: '20' });
    await rent(h);
    await progress(h);
    assert.equal((await hold(h)).status, 200);
    if (restart) h.restart();
    h.time.value += 10 * minute;
    assert.equal((await hold(h)).status, 200);
    const r = await record(h);
    assert.equal(r.manualDeadline, clock + 20 * minute);
    assert.equal(r.hardDeadline, clock + 20 * minute);
    h.time.value = clock + 20 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).closed, 'HARD_TIMEOUT');
  });
for (const restart of [false, true])
  test('V5 owner stop-timer leaves absolute MANUAL ceiling ' + restart, async () => {
    const h = await v5({ DAILY_SPEND_USD: '6' });
    await rent(h);
    await progress(h);
    await hold(h);
    const response = await h.call('/stop-timer/123-1', { token: env.OWNER_STOP_TOKEN });
    assert.equal(response.status, 200);
    if (restart) h.restart();
    assert.equal((await record(h)).hardDeadline, null);
    h.time.value = clock + 181 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).ended, null);
    h.time.value = clock + 480 * minute;
    await h.service.sweep();
    assert.equal((await record(h)).closed, 'MANUAL_TIMEOUT');
  });
for (const auth of ['box', 'oidc'])
  test('V5 only owner may remove hard limit ' + auth, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    await hold(h);
    const token =
      auth === 'box' ? callbackToken(await record(h), h.bindings.RUN_TOKEN_SECRET) : undefined;
    assert.equal((await h.call('/stop-timer/123-1', { token })).status, 401);
    assert.equal((await record(h)).hardDeadline, clock + 180 * minute);
  });
for (const token of ['bad', env.OWNER_STOP_TOKEN])
  test('V5 hold refuses non-box capability ' + token.slice(0, 3), async () => {
    const h = await v5();
    await rent(h);
    assert.equal((await hold(h, token)).status, 401);
    assert.notEqual((await record(h)).phase, 'MANUAL');
  });
for (const restart of [false, true])
  test('V5 MANUAL cron always writes Worker-authored status ' + restart, async () => {
    const h = await v5();
    await rent(h);
    await progress(h);
    await hold(h);
    if (restart) h.restart();
    const before = (await record(h)).logChunks;
    h.time.value += minute;
    await h.service.sweep();
    h.time.value += minute;
    await h.service.sweep();
    const r = await record(h);
    assert.ok(r.logChunks >= before + 2);
    const chunks = [];
    for (let i = before; i < r.logChunks; i++) chunks.push(await h.storage.get('log:123-1:' + i));
    assert.ok(chunks.join('').includes('[Worker] MANUAL'));
  });
for (const restriction of ['daily', 'credit'])
  test('V5 stop-timer checks extended billing liability ' + restriction, async () => {
    const h = await v5({
      MAX_PRICE_USD_HOUR: '0.8',
      DAILY_SPEND_USD: restriction === 'daily' ? '6' : '20',
    });
    h.cloud.price = 0.8;
    await rent(h);
    await progress(h);
    await hold(h);
    if (restriction === 'credit') h.cloud.credit = 9;
    assert.equal((await h.call('/stop-timer/123-1', { token: env.OWNER_STOP_TOKEN })).status, 409);
    assert.equal((await record(h)).hardDeadline, clock + 180 * minute);
  });
for (const delay of [0, 5 * minute])
  test('V5 CREATING reservation precedes provider side effect ' + delay, async () => {
    const h = await v5();
    const original = h.cloud.fetch;
    let seen;
    h.cloud.fetch = async (url, init) => {
      if (String(url).includes('/asks/')) {
        seen = await record(h);
        h.time.value += delay;
      }
      return original(url, init);
    };
    h.restart();
    await rent(h);
    assert.equal(seen.phase, 'CREATING');
    assert.equal(seen.id, null);
    assert.equal((await record(h)).phase, 'BOOTING');
    assert.equal((await record(h)).hardDeadline, clock + 180 * minute);
    assert.equal(
      h.cloud.calls.find((c) => c.url.includes('/asks/')).body.env.KF_SOFT_DEADLINE,
      String((clock + 170 * minute) / 1000),
    );
  });
async function unbound() {
  const h = await setup();
  for (const key of [
    'MAX_PRICE_USD_HOUR',
    'DAILY_SPEND_USD',
    'ROLLING_SPEND_USD',
    'CREDIT_RESERVE_USD',
    'TTL_MINUTES',
    'HARD_RUNTIME_MINUTES',
    'BOOT_LIMIT_MINUTES',
  ])
    delete h.bindings[key];
  h.restart();
  return h;
}
test('V5 default price admits approved sub-cap production offer', async () => {
  const h = await unbound();
  h.cloud.price = 0.7;
  assert.equal((await h.call()).status, 200);
});
test('V5 default daily allowance admits existing five-dollar liability', async () => {
  const h = await unbound();
  await h.storage.put('cost:old', {
    started: clock - 3600000,
    ended: clock,
    price: 5,
    ttl: 3600000,
  });
  assert.equal((await h.call()).status, 200);
});
test('V5 default reserve refuses insufficient prepaid credit', async () => {
  const h = await unbound();
  h.cloud.credit = 8.5;
  assert.equal((await h.call()).status, 409);
});
test('V5 default rolling cap accounts for earlier-day cost', async () => {
  const h = await unbound();
  await h.storage.put('cost:old', {
    started: clock - 2 * 86400000,
    ended: clock - 86400000,
    price: 29.8 / 24,
    ttl: 86400000,
  });
  assert.equal((await h.call()).status, 409);
});
test('V5 default reservation binds owner clock plan into the lease', async () => {
  const h = await unbound();
  await rent(h);
  const r = await record(h);
  assert.equal(r.bootDeadline, clock + 40 * minute);
  assert.equal(r.hardDeadline, clock + 180 * minute);
  assert.equal(r.softDeadline, clock + 170 * minute);
  assert.equal(r.manualDeadline, clock + 480 * minute);
});
for (const verdict of ['PASS', 'FAIL'])
  test('V5 unbound default grace reaches the stored deadline ' + verdict, async () => {
    const h = await unbound();
    await rent(h);
    await progress(h);
    await finish(h, verdict);
    assert.equal((await record(h)).resultDeadline, clock + (verdict === 'PASS' ? 5 : 10) * minute);
  });
