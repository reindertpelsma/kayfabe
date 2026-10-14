#!/bin/bash
# variant.sh <name> <WIN_CPU> <ncycles> [norelaunch] — stop VM 2990 (graceful), relaunch with WIN_CPU, run S3 cycles, judge each
NAME=$1; CPU=$2; N=${3:-3}; NOREL=${4:-}
W=/var/lib/kf-windows-20261005; R=$W/boundary-kayfabe-2990; ST=$W/windows-broker-run2990.state
LOG=/data/claude-s3win-var-$NAME.log; exec > >(tee -a $LOG) 2>&1
AG() { python3 /data/kfagent_server.py run "$1" ${2:-120} 2>&1; }
echo "VARIANT $NAME cpu=$CPU start $(date -Is)"
if [ -z "$NOREL" ]; then
  Q=$(sed -n 's/^QPID=//p' $ST)
  if kill -0 $Q 2>/dev/null; then
    AG 'shutdown /s /t 0 /f' 20 >/dev/null
    for i in $(seq 60); do kill -0 $Q 2>/dev/null || break; sleep 1; done
    if kill -0 $Q 2>/dev/null; then echo "graceful shutdown did not end qemu in 60s; SIGTERM $Q"; kill -TERM $Q; for i in $(seq 40); do kill -0 $Q 2>/dev/null || break; sleep 1; done; else echo "guest shut down gracefully, qemu $Q exited"; fi
  fi
  for i in $(seq 40); do pgrep -f "[c]laude-manual-owner11|[c]laude-s3win-launch" >/dev/null || break; sleep 1; done
  sed -e "s|^export WIN_CPU=.*|export WIN_CPU=\"$CPU\"|" -e "s|^\(export WIN_QEMU_EXTRA=\".*\)\"\$|\1 ${EXTRA:-}\"|" /data/claude-manual-owner11-dmafq.sh > /data/claude-s3win-launch-$NAME.sh
  rm -f /data/kfagent/q/*.json
  (cd /data; REV=${REV:-0c5fedff} nohup flock -w 10 -o /tmp/kayfabe-fastguest.lock bash /data/claude-s3win-launch-$NAME.sh > /data/claude-s3win-launch-$NAME.out 2>&1 < /dev/null &)
  for i in $(seq 30); do grep -q OWNER_SESSION_READY /data/claude-s3win-launch-$NAME.out 2>/dev/null && break; sleep 2; done
  echo "launched: $(grep OWNER_SESSION_READY /data/claude-s3win-launch-$NAME.out | cut -c1-120)"
  Q=$(sed -n 's/^QPID=//p' $ST); echo "qemu cpu arg: $(tr '\0' '\n' < /proc/$Q/cmdline | grep -a '^host,')"
  # wait for the guest to answer, then let it settle
  rm -f /data/kfagent/q/*.json
  for t in 1 2 3 4 5; do o=$(AG '"UP"' 60); echo "$o" | grep -q UP && break; rm -f /data/kfagent/q/*.json; done
  echo "agent: $o"; sleep 60
fi
AG 'powercfg /change standby-timeout-ac 0; powercfg /hibernate off; Set-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl AutoReboot 0; "ok"' 90 | tail -1
for c in $(seq 1 $N); do
  echo "--- cycle $c $(date -Is)"
  BOOT0=$(AG '(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")' 90 | tail -1)
  RES0=$(grep -ac GSP-RESUME $R/qemu.log); RST0=$(grep -ac 'device reset #[0-9]*: vCPUs out' $R/qemu.log); WK0=$(grep -ac 'S3 WAKE' $R/qemu.log)
  bash /data/claude-s3win-cycle.sh $NAME$c 2>&1 | grep -E "status=|WAKE at"
  T0=$(date +%s); ok=0
  for i in $(seq 40); do o=$(AG '(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")' 15 | tail -1); echo "$o" | grep -q '^20' && { ok=1; break; }; rm -f /data/kfagent/q/*.json; done
  echo "agent back=$ok after $(( $(date +%s)-T0 ))s boot0=$BOOT0 boot1=$o"
  sleep 20
  echo "kf3: S3WAKE+$(( $(grep -ac 'S3 WAKE' $R/qemu.log)-WK0 )) GSP-RESUME+$(( $(grep -ac GSP-RESUME $R/qemu.log)-RES0 )) coldresets+$(( $(grep -ac 'device reset #[0-9]*: vCPUs out' $R/qemu.log)-RST0 ))"
  AG 'Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=(Get-Date).AddMinutes(-4)} -EA 0 | ? { $_.Id -in 41,1001,153,141,14,4101,107,42 } | Select TimeCreated,Id,ProviderName | ft -auto | Out-String -Width 150; nvidia-smi --query-gpu=name --format=csv,noheader' 120 | grep -v '^$' | head -12
  python3 /data/claude-shot.py; python3 -c "
from PIL import Image; im=Image.open('/tmp/claude-2990.png'); print('screen extrema', im.getextrema()[0])"
  if [ "$ok" != 1 ] || [ "$BOOT0" != "$o" ]; then echo "CYCLE $c FAIL"; break; else echo "CYCLE $c PASS-CANDIDATE (verify GSP-RESUME delta)"; fi
done
echo "VARIANT $NAME end $(date -Is)"
