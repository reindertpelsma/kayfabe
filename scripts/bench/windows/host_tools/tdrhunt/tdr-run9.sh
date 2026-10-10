#!/bin/bash
# tdr-run.sh N — TDR hunt runner (host-only tool, copy kept in git as the record of how runs 264+ were launched).
# Derived from winprod-run3.sh. ONE interactive Windows 11 guest on kf3; the CALLER holds /tmp/kayfabe-fastguest.lock (flock -o).
# env: RUN_WIN_FLAGS (extra KF3_* flags; NAME or NAME=VALUE), KF3_REV_BIN (binary under kf3-bins/, default 459da55d),
#      HOLD_SECS (hold after READY, default 900), ETW=1 (live DxgKrnl ETW session started right after QGA answers),
#      SCROLL_EVERY (seconds between Shorts "down" keys during the hold, default 20), KF_GUEST_PW (the throwaway test guest password).
# Writes $W/winprod/runN/{winprod.log,tdr-timeline.txt,guest-*.txt,etw-*.txt,...}. Cleans up exactly like winprod-run3.sh.
set -u
W=/var/lib/kf-windows-20261005
N=$1
REV=${KF3_REV_BIN:-459da55d}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod2.sh
RUN=$W/boundary-kayfabe-$N
O=$W/winprod/run$N
STOPF=/tmp/kf-stop-winprod
PW=${KF_GUEST_PW:?set KF_GUEST_PW}
HOLD=${HOLD_SECS:-900}
mkdir -p $O; echo $$ > $O/runner.pid
L(){ echo "TDRRUN $(date -u +%FT%T.%3NZ) $*" | tee -a $O/winprod.log; }
for v in $(compgen -e | grep '^KF3_'); do unset "$v"; done
unset WIN_TRACE WIN_GSP_OBSERVER WIN_OBS_ONLY WIN_DROP WIN_FLAGS PROD_EXTRA_FLAGS
export KF3_REV=$REV; export WIN_FLAGS="${RUN_WIN_FLAGS:-}"
XID0=$(dmesg | grep -c 'NVRM: Xid')
dmesg | grep 'NVRM: Xid' > $O/xid-before.txt
L "START run=$N bin=kf3-bins/$REV flags=[$WIN_FLAGS] etw=${ETW:-0} hold=$HOLD xid_before=$XID0 pre-qemu=$(pgrep -c qemu-system)"
if [ "$(pgrep -c qemu-system)" != 0 ]; then L "REFUSED: another QEMU is running"; exit 4; fi
[ -e "$RUN" ] && { L "REFUSED: $RUN exists"; exit 5; }
Q(){ timeout 15 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock "$@"; }
G(){ timeout ${GT:-40} python3 $W/boundary-tools/qmp.py $RUN/qga.sock "$@"; }
key(){ Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$1\"}]}" >/dev/null 2>&1; sleep 0.25; }
alive(){ [ -n "${QPID:-}" ] && kill -0 $QPID 2>/dev/null; }
shot(){ Q cmd screendump "{\"filename\":\"$O/$1.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1 && convert $O/$1.ppm $O/$1.png 2>/dev/null; rm -f $O/$1.ppm; }
ncyc(){ grep -a -c 'GSP phase Running -> Suspending' $RUN/qemu.log 2>/dev/null; }
snap(){
  local T=$1 Lg=$RUN/qemu.log
  { echo "== snap $T $(date -u +%FT%T)  alive=$(alive && echo 1 || echo 0) tdr_cycles=$(ncyc)"
    grep -a "phase=Running" $Lg | tail -1 | grep -a -o -E '(inval|walks|cleared|unreconciled|mapped|unmapped)=[^ ]+' | tr '\n' ' '; echo
    echo "xid_now=$(dmesg | grep -c 'NVRM: Xid') (before=$XID0)"
    echo "DEAD=$(grep -a -c 'DEAD:' $Lg) PROBE-DUMP=$(grep -a -c 'PROBE-DUMP' $Lg) PT-SNAP=$(grep -a -c 'PT-SNAP BEGIN' $Lg) panics=$(grep -a -c -i 'panicked' $Lg)"
  } >> $O/evidence.log
}
gps(){ # run a PowerShell snippet in the guest through QGA, output to $O/$1
  local out=$1; shift
  GT=${GT:-120} G qga-exec powershell.exe -NoProfile -Command "$1" 2>&1 | tr -d '\r' >> $O/$out
}
cleanup(){
  trap - TERM INT
  L "CLEANUP begin (alive=$(alive && echo 1 || echo 0)) tdr_cycles=$(ncyc)"
  snap final
  shot final-screen
  if alive; then bash $B stop $N 2>&1 | tail -3 | tee -a $O/winprod.log; fi
  for j in $(seq 1 60); do alive || break; sleep 2; done
  if alive; then Q cmd quit >/dev/null 2>&1; sleep 5; L "clean stop failed: QMP quit"; fi
  sleep 3
  L "qemu left: $(pgrep -c qemu-system)"
  bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $O/winprod.log
  GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
  snap after-stop
  L "END type=$(cat /sys/kernel/iommu_groups/$GRP/type) driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver)) xid_lines=$(dmesg | grep -c 'NVRM: Xid')"
  rm -f $STOPF
  exit 0
}
# SAMPLER=1: irq_sampler.py (LAPIC IRR/ISR/TPR, MSI-X table + PBA, eventfd counts, RFLAGS.IF per vCPU every SAMPLER_PERIOD s from the sign-in to the first TDR + SAMPLER_AFTER s).
# STALLDUMP=1: after the sign-in, watch kayfabe's display trace (needs KF3_DISPLAY_WRITE_TRACE=1): the guest acks every VSync
# (WRITE 0x611800) while LAST_DATA is enabled. When the display thread keeps raising VSyncs (VSYNC h0 ... rm=0x2) and the
# guest has acked none for 600 ms, take (a) six register samples of every vCPU (stop; info registers -a; cont, 200 ms apart)
# and (b) a full guest-memory dump at the stall, then end the hold. The run's host side keeps running while the vCPUs stop.
stall_watch(){
  local lg=$RUN/qemu.log off n a v quiet=0 samples=0
  off=$(stat -c %s $lg)
  while alive && [ ! -e $O/.stall_done ]; do
    sleep 0.1
    local cur; cur=$(stat -c %s $lg)
    a=$(tail -c +$((off+1)) $lg | head -c $((cur-off)) | grep -a -c 'WRITE 0x611800')
    v=$(tail -c +$((off+1)) $lg | head -c $((cur-off)) | grep -a -c 'VSYNC h0.*rm=0x2')
    off=$cur
    if [ "$a" = 0 ] && [ "$v" -gt 0 ]; then quiet=$((quiet+1)); else quiet=0; fi
    if [ $quiet -ge 6 ]; then
      L "STALL detected: no VSync ack for ~${quiet}00 ms while VSyncs are raised; sampling vCPUs"
      for k in 1 2 3 4 5 6; do
        Q cmd stop >/dev/null 2>&1
        { echo "== sample $k $(date -u +%FT%T.%3N)"; Q cmd human-monitor-command '{"command-line":"info registers -a"}' 2>&1; } >> $O/stall-regs.txt
        Q cmd cont >/dev/null 2>&1
        sleep 0.2
      done
      L "STALL: dumping guest memory"
      timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd dump-guest-memory "{\"paging\":false,\"protocol\":\"file:$W/dumps/run$N-stall.elf\"}" >/dev/null 2>&1
      for k in $(seq 1 90); do
        st=$(timeout 8 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd query-dump 2>&1 | tr -d '\n ')
        case "$st" in *completed*|*failed*) break;; esac; sleep 2
      done
      L "STALL: dump $st"
      touch $O/.stall_done
    fi
  done
}
# PREEMPTDUMP=1 (H-S, shape S): watch kayfabe's log for a preempt-all burst the guest never re-enables: >= PD_MIN (6)
# DISABLE_CHANNELS(bDisable=true) since the last bDisable=false, and no bDisable=false for PD_QUIET_MS (500) after the last one.
# Then: one LAPIC/MSI-X sample, info registers -a, and a full guest-memory dump (the VM resumes after it); the hold continues
# 60 s more. The open list's eventData (KEVENT addresses) are saved from the RUNLIST_PREEMPT_COMPLETE lines.
preempt_watch(){
  local lg=$RUN/qemu.log off cur chunk p m burst=0 lastp=0 now st
  off=$(stat -c %s $lg); : > $O/pdump-open.txt
  while alive && [ ! -e $O/.pdump_done ]; do
    sleep 0.1
    cur=$(stat -c %s $lg); chunk=$(tail -c +$((off+1)) $lg | head -c $((cur-off))); off=$cur
    p=$(printf '%s' "$chunk" | grep -a -c 'DISABLE_CHANNELS(bDisable=true')
    m=$(printf '%s' "$chunk" | grep -a -c 'DISABLE_CHANNELS(bDisable=false')
    now=$(date +%s%3N)
    if [ "$m" -gt 0 ]; then burst=0; : > $O/pdump-open.txt; fi
    if [ "$p" -gt 0 ]; then burst=$((burst+p)); lastp=$now; printf '%s\n' "$chunk" | grep -a 'RUNLIST_PREEMPT_COMPLETE posted\|DISABLE_CHANNELS(bDisable=true' >> $O/pdump-open.txt; fi
    if [ $burst -ge ${PD_MIN:-6} ] && [ $((now-lastp)) -ge ${PD_QUIET_MS:-1500} ]; then
      L "PREEMPT-OPEN detected: $burst disables, no enable for $((now-lastp)) ms; sampling + dumping"
      L "PREEMPT-OPEN: dump start"
      timeout 20 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd dump-guest-memory "{\"paging\":false,\"protocol\":\"file:$W/dumps/run$N-preempt.elf\"}" >/dev/null 2>&1
      local st=""
      for k in $(seq 1 120); do
        st=$(timeout 8 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock cmd query-dump 2>&1 | tr -d '\n ')
        case "$st" in *completed*|*failed*) break;; esac; sleep 2
      done
      L "PREEMPT-OPEN: dump $st"
      { echo "== after dump $(date -u +%FT%T.%3N)"; Q cmd human-monitor-command '{"command-line":"info registers -a"}' 2>&1; } > $O/pdump-regs.txt
      date +%s > $O/.pdump_done
    fi
  done
}
trap cleanup TERM INT
L "gpu: $(nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader)"
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $O/winprod.log
GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$GRP/type)
if [ "$T" != identity ]; then L "REFUSED: group $GRP is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
rm -f $STOPF
WIN_REUSE=0 bash $B run $N > $W/wr-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; tail -20 $W/wr-launch-$N.log | tee -a $O/winprod.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
L "launched qpid=$QPID"
python3 -I -c "import json;d=json.load(open('$RUN/command.json'));print('ENV_PASSED',' '.join(sorted(d['flags'])))" | tee -a $O/winprod.log
cp $RUN/command.json $O/command.json
TA=0; for j in $(seq 1 90); do alive || break; G qga-ping >/dev/null 2>&1 && { TA=$(date +%s); break; }; sleep 2; done
L "qga answered: ta=$TA alive=$(alive && echo 1 || echo 0)"
if [ "$TA" != 0 ] && alive; then
  if [ "${ETW:-0}" = 1 ]; then
    for try in 1 2 3 4 5 6; do
      : > $O/etw-arm.txt
      GT=60 G qga-exec powershell.exe -NoProfile -Command 'New-Item -ItemType Directory -Force -Path C:\kf | Out-Null; $q = (logman query kfdxg -ets 2>&1 | Out-String); if ($q -match "Running") { "ETWARMED already-running" } else { Remove-Item C:\kf\kfdxg*.etl -ErrorAction SilentlyContinue; logman create trace kfdxg -p "Microsoft-Windows-DxgKrnl" 0x1 5 -o C:\kf\kfdxg.etl -f bincirc -bs 1024 -nb 64 512 -ft 1 -max 512 -ets 2>&1; $q = (logman query kfdxg -ets 2>&1 | Out-String); if ($q -match "Running") { "ETWARMED" } }' > $O/etw-arm.txt 2>&1
      grep -q "ETWARMED" $O/etw-arm.txt && break
      sleep 5
    done
    L "ETW armed (try $try, circular; self-stop task on nvlddmkm 153): $(grep -c . $O/etw-arm.txt) lines, $(grep -c ETWARMED $O/etw-arm.txt) armed"
  fi
  G qga-exec net.exe user vast $PW >/dev/null 2>&1; L "password set rc=$?"
  gps guest-pre.txt '"utc now: " + (Get-Date).ToUniversalTime().ToString("o"); $gd="HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers"; "GraphicsDrivers: " + ((Get-ItemProperty $gd | Select-Object * -ExcludeProperty PS* | Out-String).Trim())'
  if [ "${SAMPLER:-0}" = 1 ]; then
    ( python3 $W/tdrhunt/irq_sampler.py $RUN/qmp.sock $QPID $O/irq-samples.txt $O/.sampler_stop ${SAMPLER_PERIOD:-0.25} 8 &
      SP=$!; while alive && [ ! -e $O/.signin_done ]; do sleep 0.2; done
      C0=$(ncyc); while alive && [ "$(ncyc)" -le "$C0" ]; do sleep 0.2; done; sleep ${SAMPLER_AFTER:-3}; touch $O/.sampler_stop; wait $SP ) &
  fi
  while alive && [ $(( $(date +%s) - TA )) -lt ${SIGNIN_DELAY:-30} ]; do sleep 1; done
  if alive; then
    [ "${PREEMPTDUMP:-0}" = 1 ] && { preempt_watch & }
    shot pre-signin
    L "SIGNIN keys at +$(( $(date +%s) - TA ))s"
    key spc; sleep 3
    for c in k f s i g n 7; do key $c; done; key ret
    L "SIGNIN sent"
    [ "${STALLDUMP:-0}" = 1 ] && { stall_watch & }
    touch $O/.signin_done
   if [ "${ETW:-0}" = 1 ]; then
      ( C0=$(ncyc); while alive && [ "$(ncyc)" -le "$C0" ]; do sleep 0.3; done
        alive && { touch $O/.etw_stopped; L "ETW stop (tdr_cycles=$(ncyc))"; GT=900 timeout 900 python3 $W/boundary-tools/qmp.py $RUN/qga.sock qga-exec powershell.exe -NoProfile -Command "$(cat $W/kayfabe-win-6fafcc6e/scripts/bench/windows/dxg_etw_stop_tail.ps1)" > $O/etw-stop.txt 2>&1; L "ETW stop rc=$? lines=$(wc -l < $O/etw-stop.txt)"; } ) &
    fi
    sleep 25; shot after-signin; snap after-signin
    ev(){ Q cmd input-send-event "{\"events\":$1}" >/dev/null 2>&1; sleep 0.15; }
    ev '[{"type":"abs","data":{"axis":"x","value":802}},{"type":"abs","data":{"axis":"y","value":4854}}]'
    for k in 1 2; do ev '[{"type":"btn","data":{"down":true,"button":"left"}}]'; ev '[{"type":"btn","data":{"down":false,"button":"left"}}]'; done
    L "EDGE double-click sent (abs 802,4854)"
    sleep 2; key ret; L "EDGE Enter sent"
    sleep 25; shot after-edge
    if alive; then
      Q cmd send-key '{"keys":[{"type":"qcode","data":"ctrl"},{"type":"qcode","data":"l"}]}' >/dev/null 2>&1; sleep 1
      for c in y o u t u b e dot c o m slash s h o r t s; do key $c; done; key ret
      L "SHORTS url typed"
      for n in $(seq 1 6); do sleep 9; key down; alive || break; done
      L "SHORTS scrolled"
    fi
    sleep 15; shot after-shorts; snap after-shorts
    L "READY alive=$(alive && echo 1 || echo 0) tdr_cycles=$(ncyc)"
  fi
