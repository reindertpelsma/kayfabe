#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
TREE=/data/kf-winmatrix-host
REV=2c917f3e
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-smoke-20261011
cleanup() {
  rc=$?; trap - EXIT
  bash "$B" stop 650 || true
  bash "$W/pti-iommu_nogdm.sh" DMA-FQ
  rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-boundary-650
  echo "MATRIX_SMOKE_EXIT=$rc at=$(date -Is)"
}
! pgrep qemu-system
! test -e "$W/boundary-kayfabe-650"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
! test -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS= WIN_DISABLE_MPO=1
export WIN_PRE_GUEST_CMD="bash $W/pti-iommu_nogdm.sh identity"
export WIN_POST_GUEST_CMD="bash $W/pti-iommu_nogdm.sh DMA-FQ"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
bash "$TREE/scripts/bench/windows/appmatrix/run_windows_apps.sh" \
 --run-dir "$RUN" --kf3-rev "$REV" --broker "$B" --run-base 650 \
 --iso "$W/appmatrix/image/kfapps_exfat.img" --apps nvidia_smi,deviceQuery_demo,glgears_wgl,matmul_t \
 --per-guest 4 --no-isolate
