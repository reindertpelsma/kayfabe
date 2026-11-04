// Request handling for the public, read-only API. Design: docs/design/V3_PLATFORM_TESTING.md §6.
// This module makes no network request of any kind: it reads D1 through db.js and returns JSON.
import { Db } from './db.js';
import { CACHE, json, redirect, errorResponse, methodNotAllowed } from './http.js';
import {
  ApiError,
  PATTERNS,
  SUITES,
  HIDDEN_SCORE_SUITES,
  validSuite,
  parseQuery,
  pageLimit,
  encodeCursor,
  decodeCursor,
  logsBase,
} from './validate.js';
import { API_VERSION, PARAMS, describe } from './schema.js';

const MAX_FACTS = 64;
const MAX_LOGS = 32;

function plainObject(text) {
  if (typeof text !== 'string') return null;
  try {
    const value = JSON.parse(text);
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

function ratios(text) {
  const value = plainObject(text);
  if (!value) return null;
  const out = {};
  for (const [key, ratio] of Object.entries(value).slice(0, MAX_FACTS)) {
    if (/^[a-z0-9][a-z0-9_.-]{0,63}$/.test(key) && Number.isFinite(ratio) && ratio >= 0)
      out[key] = ratio;
  }
  return out;
}

function facts(text) {
  const value = plainObject(text);
  if (!value) return {};
  const out = {};
  for (const [key, fact] of Object.entries(value).slice(0, MAX_FACTS)) {
    if (!/^[a-z][a-z0-9_]{0,31}$/.test(key)) continue;
    if (typeof fact === 'string') out[key] = fact.slice(0, 200);
    else if (typeof fact === 'boolean' || Number.isFinite(fact)) out[key] = fact;
  }
  return out;
}

function logNames(text) {
  try {
    const value = JSON.parse(text);
    if (!Array.isArray(value)) return [];
    return value
      .filter((name) => typeof name === 'string' && PATTERNS.logName.test(name))
      .slice(0, MAX_LOGS);
  } catch {
    return [];
  }
}

function scores(row) {
  if (HIDDEN_SCORE_SUITES.has(row.suite)) return { scores: null, scores_hidden: 'SPEC_RESULTS.md' };
  return { scores: ratios(row.scores_json) };
}

function platform(row) {
  return {
    key: row.platform_key,
    die: row.die,
    host_driver: row.host_driver,
    cpu_vendor: row.cpu_vendor,
    cpu_model: row.cpu_model,
    nested: row.nested === 1,
    kernel: row.kernel,
    qemu_build: row.qemu_build,
    iommu_mode: row.iommu_mode,
    guest_os: row.guest_os,
    guest_driver: row.guest_driver,
  };
}

function runSummary(row) {
  return {
    id: row.id,
    commit: row.commit_sha,
    boundary_hash: row.boundary_hash,
    requirement: row.requirement_key,
    die: row.die,
    suite: row.suite,
    state: row.verdict === null ? 'running' : 'final',
    verdict: row.verdict,
    tier: row.tier,
    source: row.source,
    started_at: row.started_at,
    finished_at: row.finished_at,
    platform: platform(row),
    ...scores(row),
  };
}

async function runPage(db, filters, cursorValue, limitValue) {
  const limit = pageLimit(limitValue);
  const cursor = decodeCursor(cursorValue);
  const rows = await db.all('runList', [
    filters.commit,
    filters.die,
    filters.suite,
    filters.verdict,
    filters.tier,
    filters.source,
    cursor ? cursor.sortAt : null,
    cursor ? cursor.id : null,
    limit + 1,
  ]);
  const page = rows.slice(0, limit);
  const last = page[page.length - 1];
  const next = rows.length > limit && last ? encodeCursor(last.sort_at, last.id) : null;
  return { runs: page.map(runSummary), next };
}

async function familiesWithDies(db, only = null) {
  const families = [];
  for (const row of await db.all('dies')) {
    if (only !== null && row.family_id !== only) continue;
    let family = families[families.length - 1];
    if (!family || family.id !== row.family_id) {
      family = { id: row.family_id, name: row.family_name, dies: [] };
      families.push(family);
    }
    if (row.die !== null) family.dies.push({ id: row.die, name: row.die_name });
  }
  return families;
}

const handlers = {
  async health(db) {
    const row = await db.first('health');
    return json({ ok: row?.ok === 1, api: API_VERSION }, { cache: CACHE.none });
  },

  async schema() {
    return json(describe(), { cache: CACHE.static });
  },

  async dies(db) {
    return json(
      { api: API_VERSION, families: await familiesWithDies(db) },
      { cache: CACHE.static },
    );
  },

  async hosts(db) {
    const counts = (await db.all('hostCounts')).map((r) => ({
      die: r.die,
      class: r.class,
      status: r.status,
      hosts: r.hosts,
    }));
    return json({ api: API_VERSION, counts });
  },

  async matrix(db, q) {
    const branch = q.branch ?? 'next';
    let commit = null;
    let refSeq = null;
    if (q.commit !== null) {
      commit = await db.first('commitBySha', [q.commit]);
      if (!commit) throw new ApiError(404, 'not_found', 'unknown commit');
      const on = (await db.all('commitBranches', [q.commit])).find((b) => b.branch === branch);
      if (on) refSeq = on.seq;
    }
    const head = await db.first('branchHead', [branch]);
    if (refSeq === null) refSeq = head ? head.seq : 0;
    const nested = q.nested === null ? null : Number(q.nested);
    const filters = [q.family, q.driver, q.cpu, nested];
    const latest = await db.all('matrixLatest', [...filters, commit ? commit.boundary_hash : null]);
    const passes = await db.all('matrixLastPass', [...filters, branch, refSeq]);
    const cells = new Map();
    const cell = (die, suite) => {
      const key = die + '|' + suite;
      if (!cells.has(key)) cells.set(key, { latest: null, last_pass: null });
      return cells.get(key);
    };
    for (const r of latest)
      cell(r.die, r.suite).latest = {
        run: r.id,
        state: r.verdict === null ? 'running' : 'final',
        verdict: r.verdict,
        tier: r.tier,
        source: r.source,
        commit: r.commit_sha,
        at: r.sort_at,
      };
    for (const r of passes)
      cell(r.die, r.suite).last_pass = {
        run: r.id,
        commit: r.commit_sha,
        tier: r.tier,
        at: r.sort_at,
        changes_since: r.changes_since,
      };
    const families = (await familiesWithDies(db, q.family)).map((f) => ({
      ...f,
      dies: f.dies.map((d) => ({
        ...d,
        cells: Object.fromEntries(
          SUITES.map((s) => [
            s.id,
            cells.get(d.id + '|' + s.id) ?? { latest: null, last_pass: null },
          ]),
        ),
      })),
    }));
    return json({
      api: API_VERSION,
      filters: {
        commit: q.commit,
        family: q.family,
        driver: q.driver,
        cpu: q.cpu,
        nested: q.nested === null ? null : q.nested === '1',
        branch,
      },
      reference: { branch, seq: refSeq, head: head ? head.sha : null },
      suites: SUITES,
      families,
    });
  },

  async cell(db, q, [die, suite]) {
    if (!PATTERNS.die.test(die)) throw new ApiError(400, 'bad_request', 'invalid die');
    if (!validSuite(suite)) throw new ApiError(404, 'not_found', 'unknown suite');
    const known = await db.first('dieById', [die]);
    if (!known) throw new ApiError(404, 'not_found', 'unknown die');
    const filters = { commit: null, die, suite, verdict: null, tier: null, source: null };
    const page = await runPage(db, filters, q.before, q.limit);
    return json({
      api: API_VERSION,
      die: { id: known.die, name: known.die_name, family: known.family_id },
      suite: SUITES.find((s) => s.id === suite),
      runs: page.runs,
      next_before: page.next,
    });
  },

  async runs(db, q) {
    const page = await runPage(db, q, q.cursor, q.limit);
    return json({ api: API_VERSION, runs: page.runs, next_cursor: page.next });
  },

  async run(db, q, [id], env) {
    if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');
    const row = await db.first('runById', [id]);
    if (!row) throw new ApiError(404, 'not_found', 'unknown run');
    const stages = (await db.all('runStages', [id])).map((s) => ({
      name: s.name,
      verdict: s.verdict,
      started_at: s.started_at,
      finished_at: s.finished_at,
    }));
    const base = logsBase(env.LOGS_BASE);
    const logs = logNames(row.logs_json).map((name) => ({
      name,
      path: '/v1/logs/' + id + '/' + name,
      ...(base ? { direct: base + '/runs/' + id + '/' + name } : {}),
    }));
    const final = row.verdict !== null && row.finished_at !== null;
    return json(
      {
        api: API_VERSION,
        run: {
          ...runSummary(row),
          family: row.family_id,
          host_facts: facts(row.host_facts_json),
          stages,
          logs,
          commit_info: {
            sha: row.commit_sha,
            message: row.commit_message,
            author: row.commit_author,
            committed_at: row.commit_committed_at,
          },
          provenance: {
            source: row.source,
            tier: row.tier,
            requirement: row.requirement_key,
            boundary_hash: row.boundary_hash,
          },
        },
      },
      { cache: final ? CACHE.immutable : CACHE.short },
    );
  },

  async commitRequirements(db, q, [sha]) {
    if (!PATTERNS.sha.test(sha)) throw new ApiError(400, 'bad_request', 'invalid sha');
    const commit = await db.first('commitBySha', [sha]);
    if (!commit) throw new ApiError(404, 'not_found', 'unknown commit');
    const branches = (await db.all('commitBranches', [sha])).map((b) => ({
      branch: b.branch,
      seq: b.seq,
    }));
    const requirements = (await db.all('commitRequirements', [sha])).map((r) => ({
      key: r.requirement_key,
      state: r.state,
      updated_at: r.updated_at,
      run:
        r.run_id === null
          ? null
          : {
              id: r.run_id,
              die: r.die,
              suite: r.suite,
              verdict: r.verdict,
              tier: r.tier,
              finished_at: r.finished_at,
            },
    }));
    return json({ api: API_VERSION, commit, branches, requirements });
  },

  async logRedirect(db, q, [id, name], env) {
    if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');
    if (!PATTERNS.logName.test(name)) throw new ApiError(400, 'bad_request', 'invalid log name');
    const row = await db.first('runLogs', [id]);
    if (!row || !logNames(row.logs_json).includes(name))
      throw new ApiError(404, 'not_found', 'unknown log');
    const base = logsBase(env.LOGS_BASE);
    if (!base) throw new ApiError(404, 'not_found', 'logs not configured');
    return redirect(base + '/runs/' + id + '/' + name);
  },
};

const ROUTES = [
  [/^\/v1\/health$/, 'health', 'none'],
  [/^\/v1\/schema$/, 'schema', 'none'],
  [/^\/v1\/dies$/, 'dies', 'none'],
  [/^\/v1\/hosts$/, 'hosts', 'none'],
  [/^\/v1\/matrix$/, 'matrix', 'matrix'],
  [/^\/v1\/cells\/([^/]{1,16})\/([^/]{1,32})$/, 'cell', 'cell'],
  [/^\/v1\/runs$/, 'runs', 'runs'],
  [/^\/v1\/runs\/([^/]{1,64})$/, 'run', 'none'],
  [/^\/v1\/commits\/([^/]{1,40})\/requirements$/, 'commitRequirements', 'none'],
  [/^\/v1\/logs\/([^/]{1,64})\/([^/]{1,68})$/, 'logRedirect', 'none'],
];

export async function handle(request, env) {
  if (request.method !== 'GET') return methodNotAllowed();
  try {
    const url = new URL(request.url);
    for (const [pattern, name, params] of ROUTES) {
      const match = url.pathname.match(pattern);
      if (!match) continue;
      const query = parseQuery(url, PARAMS[params]);
      return await handlers[name](new Db(env.DB), query, match.slice(1), env);
    }
    throw new ApiError(404, 'not_found', 'no such resource');
  } catch (error) {
    if (error instanceof ApiError) return errorResponse(error.status, error.code, error.detail);
    return errorResponse(500, 'internal', 'internal error');
  }
}
