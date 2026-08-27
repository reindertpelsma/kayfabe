#!/usr/bin/env bash
# mode2_iter.sh — one Mode-2 iteration ON THE HOST (vast.ai):
#   rebuild QEMU → restart VM from persistent overlay → run guest test →
#   print the BAR0 trace delta + the new dmesg.
#
# Assumes src files already synced to /opt/qemu-src/hw/misc and the deploy tree.
# Usage: bash scripts/mode2_iter.sh [--no-build] [--fresh]
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
BUILD=1; FRESH=0
for a in "$@"; do
    case "$a" in --no-build) BUILD=0;; --fresh) FRESH=1;; esac
done

if [ "$BUILD" = "1" ]; then
    echo "==> build QEMU"
    ( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) \
        | tail -2 || { echo "BUILD FAILED"; exit 1; }
fi

echo "==> restart VM"
# Flush guest writes before killing QEMU: kill -9 drops unflushed guest page
# cache, silently losing overlay changes (stash deletes, blacklist tweaks).
$SSHG 'sync; sync' 2>/dev/null || true
pkill -TERM qemu-system 2>/dev/null; sleep 3
pkill -9 qemu-system 2>/dev/null; sleep 2
NVKVM_FRESH=$FRESH nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh \
    >/tmp/m0_launch.log 2>&1 &
sleep 3
pgrep qemu-system >/dev/null || { echo "QEMU DIED:"; cat /tmp/m0_launch.log; exit 1; }

echo "==> wait for guest ssh"
up=0
for i in $(seq 1 40); do
    $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up after $((i*5))s"; break; }
    sleep 5
done
[ "$up" = "1" ] || { echo "GUEST DID NOT BOOT"; tail -5 /tmp/m0_launch.log; exit 1; }

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> push helpers + run guest test"
scp -q -P $PORT -o StrictHostKeyChecking=no \
    /workspace/nvkvm/scripts/mode2_guest_test.sh \
    /workspace/nvkvm/tests/mode2/nvopen.c \
    ubuntu@localhost:/tmp/ 2>/dev/null
# nvopen.c may live elsewhere; fall back to inline copy on host
$SSHG 'test -f /tmp/nvopen.c' 2>/dev/null || \
    scp -q -P $PORT -o StrictHostKeyChecking=no /tmp/nvopen.c ubuntu@localhost:/tmp/ 2>/dev/null
$SSHG 'bash /tmp/mode2_guest_test.sh' 2>&1

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo ""
echo "==> BAR0 trace delta (this iteration)"
sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -E 'nvkvm-gpu\[' > /tmp/m0_trace_delta.txt
echo "  total accesses: $(wc -l < /tmp/m0_trace_delta.txt)"
echo "  top offsets:"
grep -oE 'off=0x[0-9a-f]+' /tmp/m0_trace_delta.txt | sort | uniq -c | sort -rn | head -25
echo "  last 25 distinct-ish lines:"
tail -25 /tmp/m0_trace_delta.txt
