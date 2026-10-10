#!/bin/bash
set -u
exec >/root/gs-test/black-fix.log 2>&1
say(){ echo "[$(date +%H:%M:%S)] $*"; }
cd /root/gs-test/repo
git checkout -q main 2>/dev/null || git checkout -q -B main origin/main
git reset --hard origin/main -q
git checkout -q -B fix-black origin/main
git am /root/gs-test/gs_black.patch || { say "PATCH FAILED"; exit 1; }
say "on $(git log --oneline -1)"
say "restarting stack WITH NVKVM_PRESENT_DUMP"
docker compose down --remove-orphans >/dev/null 2>&1
export NVKVM_PRESENT_DUMP=/var/lib/nvkvm-steamos/frame.ppm
rm -f /var/lib/docker/volumes/nvkvm-steamos-state/_data/frame.ppm
docker compose up -d --build >/dev/null 2>&1 || { say "UP FAILED"; exit 1; }
say "up; waiting for guest ssh"
for i in $(seq 1 90); do ./steamos-ssh true >/dev/null 2>&1 && break; sleep 10; done
say "guest ssh up"
say "waiting for gamescope-session active"
for i in $(seq 1 60); do
  st=$(./steamos-ssh 'systemctl --user --machine=deck@ is-active gamescope-session.service' 2>/dev/null | tr -d '\r\n ')
  [ "$st" = active ] && break; sleep 5
done
say "gamescope-session: $st"
say "did convergence install the drop-in?"
./steamos-ssh 'cat /etc/systemd/user/gamescope-session.service.d/10-nvkvm-composite.conf 2>/dev/null; echo "---"; pid=$(pgrep -f "^gamescope " | head -1); tr "\0" "\n" < /proc/$pid/environ 2>/dev/null | grep COMPOSITE_FORCE || echo "NOT IN ENV"' 2>&1
say "convergence log line:"
grep -i "direct scan-out" /var/lib/docker/volumes/nvkvm-steamos-state/_data/serial.log 2>/dev/null | tail -2
sleep 30
say "PPM dump present?"
ls -la /var/lib/docker/volumes/nvkvm-steamos-state/_data/frame.ppm 2>/dev/null || say "NO DUMP FILE"
say "DONE"
