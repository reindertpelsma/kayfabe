#!/usr/bin/env bash
# Caller takes /tmp/kayfabe-fastguest.lock with flock -o.
set -euo pipefail
W=/var/lib/kf-windows-20261005
trap 'rc=$?; echo RUN643_EXIT=$rc; rm -f "$W/kf3-bins/02afce90"' EXIT
if pgrep qemu-system-x86; then exit 1; fi
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$W/boundary-kayfabe-643"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 112742891520
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW KF3_REV_BIN=02afce90 HOLD_SECS=10 DWM_OVERLAY_OFF=1 GLTEST=1 SKIP_SHORTS=1 RUN_WIN_FLAGS=KF3_RPC_TRACE=1
echo RUN643_START="$(date -Is)"
bash /data/kf-gl730101-host/scripts/bench/windows/glcrash/tdr-run29.sh 643
