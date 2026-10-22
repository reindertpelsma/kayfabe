#!/bin/bash
# The exact checked-out revision owns payload, reporting and SSH hooks.
set -euo pipefail
umask 077
command -v curl >/dev/null && command -v python3 >/dev/null && command -v git >/dev/null || {
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq curl python3 git ca-certificates
}
install -d -m 700 /run/kf-ci /workspace
python3 - <<'KF_ENV'
import os, re, shlex
keys = ['KF_PROTOCOL', 'KF_BROKER_URL', 'KF_RUN_ID', 'KF_RUN_TOKEN', 'KF_NONCE',
        'KF_SHA', 'KF_REPO', 'KF_BOOT_DEADLINE', 'KF_HARD_DEADLINE', 'KF_SOFT_DEADLINE',
        'KF_HEARTBEAT_SECONDS', 'KF_GRACE_PASS', 'KF_GRACE_FAIL', 'KF_RUNNER_NONCE']
if os.environ.get('KF_PROTOCOL') not in {'4', '5'}:
    raise SystemExit('Unsupported box protocol')
with open('/run/kf-ci/env', 'w') as stream:
    for key in keys:
        value = os.environ[key]
        if not value or '\n' in value or '\r' in value:
            raise SystemExit('Invalid environment contract')
        stream.write('export ' + key + '=' + shlex.quote(value) + '\n')
KF_ENV
cat > /usr/local/bin/kf-box-kill <<'KF_KILL'
#!/bin/sh
set -eu
. /run/kf-ci/env
# Only this run's limited capability; neither account nor owner credentials.
exec curl --silent --show-error --fail --max-time 20 --proto '=https' --request POST --header "Authorization: Bearer $KF_RUN_TOKEN" --data-binary "${1:-box-request}" "$KF_BROKER_URL/kill/$KF_RUN_ID"
KF_KILL
chmod 700 /usr/local/bin/kf-box-kill
cat > /usr/local/bin/kf-box-hold <<'KF_HOLD'
#!/bin/sh
set -eu
. /run/kf-ci/env
# This run only; cannot remove its hard limit or access account credentials.
exec curl --silent --show-error --fail --max-time 20 --proto '=https' --request POST --header "Authorization: Bearer $KF_RUN_TOKEN" --data-binary '' "$KF_BROKER_URL/hold/$KF_RUN_ID"
KF_HOLD
chmod 700 /usr/local/bin/kf-box-hold
cat > /etc/profile.d/kf-ci-hold.sh <<'KF_PROFILE'
# An authenticated SSH login takes MANUAL; ordinary non-login workloads do not.
if [ -n "${SSH_CONNECTION:-}" ]; then
    /usr/local/bin/kf-box-hold >/dev/null 2>&1 || true
fi
KF_PROFILE
chmod 644 /etc/profile.d/kf-ci-hold.sh
exec python3 scripts/ci/vast_report.py
