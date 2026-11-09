import test from 'node:test';
import assert from 'node:assert/strict';
import { providerPolicy, configuration, profileCaps } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import {
  fatalSignature,
  providerMessageClass,
  observeFatal,
  faultPenalty,
  retryEligible,
} from '../src/provider-policy.js';
import { ipPrefix, redactEndpoints, rentalLine } from '../src/rental.js';
import { redactPublic } from '../src/publiclog.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { MemoryStorage, MockCloud, env, clock, setup } from './helpers.js';
import { spawnSync } from 'node:child_process';
import { Vast } from '../src/vast.js';
import { sha256Hex } from '../src/tokens.js';
import { IMAGE } from '../src/policy.js';
const policy = providerPolicy();
for (const variant of [1, 2]) {
  test('rules fatal confirmation ' + variant, () => {
    const reason = fatalSignature(
      { status_msg: 'GPU error, unable to start instance' },
      '',
      String(variant),
      policy,
    );
    assert.equal(reason, 'GPU_START_ERROR');
    const one = observeFatal(null, reason, 1000, policy);
    assert.equal(one.confirmed, false);
    assert.equal(observeFatal(one, reason, 1000, policy).count, 1);
    assert.equal(observeFatal(one, reason, 21000, policy).confirmed, true);
    assert.equal(observeFatal(one, null, 21000, policy), null);
    assert.equal(observeFatal(one, reason, 999, policy).confirmed, false);
  });
  test('rules KVM exact owned id ' + variant, () => {
    const text =
      "libvirt: QEMU Driver error : Domain not found: no domain with matching name 'C." +
      variant +
      "'";
    assert.equal(fatalSignature({}, text, String(variant), policy), 'KVM_DOMAIN_NOT_FOUND');
    assert.equal(fatalSignature({}, text, String(variant + 1), policy), null);
    for (const line of policy.ignorableSignatures)
      assert.equal(fatalSignature({ status_msg: line }, '', String(variant), policy), null);
    assert.equal(
      fatalSignature({ status_msg: 'some error, retrying daemon' }, '', String(variant), policy),
      null,
    );
  });
  test('rules configurable provider controls ' + variant, () => {
    assert.equal(
      providerMessageClass({ status_msg: 'Preparing GPUs...' }, '', String(variant), policy).kind,
      'IGNORABLE',
    );
    assert.equal(
      providerMessageClass({ status_msg: 'some unrelated error' }, '', String(variant), policy)
        .kind,
      'UNKNOWN',
    );
    const p = providerPolicy({
      PROVIDER_CONFIRM_SECONDS: String(30 + variant),
      PROVIDER_MAX_ATTEMPTS: String(5 + variant),
      PROVIDER_SESSION_MINUTES: String(20 + variant),
      PROVIDER_PENALTY_BASE_USD_HOUR: '0.4',
      PROVIDER_PENALTY_HALF_LIFE_HOURS: '12',
      PROVIDER_PENALTY_CAP_USD_HOUR: '3',
      PROVIDER_FATAL_SIGNATURES: JSON.stringify([
        { reason: 'HOST_START_ERROR', source: 'status', contains: ['host failed to start'] },
      ]),
    });
    assert.equal(p.confirmationSeconds, 30 + variant);
    assert.equal(p.maxAttempts, 5 + variant);
    assert.equal(p.sessionMinutes, 20 + variant);
    assert.equal(p.penaltyBaseUsdHour, 0.4);
    assert.equal(p.penaltyHalfLifeHours, 12);
    assert.equal(p.penaltyCapUsdHour, 3);
    assert.equal(
      fatalSignature({ status_msg: 'host failed to start' }, '', '1', p),
      'HOST_START_ERROR',
    );
    for (const vars of [
      { PROVIDER_MAX_ATTEMPTS: '0' },
      { PROVIDER_MAX_ATTEMPTS: '33' },
      { PROVIDER_MAX_ATTEMPTS: '1.5' },
      { PROVIDER_SESSION_MINUTES: 'NaN' },
      { PROVIDER_FATAL_SIGNATURES: '[]' },
    ])
      assert.throws(() => configuration({ ...env, ...vars }));
  });
  test('rules bounded attempt and wall clock ' + variant, () => {
    const r = {
      started: clock,
      ended: clock + 1000,
      softDeadline: clock + 2e7,
      sessionDeadline: clock + 90 * 60000,
      verdict: 'PROVIDER_FAULT',
      closed: 'PROVIDER_FAULT',
      providerRetries: 6,
    };
    assert.equal(retryEligible(r, clock + 2000, policy), true);
    assert.equal(retryEligible({ ...r, firstReportAt: clock }, clock + 2000, policy), false);
    assert.equal(retryEligible({ ...r, providerRetries: 7 }, clock + 2000, policy), false);
    assert.equal(retryEligible(r, r.sessionDeadline, policy), false);
    assert.equal(retryEligible(r, clock - 1, policy), false);
    assert.equal(retryEligible({ ...r, replacementExhausted: true }, clock + 2000, policy), false);
  });
  test('rules fault escalation decay and cap ' + variant, async () => {
    const store = new MemoryStorage(),
      ledger = new Ledger(store);
    const r = { key: '1-1', nonce: 'a', machineId: variant, hostId: 100 + variant };
    await ledger.markProviderFault(r, 'GPU_START_ERROR', clock, policy);
    assert.equal(await ledger.providerPenalty(r.machineId, r.hostId, clock, policy), 0.5);
    await ledger.markProviderFault(r, 'GPU_START_ERROR', clock, policy);
    assert.equal((await ledger.get('provider-machine:' + r.machineId)).faults.length, 1);
    assert.equal(
      await ledger.providerPenalty(r.machineId, r.hostId, clock + 24 * 3600000, policy),
      0.25,
    );
    assert.equal(faultPenalty({ faults: [{ at: clock, weight: 2 }] }, clock, policy), 0.5);
    assert.equal(faultPenalty({ faults: [{ at: clock, weight: 1000 }] }, clock, policy), 5);
    await ledger.markProviderFault({ ...r, nonce: 'b' }, 'KVM_DOMAIN_NOT_FOUND', clock, policy);
    assert.equal(await ledger.providerPenalty(r.machineId, r.hostId, clock, policy), 1.5);
    await ledger.markProviderSuccess({ ...r, nonce: 'success' }, policy);
    assert.equal(await ledger.providerPenalty(r.machineId, r.hostId, clock, policy), 0.75);
    await ledger.markProviderSuccess({ ...r, nonce: 'success' }, policy);
    assert.equal(await ledger.providerPenalty(r.machineId, r.hostId, clock, policy), 0.75);
    for (let i = 0; i < 100; i++)
      await ledger.markProviderFault({ ...r, nonce: 'many' + i }, 'GPU_START_ERROR', clock, policy);
    assert.equal((await ledger.get('provider-machine:' + r.machineId)).faults.length, 32);
    assert.equal(await ledger.providerPenalty(r.machineId, r.hostId, clock, policy), 5);
    assert.equal(
      await ledger.providerPenalty(r.machineId, r.hostId, clock + 40 * 24 * 3600000, policy),
      0.0625,
    );
    assert.equal(await ledger.get('bad-machine:' + r.machineId), undefined);
    assert.equal(await ledger.get('bad-host:' + r.hostId), undefined);
    await ledger.markProviderFault(
      { ...r, machineId: 500, firstReportAt: clock },
      'GPU_START_ERROR',
      clock,
      policy,
    );
    assert.equal(await ledger.get('provider-machine:500'), undefined);
  });
  test('rules penalty history storage bound ' + variant, async () => {
    const ledger = new Ledger(new MemoryStorage());
    for (let i = 1; i <= 270; i++)
      await ledger.markProviderFault(
        { key: String(i) + '-1', nonce: 'a', machineId: i, hostId: i },
        'GPU_START_ERROR',
        clock + i,
        policy,
      );
    assert.equal((await ledger.storage.list({ prefix: 'provider-machine:' })).size, 256);
    assert.equal((await ledger.storage.list({ prefix: 'provider-host:' })).size, 256);
    assert.equal(await ledger.get('provider-machine:1'), undefined);
  });
  test('rules no endpoint in public text ' + variant, () => {
    const ip = '203.0.113.' + (70 + variant),
      port = 42000 + variant;
    const text = 'GPU error at ' + ip + ':' + port + ' ssh_port=' + port + ' "ssh_port": ' + port;
    for (const fn of [redactEndpoints, redactPublic]) {
      const safe = fn(text);
      assert.equal(safe.includes(ip), false);
      assert.equal(safe.includes(String(port)), false);
      assert.match(safe, /203\.0\.113\.0/);
      const v6 = '2001:db8:1234:5678:90ab:cdef:1234:' + variant;
      const safe6 = fn('[' + v6 + ']:' + port);
      assert.equal(safe6.includes(v6), false);
      assert.equal(safe6.includes(String(port)), false);
      assert.match(safe6, /2001:db8:1234::\/48/);
    }
    assert.equal(ipPrefix(ip), '203.0.113.0');
    assert.equal(ipPrefix('2001:db8::1'), '2001:db8::/48');
    assert.equal(ipPrefix('203.0.113.1:22'), 'unknown');
    assert.equal(ipPrefix('hostname.example'), 'unknown');
    assert.match(
      rentalLine({
        id: '900',
        machineId: 1,
        hostId: 2,
        datacenter: true,
        country: 'US',
        ipPrefix: ipPrefix(ip),
        gpu: 'GTX_1660',
        dphTotal: 0.82,
      }),
      /^RENTAL instance=900 host=2 machine=1 datacenter=true country=US ip=203\.0\.113\.0 gpu=GTX_1660 price=0.820000$/,
    );
  });
  test('rules middle pin order and penalty ' + variant, async () => {
    const cfg = configuration(env),
      book = new OfferBook(() => clock),
      ledger = new Ledger(new MemoryStorage());
    assert.deepEqual(cfg.pinnedMiddleDies, ['AD102', 'AD103', 'AD104', 'AD106']);
    assert.equal(
      profileCaps(applyProfile(configuration(env), 'check', 'middle'), 'check').dies.includes(
        variant === 1 ? 'AD103' : 'AD106',
      ),
      true,
    );
    assert.equal(
      profileCaps(applyProfile(configuration(env), 'check', 'small'), 'check').dies.includes(
        variant === 1 ? 'AD103' : 'AD106',
      ),
      false,
    );
    const select = async (...args) => {
      let result;
      await assert.doesNotReject(async () => {
        result = await book.select(...args);
      });
      return result;
    };
    const row = (die, id, penalty = 0) => ({
      die,
      id,
      machine_id: id,
      host_id: id,
      country: 'US',
      pricing: { effective: 0.1 },
      penalty,
    });
    let e = {
      allowed: [
        row('AD104', 3),
        row('AD103', 2),
        row('AD102', 1, 1),
        row('AD102', 4),
        row('GB205', 5),
        row('TU106', 6),
      ],
    };
    let chosen = await select(e, cfg, ledger, 'check', [variant === 1 ? 'GB205' : 'TU106'], {
      tierName: 'middle',
    });
    assert.equal(chosen.offer.id, 4);
    assert.equal(chosen.reason, 'PIN');
    e.allowed = e.allowed.filter((o) => o.die !== 'AD102');
    chosen = await select(e, cfg, ledger, 'check', undefined, { tierName: 'middle' });
    assert.equal(chosen.offer.die, 'AD103');
    assert.deepEqual(chosen.pinSkipped, ['AD102']);
    e.allowed = [row('TU106', 20)];
    chosen = await select(e, cfg, ledger, 'check', undefined, { tierName: 'middle' });
    assert.equal(chosen.reason, 'PIN_EXHAUSTED');
    assert.equal(chosen.offer.die, 'TU106');
    chosen = await select(e, { ...cfg, pinnedMiddleDies: [] }, ledger, 'check', undefined, {
      tierName: 'middle',
    });
    assert.equal(chosen.reason, 'ROTATE');
    for (const list of [['ad102'], ['AD102', 'AD102'], ['MISSING']])
      assert.throws(() => configuration({ ...env, PINNED_MIDDLE_DIES: JSON.stringify(list) }));
  });
  test('rules all penalised still rentable and deterministic ' + variant, async () => {
    const book = new OfferBook(() => clock),
      ledger = new Ledger(new MemoryStorage()),
      cfg = configuration(env);
    const a = {
      die: 'AD102',
      id: 2,
      machine_id: 2,
      host_id: 2,
      country: 'US',
      pricing: { effective: 0.1 },
      penalty: 1,
    };
    const b = { ...a, id: 1, machine_id: 1, penalty: 1 };
    for (const allowed of [
      [a, b],
      [b, a],
    ])
      assert.equal(
        (await book.select({ allowed }, cfg, ledger, 'check', undefined, { tierName: 'middle' }))
          .offer.id,
        1,
      );
    assert.equal(
      (await book.select({ allowed: [a] }, cfg, ledger, 'check', undefined, { tierName: 'middle' }))
        .offer.id,
      2,
    );
  });
  test('rules report failure cannot be rerented ' + variant, async () => {
    const h = await setup();
    await h.call();
    const r = await h.storage.get('record:123-1');
    if (variant === 2) await h.report({ seq: 1, kind: 'log', text: 'first box heartbeat' });
    await h.report({
      seq: variant,
      kind: 'result',
      result: {
        nonce: r.nonce,
        verdict: 'PROVIDER_FAULT',
        suites: [{ name: 'provider_kvm_missing', pass: 0, fail: 1, skip: 0 }],
        hardware: { gpu_name: 'unknown', compute_cap: 0, driver_version: 'unknown' },
      },
    });
    assert.equal((await h.storage.get('record:123-1')).verdict, 'FAIL');
    await h.call();
    assert.equal(h.cloud.created.length, 1);
    await h.call('/stop/123-1');
  });
}

