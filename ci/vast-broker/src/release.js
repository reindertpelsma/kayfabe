import { rankingPrice } from './provider-policy.js';
// The large (release) tier: plan, leg classification and the release verdict. Pure functions over
// already evaluated offers and ledger records; src/index.js does the renting through the same
// admission path as every other rental (2026-11-05, wtier; design: V3_CI_TIERS_AND_IGNORE_LADDER.md
// §1, agreed and owner decisions of 2026-11-04/05).
//
// Owner decisions of 2026-11-07 live in release-matrix.json: eight explicit dies,
// two per die family, one full leg per die, eight parallel slots. Each leg carries
// separate host/Linux/Windows drivers and subruns; Windows still runs twice per die.
// The matrix is proposed, not an executable coverage claim: the payload refuses it.
// Existing verdict, retry, cost and admission rules remain in force.
import { familyOf } from './tier.js';
import {
  RELEASE_MATRIX,
  validateReleaseMatrix,
  hostProvidesDrivers,
  matrixExecutionProblem,
  matrixReceiptDefinitions,
  checkedMatrixReceipts,
  sameMatrixRow,
  driverPinsFor,
  legTierOfRow,
  vendorOfCpuName,
  CPU_VENDOR_REQUIRED,
} from './release-matrix.js';
import { DEFAULTS } from './defaults.js';
import { gpuClass, resourceCaps, validateStagePolicy } from './policy.js';
export const LINUX_LAST_ROW = 'cuda_fat';
export const VERDICTS = Object.freeze(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED']);
const round = (x) => Math.round(x * 10000) / 10000;
const countryRank = (offer, cfg) => {
  const i = cfg.preferredCountries.indexOf(offer.country);
  return i < 0 ? cfg.preferredCountries.length : i;
};
const byPreference = (cfg) => (a, b) =>
  countryRank(a, cfg) - countryRank(b, cfg) || rankingPrice(a) - rankingPrice(b) || a.id - b.id;
// One credit rule for the release plan and the rehearsal plan (NaN-safe: anything but a sufficient number is short).
const creditShort = (credit, cfg, estimate, budget) =>
  credit !== undefined && !(credit >= cfg.reserve + estimate + budget.outstanding_usd);
export const requiredStageRows = (stages) => {
  validateStagePolicy(stages);
  return Object.fromEntries(
    stages.map((stage) => [
      stage,
      stage === 'specviewperf' ? [...DEFAULTS.stageSlots.specviewperf.requiredRowsPerDie] : [stage],
    ]),
  );
};
// One full leg per matrix die; the compatibility path for other callers retains the old split.
// `rehearsal` (2026-11-12): the named one-die rehearsal scope. The leg carries NO driver matrix (no release_matrix, drivers,
// pins or subruns: the payload would refuse them, and they would be a false claim), and the host is not filtered by what it
// could swap, because nothing is swapped. The row is still the die's matrix row, for the box policy only.
// `pin` (planRelease's CPU-vendor swap): {first?: offer, second?: offer|null} replaces the cheapest-first choice for that
// leg; `second: null` pins "no second machine" (the second leg then reuses the first box).
// Matrix v2 (owner 2026-11-12): a die whose matrix profile is 'middle' (the second die of each family) is ONE middle leg:
// one Windows run, the apps, no SPEC lanes, priced at the middle hard limit (RELEASE_MIDDLE_HARD_RUNTIME_MINUTES).
// Hosts whose cpu_name is an ARM CPU are never planned (every box is x86).
export function eligibleOffers(offers, releaseMatrix, matrixRow) {
  const swappable = matrixRow
    ? offers.filter((o) =>
        hostProvidesDrivers(
          o.host_capabilities ?? releaseMatrix.host_capabilities.vast_kvm,
          matrixRow,
        ),
      )
    : offers;
  return swappable.filter((o) => vendorOfCpuName(o.cpu_name) !== 'arm');
}
export function legEstimate(o, cfg, legTier) {
  if (legTier !== 'middle') return o.pricing.total;
  return (
    (o.pricing.computeStorage * (cfg.releaseMiddleHard + cfg.cronPeriod)) / 3600000 +
    o.pricing.bandwidth
  );
}
export function dieLegs(offers, cfg, matrixRow, { rehearsal = false, pin } = {}) {
  validateStagePolicy(cfg.releaseStages);
  const releaseMatrix = cfg.releaseMatrix ?? RELEASE_MATRIX;
  matrixRow ??= releaseMatrix.dies.find((d) => d.die === offers[0]?.die);
  if (!rehearsal) offers = eligibleOffers(offers, releaseMatrix, matrixRow);
  const sorted = [...offers].sort(byPreference(cfg));
  if (!sorted.length) return [];
  const first = pin?.first ?? sorted[0];
  const others = sorted.filter((o) => o.machine_id !== first.machine_id);
  const second =
    pin && 'second' in pin
      ? (pin.second ?? undefined)
      : (others.find((o) => o.host_id !== first.host_id) ?? others[0]);
  const legTier = rehearsal ? 'large' : legTierOfRow(matrixRow);
  const leg = (o, windowsRuns, extra = {}) => ({
    ...extra,
    ...(rehearsal ? {} : { leg_tier: legTier }),
    cpu_vendor: vendorOfCpuName(o.cpu_name),
    windows_runs: windowsRuns,
    ...(rehearsal ? { rehearsal: true } : {}),
    ...(matrixRow && !rehearsal
      ? {
          release_matrix: structuredClone(matrixRow),
          drivers: structuredClone(matrixRow.drivers),
          driver_pins: structuredClone(driverPinsFor(matrixRow, releaseMatrix)),
          subruns: structuredClone(matrixRow.subruns),
          cross_subrun: structuredClone(matrixRow.cross_subrun),
          profile: matrixRow.profile,
        }
      : {}),
    offer_id: o.id,
    machine_id: o.machine_id,
    host_id: o.host_id ?? null,
    gpu_name: String(o.gpu_name).toUpperCase().replaceAll(' ', '_'),
    country: o.country,
    dph_total: round(o.dph_total),
    est_usd: round(legEstimate(o, cfg, legTier)),
    ...(legTier === 'middle' ? { hard_minutes: cfg.releaseMiddleHard / 60000 } : {}),
  });
  if (legTier === 'middle') return [leg(first, 1)];
  const want = cfg.releaseWindowsRuns;
  if (cfg.releaseStages.includes('specviewperf')) {
    const lanes = DEFAULTS.stageSlots.specviewperf.lanes;
    return lanes.map((spec, index) =>
      leg(index === 0 ? first : (second ?? first), 1, {
        spec,
        // Large includes middle, but the apps (about 57 min) run only on the LAST lane leg: lane A needs 169 min left
        // for SPEC and the apps guard does not reserve SPEC's time, so apps before lane A would starve it on a slow host
        // and trigger a paid rerun. Lane B (133 min) carries them. Absent `apps` means true (legs without SPEC).
        ...(index < lanes.length - 1 ? { apps: false } : {}),
        phase: index,
        ...(index > 0 ? { after_lane: lanes[0], reuse_machine: !second } : {}),
      }),
    );
  }
  if (second && want >= 2 && (!matrixRow || releaseMatrix.policy.boxes_per_die > 1))
    return [leg(first, 1), leg(second, 1)];
  return [leg(first, want)];
}
// The CPU-vendor cover over the large legs (owner 2026-11-12). Each die's cheapest admissible offers are already chosen; if the
// large legs lack a required vendor, swap ONE slot at a time to the cheapest admissible other-vendor offer of the same die
// (same constraints: the matrix host filter, no ARM, the SPEC lanes on different machines when two are offered) without
// dropping another required vendor below its minimum. Middle legs never take part. Returns the report; mutates the cells.
export function coverCpuVendors(families, pools, cfg, releaseMatrix) {
  const min = releaseMatrix.policy.cpu_vendor_min;
  const largeCells = () => families.flatMap((f) => f.cells).filter((c) => c.leg_tier === 'large');
  const count = (cellsOverride) => {
    const n = { amd: 0, intel: 0, other: 0, arm: 0 };
    for (const c of cellsOverride ?? largeCells()) for (const l of c.legs) n[l.cpu_vendor] += 1;
    return n;
  };
  const deficit = (n) => CPU_VENDOR_REQUIRED.filter((v) => n[v] < min[v]);
  const lack = (n) => CPU_VENDOR_REQUIRED.reduce((sum, v) => sum + Math.max(0, min[v] - n[v]), 0);
  const before = count();
  const swaps = [];
  const costOf = (legs) => legs.reduce((sum, l) => sum + l.est_usd, 0);
  for (let guard = 0; guard < 16 && deficit(count()).length; guard++) {
    const cur = count(),
      lacking = deficit(cur);
    let best;
    for (const cell of largeCells()) {
      const { row, pool } = pools.get(cell);
      const byId = new Map(pool.map((o) => [o.id, o]));
      const current = cell.legs.map((l) => byId.get(l.offer_id));
      if (current.some((o) => o === undefined)) continue;
      const slots =
        cell.legs.length === 2 && !cell.same_machine
          ? [[0], [1]]
          : [[0, 1].slice(0, cell.legs.length)];
      for (const vendorWanted of lacking)
        for (const candidate of pool.filter((o) => vendorOfCpuName(o.cpu_name) === vendorWanted)) {
          for (const slot of slots) {
            // The pinned offers of the new plan of this die.
            const pin =
              slot.length === 2
                ? { first: candidate, second: null }
                : slot[0] === 0
                  ? { first: candidate }
                  : { first: current[0], second: candidate };
            if (pin.first.machine_id === pin.second?.machine_id) continue;
            if (
              slot[0] === 0 &&
              slot.length === 1 &&
              current[1] &&
              current[1].machine_id === candidate.machine_id
            )
              pin.second = pool.find(
                (o) => o.machine_id !== candidate.machine_id && o.id !== candidate.id,
              );
            const legs = dieLegs(pool, cfg, row, { pin });
            if (!legs.length) continue;
            const after = count(largeCells().map((c) => (c === cell ? { legs } : c)));
            // Never trade one required vendor for another: every step must strictly reduce what is still lacking
            // (a swap that drops a vendor below its minimum raises the lack on that side and is refused).
            if (lack(after) >= lack(cur)) continue;
            const delta = costOf(legs) - costOf(cell.legs);
            if (
              !best ||
              delta < best.delta - 1e-9 ||
              (Math.abs(delta - best.delta) <= 1e-9 && candidate.id < best.id)
            )
              best = { cell, legs, delta, id: candidate.id, vendor: vendorWanted, slot };
          }
        }
    }
    if (!best) break;
    swaps.push({
      die: best.cell.die,
      slot: best.slot.length === 2 ? 'both' : best.slot[0] === 0 ? 'lane_a' : 'lane_b',
      to_vendor: best.vendor,
      offer_id: best.id,
      delta_usd: round(best.delta),
    });
    best.cell.legs = best.legs;
    best.cell.same_machine = new Set(best.legs.map((l) => l.machine_id)).size === 1;
    best.cell.est_usd = round(costOf(best.legs));
  }
  const planned = count();
  // The legs that cover a vendor are pinned to it at rent time (index.js admit): cheapest offers elsewhere may pick any vendor.
  const pinned = {};
  for (const v of CPU_VENDOR_REQUIRED)
    if (min[v] > 0)
      for (const cell of largeCells())
        for (const l of cell.legs)
          if (l.cpu_vendor === v && (pinned[v] ?? 0) < min[v] && !l.pin_cpu_vendor) {
            l.pin_cpu_vendor = v;
            pinned[v] = (pinned[v] ?? 0) + 1;
          }
  return {
    required: { ...min },
    scope: releaseMatrix.policy.cpu_vendor_scope,
    planned,
    before_swaps: before,
    swaps,
    missing: deficit(planned),
  };
}
// The plan: which dies and machines a release WOULD rent, and its worst-case cost. `allowed` are
// offers already evaluated under the release profile (src/offers.js, the same rejection rules as
// every rental). `budget` (optional) is the ledger's budget summary, `credit` the account credit.
export function planRelease(allowed, cfg, { budget, credit } = {}) {
  validateStagePolicy(cfg.releaseStages);
  const releaseMatrix = cfg.releaseMatrix ?? RELEASE_MATRIX;
  const matrixErrors = validateReleaseMatrix(releaseMatrix, cfg);
  if (matrixErrors.length) throw Error('release matrix: ' + matrixErrors.join('; '));
  const families = [];
  const pools = new Map();
  for (const family of Object.keys(cfg.releaseDies)) {
    const cells = [],
      missing = [];
    for (const matrixRow of releaseMatrix.dies.filter((d) => d.die_family === family)) {
      const die = matrixRow.die;
      if (cells.length >= cfg.releaseDiesPerFamily) break;
      const offers = allowed.filter((o) => o.die === die && familyOf(o.die, cfg) === family);
      const planned = dieLegs(offers, cfg, matrixRow);
      if (!planned.length) {
        missing.push({ die, reason: 'NO_OFFER' });
        continue;
      }
      const cell = {
        die,
        leg_tier: legTierOfRow(matrixRow),
        offers: offers.length,
        same_machine: new Set(planned.map((l) => l.machine_id)).size === 1,
        legs: planned,
        est_usd: round(planned.reduce((s, l) => s + l.est_usd, 0)),
      };
      pools.set(cell, { row: matrixRow, pool: eligibleOffers(offers, releaseMatrix, matrixRow) });
      cells.push(cell);
    }
    families.push({
      family,
      dies_wanted: releaseMatrix.dies.filter((d) => d.die_family === family).map((d) => d.die),
      cells,
      missing,
      complete: cells.length >= cfg.releaseDiesPerFamily,
    });
  }
  // Owner 2026-11-12: the CPU-vendor rule is over the LARGE legs of the whole release.
  const vendor = coverCpuVendors(families, pools, cfg, releaseMatrix);
  let estimate = 0,
    legs = 0;
  const perFamily = {};
  for (const f of families) {
    f.est_usd = round(f.cells.reduce((sum, c) => sum + c.est_usd, 0));
    perFamily[f.family] = f.est_usd;
    estimate += f.cells.reduce((sum, c) => sum + c.est_usd, 0);
    legs += f.cells.reduce((sum, c) => sum + c.legs.length, 0);
  }
  const parallel = Math.min(cfg.maxConcurrentProfile.release, cfg.maxConcurrent);
  const refusals = [];
  const executionProblem = matrixExecutionProblem(releaseMatrix, cfg);
  if (executionProblem) refusals.push(executionProblem);
  if (!families.every((f) => f.complete)) refusals.push('INCOMPLETE');
  // A release whose large legs cannot cover every required CPU vendor is INCOMPLETE and never admitted (plan and judge).
  if (vendor.missing.length) {
    refusals.push('CPU_VENDOR_MISSING');
    if (!refusals.includes('INCOMPLETE')) refusals.push('INCOMPLETE');
  }
  if (estimate > cfg.releaseCostCap) refusals.push('COST_CAP');
  // Review F5: a release runs at most account cap - 1 boxes; with the default account cap of 1 it
  // has no slot until the owner raises MAX_CONCURRENT_RENTALS.
  if (parallel < releaseMatrix.policy.parallel_boxes) refusals.push('PARALLEL_SLOTS');
  if (parallel < 1) refusals.push('NO_SLOTS');
  if (!cfg.releaseEnabled) refusals.push('RELEASE_DISABLED');
  // large includes middle: the release never inherits the middle apps kill switch (a release without apps is not a release)
  if (!cfg.middleAppsEnabled) refusals.push('APPS_DISABLED');
  if (budget) {
    const today = budget.profiles.release;
    if (today.daily_usd + estimate > cfg.releaseDaily) refusals.push('DAILY_USD');
    if (budget.daily_usd + estimate > cfg.accountDaily) refusals.push('ACCOUNT_DAILY_USD');
    if (today.count_today + legs > cfg.releaseDailyCount) refusals.push('DAILY_COUNT');
    if (budget.rolling_usd + estimate > cfg.rolling) refusals.push('ROLLING_USD');
    if (creditShort(credit, cfg, estimate, budget)) refusals.push('CREDIT_RESERVE');
  }
  return {
    resource_profile: resourceCaps(cfg, 'release', 'large'),
    matrix_status: releaseMatrix.status,
    matrix_blockers: releaseMatrix.dies.flatMap((d) =>
      d.blockers.map((reason) => ({ die: d.die, reason })),
    ),
    execution_ready: executionProblem === null,
    tier: 'large',
    profile: 'release',
    hard_minutes: cfg.hardRelease / 60000,
    middle_hard_minutes: cfg.releaseMiddleHard / 60000,
    price_cap_usd_hour: cfg.releasePrice,
    cost_cap_usd: cfg.releaseCostCap,
    account_daily_cap_usd: cfg.accountDaily,
    release_enabled: cfg.releaseEnabled,
    daily_cap_usd: cfg.releaseDaily,
    daily_rentals_cap: cfg.releaseDailyCount,
    rolling_cap_usd: cfg.rolling,
    parallel,
    cpu_vendor: vendor,
    large_legs: families
      .flatMap((f) => f.cells)
      .filter((c) => c.leg_tier === 'large')
      .reduce((n, c) => n + c.legs.length, 0),
    middle_legs: families
      .flatMap((f) => f.cells)
      .filter((c) => c.leg_tier === 'middle')
      .reduce((n, c) => n + c.legs.length, 0),
    dies_per_family: cfg.releaseDiesPerFamily,
    windows_runs_per_die: cfg.releaseWindowsRuns,
    reruns_per_leg: cfg.releaseReruns,
    stages: cfg.releaseStages,
    required_stage_rows_per_die: requiredStageRows(cfg.releaseStages),
    families,
    legs,
    estimate_usd: round(estimate),
    estimate_by_family_usd: perFamily,
    rerun_headroom_usd: round(Math.max(0, cfg.releaseCostCap - estimate)),
    admitted: refusals.length === 0,
    refusals,
  };
}
// The named REHEARSAL scope (owner request 2026-11-12; docs/design/V3_RELEASE_REHEARSAL.md): the large tier's payload on ONE
// matrix die, through the same admission, cost and verdict machinery, WITHOUT the driver sub-run matrix. It never hides a
// matrix blocker: they are listed in the plan as `matrix_blockers`, `execution_ready` stays false, and every report row says
// the sub-run matrix was not executed. The most it can end in is REHEARSAL_PASS (releaseVerdict); it is never a release PASS.
export const REHEARSAL_SCOPE = Object.freeze({
  scope: 'REHEARSAL',
  driver_subrun_matrix: 'NOT_EXECUTED',
  max_verdict: 'REHEARSAL_PASS',
  counts_toward_release: false,
});
export function planRehearsal(allowed, cfg, { budget, credit, die } = {}) {
  validateStagePolicy(cfg.releaseStages);
  const releaseMatrix = cfg.releaseMatrix ?? RELEASE_MATRIX;
  const matrixErrors = validateReleaseMatrix(releaseMatrix, cfg);
  if (matrixErrors.length) throw Error('release matrix: ' + matrixErrors.join('; '));
  const row = releaseMatrix.dies.find((d) => d.die === die);
  const refusals = [];
  if (!row) refusals.push('DIE_NOT_IN_MATRIX');
  const family = row?.die_family;
  const offers = row ? allowed.filter((o) => o.die === die && familyOf(o.die, cfg) === family) : [];
  const planned = row ? dieLegs(offers, cfg, row, { rehearsal: true }) : [];
  if (row && !planned.length) refusals.push('NO_OFFER');
  const estimate = planned.reduce((sum, l) => sum + l.est_usd, 0);
  // Legs of one die run one after the other (SPEC lanes) or on one box, so one slot is enough.
  const parallel = Math.min(cfg.maxConcurrentProfile.release, cfg.maxConcurrent);
  if (cfg.releaseCostCap < estimate) refusals.push('COST_CAP');
  if (parallel < 1) refusals.push('NO_SLOTS');
  if (!cfg.releaseEnabled) refusals.push('RELEASE_DISABLED');
  if (!cfg.middleAppsEnabled) refusals.push('APPS_DISABLED');
  if (budget) {
    // Same caps as a release plan, written out here so a change to one is a visible change to both.
    const today = budget.profiles.release;
    if (cfg.releaseDaily < today.daily_usd + estimate) refusals.push('DAILY_USD');
    if (cfg.accountDaily < budget.daily_usd + estimate) refusals.push('ACCOUNT_DAILY_USD');
    if (cfg.releaseDailyCount < today.count_today + planned.length) refusals.push('DAILY_COUNT');
    if (cfg.rolling < budget.rolling_usd + estimate) refusals.push('ROLLING_USD');
    if (creditShort(credit, cfg, estimate, budget)) refusals.push('CREDIT_RESERVE');
  }
  const executionProblem = matrixExecutionProblem(releaseMatrix, cfg);
  return {
    ...REHEARSAL_SCOPE,
    rehearsal: true,
    rehearsal_die: die,
    resource_profile: resourceCaps(cfg, 'release', 'large'),
    matrix_status: releaseMatrix.status,
    // Stated, never removed: what the matrix still lacks. A rehearsal does not clear any of it.
    matrix_blockers: releaseMatrix.dies.flatMap((d) =>
      d.blockers.map((reason) => ({ die: d.die, reason })),
    ),
    matrix_execution_problem: executionProblem,
    execution_ready: executionProblem === null,
    tier: 'large',
    profile: 'release',
    hard_minutes: cfg.hardRelease / 60000,
    price_cap_usd_hour: cfg.releasePrice,
    cost_cap_usd: cfg.releaseCostCap,
    account_daily_cap_usd: cfg.accountDaily,
    release_enabled: cfg.releaseEnabled,
    daily_cap_usd: cfg.releaseDaily,
    daily_rentals_cap: cfg.releaseDailyCount,
    rolling_cap_usd: cfg.rolling,
    parallel,
    windows_runs_per_die: cfg.releaseWindowsRuns,
    reruns_per_leg: cfg.releaseReruns,
    stages: cfg.releaseStages,
    required_stage_rows_per_die: requiredStageRows(cfg.releaseStages),
    families: row
      ? [
          {
            family,
            dies_wanted: [die],
            cells: planned.length
              ? [
                  {
                    die,
                    offers: offers.length,
                    same_machine: new Set(planned.map((l) => l.machine_id)).size === 1,
                    legs: planned,
                    est_usd: round(estimate),
                  },
                ]
              : [],
            missing: planned.length ? [] : [{ die, reason: 'NO_OFFER' }],
            complete: planned.length > 0,
            est_usd: round(estimate),
          },
        ]
      : [],
    legs: planned.length,
    estimate_usd: round(estimate),
    rerun_headroom_usd: round(Math.max(0, cfg.releaseCostCap - estimate)),
    admitted: refusals.length === 0,
    refusals,
  };
}
// Rows that say "not a verdict on the guest": a host, harness, time or rental fault.
export function retryRow(name) {
  return (
    /^windows_retry_[a-z0-9_]+$/.test(name) ||
    /^[a-z0-9]+_retry_[a-z0-9_]+$/.test(name) ||
    ['infrastructure', 'preflight', 'provider_kvm_missing', 'provider_gpu_unavailable'].includes(
      name,
    )
  );
}
// One leg's outcome per platform, from the Worker's own record (verdict, closing reason and the
// box's checked result rows). Box data can only make a leg worse: a PASS needs the Worker's PASS
// verdict and every expected row passing.
// Review F3: the box's own report must be the planned die. MATCH needs a known driver, the
// family's compute capability and the die class of the reported GPU name (the same die rules as
// the offers); UNKNOWN = the box reported no usable hardware; anything else is MISMATCH.
export function hardwareMatch(record, { die, family, cfg } = {}) {
  const h = record?.hardware ?? record?.result?.hardware;
  if (
    !h ||
    typeof h.gpu_name !== 'string' ||
    !Number.isSafeInteger(h.compute_cap) ||
    typeof h.driver_version !== 'string' ||
    h.driver_version === 'unknown' ||
    h.gpu_name === 'unknown'
  )
    return 'UNKNOWN';
  if (!cfg || !die || !family || !Object.hasOwn(cfg.families, family)) return 'MISMATCH';
  const boxDie = gpuClass(
    { gpu_arch: 'nvidia', compute_cap: h.compute_cap, gpu_name: h.gpu_name },
    cfg,
  );
  return h.compute_cap === cfg.families[family] && boxDie === die ? 'MATCH' : 'MISMATCH';
}
export function classifyLeg(
  record,
  { windowsRuns = 1, stages = [], die, family, cfg, releaseMatrix, spec, rehearsal = false } = {},
) {
  const windowsNames =
    windowsRuns === 0
      ? []
      : windowsRuns === 2
        ? ['windows_gate', 'windows_gate_2']
        : ['windows_gate'];
  // A rehearsal leg is graded WITHOUT matrix receipts: the sub-run matrix is not executed, and that is stated in the
  // verdict (releaseVerdict), never turned into a pass. A rehearsal that also carries a matrix row is not a rehearsal.
  const matrixRequired = releaseMatrix !== undefined || cfg?.profile === 'release';
  const matrixCoverage = matrixRequired && !rehearsal;
  const matrixRow =
    releaseMatrix ?? (cfg?.releaseMatrix ?? RELEASE_MATRIX).dies?.find((r) => r.die === die);
  const definitions = matrixCoverage ? matrixReceiptDefinitions(matrixRow) : [];
  const stageRequirements = requiredStageRows(stages);
  const ownStageRows = Object.fromEntries(
    stages.map((stage) => [
      stage,
      stage === 'specviewperf' && spec ? ['specviewperf_' + spec] : stageRequirements[stage],
    ]),
  );
  const all = (v) => ({
    linux: v,
    windows: windowsNames.map(() => v),
    stages: Object.fromEntries(stages.map((s) => [s, v])),
    ...(matrixCoverage
      ? {
          matrix: Object.fromEntries(
            (definitions.length ? definitions : [{ id: 'missing' }]).map((r) => [r.id, v]),
          ),
        }
      : {}),
    ...(stages.length
      ? {
          stageRows: Object.fromEntries(
            Object.values(ownStageRows)
              .flat()
              .map((n) => [n, v]),
          ),
        }
      : {}),
  });
  // Only the Worker's own PASS verdict can make a PASS leg; any other record whose rows all pass
  // is inconsistent and is a RETRY, never a pass.
  const finish = (parts, reason, passAllowed = false) => {
    const values = [
      parts.linux,
      ...parts.windows,
      ...Object.values(parts.stages),
      ...Object.values(parts.matrix ?? {}),
      ...Object.values(parts.stageRows ?? {}),
    ];
    if (!passAllowed && values.every((v) => v === 'PASS'))
      return { verdict: 'RETRY', reason: 'INCONSISTENT', ...all('RETRY') };
    const verdict = values.includes('FAIL')
      ? 'FAIL'
      : values.includes('UNATTRIBUTED')
        ? 'UNATTRIBUTED'
        : values.includes('RETRY')
          ? 'RETRY'
          : values.every((v) => v === 'PASS')
            ? 'PASS'
            : 'RETRY';
    return { verdict, reason, ...parts };
  };
  if (!record || record.ended === null || record.ended === undefined)
    return finish(all('RETRY'), 'NOT_ENDED');
  const result = record.result,
    rows = Array.isArray(result?.suites) ? result.suites : [];
  const green = result?.host_reference === 'GREEN';
  if (record.closed === 'STOPPED' && record.verdict !== 'PASS')
    return finish(all('UNATTRIBUTED'), 'STOPPED');
  const hardware = hardwareMatch(record, { die, family, cfg });
  // A box that reports another GPU (or none) can neither pass nor fail this die's cell.
  if (
    (record.verdict === 'PASS' && hardware !== 'MATCH') ||
    (record.verdict === 'FAIL' && result && hardware === 'MISMATCH')
  )
    return finish(all('RETRY'), 'HARDWARE_MISMATCH');

  if (rows.some((r) => r.fail > 0 && /^(?:[a-z0-9]+_)*driver_media_drift$/.test(r.name))) {
    const realFailure = rows.some(
      (r) => r.fail > 0 && !/^(?:[a-z0-9]+_)*driver_media_drift$/.test(r.name) && !retryRow(r.name),
    );
    if (realFailure)
      return finish(all(green ? 'FAIL' : 'UNATTRIBUTED'), 'TEST_FAILURE_WITH_MEDIA_DRIFT');
    return finish(all('RETRY'), 'MEDIA_DRIFT');
  }
  const passing = (name) => rows.some((r) => r.name === name && r.pass > 0 && r.fail === 0);
  const rowVerdict = (name) => {
    const failures = rows.filter(
      (r) => r.fail > 0 && (r.name === name || r.name.startsWith(name + '_')),
    );
    if (failures.some((r) => !retryRow(r.name))) return green ? 'FAIL' : 'UNATTRIBUTED';
    return passing(name) ? 'PASS' : 'RETRY';
  };
  const stageRows = Object.fromEntries(
    Object.values(ownStageRows)
      .flat()
      .map((name) => [name, rowVerdict(name)]),
  );
  const stageParts = Object.fromEntries(
    stages.map((stage) => {
      const values = ownStageRows[stage].map((name) => stageRows[name]);
      return [
        stage,
        values.includes('FAIL')
          ? 'FAIL'
          : values.includes('UNATTRIBUTED')
            ? 'UNATTRIBUTED'
            : values.every((v) => v === 'PASS')
              ? 'PASS'
              : 'RETRY',
      ];
    }),
  );
  const proof = result?.matrix_receipts;
  const matrixParts = Object.fromEntries(
    definitions.map((definition) => {
      const names =
        definition.kind === 'full' ? [LINUX_LAST_ROW, ...windowsNames] : [definition.suite];
      const rowValues = names.map(rowVerdict);
      if (rowValues.includes('FAIL')) return [definition.id, 'FAIL'];
      if (rowValues.includes('UNATTRIBUTED')) return [definition.id, 'UNATTRIBUTED'];
      if (!checkedMatrixReceipts(proof)) return [definition.id, 'RETRY'];
      const receipt = proof.rows.find((r) => r.id === definition.id);
      if (!receipt) return [definition.id, 'RETRY'];
      if (
        proof.die !== die ||
        receipt.windows_guest_abi !== definition.windows_guest_abi ||
        !Object.keys(definition.drivers).every(
          (axis) => receipt.drivers[axis] === definition.drivers[axis],
        ) ||
        (definition.kind === 'full' &&
          (record.hardware ?? result.hardware)?.driver_version !== matrixRow.drivers.host_driver)
      )
        return [definition.id, 'UNATTRIBUTED'];
      if (receipt.verdict === 'FAIL') return [definition.id, green ? 'FAIL' : 'UNATTRIBUTED'];
      return [
        definition.id,
        receipt.verdict === 'PASS' && rowValues.every((v) => v === 'PASS') ? 'PASS' : 'RETRY',
      ];
    }),
  );
  const coverage = {
    ...(matrixCoverage ? { matrix: definitions.length ? matrixParts : { missing: 'RETRY' } } : {}),
    ...(stages.length ? { stageRows } : {}),
  };
  if (record.verdict === 'PASS' && result?.verdict === 'PASS') {
    const parts = {
      linux: passing(LINUX_LAST_ROW) ? 'PASS' : 'RETRY',
      windows: windowsNames.map((n) => (passing(n) ? 'PASS' : 'RETRY')),
      stages: stageParts,
      ...coverage,
    };
    return finish(parts, 'RESULT', true);
  }
  if (record.verdict !== 'FAIL' || !result) return finish(all('RETRY'), record.verdict ?? 'NONE');
  // A FAIL: the payload stops at the first failing stage, so the rows say how far it got.
  const failed = rows.filter((r) => r.fail > 0).map((r) => r.name);
  const real = failed.filter((n) => !retryRow(n));
  const kind = (names) =>
    names.some((n) => !retryRow(n)) ? (green ? 'FAIL' : 'UNATTRIBUTED') : 'RETRY';
  const linuxFailed = failed.filter(
    (n) => !n.startsWith('windows_') && !stages.some((s) => n === s || n.startsWith(s + '_')),
  );
  const parts = {
    linux: 'NOT_RUN',
    windows: windowsNames.map(() => 'NOT_RUN'),
    stages: {},
    ...coverage,
  };
  for (const s of stages) parts.stages[s] = 'NOT_RUN';
  if (linuxFailed.length || !passing(LINUX_LAST_ROW)) {
    parts.linux = linuxFailed.length ? kind(linuxFailed) : green ? 'FAIL' : 'UNATTRIBUTED';
    return finish(parts, 'LINUX');
  }
  parts.linux = 'PASS';
  // Windows runs are sequential and the payload stops at the first failing one, so every failing
  // windows_* row belongs to the first run that did not pass.
  const windowsFailed = failed.filter((n) => n.startsWith('windows_'));
  let stopped = false;
  for (const [i, name] of windowsNames.entries()) {
    if (passing(name)) {
      parts.windows[i] = 'PASS';
      continue;
    }
    parts.windows[i] = windowsFailed.length ? kind(windowsFailed) : 'RETRY';
    stopped = true;
    break;
  }
  // A planned or additional failing Windows subrun cannot disappear merely
  // because both ordinary gate rows passed (RV1's exact accepted FAIL shape).
  if (!stopped && windowsFailed.some((name) => !retryRow(name))) {
    parts.windows[Math.max(0, parts.windows.length - 1)] = kind(windowsFailed);
    stopped = true;
  }
  if (!stopped)
    for (const s of stages) {
      const mine = failed.filter((n) => n === s || n.startsWith(s + '_'));
      if (stageParts[s] === 'PASS') parts.stages[s] = 'PASS';
      else {
        parts.stages[s] = mine.length ? kind(mine) : 'RETRY';
        break;
      }
    }
  return finish(parts, real.length ? 'FAIL_ROWS' : 'RETRY_ROWS');
}
// The release verdict over (family, die, platform) cells. A cell FAILs on any FAIL; linux passes
// on at least one PASS; windows on at least `windowsRuns` passing Windows runs; an enabled extra
// stage on every required lane row, and matrix coverage on every planned driver receipt.
// The release PASSes only when every die family has its full number of
// dies with every cell passing; RUNNING while a leg is pending or started; otherwise INCOMPLETE.
export function releaseVerdict(batch, { releaseMatrix = RELEASE_MATRIX } = {}) {
  const need = batch.windowsRunsPerDie ?? 2,
    stages = batch.stages ?? [],
    stageRequirements = requiredStageRows(stages);
  const cells = [];
  const active = batch.legs.some((l) => l.state === 'PENDING' || l.state === 'STARTED');
  const keyed = new Map();
  for (const leg of batch.legs) {
    const id = leg.family + '/' + leg.die;
    if (!keyed.has(id)) keyed.set(id, []);
    keyed.get(id).push(leg);
  }
  const cell = (family, die, platform, values, required, legs) => {
    const passes = values.filter((v) => v === 'PASS').length;
    const verdict = values.includes('FAIL')
      ? 'FAIL'
      : passes >= required
        ? 'PASS'
        : legs.some((l) => l.state === 'PENDING' || l.state === 'STARTED')
          ? 'RUNNING'
          : 'INCOMPLETE';
    cells.push({
      family,
      die,
      platform,
      verdict,
      passes,
      required,
      runs: legs.map((l) => l.key),
      machines: new Set(legs.filter((l) => l.machineId).map((l) => l.machineId)).size,
    });
  };
  for (const legs of keyed.values()) {
    const { family, die } = legs[0];
    // Matrix v2: the second die of a family is a MIDDLE leg (one Windows run, no extra stages, apps via the payload).
    const middleDie = legs.every((l) => l.legTier === 'middle');
    const ended = legs.filter((l) => l.outcome);
    cell(
      family,
      die,
      'linux',
      ended.map((l) => l.outcome.linux),
      1,
      legs,
    );
    cell(
      family,
      die,
      'windows',
      ended.flatMap((l) => l.outcome.windows),
      batch.tier === 'large' && !middleDie ? need : 1,
      legs,
    );
    for (const s of middleDie ? [] : stages)
      for (const name of stageRequirements[s])
        cell(
          family,
          die,
          'stage:' + name,
          ended.map(
            (l) => l.outcome.stageRows?.[name] ?? (name === s ? l.outcome.stages?.[s] : undefined),
          ),
          1,
          legs,
        );
    if (batch.tier === 'large' && batch.rehearsal === true) {
      // Stated in the report: the driver sub-run matrix was not executed. This cell can never be PASS.
      cell(family, die, 'matrix:driver_subruns', [], 1, legs);
      cells[cells.length - 1].verdict = 'NOT_EXECUTED';
    } else if (batch.tier === 'large') {
      const row = releaseMatrix?.dies?.find((r) => r.die === die);
      const definitions = matrixReceiptDefinitions(row);
      for (const definition of definitions.length ? definitions : [{ id: 'missing' }])
        cell(
          family,
          die,
          'matrix:' + definition.id,
          ended.map((l) =>
            l.outcome.matrix?.[definition.id] === 'FAIL'
              ? 'FAIL'
              : sameMatrixRow(l.releaseMatrix, row)
                ? l.outcome.matrix?.[definition.id]
                : 'UNATTRIBUTED',
          ),
          1,
          legs,
        );
      if (matrixExecutionProblem(releaseMatrix, DEFAULTS))
        cell(family, die, 'matrix:execution', ['UNATTRIBUTED'], 1, legs);
    }
  }
  const families = {};
  for (const c of cells) {
    families[c.family] ??= new Set();
    families[c.family].add(c.die);
  }
  if (batch.tier === 'large' && batch.rehearsal === true) {
    // One die, no sub-run matrix: the highest verdict is REHEARSAL_PASS, over every cell except the one that says
    // "not executed". It is not a release verdict and a release PASS stays impossible while matrix blockers exist.
    const executed = cells.filter((c) => c.platform !== 'matrix:driver_subruns');
    const verdict = cells.some((c) => c.verdict === 'FAIL')
      ? 'FAIL'
      : active
        ? 'RUNNING'
        : executed.length && executed.every((c) => c.verdict === 'PASS')
          ? 'REHEARSAL_PASS'
          : 'INCOMPLETE';
    return {
      verdict,
      complete: false,
      ...REHEARSAL_SCOPE,
      rehearsal: true,
      cells: cells.map((c) => ({ ...c, ...REHEARSAL_SCOPE })),
    };
  }
  const largeFamilies = new Set(
    batch.legs.filter((l) => l.legTier !== 'middle').map((l) => l.family),
  );
  const middleFamilies = new Set(
    batch.legs.filter((l) => l.legTier === 'middle').map((l) => l.family),
  );
  const complete =
    batch.tier !== 'large' ||
    Object.keys(batch.releaseDies ?? {}).every(
      (f) =>
        (families[f]?.size ?? 0) >= (batch.diesPerFamily ?? 2) &&
        // matrix v2: one large die AND one middle die per family (batches without leg tiers keep the old shape)
        (middleFamilies.size === 0 || (largeFamilies.has(f) && middleFamilies.has(f))),
    );
  // CPU-vendor judge (owner 2026-11-12): a vendor counts only from a PASSING large leg whose rent-time vendor (the offer's
  // cpu_name) AND the box's own report agree. A mismatch or a missing report counts for neither vendor.
  const vendor = cpuVendorCoverage(batch, releaseMatrix);
  const verdict = cells.some((c) => c.verdict === 'FAIL')
    ? 'FAIL'
    : active
      ? 'RUNNING'
      : complete &&
          vendor.missing.length === 0 &&
          cells.length &&
          cells.every((c) => c.verdict === 'PASS')
        ? 'PASS'
        : 'INCOMPLETE';
  return {
    verdict,
    complete,
    cpu_vendor: vendor,
    ...(vendor.missing.length ? { reason: 'CPU_VENDOR_MISSING' } : {}),
    cells,
  };
}
// Per vendor: the passing large legs whose offered and reported vendors agree. `required` is the matrix policy; a batch of a
// matrix without the rule (or a non-large batch) requires nothing.
export function cpuVendorCoverage(batch, releaseMatrix = RELEASE_MATRIX) {
  const required = batch.tier === 'large' ? (releaseMatrix?.policy?.cpu_vendor_min ?? {}) : {};
  const confirmed = { amd: 0, intel: 0 },
    mismatched = [],
    unreported = [];
  for (const l of batch.legs) {
    if (l.legTier === 'middle' || l.outcome?.verdict !== 'PASS') continue;
    const offered = l.cpuVendorOffered ?? null,
      reported = l.cpuVendorReported ?? null;
    if (reported === null || offered === null) unreported.push(l.key);
    else if (offered !== reported) mismatched.push(l.key);
    else if (Object.hasOwn(confirmed, offered)) confirmed[offered] += 1;
  }
  return {
    required: { ...required },
    confirmed,
    mismatched,
    unreported,
    missing: CPU_VENDOR_REQUIRED.filter((v) => confirmed[v] < (required[v] ?? 0)),
  };
}
// Whether an ended leg gets a fresh-box re-run: only RETRY and UNATTRIBUTED (never FAIL, never an
// owner stop), only while the leg's re-run allowance lasts.
export function wantsRerun(leg, reruns) {
  return (
    leg.outcome &&
    ['RETRY', 'UNATTRIBUTED'].includes(leg.outcome.verdict) &&
    leg.outcome.reason !== 'STOPPED' &&
    (leg.rerunDepth ?? 0) < reruns
  );
}
