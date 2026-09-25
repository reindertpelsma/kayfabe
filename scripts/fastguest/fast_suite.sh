#!/usr/bin/env bash
# ★★★★★ THE FAST SUITE — every arm, one boot each, one budget each.
#
# > Owner, 2026-09-18: *"Why not make a raw client fast that passes 10s on bare metal, set 20s
# > limit for the entire thing in guest... Each timeout is seen as a crash."*
#
# ⊘ ONE ARM PER BOOT, deliberately. `[measured w760]` the fat suite ran three arms per boot and
# a SIGKILLed arm left the device unusable for its successor — hours went into telling "this arm
# is broken" from "its predecessor poisoned it", and the answer was collateral three times out
# of three. A boot costs ~8 s here; isolation is cheaper than the ambiguity.
#
# usage: fast_suite.sh [tag] [budget-seconds] [arm ...]
set -uo pipefail
TAG=${1:-fastsuite}; shift || true
# ★ Budget (owner, 2026-09-25: "maybe increase timeout for now"): 120 s per arm. Measured on the
# vast benches, which are themselves KVM guests (nested): every MMIO exit costs 70-92 µs instead of
# ~2-5 µs, boot alone is 15-27 s, passing arms take 33-58 s, and even BARE METAL on that box runs
# --ce-client-guest-ram in 17-37 s (9 s on a non-nested reference). 60 s was noise-bound there.
# ⊘ Still a ceiling, not a formality: a hang is a failure. Revisit on a non-nested KVM host.
BUDGET=${1:-120}; shift || true
BENCH=${BENCH_DIR:-/workspace/bench}
ARMS=("$@")
if [ "${#ARMS[@]}" -eq 0 ]; then
    ARMS=(--timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep --concurrency
          --defer-liveness --blockage-coverage --alias-two-vas --alias-unmap-observe
          --map-propagation --late-map-race --missing-page-fault --uvm-invalidate --uvm-mean
          --executor-vas --guest-ring-channel --dictated-ring --dictated-ring-negative
          --ce-client --ce-client-guest-ram --guest-ram-pin --bar1-crossing --atomics-probe
          --pce-mask-probe --gpga-reserve-probe --map-stress --concurrent-fuzz
          --cross-client-leak --rpc-mixed-allocs)
