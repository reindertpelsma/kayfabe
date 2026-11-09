import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, clock, env } from './helpers.js';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';
import { rentalLine } from '../src/rental.js';
const row = (h) => h.storage.get('record:123-1');
async function market(vars = {}) {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '75', ...vars });
  h.cloud.offer.host_id = 100;
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 900 + i);
  h.cloud.extraOffers = Array.from({ length: 10 }, (_, i) => ({
    ...h.cloud.offer,
    id: 20 + i,
    machine_id: 200000 + i,
    host_id: 200 + i,
    price: 0.21 + i * 0.001,
  }));
  return h;
}
async function rent(h) {
  const response = await h.call();
  assert.equal(response.status, 200);
  return row(h);
}
async function fatal(h) {
  const r = await row(h);
  Object.assign(h.cloud.instances.get(r.id), {
    actual_status: 'created',
    status_msg: 'GPU error, unable to start instance',
  });
  for (let i = 0; i < 2; i++) {
    h.time.value += 20000;
    await h.service.sweep();
  }
  return row(h);
}
async function clean(h) {
  const r = await row(h);
  if (r?.ended === null) await h.call('/stop/123-1');
  for (let i = 0; i < 3; i++) await h.service.sweep();
  for (const c of h.cloud.created) {
    assert.equal(h.cloud.instances.has(String(c.contract)), false, 'I1 no leaked owned rental');
    assert.ok(h.cloud.inspected.includes(String(c.contract)), 'I1 absence inspected');
  }
  assert.equal((await new Ledger(h.storage).openRecords()).length, 0);
  assert.ok(
    h.cloud.instances.has('777') && h.cloud.instances.has('888'),
    'foreign instances untouched',
  );
  const b = await new Ledger(h.storage).budgetSummary(
    configuration({ ...env, HARD_RUNTIME_MINUTES: '180' }),
    h.time.value,
  );
  assert.ok(b.rolling_usd <= 3, 'I2 cumulative spend cap');
}
for (const v of [1, 2]) {
  test('worst every offer fatal-fails ' + v, async () => {
    const h = await market({ PROVIDER_MAX_ATTEMPTS: String(v + 2) });
    await rent(h);
    for (let i = 0; i < v + 2; i++) {
      await fatal(h);
      await rent(h);
    }
    const r = await row(h);
    assert.equal(h.cloud.created.length, v + 2);
    assert.equal(r.replacementExhausted, true);
    assert.equal(r.verdict, 'PROVIDER_FAULT');
    assert.equal(r.rentalAttempts.length, v + 2);
    assert.equal(new Set(r.rentalAttempts.map((a) => a.machine)).size, v + 2);
    assert.equal(new Set(r.rentalAttempts.map((a) => a.host)).size, v + 2);
    for (const a of r.rentalAttempts)
      assert.match(
        r.log,
        new RegExp(
          'machine=' + a.machine + ' host=' + a.host + ' reason=' + a.reason + ' seconds=',
        ),
      );
    await clean(h);
  });
  test('worst same host twice is skipped ' + v, async () => {
    const h = await market();
    h.cloud.extraOffers[0].host_id = 100;
    await rent(h);
    await fatal(h);
    const r = await rent(h);
    assert.notEqual(r.hostId, 100);
    assert.notEqual(r.machineId, 100001);
    await clean(h);
  });
  test('worst all candidates penalised remain usable ' + v, async () => {
    const h = await market();
    const ledger = new Ledger(h.storage);
    for (const o of [h.cloud.offer, ...h.cloud.extraOffers])
      await ledger.markProviderFault(
        { key: 'old-' + o.id, nonce: 'x', machineId: o.machine_id, hostId: o.host_id },
        'GPU_START_ERROR',
        clock,
        configuration(env).provider,
      );
    await rent(h);
    assert.equal(h.cloud.created.length, 1);
    assert.equal((await h.storage.list({ prefix: 'bad-machine:' })).size, 0);
    await clean(h);
  });
  test('worst offer disappears before create ' + v, async () => {
    const h = await market();
    const f = h.cloud.fetch;
    let failed = false;
    h.cloud.fetch = async (url, init) => {
      if (!failed && String(url).includes('/asks/')) {
        failed = true;
        return Response.json({ success: false });
      }
      return f(url, init);
    };
    h.restart();
    let r = await rent(h);
    assert.equal(r.verdict, 'PROVIDER_FAULT');
    assert.equal(r.noInstance, true);
    assert.equal(h.cloud.creates, 0);
    r = await rent(h);
    assert.equal(r.providerRetries, 1);
    assert.notEqual(r.hostId, 100);
    assert.equal(h.cloud.creates, 1);
    await clean(h);
  });
  test('worst foreign create response recovers only owned instance ' + v, async () => {
    const h = await market();
    const f = h.cloud.fetch;
    let changed = false;
    h.cloud.fetch = async (url, init) => {
      const response = await f(url, init);
      if (!changed && String(url).includes('/asks/')) {
        changed = true;
        return Response.json({ success: true, new_contract: 888 });
      }
      return response;
    };
    h.restart();
    const r = await rent(h);
    assert.equal(r.id, '900');
    assert.equal(r.verdict, 'PROVIDER_FAULT');
    assert.deepEqual(h.cloud.deletes, ['900']);
    assert.ok(h.cloud.instances.has('888'));
    await clean(h);
  });
  test('worst created forever expires within session ' + v, async () => {
    const h = await market({ PROVIDER_SESSION_MINUTES: String(v + 1) });
    const r = await rent(h);
    h.cloud.instances.get(r.id).actual_status = 'created';
    h.time.value = r.sessionDeadline;
    await h.service.sweep();
    await rent(h);
    const end = await row(h);
    assert.equal(end.verdict, 'PROVIDER_FAULT');
    assert.equal(end.replacementExhausted, true);
    assert.equal(h.cloud.creates, 1);
    assert.ok(end.ended <= r.sessionDeadline);
    await clean(h);
  });
  test('worst fatal recovery fatal needs fresh confirmation ' + v, async () => {
    const h = await market();
    const r = await rent(h),
      i = h.cloud.instances.get(r.id);
    i.status_msg = 'GPU error, unable to start instance';
    h.time.value += 20000;
    await h.service.sweep();
    i.status_msg = 'Preparing GPUs...';
    h.time.value += 20000;
    await h.service.sweep();
    i.status_msg = 'GPU error, unable to start instance';
    h.time.value += 20000;
    await h.service.sweep();
    assert.equal((await row(h)).ended, null);
    h.time.value += 20000;
    await h.service.sweep();
    assert.notEqual((await row(h)).ended, null);
    await clean(h);
  });
  for (const mode of ['5xx', 'timeout', 'garbage'])
    test('worst provider ' + mode + ' during destroy and absence verification ' + v, async () => {
      const h = await market();
      await rent(h);
      const f = h.cloud.fetch;
      let broken = true,
        deleted = false;
      h.cloud.fetch = async (url, init = {}) => {
        if (
          String(url).includes('/instances/900') &&
          (init.method === 'DELETE' || (deleted && (!init.method || init.method === 'GET'))) &&
          broken
        ) {
          if (init.method === 'DELETE') deleted = true;
          if (mode === 'timeout') throw Error('mock timeout');
          return new Response(mode === 'garbage' ? 'not-json' : '{}', {
            status: mode === '5xx' ? 503 : 200,
          });
        }
        return f(url, init);
      };
      h.restart();
      await fatal(h);
      await h.call();
      assert.equal(h.cloud.created.length, 1, 'I1 no overlapping replacement');
      assert.equal((await row(h)).ended, null);
      assert.ok(h.cloud.instances.has('900'));
      broken = false;
      h.time.value += 60000;
      await h.service.sweep();
      assert.notEqual((await row(h)).ended, null);
      await rent(h);
      assert.equal(h.cloud.created.length, 2);
      await clean(h);
    });
  test('worst cron runner race waits for confirmed teardown ' + v, async () => {
    const h = await market();
    await rent(h);
    const r = await fatal(h);
    assert.notEqual(r.ended, null);
    const results = await Promise.allSettled([
      h.call(),
      h.call(),
      h.service.sweep(),
      h.call('/status/123-1', { method: 'GET' }),
    ]);
    assert.equal(results.length, 4);
    assert.equal(h.cloud.created.length, 2);
    await clean(h);
  });
  test('worst daily cap reached mid replacement ' + v, async () => {
    const h = await market();
    await rent(h);
    await fatal(h);
    await h.storage.put('cost:other-' + v, {
      profile: 'check',
      started: clock - 3600000,
      ended: clock,
      price: 3,
      created: true,
    });
    const r = await rent(h);
    assert.equal(r.replacementExhausted, true);
    assert.equal(h.cloud.created.length, 1);
    assert.match(r.log, /rental attempts exhausted/);
    // The independent spend deliberately fills the daily cap; owned rental teardown still completes.
    assert.equal(h.cloud.instances.has('900'), false);
    assert.equal((await new Ledger(h.storage).openRecords()).length, 0);
  });
  test('worst cost cap reached mid retry ' + v, async () => {
    const h = await market({ DAILY_SPEND_USD: '0.7' });
    await rent(h);
    h.time.value += 35 * 60000;
    await fatal(h);
    for (const o of h.cloud.extraOffers) o.price = 0.45;
    const r = await rent(h);
    assert.equal(h.cloud.created.length, 1);
    assert.equal(r.replacementExhausted, undefined);
    h.time.value = r.sessionDeadline;
    const exhausted = await rent(h);
    assert.equal(exhausted.replacementExhausted, true);
    await clean(h);
  });
  test('worst duplicate machine ids and restart ' + v, async () => {
    const h = await market();
    h.cloud.extraOffers[0].machine_id = h.cloud.offer.machine_id;
    await rent(h);
    await fatal(h);
    h.restart();
    const r = await rent(h);
    assert.notEqual(r.machineId, h.cloud.offer.machine_id);
    await clean(h);
  });
  test('worst clock skew cannot reopen session ' + v, async () => {
    const h = await market();
    await rent(h);
    await fatal(h);
    h.time.value = clock - 1;
    const response = await h.call();
    assert.ok([200, 409, 503].includes(response.status));
    assert.equal(h.cloud.created.length, 1);
    h.time.value = clock + 120000;
    await clean(h);
  });
  test('worst full IP and ssh port absent in every attempt ' + v, async () => {
    const h = await market();
    const f = h.cloud.fetch,
      ip = '203.0.113.' + (50 + v),
      port = 45000 + v;
    h.cloud.fetch = async (url, init = {}) => {
      const response = await f(url, init);
      if (
        String(url).includes('/instances/') &&
        (!init.method || init.method === 'GET') &&
        response.status === 200
      ) {
        const d = await response.json();
        if (d.instances) Object.assign(d.instances, { public_ipaddr: ip, ssh_port: port });
        return Response.json(d);
      }
      return response;
    };
    h.restart();
    await rent(h);
    for (let n = 0; n < 2; n++) {
      const r = await row(h);
      h.cloud.instances.get(r.id).status_msg =
        'GPU error, unable to start instance ' + ip + ':' + port + ' ssh_port=' + port;
      await fatal(h);
      await rent(h);
    }
    const r = await row(h);
    for (const text of [
      r.log,
      rentalLine(r),
      JSON.stringify(await (await h.call('/status/123-1', { method: 'GET' })).json()),
    ]) {
      assert.equal(text.includes(ip), false);
      assert.equal(text.includes(String(port)), false);
    }
    await clean(h);
  });
}

