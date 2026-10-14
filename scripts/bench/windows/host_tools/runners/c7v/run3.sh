#!/bin/bash
W=/var/lib/kf-windows-20261005; C=$W/c7v
cd $C/src
echo "S3_START $(date -Is) rev=$(git rev-parse --short=8 HEAD)" >> $C/run3.marker
for attempt in $(seq 400); do
  if [ -z "$(pgrep -x qemu-system-x86)" ] && flock -n /tmp/kayfabe-fastguest.lock true; then
    echo "S3_ATTEMPT $attempt $(date -Is)" >> $C/run3.marker
    KF3_REV=c7d83a5f PROOF_REBOOTS=1 PROOF_REATTACH=1 bash scripts/bench/display/input_proof.sh c7 > $C/proof_c7.stdout 2>&1
    rc=$?
    if grep -q "already running\|is held by another run" $C/proof_c7.stdout $W/broker-interactive/proof-c7/vm1.out 2>/dev/null; then echo "S3_LOCKRACE retry" >> $C/run3.marker; sleep 5; continue; fi
    echo "S3_EXIT rc=$rc $(date -Is)" >> $C/run3.marker; exit $rc
  fi
  sleep 5
done
echo "S3_GAVE_UP $(date -Is)" >> $C/run3.marker
