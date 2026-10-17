#!/bin/bash
# sample_boot.sh TAG REV : boot run 3081 and sample QEMU main thread state/stack during the first ~12 s (startup stall hunt)
cd /data/claude-hitch; TAG=$1; REV=${2:-388f3289}; O=ev/arm/$TAG.stacks.txt; : > $O
export BROKER_BIN=/data/claude-hitch/bin/nvkvm-display-broker-stats2 BROKER_ARGS=--stats-line
rm -f /tmp/kf-stop-hitch
( REV=$REV setsid nohup bash perf/startk.sh $TAG "" > ev/start_$TAG.out 2>&1 < /dev/null & )
for i in $(seq 200); do QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run3081.state 2>/dev/null); [ -n "$QP" ] && kill -0 $QP 2>/dev/null && grep -q "kf3-gpu" /proc/$QP/cmdline 2>/dev/null && [ $(stat -c %Y /var/lib/kf-windows-20261005/windows-broker-run3081.state) -ge $(( $(date +%s) - 60 )) ] && break; sleep 0.2; done
echo "QPID=$QP t0=$(date +%s.%N)" >> $O
for k in $(seq 40); do
  echo "== t=$(date +%s.%N) main: $(cut -d" " -f3 /proc/$QP/stat) wchan=$(cat /proc/$QP/wchan 2>/dev/null) conn=$(grep -c . /var/lib/kf-windows-20261005/boundary-kayfabe-3081/broker-*.log 2>/dev/null | tail -1)" >> $O
  if [ $((k % 4)) = 0 ]; then timeout 20 gdb -p $QP -batch -ex "thread 1" -ex bt 2>&1 | grep -E "^#" | head -14 >> $O; fi
  sleep 0.25
done