fi
# ---- timed hold: scroll Shorts, log the TDR (GSP cycle) count against the host clock every 5 s ----
T0=$(date +%s); LASTC=-1; LASTK=0; I=0
while alive && [ ! -e $STOPF ] && [ ! -e $O/.stall_done ] && { [ ! -e $O/.pdump_done ] || [ $(( $(date +%s) - $(cat $O/.pdump_done) )) -lt 60 ]; } && [ $(( $(date +%s) - T0 )) -lt $HOLD ]; do
  C=$(ncyc); NOW=$(date +%s)
  if [ "$C" != "$LASTC" ]; then echo "$(date -u +%FT%T) tdr_cycles=$C hold_t=$((NOW-T0))" >> $O/tdr-timeline.txt; LASTC=$C; fi
  if [ $((NOW-LASTK)) -ge ${SCROLL_EVERY:-20} ]; then key down; LASTK=$NOW; fi
  I=$((I+1)); [ $((I % 12)) = 0 ] && { snap periodic; shot "hold-$((NOW-T0))"; }
  sleep 5
done
L "HOLD ended: alive=$(alive && echo 1 || echo 0) tdr_cycles=$(ncyc) stopfile=$([ -e $STOPF ] && echo 1 || echo 0) hold_s=$(( $(date +%s) - T0 ))"
# ---- guest evidence (read-only) ----
if alive; then
  gps guest-events.txt '"utc now: " + (Get-Date).ToUniversalTime().ToString("o"); "uptime: " + ((Get-Date) - (gcim Win32_OperatingSystem).LastBootUpTime).ToString(); Get-WinEvent -FilterHashtable @{LogName="System";StartTime=(Get-Date).AddMinutes(-60)} -ErrorAction SilentlyContinue | Where-Object { $_.ProviderName -match "nvlddmkm|Display|dxg|Kernel-Power|WER|BugCheck|LiveKernel|Watchdog|Wininit" -or $_.Id -in 4101,141,117,116,1001,41 } | Sort-Object TimeCreated | ForEach-Object { "{0} {1} {2} {3}" -f $_.TimeCreated.ToUniversalTime().ToString("o"),$_.ProviderName,$_.Id,(($_.Message -replace "\s+"," ")[0..220] -join "") }; "LiveKernelReports: " + ((Get-ChildItem C:\Windows\LiveKernelReports -Recurse -ErrorAction SilentlyContinue | Select-Object -First 30 FullName,Length,LastWriteTime | Out-String).Trim())'
fi
if [ "${ETW:-0}" = 1 ] && [ ! -e $O/.etw_stopped ] && alive; then
  L "ETW stop at the end of the hold (no TDR after sign-in)"
  GT=900 timeout 900 python3 $W/boundary-tools/qmp.py $RUN/qga.sock qga-exec powershell.exe -NoProfile -Command "$(cat $W/kayfabe-win-6fafcc6e/scripts/bench/windows/dxg_etw_stop_tail.ps1)" > $O/etw-stop.txt 2>&1
  L "ETW decode rc=$? lines=$(wc -l < $O/etw-stop.txt)"
fi
L "DONE tdr_cycles=$(ncyc)"
cleanup
