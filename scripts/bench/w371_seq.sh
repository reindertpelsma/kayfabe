#!/usr/bin/env bash
# ★★★★★ w371 — NAME THE FAILING CALL. Zero rebuild.
#
# MEASURED w370: nvidia-smi x8, no CUDA at all -> #1-4 print the GPU, #5-8 "No devices
# were found", and a torch init after them fails too. So the resource is consumed by
# DEVICE OPEN, not by creating a CUDA context. From cycle 5 our GSP plane answers
# `PeerWritePtrOutOfRange` forever: we are reading a msgq the guest tore down.
#
# ⊘ THE HARNESS HAS NEVER MEASURED THE GUEST KERNEL DURING A WORKLOAD. `*_dmesg.log` is
#   5379 bytes on EVERY boot w361..w370, byte-identical, mtime at BOOT START, last line at
#   28.99 s uptime. I reported "the guest kernel logged nothing" as a measured fact. It was
#   never measured -- the capture stopped before the workloads began. CLAUDE.md's serial-log
#   trap, verbatim. This script fixes that permanently: dmesg AFTER each step, and the full
#   log persisted at the end.
#
# NVRM prints `RmInitAdapter failed! (0xSS:0xII:LLLL)` at DEFAULT verbosity, and LLLL is the
# SOURCE LINE in the 580.159.04 tree we have checked out locally. Pre-registered readings:
#   RmInitAdapter only after #5  => look up LLLL in research_clones/ogkm-580.159.04, that is
#                                   the failing call. status 0x65/timeout => the guest waited
#                                   on a reply we never served => PC-D7 cursor carry at the
#                                   Nth rebind (boot.rs:1135-1176).
#   `Timeout waiting for RPC`    => guest never saw a status-queue message at cycle 5 =>
#                                   the TxCursor::fresh-vs-preserved-read-ptr asymmetry.
#   NVRM errors after #4's CLOSE => consumed at CLOSE; the 5th open is collateral.
#   silent through #5            => failure is ABOVE the kernel; only then reach for nvdiff.
set -uo pipefail
SELF=$(readlink -f "$0")
STEP_TIMEOUT=${STEP_TIMEOUT:-120}
if [ "${W371_ROLE:-}" = hook ]; then
  TAG=${1:?tag}; REPO=${KAYFABE_REPO:?}; G="$REPO/scripts/bench/gssh_nv"
  echo "=== w371 NAME-THE-FAILING-CALL tag=$TAG (nvidia-smi x8 + per-step guest dmesg) ==="
  if ! $G true >/dev/null 2>&1; then echo "W371_OUTCOME=UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi
  # watermark: how many kernel lines exist BEFORE any workload
  base=$($G "dmesg | wc -l" 2>/dev/null | tr -d '\r'); base=${base:-0}
  echo "GUEST_DMESG_WATERMARK=$base  (lines already present before step 1)"
  for i in 1 2 3 4 5 6 7 8; do
    out=$($G "timeout ${STEP_TIMEOUT} nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1" 2>&1 | tr -d '\r')
    [ -n "$out" ] && echo "SMI#$i => $out" || echo "SMI#$i => ⊘ HUNG (no line in ${STEP_TIMEOUT}s)"
    # ★ the delta ONLY -- absolute tails re-print old lines and read as new events
    d=$($G "dmesg | tail -n +$((base+1)) | grep -aiE 'NVRM|Xid|nvidia' | tail -25" 2>&1 | tr -d '\r')
    if [ -n "$d" ]; then echo "$d" | sed "s/^/  SMI#$i dmesg| /"; else echo "  SMI#$i dmesg| (no new NVRM/Xid lines)"; fi
    base=$($G "dmesg | wc -l" 2>/dev/null | tr -d '\r'); base=${base:-$base}
  done
  echo "--- ★ persist the FULL guest dmesg (the harness has never done this) ---"
  $G "dmesg" > "/workspace/bench/run_${TAG}_guest_dmesg_AFTER.log" 2>/dev/null
  n=$(wc -l < "/workspace/bench/run_${TAG}_guest_dmesg_AFTER.log" 2>/dev/null || echo 0)
  nv=$(grep -aci NVRM "/workspace/bench/run_${TAG}_guest_dmesg_AFTER.log" 2>/dev/null || echo 0)
  echo "GUEST_DMESG_AFTER lines=$n NVRM=$nv  ⚠ if lines==0 or NVRM==0 this capture MEASURED NOTHING"
  echo "--- ★★★★★ the money line, if the guest printed it ---"
  grep -aoE "RmInitAdapter failed![^\"]*" "/workspace/bench/run_${TAG}_guest_dmesg_AFTER.log" 2>/dev/null | sort | uniq -c | head
  grep -aiE "Timeout waiting|rpc|GSP" "/workspace/bench/run_${TAG}_guest_dmesg_AFTER.log" 2>/dev/null | tail -15
  echo "=== w371 STEPS DONE ==="
  exit 0
fi
case "${1:-}" in run) ;; *) echo "usage: $0 run" >&2; exit 64 ;; esac
REPO=${KAYFABE_REPO:?}
export KAYFABE_REPO="$REPO" KAYFABE_TAG=${KAYFABE_TAG:-w371seq} W371_ROLE=hook STEP_TIMEOUT
export POST_CAPTURE_HOOK="$SELF" GQ_TIMEOUT=${GQ_TIMEOUT:-900}
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W371_ARM:-drain}"
