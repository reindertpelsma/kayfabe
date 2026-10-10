#!/usr/bin/env bash
exec > /root/tarball2.log 2>&1
echo "START $(date -u +%FT%TZ)"
cd /root/tb-test/nvkvm-v0.2.2 || exit 1
# stop the previous test VM (match on comm, never on our own cmdline)
pids=$(ps -eo pid,comm | awk '$2 ~ /^qemu-system/ {print $1}')
for p in $pids; do
  tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null | grep -q 'hostfwd=tcp:127.0.0.1:2299' && kill $p 2>/dev/null && echo "stopped previous test VM pid=$p"
done
sleep 3

echo "=== THE MISSING STEP: make_host_bundle.sh on the host ==="
bash scripts/make_host_bundle.sh 2>&1 | tail -4
B=$(ls -d /root/tb-test/nvkvm-v0.2.2/host-libs-* 2>/dev/null | head -1)
echo "bundle: ${B:-NONE}"
[ -z "$B" ] && { echo "no bundle produced"; echo "END $(date -u +%FT%TZ)"; exit 1; }

echo "=== relaunch with NVKVM_HOSTLIBS_DIR set ==="
export QEMU_BIN=/root/tb-test/nvkvm-v0.2.2/qemu-nvkvm/bin/qemu-system-x86_64
export NVKVM_STUB_PATH=/root/tb-test/nvkvm-v0.2.2/src/stub/nvkvm_stub
export NVKVM_GUEST_DIR=/root/tb-test/guest
export VM_IMG=/root/tb-test/guest/ubuntu-24.04.qcow2
export VM_SEED=/root/tb-test/guest/seed.iso
export VM_SSH_PORT=2299
export NVKVM_HOSTLIBS_DIR="$B"
nohup setsid env NVKVM_DEV_HARNESS_INSECURE_RW=1 bash scripts/run_test_vm.sh > /root/tb-vm2.log 2>&1 &
S="sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o ConnectTimeout=6 -p 2299 ubuntu@127.0.0.1"
for i in $(seq 1 60); do
  sleep 10
  st=$(timeout 8 $S 'cloud-init status 2>&1|head -1' 2>/dev/null | tail -1)
  case "$st" in *done*) echo "guest up at $((i*10))s"; break;; esac
done
echo "--- is the libs share visible in the guest? ---"
timeout 20 $S 'mount | grep -c nvkvm_libs; ls /opt/nvidia-host 2>/dev/null | head -3' 2>&1 | tail -4

echo "=== THE OTHER MISSING STEP: stage_guest_libs.sh inside the guest ==="
timeout 300 $S 'sudo bash /mnt/nvkvm/scripts/stage_guest_libs.sh 2>&1 | tail -6' 2>&1 | tail -8

echo "=== now: does the guest see the GPU? ==="
timeout 60 $S 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1 | head -2' 2>&1 | tail -2
timeout 400 $S 'bash /mnt/nvkvm/tests/validate.sh 2>&1 | tail -4' 2>&1 | tail -5
echo "END $(date -u +%FT%TZ)"
