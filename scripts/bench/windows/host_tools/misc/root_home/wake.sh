#!/bin/bash
echo "== tools =="; for t in xdotool wtype evemu-event; do command -v $t >/dev/null && echo "  have $t" || echo "  no $t"; done
echo "== gamescope CPU before =="; top -b -n1 2>/dev/null | grep -E "gamescope" | head -3
echo "== fb before =="; for i in 1 2 3; do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state; sleep 1; done
echo "== INJECT INPUT =="
if command -v xdotool >/dev/null; then
  for i in 1 2 3 4 5; do sudo -u deck env DISPLAY=:0 xdotool mousemove $((200+i*50)) $((200+i*30)) 2>&1; sleep 0.3; done
  sudo -u deck env DISPLAY=:0 xdotool key Escape 2>&1
  echo "  injected via xdotool on :0"
else
  echo "  no xdotool; trying uinput"
fi
sleep 3
echo "== fb after =="; for i in 1 2 3; do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state; sleep 1; done
echo "== gamescope CPU after =="; top -b -n1 2>/dev/null | grep -E "gamescope" | head -3
echo "== any NEW gamescope log lines? =="
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager 2>/dev/null | tail -4
