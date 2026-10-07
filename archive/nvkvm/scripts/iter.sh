#!/usr/bin/env bash
# iter.sh — fast nvkvm iteration: rsync → VM restart → build → run
#
# Designed for the vast.ai remote host. Restarts the VM cleanly (no rmmod -f
# kernel-corruption risk) and rebuilds anything the previous boot lost.
#
# Usage:
#   scripts/iter.sh             # restart + build module + run cuinit_test
#   scripts/iter.sh ioctl_fwd   # also runs test_ioctl_fwd
#   scripts/iter.sh no-restart  # skip restart (useful when only host-side QEMU changed)

set -euo pipefail

REMOTE_HOST="root@77.104.167.149"
REMOTE_PORT=44850
GUEST_PORT=2222
GUEST_USER=ubuntu

RUN_IOCTL_FWD=0
SKIP_RESTART=0
for arg in "$@"; do
    case "$arg" in
        ioctl_fwd) RUN_IOCTL_FWD=1 ;;
        no-restart) SKIP_RESTART=1 ;;
    esac
done

REPO="$(realpath "$(dirname "$0")/..")"

echo "==> Sync source to remote"
rsync -azq --delete -e "ssh -p $REMOTE_PORT" \
    "$REPO/src" "$REPO/tests" "$REPO/scripts" "$REMOTE_HOST:/workspace/nvkvm/"

if [ "$SKIP_RESTART" -eq 0 ]; then
    echo "==> Restart QEMU + boot VM"
    ssh -p "$REMOTE_PORT" "$REMOTE_HOST" '
        kill -9 $(pgrep qemu-system) 2>/dev/null
        sleep 2
        rm -f /tmp/qemu.log
        nohup bash /workspace/nvkvm/scripts/run_test_vm.sh > /tmp/qemu.log 2>&1 &
    '
    echo "==> Wait for guest SSH"
    until ssh -p "$REMOTE_PORT" "$REMOTE_HOST" \
        "ssh -p $GUEST_PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 $GUEST_USER@localhost echo VM_OK 2>/dev/null" \
        2>/dev/null | grep -q VM_OK; do
        sleep 4
    done
fi

echo "==> Mount 9p, rebuild module, insmod (idempotent), build tests"
ssh -p "$REMOTE_PORT" "$REMOTE_HOST" "ssh -p $GUEST_PORT -o StrictHostKeyChecking=no $GUEST_USER@localhost 'bash -s'" <<'GUEST'
set -e
sudo umount /mnt/nvkvm 2>/dev/null || true
sudo mkdir -p /mnt/nvkvm
sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576 nvkvm_src /mnt/nvkvm
cd /mnt/nvkvm/src/guest && sudo make -s >/dev/null
if ! lsmod | grep -q nvkvm_guest; then
    sudo insmod /mnt/nvkvm/src/guest/nvkvm-guest.ko
fi
mkdir -p /tmp/build/abi
cp /mnt/nvkvm/src/abi/*.h /tmp/build/abi/
[ -x /tmp/cuinit_test ] || gcc -O0 -g -Wall -ldl -o /tmp/cuinit_test /mnt/nvkvm/tests/integration/cuinit_test.c
[ -x /tmp/test_ioctl_fwd ] || gcc -O0 -g -Wall -I/tmp/build -o /tmp/test_ioctl_fwd /mnt/nvkvm/tests/integration/test_ioctl_fwd.c
echo "  module: $(lsmod | grep nvkvm_guest)"
GUEST

if [ "$RUN_IOCTL_FWD" -eq 1 ]; then
    echo "==> test_ioctl_fwd"
    ssh -p "$REMOTE_PORT" "$REMOTE_HOST" \
        "ssh -p $GUEST_PORT $GUEST_USER@localhost 'timeout 30 /tmp/test_ioctl_fwd 2>&1 | tail -2'"
fi

echo "==> cuinit_test"
ssh -p "$REMOTE_PORT" "$REMOTE_HOST" \
    "ssh -p $GUEST_PORT $GUEST_USER@localhost 'timeout 60 /tmp/cuinit_test 2>&1; echo EXIT=\$?'"
