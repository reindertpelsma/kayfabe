#!/usr/bin/env bash
# ★★★★★ ONE RUN OF THE ADVERSARIAL GUEST — boot, attack, verdict, under a HARD budget.
#
# Modelled on run_fast_guest.sh and it inherits that file's central rule:
#
#   ## A TIMEOUT IS A CRASH.
#
# The whole run gets ONE budget covering QEMU start, kernel, driver load, the
# suite and poweroff. A VMM that HANGS inside a trap handler wedges the vCPU on
# its MMIO exit and no more guest code runs — this budget is the only thing that
# catches that, and it is reported as CRASH, not "slow". A VMM that FAULTED and
# stopped decoding is caught in-guest by the module's liveness check (fail=N).
#
# usage: run_adv_guest.sh [tag] [budget-seconds]
#   ADV_STORM=N ADV_THREADS=N   passed through to the module
#   ADV_POST=1                  attack a post-init GPU (initrd must have been built
#                               with ADV_POST_INIT=1)
set -uo pipefail

TAG=${1:-adv}
BUDGET=${2:-30}
BENCH=${BENCH_DIR:-/workspace/bench}
AG=$BENCH/advguest
Q=${QEMU_BIN:-$BENCH/qemu-build/qemu-system-x86_64}
SER=$BENCH/adv_${TAG}_serial.log
QLOG=$BENCH/adv_${TAG}_qemu.log

for f in "$AG/vmlinuz" "$AG/initrd.cpio.gz" "$Q"; do
    [ -f "$f" ] || { echo "run_adv_guest: missing $f — run build_adv_guest.sh"; exit 2; }
done
rm -f "$SER" "$QLOG"

# ⊘ kill on its own line, bracket trick, no later word naming the binary.
pkill -9 -x qemu-system-x86 2>/dev/null
sleep 1

# ⚠ Same stale-binary refusal as the fast lane: the device is a Rust archive
# linked into qemu-system-x86_64, so a source change never relinked runs the OLD
# device while the tree says otherwise. Refuse when the archive's sources are
# newer than the linked binary.
KF_ROOT=${KF_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
if [ -d "$KF_ROOT/crates" ]; then
    newest=$(find "$KF_ROOT/crates" -name '*.rs' -newer "$Q" -print -quit 2>/dev/null)
    if [ -n "$newest" ]; then
        echo "run_adv_guest: ⊘⊘ REFUSED — $Q is OLDER than $newest (relink did not happen)." >&2
        exit 3
    fi
fi
echo "== qemu:  $Q  (built $(date -r "$Q" "+%Y-%m-%d %H:%M:%S" 2>/dev/null || echo unknown))"

# Device line + RAM backing derived exactly as the fast lane (a memfd shared with
# the isolate/archive, or every store-backed path refuses; nvkvm-gpu, not
# kayfabe-gpu; BAR1=128 MiB fits the host aperture). See run_fast_guest.sh.
NVKVM_RAM_MB=${NVKVM_RAM_MB:-2048}
RAMARGS=(-object "memory-backend-memfd,id=ram0,size=${NVKVM_RAM_MB}M,share=on"
         -machine "q35,accel=kvm,memory-backend=ram0" -m "$NVKVM_RAM_MB")
export KAYFABE_GUEST_BAR1_MB=${KAYFABE_GUEST_BAR1_MB:-128}
BAR1_BYTES=$(( KAYFABE_GUEST_BAR1_MB * 1024 * 1024 ))

ADV_STORM=${ADV_STORM:-4096}
ADV_THREADS=${ADV_THREADS:-4}
ADV_POST=${ADV_POST:-0}

echo "== budget: ${BUDGET}s  storm: $ADV_STORM  threads: $ADV_THREADS  post_init: $ADV_POST"

start=$(date +%s)
timeout --kill-after=3 "$BUDGET" "$Q" \
    "${RAMARGS[@]}" -cpu host -smp "${ADV_SMP:-4}" \
    -kernel "$AG/vmlinuz" -initrd "$AG/initrd.cpio.gz" \
    -append "console=ttyS0 panic=1 loglevel=6 ADV_STORM=$ADV_STORM ADV_THREADS=$ADV_THREADS ADV_POST=$ADV_POST" \
    -device "nvkvm-gpu,bar1-size=$BAR1_BYTES,bar2-size=33554432,id=kf0${NVKVM_DEV_EXTRA:+,$NVKVM_DEV_EXTRA}" \
    -msg timestamp=on \
    -serial "file:$SER" -display none \
    > "$QLOG" 2>&1
rc=$?
elapsed=$(( $(date +%s) - start ))

echo "=== ADV GUEST $TAG — ${elapsed}s of a ${BUDGET}s budget, qemu rc=$rc ==="
grep -a 'ADVGUEST' "$SER" 2>/dev/null | sed 's/\r$//; s/^/    /'

# ── verdict ─────────────────────────────────────────────────────────────────────
# 124/137 = timeout/kill = CRASH, by the rule at the top of this file.
if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
    echo "ADV_VERDICT=CRASH (budget ${BUDGET}s exceeded — a timeout IS a crash; VMM hang or guest wedge)"
    echo "⊘ serial tail:"; tail -6 "$SER" 2>/dev/null | tr -d '\r' | sed 's/^/    /'
    exit 1
fi

# BEGIN present but TOTAL absent = the suite crashed mid-run (a case wedged the
# guest or the module). Absence is detectable exactly because of the BEGIN marker.
if ! grep -aq 'ADVGUEST_BEGIN' "$SER" 2>/dev/null; then
    echo "ADV_VERDICT=CRASH (no ADVGUEST_BEGIN — the suite never started)"
    exit 1
fi
TOTAL=$(grep -a 'ADVGUEST_TOTAL' "$SER" | tail -1)
if [ -z "$TOTAL" ]; then
    echo "ADV_VERDICT=CRASH (ADVGUEST_BEGIN present but no ADVGUEST_TOTAL — suite died mid-run)"
    exit 1
fi

# The module's own aggregate verdict, written by the code that counts.
GV=$(grep -a 'ADVGUEST_VERDICT=' "$SER" | tail -1 | sed 's/.*ADVGUEST_VERDICT=//')
fail=$(echo "$TOTAL" | grep -o 'fail=[0-9]*' | cut -d= -f2)

# ── host-side refusal-by-name cross-check ────────────────────────────────────────
# The guest cannot see kayfabe's by-name refusals; they land in the qemu log. This
# is a COVERAGE report, not the gate (a true Unallocated non-event emits no line by
# design — dbtable.rs). The gate is fail=0 + alive + no timeout.
echo "== host-side kayfabe refusal lines seen (qemu log):"
grep -aoE 'DOORBELL[^]]*REFUSED[^]]*\]|FaultTag\("[^"]+"\)|kayfabe: [A-Z-]+ .*REFUSED|MMUINVAL-COMPLETE[^\n]*' "$QLOG" 2>/dev/null \
    | sort | uniq -c | sed 's/^/    /' | head -40
[ -s "$QLOG" ] || echo "    (qemu log empty)"

echo "== $TOTAL"
if [ "${fail:-1}" = 0 ] && echo "$GV" | grep -q '^PASS'; then
    echo "ADV_VERDICT=PASS (${elapsed}s — every adversarial input contained; VMM still serving)"
    exit 0
fi
echo "ADV_VERDICT=FAIL (${GV:-no module verdict}; fail=${fail:-?})"
exit 1
