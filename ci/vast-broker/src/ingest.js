// Ledger -> result store. The Worker learns a run's final verdict in the ledger (src/index.js:
// the box result in report(), the close reasons in update()/kill/stop, settlement in ledger.js).
// This module never changes that verdict; it only records it in D1 (src/results.js), from a
// small queue in the ledger's storage that the cron sweep drains:
//   store:<run key>:<leg>  {attempts, inflight, tree?, record?}
// written when a rental is reserved (leg = provider retry number), and frozen with a snapshot of
// the old record when a provider-fault replacement reuses the run key. The sweep records an
// in-flight row once (so callers can join it) and the final row after settlement, then deletes
// the entry. With no D1 binding or no ROW_TAG_KEY nothing is queued or written.
// Review of #258 (M1/H1): tree keys are computed OUTSIDE the ledger's serial queue
// (storeTrees, from the cron entry point) and cached per commit as treekey:<sha>; the serialized
// storeTick only reads that cache, saves its attempt counter before any work, and stops after
// TICK_BUDGET_MS.
import {
  recordRun,
  requirementsMet,
  recordRequirementSnapshot,
  closeStale,
  cleanClaims,
} from './results.js';
import { computeTreeKey, commitKey, treeSource } from './treekey.js';
import { tier as profileShape, familyOf as cfgFamilyOf } from './tier.js';
import { normalTier, INFLIGHT_MAX_MS } from './requirements.js';
import { rentalCost } from './ledger.js';
import { refuse, budgetClass, configuration } from './policy.js';
import { hardwareMatch } from './release.js';

export const STORE_PREFIX = 'store:';
export const MAX_ATTEMPTS = 60; // about an hour of cron ticks
export const TREE_ATTEMPTS = 3; // then the per-commit key (no fall-through)
export const PER_TICK = 4;
export const TICK_BUDGET_MS = 10000; // wall time of the store work inside one serial sweep
export const TREE_BUDGET_MS = 25000; // one tree-key computation, outside the serial queue
export const TREE_PREFIX = 'treekey:'; // treekey:<sha> {key, kind, at} | {kind:'pending', attempts, at}
export const TREE_RETENTION_MS = 7 * 86400000;
export const CHECK_PATH = '/v1/requirements/check';

export const storeEnabled = (env) =>
  !!env?.DB &&
  typeof env.DB.prepare === 'function' &&
  typeof env.ROW_TAG_KEY === 'string' &&
  env.ROW_TAG_KEY.length >= 32;

// The stored tier of a Worker profile: the master gate profile has the middle-tier shape
// (one box, Windows required); the owner's die sweep is outside the tiers.
export function storeTierOf(profile) {
  if (profile === 'check') return 'middle';
  if (profile === 'sweep') return 'adhoc';
  if (profile === 'release') return 'largest';
  return normalTier(profile) ?? 'adhoc';
}

// Review rev259 F7 (integration of #258 with #259): the record's own tier (set by admit() from
// the event or the admin start: small | middle | large | adhoc) wins; the budget class is the
// fallback for records written before the tier field existed. A release leg is stored as
// profile 'check' plus budgetClass 'release', so the profile alone would make it middle/adhoc.
export function storeTierOfRecord(r) {
  return normalTier(r.tier) ?? storeTierOf(budgetClass(r)); // normalTier keeps 'adhoc'
}

// Attribution for the store only; the ledger's verdict is untouched.
// PASS: a verified box result PASS. FAIL: a box result FAIL, or a box that reported and then hit
// a test-side limit (hard/heartbeat timeout, report limits). RETRY: a provider fault (the Worker
// may replace the box once). Everything else (no box report, owner stop, supersede, provider
// absent, rental refused) is UNATTRIBUTED, which never answers a requirement.
export function storeVerdict(r) {
  if (r.ended === null || r.ended === undefined) return null;
  if (r.verdict === 'PASS' && r.result?.verdict === 'PASS' && r.resultAt !== undefined)
    return 'PASS';
  if (r.verdict === 'PROVIDER_FAULT') return 'RETRY';
  if (r.verdict === 'FAIL' && r.result?.verdict === 'FAIL') return 'FAIL';
  if (
    r.verdict === 'FAIL' &&
    r.firstReportAt !== undefined &&
    ['HARD_TIMEOUT', 'HEARTBEAT_TIMEOUT', 'LIMIT', 'TTL'].includes(r.closed)
  )
    return 'FAIL';
  return 'UNATTRIBUTED';
}

