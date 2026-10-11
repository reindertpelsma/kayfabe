#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
RUN=$W/codex-native-video-events-20261011
PROBE=/tmp/kf-video-event-probe
START=$(date -u '+%Y-%m-%d %H:%M:%S UTC')
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
! test -e "$RUN/probe.log"
mkdir -p "$RUN"
echo "VIDEO_EVENTS_START=$(date -Is)" > "$RUN/probe.log"
sha256sum "$PROBE" >> "$RUN/probe.log"
cleanup() {
 rc=$?; trap - EXIT
 python3 /tmp/kf-capture-xids.py "$RUN" "$START" || true
 echo "VIDEO_EVENTS_EXIT=$rc at=$(date -Is)" >> "$RUN/probe.log"
}
trap cleanup EXIT
for events in events fifo-nsi; do
 for mode in release trap; do
  echo "VIDEO_EVENTS_CASE_START mode=$mode events=$events at=$(date -Is)" >> "$RUN/probe.log"
  set +e
  timeout 30 setpriv --reuid=65534 --regid=65534 --clear-groups "$PROBE" "$mode" 1 "$events" >> "$RUN/probe.log" 2>&1
  rc=$?
  set -e
  echo "VIDEO_EVENTS_CASE_EXIT mode=$mode events=$events rc=$rc at=$(date -Is)" >> "$RUN/probe.log"
  test "$rc" = 0
 done
done
