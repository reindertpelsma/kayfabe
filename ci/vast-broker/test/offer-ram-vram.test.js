import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import * as policy from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { DEFAULTS } from '../src/defaults.js';
import { legStages, plannedStages } from '../src/bandwidth.js';
import { offlinePlan } from '../release-plan.mjs';
import { env, setup, MemoryStorage, MockCloud, rentBody, sha } from './helpers.js';
const { configuration, offerRejections } = policy;
// A Vast KVM box listed with an RTX 4090 of 49140 MiB and 125 GB of RAM (MemTotal 103.5 GB)
// made the kfml row `managed_oversub` NOT_RUN ("host RAM too small to oversubscribe VRAM by 25%"). The row wants
// 1.25 x VRAM <= 0.6 x MemAvailable, and a KVM box gets only about 0.75 to 0.83 of the LISTED RAM as MemTotal, so a
// flat RAM floor calibrated on 24 GB dies cannot hold. The offer policy derives the listed-RAM floor from the offer's
// own gpu_ram, and applies it to a LEG, not to a profile: only a leg whose planned stages include kfml AND whose tier
// runs that row (the large tier; the middle tier's kfml plan leaves it out) is subject. The stage list is the Worker's
// own (src/bandwidth.js legStages, which is plannedStages with the payload's --only/--skip rule).
//
// FALSIFIER (written before the code): (1) a 49140 MiB offer with 125 GB listed is admitted to a kfml leg; (2) a
// 24564 MiB offer with 64 GB listed is admitted to a kfml leg; (3) the rule is applied to a leg without kfml (--skip
// kfml, --only without kfml), to a middle, small or adhoc leg, or to an evaluation that names no leg (the whole
// release profile), in the offer policy, in the release plan or in the rent path; (4) the rule's numbers differ from
// the numbers the kfml workload uses (a retyped constant); (5) the refusal carries no named reason, or a reason the
// runner does not know; (6) an offer with no numeric gpu_ram or cpu_ram is admitted to a kfml leg; (7) the unit of
// gpu_ram or cpu_ram (MiB) is confused with decimal MB or GB.
const WORKLOAD = new URL(
  '../../../scripts/kfml/kfml/workloads/managed_oversub.py',
  import.meta.url,
);
const source = readFileSync(WORKLOAD, 'utf8');
const python = (name) => {
  const m = new RegExp('^' + name + '\\s*=\\s*([0-9]+(?:\\.[0-9]+)?)\\s*(?:#.*)?$', 'm').exec(
    source,
  );
  assert.ok(m, name + ' is a named module constant of managed_oversub.py');
  return Number(m[1]);
};
const MIB = 1024;
const cfgFor = (tier) =>
  applyProfile(
    configuration({
      ...env,
      HARD_RUNTIME_MINUTES: '300',
      BOOT_LIMIT_MINUTES: '75',
      MAX_PRICE_USD_HOUR: '2',
      MAX_CONCURRENT_RENTALS: '9',
      ...(tier === 'large' ? { MIDDLE_APPS_ENABLED: undefined } : {}),
    }),
    tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check',
    tier,
  );
