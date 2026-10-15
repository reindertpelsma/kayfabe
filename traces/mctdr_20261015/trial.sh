#!/bin/bash
# trial.sh CFG N MODE [DRIVE_S]   one Minecraft trial on the running VM (run on the trusted host)
# MODE = agent | owner (see guest/mcstart2.ps1). Evidence in /data/claude-mctdr/ev/trials/<CFG>_t<N>/
CFG=$1; N=$2; MODE=${3:-agent}; DRV=${4:-180}
TAG=${CFG}_t$N; EV=/data/claude-mctdr/ev/trials/$TAG; mkdir -p $EV; exec > >(tee -a $EV/trial.log) 2>&1
RUN=/var/lib/kf-windows-20261005/boundary-kayfabe-2990; LOG=$RUN/qemu.log; GD=/data/claude-mctdr/ev/guest
QPID=$(sed -n 's/^QPID=//p' /var/lib/kf-windows-20261005/windows-broker-run2990.state)
G() { python3 /data/kfagent_server.py run "$1" ${2:-60}; }
gpush() { local b; b=$(base64 -w0 "$1"); G "New-Item -ItemType Directory -Force C:\\kf\\mctdr | Out-Null; [IO.File]::WriteAllBytes('$2',[Convert]::FromBase64String('$b')); (Get-Item '$2').Length" >/dev/null; }
runi() { G "\$a=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-WindowStyle Hidden -NoProfile -ExecutionPolicy Bypass $2'; \$p=New-ScheduledTaskPrincipal -UserId 'kfadmin' -LogonType Interactive -RunLevel Limited; Unregister-ScheduledTask -TaskName 'mct-$1' -Confirm:\$false -ErrorAction SilentlyContinue; Register-ScheduledTask -TaskName 'mct-$1' -Action \$a -Principal \$p | Out-Null; Start-ScheduledTask -TaskName 'mct-$1'; 'STARTED $1 ' + (Get-Date).ToString('HH:mm:ss.fff')"; }
echo "TRIAL $TAG mode=$MODE drive=${DRV}s qpid=$QPID rev=$(readlink /proc/$QPID/exe | sed 's#.*/kf3-bins/##') env_prof=$(tr '\0' '\n' < /proc/$QPID/environ | grep -c '^KF3_PROF=1') host=$(date -Is)"
kill -0 $QPID || { echo "qemu not alive"; exit 2; }
L0=$(wc -l < $LOG); T0=$(date +%s); echo "L0=$L0"
gpush $GD/mcstart2.ps1 'C:\kf\mctdr\mcstart2.ps1'; gpush $GD/gstate.ps1 'C:\kf\mctdr\gstate.ps1'; gpush $GD/mcstop.ps1 'C:\kf\mctdr\mcstop.ps1'
sed 's#C:\\kf\\live4\\drive.log#C:\\kf\\mctdr\\drive.log#' $GD/drive2.ps1 > $EV/drive.ps1; gpush $EV/drive.ps1 'C:\kf\mctdr\drive.ps1'
G '$o="C:\Users\kfadmin\AppData\Roaming\.minecraft\options.txt"; if(-not (Test-Path C:\kf\mctdr\options.txt.orig)){Copy-Item $o C:\kf\mctdr\options.txt.orig}; (Get-Content $o) -replace "^pauseOnLostFocus:.*","pauseOnLostFocus:false" | Set-Content $o; Remove-Item C:\kf\mctdr\drive.log -ErrorAction SilentlyContinue; Remove-Item C:\kf\mctdr\mc_start_*.txt -ErrorAction SilentlyContinue; "GUESTNOW " + (Get-Date).ToString("HH:mm:ss")' | tee $EV/guestnow.txt
GN=$(sed -n 's/^GUESTNOW //p' $EV/guestnow.txt | tr -d '\r')
SINCE=$(date -u -d "today $GN 15 minutes ago" +%H:%M:%S)
TSTART=$(date -u -d "today $GN 5 seconds ago" +%H:%M:%S)
echo "--- pre-trial guest state (events since $SINCE, javaw must be none)"; G "C:\\kf\\mctdr\\gstate.ps1 -since $SINCE" | tee $EV/pre-state.txt
runi launch "-File C:\\kf\\mctdr\\mcstart2.ps1 -Tag $TAG -Mode $MODE"
TL=$(date +%s); echo "launch host=$(date +%T) guest~$GN"
sleep 25; [ "${NODRIVE:-0}" = 1 ] && echo "NODRIVE (idle at spawn)" || runi drive "-File C:\\kf\\mctdr\\drive.ps1 -Plan both -PlainS 60 -BusyS $((DRV-60))"
if [ -n "${ACT:-}" ]; then
  sleep 11; echo "ACT at +$(( $(date +%s)-TL ))s host=$(date +%T): $ACT" | tee $EV/act-start.txt
  ( G "$ACT; (Get-Date).ToString('HH:mm:ss')" 100 > $EV/act.out 2>&1 & )
elif [ "${PM:-0}" = 1 ]; then
  sleep 11; echo "PM: starting PresentMon exactly as live4 (powershell -ep bypass -File C:\\kf\\presentmon_minecraft.ps1 -Timed 12), at +$(( $(date +%s)-TL ))s host=$(date +%T)" | tee $EV/pm-start.txt
  ( G "powershell -ep bypass -File C:\\kf\\presentmon_minecraft.ps1 -Timed 12 -Tag $TAG; Start-Sleep 20; Get-ChildItem C:\\kf\\pm_${TAG}_* | Select Name,Length | Out-String; (Get-Date).ToString('HH:mm:ss')" 100 > $EV/pm.out 2>&1 & )
