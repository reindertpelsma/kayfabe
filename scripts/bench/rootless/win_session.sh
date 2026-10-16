#!/bin/bash
# win_session.sh <uidmode: user|root> <rundir> <overlay> <vars> : hold the shared host lock for the life of one guest.
# Prints SESSION_START / SESSION_EXIT markers. Stop: touch <rundir>/STOP (powers the guest off via QMP) or kill QEMU.
mode=$1; RUN=$2; OVL=$3; VARS=$4
echo "SESSION_START $(date -Is) mode=$mode run=$RUN"
launcher="env INST=/data/claude-rootless/inst RUN=$RUN OVL=$OVL VARS=$VARS RAM_MB=${RAM_MB:-16384} bash /data/claude-rootless/wt/scripts/bench/rootless/win_launch.sh start"
if [ "$mode" = user ]; then /data/claude-rootless/bin/as_user.sh $launcher; else $launcher; fi
rc=$?
[ $rc -eq 0 ] || { echo "SESSION_EXIT launch-failed rc=$rc $(date -Is)"; exit $rc; }
Q=$(cat $RUN/qemu.pid)
while kill -0 $Q 2>/dev/null; do
  [ -e $RUN/STOP ] && { rm -f $RUN/STOP; python3 -c "
import socket,json,sys
s=socket.socket(socket.AF_UNIX); s.connect('$RUN/qmp.sock'); s.recv(4096)
s.send(b'{\"execute\":\"qmp_capabilities\"}'); s.recv(4096); s.send(b'{\"execute\":\"system_powerdown\"}'); print(s.recv(4096))"; }
  sleep 3
done
[ -s $RUN/broker.pid ] && kill -- -$(cat $RUN/broker.pid) 2>/dev/null
echo "SESSION_EXIT qemu-gone $(date -Is)"
