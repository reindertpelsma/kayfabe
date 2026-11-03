#!/bin/bash
# Start the already-approved first arm only after successful sealed-parent preparation.
set -euo pipefail
exec >/root/prov/first_on.log 2>&1
trap 'rc=$?; echo "FIRST_ON_CONTROLLER_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "FIRST_ON_WAIT_START pid=$$ $(date -u -Is)"
echo 'H-ON falsifier BEFORE run: actual Code43 after cursor4; preserve actual RPC ordinals. Code0 requires600second hold and real applications for PASS.'
start=$SECONDS
while ! grep -q '^SWEEP_PROVISION_EXIT ' /root/prov/sweep_20261020_provision.log; do
 kill -0 2002
 [ $((SECONDS-start)) -lt 5400 ]
 sleep 5
done
grep '^SWEEP_PROVISION_EXIT rc=0 ' /root/prov/sweep_20261020_provision.log
grep '^SWEEP2_BIND_READY ' /root/prov/sweep_20261020_provision.log
[ ! -e /root/prov/arm_bw_on_m1.started ]
echo "FIRST_ON_RELEASE $(date -u -Is)"
bash /root/sweep2/arm.sh on bw_on_m1 /workspace/bench/kf3-bins/af8bdcda/qemu-system-x86_64 af8bdcda4c700d8b40163ee3bf27feb7cfa6539c
