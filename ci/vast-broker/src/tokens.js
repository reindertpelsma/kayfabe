import { SignJWT, jwtVerify } from 'jose';
import { numericId, hexSha, refuse } from './policy.js';
const tokenEncoder = new TextEncoder();
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
// Platform seam: Cloudflare Workers provide crypto.subtle.timingSafeEqual; plain Node (the
// tests) installs node:crypto's timingSafeEqual with useTimingSafeEqual. Without one, refuse.
let timingSafeEqual = globalThis.crypto?.subtle?.timingSafeEqual?.bind(globalThis.crypto.subtle);
export const useTimingSafeEqual = (compare) => (timingSafeEqual = compare);
export async function constantTimeEqual(expected, provided) {
  if (typeof expected !== 'string' || typeof provided !== 'string' || provided.length > 256)
    return false;
  if (!timingSafeEqual) refuse();
  // Equal-length digests: the platform compare needs equal lengths and leaks no length.
  const digest = async (text) =>
    new Uint8Array(await crypto.subtle.digest('SHA-256', tokenEncoder.encode(text)));
  return timingSafeEqual(await digest(expected), await digest(provided));
}
// Per-run box capability: a jose HS256 JWT over run, attempt, tested sha and the per-rental
// nonce, expiring at the hard limit (owner addendum 2026-10-23 19:45).
export class RunTokens {
  constructor(secret, now = Date.now) {
    this.now = now;
    this.secret = secret;
  }
  key() {
    if (!validSecret(this.secret)) refuse();
    return tokenEncoder.encode(this.secret);
  }
  async issue(r) {
    if (
      !numericId(r.run) ||
      !numericId(r.attempt) ||
      !hexSha(r.sha) ||
      !/^[0-9a-f]{32}$/.test(r.nonce ?? '') ||
      r.key !== r.run + '-' + r.attempt ||
      r.label !== 'kf-ci-' + r.key ||
      !Number.isSafeInteger(r.reportExpiry)
    )
      refuse();
    return new SignJWT({ run: r.run, attempt: r.attempt, sha: r.sha, nonce: r.nonce })
      .setProtectedHeader({ alg: 'HS256' })
      .setExpirationTime(r.reportExpiry)
      .sign(this.key());
  }
  async verifyStateless(token) {
    if (typeof token !== 'string' || token.length > 1024) return null;
    try {
      const { payload: c } = await jwtVerify(token, this.key(), {
        algorithms: ['HS256'],
        currentDate: new Date(this.now()),
        requiredClaims: ['exp'],
      });
      if (
        !numericId(c.run) ||
        !numericId(c.attempt) ||
        !hexSha(c.sha) ||
        !/^[0-9a-f]{32}$/.test(c.nonce)
      )
        return null;
      const key = c.run + '-' + c.attempt;
      return { ...c, key, label: 'kf-ci-' + key, reportExpiry: c.exp };
    } catch {
      return null;
    }
  }
  async verify(token, r) {
    const fields = await this.verifyStateless(token);
    return !!(
      fields &&
      r &&
      fields.run === r.run &&
      fields.attempt === r.attempt &&
      fields.key === r.key &&
      fields.label === r.label &&
      fields.sha === r.sha &&
      fields.nonce === r.nonce &&
      fields.reportExpiry === r.reportExpiry
    );
  }
}
