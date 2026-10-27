import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, callbackToken, env } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';

// FALSIFIER: a provider fault already gets one fresh-machine retry, preserving total time/money.
const row = (h) => h.storage.get('record:123-1');
async function fixture(bindings = {}, offerPatch = {}) {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '40', ...bindings });
  Object.assign(h.cloud.offer, offerPatch);
  h.cloud.contracts = [900, 901, 902];
  assert.equal((await h.call()).status, 200);
  const original = await row(h);
  h.cloud.extraOffers = [{ ...h.cloud.offer, id: 6, machine_id: 100002, price: 0.1 }];
  return { h, original };
}
async function fault(h, kind = 'report') {
  const r = await row(h);
  if (kind !== 'report') {
    h.cloud.instances.get(r.id).actual_status = kind === 'running-boot' ? 'running' : 'created';
    h.cloud.logText =
      "libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C." +
      r.id +
      "'\n";
    for (const at of [r.started + 60000, r.started + 240000, r.bootDeadline - 60000]) {
      h.time.value = at;
      await h.service.sweep();
    }
    h.time.value = r.bootDeadline;
    return h.call('/status/123-1');
  }
  return h.report({
    seq: 1,
    kind: 'result',
    result: {
      nonce: r.nonce,
      verdict: 'PROVIDER_FAULT',
      suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
      hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
    },
  });
}
for (const kind of ['boot', 'running-boot', 'report'])
  for (const restart of [false, true])
    test('green one provider replacement ' + kind + ' restart=' + restart, async () => {
      const { h, original } = await fixture();
      const stale = callbackToken(original, env.RUN_TOKEN_SECRET);
      assert.equal((await fault(h, kind)).status, 200);
      const failed = await row(h);
      assert.equal(failed.verdict, 'PROVIDER_FAULT');
      assert.notEqual(failed.ended, null);
      assert.equal(h.cloud.instances.has('900'), false);
      assert.ok(await h.storage.get('bad-machine:100001'));
      if (restart) h.restart();
      const response = await h.call();
      assert.equal(response.status, 200);
      const data = await response.json();
      assert.equal(data.id, '901');
      assert.equal(data.provider_retries, 1);
      assert.equal(data.machine_id, 100002);
      const replacement = await row(h);
      assert.equal(replacement.hardDeadline, original.hardDeadline);
      assert.equal(replacement.softDeadline, original.softDeadline);
      assert.equal(replacement.runBudgetUsd, original.runBudgetUsd);
      assert.notEqual(replacement.nonce, original.nonce);
      const environment = h.cloud.calls.filter((call) => call.url.includes('/api/v0/asks/')).at(-1)
        .body.env;
      assert.equal(environment.KF_NONCE, replacement.nonce);
      assert.equal(h.cloud.created.length, 2);
      assert.equal(
        (await h.report({ seq: 2, kind: 'log', text: 'stale' }, { token: stale })).status,
        401,
      );
      assert.equal((await fault(h)).status, 200);
      const again = await h.call();
      assert.equal(again.status, 200);
      assert.equal((await again.json()).verdict, 'PROVIDER_FAULT');
      assert.equal(h.cloud.created.length, 2);
      assert.deepEqual(h.cloud.deletes.sort(), ['900', '901']);
      assert.ok(h.cloud.instances.has('777') && h.cloud.instances.has('888'));
      const costs = [...(await h.storage.list({ prefix: 'cost:' }))];
      assert.equal(costs.length, 2);
      const budget = await new Ledger(h.storage).budgetSummary(
        { ...configuration(env), profile: 'check' },
        h.time.value,
      );
      assert.equal(budget.profiles.check.count_today, 1);
      assert.ok(budget.rolling_usd <= original.runBudgetUsd);
    });
for (const verdict of ['FAIL', 'UNAVAILABLE'])
  test('green ordinary result does not retry ' + verdict, async () => {
    const { h, original } = await fixture();
    assert.equal(
      (await h.report({ seq: 1, kind: 'result', result: { nonce: original.nonce, verdict } }))
        .status,
      200,
    );
    assert.equal((await h.call()).status, 200);
    assert.equal(h.cloud.created.length, 1);
    assert.equal(await h.storage.get('bad-machine:100001'), undefined);
  });
for (const kind of ['boot', 'report'])
  test('green replacement needs confirmed cleanup ' + kind, async () => {
    const { h } = await fixture();
    const fetcher = h.cloud.fetch;
    h.cloud.fetch = async (url, init = {}) =>
      init.method === 'DELETE' ? Response.json({ success: true }) : fetcher(url, init);
    h.restart();
    await fault(h, kind);
    await h.call();
    assert.equal(h.cloud.created.length, 1);
    assert.equal((await row(h)).ended, null);
    assert.ok(h.cloud.instances.has('900'));
  });