fi
TD=$(date +%s); echo "drive started at +$((TD-TL))s"
RESULT=SURVIVED; CRASHAT=; i=0
while :; do
  sleep 8; NOW=$(date +%s); EL=$((NOW-TL))
  REF=$(tail -n +$L0 $LOG | grep -a -c -E 'ring REFUSED|chan token .* DEAD')
  if [ "${QUIET:-0}" = 1 ] && [ $EL -lt $((25+DRV)) ]; then
    # no guest command at all during the run: host-side signs only (qemu.log), one guest query at the end
    REF2=$(tail -n +$L0 $LOG | grep -a -c -E 'ring REFUSED|GSP REFUSED fn76/0x906f0102|GSP phase Running -> Suspending')
    echo "+${EL}s QUIET host_signs=$REF2"
    [ "$REF2" -gt 0 ] && { RESULT=TDR; CRASHAT=$EL; break; }
    continue
  fi
  ST=$(G "C:\\kf\\mctdr\\gstate.ps1 -since $TSTART" 40); echo "$ST" > $EV/state-last.txt
  T153=$(echo "$ST" | grep -c -E 'EVT .* id=(153|4101) ')
  JA=$(echo "$ST" | sed -n 's/^JAVAW //p' | tr -d '\r')
  echo "+${EL}s refused_lines=$REF tdr_events=$T153 javaw=$JA"
  if [ "$T153" -gt 0 ] || [ "$REF" -gt 0 ] || ! kill -0 $QPID 2>/dev/null; then RESULT=TDR; CRASHAT=$EL; break; fi
  if [ "$JA" = none ]; then RESULT=JAVAW_EXIT; CRASHAT=$EL; break; fi
  [ $EL -ge $((25+DRV+8)) ] && break
done
[ "$RESULT" != SURVIVED ] && { echo "event at +${CRASHAT}s, collecting after 20 s"; sleep 20; }
G "C:\\kf\\mctdr\\gstate.ps1 -since $TSTART" 60 > $EV/state-final.txt
echo "RESULT=$RESULT at=${CRASHAT:-none}s" | tee $EV/result.txt
L1=$(wc -l < $LOG); tail -n +$L0 $LOG > $EV/qemu-slice.log; gzip -f $EV/qemu-slice.log
zcat $EV/qemu-slice.log.gz | grep -a -E 'ring REFUSED|DEAD|RETIRED.*dead=Some|RESET_CHANNEL|0x906f0102|GSP REFUSED|GSP phase|Xid|USER_RING_MIGRATED|rc\[armed' | cut -c1-330 > $EV/qemu-key.txt
zcat $EV/qemu-slice.log.gz | grep -a -o 'late_max_us=[0-9]*' | cut -d= -f2 | sort -n | tail -1 > $EV/late_max_us.txt
zcat $EV/qemu-slice.log.gz | grep -a 'PROF t=.*bar0_handler' | sed 's/^kf3: //' | awk '{print $2,$3,$4,$5,$NF}' | tail -3 > $EV/prof_bar0_tail.txt
journalctl -k --since "@$T0" --no-pager 2>/dev/null | grep -a -i -E 'xid|nvrm|kvm|vfio' > $EV/host-kernel.txt; echo "host kernel Xid lines: $(grep -c -i xid $EV/host-kernel.txt)"
echo "qemu_alive=$(kill -0 $QPID 2>/dev/null && echo yes || echo no) broker_alive=$(pgrep -f 'nvkvm-display-broker.*run2990' | head -1 | sed 's/^$/no/;t;s/.*/yes/') lines=$L0-$L1" | tee $EV/alive.txt
python3 /data/claude-mctdr/ev/screendump.py $EV/shot.ppm >/dev/null && convert $EV/shot.ppm -resize 800x450 -quality 55 $EV/shot.jpg && rm -f $EV/shot.ppm
G 'Get-ChildItem C:\Users\kfadmin\AppData\Roaming\.minecraft\crash-reports,C:\Users\kfadmin\AppData\Roaming\.minecraft\hs_err*,C:\Windows\System32\hs_err*,C:\Users\kfadmin\hs_err* -ErrorAction SilentlyContinue | ? { $_.LastWriteTime -gt (Get-Date).AddMinutes(-8) } | select FullName,Length,LastWriteTime | ft -auto | out-string -width 200; Get-Content C:\kf\mctdr\drive.log -ErrorAction SilentlyContinue | select -last 6; Get-Content C:\kf\mctdr\mc_start_*.txt; Get-Content C:\Users\kfadmin\AppData\Roaming\.minecraft\logs\latest.log | ? { $_ -notmatch "^\s+at |Resizing|Caused|\.\.\. " } | select -last 12 | % { if($_.Length -gt 180){$_.Substring(0,180)}else{$_} }' 60 > $EV/guest-misc.txt
echo "--- stopping Minecraft"; G 'C:\kf\mctdr\mcstop.ps1' 60
echo "TRIAL_DONE $TAG $RESULT $(date -Is)"
