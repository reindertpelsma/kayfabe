// prettier-ignore
import {OidcVerifier} from './oidc.js';
// prettier-ignore
import {refuse} from './policy.js';
// prettier-ignore
import {routeKind,validPathPrefix} from './routes.js';
const accessVars = [
  'ACCESS_TEAM_DOMAIN',
  'ACCESS_AUD',
  'ACCESS_RUNNER_CLIENT_ID',
  'ACCESS_BOX_CLIENT_ID',
];
export class AccessPolicy {
  constructor(fetcher, now) {
    this.fetcher = fetcher;
    this.now = now;
    this.verifiers = new Map();
  }
  async authorize(request, env) {
    if (accessVars.every((key) => !env[key])) return;
    for (const key of accessVars)
      if (typeof env[key] !== 'string' || !env[key]) refuse('CONFIG_ACCESS:' + key, 503);
    if (
      !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.cloudflareaccess\.com$/.test(env.ACCESS_TEAM_DOMAIN)
    )
      refuse('CONFIG_ACCESS:ACCESS_TEAM_DOMAIN', 503);
    for (const key of accessVars.slice(1))
      if (!/^[A-Za-z0-9_.-]{1,256}$/.test(env[key])) refuse('CONFIG_ACCESS:' + key, 503);
    if (env.ACCESS_RUNNER_CLIENT_ID === env.ACCESS_BOX_CLIENT_ID)
      refuse('CONFIG_ACCESS:ACCESS_BOX_CLIENT_ID', 503);
    const issuer = 'https://' + env.ACCESS_TEAM_DOMAIN;
    let verifier = this.verifiers.get(issuer);
    if (!verifier) {
      if (this.verifiers.size >= 4) this.verifiers.clear();
      verifier = new OidcVerifier(this.fetcher, this.now, {
        issuer,
        jwks: issuer + '/cdn-cgi/access/certs',
        maxLifetime: 86400,
        maxAge: 86400,
        requireNbf: false,
        allowAudienceArray: true,
      });
      this.verifiers.set(issuer, verifier);
    }
    let claims;
    try {
      claims = await verifier.verify(
        request.headers.get('Cf-Access-Jwt-Assertion'),
        env.ACCESS_AUD,
      );
    } catch {
      refuse('FORBIDDEN', 403);
    }
    // Prefix comparison still happens in routePath; here only classify the suffix.
    const path = new URL(request.url).pathname;
    const scoped = env.BROKER_PATH_PREFIX ? '/' + path.split('/').slice(2).join('/') : path;
    const invalidPrefix = env.BROKER_PATH_PREFIX && !validPathPrefix(env.BROKER_PATH_PREFIX);
    const kind = invalidPrefix && path.endsWith('/availability') ? 'runner' : routeKind(scoped);
    // Only Access A reaches this diagnostic candidate; owner OIDC is checked next.
    if (
      !kind ||
      claims.common_name !==
        (kind === 'runner' ? env.ACCESS_RUNNER_CLIENT_ID : env.ACCESS_BOX_CLIENT_ID)
    )
      refuse('FORBIDDEN', 403);
  }
}
