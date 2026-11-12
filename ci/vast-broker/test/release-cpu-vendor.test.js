import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  matrixTest,
  proposedMatrix,
  capablePayload,
  receipts,
  subrunSuites,
  withoutVendorRule,
} from './release-fixture.js';
import { RELEASE_MATRIX, validateReleaseMatrix, vendorOfCpuName } from '../src/release-matrix.js';
import { planRelease, releaseVerdict, cpuVendorCoverage } from '../src/release.js';
import { configuration } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { tierEnvironment } from '../src/tier.js';
import { setup, env, sha } from './helpers.js';

// Matrix v2 (owner 2026-11-12, OWNER-CONFIRMED release shape): per family the flagship die is the LARGE run and the
// second die the MIDDLE profile; across ALL large boxes at least one Intel and one AMD host CPU, everything else any x86 CPU.
// FALSIFIERS (written before the code): a plan with a vendor missing is admitted; a vendor swap breaks the other vendor or
// takes the dearer offer; a middle die is planned as a large run (SPEC lanes, two Windows runs, the 300-minute price); the
// judge passes a release whose large legs are all one vendor, whose vendor claim disagrees with the offer, or whose only
// second vendor is a middle box; an ARM host is planned.
const AMD = 'AMD EPYC 7763 64-Core Processor';
const INTEL = 'Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz';
const LARGE = ['TU106', 'GA102', 'AD102', 'GB202'];
const MIDDLE = ['TU116', 'GA104', 'AD104', 'GB205'];
const cfgOf = (extra = {}) =>
  configuration({
    ...env,
    RELEASE_ENABLED: 'true',
    MAX_CONCURRENT_RENTALS: '9',
    MIDDLE_APPS_ENABLED: undefined,
    ...extra,
  });
const hours = (cfg) => (cfg.hardRelease + cfg.cronPeriod) / 3600000;
let nextId = 1;
function offer(cfg, die, { cpu = AMD, price = 0.2, id = nextId++ } = {}) {
  const cc = cfg.dieRules.find((r) => r.die === die).cc;
  return {
    id,
    die,
    machine_id: 1000 + id,
    host_id: 5000 + id,
    gpu_name: 'GPU ' + die,
    cpu_name: cpu,
    compute_cap: cc,
    country: 'US',
    dph_total: price,
    pricing: { computeStorage: price, bandwidth: 0, effective: price, total: price * hours(cfg) },
  };
}
// One cheap offer per die, all AMD; `extra` adds more offers.
function market(cfg, extra = [], base = AMD) {
  nextId = 1;
  return [...LARGE, ...MIDDLE].map((die) => offer(cfg, die, { cpu: base })).concat(extra);
}
const largeLegs = (plan) =>
  plan.families.flatMap((f) =>
    f.cells.filter((c) => c.leg_tier === 'large').flatMap((c) => c.legs),
  );

test('vendorOfCpuName: AMD, Intel, ARM and unknown are classified; nothing is guessed', () => {
  for (const [name, vendor] of [
    [AMD, 'amd'],
    ['AMD Ryzen 9 7950X 16-Core Processor', 'amd'],
    [INTEL, 'intel'],
    ['Intel(R) Core(TM) i9-13900K', 'intel'],
    ['Neoverse-N1', 'arm'],
    ['Ampere Altra Q80-30', 'arm'],
    ['Some Future CPU', 'other'],
    ['', 'other'],
    [undefined, 'other'],
    [{ amd: 1 }, 'other'],
    ['x'.repeat(200) + ' AMD', 'other'],
  ])
    assert.equal(vendorOfCpuName(name), vendor, String(name));
});

