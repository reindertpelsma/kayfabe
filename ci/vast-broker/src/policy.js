// prettier-ignore
import {DEFAULTS} from './defaults.js';
// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
export class BrokerError extends Error {
  constructor(state = 'UNAVAILABLE', status = 503, reason) {
    super(state);
    this.state = state;
    this.status = status;
    this.reason = reason;
  }
}
export function refuse(state = 'UNAVAILABLE', status = 503, reason) {
  throw new BrokerError(state, status, reason);
}
export const numericId = (v) => typeof v === 'string' && /^[1-9][0-9]{0,19}$/.test(v);
export const hexSha = (v) => typeof v === 'string' && /^[0-9a-f]{40}$/.test(v);
export const IMAGE = 'docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16';
export const MODELS = []; // Historical local-smoke import only; production has no SKU query whitelist.
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
  let u;
  try {
    u = new URL(env.WORKER_URL);
  } catch {
    refuse();
  }
  if (
    u.protocol !== 'https:' ||
    u.username ||
    u.password ||
    u.search ||
    u.hash ||
    u.pathname !== '/'
  )
    refuse();
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
    !/^repo:[-\w.]+\/[-\w.]+:environment:[A-Za-z0-9_-]+$/.test(oidcSubject)
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
    !['true', 'false'].includes(env.ALLOW_UNKNOWN_COUNTRY ?? 'true')
  )
    refuse();
  const cfg = {
    preferredCountries,
    deniedCountries,
    allowUnknownCountry: (env.ALLOW_UNKNOWN_COUNTRY ?? 'true') === 'true',
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
    dailyCount: number('DAILY_RENTALS', DEFAULTS.dailyCount, 20),
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
  cfg.ttl = cfg.hard;
  cfg.provision = cfg.boot;
  cfg.grace = cfg.passGrace;
  cfg.sweepPrice = number('SWEEP_MAX_PRICE_USD_HOUR', DEFAULTS.sweepPrice, 10);
  cfg.sweepDaily = number('SWEEP_DAILY_USD', DEFAULTS.sweepDaily);
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
  cfg.quality = {};
  for (const profile of ['check', 'sweep']) {
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
  cfg.driverTags = data('OFFER_DRIVER_TAGS', DEFAULTS.driverTags);
  if (
    !Array.isArray(cfg.driverTags) ||
    !cfg.driverTags.length ||
    cfg.driverTags.length > 128 ||
    !cfg.driverTags.every((t) => typeof t === 'string' && /^\d{3}\.\d{1,3}(?:\.\d{1,2})?$/.test(t))
  )
    refuse();
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
export function offerRejections(
  offer,
  cfg,
  { profile = 'check', median = offer.dlperf, badMachines = [] } = {},
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
  if (!die || !(profile === 'check' ? cfg.checkDies : cfg.sweepDies).includes(die))
    reasons.push('DIE');
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
  if (!cfg.driverTags.includes(offer.driver_version)) reasons.push('DRIVER');
  if (!price) reasons.push('PRICE_FIELDS');
  else if (
    offer.inet_up_cost > cfg.maxInet ||
    offer.inet_down_cost > cfg.maxInet ||
    price.effective > (profile === 'check' ? cfg.price : cfg.sweepPrice) ||
    price.total >
      ((profile === 'check' ? cfg.price : cfg.sweepPrice) * (cfg.hard + cfg.cronPeriod)) / 3600000
  )
    reasons.push('PRICE');
  return reasons;
}
export function validOffer(offer, cfg, context) {
  return offerRejections(offer, cfg, context).length === 0;
}
