#!/bin/bash
# hv.sh LABEL API(direct3d11|opengl) [SMI]  : assert environment, then one Heaven in-app benchmark on the running kayfabe VM; records host Xid delta and guest events 153/4101
LABEL=$1; API=$2; P=/data/claude-gap/perf; EV=/data/claude-gap/ev/heaven/$LABEL; mkdir -p $EV
bash $P/assert_env.sh $EV/assert.txt > /dev/null
XB=$(dmesg | grep -c 'NVRM: Xid')
# --- coordinator item 2026-10-16: record the environment of EVERY run (broker backend/uid, memory, swap, steal, interrupts, power) ---
envrec() { echo "== $1 $(date -Is)"; bash $P/memrec.sh $1; ps -eo pid,user,args | grep "[n]vkvm-display-broker" | grep -v runuser | cut -c1-260; nvidia-smi --query-gpu=power.draw,clocks.sm,clocks.mem,utilization.gpu,pstate,pcie.link.gen.current --format=csv,noheader; }
envrec pre > $EV/env_pre.txt 2>&1
# only reclaimable page cache may be dropped, and only if free < 1 GB with swap in use; never kills anything
MF=$(awk '/^MemFree/{print $2}' /proc/meminfo); SU=$(awk '/^SwapTotal/{t=$2}/^SwapFree/{f=$2}END{print t-f}' /proc/meminfo)
if [ "$MF" -lt 1048576 ] && [ "$SU" -gt 0 ]; then echo "LOWMEM free_kB=$MF swap_used_kB=$SU: sync; drop_caches=1" >> $EV/env_pre.txt; sync; echo 1 > /proc/sys/vm/drop_caches; fi
timeout 420 vmstat 1 > $EV/vmstat.txt 2>&1 & VMP=$!
if [ -z "${HOOK:-}" ]; then export HOOK="bash $P/kfp.sh pi_$LABEL interactive $P/jobs/presentinfo.ps1 120" HDELAY=100; fi
echo "HV_START $LABEL $API $(date -Is) xid_before=$XB" > $EV/startmarker
bash $P/heaven_run.sh $LABEL $API K ${3:-0} > /data/claude-gap/ev/heaven/$LABEL.out 2>&1
kill $VMP 2>/dev/null
envrec post > $EV/env_post.txt 2>&1
XA=$(dmesg | grep -c 'NVRM: Xid')
echo "HV_EXIT $LABEL rc=$? $(date -Is) xid_after=$XA" >> $EV/startmarker
timeout 90 python3 /data/kfagent_server.py run 'Get-WinEvent -FilterHashtable @{LogName="System"; Id=153,4101; StartTime=(Get-Date).AddMinutes(-9)} -ErrorAction SilentlyContinue | % { "EVT " + $_.Id + " " + $_.TimeCreated.ToString("HH:mm:ss") + " " + $_.ProviderName }; "EVTDONE"' 70 > $EV/guest_events.txt 2>&1
echo "events: gpu_driver=$(grep -c -E "^EVT [0-9]+ .* (nvlddmkm|Display)" $EV/guest_events.txt) other=$(grep -c -E "^EVT" $EV/guest_events.txt) (153/4101 in last 9 min), host xid delta $((XA-XB))" | tee -a $EV/startmarker
grep -a -E "FPS:" /data/claude-gap/ev/heaven/$LABEL.out | sed 's/&nbsp;/ /g' | sed -E 's/[0-9]{2}:[0-9]{2}:[0-9]{2} / /g' | head -2
