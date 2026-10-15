#!/bin/bash
# guirun.sh <tag> "<extra blender flags e.g. --gpu-backend vulkan>" "<gui_redraw.py args>" [max_wait_s=600]
# Test (c): starts a WINDOWED (1280x800 at 60,60) Blender GUI in kfadmin's interactive session via the one-shot scheduled-task
# pattern (C:\kf\ogl\runi.ps1), waits for its status in gui.json (the script quits Blender itself), kills only that pid on timeout.
TAG=$1; FL=$2; GA=$3; MAX=${4:-600}; U=/data/claude-blender/up; D='C:\kf\apps\blender\out\'$TAG
rm -f $U/${TAG}__*
K() { timeout "$2" python3 /data/kfagent_server.py run "$1" $(( $2 - 5 )); }
K "New-Item -ItemType Directory -Force $D | Out-Null; Remove-Item $D\\gui.json -ErrorAction SilentlyContinue; 'dir ok'" 30
echo "GUI_START $(date -Is) monitor: $(/data/claude-blender/wt/scripts/bench/blender/monitor.sh once)"
K "& C:\\kf\\ogl\\runi.ps1 -Name blgui -Cmd '\"C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe\" --factory-startup --window-geometry 60 60 1280 800 $FL -P C:\\kf\\apps\\blender\\gui_redraw.py -- $GA --out $D\\gui.json > $D\\stdout.txt 2>&1'" 40
t=0
while [ $t -lt $MAX ]; do
  r=$(K "if (Test-Path $D\\gui.json) { (Get-Content $D\\gui.json -Raw | ConvertFrom-Json).status } else { 'none' }" 25 | tr -d '\r' | tail -1)
  case "$r" in ok|error) break;; esac
  sleep 15; t=$((t+15))
done
echo "GUI_STATUS=$r after ${t}s  $(date -Is) monitor: $(/data/claude-blender/wt/scripts/bench/blender/monitor.sh once)"
K "Start-Sleep 5; \$p = Get-CimInstance Win32_Process -Filter \"Name='blender.exe'\" | Where-Object { \$_.CommandLine -match 'gui_redraw' }; if (\$p) { 'still running pid ' + \$p.ProcessId + ' -> killing own'; Stop-Process -Id \$p.ProcessId -Force }; foreach (\$n in 'gui.json','stdout.txt') { if (Test-Path $D\\\$n) { Invoke-RestMethod -Method Put -Uri http://10.0.2.2:8790/${TAG}__\$n -InFile $D\\\$n -UseBasicParsing | Out-Null } }; 'uploaded'" 60
cat $U/${TAG}__gui.json 2>/dev/null | head -60; grep -h "GUIBENCH" $U/${TAG}__stdout.txt | head