export function stagesOf(r) {
  const suites = Array.isArray(r.result?.suites) ? r.result.suites.slice(0, 64) : [];
  return suites.map((s) => ({
    name: s.name,
    rc: null,
    duration_ms: null,
    result: s.fail > 0 ? 'FAIL' : s.pass > 0 && s.skip === 0 ? 'PASS' : 'SKIP',
  }));
}

const version = (v) =>
  typeof v === 'string' && /^(?:unknown|[0-9]{1,4}(?:\.[0-9]{1,5}){1,3})$/.test(v) ? v : null;

// The contract row of one leg of a ledger record.
// `cfg` (the Worker configuration) supplies the die rules for the hardware check.
export function rowFromRecord(r, leg, tree, now, cfg) {
  // Review rev259 F7/F3: a PASS from a box that reported another GPU than the record's die (or
  // no known GPU and driver) is not a PASS. The family falls back to the die's family for records
  // written before the family field existed; hardwareMatch still requires the die itself.
  let verdict = storeVerdict(r);
  if (
    verdict === 'PASS' &&
    (!cfg ||
      hardwareMatch(r, { die: r.die, family: r.family ?? cfgFamilyOf(r.die, cfg), cfg }) !==
        'MATCH')
  )
    verdict = 'UNATTRIBUTED';
  const profile = budgetClass(r); // 'release' for release legs (stored as check + budgetClass)
  const drivers = {};
  if (version(r.hostDriverVersion)) drivers.host = r.hostDriverVersion;
  if (version(r.hardware?.driver_version)) drivers.guest = r.hardware.driver_version;
  const gpu = r.hardware?.gpu_name ?? r.gpu ?? null;
  // Setup picker claims. The host driver is the driver the box's nvidia-smi reported (the machine
  // that runs kayfabe), never the provider's offer label (a KVM host's label is not a fact about
  // this setup). Everything else is the box's optional host_claims block. All are claims: they
  // group and filter public rows and decide nothing (the PASS cross-check above never reads them).
  const claims = cleanClaims({
    ...(r.result?.host_claims ?? {}),
    host_driver: version(r.hardware?.driver_version),
  });
  const cost = rentalCost(r, r.ended ?? now);
  return {
    run_id: 'vast-' + r.key + '-' + leg,
    commit_sha: r.sha,
    tree_hash: tree.key,
    tree_kind: tree.kind,
    tier: storeTierOfRecord(r),
    profile,
    die: r.die,
    platform: profileShape(profile).windows ? 'both' : 'linux',
    verdict,
    stages: verdict === null ? [] : verdict === 'PASS' || verdict === 'FAIL' ? stagesOf(r) : [],
    gpu_name: typeof gpu === 'string' && /^[A-Za-z0-9_. -]{1,100}$/.test(gpu) ? gpu : null,
    driver_versions: drivers,
    claims,
    machine_id: Number.isSafeInteger(r.machineId) && r.machineId > 0 ? r.machineId : null,
    price_usd_h: Number.isFinite(r.price) && r.price >= 0 ? r.price : null,
    cost_usd: Number.isFinite(cost) && cost >= 0 ? cost : null,
    started_at: r.started,
    ended_at: verdict === null ? null : r.ended,
    log_key: null,
  };
}

const entryKey = (record) => STORE_PREFIX + record.key + ':' + (record.providerRetries ?? 0);

export async function enqueue(ledger, record) {
  await ledger.put(entryKey(record), { attempts: 0, inflight: false });
}

// Before a replacement leg overwrites record:<key>, keep the old leg's final state.
export async function freeze(ledger, prior) {
  const { log, ...snapshot } = prior;
  const existing = (await ledger.get(entryKey(prior))) ?? { attempts: 0, inflight: false };
  await ledger.put(entryKey(prior), { ...existing, record: snapshot });
}

