// AWS Signature Version 4 query-string signing (presigned GET) with WebCrypto only.
// Pure and offline: the caller supplies credentials, time and endpoint. Used for Cloudflare R2's
// S3 API (region "auto", service "s3"); proven against the AWS documentation example in
// test/sigv4.test.js. Only the `host` header is signed and the payload is UNSIGNED-PAYLOAD.
const encoder = new TextEncoder();
const hex = (bytes) =>
  Array.from(new Uint8Array(bytes), (x) => x.toString(16).padStart(2, '0')).join('');
async function hmac(key, text) {
  const k = await crypto.subtle.importKey(
    'raw',
    typeof key === 'string' ? encoder.encode(key) : key,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  return crypto.subtle.sign('HMAC', k, encoder.encode(text));
}
// RFC 3986 unreserved characters stay; everything else is percent-encoded as UTF-8 (uppercase).
export const uriEncode = (text) =>
  encodeURIComponent(text).replace(
    /[!'()*]/g,
    (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase(),
  );
// S3 canonical URI: each path segment is encoded once and slashes are kept.
export const canonicalPath = (path) => path.split('/').map(uriEncode).join('/');
const stamp = (date) =>
  date
    .toISOString()
    .replace(/[-:]/g, '')
    .replace(/\.\d{3}Z$/, 'Z');
export const MAX_PRESIGN_SECONDS = 604800;
// Returns the full presigned URL for GET https://<host><path>. `path` is the unencoded object
// path starting with "/" (bucket included for path-style endpoints).
export async function presignGet({
  accessKeyId,
  secretAccessKey,
  host,
  path,
  region = 'auto',
  service = 's3',
  expires,
  date,
}) {
  if (!Number.isInteger(expires) || expires < 1 || expires > MAX_PRESIGN_SECONDS)
    throw new RangeError('expires');
  if (!(date instanceof Date) || Number.isNaN(date.getTime())) throw new RangeError('date');
  if (typeof path !== 'string' || !path.startsWith('/')) throw new RangeError('path');
  const amzDate = stamp(date),
    day = amzDate.slice(0, 8),
    scope = day + '/' + region + '/' + service + '/aws4_request',
    query = [
      ['X-Amz-Algorithm', 'AWS4-HMAC-SHA256'],
      ['X-Amz-Credential', accessKeyId + '/' + scope],
      ['X-Amz-Date', amzDate],
      ['X-Amz-Expires', String(expires)],
      ['X-Amz-SignedHeaders', 'host'],
    ]
      .map(([k, v]) => [uriEncode(k), uriEncode(v)])
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([k, v]) => k + '=' + v)
      .join('&'),
    uri = canonicalPath(path),
    canonical = ['GET', uri, query, 'host:' + host + '\n', 'host', 'UNSIGNED-PAYLOAD'].join('\n'),
    toSign = [
      'AWS4-HMAC-SHA256',
      amzDate,
      scope,
      hex(await crypto.subtle.digest('SHA-256', encoder.encode(canonical))),
    ].join('\n');
  let key = await hmac('AWS4' + secretAccessKey, day);
  for (const part of [region, service, 'aws4_request']) key = await hmac(key, part);
  const signature = hex(await hmac(key, toSign));
  return 'https://' + host + uri + '?' + query + '&X-Amz-Signature=' + signature;
}
