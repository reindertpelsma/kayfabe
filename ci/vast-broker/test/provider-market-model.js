import assert from 'node:assert/strict';
import { configuration, profileCaps } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { Ledger, rentalCost } from '../src/ledger.js';
import { OfferBook } from '../src/offers.js';
import { Vast } from '../src/vast.js';
import { retryEligible, attemptReceipt } from '../src/provider-policy.js';
import { ipPrefix, rentalLine, redactEndpoints } from '../src/rental.js';
import { MockCloud, MemoryStorage, env, clock, sha } from './helpers.js';
import { randomMarket, rng } from './simulation-model.js';
export async function simulateProviderSeed(seed) {
  const random = rng(seed),
    storage = new MemoryStorage(),
    ledger = new Ledger(storage),
    cloud = new MockCloud();
  let at = clock,
    prior = null,
    rentals = 0,
    reported = false;
  const cfg = applyProfile(
    configuration({
      ...env,
      BAD_HOST_IDS: '[99991,99993]',
      BAD_MACHINE_IDS: '[99992,99994]',
      PINNED_MIDDLE_DIES:
        seed % 5 === 0 ? '[]' : JSON.stringify(['AD102', 'AD103', 'AD104', 'AD106']),
      PROVIDER_MAX_ATTEMPTS: String(2 + (seed % 4)),
    }),
    'check',
    'middle',
  );
  cloud.credit = 500;
  cloud.contracts = Array.from({ length: 16 }, (_, i) => 900 + i);
  let market = randomMarket(seed).filter((o, i) => i % 8 === 0 || i % 8 === 1 + (seed % 7));
  // Some seeds have no first pinned die or no market; these are not GPU failures.
  if (seed % 3 === 0) market = market.filter((o) => o.gpu_name !== 'RTX_4090');
  if (seed % 31 === 0) market = [];
  const prices = market.map((o) => ({
    ...o,
    price: o.dph_base + (o.storage_cost * cfg.diskGb) / 720,
  }));
  cloud.offer = prices[0] ?? { ...cloud.offer, verified: false };
  cloud.price = prices[0]?.price ?? 0.2;
  cloud.extraOffers = prices.slice(1);
  market = prices.map(({ price, ...o }) => ({ ...o, dph_total: price }));
  const vast = new Vast(cloud.fetch, cfg, 'mock-market-key'),
    book = new OfferBook(() => at),
    cap = profileCaps(cfg, 'check').dailyUsd;
  const output = [];
  for (let n = 0; n < cfg.provider.maxAttempts; n++) {
    if (prior && !retryEligible(prior, at, cfg.provider)) break;
    const evaluation = await book.evaluate(vast, cfg, ledger, 'check', { market });
    const previous = prior?.rentalAttempts ?? [];
    evaluation.allowed = evaluation.allowed.filter(
      (o) => !previous.some((a) => a.machine === o.machine_id || a.host === o.host_id),
    );
    const spent = (prior?.retryCostUsd ?? 0) + (prior ? rentalCost(prior, prior.ended) : 0);
    evaluation.allowed = evaluation.allowed.filter((o) => o.pricing.total + spent <= cap);
    if (!evaluation.allowed.length) {
      if (prior) at = Math.min(prior.sessionDeadline, prior.softDeadline);
      break;
    }
    const chosen = await book.select(evaluation, cfg, ledger, 'check', undefined, {
      tierName: 'middle',
    });
    const reversed = await book.select(
      { ...evaluation, allowed: [...evaluation.allowed].reverse() },
      cfg,
      ledger,
      'check',
      undefined,
      { tierName: 'middle' },
    );
    assert.equal(chosen.offer.id, reversed.offer.id, 'I5 deterministic ranking');
    const o = chosen.offer;
    const eligiblePins = cfg.pinnedMiddleDies.filter((d) =>
      evaluation.allowed.some((x) => x.die === d),
    );
    if (eligiblePins.length) assert.equal(o.die, eligiblePins[0], 'I5 pin order');
    else if (cfg.pinnedMiddleDies.length) assert.equal(chosen.reason, 'PIN_EXHAUSTED');
    const equivalent = evaluation.allowed.filter((x) => x.die === o.die && x.country === o.country);
    assert.ok(
      equivalent.every(
        (x) =>
          o.pricing.effective + (o.penalty ?? 0) <= x.pricing.effective + (x.penalty ?? 0) + 1e-12,
      ),
      'I5 weighted price',
    );
    assert.ok(o.pricing.effective <= profileCaps(cfg, 'check').price, 'I2 price cap');
    assert.ok(o.inet_down_cost <= cfg.maxInet && o.inet_up_cost <= cfg.maxInet, 'I2 per GB caps');
    assert.equal(
      previous.some((a) => a.machine === o.machine_id || a.host === o.host_id),
      false,
      'I5 fresh failure identities',
    );
    const r = {
      key: '123-1',
      run: '123',
      attempt: 1,
      sha,
      branch: 'simulation',
      profile: 'check',
      runnerNonce: 'f'.repeat(64),
      nonce: (n + 1).toString(16).padStart(32, '0'),
      machineId: o.machine_id,
      hostId: o.host_id,
      gpu: o.gpu_name,
      country: o.country,
      datacenter: seed % 2 === 0,
      ipPrefix: ipPrefix('203.0.113.' + (1 + (seed % 250))),
      dphTotal: o.dph_total,
      started: at,
      ended: null,
      id: null,
      label: 'kf-ci-123-1',
      price: o.pricing.effective,
      computeStoragePrice: o.pricing.computeStorage,
      bandwidthCost: o.pricing.bandwidth,
      ttl: cfg.hard,
      billingTtl: cfg.hard + cfg.cronPeriod,
      hardDeadline: clock + cfg.hard,
      softDeadline: clock + cfg.hard - cfg.softMargin,
      sessionDeadline: clock + cfg.provider.sessionMinutes * 60000,
      runBudgetUsd: cap,
      providerRetries: n,
      rentalAttempts: structuredClone(previous),
    };
    if (prior) {
      r.originalStarted = clock;
      r.retryCostUsd = spent;
      r.ttl = r.hardDeadline - at;
      r.billingTtl = r.ttl + cfg.cronPeriod;
    }
    const cost = o.pricing.total;
    await ledger.reserve(r, { cfg, now: at, cost, credit: 500, retryNonce: prior?.nonce });
    const script = 'mock boot';
    r.id = await vast.create(o, r, script, {});
    r.idSource = 'create';
    r.contractVerified = true;
    await ledger.save(r);
    rentals++;
    assert.ok(await vast.inspect(r), 'I1 owned instance exists');
    assert.ok(rentals <= cfg.provider.maxAttempts, 'I4 attempts');
    assert.ok(at < r.sessionDeadline, 'I4 session clock');
    const budget = await ledger.budgetSummary(cfg, at);
    assert.ok(
      budget.profiles.check.daily_usd <= cap + 1e-10,
      'I2 all reservations and failed transfers charged',
    );
    output.push(rentalLine(r));
    at += 1000 + Math.floor(random() * 20000);
    const fault = n < (seed % 8 === 0 ? 1 : 0);
    if (fault) {
      r.faultReason = 'GPU_START_ERROR';
      r.closed = 'PROVIDER_FAULT';
      r.verdict = 'PROVIDER_FAULT';
      await ledger.markProviderFault(r, r.faultReason, at, cfg.provider);
      r.rentalAttempts.push(attemptReceipt(r, at));
    } else {
      reported = true;
      r.firstReportAt = at;
      r.verdict = seed % 7 === 0 ? 'FAIL' : 'PASS';
      r.closed = 'RESULT';
      if (r.verdict === 'PASS') await ledger.markProviderSuccess(r, cfg.provider);
    }
    await vast.destroy(r);
    assert.equal(await vast.inspect(r), null, 'I1 absence verified');
    await ledger.settle(r, at);
    assert.ok(
      rentalCost(r, r.ended) + (r.retryCostUsd ?? 0) <= cap + 1e-10,
      'I2 final cumulative spend',
    );
    if (reported) {
      assert.equal(retryEligible(r, at, cfg.provider), false, 'I3 report is final');
      break;
    }
    prior = r;
  }
  for (const c of cloud.created)
    assert.equal(cloud.instances.has(String(c.contract)), false, 'I1 no leak at run end');
  assert.equal((await ledger.openRecords()).length, 0, 'I1 ledger settled');
  const text = redactEndpoints(output.join('\n'));
  assert.equal(
    text.includes('203.0.113.' + (1 + (seed % 250))) && 1 + (seed % 250) !== 0,
    false,
    'I7 full IP absent',
  );
  assert.equal(text.includes('ssh_port='), false, 'I7 no port field');
  if (prior && !reported) {
    assert.equal(prior.verdict, 'PROVIDER_FAULT', 'I6 only provider exhaustion RETRY');
    assert.equal(prior.rentalAttempts.length, rentals, 'I6 all attempts accounted');
  }
  return rentals;
}
