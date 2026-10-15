#!/bin/bash
# monitor.sh — READ-ONLY host monitor for Win11 VM 2990 while the Blender tests run (trusted host only).
#   monitor.sh start <outdir> [period_s=30] : loop until <outdir>/monitor.stop exists; appends <outdir>/monitor.csv
#   monitor.sh once  [t0_epoch] [g0_guest_iso]  : one CSV line (columns below) — used at test boundaries
# Columns: host_time, xid_all_since_t0 (journalctl -k lines with "Xid" since the monitor start), xid_vm_pid (those naming the
#   VM's qemu pid), gsp_resets (qemu.log "GSP phase Running -> Suspending"), ring_refused (USER_RING..REFUS lines),
#   life_ended ("life ENDED" lines), host_vram_used_mib, host_vram_free_mib, host_ram_free_mib, host_ram_avail_mib,
#   guest_events (nvlddmkm / Display 4101 / Kernel-Power 41 / 141 since the monitor start; polled every 2 min, else last value).
# It only reads: journalctl, qemu.log, nvidia-smi, /proc, and one read-only PowerShell query through the kfagent channel.
W=/var/lib/kf-windows-20261005; R=$W/boundary-kayfabe-2990; STATE=$W/windows-broker-run2990.state
KFAGENT=${KFAGENT:-/data/kfagent_server.py}
QPID=$(sed -n 's/^QPID=//p' $STATE)
HDR="host_time,xid_all,xid_vm_pid,gsp_resets,ring_refused,life_ended,host_vram_used_mib,host_vram_free_mib,host_ram_free_mib,host_ram_avail_mib,guest_events"
GEV='$g=[datetime]::Parse("@G0@"); @(Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=$g} -ErrorAction SilentlyContinue | Where-Object { $_.ProviderName -eq "nvlddmkm" -or ($_.ProviderName -match "Display" -and $_.Id -eq 4101) -or $_.Id -in 41,141 }).Count'
guest_events() { # $1 = guest ISO time of the monitor start
  local q=${GEV//@G0@/$1}
  timeout 25 python3 $KFAGENT run "$q" 20 2>/dev/null | tr -d '\r' | tail -1 | grep -E '^[0-9]+$' || echo "?"
}
line() { # $1 t0 date string, $2 guest events
  local x xp gr rr le vm
  x=$(journalctl -k --since "$1" 2>/dev/null | grep -c "Xid")
  xp=$(journalctl -k --since "$1" 2>/dev/null | grep "Xid" | grep -c "pid=$QPID")
  gr=$(grep -c "GSP phase Running -> Suspending" $R/qemu.log)
  rr=$(grep -c "USER_RING.*REFUS\|REFUS.*USER_RING" $R/qemu.log)
  le=$(grep -c "life ENDED" $R/qemu.log)
  vm=$(nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader,nounits | tr -d ' ')
  echo "$(date +%T),$x,$xp,$gr,$rr,$le,$vm,$(awk '/MemFree/{f=$2}/MemAvailable/{a=$2}END{printf "%d,%d",f/1024,a/1024}' /proc/meminfo),$2"
}
case "$1" in
start)
  O=${2:?outdir}; P=${3:-30}; mkdir -p "$O"; rm -f "$O/monitor.stop"
  T0=$(date '+%Y-%m-%d %H:%M:%S')
  G0=$(timeout 25 python3 $KFAGENT run '(Get-Date).ToString("yyyy-MM-ddTHH:mm:ss")' 20 | tr -d '\r' | tail -1)
  echo "# monitor start host=$T0 guest=$G0 qemu_pid=$QPID period=${P}s" >> "$O/monitor.csv"; echo "$HDR" >> "$O/monitor.csv"
  last=? ; n=0
  while [ ! -e "$O/monitor.stop" ]; do
    if [ $((n % 4)) -eq 0 ]; then last=$(guest_events "$G0"); fi
    line "$T0" "$last" >> "$O/monitor.csv"; n=$((n + 1)); sleep "$P"
  done
  echo "# monitor stop $(date +%T)" >> "$O/monitor.csv" ;;
once)
  line "${2:-$(date -d '-1 hour' '+%Y-%m-%d %H:%M:%S')}" "${3:-?}" ;;
*) echo "usage: monitor.sh start <outdir> [period] | once [t0] [events]"; exit 2 ;;
esac
