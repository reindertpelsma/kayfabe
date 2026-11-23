#!/bin/bash
# usage: go.sh <name> <attempts>   (boots hold guest, runs the Wukong driver, releases at end)
N=$1; A=$2
cd /workspace/bench/pcnight_20261114
rm -f release_$N
bash j3run.sh $N edge --hold-until-file /workspace/bench/pcnight_20261114/release_$N $EXTRA > $N.out 2>&1 &
for i in $(seq 1 120); do grep -q '^verify' $N.out && break; grep -q '^EXIT' $N.out && exit 1; sleep 5; done
bash restart_drv.sh $N wuk_$N $A
touch release_$N
