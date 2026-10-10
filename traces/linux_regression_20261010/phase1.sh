export PATH=$HOME/.cargo/bin:$PATH
W=/var/lib/kf-windows-20261005; V=$W/lr-c66d1b96; R=$W/kayfabe-linuxreg; REV=c66d1b96
QB=$W/kf3-bins/$REV/qemu-system-x86_64; cd $R
stage() { n=$1; shift
  dmesg | grep -a Xid > $V/logs/xid_before_$n.txt
  echo "LANE_START $n $(date -Is) xid_before=$(wc -l < $V/logs/xid_before_$n.txt) uptime=$(cut -d' ' -f1 /proc/uptime)"
  "$@"; rc=$?
  dmesg | grep -a Xid > $V/logs/xid_after_$n.txt
  echo "LANE_END $n rc=$rc $(date -Is) uptime=$(cut -d' ' -f1 /proc/uptime) new_xid=$(diff $V/logs/xid_before_$n.txt $V/logs/xid_after_$n.txt | grep -c '^>')"; }
stage ladder_guest env BENCH_DIR=/workspace/bench KF_GUEST_IMG=/workspace/bench/guest-595.84.qcow2 QEMU_BIN=$QB bash -c "timeout 2400 bash scripts/bench/cuda_ladder.sh guest lr_g 3 > $V/logs/ladder_guest.out 2>&1"
stage validate env BENCH_DIR=/workspace/bench KF_GUEST_IMG=/workspace/bench/guest-595.84.qcow2 QEMU_BIN=$QB KF_DEVICE=kf3 POST_CAPTURE_HOOK=$V/validate_hook.sh GQ_TIMEOUT=1200 bash -c "timeout 1800 bash scripts/bench/boot_capture.sh lr_validate > $V/logs/validate_driver.log 2>&1"