test('matrix: the second dies run the middle profile, the flagships the full profile, the vendor rule is set', () => {
  assert.deepEqual(proposedMatrix.policy.cpu_vendor_min, { amd: 1, intel: 1 });
  assert.equal(proposedMatrix.policy.cpu_vendor_scope, 'full');
  assert.equal(proposedMatrix.policy.second_die_profile, 'middle');
  assert.equal(proposedMatrix.policy.full_profile_dies_per_family, 1);
  assert.equal(proposedMatrix.status, 'proposed');
  assert.ok(
    proposedMatrix.dies.every((d) => d.blockers.length > 0),
    'blockers are never removed',
  );
  for (const row of proposedMatrix.dies)
    assert.equal(row.profile, row.role === 'flagship' ? 'full' : 'middle', row.die);
  assert.deepEqual(validateReleaseMatrix(proposedMatrix, DEFAULTS), []);
});
for (const [name, mutate, error] of [
  ['flagship on the middle profile', (m) => (m.dies[0].profile = 'middle'), 'profile'],
  ['second die on the full profile', (m) => (m.dies[1].profile = 'full'), 'profile'],
  ['vendor rule absent', (m) => delete m.policy.cpu_vendor_min, 'policy schema'],
  [
    'vendor rule names another vendor',
    (m) => (m.policy.cpu_vendor_min = { amd: 1, via: 1 }),
    'policy schema',
  ],
  ['vendor rule with an extra vendor', (m) => (m.policy.cpu_vendor_min.via = 1), 'policy schema'],
  [
    'vendor rule with an extra zero vendor',
    (m) => (m.policy.cpu_vendor_min.arm = 0),
    'policy schema',
  ],
  ['vendor rule negative', (m) => (m.policy.cpu_vendor_min.intel = -1), 'policy schema'],
  ['vendor rule beyond the large legs', (m) => (m.policy.cpu_vendor_min.amd = 99), 'policy schema'],
  [
    'vendor rule scoped to another profile',
    (m) => (m.policy.cpu_vendor_scope = 'middle'),
    'policy schema',
  ],
  ['two full dies per family', (m) => (m.policy.full_profile_dies_per_family = 2), 'policy schema'],
  ['second die profile changed', (m) => (m.policy.second_die_profile = 'small'), 'policy schema'],
])
  test('matrix: rejects ' + name, () => {
    const m = structuredClone(proposedMatrix);
    mutate(m);
    assert.ok(
      validateReleaseMatrix(m, DEFAULTS).some((e) => e.includes(error)),
      name,
    );
  });

test('plan: second dies are single middle legs, no SPEC lanes, priced at the middle hard limit', () => {
  const cfg = cfgOf({ RELEASE_STAGES: '["specviewperf"]' });
  const plan = planRelease(market(cfg, [offer(cfg, 'AD102', { cpu: INTEL, price: 0.4 })]), cfg);
  for (const f of plan.families)
    for (const c of f.cells) {
      if (LARGE.includes(c.die)) {
        assert.equal(c.leg_tier, 'large');
        assert.deepEqual(
          c.legs.map((l) => l.spec),
          ['lanea', 'laneb'],
          c.die,
        );
        assert.ok(c.legs.every((l) => l.windows_runs === 1 && l.hard_minutes === undefined));
      } else {
        assert.equal(c.leg_tier, 'middle');
        assert.equal(c.legs.length, 1, c.die);
        assert.equal(c.legs[0].spec, undefined);
        assert.equal(c.legs[0].windows_runs, 1);
        assert.equal(c.legs[0].profile, 'middle');
        assert.equal(c.legs[0].hard_minutes, cfg.releaseMiddleHard / 60000);
        assert.ok(
          Math.abs(c.legs[0].est_usd - (0.2 * (cfg.releaseMiddleHard + cfg.cronPeriod)) / 3600000) <
            1e-3,
        );
      }
    }
  assert.deepEqual([plan.large_legs, plan.middle_legs, plan.legs], [8, 4, 12]);
});
test('plan: the middle estimate follows RELEASE_MIDDLE_HARD_RUNTIME_MINUTES and never the large price', () => {
  const at = (minutes) => {
    const cfg = cfgOf({ RELEASE_MIDDLE_HARD_RUNTIME_MINUTES: String(minutes) });
    const plan = planRelease(market(cfg, [offer(cfg, 'AD102', { cpu: INTEL })]), cfg);
    return plan.families.flatMap((f) => f.cells).filter((c) => c.leg_tier === 'middle');
  };
  const [short, long] = [at(120), at(240)];
  assert.ok(short.every((c, i) => c.est_usd < long[i].est_usd));
  assert.ok(Math.abs(short[0].est_usd - (0.2 * (120 + 1)) / 60) < 1e-3);
  assert.throws(() => cfgOf({ RELEASE_MIDDLE_HARD_RUNTIME_MINUTES: '301' }));
  assert.throws(() => cfgOf({ RELEASE_MIDDLE_HARD_RUNTIME_MINUTES: '5' }));
  assert.equal(cfgOf().releaseMiddleHard / 60000, DEFAULTS.releaseMiddleHardMinutes);
});

