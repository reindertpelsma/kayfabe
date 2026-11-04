// Reference implementation of the run-row HMAC tag (design section 8, proposal).
// For the CI Worker, which holds ROW_TAG_KEY. The public Worker never imports this file
// (test/static.test.js checks it): it neither verifies nor returns tags.
// tag = base64url(HMAC-SHA256(key, canonical)); canonical = 'kf-row-v1' + LF-joined fields.

export const TAG_FIELDS = Object.freeze([
  'id',
  'commit_sha',
  'boundary_hash',
  'requirement_key',
  'platform_key',
  'suite',
  'verdict',
  'source',
  'tier',
  'finished_at',
]);

export function canonicalRow(row) {
  const parts = ['kf-row-v1'];
  for (const field of TAG_FIELDS) {
    const value = row[field];
    if (typeof value !== 'string' || value === '' || /[\n\r]/.test(value))
      throw new Error('row tag field ' + field);
    parts.push(value);
  }
  return parts.join('\n');
}

const base64url = (bytes) =>
  btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/, '');

async function hmacKey(secret, usage) {
  if (typeof secret !== 'string' || secret.length < 32) throw new Error('row tag key');
  return crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    [usage],
  );
}

export async function rowTag(secret, row) {
  const key = await hmacKey(secret, 'sign');
  const mac = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(canonicalRow(row)));
  return base64url(new Uint8Array(mac));
}

// Constant-time verification through WebCrypto; a malformed tag is simply invalid.
export async function verifyRowTag(secret, row, tag) {
  if (typeof tag !== 'string' || !/^[A-Za-z0-9_-]{43}$/.test(tag)) return false;
  let canonical;
  try {
    canonical = canonicalRow(row);
  } catch {
    return false;
  }
  const padded = tag.replaceAll('-', '+').replaceAll('_', '/') + '=';
  const mac = Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
  const key = await hmacKey(secret, 'verify');
  return crypto.subtle.verify('HMAC', key, mac, new TextEncoder().encode(canonical));
}
