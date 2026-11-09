import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env, clock } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';
const record = (h) => h.storage.get('record:123-1');
async function boot(vars = {}) {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '75', ...vars });
  h.cloud.contracts = Array.from({ length: 16 }, (_, i) => 900 + i);
  h.cloud.offer.host_id = 100;
  assert.equal((await h.call()).status, 200);
  h.cloud.instances.get('900').actual_status = 'created';
  return h;
}
async function fatal(h, text = 'Error: GPU error, unable to start instance.') {
  const r = await record(h);
  h.cloud.instances.get(r.id).status_msg = text;
  for (let i = 0; i < 2; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  return record(h);
}
for (const restart of [false, true]) {
  test('session fatal confirmed quickly ' + restart, async () => {
    const h = await boot();
    h.cloud.instances.get('900').status_msg = 'Error: GPU error, unable to start instance.';
    h.time.value += 20000;
    await h.service.sweep();
    assert.ok(h.cloud.instances.has('900'));
    if (restart) h.restart();
    h.time.value += 20000;
    await h.service.sweep();
    const r = await record(h);
    assert.equal(r.verdict, 'PROVIDER_FAULT');
    assert.notEqual(r.ended, null);
    assert.deepEqual(h.cloud.deletes, ['900']);
    assert.ok(await h.storage.get('provider-machine:100001'));
    assert.equal(await h.storage.get('bad-machine:100001'), undefined);
  });
  test('session repeated replacements retain accounting ' + restart, async () => {
    const h = await boot();
    for (let i = 0; i < 3; i++) {
      const old = await fatal(h);
      assert.equal(old.verdict, 'PROVIDER_FAULT');
      h.cloud.extraOffers.push({
        ...h.cloud.offer,
        id: 20 + i,
        machine_id: 200000 + i,
        host_id: 200 + i,
        price: 0.21,
      });
      if (restart) h.restart();
      const response = await h.call();
      assert.equal(response.status, 200);
      const r = await record(h);
      assert.equal(r.providerRetries, i + 1);
      assert.equal(r.originalStarted, clock);
      assert.equal(r.hardDeadline, clock + 180 * 60000);
      assert.equal(r.rentalAttempts.length, i + 1);
      assert.equal((await h.storage.list({ prefix: 'cost:' })).size, i + 2);
      assert.equal(h.cloud.instances.has(old.id), false);
    }
    await h.call('/stop/123-1');
  });
  test('session first report permanently forbids replacement ' + restart, async () => {
    const h = await boot();
    h.cloud.instances.get('900').actual_status = 'running';
    const r = await record(h);
    await h.report({ seq: 1, kind: 'log', text: 'first report' });
    if (restart) h.restart();
    await h.report({
      seq: 2,
      kind: 'result',
      result: {
        nonce: r.nonce,
        verdict: 'PROVIDER_FAULT',
        suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
      },
    });
    assert.equal((await record(h)).verdict, 'FAIL');
    await h.call();
    assert.equal(h.cloud.created.length, 1);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
}
for (const msg of [
  'Verifying Checksum',
  'Preparing GPUs...',
  'image pull error: retrying',
  'daemon error: transient retry',
  'GPU error in a test',
  'Domain not found: no domain with matching name C.901',
])
  test('session ignores nonfatal ' + msg, async () => {
    const h = await boot();
    await fatal(h, msg);
    assert.equal((await record(h)).ended, null);
    assert.equal(h.cloud.deletes.length, 0);
    await h.call('/stop/123-1');
  });
for (const limit of [2, 3])
  test('session attempt cap ' + limit, async () => {
    const h = await boot({ PROVIDER_MAX_ATTEMPTS: String(limit) });
    h.cloud.extraOffers = Array.from({ length: 8 }, (_, i) => ({
      ...h.cloud.offer,
      id: 20 + i,
      machine_id: 200000 + i,
      host_id: 200 + i,
      price: 0.21,
    }));
    for (let i = 0; i < limit; i++) {
      await fatal(h);
      await h.call();
    }
    assert.equal(h.cloud.created.length, limit);
    const r = await record(h);
    assert.ok(r.replacementExhausted);
    assert.match(r.log, /rental attempts exhausted/);
    for (const a of r.rentalAttempts)
      assert.match(
        r.log,
        new RegExp(
          'machine=' + a.machine + ' host=' + a.host + ' reason=' + a.reason + ' seconds=',
        ),
      );
  });

for (const [gpu, die, skipped] of [
  ['RTX_4080', 'AD103', 'AD102'],
  ['RTX_4060_TI', 'AD106', 'AD102,AD103,AD104'],
])
  test('session middle pin actually admits ' + die, async () => {
    const h = await setup();
    h.cloud.offer.gpu_name = gpu;
    h.cloud.offer.compute_cap = 890;
    const response = await h.call();
    assert.equal(response.status, 200);
    const r = await record(h);
    assert.equal(r.die, die);
    assert.equal(r.selectionReason, 'PIN');
    assert.match(r.log, new RegExp('pin_skipped=' + skipped));
    await h.call('/stop/123-1');
  });

for (const restart of [false, true])
  test('session waits for fresh market within original deadline ' + restart, async () => {
    const h = await boot();
    const old = await fatal(h);
    if (restart) h.restart();
    const waiting = await (await h.call()).json();
    assert.equal(waiting.provider_retry_allowed, true);
    assert.equal(h.cloud.creates, 1);
    assert.equal((await record(h)).replacementExhausted, undefined);
    h.time.value += 60000;
    h.cloud.extraOffers.push({
      ...h.cloud.offer,
      id: 25,
      machine_id: 2025,
      host_id: 225,
      price: 0.21,
    });
    assert.equal((await h.call()).status, 200);
    const next = await record(h);
    assert.equal(next.providerRetries, 1);
    assert.equal(next.sessionDeadline, old.sessionDeadline);
    assert.equal(h.cloud.instances.has(old.id), false);
    await h.call('/stop/123-1');
  });

for (const retry of [false, true])
  test('session pinned fallback respects remaining spending room ' + retry, async () => {
    const h = await setup({
      HARD_RUNTIME_MINUTES: '180',
      BOOT_LIMIT_MINUTES: '75',
      DAILY_SPEND_USD: '3',
      ACCOUNT_DAILY_USD: '0.7',
    });
    h.cloud.offer.host_id = 100;
    h.cloud.contracts = [900, 901];
    if (retry) {
      assert.equal((await h.call()).status, 200);
      await fatal(h);
      h.cloud.extraOffers.push({
        ...h.cloud.offer,
        id: 8,
        machine_id: 222,
        host_id: 222,
        price: 0.2,
      });
    }
    h.cloud.extraOffers.push({
      ...h.cloud.offer,
      id: 7,
      machine_id: 7777,
      host_id: 7777,
      gpu_name: 'RTX_4090',
      compute_cap: 890,
      price: 0.49,
    });
    assert.equal((await h.call()).status, 200);
    const r = await record(h);
    assert.equal(r.die, 'AD104');
    assert.equal(r.selectionReason, 'PIN');
    assert.ok(r.pinSkipped.includes('AD102'));
    assert.equal(h.cloud.created.length, retry ? 2 : 1);
    await h.call('/stop/123-1');
  });

for (const price of [0.4, 0.5])
  test('session cumulative transfers obey original total-run ceiling ' + price, async () => {
    const vars = { MAX_PRICE_USD_HOUR: String(price) };
    const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '75', ...vars });
    h.cloud.offer.host_id = 100;
    h.cloud.offer.inet_down_cost = 0.005;
    h.cloud.contracts = Array.from({ length: 16 }, (_, i) => 900 + i);
    h.cloud.extraOffers = Array.from({ length: 10 }, (_, i) => ({
      ...h.cloud.offer,
      id: 20 + i,
      machine_id: 200000 + i,
      host_id: 200 + i,
      price: 0.21,
    }));
    assert.equal((await h.call()).status, 200);
    const cap = (price * (180 + 1)) / 60;
    for (let i = 0; i < 8; i++) {
      const r = await record(h);
      if (r.ended !== null) break;
      await fatal(h);
      await h.call();
    }
    const r = await record(h);
    assert.ok(r.runBudgetUsd <= cap, 'the original per-run ceiling is retained');
    const ledger = new Ledger(h.storage);
    const cost = await ledger.budgetSummary(configuration({ ...env, ...vars }), h.time.value);
    assert.ok(
      cost.rolling_usd <= cap,
      'failed transfers plus the live reservation fit the run cap',
    );
    assert.ok(h.cloud.created.length < 8, 'daily room cannot fund replacements above the run cap');
    if (r.ended === null) await h.call('/stop/123-1');
    for (const c of h.cloud.created) assert.equal(h.cloud.instances.has(String(c.contract)), false);
  });