test('plan: an all-AMD cheapest set swaps the cheapest Intel offer in; AMD stays covered', () => {
  const cfg = cfgOf();
  // Intel exists for two large dies; GA102 is the cheaper swap (delta 0.05 against 0.2).
  const plan = planRelease(
    market(cfg, [
      offer(cfg, 'GA102', { cpu: INTEL, price: 0.25 }),
      offer(cfg, 'AD102', { cpu: INTEL, price: 0.4 }),
    ]),
    cfg,
  );
  assert.deepEqual(plan.cpu_vendor.missing, []);
  assert.deepEqual(plan.cpu_vendor.before_swaps, { amd: 4, intel: 0, other: 0, arm: 0 });
  assert.deepEqual(plan.cpu_vendor.planned, { amd: 3, intel: 1, other: 0, arm: 0 });
  assert.equal(plan.cpu_vendor.swaps.length, 1);
  assert.equal(plan.cpu_vendor.swaps[0].die, 'GA102');
  assert.equal(plan.cpu_vendor.swaps[0].to_vendor, 'intel');
  assert.ok(Math.abs(plan.cpu_vendor.swaps[0].delta_usd - 0.05 * hours(cfg)) < 1e-3);
  assert.ok(!plan.refusals.includes('CPU_VENDOR_MISSING'));
  // Same die, same constraints: the swapped leg is still a GA102 offer of the matrix.
  const ga = plan.families.find((f) => f.family === 'ampere').cells[0];
  assert.equal(ga.legs[0].gpu_name, 'GPU_GA102');
  assert.equal(ga.legs[0].cpu_vendor, 'intel');
  // The covering legs are pinned to their vendor for the rent step.
  assert.deepEqual(
    largeLegs(plan)
      .filter((l) => l.pin_cpu_vendor)
      .map((l) => l.pin_cpu_vendor)
      .sort(),
    ['amd', 'intel'],
  );
});
test('plan: an all-Intel cheapest set swaps an AMD offer in and keeps Intel covered', () => {
  const cfg = cfgOf();
  // Two AMD candidates: GB202 (delta 0.1) is cheaper than AD102 (delta 0.3).
  const plan = planRelease(
    market(
      cfg,
      [
        offer(cfg, 'GB202', { cpu: AMD, price: 0.3 }),
        offer(cfg, 'AD102', { cpu: AMD, price: 0.5 }),
      ],
      INTEL,
    ),
    cfg,
  );
  assert.deepEqual(plan.cpu_vendor.planned, { amd: 1, intel: 3, other: 0, arm: 0 });
  assert.equal(plan.cpu_vendor.swaps[0].die, 'GB202');
  assert.equal(plan.cpu_vendor.swaps[0].to_vendor, 'amd');
});
// The other three large dies offer only unknown CPUs, which count for neither vendor and are not swappable (no offer).
for (const [lone, other, name] of [
  [AMD, INTEL, 'AMD'],
  [INTEL, AMD, 'Intel'],
])
  test(`plan: a swap never trades the lone ${name} leg for the other vendor`, () => {
    const cfg = cfgOf();
    nextId = 1;
    const rows = [
      offer(cfg, 'TU106', { cpu: lone }),
      ...['GA102', 'AD102', 'GB202'].map((die) => offer(cfg, die, { cpu: 'Mystery CPU' })),
      ...MIDDLE.map((die) => offer(cfg, die)),
      offer(cfg, 'TU106', { cpu: other, price: 0.21 }),
    ];
    const plan = planRelease(rows, cfg);
    assert.equal(plan.cpu_vendor.swaps.length, 0);
    assert.deepEqual(plan.cpu_vendor.missing, [name === 'AMD' ? 'intel' : 'amd']);
    assert.equal(plan.cpu_vendor.planned[name.toLowerCase()], 1);
    assert.equal(plan.admitted, false);
  });