const event = (fields) => console.log(JSON.stringify({ event: 'store', ...fields }));
const finalTree = (t) =>
  t && (t.kind === 'tree' || t.kind === 'commit') && typeof t.key === 'string';

// Queue entries -> the commits whose tree key is still missing.
async function pendingShas(ledger) {
  const shas = [];
  for (const [key, stored] of await ledger.storage.list({ prefix: STORE_PREFIX, limit: 64 })) {
    const m = /^store:([1-9][0-9]{0,19}-[1-9][0-9]{0,19}):[0-9]$/.exec(key);
    const record = m ? (stored?.record ?? (await ledger.get('record:' + m[1]))) : null;
    if (record && /^[0-9a-f]{40}$/.test(record.sha ?? '') && !shas.includes(record.sha))
      shas.push(record.sha);
  }
  return shas;
}

async function withBudget(promise, ms) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((resolve) => (timer = setTimeout(() => resolve(null), ms))),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

// The cached result key of a commit, computing it (bounded in time) when it is missing. The
// attempt is recorded before the computation, so a computation that never returns still counts.
// After TREE_ATTEMPTS failures the per-commit key is stored (no fall-through).
export async function treeKeyFor({ ledger, sha, now, treeKey, budgetMs = TREE_BUDGET_MS }) {
  const cached = await ledger.get(TREE_PREFIX + sha);
  if (finalTree(cached)) return cached;
  const attempts = (cached?.attempts ?? 0) + 1;
  if (attempts > TREE_ATTEMPTS) {
    const fallback = { key: await commitKey(sha), kind: 'commit', at: now };
    await ledger.put(TREE_PREFIX + sha, fallback);
    return fallback;
  }
  await ledger.put(TREE_PREFIX + sha, { kind: 'pending', attempts, at: now });
  let tree = null;
  try {
    tree = await withBudget(treeKey(sha), budgetMs);
  } catch {}
  if (!tree) return null;
  const value = { key: tree.key, kind: tree.kind, at: now };
  await ledger.put(TREE_PREFIX + sha, value);
  return value;
}

// Outside the serial queue (cron entry point): compute at most one missing tree key, and prune
// old cache entries. Touches only treekey:<sha> entries, which nothing else writes.
export async function storeTrees({ ledger, env, now, treeKey }) {
  if (!storeEnabled(env)) return;
  for (const sha of await pendingShas(ledger)) {
    const cached = await ledger.get(TREE_PREFIX + sha);
    if (finalTree(cached)) continue;
    await treeKeyFor({ ledger, sha, now, treeKey });
    break;
  }
  let pruned = 0;
  for (const [key, value] of await ledger.storage.list({ prefix: TREE_PREFIX, limit: 256 }))
    if (pruned < 16 && !(now - value?.at < TREE_RETENTION_MS)) {
      await ledger.storage.delete(key);
      pruned++;
    }
}

async function drop(ledger, env, key, runId, now, why) {
  await ledger.storage.delete(key);
  event({ state: 'DROPPED', why });
  try {
    if (runId) await closeStale(env, { now, runId, olderThanMs: 0, limit: 1 });
  } catch {}
}

