#!/bin/bash
# heaven_run.sh LABEL API(direct3d11|opengl) CFG(K|V) [SMI=0|1] [DEFS]  : one Heaven in-app benchmark, K host dmon + qemu.log slice, V guest smi
LABEL=$1; API=$2; CFG=$3; SMI=${4:-0}; DEFS=${5:-RELEASE,QUALITY_ULTRA}
P=/data/claude-perf/wt/scripts/bench/windows/perf; EV=/data/claude-perf/ev/heaven/$LABEL; mkdir -p $EV/guest; export EV
if [ $CFG = K ]; then RUN=/var/lib/kf-windows-20261005/boundary-kayfabe-2991; QMPSOCK=$RUN/qmp.sock; LOG=$RUN/qemu.log; OFF=$(stat -c %s $LOG); QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2991.state); python3 $P/threadcpu.py snap $QP $EV/thr0.json
 (timeout 560 nvidia-smi dmon -s pucm -d 1 -o T > $EV/host_dmon.txt 2>&1 &); DM=$!; else QMPSOCK=/data/claude-perf/vfio/latest/qmp.sock; fi
export QMPSOCK
$P/mkjob.sh $P/jobs/heaven.ps1.in $EV/job.ps1 API=$API DEFS=$DEFS SMI=$SMI
TAILN=60 bash $P/kfp.sh hv_$LABEL interactive $EV/job.ps1 700 > $EV/kfp.out 2>&1 &
KP=$!
sleep 38; $P/inputs.sh click >/dev/null; sleep 1; $P/inputs.sh key f9 > /dev/null
if [ -n "${HARVEST:-}" ]; then ( sleep ${HDELAY:-60}; OFFCPU=${OFFCPU:-0} HOUT=$EV bash $P/harvest.sh $QP $HARVEST $LABEL > $EV/harvest.out 2>&1 ) & HP=$!; fi
if [ -n "${HOOK:-}" ]; then ( sleep ${HDELAY:-60}; eval "$HOOK" > $EV/hook.out 2>&1 ) & HK=$!; fi
wait $KP
[ -n "${HK:-}" ] && wait $HK
[ -n "${HP:-}" ] && wait $HP
pkill -f "nvidia-smi dmon" 2>/dev/null
[ $CFG = K ] && { python3 $P/threadcpu.py snap $QP $EV/thr1.json; python3 $P/threadcpu.py diff $EV/thr0.json $EV/thr1.json 16 > $EV/threads_cpu.txt; }
if [ $CFG = K ]; then tail -c +$((OFF+1)) $LOG | grep -a -E "display fps|PROF|USER_RING_LOOP|REFUSED" | cut -c1-1500 > $EV/qemu_slice.txt; fi
grep -a -E "^(START|PRE|LAUNCH|RESULT_FOUND|HLOG|END)" $EV/guest/hv_$LABEL.log | sed 's/\r//' | cut -c1-200
