// /v1/ci/subruns, /v1/ci/stats and the report fields of /v1/ci/runs/<id> (migration 0004).
// Read-only: fixed statements through db.js, no I/O of its own. Everything read back from D1 is a
// CLAIM of the test box (the CI Worker bounded it on the way in) and is re-bounded here on the way
// out, so a row that somehow violated the CI Worker's checks still cannot put free text, a token
// shape or a SPECviewperf datum on a public page. Ranking: src/rank.js.
import { ApiError, PATTERNS } from './validate.js';
import { DIMS, parseCpu, rank } from './rank.js';
import { familyOfDie } from './dies.js';

export const PROBE_ROWS = 60;
export const MAX_PROBES = 9;
export const DEFAULT_SUBRUN_LIMIT = 20;
export const MAX_SUBRUN_LIMIT = 100;
export const DEFAULT_STATS_LIMIT = 20;
export const INCLUDES = Object.freeze(['stages', 'tests', 'perf', 'apps', 'counts']);
// Rows per expanded subrun; the totals shrink the per-subrun cap as the page grows (bounded body).
const TEST_BUDGET = 1000;
const PERF_BUDGET = 500;

const STR = /^[\x20-\x7e]{0,200}$/;
const text = (v, max = 200) => (typeof v === 'string' && v.length <= max && STR.test(v) ? v : null);
const int = (v, max = 2 ** 40) => (Number.isSafeInteger(v) && v >= 0 && v <= max ? v : null);
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const DRIVER_OUT = /^[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?$/;
const driver = (v) => (typeof v === 'string' && DRIVER_OUT.test(v) ? v : null);
const TOKENISH = /eyJ[A-Za-z0-9_-]{8,}|:\/\/|[0-9a-f]{40,}/;
// Free text from a box: printable ASCII, bounded, nothing token-shaped, never SPECviewperf.
const safe = (v, max = 200) => {
  const t = text(v, max);
  return t === null || TOKENISH.test(t) || /specviewperf/i.test(t) ? null : t;
};
const RESULTS = new Set(['PASS', 'FAIL', 'SKIP', 'REFUSED', 'NOT_RUN', 'RETRY', 'UNATTRIBUTED']);
const SUB_VERDICTS = new Set(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED', 'NOT_RUN']);
const KINDS = new Set(['host_bare', 'linux_guest', 'windows_guest']);
const OSES = new Set(['linux', 'windows', 'host']);
const SUB_ID =
  /^(?:host|(?:linux|windows)(?:-(?:unknown|[0-9]{3,4}\.[0-9]{1,5}(?:\.[0-9]{1,5})?))?)$/;

export function subrunOut(row) {
  const stages = (() => {
    try {
      const v = JSON.parse(row.stages_json ?? '[]');
      return Array.isArray(v)
        ? v.filter((s) => typeof s === 'string' && /^[a-z][a-z0-9_]{0,47}$/.test(s)).slice(0, 16)
        : [];
    } catch {
      return [];
    }
  })();
  const start = int(row.log_start, 2 ** 31);
  const end = int(row.log_end, 2 ** 31);
  return {
    sub: SUB_ID.test(row.sub) ? row.sub : null,
    kind: KINDS.has(row.kind) ? row.kind : null,
    os: OSES.has(row.os) ? row.os : null,
    guest_driver: driver(row.guest_driver),
    host_driver: driver(row.host_driver),
    verdict: SUB_VERDICTS.has(row.verdict) ? row.verdict : null,
    reason: safe(row.reason),
    stages,
    counts: {
      pass: int(row.pass_count, 1e8) ?? 0,
      fail: int(row.fail_count, 1e8) ?? 0,
      skip: int(row.skip_count, 1e8) ?? 0,
    },
    started_at: PATTERNS.time.test(row.started_at ?? '') ? row.started_at : null,
    ended_at: PATTERNS.time.test(row.ended_at ?? '') ? row.ended_at : null,
    duration_ms: int(row.duration_ms, 604800000),
    log_range:
      start !== null && end !== null && end >= start ? { start_byte: start, end_byte: end } : null,
  };
}

// A stored report (JSON text), or null. The CI Worker bounded and sanitised it; the public Worker
// parses it as data, requires schema 1 and an object, and passes it on (it never evaluates it).
export function reportOut(textValue, id, logInfo) {
  let doc = null;
  try {
    doc = typeof textValue === 'string' ? JSON.parse(textValue) : null;
  } catch {
    doc = null;
  }
  if (doc === null || typeof doc !== 'object' || Array.isArray(doc) || doc.schema !== 1)
    return null;
  const clean = JSON.parse(JSON.stringify(doc));
  clean.run_id = id;
  clean.log = logInfo;
  return clean;
}

export const testOut = (r) => ({
  sub: text(r.sub, 40),
  suite: safe(r.suite, 48),
  name: safe(r.name, 120),
  result: RESULTS.has(r.result) ? r.result : null,
  ms: int(r.ms, 604800000),
});
export const perfOut = (r) => ({
  sub: text(r.sub, 40),
  name: safe(r.name, 120),
  value: num(r.value),
  unit: safe(r.unit, 16),
  higher_is_better: r.higher_is_better === 1,
  suite: safe(r.suite, 48),
  ref_value: num(r.ref_value),
  ref_ratio: num(r.ref_ratio),
});
export const appOut = (r) => ({
  sub: text(r.sub, 40),
  name: safe(r.name, 120),
  os: r.os === 'linux' || r.os === 'windows' ? r.os : null,
  result: ['PASS', 'FAIL', 'NOT_RUN'].includes(r.result) ? r.result : null,
  note: safe(r.note),
});

// ---- parameters ----------------------------------------------------------------------------

function dimensions(name, value) {
  if (value === null) return null;
  if (value === 'none') return [];
  const words = value.split(',');
  const unknown = words.some((w) => !DIMS.includes(w));
  if (unknown)
    throw new ApiError(400, 'bad_request', 'unknown dimension', { parameter: name, allowed: DIMS });
  if (new Set(words).size !== words.length)
    throw new ApiError(400, 'bad_request', 'repeated dimension', {
      parameter: name,
      allowed: DIMS,
    });
  return words;
}

function words(name, value, allowed) {
  if (value === null) return null;
  const list = value.split(',');
  if (list.some((w) => !allowed.includes(w)))
    throw new ApiError(400, 'bad_request', 'unknown word', { parameter: name, allowed });
  if (new Set(list).size !== list.length)
    throw new ApiError(400, 'bad_request', 'repeated word', { parameter: name, allowed });
  return list;
}

function listLimit(value, fallback) {
  if (value === null) return fallback;
  const n = Number(value);
  if (!Number.isSafeInteger(n) || n < 1 || n > MAX_SUBRUN_LIMIT)
    throw new ApiError(400, 'bad_request', 'invalid limit', { allowed: ['1-' + MAX_SUBRUN_LIMIT] });
  return n;
}

// The effective dimension sets of a request: `require` is exact, `ignore` is dropped, `relax` is
// the ordered list of dimensions that may be relaxed (default: the ladder), minus the other two.
export function effectiveDims(q) {
  const require = dimensions('require', q.require) ?? [];
  const ignore = dimensions('ignore', q.ignore) ?? [];
  if (require.some((d) => ignore.includes(d)))
    throw new ApiError(400, 'bad_request', 'dimension in both require and ignore', {
      allowed: DIMS,
    });
  const given = dimensions('relax', q.relax);
  const relax = (given ?? [...DIMS]).filter((d) => !require.includes(d) && !ignore.includes(d));
  return { require, ignore, relax, relaxGiven: given !== null };
}

function wantedOf(q) {
  return {
    commit: q.commit,
    tree: q.tree,
    die: q.die,
    os: q.os,
    host_driver: q.host_driver,
    guest_driver: q.guest_driver,
    cpu: q.cpu === null ? null : parseCpu(q.cpu),
  };
}
const hasWanted = (dim, w) =>
  ({
    commit: !!(w.commit || w.tree),
    cpu: !!w.cpu,
    host_driver: !!w.host_driver,
    guest_driver: !!w.guest_driver,
    die: !!w.die,
    os: !!w.os,
    family: !!w.die && familyOfDie(w.die) !== 'unknown',
  })[dim];

// FNV-1a of the canonical request, so that a cursor only resumes the request that made it.
function fingerprint(textValue) {
  let h = 0x811c9dc5;
  for (let i = 0; i < textValue.length; i++) {
    h ^= textValue.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, '0');
}
const toB64 = (t) => btoa(t).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');

function readCursor(value, fp) {
  if (value === null) return null;
  const bad = new ApiError(400, 'bad_request', 'invalid cursor');
  let parsed;
  try {
    const padded = value.replaceAll('-', '+').replaceAll('_', '/');
    parsed = JSON.parse(atob(padded + '='.repeat((4 - (padded.length % 4)) % 4)));
  } catch {
    throw bad;
  }
  if (
    parsed === null ||
    typeof parsed !== 'object' ||
    Array.isArray(parsed) ||
    Object.keys(parsed).sort().join(',') !== 'f,i,r,s' ||
    !Number.isSafeInteger(parsed.i) ||
    parsed.i < 0 ||
    parsed.i > 100000 ||
    typeof parsed.r !== 'string' ||
    !PATTERNS.runId.test(parsed.r) ||
    typeof parsed.s !== 'string' ||
    !SUB_ID.test(parsed.s) ||
    parsed.f !== fp
  )
    throw bad;
  return parsed;
}

// ---- candidate selection (fixed statements, bounded) -----------------------------------------

// One read of ciSubrunCandidates with the given exact filters.
const read = (db, f) =>
  db.all('ciSubrunCandidates', [
    f.die ?? null,
    f.os ?? null,
    f.host_driver ?? null,
    f.guest_driver ?? null,
    f.family ?? null,
    f.tree ?? null,
    f.commit ?? null,
    f.cpu ?? null,
    PROBE_ROWS,
  ]);

// The exact filter of each wanted dimension (commit is two alternatives: the tree or the commit).
function filtersOf(dim, w) {
  switch (dim) {
    case 'commit':
      return [w.tree ? { tree: w.tree } : null, w.commit ? { commit: w.commit } : null].filter(
        Boolean,
      );
    case 'cpu':
      return w.cpu ? [{ cpu: w.cpu.vendor }] : [];
    case 'host_driver':
    case 'guest_driver':
    case 'die':
    case 'os':
      return w[dim] ? [{ [dim]: w[dim] }] : [];
    case 'family':
      return w.die ? [{ family: familyOfDie(w.die) }] : [];
    default:
      return [];
  }
}

// Required dimensions are filtered exactly in SQL. Then: one read with every wanted dimension
// (the exact matches), one read per wanted dimension left out (the nearest by one step), and one
// read of the newest rows under the required filters alone. At most 9 reads of PROBE_ROWS rows.
async function candidates(db, w, active, required) {
  const base = {};
  const alternatives = []; // commit: tree OR commit (a required commit has two exact filters)
  for (const dim of required) {
    const options = filtersOf(dim, w);
    if (dim === 'commit' && options.length > 1) alternatives.push(...options);
    else for (const o of options) Object.assign(base, o);
  }
  const wanted = DIMS.filter((d) => active.has(d) && hasWanted(d, w));
  const withDims = (dims) => {
    const variants = [{}];
    for (const dim of dims) {
      const options = required.includes(dim) ? [{}] : filtersOf(dim, w);
      if (!options.length) continue;
      const next = [];
      for (const v of variants) for (const o of options) next.push({ ...v, ...o });
      variants.length = 0;
      variants.push(...next);
    }
    return variants;
  };
  const roots = alternatives.length ? alternatives : [{}];
  // Order matters (only the first MAX_PROBES are read): every all-dimensions read, then the
  // leave-one-out reads, then the newest-rows reads.
  const exact = [];
  const nearest = [];
  const newest = [];
  for (const root of roots) {
    exact.push(...withDims(wanted).map((v) => ({ ...base, ...root, ...v })));
    for (const dim of wanted)
      if (!required.includes(dim))
        nearest.push(
          ...withDims(wanted.filter((d) => d !== dim)).map((v) => ({ ...base, ...root, ...v })),
        );
    newest.push({ ...base, ...root });
  }
  const sets = [...exact, ...nearest, ...newest];
  const seen = new Map();
  const done = new Set();
  // At most MAX_PROBES reads per request (the wanted dimensions can multiply the sets, for example a
  // tree and a commit both given): the all-dimensions read comes first, so the exact matches and the
  // nearest ones are always read (review of #282, F8).
  for (const set of sets) {
    const key = JSON.stringify(Object.entries(set).sort());
    if (done.has(key)) continue;
    if (done.size >= MAX_PROBES) break;
    done.add(key);
    for (const row of await read(db, set)) seen.set(row.run_id + '|' + row.sub, row);
  }
  return [...seen.values()];
}

// ---- /v1/ci/subruns --------------------------------------------------------------------------

const cap = (budget, hits, lo, hi) =>
  Math.max(lo, Math.min(hi, Math.floor(budget / Math.max(1, hits))));

async function expand(db, hits, include) {
  const out = new Map(hits.map((h) => [h.run + '|' + h.sub, {}]));
  const pairs = JSON.stringify(hits.map((h) => h.run + '|' + h.sub));
  const group = (rows, shape, key, cap) => {
    const by = new Map();
    for (const r of rows) {
      const k = r.run_id + '|' + r.sub;
      if (!by.has(k)) by.set(k, []);
      by.get(k).push(r);
    }
    for (const [k, target] of out) {
      const list = by.get(k) ?? [];
      target[key] = { rows: list.slice(0, cap).map(shape), truncated: list.length > cap };
    }
  };
  if (include.includes('tests')) {
    const n = cap(TEST_BUDGET, hits.length, 5, 100);
    group(
      await db.all('ciSubrunTests', [pairs, n + 1]),
      (r) => {
        const { sub, ...rest } = testOut(r);
        return rest;
      },
      'tests',
      n,
    );
  }
  if (include.includes('perf')) {
    const n = cap(PERF_BUDGET, hits.length, 5, 50);
    group(
      await db.all('ciSubrunPerf', [pairs, n + 1]),
      (r) => {
        const { sub, ...rest } = perfOut(r);
        return rest;
      },
      'perf',
      n,
    );
  }
  if (include.includes('apps')) {
    const n = cap(PERF_BUDGET, hits.length, 5, 50);
    group(
      await db.all('ciSubrunApps', [pairs, n + 1]),
      (r) => {
        const { sub, ...rest } = appOut(r);
        return rest;
      },
      'apps',
      n,
    );
  }
  if (include.includes('stages')) {
    const runs = JSON.stringify([...new Set(hits.map((h) => h.run))]);
    const stageRows = await db.all('ciStagesOf', [runs]);
    for (const h of hits) {
      const rows = stageRows.filter((s) => s.run_id === h.run && h.stage_names.includes(s.name));
      out.get(h.run + '|' + h.sub).stages = {
        rows: rows.slice(0, 16).map((s) => ({
          name: text(s.name, 48),
          result: RESULTS.has(s.result) ? s.result : null,
          rc: Number.isSafeInteger(s.rc) ? s.rc : null,
          duration_ms: int(s.duration_ms, 604800000),
        })),
        truncated: rows.length > 16,
      };
    }
  }
  return out;
}

export async function ciSubruns(db, q) {
  const { require, ignore, relax } = effectiveDims(q);
  const limit = listLimit(q.limit, DEFAULT_SUBRUN_LIMIT);
  const sort = q.sort ?? 'closeness';
  const include = words('include', q.include, INCLUDES) ?? [];
  const maxRelaxed = q.min_closeness === null ? DIMS.length : Number(q.min_closeness);
  const wanted = wantedOf(q);
  for (const dim of require)
    if (!hasWanted(dim, wanted))
      throw new ApiError(400, 'bad_request', 'required dimension has no wanted value', {
        parameter: 'require',
        dimension: dim,
        allowed: DIMS,
      });
  const active = new Set(DIMS.filter((d) => !ignore.includes(d)));
  const effective = {
    relax,
    require,
    ignore,
    limit,
    sort,
    include,
    min_closeness: maxRelaxed,
  };
  const fp = fingerprint(
    JSON.stringify([
      effective,
      wanted.commit,
      wanted.tree,
      wanted.die,
      wanted.os,
      wanted.host_driver,
      wanted.guest_driver,
      q.cpu,
    ]),
  );
  const cursor = readCursor(q.cursor, fp);
  const pool = await candidates(db, wanted, active, require);
  if (active.has('commit') && (wanted.tree || wanted.commit)) {
    const ref = await db.first('ciCommitRef', [wanted.tree ?? null, wanted.commit ?? null]);
    const at = ref?.first_at ? Date.parse(ref.first_at) : NaN;
    wanted.refTime = Number.isFinite(at) ? at : null;
  }
  const ranked = rank(pool, wanted, { active, relax, sort, maxRelaxed });
  let start = 0;
  if (cursor) {
    const at = ranked.findIndex((r) => r.row.run_id === cursor.r && r.row.sub === cursor.s);
    start = at >= 0 ? at + 1 : Math.min(cursor.i, ranked.length);
  }
  const page = ranked.slice(start, start + limit);
  const more = start + limit < ranked.length;
  const hits = page.map(({ row, match }) => ({
    run: row.run_id,
    sub: SUB_ID.test(row.sub) ? row.sub : null,
    stage_names: subrunOut(row).stages,
    kind: KINDS.has(row.kind) ? row.kind : null,
    os: OSES.has(row.os) ? row.os : null,
    die: /^[A-Z]{2}[0-9]{3}[A-Z]?$/.test(row.die) ? row.die : null,
    family: /^[a-z][a-z0-9]{2,15}$/.test(row.family) ? row.family : null,
    gpu_name: safe(row.gpu_name, 100),
    tier: PATTERNS.tier.test(row.tier) ? row.tier : null,
    host_driver: driver(row.host_driver),
    guest_driver: driver(row.guest_driver),
    cpu: {
      vendor: ['amd', 'intel', 'arm', 'other'].includes(row.cpu_vendor) ? row.cpu_vendor : null,
      arch: ['x86_64', 'aarch64'].includes(row.cpu_arch) ? row.cpu_arch : null,
    },
    verdict: SUB_VERDICTS.has(row.verdict) ? row.verdict : null,
    session_verdict: ['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED'].includes(row.session_verdict)
      ? row.session_verdict
      : null,
    reason: safe(row.reason),
    commit: PATTERNS.sha.test(row.commit_sha) ? row.commit_sha : null,
    tree: PATTERNS.hash.test(row.tree_hash) ? row.tree_hash : null,
    ended_at: PATTERNS.time.test(row.ended_at ?? '') ? row.ended_at : null,
    duration_ms: int(row.duration_ms, 604800000),
    log_range: subrunOut(row).log_range,
    match,
    session: '/v1/ci/runs/' + row.run_id,
    claimed: ['cpu', 'host_driver', 'guest_driver', 'verdict'],
  }));
  const extra = hits.length && include.length ? await expand(db, hits, include) : new Map();
  for (const hit of hits) {
    const more1 = extra.get(hit.run + '|' + hit.sub) ?? {};
    if (include.includes('counts')) {
      const row = page.find((p) => p.row.run_id === hit.run && p.row.sub === hit.sub).row;
      hit.counts = subrunOut(row).counts;
    }
    for (const key of ['stages', 'tests', 'perf', 'apps']) if (more1[key]) hit[key] = more1[key];
    delete hit.stage_names;
  }
  const last = page[page.length - 1];
  return {
    effective,
    wanted: {
      commit: wanted.commit,
      tree: wanted.tree,
      die: wanted.die,
      os: wanted.os,
      host_driver: wanted.host_driver,
      guest_driver: wanted.guest_driver,
      cpu: q.cpu,
    },
    candidates: pool.length,
    eligible: ranked.length,
    hits,
    next_cursor:
      more && last
        ? toB64(
            JSON.stringify({ i: start + page.length, r: last.row.run_id, s: last.row.sub, f: fp }),
          )
        : null,
  };
}

// ---- /v1/ci/stats ----------------------------------------------------------------------------

export async function ciStats(db, q) {
  const limit = listLimit(q.limit, DEFAULT_STATS_LIMIT);
  const sort = q.sort ?? 'recent';
  const effective = { die: q.die, tier: q.tier, driver: q.driver, os: q.os, limit, sort };
  const runs = await db.all('ciStats', [q.die, q.tier, q.driver, q.os, sort, limit]);
  const ids = JSON.stringify(runs.map((r) => r.run_id));
  const subs = runs.length ? await db.all('ciSubrunsOf', [ids, q.os]) : [];
  const perfCap = 50;
  const perf = runs.length ? await db.all('ciStatsPerf', [ids, q.os, perfCap + 1]) : [];
  const groups = runs.map((r) => {
    const mine = subs.filter((s) => s.run_id === r.run_id);
    const rows = perf.filter((p) => p.run_id === r.run_id);
    const counts = { pass: 0, fail: 0, skip: 0 };
    for (const s of mine) {
      counts.pass += int(s.pass_count, 1e8) ?? 0;
      counts.fail += int(s.fail_count, 1e8) ?? 0;
      counts.skip += int(s.skip_count, 1e8) ?? 0;
    }
    return {
      die: /^[A-Z]{2}[0-9]{3}[A-Z]?$/.test(r.die) ? r.die : null,
      family: /^[a-z][a-z0-9]{2,15}$/.test(r.family) ? r.family : null,
      gpu_name: safe(r.gpu_name, 100),
      host_driver: driver(r.host_driver),
      tier: PATTERNS.tier.test(r.tier) ? r.tier : null,
      run: r.run_id,
      commit: PATTERNS.sha.test(r.commit_sha) ? r.commit_sha : null,
      ended_at: PATTERNS.time.test(r.sort_at ?? '') ? r.sort_at : null,
      verdict: ['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED'].includes(r.verdict) ? r.verdict : null,
      counts,
      subruns: mine.map((s) => ({
        sub: SUB_ID.test(s.sub) ? s.sub : null,
        os: OSES.has(s.os) ? s.os : null,
        kind: KINDS.has(s.kind) ? s.kind : null,
        guest_driver: driver(s.guest_driver),
        verdict: SUB_VERDICTS.has(s.verdict) ? s.verdict : null,
        counts: {
          pass: int(s.pass_count, 1e8) ?? 0,
          fail: int(s.fail_count, 1e8) ?? 0,
          skip: int(s.skip_count, 1e8) ?? 0,
        },
      })),
      perf: { rows: rows.slice(0, perfCap).map(perfOut), truncated: rows.length > perfCap },
      session: '/v1/ci/runs/' + r.run_id,
      claimed: ['host_driver', 'perf', 'counts'],
    };
  });
  return { effective, groups };
}
