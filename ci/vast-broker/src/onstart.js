import { DEFAULTS } from './defaults.js';
import { hexSha } from './policy.js';
// The provider's onstart: clone, check out the exact tested commit, exec the repo-side script.
// Only the two interpolated values are inputs; per-run data travels in the KF_* environment.
export function onstart(sha, repo) {
  if (!hexSha(sha) || !/^[-\w.]+\/[-\w.]+$/.test(repo)) throw Error('Invalid fixed onstart inputs');
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
