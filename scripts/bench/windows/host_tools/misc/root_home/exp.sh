#!/bin/bash
fbs() { for i in $(seq 1 6); do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null | tr -d '\t '; sleep 0.7; done | tr '\n' ' '; echo; }
echo "== BEFORE: fb ids over ~4s =="; fbs
echo "== BEFORE: import errors =="; journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager 2>/dev/null | grep -c "Cannot import FB"
echo
echo "== applying GAMESCOPE_COMPOSITE_FORCE=1 and restarting the session =="
systemctl --user --machine=deck@ set-environment GAMESCOPE_COMPOSITE_FORCE=1 2>&1
systemctl --user --machine=deck@ restart gamescope-session.service 2>&1
for i in $(seq 1 30); do
  st=$(systemctl --user --machine=deck@ is-active gamescope-session.service 2>/dev/null)
  [ "$st" = active ] && break; sleep 2
done
echo "  session: $st after $((i*2))s"
sleep 20
echo "== AFTER: fb ids over ~4s =="; fbs
echo "== AFTER: env actually seen by gamescope? =="
pid=$(pgrep -x gamescope | head -1); tr '\0' '\n' < /proc/$pid/environ 2>/dev/null | grep -i COMPOSITE || echo "  (not in env!)"
echo "== AFTER: new import errors since restart =="
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager --since "-2 min" 2>/dev/null | grep -c "Cannot import FB"
echo "== AFTER: fps =="
P=$(ls -d /run/user/1000/gamescope.*/ 2>/dev/null | head -1); timeout 4 cat "$P/stats.pipe" 2>/dev/null | grep fps | head -3
