// prettier-ignore
import {refuse} from './policy.js';
export function checkedResult(result) {
  const object = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
  if (
    !object(result) ||
    Object.keys(result)
      .filter((k) => k !== 'host_reference' && k !== 'host_claims')
      .sort()
      .join(',') !== 'hardware,nonce,runner_nonce,sha,suites,verdict' ||
    (result.host_reference !== undefined &&
      !['GREEN', 'NOT_GREEN', 'MISSING'].includes(result.host_reference)) ||
    !['PASS', 'FAIL', 'UNAVAILABLE', 'PROVIDER_FAULT'].includes(result.verdict) ||
    !/^([0-9a-f]{32})$/.test(result.nonce) ||
    !/^([0-9a-f]{64})$/.test(result.runner_nonce) ||
    !/^([0-9a-f]{40})$/.test(result.sha)
  )
    refuse('BAD_REPORT', 400);
  const rows = result.suites;
  if (
    !Array.isArray(rows) ||
    rows.length < 1 ||
    rows.length > 20 ||
    new Set(rows.map((x) => x?.name)).size !== rows.length
  )
    refuse('BAD_REPORT', 400);
  for (const r of rows)
    if (
      !object(r) ||
      Object.keys(r).sort().join(',') !== 'fail,name,pass,skip' ||
      typeof r.name !== 'string' ||
      !/^[a-z][a-z0-9_]{0,47}$/.test(r.name) ||
      !['pass', 'fail', 'skip'].every(
        (k) => Number.isSafeInteger(r[k]) && r[k] >= 0 && r[k] <= 10000000,
      )
    )
      refuse('BAD_REPORT', 400);
  if (
    result.verdict === 'PROVIDER_FAULT' &&
    !rows.some(
      (r) => ['provider_kvm_missing', 'provider_gpu_unavailable'].includes(r.name) && r.fail > 0,
    )
  )
    refuse('BAD_REPORT', 400);
  // Optional host claims (setup picker, 2026-11-06): sent only by a box whose Worker asked for them
  // (KF_HOST_CLAIMS). They are claims, never evidence: bounded, pattern-checked, and a malformed or
  // oversized one is dropped field by field instead of failing the report, so a claim can neither
  // upgrade nor block a verdict.
  if (result.host_claims !== undefined) result.host_claims = cleanClaims(result.host_claims);
  const h = result.hardware;
  if (
    !object(h) ||
    Object.keys(h).sort().join(',') !== 'compute_cap,driver_version,gpu_name' ||
    typeof h.gpu_name !== 'string' ||
    !/^[A-Za-z0-9_. -]{1,100}$/.test(h.gpu_name) ||
    !Number.isSafeInteger(h.compute_cap) ||
    h.compute_cap < 0 ||
    h.compute_cap > 2000 ||
    typeof h.driver_version !== 'string' ||
    !/^(?:unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?)$/.test(h.driver_version)
  )
    refuse('BAD_REPORT', 400);
  if (result.verdict === 'PASS') {
    if (
      rows.some((r) => r.fail !== 0 || r.skip !== 0 || r.pass <= 0) ||
      h.gpu_name === 'unknown' ||
      h.driver_version === 'unknown' ||
      h.compute_cap < 750
    )
      refuse('BAD_REPORT', 400);
  } else if (!rows.some((r) => r.fail > 0)) refuse('BAD_REPORT', 400);
  return result;
}

