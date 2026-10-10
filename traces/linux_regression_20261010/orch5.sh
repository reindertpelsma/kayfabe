#!/usr/bin/env bash
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005; V=$W/lr-c66d1b96; L=/tmp/kayfabe-fastguest.lock
while pgrep -f '[l]r-c66d1b96/orch4.sh' >/dev/null; do sleep 20; done
echo "ORCH5_START $(date -Is)"
cat > $V/phase3.sh <<'YY'
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005; V=$W/lr-c66d1b96; PB=$W/kayfabe-linuxreg-pb; R=$W/kayfabe-linuxreg
stage() { n=$1; shift
  dmesg | grep -a Xid > $V/logs/xid_before_$n.txt
  echo "LANE_START $n $(date -Is) xid_before=$(wc -l < $V/logs/xid_before_$n.txt) uptime=$(cut -d' ' -f1 /proc/uptime)"
  "$@"; rc=$?
  dmesg | grep -a Xid > $V/logs/xid_after_$n.txt
  echo "LANE_END $n rc=$rc $(date -Is) uptime=$(cut -d' ' -f1 /proc/uptime) new_xid=$(diff $V/logs/xid_before_$n.txt $V/logs/xid_after_$n.txt | grep -c '^>')"; }
cd $R
stage validate_c66 env BENCH_DIR=/workspace/bench KF_GUEST_IMG=/workspace/bench/guest-595.84.qcow2 QEMU_BIN=$W/kf3-bins/c66d1b96/qemu-system-x86_64 KF_DEVICE=kf3 POST_CAPTURE_HOOK=$V/validate_hook.sh GQ_TIMEOUT=1200 bash -c "timeout 1800 bash scripts/bench/boot_capture.sh lr_validate2 > $V/logs/validate_driver2.log 2>&1"
cd $PB
stage gates_pb env CARGO_TARGET_DIR=$W/target-linuxreg-pb bash scripts/bench/v3_gates.sh $V/logs/v3_gates_pb.log
stage fast_pb env KF_LOCK=$V/private.lock KF_DEVICE=kf3 BENCH_DIR=$V/bench KF_FASTGUEST_DIR=$V/fastguest QEMU_BIN=$W/kf3-bins/89721ca4/qemu-system-x86_64 timeout 5000 bash scripts/fastguest/fast_suite.sh lrpb89721ca4 180
YY
cat > $V/phase4.sh <<'YY'
export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005; V=$W/lr-c66d1b96; PB=$W/kayfabe-linuxreg-pb
stage() { n=$1; shift
  dmesg | grep -a Xid > $V/logs/xid_before_$n.txt
  echo "LANE_START $n $(date -Is) xid_before=$(wc -l < $V/logs/xid_before_$n.txt) uptime=$(cut -d' ' -f1 /proc/uptime)"
  "$@"; rc=$?
  dmesg | grep -a Xid > $V/logs/xid_after_$n.txt
  echo "LANE_END $n rc=$rc $(date -Is) uptime=$(cut -d' ' -f1 /proc/uptime) new_xid=$(diff $V/logs/xid_before_$n.txt $V/logs/xid_after_$n.txt | grep -c '^>')"; }
cd $PB
stage desktop_pb env KF_LOCK=$V/private.lock KF3_REV=89721ca4 bash scripts/bench/display/input_proof.sh pb1
grep -aE "^PROOF_" $W/broker-interactive/proof-pb1/proof.log | cut -c1-230
YY
flock -o $L bash $V/phase3.sh
flock -o $L bash $V/phase4.sh
echo ORCH5_DONE $(date -Is)
