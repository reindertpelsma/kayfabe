#!/usr/bin/env bash
set -euo pipefail
CURRENT=208739
RUN=/data/kf-native-reference-20261011/rest-reference
LOG=/tmp/kf-native-rest-reference.log
echo "REFERENCE_REST_QUEUED=$(date -Is) waits_for=$CURRENT" > "$LOG"
while test -r "/proc/$CURRENT/cmdline" && tr '\0' ' ' < "/proc/$CURRENT/cmdline" | grep -q '/data/kf-native-reference-20261011/remaining-apps'; do sleep 10; done
# Independent stock validation of all currently unmeasured main-lane rows.
# Previous stock failures stay preserved, GravityMarkVK is already a known stock TIMEOUT.
# GPU jobs remain serial on the owned native instance.
IFS= read -r APPS < /tmp/kf-reference-rest-apps.txt
export KF_SSH_HOST=173.185.79.174 KF_SSH_PORT=44316 KF_SSH_USER=vast
export KF_SSH_KNOWN_HOSTS=/tmp/kf-winmatrix-vast-windows-known-hosts
set +e
python3 -I /data/kf-native-harness-demo-qa/scripts/bench/windows/appmatrix/native/native_windows_apps.py run --run-dir "$RUN" --override-dir /tmp/kf-reference-tools --apps "$APPS" --tier 3 --no-isolate >> "$LOG" 2>&1
rc=$?
set -e
echo "REFERENCE_REST_EXIT=$rc at=$(date -Is)" >> "$LOG"
exit "$rc"
