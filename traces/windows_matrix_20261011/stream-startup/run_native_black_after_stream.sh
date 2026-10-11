#!/usr/bin/env bash
set -euo pipefail
LOG=/tmp/kf-native-black-frozen.log
echo "FROZEN_BLACK_REFERENCE_QUEUED=$(date -Is) waits_for=234980" > "$LOG"
while test -r /proc/234980/cmdline && tr '\0' ' ' < /proc/234980/cmdline | grep -q 'stream-startup/run_native_after_rest.sh'; do sleep 10; done
grep -q '^DIRECT_STREAM_REFERENCE_EXIT=' /tmp/kf-native-direct-stream.log
export KF_SSH_HOST=173.185.79.174 KF_SSH_PORT=44316 KF_SSH_USER=vast
export KF_SSH_KNOWN_HOSTS=/tmp/kf-winmatrix-vast-windows-known-hosts
echo "FROZEN_BLACK_REFERENCE_START=$(date -Is) harness=738b5385" >> "$LOG"
set +e
python3 -I /data/kf-stream-harness-738b5385/scripts/bench/windows/appmatrix/native/native_windows_apps.py run \
 --run-dir /data/kf-native-reference-20261011/black-frozen-738b5385 \
 --override-dir /tmp/kf-reference-tools --apps blackscholes_t --tier 3 --no-isolate >> "$LOG" 2>&1
rc=$?
echo "FROZEN_BLACK_REFERENCE_EXIT=$rc at=$(date -Is)" >> "$LOG"
exit "$rc"
