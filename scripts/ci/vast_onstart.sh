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
cat > /usr/local/bin/kf-box-blob-url <<'KF_BLOB'
#!/bin/sh
set -eu
. /run/kf-ci/env
[ "$#" -eq 2 ] || exit 2
for value in "$1" "$2"; do
  case "$value" in ''|.|..|*[!A-Za-z0-9._-]*) exit 2;; esac
done
[ "${#1}" -le 64 ] && [ "${#2}" -le 100 ] || exit 2
exec curl --silent --show-error --fail --max-time 20 --proto '=https' --header "Authorization: Bearer $KF_RUN_TOKEN" "$KF_BROKER_URL/blob-url/$KF_RUN_ID/$1/$2"
KF_BLOB
chmod 700 /usr/local/bin/kf-box-blob-url
cat > /etc/profile.d/kf-ci-hold.sh <<'KF_PROFILE'
# An authenticated SSH login takes MANUAL; ordinary non-login workloads do not.
if [ -n "${SSH_CONNECTION:-}" ]; then
    /usr/local/bin/kf-box-hold >/dev/null 2>&1 || true
fi
KF_PROFILE
chmod 644 /etc/profile.d/kf-ci-hold.sh
# Deliberate lifecycle faults are confined to this dummy branch's checked-in mode.
mode=$(cat scripts/ci/vast_dummy_mode)
case "$mode" in
  D) exec sleep 600 ;;
  C)
    curl --silent --show-error --fail --max-time 20 --proto '=https' --request POST \
      --header "Authorization: Bearer $KF_RUN_TOKEN" --header 'Content-Type: application/json' \
      --data-binary '{"seq":1,"kind":"progress","text":"DUMMY_C one report then silent"}' \
      "$KF_BROKER_URL/report/$KF_RUN_ID" >/dev/null
    exec sleep 600 ;;
esac
python3 scripts/ci/vast_report.py
if [ "$mode" = B ]; then kf-box-kill dummy-pass-complete; fi
