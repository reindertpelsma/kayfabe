#!/bin/bash
# A/B the TimeoutStartSec fix by making gamescope deliberately slow to report ready.
set -u
exec >/root/gs-test/timeout-ab.log 2>&1
say(){ echo "[$(date +%H:%M:%S)] $*"; }
cd /root/gs-test/repo
G(){ timeout 60 ./steamos-ssh "$1" 2>&1; }

say "guest unit as it stands:"
G 'systemctl --user --machine=deck@ show gamescope-session.service -p TimeoutStartUSec -p Type; grep -n "read -r -t" /usr/lib/steamos/gamescope-session'

say "injecting a deliberate 15s delay at the top of the session script"
G 'set -e
   sudo steamos-readonly disable 2>/dev/null || true
   S=/usr/lib/steamos/gamescope-session
   sudo cp -n $S $S.ab-orig
   sudo grep -q "NVKVM-AB-DELAY" $S || sudo sed -i "2i sleep 15  # NVKVM-AB-DELAY" $S
   sed -n "1,3p" $S'

run_phase() {
  say "=== PHASE $1 : $2 ==="
  G "$3"
  G 'sudo systemctl restart display-manager' >/dev/null
  local st=""
  for i in $(seq 1 24); do
    st=$(G 'systemctl --user --machine=deck@ is-active gamescope-session.service' | tr -d "\r\n ")
    [ "$st" = active ] && break
    sleep 5
  done
  say "  gamescope-session after ~$((i*5))s: ${st:-unknown}"
  say "  journal evidence:"
  G 'journalctl -b --no-pager --since "-3 min" | grep -iE "gamescope-session.service: (start operation|State)|dependency job for gamescope-session" | tail -4'
  say "  effective deadline: $(G 'systemctl --user --machine=deck@ show gamescope-session.service -p TimeoutStartUSec' | tr -d "\r\n")"
}

run_phase A "CONTROL - no drop-in, stock TimeoutStartSec=5" \
  'sudo rm -f /etc/systemd/user/gamescope-session.service.d/10-nvkvm-start-timeout.conf; sudo systemctl daemon-reload'

run_phase B "FIX - TimeoutStartSec=120 drop-in" \
  'sudo mkdir -p /etc/systemd/user/gamescope-session.service.d
   printf "[Service]\nTimeoutStartSec=120\n" | sudo tee /etc/systemd/user/gamescope-session.service.d/10-nvkvm-start-timeout.conf >/dev/null
   sudo systemctl daemon-reload'

say "=== restoring the guest (removing the injected delay) ==="
G 'S=/usr/lib/steamos/gamescope-session; sudo sed -i "/NVKVM-AB-DELAY/d" $S; sudo rm -f $S.ab-orig; grep -c NVKVM-AB-DELAY $S || true'
G 'sudo systemctl restart display-manager' >/dev/null
sleep 20
say "final: $(G 'systemctl --user --machine=deck@ is-active gamescope-session.service' | tr -d "\r\n")"
say "DONE"
