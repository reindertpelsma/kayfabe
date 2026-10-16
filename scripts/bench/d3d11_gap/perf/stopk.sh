#!/bin/bash
# stopk.sh TAG : graceful guest shutdown, wait, save qemu.log copy
TAG=$1; W=/var/lib/kf-windows-20261005
Q=$(sed -n "s/^QPID=//p" $W/windows-broker-run2995.state)
timeout 40 python3 /data/kfagent_server.py run 'shutdown /s /t 0 /f; "issued"' 30
for i in $(seq 50); do kill -0 $Q 2>/dev/null || break; sleep 3; done
if kill -0 $Q 2>/dev/null; then echo "STILL_ALIVE $Q (guest agent did not shut it down): broker stop of run 2995 (own pid)"; bash /var/lib/kf-windows-20261005/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude_oglw.sh stop 2995 2>&1 | tail -2; fi
sleep 6
L=$W/boundary-kayfabe-2995/qemu.log
mkdir -p /data/claude-gap/ev/qemulog; gzip -c $L > /data/claude-gap/ev/qemulog/$TAG.qemu.log.gz
grep -a "USER_RING_LOOP SUMMARY" $L | cut -c1-2500 > /data/claude-gap/ev/qemulog/$TAG.user_ring_loop_summary.txt
tail -2 /data/claude-gap/ev/launch-$TAG.out | cut -c1-160
sleep 2; pkill -f "[n]vkvm-display-broker-666ea966 --socket /run/user/1000/nvkvm/windows-run2995"; pkill -f "[r]unuser -u ubuntu .*windows-run2995"; true
rm -rf /data/claude-gap/ev/boot-$TAG
