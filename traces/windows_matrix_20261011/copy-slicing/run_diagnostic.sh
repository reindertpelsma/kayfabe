#!/usr/bin/env bash
set -euo pipefail
RUN_START=$(date -u "+%Y-%m-%d %H:%M:%S UTC")
W=/var/lib/kf-windows-20261005
TREE=/data/kf-slice-host-0f9c01f9
REV=0f9c01f9
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
RUN=$W/codex-matrix-slice-diagnostic-20261011
cleanup() {
  rc=$?; trap - EXIT
  for n in {852..855}; do test ! -f "$W/windows-broker-run$n.state" || bash "$B" stop "$n" || true; done
  bash /tmp/kf-slice-post.sh
  python3 /tmp/kf-capture-xids.py "$RUN" "$RUN_START" || true
  rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-boundary-852
  echo "MATRIX_SLICE_DIAGNOSTIC_EXIT=$rc at=$(date -Is)"
}
! pgrep qemu-system
! test -e "$W/boundary-kayfabe-852"
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
 --run-dir "$RUN" --kf3-rev "$REV" --broker "$B" --run-base 852 \
 --iso "$W/appmatrix/image/kfapps_exfat.img" --override-dir "$W/codex-matrix-focus-tools-20261011" --tier 3 --apps-json /tmp/kf-slice-diagnostic-apps.json --apps glgears_wgl,torch_startup_diagnostic,blackscholes_t \
 --per-guest 3 --no-isolate