test('plan: Intel on every middle die still leaves the large legs without Intel', () => {
  const cfg = cfgOf();
  nextId = 1;
  const rows = [
    ...LARGE.map((d) => offer(cfg, d)),
    ...MIDDLE.map((d) => offer(cfg, d, { cpu: INTEL })),
  ];
  const plan = planRelease(rows, cfg);
  assert.deepEqual(plan.cpu_vendor.missing, ['intel']);
  assert.equal(plan.admitted, false);
});
test('plan: both vendors already cheapest means no swap and no extra cost', () => {
  const cfg = cfgOf();
  nextId = 1;
  const rows = [...LARGE, ...MIDDLE].map((die, i) => offer(cfg, die, { cpu: i % 2 ? INTEL : AMD }));
  const plan = planRelease(rows, cfg);
  assert.equal(plan.cpu_vendor.swaps.length, 0);
  assert.deepEqual(plan.cpu_vendor.missing, []);
});
test('plan: no admissible other-vendor offer makes the plan INCOMPLETE and never admitted', () => {
  const cfg = cfgOf({ RELEASE_COST_CAP_USD: '40' });
  const plan = planRelease(market(cfg), cfg);
  assert.deepEqual(plan.cpu_vendor.missing, ['intel']);
  assert.ok(plan.refusals.includes('CPU_VENDOR_MISSING'));
  assert.ok(plan.refusals.includes('INCOMPLETE'));
  assert.equal(plan.admitted, false);
});
test('plan: an Intel offer on a MIDDLE die never satisfies the rule', () => {
  const cfg = cfgOf();
  const plan = planRelease(market(cfg, [offer(cfg, 'TU116', { cpu: INTEL, price: 0.1 })]), cfg);
  assert.deepEqual(plan.cpu_vendor.missing, ['intel']);
  assert.equal(plan.cpu_vendor.swaps.length, 0);
  assert.equal(plan.admitted, false);
});
test('plan: ARM hosts are never planned', () => {
  const cfg = cfgOf();
  const armOffer = offer(cfg, 'GA102', { cpu: 'Neoverse-N1', price: 0.01, id: 900 });
  const arm = planRelease(market(cfg, [armOffer, offer(cfg, 'AD102', { cpu: INTEL })]), cfg);
  // the cheapest GA102 offer is the ARM one: it is skipped, not merely classified
  assert.ok(
    arm.families
      .flatMap((f) => f.cells.flatMap((c) => c.legs))
      .every((l) => l.offer_id !== armOffer.id),
  );
  assert.ok(largeLegs(arm).every((l) => l.cpu_vendor === 'amd' || l.cpu_vendor === 'intel'));
  assert.deepEqual(arm.cpu_vendor.missing, []);
});
// Owner review of #361: an empty or unfamiliar cpu_name is not rentable for a release leg (it could never satisfy the vendor rule).
for (const [name, cpu] of [
  ['unfamiliar', 'Mystery CPU'],
  ['empty', ''],
  ['missing', undefined],
]) {
  test(`plan: a ${name} cpu_name is never planned`, () => {
    const cfg = cfgOf();
    const odd = offer(cfg, 'AD102', { cpu: cpu ?? '', price: 0.01, id: 901 });
    // Intel is offered (at the same price) on another large die, so the cover needs no swap on AD102 itself.
    const intel = offer(cfg, 'GA102', { cpu: INTEL, price: 0.2, id: 902 });
    const plan = planRelease(market(cfg, [odd, intel]), cfg);
    assert.ok(
      plan.families
        .flatMap((f) => f.cells.flatMap((c) => c.legs))
        .every((l) => l.offer_id !== odd.id),
    );
    assert.deepEqual(plan.cpu_vendor.planned.other, 0);
  });
}
test('plan: with SPEC the two lanes of the swapped die stay on different machines when two are offered', () => {
  const cfg = cfgOf({ RELEASE_STAGES: '["specviewperf"]' });
  const rows = market(cfg, [
    offer(cfg, 'AD102', { cpu: AMD, price: 0.3 }),
    offer(cfg, 'AD102', { cpu: INTEL, price: 0.5 }),
  ]);
  const plan = planRelease(rows, cfg);
  assert.deepEqual(plan.cpu_vendor.missing, []);
  const ada = plan.families.find((f) => f.family === 'ada').cells[0];
  assert.equal(new Set(ada.legs.map((l) => l.machine_id)).size, 2);
  assert.ok(ada.legs.some((l) => l.cpu_vendor === 'intel'));
});
test('plan: the vendor rule is read from the matrix policy (zero minimum = no refusal)', () => {
  const cfg = cfgOf();
  cfg.releaseMatrix = withoutVendorRule();
  const plan = planRelease(market(cfg), cfg);
  assert.ok(!plan.refusals.includes('CPU_VENDOR_MISSING'));
  assert.equal(plan.cpu_vendor.swaps.length, 0);
});