// Inside the serial sweep, after ledger.expire: record rows for queue entries whose tree key is
// cached, within TICK_BUDGET_MS, then close rows left in flight for longer than INFLIGHT_MAX_MS.
export async function storeTick({
  ledger,
  env,
  now,
  cfg,
  clock = Date.now,
  budgetMs = TICK_BUDGET_MS,
}) {
  const deadline = clock() + budgetMs;
  const entries = await ledger.storage.list({ prefix: STORE_PREFIX, limit: 64 });
  let worked = 0;
  for (const [key, stored] of entries) {
    if (worked >= PER_TICK || clock() >= deadline) break;
    const m = /^store:([1-9][0-9]{0,19}-[1-9][0-9]{0,19}):([0-9])$/.exec(key);
    const entry = { attempts: 0, inflight: false, ...stored };
    const runId = m ? 'vast-' + m[1] + '-' + m[2] : null;
    const record = m ? (entry.record ?? (await ledger.get('record:' + m[1]))) : null;
    if (!m || !record || (record.providerRetries ?? 0) !== Number(m[2])) {
      await drop(ledger, env, key, runId, now, 'record');
      continue;
    }
    const ended = record.ended !== null && record.ended !== undefined;
    if (!ended && entry.inflight) continue;
    if (ended && !record.id) {
      await ledger.storage.delete(key); // never created: no rental to record
      continue;
    }
    const tree = await ledger.get(TREE_PREFIX + record.sha);
    if (!finalTree(tree)) continue; // storeTrees computes it outside the queue
    worked++;
    // Count the attempt before any work: a call that never returns still counts.
    entry.attempts++;
    if (entry.attempts > MAX_ATTEMPTS) {
      await drop(ledger, env, key, runId, now, 'attempts');
      continue;
    }
    await ledger.put(key, entry);
    try {
      const result = await recordRun(
        env,
        rowFromRecord(
          record,
          Number(m[2]),
          tree,
          now,
          // Only a PASS needs the die rules (a configuration error throws: retried later).
          cfg ?? (storeVerdict(record) === 'PASS' ? configuration(env) : null),
        ),
        {
          now: () => now,
          verifiedTree: tree.kind === 'tree',
        },
      );
      if (!result.stored && !['DUPLICATE', 'CONFLICT'].includes(result.state))
        throw Error(result.state);
      event({ state: result.state, final: ended, kind: tree.kind });
      if (ended) await ledger.storage.delete(key);
      else await ledger.put(key, { ...entry, attempts: 0, inflight: true });
    } catch {
      event({ state: 'RETRY_LATER' });
    }
  }
  if (clock() < deadline)
    try {
      await closeStale(env, { now, olderThanMs: INFLIGHT_MAX_MS });
    } catch {}
}

// The tree key through the GitHub trees API, or null (fail closed).
export const githubTreeKey = (github) => (sha) => computeTreeKey(treeSource(github), sha);

function decodeContent(file) {
  if (file?.type !== 'file' || file.encoding !== 'base64' || typeof file.content !== 'string')
    return null;
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(
      Uint8Array.from(atob(file.content.replace(/\n/g, '')), (c) => c.charCodeAt(0)),
    );
  } catch {
    return null;
  }
}

// The BASE branch whose requirements apply, decided by the Worker, never by the caller (review of
// #258, L3): the default branch (master) for the middle and release tiers, next for small.
export const baseFor = (tier) => (normalTier(tier) === 'small' ? 'next' : 'master');

// POST /v1/requirements/check {commit_sha, tier: small|middle|large}. Runs outside the ledger's
// serial queue and is read-only towards rentals: it never reserves, rents or queues anything.
export async function checkRequirements({ bytes, env, github, ledger, treeKey, now }) {
  let body;
  try {
    body = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    refuse('BAD_REQUEST', 400);
  }
  if (
    body === null ||
    typeof body !== 'object' ||
    Array.isArray(body) ||
    Object.keys(body).sort().join(',') !== 'commit_sha,tier' ||
    typeof body.commit_sha !== 'string' ||
    !/^[0-9a-f]{40}$/.test(body.commit_sha) ||
    !['small', 'middle', 'large', 'largest'].includes(body.tier)
  )
    refuse('BAD_REQUEST', 400);
  if (!storeEnabled(env)) refuse('STORE_UNAVAILABLE', 503);
  const base = baseFor(body.tier);
  const file = await github.get('contents/ci/requirements.toml?ref=' + base, true, true);
  const text = file === null ? '' : decodeContent(file);
  const tree = (await treeKeyFor({ ledger, sha: body.commit_sha, now, treeKey })) ?? {
    key: await commitKey(body.commit_sha),
    kind: 'commit',
  };
  const answer = await requirementsMet(env, {
    commit_sha: body.commit_sha,
    tree_hash: tree.key,
    tier: body.tier,
    requirements: text ?? '',
    now,
  });
  let snapshot = null;
  if (typeof text === 'string' && text !== '')
    try {
      snapshot = await recordRequirementSnapshot(env, { base_ref: base, content: text, now });
    } catch {}
  return {
    ...answer,
    tier: normalTier(body.tier),
    base,
    tree_hash: tree.key,
    tree_kind: tree.kind,
    requirements_sha256: snapshot,
  };
}
