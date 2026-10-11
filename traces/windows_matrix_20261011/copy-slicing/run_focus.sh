#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-slice-host-0f9c01f9
REV=0f9c01f9
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-slice-focus-20261011
cleanup() {
  rc=$?; trap - EXIT
  bash "$B" stop 850 || true
  bash /tmp/kf-slice-post.sh
  rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-boundary-850
  echo "MATRIX_SLICE_FOCUS_EXIT=$rc at=$(date -Is)"
}
! pgrep qemu-system
! test -e "$W/boundary-kayfabe-850"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
! test -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash /tmp/kf-slice-post.sh"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" \
 --run-dir "$RUN" --kf3-rev "$REV" --broker "$B" --run-base 850 \
 --iso "$W/appmatrix/image/kfapps_exfat.img" --override-dir "$W/codex-matrix-focus-tools-20261011" --tier 3 --apps blackscholes_t,cuda_callback,cuda_occupancy,deviceQuery_demo,glgears_wgl,matmul_t \
 --per-guest 6 --no-isolate