// ---- the judge ----
const cell = (
  family,
  die,
  tier,
  vendor,
  { reported = vendor, offered = vendor, verdict = 'PASS' } = {},
) => ({
  key: die + '-' + (vendor ?? 'x'),
  family,
  die,
  state: 'ENDED',
  machineId: 1,
  ...(tier === 'middle' ? { legTier: 'middle' } : {}),
  ...(offered ? { cpuVendorOffered: offered } : {}),
  ...(reported ? { cpuVendorReported: reported } : {}),
  releaseMatrix: RELEASE_MATRIX.dies.find((r) => r.die === die),
  outcome: {
    verdict,
    linux: verdict,
    windows: tier === 'middle' ? [verdict] : [verdict, verdict],
    stages: {},
    matrix: Object.fromEntries(
      receipts(RELEASE_MATRIX.dies.find((r) => r.die === die)).rows.map((r) => [r.id, verdict]),
    ),
  },
});
function judgedBatch(vendors, legsOverride) {
  const legs = [];
  const families = {
    turing: ['TU106', 'TU116'],
    ampere: ['GA102', 'GA104'],
    ada: ['AD102', 'AD104'],
    blackwell: ['GB202', 'GB205'],
  };
  for (const [i, [family, [large, middle]]] of Object.entries(families).entries()) {
    legs.push(cell(family, large, 'large', vendors[i]));
    legs.push(cell(family, middle, 'middle', 'amd'));
  }
  return {
    tier: 'large',
    windowsRunsPerDie: 2,
    diesPerFamily: 2,
    releaseDies: DEFAULTS.releaseDies,
    stages: [],
    legs: legsOverride ? legsOverride(legs) : legs,
  };
}
matrixTest('judge: PASS needs one confirmed Intel and one confirmed AMD large leg', () => {
  const ok = releaseVerdict(judgedBatch(['amd', 'intel', 'amd', 'amd']));
  assert.equal(ok.verdict, 'PASS');
  assert.deepEqual(ok.cpu_vendor.confirmed, { amd: 3, intel: 1 });
  assert.deepEqual(ok.cpu_vendor.missing, []);
});
matrixTest(
  'judge: every large leg one vendor is INCOMPLETE (CPU_VENDOR_MISSING), never PASS',
  () => {
    for (const [vendors, missing] of [
      [['amd', 'amd', 'amd', 'amd'], ['intel']],
      [['intel', 'intel', 'intel', 'intel'], ['amd']],
    ]) {
      const view = releaseVerdict(judgedBatch(vendors));
      assert.equal(view.verdict, 'INCOMPLETE');
      assert.equal(view.reason, 'CPU_VENDOR_MISSING');
      assert.deepEqual(view.cpu_vendor.missing, missing);
      assert.ok(
        view.cells.every((c) => c.verdict === 'PASS'),
        'the cells themselves pass',
      );
    }
  },
);
matrixTest('judge: a middle box of the missing vendor does not count', () => {
  const view = releaseVerdict(
    judgedBatch(['amd', 'amd', 'amd', 'amd'], (legs) =>
      legs.map((l) =>
        l.legTier === 'middle'
          ? { ...l, cpuVendorOffered: 'intel', cpuVendorReported: 'intel' }
          : l,
      ),
    ),
  );
  assert.equal(view.verdict, 'INCOMPLETE');
  assert.deepEqual(view.cpu_vendor.confirmed, { amd: 4, intel: 0 });
});
matrixTest(
  'judge: the box report is cross-checked with the offer; a mismatch or a missing report counts for nothing',
  () => {
    const mismatch = releaseVerdict(
      judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
        legs.map((l) => (l.die === 'GA102' ? { ...l, cpuVendorReported: 'amd' } : l)),
      ),
    );
    assert.equal(mismatch.verdict, 'INCOMPLETE');
    assert.deepEqual(mismatch.cpu_vendor.mismatched, ['GA102-intel']);
    const unreported = releaseVerdict(
      judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
        legs.map((l) => (l.die === 'GA102' ? { ...l, cpuVendorReported: undefined } : l)),
      ),
    );
    assert.equal(unreported.verdict, 'INCOMPLETE');
    assert.deepEqual(unreported.cpu_vendor.unreported, ['GA102-intel']);
    const unknownOffer = releaseVerdict(
      judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
        legs.map((l) => (l.die === 'GA102' ? { ...l, cpuVendorOffered: undefined } : l)),
      ),
    );
    assert.equal(unknownOffer.verdict, 'INCOMPLETE');
  },
);
matrixTest('judge: a failing leg is FAIL whatever the vendors', () => {
  const failed = releaseVerdict(
    judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
      legs.map((l) =>
        l.die === 'GA102' ? cell(l.family, l.die, 'large', 'intel', { verdict: 'FAIL' }) : l,
      ),
    ),
  );
  assert.equal(failed.verdict, 'FAIL');
});
for (const verdict of ['RETRY', 'UNATTRIBUTED'])
  matrixTest('judge: a ' + verdict + ' large leg does not count for its vendor', () => {
    const batch = judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
      legs.map((l) => (l.die === 'GA102' ? { ...l, outcome: { ...l.outcome, verdict } } : l)),
    );
    assert.deepEqual(cpuVendorCoverage(batch).confirmed, { amd: 3, intel: 0 });
    assert.notEqual(releaseVerdict(batch).verdict, 'PASS');
  });