// ---------------------------------------------------------------------------------------------
// Host claims: what the box says about its CPU, OS versions and drivers. Public (the picker on the
// status site) but only ever CLAIMS: the box read them from its own /proc and nvidia-smi, the
// Worker checks their shape and nothing else, and no verdict, die or requirement reads them.
// cleanClaims is TOTAL: any input gives a clean object, every invalid field becomes null, so a bad
// claim can never refuse a report or a row (which would lose the run's result).
// ---------------------------------------------------------------------------------------------
export const CLAIM_KEYS = Object.freeze([
  'cpu_vendor',
  'cpu_model',
  'cpu_arch',
  'host_driver',
  'linux_os_version',
  'linux_driver',
  'windows_os_version',
  'windows_driver',
]);
export const CPU_VENDORS = Object.freeze(['amd', 'intel', 'arm', 'other']);
export const CPU_ARCHS = Object.freeze(['x86_64', 'aarch64']);
const CLAIM_MODEL = /^[A-Za-z0-9][A-Za-z0-9 ().@+/_-]{0,63}$/;
const CLAIM_OS_VERSION = /^[a-z0-9][a-z0-9 ._()+-]{0,31}$/;
// 3-4 digit major, then one or two dotted parts: 580.95.05, 580.88 (5 to 16 characters).
const CLAIM_DRIVER = /^[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?$/;
// Text that could carry an identifier: a UUID, a long hex run (serial, machine id, hash), an
// IPv4-shaped number, a URL or an address with a host part. No CPU model or OS version needs any.
const IDENTIFIER_SHAPED =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}|[0-9a-f]{12,}|[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|:\/\/|@[a-z0-9-]+\./i;
const claimText = (value, pattern) =>
  typeof value === 'string' &&
  pattern.test(value) &&
  !IDENTIFIER_SHAPED.test(value) &&
  !TOKEN_SHAPED.test(value)
    ? value
    : null;

export function cleanClaims(raw) {
  const out = Object.fromEntries(CLAIM_KEYS.map((k) => [k, null]));
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return out;
  const get = (key) => (Object.hasOwn(raw, key) ? raw[key] : null);
  out.cpu_vendor = CPU_VENDORS.includes(get('cpu_vendor')) ? get('cpu_vendor') : null;
  out.cpu_arch = CPU_ARCHS.includes(get('cpu_arch')) ? get('cpu_arch') : null;
  const model = get('cpu_model');
  out.cpu_model = claimText(
    typeof model === 'string' ? model.replace(/\s+/g, ' ').trim() : null,
    CLAIM_MODEL,
  );
  for (const key of ['host_driver', 'linux_driver', 'windows_driver'])
    out[key] = claimText(get(key), CLAIM_DRIVER);
  for (const key of ['linux_os_version', 'windows_os_version'])
    out[key] = claimText(get(key), CLAIM_OS_VERSION);
  return out;
}

// The guest OSes a run covered, in the order of ci_runs.platform.
export const guestsOf = (platform) =>
  platform === 'both'
    ? ['linux', 'windows']
    : platform === 'linux' || platform === 'windows'
      ? [platform]
      : [];

// ---------------------------------------------------------------------------------------------
// The result store (D1 `kayfabe-platform`, migration ci/platform-api/migrations/0002_ci_results.sql).
// Design: docs/design/V3_PLATFORM_TESTING.md §4/§8, docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md §3.
// Interface shared with the tier/orchestration code (contract of 2026-11-05):
//   recordRun(env, row)                       -> {stored, state, run_id, tree_hash, tree_kind}
//   requirementsMet(env, {commit_sha, tree_hash, tier, requirements})
//                                             -> {met, missing:[{family,die,platform,reason}], inflight:[run_id]}
// Rows are data. Every stored string is checked against a fixed pattern or enum, so no box text
// reaches the database; nothing read back from it is ever executed or interpreted as a command.
// Only the CI Worker writes these tables. A row counts for a gate only with a valid HMAC tag over
// its fields and stages (key: Worker secret ROW_TAG_KEY, never a run token).
// ---------------------------------------------------------------------------------------------
import { commitKey } from './treekey.js';
import {
  DIE,
  FAMILY,
  STAGE,
  familyOf,
  normalTier,
  parseRequirements,
  evaluate,
} from './requirements.js';

export const VERDICTS = Object.freeze(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED']);
export const STAGE_RESULTS = Object.freeze([
  'PASS',
  'FAIL',
  'SKIP',
  'NOT_RUN',
  'RETRY',
  'UNATTRIBUTED',
]);
export const LIMITS = Object.freeze({ stages: 64, rowBytes: 16384, scanRows: 1000 });

const RUN_ID = /^[a-z0-9][a-z0-9-]{5,63}$/;
const HEX40 = /^[0-9a-f]{40}$/;
const HEX64 = /^[0-9a-f]{64}$/;
const PROFILE = /^[a-z][a-z0-9_-]{0,31}$/;
const LOG_NAME = '[a-z0-9][a-z0-9._-]{0,63}\\.(?:log|txt)';
const DRIVER_KEY = /^[a-z][a-z0-9_]{0,31}$/;
const DRIVER = /^(?:unknown|[0-9]{1,4}(?:\.[0-9]{1,5}){1,3})$/;
const GPU_NAME = /^[A-Za-z0-9_. -]{1,100}$/;
// A run token is an HS256 JWT; anything token-shaped in a stored string refuses the row.
const TOKEN_SHAPED = /eyJ[A-Za-z0-9_-]{8,}/;

const bad = (field) => {
  throw new TypeError('BAD_ROW:' + field);
};
const encoder = new TextEncoder();

function isoTime(value, field, optional = false) {
  if (value === null || value === undefined) return optional ? null : bad(field);
  let ms;
  if (Number.isSafeInteger(value)) ms = value;
  else if (
    typeof value === 'string' &&
    /^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,3})?Z$/.test(value)
  )
    ms = Date.parse(value);
  else bad(field);
  if (!Number.isFinite(ms) || ms < Date.UTC(2020, 0, 1) || ms > Date.UTC(2100, 0, 1)) bad(field);
  return new Date(ms).toISOString().replace(/\.[0-9]{3}Z$/, 'Z');
}

