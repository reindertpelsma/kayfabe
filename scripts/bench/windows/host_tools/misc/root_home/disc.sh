#!/bin/bash
fbs() { for i in $(seq 1 8); do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null | tr -d '\t '; sleep 0.5; done | tr '\n' ' '; echo; }
echo "== env of the RUNNING gamescope =="
pid=$(pgrep -f "^gamescope " | head -1)
tr '\0' '\n' < /proc/$pid/environ 2>/dev/null | grep -i COMPOSITE_FORCE || echo "  COMPOSITE_FORCE absent!"
echo "== idle fb ids =="; fbs
echo "== inject input =="
for i in 1 2 3 4 5 6; do sudo -u deck env DISPLAY=:0 xdotool mousemove $((300+i*60)) $((250+i*40)) 2>/dev/null; sleep 0.25; done
echo "== fb ids DURING/AFTER input =="; fbs
echo "== import errors total / in last 5 min =="
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager 2>/dev/null | grep -c "Cannot import FB"
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager --since "-5 min" 2>/dev/null | grep -c "Cannot import FB"
echo "== fps =="
P=$(ls -d /run/user/1000/gamescope.*/ 2>/dev/null | head -1); timeout 4 cat "$P/stats.pipe" 2>/dev/null | grep fps | head -3
