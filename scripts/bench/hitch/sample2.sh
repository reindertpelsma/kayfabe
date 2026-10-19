#!/bin/bash
cd /data/claude-hitch; TAG=$1; REV=${2:-388f3289}; O=ev/arm/$TAG.allthreads.txt; : > $O
export BROKER_BIN=/data/claude-hitch/bin/nvkvm-display-broker-stats2 BROKER_ARGS=--stats-line
rm -f /tmp/kf-stop-hitch /var/lib/kf-windows-20261005/windows-broker-run3081.state.old
OLD=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run3081.state 2>/dev/null)
( REV=$REV setsid nohup bash perf/startk.sh $TAG "" > ev/start_$TAG.out 2>&1 < /dev/null & )
for i in $(seq 300); do QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run3081.state 2>/dev/null); [ -n "$QP" ] && [ "$QP" != "$OLD" ] && kill -0 $QP 2>/dev/null && break; sleep 0.1; done
echo "QPID=$QP t0=$(date +%s.%N)" >> $O
for n in 1 2 3; do
  echo "=== attach $n at $(date +%s.%N)" >> $O
  timeout 40 gdb -p $QP -batch -ex "thread apply all bt 14" 2>&1 | grep -E "^Thread|^#" >> $O
  echo "=== detached $n at $(date +%s.%N)" >> $O
  sleep 0.3
done
