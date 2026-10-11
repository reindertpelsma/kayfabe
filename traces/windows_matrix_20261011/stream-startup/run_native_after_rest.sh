#!/usr/bin/env bash
set -euo pipefail
LOG=/tmp/kf-native-direct-stream.log
echo "DIRECT_STREAM_REFERENCE_QUEUED=$(date -Is) waits_for=227144" > "$LOG"
while test -r /proc/227144/cmdline && tr '\0' ' ' < /proc/227144/cmdline | grep -q '/data/kf-native-reference-20261011/rest-reference'; do sleep 10; done
export KF_SSH_HOST=173.185.79.174 KF_SSH_PORT=44316 KF_SSH_USER=vast
export KF_SSH_KNOWN_HOSTS=/tmp/kf-winmatrix-vast-windows-known-hosts
echo "DIRECT_STREAM_REFERENCE_START=$(date -Is) harness=738b5385" >> "$LOG"
set +e
python3 -I /data/kf-stream-harness-738b5385/scripts/bench/windows/appmatrix/native/native_windows_apps.py run \
 --run-dir /data/kf-native-reference-20261011/direct-stream-738b5385 \
 --override-dir /tmp/kf-reference-tools --apps stream_perthread_t --tier 3 --no-isolate >> "$LOG" 2>&1
rc=$?
echo "DIRECT_STREAM_REFERENCE_EXIT=$rc at=$(date -Is)" >> "$LOG"
exit "$rc"
