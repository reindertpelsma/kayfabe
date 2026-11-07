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
} from './release-matrix.js';
import { DEFAULTS } from './defaults.js';
import { gpuClass, resourceCaps } from './policy.js';
export const LINUX_LAST_ROW = 'cuda_fat';
export const VERDICTS = Object.freeze(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED']);
const round = (x) => Math.round(x * 10000) / 10000;
const countryRank = (offer, cfg) => {
  const i = cfg.preferredCountries.indexOf(offer.country);
  return i < 0 ? cfg.preferredCountries.length : i;
};
const byPreference = (cfg) => (a, b) =>
  countryRank(a, cfg) - countryRank(b, cfg) ||
  a.pricing.effective - b.pricing.effective ||
  a.id - b.id;
export const requiredStageRows = (stages) =>
  Object.fromEntries(
    stages.map((stage) => [stage, DEFAULTS.stageSlots[stage]?.requiredRowsPerDie ?? [stage]]),
  );
// One full leg per matrix die; the compatibility path for other callers retains the old split.
export function dieLegs(offers, cfg, matrixRow) {
  const releaseMatrix = cfg.releaseMatrix ?? RELEASE_MATRIX;
  matrixRow ??= releaseMatrix.dies.find((d) => d.die === offers[0]?.die);
  if (matrixRow)
    offers = offers.filter((o) =>
      hostProvidesDrivers(
        o.host_capabilities ?? releaseMatrix.host_capabilities.vast_kvm,
        matrixRow,
      ),
    );
  const sorted = [...offers].sort(byPreference(cfg));
  if (!sorted.length) return [];
  const first = sorted[0];
  const others = sorted.filter((o) => o.machine_id !== first.machine_id);
  const second = others.find((o) => o.host_id !== first.host_id) ?? others[0];
  const leg = (o, windowsRuns, extra = {}) => ({
    ...extra,
    windows_runs: windowsRuns,
    ...(matrixRow
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
    est_usd: round(o.pricing.total),
  });
  const want = cfg.releaseWindowsRuns;
  if (cfg.releaseStages.includes('specviewperf')) {
    const lanes = DEFAULTS.stageSlots.specviewperf.lanes;
    return lanes.map((spec, index) =>
      leg(index === 0 ? first : (second ?? first), 1, {
        spec,
        phase: index,
        ...(index > 0 ? { after_lane: lanes[0], reuse_machine: !second } : {}),
      }),
    );
  }
  if (second && want >= 2 && (!matrixRow || releaseMatrix.policy.boxes_per_die > 1))
    return [leg(first, 1), leg(second, 1)];
  return [leg(first, want)];
}
// The plan: which dies and machines a release WOULD rent, and its worst-case cost. `allowed` are
// offers already evaluated under the release profile (src/offers.js, the same rejection rules as
// every rental). `budget` (optional) is the ledger's budget summary, `credit` the account credit.
export function planRelease(allowed, cfg, { budget, credit } = {}) {
  const releaseMatrix = cfg.releaseMatrix ?? RELEASE_MATRIX;
  const matrixErrors = validateReleaseMatrix(releaseMatrix, cfg);
  if (matrixErrors.length) throw Error('release matrix: ' + matrixErrors.join('; '));
  const families = [];
  let estimate = 0,
    legs = 0;
  const perFamily = {};
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
      const cost = planned.reduce((s, l) => s + l.est_usd, 0);
      cells.push({
        die,
        offers: offers.length,
        same_machine: new Set(planned.map((l) => l.machine_id)).size === 1,
        legs: planned,
        est_usd: round(cost),
      });
      estimate += cost;
      legs += planned.length;
    }
    const familyCost = round(cells.reduce((s, c) => s + c.est_usd, 0));
    perFamily[family] = familyCost;
    families.push({
      family,
      dies_wanted: releaseMatrix.dies.filter((d) => d.die_family === family).map((d) => d.die),
      cells,
      missing,
      complete: cells.length >= cfg.releaseDiesPerFamily,
      est_usd: familyCost,
    });
  }
  const parallel = Math.min(cfg.maxConcurrentProfile.release, cfg.maxConcurrent);
  const refusals = [];
  const executionProblem = matrixExecutionProblem(releaseMatrix, cfg);
  if (executionProblem) refusals.push(executionProblem);
  if (!families.every((f) => f.complete)) refusals.push('INCOMPLETE');
  if (estimate > cfg.releaseCostCap) refusals.push('COST_CAP');
  // Review F5: a release runs at most account cap - 1 boxes; with the default account cap of 1 it
  // has no slot until the owner raises MAX_CONCURRENT_RENTALS.
  if (parallel < releaseMatrix.policy.parallel_boxes) refusals.push('PARALLEL_SLOTS');
  if (parallel < 1) refusals.push('NO_SLOTS');
  if (!cfg.releaseEnabled) refusals.push('RELEASE_DISABLED');
  if (budget) {
    const today = budget.profiles.release;
    if (today.daily_usd + estimate > cfg.releaseDaily) refusals.push('DAILY_USD');
    if (budget.daily_usd + estimate > cfg.accountDaily) refusals.push('ACCOUNT_DAILY_USD');
    if (today.count_today + legs > cfg.releaseDailyCount) refusals.push('DAILY_COUNT');
    if (budget.rolling_usd + estimate > cfg.rolling) refusals.push('ROLLING_USD');
    if (credit !== undefined && !(credit >= cfg.reserve + estimate + budget.outstanding_usd))
      refusals.push('CREDIT_RESERVE');
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
    price_cap_usd_hour: cfg.releasePrice,
    cost_cap_usd: cfg.releaseCostCap,
    account_daily_cap_usd: cfg.accountDaily,
    release_enabled: cfg.releaseEnabled,
    daily_cap_usd: cfg.releaseDaily,
    daily_rentals_cap: cfg.releaseDailyCount,
    rolling_cap_usd: cfg.rolling,
    parallel,
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
  { windowsRuns = 1, stages = [], die, family, cfg, releaseMatrix, spec } = {},
) {
  const windowsNames =
    windowsRuns === 0
      ? []
      : windowsRuns === 2
        ? ['windows_gate', 'windows_gate_2']
        : ['windows_gate'];
  const matrixRequired = releaseMatrix !== undefined || cfg?.profile === 'release';
  const matrixRow =
    releaseMatrix ?? (cfg?.releaseMatrix ?? RELEASE_MATRIX).dies?.find((r) => r.die === die);
  const definitions = matrixRequired ? matrixReceiptDefinitions(matrixRow) : [];
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
    ...(matrixRequired
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
    ...(matrixRequired ? { matrix: definitions.length ? matrixParts : { missing: 'RETRY' } } : {}),
    ...(stages.length ? { stageRows } : {}),
  };
  if (record.closed === 'STOPPED' && record.verdict !== 'PASS')
    return finish(all('UNATTRIBUTED'), 'STOPPED');
  const hardware = hardwareMatch(record, { die, family, cfg });
  // A box that reports another GPU (or none) can neither pass nor fail this die's cell.
  if (
    (record.verdict === 'PASS' && hardware !== 'MATCH') ||
    (record.verdict === 'FAIL' && result && hardware === 'MISMATCH')
  )
    return finish(all('RETRY'), 'HARDWARE_MISMATCH');
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
      batch.tier === 'large' ? need : 1,
      legs,
    );
    for (const s of stages)
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
    if (batch.tier === 'large') {
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
  const complete =
    batch.tier !== 'large' ||
    Object.keys(batch.releaseDies ?? {}).every(
      (f) => (families[f]?.size ?? 0) >= (batch.diesPerFamily ?? 2),
    );
  const verdict = cells.some((c) => c.verdict === 'FAIL')
    ? 'FAIL'
    : active
      ? 'RUNNING'
      : complete && cells.length && cells.every((c) => c.verdict === 'PASS')
        ? 'PASS'
        : 'INCOMPLETE';
  return { verdict, complete, cells };
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