for (const price of [0.4, 0.45])
  test('green replacement cannot enlarge run allowance ' + price, async () => {
    const { h } = await fixture({ DAILY_SPEND_USD: '0.7' });
    await fault(h, 'boot');
    h.cloud.extraOffers[0].price = price;
    h.time.value += 60001;
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).verdict, 'PROVIDER_FAULT');
    assert.equal(h.cloud.created.length, 1);
    assert.equal((await row(h)).verdict, 'PROVIDER_FAULT');
  });

for (const kind of ['pass', 'fault', 'budget', 'fail', 'unavailable', 'busy'])
  for (const variant of [1, 2])
    test('green BASE runner provider replacement ' + kind + variant, async () => {
      const { spawnSync } = await import('node:child_process');
      const r = spawnSync(
        'python3',
        [
          '-I',
          new URL('./green_provider_runner_fixture.py', import.meta.url).pathname,
          kind + variant,
        ],
        { encoding: 'utf8' },
      );
      assert.equal(r.status, 0, r.stderr || r.stdout);
    });

for (const price of [0.4, 0.45])
  test('green actual replacement contract retains allowance ' + price, async () => {
    const { h } = await fixture({ DAILY_SPEND_USD: '0.7' });
    assert.equal((await fault(h)).status, 200);
    const fetcher = h.cloud.fetch;
    h.cloud.fetch = async (url, init = {}) => {
      const response = await fetcher(url, init);
      if (
        String(url).replace(/\/$/, '').endsWith('/instances/901') &&
        (!init.method || init.method === 'GET') &&
        response.status === 200
      ) {
        const data = await response.json();
        if (data.instances) {
          data.instances.dph_total = price;
          data.instances.dph_base = price - 0.02;
        }
        return Response.json(data);
      }
      return response;
    };
    h.restart();
    const response = await h.call();
    assert.equal(response.status, 409);
    assert.equal((await response.json()).reason, 'BUDGET:run-usd');
    assert.equal(h.cloud.instances.has('901'), false);
    assert.equal((await row(h)).verdict, 'PROVIDER_FAULT');
    assert.equal((await h.storage.get('cost:123-1')).price, price);
  });
for (const kind of ['daily-usd', 'rolling-usd', 'credit-reserve', 'daily-count'])
  for (const variant of [1, 2])
    test('green replacement retains ' + kind + variant, async () => {
      const { h } = await fixture();
      assert.equal((await fault(h)).status, 200);
      if (kind === 'credit-reserve') h.cloud.credit = 8;
      else
        for (let i = 0; i < (kind === 'daily-count' ? 3 : 1); i++)
          await h.storage.put('cost:other-' + variant + '-' + i, {
            started: h.time.value - 3600000,
            ended: h.time.value,
            price: kind === 'daily-count' ? 0 : kind === 'daily-usd' ? 3 : 30,
            profile: kind === 'rolling-usd' ? 'sweep' : 'check',
            created: true,
          });
      const response = await h.call();
      if (kind === 'daily-count') {
        assert.equal(response.status, 200);
        assert.equal((await response.json()).provider_retries, 1);
        assert.equal(h.cloud.created.length, 2);
        await assert.rejects(
          new Ledger(h.storage).budgets(configuration(env), h.time.value, 0.1),
          (error) => error.reason === 'BUDGET:daily-count',
        );
      } else {
        assert.equal(response.status, 409);
        assert.equal((await response.json()).reason, 'BUDGET:' + kind);
        assert.equal(h.cloud.created.length, 1);
      }
    });

for (const price of [0.21, 0.25])
  test('green replacement can use a higher quote within configured caps ' + price, async () => {
    const { h } = await fixture();
    assert.equal((await fault(h)).status, 200);
    h.cloud.extraOffers[0].price = price;
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).id, '901');
    assert.equal(h.cloud.created.length, 2);
  });

for (const kind of ['report', 'boot'])
  test('green provider replacement keeps one daily slot ' + kind, async () => {
    const { h } = await fixture({ DAILY_RENTALS: '1' });
    assert.equal((await fault(h, kind)).status, 200);
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).id, '901');
    const cfg = configuration({ ...env, DAILY_RENTALS: '1' });
    const budget = await new Ledger(h.storage).budgetSummary(cfg, h.time.value);
    assert.equal(budget.profiles.check.count_today, 1);
  });
for (const host of [200, 201])
  test('green provider replacement prefers a different eligible host ' + host, async () => {
    const { h } = await fixture({}, { host_id: 100 });
    h.cloud.extraOffers.push({
      ...h.cloud.extraOffers[0],
      id: 7,
      machine_id: 100003,
      host_id: host,
      price: 0.11,
    });
    assert.equal((await fault(h)).status, 200);
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).host_id, host);
  });
for (const invalid of [{ verified: false }, { price: 2 }])
  test('green other-host preference retains eligibility ' + JSON.stringify(invalid), async () => {
    const { h } = await fixture({}, { host_id: 100 });
    h.cloud.extraOffers.push({
      ...h.cloud.extraOffers[0],
      id: 7,
      machine_id: 100003,
      host_id: 200,
      ...invalid,
    });
    assert.equal((await fault(h)).status, 200);
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).machine_id, 100002);
  });
