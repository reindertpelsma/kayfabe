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
  { path: '/v1/hosts', params: 'none', about: 'host counts by die, class and status' },
  { path: '/v1/logs/{run}/{name}', params: 'none', about: '302 to the public log bucket' },
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
      run: PATTERNS.runId.source,
      log_name: PATTERNS.logName.source,
    },
    suites: SUITES,
    limit: { default: DEFAULT_LIMIT, max: MAX_LIMIT },
    verdicts: ['PASS', 'FAIL', 'RETRY', 'UNAVAILABLE'],
    notes: [
      'verdict null and state "running" while a run is in flight',
      'scores are ratios to a same-box reference and never gate',
      'SPECviewperf scores are not published by this API (docs/SPEC_RESULTS.md)',
    ],
  };
}
