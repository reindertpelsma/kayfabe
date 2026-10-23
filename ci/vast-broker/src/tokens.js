// prettier-ignore
import {numericId,hexSha,refuse} from './policy.js';
const tokenEncoder = new TextEncoder();
const tag64 = (bytes) =>
  btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/, '');
const hmacKey = (secret) =>
  crypto.subtle.importKey(
    'raw',
    tokenEncoder.encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign', 'verify'],
  );
export const validSecret = (value) =>
  typeof value === 'string' && /^[A-Za-z0-9_-]{32,256}$/.test(value);
export async function sha256Hex(value) {
  return Array.from(
    new Uint8Array(
      await crypto.subtle.digest(
        'SHA-256',
        typeof value === 'string' ? tokenEncoder.encode(value) : value,
      ),
    ),
    (x) => x.toString(16).padStart(2, '0'),
  ).join('');
}
export async function logChain(previous, text) {
  const bytes = tokenEncoder.encode(text),
    wire = new Uint8Array(36 + bytes.length);
  wire.set(Uint8Array.from(previous.match(/../g), (x) => parseInt(x, 16)));
  new DataView(wire.buffer).setUint32(32, bytes.length);
  wire.set(bytes, 36);
  return sha256Hex(wire);
}
export async function constantTimeEqual(expected, provided) {
  if (typeof expected !== 'string' || typeof provided !== 'string' || provided.length > 256)
    return false;
  // Native HMAC verification compares fixed-size tags inside the crypto implementation.
  const key = await hmacKey('kayfabe-owner-stop-comparison-v1');
  const tag = await crypto.subtle.sign('HMAC', key, tokenEncoder.encode(expected));
  return crypto.subtle.verify('HMAC', key, tag, tokenEncoder.encode(provided));
}
export class RunTokens {
  constructor(secret) {
    this.secret = secret;
    this.key = null;
  }
  async cryptoKey() {
    if (!validSecret(this.secret)) refuse();
    return (this.key ??= await hmacKey(this.secret));
  }
  message(r, expiry) {
    return ['v1', r.run, r.attempt, r.sha, r.label, expiry].join('|');
  }
  async issue(r) {
    if (
      !numericId(r.run) ||
      !numericId(r.attempt) ||
      !hexSha(r.sha) ||
      r.key !== r.run + '-' + r.attempt ||
      r.label !== 'kf-ci-' + r.key ||
      !Number.isSafeInteger(r.reportExpiry)
    )
      refuse();
    const tag = new Uint8Array(
      await crypto.subtle.sign(
        'HMAC',
        await this.cryptoKey(),
        tokenEncoder.encode(this.message(r, r.reportExpiry)),
      ),
    );
    return ['v1', r.run, r.attempt, r.reportExpiry, tag64(tag)].join('.');
  }
  async verify(token, r) {
    try {
      const parts =
        /^v1\.([1-9][0-9]{0,19})\.([1-9][0-9]{0,19})\.([1-9][0-9]{0,12})\.([A-Za-z0-9_-]{43})$/.exec(
          token || '',
        );
      if (
        !parts ||
        !r ||
        parts[1] !== r.run ||
        parts[2] !== r.attempt ||
        Number(parts[3]) !== r.reportExpiry ||
        r.key !== r.run + '-' + r.attempt ||
        r.label !== 'kf-ci-' + r.key ||
        !hexSha(r.sha)
      )
        return false;
      const bytes = Uint8Array.from(
        atob(parts[4].replaceAll('-', '+').replaceAll('_', '/') + '='),
        (x) => x.charCodeAt(0),
      );
      if (bytes.length !== 32 || tag64(bytes) !== parts[4]) return false;
      return await crypto.subtle.verify(
        'HMAC',
        await this.cryptoKey(),
        bytes,
        tokenEncoder.encode(this.message(r, Number(parts[3]))),
      );
    } catch {
      return false;
    }
  }
}
