#!/bin/bash
# owner-session.sh — queue an interactive Windows run for the owner (no scripted scrolling), then hot-plug the app disk once signed in.
W=/var/lib/kf-windows-20261005; N=500; REV=89721ca4; HOLD=3600
LOG=$W/tdropus/owner-session.log
echo "queued $(date -u +%FT%T) rev=$REV run=$N hold=$HOLD" > $LOG
setsid nohup bash $W/tdropus/queue.sh $N $REV $HOLD tdr-run16.sh KF_GUEST_PW=${KF_GUEST_PW:?set KF_GUEST_PW} SCROLL_EVERY=1000000 RUN_WIN_FLAGS= >> $LOG 2>&1 < /dev/null &
(
  while :; do grep -q "READY" $W/winprod/run$N/winprod.log 2>/dev/null && break; sleep 5; done
  sleep 3
  python3 - >> $LOG 2>&1 <<PY
import sys
sys.path.insert(0, "$W/tdropus")
import qga_attach_copy as q
m = q.Qmp("$W/boundary-kayfabe-$N/qmp.sock")
print("attach:", m.attach_cdrom("$W/appmatrix/image/kfapps.iso"))
PY
  echo "app disk attach attempted $(date -u +%FT%T)" >> $LOG
) > /dev/null 2>&1 < /dev/null &
echo started
