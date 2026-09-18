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

# ⊘⊘⊘ **ONE WHITESPACE-FREE TOKEN, OR THE ARMS SILENTLY DO NOT ARRIVE.** The kernel command
# line is split on whitespace and no quoting survives it, so a space-separated `KF_ARMS`
# reaches `/init` truncated at the first space and the guest runs the DEFAULT arms while the
# log says it was asked for others — a green run that measured the wrong thing. Commas here,
# `tr ',' ' '` in `/init`.
ARMS_TOK=$(echo "${KF_ARMS:-}" | tr -s ' ' ',' | sed 's/^,//; s/,$//')
[ -n "$ARMS_TOK" ] || ARMS_TOK="--timer,--engines,--doorbell-census"
case "$ARMS_TOK" in *[[:space:]]*) echo "run_fast_guest: KF_ARMS still holds whitespace after folding: [$ARMS_TOK]" >&2; exit 2 ;; esac

# ★ The client's OWN deadline fires INSIDE the guest, below the harness budget, so it can
# dump the ioctl ring over the serial console before QEMU is killed from outside. A deadline at
# or above the outer budget is a deadline that never speaks.
DEADLINE_MS=$(( (BUDGET - 4) * 1000 ))
[ "$DEADLINE_MS" -gt 1000 ] || DEADLINE_MS=1000

echo "== arms: $ARMS_TOK   budget: ${BUDGET}s   self-deadline: ${DEADLINE_MS}ms"

# ⊘⊘⊘ **THE DEVICE LINE AND THE RAM BACKING ARE NOT THE FAST LANE’S TO INVENT.** As first
# written this file said `-device kayfabe-gpu` and `-machine q35,accel=kvm -m 4096`, and BOTH
# were wrong in a way that would have been read as a kayfabe defect:
#
#   1. The QOM type is **`nvkvm-gpu`**, not `kayfabe-gpu` — QEMU would have exited before the
#      kernel ran, and the verdict printed would have been `CRASH`, indistinguishable here from
#      a guest that hung.
#   2. `-m` ALONE gives an anonymous `MAP_PRIVATE` block **no other process can see**. The
#      scratchpad isolate lives in another process and adopts the hypervisor’s
#      `memory-backend-memfd` to reach guest RAM at all; without `share=on` the guest-RAM
#      crossing never arms and every store-backed path refuses — a REAL failure caused
#      entirely by the harness, on the lane built to stop exactly that.
#
# ⇒ Both derive from the fat guest’s own boot script (`scripts/bench/boot_nvkvm.sh`) rather
# than being restated here, so the two lanes cannot silently diverge on the thing under test.
NVKVM_RAM_MB=${NVKVM_RAM_MB:-2048}
RAMARGS=(-object "memory-backend-memfd,id=ram0,size=${NVKVM_RAM_MB}M,share=on"
         -machine "q35,accel=kvm,memory-backend=ram0" -m "$NVKVM_RAM_MB")

# ⚠ One variable, both halves: the device registers this BAR1 and the chip row tells the guest
# the same number. `nvkvm_apply_identity` refuses at realize if they differ.
# ★ **128, not 256** -- and the device told us so itself, in one line, in 4 seconds:
#   "a guest BAR1 of 256 MiB does not fit: 256 MiB + 16 MiB headroom > 256 MiB host aperture.
#    The largest that fits here is 128 MiB, and a 128-MiB-BAR1 GA106 is a real hardware
#    configuration -- a different truthful board, not a lie."
# ⊘ `boot_nvkvm.sh` defaults to 256 because the fat lane passes this variable explicitly on
# every invocation; a lane that passes NOTHING must default to something that boots. 128 is also
# the plan-of-record value for the single store, so the two agree.
BAR1_BYTES=$(( ${KAYFABE_GUEST_BAR1_MB:-128} * 1024 * 1024 ))
export KAYFABE_GUEST_BAR1_MB=${KAYFABE_GUEST_BAR1_MB:-128}

start=$(date +%s)
timeout --kill-after=3 "$BUDGET" "$Q" \
    "${RAMARGS[@]}" -cpu host -smp "${KF_SMP:-3}" \
    -kernel "$FG/vmlinuz" -initrd "$FG/initrd.cpio.gz" \
    -append "console=ttyS0 quiet panic=1 KF_ARMS=$ARMS_TOK KF_IOCTL_TRACE=${KF_IOCTL_TRACE:-ring} KF_SELF_DEADLINE_MS=$DEADLINE_MS" \
    -device "nvkvm-gpu,bar1-size=$BAR1_BYTES,bar2-size=33554432,id=kf0${NVKVM_DEV_EXTRA:+,$NVKVM_DEV_EXTRA}" \
    -msg timestamp=on \
    -serial "file:$SER" -display none \
    > "$BENCH/fast_${TAG}_qemu.log" 2>&1
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
