#!/bin/bash
# wrapper: flock -o around one wr-run.sh boot
W=/var/lib/kf-windows-20261005; N=$1; shift
H=$W/wr-harness-$N.log
echo "LOCK_WAIT $(date -Is)" >> $H
flock -o /tmp/kayfabe-fastguest.lock bash -c "echo LOCK_TAKEN \$(date -Is) >> $H; bash $W/wr-run.sh $N $* >> $H 2>&1; echo LOCK_RELEASED \$(date -Is) >> $H"
echo "WRAPPER_EXIT $(date -Is)" >> $H
