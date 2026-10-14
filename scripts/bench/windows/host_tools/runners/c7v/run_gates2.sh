#!/bin/bash
C=/var/lib/kf-windows-20261005/c7v
cd $C/src
echo "G2_START $(date -Is) rev=$(git rev-parse --short=8 HEAD)" >> $C/gates2.marker
CARGO_TARGET_DIR=$C/target-gates2 flock -o /tmp/kayfabe-fastguest.lock bash scripts/bench/v3_gates.sh $C/v3_gates2.log > $C/v3_gates2.stdout 2>&1
echo "G2_EXIT rc=$? $(date -Is)" >> $C/gates2.marker
