import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setup, env, clock, MockCloud, MemoryStorage } from './helpers.js';
import { configuration, providerPolicy } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { observeFatal, bootLogDelta } from '../src/provider-policy.js';
import { Ledger, rentalCost } from '../src/ledger.js';
import { OfferBook } from '../src/offers.js';
import { Vast } from '../src/vast.js';
import { redactEndpoints } from '../src/rental.js';
import { redactPublic } from '../src/publiclog.js';
import { publicLogRenderer } from '../public-log.js';
import { publishTick } from '../src/publish.js';
// FALSIFIERS: recovery survives retained diagnostics; only current, timed failures close;
// private endpoints stay private; publication and ranking cover every supported identity/time.
const row = (h) => h.storage.get('record:123-1');
const domainError = (id) =>
  `libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C.${id}'\n`;
async function boot(vars = {}) {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180', BOOT_LIMIT_MINUTES: '75', ...vars });
  h.cloud.offer.host_id = 100;
  h.cloud.contracts = [900, 901, 902, 903, 904];
  h.cloud.extraOffers = Array.from({ length: 4 }, (_, i) => ({
    ...h.cloud.offer,
    id: 20 + i,
    machine_id: 200000 + i,
    host_id: 200 + i,
    price: 0.21 + i * 0.001,
  }));
  assert.equal((await h.call()).status, 200);
  return h;
}
async function tick(h, milliseconds) {
  h.time.value += milliseconds;
  await h.service.sweep();
}
for (const restart of [false, true]) {
  test('RV2 healthy status text stays historical across provider flap ' + restart, async () => {
    const h = await boot();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'running',
      status_msg: 'GPU error, unable to start instance',
    });
    await tick(h, 1000);
    if (restart) h.restart();
    h.cloud.instances.get('900').actual_status = 'created';
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal((await row(h)).providerMessageClass, 'IGNORABLE');
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
  test(
    'RV2 changed fatal status after healthy baseline remains actionable ' + restart,
    async () => {
      const h = await boot();
      await tick(h, 1000);
      if (restart) h.restart();
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: 'created',
        status_msg: 'GPU error, unable to start instance',
      });
      await tick(h, 1000);
      assert.equal((await row(h)).closed, null);
      await tick(h, 90000);
      assert.equal((await row(h)).closed, 'PROVIDER_FAULT');
      assert.deepEqual(h.cloud.deletes, ['900']);
    },
  );
  test('RV2 unreadable recovery baseline fences historical fatal log ' + restart, async () => {
    const h = await boot();
    h.cloud.logText = 'preparation\n';
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Preparing GPUs...',
    });
    await tick(h, 1000);
    h.cloud.logText += domainError('900');
    h.cloud.logStatus = 503;
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'running',
      status_msg: 'Preparing GPUs complete; healthy',
    });
    await tick(h, 1000);
    if (restart) h.restart();
    h.cloud.logStatus = 200;
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Preparing GPUs...',
    });
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
  test(
    'RV2 fresh fatal after healthy baseline still confirms within bound ' + restart,
    async () => {
      const h = await boot();
      h.cloud.logText = domainError('900') + 'Domain successfully started; GPUs ready\n';
      await tick(h, 1000);
      if (restart) h.restart();
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: 'created',
        status_msg: 'Retrying',
      });
      h.cloud.logText += domainError('900');
      await tick(h, 1000);
      assert.equal((await row(h)).closed, null);
      await tick(h, 90000);
      assert.equal((await row(h)).closed, 'PROVIDER_FAULT');
      assert.deepEqual(h.cloud.deletes, ['900']);
    },
  );
  test(
    'RV2 first observed running state fences historical fatal log on later flap ' + restart,
    async () => {
      const h = await boot();
      h.cloud.logText = domainError('900') + 'Domain successfully started; GPUs ready\n';
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: 'running',
        status_msg: 'Preparing GPUs complete; healthy',
      });
      await tick(h, 1000);
      if (restart) h.restart();
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: 'created',
        status_msg: 'Preparing GPUs...',
      });
      await tick(h, 1000);
      await tick(h, 90000);
      assert.equal((await row(h)).closed, null);
      assert.equal(h.cloud.instances.has('900'), true);
      assert.equal(await h.storage.get('provider-machine:100001'), undefined);
      assert.equal(await h.storage.get('provider-host:100'), undefined);
      await h.call('/stop/123-1');
    },
  );
  test('RV2 recovered running box ignores retained fatal boot history ' + restart, async () => {
    const h = await boot();
    h.cloud.logText = domainError('900');
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Retrying',
    });
    await tick(h, 1000);
    assert.equal((await row(h)).closed, null);
    if (restart) h.restart();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'running',
      status_msg: 'Preparing GPUs complete; healthy',
    });
    h.cloud.logText += 'Domain successfully started; GPUs ready\n';
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal((await row(h)).bootFailure, null);
    assert.equal(h.cloud.instances.has('900'), true);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    await h.call('/stop/123-1');
  });
  test('RV2 unchanged current fatal waits for window across close polls ' + restart, async () => {
    const h = await boot();
    h.cloud.logText = domainError('900');
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Retrying',
    });
    await tick(h, 1000);
    const first = h.time.value;
    if (restart) h.restart();
    for (const delta of [1, 1, 998, 88000]) {
      await tick(h, delta);
      assert.equal((await row(h)).closed, null, 'no early confirmation');
    }
    h.time.value = first + 90000;
    await h.service.sweep();
    const r = await row(h);
    assert.equal(r.closed, 'PROVIDER_FAULT');
    assert.equal(r.bootFailure.confirmed, true);
    assert.equal(r.ended, first + 90000);
    assert.deepEqual(h.cloud.deletes, ['900']);
  });
  test('RV2 first accepted log permanently fences provider replacement ' + restart, async () => {
    const h = await boot({ PROVIDER_CONFIRM_SECONDS: '1' });
    assert.equal(
      (await h.report({ seq: 1, kind: 'log', text: 'first accepted report' })).status,
      200,
    );
    if (restart) h.restart();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance',
    });
    await tick(h, 20000);
    await tick(h, 20000);
    await h.call();
    assert.equal(h.cloud.created.length, 1);
    assert.equal((await row(h)).bootFailure, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
  test('RV2 settled failures release unused TTL reservations ' + restart, async () => {
    const h = await boot({ PROVIDER_CONFIRM_SECONDS: '1' });
    let spent = 0;
    for (let n = 0; n < 4; n++) {
      const current = await row(h);
      Object.assign(h.cloud.instances.get(current.id), {
        actual_status: 'created',
        status_msg: 'GPU error, unable to start instance',
      });
      await tick(h, 20000);
      await tick(h, 20000);
      const r = await row(h);
      assert.equal(r.ended - r.started, 40000);
      spent += (r.computeStoragePrice * 40) / 3600 + r.bandwidthCost;
      assert.ok(Math.abs(rentalCost(r, r.ended) + (r.retryCostUsd ?? 0) - spent) < 1e-10);
      const b = await new Ledger(h.storage).budgetSummary(
        configuration({ ...env, HARD_RUNTIME_MINUTES: '180' }),
        h.time.value,
      );
      assert.equal(b.outstanding_usd, 0);
      assert.ok(Math.abs(b.rolling_usd - spent) < 1e-10);
      assert.ok(b.rolling_usd < 0.02);
      if (n !== 3) {
        if (restart) h.restart();
        assert.equal((await h.call()).status, 200);
      }
    }
  });
}
for (const seconds of [1, 90, 600]) {
  test('RV2 fatal helper enforces elapsed window ' + seconds, () => {
    const p = { ...providerPolicy(), confirmationSeconds: seconds };
    const first = observeFatal(null, 'GPU_START_ERROR', 1000, p);
    assert.equal(observeFatal(first, 'GPU_START_ERROR', 1001, p).confirmed, false);
    assert.equal(
      observeFatal(first, 'GPU_START_ERROR', 1000 + seconds * 1000 - 1, p).confirmed,
      false,
    );
    assert.equal(observeFatal(first, 'GPU_START_ERROR', 1000 + seconds * 1000, p).confirmed, true);
  });
}
for (const command of ['ssh', 'scp']) {
  test('RV2 compact Port removed by JS boundaries ' + command, () => {
    for (const option of [
      '-oPort=23456',
      '-oport=23456',
      '-oPORT="23456"',
      "-o'Port=23456'",
      '-o "Port 23456"',
    ]) {
      const raw =
        command +
        ' ' +
        option +
        ' user@203.0.113.91 count=31999 version=580.159.04 MAC=aa:bb:cc:dd:ee:ff';
      for (const clean of [redactEndpoints(raw), redactPublic(raw, [])]) {
        assert.equal(clean.includes('23456'), false, option);
        assert.equal(clean.includes('203.0.113.91'), false);
        assert.match(clean, /count=31999/);
        assert.match(clean, /version=580.159.04/);
        assert.match(clean, /aa:bb:cc:dd:ee:ff/);
      }
    }
  });
  test('RV2 compact Port removed by Python pipelines ' + command, () => {
    const raw =
      command +
      ' -oPort=23456 user@203.0.113.91 count=31999 version=580.159.04 MAC=aa:bb:cc:dd:ee:ff';
    const script = `import ast,ipaddress,json,re,runpy,sys
x=json.load(sys.stdin)
r=runpy.run_path(sys.argv[1],run_name='review_fixture')
nodes=[n for n in ast.parse(x['renderer']).body if isinstance(n,ast.FunctionDef)]
g={'re':re,'ipaddress':ipaddress};exec(compile(ast.Module(body=nodes,type_ignores=[]),'literal_renderer','exec'),g)
print(json.dumps({'runner':r['sanitise'](x['raw'])[0],'renderer':g['clean'](x['raw'])}))`;
    const out = spawnSync(
      'python3',
      ['-I', '-c', script, fileURLToPath(new URL('../runner.py', import.meta.url))],
      { input: JSON.stringify({ raw, renderer: publicLogRenderer }), encoding: 'utf8' },
    );
    assert.equal(out.status, 0, out.stderr);
    for (const [pipeline, clean] of Object.entries(JSON.parse(out.stdout))) {
      assert.equal(clean.includes('23456'), false, pipeline);
      assert.equal(clean.includes('203.0.113.91'), false, pipeline);
      assert.match(clean, /count=31999/);
      assert.match(clean, /version=580.159.04/);
      assert.match(clean, /aa:bb:cc:dd:ee:ff/);
    }
  });
  test('RV2 compact Port removed from Worker diagnostics ' + command, async () => {
    const h = await boot({ PROVIDER_CONFIRM_SECONDS: '1' });
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg:
        'GPU error, unable to start instance; ' + command + ' -oPort=23456 user@203.0.113.91',
    });
    await tick(h, 1000);
    await tick(h, 1000);
    const r = await row(h);
    assert.equal(r.closed, 'PROVIDER_FAULT');
    assert.equal(r.log.includes('23456'), false);
    assert.equal(r.log.includes('203.0.113.91'), false);
  });
}
for (const advance of [1, 20]) {
  test('RV2 advancing clock preserves equal penalty ranking ' + advance, async () => {
    const selected = [];
    for (const reverse of [false, true]) {
      const cfg = applyProfile(configuration(env), 'check', 'middle');
      const ledger = new Ledger(new MemoryStorage()),
        cloud = new MockCloud();
      cloud.offer.host_id = 100;
      cloud.extraOffers = [
        { ...cloud.offer, id: 6, machine_id: 100002, host_id: 101, price: cloud.price },
      ];
      const vast = new Vast(cloud.fetch, cfg, 'mock-key'),
        market = await vast.offers();
      for (const o of market)
        await ledger.markProviderFault(
          { key: 'old' + o.id, nonce: 'old', machineId: o.machine_id, hostId: o.host_id },
          'GPU_START_ERROR',
          clock,
          cfg.provider,
        );
      let at = clock + 1000;
      const book = new OfferBook(() => {
        const old = at;
        at += advance;
        return old;
      });
      const e = await book.evaluate(vast, cfg, ledger, 'check', {
        market: reverse ? [...market].reverse() : market,
      });
      assert.equal(e.allowed[0].penalty, e.allowed[1].penalty);
      selected.push(
        (await book.select(e, cfg, ledger, 'check', undefined, { tierName: 'middle' })).offer.id,
      );
    }
    assert.deepEqual(selected, [5, 5]);
  });
}
for (const leg of [32, 33, 39, -1, '01', '10x'])
  test('RV2 publication rejects malformed attempt ' + leg, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      puts = [];
    const r = {
      key: '123-1',
      run: '123',
      attempt: 1,
      sha: 'a'.repeat(40),
      nonce: 'b'.repeat(32),
      runnerNonce: 'f'.repeat(64),
      reportExpiry: clock / 1000 + 9999,
      id: '900',
      started: clock,
      ended: null,
      price: 0.2,
      ttl: 10800000,
      providerRetries: leg,
      profile: 'check',
      verdict: 'PASS',
      closed: 'RESULT',
    };
    await ledger.appendPublicLog(r, 'nonsecret review log\n');
    await ledger.settle(r, clock + 1000);
    await publishTick({
      ledger,
      env: { ...env, LOGS_BUCKET: { put: async (...args) => puts.push(args) } },
      now: clock + 2000,
      clock: () => clock + 2000,
    });
    assert.equal(puts.length, 0);
    assert.equal((await storage.list({ prefix: 'plogq:' })).size, 0);
  });

