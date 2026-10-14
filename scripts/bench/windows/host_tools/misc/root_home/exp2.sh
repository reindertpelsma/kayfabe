#!/bin/bash
fbs() { for i in $(seq 1 6); do sudo grep -m1 "fb=" /sys/kernel/debug/dri/0/state 2>/dev/null | tr -d '\t '; sleep 0.7; done | tr '\n' ' '; echo; }
echo "== BEFORE fb ids =="; fbs
sudo steamos-readonly disable 2>/dev/null
sudo mkdir -p /etc/systemd/user/gamescope-session.service.d
sudo tee /etc/systemd/user/gamescope-session.service.d/10-nvkvm-composite.conf >/dev/null <<'CONF'
[Service]
Environment=GAMESCOPE_COMPOSITE_FORCE=1
CONF
echo "  drop-in written:"; cat /etc/systemd/user/gamescope-session.service.d/10-nvkvm-composite.conf | sed 's/^/    /'
sudo systemctl restart display-manager
echo "  display-manager restarted; waiting for the session"
for i in $(seq 1 45); do
  st=$(systemctl --user --machine=deck@ is-active gamescope-session.service 2>/dev/null)
  [ "$st" = active ] && break; sleep 2
done
echo "  gamescope-session: $st after ~$((i*2))s"
sleep 25
echo "== AFTER fb ids =="; fbs
echo "== env reached gamescope? =="
pid=$(pgrep -f "^gamescope " | head -1); echo "  pid=$pid"
tr '\0' '\n' < /proc/$pid/environ 2>/dev/null | grep -i COMPOSITE_FORCE || echo "  (COMPOSITE_FORCE not in gamescope env)"
echo "== import errors in the NEW boot of the session =="
journalctl -b _SYSTEMD_USER_UNIT=gamescope-session.service --no-pager --since "-3 min" 2>/dev/null | grep -c "Cannot import FB"
echo "== fps now =="
P=$(ls -d /run/user/1000/gamescope.*/ 2>/dev/null | head -1); echo "  stats: $P"; timeout 5 cat "$P/stats.pipe" 2>/dev/null | grep fps | head -4
