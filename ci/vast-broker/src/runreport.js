// The structured run report (2026-11-07; contract /root/night/REPORT_CONTRACT.md plus the host
// sessions addendum; box side: scripts/ci/vast_run_report.py; docs/BOX_PROTOCOL.md "Run report").
//
// The box builds report.json (schema 1) from output the repository already produces and POSTs it
// to POST /run-report/<run>-<attempt> while its run token is live. Everything in it is a CLAIM of
// an untrusted box: this module keeps the SHAPE and drops the rest. Unknown keys are dropped,
// strings are printable ASCII and bounded, numbers are finite, arrays are capped, a row that names
// no known subrun is dropped, anything token-shaped or about SPECviewperf is dropped (SPEC scores
// are never public, docs/SPEC_RESULTS.md), and the whole document stays within 256 KiB. The
// IDENTITY of the run (id, commit, tree, tier, die, family, verdict, times, GPU, host driver) is
// never taken from the box: the Worker sets it from its own row when it publishes. The verdict of
// the run never depends on any of this, and a report never gates anything.
import { cleanClaims } from './results.js';
import { redactPublic } from './publiclog.js';

export const REPORT_MAX_BYTES = 262144;
export const LIMITS = Object.freeze({
  tests: 2000,
  apps: 500,
  perf: 200,
  stages: 64,
  suites: 20,
  subruns: 64,
  probes: 8,
  subStages: 16,
});
const ISO = /^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,3})?Z$/;
const STAGE = /^[a-z][a-z0-9_]{0,47}$/;
const NAME = /^[A-Za-z0-9][A-Za-z0-9 _.:+/=,()#@?-]{0,119}$/;
const UNIT = /^[A-Za-z0-9%/_. -]{1,16}$/;
const DRIVER = /^[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?$/;
const SUB_ID =
  /^(?:host|(?:linux|windows)(?:-(?:unknown|[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?))?)$/;
// Anything that could carry a secret or an identifier, and the data that is never public.
const TOKENISH = /eyJ[A-Za-z0-9_-]{8,}|:\/\/|[0-9a-f]{40,}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}/i;
const FORBIDDEN = /specviewperf|spec[ _-]?viewperf|viewperf/i;

export const TEST_RESULTS = Object.freeze(['PASS', 'FAIL', 'SKIP', 'REFUSED']);
export const STAGE_RESULTS = Object.freeze([
  'PASS',
  'FAIL',
  'SKIP',
  'NOT_RUN',
  'RETRY',
  'UNATTRIBUTED',
]);
export const SUB_VERDICTS = Object.freeze(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED', 'NOT_RUN']);
export const APP_RESULTS = Object.freeze(['PASS', 'FAIL', 'NOT_RUN']);
export const SUB_KINDS = Object.freeze({
  host_bare: 'host',
  linux_guest: 'linux',
  windows_guest: 'windows',
});

const object = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
const own = (o, k) => Object.hasOwn(o, k);

// Printable ASCII, bounded; null for anything token-shaped, URL-shaped, or about SPECviewperf.
export function text(value, max = 200) {
  if (typeof value !== 'string') return null;
  const t = value
    .replace(/[^\x20-\x7e]/g, '?')
    .trim()
    .slice(0, max);
  if (t === '' || TOKENISH.test(t) || FORBIDDEN.test(t)) return null;
  return t;
}
const named = (value, pattern, max = 120) => {
  const t = text(value, max);
  return t !== null && pattern.test(t) ? t : null;
};
const int = (value, lo, hi) =>
  Number.isSafeInteger(value) && value >= lo && value <= hi ? value : null;
const number = (value) =>
  typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= 1e15 ? value : null;
const oneOf = (value, list) => (list.includes(value) ? value : null);
const iso = (value) => {
  if (typeof value !== 'string' || !ISO.test(value)) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) && ms >= Date.UTC(2020, 0, 1) && ms < Date.UTC(2100, 0, 1)
    ? new Date(ms).toISOString().replace(/\.[0-9]{3}Z$/, 'Z')
    : null;
};
const list = (value, cap) => (Array.isArray(value) ? value.slice(0, cap) : []);

function cleanSubrun(raw, ord) {
  if (!object(raw)) return null;
  const sub = typeof raw.sub === 'string' && SUB_ID.test(raw.sub) ? raw.sub : null;
  const kind = own(SUB_KINDS, raw.kind) ? raw.kind : null;
  if (sub === null || kind === null) return null;
  const os = SUB_KINDS[kind];
  // The id names the OS, and the OS follows from the kind: a subrun that contradicts itself is dropped.
  if (raw.os !== os || (os === 'host' ? sub !== 'host' : !sub.startsWith(os))) return null;
  const guest =
    raw.guest_driver === null || raw.guest_driver === undefined ? null : raw.guest_driver;
  const guestDriver = typeof guest === 'string' && DRIVER.test(guest) ? guest : null;
  const hostDriver =
    typeof raw.host_driver === 'string' && DRIVER.test(raw.host_driver) ? raw.host_driver : null;
  // `windows-580.88` must carry the driver of its id.
  if (/^(?:linux|windows)-[0-9]/.test(sub) && sub.slice(sub.indexOf('-') + 1) !== guestDriver)
    return null;
  const verdict = oneOf(raw.verdict, SUB_VERDICTS);
  if (verdict === null) return null;
  let reason = text(raw.reason, 200);
  if (verdict === 'NOT_RUN' && reason === null) reason = 'not reported';
  const counts = object(raw.counts) ? raw.counts : {};
  return {
    ord,
    sub,
    kind,
    os,
    guest_driver: guestDriver,
    host_driver: hostDriver,
    verdict,
    reason,
    stages: list(raw.stages, LIMITS.subStages)
      .map((s) => named(s, STAGE, 48))
      .filter((s) => s !== null),
    // Stage markers whose position in the published log gives this subrun's log_range; used by the
    // Worker, then dropped from the document.
    log_stages: list(raw.log_stages, 4)
      .map((s) => named(s, STAGE, 48))
      .filter((s) => s !== null),
    started_at: iso(raw.started_at),
    ended_at: iso(raw.ended_at),
    // Sum of the measured test times of the subrun (the payload prints no timestamps).
    duration_ms: int(raw.duration_ms, 0, 7 * 86400000),
    counts: {
      pass: int(counts.pass, 0, 1e8) ?? 0,
      fail: int(counts.fail, 0, 1e8) ?? 0,
      skip: int(counts.skip, 0, 1e8) ?? 0,
    },
  };
}

// The box's claims, shaped. Returns the claim document (no identity) or null for a body that is
// not a report of schema 1 at all.
export function cleanClaimsReport(raw) {
  if (!object(raw) || raw.schema !== 1) return null;
  const subruns = [];
  const ids = new Set();
  for (const s of list(raw.subruns, LIMITS.subruns)) {
    const c = cleanSubrun(s, subruns.length);
    if (c && !ids.has(c.sub)) {
      ids.add(c.sub);
      subruns.push(c);
    }
  }
  const stages = [];
  const stageNames = new Set();
  for (const s of list(raw.stages, LIMITS.stages)) {
    if (!object(s)) continue;
    const name = named(s.name, STAGE, 48);
    const result = oneOf(s.result, STAGE_RESULTS);
    if (name === null || result === null || stageNames.has(name)) continue;
    stageNames.add(name);
    stages.push({
      name,
      result,
      rc: int(s.rc, -255, 255),
      duration_ms: int(s.duration_ms, 0, 7 * 86400000),
    });
  }
  const suites = [];
  const suiteNames = new Set();
  for (const s of list(raw.suites, LIMITS.suites)) {
    if (!object(s)) continue;
    const name = named(s.name, STAGE, 48);
    if (name === null || suiteNames.has(name) || FORBIDDEN.test(name)) continue;
    suiteNames.add(name);
    suites.push({
      name,
      pass: int(s.pass, 0, 1e8) ?? 0,
      fail: int(s.fail, 0, 1e8) ?? 0,
      skip: int(s.skip, 0, 1e8) ?? 0,
    });
  }
  const tests = [];
  for (const t of list(raw.tests, LIMITS.tests)) {
    if (!object(t) || typeof t.sub !== 'string' || !ids.has(t.sub)) continue;
    const suite = named(t.suite, STAGE, 48);
    const name = named(t.name, NAME);
    const result = oneOf(t.result, TEST_RESULTS);
    if (suite === null || name === null || result === null) continue;
    tests.push({ sub: t.sub, suite, name, result, ms: int(t.ms, 0, 7 * 86400000) });
  }
  const apps = [];
  for (const a of list(raw.apps, LIMITS.apps)) {
    if (!object(a) || typeof a.sub !== 'string' || !ids.has(a.sub)) continue;
    const name = named(a.name, NAME);
    const result = oneOf(a.result, APP_RESULTS);
    const os = oneOf(a.os, ['linux', 'windows']);
    if (name === null || result === null || os === null) continue;
    apps.push({ sub: a.sub, name, os, result, note: text(a.note, 200) ?? '' });
  }
  const perf = [];
  for (const p of list(raw.perf, LIMITS.perf)) {
    if (!object(p) || typeof p.sub !== 'string' || !ids.has(p.sub)) continue;
    const name = named(p.name, NAME);
    const value = number(p.value);
    const unit =
      typeof p.unit === 'string' && UNIT.test(p.unit) && !FORBIDDEN.test(p.unit) ? p.unit : null;
    const suite = named(p.suite, STAGE, 48);
    if (name === null || value === null || unit === null || suite === null) continue;
    perf.push({
      sub: p.sub,
      name,
      value,
      unit,
      higher_is_better: p.higher_is_better === true,
      suite,
      ref_value: number(p.ref_value),
      ref_ratio: number(p.ref_ratio),
    });
  }
  const w = object(raw.windows) ? raw.windows : {};
  const guests = object(raw.guest_drivers) ? raw.guest_drivers : {};
  return {
    schema: 1,
    guest_drivers: {
      linux: typeof guests.linux === 'string' && DRIVER.test(guests.linux) ? guests.linux : null,
      windows:
        typeof guests.windows === 'string' && DRIVER.test(guests.windows) ? guests.windows : null,
    },
    stages,
    suites,
    subruns,
    tests,
    apps,
    perf,
    windows: {
      gate: oneOf(w.gate, ['PASS', 'FAIL', 'RETRY']),
      install_s: int(w.install_s, 0, 7 * 86400),
      probes: list(w.probes, LIMITS.probes)
        .map((s) => named(s, /^[a-z][a-z0-9_]{0,23}$/, 24))
        .filter((s) => s !== null),
      resets: int(w.resets, 0, 100000),
      live_dumps_193: int(w.live_dumps_193, 0, 100000),
    },
  };
}

// The report goes through the same redaction as the log (review of #282, F1): every string VALUE of
// the document (keys are fixed words) is run over the Worker's exact values and the secret shapes,
// so a payload error string naming the secret path, the Access pair, a nonce or a password-like
// assignment never reaches report.json, D1 or the public API.
export function redactDoc(value, exact) {
  if (typeof value === 'string') return redactPublic(value, exact);
  if (Array.isArray(value)) return value.map((v) => redactDoc(v, exact));
  if (value !== null && typeof value === 'object')
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, redactDoc(v, exact)]));
  return value;
}

