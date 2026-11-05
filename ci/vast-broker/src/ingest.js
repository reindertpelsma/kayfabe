// Ledger -> result store. The Worker learns a run's final verdict in the ledger (src/index.js:
// the box result in report(), the close reasons in update()/kill/stop, settlement in ledger.js).
// This module never changes that verdict; it only records it in D1 (src/results.js), from a
// small queue in the ledger's storage that the cron sweep drains:
//   store:<run key>:<leg>  {attempts, inflight, tree?, record?}
// written when a rental is reserved (leg = provider retry number), and frozen with a snapshot of
// the old record when a provider-fault replacement reuses the run key. The sweep records an
// in-flight row once (so callers can join it) and the final row after settlement, then deletes
// the entry. With no D1 binding or no ROW_TAG_KEY nothing is queued or written.
import { recordRun, requirementsMet, recordRequirementSnapshot } from './results.js';
import { computeTreeKey, commitKey, treeSource } from './treekey.js';
import { tier as profileShape } from './tier.js';
import { normalTier } from './requirements.js';
import { rentalCost } from './ledger.js';
import { refuse } from './policy.js';

export const STORE_PREFIX = 'store:';
export const MAX_ATTEMPTS = 60; // about an hour of cron ticks
export const TREE_ATTEMPTS = 3; // then the per-commit key (no fall-through)
export const PER_TICK = 4;
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
  return normalTier(profile) ?? 'adhoc';
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
export function rowFromRecord(r, leg, tree, now) {
  const verdict = storeVerdict(r);
  const profile = r.profile ?? 'check';
  const drivers = {};
  if (version(r.hostDriverVersion)) drivers.host = r.hostDriverVersion;
  if (version(r.hardware?.driver_version)) drivers.guest = r.hardware.driver_version;
  const gpu = r.hardware?.gpu_name ?? r.gpu ?? null;
  const cost = rentalCost(r, r.ended ?? now);
  return {
    run_id: 'vast-' + r.key + '-' + leg,
    commit_sha: r.sha,
    tree_hash: tree.key,
    tree_kind: tree.kind,
    tier: storeTierOf(profile),
    profile,
    die: r.die,
    platform: profileShape(profile).windows ? 'both' : 'linux',
    verdict,
    stages: verdict === null ? [] : verdict === 'PASS' || verdict === 'FAIL' ? stagesOf(r) : [],
    gpu_name: typeof gpu === 'string' && /^[A-Za-z0-9_. -]{1,100}$/.test(gpu) ? gpu : null,
    driver_versions: drivers,
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

// Drain a few queue entries. `treeKey(sha)` -> {key, kind} | null (GitHub trees API).
export async function storeTick({ ledger, env, now, treeKey }) {
  const entries = await ledger.storage.list({ prefix: STORE_PREFIX, limit: 64 });
  let worked = 0;
  for (const [key, stored] of entries) {
    if (worked >= PER_TICK) break;
    const m = /^store:([1-9][0-9]{0,19}-[1-9][0-9]{0,19}):([0-9])$/.exec(key);
    const entry = { attempts: 0, inflight: false, ...stored };
    const record = m ? (entry.record ?? (await ledger.get('record:' + m[1]))) : null;
    if (!m || !record || (record.providerRetries ?? 0) !== Number(m[2])) {
      await ledger.storage.delete(key);
      event({ state: 'DROPPED', why: 'record' });
      continue;
    }
    const ended = record.ended !== null && record.ended !== undefined;
    if (!ended && entry.inflight) continue;
    if (ended && !record.id) {
      await ledger.storage.delete(key); // never created: no rental to record
      continue;
    }
    worked++;
    try {
      if (!entry.tree) {
        const tree = await treeKey(record.sha);
        if (tree) entry.tree = tree;
        else if ((entry.treeAttempts = (entry.treeAttempts ?? 0) + 1) >= TREE_ATTEMPTS)
          entry.tree = { key: await commitKey(record.sha), kind: 'commit' };
        else {
          await ledger.put(key, entry);
          continue;
        }
      }
      const result = await recordRun(env, rowFromRecord(record, Number(m[2]), entry.tree, now), {
        now: () => now,
      });
      if (!result.stored && !['DUPLICATE', 'CONFLICT'].includes(result.state))
        throw Error(result.state);
      event({ state: result.state, final: ended, kind: entry.tree.kind });
      if (ended) await ledger.storage.delete(key);
      else await ledger.put(key, { ...entry, inflight: true });
    } catch {
      entry.attempts++;
      if (entry.attempts >= MAX_ATTEMPTS) {
        await ledger.storage.delete(key);
        event({ state: 'DROPPED', why: 'attempts' });
      } else await ledger.put(key, entry);
    }
  }
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

// POST /v1/requirements/check {commit_sha, base: master|next, tier: small|middle|large}.
// Read-only towards rentals: it never reserves, rents or queues anything. The requirement text
// comes from the BASE branch named in the request (a protected branch), never from the commit.
export async function checkRequirements({ bytes, env, github, treeKey, cache, now }) {
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
    Object.keys(body).sort().join(',') !== 'base,commit_sha,tier' ||
    typeof body.commit_sha !== 'string' ||
    !/^[0-9a-f]{40}$/.test(body.commit_sha) ||
    !['master', 'next'].includes(body.base) ||
    !['small', 'middle', 'large', 'largest'].includes(body.tier)
  )
    refuse('BAD_REQUEST', 400);
  if (!storeEnabled(env)) refuse('STORE_UNAVAILABLE', 503);
  const file = await github.get('contents/ci/requirements.toml?ref=' + body.base, true, true);
  const text = file === null ? '' : decodeContent(file);
  let tree = cache.get(body.commit_sha);
  if (!tree) {
    tree = await treeKey(body.commit_sha);
    if (tree) {
      if (cache.size >= 32) cache.delete(cache.keys().next().value);
      cache.set(body.commit_sha, tree);
    } else tree = { key: await commitKey(body.commit_sha), kind: 'commit' };
  }
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
      snapshot = await recordRequirementSnapshot(env, { base_ref: body.base, content: text, now });
    } catch {}
  return {
    ...answer,
    tier: normalTier(body.tier),
    tree_hash: tree.key,
    tree_kind: tree.kind,
    requirements_sha256: snapshot,
  };
}
