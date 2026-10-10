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
