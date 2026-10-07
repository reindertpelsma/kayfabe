#!/usr/bin/env bash
# persist_execd.sh — make a vx box's execd and its cloudflared tunnel survive a reboot.
# Run ON THE BOX, through vx (scripts/bench/box/README.md, "Driving a box from a cloud session"):
#   vput scripts/bench/box/persist_execd.sh /root/persist_execd.sh && vx 'bash /root/persist_execd.sh'
# Last line: PERSIST_EXECD kf-execd=<state> kf-tunnel=<state> enabled=<a>/<b> url=<tunnel URL>.
#
# ★ Why [observed 2026-09-27]: the box's onstart (written by `vast-up`) restarts only execd after
# a reboot, not cloudflared — so a rebooted box had no tunnel, printed no EXECD_URL on its serial
# console, and was unreachable from a cloud session (no SSH egress there). Two systemd units fix
# that: kf-execd (execd on 127.0.0.1:8765) and kf-tunnel (/root/cf-url.sh: a quick tunnel, its URL
# written to /dev/ttyS0 every 30 s, where `vast-url <id>` reads it). A new tunnel has a NEW URL.
#
# ⚠ Idempotent, and it never restarts a unit that is already active: the vx request running this
# script travels through execd and possibly through kf-tunnel. A changed file is picked up at that
# unit's next restart or boot.
# ⊘ No secret passes through here: the PSK stays where the onstart put it (/root/.execd_key), and
# this script only checks that it exists.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "persist_execd: run as root" >&2; exit 1; }
for f in /root/execd.py /root/.execd_key /usr/local/bin/cloudflared; do
  [ -s "$f" ] || { echo "persist_execd: $f is missing -- not a vx box (vast-up's onstart writes it)" >&2; exit 1; }
done

changed=""
put() {  # put PATH MODE < content — write only when the content differs
  local tmp; tmp=$(mktemp)
  cat > "$tmp"
  if cmp -s "$tmp" "$1"; then rm -f "$tmp"; return 0; fi
  install -m "$2" "$tmp" "$1"; rm -f "$tmp"; changed="$changed $1"
}

# ⚠ Yield the port instead of crash-looping on it: if something already serves 127.0.0.1:8765 (the
# onstart's own execd loop, which it restarts at boot), wait, and take over only when it is gone.
# During the first 5 min of uptime, give that loop time to bind first.
put /root/kf-execd.sh 755 <<'EOF'
#!/bin/bash
# kf-execd (systemd unit; installed by scripts/bench/box/persist_execd.sh)
served() { ss -Hltn 'sport = :8765' | grep -q .; }
if [ "$(cut -d. -f1 /proc/uptime)" -lt 300 ]; then
  for _ in $(seq 36); do served && break; sleep 5; done
fi
while served; do sleep 30; done
exec /usr/bin/python3 /root/execd.py >>/root/execd.log 2>&1
EOF

# ⚠ Its own log and URL file: the onstart's tunnel (if any is still up) keeps /root/cf.log and
# /root/execd.url. `api.trycloudflare.com` (the endpoint cloudflared asks for a tunnel, which its
# error lines can name) is never the tunnel's URL.
put /root/cf-url.sh 755 <<'EOF'
#!/bin/bash
# kf-tunnel (systemd unit; installed by scripts/bench/box/persist_execd.sh)
log=/root/kf-tunnel.log; url=/root/kf-tunnel.url
rm -f "$url"; : > "$log"
/usr/local/bin/cloudflared tunnel --no-autoupdate --url http://127.0.0.1:8765 >>"$log" 2>&1 &
cf=$!
u=""
for _ in $(seq 90); do
  u=$(grep -ao 'https://[a-z0-9-]*\.trycloudflare\.com' "$log" | grep -v '^https://api\.' | head -1)
  [ -n "$u" ] && break
  kill -0 "$cf" 2>/dev/null || exit 1
  sleep 2
done
[ -n "$u" ] || { kill "$cf"; exit 1; }
echo "$u" > "$url"
while kill -0 "$cf" 2>/dev/null; do echo "EXECD_URL=$u" > /dev/ttyS0; sleep 30; done
exit 1   # cloudflared died: systemd restarts the unit, with a new URL
EOF

# ⊘⊘ KillMode=process on kf-execd: every `vx -b` job is a child of execd and lives in this unit's
# cgroup (setsid does not leave a cgroup). With the default control-group mode, an execd restart
# would kill a running provision_full.sh or merge_check.sh along with it.
put /etc/systemd/system/kf-execd.service 644 <<'EOF'
[Unit]
Description=kayfabe vx: execd (PSK-signed exec and file endpoint on 127.0.0.1:8765)
After=network.target

[Service]
ExecStart=/root/kf-execd.sh
WorkingDirectory=/root
Environment=HOME=/root
Restart=always
RestartSec=5
KillMode=process

[Install]
WantedBy=multi-user.target
EOF

put /etc/systemd/system/kf-tunnel.service 644 <<'EOF'
[Unit]
Description=kayfabe vx: cloudflared quick tunnel to execd, URL on the serial console
Wants=network-online.target
After=network-online.target kf-execd.service

[Service]
ExecStart=/root/cf-url.sh
WorkingDirectory=/root
Restart=always
RestartSec=30

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
for u in kf-execd kf-tunnel; do
  systemctl enable -q "$u.service"
  if systemctl is-active -q "$u.service"; then
    echo "$u: active, left running"
  else
    systemctl start "$u.service"; echo "$u: started"
  fi
done
if [ -n "$changed" ]; then
  echo "changed:$changed (an active unit picks this up at its next restart or boot)"
fi
for _ in $(seq 30); do [ -s /root/kf-tunnel.url ] && break; sleep 2; done
echo "PERSIST_EXECD kf-execd=$(systemctl is-active kf-execd) kf-tunnel=$(systemctl is-active kf-tunnel)" \
     "enabled=$(systemctl is-enabled kf-execd)/$(systemctl is-enabled kf-tunnel)" \
     "url=$(cat /root/kf-tunnel.url 2>/dev/null || echo none)"
