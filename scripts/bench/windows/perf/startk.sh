#!/bin/bash
# startk.sh TAG [FLAGS]  : start kayfabe on overlay K with boot screendumps; wait for the guest
TAG=$1; FL=${2:-}
cd /data/claude-perf
P=/data/claude-perf/wt/scripts/bench/windows/perf
rm -f /var/lib/kf-windows-20261005/boundary-kayfabe-2991/qmp.sock
mkdir -p ev/boot-$TAG
(setsid nohup python3 $P/bootshots.py /var/lib/kf-windows-20261005/boundary-kayfabe-2991/qmp.sock /data/claude-perf/ev/boot-$TAG 30 8 > ev/boot-$TAG.log 2>&1 < /dev/null &)
OVL=/data/claude-perf/disk/K.qcow2 VARS=/data/claude-perf/disk/K.vars.fd PERF_FLAGS="$FL" setsid nohup flock -o /tmp/kayfabe-fastguest.lock bash $P/launch_k.sh > ev/launch-$TAG.out 2>&1 < /dev/null &
sleep 15
for i in $(seq 60); do grep -q "PERF_SESSION_READY\|exited\|EXIT=" ev/launch-$TAG.out && break; sleep 2; done
tail -2 ev/launch-$TAG.out | cut -c1-200
bash /data/claude-mctdr/ev/waitguest.sh
