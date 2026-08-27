#!/usr/bin/env bash
# cap1_capture_host.sh — RUNS ON THE BENCH HOST.  One complete `cap1` capture:
# fresh boot -> hermetic cold GSP bring-up -> nvidia-smi -q -> poweroff -> the
# trace file is flushed and complete.
#
# `cap1` is the ONLY closeable reference trace, so it must be reproducible as a
# single command: two captures taken weeks apart have to be the same experiment
# or the differential compares experiments instead of implementations.
#
#   OUT=/root/traces2/cap1x.rec NOTE="..." bash cap1_capture_host.sh
#
# Property vector is fixed by construction: m2fwd=off m2exec=off m2romregs=off,
# full mask, BAR0 trace on — exactly the original cap1 vector.
set -u

REPO="${NVKVM_REPO:-/workspace/nvkvm}"
OUT="${OUT:-/tmp/cap1.rec}"
PORT="${SSH_PORT:-2223}"
NOTE="${NOTE:-Capture 1: hermetic cold GSP bring-up to GSP_INIT_DONE; full mask; no host GPU involvement}"
GUEST_SH="$REPO/scripts/mode2_diag/cap1_coldboot_guest.sh"

OUT="$OUT" MODE=hermetic NOTE="$NOTE" bash "$REPO/scripts/mode2_diag/rec_capture.sh" || exit 1
bash "$REPO/scripts/mode2_diag/bench_wait.sh" || exit 1

scp -P "$PORT" -o BatchMode=yes "$GUEST_SH" ubuntu@localhost:/tmp/ >/dev/null || exit 1
ssh -p "$PORT" -o BatchMode=yes ubuntu@localhost "bash /tmp/$(basename "$GUEST_SH")" 2>&1 | tail -25

# The trace is only COMPLETE once QEMU exits and its exit notifier flushes the
# staging buffer + patches the header counters.  Wait for that, do not read early.
echo "--- waiting for QEMU to exit (poweroff) ---"
for _ in $(seq 60); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 2; done
if pgrep -x qemu-system-x86 >/dev/null; then
    echo "WARNING: qemu still alive; killing (trace still flushed by the notifier)" >&2
    pkill -9 -f "qemu-system-x86_6[4]"
    sleep 3
fi
ls -la "$OUT"
python3 "$REPO/scripts/mode2_diag/rec_dump.py" "$OUT" | tail -14
