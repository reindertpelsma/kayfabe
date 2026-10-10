#!/bin/bash
cd /opt/nvkvm-steamos-latest
S=./steamos-ssh
echo "=== terminate the wine/proton chain ==="
$S 'pkill -TERM -f wineserver; pkill -TERM -f PlayRDR2; pkill -TERM -f pv-adverb; pkill -TERM -f "proton waitforexitandrun"' 2>&1 | tail -2
sleep 6
$S 'pkill -KILL -f RDR2.exe; pkill -KILL -f wineserver; pkill -KILL -f Launcher.exe' 2>&1 | tail -2
sleep 3
echo "=== survivors ==="
$S 'ps -eo pid,comm --no-headers | grep -iE "RDR2|wine|proton|pv-adverb|Launcher" | grep -v oom_reaper | head -6; echo "--eol--"' 2>&1 | tail -8
echo "=== GPU now ==="
nvidia-smi --query-gpu=utilization.gpu,memory.used,power.draw --format=csv,noheader
echo "=== compositor state ==="
$S 'pgrep -a kwin_wayland | head -2; pgrep -a gamescope | head -2; echo "--eol--"' 2>&1 | tail -5
