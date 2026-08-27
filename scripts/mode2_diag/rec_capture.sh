#!/usr/bin/env bash
# rec_capture.sh — RUNS ON THE BENCH HOST.  Take one §6 replay-trace capture.
#
# Same safe-restart discipline as bench_boot.sh (whose two traps are reproduced
# below, because getting them wrong silently produces a vacuous run), but with
# FULL control of the m2* property vector — bench_boot.sh forces m2cefwd=on,
# which is wrong for a hermetic capture.
#
#   TRAP 1  `pgrep -x qemu-system-x86_64` can NEVER match: /proc/PID/comm is
#           truncated to 15 chars.  Verify with `pgrep -x qemu-system-x86` AND
#           with `ss -tln | grep 2223`, which does not lie.
#   TRAP 2  `pkill -f qemu-system-x86_64` matches its own command line.  Use the
#           bracket form `qemu-system-x86_6[4]`.
#
# Every clean run needs a FULL QEMU restart (the emulated GSP's WPR2 state only
# resets on a fresh process) and GPU work is STRICTLY SERIAL.
#
# Usage:
#   OUT=/root/traces/cap1.rec MODE=hermetic bash rec_capture.sh
#   OUT=... MODE=forwarding NOTE="cuCtxCreate + 2048^2 matmul" bash rec_capture.sh
#
# MODE:
#   hermetic    m2fwd=off m2exec=off m2romregs=off  — the ONLY closeable trace
#   forwarding  m2fwd=on  m2exec=on                 — NON-HERMETIC (host GPU
#                                                     DMAs guest RAM behind us)
# MASK: the declared filter (NVKVM_REC_M_* from nvkvm_m2_rec.h).  Default all.
set -u

REPO="${NVKVM_REPO:-/workspace/nvkvm}"
OVL="${NVKVM_OVERLAY:-/opt/nvkvm-guest/mode2-overlay.qcow2}"
PORT="${SSH_PORT:-2223}"
OUT="${OUT:-/tmp/m0_rec.bin}"
MODE="${MODE:-hermetic}"
MASK="${MASK:-18446744073709551615}"     # NVKVM_REC_M_ALL
FRESH="${FRESH:-1}"

pkill -9 -f "qemu-system-x86_6[4]" 2>/dev/null || true
for _ in $(seq 30); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 1; done
if pgrep -x qemu-system-x86 >/dev/null; then
    echo "FATAL: qemu still alive after pkill:" >&2; pgrep -a -x qemu-system-x86 >&2; exit 1
fi
for _ in $(seq 30); do ss -tln | grep -q ":${PORT} " || break; sleep 1; done
if ss -tln | grep -q ":${PORT} "; then
    echo "FATAL: port ${PORT} still bound; a new QEMU would die on hostfwd" >&2; exit 1
fi

[ "$FRESH" = "1" ] && rm -f "$OVL"
rm -f "$OUT"
mkdir -p "$(dirname "$OUT")"

# Clear every m2* knob, then set exactly the ones this mode wants.  Explicit
# rather than inherited: an unnoticed knob is a trace whose header lies.
unset NVKVM_M2FWD NVKVM_M2FWD_OFF NVKVM_M2EXEC NVKVM_M2EXEC_OFF NVKVM_M2RING \
      NVKVM_M2HOSTSEM NVKVM_M2CEFWD NVKVM_M2CEXEC NVKVM_M2OPAQUE NVKVM_M2TRACE \
      NVKVM_M2ROMREGS NVKVM_M2SEMVAL NVKVM_M2SEMPAGE
case "$MODE" in
  hermetic)
    export NVKVM_M2FWD_OFF=1 NVKVM_M2EXEC_OFF=1 ;;
  forwarding)
    export NVKVM_M2FWD=1 NVKVM_M2EXEC=1 ;;
  *) echo "FATAL: unknown MODE=$MODE (hermetic|forwarding)" >&2; exit 1 ;;
esac
export NVKVM_M2REC=1 NVKVM_M2RECFILE="$OUT" NVKVM_M2RECMASK="$MASK"
export NVKVM_M2REC_NOTE="${NOTE:-} [MODE=$MODE]"

cd "$REPO" || exit 1
nohup setsid bash scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 </dev/null &
disown
echo "launched MODE=$MODE OUT=$OUT MASK=$MASK"
echo "  launch log /tmp/m0_launch.log, serial /tmp/m0_serial.log, qemu /tmp/m0_qemu.log"
echo "  now: bash scripts/mode2_diag/bench_wait.sh   (guest needs ~20-25 s; the"
echo "       serial log LAGS — do not call a slow boot a crash)"
echo "  the trace file is only COMPLETE after QEMU exits or is killed (the exit"
echo "  notifier flushes it); a live file is a usable dense PREFIX."
