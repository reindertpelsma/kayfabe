#!/usr/bin/env bash
# ★★★★★ w373 — PER-CYCLE guest-kernel delta across the 5th-device-open wall.
#
# w372 got the guest kernel talking for the FIRST TIME in this series (w361..w372 all
# captured dmesg at BOOT ONLY -- 5379 bytes, byte-identical, mtime before the workloads).
# ⊘ w372's per-step delta was still broken: RD held a `a || b || c` chain, so
#   "$RD | wc -l" bound the pipe to the LAST alternative only and `base` got the whole log
#   instead of a count -> $((base+1)) threw a syntax error every step. Fixed: one reader,
#   no chain. ⚠ The failure printed a screenful and still exited 0.
#
# Boot-time capture already showed, BEFORE any workload:
#   NVRM: vaListDestroy: non-zero mapCount(pVaList): 0x1   x4  <- live mappings AT DESTROY
#   NVRM: Assertion failed: status == NV_OK @ kernel_rc_watchdog.c:1198
#   NVRM: kgrobjPromoteContext ... kernel_graphics_object.c:224  status=0x56
#   NVRM: GspRmAlloc failed: hClass=0x208f / 0x402c / 0xc36f / 0x70  status=0x56
# Guest module is the OPEN kernel module 580.159.04 => ogkm patching is available.
#
# THE QUESTION THIS RUN ANSWERS: which of those is PER-CYCLE (accumulates 1..4 then breaks
# at 5) and which is BOOT-ONLY noise present in every passing cycle? A line that appears in
# cycles 1-4 as well CANNOT be the cause -- same negative-control rule that killed
# PUBCONFLICT and the NoVas refusal.
set -uo pipefail
SELF=$(readlink -f "$0")
STEP_TIMEOUT=${STEP_TIMEOUT:-120}
if [ "${W373_ROLE:-}" = hook ]; then
  TAG=${1:?tag}; REPO=${KAYFABE_REPO:?}; G="$REPO/scripts/bench/gssh_nv"
  D=/workspace/bench/run_${TAG}
  echo "=== w373 PER-CYCLE GUEST-KERNEL DELTA tag=$TAG ==="
  if ! $G true >/dev/null 2>&1; then echo "W373_OUTCOME=UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi
  probe=$($G "sudo -n dmesg 2>&1 | wc -l" 2>&1 | tr -d '\r')
  case "$probe" in ''|*[!0-9]*) echo "⊘ W373_UNMEASURED: cannot read kernel buffer ('$probe'). This is NOT an absence of NVRM lines."; exit 0 ;; esac
  echo "GUEST_DMESG_WATERMARK=$probe lines (before step 1)"
  base=$probe
  for i in 1 2 3 4 5 6 7 8; do
    out=$($G "timeout ${STEP_TIMEOUT} nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1" 2>&1 | tr -d '\r')
    [ -n "$out" ] && echo "SMI#$i => $out" || echo "SMI#$i => ⊘ HUNG"
    $G "sudo -n dmesg | tail -n +$((base+1))" > "$D.cyc$i" 2>/dev/null
    n=$(wc -l < "$D.cyc$i"); nv=$(grep -aci NVRM "$D.cyc$i" || echo 0)
    echo "  cycle$i: $n new kernel lines, $nv NVRM"
    grep -aoE "@ [a-z_0-9]+\.c:[0-9]+|hClass=0x[0-9a-f]+|Xid[^ ]*|vaListDestroy[^ ]*|RmInitAdapter[^\"]*" "$D.cyc$i" 2>/dev/null \
      | sort | uniq -c | sort -rn | head -12 | sed "s/^/    cyc$i| /"
    base=$($G "sudo -n dmesg | wc -l" 2>/dev/null | tr -d '\r'); base=${base:-0}
  done
  echo "--- ★★★★★ WHAT IS UNIQUE TO THE FAILING CYCLE (in cyc5, absent from cyc1-4) ---"
  cat "$D.cyc1" "$D.cyc2" "$D.cyc3" "$D.cyc4" 2>/dev/null | grep -aoE "@ [a-z_0-9]+\.c:[0-9]+" | sort -u > "$D.pass_sites"
  grep -aoE "@ [a-z_0-9]+\.c:[0-9]+" "$D.cyc5" 2>/dev/null | sort -u > "$D.fail_sites"
  echo "sites in cyc5 NOT in cyc1-4:"; comm -13 "$D.pass_sites" "$D.fail_sites" | sed 's/^/    ★ /'
  echo "sites in cyc1-4 NOT in cyc5:"; comm -23 "$D.pass_sites" "$D.fail_sites" | sed 's/^/    ⊘ /'
  echo "--- ★ per-cycle counts of the leak line (does it ACCUMULATE?) ---"
  for i in 1 2 3 4 5 6 7 8; do printf "  cyc%d vaListDestroy=%s NVRM=%s\n" $i \
    "$(grep -aci vaListDestroy "$D.cyc$i" 2>/dev/null || echo 0)" "$(grep -aci NVRM "$D.cyc$i" 2>/dev/null || echo 0)"; done
  $G "sudo -n dmesg" > "$D_guest_dmesg_AFTER.log" 2>/dev/null
  echo "=== w373 STEPS DONE ==="
  exit 0
fi
case "${1:-}" in run) ;; *) echo "usage: $0 run" >&2; exit 64 ;; esac
REPO=${KAYFABE_REPO:?}
export KAYFABE_REPO="$REPO" KAYFABE_TAG=${KAYFABE_TAG:-w373seq} W373_ROLE=hook STEP_TIMEOUT
export POST_CAPTURE_HOOK="$SELF" GQ_TIMEOUT=${GQ_TIMEOUT:-900}
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W373_ARM:-drain}"
