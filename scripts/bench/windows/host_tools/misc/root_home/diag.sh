#!/bin/bash
echo "== DRM state now =="
sudo cat /sys/kernel/debug/dri/0/state 2>/dev/null | grep -E "^(plane|crtc|connector)\[|	(fb|active|crtc-pos|crtc)=" | head -20
echo "== is the scanout fb CHANGING? (3 samples, 2s apart) =="
for i in 1 2 3; do
  echo -n "  sample $i: "
  sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null || echo "(no fb line)"
  sleep 2
done
echo "== steam power / blanking settings =="
grep -riE "ScreenSaver|Blank|Dim|Idle" /home/deck/.local/share/Steam/config/*.vdf 2>/dev/null | head -5
echo "== what does Steam think about the display =="
sudo -u deck env DISPLAY=:0 xset q 2>/dev/null | grep -iA2 "screen saver"
echo "== gamescope stats socket =="
ls -la /run/user/1000/gamescope-stats 2>/dev/null
echo "== recent gamescope log (last 15, any level) =="
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager 2>/dev/null | tail -15
