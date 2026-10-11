#!/usr/bin/env bash
# Single Windows regression of the truthful caps; caller holds the GPU flock.
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=02afce90
test ! -e "$W/winprod/run642"
test ! -e "$W/boundary-kayfabe-642"
if pgrep qemu-system-x86 >/dev/null; then exit 1; fi
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
available=$(df -B1 --output=avail /var/lib | tail -1)
test "$available" -gt 118111600640 # keep more than 100 GiB free after the expected ~3 GiB overlay
LINK=$W/kf3-bins/$REV
CREATED_LINK=0
trap 'rc=$?; if [ "$CREATED_LINK" = 1 ]; then rm -f "$LINK"; fi; echo "WINDOWS_CHECK_EXIT rc=$rc $(date -Is)"' EXIT
if [ ! -e "$LINK" ] && [ ! -L "$LINK" ]; then
    ln -s "$W/readtrace/kf3-bins/$REV" "$LINK"
    CREATED_LINK=1
fi
cmp "$LINK/qemu-system-x86_64" "$W/readtrace/kf3-bins/$REV/qemu-system-x86_64"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
test -n "$KF_GUEST_PW"
export KF_GUEST_PW
rm -f /tmp/kf-stop-winprod
export KF3_REV_BIN=$REV HOLD_SECS=10 DWM_OVERLAY_OFF=1 GLTEST=1 SKIP_SHORTS=1
export RUN_WIN_FLAGS=KF3_RPC_TRACE=1
bash /data/kf-gl730101-host/scripts/bench/windows/glcrash/tdr-run28.sh 642
for probe in cup2 cup3 cup8; do
    grep -q "$probe.exe exit=0" "$W/winprod/run642/gl-result.txt"
done
grep -q 'ADAPTER NVIDIA.*code=0' "$W/winprod/run642/winprod.log"
grep -q 'DONE tdr_cycles=0' "$W/winprod/run642/winprod.log"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
if pgrep qemu-system-x86 >/dev/null; then exit 1; fi
