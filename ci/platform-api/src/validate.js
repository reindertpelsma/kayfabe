// Strict input validation. Every query parameter and path segment has one anchored pattern;
// unknown and repeated parameters are refused. Error details are fixed strings and never
// echo the request.

export class ApiError extends Error {
  constructor(status, code, detail) {
    super(code);
    this.status = status;
    this.code = code;
    this.detail = detail;
  }
}

export const SUITES = Object.freeze([
  { id: 'linux-gate', os: 'linux', label: 'Linux gate' },
  { id: 'linux-fast', os: 'linux', label: 'Fast suite' },
  { id: 'linux-apps', os: 'linux', label: 'Linux apps' },
  { id: 'windows-gate', os: 'windows', label: 'Windows gate' },
  { id: 'windows-apps', os: 'windows', label: 'Windows apps' },
  { id: 'windows-specviewperf', os: 'windows', label: 'SPECviewperf' },
]);
const SUITE_IDS = new Set(SUITES.map((s) => s.id));
// Scores of these suites are never returned (docs/SPEC_RESULTS.md; design section 8).
export const HIDDEN_SCORE_SUITES = Object.freeze(new Set(['windows-specviewperf']));

export const PATTERNS = Object.freeze({
  sha: /^[0-9a-f]{40}$/,
  hash: /^[0-9a-f]{64}$/,
  die: /^[A-Z]{2}[0-9]{3}[A-Z]?$/,
  family: /^[a-z][a-z0-9]{2,15}$/,
  driver: /^[0-9]{3}\.[0-9]{1,3}(?:\.[0-9]{1,3})?$/,
  cpu: /^(?:intel|amd)$/,
  // /v1/ci/setups and /v1/ci/facets (migration 0003): the CPU vendor the box reported, the guest OS
  // and one to four GPU dies, comma separated (a marketing name can map to several dies).
  cpuVendor: /^(?:amd|intel|arm|other)$/,
  os: /^(?:linux|windows)$/,
  // Where a row comes from. 'reproduced' = a run of the CI Worker (the only rows the stores hold
  // today). 'imported' = evidence imported from the repository: a static file on the site, not a
  // database row, so asking for it here returns nothing. 'community' = reported or reviewed rows
  // of a later, separate store (ci_runs.source 'reported' / 'reviewed': reserved, none exist).
  sourceList: /^(?:reproduced|imported|community)(?:,(?:reproduced|imported|community)){0,2}$/,
  // Of a community row: 'unverified' (reported) or 'reviewed'. Never set on a reproduced row.
  reportTier: /^(?:unverified|reviewed)$/,
  gpuList: /^[A-Z]{2}[0-9]{3}[A-Z]?(?:,[A-Z]{2}[0-9]{3}[A-Z]?){0,3}$/,
  nested: /^[01]$/,
  branch: /^(?:master|next)$/,
  verdict: /^(?:PASS|FAIL|RETRY|UNAVAILABLE|RUNNING)$/,
  tier: /^(?:largest|middle|small|adhoc)$/,
  source: /^(?:reproduced|reviewed|reported)$/,
  limit: /^[1-9][0-9]{0,2}$/,
  cursor: /^[A-Za-z0-9_-]{8,200}$/,
  runId: /^[a-z0-9][a-z0-9-]{5,63}$/,
  logName: /^[a-z0-9][a-z0-9._-]{0,63}\.(?:log|txt)$/,
  time: /^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$/,
});

export const MAX_LIMIT = 100;
export const DEFAULT_LIMIT = 50;

export function validSuite(value) {
  return typeof value === 'string' && SUITE_IDS.has(value);
}

// Parse the query string against an allow-list {name: pattern-key}. Returns {name: value|null}.
export function parseQuery(url, allowed) {
  const out = {};
  for (const name of Object.keys(allowed)) out[name] = null;
  const seen = new Set();
  for (const [name, value] of url.searchParams) {
    if (!Object.hasOwn(allowed, name)) throw new ApiError(400, 'bad_request', 'unknown parameter');
    if (seen.has(name)) throw new ApiError(400, 'bad_request', 'repeated parameter');
    seen.add(name);
    const pattern = allowed[name];
    const ok = pattern === 'suite' ? validSuite(value) : PATTERNS[pattern].test(value);
    if (!ok) throw new ApiError(400, 'bad_request', 'invalid ' + name);
    out[name] = value;
  }
  return out;
}

export function pageLimit(value) {
  if (value === null) return DEFAULT_LIMIT;
  const n = Number(value);
  if (!Number.isSafeInteger(n) || n < 1 || n > MAX_LIMIT)
    throw new ApiError(400, 'bad_request', 'invalid limit');
  return n;
}

const toBase64Url = (text) =>
  btoa(text).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');

export function encodeCursor(sortAt, id) {
  if (!PATTERNS.time.test(sortAt) || !PATTERNS.runId.test(id)) throw new Error('cursor fields');
  return toBase64Url(JSON.stringify({ s: sortAt, i: id }));
}

// A cursor is opaque base64url JSON; its fields are re-validated after decoding.
export function decodeCursor(value) {
  if (value === null) return null;
  const bad = new ApiError(400, 'bad_request', 'invalid cursor');
  if (!PATTERNS.cursor.test(value)) throw bad;
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
    Object.keys(parsed).length !== 2 ||
    typeof parsed.s !== 'string' ||
    typeof parsed.i !== 'string' ||
    !PATTERNS.time.test(parsed.s) ||
    !PATTERNS.runId.test(parsed.i)
  )
    throw bad;
  return { sortAt: parsed.s, id: parsed.i };
}

// LOGS_BASE must be an https origin on kayfabe.dev (no path, credentials, query or port).
export function logsBase(value) {
  if (typeof value !== 'string' || value === '') return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  const host = url.hostname;
  const onSite = host === 'kayfabe.dev' || host.endsWith('.kayfabe.dev');
  if (
    url.protocol !== 'https:' ||
    !onSite ||
    url.username ||
    url.password ||
    url.port ||
    url.search ||
    url.hash ||
    url.pathname !== '/' ||
    !(value === url.origin || value === url.origin + '/')
  )
    return null;
  return url.origin;
}
