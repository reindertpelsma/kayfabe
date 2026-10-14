#!/bin/bash
# Host-side: ONE noisy-neighbour run. Holds the SHARED GPU lock for the whole pair (two kf3 guests at
# once), so each guest's harness uses a PRIVATE lock inside it. The neighbour runs from a copy of
# run_fast_guest.sh whose QEMU is named kf-nsi-neighbour and which kills nothing (the stock harness
# pkills every `kf-fastguest`, which would shoot the other guest).
# usage: nsi-noisy.sh <rev> <victim-pace-us> <neighbour-iterations-per-leg>
set -uo pipefail
R=$1; PACE=$2; NIT=${3:-600}
W=/var/lib/kf-nsi-20261008
K=/var/lib/kf-windows-20261005
B=$W/bench; mkdir -p $B
cd /root/kf-nsi-nogate-20261008
sed -e 's/-name kf-fastguest/-name kf-nsi-neighbour/' -e "s/^pkill -9 -f '\[k\]f-fastguest'.*/: # neighbour: kills nothing/" \
    scripts/fastguest/run_fast_guest.sh > $W/neighbour_run_fast_guest.sh
grep -c "kf-nsi-neighbour" $W/neighbour_run_fast_guest.sh >/dev/null || { echo "sed failed"; exit 2; }
export BENCH_DIR=$B KF_DEVICE=kf3 KF_FASTGUEST_DIR=$W/fastguest-$R QEMU_BIN=$K/kf3-bins/$R/qemu-system-x86_64 KF3_FB_MB=4096
cp scripts/fastguest/stranded_tokens.awk $W/ 2>/dev/null
exec 8>/tmp/kayfabe-fastguest.lock
echo "NOISY_WAIT_LOCK $(date -Is)"; flock 8; echo "NOISY_LOCKED $(date -Is) qemu=$(pgrep -c qemu-system)"
KF_LOCK=$W/neighbour.lock KF_ARMS="--ce-interrupt" KF_APPEND="KF_CE_IRQ_ITERATIONS=$NIT " bash $W/neighbour_run_fast_guest.sh nsi_N${NIT}_$R 240 > $W/noisy-neighbour-$R.out 2>&1 &
NP=$!
# wait until the neighbour's client is running
for i in $(seq 1 60); do grep -q "R35" $B/fast_nsi_N${NIT}_${R}_serial.log 2>/dev/null && break; sleep 1; done
echo "NOISY_NEIGHBOUR_RUNNING $(date -Is) after ${i}s qemu=$(pgrep -c qemu-system)"
KF_LOCK=$W/victim.lock KF3_PT_NSI_MIN_INTERVAL_US=$PACE bash scripts/fastguest/fast_suite.sh nsi_V${PACE}_$R 180 --ce-interrupt 2>&1 | tail -4
echo "NOISY_VICTIM_DONE $(date -Is) qemu=$(pgrep -c qemu-system)"
wait $NP; echo "NOISY_NEIGHBOUR_DONE rc=$? $(date -Is)"
tail -3 $W/noisy-neighbour-$R.out
flock -u 8; echo "NOISY_UNLOCKED $(date -Is) qemu=$(pgrep -c qemu-system)"
