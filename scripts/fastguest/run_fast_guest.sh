#!/usr/bin/env bash
# ★★★★★ ONE ITERATION OF THE FAST LANE — boot, run, verdict, under a HARD budget.
#
# usage: run_fast_guest.sh [tag] [budget-seconds]
#   KF_ARMS="--timer --engines"   which raw-client arms to run (default: a small set)
#
# ## The rule this file exists to enforce: A TIMEOUT IS A CRASH.
#
# `[measured w760]` the fat-guest suite gave a timing-out arm 300 s, SIGKILLed it, and the arm
# AFTER it then failed on a device the kill had left unusable. Hours went into separating "this
# arm is broken" from "its predecessor poisoned it" — and the answer was collateral three times
# out of three. Under one whole-run budget that category cannot exist: nothing runs long enough
# to poison a successor, and a slow path is simply RED.
#
# ⇒ Perf is not a separate lane. A regression that doubles a round trip fails this run.
#
# ⊘ The budget covers EVERYTHING — QEMU start, kernel, insmod, the client, poweroff. A boot that
# hangs and a client that hangs are the same verdict here, deliberately: both mean "we cannot
# iterate", and distinguishing them is the serial log's job, not the harness's.
set -uo pipefail

TAG=${1:-fast}
BUDGET=${2:-20}
BENCH=${BENCH_DIR:-/workspace/bench}
FG=$BENCH/fastguest
Q=${QEMU_BIN:-$BENCH/qemu-build/qemu-system-x86_64}
SER=$BENCH/fast_${TAG}_serial.log

for f in "$FG/vmlinuz" "$FG/initrd.cpio.gz" "$Q"; do
    [ -f "$f" ] || { echo "run_fast_guest: missing $f — run build_fast_guest.sh"; exit 2; }
done
# ⚠ Serial, not ssh. See build_fast_guest.sh for the two boots this cost before it was serial.
rm -f "$SER"

# ⊘ `pkill` on its own line and with the bracket trick: a pattern that appears later on the same
# command line matches the shell running it, and then everything after silently never runs
# (`nvkvm-pv`, 2026-08-17).
pkill -9 -x qemu-system-x86 2>/dev/null
sleep 1

start=$(date +%s)
timeout --kill-after=3 "$BUDGET" "$Q" \
    -machine q35,accel=kvm -cpu host -m "${KF_MEM:-4096}" -smp "${KF_SMP:-4}" \
    -kernel "$FG/vmlinuz" -initrd "$FG/initrd.cpio.gz" \
    -append "console=ttyS0 quiet panic=1 KF_ARMS=\"${KF_ARMS:-}\"" \
    -device "${KF_DEVICE:-kayfabe-gpu}" \
    -nographic -serial "file:$SER" -display none \
    >/dev/null 2>&1
rc=$?
elapsed=$(( $(date +%s) - start ))

echo "=== FAST GUEST $TAG — ${elapsed}s of a ${BUDGET}s budget, qemu rc=$rc ==="
grep -a 'FASTGUEST:' "$SER" 2>/dev/null | sed 's/^/    /'

# ★ 124 is `timeout`'s own code. ⊘ Reported as a CRASH and not as "slow": that is the whole rule.
if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
    echo "FAST_VERDICT=CRASH (budget ${BUDGET}s exceeded — a timeout IS a crash)"
    echo "⊘ the serial log's last lines are where it stopped:"
    tail -5 "$SER" 2>/dev/null | tr -d '\r' | sed 's/^/    /'
    exit 1
fi
if ! grep -aq 'FASTGUEST: DONE' "$SER" 2>/dev/null; then
    echo "FAST_VERDICT=CRASH (no DONE marker — the guest died before reporting)"
    exit 1
fi
# ⊘ The client's own rc, not QEMU's: a clean poweroff with a failing client is still a failure.
crc=$(grep -ao 'FASTGUEST: client rc=[0-9]*' "$SER" | tail -1 | grep -o '[0-9]*$')
if [ "${crc:-1}" != 0 ]; then
    echo "FAST_VERDICT=FAIL (raw client rc=${crc:-?})"
    exit 1
fi
echo "FAST_VERDICT=PASS (${elapsed}s)"
