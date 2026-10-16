#!/bin/bash
# win_heaven.sh LABEL MODE(user|root) [API=direct3d11] : one Heaven in-app benchmark in a fresh overlay of the staged image U0,
# QEMU + broker as the given uid. Host-side harness (kfagent, QMP input, nvidia-smi dmon) runs as root; none of that is the product.
LABEL=$1; MODE=$2; API=${3:-direct3d11}
P=/data/claude-perf/wt/scripts/bench/windows/perf; EV=/data/claude-rootless/ev/heaven/$LABEL; mkdir -p $EV/guest; export EV
if [ $MODE = user ]; then D=/home/kfrootless/win; AS="/data/claude-rootless/bin/as_user.sh"; else D=/data/claude-rootless/win-root; AS=""; mkdir -p $D; fi
RUN=$D/$LABEL; OVL=$D/$LABEL.qcow2; VARS=$D/$LABEL.vars.fd
$AS qemu-img create -q -f qcow2 -F qcow2 -b /home/kfrootless/win/u0.qcow2 $OVL || exit 2
if [ $MODE = user ]; then cp /home/kfrootless/win/u0.vars.fd $VARS; chown kfrootless $VARS; else cp /home/kfrootless/win/u0.vars.fd $VARS; fi
echo "HEAVEN_RUN_START $LABEL mode=$MODE $(date -Is)"
nohup flock -o /tmp/kayfabe-fastguest.lock /data/claude-rootless/bin/win_session.sh $MODE $RUN $OVL $VARS > $EV/session.log 2>&1 &
SP=$!
for i in $(seq 120); do grep -q WIN_STARTED $EV/session.log 2>/dev/null && break; sleep 2; done
grep -q WIN_STARTED $EV/session.log || { echo "NOT STARTED"; cat $EV/session.log; exit 3; }
QP=$(cat $RUN/qemu.pid)
echo "QEMU pid=$QP $(grep -E '^(Uid|CapEff)' /proc/$QP/status | tr '\t\n' '  ')" | tee $EV/proc_status.txt
for i in $(seq 60); do r=$(timeout 40 python3 /data/kfagent_server.py run '"UP"' 30 2>&1 | tr -d "\r" | tail -1); [ "$r" = UP ] && break; done
echo "GUEST_UP $(date -Is) tries=$i"
export QMPSOCK=$RUN/qmp.sock
sleep 20   # let the desktop settle after the agent comes up
$P/mkjob.sh $P/jobs/heaven.ps1.in $EV/job.ps1 API=$API DEFS=RELEASE,QUALITY_ULTRA SMI=0
OFF=$(stat -c %s $RUN/qemu.log)
(timeout 560 nvidia-smi dmon -s pucm -d 1 -o T > $EV/host_dmon.txt 2>&1 &)
python3 $P/threadcpu.py snap $QP $EV/thr0.json 2>/dev/null
sed -e 's#/data/kfagent_server.py#/data/kfagent_server.py#' $P/kfp.sh > /dev/null
TAILN=60 bash $P/kfp.sh hv_$LABEL interactive $EV/job.ps1 700 > $EV/kfp.out 2>&1 &
KP=$!
sleep 38; bash $P/inputs.sh click >/dev/null; sleep 1; bash $P/inputs.sh key f9 > /dev/null
wait $KP
pkill -x nvidia-smi 2>/dev/null
python3 $P/threadcpu.py snap $QP $EV/thr1.json 2>/dev/null; python3 $P/threadcpu.py diff $EV/thr0.json $EV/thr1.json 16 > $EV/threads_cpu.txt 2>&1
tail -c +$((OFF+1)) $RUN/qemu.log | grep -a -E "display fps|PROF|USER_RING_LOOP|REFUSED" | cut -c1-1500 > $EV/qemu_slice.txt
grep -a -E "^(START|PRE|LAUNCH|RESULT_FOUND|HLOG|END)" $EV/guest/hv_$LABEL.log | sed 's/\r//' | cut -c1-200 > $EV/result.txt
cat $EV/result.txt | head -30
grep -E "^(Uid|CapEff)" /proc/$QP/status > $EV/proc_status_end.txt 2>/dev/null
timeout 40 python3 /data/kfagent_server.py run 'shutdown /s /t 0 /f; "issued"' 30
for i in $(seq 100); do kill -0 $QP 2>/dev/null || break; sleep 3; done
kill -0 $QP 2>/dev/null && echo "STILL_ALIVE $QP"
gzip -c $RUN/qemu.log > $EV/qemu.log.gz; cp $RUN/flags.txt $RUN/argv.txt $EV/
echo "HEAVEN_RUN_END $LABEL $(date -Is)"
