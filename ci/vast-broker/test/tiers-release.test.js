import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration, profileCaps } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { tierForEvent, tierEnvironment, familyOf, TIER_SETTINGS } from '../src/tier.js';
import {
  planRelease,
  dieLegs,
  classifyLeg,
  releaseVerdict,
  wantsRerun,
  retryRow,
} from '../src/release.js';
import { env } from './helpers.js';
// wtier, 2026-11-05: tiers small/middle/large and the release (large) planner, classifier and
// verdict, GPU-free. FALSIFIERS (written before the code): a request field or an unverified event
// picks a tier; a release plans fewer than 2 dies of a family or fewer than 2 Windows runs per die;
// a second Windows leg lands on the same machine while another machine offers the die; a plan over
// the cost cap or a cap is admitted; a RETRY, UNATTRIBUTED, provider fault or owner stop counts as
// a PASS; a FAIL is re-rented; a DESIGN_ONLY stage (two_guests, trio) can be enabled.
// Release-ready defaults for these tests: the kill switch on and an account cap of 9 (release 8).
const cfgOf = (extra = {}) =>
  configuration({ ...env, RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9', ...extra });
const hours = (cfg) => (cfg.hardRelease + cfg.cronPeriod) / 3600000;
// One allowed offer as src/offers.js evaluates it (die, country and pricing filled in).
function allowed(cfg, die, { id, machine, host, price = 0.2, country = 'US' }) {
  const cc = cfg.dieRules.find((r) => r.die === die).cc;
  return {
    id,
    die,
    machine_id: machine,
    host_id: host,
    gpu_name: 'GPU ' + die,
    compute_cap: cc,
    country,
    dph_total: price,
    pricing: { computeStorage: price, bandwidth: 0, effective: price, total: price * hours(cfg) },
  };
}
// The measured release dies (defaults.json releaseDies, first two per family), one or two
// machines each: ids are unique, hosts too unless stated.
function market(cfg, { second = ['TU116', 'GA106', 'AD104', 'GB205'], drop = [], price } = {}) {
  const rows = [];
  let n = 1;
  for (const dies of Object.values(cfg.releaseDies))
    for (const die of dies) {
      if (drop.includes(die)) continue;
      rows.push(allowed(cfg, die, { id: n, machine: 1000 + n, host: 5000 + n, price }));
      n++;
      if (second.includes(die)) {
        rows.push(allowed(cfg, die, { id: n, machine: 1000 + n, host: 5000 + n, price }));
        n++;
      }
    }
  return rows;
}
test('the event picks the tier; a request field never does', () => {
  assert.equal(tierForEvent({ event: 'pull_request_target' }), 'middle');
  assert.equal(tierForEvent({ event: 'pull_request_target', baseRef: 'master' }), 'middle');
  assert.equal(tierForEvent({ event: 'pull_request_target', baseRef: 'next' }), 'small');
  assert.equal(tierForEvent({ event: 'push', ref: 'refs/heads/master' }), 'small');
  assert.equal(tierForEvent({ event: 'push', ref: 'refs/tags/v0.3.0' }), 'large');
  assert.equal(tierForEvent({ event: 'push', ref: 'refs/tags/nightly' }), 'small');
  assert.equal(tierForEvent({ event: 'workflow_dispatch' }), 'small');
  assert.equal(tierForEvent({ event: 'workflow_dispatch', profile: 'sweep' }), 'adhoc');
  assert.equal(tierForEvent({}), 'small');
  assert.deepEqual(
    Object.fromEntries(Object.entries(TIER_SETTINGS).map(([k, v]) => [k, v.profile])),
    { small: 'check', middle: 'check', large: 'release' },
  );
});
test('the die lists are the measured ones, every family first-class, two dies per family', () => {
  // Measured on 2026-11-04 at rev 0c515b3b (traces/die_coverage_20261104/*/README.md): Windows
  // gate PASS on TU106, TU116, GA104, AD102, AD104, GB202, GB205, GB206; GA106 the everyday bench.
  assert.deepEqual(Object.keys(DEFAULTS.families), ['turing', 'ampere', 'ada', 'blackwell']);
  assert.deepEqual(
    Object.values(DEFAULTS.releaseDies).map((d) => d.slice(0, 2)),
    [
      ['TU106', 'TU116'],
      ['GA106', 'GA104'],
      ['AD104', 'AD102'],
      ['GB205', 'GB206'],
    ],
  );
  for (const list of [DEFAULTS.releaseDies, DEFAULTS.middleRotation])
    for (const [family, dies] of Object.entries(list))
      for (const die of dies) {
        assert.ok(DEFAULTS.sweepDies.includes(die), die);
        assert.equal(familyOf(die, DEFAULTS), family, die);
      }
  assert.equal(DEFAULTS.releaseDiesPerFamily, 2);
  assert.equal(DEFAULTS.releaseWindowsRunsPerDie, 2);
  assert.equal(DEFAULTS.middleVramFloorMb, 0, 'TBD until the trio footprints are measured');
  assert.deepEqual(DEFAULTS.releaseStages, []);
});
test('release caps are explicit, per profile, and bounded', () => {
  const cfg = cfgOf();
  assert.deepEqual(
    [cfg.releasePrice, cfg.releaseDaily, cfg.releaseDailyCount, cfg.releaseCostCap],
    [2, 40, 24, 40],
  );
  assert.equal(cfg.hardRelease, 300 * 60000);
  assert.equal(cfg.releaseReruns, 1);
  const caps = ['check', 'sweep', 'release'].map((p) => profileCaps(cfg, p));
  assert.deepEqual(
    caps.map((c) => c.price),
    [cfg.price, cfg.sweepPrice, cfg.releasePrice],
  );
  assert.deepEqual(
    caps.map((c) => c.dailyUsd),
    [cfg.daily, cfg.sweepDaily, cfg.releaseDaily],
  );
  assert.deepEqual(
    caps.map((c) => c.dailyCount),
    [cfg.dailyCount, cfg.dailyCount, cfg.releaseDailyCount],
  );
  assert.ok(caps[2].dies.includes('GB206') && !caps[0].dies.includes('GB202'));
  assert.throws(() => profileCaps(cfg, 'large'), /unknown profile/);
  // The release hard limit may not pass the check rule (the reaper deletes after 300 + 15).
  assert.throws(() => cfgOf({ RELEASE_HARD_RUNTIME_MINUTES: '301' }));
  assert.equal(cfgOf({ RELEASE_HARD_RUNTIME_MINUTES: '240' }).hardRelease, 240 * 60000);
  assert.throws(() => cfgOf({ RELEASE_COST_CAP_USD: '41' }), /RELEASE_COST_CAP_USD/);
  assert.equal(cfgOf({ RELEASE_COST_CAP_USD: '25' }).releaseCostCap, 25);
  for (const bad of ['3', '-1', 'x'])
    assert.throws(() => cfgOf({ RELEASE_RERUNS_PER_LEG: bad }), /RELEASE_RERUNS_PER_LEG/);
  assert.equal(cfgOf({ MIDDLE_VRAM_FLOOR_MB: '12000' }).middleVramFloorMb, 12000);
  for (const bad of ['-1', '1.5', '01', '9999999'])
    assert.throws(() => cfgOf({ MIDDLE_VRAM_FLOOR_MB: bad }), /MIDDLE_VRAM_FLOOR_MB/);
});
test('stage slots: specviewperf may be enabled for the release; two_guests and trio may not', () => {
  assert.deepEqual(cfgOf({ RELEASE_STAGES: '["specviewperf"]' }).releaseStages, ['specviewperf']);
  for (const bad of ['["trio"]', '["two_guests"]', '["apps"]', '["specviewperf","specviewperf"]'])
    assert.throws(() => cfgOf({ RELEASE_STAGES: bad }), /RELEASE_STAGES/, bad);
  assert.equal(DEFAULTS.stageSlots.two_guests.state, 'DESIGN_ONLY');
  assert.equal(DEFAULTS.stageSlots.trio.state, 'DESIGN_ONLY');
  assert.deepEqual(DEFAULTS.stageSlots.specviewperf.tiers, ['large']);
});
test('a family list cannot move a die across families or drop a family', () => {
  const rotation = structuredClone(DEFAULTS.middleRotation);
  rotation.turing = ['GA106'];
  assert.throws(() => cfgOf({ MIDDLE_ROTATION: JSON.stringify(rotation) }), /MIDDLE_ROTATION/);
  const { blackwell: _, ...three } = DEFAULTS.releaseDies;
  assert.throws(() => cfgOf({ RELEASE_DIES: JSON.stringify(three) }), /RELEASE_DIES/);
  const one = { ...DEFAULTS.releaseDies, turing: ['TU106'] };
  assert.throws(() => cfgOf({ RELEASE_DIES: JSON.stringify(one) }), /RELEASE_DIES/);
});
test('box environment: only the release profile may ask for a second Windows run or a stage', () => {
  assert.deepEqual(
    tierEnvironment('release', { tierName: 'large', windowsRuns: 2, stages: ['specviewperf'] }),
    { KF_MERGE_WINDOWS: '1', KF_TIER: 'large', KF_WINDOWS_RUNS: '2', KF_STAGES: 'specviewperf' },
  );
  assert.deepEqual(tierEnvironment('check', { tierName: 'middle' }), {
    KF_MERGE_WINDOWS: '1',
    KF_TIER: 'middle',
  });
  assert.throws(() => tierEnvironment('check', { windowsRuns: 2 }), /windows runs/);
  assert.throws(() => tierEnvironment('release', { windowsRuns: 3 }), /windows runs/);
  assert.throws(() => tierEnvironment('sweep', { stages: ['specviewperf'] }), /stages/);
  assert.throws(() => tierEnvironment('release', { stages: ['Bad-Name'] }), /stages/);
});
test('KF_SPEC (the SPECviewperf lane) is sent only when asked for, only for release, only with the stage slot, only for a named lane', () => {
  // today's callers pass nothing: no box sees the field
  for (const p of ['check', 'sweep', 'release']) assert.ok(!('KF_SPEC' in tierEnvironment(p)), p);
  assert.ok(
    !('KF_SPEC' in tierEnvironment('release', { tierName: 'large', stages: ['specviewperf'] })),
  );
  for (const lane of ['lanea', 'laneb', 'both'])
    assert.deepEqual(
      tierEnvironment('release', { tierName: 'large', stages: ['specviewperf'], spec: lane }),
      {
        KF_MERGE_WINDOWS: '1',
        KF_TIER: 'large',
        KF_STAGES: 'specviewperf',
        KF_SPEC: lane,
      },
    );
  // a lane without the slot, on another profile, or with a name that is not a lane
  assert.throws(() => tierEnvironment('release', { spec: 'lanea' }), /spec/);
  assert.throws(() => tierEnvironment('release', { stages: ['trio'], spec: 'lanea' }), /spec/);
  assert.throws(
    () => tierEnvironment('check', { stages: ['specviewperf'], spec: 'lanea' }),
    /stages/,
  );
  assert.throws(() => tierEnvironment('sweep', { spec: 'lanea' }), /spec/);
  for (const bad of ['', 'lanec', 'LANEA', 'lanea:laneb', 'all', null, 1])
    assert.throws(
      () => tierEnvironment('release', { stages: ['specviewperf'], spec: bad }),
      /spec/,
      String(bad),
    );
});
test('the Worker sends no SPECviewperf stage and no lane by default', () => {
  assert.deepEqual(DEFAULTS.releaseStages, []);
  assert.deepEqual(DEFAULTS.stageSlots.specviewperf, { tiers: ['large'], state: 'PLUMBING' });
});
test('plan: two dies per family, two Windows runs per die, a second machine when offered', () => {
  const cfg = cfgOf();
  const plan = planRelease(market(cfg), cfg);
  assert.deepEqual(plan.refusals, []);
  assert.equal(plan.admitted, true);
  assert.equal(plan.parallel, 8);
  assert.deepEqual(
    plan.families.map((f) => [f.family, f.cells.map((c) => c.die)]),
    [
      ['turing', ['TU106', 'TU116']],
      ['ampere', ['GA106', 'GA104']],
      ['ada', ['AD104', 'AD102']],
      ['blackwell', ['GB205', 'GB206']],
    ],
  );
  for (const f of plan.families)
    for (const c of f.cells) {
      const runs = c.legs.reduce((s, l) => s + l.windows_runs, 0);
      assert.ok(runs >= 2, c.die);
      assert.equal(new Set(c.legs.map((l) => l.machine_id)).size, c.legs.length, c.die);
      assert.equal(c.same_machine, c.legs.length === 1);
    }
  // TU116, GA106, AD104, GB205 have two machines (two legs); the other four run Windows twice.
  assert.equal(plan.legs, 12);
  const per = 0.2 * hours(cfg);
  assert.ok(Math.abs(plan.estimate_usd - 12 * per) < 1e-3, String(plan.estimate_usd));
  assert.deepEqual(Object.keys(plan.estimate_by_family_usd), Object.keys(cfg.families));
});
test('plan: a missing die falls back to the next measured die of its family', () => {
  const cfg = cfgOf();
  const plan = planRelease(market(cfg, { drop: ['GA104'] }), cfg);
  const ampere = plan.families.find((f) => f.family === 'ampere');
  assert.deepEqual(
    ampere.cells.map((c) => c.die),
    ['GA106', 'GA102'],
  );
  assert.deepEqual(ampere.missing, [{ die: 'GA104', reason: 'NO_OFFER' }]);
  assert.equal(plan.admitted, true);
});
test('plan: a family short of two offered dies is INCOMPLETE and not admitted', () => {
  const cfg = cfgOf();
  const plan = planRelease(market(cfg, { drop: ['TU116'] }), cfg);
  assert.equal(plan.families[0].complete, false);
  assert.deepEqual(plan.refusals, ['INCOMPLETE']);
  assert.equal(plan.admitted, false);
});
test('plan: over the cost cap or any cap is refused before renting', () => {
  const cfg = cfgOf({ RELEASE_COST_CAP_USD: '5' });
  const plan = planRelease(market(cfg), cfg);
  assert.ok(plan.estimate_usd > 5);
  assert.deepEqual(plan.refusals, ['COST_CAP']);
  const big = cfgOf();
  const budget = {
    profiles: { release: { daily_usd: 39, count_today: 20 } },
    rolling_usd: 29.5,
    outstanding_usd: 0,
  };
  const capped = planRelease(market(big), big, { budget, credit: 9 });
  assert.deepEqual(capped.refusals, ['DAILY_USD', 'DAILY_COUNT', 'ROLLING_USD', 'CREDIT_RESERVE']);
});
test('dieLegs: the second machine is on another host when one exists', () => {
  const cfg = cfgOf();
  const offers = [
    allowed(cfg, 'AD104', { id: 1, machine: 11, host: 7, price: 0.1 }),
    allowed(cfg, 'AD104', { id: 2, machine: 12, host: 7, price: 0.11 }),
    allowed(cfg, 'AD104', { id: 3, machine: 13, host: 8, price: 0.3 }),
  ];
  const legs = dieLegs(offers, cfg);
  assert.deepEqual(
    legs.map((l) => [l.machine_id, l.host_id, l.windows_runs]),
    [
      [11, 7, 1],
      [13, 8, 1],
    ],
  );
  // Same host only: still another machine.
  assert.deepEqual(
    dieLegs(offers.slice(0, 2), cfg).map((l) => l.machine_id),
    [11, 12],
  );
  // One machine: one leg that runs Windows twice.
  assert.deepEqual(
    dieLegs(offers.slice(0, 1), cfg).map((l) => l.windows_runs),
    [2],
  );
});
const rows = (spec) =>
  Object.entries(spec).map(([name, v]) => ({
    name,
    pass: v > 0 ? v : 0,
    fail: v < 0 ? 1 : 0,
    skip: 0,
  }));
const LINUX = { provision: 1, gates: 9, thin_suite: 30, cuda_host: 4, cuda_fat: 4 };
// The leg under test is an AD104 leg; its box reports an RTX 4070 (review F3).
const AD104 = {
  gpu_name: 'NVIDIA GeForce RTX 4070',
  compute_cap: 890,
  driver_version: '580.159.04',
};
const classify = (r, o = {}) => classifyLeg(r, { die: 'AD104', family: 'ada', cfg: cfgOf(), ...o });
const rec = (verdict, spec, { green = true, closed = 'RESULT', resultVerdict = verdict } = {}) => ({
  key: '8-1',
  ended: 1,
  hardware: AD104,
  closed,
  verdict,
  result: {
    verdict: resultVerdict,
    suites: rows(spec),
    host_reference: green ? 'GREEN' : 'NOT_GREEN',
  },
});
test('classify: PASS only from the Worker PASS with every expected row passing', () => {
  const ok = classify(rec('PASS', { ...LINUX, windows_gate: 1 }));
  assert.deepEqual([ok.verdict, ok.linux, ok.windows], ['PASS', 'PASS', ['PASS']]);
  const two = classify(rec('PASS', { ...LINUX, windows_gate: 1, windows_gate_2: 1 }), {
    windowsRuns: 2,
  });
  assert.deepEqual([two.verdict, two.windows], ['PASS', ['PASS', 'PASS']]);
  // A PASS whose second Windows row is missing is not two passes.
  const short = classify(rec('PASS', { ...LINUX, windows_gate: 1 }), { windowsRuns: 2 });
  assert.deepEqual([short.verdict, short.windows], ['RETRY', ['PASS', 'RETRY']]);
  // Box rows cannot turn a Worker FAIL or a provider fault into a pass.
  assert.equal(classify(rec('FAIL', { ...LINUX, windows_gate: 1 })).verdict, 'RETRY');
  assert.equal(classify(rec('PROVIDER_FAULT', { ...LINUX, windows_gate: 1 })).verdict, 'RETRY');
  assert.equal(classify({ ...rec('PASS', LINUX), ended: null }).verdict, 'RETRY');
  assert.equal(classify(undefined).verdict, 'RETRY');
});
test('classify: FAIL is the guest; no host reference is UNATTRIBUTED; harness rows are RETRY', () => {
  const linux = classify(rec('FAIL', { provision: 1, gates: 9, thin_suite: -1 }));
  assert.deepEqual([linux.verdict, linux.linux, linux.windows], ['FAIL', 'FAIL', ['NOT_RUN']]);
  const blind = classify(rec('FAIL', { provision: -1 }, { green: false }));
  assert.deepEqual([blind.verdict, blind.linux], ['UNATTRIBUTED', 'UNATTRIBUTED']);
  const winRetry = classify(rec('FAIL', { ...LINUX, windows_retry_media: -1 }));
  assert.deepEqual(
    [winRetry.verdict, winRetry.linux, winRetry.windows],
    ['RETRY', 'PASS', ['RETRY']],
  );
  const win = classify(rec('FAIL', { ...LINUX, windows_gate: -1 }));
  assert.deepEqual([win.verdict, win.linux, win.windows], ['FAIL', 'PASS', ['FAIL']]);
  const second = classify(rec('FAIL', { ...LINUX, windows_gate: 1, windows_gate_2: -1 }), {
    windowsRuns: 2,
  });
  assert.deepEqual([second.verdict, second.windows], ['FAIL', ['PASS', 'FAIL']]);
  const infra = classify(rec('FAIL', { provision: 1, infrastructure: -1 }));
  assert.deepEqual([infra.verdict, infra.linux], ['RETRY', 'RETRY']);
  const stopped = classify(rec('FAIL', LINUX, { closed: 'STOPPED' }));
  assert.deepEqual([stopped.verdict, stopped.reason], ['UNATTRIBUTED', 'STOPPED']);
  assert.ok(retryRow('windows_retry_hostref') && retryRow('specviewperf_retry_not_built'));
  assert.ok(!retryRow('windows_gate') && !retryRow('timeout') && !retryRow('host_xid'));
});
test('classify: an enabled stage is its own part; its retry row is not a pass', () => {
  const stages = ['specviewperf'];
  const ok = classify(rec('PASS', { ...LINUX, windows_gate: 1, specviewperf: 1 }), { stages });
  assert.deepEqual([ok.verdict, ok.stages], ['PASS', { specviewperf: 'PASS' }]);
  const stub = classify(
    rec('FAIL', { ...LINUX, windows_gate: 1, specviewperf_retry_not_built: -1 }),
    { stages },
  );
  assert.deepEqual(
    [stub.verdict, stub.linux, stub.windows, stub.stages],
    ['RETRY', 'PASS', ['PASS'], { specviewperf: 'RETRY' }],
  );
});
const leg = (key, family, die, outcome, state = 'ENDED', machineId = 1) => ({
  key,
  family,
  die,
  state,
  machineId,
  outcome,
});
const out = (linux, windows) => ({ verdict: 'X', linux, windows, stages: {} });
const fullBatch = (legs) => ({
  tier: 'large',
  windowsRunsPerDie: 2,
  diesPerFamily: 2,
  releaseDies: DEFAULTS.releaseDies,
  stages: [],
  legs,
});
function passingLegs() {
  const legs = [];
  let n = 1;
  for (const [family, dies] of Object.entries(DEFAULTS.releaseDies))
    for (const die of dies.slice(0, 2))
      legs.push(leg('8-' + n++, family, die, out('PASS', ['PASS', 'PASS']), 'ENDED', n));
  return legs;
}
test('verdict: PASS needs every family, two dies each, linux and two Windows passes', () => {
  const view = releaseVerdict(fullBatch(passingLegs()));
  assert.equal(view.verdict, 'PASS');
  assert.equal(view.cells.length, 16);
  assert.ok(view.cells.every((c) => c.verdict === 'PASS'));
  // One Windows pass on a die is not enough.
  const legs = passingLegs();
  legs[0].outcome = out('PASS', ['PASS', 'RETRY']);
  assert.equal(releaseVerdict(fullBatch(legs)).verdict, 'INCOMPLETE');
  // ... until a fresh-box re-run passes; the RETRY itself never counts.
  legs.push(leg('8-99', legs[0].family, legs[0].die, out('PASS', ['PASS']), 'ENDED', 77));
  const after = releaseVerdict(fullBatch(legs));
  assert.equal(after.verdict, 'PASS');
  const cell = after.cells.find((c) => c.die === legs[0].die && c.platform === 'windows');
  assert.deepEqual([cell.passes, cell.machines], [2, 2]);
});
test('verdict: any FAIL fails the release; a missing family or die is INCOMPLETE', () => {
  const legs = passingLegs();
  legs[5].outcome = out('PASS', ['FAIL', 'PASS']);
  assert.equal(releaseVerdict(fullBatch(legs)).verdict, 'FAIL');
  const missing = passingLegs().filter((l) => l.family !== 'blackwell');
  assert.equal(releaseVerdict(fullBatch(missing)).verdict, 'INCOMPLETE');
  const one = passingLegs().filter((l) => l.die !== 'TU116');
  assert.equal(releaseVerdict(fullBatch(one)).verdict, 'INCOMPLETE');
  const running = passingLegs();
  running.push(leg('8-50', 'ada', 'AD104', undefined, 'STARTED'));
  assert.equal(releaseVerdict(fullBatch(running)).verdict, 'RUNNING');
  assert.equal(releaseVerdict(fullBatch([])).verdict, 'INCOMPLETE');
});
test('re-runs: RETRY and UNATTRIBUTED within the allowance; never FAIL or an owner stop', () => {
  const l = (verdict, reason = 'RESULT', depth = 0) => ({
    outcome: { verdict, reason },
    rerunDepth: depth,
  });
  assert.equal(wantsRerun(l('RETRY'), 1), true);
  assert.equal(wantsRerun(l('UNATTRIBUTED'), 1), true);
  assert.equal(wantsRerun(l('RETRY', 'RESULT', 1), 1), false);
  assert.equal(wantsRerun(l('FAIL'), 2), false);
  assert.equal(wantsRerun(l('PASS'), 2), false);
  assert.equal(wantsRerun(l('UNATTRIBUTED', 'STOPPED'), 2), false);
  assert.equal(wantsRerun(l('RETRY'), 0), false);
});
test('offline plan CLI: the synthetic market plans 12 legs; parallelism follows the caps', async () => {
  const { offlinePlan } = await import('../release-plan.mjs');
  const { readFile } = await import('node:fs/promises');
  const doc = JSON.parse(
    await readFile(new URL('./fixtures/release-market-synthetic.json', import.meta.url), 'utf8'),
  );
  assert.match(doc.STATUS, /^SYNTHETIC/);
  // Committed defaults: account cap 1 leaves the release no slot, and the kill switch is off.
  const off = await offlinePlan(doc.offers);
  assert.deepEqual([off.parallel, off.admitted], [0, false]);
  assert.deepEqual(off.refusals, ['NO_SLOTS', 'RELEASE_DISABLED']);
  const on = { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '2' };
  const plan = await offlinePlan(doc.offers, on);
  assert.deepEqual([plan.legs, plan.parallel, plan.admitted], [12, 1, true]);
  const sum = plan.families.reduce((s, f) => s + f.est_usd, 0);
  assert.ok(Math.abs(sum - plan.estimate_usd) < 1e-3);
  const eight = await offlinePlan(doc.offers, { ...on, MAX_CONCURRENT_RENTALS: '9' });
  assert.equal(eight.parallel, 8);
  const capped = await offlinePlan(doc.offers, { ...on, RELEASE_COST_CAP_USD: '10' });
  assert.deepEqual(capped.refusals, ['COST_CAP']);
  const poor = await offlinePlan(doc.offers, on, { credit: 20 });
  const account = await offlinePlan(doc.offers, { ...on, ACCOUNT_DAILY_USD: '10' });
  assert.deepEqual(account.refusals, ['ACCOUNT_DAILY_USD']);
  assert.deepEqual(poor.refusals, ['CREDIT_RESERVE']);
});
test('the deploy helper accepts the tier variables and the configuration reads them', async () => {
  const { parseVars } = await import('../deploy.mjs');
  const text = [
    'OWNER_GITHUB_ID=' + env.OWNER_GITHUB_ID,
    'MAX_CONCURRENT_RENTALS=9',
    'MAX_CONCURRENT_CHECK=1',
    'MAX_CONCURRENT_SWEEP=1',
    'MAX_CONCURRENT_RELEASE=8',
    'RELEASE_ENABLED=true',
    'ACCOUNT_DAILY_USD=50',
    'RELEASE_MAX_PRICE_USD_HOUR=1.5',
    'RELEASE_DAILY_USD=40',
    'RELEASE_DAILY_RENTALS=24',
    'RELEASE_COST_CAP_USD=30',
    'RELEASE_HARD_RUNTIME_MINUTES=300',
    'RELEASE_RERUNS_PER_LEG=1',
    'RELEASE_STAGES=[]',
    'RELEASE_DIES=' + JSON.stringify(DEFAULTS.releaseDies),
    'MIDDLE_ROTATION=' + JSON.stringify(DEFAULTS.middleRotation),
    'MIDDLE_VRAM_FLOOR_MB=0',
  ].join('\n');
  const vars = Object.fromEntries(parseVars(text));
  const cfg = cfgOf(vars);
  assert.deepEqual(cfg.maxConcurrentProfile, { check: 1, sweep: 1, release: 8 });
  assert.deepEqual([cfg.releasePrice, cfg.releaseCostCap], [1.5, 30]);
});

test('classify (review F3): the box must report the planned die and a known driver', () => {
  const pass = { ...LINUX, windows_gate: 1 };
  assert.equal(classify(rec('PASS', pass)).verdict, 'PASS');
  for (const [hardware, why] of [
    [{ ...AD104, gpu_name: 'NVIDIA GeForce RTX 4090' }, 'another die of the family'],
    [{ ...AD104, gpu_name: 'NVIDIA GeForce RTX 3060', compute_cap: 860 }, 'another family'],
    [{ ...AD104, compute_cap: 860 }, 'compute capability of another family'],
    [{ ...AD104, driver_version: 'unknown' }, 'unknown driver'],
    [undefined, 'no hardware'],
  ]) {
    const out = classify({ ...rec('PASS', pass), hardware });
    assert.deepEqual([out.verdict, out.reason], ['RETRY', 'HARDWARE_MISMATCH'], why);
  }
  // Without the planned die no box can pass.
  assert.equal(classify(rec('PASS', pass), { die: undefined }).verdict, 'RETRY');
  assert.equal(classify(rec('PASS', pass), { family: 'turing' }).verdict, 'RETRY');
  // A failing box of another GPU is not this die's FAIL; a failing box without hardware keeps
  // the ordinary attribution (a provisioning failure reports no hardware).
  const wrong = classify({
    ...rec('FAIL', { ...LINUX, windows_gate: -1 }),
    hardware: { ...AD104, gpu_name: 'NVIDIA GeForce RTX 4090' },
  });
  assert.deepEqual([wrong.verdict, wrong.reason], ['RETRY', 'HARDWARE_MISMATCH']);
  const blind = classify({
    ...rec('FAIL', { provision: -1 }, { green: false }),
    hardware: undefined,
  });
  assert.equal(blind.verdict, 'UNATTRIBUTED');
});
