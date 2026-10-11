#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
LOG=$W/codex-capacity-after-manual.log
exec >> "$LOG" 2>&1
echo "CAPACITY_WAIT_MANUAL_START=$(date -Is) source=65530e6a"
trap 'rc=$?; echo "CAPACITY_AFTER_MANUAL_EXIT=$rc at=$(date -Is)"' EXIT
# Do not compete for the GPU lock while the already-queued personal session
# waits or runs. No deadline, input, stop marker or reboot is sent to that VM.
while kill -0 209237 2>/dev/null; do sleep 5; done
grep -q '^MANUAL_SESSION_READY=' "$W/codex-prod-defaults-campaign.log"
grep -q '^MANUAL_SESSION_EXIT=' "$W/codex-prod-defaults-campaign.log"
echo "CAPACITY_MANUAL_FINISHED=$(date -Is)"
flock -o /tmp/kayfabe-fastguest.lock bash -c '
set -euo pipefail
W=/var/lib/kf-windows-20261005
test "$(git -C /data/kf-capacity-host rev-parse --short=8 HEAD)" = 65530e6a
bash /tmp/kf-capacity-regression.sh > "$W/codex-capacity-regression.log" 2>&1
bash /tmp/kf-capacity-focus.sh > "$W/codex-capacity-focus.log" 2>&1
'
