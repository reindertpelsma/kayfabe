#!/bin/bash
# usage: reboot_guest.sh <work>   in-guest reboot (needed for dxgkrnl to read TdrDelay); waits until ssh answers with a new boot time
cd /workspace/bench/pcnight4_20261115; export KF_WORK=$1
b0=$(python3 -B -I gx2.py ps '(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")' 30 | tr -d '\r\n ')
python3 -B -I gx2.py ps 'shutdown /r /t 3' 30 >/dev/null 2>&1
sleep 20
for i in $(seq 1 60); do
  b1=$(python3 -B -I gx2.py ps '(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")' 15 2>/dev/null | tr -d '\r\n ')
  [ -n "$b1" ] && [ "$b1" != "$b0" ] && { echo "rebooted $b0 -> $b1"; sleep 25; exit 0; }
  sleep 4
done
echo "REBOOT FAILED"; exit 1
