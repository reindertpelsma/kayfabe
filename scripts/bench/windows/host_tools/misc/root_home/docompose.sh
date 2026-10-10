#!/usr/bin/env bash
exec > /root/compose.log 2>&1
echo "START $(date -u +%FT%TZ)"
rm -rf /root/compose-test && mkdir -p /root/compose-test && cd /root/compose-test
git clone -q --depth 1 https://github.com/reindertpelsma/nvkvm-pv.git . 2>&1 | tail -2
echo "head: $(git log --oneline -1)"
echo "compose pins: $(grep -m1 'image:' docker-compose.yml)"
echo
echo "=== A) docker compose as shipped (pins v0.2.0 -- the BROKEN bind) ==="
docker compose down -v >/dev/null 2>&1
timeout 600 docker compose pull 2>&1 | tail -2
docker compose up -d 2>&1 | tail -3
echo "waiting 4 min, then trying the documented ssh line"
sleep 240
echo "--- ssh -p 2222 ubuntu@127.0.0.1 (as docker-compose.yml's header instructs) ---"
timeout 15 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=8 -p 2222 ubuntu@127.0.0.1 'echo REACHED_GUEST' 2>&1 | tail -2
docker compose down -v >/dev/null 2>&1
echo
echo "=== B) same compose, pin corrected to v0.2.2 ==="
sed -i 's|nvkvm-pv:v0.2.0|nvkvm-pv:v0.2.2|' docker-compose.yml
timeout 600 docker compose pull 2>&1 | tail -2
docker compose up -d 2>&1 | tail -2
for i in $(seq 1 60); do
  sleep 10
  st=$(timeout 8 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o StrictHostKeyChecking=no -o ConnectTimeout=6 -p 2222 ubuntu@127.0.0.1 \
       'echo "$(cloud-init status 2>&1|head -1)|$(command -v nvidia-smi||echo MISSING)"' 2>/dev/null | tail -1)
  [ -n "$st" ] && [ $((i % 3)) -eq 0 ] && echo "  $((i*10))s: $st"
  case "$st" in *nvidia-smi*) echo "READY at $((i*10))s"; break;; esac
done
echo "--- guest nvidia-smi ---"
timeout 20 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o StrictHostKeyChecking=no -p 2222 ubuntu@127.0.0.1 \
  'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader' 2>&1 | tail -2
echo "END $(date -u +%FT%TZ)"