async function reservationFixture() {
  const cfg = configuration(env),
    storage = new MemoryStorage(),
    ledger = new Ledger(storage);
  const prior = {
    key: '1-1',
    run: '1',
    attempt: 1,
    sha: 'a'.repeat(40),
    branch: 'x',
    profile: 'check',
    runnerNonce: 'f'.repeat(64),
    nonce: 'a'.repeat(32),
    id: '900',
    machineId: 1,
    hostId: 100,
    started: clock,
    ended: clock + 1000,
    price: 0.1,
    closed: 'PROVIDER_FAULT',
    verdict: 'PROVIDER_FAULT',
    hardDeadline: clock + 1200000,
    softDeadline: clock + 600000,
    sessionDeadline: clock + 5400000,
    runBudgetUsd: 0.1,
  };
  await ledger.save(prior);
  await ledger.grant(prior.key, prior);
  await ledger.put('cost:1-1', {
    profile: 'check',
    started: clock,
    ended: prior.ended,
    price: 0.1,
    created: true,
  });
  const record = {
    ...prior,
    id: null,
    ended: null,
    closed: null,
    verdict: null,
    started: clock + 2000,
    nonce: 'b'.repeat(32),
    providerRetries: 1,
    machineId: 2,
    hostId: 200,
    originalStarted: clock,
    retryCostUsd: 0.1 / 3600,
    ttl: 1200000,
    billingTtl: 1260000,
  };
  return { cfg, ledger, prior, record };
}
for (const v of [1, 2]) {
  test('rules cumulative cap is transactional ' + v, async () => {
    const { cfg, ledger, record } = await reservationFixture();
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: v === 1 ? 1 : 0.2,
        credit: 100,
        retryNonce: 'a'.repeat(32),
      }),
    );
    assert.equal(await ledger.get('cost:1-1:provider:0'), undefined);
  });
  test('rules reservation archives every attempt ' + v, async () => {
    const { cfg, ledger, record } = await reservationFixture();
    record.runBudgetUsd = 0.1;
    record.machineId = 2 + v;
    record.hostId = 200 + v;
    await assert.doesNotReject(() =>
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01 * v,
        credit: 100,
        retryNonce: 'a'.repeat(32),
      }),
    );
    assert.ok(await ledger.get('cost:1-1:provider:0'));
    const history = await ledger.budgetSummary(cfg, record.started);
    assert.ok(history.spend_rows.some((r) => r.key === '1-1:provider:0'));
  });
  test('rules daily cap cannot be bypassed by replacement ' + v, async () => {
    const { cfg, ledger, record } = await reservationFixture();
    await ledger.put('cost:other-' + v, {
      profile: 'check',
      started: clock - 3600000,
      ended: clock,
      price: 3,
      created: true,
    });
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: 'a'.repeat(32),
      }),
    );
  });
  test('rules teardown needs verified absence ' + v, async () => {
    const cloud = new MockCloud(),
      cfg = configuration(env),
      record = { id: '900', label: 'kf-ci-1-1', idSource: 'create', contractVerified: true };
    cloud.instances.set('900', { id: 900, label: record.label, actual_status: 'running' });
    const f = cloud.fetch;
    const vast = new Vast(
      (url, init = {}) =>
        init.method === 'DELETE' ? Response.json({ success: true }) : f(url, init),
      cfg,
      'mock-key',
    );
    await assert.rejects(vast.destroy(record));
    assert.ok(cloud.instances.has('900'));
  });
  test('rules pending cleanup cannot authorise retry ' + v, () => {
    const r = {
      started: clock,
      ended: null,
      providerRetries: v - 1,
      closed: 'PROVIDER_FAULT',
      verdict: 'PROVIDER_FAULT',
      softDeadline: clock + 1e7,
      sessionDeadline: clock + 1e7,
    };
    assert.equal(retryEligible(r, clock + 1000, policy), false);
  });
  test('rules legacy fault migration preserves permanent controls ' + v, async () => {
    const ledger = new Ledger(new MemoryStorage());
    await ledger.put('bad-machine:1', { reason: 'PROVIDER_FAULT', at: clock });
    await ledger.put('bad-host:2', { reason: 'KVM_DOMAIN_NOT_FOUND', at: clock, permanent: true });
    await ledger.storage.transaction((storage) => new Ledger(storage).migrateLegacyFaults(policy));
    assert.equal(await ledger.get('bad-machine:1'), undefined);
    assert.ok(await ledger.get('provider-machine:1'));
    assert.ok(await ledger.get('bad-host:2'));
  });
}

