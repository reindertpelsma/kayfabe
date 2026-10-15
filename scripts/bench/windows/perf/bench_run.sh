#!/bin/bash
# bench_run.sh LABEL TEMPLATE(jobs/X.ps1.in) MODE CFG(K|V) KEY=VAL...  : fill the template, run it in the guest, K: host dmon + per-thread cpu diff, collect guest log
LABEL=$1; TPL=$2; MODE=$3; CFG=$4; shift 4
P=/data/claude-perf/wt/scripts/bench/windows/perf; EV=/data/claude-perf/ev/bench/$LABEL; mkdir -p $EV/guest; export EV
if [ $CFG = K ]; then RUN=/var/lib/kf-windows-20261005/boundary-kayfabe-2991; LOG=$RUN/qemu.log; OFF=$(stat -c %s $LOG)
  QP=$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2991.state); python3 $P/threadcpu.py snap $QP $EV/thr0.json
  (timeout ${DMON_S:-900} nvidia-smi dmon -s pucm -d 1 -o T > $EV/host_dmon.txt 2>&1 &); fi
$P/mkjob.sh $P/jobs/$TPL $EV/job.ps1 "$@"
TAILN=${TAILN:-30} bash $P/kfp.sh b_$LABEL $MODE $EV/job.ps1 ${TO:-1500} > $EV/kfp.out 2>&1
pkill -f "nvidia-smi dmon" 2>/dev/null
if [ $CFG = K ]; then python3 $P/threadcpu.py snap $QP $EV/thr1.json; python3 $P/threadcpu.py diff $EV/thr0.json $EV/thr1.json 16 > $EV/threads_cpu.txt
  tail -c +$((OFF+1)) $LOG | grep -a -E "display fps|PROF|USER_RING_LOOP|REFUSED" | cut -c1-1500 > $EV/qemu_slice.txt; fi
grep -a -E "^(START|PRE|REP|R[0-9]|RESULT|TIMEOUT|END|NO_|GM_)" $EV/guest/b_$LABEL.log | sed 's/\r//' | cut -c1-230
