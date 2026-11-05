// The large (release) tier: plan, leg classification and the release verdict. Pure functions over
// already evaluated offers and ledger records; src/index.js does the renting through the same
// admission path as every other rental (2026-11-05, wtier; design: V3_CI_TIERS_AND_IGNORE_LADDER.md
// §1, agreed and owner decisions of 2026-11-04/05).
//
// Rules encoded here (each pinned by test/release.test.js):
// - at least `releaseDiesPerFamily` (2) dies of every family (Turing, Ampere, Ada, Blackwell), dies
//   taken in the configured order (measured lists, defaults.json releaseDies), one driver per die
//   (the host driver of the rented box; the payload installs the pinned one);
// - every leg is one box that runs Linux and Windows; at least `releaseWindowsRunsPerDie` (2)
//   Windows runs per die: a second leg on a DIFFERENT machine when the die is offered on two
//   machines, otherwise one leg that runs the Windows gate twice (KF_WINDOWS_RUNS=2);
// - PASS only from a verified guest verdict. RETRY and UNATTRIBUTED are never PASS; they schedule a
//   fresh-box re-run within the release cost cap; a FAIL stays a FAIL and is never re-rented;
// - the estimated cost (offer price for the full hard limit, plus the transfer allowance) must
//   not exceed the release cost cap, and every per-profile cap must admit it, before any rental.
import { familyOf } from './tier.js';
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
// The legs of one die: two machines when offered (prefer different hosts), else one machine
// that runs Windows twice.
export function dieLegs(offers, cfg) {
  const sorted = [...offers].sort(byPreference(cfg));
  if (!sorted.length) return [];
  const first = sorted[0];
  const others = sorted.filter((o) => o.machine_id !== first.machine_id);
  const second = others.find((o) => o.host_id !== first.host_id) ?? others[0];
  const leg = (o, windowsRuns) => ({
    windows_runs: windowsRuns,
    offer_id: o.id,
    machine_id: o.machine_id,
    host_id: o.host_id ?? null,
    gpu_name: String(o.gpu_name).toUpperCase().replaceAll(' ', '_'),
    country: o.country,
    dph_total: round(o.dph_total),
    est_usd: round(o.pricing.total),
  });
  const want = cfg.releaseWindowsRuns;
  if (second && want >= 2) return [leg(first, 1), leg(second, 1)];
  return [leg(first, want)];
}
// The plan: which dies and machines a release WOULD rent, and its worst-case cost. `allowed` are
// offers already evaluated under the release profile (src/offers.js, the same rejection rules as
// every rental). `budget` (optional) is the ledger's budget summary, `credit` the account credit.
export function planRelease(allowed, cfg, { budget, credit } = {}) {
  const families = [];
  let estimate = 0,
    legs = 0;
  const perFamily = {};
  for (const family of Object.keys(cfg.releaseDies)) {
    const cells = [],
      missing = [];
    for (const die of cfg.releaseDies[family]) {
      if (cells.length >= cfg.releaseDiesPerFamily) break;
      const offers = allowed.filter((o) => o.die === die && familyOf(o.die, cfg) === family);
      const planned = dieLegs(offers, cfg);
      if (!planned.length) {
        missing.push({ die, reason: 'NO_OFFER' });
        continue;
      }
      const cost = planned.reduce((s, l) => s + l.est_usd, 0);
      cells.push({
        die,
        offers: offers.length,
        same_machine: planned.length === 1,
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
      dies_wanted: cfg.releaseDies[family],
      cells,
      missing,
      complete: cells.length >= cfg.releaseDiesPerFamily,
      est_usd: familyCost,
    });
  }
  const parallel = Math.min(cfg.maxConcurrentProfile.release, cfg.maxConcurrent);
  const refusals = [];
  if (!families.every((f) => f.complete)) refusals.push('INCOMPLETE');
  if (estimate > cfg.releaseCostCap) refusals.push('COST_CAP');
  if (budget) {
    const today = budget.profiles.release;
    if (today.daily_usd + estimate > cfg.releaseDaily) refusals.push('DAILY_USD');
    if (today.count_today + legs > cfg.releaseDailyCount) refusals.push('DAILY_COUNT');
    if (budget.rolling_usd + estimate > cfg.rolling) refusals.push('ROLLING_USD');
    if (credit !== undefined && !(credit >= cfg.reserve + estimate + budget.outstanding_usd))
      refusals.push('CREDIT_RESERVE');
  }
  return {
    tier: 'large',
    profile: 'release',
    hard_minutes: cfg.hardRelease / 60000,
    price_cap_usd_hour: cfg.releasePrice,
    cost_cap_usd: cfg.releaseCostCap,
    daily_cap_usd: cfg.releaseDaily,
    daily_rentals_cap: cfg.releaseDailyCount,
    rolling_cap_usd: cfg.rolling,
    parallel,
    dies_per_family: cfg.releaseDiesPerFamily,
    windows_runs_per_die: cfg.releaseWindowsRuns,
    reruns_per_leg: cfg.releaseReruns,
    stages: cfg.releaseStages,
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
export function classifyLeg(record, { windowsRuns = 1, stages = [] } = {}) {
  const windowsNames =
    windowsRuns === 0
      ? []
      : windowsRuns === 2
        ? ['windows_gate', 'windows_gate_2']
        : ['windows_gate'];
  const all = (v) => ({
    linux: v,
    windows: windowsNames.map(() => v),
    stages: Object.fromEntries(stages.map((s) => [s, v])),
  });
  // Only the Worker's own PASS verdict can make a PASS leg; any other record whose rows all pass
  // is inconsistent and is a RETRY, never a pass.
  const finish = (parts, reason, passAllowed = false) => {
    const values = [parts.linux, ...parts.windows, ...Object.values(parts.stages)];
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
  const passing = (name) => rows.some((r) => r.name === name && r.pass > 0 && r.fail === 0);
  if (record.closed === 'STOPPED' && record.verdict !== 'PASS')
    return finish(all('UNATTRIBUTED'), 'STOPPED');
  if (record.verdict === 'PASS' && result?.verdict === 'PASS') {
    const parts = {
      linux: passing(LINUX_LAST_ROW) ? 'PASS' : 'RETRY',
      windows: windowsNames.map((n) => (passing(n) ? 'PASS' : 'RETRY')),
      stages: Object.fromEntries(stages.map((s) => [s, passing(s) ? 'PASS' : 'RETRY'])),
    };
    return finish(parts, 'RESULT', true);
  }
  if (record.verdict !== 'FAIL' || !result) return finish(all('RETRY'), record.verdict ?? 'NONE');
  // A FAIL: the payload stops at the first failing stage, so the rows say how far it got.
  const failed = rows.filter((r) => r.fail > 0).map((r) => r.name);
  const green = result.host_reference === 'GREEN';
  const real = failed.filter((n) => !retryRow(n));
  const kind = (names) =>
    names.some((n) => !retryRow(n)) ? (green ? 'FAIL' : 'UNATTRIBUTED') : 'RETRY';
  const linuxFailed = failed.filter(
    (n) => !n.startsWith('windows_') && !stages.some((s) => n === s || n.startsWith(s + '_')),
  );
  const parts = { linux: 'NOT_RUN', windows: windowsNames.map(() => 'NOT_RUN'), stages: {} };
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
  if (!stopped)
    for (const s of stages) {
      const mine = failed.filter((n) => n === s || n.startsWith(s + '_'));
      if (passing(s)) parts.stages[s] = 'PASS';
      else {
        parts.stages[s] = mine.length ? kind(mine) : 'RETRY';
        break;
      }
    }
  return finish(parts, real.length ? 'FAIL_ROWS' : 'RETRY_ROWS');
}
// The release verdict over (family, die, platform) cells. A cell FAILs on any FAIL; linux passes
// on at least one PASS; windows on at least `windowsRuns` passing Windows runs; an enabled extra
// stage on at least one PASS. The release PASSes only when every family has its full number of
// dies with every cell passing; RUNNING while a leg is pending or started; otherwise INCOMPLETE.
export function releaseVerdict(batch) {
  const need = batch.windowsRunsPerDie ?? 2,
    stages = batch.stages ?? [];
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
      cell(
        family,
        die,
        'stage:' + s,
        ended.map((l) => l.outcome.stages?.[s]),
        1,
        legs,
      );
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
