#!/bin/bash
# Host-side: the passthrough-interrupt hardware cycle. Every step takes the SHARED GPU lock
# (/tmp/kayfabe-fastguest.lock) through the suite scripts themselves, per arm, and releases it.
# usage: pti-run.sh <rev> <step...>   steps: bare | guestA (relay off) | guestB (relay on) | suite30
set -uo pipefail
R=$1; shift
W=/var/lib/kf-windows-20261005
B=$W/pti-bench; mkdir -p $B
cd $W/pti-20261008
export BENCH_DIR=$B KF_DEVICE=kf3
export KF_LADDER=$W/ceint-bins/9925108e/kayfabe-rm-ladder
export KF_FASTGUEST_DIR=$W/ceint-fastguest-9925108e
export QEMU_BIN=$W/kf3-bins/$R/qemu-system-x86_64
echo "PTI_START $(date -Is) rev=$R steps=$* src=$(git rev-parse --short=8 HEAD)"
for step in "$@"; do
  echo "== STEP $step start $(date -Is) qemu=$(pgrep -c qemu-system)"
  case $step in
    bare)    bash scripts/fastguest/bare_metal_suite.sh pti_bare_$R 180 --ce-interrupt 2>&1 | tail -4 ;;
    guestA)  KF3_PT_NSI_RELAY=0 bash scripts/fastguest/fast_suite.sh pti_A_$R 180 --ce-interrupt 2>&1 | tail -4 ;;
    guestB)  bash scripts/fastguest/fast_suite.sh pti_B_$R 180 --ce-client --ce-interrupt 2>&1 | tail -5 ;;
    suite30) bash scripts/fastguest/fast_suite.sh pti_S_$R 180 2>&1 | tail -8 ;;
  esac
  echo "== STEP $step exit $(date -Is)"
done
echo "PTI_DONE $(date -Is)"
