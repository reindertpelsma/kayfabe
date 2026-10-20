#!/bin/bash
# session3.sh - one fast-guest boot running a baton sequence.  Run it under the caller's flock on
# /tmp/kayfabe-fastguest.lock (the harness takes its own, different lock).
#   session3.sh TAG SEQ [KEY=VAL ...]       e.g. session3.sh a1 pair:90,pair:90 KF3_DEBUG=1 SMP=4 QEMU_BIN=...
# Special keys (not exported to the VMM): SMP (vCPUs, default 4), QEMU_BIN, E (evidence dir),
# FGD (fast-guest dir with vmlinuz+initrd), NOFORCE=1 (do not set the debug/force-migratable pair).
# Every other KEY=VAL is exported to the VMM.  All inherited KF3_* are unset first.
set -u
TAG=$1; SEQ=$2; shift 2
SMP=4; E=/data/claude-baton-debug-20261020; FGD=$E/fg
QEMU_BIN=/var/lib/kf-windows-20261005/kf3-bins/0d7ba743/qemu-system-x86_64
NOFORCE=0; EXTRA=()
for kv in "$@"; do
  case $kv in
    SMP=*) SMP=${kv#SMP=};; QEMU_BIN=*) QEMU_BIN=${kv#QEMU_BIN=};; E=*) E=${kv#E=};; FGD=*) FGD=${kv#FGD=};;
    NOFORCE=1) NOFORCE=1;; *) EXTRA+=("$kv");;
  esac
done
R=$E/runs/$TAG; mkdir -p $R
for n in $(compgen -e | sed -n '/^KF3_/p'); do unset "$n"; done
export KF3_TSG_MIRROR=0
if [ $NOFORCE = 0 ]; then export KF3_DEBUG=1 KF3_DEBUG_FORCE_MIGRATABLE=1; fi
for kv in "${EXTRA[@]}"; do export "$kv"; done
# total budget = sum of run durations + 25 s per run + 90 s
BUDGET=90
for item in $(echo "$SEQ" | tr ',' ' '); do d=$(echo "$item" | cut -d: -f2); BUDGET=$((BUDGET + d + 25)); done
export BENCH_DIR=$R KF_FASTGUEST_DIR=$FGD QEMU_BIN NVKVM_RAM_MB=6144 KF_SMP=$SMP KF_LOCK=/tmp/claude-baton-debug.lock KF_REQUIRE_FORWARD=0
export KF_APPEND="KF_BATON_SEQ=$SEQ loglevel=4 "
xid() { dmesg | grep -c Xid; }
X0=$(xid)
echo "SESSION $TAG start $(date -Is) smp=$SMP seq=$SEQ qemu=$QEMU_BIN host_xid0=$X0 budget=$BUDGET env_KF3=$(env | grep ^KF3_ | tr '\n' ' ')" | tee $R/session.log
nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.sm,temperature.gpu,memory.used --format=csv,noheader -l 1 > $R/nvsmi.csv 2>/dev/null &
SMI=$!
( while :; do sleep 1; if [ "$(xid)" -gt "$X0" ]; then echo "HOST XID ROSE $(date -Is)" | tee -a $R/session.log; pkill -TERM -f '[k]f-fastguest'; break; fi; done ) &
XW=$!
bash /data/claude-baton-linux-20261020/scripts/fastguest/run_fast_guest.sh $TAG $BUDGET > $R/harness.out 2>&1
rc=$?
kill $SMI $XW 2>/dev/null
echo "SESSION $TAG end $(date -Is) harness_rc=$rc host_xid_after=$(xid)" | tee -a $R/session.log
cp $R/fast_${TAG}_serial.log $R/serial.log 2>/dev/null
mkdir -p $R/files
tr -d '\r' < $R/fast_${TAG}_serial.log | awk -v d=$R/files '/^BATON_FILE_BEGIN /{f=d"/"$2; next} /^BATON_FILE_END /{close(f); f=""; next} f!=""{print > f}'
grep -a 'BATON:' $R/fast_${TAG}_serial.log | tr -d '\r' > $R/guest_baton.txt
grep -a -o -E '(USER_RING_[A-Z_]*|BACKSTOP[A-Za-z_]*|BORN [A-Za-z]*|TSG_MIRROR[A-Za-z_]*|backstop_[a-z_]*=[0-9]*)' $R/fast_${TAG}_qemu.log 2>/dev/null | sort | uniq -c | sort -rn | head -40 > $R/qemu_counters.txt
python3 -I $(dirname $0)/campaign_report.py $R/files 2>&1 | tee $R/report.tsv | sed "s/^/REPORT /"
tail -3 $R/harness.out | tee -a $R/session.log
