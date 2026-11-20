import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import * as policy from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { OfferBook } from '../src/offers.js';
import { Ledger } from '../src/ledger.js';
import { DEFAULTS } from '../src/defaults.js';
import { env, MemoryStorage, MockCloud } from './helpers.js';
const { configuration, offerRejections } = policy;
// 2026-11-20. MEASURED: a Vast KVM box listed with an RTX 4090 of 49140 MiB and 125 GB of RAM (MemTotal 103.5 GB)
// made the kfml row `managed_oversub` NOT_RUN ("host RAM too small to oversubscribe VRAM by 25%"). The row wants
// 1.25 x VRAM <= 0.6 x MemAvailable, and a KVM box gets only about 0.75 to 0.83 of the LISTED RAM as MemTotal, so a
// flat RAM floor calibrated on 24 GB dies cannot hold. The offer policy now derives the listed-RAM floor from the
// offer's own gpu_ram for the profile whose kfml plan runs that row (the large tier, which is the release profile).
//
// FALSIFIER (written before the code): (1) a 49140 MiB offer with 125 GB listed is admitted in the large profile;
// (2) a 24564 MiB offer with 96 GB listed is refused; (3) the rule leaks into a profile that does not run the row
// (small, middle, adhoc) or is missing from the large one; (4) the rule's numbers differ from the numbers the kfml
// workload uses (a retyped constant); (5) the refusal carries no named reason, or a reason the runner does not know;
// (6) an offer with no numeric gpu_ram or cpu_ram is admitted to the large profile; (7) the unit of gpu_ram (MiB) or
// of cpu_ram (MiB) is confused with decimal MB or GB.
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
const reasons = (tier, gpuRam, cpuRam) => {
  const cfg = cfgFor(tier);
  return offerRejections(offer(cfg, gpuRam, cpuRam), cfg, { profile: profileOf(tier) });
};
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
test('constants tie: the rule covers the profiles whose kfml plan runs managed_oversub', () => {
  const tier = /tier="(\w+)"/.exec(source)?.[1];
  assert.equal(tier, 'large', 'managed_oversub is a large-tier kfml row');
  assert.deepEqual(policy.KFML_OVERSUB_PROFILES, ['large']);
  for (const name of policy.KFML_OVERSUB_PROFILES)
    assert.ok(DEFAULTS.payloadStageResources.kfml.profiles.includes(name), name);
});
test('the KVM fraction is the conservative end of the measured 0.75 to 0.83 (2026-11-20)', () => {
  assert.equal(policy.KVM_RAM_FRACTION, 0.75);
});
test('the derived floor: 24564 MiB needs 68234 MiB listed, 49140 MiB needs 136500 MiB listed', () => {
  assert.equal(Math.ceil(policy.listedRamNeededMb(D24)), 68234);
  assert.ok(Math.abs(policy.listedRamNeededMb(D48) - 136500) < 1e-6);
});

test('24 GB die: 96 GB listed is admitted, the 64 GB listing of the measured box class is refused', () => {
  assert.ok(!reasons('large', D24, 96 * MIB).includes(RULE));
  assert.ok(reasons('large', D24, 64361).includes(RULE));
  assert.deepEqual(reasons('large', D24, 96 * MIB), []);
});
test('48 GB die: the measured 125 GB listing and 96 GB are refused with the named reason, 134 GB is admitted', () => {
  assert.ok(reasons('large', D48, 128928).includes(RULE)); // the measured box: 125 GB listed
  assert.ok(reasons('large', D48, 96 * MIB).includes(RULE));
  assert.ok(!reasons('large', D48, 134 * MIB).includes(RULE));
  assert.deepEqual(reasons('large', D48, 134 * MIB), []);
});
test('boundary: one MiB below the derived floor is refused, the floor itself is admitted', () => {
  assert.ok(reasons('large', D48, 136499).includes(RULE));
  assert.ok(!reasons('large', D48, 136500).includes(RULE));
  assert.ok(reasons('large', D24, 68233).includes(RULE));
  assert.ok(!reasons('large', D24, 68234).includes(RULE));
});
test('units: gpu_ram and cpu_ram are both MiB (a 12 GB die with 64 GB listed is admitted, 24 GB with 48 GB is not)', () => {
  assert.ok(!reasons('large', 12288, 65536).includes(RULE));
  assert.ok(reasons('large', 24576, 48 * MIB).includes(RULE));
  assert.ok(!reasons('large', 24576, 72 * MIB).includes(RULE));
});
test('the refusal is the named reason alone: the flat CPU_RAM floor is not what refused it', () => {
  const r = reasons('large', D48, 128928);
  assert.ok(!r.includes('CPU_RAM'));
  assert.deepEqual(r, [RULE]);
});
test('the rule is a profile rule: small, middle and adhoc offers with 64 GB listed keep passing', () => {
  for (const tier of ['small', 'middle', 'adhoc'])
    assert.ok(!reasons(tier, D24, 64361).includes(RULE), tier);
  assert.ok(reasons('large', D24, 64361).includes(RULE));
});
test('fail closed: a large offer without a numeric gpu_ram or cpu_ram is refused with the named reason', () => {
  for (const bad of [undefined, null, '24564', NaN, Infinity]) {
    assert.ok(reasons('large', bad, 256 * MIB).includes(RULE), 'gpu_ram ' + String(bad));
    assert.ok(reasons('large', D24, bad).includes(RULE), 'cpu_ram ' + String(bad));
  }
});
test('the runner knows the reason (a name the runner rejects would void the whole availability report)', () => {
  const runner = readFileSync(new URL('../runner.py', import.meta.url), 'utf8');
  const codes = /REJECTION_CODES = frozenset\(\s*\{([^}]*)\}/.exec(runner)?.[1] ?? '';
  assert.ok(codes.includes('"' + RULE + '"'));
  assert.ok(codes.includes('"CPU_RAM"'));
});
test('the offer book counts the refusal under the named reason and admits only the large-enough listing', async () => {
  const cfg = cfgFor('large');
  const market = [
    offer(cfg, D48, 128928, 1), // the measured box
    offer(cfg, D48, 134 * MIB, 2),
    offer(cfg, D24, 96 * MIB, 3),
    offer(cfg, D24, 64361, 4),
  ];
  const book = new OfferBook(() => 0),
    ledger = new Ledger(new MemoryStorage());
  const evaluation = await book.evaluate(null, cfg, ledger, 'release', { market });
  assert.deepEqual(evaluation.allowed.map((o) => o.id).sort(), [2, 3]);
  assert.equal(evaluation.view.rejected[RULE], 2);
  assert.ok(evaluation.view.rejected_offers.every((r) => r.reasons.includes(RULE)));
});
test('the runner strict validator accepts the release view that carries the reason (count and row)', async () => {
  const cfg = cfgFor('large');
  const market = [offer(cfg, D48, 128928, 1), offer(cfg, D24, 96 * MIB, 2)];
  const view = (
    await new OfferBook(() => 0).evaluate(null, cfg, new Ledger(new MemoryStorage()), 'release', {
      market,
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
