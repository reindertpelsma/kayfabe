#!/bin/bash
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005
cd $W/kayfabe-linuxreg || exit 1
export CARGO_TARGET_DIR=$W/target-linuxreg
L=$W/build-linuxreg.log
R=$(git rev-parse --short=8 HEAD)
echo "START $(date -u +%T) $R" > $L
bash scripts/bench/build_kf3.sh $W/win-qemu/qemu-10.2.4 $W/win-qemu/qemu-build-kf3 >> $L 2>&1
echo "BUILD_KF3_EXIT=$? $(date -u +%T)" >> $L
KAYFABE_BUILD_REV=$R cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >> $L 2>&1
echo "BUILD_CLIENT_EXIT=$? $(date -u +%T)" >> $L
cargo build --release -p kf-harness --bins >> $L 2>&1
echo "BUILD_HARNESS_EXIT=$? $(date -u +%T)" >> $L
echo DONE >> $L