for (const restart of [false, true]) {
  test('RV2 pending progress cannot replay historical fatal lines ' + restart, async () => {
    const h = await boot();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Retrying',
    });
    h.cloud.logText = domainError('900');
    await tick(h, 1000);
    if (restart) h.restart();
    h.cloud.logText += 'Domain successfully started; setup is advancing\n';
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal((await row(h)).bootFailure, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
  test('RV2 running state cancels a retained fatal status message ' + restart, async () => {
    const h = await boot();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'running',
      status_msg: 'GPU error, unable to start instance',
    });
    await tick(h, 1000);
    if (restart) h.restart();
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal((await row(h)).bootFailure, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
}
for (const seconds of [90, 600])
  test('RV2 close polls are not independent fatal observations ' + seconds, () => {
    const p = { ...providerPolicy(), confirmationSeconds: seconds };
    const first = observeFatal(null, 'GPU_START_ERROR', 1000, p);
    const second = observeFatal(first, 'GPU_START_ERROR', 1001, p);
    assert.equal(second.count, 1);
    assert.equal(second.confirmed, false);
  });

for (const n of [1, 2]) {
  test('RV2 log cursor returns only appended or rotated suffix ' + n, async () => {
    const old = Array.from({ length: 40 }, (_, i) => 'line-' + n + '-' + i).join('\n') + '\n';
    const first = await bootLogDelta(null, old);
    const unchanged = await bootLogDelta(first.cursor, old);
    assert.equal(unchanged.fresh, '');
    const appended = await bootLogDelta(first.cursor, old + 'fresh-only\n');
    assert.equal(appended.fresh, 'fresh-only\n');
    const rotated = await bootLogDelta(
      first.cursor,
      old.split('\n').slice(1).join('\n') + 'new-fatal\n',
    );
    assert.equal(rotated.fresh, 'new-fatal');
    const rewritten = await bootLogDelta(first.cursor, 'unrelated retained history\n');
    assert.equal(rewritten.fresh, '');
    assert.equal(rewritten.stable, false);
  });
  test('RV2 log cursor storage stays bounded without raw text ' + n, async () => {
    const text = Array.from({ length: 200 + n }, (_, i) => 'secret-marker-' + i).join('\n');
    const { cursor } = await bootLogDelta(null, text);
    assert.ok(cursor.tail.length <= 64);
    assert.ok(JSON.stringify(cursor).length < 5000);
    assert.equal(JSON.stringify(cursor).includes('secret-marker'), false);
  });
  test('RV2 publication rejects mismatched later attempt identity ' + n, async () => {
    const storage = new MemoryStorage(),
      ledger = new Ledger(storage),
      puts = [];
    const leg = n === 1 ? 10 : 31;
    const r = {
      key: '123-1',
      run: '123',
      attempt: 1,
      sha: 'a'.repeat(40),
      nonce: 'b'.repeat(32),
      runnerNonce: 'f'.repeat(64),
      reportExpiry: clock / 1000 + 9999,
      id: '900',
      started: clock,
      ended: null,
      price: 0.2,
      ttl: 10800000,
      providerRetries: leg,
      profile: 'check',
      verdict: 'PASS',
      closed: 'RESULT',
    };
    await ledger.appendPublicLog(r, 'nonsecret review log\n');
    await ledger.settle(r, clock + 1000);
    const key = 'plogq:123-1:' + leg,
      entry = await ledger.get(key);
    await ledger.put(key, { ...entry, leg: 0 });
    await publishTick({
      ledger,
      env: { ...env, LOGS_BUCKET: { put: async (...args) => puts.push(args) } },
      now: clock + 2000,
      clock: () => clock + 2000,
    });
    assert.equal(puts.length, 0);
    assert.equal(await ledger.get(key), undefined);
  });
}

for (const spacing of [2, 5])
  test('RV2 fatal observation spacing comes from validated data ' + spacing, () => {
    const p = providerPolicy({
      PROVIDER_CONFIRM_SECONDS: '10',
      PROVIDER_OBSERVATION_SPACING_SECONDS: String(spacing),
    });
    assert.equal(p.observationSpacingSeconds, spacing);
    const first = observeFatal(null, 'GPU_START_ERROR', 1000, p);
    assert.equal(observeFatal(first, 'GPU_START_ERROR', 2000, p).count, 1);
    assert.equal(observeFatal(first, 'GPU_START_ERROR', 1000 + spacing * 1000, p).count, 2);
    assert.throws(() =>
      providerPolicy({
        PROVIDER_CONFIRM_SECONDS: '1',
        PROVIDER_OBSERVATION_SPACING_SECONDS: String(spacing),
      }),
    );
  });
for (const seconds of [1, 600])
  test('RV2 configured confirmation bound survives a restart ' + seconds, async () => {
    const h = await boot({ PROVIDER_CONFIRM_SECONDS: String(seconds) });
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'GPU error, unable to start instance',
    });
    await tick(h, 1000);
    const first = h.time.value;
    await tick(h, 1);
    assert.equal((await row(h)).closed, null);
    h.restart();
    h.time.value = first + seconds * 1000 - 1;
    await h.service.sweep();
    assert.equal((await row(h)).closed, null);
    h.time.value = first + seconds * 1000;
    await h.service.sweep();
    assert.equal((await row(h)).closed, 'PROVIDER_FAULT');
    assert.equal((await row(h)).ended, first + seconds * 1000);
  });