for (const v of [1, 2])
  test('rules exhaustion lists every attempt ' + v, async () => {
    const h = await setup({ PROVIDER_MAX_ATTEMPTS: '1' });
    h.cloud.offer.host_id = 100 + v;
    await h.call();
    const instance = h.cloud.instances.get('900');
    instance.status_msg =
      v === 1
        ? 'GPU error, unable to start instance'
        : 'Error: GPU error, unable to start instance.';
    for (let i = 0; i < 2; i++) {
      h.time.value += 20000;
      await h.service.sweep();
    }
    const r = await h.storage.get('record:123-1');
    assert.equal(r.replacementExhausted, true);
    assert.equal(r.rentalAttempts.length, 1);
    assert.match(r.log, /rental attempts exhausted/);
    assert.match(r.log, /attempt machine=100001 host=[0-9]+ reason=GPU_START_ERROR seconds=/);
    assert.equal(r.ended !== null, true);
  });

for (const v of [1, 2]) {
  test('rules foreign initial contract is never accepted ' + v, async () => {
    const cloud = new MockCloud(),
      vast = new Vast(cloud.fetch, configuration(env), 'mock-key');
    await assert.rejects(
      vast.inspect({
        id: v === 1 ? '777' : '888',
        label: 'kf-ci-1-1',
        idSource: 'create',
        contractVerified: false,
        proofVersion: 1,
      }),
    );
  });
  test('rules failed identities cannot be reused ' + v, async () => {
    const { cfg, ledger, record, prior } = await reservationFixture();
    if (v === 1) record.machineId = prior.machineId;
    else record.hostId = prior.hostId;
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: prior.nonce,
      }),
    );
  });
  test('rules prior attempt receipts cannot disappear ' + v, async () => {
    const { cfg, ledger, record, prior } = await reservationFixture();
    prior.rentalAttempts = [
      { instance: '899', machine: 30 + v, host: 300 + v, reason: 'GPU_START_ERROR', seconds: 1 },
    ];
    await ledger.save(prior);
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: prior.nonce,
      }),
    );
  });
}

