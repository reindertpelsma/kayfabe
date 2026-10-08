#!/bin/bash
# Host-side: the non-stall relay hardware cycle for revision $1. The suite scripts take the SHARED GPU
# lock (/tmp/kayfabe-fastguest.lock) per arm themselves and release it; nothing here wraps them.
# usage: nsi-run.sh <rev> <step...>
#   bare    bare metal --ce-interrupt (GR0 watched: the GRCE falsifier), twice
#   guest   kf3 guest --ce-client --ce-interrupt, default settings (no pacing)
#   suite30 the 30 default arms in the kf3 guest
#   paced   kf3 guest --ce-interrupt alone with KF3_PT_NSI_MIN_INTERVAL_US=$PACE_US (no other edge source)
set -uo pipefail
R=$1; shift
W=/var/lib/kf-nsi-20261008
K=/var/lib/kf-windows-20261005
B=$W/bench; mkdir -p $B
cd /root/kf-nsi-nogate-20261008
export BENCH_DIR=$B KF_DEVICE=kf3
export KF_LADDER=$W/bins/$R/kayfabe-rm-ladder
export KF_FASTGUEST_DIR=$W/fastguest-$R
export QEMU_BIN=$K/kf3-bins/$R/qemu-system-x86_64
PACE_US=${PACE_US:-2000}
echo "NSI_START $(date -Is) rev=$R steps=$* src=$(git rev-parse --short=8 HEAD) qemu=$(pgrep -c qemu-system) $(nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader)"
for step in "$@"; do
  echo "== STEP $step start $(date -Is) qemu=$(pgrep -c qemu-system)"
  case $step in
    bare)    for i in 1 2; do bash scripts/fastguest/bare_metal_suite.sh nsi_bare${i}_$R 180 --ce-interrupt 2>&1 | grep "^--ce-interrupt"; done ;;
    guest)   bash scripts/fastguest/fast_suite.sh nsi_G_$R 180 --ce-client --ce-interrupt 2>&1 | tail -5 ;;
    suite30) bash scripts/fastguest/fast_suite.sh nsi_S_$R 180 2>&1 | tail -8 ;;
    guestwait) KF_APPEND="KF_POWEROFF_DELAY_S=5 " bash scripts/fastguest/fast_suite.sh nsi_W_$R 180 --ce-interrupt 2>&1 | tail -4 ;;
    paced)   KF3_PT_NSI_MIN_INTERVAL_US=$PACE_US bash scripts/fastguest/fast_suite.sh nsi_P${PACE_US}_$R 180 --ce-interrupt 2>&1 | tail -4 ;;
  esac
  echo "== STEP $step exit $(date -Is) qemu=$(pgrep -c qemu-system)"
done
echo "NSI_DONE $(date -Is)"