for (const v of [1, 2]) {
  for (const mode of ['5xx', 'timeout', 'garbage']) {
    test('worst ' + mode + ' while verifying absence ' + v, async () => {
      const h = await market();
      await rent(h);
      const f = h.cloud.fetch;
      let deleted = false,
        broken = true;
      h.cloud.fetch = async (url, init = {}) => {
        if (String(url).includes('/instances/900') && init.method === 'DELETE') {
          const response = await f(url, init);
          deleted = true;
          return response;
        }
        if (
          deleted &&
          broken &&
          String(url).includes('/instances/900') &&
          (!init.method || init.method === 'GET')
        ) {
          if (mode === 'timeout') throw Error('mock timeout');
          return new Response(mode === 'garbage' ? 'not-json' : '{}', {
            status: mode === '5xx' ? 503 : 200,
          });
        }
        return f(url, init);
      };
      h.restart();
      await fatal(h);
      await h.call();
      assert.equal(h.cloud.creates, 1);
      assert.equal((await row(h)).ended, null);
      assert.equal(h.cloud.instances.has('900'), false);
      broken = false;
      h.time.value += 60000;
      await h.service.sweep();
      assert.notEqual((await row(h)).ended, null);
      await rent(h);
      assert.equal(h.cloud.creates, 2);
      await clean(h);
    });
    test('worst ' + mode + ' poll recovers then host reboot fails gate ' + v, async () => {
      const h = await market();
      await rent(h);
      const f = h.cloud.fetch;
      let broken = true;
      h.cloud.fetch = async (url, init = {}) => {
        if (
          broken &&
          String(url).includes('/instances/900') &&
          (!init.method || init.method === 'GET')
        ) {
          if (mode === 'timeout') throw Error('mock timeout');
          return new Response(mode === 'garbage' ? 'not-json' : '{}', {
            status: mode === '5xx' ? 503 : 200,
          });
        }
        return f(url, init);
      };
      h.restart();
      for (let i = 0; i < 2; i++) {
        h.time.value += 20000;
        await h.service.sweep();
      }
      assert.equal((await row(h)).ended, null);
      assert.equal(h.cloud.creates, 1);
      broken = false;
      await h.service.sweep();
      await h.report({ seq: 1, kind: 'log', text: 'first heartbeat' });
      h.cloud.instances.get('900').actual_status = 'offline';
      h.time.value += 16 * 60000;
      await h.service.sweep();
      assert.equal((await row(h)).verdict, 'FAIL');
      await rent(h);
      assert.equal(h.cloud.creates, 1);
      assert.equal(await h.storage.get('provider-machine:100001'), undefined);
      await clean(h);
    });
  }
  test('worst unexpected create id reconciles after restart ' + v, async () => {
    const h = await market();
    const f = h.cloud.fetch;
    let changed = false;
    h.cloud.fetch = async (url, init) => {
      const response = await f(url, init);
      if (!changed && String(url).includes('/asks/')) {
        changed = true;
        return Response.json({ success: true, new_contract: v === 1 ? 0 : '900' });
      }
      return response;
    };
    h.restart();
    assert.equal((await h.call()).status, 503);
    assert.equal(h.cloud.creates, 1);
    h.restart();
    const response = await h.call('/status/123-1', { method: 'GET' });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).id, '900');
    assert.equal(h.cloud.creates, 1);
    await clean(h);
  });
}

