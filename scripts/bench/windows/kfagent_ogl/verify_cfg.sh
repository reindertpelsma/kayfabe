#!/bin/bash
# verify_cfg.sh — after a VM 2990 relaunch with a new size config (launch-2990-solidworks.sh): ~5 min check.
# Host: the realize lines, the BAR sizes QEMU registered, 0 Xid; guest (kfagent): RAM, VRAM, the NVIDIA device's
# memory ranges (BAR0/BAR1/BAR2 as Windows assigned them), nvidia-smi, then a 60 s Heaven OpenGL render proof.
R=/var/lib/kf-windows-20261005/boundary-kayfabe-2990; K=${KFAGENT:-/data/kfagent_server.py}
ka() { timeout "${2:-60}" python3 "$K" run "$1" $((${2:-60} - 5)); }
QP=$(sed -n 's/^QPID=//p' /var/lib/kf-windows-20261005/windows-broker-run2990.state)
echo "== host: kf3 config"; grep -o "fb-mb=[0-9]*,bar1-size=[0-9]*" $R/command.json | head -1; grep -o "memory-backend-memfd,id=ram0,size=[0-9]*M" $R/command.json | head -1
grep -m3 "boot display ON\|BAR1 budget\|store=" $R/qemu.log | cut -c1-200
echo "== host: Xid for pid $QP: $(journalctl -k --since "$(date -d @$(stat -c %Y $R/command.json) +%T)" | grep -c "pid=$QP")  GSP resets: $(grep -c 'Running -> Suspending' $R/qemu.log)"
echo "== guest"
ka "'RAM_GB=' + [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1); & 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=name,memory.total,driver_version,pci.bus_id --format=csv,noheader; \$d=Get-CimInstance Win32_PnPEntity | Where-Object { \$_.Name -match 'NVIDIA' -and \$_.PNPClass -eq 'Display' }; \$d.Name; Get-CimInstance -Query (\"ASSOCIATORS OF {Win32_PnPEntity.DeviceID='\" + \$d.DeviceID.Replace('\\','\\\\') + \"'} WHERE ResultClass=Win32_DeviceMemoryAddress\") | ForEach-Object { '{0} .. {1}  ({2:N0} MiB)' -f \$_.StartingAddress.ToString('X'), \$_.EndingAddress.ToString('X'), ((\$_.EndingAddress - \$_.StartingAddress + 1)/1MB) }" 90
echo "== guest: Heaven OpenGL 60 s render proof"
ka "& C:\kf\ogl\runi.ps1 -Name verifyhv -Cmd \"powershell -ExecutionPolicy Bypass -File C:\kf\apps\heaven.ps1 -Api opengl -Fs 0 -Tag verify_heaven_gl\"" 50 >/dev/null
sleep 60; python3 /data/claude-shot.py && cp /tmp/claude-2990.png /data/claude-urelay/verify_heaven.png && echo "screenshot /data/claude-urelay/verify_heaven.png"
ka "Get-Process Heaven -ErrorAction SilentlyContinue | Stop-Process -Force; 'stopped'" 30
echo "== after: Xid $(journalctl -k --since "$(date -d @$(stat -c %Y $R/command.json) +%T)" | grep -c "pid=$QP") resets $(grep -c 'Running -> Suspending' $R/qemu.log); kfagent: $(ka hostname 20)"
