#!/bin/bash
# split_run.sh LIST REV : runs under the fastguest lock: build the frame-pointer binary of the host tree (start/exit markers), install it, then the chain.
cd /data/claude-gap
echo "BUILD_START $(date -Is) (no VM running; host tree rev $(git -C src rev-parse --short=8 HEAD))"
bash buildfp.sh 2>&1 | tail -3
mkdir -p /var/lib/kf-windows-20261005/kf3-bins/$2 && cp -a fp/kf3-bins/$2/. /var/lib/kf-windows-20261005/kf3-bins/$2/
echo "BUILD_EXIT $(date -Is)"
CHAIN_LOCKED=1 bash perf/chain7.sh $1