test('worst default eight attempts all fatal-fail without leaks', async () => {
  const h = await market();
  await rent(h);
  for (let i = 0; i < 8; i++) {
    await fatal(h);
    await rent(h);
  }
  const r = await row(h);
  assert.equal(h.cloud.created.length, 8);
  assert.equal(r.rentalAttempts.length, 8);
  assert.equal(r.replacementExhausted, true);
  await clean(h);
});
for (const v of [1, 2])
  test('worst quote repriced before initial ownership inspection ' + v, async () => {
    const h = await market();
    const original = h.cloud.fetch;
    h.cloud.fetch = async (url, init = {}) => {
      const response = await original(url, init);
      if (
        String(url).includes('/instances/900') &&
        (!init.method || init.method === 'GET') &&
        response.status === 200
      ) {
        const body = await response.json();
        if (body.instances) {
          body.instances.dph_total = 0.7 + v * 0.1;
          body.instances.dph_base = 0.68 + v * 0.1;
        }
        return Response.json(body);
      }
      return response;
    };
    h.restart();
    const response = await h.call();
    assert.equal(response.status, 503);
    const r = await row(h);
    assert.equal(r.closed, 'CONTRACT_PRICE');
    assert.notEqual(r.ended, null);
    assert.match(r.log, /RENTAL instance=900/);
    await clean(h);
  });

