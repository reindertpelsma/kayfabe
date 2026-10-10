#!/bin/bash
W=/var/lib/kf-windows-20261005
C=$W/c7v
cd $W/kayfabe-96d93c32
echo "STEP1_START $(date -Is) rev=$(git rev-parse --short=8 HEAD)" >> $C/run12.marker
CARGO_TARGET_DIR=$C/target-gates flock -o /tmp/kayfabe-fastguest.lock bash scripts/bench/v3_gates.sh $C/v3_gates.log > $C/v3_gates.stdout 2>&1
echo "STEP1_EXIT rc=$? $(date -Is)" >> $C/run12.marker
echo "STEP2_START $(date -Is)" >> $C/run12.marker
KF_DEVICE=kf3 BENCH_DIR=$C/bench KF_FASTGUEST_DIR=$C/fastguest QEMU_BIN=$W/kf3-bins/c7d83a5f/qemu-system-x86_64 bash scripts/fastguest/fast_suite.sh c7d83a5f_fs 180 > $C/fast_suite.stdout 2>&1
echo "STEP2_EXIT rc=$? $(date -Is)" >> $C/run12.marker
