#!/usr/bin/env bash
# bench_boot.sh — RUNS ON THE BENCH HOST. Safe fresh Mode-2 boot.
#
# Every clean Mode-2 run needs a FULL QEMU restart (the emulated GSP's WPR2
# state only resets on a fresh process).  Doing that by hand has burned whole
# cycles on two traps, both encoded here:
#
#   TRAP 1 — the verify step lies.  `pgrep -x qemu-system-x86_64` and
#     `pgrep -a qemu-system-x86_64` match against /proc/PID/comm, which the
#     kernel TRUNCATES to 15 chars ("qemu-system-x86").  The _64 form can
#     therefore NEVER match, so it reports "no QEMU running" while a QEMU is
#     very much running.  You then launch a second one, it loses the race for
#     port 2223, and dies with
#         Could not set up host forwarding rule 'tcp::2223-:22'
#     Verify with `pgrep -x qemu-system-x86` (comm) — that one works.
#
#   TRAP 2 — pkill matching its own command line.  A plain
#     `pkill -9 -f qemu-system-x86_64` matches the pkill invocation itself and
#     any shell whose command line quotes that string, killing your session.
#     Use the bracket form `qemu-system-x86_6[4]`, which does not match its own
#     literal text.
#
# Kill and launch are separated by a real verified wait, and the launch is
# properly detached (nohup setsid ... </dev/null & disown) so it survives the
# ssh session that started it.
#
# Env: NVKVM_M2CEFWD (default 1) and any other NVKVM_M2* run_mode2_vm.sh knobs
# are passed through; NVKVM_REPO (default /workspace/nvkvm).
set -u

REPO="${NVKVM_REPO:-/workspace/nvkvm}"
OVL="${NVKVM_OVERLAY:-/opt/nvkvm-guest/mode2-overlay.qcow2}"
PORT="${SSH_PORT:-2223}"

pkill -9 -f "qemu-system-x86_6[4]" 2>/dev/null || true

# Wait for it to actually die — verify against the TRUNCATED comm.
for _ in $(seq 30); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 1; done
if pgrep -x qemu-system-x86 >/dev/null; then
    echo "FATAL: qemu still alive after pkill:" >&2
    pgrep -a -x qemu-system-x86 >&2
    exit 1
fi

# ...and for the dying QEMU to release the hostfwd port.
for _ in $(seq 30); do ss -tln | grep -q ":${PORT} " || break; sleep 1; done
if ss -tln | grep -q ":${PORT} "; then
    echo "FATAL: port ${PORT} still bound; a new QEMU would die on hostfwd" >&2
    ss -tlnp | grep ":${PORT} " >&2
    exit 1
fi

rm -f "$OVL"

cd "$REPO" || exit 1
export NVKVM_M2CEFWD="${NVKVM_M2CEFWD:-1}"
nohup setsid bash scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 </dev/null &
disown
echo "launched (log /tmp/m0_launch.log, serial /tmp/m0_serial.log, qemu /tmp/m0_qemu.log)"
echo "now run: bash scripts/mode2_diag/bench_wait.sh"
