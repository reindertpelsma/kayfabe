#!/usr/bin/env bash
# Caller owns the serial GPU lock for the complete session.
set -euo pipefail
W=/var/lib/kf-windows-20261005
ROOT=$W/codex-vfio-gravity-20261011
HERE=$ROOT/tooling
REF=/data/kf-gl730101-host/scripts/bench/windows/glcrash/vfio_gl_reference.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
export VDR_DIR=$ROOT/vfio VDR_TRACE_BAR0=0
START=$(date -u '+%Y-%m-%d %H:%M:%S.%6N UTC')
! pgrep qemu-system
! test -e "$ROOT/lane/session.log"
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 128849018880
echo "VFIO_GRAVITY_REFERENCE_START=$(date -Is) guest_driver=580.88 gpu=RTX4070"
cleanup() {
 rc=$?; trap - EXIT
 for n in 1 2 3; do
  test -e "$VDR_DIR/boot$n/qemu.pid" || continue
  bash "$REF" stop "$n" || true
 done
 if pgrep qemu-system; then
  echo VFIO_GRAVITY_REFERENCE_UNSAFE_TO_RESTORE
  exit 1
 fi
 bash "$REF" restore
 bash /tmp/kf-nbody-stall-post.sh
 python3 /tmp/kf-capture-xids.py "$ROOT/lane" "$START" || true
 echo "VFIO_GRAVITY_REFERENCE_EXIT=$rc at=$(date -Is)"
}
trap cleanup EXIT
# Falsifier: same workload under VFIO exhibits device-lost/recovery too.
# A timeout without device-lost is a different symptom, not a passing baseline.
# A clean reference pass supports a kayfabe defect but identifies no method.
# Original inventory, 600s deadline, proof oracle and pinned bench unchanged.
bash "$REF" detach
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW WIN_DISABLE_MPO=1
unset WIN_PRE_GUEST_CMD WIN_POST_GUEST_CMD
python3 -I "$HERE/winapps_vfio.py" --run-dir "$ROOT/lane" \
 --broker "$REF" --w-dir "$W" --kf3-rev reference-vfio-580.88 --run-base 1 \
 --iso "$MEDIA" --image-manifest "$(dirname "$MEDIA")/manifest.json" \
 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --apps gravitymark_vk --tier 3 --per-guest 1 --no-isolate
