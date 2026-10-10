#!/bin/bash
# Host-side: the hardware cycle for revision $1 (bare 30 + bare ce-interrupt x3 + unprivileged + kf3 fast guest).
# Waits for a tenant-free GPU (no other QEMU) up to GUESTRUN_MAX_WAIT seconds, then runs anyway and SAYS SO.
set -uo pipefail
R=$1
W=/var/lib/kf-windows-20261005
B=$W/ceint-bench
BIN=$W/ceint-bins/$R/kayfabe-rm-ladder
echo "RUNALL_START $(date -Is) rev=$R"
waited=0
while [ -n "$(pgrep qemu-system)" ]; do
    sleep 3; waited=$((waited+3))
    [ $waited -ge ${GUESTRUN_MAX_WAIT:-2400} ] && { echo "RUNALL_TENANT_STILL_PRESENT after ${waited}s: $(pgrep -a qemu-system | cut -c1-90)"; break; }
done
echo "RUNALL_GPU_STATE waited=${waited}s qemu=$(pgrep -c qemu-system)"
cd $W/kayfabe-ceint
export BENCH_DIR=$B KF_LADDER=$BIN
echo "== bare 30 default arms"
bash scripts/fastguest/bare_metal_suite.sh ceint_F30 180 > $B/ceint_F30_suite.out 2>&1
tail -2 $B/ceint_F30_suite.out | cut -c1-200
for i in 1 2 3; do
    q0=$(pgrep -c qemu-system)
    bash scripts/fastguest/bare_metal_suite.sh ceint_FA$i 180 --ce-interrupt 2>&1 | grep "^--ce-interrupt" | tr -s ' ' | tr '\n' ' '
    echo "tenant-qemu before=$q0 after=$(pgrep -c qemu-system)"
done
echo "== unprivileged (euid 65534, no groups)"
q0=$(pgrep -c qemu-system)
setpriv --reuid=65534 --regid=65534 --clear-groups $BIN --ce-interrupt > $B/ceint_FU_unpriv.log 2>&1
echo "unpriv rc=$? tenant-qemu before=$q0 after=$(pgrep -c qemu-system)"
echo "== guest (kf3 6d9e6a76, initrd with $R's client)"
GUESTRUN_IGNORE_TENANT=1 KF_FASTGUEST_DIR=$W/ceint-fastguest-$R bash $W/ceint-guestrun.sh ceint_fastF --ce-client --ce-interrupt 2>&1 | tail -9 | cut -c1-200
echo "RUNALL_DONE $(date -Is)"
