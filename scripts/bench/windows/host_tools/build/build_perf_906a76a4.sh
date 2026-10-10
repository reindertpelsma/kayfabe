#!/bin/bash
export PATH=$HOME/.cargo/bin:$PATH
cd /var/lib/kf-windows-20261005/kayfabe-perf-906a76a4
export CARGO_TARGET_DIR=/var/lib/kf-windows-20261005/target-perf-906a76a4
LOG=/var/lib/kf-windows-20261005/perf-baseline/build-906a76a4.log
echo "START $(date -u +%T)" > $LOG
bash scripts/bench/build_kf3.sh /var/lib/kf-windows-20261005/perf-qemu/qemu-10.2.4 /var/lib/kf-windows-20261005/perf-qemu/qemu-build-kf3 >> $LOG 2>&1
echo "BUILD_KF3_EXIT=$? $(date -u +%T)" >> $LOG
