// The public API's own description, served at /v1/schema. The router reads the parameter
// allow-lists from here, so the description and the validation cannot drift apart.
import { PATTERNS, SUITES, MAX_LIMIT, DEFAULT_LIMIT } from './validate.js';

export const API_VERSION = 'v1';

export const PARAMS = Object.freeze({
  none: Object.freeze({}),
  matrix: Object.freeze({
    commit: 'sha',
    family: 'family',
    driver: 'driver',
    cpu: 'cpu',
    nested: 'nested',
    branch: 'branch',
  }),
  cell: Object.freeze({ before: 'cursor', limit: 'limit' }),
  ciTrees: Object.freeze({ tier: 'tier', limit: 'limit' }),
  ciMatrix: Object.freeze({ tier: 'tier', tree: 'hash', family: 'family' }),
  ciSetups: Object.freeze({
    os: 'os',
    gpu: 'gpuList',
    cpu: 'cpuVendor',
    driver: 'driver',
    tier: 'tier',
    source: 'sourceList',
    report_tier: 'reportTier',
    limit: 'limit',
  }),
  ciSubruns: Object.freeze({
    commit: 'sha',
    tree: 'hash',
    die: 'die',
    os: 'osAny',
    host_driver: 'driver',
    guest_driver: 'driver',
    cpu: 'cpuSpec',
    relax: 'dimList',
    require: 'dimList',
    ignore: 'dimList',
    limit: 'limit',
    cursor: 'cursor',
    sort: 'subSort',
    include: 'includeList',
    min_closeness: 'closeness',
  }),
  ciStats: Object.freeze({
    die: 'die',
    tier: 'tier',
    driver: 'driver',
    os: 'osAny',
    limit: 'limit',
    sort: 'statsSort',
  }),
  runs: Object.freeze({
    commit: 'sha',
    die: 'die',
    suite: 'suite',
    verdict: 'verdict',
    tier: 'tier',
    source: 'source',
    cursor: 'cursor',
    limit: 'limit',
  }),
});

export const ENDPOINTS = Object.freeze([
  { path: '/v1/matrix', params: 'matrix', about: 'dies by family x suites, latest run per cell' },
  { path: '/v1/cells/{die}/{suite}', params: 'cell', about: 'run history of one cell' },
  { path: '/v1/runs', params: 'runs', about: 'runs, newest first' },
  { path: '/v1/runs/{id}', params: 'none', about: 'one run: platform, stages, logs' },
  { path: '/v1/commits/{sha}/requirements', params: 'none', about: 'requirement checklist' },
  { path: '/v1/dies', params: 'none', about: 'families and dies' },
  {
    path: '/v1/ci/matrix',
    params: 'ciMatrix',
    about: 'CI Worker results: latest run per tier, family, die and platform',
  },
  {
    path: '/v1/ci/trees',
    params: 'ciTrees',
    about: 'CI Worker result keys (tree hashes), newest first',
  },
  { path: '/v1/ci/trees/{hash}', params: 'none', about: 'one result key: its commits and runs' },
  { path: '/v1/ci/runs/{id}', params: 'none', about: 'one CI Worker run: stages and host facts' },
  {
    path: '/v1/ci/commits/{sha}',
    params: 'none',
    about: 'a commit: its result keys and their runs',
  },
  {
    path: '/v1/ci/setups',
    params: 'ciSetups',
    about:
      'CI Worker results grouped by guest OS, GPU die, CPU vendor, host driver and guest driver (CPU and driver facts are reported by the test box, not verified)',
  },
  {
    path: '/v1/ci/facets',
    params: 'none',
    about:
      'the values that exist for the setups filters: dies and reported GPU names, CPU vendors, drivers',
  },
  {
    path: '/v1/ci/subruns',
    params: 'ciSubruns',
    about:
      'host-session subruns ranked by similarity to the wanted commit, CPU, drivers, die and OS; every hit explains what was relaxed (match.exact, match.relaxed); relax, require, ignore, sort, include, limit, cursor and min_closeness shape the answer and effective states what was applied',
  },
  {
    path: '/v1/ci/stats',
    params: 'ciStats',
    about:
      'the latest reported session per die and host driver: subrun pass counts and the perf rows the test box measured (claims; never a gate)',
  },
  { path: '/v1/hosts', params: 'none', about: 'host counts by die, class and status' },
  {
    path: '/v1/logs/{run}/{name}',
    params: 'none',
    about: '302 to the public log bucket; a CI run has log.txt and report.json',
  },
  { path: '/v1/schema', params: 'none', about: 'this description' },
  { path: '/v1/health', params: 'none', about: 'liveness and one database read' },
]);

export function describe() {
  const pattern = (key) => (key === 'suite' ? 'one of suites[].id' : PATTERNS[key].source);
  return {
    api: API_VERSION,
    methods: ['GET'],
    design: 'docs/design/V3_PLATFORM_TESTING.md',
    endpoints: ENDPOINTS.map((e) => ({
      path: e.path,
      about: e.about,
      params: Object.fromEntries(
        Object.entries(PARAMS[e.params]).map(([name, key]) => [name, pattern(key)]),
      ),
    })),
    path_patterns: {
      die: PATTERNS.die.source,
      sha: PATTERNS.sha.source,
      tree: PATTERNS.hash.source,
      run: PATTERNS.runId.source,
      log_name: PATTERNS.logName.source,
    },
    suites: SUITES,
    limit: { default: DEFAULT_LIMIT, max: MAX_LIMIT },
    verdicts: ['PASS', 'FAIL', 'RETRY', 'UNAVAILABLE'],
    ci_verdicts: ['PASS', 'FAIL', 'RETRY', 'UNATTRIBUTED'],
    notes: [
      'verdict null and state "running" while a run is in flight',
      'scores are ratios to a same-box reference and never gate',
      'SPECviewperf scores are not published by this API (docs/SPEC_RESULTS.md)',
      'setups: source filters rows by origin (comma list of reproduced, imported, community; default reproduced; imported evidence is a static site file, so it returns no rows here; community rows are not populated yet) and report_tier by the review state of a community row; every row carries its source and report_tier; rows of different sources are never merged',
      'setups: one row per (guest OS, die, CPU vendor, host driver, guest driver, source); a run over both guest OSes counts once in each OS and a FAIL is charged to an OS unless every failed stage is known to belong to the other OS (windows* and specviewperf belong to Windows, the payload Linux stage names to Linux; any other failed stage counts against both), the spared OS gets UNATTRIBUTED; CPU vendor, host driver and guest driver are claims by the test box (null: not reported) and never change a verdict',
      'CI rows: RETRY and UNATTRIBUTED never pass; runs of other commits with the same tree hash are shown for information only: the CI Worker counts them for a commit only when its tree-key carry-over is enabled (off by default) and the key is Worker-computed (tree.kind "tree")',
    ],
  };
}
