#!/usr/bin/env bash
exec > /root/ctr.log 2>&1
echo "START $(date -u +%FT%TZ)"
rm -rf /root/br && mkdir -p /root/br && cd /root/br
git clone -q --depth 1 --branch fix/post-release-followups https://github.com/reindertpelsma/nvkvm-pv.git . 2>&1 | tail -1
echo "branch: $(git log --oneline -1)"
echo "=== build the image from the branch ==="
t0=$(date +%s); docker build -q -t nvkvm-branch:test . 2>&1 | tail -2; echo "build took $(( $(date +%s)-t0 ))s"

echo "=== quickstart, README verbatim, branch image ==="
docker rm -f brtest >/dev/null 2>&1; docker volume rm brvol >/dev/null 2>&1
docker run -d --name brtest --device /dev/kvm --gpus all \
  -e NVIDIA_DRIVER_CAPABILITIES=compute,utility,graphics,display,video \
  -p 127.0.0.1:2244:2222 -v brvol:/opt/nvkvm-guest nvkvm-branch:test >/dev/null
S="sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o ConnectTimeout=6 -p 2244 ubuntu@127.0.0.1"
for i in $(seq 1 90); do
  sleep 10
  st=$(timeout 8 $S 'echo "$(cloud-init status 2>&1|head -1)|$(command -v nvidia-smi||echo MISSING)"' 2>/dev/null | tail -1)
  [ -n "$st" ] && [ $((i % 6)) -eq 0 ] && echo "  $((i*10))s: $st"
  case "$st" in *nvidia-smi*) echo "READY at $((i*10))s"; break;; esac
done
echo "=== guest ==="
timeout 60 $S 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader' 2>&1 | grep -v Warning | tail -2
timeout 500 $S 'bash /mnt/nvkvm/tests/validate.sh 2>&1 | tail -5' 2>&1 | grep -v Warning | tail -6

echo "=== NEW LOGIC: does a driver-version change force a rebundle on restart? ==="
echo "--- bundle now ---"; docker exec brtest sh -c 'ls -d /opt/nvkvm/host-libs-* 2>/dev/null'
docker exec brtest sh -c 'mv /opt/nvkvm/host-libs-* /opt/nvkvm/host-libs-1.2.3 2>/dev/null; ls -d /opt/nvkvm/host-libs-*'
echo "--- restart (writable layer survives) ---"
docker restart brtest >/dev/null 2>&1; sleep 25
docker logs --tail 30 brtest 2>&1 | grep -iE "discarding stale|assembling the host driver" | head -3
echo "--- bundle after restart ---"; docker exec brtest sh -c 'ls -d /opt/nvkvm/host-libs-* 2>/dev/null'
echo "END $(date -u +%FT%TZ)"