const money = (value, field) => {
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 10000)
    bad(field);
  return value;
};

// Validate and normalise a contract row. Throws TypeError('BAD_ROW:<field>').
// options.verifiedTree: only the Worker's own ingest, with a key from computeTreeKey, may store a
// tree hash as kind 'tree'; any other caller-supplied hash is stored as 'claimed' (review of #258,
// L4) and never answers for another commit.
export async function normaliseRow(row, { verifiedTree = false } = {}) {
  if (row === null || typeof row !== 'object' || Array.isArray(row)) bad('row');
  let size;
  try {
    size = encoder.encode(JSON.stringify(row)).length;
  } catch {
    bad('row');
  }
  if (size > LIMITS.rowBytes) bad('size');
  const verdict = row.verdict ?? null;
  if (verdict !== null && !VERDICTS.includes(verdict)) bad('verdict');
  if (typeof row.run_id !== 'string' || !RUN_ID.test(row.run_id)) bad('run_id');
  if (typeof row.commit_sha !== 'string' || !HEX40.test(row.commit_sha)) bad('commit_sha');
  let treeHash = row.tree_hash ?? null,
    treeKind = row.tree_kind ?? (treeHash === null ? 'commit' : 'tree');
  if (!['tree', 'commit', 'claimed'].includes(treeKind)) bad('tree_kind');
  if (treeKind === 'tree' && !verifiedTree) treeKind = 'claimed';
  if (treeHash === null) {
    if (treeKind !== 'commit') bad('tree_hash');
    treeHash = await commitKey(row.commit_sha);
  } else if (typeof treeHash !== 'string' || !HEX64.test(treeHash)) bad('tree_hash');
  else if (treeKind === 'commit' && treeHash !== (await commitKey(row.commit_sha)))
    bad('tree_hash');
  const tier = normalTier(row.tier);
  if (tier === null) bad('tier');
  if (typeof row.profile !== 'string' || !PROFILE.test(row.profile)) bad('profile');
  if (typeof row.die !== 'string' || !DIE.test(row.die)) bad('die');
  const family = row.family ?? familyOf(row.die);
  if (typeof family !== 'string' || !FAMILY.test(family)) bad('family');
  if (familyOf(row.die) !== 'unknown' && family !== familyOf(row.die)) bad('family');
  if (!['linux', 'windows', 'both'].includes(row.platform)) bad('platform');
  const stages = row.stages ?? [];
  if (!Array.isArray(stages) || stages.length > LIMITS.stages) bad('stages');
  const names = new Set();
  const cleanStages = stages.map((s, ord) => {
    if (s === null || typeof s !== 'object' || Array.isArray(s)) bad('stage');
    if (typeof s.name !== 'string' || !STAGE.test(s.name) || names.has(s.name)) bad('stage.name');
    names.add(s.name);
    const rc = s.rc ?? null;
    if (rc !== null && (!Number.isSafeInteger(rc) || rc < -255 || rc > 255)) bad('stage.rc');
    const duration = s.duration_ms ?? null;
    if (
      duration !== null &&
      (!Number.isSafeInteger(duration) || duration < 0 || duration > 7 * 86400000)
    )
      bad('stage.duration_ms');
    if (!STAGE_RESULTS.includes(s.result)) bad('stage.result');
    return { ord, name: s.name, rc, duration_ms: duration, result: s.result };
  });
  // PASS needs every recorded stage to have passed or been skipped by design; RETRY and
  // UNATTRIBUTED are never a PASS (contract), so nothing else is inferred here.
  if (verdict === 'PASS' && cleanStages.some((s) => !['PASS', 'SKIP'].includes(s.result)))
    bad('verdict.stages');
  const gpuName = row.gpu_name ?? null;
  if (gpuName !== null && (typeof gpuName !== 'string' || !GPU_NAME.test(gpuName))) bad('gpu_name');
  const drivers = row.driver_versions ?? {};
  if (drivers === null || typeof drivers !== 'object' || Array.isArray(drivers)) bad('drivers');
  const driverEntries = Object.entries(drivers);
  if (
    driverEntries.length > 8 ||
    !driverEntries.every(([k, v]) => DRIVER_KEY.test(k) && typeof v === 'string' && DRIVER.test(v))
  )
    bad('driver_versions');
  const claims = cleanClaims(row.claims);
  const machine = row.machine_id ?? null;
  if (machine !== null && (!Number.isSafeInteger(machine) || machine <= 0)) bad('machine_id');
  const started = isoTime(row.started_at, 'started_at');
  const ended = isoTime(row.ended_at, 'ended_at', true);
  if (verdict === null ? ended !== null : ended === null) bad('ended_at');
  if (ended !== null && ended < started) bad('ended_at');
  const logKey = row.log_key ?? null;
  if (
    logKey !== null &&
    (typeof logKey !== 'string' ||
      !new RegExp('^runs/' + row.run_id + '/' + LOG_NAME + '$').test(logKey))
  )
    bad('log_key');
  const source = row.source ?? 'reproduced';
  if (source !== 'reproduced') bad('source'); // community rows are not built (owner ruling 2026-11-04)
  const out = {
    run_id: row.run_id,
    commit_sha: row.commit_sha,
    tree_hash: treeHash,
    tree_kind: treeKind,
    tier,
    profile: row.profile,
    family,
    die: row.die,
    platform: row.platform,
    verdict,
    started_at: started,
    ended_at: ended,
    sort_at: ended ?? started,
    log_key: logKey,
    source,
    stages: cleanStages,
    gpu_name: gpuName,
    driver_versions_json: JSON.stringify(Object.fromEntries(driverEntries.sort())),
    claims,
    machine_id: machine,
    price_usd_h: money(row.price_usd_h, 'price_usd_h'),
    cost_usd: money(row.cost_usd, 'cost_usd'),
  };
  for (const v of Object.values(out))
    if (typeof v === 'string' && TOKEN_SHAPED.test(v)) bad('token-shaped');
  // (claims were cleaned above: token-shaped text is already null there)
  return out;
}

