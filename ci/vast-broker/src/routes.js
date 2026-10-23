// prettier-ignore
import {constantTimeEqual} from './tokens.js';
// prettier-ignore
import {refuse} from './policy.js';
// This table is the gateway's route recognition.
export function routeKind(path) {
  if (
    /^\/(?:rent|availability|stop-all)$/.test(path) ||
    /^\/(?:status|stop|logs)\/[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(path)
  )
    return 'runner';
  if (
    /^\/(?:report|kill)\/[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(path) ||
    new RegExp(
      '^/blob-url/[1-9][0-9]{0,19}-[1-9][0-9]{0,19}/' +
        '[A-Za-z0-9._-]{1,64}/[A-Za-z0-9._-]{1,100}$',
    ).test(path)
  )
    return 'box';
  return null;
}
export function secretFormatError(env) {
  for (const name of ['RUN_TOKEN_SECRET', 'OWNER_STOP_TOKEN'])
    if (typeof env[name] !== 'string' || !/^[A-Za-z0-9_-]{32,256}$/.test(env[name]))
      return 'CONFIG_SECRET_FORMAT:' + name;
  if (env.BROKER_PATH_PREFIX && !validPathPrefix(env.BROKER_PATH_PREFIX))
    return 'CONFIG_SECRET_FORMAT:BROKER_PATH_PREFIX';
  return null;
}
export const validPathPrefix = (value) =>
  typeof value === 'string' && /^[A-Za-z0-9_-]{32,128}$/.test(value);
export const notFound = () =>
  new Response('Not found', {
    status: 404,
    headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
  });
export async function routePath(request, env) {
  const path = new URL(request.url).pathname,
    prefix = env.BROKER_PATH_PREFIX;
  if (prefix === undefined || prefix === '') {
    if (env.RENTING_ENABLED === 'true') refuse('PREFIX_REQUIRED', 503);
    return path;
  }
  if (!validPathPrefix(prefix)) return null;
  const parts = path.split('/');
  if (!(await constantTimeEqual(prefix, parts[1] || ''))) return null; // MUTANT prefix-match
  return '/' + parts.slice(2).join('/');
}
export function endpointSecrets(env) {
  const values = [env.WORKER_URL, env.BROKER_PATH_PREFIX];
  try {
    const url = new URL(env.WORKER_URL),
      origin = url.origin;
    values.push(
      origin,
      url.hostname,
      ...url.hostname.split('.'),
      encodeURIComponent(env.WORKER_URL),
      encodeURIComponent(origin),
    );
    if (validPathPrefix(env.BROKER_PATH_PREFIX)) values.push(origin + '/' + env.BROKER_PATH_PREFIX);
  } catch {}
  return [...new Set(values.filter((x) => typeof x === 'string' && x))].sort(
    (a, b) => b.length - a.length,
  );
}
