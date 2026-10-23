// GitHub Actions OIDC verification by jose (owner decision 2026-10-23 19:35): signature,
// algorithm, issuer, audience and time claims. Repository/workflow claims are checked after
// this by the caller. Key fetches are cached and rate-bounded by jose (cacheMaxAge, cooldown).
import { createRemoteJWKSet, customFetch, errors, jwtVerify } from 'jose';
import { DEFAULTS } from './defaults.js';
import { refuse } from './policy.js';
const JWKS = 'https://token.actions.githubusercontent.com/.well-known/jwks';
const ISSUER = 'https://token.actions.githubusercontent.com';
export class OidcVerifier {
  constructor(fetcher = fetch, now = Date.now, { issuer = ISSUER, jwks = JWKS } = {}) {
    this.now = now;
    this.issuer = issuer;
    this.keys = createRemoteJWKSet(new URL(jwks), {
      cacheMaxAge: DEFAULTS.jwksCacheMs,
      cooldownDuration: DEFAULTS.jwksCooldownMs,
      timeoutDuration: 8000,
      [customFetch]: fetcher,
    });
  }
  async verify(token, audience) {
    if (typeof token !== 'string' || token.length > 8192) refuse('UNAUTHORIZED', 401);
    try {
      const { payload } = await jwtVerify(token, this.keys, {
        algorithms: ['RS256'],
        issuer: this.issuer,
        audience,
        currentDate: new Date(this.now()),
        clockTolerance: 30,
        maxTokenAge: 600,
        requiredClaims: ['iat', 'nbf', 'exp'],
      });
      return payload;
    } catch (error) {
      // A key-set fetch that timed out or failed on the network is an outage (503). A JWKS HTTP
      // error status is a plain JOSEError and refuses 401 (final review J1, measured).
      if (error instanceof errors.JWKSTimeout || !(error instanceof errors.JOSEError)) refuse();
      refuse('UNAUTHORIZED', 401);
    }
  }
}
