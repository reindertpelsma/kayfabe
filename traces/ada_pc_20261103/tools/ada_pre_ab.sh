#!/bin/bash
# A/B for the walk-completion change: build the pre-merge first parent 9297ffdd9's kf3 and run the SAME
# fast suite (next's harness, next's fast guest and raw client) with only QEMU_BIN swapped.
set -uo pipefail
D=/data/claude-ada-20261103; W=/var/lib/kf-windows-20261005
P=$D/kayfabe-pre; R=$D/kayfabe
export PATH=$HOME/.cargo/bin:$PATH CARGO_BUILD_JOBS=12 CARGO_TARGET_DIR=$D/target-pre
O=$D/ab-pre; mkdir -p $O; S=$O/summary.log
rev=$(git -C $P rev-parse --short=8 HEAD)
echo "AB_START pre=$rev $(date -Is) xid=$(dmesg | grep -c 'NVRM: Xid')" | tee -a $S
t0=$(date +%s)
nice -n 10 bash $P/scripts/bench/build_kf3.sh $W/qemu-10.2.4 $W/qemu-build-kf3 > $O/build_kf3.log 2>&1
echo "BUILD_KF3 rc=$? $(tail -1 $O/build_kf3.log | cut -c1-200) secs=$(( $(date +%s) - t0 ))" | tee -a $S
mkdir -p $O/bench; cd $R
t0=$(date +%s)
QEMU_BIN=$W/kf3-bins/$rev/qemu-system-x86_64 KF_FASTGUEST_DIR=$D/validate-70e3fabd/fastguest KF_DEVICE=kf3 \
  KF_LOCK=/tmp/claude-ada-fast.lock BENCH_DIR=$O/bench KF3_FB_MB=8192 \
  flock /tmp/claude-linux-guest.lock bash scripts/fastguest/fast_suite.sh pre 180 > $O/fast.log 2>&1
echo "FAST rc=$? $(grep -E '^FAST_SUITE_(PASS|RC)' $O/bench/pre_suite.out | tr '\n' ' ') secs=$(( $(date +%s) - t0 ))" | tee -a $S
echo "AB_EXIT $(date -Is) xid=$(dmesg | grep -c 'NVRM: Xid')" | tee -a $S
