#!/bin/bash
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-perf-6692e621 || exit 1
export CARGO_TARGET_DIR=$W/target-perf-6692e621
LOG=$W/perf-baseline/build-6692e621.log
echo "START $(date -u +%T) $(git rev-parse --short=8 HEAD)" > $LOG
bash scripts/bench/build_kf3.sh $W/perf-qemu/qemu-10.2.4 $W/perf-qemu/qemu-build-kf3 >> $LOG 2>&1
rc=$?
echo "BUILD_KF3_EXIT=$rc $(date -u +%T)" >> $LOG
if [ $rc = 0 ]; then
  KAYFABE_BUILD_REV=$(git rev-parse --short=8 HEAD) cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >> $LOG 2>&1
  echo "BUILD_CLIENT_EXIT=$? $(date -u +%T)" >> $LOG
fi
echo "DONE" >> $LOG
