#!/bin/bash
# vmstart.sh REV PROF(0|1) : start VM 2990 at REV (broker from the same commit), production profile, KF3_PROF only when PROF=1. Detached; launcher stays alive with the VM.
REV=$1; PROF=${2:-0}
case $REV in
 a07a9ba0) BB=/data/claude-cursor2/broker-bin/nvkvm-display-broker-a07a9ba0;;
 64449dc5) BB=/data/claude-cursor2/broker-bin/nvkvm-display-broker-64449dc5;;
 666ea966) BB=/data/claude-resize/broker-bin/nvkvm-display-broker-666ea966;;
 eb9f2aa3|2f99e673) BB=/data/claude-cursor2/broker-bin/nvkvm-display-broker-badf2d7-TRACE;;
 *) echo "no broker for $REV"; exit 2;;
esac
test -x $BB && test -x /var/lib/kf-windows-20261005/kf3-bins/$REV/qemu-system-x86_64 || { echo missing binaries; exit 2; }
if pgrep -f "boundary-kayfabe-2990" >/dev/null; then echo "a qemu for run 2990 is still running"; exit 3; fi
for p in $(pgrep -f "nvkvm-display-broker.*windows-run2990.sock"); do kill $p 2>/dev/null; done; sleep 2
sed -e 's/WIN_FLAGS="${OGL_FLAGS:-KF3_PROF=1}"/WIN_FLAGS="${MCTDR_FLAGS}"/' /data/claude-cursor2/live3/launch3.sh > /data/claude-mctdr/ev/launch_mctdr.sh
grep -c 'MCTDR_FLAGS' /data/claude-mctdr/ev/launch_mctdr.sh
FL=""; [ "$PROF" = 1 ] && FL="KF3_PROF=1"
cd /data/claude-mctdr/ev
REV=$REV BROKER_BIN=$BB MCTDR_FLAGS="$FL" OGL_FLAGS= setsid nohup flock -o /tmp/kayfabe-fastguest.lock bash /data/claude-mctdr/ev/launch_mctdr.sh > /data/claude-mctdr/ev/launch-$REV-prof$PROF.out 2>&1 < /dev/null &
sleep 2; echo started $REV prof=$PROF at $(date -Is)
