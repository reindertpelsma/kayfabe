#!/usr/bin/env bash
set -euo pipefail
echo "NATIVE_VIDEO_DEFERRED_BATCH_START=$(date -Is)"
trap 'rc=$?; echo "NATIVE_VIDEO_DEFERRED_BATCH_EXIT=$rc at=$(date -Is)"' EXIT
set +e
bash /tmp/kf-native-video-event-cases.sh
video_rc=$?
bash /tmp/kf-deferred-update-native.sh
deferred_rc=$?
set -e
echo "NATIVE_BATCH_CASE_STATUS video=$video_rc deferred=$deferred_rc"
test "$video_rc" = 0 && test "$deferred_rc" = 0
