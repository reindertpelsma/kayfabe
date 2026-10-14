#!/bin/bash
W=/var/lib/kf-windows-20261005; G=$W/guestlogs
for i in $(seq 1 200); do grep -q "GLRUN.*END type" $G/run239/gl-run.log 2>/dev/null && break; sleep 3; done
R=$W/boundary-kayfabe-239; mv $R/qemu.log $R/qemu-phase1.log 2>/dev/null
cd $G
export EXTRA_FLAGS="KF3_GSS_NATIVE KF3_WIN_KERNEL_PID4 KF3_SCHEDULE_LATE_JOINERS" OBS=0 WIN_REUSE=1 ETWSTOP=1 ETW_WAIT=60 EDGE_CLICK=1 SHORTS=1 SHORTS_N=18 KF3_DISPLAY_TRACE_CAP=400000
unset DUMPCFG ETWARM EDGE DROP_MORE STALLDUMP
bash gl-launch-signinsnap13.sh 239 8cd433a8 > /dev/null 2>&1
