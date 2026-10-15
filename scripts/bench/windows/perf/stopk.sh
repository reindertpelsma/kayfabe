#!/bin/bash
# stopk.sh TAG : graceful guest shutdown, wait, save qemu.log copy
TAG=$1; W=/var/lib/kf-windows-20261005
Q=$(sed -n "s/^QPID=//p" $W/windows-broker-run2991.state)
timeout 40 python3 /data/kfagent_server.py run 'shutdown /s /t 0 /f; "issued"' 30
for i in $(seq 100); do kill -0 $Q 2>/dev/null || break; sleep 3; done
kill -0 $Q 2>/dev/null && echo "STILL_ALIVE $Q (not killing)"
sleep 6
L=$W/boundary-kayfabe-2991/qemu.log
mkdir -p /data/claude-perf/ev/qemulog; gzip -c $L > /data/claude-perf/ev/qemulog/$TAG.qemu.log.gz
grep -a "USER_RING_LOOP SUMMARY" $L | cut -c1-2500 > /data/claude-perf/ev/qemulog/$TAG.user_ring_loop_summary.txt
tail -2 /data/claude-perf/ev/launch-$TAG.out | cut -c1-160
