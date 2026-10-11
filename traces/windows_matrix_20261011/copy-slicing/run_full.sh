#!/usr/bin/env bash
set -euo pipefail
RUN_START=$(date -u "+%Y-%m-%d %H:%M:%S UTC")
W=/var/lib/kf-windows-20261005
TREE=/data/kf-slice-host-0f9c01f9
REV=0f9c01f9
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-slice-full-20261011
POST=/tmp/kf-slice-post.sh
cleanup() {
 rc=$?; trap - EXIT
 # BrokerVm normally stops each guest; recover only this campaign's active run.
 for state in "$W"/windows-broker-run{851..1049}.state; do
  test -f "$state" || continue
  n=${state##*run}; n=${n%.state}
  bash "$B" stop "$n" || true
 done
 bash "$POST"
 python3 /tmp/kf-capture-xids.py "$RUN" "$RUN_START" || true
 rm -f "$W/kf3-bins/$REV"
 echo "MATRIX_SLICE_FULL_EXIT=$rc at=$(date -Is)"
}
! pgrep qemu-system
! test -e "$RUN/session.log"
for n in {851..1049}; do ! test -e "$W/boundary-kayfabe-$n"; done
# Never promote a failed smoke to an unattended long run.
test "$(grep -c 'verdict=PASS' "$W/codex-matrix-slice-focus-20261011/win.res")" = 6
! test -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt 107374182400 && bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" \
 --run-dir "$RUN" --kf3-rev "$REV" --broker "$B" --run-base 851 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --iso "$W/appmatrix/image/kfapps_exfat.img" --apps "$(cat /tmp/kf-slice-app-order.txt)" --tier 3 --per-guest 8 \
 --exclude overlay_yuv_fullscreen,overlay_yuv_hide,overlay_yuv_move,overlay_yuv_occlude,overlay_yuv_probe,overlay_yuv_recreate,overlay_yuv_resize,overlay_yuv_soak
