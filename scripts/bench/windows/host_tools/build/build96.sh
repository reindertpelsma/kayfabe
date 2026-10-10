#!/bin/bash
W=/var/lib/kf-windows-20261005
export PATH=$HOME/.cargo/bin:$PATH
cd $W/kayfabe-96d93c32 || exit 1
export CARGO_TARGET_DIR=$W/target-fb8150ed
echo "START $(date -u +%T) $(git rev-parse --short=8 HEAD)" > $W/build-96d93c32.log
bash scripts/bench/build_kf3.sh $W/irqflood-qemu/qemu-10.2.4 $W/irqflood-qemu/qemu-build-kf3 >> $W/build-96d93c32.log 2>&1
echo "BUILD_EXIT=$?" >> $W/build-96d93c32.log
