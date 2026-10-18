#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# run_trace.sh TAG PROFILE ITER [QPID]   (export SN_HOST=root@<trusted host>)
# One traced submit-latency run against a running kayfabe guest (no change to the VM, no kayfabe flag):
#   PROFILE: none | tp | up2 | up   (gen_bt.py: kernel tracepoints only / + uprobes on the relay symbols)
#   + fencepoll (host-side read-only observer of the guest's fence word in the guest-RAM memfd) when FENCE=1
# Output: /data/claude-submitlat-20261018/TAG/{trace.txt,fence.txt,guest.txt,raw.csv,sched_before/after}.
set -euo pipefail
TAG=$1; PROF=$2; ITER=$3; H=${SN_HOST:?}; D=/data/claude-submitlat-20261018; FENCE=${FENCE:-1}; PRE=${PRE:-22000}
QBIN=${QBIN:-/var/lib/kf-windows-20261005/kf3-bins/0452613b/qemu-system-x86_64}
QPID=${4:-$(ssh $H "ps -eo pid,args | grep -F '$QBIN' | grep -v grep | awk 'NR==1{print \$1}'")}
HERE=$(cd "$(dirname "$0")" && pwd)
echo "run $TAG profile=$PROF iter=$ITER qpid=$QPID fence=$FENCE"
ssh $H "mkdir -p $D/$TAG; cd $D/$TAG
  for t in /proc/$QPID/task/*; do echo \"\$(basename \$t) \$(cat \$t/comm) \$(cat \$t/schedstat)\"; done > sched_before.txt
  if [ $PROF != none ]; then python3 $D/gen_bt.py $QBIN $QPID $PROF > trace.bt; (BPFTRACE_PERF_RB_PAGES=8192 timeout 110 bpftrace trace.bt > trace.txt 2> trace.err &); fi
  if [ $FENCE = 1 ]; then FD=\$(ls -l /proc/$QPID/fd | awk '/memory-backend-memfd/{print \$9}' | head -1); (taskset -c 10 $D/fencepoll $QPID \$FD 0x5A5A5A5A 0xC8 100 fence.txt > fence.log 2>&1 &); fi
  sleep 3"
PAT=${SEQ:+SEQ_END}; PAT=${PAT:-iteration period}
cat > /tmp/claude-0/rt_guest.ps1 <<PS
Remove-Item C:\kf\sn\ui.txt -ea 0
& C:\kf\sn\sl_it.ps1 -Script C:\kf\sn\sl_probe.ps1 -ScriptArgs "-Out C:\kf\sn\ui.txt ${SEQ:+-Seq $SEQ} -Mode T -Iter $ITER -FenceBase 5A5A5A5A00000000 -PreMs $PRE -Raw C:\kf\sn\raw_$TAG.csv ${PROBE_ARGS:-}" -Wait 120 | Out-Null
\$e=(Get-Date).AddSeconds(150); while (-not (Select-String -Path C:\kf\sn\ui.txt -Pattern '$PAT' -Quiet -ErrorAction SilentlyContinue) -and (Get-Date) -lt \$e) { Start-Sleep 1 }
Get-Content C:\kf\sn\ui.txt
PS
"$HERE/../steelnomad_gap/guest_run.sh" /tmp/claude-0/rt_guest.ps1 200 | tee /tmp/claude-0/rt_guest_$TAG.txt
ssh $H "cd $D/$TAG; cat > guest.txt <<'EOT'
$(cat /tmp/claude-0/rt_guest_$TAG.txt)
EOT
  for t in /proc/$QPID/task/*; do echo \"\$(basename \$t) \$(cat \$t/comm) \$(cat \$t/schedstat)\"; done > sched_after.txt
  sleep 2; pkill -x -f 'bpftrace trace.bt' || true; pkill -x fencepoll || true; sleep 1; wc -l trace.txt fence.txt raw.csv 2>&1 | tail -4"
for x in "" .1 .2 .3 .4; do   # raw per-iteration guest stamps (one file per step of a -Seq run)
  ssh $H "cd /data && python3 kfagent_server.py run 'if (Test-Path C:\\kf\\sn\\raw_${TAG}.csv$x) { Get-Content C:\\kf\\sn\\raw_${TAG}.csv$x }' 60 > $D/$TAG/raw.csv${x:-.0}" 2>/dev/null || true
done
ssh $H "cd $D/$TAG && ln -sf raw.csv.0 raw.csv; wc -l raw.csv.* trace.txt fence.txt | tail -8"
