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
BUDGET=${1:-30}; shift || true
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
  printf '%-28s %-8s %s\n' ARM VERDICT "seconds"
} > "$OUT"
pass=0; fail=0; crash=0
for arm in "${ARMS[@]}"; do
    name=${arm#--}
    line=$(KF_ARMS="$arm" bash "$(dirname "$0")/run_fast_guest.sh" "${TAG}_${name}" "$BUDGET" 2>&1)
    v=$(echo "$line" | grep -o 'FAST_VERDICT=[A-Z]*' | head -1 | cut -d= -f2)
    secs=$(echo "$line" | grep -oE '— [0-9]+s of a' | grep -oE '[0-9]+' | head -1)
    case "$v" in
        PASS)  pass=$((pass+1)) ;;
        FAIL)  fail=$((fail+1)) ;;
        *)     v=${v:-CRASH}; crash=$((crash+1)) ;;
    esac
    printf '%-28s %-8s %ss\n' "$arm" "$v" "${secs:-?}" >> "$OUT"
done
{ echo
  echo "FAST_SUITE_PASS=$pass FAST_SUITE_FAIL=$fail FAST_SUITE_CRASH=$crash ARMS=${#ARMS[@]}"
  echo "FAST_SUITE_RC=0"
} >> "$OUT"
cat "$OUT"