// Serialises a claim document so that it stays within the limit: the largest arrays shrink first,
// and the names of the arrays that were cut are listed in `truncated`.
export function fitReport(doc, max = REPORT_MAX_BYTES) {
  const size = (d) => new TextEncoder().encode(JSON.stringify(d)).length;
  const cut = [];
  let out = doc;
  for (const key of ['tests', 'apps', 'perf']) {
    while (size(out) > max && out[key].length > 0) {
      out = { ...out, [key]: out[key].slice(0, Math.floor(out[key].length / 2)) };
      if (!cut.includes(key)) cut.push(key);
    }
  }
  if (size(out) > max) return null;
  return cut.length ? { ...out, truncated: cut } : out;
}

// The published document: the box's claims with the Worker's own identity. `row` is the final
// ci_runs row (+ host facts), `log` the identity of the published log (or null), `offsets` the
// stage-marker positions found in that log.
export function buildReport({ claims, row, runId, log, offsets, logsBase, offerFacts = null }) {
  const subruns = claims.subruns.map(({ ord, log_stages, ...s }) => {
    let range = null;
    if (offsets && log_stages.length) {
      const hits = log_stages.map((n) => offsets.get(n)).filter(Boolean);
      if (hits.length)
        range = {
          start_byte: Math.min(...hits.map((h) => h.start_byte)),
          end_byte: Math.max(...hits.map((h) => h.end_byte)),
        };
    }
    const span =
      s.started_at && s.ended_at ? Date.parse(s.ended_at) - Date.parse(s.started_at) : null;
    return {
      ...s,
      duration_ms:
        Number.isSafeInteger(span) && span >= 0 && span <= 7 * 86400000 ? span : s.duration_ms,
      log_range: range,
    };
  });
  return {
    schema: 1,
    run_id: runId,
    offer_facts: offerFacts,
    commit: row.commit_sha,
    tree: row.tree_kind === 'tree' ? row.tree_hash : null,
    tier: row.tier,
    die: row.die,
    family: row.family,
    gpu_name: row.gpu_name ?? null,
    host_driver: row.host_driver ?? null,
    guest_drivers: claims.guest_drivers,
    started_at: row.started_at,
    ended_at: row.ended_at,
    verdict: row.verdict,
    stages: claims.stages,
    suites: claims.suites,
    subruns,
    tests: claims.tests,
    apps: claims.apps,
    perf: claims.perf,
    windows: claims.windows,
    ...(claims.truncated ? { truncated: claims.truncated } : {}),
    log: log
      ? {
          ...(logsBase ? { url: logsBase + '/' + runId + '/log.txt' } : {}),
          bytes: log.bytes,
          sha256: log.sha256,
        }
      : null,
  };
}