for (const v of [1, 2]) {
  test('worst pre-report instance vanishes and replacement stays bounded ' + v, async () => {
    const h = await market();
    await rent(h);
    h.cloud.instances.delete('900');
    h.time.value += 20000;
    await h.service.sweep();
    const lost = await row(h);
    assert.equal(lost.verdict, 'PROVIDER_FAULT');
    assert.notEqual(lost.ended, null);
    assert.equal(lost.rentalAttempts.length, 1);
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    const next = await rent(h);
    assert.equal(next.providerRetries, 1);
    assert.equal(h.cloud.created.length, 2);
    await clean(h);
  });
  test('worst pre-report API outage waits for cleanup then replaces ' + v, async () => {
    const h = await market();
    await rent(h);
    h.cloud.vastError = v === 1 ? 500 : 503;
    for (let i = 0; i < 14; i++) {
      h.time.value += 60000;
      await h.service.sweep();
    }
    const lost = await row(h);
    assert.equal(lost.verdict, 'PROVIDER_FAULT');
    assert.equal(lost.ended, null);
    assert.equal(h.cloud.created.length, 1);
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    h.cloud.vastError = 0;
    const next = await rent(h);
    assert.equal(next.providerRetries, 1);
    assert.equal(h.cloud.instances.has('900'), false);
    await clean(h);
  });
  test('worst running without first report can replace without hardware blame ' + v, async () => {
    const h = await market({ BOOT_LIMIT_MINUTES: String(4 + v) });
    await rent(h);
    h.time.value += (5 + v) * 60000;
    await h.service.sweep();
    const lost = await row(h);
    assert.equal(lost.verdict, 'PROVIDER_FAULT');
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    const next = await rent(h);
    assert.equal(next.providerRetries, 1);
    await clean(h);
  });
}