// --- HMAC row tag ----------------------------------------------------------------------------

export const TAG_VERSION = 'kf-ci-run-v1';
const TAGGED = [
  'run_id',
  'commit_sha',
  'tree_hash',
  'tree_kind',
  'tier',
  'profile',
  'family',
  'die',
  'platform',
  'verdict',
  'source',
  'started_at',
  'ended_at',
];

async function hex(text) {
  const d = await crypto.subtle.digest('SHA-256', encoder.encode(text));
  return Array.from(new Uint8Array(d), (x) => x.toString(16).padStart(2, '0')).join('');
}

// canonical = TAG_VERSION LF field LF ... LF sha256(stage lines); a stage line is
// ord TAB name TAB rc TAB duration_ms TAB result LF ('-' for null).
export async function canonicalRun(row, stages) {
  const parts = [TAG_VERSION];
  for (const f of TAGGED) {
    const v = row[f];
    if (typeof v !== 'string' || v === '' || /[\n\r]/.test(v)) throw TypeError('tag field ' + f);
    parts.push(v);
  }
  const lines = [...stages]
    .sort((a, b) => a.ord - b.ord)
    .map((s) => [s.ord, s.name, s.rc ?? '-', s.duration_ms ?? '-', s.result].join('\t') + '\n')
    .join('');
  parts.push(await hex(lines));
  return parts.join('\n');
}

