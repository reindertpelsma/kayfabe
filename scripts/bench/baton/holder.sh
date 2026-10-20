#!/bin/bash
# holder.sh - hold /tmp/kayfabe-fastguest.lock (taken by the caller: flock -o LOCK holder.sh QDIR) and
# run queue scripts QDIR/NN_*.sh in name order, waiting for new ones, until QDIR/STOP exists.
# Writes QDIR/holder.log with start and exit markers.  Releasing = exiting this script.
QDIR=$1; LOG=$QDIR/holder.log
echo "HOLDER start $(date -Is) pid=$$" >> $LOG
while :; do
  next=$(ls $QDIR/[0-9]*_*.sh 2>/dev/null | while read f; do [ -e "$f.done" ] || { echo $f; break; }; done)
  if [ -n "$next" ]; then
    echo "HOLDER run $next $(date -Is)" >> $LOG
    bash $next >> $next.out 2>&1; echo "rc=$?" > $next.done
    echo "HOLDER done $next $(date -Is) $(cat $next.done)" >> $LOG
  elif [ -e $QDIR/STOP ]; then break
  else sleep 3; fi
done
echo "HOLDER exit $(date -Is)" >> $LOG
