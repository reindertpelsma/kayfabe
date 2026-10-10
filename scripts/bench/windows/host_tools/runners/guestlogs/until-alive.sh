#!/bin/bash
# until-alive.sh START_N [MAX_TRIES] [HOLD_S]: run 118-style boots (gl-launch.sh, binary 66eeebb6, QGA monitor) until one is still alive HOLD_S
# seconds after launch. Early deaths (UnloadingGuestDriver in qemu.log, or QEMU gone) are SIGKILLed and the next run number is tried.
# On success writes /tmp/kf-go with the run number and the time, then lets the run finish.
W=/var/lib/kf-windows-20261005; G=$W/guestlogs
N=$1; MAX=${2:-8}; HOLD=${3:-80}
LOG=/tmp/kf-until-alive.log
rm -f /tmp/kf-go; echo "START $(date -u +%T) first=$N max=$MAX hold=$HOLD" > $LOG
for t in $(seq 1 $MAX); do
  R=$W/boundary-kayfabe-$N
  bash $G/gl-launch.sh $N 66eeebb6 > /dev/null 2>&1 &
  LP=$!
  t0=$(date +%s)
  echo "TRY $t run=$N launched $(date -u +%T)" >> $LOG
  for i in $(seq 1 90); do [ -s $R/qemu.log ] && break; sleep 1; done
  verdict=timeout
  while :; do
    el=$(( $(date +%s) - t0 ))
    if grep -a -q UnloadingGuestDriver $R/qemu.log 2>/dev/null; then verdict=EARLY-DEATH; break; fi
    if [ "$(pgrep -c qemu-system)" = 0 ] && [ $el -gt 20 ]; then verdict=QEMU-GONE; break; fi
    if [ $el -ge $HOLD ]; then verdict=ALIVE; break; fi
    sleep 2
  done
  echo "  run=$N verdict=$verdict at +${el}s $(date -u +%T)" >> $LOG
  if [ $verdict = ALIVE ]; then
    echo "run=$N alive-at=$(date -u +%FT%T) +${el}s" > /tmp/kf-go
    wait $LP
    echo "  run=$N finished $(date -u +%T)" >> $LOG
    exit 0
  fi
  for p in $(pgrep qemu-system); do kill -KILL $p 2>/dev/null; done
  for p in $(pgrep -f "gl-monitor.py $N "); do kill $p 2>/dev/null; done
  wait $LP
  echo "  run=$N cleaned $(date -u +%T) iommu=$(cat /sys/kernel/iommu_groups/11/type)" >> $LOG
  N=$((N+1))
done
echo "GAVE UP after $MAX tries" >> $LOG
