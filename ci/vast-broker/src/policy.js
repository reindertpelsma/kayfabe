// prettier-ignore
import {DEFAULTS} from './defaults.js';
// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
export class BrokerError extends Error {
  constructor(state = 'UNAVAILABLE', status = 503, reason, why) {
    super(state);
    this.why = why; // Log-only: a fixed check code for the owner's observability, never sent.
    this.state = state;
    this.status = status;
    this.reason = reason;
  }
}
export function refuse(state = 'UNAVAILABLE', status = 503, reason, why) {
  throw new BrokerError(state, status, reason, why);
}
// Public budget diagnostics contain only fixed keys and finite numbers (or unknown/null).
export function budgetRefuse(reason, values = {}) {
  const error = new BrokerError('UNAVAILABLE_BUDGET', 409, 'BUDGET:' + reason);
  error.budget = Object.fromEntries(
    Object.entries(values).map(([key, value]) => [key, Number.isFinite(value) ? value : null]),
  );
  throw error;
}
export const numericId = (v) => typeof v === 'string' && /^[1-9][0-9]{0,19}$/.test(v);
export const hexSha = (v) => typeof v === 'string' && /^[0-9a-f]{40}$/.test(v);
// Public Vast ids (machine_id, host_id) as shown in /availability and the status object
// (owner ruling 2026-10-24 item 9): a positive safe integer, otherwise null, never invented.
export const providerId = (v) => (Number.isSafeInteger(v) && v > 0 ? v : null);
export const IMAGE = 'docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16';
export const MODELS = []; // Historical local-smoke import only; production has no SKU query whitelist.
// Review of PR #269, R3: the public vars that hold a boolean. Each is exactly 'true' or 'false'
// (or unset); any other value is a named configuration error (deploy.mjs refuses it as well).
export const BOOLEAN_VARS = Object.freeze([
  'RENTING_ENABLED',
  'RELEASE_ENABLED',
  'ALLOW_UNKNOWN_COUNTRY',
  'TREE_KEY_CARRY_OVER',
]);
// The deployment's identity: without these no request can be authorized. A configuration error in
// any other (policy) var must not stop the cron's teardown, which then runs on these vars and the
// committed defaults only (teardownConfiguration, review of PR #269 R3).
export const IDENTITY_VARS = Object.freeze([
  'OWNER_GITHUB_ID',
  'REPOSITORY',
  'REPOSITORY_GITHUB_ID',
  'WORKER_URL',
  'OIDC_SUB',
  'CHECK_JOB_WORKFLOW_REFS',
  'SWEEP_JOB_WORKFLOW_REF',
]);
export function teardownConfiguration(env) {
  const identity = {};
  for (const name of IDENTITY_VARS) if (env[name] !== undefined) identity[name] = env[name];
  return configuration(identity);
}
export function configuration(env) {
  const number = (name, fallback, max = 1000) => {
    const raw = env[name] ?? String(fallback);
    if (typeof raw !== 'string' || !/^\d+(\.\d+)?$/.test(raw)) refuse();
    const n = Number(raw);
    if (!Number.isFinite(n) || n <= 0 || n > max) refuse();
    return n;
  };
  // Named (never echoed): without the owner id no rental starts and no cleanup can authorize.
  if (!numericId(env.OWNER_GITHUB_ID)) refuse('CONFIG_VAR:OWNER_GITHUB_ID', 503);
  if (!/^[-\w.]+\/[-\w.]+$/.test(env.REPOSITORY || '')) refuse();
  // The bare https origin: it is the OIDC audience and the base of KF_BROKER_URL, so a
  // trailing slash or a path is a named configuration error (review f).
  let u;
  try {
    u = new URL(env.WORKER_URL);
  } catch {
    refuse('CONFIG_VAR:WORKER_URL', 503);
  }
  if (u.protocol !== 'https:' || env.WORKER_URL !== u.origin) refuse('CONFIG_VAR:WORKER_URL', 503);
  for (const name of BOOLEAN_VARS)
    if (env[name] !== undefined && env[name] !== 'true' && env[name] !== 'false')
      refuse('CONFIG_VAR:' + name, 503);
  const list = (name, fallback) => {
    let value;
    try {
      value = JSON.parse(env[name] ?? JSON.stringify(fallback));
    } catch {
      refuse();
    }
    if (
      !Array.isArray(value) ||
      value.length > 32 ||
      !value.every(
        (x) =>
          typeof x === 'string' &&
          x.length > 0 &&
          x.length <= 128 &&
          /^[A-Za-z0-9_.*?/-]+$/.test(x),
      )
    )
      refuse();
    return value;
  };
  if (!numericId(env.REPOSITORY_GITHUB_ID)) refuse();
  const checkWorkflows = (() => {
    try {
      return JSON.parse(
        env.CHECK_JOB_WORKFLOW_REFS ??
          JSON.stringify([
            env.REPOSITORY + '/' + DEFAULTS.checkWorkflowPath + '@refs/heads/master',
          ]),
      );
    } catch {
      refuse();
    }
  })();
  const sweepWorkflow =
    env.SWEEP_JOB_WORKFLOW_REF ??
    env.REPOSITORY + '/' + DEFAULTS.sweepWorkflowPath + '@refs/heads/master';
  const validWorkflow = (value) =>
    typeof value === 'string' &&
    value.length <= 300 &&
    /^[A-Za-z0-9_.\/-]+@refs\/heads\/[A-Za-z0-9_.\/-]+$/.test(value);
  if (
    !Array.isArray(checkWorkflows) ||
    !checkWorkflows.length ||
    checkWorkflows.length > 32 ||
    !checkWorkflows.every(validWorkflow) ||
    !validWorkflow(sweepWorkflow)
  )
    refuse();
  const oidcSubject =
    env.OIDC_SUB ?? 'repo:' + env.REPOSITORY + ':environment:' + DEFAULTS.oidcEnvironment;
  if (
    typeof oidcSubject !== 'string' ||
    !/^repo:[-\w.@]+\/[-\w.@]+:environment:[A-Za-z0-9_-]+$/.test(oidcSubject)
  )
    refuse();
  const preferredCountries = list('PREFERRED_COUNTRIES', DEFAULTS.preferredCountries);
  if (
    !preferredCountries.every((x) => /^[A-Z]{2}$/.test(x)) ||
    new Set(preferredCountries).size !== preferredCountries.length
  )
    refuse();
  const deniedCountries = list('DENIED_COUNTRIES', DEFAULTS.deniedCountries);
  if (
    !deniedCountries.every((x) => /^[A-Z]{2}$/.test(x)) ||
    new Set(deniedCountries).size !== deniedCountries.length ||
    !['true', 'false'].includes(env.ALLOW_UNKNOWN_COUNTRY ?? 'false')
  )
    refuse();
  const cfg = {
    preferredCountries,
    deniedCountries,
    // Edits3 C: unknown or unparseable geolocation is rejected unless the owner opts in.
    allowUnknownCountry: env.ALLOW_UNKNOWN_COUNTRY === 'true',
    maxInet: number('MAX_INET_USD_PER_GB', DEFAULTS.maxInetUsdPerGb, 1),
    repositoryId: env.REPOSITORY_GITHUB_ID,
    checkWorkflows,
    sweepWorkflow,
    oidcSubject,
    controlIp: number('CONTROL_IP_PER_MINUTE', DEFAULTS.controlIpPerMinute, 600),
    controlGlobal: number('CONTROL_GLOBAL_PER_MINUTE', DEFAULTS.controlGlobalPerMinute, 6000),
    reportCount: number('MAX_REPORTS', DEFAULTS.reportCount, DEFAULTS.reportCount),
    reportBytes: number('MAX_REPORT_BYTES', DEFAULTS.reportBytes, DEFAULTS.reportBytes),
    reportRate: number('REPORT_RATE_LIMIT', DEFAULTS.reportRate, DEFAULTS.reportRate),
    repo: env.REPOSITORY,
    owner: env.OWNER_GITHUB_ID,
    audience: env.WORKER_URL,
    reserve: number('CREDIT_RESERVE_USD', DEFAULTS.reserve),
    price: number('MAX_PRICE_USD_HOUR', DEFAULTS.price, 2),
    boot: number('BOOT_LIMIT_MINUTES', DEFAULTS.bootMinutes, DEFAULTS.maxMinutes) * 60000,
    hard: number('HARD_RUNTIME_MINUTES', DEFAULTS.hardMinutes, DEFAULTS.maxMinutes) * 60000,
    softMargin: DEFAULTS.softMarginMinutes * 60000,
    heartbeat: DEFAULTS.heartbeatMinutes * 60000,
    heartbeatTimeout: DEFAULTS.heartbeatTimeoutMinutes * 60000,
    passGrace: number('PASS_GRACE_MINUTES', DEFAULTS.passGraceMinutes, DEFAULTS.maxMinutes) * 60000,
    failGrace: number('FAIL_GRACE_MINUTES', DEFAULTS.failGraceMinutes, DEFAULTS.maxMinutes) * 60000,
    cronPeriod: DEFAULTS.cronPeriod,
    dailyCount: number('DAILY_RENTALS', DEFAULTS.dailyCount, 40),
    daily: number('DAILY_SPEND_USD', DEFAULTS.daily),
    rolling: number('ROLLING_SPEND_USD', DEFAULTS.rolling),
    gap: number('SHA_GAP_HOURS', DEFAULTS.gapHours, 720) * 3600000,
  };
  if (
    ![
      cfg.reportCount,
      cfg.reportBytes,
      cfg.reportRate,
      cfg.dailyCount,
      cfg.controlIp,
      cfg.controlGlobal,
    ].every(Number.isInteger)
  )
    refuse();
  if (
    ![cfg.boot, cfg.hard, cfg.passGrace, cfg.failGrace].every((x) => Number.isInteger(x / 60000)) ||
    cfg.hard <= cfg.softMargin
  )
    refuse();
  // Internal duration aliases keep persisted cost rows readable; all scheduling
  // decisions use stored absolute deadlines, never elapsed poll counters.
  // Owner ruling 2026-10-24: logs are relayed, not kept; whole hours, 1 to 48.
  const retention = env.LOG_RETENTION_HOURS ?? String(DEFAULTS.logRetentionHours);
  if (!/^[1-9][0-9]?$/.test(retention) || Number(retention) > 48)
    refuse('CONFIG_VAR:LOG_RETENTION_HOURS', 503);
  cfg.logRetention = Number(retention) * 3600000;
  // Owner ruling 2026-10-24 item 8: concurrent rentals. The account cap (default 1) and optional
  // per-profile caps, which may only lower it. Each refusal names its variable. 2026-11-05
  // (wtier, review F5): the account cap may be 1 to 9; the release profile's cap is at most the
  // account cap minus 1 (default: exactly that, at most 8), so a running release always leaves a
  // slot for the master gate. Raising MAX_CONCURRENT_RENTALS alone also raises check and sweep
  // unless MAX_CONCURRENT_CHECK and MAX_CONCURRENT_SWEEP are set (DEPLOY.md, "Tiers").
  const slots = (name, fallback, low = 1) => {
    const raw = env[name] ?? fallback;
    if (typeof raw !== 'string' || !/^[0-9]$/.test(raw) || Number(raw) < low)
      refuse('CONFIG_VAR:' + name, 503);
    return Number(raw);
  };
  cfg.maxConcurrent = slots('MAX_CONCURRENT_RENTALS', String(DEFAULTS.maxConcurrentRentals));
  cfg.maxConcurrentProfile = {};
  for (const profile of ['check', 'sweep', 'release']) {
    const name = 'MAX_CONCURRENT_' + profile.toUpperCase(),
      release = profile === 'release',
      ceiling = release ? cfg.maxConcurrent - 1 : cfg.maxConcurrent,
      fallback = release ? Math.min(DEFAULTS.maxConcurrentRelease, ceiling) : ceiling,
      n = slots(name, String(fallback), release ? 0 : 1);
    if (n > ceiling) refuse('CONFIG_VAR:' + name, 503);
    cfg.maxConcurrentProfile[profile] = n;
  }
  // Lane 2, 2026-11-04: per-profile hard limits (src/tier.js). HARD_RUNTIME_MINUTES is the master
  // gate's (default 300: Linux bar plus the Windows stage); the sweep keeps at most 180 by default.
  cfg.hardCheck = cfg.hard;
  cfg.hardSweep =
    number(
      'SWEEP_HARD_RUNTIME_MINUTES',
      Math.min(cfg.hard / 60000, DEFAULTS.sweepHardMinutes),
      DEFAULTS.maxMinutes,
    ) * 60000;
  if (!Number.isInteger(cfg.hardSweep / 60000) || cfg.hardSweep <= cfg.softMargin) refuse();
  cfg.ttl = cfg.hard;
  cfg.provision = cfg.boot;
  cfg.grace = cfg.passGrace;
  cfg.sweepPrice = number('SWEEP_MAX_PRICE_USD_HOUR', DEFAULTS.sweepPrice, 10);
  cfg.sweepDaily = number('SWEEP_DAILY_USD', DEFAULTS.sweepDaily);
  // 2026-11-05 (wtier): the release profile's explicit caps. The hard limit is the check rule
  // (300 min: one leg runs the Linux bar, the Windows install and up to two Windows gate runs) and
  // may not exceed it, because the independent reaper deletes kf-ci boxes after 300 + 15 minutes.
  cfg.releasePrice = number('RELEASE_MAX_PRICE_USD_HOUR', DEFAULTS.releasePrice, 10);
  cfg.releaseDaily = number('RELEASE_DAILY_USD', DEFAULTS.releaseDaily);
  cfg.releaseDailyCount = number('RELEASE_DAILY_RENTALS', DEFAULTS.releaseDailyCount, 64);
  cfg.releaseCostCap = number('RELEASE_COST_CAP_USD', DEFAULTS.releaseCostCap);
  cfg.hardRelease =
    number('RELEASE_HARD_RUNTIME_MINUTES', DEFAULTS.releaseHardMinutes, DEFAULTS.hardMinutes) *
    60000;
  if (!Number.isInteger(cfg.releaseDailyCount)) refuse('CONFIG_VAR:RELEASE_DAILY_RENTALS', 503);
  if (!Number.isInteger(cfg.hardRelease / 60000) || cfg.hardRelease <= cfg.softMargin)
    refuse('CONFIG_VAR:RELEASE_HARD_RUNTIME_MINUTES', 503);
  if (cfg.releaseCostCap > cfg.releaseDaily) refuse('CONFIG_VAR:RELEASE_COST_CAP_USD', 503);
  // Review F4: the release kill switch. Only 'true' lets an admin Start a release or a release
  // leg rent; turning it off ends every running release at the next cron tick (pending legs and
  // re-runs are not rented; open boxes run to their result or deadline unless stopped).
  if (!['true', 'false'].includes(env.RELEASE_ENABLED ?? 'false'))
    refuse('CONFIG_VAR:RELEASE_ENABLED', 503);
  cfg.releaseEnabled = env.RELEASE_ENABLED === 'true';
  // Review F8: one daily cap across every profile (check + sweep + release, UTC day).
  cfg.accountDaily = number('ACCOUNT_DAILY_USD', DEFAULTS.accountDaily);
  const reruns = env.RELEASE_RERUNS_PER_LEG ?? String(DEFAULTS.releaseRerunsPerLeg);
  if (typeof reruns !== 'string' || !/^[0-2]$/.test(reruns))
    refuse('CONFIG_VAR:RELEASE_RERUNS_PER_LEG', 503);
  cfg.releaseReruns = Number(reruns);
  cfg.releaseStartWindow = DEFAULTS.releaseStartWindowMinutes * 60000;
  cfg.releaseDiesPerFamily = DEFAULTS.releaseDiesPerFamily;
  cfg.releaseWindowsRuns = DEFAULTS.releaseWindowsRunsPerDie;
  // The middle tier's VRAM floor (agreed 2026-11-05: middle only, from the first manual trio
  // run's footprints). 0 = off, the default until those footprints are measured (TBD).
  const vram = env.MIDDLE_VRAM_FLOOR_MB ?? String(DEFAULTS.middleVramFloorMb);
  if (typeof vram !== 'string' || !/^(0|[1-9][0-9]{0,5})$/.test(vram))
    refuse('CONFIG_VAR:MIDDLE_VRAM_FLOOR_MB', 503);
  cfg.middleVramFloorMb = Number(vram);
  cfg.bandwidthDown = number('BANDWIDTH_DOWN_GB', DEFAULTS.bandwidthDownGb, 1000);
  cfg.bandwidthUp = number('BANDWIDTH_UP_GB', DEFAULTS.bandwidthUpGb, 1000);
  const data = (name, fallback) => {
    try {
      return JSON.parse(env[name] ?? JSON.stringify(fallback));
    } catch {
      refuse();
    }
  };
  const dies = (name, fallback) => {
    const x = data(name, fallback);
    if (
      !Array.isArray(x) ||
      !x.length ||
      x.length > 32 ||
      new Set(x).size !== x.length ||
      !x.every((v) => typeof v === 'string' && /^[A-Z][A-Z0-9_]{1,23}$/.test(v))
    )
      refuse();
    return x;
  };
  cfg.checkDies = dies('CHECK_DIES', DEFAULTS.checkDies);
  cfg.sweepDies = dies('SWEEP_DIES', DEFAULTS.sweepDies);
  cfg.badMachines = data('BAD_MACHINE_IDS', DEFAULTS.badMachines);
  if (
    !Array.isArray(cfg.badMachines) ||
    cfg.badMachines.length > 256 ||
    !cfg.badMachines.every((x) => Number.isSafeInteger(x) && x > 0)
  )
    refuse();
  // Edits3 D: BAD_MACHINE_IDS replaces the built-in list; BAD_MACHINE_IDS_EXTRA adds to it.
  let extra;
  try {
    extra = JSON.parse(env.BAD_MACHINE_IDS_EXTRA ?? '[]');
  } catch {
    refuse('CONFIG_VAR:BAD_MACHINE_IDS_EXTRA', 503);
  }
  if (
    !Array.isArray(extra) ||
    extra.length > 256 ||
    !extra.every((x) => Number.isSafeInteger(x) && x > 0)
  )
    refuse('CONFIG_VAR:BAD_MACHINE_IDS_EXTRA', 503);
  cfg.badMachines = [...new Set([...cfg.badMachines, ...extra])];
  // Owner ruling 2026-10-24: hoster-level blacklist (every machine of a listed host_id).
  try {
    cfg.badHosts = JSON.parse(env.BAD_HOST_IDS ?? JSON.stringify(DEFAULTS.badHosts));
  } catch {
    refuse('CONFIG_VAR:BAD_HOST_IDS', 503);
  }
  if (
    !Array.isArray(cfg.badHosts) ||
    cfg.badHosts.length > 256 ||
    !cfg.badHosts.every((x) => Number.isSafeInteger(x) && x > 0)
  )
    refuse('CONFIG_VAR:BAD_HOST_IDS', 503);
  cfg.gpuDeny = list('GPU_DENY_PATTERNS', DEFAULTS.gpuDeny);
  cfg.dieRules = data('GPU_DIE_RULES', DEFAULTS.dieRules);
  if (
    !Array.isArray(cfg.dieRules) ||
    cfg.dieRules.length > 64 ||
    !cfg.dieRules.every(
      (r) =>
        r &&
        Object.keys(r).sort().join(',') === 'cc,die,names' &&
        /^[A-Z][A-Z0-9_]{1,23}$/.test(r.die) &&
        Number.isSafeInteger(r.cc) &&
        r.cc >= 750 &&
        r.cc <= 2000 &&
        Array.isArray(r.names) &&
        r.names.length > 0 &&
        r.names.length <= 16 &&
        r.names.every((n) => typeof n === 'string' && /^[A-Z0-9_*]{1,80}$/.test(n)),
    )
  )
    refuse();
  // Families, the middle rotation and the release die lists (2026-11-05, wtier). Family -> die
  // lists in the fixed family order; every die must belong to its family by the die rules'
  // compute capability, so a list cannot move a die across families.
  cfg.families = DEFAULTS.families;
  const familyOrder = Object.keys(cfg.families);
  const byFamily = (name, fallback) => {
    const x = data(name, fallback);
    if (
      !x ||
      typeof x !== 'object' ||
      Array.isArray(x) ||
      Object.keys(x).join(',') !== familyOrder.join(',') ||
      !familyOrder.every(
        (f) =>
          Array.isArray(x[f]) &&
          x[f].length > 0 &&
          x[f].length <= 8 &&
          new Set(x[f]).size === x[f].length &&
          x[f].every(
            (d) =>
              typeof d === 'string' &&
              /^[A-Z][A-Z0-9_]{1,23}$/.test(d) &&
              cfg.dieRules.some((r) => r.die === d && r.cc === cfg.families[f]),
          ),
      )
    )
      refuse('CONFIG_VAR:' + name, 503);
    return x;
  };
  cfg.middleRotation = byFamily('MIDDLE_ROTATION', DEFAULTS.middleRotation);
  cfg.releaseDies = byFamily('RELEASE_DIES', DEFAULTS.releaseDies);
  if (!familyOrder.every((f) => cfg.releaseDies[f].length >= cfg.releaseDiesPerFamily))
    refuse('CONFIG_VAR:RELEASE_DIES', 503);
  cfg.checkAdmitDies = [
    ...new Set([...cfg.checkDies, ...Object.values(cfg.middleRotation).flat()]),
  ];
  cfg.releaseAdmitDies = Object.values(cfg.releaseDies).flat();
  // Extra payload stages (release only). A DESIGN_ONLY slot (two_guests, trio) cannot be enabled.
  cfg.releaseStages = data('RELEASE_STAGES', DEFAULTS.releaseStages);
  if (
    !Array.isArray(cfg.releaseStages) ||
    new Set(cfg.releaseStages).size !== cfg.releaseStages.length ||
    !cfg.releaseStages.every(
      (s) =>
        typeof s === 'string' &&
        Object.hasOwn(DEFAULTS.stageSlots, s) &&
        DEFAULTS.stageSlots[s].tiers.includes('large') &&
        DEFAULTS.stageSlots[s].state !== 'DESIGN_ONLY',
    )
  )
    refuse('CONFIG_VAR:RELEASE_STAGES', 503);
  cfg.quality = {};
  for (const profile of ['check', 'sweep', 'release']) {
    const q = data(profile.toUpperCase() + '_HOST_QUALITY', DEFAULTS.quality);
    if (
      !q ||
      Object.keys(q).sort().join(',') !== Object.keys(DEFAULTS.quality).sort().join(',') ||
      !Object.values(q).every((n) => typeof n === 'number' && Number.isFinite(n) && n > 0) ||
      q.reliability > 1 ||
      q.medianRatio > 1
    )
      refuse();
    cfg.quality[profile] = q;
  }
  cfg.skipPassedDays = number(
    'SWEEP_SKIP_PASSED_DAYS',
    DEFAULTS.sweepSkipPassedDays || 0.000001,
    365,
  );
  if (env.SWEEP_SKIP_PASSED_DAYS === undefined) cfg.skipPassedDays = DEFAULTS.sweepSkipPassedDays;
  return cfg;
}
// Local smoke trust requires both explicit opt-in and a loopback HTTP caller.
// Cloudflare's public Worker URL cannot satisfy this gate. Never derive trust from JWT URLs.
export function localIdentity(env, requestUrl) {
  const caller = new URL(requestUrl),
    loopback = (u) =>
      u.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(u.hostname);
  if (env.ALLOW_TEST_ISSUER !== 'true' || !loopback(caller)) return undefined;
  let issuer;
  try {
    issuer = new URL(env.TEST_ISSUER);
  } catch {
    refuse();
  }
  if (
    !loopback(issuer) ||
    issuer.username ||
    issuer.password ||
    issuer.search ||
    issuer.hash ||
    issuer.pathname !== '/'
  )
    refuse();
  return {
    issuer: issuer.origin,
    jwks: issuer.origin + '/.well-known/jwks',
    githubOrigin: issuer.origin,
  };
}
export async function readBounded(response, limit = 262144) {
  if (Number(response.headers.get('content-length') || 0) > limit) refuse();
  const reader = response.body?.getReader();
  if (!reader) return new Uint8Array();
  const chunks = [];
  let length = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > limit) refuse();
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  const result = new Uint8Array(length);
  let at = 0;
  for (const chunk of chunks) {
    result.set(chunk, at);
    at += chunk.byteLength;
  }
  return result;
}
export async function fetchBounded(fetcher, url, init = {}, limit = 262144, allow404 = false) {
  const response = await fetcher(url, {
    ...init,
    redirect: 'manual',
    signal: AbortSignal.timeout(8000),
  });
  if (allow404 && response.status === 404) return null;
  if (!response.ok || response.redirected) refuse();
  return readBounded(response, limit);
}
export async function fetchJson(fetcher, url, init = {}, allow404 = false) {
  const bytes = await fetchBounded(fetcher, url, init, 262144, allow404);
  if (bytes === null) return null;
  try {
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    refuse();
  }
}
export function gpuClass(offer, cfg) {
  if (
    offer.gpu_arch !== 'nvidia' ||
    !Number.isSafeInteger(offer.compute_cap) ||
    offer.compute_cap < 750 ||
    offer.compute_cap > 2000 ||
    offer.compute_cap % 10 !== 0 ||
    typeof offer.gpu_name !== 'string' ||
    offer.gpu_name.length > 80
  )
    return null;
  const name = offer.gpu_name
    .toUpperCase()
    .replace(/ /g, '_')
    .replace(/^NVIDIA_/, '')
    .replace(/^GEFORCE_/, '')
    .replace(/^QUADRO_/, '');
  if (!/^(?:RTX_[A-Z0-9_]+|GTX_16[A-Z0-9_]*)$/.test(name)) return null;
  const match = (pattern) => {
    let at = 0;
    for (const part of pattern.split('*')) {
      const i = name.indexOf(part, at);
      if (i < 0 || (at === 0 && !pattern.startsWith('*') && i !== 0)) return false;
      at = i + part.length;
    }
    return pattern.endsWith('*') || at === name.length;
  };
  if (cfg.gpuDeny.some(match)) return null;
  const rules = cfg.dieRules.filter((r) => r.cc === offer.compute_cap && r.names.some(match));
  if (rules.length > 1) return null;
  return rules[0]?.die ?? 'CC_' + offer.compute_cap / 10;
}
export function offerPrice(offer, cfg) {
  for (const key of ['dph_base', 'dph_total', 'storage_cost', 'inet_down_cost', 'inet_up_cost'])
    if (
      typeof offer[key] !== 'number' ||
      !Number.isFinite(offer[key]) ||
      offer[key] < 0 ||
      offer[key] > 1000
    )
      return null;
  const storage = (offer.storage_cost * DEFAULTS.diskGb) / DEFAULTS.storageMonthHours,
    computeStorage = offer.dph_base + storage;
  // Recorded API allocated_storage=120: dph_total already includes this storage.
  if (Math.abs(offer.dph_total - computeStorage) > Math.max(1e-7, computeStorage * 1e-6))
    return null;
  const bandwidth = cfg.bandwidthDown * offer.inet_down_cost + cfg.bandwidthUp * offer.inet_up_cost;
  const effective = computeStorage + bandwidth / (cfg.hard / 3600000);
  return {
    computeStorage,
    bandwidth,
    effective,
    total: (computeStorage * (cfg.hard + cfg.cronPeriod)) / 3600000 + bandwidth,
  };
}
// The explicit per-profile caps: price per hour, daily USD, daily rental count, concurrent rentals,
// hard runtime and the die set offers are admitted from. Every admission, contract and budget
// check reads its numbers here, so a tier cannot borrow another tier's cap.
// The budget class of a stored record or spend row (review F4): release rows are written as
// profile `check` (so an older Worker can still read, count and prune them, and run check
// deadlines on them) with `budgetClass: 'release'`, which this Worker reads first.
export const budgetClass = (row) => row?.budgetClass ?? row?.profile ?? 'check';
export function profileCaps(cfg, profile) {
  if (!['check', 'sweep', 'release'].includes(profile)) throw Error('unknown profile');
  if (profile === 'check')
    return {
      price: cfg.price,
      dailyUsd: cfg.daily,
      dailyCount: cfg.dailyCount,
      concurrent: cfg.maxConcurrentProfile.check,
      hard: cfg.hardCheck,
      dies: cfg.checkAdmitDies ?? cfg.checkDies,
    };
  if (profile === 'sweep')
    return {
      price: cfg.sweepPrice,
      dailyUsd: cfg.sweepDaily,
      dailyCount: cfg.dailyCount,
      concurrent: cfg.maxConcurrentProfile.sweep,
      hard: cfg.hardSweep,
      dies: cfg.sweepDies,
    };
  return {
    price: cfg.releasePrice,
    dailyUsd: cfg.releaseDaily,
    dailyCount: cfg.releaseDailyCount,
    concurrent: cfg.maxConcurrentProfile.release,
    hard: cfg.hardRelease,
    dies: cfg.releaseAdmitDies,
  };
}
export function offerRejections(
  offer,
  cfg,
  { profile = 'check', median = offer.dlperf, badMachines = [], badHosts = [], reserved = [] } = {},
) {
  const q = cfg.quality[profile],
    reasons = [],
    die = gpuClass(offer, cfg),
    price = offerPrice(offer, cfg);
  const enough = (key, floor) =>
    typeof offer[key] === 'number' && Number.isFinite(offer[key]) && offer[key] >= floor;
  if (
    !(
      offer.verified === true ||
      (offer.verified === undefined && offer.verification === 'verified')
    )
  )
    reasons.push('VERIFICATION');
  if (
    !Number.isSafeInteger(offer.id) ||
    offer.id <= 0 ||
    offer.vms_enabled !== true ||
    offer.rentable !== true ||
    offer.rented !== false ||
    offer.num_gpus !== 1 ||
    !enough('disk_space', DEFAULTS.diskGb)
  )
    reasons.push('CAPABILITY');
  if (
    !Number.isSafeInteger(offer.machine_id) ||
    offer.machine_id <= 0 ||
    cfg.badMachines.includes(offer.machine_id) ||
    badMachines.includes(offer.machine_id)
  )
    reasons.push('MACHINE');
  if (cfg.badHosts.includes(offer.host_id) || badHosts.includes(offer.host_id))
    reasons.push('BAD_HOST');
  // Item 8: a machine is held from CREATING until its run has ended; no second run takes it.
  if (reserved.includes(offer.machine_id)) reasons.push('IN_USE');
  const caps = profileCaps(cfg, profile);
  if (!die || !caps.dies.includes(die)) reasons.push('DIE');
  if (!enough('reliability', q.reliability) || offer.reliability > 1) reasons.push('RELIABILITY');
  if (
    !enough('cpu_cores_effective', q.cores) ||
    !enough('cpu_ghz', q.cpuGhz) ||
    !enough('cpu_ram', q.ramMb)
  )
    reasons.push('CPU_RAM');
  if (
    !enough('dlperf', Number.MIN_VALUE) ||
    typeof median !== 'number' ||
    !Number.isFinite(median) ||
    median <= 0 ||
    offer.dlperf < median * q.medianRatio
  )
    reasons.push('PERFORMANCE');
  if (!enough('inet_down', q.downMbps) || !enough('inet_up', q.upMbps)) reasons.push('NETWORK');
  if (!enough('disk_bw', q.diskMbps)) reasons.push('DISK_BW');
  if (!price) reasons.push('PRICE_FIELDS');
  else if (
    offer.inet_up_cost > cfg.maxInet ||
    offer.inet_down_cost > cfg.maxInet ||
    price.effective > caps.price ||
    price.total > (caps.price * (cfg.hard + cfg.cronPeriod)) / 3600000
  )
    reasons.push('PRICE');
  return reasons;
}
export function validOffer(offer, cfg, context) {
  return offerRejections(offer, cfg, context).length === 0;
}