for (const v of [1, 2]) {
  test('rules previous machine is forbidden ' + v, async () => {
    const { cfg, ledger, record, prior } = await reservationFixture();
    record.machineId = prior.machineId;
    record.hostId = 200 + v;
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: prior.nonce,
      }),
    );
  });
  test('rules previous host is forbidden ' + v, async () => {
    const { cfg, ledger, record, prior } = await reservationFixture();
    record.hostId = prior.hostId;
    record.machineId = 200 + v;
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: prior.nonce,
      }),
    );
  });
}

for (const v of [1, 2])
  test('rules renamed owned contract has unique startup proof ' + v, async () => {
    const cloud = new MockCloud(),
      script = 'bootstrap\n# rental-proof=' + String(v).repeat(32);
    const record = {
      id: '900',
      label: 'kf-ci-1-1',
      idSource: 'create',
      contractVerified: false,
      proofVersion: 1,
      scriptHash: await sha256Hex(script),
    };
    cloud.instances.set('900', {
      id: 900,
      label: 'provider-renamed',
      onstart: script,
      image_uuid: 'opaque-image-uuid-' + v,
    });
    const vast = new Vast(cloud.fetch, configuration(env), 'mock-key');
    await assert.doesNotReject(() => vast.inspect(record));
    await vast.destroy(record);
    assert.equal(await vast.inspect(record), null);
    assert.deepEqual(cloud.deletes, ['900']);
  });