for (const v of [1, 2])
  test('RV2 running boot timeout ignores old confirmed history for penalties ' + v, async () => {
    const h = await boot();
    const r = await row(h);
    r.bootFailure = { confirmed: true, reason: 'KVM_DOMAIN_NOT_FOUND', first: clock, last: clock };
    await h.storage.put('record:123-1', r);
    h.time.value = r.bootDeadline + v;
    await h.service.sweep();
    assert.equal((await row(h)).faultReason, 'PROVISIONING_TIMEOUT');
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    assert.equal(await h.storage.get('provider-host:100'), undefined);
    assert.equal(h.cloud.instances.has('900'), false);
  });
for (const advance of [1, 20]) {
  test('RV2 advancing clock preserves genuinely different penalties ' + advance, async () => {
    const cfg = applyProfile(configuration(env), 'check', 'middle');
    const ledger = new Ledger(new MemoryStorage()),
      cloud = new MockCloud();
    cloud.offer.host_id = 100;
    cloud.extraOffers = [
      { ...cloud.offer, id: 6, machine_id: 100002, host_id: 101, price: cloud.price },
    ];
    const vast = new Vast(cloud.fetch, cfg, 'mock-key'),
      market = await vast.offers();
    for (const o of market)
      await ledger.markProviderFault(
        { key: 'old' + o.id, nonce: 'old', machineId: o.machine_id, hostId: o.host_id },
        'GPU_START_ERROR',
        clock,
        cfg.provider,
      );
    await ledger.markProviderFault(
      { key: 'again', nonce: 'again', machineId: market[0].machine_id, hostId: market[0].host_id },
      'GPU_START_ERROR',
      clock,
      cfg.provider,
    );
    let at = clock + 1000;
    const book = new OfferBook(() => {
      const t = at;
      at += advance;
      return t;
    });
    const e = await book.evaluate(vast, cfg, ledger, 'check', { market });
    assert.equal(
      (await book.select(e, cfg, ledger, 'check', undefined, { tierName: 'middle' })).offer.id,
      6,
    );
  });
  test('RV2 advancing clock retains pinned die precedence ' + advance, async () => {
    const cfg = applyProfile(configuration(env), 'check', 'middle');
    const ledger = new Ledger(new MemoryStorage()),
      cloud = new MockCloud();
    Object.assign(cloud.offer, { gpu_name: 'RTX_4090', compute_cap: 890, host_id: 100 });
    cloud.extraOffers = [
      {
        ...cloud.offer,
        id: 6,
        machine_id: 100002,
        host_id: 101,
        gpu_name: 'RTX_4080',
        price: cloud.price,
      },
    ];
    await ledger.markProviderFault(
      { key: 'old5', nonce: 'old', machineId: cloud.offer.machine_id, hostId: 100 },
      'GPU_START_ERROR',
      clock,
      cfg.provider,
    );
    const vast = new Vast(cloud.fetch, cfg, 'mock-key'),
      market = await vast.offers();
    let at = clock + 1000;
    const book = new OfferBook(() => {
      const t = at;
      at += advance;
      return t;
    });
    const e = await book.evaluate(vast, cfg, ledger, 'check', { market: [...market].reverse() });
    assert.equal(
      (await book.select(e, cfg, ledger, 'check', undefined, { tierName: 'middle' })).offer.id,
      5,
    );
  });
}

