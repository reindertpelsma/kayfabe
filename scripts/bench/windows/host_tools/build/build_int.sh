#!/bin/bash
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-96d93c32 || exit 1
git fetch -q origin claude/tmode-pieces-20261009 && git checkout -q FETCH_HEAD
export CARGO_TARGET_DIR=$W/target-fb8150ed
echo "START $(date -u +%T) $(git rev-parse --short=8 HEAD)" > $W/build-int.log
bash scripts/bench/build_kf3.sh $W/irqflood-qemu/qemu-10.2.4 $W/irqflood-qemu/qemu-build-kf3 >> $W/build-int.log 2>&1
rc=$?
R=$(git rev-parse --short=8 HEAD)
mkdir -p $W/kf3-bins/$R && cp -a $W/irqflood-qemu/kf3-bins/$R/qemu-system-x86_64 $W/irqflood-qemu/kf3-bins/$R/pc-bios $W/irqflood-qemu/kf3-bins/$R/qemu-bundle $W/kf3-bins/$R/ 2>/dev/null
echo "BUILD_EXIT=$rc $(ls -la $W/kf3-bins/$R/qemu-system-x86_64 | cut -c20-90)" >> $W/build-int.log
