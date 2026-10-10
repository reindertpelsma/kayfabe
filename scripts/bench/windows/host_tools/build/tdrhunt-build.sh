#!/bin/bash
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-tdrhunt || exit 1
export CARGO_TARGET_DIR=$W/target-tdrhunt
LOG=$W/tdrhunt/build.log
echo "START $(date -u +%T) $(git rev-parse --short=8 HEAD)" > $LOG
bash scripts/bench/build_kf3.sh $W/win-qemu/qemu-10.2.4 $W/win-qemu/qemu-build-kf3 >> $LOG 2>&1
echo "BUILD_KF3_EXIT=$? $(date -u +%T)" >> $LOG
