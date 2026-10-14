#!/bin/bash
# Host-side: run the fast-guest arms; optionally wait until no other tenant's QEMU is on the GPU.
# usage: guestrun.sh <tag> <arm...>      (GUESTRUN_IGNORE_TENANT=1 to run beside other tenants)
set -uo pipefail
W=/var/lib/kf-windows-20261005
TAG=$1; shift
echo "GUESTRUN_START $(date -Is) tag=$TAG arms=$*"
if [ -n "${GUESTRUN_IGNORE_TENANT:-}" ]; then
    echo "GUESTRUN_IGNORING_TENANTS: $(pgrep -a qemu-system | cut -c1-90)"
else
    waited=0
    while [ -n "$(pgrep qemu-system)" ]; do
        sleep 2; waited=$((waited+2))
        if [ $waited -ge ${GUESTRUN_MAX_WAIT:-3000} ]; then echo "GUESTRUN_GAVE_UP waited=${waited}s still: $(pgrep -a qemu-system | cut -c1-90)"; exit 3; fi
    done
    echo "GUESTRUN_QUIET after ${waited}s; qemu now: $(pgrep -c qemu-system)"
fi
cd $W/kayfabe-ceint
export KF_DEVICE=kf3
export BENCH_DIR=$W/ceint-bench
export KF_FASTGUEST_DIR=${KF_FASTGUEST_DIR:-$W/ceint-fastguest}
export QEMU_BIN=$W/kf3-bins/6d9e6a76/qemu-system-x86_64
bash scripts/fastguest/fast_suite.sh "$TAG" 180 "$@"
echo "GUESTRUN_EXIT=$? $(date -Is) qemu-after: $(pgrep -c qemu-system)"
