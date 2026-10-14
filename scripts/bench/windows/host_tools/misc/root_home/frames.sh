#!/bin/bash
P=$(ls -d /run/user/1000/gamescope.*/ 2>/dev/null | head -1)
echo "  stats dir: $P"
echo "== gamescope process CPU (ps, 2 samples 3s apart) =="
ps -o pid,etimes,time,pcpu,comm -C gamescope 2>/dev/null
sleep 3
ps -o pid,etimes,time,pcpu,comm -C gamescope 2>/dev/null
echo "== reading stats.pipe for 5s (frames produced => output) =="
timeout 5 cat "$P/stats.pipe" 2>/dev/null | head -12
echo "  (empty above = gamescope is producing NO frames)"
echo "== does the CRTC report vblank activity? =="
sudo cat /sys/kernel/debug/dri/0/crtc-0/vblank_count 2>/dev/null || echo "  (no vblank_count node)"
echo "== steamcompmgr / xwayland alive? =="
pgrep -a steamcompmgr | head -2
