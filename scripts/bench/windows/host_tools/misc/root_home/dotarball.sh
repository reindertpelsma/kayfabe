#!/usr/bin/env bash
exec > /root/tarball.log 2>&1
echo "START $(date -u +%FT%TZ)"
# Fully contained: nothing written to /opt, /usr/lib, or the shared guest dir.
export NVKVM_GUEST_DIR=/root/tb-test/guest
docker rm -f compose-test-nvkvm-1 >/dev/null 2>&1   # my earlier test; frees 2222
rm -rf /root/tb-test && mkdir -p /root/tb-test/guest && cd /root/tb-test

echo "=== PATH 2: prebuilt tarball, per install.md ==="
url=$(curl -sS https://api.github.com/repos/reindertpelsma/nvkvm-pv/releases/tags/v0.2.2 | grep -o 'https://[^"]*linux-x86_64.tar.gz' | head -1)
curl -sSL -o nvkvm.tar.gz "$url"
echo "sha256: $(sha256sum nvkvm.tar.gz | cut -d' ' -f1)"
tar xzf nvkvm.tar.gz
cd nvkvm-v0.2.2 || exit 1
echo "--- contents ---"; ls | head -10

echo "=== the binary the tarball ships ==="
./qemu-nvkvm/bin/qemu-system-x86_64 --version 2>&1 | head -2
echo "--- is it the patched build? ---"
./qemu-nvkvm/bin/qemu-system-x86_64 -device help 2>/dev/null | grep -c nvgpu
echo "--- stub present? ---"; ls -l src/stub/nvkvm_stub 2>&1 | head -1

echo "=== runtime deps per install.md ==="
DEBIAN_FRONTEND=noninteractive apt-get -qq install -y libglib2.0-0t64 libpixman-1-0 libslirp0 qemu-utils genisoimage sshpass >/dev/null 2>&1
echo "apt rc=$?"

echo "=== setup_guest.sh (contained guest dir) ==="
bash scripts/setup_guest.sh > /root/tb-setup.log 2>&1
echo "setup rc=$?  (tail:)"; tail -4 /root/tb-setup.log

echo "=== boot it, using the tarball's own qemu and stub ==="
export QEMU_BIN=/root/tb-test/nvkvm-v0.2.2/qemu-nvkvm/bin/qemu-system-x86_64
export NVKVM_STUB_PATH=/root/tb-test/nvkvm-v0.2.2/src/stub/nvkvm_stub
export VM_SSH_PORT=2299
export VM_IMG=/root/tb-test/guest/ubuntu-24.04.qcow2
export VM_SEED=/root/tb-test/guest/seed.iso
nohup setsid env NVKVM_DEV_HARNESS_INSECURE_RW=1 bash scripts/run_test_vm.sh > /root/tb-vm.log 2>&1 &
for i in $(seq 1 90); do
  sleep 10
  st=$(timeout 6 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o ConnectTimeout=5 -p 2299 ubuntu@127.0.0.1 \
       'echo "$(cloud-init status 2>&1|head -1)|$(command -v nvidia-smi||echo MISSING)"' 2>/dev/null | tail -1)
  [ -n "$st" ] && [ $((i % 3)) -eq 0 ] && echo "  $((i*10))s: $st"
  case "$st" in *nvidia-smi*) echo "GUEST READY at $((i*10))s"; break;; esac
done
echo "--- guest nvidia-smi + validate ---"
timeout 300 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -p 2299 ubuntu@127.0.0.1 \
  'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; bash /mnt/nvkvm/tests/validate.sh 2>&1 | tail -4' 2>&1 | grep -vE "Warning|Permanently" | tail -8
echo "END $(date -u +%FT%TZ)"
