// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {hexSha} from './policy.js';
// prettier-ignore
import {validPathPrefix} from './routes.js';
export function onstart(sha, nonce, minutes, repo = 'reindertpelsma/kayfabe', callback) {
  if (
    !hexSha(sha) ||
    typeof nonce !== 'string' ||
    !/^[0-9a-f]{32}$/.test(nonce) ||
    !Number.isInteger(minutes) ||
    minutes < 1 ||
    minutes > DEFAULTS.maxMinutes ||
    !/^[-\w.]+\/[-\w.]+$/.test(repo)
  )
    throw Error('Invalid fixed onstart inputs');
  let origin;
  try {
    const u = new URL(callback.url);
    if (
      u.protocol !== 'https:' ||
      u.username ||
      u.password ||
      u.search ||
      u.hash ||
      u.pathname !== '/'
    )
      throw Error();
    origin = u.origin;
  } catch {
    throw Error('Invalid callback origin');
  }
  if (
    !/^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(callback.run) ||
    !/^v1\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,19}\.[1-9][0-9]{0,12}\.[A-Za-z0-9_-]{43}$/.test(
      callback.token,
    ) ||
    callback.token.split('.').slice(1, 3).join('-') !== callback.run
  )
    throw Error('Invalid callback capability');
  const prefix = callback.path ?? '';
  if (prefix !== '' && !validPathPrefix(prefix)) throw Error('Invalid callback prefix');
  const grace = callback.graceMinutes ?? DEFAULTS.failGraceMinutes;
  if (!Number.isInteger(grace) || grace < 1 || grace > 15) throw Error('Invalid grace');
  if (callback.runnerNonce !== undefined && !/^[0-9a-f]{64}$/.test(callback.runnerNonce))
    throw Error('Invalid runner nonce');
  if (
    callback.softDeadline !== undefined &&
    (!Number.isSafeInteger(callback.softDeadline) || callback.softDeadline <= 0)
  )
    throw Error('Invalid soft deadline');
  return `#!/bin/bash
set -eu
git clone --filter=blob:none --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git fetch --depth=1 origin ${sha}
git checkout --detach ${sha}
[ "$(git rev-parse HEAD)" = '${sha}' ]
exec bash ${DEFAULTS.onstartPath}
`;
}