const profileOf = (tier) => (tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check');
// A valid offer in every other respect (price fields consistent with the tier's disk), 4090 class.
const offer = (cfg, gpuRam, cpuRam, id = 1) => ({
  ...new MockCloud().offer,
  id,
  machine_id: 80000 + id,
  gpu_name: 'RTX_4090',
  compute_cap: 890,
  gpu_ram: gpuRam,
  cpu_ram: cpuRam,
  dph_base: 0.1,
  storage_cost: 0.12,
  dph_total: 0.1 + (0.12 * cfg.diskGb) / 720,
  disk_space: 400,
  inet_down_cost: 0.001,
  inet_up_cost: 0.001,
});
const RULE = 'RAM_BELOW_VRAM_RULE';
// A leg is { tier, stages }: the tier it runs and the stage names it will run (src/bandwidth.js legStages).
const legOf = (tier, options) => ({ tier, stages: legStages(tier, options) });
const reasons = (leg, gpuRam, cpuRam, tier = leg?.tier ?? 'large') => {
  const cfg = cfgFor(tier);
  return offerRejections(offer(cfg, gpuRam, cpuRam), cfg, { profile: profileOf(tier), leg });
};
const KFML = legOf('large');
const D24 = 24564, // MiB, an RTX 4090 of 24 GB
  D48 = 49140; // MiB, the 48 GB variant of the same die

test('constants tie: the policy uses the numbers of the kfml workload, not retyped ones', () => {
  assert.equal(policy.KFML_OVERSUB_FACTOR, python('OVERSUB_FACTOR'));
  assert.equal(policy.KFML_OVERSUB_HOST_FRACTION, python('OVERSUB_HOST_RAM_FRACTION'));
});
test('constants tie: the workload applies its named constants', () => {
  assert.match(source, /int\(total \* OVERSUB_FACTOR\)/);
  assert.match(source, /_host_avail_gb\(\) \* OVERSUB_HOST_RAM_FRACTION/);
});
test('constants tie: the workload holds no second copy of the numbers', () => {
  assert.equal(source.match(/\b1\.25\b/g).length, 1, 'the factor is written once');
  assert.equal(source.match(/\b0\.6\b/g).length, 1, 'the host fraction is written once');
});
test('constants tie: the derived need agrees with the workload arithmetic for both dies', () => {
  for (const gpu of [D24, D48]) {
    const wantGiB = (python('OVERSUB_FACTOR') * gpu) / MIB; // VRAM x 1.25, in GiB
    const availGiB = wantGiB / python('OVERSUB_HOST_RAM_FRACTION'); // MemAvailable the row needs
    const listedMb = availGiB / policy.KVM_RAM_FRACTION / (1 / MIB);
    assert.ok(Math.abs(policy.listedRamNeededMb(gpu) - listedMb) < 1e-6, String(gpu));
  }
});
test('constants tie: the rule covers the tiers whose kfml run includes managed_oversub', () => {
  const tier = /tier="(\w+)"/.exec(source)?.[1];
  assert.equal(tier, 'large', 'managed_oversub is a large-tier kfml row');
  assert.deepEqual(policy.KFML_OVERSUB_TIERS, ['large']);
  for (const name of policy.KFML_OVERSUB_TIERS)
    assert.ok(DEFAULTS.payloadStageResources.kfml.profiles.includes(name), name);
});
test('the KVM fraction is the conservative end of the measured 0.75 to 0.83 (2026-11-20)', () => {
  assert.equal(policy.KVM_RAM_FRACTION, 0.75);
});
test('the derived floor: 24564 MiB needs 68234 MiB listed, 49140 MiB needs 136500 MiB listed', () => {
  assert.equal(Math.ceil(policy.listedRamNeededMb(D24)), 68234);
  assert.ok(Math.abs(policy.listedRamNeededMb(D48) - 136500) < 1e-6);
});

test('kfml leg, 24 GB die: 96 GB listed is admitted, the 64 GB listing of the measured box class is refused', () => {
  assert.ok(!reasons(KFML, D24, 96 * MIB).includes(RULE));
  assert.ok(reasons(KFML, D24, 64361).includes(RULE));
  assert.deepEqual(reasons(KFML, D24, 96 * MIB), []);
});
test('kfml leg, 48 GB die: the measured 125 GB listing and 96 GB are refused with the named reason, 134 GB is admitted', () => {
  assert.ok(reasons(KFML, D48, 128928).includes(RULE)); // the measured box: 125 GB listed
  assert.ok(reasons(KFML, D48, 96 * MIB).includes(RULE));
  assert.ok(!reasons(KFML, D48, 134 * MIB).includes(RULE));
  assert.deepEqual(reasons(KFML, D48, 134 * MIB), []);
});
test('boundary: one MiB below the derived floor is refused, the floor itself is admitted', () => {
  assert.ok(reasons(KFML, D48, 136499).includes(RULE));
  assert.ok(!reasons(KFML, D48, 136500).includes(RULE));
  assert.ok(reasons(KFML, D24, 68233).includes(RULE));
  assert.ok(!reasons(KFML, D24, 68234).includes(RULE));
});
test('units: gpu_ram and cpu_ram are both MiB (a 12 GB die with 64 GB listed is admitted, 24 GB with 48 GB is not)', () => {
  assert.ok(!reasons(KFML, 12288, 65536).includes(RULE));
  assert.ok(reasons(KFML, 24576, 48 * MIB).includes(RULE));
  assert.ok(!reasons(KFML, 24576, 72 * MIB).includes(RULE));
});
test('the refusal is the named reason alone: the flat CPU_RAM floor is not what refused it', () => {
  const r = reasons(KFML, D48, 128928);
  assert.ok(!r.includes('CPU_RAM'));
  assert.deepEqual(r, [RULE]);
});
test('fail closed: a kfml leg offer without a numeric gpu_ram or cpu_ram is refused with the named reason', () => {
  for (const bad of [undefined, null, '24564', NaN, Infinity]) {
    assert.ok(reasons(KFML, bad, 256 * MIB).includes(RULE), 'gpu_ram ' + String(bad));
    assert.ok(reasons(KFML, D24, bad).includes(RULE), 'cpu_ram ' + String(bad));
  }
});

// Stage awareness: the same offers that a kfml leg refuses are admitted by a leg that does not run kfml.
const REFUSED = [
  [D24, 64361],
  [D48, 128928],
];
test('stage list: kfml is planned for the middle and the large tier, never for small or adhoc', () => {
  assert.ok(legStages('large').includes('kfml'));
  assert.ok(legStages('middle').includes('kfml'));
  assert.ok(!legStages('small').includes('kfml'));
  assert.ok(!legStages('adhoc').includes('kfml'));
});
test('stage list: legStages is plannedStages of the Worker, narrowed by the payload --only/--skip rule', () => {
  assert.deepEqual(legStages('large'), plannedStages({ tier: 'large' }));
  assert.deepEqual(
    legStages('large', { skip: ['kfml'] }),
    plannedStages({ tier: 'large' }).filter((n) => n !== 'kfml'),
  );
  assert.deepEqual(legStages('large', { only: ['cuda_host'] }), ['cuda_host']);
  assert.deepEqual(legStages('large', { only: ['kfml'] }), ['kfml']);
  assert.deepEqual(legStages('large', { only: ['kfml'], skip: ['kfml'] }), []);
});
test('--skip kfml: a large leg that skips kfml admits the offers a kfml leg refuses', () => {
  for (const [g, c] of REFUSED) {
    assert.ok(reasons(KFML, g, c).includes(RULE));
    assert.ok(!reasons(legOf('large', { skip: ['kfml'] }), g, c).includes(RULE));
  }
});
test('--only without kfml: a leg that runs other stages only admits the offers a kfml leg refuses', () => {
  for (const [g, c] of REFUSED) {
    assert.ok(!reasons(legOf('large', { only: ['cuda_host'] }), g, c).includes(RULE));
    assert.ok(reasons(legOf('large', { only: ['kfml'] }), g, c).includes(RULE));
    assert.ok(!reasons(legOf('large', { only: ['kfml'], skip: ['kfml'] }), g, c).includes(RULE));
  }
});
test('a leg whose stage list is empty or missing is not subject to the rule', () => {
  for (const [g, c] of REFUSED) {
    assert.ok(!reasons({ tier: 'large', stages: [] }, g, c).includes(RULE));
    assert.ok(!reasons({ tier: 'large' }, g, c).includes(RULE));
  }
});
test('the tier decides too: the middle tier plans kfml but its kfml rows leave managed_oversub out', () => {
  assert.ok(legOf('middle').stages.includes('kfml'));
  for (const tier of ['middle', 'small', 'adhoc'])
    for (const [g, c] of REFUSED) assert.ok(!reasons(legOf(tier), g, c).includes(RULE), tier);
});
test('no leg, no rule: an evaluation that names no leg (a profile, /availability, the market read) never applies it', () => {
  for (const [g, c] of REFUSED) {
    const cfg = cfgFor('large');
    assert.ok(!offerRejections(offer(cfg, g, c), cfg, { profile: 'release' }).includes(RULE));
    assert.ok(!reasons(undefined, g, c).includes(RULE));
  }
});
test('the runner knows the reason (a name the runner rejects would void the whole availability report)', () => {
  const runner = readFileSync(new URL('../runner.py', import.meta.url), 'utf8');
  const codes = /REJECTION_CODES = frozenset\(\s*\{([^}]*)\}/.exec(runner)?.[1] ?? '';
  assert.ok(codes.includes('"' + RULE + '"'));
  assert.ok(codes.includes('"CPU_RAM"'));
});
test('the offer book counts the refusal under the named reason for a kfml leg and admits only the large-enough listing', async () => {
  const cfg = cfgFor('large');
  const market = [
    offer(cfg, D48, 128928, 1), // the measured box
    offer(cfg, D48, 134 * MIB, 2),
    offer(cfg, D24, 96 * MIB, 3),
    offer(cfg, D24, 64361, 4),
  ];
  const book = new OfferBook(() => 0),
    ledger = new Ledger(new MemoryStorage());
  const evaluation = await book.evaluate(null, cfg, ledger, 'release', { market, leg: KFML });
  assert.deepEqual(evaluation.allowed.map((o) => o.id).sort(), [2, 3]);
  assert.equal(evaluation.view.rejected[RULE], 2);
  assert.ok(evaluation.view.rejected_offers.every((r) => r.reasons.includes(RULE)));
  // the same market for an evaluation that names no leg, and for a leg that skips kfml: all four are admitted
  const none = await new OfferBook(() => 0).evaluate(null, cfg, ledger, 'release', { market });
  assert.equal(none.allowed.length, 4);
  const skip = await new OfferBook(() => 0).evaluate(null, cfg, ledger, 'release', {
    market,
    leg: legOf('large', { skip: ['kfml'] }),
  });
  assert.equal(skip.allowed.length, 4);
});
test('the runner strict validator accepts the release view that carries the reason (count and row)', async () => {
  const cfg = cfgFor('large');
  const market = [offer(cfg, D48, 128928, 1), offer(cfg, D24, 96 * MIB, 2)];
  const view = (
    await new OfferBook(() => 0).evaluate(null, cfg, new Ledger(new MemoryStorage()), 'release', {
      market,
      leg: KFML,
    })
  ).view;
  assert.equal(view.rejected[RULE], 1);
  const fixture = new URL('edits2_identity_fixture.py', import.meta.url).pathname;
  const p = spawnSync('python3', ['-I', fixture, 'validate', 'release'], {
    encoding: 'utf8',
    input: JSON.stringify(view),
    timeout: 10000,
  });
  assert.equal(p.status, 0, p.stderr + p.stdout);
});

// The release plan: the rule is applied per leg, where the leg tier is known (src/release.js).
const MARKET = JSON.parse(
  readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
).offers;
const planWith = (changes) =>
  offlinePlan(
    MARKET.map((o) => ({ ...o, ...(changes[o.id] ?? {}) })),
    { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' },
  );
const cellOf = (plan, family, die) =>
  plan.families.find((f) => f.family === family).cells.find((c) => c.die === die);
const missingOf = (plan, family) => plan.families.find((f) => f.family === family).missing;
const TIGHT = { gpu_ram: D24, cpu_ram: 64361 }; // 24 GB die, 64 GB listed: a kfml leg refuses it
const ROOMY = { gpu_ram: D24, cpu_ram: 96 * MIB };
test('release plan: a large leg (kfml) has no offer on a 64 GB listing of a 24 GB die, the same listing at 96 GB plans', async () => {
  const tight = await planWith({ 7007: TIGHT, 7008: TIGHT });
  assert.equal(cellOf(tight, 'ada', 'AD102'), undefined);
  assert.deepEqual(missingOf(tight, 'ada'), [{ die: 'AD102', reason: 'NO_OFFER' }]);
  const roomy = await planWith({ 7007: ROOMY, 7008: ROOMY });
  assert.ok(cellOf(roomy, 'ada', 'AD102'));
  assert.deepEqual(missingOf(roomy, 'ada'), []);
});
test('release plan: a middle leg does not run managed_oversub, so the same listing plans for it', async () => {
  const tight = await planWith({ 7005: TIGHT, 7006: TIGHT });
  const cell = cellOf(tight, 'ada', 'AD104');
  assert.ok(cell, 'the middle die of the family is planned on a 64 GB listing');
  assert.equal(cell.leg_tier, 'middle');
  assert.ok(cell.legs.every((l) => l.leg_tier === 'middle'));
});
test('release plan: a family with a roomy large die and a 64 GB middle die is complete, the middle die is not missing', async () => {
  const plan = await planWith({ 7005: TIGHT, 7006: TIGHT, 7007: ROOMY, 7008: ROOMY });
  const ada = plan.families.find((f) => f.family === 'ada');
  assert.deepEqual(ada.missing, []);
  assert.equal(ada.complete, true);
});
test('release plan: the rehearsal of a large die is a kfml leg too', async () => {
  const tight = await offlinePlan(
    MARKET.map((o) => (o.id === 7007 || o.id === 7008 ? { ...o, ...TIGHT } : o)),
    { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' },
    { rehearsal: 'AD102' },
  );
  assert.ok(tight.refusals.includes('NO_OFFER'), JSON.stringify(tight.refusals));
  const roomy = await offlinePlan(
    MARKET.map((o) => (o.id === 7007 || o.id === 7008 ? { ...o, ...ROOMY } : o)),
    { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' },
    { rehearsal: 'AD102' },
  );
  assert.ok(!roomy.refusals.includes('NO_OFFER'), JSON.stringify(roomy.refusals));
});

// The rent path: a leg is rented from the offers its own stage list allows, whatever the plan chose.
const asks = (h) =>
  h.cloud.calls
    .filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url))
    .map((c) => Number(/asks\/(\d+)\//.exec(c.url)[1]));
async function rehearsal(gpuRam, cheap, dear) {
  const h = await setup({
    MAX_CONCURRENT_RENTALS: '2',
    ROLLING_SPEND_USD: '200',
    RELEASE_ENABLED: 'true',
    RELEASE_COST_CAP_USD: '12',
    RELEASE_RERUNS_PER_LEG: '0',
    MIDDLE_APPS_ENABLED: undefined,
  });
  h.cloud.reporterText = 'BOX_PROTOCOL = "7"\n';
  h.cloud.payloadText = 'PAYLOAD_RELEASE_REHEARSAL = 1\n';
  const base = {
    ...h.cloud.offer,
    gpu_name: 'RTX_4090',
    cpu_name: 'AMD EPYC 7B13',
    compute_cap: 890,
    gpu_ram: gpuRam,
    disk_space: 400,
  };
  h.cloud.offer = { ...base, id: 11, machine_id: 200011, host_id: 3011, cpu_ram: cheap };
  h.cloud.price = 0.3;
  h.cloud.extraOffers = [
    { ...base, id: 12, machine_id: 200012, host_id: 3012, cpu_ram: dear, price: 0.4 },
  ];
  h.cloud.credit = 500;
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 901 + i);
  return h;
}
test('rent path: a kfml leg skips the cheaper 24 GB offer on a 64 GB listing and rents the 96 GB one', async () => {
  const h = await rehearsal(D24, 64361, 96 * MIB);
  const started = await h.service.adminStart({
    commit_sha: sha,
    tier: 'large',
    die: 'AD102',
    rehearsal: true,
  });
  assert.equal(started.legs[0].state, 'STARTED');
  assert.deepEqual(asks(h), [12]);
});
test('rent path: a kfml leg skips the cheaper 48 GB offer at 125 GB listed and rents the 134 GB one', async () => {
  const h = await rehearsal(D48, 128928, 134 * MIB);
  const started = await h.service.adminStart({
    commit_sha: sha,
    tier: 'large',
    die: 'AD102',
    rehearsal: true,
  });
  assert.equal(started.legs[0].state, 'STARTED');
  assert.deepEqual(asks(h), [12]);
});
const rentCheap = async (push) => {
  const h = await setup({ MAX_CONCURRENT_RENTALS: '2', ROLLING_SPEND_USD: '200' });
  h.cloud.offer = { ...h.cloud.offer, gpu_ram: D24, cpu_ram: 64361 };
  if (push)
    h.cloud.run = { ...h.cloud.run, event: 'push', head_branch: 'master', pull_requests: [] };
  const r = push
    ? await h.call('/rent', { claims: { event_name: 'push' } })
    : await h.call('/rent', { body: rentBody(sha) });
  assert.equal(r.status, 200, String(push) + ' ' + (await r.text()));
  assert.equal(asks(h).length, 1);
};
test('rent path: a PR (middle tier) rents the 64 GB listing of a 24 GB die, its kfml run has no managed_oversub row', () =>
  rentCheap(false));
test('rent path: a master push (small tier, no kfml stage) rents the 64 GB listing of a 24 GB die', () =>
  rentCheap(true));