for (const v of [1, 2])
  test('rules uncreated retry needs a daily count slot ' + v, async () => {
    const { cfg, ledger, prior, record } = await reservationFixture();
    cfg.dailyCount = v;
    const caps = cfg.profiles?.check;
    if (caps) caps.dailyCount = v;
    prior.id = null;
    prior.noInstance = true;
    prior.price = 0;
    await ledger.save(prior);
    await ledger.put('cost:1-1', {
      profile: 'check',
      started: clock,
      ended: prior.ended,
      price: 0,
      created: false,
    });
    record.retryCostUsd = 0;
    for (let i = 0; i < v; i++)
      await ledger.put('cost:other-' + i, {
        profile: 'check',
        started: clock,
        ended: clock + 1,
        price: 0.001,
        created: true,
      });
    await assert.rejects(
      ledger.reserve(record, {
        cfg,
        now: record.started,
        cost: 0.01,
        credit: 100,
        retryNonce: prior.nonce,
      }),
      (e) => e.reason === 'BUDGET:daily-count',
    );
    assert.equal(await ledger.get('cost:1-1:provider:0'), undefined);
  });

for (const v of [1, 2])
  test('rules generic create refusal never penalises hardware ' + v, async () => {
    const h = await setup();
    const original = h.cloud.fetch;
    h.cloud.fetch = (url, init) =>
      String(url).includes('/asks/')
        ? Response.json(
            { success: false, msg: v === 1 ? 'Insufficient credit' : 'Account instance limit' },
            { status: 400 },
          )
        : original(url, init);
    h.restart();
    assert.equal((await h.call()).status, 200);
    const r = await h.storage.get('record:123-1');
    assert.equal(r.verdict, 'PROVIDER_FAULT');
    assert.equal(r.noInstance, true);
    assert.equal(await h.storage.get('provider-machine:' + r.machineId), undefined);
    assert.equal(await h.storage.get('provider-host:' + r.hostId), undefined);
    assert.equal(h.cloud.creates, 0);
  });

