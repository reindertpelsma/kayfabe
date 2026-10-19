#!/bin/bash
cd /data/claude-hitch
export BROKER_BIN=/data/claude-hitch/bin/nvkvm-display-broker-stats2 BROKER_ARGS=--stats-line
while [ ! -e ev/queue2.done ]; do sleep 5; done
for spec in "D3 54266072" "C3 388f3289"; do
  set -- $spec
  [ -e /tmp/hitch-stop-queue3 ] && break
  rm -f /tmp/kf-stop-hitch
  bash chainh.sh $1 $2 "" 1 1 > ev/chain_$1.log 2>&1
  echo "epoch=$(date +%s.%N) uptime=$(cut -d" " -f1 /proc/uptime)" > ev/arm/$1.clk
  cp "$(ls -t /var/lib/kf-windows-20261005/boundary-kayfabe-3081/broker-*.log | head -1)" ev/arm/$1.broker.log
done
echo QUEUE3_DONE > ev/queue3.done
