#!/bin/bash
# winprod-launch.sh N — queue behind /tmp/kayfabe-fastguest.lock (blocks), then run the production Windows guest.
N=$1; W=/var/lib/kf-windows-20261005; O=$W/winprod/run$N; mkdir -p $O
echo "QUEUED $(date -u +%FT%T)" > $O/outer.log
flock -o /tmp/kayfabe-fastguest.lock bash $W/winprod/winprod-run.sh $N >> $O/outer.log 2>&1
echo "EXIT rc=$? $(date -u +%FT%T)" >> $O/outer.log