const validTagKey = (key) => typeof key === 'string' && key.length >= 32 && key.length <= 512;
async function hmacKey(secret, usage) {
  return crypto.subtle.importKey(
    'raw',
    encoder.encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    [usage],
  );
}
const base64url = (bytes) =>
  btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/, '');

export async function runTag(secret, row, stages) {
  if (!validTagKey(secret)) throw TypeError('ROW_TAG_KEY');
  const mac = await crypto.subtle.sign(
    'HMAC',
    await hmacKey(secret, 'sign'),
    encoder.encode(await canonicalRun(row, stages)),
  );
  return base64url(new Uint8Array(mac));
}

export async function verifyRunTag(secret, row, stages, tag) {
  if (!validTagKey(secret) || typeof tag !== 'string' || !/^[A-Za-z0-9_-]{43}$/.test(tag))
    return false;
  let canonical;
  try {
    canonical = await canonicalRun(row, stages);
  } catch {
    return false;
  }
  const mac = Uint8Array.from(atob(tag.replaceAll('-', '+').replaceAll('_', '/') + '='), (c) =>
    c.charCodeAt(0),
  );
  return crypto.subtle.verify(
    'HMAC',
    await hmacKey(secret, 'verify'),
    mac,
    encoder.encode(canonical),
  );
}

// --- statements ------------------------------------------------------------------------------

export const SQL = Object.freeze({
  existing: `SELECT run_id, commit_sha, tree_hash, tree_kind, tier, profile, family, die, platform,
      verdict, started_at, source, tag FROM ci_runs WHERE run_id = ?1`,
  insertRun: `INSERT OR IGNORE INTO ci_runs (run_id, commit_sha, tree_hash, tree_kind, tier,
      profile, family, die, platform, verdict, started_at, ended_at, sort_at, log_key, source, tag,
      recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)`,
  // Final verdicts never change: only an in-flight row with the same identity is finalised.
  finalizeRun: `UPDATE ci_runs SET verdict = ?2, ended_at = ?3, sort_at = ?4, log_key = ?5,
      tag = ?6, recorded_at = ?7
    WHERE run_id = ?1 AND verdict IS NULL AND commit_sha = ?8 AND tree_hash = ?9
      AND tree_kind = ?10 AND tier = ?11 AND profile = ?12 AND family = ?13 AND die = ?14
      AND platform = ?15 AND started_at = ?16 AND source = ?17`,
  // Stage, fact and private writes apply only to the row this call tagged (or, for an in-flight
  // call, to a row still in flight), so a losing concurrent writer changes nothing.
  clearStages: `DELETE FROM ci_run_stages WHERE run_id = ?1
    AND EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND tag = ?2)`,
  insertStage: `INSERT INTO ci_run_stages (run_id, ord, name, rc, duration_ms, result)
    SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND tag = ?7)`,
  upsertFacts: `INSERT INTO ci_host_facts (run_id, gpu_name, driver_versions_json)
    SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1
      AND ((?4 IS NULL AND verdict IS NULL) OR tag = ?4))
    ON CONFLICT (run_id) DO UPDATE SET gpu_name = excluded.gpu_name,
      driver_versions_json = excluded.driver_versions_json`,
  // One row per guest OS the run covered; only for the row this call tagged (a final call), and
  // only when the box reported at least one value. claimed = 1: reported by the box, not verified.
  upsertClaims: `INSERT INTO ci_run_claims (run_id, guest_os, guest_os_version, guest_driver,
      host_driver, cpu_vendor, cpu_model, cpu_arch, claimed)
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1 WHERE EXISTS (SELECT 1 FROM ci_runs
      WHERE run_id = ?1 AND tag = ?9)
    ON CONFLICT (run_id, guest_os) DO UPDATE SET guest_os_version = excluded.guest_os_version,
      guest_driver = excluded.guest_driver, host_driver = excluded.host_driver,
      cpu_vendor = excluded.cpu_vendor, cpu_model = excluded.cpu_model,
      cpu_arch = excluded.cpu_arch`,
  upsertPrivate: `INSERT INTO ci_runs_private (run_id, machine_id, price_usd_h, cost_usd)
    SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1
      AND ((?5 IS NULL AND verdict IS NULL) OR tag = ?5))
    ON CONFLICT (run_id) DO UPDATE SET machine_id = excluded.machine_id,
      price_usd_h = excluded.price_usd_h, cost_usd = excluded.cost_usd`,
  // Rows of one result key (?1) or of one commit (?2). Ordering in requirements.js uses the
  // tagged ended_at; this order only bounds the scan.
  keyRuns: `SELECT run_id, commit_sha, tree_hash, tree_kind, tier, profile, family, die, platform,
      verdict, source, started_at, ended_at, sort_at, tag
    FROM ci_runs WHERE (?1 IS NOT NULL AND tree_hash = ?1) OR (?2 IS NOT NULL AND commit_sha = ?2)
    ORDER BY started_at DESC, run_id DESC LIMIT ?3`,
  keyStages: `SELECT s.run_id, s.ord, s.name, s.rc, s.duration_ms, s.result
    FROM ci_run_stages s JOIN ci_runs r ON r.run_id = s.run_id
    WHERE (?1 IS NOT NULL AND r.tree_hash = ?1) OR (?2 IS NOT NULL AND r.commit_sha = ?2)
    ORDER BY s.run_id, s.ord`,
  // In-flight rows to close as UNATTRIBUTED: one run (?1), or every row started before ?2.
  staleRuns: `SELECT r.run_id, r.commit_sha, r.tree_hash, r.tree_kind, r.tier, r.profile, r.family,
      r.die, r.platform, r.started_at, r.source, f.gpu_name, f.driver_versions_json,
      p.machine_id, p.price_usd_h, p.cost_usd
    FROM ci_runs r LEFT JOIN ci_host_facts f ON f.run_id = r.run_id
      LEFT JOIN ci_runs_private p ON p.run_id = r.run_id
    WHERE r.verdict IS NULL AND r.tag IS NULL
      AND ((?1 IS NOT NULL AND r.run_id = ?1) OR (?1 IS NULL AND r.started_at < ?2))
    ORDER BY r.started_at LIMIT ?3`,
  snapshot: `INSERT OR IGNORE INTO ci_requirement_snapshots (sha256, base_ref, content, first_seen_at)
    VALUES (?1, ?2, ?3, ?4)`,
});