for (const v of [1, 2])
  test('worst foreign create id copies public label but cannot own the rental ' + v, async () => {
    const h = await market();
    const original = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      const response = await original(url, init);
      if (String(url).includes('/asks/')) {
        const body = JSON.parse(init.body);
        Object.assign(h.cloud.instances.get('777'), {
          label: body.label,
          onstart: 'foreign-start-' + v,
        });
        return Response.json({ success: true, new_contract: 777 });
      }
      return response;
    };
    h.restart();
    await rent(h);
    assert.equal((await row(h)).verdict, 'PROVIDER_FAULT');
    assert.equal(h.cloud.instances.has('900'), false);
    assert.equal(h.cloud.instances.has('777'), true);
    assert.deepEqual(h.cloud.deletes, ['900']);
    await clean(h);
  });

for (const state of ['exited', 'stopped'])
  test('worst provider stops before first report and a fresh box replaces ' + state, async () => {
    const h = await market();
    await rent(h);
    h.cloud.instances.get('900').actual_status = state;
    for (let i = 0; i < 3; i++) {
      h.time.value += 60000;
      await h.service.sweep();
    }
    const lost = await row(h);
    assert.equal(lost.verdict, 'PROVIDER_FAULT');
    assert.notEqual(lost.ended, null);
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    const next = await rent(h);
    assert.equal(next.providerRetries, 1);
    assert.equal(h.cloud.instances.has('900'), false);
    await clean(h);
  });