for (const progress of ['Preparing GPUs', 'Pulling fs layer'])
  test('RV2 changing provider progress clears prior witness ' + progress, async () => {
    const h = await boot();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Retrying',
    });
    h.cloud.logText = domainError('900');
    await tick(h, 1000);
    h.cloud.instances.get('900').status_msg = progress;
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal((await row(h)).bootFailure, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });

for (const command of ['ssh', 'scp'])
  test('RV2 malformed option text retains unrelated numbers ' + command, () => {
    const raw =
      command +
      ' -o Port' +
      ' '.repeat(12000) +
      'not-a-number count=31999 version=580.159.04 MAC=aa:bb:cc:dd:ee:ff';
    assert.equal(redactEndpoints(raw), raw);
  });

for (const state of ['unrecognised', { invalid: true }])
  test(
    'RV2 malformed provider state cannot confirm hardware blame ' + JSON.stringify(state),
    async () => {
      const h = await boot({ PROVIDER_CONFIRM_SECONDS: '1' });
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: state,
        status_msg: 'GPU error, unable to start instance',
      });
      await tick(h, 1000);
      await tick(h, 1000);
      assert.equal((await row(h)).closed, null);
      assert.equal((await row(h)).bootFailure, null);
      assert.equal(await h.storage.get('provider-machine:100001'), undefined);
      await h.call('/stop/123-1');
    },
  );