const IDENTITY = [
  'commit_sha',
  'tree_hash',
  'tree_kind',
  'tier',
  'profile',
  'family',
  'die',
  'platform',
  'started_at',
  'source',
];

// Record one run (idempotent on run_id). A row without a verdict is in flight; a final row
// (verdict + ended_at) finalises it once. A repeated identical final call is DUPLICATE; a final
// call that differs from a stored final row is CONFLICT and changes nothing.
export async function recordRun(env, row, { now = Date.now, verifiedTree = false } = {}) {
  const r = await normaliseRow(row, { verifiedTree });
  const result = (stored, state) => ({
    stored,
    state,
    run_id: r.run_id,
    tree_hash: r.tree_hash,
    tree_kind: r.tree_kind,
  });
  const db = env?.DB;
  if (!db || typeof db.prepare !== 'function') return result(false, 'NO_DB');
  if (!validTagKey(env.ROW_TAG_KEY)) return result(false, 'NO_TAG_KEY');
  const final = r.verdict !== null;
  const tag = final ? await runTag(env.ROW_TAG_KEY, r, r.stages) : null;
  const before = await db.prepare(SQL.existing).bind(r.run_id).first();
  const sameIdentity = before && IDENTITY.every((f) => before[f] === r[f]);
  if (before && !sameIdentity) return result(false, 'CONFLICT');
  if (before && before.verdict !== null)
    return result(false, final && before.tag === tag ? 'DUPLICATE' : 'CONFLICT');
  if (before && !final) return result(false, 'DUPLICATE');
  const at = new Date(now()).toISOString().replace(/\.[0-9]{3}Z$/, 'Z');
  const bind = (sql, ...values) => db.prepare(sql).bind(...values);
  const statements = [
    bind(
      SQL.insertRun,
      r.run_id,
      r.commit_sha,
      r.tree_hash,
      r.tree_kind,
      r.tier,
      r.profile,
      r.family,
      r.die,
      r.platform,
      r.verdict,
      r.started_at,
      r.ended_at,
      r.sort_at,
      r.log_key,
      r.source,
      tag,
      at,
    ),
  ];
  if (final) {
    statements.push(
      bind(
        SQL.finalizeRun,
        r.run_id,
        r.verdict,
        r.ended_at,
        r.sort_at,
        r.log_key,
        tag,
        at,
        ...IDENTITY.map((f) => r[f]),
      ),
      bind(SQL.clearStages, r.run_id, tag),
      ...r.stages.map((s) =>
        bind(SQL.insertStage, r.run_id, s.ord, s.name, s.rc, s.duration_ms, s.result, tag),
      ),
    );
  }
  if (final && CLAIM_KEYS.some((k) => r.claims[k] !== null))
    for (const os of guestsOf(r.platform))
      statements.push(
        bind(
          SQL.upsertClaims,
          r.run_id,
          os,
          r.claims[os + '_os_version'],
          r.claims[os + '_driver'],
          r.claims.host_driver,
          r.claims.cpu_vendor,
          r.claims.cpu_model,
          r.claims.cpu_arch,
          tag,
        ),
      );
  statements.push(
    bind(SQL.upsertFacts, r.run_id, r.gpu_name, r.driver_versions_json, tag),
    bind(SQL.upsertPrivate, r.run_id, r.machine_id, r.price_usd_h, r.cost_usd, tag),
  );
  await db.batch(statements);
  const after = await db.prepare(SQL.existing).bind(r.run_id).first();
  if (!after || (final ? after.tag !== tag : after.verdict !== null))
    return result(false, 'CONFLICT');
  return result(true, before ? 'FINALIZED' : 'INSERTED');
}