// ---- D1 --------------------------------------------------------------------------------------

export const SQL = Object.freeze({
  row: `SELECT r.run_id, r.commit_sha, r.tree_hash, r.tree_kind, r.tier, r.family, r.die,
      r.verdict, r.started_at, r.ended_at, f.gpu_name, f.driver_versions_json
    FROM ci_runs r LEFT JOIN ci_host_facts f ON f.run_id = r.run_id WHERE r.run_id = ?1`,
  dropTests: `DELETE FROM ci_run_tests WHERE run_id = ?1`,
  dropPerf: `DELETE FROM ci_run_perf WHERE run_id = ?1`,
  dropApps: `DELETE FROM ci_run_apps WHERE run_id = ?1`,
  dropSubruns: `DELETE FROM ci_subruns WHERE run_id = ?1`,
  dropReport: `DELETE FROM ci_run_reports WHERE run_id = ?1`,
  // Rows are written only for a FINAL run (the guard repeats in every statement).
  insertReport: `INSERT INTO ci_run_reports (run_id, schema_version, report_json, host_driver,
      log_bytes, log_sha256, recorded_at)
    SELECT ?1, 1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS
      (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND verdict IS NOT NULL)`,
  insertSubruns: `INSERT INTO ci_subruns (run_id, sub, ord, kind, os, guest_driver, host_driver,
      verdict, reason, stages_json, pass_count, fail_count, skip_count, started_at, ended_at,
      duration_ms, log_start, log_end)
    SELECT ?1, json_extract(j.value, '$.sub'), CAST(j.key AS INTEGER),
           json_extract(j.value, '$.kind'), json_extract(j.value, '$.os'),
           json_extract(j.value, '$.guest_driver'), json_extract(j.value, '$.host_driver'),
           json_extract(j.value, '$.verdict'), json_extract(j.value, '$.reason'),
           json_extract(j.value, '$.stages'), json_extract(j.value, '$.counts.pass'),
           json_extract(j.value, '$.counts.fail'), json_extract(j.value, '$.counts.skip'),
           json_extract(j.value, '$.started_at'), json_extract(j.value, '$.ended_at'),
           json_extract(j.value, '$.duration_ms'), json_extract(j.value, '$.log_range.start_byte'),
           json_extract(j.value, '$.log_range.end_byte')
    FROM json_each(?2) j
    WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND verdict IS NOT NULL)`,
  insertTests: `INSERT INTO ci_run_tests (run_id, sub, ord, suite, name, result, ms)
    SELECT ?1, json_extract(j.value, '$.sub'), CAST(j.key AS INTEGER),
           json_extract(j.value, '$.suite'), json_extract(j.value, '$.name'),
           json_extract(j.value, '$.result'), json_extract(j.value, '$.ms')
    FROM json_each(?2) j
    WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND verdict IS NOT NULL)`,
  insertPerf: `INSERT INTO ci_run_perf (run_id, sub, ord, name, value, unit, higher_is_better,
      suite, ref_value, ref_ratio)
    SELECT ?1, json_extract(j.value, '$.sub'), CAST(j.key AS INTEGER),
           json_extract(j.value, '$.name'), json_extract(j.value, '$.value'),
           json_extract(j.value, '$.unit'), json_extract(j.value, '$.higher_is_better'),
           json_extract(j.value, '$.suite'), json_extract(j.value, '$.ref_value'),
           json_extract(j.value, '$.ref_ratio')
    FROM json_each(?2) j
    WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND verdict IS NOT NULL)`,
  insertApps: `INSERT INTO ci_run_apps (run_id, sub, ord, name, os, result, note)
    SELECT ?1, json_extract(j.value, '$.sub'), CAST(j.key AS INTEGER),
           json_extract(j.value, '$.name'), json_extract(j.value, '$.os'),
           json_extract(j.value, '$.result'), json_extract(j.value, '$.note')
    FROM json_each(?2) j
    WHERE EXISTS (SELECT 1 FROM ci_runs WHERE run_id = ?1 AND verdict IS NOT NULL)`,
  stored: `SELECT run_id FROM ci_run_reports WHERE run_id = ?1`,
});

