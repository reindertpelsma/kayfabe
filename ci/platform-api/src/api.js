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
  MAX_LIMIT,
} from './validate.js';
import { API_VERSION, PARAMS, describe } from './schema.js';
import { subrunOut, reportOut, ciSubruns, ciStats } from './subruns.js';

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

// --- CI Worker result store (migrations/0002_ci_results.sql) ---------------------------------

const CI_LOG_KEY = /^runs\/[a-z0-9][a-z0-9-]{5,63}\/[a-z0-9][a-z0-9._-]{0,63}\.(?:log|txt)$/;
const MAX_CI_TREES = 5;
const CI_RESULTS = new Set(['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED']);
const CPU_VENDORS = new Set(['amd', 'intel', 'arm', 'other']);
const CPU_ARCHS = new Set(['x86_64', 'aarch64']);
const DRIVER_OUT = /^[0-9]{1,4}(?:\.[0-9]{1,5}){1,3}$/;
const driverOut = (v) => (typeof v === 'string' && DRIVER_OUT.test(v) ? v : null);
// ci_runs.source -> the public vocabulary: reproduced stays; the reserved community values
// ('reported' = unverified, 'reviewed') become source 'community' with a report_tier.
const sourceOut = (v) =>
  v === 'reproduced' ? 'reproduced' : v === 'reported' || v === 'reviewed' ? 'community' : null;
const reportTierOut = (v) =>
  v === 'reported' ? 'unverified' : v === 'reviewed' ? 'reviewed' : null;
const osVersionOut = (v) =>
  typeof v === 'string' && /^[a-z0-9][a-z0-9 ._()+-]{0,31}$/.test(v) ? v : null;
const vendorOut = (v) => (CPU_VENDORS.has(v) ? v : null);
// The setups response says which of its fields are claims by the test box, not verified facts.
const CLAIMED_FIELDS = Object.freeze(['cpu_vendor', 'host_driver', 'guest_driver']);

function ciSummary(row) {
  return {
    id: row.run_id,
    commit: row.commit_sha,
    tree: { hash: row.tree_hash, kind: row.tree_kind },
    tier: row.tier,
    family: row.family,
    die: row.die,
    platform: row.platform,
    state: row.verdict === null ? 'running' : 'final',
    verdict: row.verdict,
    started_at: row.started_at,
    ended_at: row.ended_at,
  };
}