// Read the rows of one result key and evaluate the requirement blocks of `tier`.
// `requirements`: the BASE branch's ci/requirements.toml text, or its parsed {blocks}.
export async function requirementsMet(env, { commit_sha, tree_hash, tier, requirements, now }) {
  const at = now ?? Date.now();
  const wanted = normalTier(tier);
  if (!['small', 'middle', 'largest'].includes(wanted)) throw new TypeError('BAD_CHECK:tier');
  if (typeof commit_sha !== 'string' || !HEX40.test(commit_sha))
    throw new TypeError('BAD_CHECK:commit_sha');
  const key = tree_hash ?? (await commitKey(commit_sha));
  if (typeof key !== 'string' || !HEX64.test(key)) throw new TypeError('BAD_CHECK:tree_hash');
  let blocks;
  try {
    blocks = (typeof requirements === 'string' ? parseRequirements(requirements) : requirements)
      ?.blocks;
    if (!Array.isArray(blocks)) throw Error();
  } catch {
    return {
      met: false,
      missing: [{ family: 'any', die: 'any', platform: 'any', reason: 'REQUIREMENTS_INVALID' }],
      inflight: [],
    };
  }
  const db = env?.DB;
  if (!db || typeof db.prepare !== 'function') throw new TypeError('NO_DB');
  // Review of #258, H2: until the box deletes ignored files after cloning, a tree key does not
  // prove what a run tested, so only rows of this very commit count. TREE_KEY_CARRY_OVER='true'
  // (off by default) also counts other commits' rows under a Worker-verified tree key.
  const carry = env.TREE_KEY_CARRY_OVER === 'true';
  const scan = carry ? [key, commit_sha] : [null, commit_sha];
  const rows = (
    (
      await db
        .prepare(SQL.keyRuns)
        .bind(...scan, LIMITS.scanRows)
        .all()
    ).results ?? []
  ).filter(
    (r) => r.commit_sha === commit_sha || (carry && r.tree_hash === key && r.tree_kind === 'tree'),
  );
  const stageRows =
    (
      await db
        .prepare(SQL.keyStages)
        .bind(...scan)
        .all()
    ).results ?? [];
  const stages = new Map();
  for (const s of stageRows) {
    if (!stages.has(s.run_id)) stages.set(s.run_id, []);
    stages.get(s.run_id).push(s);
  }
  const checked = [];
  for (const row of rows) {
    const own = stages.get(row.run_id) ?? [];
    // A final row whose tag does not verify is TAMPERED (requirements.js), never skipped.
    const valid = row.verdict !== null && (await verifyRunTag(env.ROW_TAG_KEY, row, own, row.tag));
    // An in-flight row never carries a tag: one that does is a final row set back to "in flight"
    // (review of #258, re-verification), so it is TAMPERED, not joinable, and closeStale skips it.
    const tampered = (row.verdict !== null && !valid) || (row.verdict === null && row.tag !== null);
    checked.push({ ...row, stages: own, valid, tampered });
  }
  return evaluate(blocks, wanted, checked, at);
}

