#!/bin/bash
# queue.sh N BIN HOLD RUNNER [extra env...] — wait until the GPU is idle (no QEMU, no other tdr-run), then run under the flock.
W=/var/lib/kf-windows-20261005; N=$1; BIN=$2; HOLD=$3; RUNNER=$4; shift 4
while :; do
  q=$(pgrep -c qemu-system-x86); r=$(pgrep -fc "tdr-run[0-9]*\.sh [0-9]")
  [ "$q" = 0 ] && [ "$r" = 0 ] && break
  sleep 15
done
cd $W && exec env "$@" KF3_REV_BIN=$BIN HOLD_SECS=$HOLD flock -w 28800 -o /tmp/kayfabe-fastguest.lock bash tdrhunt/$RUNNER $N
