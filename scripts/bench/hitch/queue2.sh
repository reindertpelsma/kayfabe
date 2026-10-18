#!/bin/bash
cd /data/claude-hitch
export BROKER_BIN=/data/claude-hitch/bin/nvkvm-display-broker-stats BROKER_ARGS=--stats-line
for spec in "D1 54266072" "C1 388f3289" "D2 54266072" "C2 388f3289"; do
  set -- $spec
  [ -e /tmp/hitch-stop-queue ] && break
  rm -f /tmp/kf-stop-hitch
  bash chainh.sh $1 $2 "" 1 1 > ev/chain_$1.log 2>&1
  echo "epoch=$(date +%s.%N) uptime=$(cut -d" " -f1 /proc/uptime)" > ev/arm/$1.clk
  cp "$(ls -t /var/lib/kf-windows-20261005/boundary-kayfabe-3081/broker-*.log | head -1)" ev/arm/$1.broker.log
done
echo QUEUE2_DONE > ev/queue2.done
