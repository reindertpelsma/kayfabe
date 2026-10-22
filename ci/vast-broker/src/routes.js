// prettier-ignore
import {constantTimeEqual} from './tokens.js';
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
  if (prefix === undefined || prefix === '') return path;
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