// The die is a second LARGE-looking die (two Windows passes, so its cells pass): only the family shape can refuse it.
for (const die of ['TU116', 'GA104'])
  matrixTest('judge: a family needs a large AND a middle die ' + die, () => {
    const view = releaseVerdict(
      judgedBatch(['amd', 'intel', 'amd', 'amd'], (legs) =>
        legs.map((l) => {
          if (l.die !== die) return l;
          const { legTier, ...large } = l;
          return { ...large, outcome: { ...l.outcome, windows: ['PASS', 'PASS'] } };
        }),
      ),
    );
    assert.ok(view.cells.every((c) => c.verdict === 'PASS'));
    assert.equal(view.verdict, 'INCOMPLETE');
  });
matrixTest('judge: a middle die needs one Windows pass only', () => {
  const view = releaseVerdict(judgedBatch(['amd', 'intel', 'amd', 'amd']));
  assert.equal(view.cells.find((c) => c.die === 'TU116' && c.platform === 'windows').required, 1);
  assert.equal(view.cells.find((c) => c.die === 'TU106' && c.platform === 'windows').required, 2);
});
matrixTest('judge: a batch of a matrix without the rule requires no vendor', () => {
  const view = releaseVerdict(judgedBatch(['amd', 'amd', 'amd', 'amd']), {
    releaseMatrix: withoutVendorRule(),
  });
  assert.equal(view.verdict, 'PASS');
});

