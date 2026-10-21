import {hexSha,IMAGE} from './policy.js';
export function onstart(sha,nonce,minutes,repo='reindertpelsma/kayfabe'){
  if(!hexSha(sha)||typeof nonce!=='string'||!/^[0-9a-f]{32}$/.test(nonce)||!Number.isInteger(minutes)||minutes<1||minutes>150||!/^[-\w.]+\/[-\w.]+$/.test(repo))throw Error('Invalid fixed onstart inputs');
  return `#!/bin/bash
set -uo pipefail
summary=$(mktemp)
if timeout --signal=TERM --kill-after=30s ${minutes*60}s bash -s <<'KF_CI_WORK' 2>&1 | tee "$summary"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
command -v git >/dev/null || { apt-get update -qq; apt-get install -y -qq git ca-certificates; }
git clone --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src
cd /workspace/kf-ci-src
git fetch origin ${sha}
git checkout --detach ${sha}
[ "$(git rev-parse HEAD)" = '${sha}' ]
bash scripts/ci/vast_box_main.sh '${sha}'
KF_CI_WORK
then result=PASS; else result=FAIL; fi
digest=$(sha256sum "$summary"); digest=\${digest%% *}
printf 'KF_RESULT ${nonce} %s %s\\n' "$result" "$digest"
`;
}
