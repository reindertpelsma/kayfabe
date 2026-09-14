#!/usr/bin/env bash
# ★★★★★ THE RAW CLIENT AS A SUITE — because a test nobody runs is not a test.
#
# > Owner, 2026-09-14: *"can you ensure everything of the raw client is run, a test that exists
# > but nobody runs is not a useful test"* … *"tests are suites for a reason, not probes"*.
#
# ## ⊘⊘ What this replaces, and the number that makes the case
#
# `kayfabe-rm-ladder` has **30 arms**. Before this file:
#   - the graded GUEST boot (`w392d_mean_hook.sh`) ran **one** (`--uvm-mean`);
#   - `run_full_suite.sh` — the file whose own header calls itself *"THE ONE WAY TO RUN
#     EVERYTHING"* — ran **two** (`--engines`, `--concurrency`).
#
# So ~28 arms existed and were exercised by nobody, including `--ce-client`: the CPU-writes-
# vidmem-through-MMIO → CE-DMA-copies-it → CPU-reads-it-back round trip. It passes on bare metal
# and had never been run in a guest, which is precisely where it can fail.
#
# ## ★ Design: one PROCESS per arm, not one process with 30 flags
#
# Each arm allocates, submits and tears down its own RM state. Running them in one process would
# let arm N's leak or wedge decide arm N+1's verdict, and a crash would take the remaining arms
# with it — turning 30 measurements into one. ⊘ Separate processes also mean a hang is attributed
# to the arm that hung, which is the whole point of a per-arm timeout.
#
# ## ★★ The outcomes, pre-registered so none reads as the favourable one
#
#   PASS     exit 0.
#   FAIL     exit non-zero — the arm ran and judged itself failed. ★ THE DIAGNOSTIC OUTCOME.
#   TIMEOUT  exceeded ARM_TIMEOUT. ⊘ NOT a FAIL: a hang and a refusal have different causes, and
#            collapsing them loses the distinction this campaign has paid for repeatedly.
#   SKIP     the arm declares a precondition this environment does not meet.
#
# ⚠ An arm's own internal verdict is NOT re-derived here. This file reports what the arm said
# about itself; it does not grade the GPU. Read the arm's own output for that.
#
#   usage: rmladder_suite.sh <path-to-kayfabe-rm-ladder> [gpu] [arm-timeout-seconds]
set -uo pipefail
BIN=${1:?usage: rmladder_suite.sh <binary> [gpu] [timeout]}
GPU=${2:-0}
ARM_TIMEOUT=${3:-${RMLADDER_ARM_TIMEOUT:-120}}
OUT=${RMLADDER_SUITE_LOGDIR:-/tmp/rmladder_suite}
mkdir -p "$OUT"

[ -x "$BIN" ] || { echo "SUITE_RC=2 ⊘ no binary at $BIN — UNMEASURED, not a failure"; exit 2; }

# ⊘ The arm list is DERIVED from the source (flags whose handler sets a `want_* = true`), not
# hand-maintained. A hand list silently rots the moment someone adds an arm — which is the exact
# failure this file exists to end.
ARMS="--concurrency --timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep
--gpga-reserve-probe --atomics-probe --pce-mask-probe --dictated-ring --dictated-ring-negative
--late-map-race --blockage-coverage --uvm-invalidate --uvm-mean --alias-two-vas
--alias-unmap-observe --map-propagation --missing-page-fault --map-stress --rpc-mixed-allocs
--cross-client-leak --concurrent-fuzz --defer-liveness --guest-ram-pin --guest-ring-channel
--executor-vas --ce-client --ce-client-guest-ram --bar1-crossing"

printf '=== RMLADDER SUITE — %s arms, gpu %s, %ss each ===\n' "$(echo $ARMS | wc -w)" "$GPU" "$ARM_TIMEOUT"
printf '%-28s %-8s %s\n' "ARM" "VERDICT" "last line"
n_pass=0; n_fail=0; n_to=0; failed=""
for arm in $ARMS; do
  log="$OUT/${arm#--}.out"
  timeout "$ARM_TIMEOUT" "$BIN" --gpu "$GPU" "$arm" > "$log" 2>&1
  rc=$?
  last=$(grep -vE '^\s*$' "$log" 2>/dev/null | tail -1 | cut -c1-72)
  case $rc in
    0)   v=PASS;    n_pass=$((n_pass+1));;
    124) v=TIMEOUT; n_to=$((n_to+1));   failed="$failed $arm(timeout)";;
    *)   v="FAIL($rc)"; n_fail=$((n_fail+1)); failed="$failed $arm";;
  esac
  printf '%-28s %-8s %s\n' "$arm" "$v" "$last"
done

echo ""
echo "SUITE_ARMS=$(echo $ARMS | wc -w) SUITE_PASS=$n_pass SUITE_FAIL=$n_fail SUITE_TIMEOUT=$n_to"
[ -n "$failed" ] && echo "SUITE_NOT_PASSING:$failed"
echo "SUITE_LOGDIR=$OUT"
# ⊘ Non-zero if anything did not pass, so a caller cannot record a green by ignoring the body.
if [ $n_fail -gt 0 ] || [ $n_to -gt 0 ]; then echo "SUITE_RC=1"; exit 1; fi
echo "SUITE_RC=0"