for (const restart of [false, true]) {
  test('RV2 terminal historical log after running remains inert ' + restart, async () => {
    const h = await boot();
    h.cloud.logText = domainError('900');
    await tick(h, 1000);
    if (restart) h.restart();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Retrying',
    });
    await tick(h, 1000);
    await tick(h, 90000);
    assert.equal((await row(h)).closed, null);
    assert.equal(await h.storage.get('provider-machine:100001'), undefined);
    await h.call('/stop/123-1');
  });
  for (const seeded of [false, true])
    for (const [name, events, split, fails] of [
      ['fatal-progress', ['fatal', 'progress'], false, false],
      ['progress-fatal', ['progress', 'fatal'], false, true],
      ['interleaved-fatal', ['fatal', 'progress', 'fatal'], false, true],
      ['interleaved-progress', ['progress', 'fatal', 'progress'], false, false],
      ['split-progress', ['fatal', 'progress'], true, false],
    ])
      test(`RV2 ordered boot batch ${name} seeded=${seeded} restart=${restart}`, async () => {
        const h = await boot();
        Object.assign(h.cloud.instances.get('900'), {
          actual_status: 'created',
          status_msg: 'Preparing GPUs',
        });
        h.cloud.logText = seeded ? 'VM preparation begins\n' : '';
        if (seeded) await tick(h, 500);
        for (const event of events) {
          h.cloud.logText +=
            event === 'fatal' ? domainError('900') : 'Domain successfully started; GPUs ready\n';
          if (split) await tick(h, 1000);
        }
        if (!split) await tick(h, 1000);
        if (restart) h.restart();
        await tick(h, 90000);
        const r = await row(h);
        assert.equal(r.closed, fails ? 'PROVIDER_FAULT' : null);
        if (!fails) {
          assert.equal(await h.storage.get('provider-machine:100001'), undefined);
          await h.call('/stop/123-1');
        } else assert.deepEqual(h.cloud.deletes, ['900']);
      });
}
function portSpellings(port) {
  const rows = [];
  for (const zeros of [0, 1, 3, 8, 32, 128])
    for (const sign of ['', '+']) {
      const number = sign + '0'.repeat(zeros) + port;
      for (const raw of [
        `ssh -oPort=${number} user@203.0.113.91`,
        `ssh -o "Port\t${number}" user@203.0.113.91`,
        `ssh -p${number} user@203.0.113.91`,
        `scp -P '${number}' file user@203.0.113.91:path`,
        `ssh --port\t"${number}" user@203.0.113.91`,
        `ssh -p "\v${number}" user@203.0.113.91`,
        `ssh -o "Port\f${number}" user@203.0.113.91`,
        `ssh --ssh-port=${number} user@203.0.113.91`,
        `ssh ssh://user@host.example:${number}/path`,
        `ssh barehost:${number}`,
        `ssh user@203.0.113.91:${number}`,
        `ssh user@[2001:db8:1234:5678:90ab:cdef:1234:5678]:${number}`,
        `ssh_port = '${number}'`,
      ])
        rows.push(raw + ' count=31999 count:31999 version=580.159.04 MAC=aa:12:34:56:78:90');
    }
  return rows;
}
function assertPortPrivate(clean, port) {
  assert.equal(clean.includes(String(port)), false, clean);
  assert.equal(clean.includes('203.0.113.91'), false, clean);
  assert.equal(clean.includes('2001:db8:1234:5678:90ab:cdef:1234:5678'), false, clean);
  assert.match(clean, /count=31999/);
  assert.match(clean, /count:31999/);
  assert.match(clean, /version=580.159.04/);
  assert.match(clean, /aa:12:34:56:78:90/);
}
for (const port of [23456, 22331]) {
  test('RV2 structural port spellings JS property ' + port, () => {
    for (const raw of portSpellings(port))
      for (const clean of [redactEndpoints(raw), redactPublic(raw, [])])
        assertPortPrivate(clean, port);
  });
  test('RV2 structural port spellings Python property ' + port, () => {
    const script = `import ast,ipaddress,json,re,runpy,sys
x=json.load(sys.stdin);r=runpy.run_path(sys.argv[1],run_name='residual_fixture')
g={'re':re,'ipaddress':ipaddress};exec(compile(ast.Module(body=[n for n in ast.parse(x['renderer']).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),'literal_renderer','exec'),g)
print(json.dumps([[r['sanitise'](raw)[0],g['clean'](raw)] for raw in x['rows']]))`;
    const result = spawnSync(
      'python3',
      ['-I', '-c', script, fileURLToPath(new URL('../runner.py', import.meta.url))],
      {
        input: JSON.stringify({ rows: portSpellings(port), renderer: publicLogRenderer }),
        encoding: 'utf8',
      },
    );
    assert.equal(result.status, 0, result.stderr);
    for (const pair of JSON.parse(result.stdout))
      for (const clean of pair) assertPortPrivate(clean, port);
  });
  test('RV2 accepted numeric SSH spellings ' + port, () => {
    for (const whitespace of [' ', '\t', '\n', '\r', '\v', '\f']) {
      const result = spawnSync(
        'ssh',
        ['-G', '-F', '/dev/null', '-p', whitespace + '+000' + port, 'user@203.0.113.91'],
        { encoding: 'utf8' },
      );
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, new RegExp('^port ' + port + '$', 'm'));
    }
    for (const option of [`-oPort=000${port}`, `-oPort=+${port}`, `-p000${port}`, `-p+${port}`]) {
      const result = spawnSync('ssh', ['-G', '-F', '/dev/null', option, 'user@203.0.113.91'], {
        encoding: 'utf8',
      });
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, new RegExp('^port ' + port + '$', 'm'));
    }
  });
  test('RV2 structural port Worker diagnostics ' + port, async () => {
    for (const option of [`-oPort=000${port}`, `-oPort=+${port}`, `-p000${port}`, `-p+${port}`]) {
      const h = await boot({ PROVIDER_CONFIRM_SECONDS: '1' });
      Object.assign(h.cloud.instances.get('900'), {
        actual_status: 'created',
        status_msg: 'GPU error, unable to start instance; ssh ' + option + ' user@203.0.113.91',
      });
      await tick(h, 1000);
      await tick(h, 1000);
      const r = await row(h);
      assert.equal(r.closed, 'PROVIDER_FAULT');
      assert.equal(r.log.includes(String(port)), false);
      assert.equal(r.log.includes('203.0.113.91'), false);
    }
  });
}

for (const restart of [false, true])
  test('RV2 interleaved progress restarts current fatal confirmation ' + restart, async () => {
    const h = await boot();
    Object.assign(h.cloud.instances.get('900'), {
      actual_status: 'created',
      status_msg: 'Preparing GPUs',
    });
    h.cloud.logText = domainError('900');
    await tick(h, 1000);
    h.cloud.logText += 'Domain successfully started; GPUs ready\n' + domainError('900');
    await tick(h, 80000);
    if (restart) h.restart();
    await tick(h, 10000);
    assert.equal((await row(h)).closed, null);
    await tick(h, 80000);
    assert.equal((await row(h)).closed, 'PROVIDER_FAULT');
  });