// ---- the rental path ----
test('environment: a release middle leg is a middle-tier box with one Windows run, the apps and no stages', () => {
  const e = tierEnvironment('release', {
    tierName: 'middle',
    windowsRuns: 1,
    linuxApps: true,
    winApps: 'mid',
  });
  assert.equal(e.KF_TIER, 'middle');
  assert.equal(e.KF_LINUX_APPS, '1');
  assert.equal(e.KF_WINAPPS, 'mid');
  assert.equal(e.KF_WINDOWS_RUNS, undefined);
  assert.equal(e.KF_STAGES, undefined);
  assert.throws(
    () => tierEnvironment('sweep', { tierName: 'middle', linuxApps: true }),
    /linuxapps/,
  );
});

const synthetic = () =>
  JSON.parse(readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)))
    .offers;
async function harness(offers, extra = {}) {
  const h = await setup({
    MAX_CONCURRENT_RENTALS: '9',
    RELEASE_ENABLED: 'true',
    HARD_RUNTIME_MINUTES: '300',
    BOOT_LIMIT_MINUTES: '75',
    ROLLING_SPEND_USD: '200',
    DAILY_SPEND_USD: '40',
    ACCOUNT_DAILY_USD: '100',
    MIDDLE_APPS_ENABLED: undefined,
    ...extra,
  });
  h.cloud.credit = 500;
  capablePayload(h);
  h.cloud.extraOffers = offers.map((o) => ({ ...o, price: o.dph_total }));
  h.cloud.contracts = Array.from({ length: 40 }, (_, i) => 900 + i);
  return h;
}
async function finish(h, leg, { claim } = {}) {
  const record = await h.storage.get('record:' + leg.key);
  const row = leg.releaseMatrix;
  const offer = [h.cloud.offer, ...h.cloud.extraOffers].find(
    (o) => o.machine_id === record.machineId,
  );
  const suites = [
    ...['cuda_fat', 'windows_gate', 'windows_gate_2']
      .filter((n) => leg.windowsRuns === 2 || n !== 'windows_gate_2')
      .map((name) => ({ name, pass: name === 'cuda_fat' ? 4 : 1, fail: 0, skip: 0 })),
    ...subrunSuites(row),
  ];
  if (leg.spec) suites.push({ name: 'specviewperf_' + leg.spec, pass: 1, fail: 0, skip: 0 });
  if (record.middleAppsRequested)
    suites.push(
      { name: 'apps', pass: 66, fail: 0, skip: 0 },
      { name: 'windows_apps', pass: 10, fail: 0, skip: 0 },
    );
  await h.report(
    {
      seq: 1,
      kind: 'result',
      result: {
        nonce: record.nonce,
        verdict: 'PASS',
        host_reference: 'GREEN',
        suites,
        matrix_receipts: receipts(row),
        host_claims: { cpu_vendor: claim ?? (/AMD/i.test(offer.cpu_name ?? '') ? 'amd' : 'intel') },
        hardware: {
          gpu_name: offer.gpu_name,
          compute_cap: offer.compute_cap,
          driver_version: row.drivers.host_driver,
        },
      },
    },
    { run: leg.key },
  );
}
// Every synthetic offer AMD except one pricey Intel 4090 and one ARM host cheaper than everything.
const vendorMarket = () => {
  const rows = synthetic().map((o) => ({ ...o, cpu_name: AMD }));
  const intel = rows.find((o) => o.id === 7008);
  intel.cpu_name = INTEL;
  rows.push({
    ...rows.find((o) => o.id === 7001),
    id: 7500,
    machine_id: 75000,
    host_id: 9500,
    cpu_name: 'Neoverse-N1',
    dph_total: 0.12,
    dph_base: 0.12 - 0.2 * (300 / 720),
  });
  return rows;
};
// What every rented leg must look like: vendor pin honoured, never ARM, the right hard limit and tier.
async function assertRented(h, batch) {
  // exactly the two covering legs carry a vendor pin into the batch
  assert.equal(batch.legs.filter((l) => l.cpuVendor).length, 2);
  for (const leg of batch.legs) {
    const record = await h.storage.get('record:' + leg.key);
    assert.notEqual(record.cpuVendorOffered, 'arm');
    if (leg.cpuVendor) assert.equal(record.cpuVendorOffered, leg.cpuVendor);
    if (leg.legTier === 'middle') {
      assert.equal(record.hardDeadline - record.started, 240 * 60000);
      const create = h.cloud.calls.find((c) => c.body?.label === record.label);
      assert.equal(create.body.env.KF_TIER, 'middle');
      assert.equal(create.body.env.KF_LINUX_APPS, '1');
      assert.equal(create.body.env.KF_STAGES, undefined);
    } else {
      assert.equal(record.hardDeadline - record.started, 300 * 60000);
      assert.equal(
        h.cloud.calls.find((c) => c.body?.label === record.label).body.env.KF_TIER,
        'large',
      );
    }
  }
}
matrixTest(
  'rent: the Intel-pinned leg rents only an Intel host; middle legs rent the middle limit; ARM is never rented',
  async () => {
    const h = await harness(vendorMarket());
    const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
    assert.deepEqual(plan.cpu_vendor.missing, []);
    assert.equal(plan.cpu_vendor.swaps.length, 1);
    assert.equal(plan.admitted, true, plan.refusals.join());
    assert.ok(
      plan.families
        .flatMap((f) => f.cells.flatMap((c) => c.legs))
        .every((l) => l.cpu_vendor !== 'arm'),
    );
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id);
    assert.deepEqual(
      batch.legs
        .filter((l) => l.cpuVendor)
        .map((l) => l.cpuVendor)
        .sort(),
      ['amd', 'intel'],
    );
    assert.equal(batch.legs.filter((l) => l.legTier === 'middle').length, 4);
    await assertRented(h, batch);
    // Everything passes with the right claims: the release PASSes with both vendors confirmed.
    for (const leg of batch.legs) await finish(h, leg);
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const done = await h.storage.get('batch:' + id);
    assert.equal(done.verdict, 'PASS');
  },
);
matrixTest(
  'rent: a box that reports another CPU vendor than the offer leaves the release INCOMPLETE',
  async () => {
    const h = await harness(vendorMarket());
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id);
    await assertRented(h, batch);
    for (const leg of batch.legs)
      await finish(h, leg, leg.cpuVendor === 'intel' ? { claim: 'amd' } : {});
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const done = await h.storage.get('batch:' + id);
    assert.equal(done.verdict, 'INCOMPLETE');
  },
);
matrixTest(
  'start: an all-AMD market is refused at plan time with CPU_VENDOR_MISSING and rents nothing',
  async () => {
    const h = await harness(synthetic().map((o) => ({ ...o, cpu_name: AMD })));
    const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
    assert.ok(plan.refusals.includes('CPU_VENDOR_MISSING'));
    assert.ok(plan.refusals.includes('INCOMPLETE'));
    await assert.rejects(
      () => h.service.adminStart({ commit_sha: sha, tier: 'large' }),
      (e) => e.reason === 'PLAN_REFUSED',
    );
    assert.equal(h.cloud.creates, 0);
  },
);
matrixTest(
  'rent: the box CPU report is kept on every ended leg next to the offered vendor',
  async () => {
    const h = await harness(vendorMarket());
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id);
    for (const leg of batch.legs) await finish(h, leg);
    h.time.value += 11 * 60000;
    await h.service.sweep();
    const done = await h.storage.get('batch:' + id);
    for (const leg of done.legs) {
      assert.ok(['amd', 'intel'].includes(leg.cpuVendorReported), leg.key);
      assert.equal(leg.cpuVendorReported, leg.cpuVendorOffered);
    }
  },
);
matrixTest(
  'rent: a host with an empty cpu_name is never rented, even when it is the cheapest',
  async () => {
    const rows = vendorMarket();
    rows.push({
      ...rows.find((o) => o.id === 7001),
      id: 7600,
      machine_id: 76000,
      host_id: 9600,
      cpu_name: '',
      dph_total: 0.12,
      dph_base: 0.12 - 0.2 * (300 / 720),
    });
    const h = await harness(rows);
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const batch = await h.storage.get('batch:' + id);
    for (const leg of batch.legs) {
      const record = await h.storage.get('record:' + leg.key);
      assert.notEqual(record.machineId, 76000);
      assert.ok(['amd', 'intel'].includes(record.cpuVendorOffered));
    }
  },
);
