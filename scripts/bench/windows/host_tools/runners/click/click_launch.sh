#!/bin/bash
# click_launch.sh MODE N REV [GESTURE] — click_run.sh under the fastguest flock (waits for any other holder, e.g. a coordinator run).
MODE=$1; N=$2; REV=${3:-66eeebb6}; G=${4:-drag}
W=/var/lib/kf-windows-20261005; C=$W/click
mkdir -p $C/evidence/run$N-$MODE
O=$C/evidence/run$N-$MODE/outer.log
echo "WAITLOCK$N $(date -u +%FT%T) mode=$MODE rev=$REV gesture=$G" > $O
flock -o /tmp/kayfabe-fastguest.lock bash $C/click_run.sh $MODE $N $REV $G >> $O 2>&1
echo "EXIT$N rc=$? $(date -u +%FT%T) xid=$(dmesg | grep -c 'NVRM: Xid')" >> $O
