// Licensed benchmark copy in a private R2 bucket (docs/DOWNLOADS.md, "SPECviewperf private copy:
// handling rules"). A box with a live run token asks for one allow-listed object and gets a 302 to a
// short-lived presigned S3 GET URL. The Worker holds a READ-ONLY R2 S3 key pair and signs locally
// (src/sigv4.js); it makes no network call here, and no Cloudflare credential reaches a box.
// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {refuse} from './policy.js';
// prettier-ignore
import {presignGet} from './sigv4.js';
// The only names a box may request, mapped to R2 object keys. Anything else is a 404: no listing,
// no pattern, no path traversal. A Map lookup, so names like "__proto__" are simply absent.
export const BENCH_OBJECTS = new Map([
  ['specviewperf-15.0.0.tar.zst', 'specviewperf-15.0.0/specviewperf-15.0.0.tar.zst'],
  ['specviewperf-15.0.0.CHECKSUMS.txt', 'specviewperf-15.0.0/CHECKSUMS.txt'],
  ['specviewperf-15.0.0.SPEC_General_License.pdf', 'specviewperf-15.0.0/SPEC_General_License.pdf'],
  [
    'specviewperf-15.0.0.SPECviewperf-Setup.exe',
    'specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe',
  ],
]);
export function benchObject(name) {
  const key = BENCH_OBJECTS.get(name);
  if (key === undefined) refuse('NOT_FOUND', 404);
  return key;
}
// At most N presigned URLs per run token per object; a mint counts when it is signed.
export function benchBudget(record, name) {
  const used = record.benchMints?.[name] ?? 0;
  if (used >= DEFAULTS.benchDataUrlsPerObject) refuse('RATE_LIMITED', 429, 'BENCH_DATA_URL_LIMIT');
  return used;
}
export function benchCount(record, name, used) {
  record.benchMints = { ...(record.benchMints ?? {}), [name]: used + 1 };
}
// Validated lazily so a missing R2 setting only breaks this route.
function benchEndpoint(env) {
  const good = (value, re) => typeof value === 'string' && re.test(value);
  if (
    !good(env.R2_ACCOUNT_ID, /^[0-9a-f]{32}$/) ||
    !good(env.R2_BUCKET, /^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$/) ||
    !good(env.R2_READ_ACCESS_KEY_ID, /^[A-Za-z0-9]{16,128}$/) ||
    !good(env.R2_READ_SECRET_ACCESS_KEY, /^[A-Za-z0-9/+=_-]{32,128}$/)
  )
    refuse('UNAVAILABLE', 503, 'CONFIG_BENCH_DATA');
  return {
    host: env.R2_ACCOUNT_ID + '.r2.cloudflarestorage.com',
    bucket: env.R2_BUCKET,
    accessKeyId: env.R2_READ_ACCESS_KEY_ID,
    secretAccessKey: env.R2_READ_SECRET_ACCESS_KEY,
  };
}
// The URL never outlives the run: lifetime is the cap, or the time left to the run's next deadline.
export function benchExpiry(nowMs, deadlineMs) {
  const seconds = Math.min(
    DEFAULTS.benchDataMaxExpirySeconds,
    Math.floor((deadlineMs - nowMs) / 1000),
  );
  if (!(seconds >= DEFAULTS.benchDataMinExpirySeconds)) refuse('UNAUTHORIZED', 401);
  return seconds;
}
export async function benchUrl(env, objectKey, expires, nowMs) {
  const e = benchEndpoint(env);
  return presignGet({
    accessKeyId: e.accessKeyId,
    secretAccessKey: e.secretAccessKey,
    host: e.host,
    path: '/' + e.bucket + '/' + objectKey,
    region: 'auto',
    expires,
    date: new Date(nowMs),
  });
}
export const benchRedirect = (location) =>
  new Response(null, {
    status: 302,
    headers: {
      Location: location,
      'Cache-Control': 'no-store',
      'Referrer-Policy': 'no-referrer',
      'X-Content-Type-Options': 'nosniff',
    },
  });
