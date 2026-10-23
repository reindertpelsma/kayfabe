// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {fetchJson,refuse} from './policy.js';
const JWKS = 'https://token.actions.githubusercontent.com/.well-known/jwks';
const ISSUER = 'https://token.actions.githubusercontent.com';
export class OidcVerifier {
  constructor(
    fetcher = fetch,
    now = Date.now,
    {
      issuer = ISSUER,
      jwks = JWKS,
      maxLifetime = 3600,
      maxAge = 600,
      requireNbf = true,
      allowAudienceArray = false,
    } = {},
  ) {
    this.fetcher = fetcher;
    this.now = now;
    this.maxLifetime = maxLifetime;
    this.maxAge = maxAge;
    this.requireNbf = requireNbf;
    this.allowAudienceArray = allowAudienceArray;
    this.issuer = issuer;
    this.jwks = jwks;
    this.keys = new Map();
    this.loaded = null;
    this.refreshing = null;
    this.lastRefreshAttempt = null;
  }
  async refresh() {
    if (this.refreshing) return this.refreshing;
    if (
      this.lastRefreshAttempt !== null &&
      this.now() - this.lastRefreshAttempt < DEFAULTS.jwksCooldownMs
    )
      return;
    this.lastRefreshAttempt = this.now();
    this.refreshing = this.loadKeys();
    try {
      await this.refreshing;
    } finally {
      this.refreshing = null;
    }
  }
  async loadKeys() {
    const data = await fetchJson(this.fetcher, this.jwks);
    if (!Array.isArray(data.keys) || data.keys.length > 16) refuse('UNAUTHORIZED', 401);
    const keys = new Map();
    for (const k of data.keys) {
      if (
        k.kty !== 'RSA' ||
        k.alg !== 'RS256' ||
        k.use !== 'sig' ||
        typeof k.kid !== 'string' ||
        k.kid.length > 128
      )
        continue;
      keys.set(
        k.kid,
        await crypto.subtle.importKey(
          'jwk',
          k,
          { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
          false,
          ['verify'],
        ),
      );
    }
    this.keys = keys;
    this.loaded = this.now();
  }
  async verify(token, audience) {
    if (typeof token !== 'string' || token.length > 8192) refuse('UNAUTHORIZED', 401);
    const parts = token.split('.');
    if (parts.length !== 3 || parts.some((p) => !p || !/^[A-Za-z0-9_-]+$/.test(p)))
      refuse('UNAUTHORIZED', 401);
    const decode = (s) =>
      Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
    let h, c, sig;
    try {
      h = JSON.parse(new TextDecoder().decode(decode(parts[0])));
      c = JSON.parse(new TextDecoder().decode(decode(parts[1])));
      sig = decode(parts[2]);
    } catch {
      refuse('UNAUTHORIZED', 401);
    }
    if (
      !h ||
      typeof h !== 'object' ||
      Array.isArray(h) ||
      !c ||
      typeof c !== 'object' ||
      Array.isArray(c)
    )
      refuse('UNAUTHORIZED', 401);
    if (h.alg !== 'RS256' || typeof h.kid !== 'string' || h.jku || h.jwk || h.x5u || h.crit)
      refuse('UNAUTHORIZED', 401);
    const input = new TextEncoder().encode(parts[0] + '.' + parts[1]);
    if (
      this.loaded === null ||
      this.now() - this.loaded > DEFAULTS.jwksCacheMs ||
      !this.keys.has(h.kid)
    )
      await this.refresh();
    const verify = async () =>
      this.keys.has(h.kid) &&
      (await crypto.subtle.verify('RSASSA-PKCS1-v1_5', this.keys.get(h.kid), sig, input));
    if (!(await verify())) refuse('UNAUTHORIZED', 401);
    const seconds = this.now() / 1000;
    const allowedArray =
      this.allowAudienceArray &&
      Array.isArray(c.aud) &&
      c.aud.length > 0 &&
      c.aud.length <= 16 &&
      c.aud.every(
        (value) => typeof value === 'string' && value.length > 0 && value.length <= 256,
      ) &&
      c.aud.includes(audience);
    if (
      !Number.isSafeInteger(c.iat) ||
      c.iat > seconds + 60 ||
      seconds - c.iat > this.maxAge ||
      c.exp <= c.iat ||
      c.exp - c.iat > this.maxLifetime ||
      c.iss !== this.issuer ||
      (c.aud !== audience && !allowedArray) ||
      typeof c.exp !== 'number' ||
      !Number.isFinite(c.exp) ||
      c.exp <= seconds ||
      ((this.requireNbf || c.nbf !== undefined) &&
        (typeof c.nbf !== 'number' || !Number.isFinite(c.nbf) || c.nbf > seconds))
    )
      refuse('UNAUTHORIZED', 401);
    return c;
  }
}