// Close in-flight rows as UNATTRIBUTED (review of #258, L1): one run (`runId`, when its queue
// entry is dropped) or every row in flight for longer than `olderThanMs`. Returns the count.
export async function closeStale(env, { now, runId = null, olderThanMs, limit = 4 }) {
  if (!env?.DB || !validTagKey(env.ROW_TAG_KEY)) return 0;
  const cutoff = new Date(now - olderThanMs).toISOString().replace(/\.[0-9]{3}Z$/, 'Z');
  const rows = (await env.DB.prepare(SQL.staleRuns).bind(runId, cutoff, limit).all()).results ?? [];
  let closed = 0;
  for (const r of rows) {
    let drivers = {};
    try {
      drivers = JSON.parse(r.driver_versions_json ?? '{}');
    } catch {}
    const result = await recordRun(
      env,
      {
        run_id: r.run_id,
        commit_sha: r.commit_sha,
        tree_hash: r.tree_hash,
        tree_kind: r.tree_kind,
        tier: r.tier,
        profile: r.profile,
        family: r.family,
        die: r.die,
        platform: r.platform,
        verdict: 'UNATTRIBUTED',
        stages: [],
        gpu_name: r.gpu_name ?? null,
        driver_versions: drivers,
        machine_id: r.machine_id ?? null,
        price_usd_h: r.price_usd_h ?? null,
        cost_usd: r.cost_usd ?? null,
        started_at: r.started_at,
        ended_at: Math.max(now, Date.parse(r.started_at)),
        source: r.source,
      },
      { now: () => now, verifiedTree: r.tree_kind === 'tree' },
    );
    if (result.stored) closed++;
  }
  return closed;
}

// Content-addressed copy of the BASE requirements text a check used (INSERT OR IGNORE).
export async function recordRequirementSnapshot(env, { base_ref, content, now = Date.now() }) {
  if (!env?.DB || typeof content !== 'string' || !['master', 'next'].includes(base_ref))
    return null;
  const digest = await hex(content);
  const at = new Date(now).toISOString().replace(/\.[0-9]{3}Z$/, 'Z');
  await env.DB.prepare(SQL.snapshot).bind(digest, base_ref, content, at).run();
  return digest;
}

// --- log redaction ---------------------------------------------------------------------------

// Exact-value redaction of run tokens over a chunked stream: a token split across two chunks is
// still caught, because the last (longest token - 1) characters are held back until the next
// chunk or the end. For the public log copy at log_key (no R2 writer exists yet).
export class TokenRedactor {
  constructor(tokens, replacement = '[redacted]') {
    this.tokens = [...new Set(tokens.filter((t) => typeof t === 'string' && t.length >= 8))].sort(
      (a, b) => b.length - a.length,
    );
    this.hold = Math.max(0, ...this.tokens.map((t) => t.length)) - 1;
    this.replacement = replacement;
    this.buffer = '';
  }
  #scrub(text) {
    for (const t of this.tokens) text = text.split(t).join(this.replacement);
    return text;
  }
  push(chunk) {
    let text = this.#scrub(this.buffer + chunk);
    if (this.hold <= 0) {
      this.buffer = '';
      return text;
    }
    // Hold back a tail that could be the start of a token.
    let cut = Math.max(0, text.length - this.hold);
    this.buffer = text.slice(cut);
    return text.slice(0, cut);
  }
  end() {
    const text = this.#scrub(this.buffer);
    this.buffer = '';
    return text;
  }
}
