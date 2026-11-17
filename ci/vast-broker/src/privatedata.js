// Private (licence-restricted) objects in the private R2 bucket: `GET /bench-data/<run>/private/<prefix>/<object path>` answers a box
// that holds a live run token WITH THE PRIVATE SCOPE with a 302 to a short-lived presigned S3 GET URL
// (docs/design/V3_WORKER_PRIVATE_PRESIGN.md). Private means licence-restricted for redistribution, not sensitive. There is no object
// list: any object under a prefix of private-objects.json is allowed, an object that does not exist is OBJECT_MISSING, and no
// response ever names another object. The scope is decided at lease time (src/trust.js) and read from the token and the record; the
// route also checks that the run is live, that its profile or source may fetch private objects at all, and a per-run URL limit.
// The Worker holds a READ-ONLY R2 key pair and signs locally; the only network call is one presigned HEAD to prove the object exists.
// A refusal is a named reason: TOKEN_INVALID, TOKEN_EXPIRED, TRUST_SCOPE_MISSING, TIER_NOT_ALLOWED, PREFIX_NOT_PRIVATE,
// OBJECT_MISSING, RATE_LIMITED (reason PRIVATE_URL_LIMIT). Nothing here logs or returns a URL except the 302 Location itself.
import config from '../private-objects.json' with { type: 'json' };
// prettier-ignore
import {refuse,budgetClass} from './policy.js';
import { presignGet } from './sigv4.js';
import { benchEndpoint } from './benchdata.js';
import { hasPrivateScope } from './trust.js';

export const PRIVATE_OBJECTS = config;
export const PRIVATE_REFUSALS = Object.freeze([
  'TOKEN_INVALID',
  'TOKEN_EXPIRED',
  'TRUST_SCOPE_MISSING',
  'TIER_NOT_ALLOWED',
  'PREFIX_NOT_PRIVATE',
  'OBJECT_MISSING',
  'PRIVATE_URL_LIMIT',
]);
const RUN = '[1-9][0-9]{0,19}-[1-9][0-9]{0,19}';
const SEGMENT = '[A-Za-z0-9._-]{1,100}';
// `private/<prefix>/<path>`: the prefix is one segment, the path one to five. The public namespace (`/bench-data/<run>/<name>`) has
// no slash in a name, so the two never overlap.
export const PRIVATE_PATH = new RegExp(
  '^/bench-data/(' +
    RUN +
    ')/private/([A-Za-z0-9][A-Za-z0-9._-]{0,63})/(' +
    SEGMENT +
    '(?:/' +
    SEGMENT +
    '){0,4})$',
);
const KEY = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$/;
const PREFIX = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const only = (v, allowed) => isObject(v) && Object.keys(v).every((k) => allowed.includes(k));

// Every problem of a configuration as a named token; an empty list is a good one.
export function privateConfigProblems(value = PRIVATE_OBJECTS) {
  const problems = [];
  const bad = (reason, ok = false) => {
    if (!ok) problems.push(reason);
    return ok;
  };
  if (
    !bad(
      'SCHEMA',
      only(value, [
        'schema',
        'status',
        'prefixes',
        'allow',
        'max_ttl_seconds',
        'min_ttl_seconds',
        'urls_per_run',
      ]) && value.schema === 'kayfabe.worker-private-objects/1',
    )
  )
    return problems;
  if (bad('PREFIXES', Array.isArray(value.prefixes) && value.prefixes.length > 0)) {
    const seen = new Set();
    for (const p of value.prefixes) {
      if (
        !bad(
          'PREFIX_SCHEMA',
          only(p, ['prefix', 'basis']) && typeof p.basis === 'string' && p.basis,
        )
      )
        continue;
      bad(
        'PREFIX:' + String(p.prefix).slice(0, 64),
        typeof p.prefix === 'string' && PREFIX.test(p.prefix),
      );
      bad('DUPLICATE_PREFIX:' + String(p.prefix).slice(0, 64), !seen.has(p.prefix));
      seen.add(p.prefix);
    }
  }
  if (bad('ALLOW', Array.isArray(value.allow) && value.allow.length > 0))
    for (const a of value.allow) {
      const ok =
        only(a, ['profile', 'source', 'why']) &&
        typeof a.why === 'string' &&
        (a.profile === undefined || ['check', 'release'].includes(a.profile)) &&
        (a.source === undefined || ['admin', 'github'].includes(a.source)) &&
        (a.profile !== undefined || a.source !== undefined);
      bad('ALLOW_ROW', ok);
      // the sweep profile and a bare "github" source (a pull request gate) are never allowed
      bad('ALLOW_GITHUB', a?.source !== 'github' || a?.profile === 'release');
    }
  const int = (n, lo, hi) => Number.isSafeInteger(n) && n >= lo && n <= hi;
  // the owner's cap: a URL lives at most 12 hours; the data may only lower it
  bad('MAX_TTL', int(value.max_ttl_seconds, 1, 43200));
  bad(
    'MIN_TTL',
    int(value.min_ttl_seconds, 1, 3600) && value.min_ttl_seconds <= value.max_ttl_seconds,
  );
  bad('URLS_PER_RUN', int(value.urls_per_run, 1, 1000));
  return problems;
}
export function checkPrivateConfig(value = PRIVATE_OBJECTS) {
  const problems = privateConfigProblems(value);
  if (problems.length) throw Error('private objects config: ' + problems[0]);
  return value;
}

