#!/bin/bash
# hostmon.sh start|stop ARM REP  (on PC)  — dmon + marks
O=/workspace/bench/pcnight_20261113/arm_$2; R=$3
case $1 in
start) echo "$(wc -l < $O/qemu.out) $(dmesg | grep -c 'NVRM: Xid') $(date +%s)" > $O/mark_$R.start
  nohup nvidia-smi dmon -s pucm -d 1 -o T > $O/dmon_$R.txt 2>&1 < /dev/null & echo $! > $O/dmon_$R.pid;;
stop) kill $(cat $O/dmon_$R.pid) 2>/dev/null; read L X T < $O/mark_$R.start
  tail -n +$((L+1)) $O/qemu.out > $O/kf3_$R.log
  echo "xid_new=$(( $(dmesg | grep -c 'NVRM: Xid') - X )) dur_s=$(( $(date +%s) - T ))" > $O/mark_$R.end; cat $O/mark_$R.end;;
esac