// The final public row of a run, or null (missing, or still in flight).
export async function readRunRow(env, runId) {
  const row = await env.DB.prepare(SQL.row).bind(runId).first();
  if (!row || row.verdict === null || row.verdict === undefined) return null;
  let host = null;
  try {
    const drivers = JSON.parse(row.driver_versions_json ?? '{}');
    host = typeof drivers?.host === 'string' && DRIVER.test(drivers.host) ? drivers.host : null;
  } catch {}
  return { ...row, host_driver: host };
}

// One atomic batch: the previous rows of this run are replaced, never mixed with the new ones.
// `doc` may be null (no box report: only the log's identity is recorded). Returns 'STORED' or
// 'ROW_MISSING'. Throws on a D1 error (the caller fails open).
export async function storeReport(env, { runId, doc, hostDriver, log, now }) {
  const db = env.DB;
  const at = new Date(now).toISOString().replace(/\.[0-9]{3}Z$/, 'Z');
  const bind = (sql, ...v) => db.prepare(sql).bind(...v);
  const json = doc === null ? null : JSON.stringify(doc);
  const statements = [
    bind(SQL.dropTests, runId),
    bind(SQL.dropPerf, runId),
    bind(SQL.dropApps, runId),
    bind(SQL.dropSubruns, runId),
    bind(SQL.dropReport, runId),
    bind(
      SQL.insertReport,
      runId,
      json,
      hostDriver ?? null,
      log?.bytes ?? null,
      log?.sha256 ?? null,
      at,
    ),
  ];
  if (doc !== null) {
    const rows = (key, mapRow = (x) => x) => JSON.stringify(doc[key].map(mapRow));
    statements.push(bind(SQL.insertSubruns, runId, rows('subruns')));
    if (doc.tests.length) statements.push(bind(SQL.insertTests, runId, rows('tests')));
    if (doc.perf.length)
      statements.push(
        bind(
          SQL.insertPerf,
          runId,
          rows('perf', (p) => ({ ...p, higher_is_better: p.higher_is_better ? 1 : 0 })),
        ),
      );
    if (doc.apps.length) statements.push(bind(SQL.insertApps, runId, rows('apps')));
  }
  await db.batch(statements);
  const after = await db.prepare(SQL.stored).bind(runId).first();
  return after ? 'STORED' : 'ROW_MISSING';
}

// Claims the Worker already holds about the same guests (cleaned by results.js), used when the box
// sent no driver of its own.
export const claimDrivers = (claims) => {
  const c = cleanClaims(claims ?? {});
  return { linux: c.linux_driver, windows: c.windows_driver };
};