fi
OUT=$BENCH/${TAG}_suite.out
{ echo "FAST_SUITE_STARTED=$(date -Is) arms=${#ARMS[@]} budget=${BUDGET}s"
  echo "rev=$(git -C "$(dirname "$0")/../.." rev-parse --short HEAD 2>/dev/null || echo ?)"
  printf '%-28s %-9s %-5s %s\n' ARM VERDICT secs why
} > "$OUT"
pass=0; fail=0; crash=0; notrun=0
for arm in "${ARMS[@]}"; do
    name=${arm#--}
    line=$(KF_ARMS="$arm" bash "$(dirname "$0")/run_fast_guest.sh" "${TAG}_${name}" "$BUDGET" 2>&1)
    # ⊘⊘⊘ **THE REASON, NOT JUST THE WORD.** `[measured w763]` two arms scored `CRASH` at 2s
    # and 6s of a 30s budget — QEMU exited during boot, which `run_fast_guest.sh` reports as
    # `CRASH (no DONE marker)` and a budget overrun as `CRASH (budget exceeded)`. Grepping
    # `[A-Z]*` collapsed the two into one word, so "the arm is too slow" and "the VMM died
    # before the arm ran" landed in the same column. ⚠ Third conflation in this harness:
    # `FAIL` also covers a self-deadline abort, and a missing device node looked like an arm
    # failure. A verdict vocabulary that merges causes makes the scoreboard unreadable
    # exactly where it matters.
    v=$(echo "$line" | grep -o 'FAST_VERDICT=[A-Z]*' | head -1 | cut -d= -f2)
    why=$(echo "$line" | grep -o 'FAST_VERDICT=[A-Z]* ([^)]*)' | head -1 | sed 's/.*(//; s/)//')
    case "$why" in
        *"budget"*)      v=TIMEOUT ;;
        *"DONE marker"*) v=VMM_DIED ;;
    esac
    secs=$(echo "$line" | grep -oE '— [0-9]+s of a' | grep -oE '[0-9]+' | head -1)
    # ⊘⊘⊘ **FOURTH CONFLATION, AND IT COST A WHOLE READING `[w824]`.** This `case` used to end
    # `*) v=${v:-TIMEOUT}` — so an arm that produced **no verdict at all** was scored `TIMEOUT`,
    # the word reserved for *"ran and exceeded its budget"*. On 2026-09-21 that printed
    # **30/30 TIMEOUT**, which reads as a catastrophic regression. The truth was a missing
    # `vmlinuz`: `run_fast_guest.sh` had refused in one clear line and nothing was ever booted.
    #
    # ★ **`?s` was the only tell, and it is not a word anyone reads.** A real timeout records its
    # seconds; an arm that never began cannot. ⇒ A verdict and a missing field disagreed, and the
    # verdict is what a reader believes.
    #
    # ⇒ **NOTRUN is its own word.** "Never started" and "took too long" are different causes, and
    # this harness already learned that twice (CRASH split into VMM_DIED, and w823's three 45 s
    # TIMEOUTs splitting at 300 s into a correctness bug, a hang and a 3.8x slowdown). ⚠ The arm
    # binary has had `RUNG_<arm>=NOTRUN` since w381; only this script lacked it.
    notrun_this=0
    case "$v" in
        PASS)     pass=$((pass+1)) ;;
        FAIL)     fail=$((fail+1)) ;;
        VMM_DIED) crash=$((crash+1)) ;;
        TIMEOUT)  crash=$((crash+1)) ;;
        "")       v=NOTRUN; notrun=$((notrun+1)); notrun_this=1
                  [ -n "$why" ] || why="no FAST_VERDICT line — a precondition refused before the arm ran" ;;
        *)        crash=$((crash+1)) ;;
    esac
    [ "$notrun_this" = 1 ] && [ -z "$secs" ] && secs="-"
    # ★ w827: the raw client's OWN wall inside the guest (ns stamps around it in /init), boot
    # excluded — the number comparable to bare_metal_suite.sh's per-arm `ms=`.
    cms=$(grep -ao 'FASTGUEST: client wall_ms=[0-9]*' "$BENCH/fast_${TAG}_${name}_serial.log" 2>/dev/null | tail -1 | grep -o '[0-9]*$')
    printf '%-28s %-9s %-5s %s\n' "$arm" "$v" "${secs:-?}s" "$why" >> "$OUT"
    echo "FAST_CELL_ARM arm=$name verdict=$v secs=${secs:-?} client_ms=${cms:-?}" >> "$OUT"
done
# ⊘⊘⊘ **AND THE SUITE USED TO HARDCODE `FAST_SUITE_RC=0`** — it reported a scoreboard and
# gated on nothing, so a caller chaining on it proceeded over any result at all. `[w824]` five
# scripts in this tree did the same thing in one session. ⇒ The exit code IS the verdict.
# ⊘ Explicit `if`, not an `A || B && C` chain: shell precedence there is left-to-right and
# reads as if it were grouped the other way. A gate whose own logic needs a second look is the
# kind that silently passes.
rc=0
if [ "$notrun" -gt 0 ]; then
    rc=2                                   # the suite did NOT run — distinct from "ran and failed"
elif [ "$fail" -gt 0 ] || [ "$crash" -gt 0 ]; then
    rc=1
fi
{ echo
  echo "FAST_SUITE_PASS=$pass FAST_SUITE_FAIL=$fail FAST_SUITE_CRASH=$crash NOTRUN=$notrun ARMS=${#ARMS[@]}"
  echo "FAST_SUITE_RC=$rc"
  [ "$notrun" -gt 0 ] && echo "⊘ $notrun arm(s) NEVER RAN — this is not a measurement. Fix the precondition and re-run."
} >> "$OUT"
cat "$OUT"
exit "$rc"