const newestCi = (a, b) =>
  (b.ended_at ?? b.started_at).localeCompare(a.ended_at ?? a.started_at) ||
  b.run_id.localeCompare(a.run_id);

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

  async ciTrees(db, q) {
    const limit = pageLimit(q.limit);
    const trees = (await db.all('ciTrees', [q.tier, limit])).map((t) => ({
      hash: t.tree_hash,
      kind: t.tree_kind,
      first_at: t.first_at,
      last_at: t.last_at,
      runs: t.runs,
      passed: t.passed,
      failed: t.failed,
      running: t.running,
    }));
    return json({ api: API_VERSION, filters: { tier: q.tier }, trees });
  },

  async ciMatrix(db, q) {
    const params = [q.tier, q.tree, q.family];
    const cells = new Map();
    const key = (r) => [r.tier, r.family, r.die, r.platform].join('|');
    for (const r of await db.all('ciMatrixLatest', params))
      cells.set(key(r), {
        tier: r.tier,
        family: r.family,
        die: r.die,
        platform: r.platform,
        latest: ciSummary(r),
        last_pass: null,
      });
    for (const r of await db.all('ciMatrixLastPass', params)) {
      const cell = cells.get(key(r));
      if (cell)
        cell.last_pass = {
          id: r.run_id,
          commit: r.commit_sha,
          tree: r.tree_hash,
          ended_at: r.ended_at,
        };
    }
    return json({
      api: API_VERSION,
      filters: { tier: q.tier, tree: q.tree, family: q.family },
      cells: [...cells.values()],
    });
  },

  // Setup picker: one row per (guest OS, die, CPU vendor, host driver, guest driver). The CPU
  // vendor and both drivers are claims by the test box (null: not reported) and never a verdict.
  async ciSetups(db, q) {
    const limit = pageLimit(q.limit);
    const dies = q.gpu === null ? [] : [...new Set(q.gpu.split(','))];
    // No source filter = the reproduced rows (community rows are opt-in); imported evidence is a
    // static file on the site, never a database row, so it adds nothing here.
    const sources = new Set(q.source === null ? ['reproduced'] : q.source.split(','));
    const params = [
      q.os,
      dies[0] ?? null,
      dies[1] ?? null,
      dies[2] ?? null,
      dies[3] ?? null,
      q.cpu,
      q.driver,
      q.tier,
      sources.has('reproduced') ? 1 : 0,
      sources.has('community') ? 1 : 0,
      q.report_tier,
      limit + 1,
    ];
    const rows = await db.all('ciSetups', params);
    const result = (value) => (CI_RESULTS.has(value) ? value : null);
    const setups = rows.slice(0, limit).map((r) => ({
      os: r.os,
      die: r.die,
      family: r.family,
      source: sourceOut(r.source),
      report_tier: reportTierOut(r.source),
      cpu_vendor: vendorOut(r.cpu_vendor),
      host_driver: driverOut(r.host_driver),
      guest_driver: driverOut(r.guest_driver),
      gpu_name: typeof r.gpu_name === 'string' ? r.gpu_name.slice(0, 100) : null,
      runs: r.runs,
      passed: r.passed,
      failed: r.failed,
      other: r.runs - r.passed - r.failed,
      last_at: r.last_at,
      latest: {
        id: r.latest_id,
        result: result(r.latest_result),
        verdict: result(r.latest_verdict),
        tier: r.latest_tier,
        commit: r.latest_commit,
        tree: r.latest_tree,
        ended_at: r.latest_ended_at,
      },
      last_pass:
        r.pass_id === null || r.pass_id === undefined
          ? null
          : { id: r.pass_id, commit: r.pass_commit, ended_at: r.pass_ended_at },
    }));
    return json({
      api: API_VERSION,
      filters: {
        os: q.os,
        gpu: dies,
        cpu: q.cpu,
        driver: q.driver,
        tier: q.tier,
        source: [...sources],
        report_tier: q.report_tier,
      },
      claimed: CLAIMED_FIELDS,
      truncated: rows.length > limit,
      setups,
    });
  },

  async ciFacets(db) {
    const named = new Map();
    for (const r of await db.all('ciFacetNames')) {
      if (!named.has(r.die)) named.set(r.die, []);
      named.get(r.die).push({ name: r.gpu_name, runs: r.runs });
    }
    const values = async (column, ok) =>
      (await db.all('ciFacetClaim', [column]))
        .filter((r) => ok(r.value))
        .map((r) => ({ value: r.value, runs: r.runs }));
    const isDriver = (v) => driverOut(v) !== null;
    const platforms = await db.all('ciFacetPlatforms');
    const bySource = await db.all('ciFacetSources');
    const count = (names) =>
      bySource.filter((r) => names.includes(r.source)).reduce((n, r) => n + r.runs, 0);
    const tierRuns = (name) => bySource.find((r) => r.source === name)?.runs ?? 0;
    const osRuns = (os) =>
      platforms
        .filter((p) => p.platform === os || p.platform === 'both')
        .reduce((n, p) => n + p.runs, 0);
    return json({
      api: API_VERSION,
      claimed: CLAIMED_FIELDS,
      os: ['linux', 'windows'].map((os) => ({ value: os, runs: osRuns(os) })).filter((o) => o.runs),
      dies: (await db.all('ciFacetDies')).map((d) => ({
        id: d.die,
        family: d.family,
        runs: d.runs,
        last_at: d.last_at,
        gpu_names: (named.get(d.die) ?? []).slice(0, 8),
      })),
      cpu_vendors: await values('cpu_vendor', (v) => CPU_VENDORS.has(v)),
      cpu_archs: await values('cpu_arch', (v) => CPU_ARCHS.has(v)),
      host_drivers: await values('host_driver', isDriver),
      guest_drivers: await values('guest_driver', isDriver),
      // Every value the source and report_tier filters accept, with what exists behind it. imported
      // is the static evidence file on the site, which this API does not serve.
      sources: [
        { value: 'reproduced', runs: count(['reproduced']), available: count(['reproduced']) > 0 },
        { value: 'imported', runs: 0, available: false, served_by: 'site evidence file' },
        {
          value: 'community',
          runs: count(['reported', 'reviewed']),
          available: count(['reported', 'reviewed']) > 0,
        },
      ],
      report_tiers: [
        { value: 'unverified', runs: tierRuns('reported') },
        { value: 'reviewed', runs: tierRuns('reviewed') },
      ].filter((t) => t.runs > 0),
      tiers: (await db.all('ciFacetTiers')).map((t) => ({ value: t.tier, runs: t.runs })),
    });
  },

  async ciRun(db, q, [id], env) {
    if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');
    const row = await db.first('ciRunById', [id]);
    if (!row) throw new ApiError(404, 'not_found', 'unknown run');
    const stages = (await db.all('ciRunStages', [id])).map((s) => ({
      name: s.name,
      result: s.result,
      rc: s.rc,
      duration_ms: s.duration_ms,
    }));
    const base = logsBase(env.LOGS_BASE);
    const log =
      typeof row.log_key === 'string' && CI_LOG_KEY.test(row.log_key)
        ? { key: row.log_key, ...(base ? { direct: base + '/' + row.log_key } : {}) }
        : null;
    const claims = await db.all('ciRunClaims', [id]);
    const final = row.verdict !== null && row.ended_at !== null;
    // Migration 0004 (run reports): the subruns of this session, the stored report and the identity
    // of the published full log. A run without any of them (an older box, a Worker without the
    // bucket) answers null / [] and the page says "not measured yet".
    // A database that predates migration 0004 has no such tables: the run still answers, without
    // them (deployment order is migrations first, but a late migration must not take the page down).
    const soft = async (action, empty) => {
      try {
        return await action();
      } catch {
        return empty;
      }
    };
    const stored = await soft(() => db.first('ciRunReport', [id]), null);
    const logInfo =
      stored &&
      Number.isSafeInteger(stored.log_bytes) &&
      /^[0-9a-f]{64}$/.test(stored.log_sha256 ?? '')
        ? {
            bytes: stored.log_bytes,
            sha256: stored.log_sha256,
            ...(base ? { url: base + '/' + id + '/log.txt' } : {}),
          }
        : null;
    const subruns = (await soft(() => db.all('ciRunSubruns', [id]), [])).map(subrunOut);
    return json(
      {
        api: API_VERSION,
        run: {
          ...ciSummary(row),
          profile: row.profile,
          source: row.source,
          stages,
          // host_driver: the NVIDIA driver on the machine that runs kayfabe; guests: the OS version
          // and the driver inside each guest. All claims by the test box (never the provider's
          // offer label, which is not stored).
          host: {
            gpu_name: row.gpu_name ?? null,
            host_driver: driverOut(claims[0]?.host_driver),
            guests: claims.map((c) => ({
              os: c.guest_os === 'linux' || c.guest_os === 'windows' ? c.guest_os : null,
              os_version: osVersionOut(c.guest_os_version),
              driver: driverOut(c.guest_driver),
            })),
            claimed: CLAIMED_FIELDS,
          },
          log: log ?? logInfo,
          subruns,
          report: stored ? reportOut(stored.report_json, id, logInfo) : null,
        },
      },
      { cache: final ? CACHE.immutable : CACHE.short },
    );
  },

  async ciSubruns(db, q) {
    return json({ api: API_VERSION, ...(await ciSubruns(db, q)) });
  },

  async ciStats(db, q) {
    return json({ api: API_VERSION, ...(await ciStats(db, q)) });
  },

  async ciTree(db, q, [hash]) {
    if (!PATTERNS.hash.test(hash)) throw new ApiError(400, 'bad_request', 'invalid tree hash');
    const runs = await db.all('ciRunList', [hash, null, MAX_LIMIT]);
    if (!runs.length) throw new ApiError(404, 'not_found', 'unknown tree hash');
    const commits = (await db.all('ciTreeCommits', [hash])).map((c) => ({
      sha: c.commit_sha,
      runs: c.runs,
      last_at: c.last_at,
    }));
    return json({
      api: API_VERSION,
      tree: { hash, kind: runs[0].tree_kind },
      commits,
      runs: runs.map(ciSummary),
    });
  },

  async ciCommit(db, q, [sha]) {
    if (!PATTERNS.sha.test(sha)) throw new ApiError(400, 'bad_request', 'invalid sha');
    const trees = (await db.all('ciCommitTrees', [sha])).slice(0, MAX_CI_TREES);
    if (!trees.length) throw new ApiError(404, 'not_found', 'unknown commit');
    const runs = [];
    for (const t of trees)
      runs.push(...(await db.all('ciRunList', [t.tree_hash, null, MAX_LIMIT])));
    runs.sort(newestCi);
    return json({
      api: API_VERSION,
      commit: sha,
      trees: trees.map((t) => ({
        hash: t.tree_hash,
        kind: t.tree_kind,
        runs: t.runs,
        last_at: t.last_at,
      })),
      // Runs of other commits with the same tree hash, for information only: the CI Worker counts
      // them for this commit only with TREE_KEY_CARRY_OVER on and a Worker-computed key.
      runs: runs.map((r) => ({ ...ciSummary(r), other_commit: r.commit_sha !== sha })),
    });
  },

  async logRedirect(db, q, [id, name], env) {
    if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');
    // Objects the CI Worker publishes for a run (<run>/log.txt and <run>/report.json in the public
    // bucket); only for a final run that has them. Anything else is the legacy `runs` table.
    if (name === 'log.txt' || name === 'report.json') {
      const object = await db.first('ciRunObjects', [id]).catch(() => null);
      const present =
        object && (name === 'log.txt' ? object.log_bytes !== null : object.has_report === 1);
      if (present) {
        const home = logsBase(env.LOGS_BASE);
        if (!home) throw new ApiError(404, 'not_found', 'logs not configured');
        return redirect(home + '/' + id + '/' + name);
      }
    }
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
  [/^\/v1\/ci\/matrix$/, 'ciMatrix', 'ciMatrix'],
  [/^\/v1\/ci\/setups$/, 'ciSetups', 'ciSetups'],
  [/^\/v1\/ci\/facets$/, 'ciFacets', 'none'],
  [/^\/v1\/ci\/trees$/, 'ciTrees', 'ciTrees'],
  [/^\/v1\/ci\/trees\/([^/]{1,64})$/, 'ciTree', 'none'],
  [/^\/v1\/ci\/runs\/([^/]{1,64})$/, 'ciRun', 'none'],
  [/^\/v1\/ci\/commits\/([^/]{1,40})$/, 'ciCommit', 'none'],
  [/^\/v1\/ci\/subruns$/, 'ciSubruns', 'ciSubruns'],
  [/^\/v1\/ci\/stats$/, 'ciStats', 'ciStats'],
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
    if (error instanceof ApiError)
      return errorResponse(error.status, error.code, error.detail, error.extra);
    return errorResponse(500, 'internal', 'internal error');
  }
}
