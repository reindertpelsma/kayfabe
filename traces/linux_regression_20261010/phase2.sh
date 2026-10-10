export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005; V=$W/lr-c66d1b96; R=$W/kayfabe-linuxreg; REV=c66d1b96; DREV=$(cat $V/diagrev.txt)
DQB=$W/kf3-bins/$DREV/qemu-system-x86_64; cd $R
stage() { n=$1; shift
  dmesg | grep -a Xid > $V/logs/xid_before_$n.txt
  echo "LANE_START $n $(date -Is) xid_before=$(wc -l < $V/logs/xid_before_$n.txt) uptime=$(cut -d' ' -f1 /proc/uptime)"
  "$@"; rc=$?
  dmesg | grep -a Xid > $V/logs/xid_after_$n.txt
  echo "LANE_END $n rc=$rc $(date -Is) uptime=$(cut -d' ' -f1 /proc/uptime) new_xid=$(diff $V/logs/xid_before_$n.txt $V/logs/xid_after_$n.txt | grep -c '^>')"; }
stage desktop env KF_LOCK=$V/private.lock KF3_REV=$REV bash scripts/bench/display/input_proof.sh lr1
for arm in --uvm-invalidate --map-stress --ce-client-guest-ram; do
  stage diag${arm#--} env KF3_COMPLETION_PROBE=1000 KF_LOCK=$V/private.lock KF_DEVICE=kf3 BENCH_DIR=$V/bench KF_FASTGUEST_DIR=$V/fastguest QEMU_BIN=$DQB KF_ARMS=$arm bash scripts/fastguest/run_fast_guest.sh lrdiag_${arm#--} 180
done
stage diag_cup8 env KF3_COMPLETION_PROBE=1000 BENCH_DIR=/workspace/bench KF_GUEST_IMG=/workspace/bench/guest-595.84.qcow2 QEMU_BIN=$DQB bash -c "timeout 1200 bash scripts/bench/cuda_ladder.sh guest lrdiag_g 1 cup8 > $V/logs/ladder_diag.out 2>&1"
