#!/bin/bash
# Ada (RTX 4070, AD104) regression of `next` on the trusted host. Copy of the host's validate.sh
# pattern (/data/claude-s3-validate.sh): unit -> build_kf3 -> gates -> raw client -> fast guest ->
# fast suite 30 arms with this revision's kf3 binary, private lock and bench dir.
# Start marker and EXIT line; everything under $O.
set -uo pipefail
D=/data/claude-ada-20261103
R=$D/kayfabe
W=/var/lib/kf-windows-20261005
export PATH=$HOME/.cargo/bin:$PATH CARGO_BUILD_JOBS=${JOBS:-12} CARGO_TARGET_DIR=$D/target
rev=$(git -C $R rev-parse --short=8 HEAD)
O=$D/validate-$rev; mkdir -p $O
S=$O/${SUMMARY:-summary.log}
echo "VALIDATE_START rev=$rev $(date -Is) host_driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader) gpu=\"$(nvidia-smi --query-gpu=name --format=csv,noheader)\" xid=$(dmesg | grep -c 'NVRM: Xid')" | tee -a $S
cd $R
step=${STEPS:-"unit build gates fast"}
if [[ " $step " == *" unit "* ]]; then
  t0=$(date +%s)
  crates=$(ls -d crates/kf-* | xargs -n1 basename | sed 's/^/-p /' | tr '\n' ' ')
  nice -n 10 cargo test --no-fail-fast $crates > $O/unit.log 2>&1
  rc=$?
  echo "UNIT rc=$rc passed=$(grep -o 'test result: [a-z]*\. [0-9]* passed' $O/unit.log | awk '{s+=$4} END{print s}') failed=$(grep -o '[0-9]* failed' $O/unit.log | awk '{s+=$1} END{print s}') ignored=$(grep -o '[0-9]* ignored' $O/unit.log | awk '{s+=$1} END{print s}') failing_tests=$(grep -c '^test .* FAILED' $O/unit.log) secs=$(( $(date +%s) - t0 ))" | tee -a $S
  grep '^test .* FAILED\|panicked at\|^error' $O/unit.log | head -30 >> $S
fi
if [[ " $step " == *" build "* ]]; then
  t0=$(date +%s)
  nice -n 10 bash scripts/bench/build_kf3.sh $W/qemu-10.2.4 $W/qemu-build-kf3 > $O/build_kf3.log 2>&1
  echo "BUILD_KF3 rc=$? $(tail -2 $O/build_kf3.log | tr '\n' ' ' | cut -c1-300) secs=$(( $(date +%s) - t0 ))" | tee -a $S
  ls -la $W/kf3-bins/$rev/ >> $S 2>&1
fi
if [[ " $step " == *" gates "* ]]; then
  t0=$(date +%s)
  bash scripts/bench/v3_gates.sh $O/gates.log > $O/gates.console 2>&1
  echo "GATES rc=$? $(grep 'V3_GATES_SUMMARY\|V3_GATES_BIRTHS\|V3_GATES_EXIT' $O/gates.log | tr '\n' ' ') secs=$(( $(date +%s) - t0 ))" | tee -a $S
fi
if [[ " $step " == *" fast "* ]]; then
  nice -n 10 cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > $O/client-build.log 2>&1
  echo "CLIENT rc=$?" | tee -a $S
  CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder KF_FROM_HOST=1 PATH=/data/claude-planes-bin:$PATH bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 $O/fastguest > $O/fg-build.log 2>&1
  echo "FG rc=$? $(grep 'GUEST DRIVER' $O/fg-build.log) $(tail -1 $O/fg-build.log | cut -c1-200)" | tee -a $S
  mkdir -p $O/bench
  t0=$(date +%s)
  QEMU_BIN=$W/kf3-bins/$rev/qemu-system-x86_64 KF_FASTGUEST_DIR=$O/fastguest KF_DEVICE=kf3 \
    KF_LOCK=/tmp/claude-ada-fast.lock BENCH_DIR=$O/bench KF3_FB_MB=${KF3_FB_MB:-8192} \
    flock /tmp/claude-linux-guest.lock bash scripts/fastguest/fast_suite.sh ada ${BUDGET:-180} > $O/fast.log 2>&1
  echo "FAST rc=$? $(grep -E '^FAST_SUITE_(PASS|RC)' $O/bench/ada_suite.out 2>/dev/null | tr '\n' ' ') secs=$(( $(date +%s) - t0 ))" | tee -a $S
  grep -E ' (FAIL|CRASH|TIMEOUT|VMM_DIED|NOTRUN) ' $O/bench/ada_suite.out | head -10 >> $S
fi
echo "VALIDATE_EXIT rev=$rev $(date -Is) xid=$(dmesg | grep -c 'NVRM: Xid')" | tee -a $S
