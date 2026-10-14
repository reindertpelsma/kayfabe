#!/bin/bash
# gl-launch.sh N [REV] — run N of the guest-log collection under the fastguest flock, then the usual evidence collector.
N=$1; REV=${2:-fb8150ed}
W=/var/lib/kf-windows-20261005
mkdir -p $W/guestlogs/run$N
O=$W/guestlogs/run$N/outer.log
echo "START$N $(date -u +%FT%T) rev=$REV flood=[] xid=$(dmesg | grep -c 'NVRM: Xid')" > $O
flock -o /tmp/kayfabe-fastguest.lock bash $W/guestlogs/gl-run-signinsnap14.sh $N $REV >> $O 2>&1
echo "EXIT$N rc=$? $(date -u +%FT%T) xid=$(dmesg | grep -c 'NVRM: Xid')" >> $O
bash $W/irqflood-collect.sh $N >> $O 2>&1
echo "COLLECTED$N $(date -u +%FT%T)" >> $O
