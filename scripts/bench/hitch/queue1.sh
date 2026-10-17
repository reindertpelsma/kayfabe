#!/bin/bash
cd /data/claude-hitch
for spec in "A2 54266072" "B2 6c2c7b96" "A3 54266072" "B3 6c2c7b96"; do
  set -- $spec
  rm -f /tmp/kf-stop-hitch
  bash chainh.sh $1 $2 "" 1 1 > ev/chain_$1.log 2>&1
done
echo QUEUE1_DONE > ev/queue1.done
