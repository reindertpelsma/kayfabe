#!/bin/bash
# vmstop.sh LABEL : graceful guest shutdown through kfagent, wait <=5 min for qemu to exit by itself, save shutdown-only counters. No kill.
LBL=$1; EV=/data/claude-mctdr/ev/vm-$LBL; mkdir -p $EV
RUN=/var/lib/kf-windows-20261005/boundary-kayfabe-2990; LOG=$RUN/qemu.log
QPID=$(sed -n 's/^QPID=//p' /var/lib/kf-windows-20261005/windows-broker-run2990.state)
echo "stop $LBL qpid=$QPID rev=$(readlink /proc/$QPID/exe | sed 's#.*/kf3-bins/##') at $(date -Is)" | tee $EV/stop.txt
timeout 30 python3 /data/kfagent_server.py run 'shutdown /s /t 0 /f; "shutdown issued"' 20 | tee -a $EV/stop.txt
for i in $(seq 1 60); do kill -0 $QPID 2>/dev/null || break; sleep 5; done
if kill -0 $QPID 2>/dev/null; then echo "STILL ALIVE after 5 min: NOT killing; decision needed" | tee -a $EV/stop.txt; exit 3; fi
echo "qemu exited by itself after ~$((i*5)) s at $(date -Is)" | tee -a $EV/stop.txt
sleep 3
grep -a 'USER_RING_LOOP SUMMARY' $LOG | cut -c1-1500 > $EV/user_ring_loop_summary.txt
grep -a -E 'late_max_us=' $LOG | grep -a -o 'late_max_us=[0-9]*' | cut -d= -f2 | sort -n | tail -3 | tr '\n' ' ' > $EV/late_max_us_top3.txt
grep -a -E 'PROF t=.*(bar0_handler|drainer_wake|va )' $LOG | tail -6 | cut -c1-300 > $EV/prof_tail.txt
grep -a -E 'RETIRED.*(dead=Some|forwarded)' $LOG | cut -c1-260 | tail -40 > $EV/retired.txt
grep -a -E 'ring REFUSED|chan token .* DEAD' $LOG | cut -c1-300 > $EV/ring_refused.txt
tail -n 3000 $LOG | gzip > $EV/qemu-log-tail3000.gz; cp $LOG $EV/qemu-full.log; gzip -f $EV/qemu-full.log
ls -la $EV | tail -8; wc -l $EV/ring_refused.txt $EV/user_ring_loop_summary.txt
ps -eo pid,etimes,cmd | grep -E "launch3|nvkvm-display-broker|qemu-system" | grep -v grep | cut -c1-120