// The R2 key of a request path's (prefix, object path), or a named refusal. A dot segment is not an object name.
export function privateKey(prefix, path, value = PRIVATE_OBJECTS) {
  checkPrivateConfig(value);
  if (!value.prefixes.some((p) => p.prefix === prefix))
    refuse('FORBIDDEN', 403, 'PREFIX_NOT_PRIVATE');
  const parts = path.split('/');
  if (parts.some((part) => part === '.' || part === '..' || /^\.+$/.test(part)))
    refuse('NOT_FOUND', 404, 'OBJECT_MISSING');
  const key = prefix + '/' + path;
  if (!KEY.test(key)) refuse('NOT_FOUND', 404, 'OBJECT_MISSING');
  return key;
}

// The run record and the verified token fields in, a named refusal out. Order: token (a bad or foreign token learns nothing else),
// liveness, the lease-time scope (token AND record), the profile or source, the URL limit.
export function privateAccess(r, fields, { nowMs, value = PRIVATE_OBJECTS } = {}) {
  checkPrivateConfig(value);
  if (!r || !fields) refuse('UNAUTHORIZED', 401, 'TOKEN_INVALID');
  if (r.ended !== null || r.closed || nowMs >= r.reportExpiry * 1000)
    refuse('UNAUTHORIZED', 401, 'TOKEN_EXPIRED');
  // both must carry the scope: the signed token (decided when it was issued) and the record (the same decision, kept)
  if (!hasPrivateScope(fields.scopes) || !hasPrivateScope(r.scopes))
    refuse('FORBIDDEN', 403, 'TRUST_SCOPE_MISSING');
  const allowed = value.allow.some(
    (a) =>
      (a.profile === undefined || budgetClass(r) === a.profile) &&
      (a.source === undefined || r.source === a.source),
  );
  if (!allowed) refuse('FORBIDDEN', 403, 'TIER_NOT_ALLOWED');
  if ((r.privateMints ?? 0) >= value.urls_per_run) refuse('RATE_LIMITED', 429, 'PRIVATE_URL_LIMIT');
  return r.privateMints ?? 0;
}
export const privateCount = (r, used) => {
  r.privateMints = used + 1;
};
// A URL never outlives the run: the data's cap, or the time left to the run's hard deadline.
export function privateExpiry(nowMs, deadlineMs, value = PRIVATE_OBJECTS) {
  const seconds = Math.min(value.max_ttl_seconds, Math.floor((deadlineMs - nowMs) / 1000));
  if (!(seconds >= value.min_ttl_seconds)) refuse('UNAUTHORIZED', 401, 'TOKEN_EXPIRED');
  return seconds;
}
// One presigned HEAD proves the object exists (no listing): 200 yes, 404 OBJECT_MISSING, anything else is the store being unavailable.
// The URL is never logged or returned.
export async function privateExists(fetcher, env, key, nowMs) {
  const e = benchEndpoint(env);
  const url = await presignGet({
    accessKeyId: e.accessKeyId,
    secretAccessKey: e.secretAccessKey,
    host: e.host,
    path: '/' + e.bucket + '/' + key,
    region: 'auto',
    expires: 60,
    date: new Date(nowMs),
    method: 'HEAD',
  });
  let response;
  try {
    response = await fetcher(url, { method: 'HEAD', redirect: 'manual' });
  } catch {
    refuse('UNAVAILABLE', 503, 'PRIVATE_STORE');
  }
  if (response.status === 200) return true;
  if (response.status === 404) refuse('NOT_FOUND', 404, 'OBJECT_MISSING');
  refuse('UNAVAILABLE', 503, 'PRIVATE_STORE');
}