for (const v of [1, 2])
  test('rules persisted reported fault is stored as failure ' + v, async () => {
    const { storeVerdict } = await import('../src/ingest.js');
    const verdict = v === 1 ? 'PROVIDER_FAULT' : 'UNAVAILABLE';
    assert.equal(
      storeVerdict({
        ended: clock,
        firstReportAt: clock - 1000,
        verdict,
        closed: 'PROVIDER_FAULT',
      }),
      'FAIL',
    );
    assert.equal(
      storeVerdict({ ended: clock, verdict, closed: 'PROVIDER_FAULT' }),
      v === 1 ? 'RETRY' : 'UNATTRIBUTED',
    );
  });

for (const v of [1, 2])
  test('rules address masking preserves non-address facts ' + v, () => {
    const mac = v === 1 ? '52:54:00:12:34:56' : 'aa:bb:cc:dd:ee:ff';
    assert.equal(redactEndpoints(mac), mac);
    assert.equal(redactEndpoints('580.159.04.0'), '580.159.04.0');
    assert.equal(redactEndpoints('203.0.113.71:45222'), '203.0.113.0');
    assert.equal(redactEndpoints('[2001:db8::71]:45222'), '2001:db8::/48');
  });

for (const leg of [10, 31])
  test('rules later attempt tree queue remains bounded ' + leg, async () => {
    const { storeTrees } = await import('../src/ingest.js');
    const ledger = new Ledger(new MemoryStorage());
    const sha = 'a'.repeat(40),
      other = 'b'.repeat(40),
      seen = [];
    await ledger.put('store:123-1:' + leg, { record: { sha, providerRetries: leg } });
    await ledger.put('store:124-1:32', { record: { sha: other, providerRetries: 32 } });
    const options = {
      ledger,
      env: { DB: { prepare: () => null }, ROW_TAG_KEY: 't'.repeat(48) },
      now: clock,
      treeKey: async (key) => {
        seen.push(key);
        return { kind: 'tree', key: 'c'.repeat(64) };
      },
    };
    await storeTrees(options);
    await storeTrees(options);
    assert.deepEqual(seen, [sha]);
    assert.equal((await ledger.get('treekey:' + sha)).kind, 'tree');
    assert.equal(await ledger.get('treekey:' + other), undefined);
  });

for (const flag of ['-p', '-P'])
  test('rules compact SSH port remains private ' + flag, () => {
    assert.equal(
      redactEndpoints('ssh ' + flag + '45222 user@203.0.113.71'),
      'ssh -p [redacted] user@203.0.113.0',
    );
  });

for (const v of [1, 2])
  test('rules same-label foreign contract has no rental authority ' + v, async () => {
    const cloud = new MockCloud();
    const record = {
      id: '900',
      label: 'kf-ci-1-1',
      idSource: 'create',
      proofVersion: 1,
      scriptHash: await sha256Hex('owned nonce ' + v),
    };
    cloud.instances.set('900', {
      id: 900,
      label: record.label,
      onstart: 'foreign nonce ' + v,
      image_uuid: IMAGE,
    });
    const vast = new Vast(cloud.fetch, configuration(env), 'mock-key');
    await assert.rejects(vast.inspect(record), (error) => error.state === 'OWNERSHIP_MISMATCH');
    await assert.rejects(vast.destroy(record));
    assert.equal(cloud.instances.has('900'), true);
    assert.deepEqual(cloud.deletes, []);
  });
